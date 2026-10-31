//! Câbler les graphes — §23.10 de `docs/ARCHITECTURE.md`.
//!
//! Ce que fait l'éditeur de nœuds, sans fenêtre : tout passe par des commandes,
//! et se teste ici. Un `Signal` reste opaque : un réglage passe par sa `Spec`, et
//! [`Graph::rewire`] recalcule dépendances et cycles au même endroit.

use crate::clip::{ClipSource, ParamSlot};
use crate::hierarchy::Composition;
use crate::ids::{ClipId, LayerId};
use crate::position::BeatRate;
use crate::signals::signal_from_spec;
use crate::transport::Transport;
use frogenx_core::{Constant, FeatureKind, Graph, NodeId, Param, Signal, Spec, SpecValue};
use frogenx_osc::{
    AudioFeature, Envelope, Lfo, Mix, Noise, Oscillator, Product, SampleHold, Sum, Trigger,
    Waveform,
};
use frogenx_shape::EffectKind;

/// Le graphe qu'on édite.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GraphScope {
    Composition,
    Clip(ClipId),
}

/// Ce qu'on peut ajouter à un graphe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    Constant,
    Lfo,
    Oscillator,
    Noise,
    Envelope,
    Trigger,
    SampleHold,
    Sum,
    Product,
    Mix,
    BeatRate,
    AudioFeature,
}

impl NodeKind {
    pub const ALL: [NodeKind; 12] = [
        NodeKind::Lfo,
        NodeKind::Oscillator,
        NodeKind::Noise,
        NodeKind::AudioFeature,
        NodeKind::BeatRate,
        NodeKind::Constant,
        NodeKind::Envelope,
        NodeKind::Trigger,
        NodeKind::SampleHold,
        NodeKind::Sum,
        NodeKind::Product,
        NodeKind::Mix,
    ];

    pub fn label(self) -> &'static str {
        match self {
            NodeKind::Constant => "Constante",
            NodeKind::Lfo => "LFO",
            NodeKind::Oscillator => "Oscillateur",
            NodeKind::Noise => "Bruit",
            NodeKind::Envelope => "Enveloppe",
            NodeKind::Trigger => "Seuil",
            NodeKind::SampleHold => "Échantillonneur",
            NodeKind::Sum => "Somme",
            NodeKind::Product => "Produit",
            NodeKind::Mix => "Mélange",
            NodeKind::BeatRate => "Tempo",
            NodeKind::AudioFeature => "Audio",
        }
    }

    fn build(self) -> Box<dyn Signal> {
        let p = Param::fixed;
        match self {
            NodeKind::Constant => Box::new(Constant(1.0)),
            NodeKind::Lfo => Box::new(Lfo::new(Waveform::Sine, 1.0)),
            NodeKind::Oscillator => Box::new(Oscillator::new(Waveform::Sine, 1.0)),
            NodeKind::Noise => Box::new(Noise::new(1.0, 1)),
            NodeKind::Envelope => Box::new(Envelope::new(p(0.0), 0.01, 0.2)),
            NodeKind::Trigger => Box::new(Trigger::new(p(0.0), 0.5)),
            NodeKind::SampleHold => Box::new(SampleHold {
                input: p(0.0),
                trigger: p(0.0),
            }),
            NodeKind::Sum => Box::new(Sum(vec![p(0.0), p(0.0)])),
            NodeKind::Product => Box::new(Product(vec![p(1.0), p(1.0)])),
            NodeKind::Mix => Box::new(Mix {
                a: p(0.0),
                b: p(0.0),
                mix: p(0.5),
            }),
            NodeKind::BeatRate => Box::new(BeatRate::new(4.0)),
            NodeKind::AudioFeature => Box::new(AudioFeature::new(FeatureKind::Rms)),
        }
    }
}

/// Une destination : tout `Param` qu'on peut brancher.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamTarget {
    ClipParam {
        clip: ClipId,
        slot: ParamSlot,
    },
    /// Le `input`-ième `Param` de la `Spec` du nœud.
    NodeParam {
        scope: GraphScope,
        node: NodeId,
        input: u8,
    },
    LayerOpacity(LayerId),
    ClipSpeed(ClipId),
    Tempo,
    AnalysisWindow,
}

impl ParamTarget {
    /// Le graphe où doit vivre sa source.
    pub fn scope(self) -> GraphScope {
        match self {
            ParamTarget::ClipParam { clip, .. } => GraphScope::Clip(clip),
            ParamTarget::NodeParam { scope, .. } => scope,
            _ => GraphScope::Composition,
        }
    }
}

/// Un réglage de nœud qui n'est pas un `Param`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NodeValue {
    Number(f32),
    Int(u32),
    Bool(bool),
    Word(&'static str),
}

fn set_field(spec: &mut Spec, field: usize, value: NodeValue) -> bool {
    let Some((_, actuel)) = spec.fields.get_mut(field) else {
        return false;
    };
    match (actuel, value) {
        (SpecValue::Number(v), NodeValue::Number(n)) if n.is_finite() => *v = n,
        (SpecValue::Int(v), NodeValue::Int(n)) => *v = n,
        (SpecValue::Bool(v), NodeValue::Bool(b)) => *v = b,
        (SpecValue::Word(v), NodeValue::Word(w)) => *v = w.to_string(),
        _ => return false,
    }
    true
}

/// Reconstruit le nœud depuis sa `Spec`, en gardant son état.
fn apply_spec(graph: &mut Graph, node: NodeId, spec: &Spec) -> bool {
    signal_from_spec(spec).is_ok_and(|signal| graph.rewire(node, signal).is_ok())
}

/// Les `Param` de la `Spec` d'un nœud, regardés comme une liste d'entrées.
fn node_param(graph: &Graph, node: NodeId, input: u8) -> Option<Param> {
    graph
        .node_spec(node)?
        .params_flat()
        .get(input as usize)
        .copied()
}

fn set_node_param(graph: &mut Graph, node: NodeId, input: u8, param: Param) -> bool {
    let Some(mut spec) = graph.node_spec(node) else {
        return false;
    };
    spec.set_flat(input as usize, param) && apply_spec(graph, node, &spec)
}

impl Composition {
    pub fn graph_of(&self, scope: GraphScope) -> Option<&Graph> {
        match scope {
            GraphScope::Composition => Some(self.graph()),
            GraphScope::Clip(id) => self.clip(id).map(|c| c.graph()),
        }
    }

    fn graph_of_mut(&mut self, scope: GraphScope) -> Option<&mut Graph> {
        match scope {
            GraphScope::Composition => Some(self.graph_mut()),
            GraphScope::Clip(id) => self.clip_mut(id).map(|c| c.graph_mut()),
        }
    }

    /// Le `Param` d'une destination.
    pub fn target_param(&self, target: ParamTarget) -> Option<Param> {
        match target {
            ParamTarget::ClipParam { clip, slot } => self
                .clip(clip)?
                .params()
                .iter()
                .flat_map(|g| &g.params)
                .find(|p| p.slot == slot)
                .map(|p| p.desc.value),
            ParamTarget::NodeParam { scope, node, input } => {
                node_param(self.graph_of(scope)?, node, input)
            }
            ParamTarget::LayerOpacity(layer) => Some(self.layer(layer)?.blend.opacity),
            ParamTarget::ClipSpeed(clip) => Some(self.clip(clip)?.speed),
            ParamTarget::Tempo => Some(self.tempo.tempo),
            ParamTarget::AnalysisWindow => Some(self.analysis.window),
        }
    }

    fn set_target_param(&mut self, target: ParamTarget, param: Param) -> bool {
        match target {
            ParamTarget::ClipParam { clip, slot } => self
                .clip_mut(clip)
                .is_some_and(|c| c.replace_param(slot, param)),
            ParamTarget::NodeParam { scope, node, input } => self
                .graph_of_mut(scope)
                .is_some_and(|g| set_node_param(g, node, input, param)),
            ParamTarget::LayerOpacity(layer) => match self.layer_mut(layer) {
                Some(piste) => {
                    piste.blend.opacity = param;
                    true
                }
                None => false,
            },
            ParamTarget::ClipSpeed(clip) => match self.clip_mut(clip) {
                Some(c) => {
                    c.speed = param;
                    true
                }
                None => false,
            },
            ParamTarget::Tempo => {
                self.tempo.tempo = param;
                true
            }
            ParamTarget::AnalysisWindow => {
                self.analysis.window = param;
                true
            }
        }
    }

    /// L'amplitude d'une destination qui n'a pas encore de source.
    fn default_depth(&self, target: ParamTarget) -> f32 {
        match target {
            ParamTarget::ClipParam { clip, slot } => self
                .clip(clip)
                .map(|c| c.params())
                .and_then(|groupes| {
                    groupes
                        .iter()
                        .flat_map(|g| &g.params)
                        .find(|p| p.slot == slot)
                        .map(|p| (p.desc.max - p.desc.min) * 0.5)
                })
                .unwrap_or(1.0),
            ParamTarget::Tempo => 60.0,
            ParamTarget::AnalysisWindow => 0.1,
            _ => 1.0,
        }
    }

    /// Branche `source` sur `target`. Refusé si elle n'est pas dans le bon graphe,
    /// ou si elle ferait un cycle.
    pub fn connect(&mut self, target: ParamTarget, source: NodeId) -> bool {
        let live = self
            .graph_of(target.scope())
            .is_some_and(|g| g.is_live(source));
        let Some(actuel) = self.target_param(target).filter(|_| live) else {
            return false;
        };
        let depth = actuel.depth().unwrap_or_else(|| self.default_depth(target));
        self.set_target_param(target, Param::modulated(actuel.base(), depth, source))
    }

    /// Débranche `target` : elle retombe sur sa base.
    pub fn disconnect(&mut self, target: ParamTarget) -> bool {
        match self.target_param(target) {
            Some(p) => self.set_target_param(target, Param::fixed(p.base())),
            None => false,
        }
    }

    pub fn set_target_base(&mut self, target: ParamTarget, value: f32) -> bool {
        let Some(mut p) = self.target_param(target).filter(|_| value.is_finite()) else {
            return false;
        };
        p.set_base(value);
        self.set_target_param(target, p)
    }

    pub fn set_target_depth(&mut self, target: ParamTarget, value: f32) -> bool {
        let Some(mut p) = self.target_param(target).filter(|_| value.is_finite()) else {
            return false;
        };
        p.set_depth(value) && self.set_target_param(target, p)
    }

    /// Ajoute un nœud. Rend son identifiant.
    pub fn add_node(&mut self, scope: GraphScope, kind: NodeKind) -> Option<NodeId> {
        Some(self.graph_of_mut(scope)?.add(kind.build()))
    }

    /// Le nœud appartient-il à un trig, qui en garde les références ?
    pub fn is_trig_node(&self, scope: GraphScope, node: NodeId) -> bool {
        scope == GraphScope::Composition
            && self
                .trigs()
                .iter()
                .filter_map(|id| self.trig(*id))
                .any(|t| [t.gate(), t.pulse(), t.flash()].contains(&node))
    }

    /// Un nœud que le système tient : il ne se retire pas.
    pub fn is_locked_node(&self, scope: GraphScope, node: NodeId) -> bool {
        if self.is_trig_node(scope, node) {
            return true;
        }
        let GraphScope::Clip(id) = scope else {
            return false;
        };
        self.clip(id).is_some_and(|c| {
            let position = match c.transport {
                Transport::Timeline { position, .. } | Transport::BpmSync { position, .. } => {
                    position
                }
            };
            position == node
                || c.graph()
                    .nodes()
                    .iter()
                    .any(|n| n.id == position && n.deps.contains(&node))
        })
    }

    /// Les destinations de régie, pour débrancher ce qui lit un nœud retiré.
    fn composition_targets(&self) -> Vec<ParamTarget> {
        let mut pistes = Vec::new();
        for deck in self.decks() {
            for groupe in self.deck(deck).map_or(&[][..], |d| d.groups()) {
                pistes.extend_from_slice(self.layer_group(*groupe).map_or(&[][..], |g| g.layers()));
            }
        }
        let mut cibles = vec![ParamTarget::Tempo, ParamTarget::AnalysisWindow];
        for piste in pistes {
            cibles.push(ParamTarget::LayerOpacity(piste));
            if let Some(l) = self.layer(piste) {
                for i in 0..self.columns().len() {
                    if let Some(clip) = l.cell(i) {
                        cibles.push(ParamTarget::ClipSpeed(clip));
                    }
                }
            }
        }
        cibles
    }

    /// Retire un nœud : tout ce qui le lit retombe d'abord sur sa base. Refusé sur
    /// un nœud que le système tient.
    pub fn remove_node(&mut self, scope: GraphScope, node: NodeId) -> bool {
        if self.is_locked_node(scope, node)
            || !self.graph_of(scope).is_some_and(|g| g.is_live(node))
        {
            return false;
        }
        let mut cibles = match scope {
            GraphScope::Composition => self.composition_targets(),
            GraphScope::Clip(clip) => self.clip(clip).map_or(Vec::new(), |c| {
                c.params()
                    .iter()
                    .flat_map(|g| &g.params)
                    .map(|p| ParamTarget::ClipParam { clip, slot: p.slot })
                    .collect()
            }),
        };
        for n in self.graph_of(scope).map_or(Vec::new(), |g| g.nodes()) {
            let entrees = n.spec.params_flat().len();
            cibles.extend((0..entrees).map(|i| ParamTarget::NodeParam {
                scope,
                node: n.id,
                input: i as u8,
            }));
        }
        for cible in cibles {
            let lit = self
                .target_param(cible)
                .is_some_and(|p| p.source() == Some(node));
            if lit {
                self.disconnect(cible);
            }
        }
        self.graph_of_mut(scope)
            .is_some_and(|g| g.remove(node).is_ok())
    }

    /// Change un réglage de nœud qui n'est pas un `Param`.
    pub fn set_node_value(
        &mut self,
        scope: GraphScope,
        node: NodeId,
        field: u8,
        value: NodeValue,
    ) -> bool {
        if self.is_trig_node(scope, node) {
            return false;
        }
        let Some(graph) = self.graph_of_mut(scope) else {
            return false;
        };
        let Some(mut spec) = graph.node_spec(node) else {
            return false;
        };
        set_field(&mut spec, field as usize, value) && apply_spec(graph, node, &spec)
    }

    pub fn add_effect(&mut self, clip: ClipId, kind: EffectKind) -> bool {
        match self.clip_mut(clip) {
            Some(c) if matches!(c.source, ClipSource::Shape { .. }) => {
                c.effects.push(kind.build());
                true
            }
            _ => false,
        }
    }

    pub fn remove_effect(&mut self, clip: ClipId, index: usize) -> bool {
        match self.clip_mut(clip) {
            Some(c) if index < c.effects.len() => {
                c.effects.remove(index);
                true
            }
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clip::Clip;
    use crate::transport::PlayMode;
    use frogenx_shape::Ellipse;

    struct Grille {
        c: Composition,
        piste: LayerId,
        clip: ClipId,
    }

    fn grille() -> Grille {
        let mut c = Composition::new();
        let deck = c.add_deck("test");
        let groupe = c.add_layer_group(deck).unwrap();
        let piste = c.add_layer(groupe).unwrap();
        let mut g = Graph::new();
        let transport = Transport::bpm_sync(&mut g, Param::fixed(4.0), PlayMode::Loop);
        let clip = Clip::shape(g, Box::new(Ellipse::circle(0.5)), transport);
        let clip = c.add_clip(piste, clip).unwrap();
        Grille { c, piste, clip }
    }

    fn rayon(clip: ClipId) -> ParamTarget {
        ParamTarget::ClipParam {
            clip,
            slot: ParamSlot::Generator(0),
        }
    }

    #[test]
    fn un_noeud_se_branche_sur_un_parametre_de_clip() {
        let Grille { mut c, clip, .. } = grille();
        let lfo = c.add_node(GraphScope::Clip(clip), NodeKind::Lfo).unwrap();
        assert!(c.connect(rayon(clip), lfo));
        let p = c.target_param(rayon(clip)).unwrap();
        assert_eq!(p.source(), Some(lfo));
        assert_eq!(p.base(), 0.5);
        assert_eq!(p.depth(), Some(1.0), "une demi-plage");
    }

    #[test]
    fn rebrancher_garde_l_amplitude_reglee() {
        let Grille { mut c, clip, .. } = grille();
        let scope = GraphScope::Clip(clip);
        let (a, b) = (
            c.add_node(scope, NodeKind::Lfo).unwrap(),
            c.add_node(scope, NodeKind::Noise).unwrap(),
        );
        c.connect(rayon(clip), a);
        assert!(c.set_target_depth(rayon(clip), 0.125));
        c.connect(rayon(clip), b);
        let p = c.target_param(rayon(clip)).unwrap();
        assert_eq!((p.source(), p.depth()), (Some(b), Some(0.125)));
    }

    #[test]
    fn debrancher_rend_la_base() {
        let Grille { mut c, clip, .. } = grille();
        let lfo = c.add_node(GraphScope::Clip(clip), NodeKind::Lfo).unwrap();
        c.connect(rayon(clip), lfo);
        assert!(c.disconnect(rayon(clip)));
        assert_eq!(c.target_param(rayon(clip)), Some(Param::fixed(0.5)));
    }

    #[test]
    fn une_source_d_un_autre_graphe_est_refusee() {
        let Grille { mut c, clip, piste } = grille();
        let regie = c.add_node(GraphScope::Composition, NodeKind::Lfo).unwrap();
        let dans_le_clip = c.add_node(GraphScope::Clip(clip), NodeKind::Lfo).unwrap();
        // Même identifiant possible d'un graphe à l'autre : c'est le graphe qui
        // compte, et un index hors de la composition y est refusé.
        assert!(c.connect(ParamTarget::LayerOpacity(piste), regie));
        let etranger = (0..8)
            .map(|_| c.add_node(GraphScope::Clip(clip), NodeKind::Lfo).unwrap())
            .last()
            .unwrap();
        assert!(!c.connect(ParamTarget::LayerOpacity(piste), etranger));
        assert!(c
            .graph_of(GraphScope::Clip(clip))
            .unwrap()
            .is_live(dans_le_clip));
    }

    #[test]
    fn un_noeud_se_branche_sur_l_entree_d_un_autre() {
        let Grille { mut c, clip, .. } = grille();
        let scope = GraphScope::Clip(clip);
        let (rapide, lent) = (
            c.add_node(scope, NodeKind::Lfo).unwrap(),
            c.add_node(scope, NodeKind::Lfo).unwrap(),
        );
        let cible = ParamTarget::NodeParam {
            scope,
            node: lent,
            input: 0,
        };
        assert!(c.connect(cible, rapide));
        let g = c.graph_of(scope).unwrap();
        let vue = g.nodes().into_iter().find(|n| n.id == lent).unwrap();
        assert_eq!(vue.deps, vec![rapide], "les dépendances suivent");
        assert_eq!(c.target_param(cible).unwrap().source(), Some(rapide));
    }

    #[test]
    fn une_boucle_est_refusee_et_ne_change_rien() {
        let Grille { mut c, clip, .. } = grille();
        let scope = GraphScope::Clip(clip);
        let (a, b) = (
            c.add_node(scope, NodeKind::Lfo).unwrap(),
            c.add_node(scope, NodeKind::Lfo).unwrap(),
        );
        let entree = |node| ParamTarget::NodeParam {
            scope,
            node,
            input: 0,
        };
        assert!(c.connect(entree(a), b));
        assert!(!c.connect(entree(b), a), "b lirait a, qui lit b");
        assert!(!c.connect(entree(a), a), "un nœud ne se lit pas lui-même");
        assert_eq!(c.target_param(entree(b)).unwrap().source(), None);
    }

    #[test]
    fn un_flash_de_trig_module_une_opacite() {
        let Grille { mut c, piste, .. } = grille();
        let trig = c.add_default_trig().unwrap();
        let flash = c.trig(trig).unwrap().flash();
        let cible = ParamTarget::LayerOpacity(piste);
        assert!(c.connect(cible, flash));
        assert!(c.is_trig_node(GraphScope::Composition, flash));
        assert_eq!(c.target_param(cible).unwrap().source(), Some(flash));
    }

    #[test]
    fn chaque_destination_de_regie_se_branche() {
        let Grille { mut c, clip, piste } = grille();
        let source = c.add_node(GraphScope::Composition, NodeKind::Lfo).unwrap();
        for cible in [
            ParamTarget::Tempo,
            ParamTarget::AnalysisWindow,
            ParamTarget::LayerOpacity(piste),
            ParamTarget::ClipSpeed(clip),
        ] {
            assert!(c.connect(cible, source), "{cible:?}");
            assert!(c.set_target_base(cible, 0.25), "{cible:?}");
            assert_eq!(c.target_param(cible).unwrap().base(), 0.25);
            assert!(c.disconnect(cible), "{cible:?}");
        }
    }

    #[test]
    fn retirer_un_noeud_debranche_ce_qui_le_lit() {
        let Grille { mut c, clip, .. } = grille();
        let scope = GraphScope::Clip(clip);
        let (a, b) = (
            c.add_node(scope, NodeKind::Lfo).unwrap(),
            c.add_node(scope, NodeKind::Lfo).unwrap(),
        );
        let entree = ParamTarget::NodeParam {
            scope,
            node: b,
            input: 0,
        };
        c.connect(rayon(clip), a);
        c.connect(entree, a);

        assert!(c.remove_node(scope, a));
        assert_eq!(c.target_param(rayon(clip)), Some(Param::fixed(0.5)));
        assert_eq!(c.target_param(entree).unwrap().source(), None);
        assert!(!c.graph_of(scope).unwrap().is_live(a));
    }

    #[test]
    fn retirer_un_noeud_de_regie_rend_les_destinations() {
        let Grille { mut c, piste, .. } = grille();
        let source = c.add_node(GraphScope::Composition, NodeKind::Lfo).unwrap();
        c.connect(ParamTarget::LayerOpacity(piste), source);
        assert!(c.remove_node(GraphScope::Composition, source));
        let p = c.target_param(ParamTarget::LayerOpacity(piste)).unwrap();
        assert_eq!(p.source(), None);
    }

    #[test]
    fn les_noeuds_d_un_trig_et_la_position_d_un_clip_ne_se_retirent_pas() {
        let Grille { mut c, clip, .. } = grille();
        let trig = c.add_default_trig().unwrap();
        let gate = c.trig(trig).unwrap().gate();
        assert!(!c.remove_node(GraphScope::Composition, gate));

        let scope = GraphScope::Clip(clip);
        for n in c.graph_of(scope).unwrap().nodes() {
            assert!(!c.remove_node(scope, n.id), "{}", n.spec.kind);
        }
    }

    #[test]
    fn un_noeud_de_trig_ne_se_regle_pas_depuis_l_editeur() {
        let Grille { mut c, .. } = grille();
        let trig = c.add_default_trig().unwrap();
        let flash = c.trig(trig).unwrap().flash();
        let reglage = NodeValue::Number(1.0);
        assert!(!c.set_node_value(GraphScope::Composition, flash, 1, reglage));
    }

    #[test]
    fn un_reglage_de_noeud_passe_par_sa_spec_et_garde_son_etat() {
        let Grille { mut c, clip, .. } = grille();
        let scope = GraphScope::Clip(clip);
        let lfo = c.add_node(scope, NodeKind::Lfo).unwrap();
        {
            let g = c.clip_mut(clip).unwrap().graph_mut();
            for _ in 0..10 {
                g.eval_frame(0.0, 0.01);
            }
        }
        let avant = c.graph_of(scope).unwrap().node_state(lfo).to_vec();
        assert!(c.set_node_value(scope, lfo, 0, NodeValue::Word("saw")));
        assert!(c.set_node_value(scope, lfo, 2, NodeValue::Bool(true)));
        let g = c.graph_of(scope).unwrap();
        let spec = g.node_spec(lfo).unwrap();
        assert_eq!(spec.word("waveform"), Ok("saw"));
        assert_eq!(spec.bool("unipolar"), Ok(true));
        assert_eq!(g.node_state(lfo), avant, "la phase ne saute pas");
    }

    #[test]
    fn un_mot_inconnu_ou_un_type_faux_est_refuse() {
        let Grille { mut c, clip, .. } = grille();
        let scope = GraphScope::Clip(clip);
        let lfo = c.add_node(scope, NodeKind::Lfo).unwrap();
        assert!(!c.set_node_value(scope, lfo, 0, NodeValue::Word("magie")));
        assert!(!c.set_node_value(scope, lfo, 0, NodeValue::Bool(true)));
        assert!(!c.set_node_value(scope, lfo, 99, NodeValue::Bool(true)));
        let env = c.add_node(scope, NodeKind::Envelope).unwrap();
        assert!(!c.set_node_value(scope, env, 1, NodeValue::Number(f32::NAN)));
    }

    #[test]
    fn chaque_type_de_noeud_s_ajoute_et_se_decrit() {
        let Grille { mut c, clip, .. } = grille();
        let scope = GraphScope::Clip(clip);
        for kind in NodeKind::ALL {
            let id = c.add_node(scope, kind).unwrap();
            let spec = c.graph_of(scope).unwrap().node_spec(id).unwrap();
            assert!(signal_from_spec(&spec).is_ok(), "{}", kind.label());
            assert!(c.remove_node(scope, id), "{}", kind.label());
        }
    }

    #[test]
    fn un_effet_s_ajoute_et_se_retire() {
        let Grille { mut c, clip, .. } = grille();
        assert!(c.add_effect(clip, EffectKind::Deform));
        assert!(c.add_effect(clip, EffectKind::Smooth));
        assert_eq!(c.clip(clip).unwrap().effects.len(), 2);
        assert_eq!(c.clip(clip).unwrap().params().len(), 3);
        assert!(c.remove_effect(clip, 0));
        assert_eq!(c.clip(clip).unwrap().effects[0].kind(), "Lissage");
        assert!(!c.remove_effect(clip, 5));
    }

    #[test]
    fn un_echo_n_a_pas_d_effets() {
        let Grille { mut c, piste, .. } = grille();
        let mut g = Graph::new();
        let transport = Transport::bpm_sync(&mut g, Param::fixed(4.0), PlayMode::Loop);
        let clip = Clip::feedback(g, Default::default(), transport);
        let echo = c.add_clip(piste, clip).unwrap();
        assert!(!c.add_effect(echo, EffectKind::Deform));
    }

    #[test]
    fn une_cible_perimee_ne_panique_pas() {
        let Grille { mut c, clip, .. } = grille();
        let scope = GraphScope::Clip(clip);
        let lfo = c.add_node(scope, NodeKind::Lfo).unwrap();
        let cible = ParamTarget::NodeParam {
            scope,
            node: lfo,
            input: 200,
        };
        assert!(!c.connect(cible, lfo));
        assert!(!c.disconnect(cible));
        assert!(!c.set_target_base(rayon(clip), f32::NAN));
    }
}
