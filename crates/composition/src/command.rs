//! Les commandes de pilotage de la grille — §23 de `docs/ARCHITECTURE.md`.
//!
//! Tout pilote passe par là : clavier, interface, MIDI, OSC. Elles s'appliquent
//! entre deux frames, jamais pendant l'évaluation, et se testent sans GPU.

use crate::clip::ParamSlot;
use crate::hierarchy::Composition;
use crate::ids::{ClipId, ColumnId, DeckId, LayerId};
use frogenx_core::Param;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Command {
    /// Rend un clip actif sur sa piste, et le fait repartir de zéro.
    TriggerClip {
        layer: LayerId,
        clip: ClipId,
    },
    /// Déclenche une colonne sur les pistes du deck actif.
    TriggerColumn {
        column: ColumnId,
    },
    /// Vide une piste : elle ne contribue plus à l'image.
    ClearLayer {
        layer: LayerId,
    },
    SetLayerOpacity {
        layer: LayerId,
        opacity: Param,
    },
    /// Règle un paramètre d'un clip, sans couper sa modulation.
    SetParam {
        clip: ClipId,
        slot: ParamSlot,
        value: f32,
    },
    SetActiveDeck(DeckId),
    SetBpm(f32),
    /// Un tap, à l'instant où la commande s'applique.
    TapTempo,
}

impl Composition {
    /// Applique une commande. Rend `false` si elle est refusée — identifiant
    /// périmé, tempo non fini : jamais de panique (§18.1).
    ///
    /// `t` est l'instant courant, qui ne sert qu'au tap tempo.
    pub fn apply(&mut self, command: Command, t: f64) -> bool {
        match command {
            Command::TriggerClip { layer, clip } => self.trigger(layer, clip),
            Command::TriggerColumn { column } => {
                let mut layers = Vec::new();
                self.layers_in_order(&mut layers);
                self.trigger_column(column, &layers)
            }
            Command::ClearLayer { layer } => self.clear_layer(layer),
            Command::SetLayerOpacity { layer, opacity } => match self.layer_mut(layer) {
                Some(l) => {
                    l.blend.opacity = opacity;
                    true
                }
                None => false,
            },
            Command::SetParam { clip, slot, value } => self
                .clip_mut(clip)
                .is_some_and(|c| c.set_param(slot, value)),
            Command::SetActiveDeck(deck) => self.set_active_deck(deck),
            Command::SetBpm(bpm) => {
                if bpm.is_finite() {
                    self.tempo.tempo = Param::fixed(bpm);
                }
                bpm.is_finite()
            }
            Command::TapTempo => {
                self.tap(t);
                true
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clip::Clip;
    use crate::position::PositionSignal;
    use crate::transport::{PlayMode, Transport};
    use frogenx_core::{Graph, NodeId};
    use frogenx_shape::Ellipse;

    /// Un deck, une piste, deux clips dont le graphe contient une position.
    fn grille() -> (Composition, LayerId, [ClipId; 2], NodeId) {
        let mut c = Composition::new();
        let deck = c.add_deck("test");
        let groupe = c.add_layer_group(deck).unwrap();
        let piste = c.add_layer(groupe).unwrap();
        let mut position = None;
        let mut clip = |c: &mut Composition| {
            let mut g = Graph::new();
            let p = g.add(Box::new(PositionSignal::fixed_speed(1.0)));
            position = Some(p);
            let transport = Transport::Timeline {
                position: p,
                speed: Param::fixed(1.0),
            };
            c.add_clip(
                piste,
                Clip::shape(g, Box::new(Ellipse::circle(0.5)), transport),
            )
            .unwrap()
        };
        let (a, b) = (clip(&mut c), clip(&mut c));
        (c, piste, [a, b], position.unwrap())
    }

    #[test]
    fn declencher_rend_le_clip_actif() {
        let (mut c, piste, [a, b], _) = grille();
        assert!(c.apply(
            Command::TriggerClip {
                layer: piste,
                clip: b
            },
            0.0
        ));
        assert_eq!(c.layer(piste).unwrap().active_clip(), Some(b));
        assert_ne!(c.layer(piste).unwrap().active_clip(), Some(a));
    }

    #[test]
    fn declencher_fait_repartir_le_clip_de_zero() {
        let (mut c, piste, [a, _], p) = grille();
        let declenche = Command::TriggerClip {
            layer: piste,
            clip: a,
        };
        c.apply(declenche, 0.0);
        let g = c.clip_mut(a).unwrap().graph_mut();
        g.eval_frame(0.0, 0.5);
        g.eval_frame(0.5, 0.5);
        assert!((g.value(p) - 1.0).abs() < 1e-6);

        // Même déjà actif : un pad qu'on frappe redémarre (§3bis.2).
        c.apply(declenche, 1.0);
        let g = c.clip_mut(a).unwrap().graph_mut();
        assert_eq!(g.value(p), 0.0);
        g.eval_frame(1.0, 0.5);
        assert!((g.value(p) - 0.5).abs() < 1e-6, "got {}", g.value(p));
    }

    #[test]
    fn un_declenchement_refuse_ne_touche_pas_a_l_etat_du_clip() {
        let (mut c, piste, [a, _], p) = grille();
        let g = c.clip_mut(a).unwrap().graph_mut();
        g.eval_frame(0.0, 0.5);

        let intrus = ClipId::new(99, 0);
        assert!(!c.apply(
            Command::TriggerClip {
                layer: piste,
                clip: intrus
            },
            0.0
        ));
        assert_eq!(c.layer(piste).unwrap().active_clip(), None);
        assert!((c.clip(a).unwrap().graph().value(p) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn vider_une_piste_retire_le_clip_actif() {
        let (mut c, piste, [a, _], _) = grille();
        c.trigger(piste, a);
        assert!(c.apply(Command::ClearLayer { layer: piste }, 0.0));
        assert_eq!(c.layer(piste).unwrap().active_clip(), None);
    }

    #[test]
    fn une_colonne_suit_l_ordre_des_pistes_du_deck_actif() {
        let mut c = Composition::new();
        let deck = c.add_deck("test");
        let groupe = c.add_layer_group(deck).unwrap();
        let (p0, p1) = (c.add_layer(groupe).unwrap(), c.add_layer(groupe).unwrap());
        let clip = |c: &mut Composition, piste| {
            let mut g = Graph::new();
            let position = g.add(Box::new(PositionSignal::fixed_speed(1.0)));
            let transport = Transport::Timeline {
                position,
                speed: Param::fixed(1.0),
            };
            c.add_clip(
                piste,
                Clip::shape(g, Box::new(Ellipse::circle(0.5)), transport),
            )
            .unwrap()
        };
        let (a, b) = (clip(&mut c, p0), clip(&mut c, p1));
        let colonne = c.add_column();
        c.set_column_slot(colonne, 0, Some(a));
        c.set_column_slot(colonne, 1, Some(b));

        assert!(c.apply(Command::TriggerColumn { column: colonne }, 0.0));
        assert_eq!(c.layer(p0).unwrap().active_clip(), Some(a));
        assert_eq!(c.layer(p1).unwrap().active_clip(), Some(b));
    }

    #[test]
    fn l_opacite_d_une_piste_survit_au_changement_de_clip() {
        let (mut c, piste, [a, b], _) = grille();
        let opacity = Param::fixed(0.25);
        assert!(c.apply(
            Command::SetLayerOpacity {
                layer: piste,
                opacity
            },
            0.0
        ));
        c.trigger(piste, a);
        c.trigger(piste, b);
        assert_eq!(c.layer(piste).unwrap().blend.opacity, opacity);
    }

    #[test]
    fn une_commande_regle_un_parametre_du_clip() {
        let (mut c, _, [a, _], _) = grille();
        let slot = ParamSlot::Generator(0);
        assert!(c.apply(
            Command::SetParam {
                clip: a,
                slot,
                value: 0.75
            },
            0.0
        ));
        let rayon = c.clip(a).unwrap().params()[0].params[0].desc.value;
        assert_eq!(rayon, Param::fixed(0.75));

        let perime = ClipId::new(99, 0);
        assert!(!c.apply(
            Command::SetParam {
                clip: perime,
                slot,
                value: 0.75
            },
            0.0
        ));
    }

    #[test]
    fn la_grille_garde_l_ordre_de_ses_colonnes_et_nomme_ses_pistes() {
        let (mut c, piste, ..) = grille();
        let (a, b) = (c.add_column(), c.add_column());
        assert_eq!(c.columns(), [a, b]);
        assert_eq!(c.layer(piste).unwrap().name, "Piste 1");
    }

    #[test]
    fn un_tempo_non_fini_est_refuse() {
        let (mut c, ..) = grille();
        assert!(c.apply(Command::SetBpm(140.0), 0.0));
        assert!(!c.apply(Command::SetBpm(f32::NAN), 0.0));
        assert!(!c.apply(Command::SetBpm(f32::INFINITY), 0.0));
        assert_eq!(c.bpm(), 140.0);
    }

    #[test]
    fn le_tempo_est_sature_a_la_lecture() {
        let (mut c, ..) = grille();
        c.apply(Command::SetBpm(5000.0), 0.0);
        assert_eq!(c.bpm(), 400.0);
        c.apply(Command::SetBpm(-3.0), 0.0);
        assert_eq!(c.bpm(), 20.0);
    }

    #[test]
    fn le_tap_tempo_calcule_le_bpm() {
        let (mut c, ..) = grille();
        for i in 0..4 {
            assert!(c.apply(Command::TapTempo, i as f64 * 0.5));
        }
        assert!((c.bpm() - 120.0).abs() < 1e-3, "got {}", c.bpm());
    }

    #[test]
    fn le_tap_tempo_oublie_apres_un_silence() {
        let (mut c, ..) = grille();
        c.apply(Command::SetBpm(90.0), 0.0);
        c.apply(Command::TapTempo, 0.0);
        c.apply(Command::TapTempo, 0.5);
        assert!((c.bpm() - 120.0).abs() < 1e-3);

        // 10 s plus tard, une nouvelle série à 60 BPM, sans moyenne avec l'ancienne.
        c.apply(Command::TapTempo, 10.5);
        assert!((c.bpm() - 120.0).abs() < 1e-3, "un seul tap ne décide rien");
        c.apply(Command::TapTempo, 11.5);
        assert!((c.bpm() - 60.0).abs() < 1e-3, "got {}", c.bpm());
    }

    #[test]
    fn le_tap_tempo_ne_garde_que_les_derniers_instants() {
        let (mut c, ..) = grille();
        // Lent d'abord, puis rapide : seuls les derniers intervalles comptent.
        let mut t = 0.0;
        for _ in 0..4 {
            c.apply(Command::TapTempo, t);
            t += 1.0;
        }
        for _ in 0..16 {
            c.apply(Command::TapTempo, t);
            t += 0.5;
        }
        assert!((c.bpm() - 120.0).abs() < 1e-2, "got {}", c.bpm());
    }

    #[test]
    fn changer_de_deck_change_les_pistes_composees() {
        let (mut c, piste, ..) = grille();
        let autre = c.add_deck("autre");
        let mut pistes = Vec::new();
        c.layers_in_order(&mut pistes);
        assert_eq!(pistes, vec![piste], "le premier deck reste actif");

        assert!(c.apply(Command::SetActiveDeck(autre), 0.0));
        c.layers_in_order(&mut pistes);
        assert!(pistes.is_empty());

        assert!(c.remove_deck(autre));
        assert!(!c.apply(Command::SetActiveDeck(autre), 0.0), "périmé");
    }

    #[test]
    fn les_pistes_se_composent_dans_l_ordre_de_la_hierarchie() {
        let mut c = Composition::new();
        let deck = c.add_deck("test");
        let (g0, g1) = (
            c.add_layer_group(deck).unwrap(),
            c.add_layer_group(deck).unwrap(),
        );
        let (a, b) = (c.add_layer(g0).unwrap(), c.add_layer(g0).unwrap());
        let d = c.add_layer(g1).unwrap();
        let mut pistes = Vec::new();
        c.layers_in_order(&mut pistes);
        assert_eq!(pistes, vec![a, b, d]);
    }

    #[test]
    fn bpm_sync_avance_en_battements_au_tempo_du_contexte() {
        let mut g = Graph::new();
        let transport = Transport::bpm_sync(&mut g, Param::fixed(4.0), PlayMode::Loop);
        let Transport::BpmSync { position, .. } = transport else {
            panic!("attendu BpmSync");
        };
        g.set_bpm(240.0); // 4 battements par seconde
        for i in 0..10 {
            g.eval_frame(i as f64 * 0.05, 0.05);
        }
        assert!(
            (g.value(position) - 2.0).abs() < 1e-4,
            "got {}",
            g.value(position)
        );
    }
}
