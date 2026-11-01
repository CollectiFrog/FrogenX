//! Reprendre ce qui joue dans une grille restaurée — §23.11 de
//! `docs/ARCHITECTURE.md`.
//!
//! Une grille relue d'un fichier repart de zéro. Annuler ne doit pas faire sauter
//! l'image : les pistes et les clips se retrouvent par leur position, puis par
//! leur nom, et ce qui joue passe de l'ancienne grille à la nouvelle.

use crate::hierarchy::Composition;
use crate::ids::{ClipId, LayerId, TrigId};

/// Ce que deviennent les identifiants d'une grille à l'autre.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IdMap {
    pub layers: Vec<(LayerId, LayerId)>,
    pub clips: Vec<(ClipId, ClipId)>,
    pub trigs: Vec<(TrigId, TrigId)>,
}

impl IdMap {
    pub fn clip(&self, ancien: ClipId) -> Option<ClipId> {
        self.clips
            .iter()
            .find(|(a, _)| *a == ancien)
            .map(|(_, n)| *n)
    }

    pub fn layer(&self, ancien: LayerId) -> Option<LayerId> {
        self.layers
            .iter()
            .find(|(a, _)| *a == ancien)
            .map(|(_, n)| *n)
    }

    pub fn trig(&self, ancien: TrigId) -> Option<TrigId> {
        self.trigs
            .iter()
            .find(|(a, _)| *a == ancien)
            .map(|(_, n)| *n)
    }
}

impl Composition {
    /// Toutes les pistes, tous decks confondus, dans l'ordre de la grille.
    pub fn all_layers(&self) -> Vec<LayerId> {
        let mut pistes = Vec::new();
        for deck in self.decks() {
            for groupe in self.deck(deck).map_or(&[][..], |d| d.groups()) {
                pistes.extend_from_slice(self.layer_group(*groupe).map_or(&[][..], |g| g.layers()));
            }
        }
        pistes
    }

    /// Associe les pistes des deux grilles : par position si leur nombre n'a pas
    /// changé, sinon par nom, chaque ancienne piste une seule fois.
    fn pair_layers(&self, ancienne: &Composition) -> Vec<(LayerId, LayerId)> {
        let (nouvelles, anciennes) = (self.all_layers(), ancienne.all_layers());
        if nouvelles.len() == anciennes.len() {
            return anciennes.into_iter().zip(nouvelles).collect();
        }
        let nom_neuf = |id: &LayerId| self.layer(*id).map(|l| l.name.as_str());
        let nom_ancien = |id: &LayerId| ancienne.layer(*id).map(|l| l.name.as_str());
        let mut libres = anciennes;
        let mut paires = Vec::new();
        for neuve in nouvelles {
            if let Some(i) = libres
                .iter()
                .position(|a| nom_ancien(a) == nom_neuf(&neuve))
            {
                paires.push((libres.remove(i), neuve));
            }
        }
        paires
    }

    /// Reprend ce qui joue dans `ancienne`, et rend la correspondance des
    /// identifiants. `self` vient d'être relue : rien n'y joue encore de vrai.
    pub fn adopt_from(&mut self, ancienne: &Composition) -> IdMap {
        let mut map = IdMap {
            layers: self.pair_layers(ancienne),
            ..Default::default()
        };

        // Le compte des trigs et les phases de la régie.
        self.graph_mut().adopt_state(ancienne.graph());
        if self.trigs().len() == ancienne.trigs().len() {
            map.trigs = ancienne
                .trigs()
                .iter()
                .copied()
                .zip(self.trigs().iter().copied())
                .collect();
        }

        let colonnes = self.columns().len();
        let colonnes_anciennes = ancienne.columns().len();
        for (vieille, neuve) in map.layers.clone() {
            let (Some(a), Some(n)) = (ancienne.layer(vieille), self.layer(neuve)) else {
                continue;
            };
            let cases_anciennes: Vec<Option<ClipId>> =
                (0..colonnes_anciennes).map(|i| a.cell(i)).collect();
            let cases: Vec<Option<ClipId>> = (0..colonnes).map(|i| n.cell(i)).collect();
            let nom = |c: &Composition, id: ClipId| c.clip(id).map(|clip| clip.name.clone());

            let mut prises = vec![false; cases_anciennes.len()];
            let mut paires = Vec::new();
            for (i, case) in cases.iter().enumerate() {
                let Some(id) = *case else { continue };
                let cherche = nom(self, id);
                let meme_place = cases_anciennes
                    .get(i)
                    .copied()
                    .flatten()
                    .filter(|v| !prises[i] && nom(ancienne, *v) == cherche)
                    .map(|v| (i, v));
                let ailleurs = || {
                    cases_anciennes.iter().enumerate().find_map(|(j, v)| {
                        let v = (*v)?;
                        (!prises[j] && nom(ancienne, v) == cherche).then_some((j, v))
                    })
                };
                if let Some((j, vieux)) = meme_place.or_else(ailleurs) {
                    prises[j] = true;
                    paires.push((vieux, id));
                }
            }

            // Annuler ne déclenche rien : ce qui jouait continue. Si le clip a
            // disparu, sa case reste la bonne : c'est elle qui jouait.
            let actif = a.active_clip();
            let neuf = actif
                .and_then(|v| paires.iter().find(|(x, _)| *x == v).map(|(_, n)| *n))
                .or_else(|| {
                    let colonne = cases_anciennes
                        .iter()
                        .position(|c| *c == actif && actif.is_some())?;
                    cases.get(colonne).copied().flatten()
                });
            self.set_active_clip(neuve, neuf);

            for (vieux, id) in &paires {
                let source = ancienne.clip(*vieux).map(|c| c.graph());
                if let (Some(source), Some(clip)) = (source, self.clip_mut(*id)) {
                    // Sinon ce clip repart de zéro : c'est lui qu'on vient d'annuler.
                    clip.graph_mut().adopt_state(source);
                }
            }
            map.clips.extend(paires);
        }
        map
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clip::Clip;
    use crate::transport::{PlayMode, Transport};
    use crate::wiring::{GraphScope, NodeKind};
    use frogenx_core::{Graph, Param};
    use frogenx_shape::Ellipse;

    fn clip(nom: &str) -> Clip {
        let mut g = Graph::new();
        let transport = Transport::bpm_sync(&mut g, Param::fixed(4.0), PlayMode::Loop);
        Clip::shape(g, Box::new(Ellipse::circle(0.5)), transport).with_name(nom)
    }

    /// Une piste, deux clips : toute grille « relue » est reconstruite à l'identique.
    fn grille() -> (Composition, LayerId, [ClipId; 2]) {
        let mut c = Composition::new();
        let deck = c.add_deck("test");
        let groupe = c.add_layer_group(deck).unwrap();
        let piste = c.add_layer(groupe).unwrap();
        let (a, b) = (
            c.add_clip(piste, clip("A")).unwrap(),
            c.add_clip(piste, clip("B")).unwrap(),
        );
        (c, piste, [a, b])
    }

    fn avance(c: &mut Composition, id: ClipId, frames: usize) {
        let g = c.clip_mut(id).unwrap().graph_mut();
        for _ in 0..frames {
            g.eval_frame(0.0, 0.1);
        }
    }

    fn etat(c: &Composition, id: ClipId) -> Vec<f32> {
        let g = c.clip(id).unwrap().graph();
        g.nodes()
            .iter()
            .flat_map(|n| g.node_state(n.id).to_vec())
            .collect()
    }

    #[test]
    fn le_clip_actif_continue_avec_sa_phase() {
        let (mut ancienne, _, [_, b]) = grille();
        let piste = ancienne.all_layers()[0];
        ancienne.trigger(piste, b);
        avance(&mut ancienne, b, 7);

        let (mut neuve, np, [_, nb]) = grille();
        let map = neuve.adopt_from(&ancienne);
        assert_eq!(neuve.layer(np).unwrap().active_clip(), Some(nb));
        assert_eq!(etat(&neuve, nb), etat(&ancienne, b));
        assert!(etat(&neuve, nb).iter().any(|v| *v != 0.0), "pas un zéro");
        assert_eq!(map.clip(b), Some(nb));
        assert_eq!(map.layer(piste), Some(np));
    }

    #[test]
    fn un_clip_dont_les_noeuds_ont_change_repart_de_zero_seul() {
        let (mut ancienne, piste, [a, b]) = grille();
        ancienne.trigger(piste, a);
        avance(&mut ancienne, a, 5);
        avance(&mut ancienne, b, 5);
        ancienne
            .add_node(GraphScope::Clip(a), NodeKind::Lfo)
            .unwrap();

        let (mut neuve, _, [na, nb]) = grille();
        neuve.adopt_from(&ancienne);
        assert!(etat(&neuve, na).iter().all(|v| *v == 0.0), "lui repart");
        assert!(etat(&neuve, nb).iter().any(|v| *v != 0.0), "pas l'autre");
    }

    #[test]
    fn annuler_ne_declenche_rien() {
        let (ancienne, _, _) = grille();
        let (mut neuve, np, [_, nb]) = grille();
        neuve.trigger(np, nb);
        neuve.adopt_from(&ancienne);
        assert_eq!(neuve.layer(np).unwrap().active_clip(), None);
    }

    #[test]
    fn un_clip_qui_a_change_de_nom_garde_sa_case_active() {
        // Annuler la pose d'un clip sur la case qui joue.
        let (mut ancienne, piste, [a, _]) = grille();
        let colonne = ancienne.columns()[0];
        let pose = ancienne.place_clip(piste, colonne, clip("Posé")).unwrap();
        ancienne.trigger(piste, pose);
        assert!(ancienne.clip(a).is_none());

        let (mut neuve, np, [na, _]) = grille();
        neuve.adopt_from(&ancienne);
        assert_eq!(neuve.layer(np).unwrap().active_clip(), Some(na));
    }

    #[test]
    fn une_colonne_retiree_se_retrouve_par_les_noms() {
        let (mut ancienne, piste, [a, b]) = grille();
        ancienne.trigger(piste, b);
        avance(&mut ancienne, b, 3);
        let premiere = ancienne.columns()[0];
        ancienne.remove_column(premiere);
        assert!(ancienne.clip(a).is_none());

        let (mut neuve, np, [_, nb]) = grille();
        neuve.adopt_from(&ancienne);
        assert_eq!(neuve.layer(np).unwrap().active_clip(), Some(nb));
        assert_eq!(etat(&neuve, nb), etat(&ancienne, b));
    }

    #[test]
    fn une_piste_retiree_se_retrouve_par_son_nom() {
        let mut ancienne = Composition::new();
        let deck = ancienne.add_deck("test");
        let g = ancienne.add_layer_group(deck).unwrap();
        let (_, deuxieme) = (
            ancienne.add_layer(g).unwrap(),
            ancienne.add_layer(g).unwrap(),
        );
        let c = ancienne.add_clip(deuxieme, clip("A")).unwrap();
        ancienne.trigger(deuxieme, c);

        let mut neuve = Composition::new();
        let deck = neuve.add_deck("test");
        let g = neuve.add_layer_group(deck).unwrap();
        let (a, b, d) = (
            neuve.add_layer(g).unwrap(),
            neuve.add_layer(g).unwrap(),
            neuve.add_layer(g).unwrap(),
        );
        neuve.layer_mut(a).unwrap().name = "Autre".into();
        neuve.layer_mut(d).unwrap().name = "Encore".into();
        let nc = neuve.add_clip(b, clip("A")).unwrap();

        neuve.adopt_from(&ancienne);
        assert_eq!(neuve.layer(b).unwrap().active_clip(), Some(nc));
    }

    #[test]
    fn le_compte_d_un_trig_tient() {
        let (mut ancienne, ..) = grille();
        let t = ancienne.add_default_trig().unwrap();
        let (mut neuve, ..) = grille();
        let nt = neuve.add_default_trig().unwrap();
        let map = neuve.adopt_from(&ancienne);
        assert_eq!(map.trig(t), Some(nt));
    }

    #[test]
    fn un_identifiant_inconnu_ne_correspond_a_rien() {
        let (_, _, [a, _]) = grille();
        assert_eq!(IdMap::default().clip(a), None);
    }
}
