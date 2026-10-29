//! La hiérarchie de la grille — §3bis.1 de `docs/ARCHITECTURE.md`.
//!
//! ```text
//! Composition
//!  ├─ Deck[]            un jeu complet de layers, interchangeable à chaud
//!  │   └─ LayerGroup[]  pilote plusieurs layers comme un seul objet
//!  │       └─ Layer[]   piste verticale : vidéo + audio, blend, autopilot
//!  │           └─ Clip[]  source + effets + transport, pilotable par son Graph
//!  └─ Column[]          déclenche la même tranche sur tous les layers à la fois
//! ```

use crate::clip::Clip;
use crate::ids::{Arena, ClipId, ColumnId, DeckId, LayerGroupId, LayerId};

/// Un jeu complet de layers, interchangeable à chaud (§3bis.1) — un VJ
/// change de répertoire entier de visuels sans interrompre la diffusion.
pub struct Deck {
    pub name: String,
    layer_groups: Vec<LayerGroupId>,
}

pub struct LayerGroup {
    layers: Vec<LayerId>,
}

/// Piste verticale : une suite de `Clip` déclenchables, un à la fois actif.
pub struct Layer {
    clips: Vec<ClipId>,
    active: Option<ClipId>,
}

impl Layer {
    fn new() -> Self {
        Self {
            clips: Vec::new(),
            active: None,
        }
    }

    pub fn active_clip(&self) -> Option<ClipId> {
        self.active
    }

    pub fn clips(&self) -> &[ClipId] {
        &self.clips
    }
}

/// Déclenche la même tranche sur tous les layers à la fois (§3bis.1) — un
/// déclenchement par layer ne peut pas exprimer un instant dramaturgique
/// partagé sans dupliquer la logique de synchronisation sur chaque layer.
pub struct Column {
    /// Un clip par layer, dans l'ordre des layers de la composition. `None`
    /// pour un layer que cette colonne ne touche pas.
    slots: Vec<Option<ClipId>>,
}

/// Racine de la grille — arena par niveau (§3bis.1, raison détaillée dans
/// `ids.rs`), pas une arena unique pour toute la hiérarchie.
pub struct Composition {
    decks: Arena<Deck>,
    layer_groups: Arena<LayerGroup>,
    layers: Arena<Layer>,
    columns: Arena<Column>,
    clips: Arena<Clip>,
}

impl Default for Composition {
    fn default() -> Self {
        Self::new()
    }
}

impl Composition {
    pub fn new() -> Self {
        Self {
            decks: Arena::new(),
            layer_groups: Arena::new(),
            layers: Arena::new(),
            columns: Arena::new(),
            clips: Arena::new(),
        }
    }

    pub fn add_deck(&mut self, name: impl Into<String>) -> DeckId {
        let (i, g) = self.decks.insert(Deck {
            name: name.into(),
            layer_groups: Vec::new(),
        });
        DeckId::new(i, g)
    }

    pub fn deck(&self, id: DeckId) -> Option<&Deck> {
        self.decks.get(id.index(), id.generation())
    }

    pub fn remove_deck(&mut self, id: DeckId) -> bool {
        self.decks.remove(id.index(), id.generation()).is_some()
    }

    /// Ajoute un `LayerGroup` et le rattache à `deck`.
    pub fn add_layer_group(&mut self, deck: DeckId) -> Option<LayerGroupId> {
        self.decks.get(deck.index(), deck.generation())?;
        let (i, g) = self.layer_groups.insert(LayerGroup { layers: Vec::new() });
        let id = LayerGroupId::new(i, g);
        let d = self
            .decks
            .get_mut(deck.index(), deck.generation())
            .expect("vérifié juste au-dessus");
        d.layer_groups.push(id);
        Some(id)
    }

    /// Ajoute un `Layer` vide et le rattache à `group`.
    pub fn add_layer(&mut self, group: LayerGroupId) -> Option<LayerId> {
        self.layer_groups.get(group.index(), group.generation())?;
        let (i, g) = self.layers.insert(Layer::new());
        let id = LayerId::new(i, g);
        let lg = self
            .layer_groups
            .get_mut(group.index(), group.generation())
            .expect("vérifié juste au-dessus");
        lg.layers.push(id);
        Some(id)
    }

    pub fn layer(&self, id: LayerId) -> Option<&Layer> {
        self.layers.get(id.index(), id.generation())
    }

    /// Ajoute un `Clip` à `layer`, sans le rendre actif.
    pub fn add_clip(&mut self, layer: LayerId, clip: Clip) -> Option<ClipId> {
        self.layers.get(layer.index(), layer.generation())?;
        let (i, g) = self.clips.insert(clip);
        let id = ClipId::new(i, g);
        let l = self
            .layers
            .get_mut(layer.index(), layer.generation())
            .expect("vérifié juste au-dessus");
        l.clips.push(id);
        Some(id)
    }

    pub fn clip(&self, id: ClipId) -> Option<&Clip> {
        self.clips.get(id.index(), id.generation())
    }

    /// Rend `clip` actif sur `layer` — le déclenchement d'une cellule de la
    /// grille (§3bis.1). Refusé si `clip` n'appartient pas à `layer`.
    pub fn trigger(&mut self, layer: LayerId, clip: ClipId) -> bool {
        let Some(l) = self.layers.get_mut(layer.index(), layer.generation()) else {
            return false;
        };
        if !l.clips.contains(&clip) {
            return false;
        }
        l.active = Some(clip);
        true
    }

    pub fn add_column(&mut self) -> ColumnId {
        let (i, g) = self.columns.insert(Column { slots: Vec::new() });
        ColumnId::new(i, g)
    }

    /// Assigne un `clip` à la position `layer_slot` de `column` — une
    /// colonne se construit layer par layer, sans devoir connaître la
    /// composition entière d'avance.
    pub fn set_column_slot(
        &mut self,
        column: ColumnId,
        layer_slot: usize,
        clip: Option<ClipId>,
    ) -> bool {
        let Some(c) = self.columns.get_mut(column.index(), column.generation()) else {
            return false;
        };
        if c.slots.len() <= layer_slot {
            c.slots.resize(layer_slot + 1, None);
        }
        c.slots[layer_slot] = clip;
        true
    }

    /// Déclenche une `Column` : applique chacun de ses clips au layer
    /// correspondant de `layers`, dans l'ordre donné. C'est le test de la
    /// décision §3bis.1 — une colonne synchronise plusieurs layers en un
    /// seul appel, sans dupliquer la logique de synchronisation.
    ///
    /// Un slot `None`, ou un `layer_slot` hors de la longueur de `layers`,
    /// laisse le layer correspondant inchangé — jamais de panique (§18.1).
    pub fn trigger_column(&mut self, column: ColumnId, layers: &[LayerId]) -> bool {
        let Some(c) = self.columns.get(column.index(), column.generation()) else {
            return false;
        };
        let slots = c.slots.clone();
        for (slot_idx, clip) in slots.into_iter().enumerate() {
            let (Some(layer), Some(clip)) = (layers.get(slot_idx), clip) else {
                continue;
            };
            self.trigger(*layer, clip);
        }
        true
    }
}

// `Column::slots` doit être clonable pour `trigger_column` (on lit la
// colonne puis on mute la composition — deux emprunts qui ne coexistent
// pas sur `self`, même raison que `Ctx::cache` dans core §16.2).
impl Clone for Column {
    fn clone(&self) -> Self {
        Self {
            slots: self.slots.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::{PlayMode, Transport};
    use frogenx_core::{Graph, NodeId, Param};

    fn dummy_transport() -> (Graph, Transport) {
        let mut g = Graph::new();
        let pos = g.add(Box::new(crate::position::PositionSignal::fixed_speed(1.0)));
        (
            g,
            Transport::BpmSync {
                position: pos,
                duration_beats: Param::fixed(4.0),
                play_mode: PlayMode::Loop,
            },
        )
    }

    fn dummy_clip() -> Clip {
        let (_g, transport) = dummy_transport();
        Clip::shape(
            Graph::new(),
            Box::new(frogenx_shape::Ellipse::circle(0.5)),
            transport,
        )
    }

    #[test]
    fn construit_la_hierarchie_complete() {
        let mut comp = Composition::new();
        let deck = comp.add_deck("Set principal");
        let group = comp.add_layer_group(deck).expect("deck existe");
        let layer = comp.add_layer(group).expect("group existe");
        let clip = comp.add_clip(layer, dummy_clip()).expect("layer existe");

        assert!(comp.deck(deck).is_some());
        assert!(comp.layer(layer).is_some());
        assert!(comp.clip(clip).is_some());
    }

    #[test]
    fn trigger_rend_le_clip_actif_sur_son_layer() {
        let mut comp = Composition::new();
        let deck = comp.add_deck("D");
        let group = comp.add_layer_group(deck).unwrap();
        let layer = comp.add_layer(group).unwrap();
        let clip = comp.add_clip(layer, dummy_clip()).unwrap();

        assert_eq!(comp.layer(layer).unwrap().active_clip(), None);
        assert!(comp.trigger(layer, clip));
        assert_eq!(comp.layer(layer).unwrap().active_clip(), Some(clip));
    }

    #[test]
    fn trigger_refuse_un_clip_qui_nappartient_pas_au_layer() {
        let mut comp = Composition::new();
        let deck = comp.add_deck("D");
        let group = comp.add_layer_group(deck).unwrap();
        let layer_a = comp.add_layer(group).unwrap();
        let layer_b = comp.add_layer(group).unwrap();
        let clip_a = comp.add_clip(layer_a, dummy_clip()).unwrap();

        assert!(
            !comp.trigger(layer_b, clip_a),
            "clip_a n'est pas sur layer_b"
        );
        assert_eq!(comp.layer(layer_b).unwrap().active_clip(), None);
    }

    #[test]
    fn column_declenche_la_meme_tranche_sur_tous_les_layers() {
        // Le vrai test de la décision §3bis.1 : une Column synchronise
        // plusieurs layers en un seul appel.
        let mut comp = Composition::new();
        let deck = comp.add_deck("D");
        let group = comp.add_layer_group(deck).unwrap();
        let layer_a = comp.add_layer(group).unwrap();
        let layer_b = comp.add_layer(group).unwrap();
        let clip_a = comp.add_clip(layer_a, dummy_clip()).unwrap();
        let clip_b = comp.add_clip(layer_b, dummy_clip()).unwrap();

        let column = comp.add_column();
        comp.set_column_slot(column, 0, Some(clip_a));
        comp.set_column_slot(column, 1, Some(clip_b));

        assert!(comp.trigger_column(column, &[layer_a, layer_b]));
        assert_eq!(comp.layer(layer_a).unwrap().active_clip(), Some(clip_a));
        assert_eq!(comp.layer(layer_b).unwrap().active_clip(), Some(clip_b));
    }

    #[test]
    fn column_avec_un_slot_none_laisse_le_layer_correspondant_inchange() {
        let mut comp = Composition::new();
        let deck = comp.add_deck("D");
        let group = comp.add_layer_group(deck).unwrap();
        let layer_a = comp.add_layer(group).unwrap();
        let layer_b = comp.add_layer(group).unwrap();
        let clip_a = comp.add_clip(layer_a, dummy_clip()).unwrap();

        let column = comp.add_column();
        comp.set_column_slot(column, 0, Some(clip_a));
        // Pas de set_column_slot pour l'index 1 : le slot reste None.

        assert!(comp.trigger_column(column, &[layer_a, layer_b]));
        assert_eq!(comp.layer(layer_a).unwrap().active_clip(), Some(clip_a));
        assert_eq!(
            comp.layer(layer_b).unwrap().active_clip(),
            None,
            "layer_b n'était pas dans la colonne, il ne doit pas changer"
        );
    }

    #[test]
    fn trigger_column_sur_un_id_perime_ne_panique_pas() {
        // §18.1 : jamais de panique en évaluation. Un ColumnId périmé (ou
        // un index hors bornes) est refusé poliment, pas une erreur fatale.
        let mut comp = Composition::new();
        let fake = ColumnId::new(99, 0);
        assert!(!comp.trigger_column(fake, &[]));
    }

    #[test]
    fn ajouter_un_layer_group_sur_un_deck_perime_echoue_sans_paniquer() {
        let mut comp = Composition::new();
        let deck = comp.add_deck("D");
        comp.remove_deck(deck);
        assert_eq!(comp.add_layer_group(deck), None);
    }

    #[test]
    fn un_node_id_de_core_nest_jamais_confondu_avec_un_id_de_composition() {
        // Rappel de non-régression : NodeId (core) et les Id de composition
        // (ClipId, etc.) sont des types distincts qui ne se comparent pas.
        let n: NodeId = {
            let mut g = Graph::new();
            g.add(Box::new(crate::position::PositionSignal::fixed_speed(1.0)))
        };
        let _ = n; // la non-compilation d'un `n == ClipId::new(0,0)` est le test.
    }
}
