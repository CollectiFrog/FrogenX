//! Les commandes de pilotage de la grille — §23 de `docs/ARCHITECTURE.md`.
//!
//! Tout pilote passe par là : clavier, interface, MIDI, OSC. Elles s'appliquent
//! entre deux frames, jamais pendant l'évaluation, et se testent sans GPU.

use crate::clip::ParamSlot;
use crate::hierarchy::Composition;
use crate::ids::{ClipId, ColumnId, DeckId, LayerId, TrigId};
use crate::trig::{TrigSlot, TrigTarget};
use crate::wiring::{GraphScope, NodeKind, NodeValue, ParamTarget};
use frogenx_core::{AmpScale, AnalysisMethod, FeatureKind, FreqScale, NodeId, Param};
use frogenx_osc::SpectrumZone;
use frogenx_shape::EffectKind;

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
    /// Règle l'amplitude d'un paramètre modulé.
    SetDepth {
        clip: ClipId,
        slot: ParamSlot,
        value: f32,
    },
    /// Règle la vitesse d'un clip, sans couper sa modulation.
    SetClipSpeed {
        clip: ClipId,
        value: f32,
    },
    AddColumn,
    RemoveColumn {
        column: ColumnId,
    },
    /// Ajoute une piste au deck actif.
    AddLayer,
    RemoveLayer {
        layer: LayerId,
    },
    /// Vide une case et libère son clip.
    RemoveClip {
        layer: LayerId,
        column: ColumnId,
    },
    SetActiveDeck(DeckId),
    SetBpm(f32),
    /// Un tap, à l'instant où la commande s'applique.
    TapTempo,
    /// Ajoute un trig sur les graves, sans cible (§3bis.6).
    AddTrig,
    /// Ajoute un trig sur une zone tracée à la souris, sans cible.
    AddTrigAt {
        freq: (f32, f32),
        level: (f32, f32),
    },
    RemoveTrig(TrigId),
    /// Règle une zone ou un compteur, sans couper leur modulation.
    SetTrigParam {
        trig: TrigId,
        slot: TrigSlot,
        value: f32,
    },
    SetTrigTarget {
        trig: TrigId,
        target: TrigTarget,
    },
    /// Le coup suivant redevient le premier du compte.
    ResetTrigCount(TrigId),
    SetAnalysisMethod(AnalysisMethod),
    SetFreqScale(FreqScale),
    SetAmpScale(AmpScale),
    /// La durée d'analyse, en secondes, sans couper sa modulation.
    SetAnalysisWindow(f32),
    /// Le tempo suit, ou non, le BPM détecté (§7.1).
    SetTempoFollow(bool),
    /// L'éditeur de nœuds (§23.10).
    AddNode {
        scope: GraphScope,
        kind: NodeKind,
    },
    RemoveNode {
        scope: GraphScope,
        node: NodeId,
    },
    Connect {
        target: ParamTarget,
        source: NodeId,
    },
    Disconnect {
        target: ParamTarget,
    },
    SetTargetBase {
        target: ParamTarget,
        value: f32,
    },
    SetTargetDepth {
        target: ParamTarget,
        value: f32,
    },
    SetNodeValue {
        scope: GraphScope,
        node: NodeId,
        field: u8,
        value: NodeValue,
    },
    AddEffect {
        clip: ClipId,
        kind: EffectKind,
    },
    RemoveEffect {
        clip: ClipId,
        index: usize,
    },
    /// Branche une feature audio sur un paramètre du clip, ou l'en débranche.
    SetAudioModulation {
        clip: ClipId,
        slot: ParamSlot,
        feature: Option<FeatureKind>,
    },
}

impl Command {
    /// Change-t-elle ce qu'un fichier enregistre (§23.8) ? Déclencher est jouer,
    /// pas modifier. Sans cas par défaut : une nouvelle commande oblige à trancher.
    pub fn is_edit(&self) -> bool {
        match self {
            Self::TriggerClip { .. }
            | Self::TriggerColumn { .. }
            | Self::ClearLayer { .. }
            | Self::ResetTrigCount(_) => false,
            Self::SetLayerOpacity { .. }
            | Self::SetParam { .. }
            | Self::SetDepth { .. }
            | Self::SetClipSpeed { .. }
            | Self::AddColumn
            | Self::RemoveColumn { .. }
            | Self::AddLayer
            | Self::RemoveLayer { .. }
            | Self::RemoveClip { .. }
            | Self::SetActiveDeck(_)
            | Self::SetBpm(_)
            | Self::TapTempo
            | Self::AddTrig
            | Self::AddTrigAt { .. }
            | Self::RemoveTrig(_)
            | Self::SetTrigParam { .. }
            | Self::SetTrigTarget { .. }
            | Self::SetTempoFollow(_)
            | Self::AddNode { .. }
            | Self::RemoveNode { .. }
            | Self::Connect { .. }
            | Self::Disconnect { .. }
            | Self::SetTargetBase { .. }
            | Self::SetTargetDepth { .. }
            | Self::SetNodeValue { .. }
            | Self::AddEffect { .. }
            | Self::RemoveEffect { .. }
            | Self::SetAudioModulation { .. }
            | Self::SetAnalysisMethod(_)
            | Self::SetFreqScale(_)
            | Self::SetAmpScale(_)
            | Self::SetAnalysisWindow(_) => true,
        }
    }
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
            Command::SetDepth { clip, slot, value } => self
                .clip_mut(clip)
                .is_some_and(|c| c.set_depth(slot, value)),
            Command::SetClipSpeed { clip, value } => match self.clip_mut(clip) {
                Some(c) if value.is_finite() => {
                    c.speed.set_base(value);
                    true
                }
                _ => false,
            },
            Command::AddColumn => {
                self.add_column();
                true
            }
            Command::RemoveColumn { column } => self.remove_column(column),
            Command::AddLayer => self.add_layer_to_active_deck().is_some(),
            Command::RemoveLayer { layer } => self.remove_layer(layer),
            Command::RemoveClip { layer, column } => self.remove_clip(layer, column),
            Command::SetActiveDeck(deck) => self.set_active_deck(deck),
            // Un geste explicite reprend la main sur le suivi (§7.1).
            Command::SetBpm(bpm) => {
                if bpm.is_finite() {
                    self.tempo.tempo = Param::fixed(bpm);
                    self.tempo_follow = false;
                }
                bpm.is_finite()
            }
            Command::TapTempo => {
                self.tap(t);
                self.tempo_follow = false;
                true
            }
            Command::AddNode { scope, kind } => self.add_node(scope, kind).is_some(),
            Command::RemoveNode { scope, node } => self.remove_node(scope, node),
            Command::Connect { target, source } => self.connect(target, source),
            Command::Disconnect { target } => self.disconnect(target),
            Command::SetTargetBase { target, value } => self.set_target_base(target, value),
            Command::SetTargetDepth { target, value } => self.set_target_depth(target, value),
            Command::SetNodeValue {
                scope,
                node,
                field,
                value,
            } => self.set_node_value(scope, node, field, value),
            Command::AddEffect { clip, kind } => self.add_effect(clip, kind),
            Command::RemoveEffect { clip, index } => self.remove_effect(clip, index),
            Command::SetTempoFollow(suivre) => {
                self.tempo_follow = suivre;
                true
            }
            Command::SetAudioModulation {
                clip,
                slot,
                feature,
            } => self
                .clip_mut(clip)
                .is_some_and(|c| c.set_audio_modulation(slot, feature)),
            Command::AddTrig => self.add_default_trig().is_some(),
            Command::AddTrigAt { freq, level } => {
                let bornes = [freq.0, freq.1, level.0, level.1];
                bornes.iter().all(|v| v.is_finite())
                    && self.add_trig(SpectrumZone::new(freq, level), 1.0).is_some()
            }
            Command::RemoveTrig(trig) => self.remove_trig(trig),
            Command::SetTrigParam { trig, slot, value } => self.set_trig_param(trig, slot, value),
            Command::SetTrigTarget { trig, target } => match self.trig_mut(trig) {
                Some(t) => {
                    t.target = target;
                    true
                }
                None => false,
            },
            Command::ResetTrigCount(trig) => self.reset_trig_count(trig),
            Command::SetAnalysisMethod(method) => {
                self.analysis.method = method;
                true
            }
            Command::SetFreqScale(scale) => {
                self.analysis.freq_scale = scale;
                true
            }
            Command::SetAmpScale(scale) => {
                self.analysis.amp_scale = scale;
                true
            }
            Command::SetAnalysisWindow(seconds) => {
                if seconds.is_finite() {
                    self.analysis.window.set_base(seconds);
                }
                seconds.is_finite()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clip::{Clip, ClipSource};
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
        let colonne = c.columns()[0];

        assert!(c.apply(Command::TriggerColumn { column: colonne }, 0.0));
        assert_eq!(c.layer(p0).unwrap().active_clip(), Some(a));
        assert_eq!(c.layer(p1).unwrap().active_clip(), Some(b));
    }

    #[test]
    fn la_vitesse_d_un_clip_se_regle_sans_couper_sa_modulation() {
        let (mut c, _, [a, _], _) = grille();
        let source = c.graph_mut().add(Box::new(frogenx_core::Constant(0.5)));
        c.clip_mut(a).unwrap().speed = Param::modulated(1.0, 0.5, source);

        let regle = Command::SetClipSpeed {
            clip: a,
            value: 2.0,
        };
        assert!(c.apply(regle, 0.0));
        assert_eq!(c.clip(a).unwrap().speed, Param::modulated(2.0, 0.5, source));

        let invalide = Command::SetClipSpeed {
            clip: a,
            value: f32::NAN,
        };
        assert!(!c.apply(invalide, 0.0));
        assert_eq!(c.clip(a).unwrap().speed.base(), 2.0);
    }

    #[test]
    fn l_amplitude_passe_par_une_commande() {
        let (mut c, _, [a, _], p) = grille();
        let slot = ParamSlot::Generator(0);
        let clip = c.clip_mut(a).unwrap();
        let ClipSource::Shape { generator, .. } = &mut clip.source else {
            panic!("un clip de forme");
        };
        *generator.param_mut(0).unwrap() = Param::modulated(0.5, 0.1, p);

        let regle = Command::SetDepth {
            clip: a,
            slot,
            value: 0.3,
        };
        assert!(c.apply(regle, 0.0));
        let ClipSource::Shape { generator, .. } = &c.clip(a).unwrap().source else {
            panic!("un clip de forme");
        };
        assert_eq!(generator.params()[0].value.depth(), Some(0.3));
    }

    #[test]
    fn la_structure_de_la_grille_se_modifie_par_commandes() {
        let (mut c, piste, [a, b], _) = grille();
        let [colonne_a, colonne_b] = [c.columns()[0], c.columns()[1]];

        assert!(c.apply(Command::AddColumn, 0.0));
        assert_eq!(c.columns().len(), 3);
        assert!(c.apply(Command::AddLayer, 0.0));
        let mut pistes = Vec::new();
        c.layers_in_order(&mut pistes);
        assert_eq!(pistes.len(), 2);

        let vide_a = Command::RemoveClip {
            layer: piste,
            column: colonne_a,
        };
        assert!(c.apply(vide_a, 0.0));
        assert!(c.clip(a).is_none());
        assert!(!c.apply(vide_a, 0.0), "la case est déjà vide");

        assert!(c.apply(Command::RemoveColumn { column: colonne_b }, 0.0));
        assert!(c.clip(b).is_none());
        assert!(c.apply(Command::RemoveLayer { layer: pistes[1] }, 0.0));
        assert!(!c.apply(Command::RemoveLayer { layer: pistes[1] }, 0.0));
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
        let avant = c.columns().len();
        let (a, b) = (c.add_column(), c.add_column());
        assert_eq!(c.columns()[avant..], [a, b]);
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
    fn les_commandes_pilotent_un_trig() {
        let (mut c, piste, [a, _], _) = grille();
        assert!(c.apply(Command::AddTrig, 0.0));
        let trig = c.trigs()[0];
        let every = TrigSlot::Every;
        assert!(c.apply(
            Command::SetTrigParam {
                trig,
                slot: every,
                value: 8.0
            },
            0.0
        ));
        assert_eq!(c.trig(trig).unwrap().every(), Param::fixed(8.0));

        let target = TrigTarget::Clip {
            layer: piste,
            clip: a,
        };
        assert!(c.apply(Command::SetTrigTarget { trig, target }, 0.0));
        assert_eq!(c.trig(trig).unwrap().target, target);

        assert!(c.apply(Command::ResetTrigCount(trig), 0.0));
        assert!(c.apply(Command::RemoveTrig(trig), 0.0));
        assert!(!c.apply(Command::RemoveTrig(trig), 0.0), "périmé");
        assert!(!c.apply(Command::ResetTrigCount(trig), 0.0));
        assert!(!c.apply(Command::SetTrigTarget { trig, target }, 0.0));
    }

    #[test]
    fn un_trig_peut_naitre_d_une_zone_tracee() {
        let (mut c, ..) = grille();
        let trace = Command::AddTrigAt {
            freq: (200.0, 900.0),
            level: (0.3, 0.6),
        };
        assert!(c.apply(trace, 0.0));
        let z = *c.trig(c.trigs()[0]).unwrap().zone();
        assert_eq!(z.freq_min.base(), 200.0);
        assert_eq!(z.level_max.base(), 0.6);

        let invalide = Command::AddTrigAt {
            freq: (f32::NAN, 900.0),
            level: (0.3, 0.6),
        };
        assert!(!c.apply(invalide, 0.0));
        assert_eq!(c.trigs().len(), 1);
    }

    #[test]
    fn les_commandes_reglent_l_analyse() {
        let (mut c, ..) = grille();
        assert!(c.apply(Command::SetAnalysisMethod(AnalysisMethod::ConstantQ), 0.0));
        assert!(c.apply(Command::SetFreqScale(FreqScale::Linear), 0.0));
        assert!(c.apply(Command::SetAmpScale(AmpScale::Linear), 0.0));
        assert!(c.apply(Command::SetAnalysisWindow(0.25), 0.0));
        assert!(!c.apply(Command::SetAnalysisWindow(f32::NAN), 0.0));
        let reglage = c.analysis.resolve(c.graph());
        assert_eq!(reglage.method, AnalysisMethod::ConstantQ);
        assert_eq!(reglage.freq_scale, FreqScale::Linear);
        assert_eq!(reglage.amp_scale, AmpScale::Linear);
        assert_eq!(reglage.window, 0.25);
    }

    #[test]
    fn suivre_le_tempo_se_choisit_et_un_geste_reprend_la_main() {
        let (mut c, ..) = grille();
        assert!(!c.tempo_follow);
        assert!(c.apply(Command::SetTempoFollow(true), 0.0));
        assert!(c.tempo_follow);

        c.apply(Command::SetBpm(100.0), 0.0);
        assert!(!c.tempo_follow, "fixer le tempo coupe le suivi");

        c.apply(Command::SetTempoFollow(true), 0.0);
        c.apply(Command::TapTempo, 0.0);
        assert!(!c.tempo_follow, "taper le tempo coupe le suivi");
    }

    #[test]
    fn un_tempo_refuse_ne_coupe_pas_le_suivi() {
        let (mut c, ..) = grille();
        c.apply(Command::SetTempoFollow(true), 0.0);
        assert!(!c.apply(Command::SetBpm(f32::NAN), 0.0));
        assert!(c.tempo_follow);
    }

    #[test]
    fn le_cablage_passe_par_les_commandes_et_compte_comme_une_modification() {
        let (mut c, ..) = grille();
        let scope = GraphScope::Composition;
        assert!(c.apply(
            Command::AddNode {
                scope,
                kind: NodeKind::Lfo
            },
            0.0
        ));
        let source = c.graph().nodes()[0].id;
        let target = ParamTarget::Tempo;
        let connect = Command::Connect { target, source };
        assert!(connect.is_edit());
        assert!(c.apply(connect, 0.0));
        assert!(c.apply(
            Command::SetTargetDepth {
                target,
                value: 30.0
            },
            0.0
        ));
        assert!(c.apply(
            Command::SetTargetBase {
                target,
                value: 100.0
            },
            0.0
        ));
        assert_eq!(c.tempo.tempo.depth(), Some(30.0));
        assert_eq!(c.tempo.tempo.base(), 100.0);
        assert!(c.apply(Command::Disconnect { target }, 0.0));
        let retire = Command::RemoveNode {
            scope,
            node: source,
        };
        assert!(retire.is_edit());
        assert!(c.apply(retire, 0.0));
        assert!(!c.apply(retire, 0.0), "déjà retiré");
    }

    #[test]
    fn regler_l_analyse_est_une_modification_du_fichier() {
        assert!(Command::SetTempoFollow(true).is_edit());
        assert!(Command::SetAnalysisWindow(0.1).is_edit());
        assert!(Command::SetFreqScale(FreqScale::Mel).is_edit());
        let trace = Command::AddTrigAt {
            freq: (20.0, 40.0),
            level: (0.1, 0.2),
        };
        assert!(trace.is_edit());
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
