//! La hiérarchie de la grille — §3bis.1 de `docs/ARCHITECTURE.md`.
//!
//! ```text
//! Composition
//!  ├─ Deck[]            un jeu complet de layers, interchangeable à chaud
//!  │   └─ LayerGroup[]  pilote plusieurs layers comme un seul objet
//!  │       └─ Layer[]   piste verticale : vidéo + audio, blend, autopilot
//!  │           └─ cases[]  une par colonne, vide ou portant un Clip
//!  └─ Column[]          déclenche la même tranche sur tous les layers à la fois
//! ```

use crate::clip::Clip;
use crate::ids::{Arena, ClipId, ColumnId, DeckId, LayerGroupId, LayerId, TrigId};
use crate::transport::{sanitize_bpm, TempoController};
use crate::trig::Trig;
use frogenx_core::{Graph, Param, DEFAULT_BPM};
use frogenx_shape::BlendParams;

/// Instants de tap conservés : au-delà, on suit le tempo qui vient de changer.
const TAP_MEMORY: usize = 8;
/// Après ce silence, en secondes, une nouvelle série de taps commence.
const TAP_TIMEOUT: f64 = 2.0;

/// Un jeu complet de layers, interchangeable à chaud (§3bis.1) — un VJ
/// change de répertoire entier de visuels sans interrompre la diffusion.
pub struct Deck {
    pub name: String,
    layer_groups: Vec<LayerGroupId>,
}

pub struct LayerGroup {
    layers: Vec<LayerId>,
}

/// Piste verticale : une case par colonne, un clip à la fois actif.
pub struct Layer {
    pub name: String,
    /// Ses `Param` se résolvent dans le graphe de composition : ils survivent
    /// aux changements de clip (§3bis.1).
    pub blend: BlendParams,
    /// Une case par colonne, à l'index de la colonne dans la grille. Plus
    /// courte que la grille si ses dernières cases n'ont jamais servi.
    cells: Vec<Option<ClipId>>,
    active: Option<ClipId>,
}

impl Layer {
    fn new(name: String) -> Self {
        Self {
            name,
            blend: BlendParams::default(),
            cells: Vec::new(),
            active: None,
        }
    }

    pub fn active_clip(&self) -> Option<ClipId> {
        self.active
    }

    /// Le clip de la case `column`, s'il y en a un.
    pub fn cell(&self, column: usize) -> Option<ClipId> {
        self.cells.get(column).copied().flatten()
    }

    fn holds(&self, clip: ClipId) -> bool {
        self.cells.contains(&Some(clip))
    }

    /// Écrit une case, en allongeant la piste si besoin. Rend l'ancien clip.
    fn set_cell(&mut self, column: usize, clip: Option<ClipId>) -> Option<ClipId> {
        if self.cells.len() <= column {
            self.cells.resize(column + 1, None);
        }
        std::mem::replace(&mut self.cells[column], clip)
    }

    /// Le clip retiré ne joue plus.
    fn forget(&mut self, clip: ClipId) {
        if self.active == Some(clip) {
            self.active = None;
        }
    }
}

/// Déclenche la même tranche sur tous les layers à la fois (§3bis.1) — un
/// déclenchement par layer ne peut pas exprimer un instant dramaturgique
/// partagé sans dupliquer la logique de synchronisation sur chaque layer.
///
/// Son contenu vit dans les cases des pistes : une colonne ne vaut que par sa
/// position dans la grille.
pub struct Column;

/// Racine de la grille — arena par niveau (§3bis.1, raison détaillée dans
/// `ids.rs`), pas une arena unique pour toute la hiérarchie.
pub struct Composition {
    decks: Arena<Deck>,
    layer_groups: Arena<LayerGroup>,
    layers: Arena<Layer>,
    columns: Arena<Column>,
    clips: Arena<Clip>,
    /// L'ordre des colonnes dans la grille.
    column_order: Vec<ColumnId>,
    /// Module ce qui appartient à la régie, pas à un clip (§3).
    pub(crate) graph: Graph,
    pub(crate) trig_arena: Arena<Trig>,
    /// L'ordre des trigs, celui de leurs commandes (§3bis.6).
    pub(crate) trig_order: Vec<TrigId>,
    pub tempo: TempoController,
    taps: Vec<f64>,
    active_deck: Option<DeckId>,
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
            column_order: Vec::new(),
            graph: Graph::new(),
            trig_arena: Arena::new(),
            trig_order: Vec::new(),
            tempo: TempoController::new(DEFAULT_BPM),
            taps: Vec::new(),
            active_deck: None,
        }
    }

    /// Le graphe de composition (§3).
    pub fn graph(&self) -> &Graph {
        &self.graph
    }

    pub fn graph_mut(&mut self) -> &mut Graph {
        &mut self.graph
    }

    /// Le tempo de la frame, à lire **après** `eval_frame` du graphe de
    /// composition. Saturé ici, chez le consommateur (§18.4).
    pub fn bpm(&self) -> f32 {
        sanitize_bpm(self.tempo.tempo.get(&self.graph))
    }

    /// Un tap à l'instant `t` : deux taps suffisent à fixer le tempo.
    pub fn tap(&mut self, t: f64) {
        if self.taps.last().is_some_and(|last| t - last > TAP_TIMEOUT) {
            self.taps.clear();
        }
        self.taps.push(t);
        if self.taps.len() > TAP_MEMORY {
            self.taps.remove(0);
        }
        if let Some(bpm) = TempoController::bpm_from_taps(&self.taps) {
            self.tempo.tempo = Param::fixed(bpm);
        }
    }

    /// Le premier deck ajouté devient le deck actif.
    pub fn add_deck(&mut self, name: impl Into<String>) -> DeckId {
        let (i, g) = self.decks.insert(Deck {
            name: name.into(),
            layer_groups: Vec::new(),
        });
        let id = DeckId::new(i, g);
        self.active_deck.get_or_insert(id);
        id
    }

    pub fn active_deck(&self) -> Option<DeckId> {
        self.active_deck
    }

    pub fn set_active_deck(&mut self, id: DeckId) -> bool {
        let existe = self.deck(id).is_some();
        if existe {
            self.active_deck = Some(id);
        }
        existe
    }

    /// Les pistes du deck actif, de bas en haut : l'ordre de composition.
    /// Écrit dans `out` pour ne pas allouer à chaque frame.
    pub fn layers_in_order(&self, out: &mut Vec<LayerId>) {
        out.clear();
        let Some(deck) = self.active_deck.and_then(|id| self.deck(id)) else {
            return;
        };
        for id in &deck.layer_groups {
            if let Some(groupe) = self.layer_groups.get(id.index(), id.generation()) {
                out.extend_from_slice(&groupe.layers);
            }
        }
    }

    pub fn deck(&self, id: DeckId) -> Option<&Deck> {
        self.decks.get(id.index(), id.generation())
    }

    pub fn remove_deck(&mut self, id: DeckId) -> bool {
        if self.active_deck == Some(id) {
            self.active_deck = None;
        }
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
        let numero = self
            .layer_groups
            .get(group.index(), group.generation())?
            .layers
            .len()
            + 1;
        let (i, g) = self.layers.insert(Layer::new(format!("Piste {numero}")));
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

    pub fn layer_mut(&mut self, id: LayerId) -> Option<&mut Layer> {
        self.layers.get_mut(id.index(), id.generation())
    }

    /// Ajoute une piste à la fin du dernier groupe du deck actif.
    pub fn add_layer_to_active_deck(&mut self) -> Option<LayerId> {
        let deck = self.active_deck.and_then(|id| self.deck(id))?;
        let groupe = *deck.layer_groups.last()?;
        self.add_layer(groupe)
    }

    /// Retire une piste et libère ses clips.
    pub fn remove_layer(&mut self, id: LayerId) -> bool {
        let Some(piste) = self.layers.remove(id.index(), id.generation()) else {
            return false;
        };
        for groupe in self.layer_groups.iter_mut() {
            groupe.layers.retain(|l| *l != id);
        }
        for clip in piste.cells.into_iter().flatten() {
            self.clips.remove(clip.index(), clip.generation());
        }
        true
    }

    /// Ajoute un `Clip` à `layer` sans le rendre actif, dans sa première case
    /// libre, en créant une colonne s'il n'en reste aucune. Pour construire
    /// une grille par code ; l'interface passe par [`Self::place_clip`].
    pub fn add_clip(&mut self, layer: LayerId, clip: Clip) -> Option<ClipId> {
        let piste = self.layer(layer)?;
        let libre = (0..self.column_order.len()).find(|i| piste.cell(*i).is_none());
        let index = libre.unwrap_or(self.column_order.len());
        if libre.is_none() {
            self.add_column();
        }
        let (i, g) = self.clips.insert(clip);
        let id = ClipId::new(i, g);
        self.layer_mut(layer)?.set_cell(index, Some(id));
        Some(id)
    }

    /// Pose `clip` dans la case (`layer`, `column`). Si la case jouait, le
    /// nouveau clip prend sa place et repart de zéro : l'image ne s'éteint pas
    /// sous un geste d'édition (§3bis.1).
    pub fn place_clip(&mut self, layer: LayerId, column: ColumnId, clip: Clip) -> Option<ClipId> {
        let index = self.column_index(column)?;
        self.layer(layer)?;
        let (i, g) = self.clips.insert(clip);
        let id = ClipId::new(i, g);
        let piste = self.layer_mut(layer)?;
        let ancien = piste.set_cell(index, Some(id));
        if let Some(ancien) = ancien {
            let jouait = piste.active == Some(ancien);
            self.clips.remove(ancien.index(), ancien.generation());
            if jouait {
                self.trigger(layer, id);
            }
        }
        Some(id)
    }

    /// Vide la case (`layer`, `column`) et libère son clip. Refusé si la case
    /// est déjà vide.
    pub fn remove_clip(&mut self, layer: LayerId, column: ColumnId) -> bool {
        let Some(index) = self.column_index(column) else {
            return false;
        };
        let Some(piste) = self.layer_mut(layer) else {
            return false;
        };
        let Some(clip) = piste.cell(index) else {
            return false;
        };
        piste.set_cell(index, None);
        piste.forget(clip);
        self.clips.remove(clip.index(), clip.generation());
        true
    }

    pub fn clip(&self, id: ClipId) -> Option<&Clip> {
        self.clips.get(id.index(), id.generation())
    }

    pub fn clip_mut(&mut self, id: ClipId) -> Option<&mut Clip> {
        self.clips.get_mut(id.index(), id.generation())
    }

    /// Rend `clip` actif sur `layer` et le fait repartir de zéro, même s'il
    /// l'était déjà — le déclenchement d'une cellule de la grille (§3bis.1,
    /// §3bis.2). Refusé si `clip` n'appartient pas à `layer`.
    pub fn trigger(&mut self, layer: LayerId, clip: ClipId) -> bool {
        let Some(l) = self.layers.get_mut(layer.index(), layer.generation()) else {
            return false;
        };
        if !l.holds(clip) {
            return false;
        }
        l.active = Some(clip);
        if let Some(c) = self.clip_mut(clip) {
            c.graph_mut().reset_state();
        }
        true
    }

    /// Vide `layer` : elle ne contribue plus à l'image.
    pub fn clear_layer(&mut self, layer: LayerId) -> bool {
        let Some(l) = self.layer_mut(layer) else {
            return false;
        };
        l.active = None;
        true
    }

    pub fn add_column(&mut self) -> ColumnId {
        let (i, g) = self.columns.insert(Column);
        let id = ColumnId::new(i, g);
        self.column_order.push(id);
        id
    }

    /// Les colonnes de la grille, dans leur ordre d'ajout.
    pub fn columns(&self) -> &[ColumnId] {
        &self.column_order
    }

    /// La position d'une colonne dans la grille.
    pub fn column_index(&self, id: ColumnId) -> Option<usize> {
        self.column_order.iter().position(|c| *c == id)
    }

    /// Retire une colonne : les cases à sa droite glissent d'un cran sur
    /// toutes les pistes, de tous les decks, et ses clips sont libérés.
    pub fn remove_column(&mut self, id: ColumnId) -> bool {
        let Some(index) = self.column_index(id) else {
            return false;
        };
        self.columns.remove(id.index(), id.generation());
        self.column_order.remove(index);
        let mut retires = Vec::new();
        for piste in self.layers.iter_mut() {
            if index < piste.cells.len() {
                if let Some(clip) = piste.cells.remove(index) {
                    piste.forget(clip);
                    retires.push(clip);
                }
            }
        }
        for clip in retires {
            self.clips.remove(clip.index(), clip.generation());
        }
        true
    }

    /// Déclenche une `Column` : la case correspondante de chaque layer de
    /// `layers`. C'est le test de la décision §3bis.1 — une colonne synchronise
    /// plusieurs layers en un seul appel, sans dupliquer la logique de
    /// synchronisation.
    ///
    /// Une case vide laisse sa piste inchangée — jamais de panique (§18.1).
    pub fn trigger_column(&mut self, column: ColumnId, layers: &[LayerId]) -> bool {
        let Some(index) = self.column_index(column) else {
            return false;
        };
        for layer in layers {
            if let Some(clip) = self.layer(*layer).and_then(|l| l.cell(index)) {
                self.trigger(*layer, clip);
            }
        }
        true
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

        let column = comp.columns()[0];
        assert!(comp.trigger_column(column, &[layer_a, layer_b]));
        assert_eq!(comp.layer(layer_a).unwrap().active_clip(), Some(clip_a));
        assert_eq!(comp.layer(layer_b).unwrap().active_clip(), Some(clip_b));
    }

    #[test]
    fn une_case_vide_laisse_sa_piste_inchangee() {
        let mut comp = Composition::new();
        let deck = comp.add_deck("D");
        let group = comp.add_layer_group(deck).unwrap();
        let layer_a = comp.add_layer(group).unwrap();
        let layer_b = comp.add_layer(group).unwrap();
        let clip_a = comp.add_clip(layer_a, dummy_clip()).unwrap();

        let column = comp.columns()[0];
        assert!(comp.trigger_column(column, &[layer_a, layer_b]));
        assert_eq!(comp.layer(layer_a).unwrap().active_clip(), Some(clip_a));
        assert_eq!(
            comp.layer(layer_b).unwrap().active_clip(),
            None,
            "layer_b n'était pas dans la colonne, il ne doit pas changer"
        );
    }

    /// Une grille de deux pistes et trois colonnes, sans clip.
    fn grille() -> (Composition, [LayerId; 2], [ColumnId; 3]) {
        let mut comp = Composition::new();
        let deck = comp.add_deck("D");
        let group = comp.add_layer_group(deck).unwrap();
        let pistes = [
            comp.add_layer(group).unwrap(),
            comp.add_layer(group).unwrap(),
        ];
        let colonnes = [comp.add_column(), comp.add_column(), comp.add_column()];
        (comp, pistes, colonnes)
    }

    #[test]
    fn un_clip_pose_reste_dans_sa_colonne_meme_si_les_precedentes_sont_vides() {
        // Le défaut que corrige le modèle à cases (§3bis.1).
        let (mut comp, [a, b], [_, _, c3]) = grille();
        let clip = comp.place_clip(a, c3, dummy_clip()).unwrap();
        assert_eq!(comp.layer(a).unwrap().cell(2), Some(clip));
        assert_eq!(comp.layer(a).unwrap().cell(0), None);
        assert_eq!(comp.layer(a).unwrap().cell(99), None);

        assert!(comp.trigger_column(c3, &[a, b]));
        assert_eq!(comp.layer(a).unwrap().active_clip(), Some(clip));
    }

    #[test]
    fn ajouter_une_piste_ne_decale_aucune_colonne() {
        let (mut comp, [a, _], [c1, ..]) = grille();
        let clip = comp.place_clip(a, c1, dummy_clip()).unwrap();
        let nouvelle = comp.add_layer_to_active_deck().unwrap();

        let mut ordre = Vec::new();
        comp.layers_in_order(&mut ordre);
        assert_eq!(ordre.len(), 3);
        assert_eq!(ordre[2], nouvelle);
        assert!(comp.trigger_column(c1, &ordre));
        assert_eq!(comp.layer(a).unwrap().active_clip(), Some(clip));
    }

    #[test]
    fn remplacer_le_clip_qui_joue_le_fait_prendre_sa_place() {
        let (mut comp, [a, _], [c1, ..]) = grille();
        let ancien = comp.place_clip(a, c1, dummy_clip()).unwrap();
        comp.trigger(a, ancien);

        let nouveau = comp.place_clip(a, c1, dummy_clip()).unwrap();
        assert_eq!(comp.layer(a).unwrap().active_clip(), Some(nouveau));
        assert!(comp.clip(ancien).is_none(), "l'ancien clip est libéré");
    }

    #[test]
    fn remplacer_un_clip_inactif_ne_declenche_rien() {
        let (mut comp, [a, _], [c1, c2, _]) = grille();
        let joue = comp.place_clip(a, c1, dummy_clip()).unwrap();
        comp.place_clip(a, c2, dummy_clip()).unwrap();
        comp.trigger(a, joue);

        comp.place_clip(a, c2, dummy_clip()).unwrap();
        assert_eq!(comp.layer(a).unwrap().active_clip(), Some(joue));
    }

    #[test]
    fn poser_sur_une_colonne_ou_une_piste_perimee_est_refuse() {
        let (mut comp, [a, b], [c1, c2, _]) = grille();
        comp.remove_column(c1);
        comp.remove_layer(b);
        assert_eq!(comp.place_clip(a, c1, dummy_clip()), None);
        assert_eq!(comp.place_clip(b, c2, dummy_clip()), None);
    }

    #[test]
    fn retirer_le_clip_qui_joue_vide_la_piste() {
        let (mut comp, [a, _], [c1, ..]) = grille();
        let clip = comp.place_clip(a, c1, dummy_clip()).unwrap();
        comp.trigger(a, clip);

        assert!(comp.remove_clip(a, c1));
        assert_eq!(comp.layer(a).unwrap().active_clip(), None);
        assert_eq!(comp.layer(a).unwrap().cell(0), None);
        assert!(comp.clip(clip).is_none());
        assert!(!comp.remove_clip(a, c1), "la case est déjà vide");
    }

    #[test]
    fn retirer_une_colonne_fait_glisser_les_cases_a_sa_droite() {
        let (mut comp, [a, _], [c1, c2, c3]) = grille();
        let premier = comp.place_clip(a, c1, dummy_clip()).unwrap();
        let milieu = comp.place_clip(a, c2, dummy_clip()).unwrap();
        let dernier = comp.place_clip(a, c3, dummy_clip()).unwrap();
        comp.trigger(a, milieu);

        assert!(comp.remove_column(c2));
        assert_eq!(comp.columns(), [c1, c3]);
        let piste = comp.layer(a).unwrap();
        assert_eq!(piste.cell(0), Some(premier));
        assert_eq!(piste.cell(1), Some(dernier), "c3 reste sous son en-tête");
        assert_eq!(piste.active_clip(), None, "le clip retiré ne joue plus");
        assert!(comp.clip(milieu).is_none());
        assert!(!comp.remove_column(c2), "identifiant périmé");
    }

    #[test]
    fn retirer_une_colonne_atteint_les_pistes_trop_courtes_sans_paniquer() {
        let (mut comp, [a, b], [_, c2, c3]) = grille();
        comp.place_clip(a, c3, dummy_clip()).unwrap();
        // `b` n'a aucune case : sa liste est plus courte que la grille.
        assert!(comp.remove_column(c2));
        assert_eq!(comp.layer(b).unwrap().cell(0), None);
        assert!(comp.layer(a).unwrap().cell(1).is_some());
    }

    #[test]
    fn retirer_une_piste_libere_ses_clips_et_la_sort_de_l_ordre() {
        let (mut comp, [a, b], [c1, ..]) = grille();
        let clip = comp.place_clip(a, c1, dummy_clip()).unwrap();

        assert!(comp.remove_layer(a));
        let mut ordre = Vec::new();
        comp.layers_in_order(&mut ordre);
        assert_eq!(ordre, vec![b]);
        assert!(comp.clip(clip).is_none());
        assert!(!comp.remove_layer(a), "identifiant périmé");
    }

    #[test]
    fn add_clip_prend_la_premiere_case_libre_et_cree_une_colonne_au_besoin() {
        let (mut comp, [a, _], [c1, c2, c3]) = grille();
        comp.place_clip(a, c1, dummy_clip()).unwrap();
        comp.place_clip(a, c3, dummy_clip()).unwrap();

        let clip = comp.add_clip(a, dummy_clip()).unwrap();
        assert_eq!(comp.layer(a).unwrap().cell(1), Some(clip));
        assert_eq!(comp.column_index(c2), Some(1));

        let debordement = comp.add_clip(a, dummy_clip()).unwrap();
        assert_eq!(comp.columns().len(), 4);
        assert_eq!(comp.layer(a).unwrap().cell(3), Some(debordement));
    }

    #[test]
    fn sans_deck_actif_on_n_ajoute_pas_de_piste() {
        assert_eq!(Composition::new().add_layer_to_active_deck(), None);
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
