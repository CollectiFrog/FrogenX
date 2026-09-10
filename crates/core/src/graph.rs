//! Le graphe de patch — §3, §16.5, §18, §20.1 de `docs/ARCHITECTURE.md`.
//!
//! DAG en **arena** : les nœuds vivent dans un `Vec` et se référencent par
//! index, jamais par pointeur. Pas de `Rc<RefCell<Node>>` — idiome Rust,
//! sérialisation triviale, pas d'emprunts imbriqués à l'exécution.

use crate::param::NodeId;
use crate::signal::{AudioFeatures, Ctx, Diagnostics, NodeState, Signal};

/// Erreurs du régime **édition** (§18.1) : câblage, suppression, chargement.
///
/// L'évaluation, elle, ne renvoie jamais d'erreur — valeur de repli et
/// compteur `Diagnostics`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GraphError {
    /// Le câblage demandé créerait un cycle. Le feedback se fait au niveau
    /// des couches (§6), où il a un délai d'une frame bien défini.
    WouldCycle,
    /// Référence vers un nœud supprimé ou une génération périmée.
    StaleNode,
    /// Suppression d'un nœud encore référencé, avec la liste des dépendants.
    StillReferenced(Vec<NodeId>),
}

impl std::fmt::Display for GraphError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            GraphError::WouldCycle => {
                write!(f, "câblage refusé : créerait un cycle (le feedback se fait au niveau des couches)")
            }
            GraphError::StaleNode => write!(f, "référence vers un nœud supprimé"),
            GraphError::StillReferenced(deps) => {
                write!(f, "nœud encore référencé par {} nœud(s)", deps.len())
            }
        }
    }
}

impl std::error::Error for GraphError {}

/// Un emplacement de l'arena. `None` = libéré, réutilisable.
struct Slot {
    signal: Option<Box<dyn Signal>>,
    gen: u32,
    /// Dépendances : les nœuds dont celui-ci a besoin. Le tri topologique
    /// et la détection de cycle s'appuient dessus.
    deps: Vec<NodeId>,
    /// Offset et longueur de l'état de ce nœud dans `state`.
    state_off: usize,
    state_len: usize,
}

/// Le graphe de signaux.
pub struct Graph {
    slots: Vec<Slot>,
    /// Cache d'évaluation, un `f32` par emplacement. Invalidé à chaque frame.
    cache: Vec<f32>,
    /// Mémoire persistante de tous les nœuds, en un seul bloc (§16.1) —
    /// sérialisation et reset gratuits.
    state: Vec<f32>,
    /// Ordre topologique, recalculé au câblage et non à chaque frame.
    order: Vec<u32>,
    order_dirty: bool,
    diagnostics: Diagnostics,
    audio: AudioFeatures,
    /// Compteur de frames, exposé aux modules via `Ctx::frame` (§16.2).
    frame: u64,
}

impl Default for Graph {
    fn default() -> Self {
        Self::new()
    }
}

impl Graph {
    pub fn new() -> Self {
        Self {
            slots: Vec::new(),
            cache: Vec::new(),
            state: Vec::new(),
            order: Vec::new(),
            order_dirty: true,
            diagnostics: Diagnostics::default(),
            audio: AudioFeatures::default(),
            frame: 0,
        }
    }

    /// Ajoute un nœud sans dépendances.
    pub fn add(&mut self, signal: Box<dyn Signal>) -> NodeId {
        self.add_with_deps(signal, Vec::new())
            .expect("un nœud sans dépendances ne peut pas créer de cycle")
    }

    /// Ajoute un nœud dépendant d'autres nœuds.
    ///
    /// Les dépendances sont validées : une référence périmée ou un cycle est
    /// refusé ici, en **édition**, plutôt que subi en lecture (§18.1).
    pub fn add_with_deps(
        &mut self,
        signal: Box<dyn Signal>,
        deps: Vec<NodeId>,
    ) -> Result<NodeId, GraphError> {
        for d in &deps {
            if !self.is_live(*d) {
                return Err(GraphError::StaleNode);
            }
        }

        let state_len = signal.state_size();
        let state_off = self.state.len();
        self.state.resize(state_off + state_len, 0.0);

        // Réutilise un emplacement libéré si possible, en incrémentant sa
        // génération : un ancien NodeId ne pointera pas vers le nouveau nœud.
        let index = self.slots.iter().position(|s| s.signal.is_none());
        let id = match index {
            Some(i) => {
                let slot = &mut self.slots[i];
                slot.signal = Some(signal);
                slot.deps = deps;
                slot.state_off = state_off;
                slot.state_len = state_len;
                NodeId::new(i as u32, slot.gen)
            }
            None => {
                self.slots.push(Slot {
                    signal: Some(signal),
                    gen: 0,
                    deps,
                    state_off,
                    state_len,
                });
                self.cache.push(0.0);
                NodeId::new((self.slots.len() - 1) as u32, 0)
            }
        };

        self.order_dirty = true;
        Ok(id)
    }

    /// Déclare que `node` dépend de `dep`. Refusé si cela crée un cycle.
    pub fn connect(&mut self, node: NodeId, dep: NodeId) -> Result<(), GraphError> {
        if !self.is_live(node) || !self.is_live(dep) {
            return Err(GraphError::StaleNode);
        }
        // Un cycle apparaîtrait si `node` est déjà atteignable depuis `dep`.
        if node == dep || self.reaches(dep, node) {
            return Err(GraphError::WouldCycle);
        }
        let deps = &mut self.slots[node.index() as usize].deps;
        if !deps.contains(&dep) {
            deps.push(dep);
            self.order_dirty = true;
        }
        Ok(())
    }

    /// Supprime un nœud. Refusé s'il est encore référencé (§18.2).
    pub fn remove(&mut self, node: NodeId) -> Result<(), GraphError> {
        if !self.is_live(node) {
            return Err(GraphError::StaleNode);
        }
        let dependants: Vec<NodeId> = self
            .slots
            .iter()
            .enumerate()
            .filter(|(i, s)| s.signal.is_some() && *i != node.index() as usize)
            .filter(|(_, s)| s.deps.contains(&node))
            .map(|(i, s)| NodeId::new(i as u32, s.gen))
            .collect();
        if !dependants.is_empty() {
            return Err(GraphError::StillReferenced(dependants));
        }

        let slot = &mut self.slots[node.index() as usize];
        slot.signal = None;
        slot.deps.clear();
        slot.state_len = 0;
        // La génération avance : tout NodeId existant devient périmé.
        slot.gen = slot.gen.wrapping_add(1);
        self.cache[node.index() as usize] = 0.0;
        self.order_dirty = true;
        Ok(())
    }

    /// Vrai si `id` désigne un nœud vivant et de génération courante.
    pub fn is_live(&self, id: NodeId) -> bool {
        self.slots
            .get(id.index() as usize)
            .is_some_and(|s| s.signal.is_some() && s.gen == id.generation())
    }

    pub fn len(&self) -> usize {
        self.slots.iter().filter(|s| s.signal.is_some()).count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Lit la valeur cachée d'un nœud.
    ///
    /// **N'évalue rien** — le graphe est entièrement évalué avant les formes
    /// (§20.1). Une référence périmée rend `0.0` : le comptage se fait pendant
    /// `eval_frame`, seul endroit qui peut muter les diagnostics.
    pub fn value(&self, id: NodeId) -> f32 {
        if self.is_live(id) {
            self.cache[id.index() as usize]
        } else {
            0.0
        }
    }

    pub fn diagnostics(&self) -> Diagnostics {
        self.diagnostics
    }

    pub fn set_audio(&mut self, audio: AudioFeatures) {
        self.audio = audio;
    }

    /// Remet tout l'état persistant à zéro, sans toucher à la topologie (§16.1).
    pub fn reset_state(&mut self) {
        self.state.iter_mut().for_each(|s| *s = 0.0);
        self.cache.iter_mut().for_each(|c| *c = 0.0);
    }

    /// Évalue tout le graphe pour la frame courante.
    ///
    /// Chaque nœud est évalué **une seule fois**, dans l'ordre topologique :
    /// un LFO branché sur cinq destinations coûte une évaluation, pas cinq.
    /// Sans ce cache, un graphe profond explose en coût exponentiel (§3).
    pub fn eval_frame(&mut self, t: f64, dt: f64) {
        self.diagnostics.reset();

        if self.order_dirty {
            self.rebuild_order();
        }

        let audio = self.audio.clone();

        for idx in 0..self.order.len() {
            let i = self.order[idx];
            let slot = &self.slots[i as usize];
            let Some(signal) = &slot.signal else { continue };

            // Une dépendance périmée est comptée ici, une fois par frame.
            for d in &slot.deps {
                let live = self
                    .slots
                    .get(d.index() as usize)
                    .is_some_and(|s| s.signal.is_some() && s.gen == d.generation());
                if !live {
                    self.diagnostics.missing_node += 1;
                }
            }

            // Le cache est prêté en lecture, l'état du nœud courant en écriture.
            // Deux champs distincts de `self` : les emprunts coexistent, ce qui
            // est précisément pourquoi `Ctx` porte le cache et non `&Graph`.
            let (off, len) = (slot.state_off, slot.state_len);
            let (cache, state) = (&self.cache, &mut self.state);
            let ctx = Ctx::with_cache(dt, self.frame, &audio, cache);
            let mut st = NodeState::new(&mut state[off..off + len]);
            let v = signal.eval(t, &ctx, &mut st);

            // §18.3 : un seul point de contrôle du non-fini, à l'écriture dans
            // le cache. Un NaN se propage silencieusement et fait disparaître
            // une forme entière sans explication — le pire défaut : muet.
            self.cache[i as usize] = if v.is_finite() {
                v
            } else {
                self.diagnostics.non_finite += 1;
                0.0
            };
        }

        self.frame = self.frame.wrapping_add(1);
    }

    /// Vrai si `to` est atteignable depuis `from` en suivant les dépendances.
    fn reaches(&self, from: NodeId, to: NodeId) -> bool {
        let mut stack = vec![from];
        let mut seen = vec![false; self.slots.len()];
        while let Some(n) = stack.pop() {
            let i = n.index() as usize;
            if i >= self.slots.len() || seen[i] {
                continue;
            }
            seen[i] = true;
            if n.index() == to.index() {
                return true;
            }
            if self.slots[i].signal.is_some() {
                stack.extend(self.slots[i].deps.iter().copied());
            }
        }
        false
    }

    /// Tri topologique (Kahn). Recalculé au câblage, pas à chaque frame.
    fn rebuild_order(&mut self) {
        let n = self.slots.len();
        let mut indegree = vec![0u32; n];
        // `deps` pointe vers les prérequis ; l'arête va donc dep -> nœud.
        for (i, slot) in self.slots.iter().enumerate() {
            if slot.signal.is_none() {
                continue;
            }
            for d in &slot.deps {
                if (d.index() as usize) < n {
                    indegree[i] += 1;
                    let _ = d;
                }
            }
        }

        let mut queue: Vec<u32> = (0..n as u32)
            .filter(|&i| self.slots[i as usize].signal.is_some() && indegree[i as usize] == 0)
            .collect();

        let mut order = Vec::with_capacity(n);
        while let Some(i) = queue.pop() {
            order.push(i);
            for (j, slot) in self.slots.iter().enumerate() {
                if slot.signal.is_none() {
                    continue;
                }
                if slot.deps.iter().any(|d| d.index() == i) {
                    indegree[j] -= 1;
                    if indegree[j] == 0 {
                        queue.push(j as u32);
                    }
                }
            }
        }

        self.order = order;
        self.order_dirty = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::param::Param;
    use crate::signal::Constant;
    use std::cell::Cell;
    use std::rc::Rc;

    /// Signal qui compte ses évaluations — sert à prouver le cache.
    struct Counter(Rc<Cell<u32>>);
    impl Signal for Counter {
        fn eval(&self, _t: f64, _ctx: &Ctx, _st: &mut NodeState) -> f32 {
            self.0.set(self.0.get() + 1);
            1.0
        }
    }

    /// Signal à état : accumule `dt` dans slots[0].
    struct Accum;
    impl Signal for Accum {
        // slots[0] = temps accumulé
        fn eval(&self, _t: f64, ctx: &Ctx, st: &mut NodeState) -> f32 {
            let v = st.get(0) + ctx.dt as f32;
            st.set(0, v);
            v
        }
        fn state_size(&self) -> usize {
            1
        }
    }

    struct AlwaysNan;
    impl Signal for AlwaysNan {
        fn eval(&self, _t: f64, _ctx: &Ctx, _st: &mut NodeState) -> f32 {
            f32::NAN
        }
    }

    /// Somme des dépendances, pour vérifier l'ordre d'évaluation.
    struct Sum(Vec<NodeId>);
    impl Signal for Sum {
        fn eval(&self, _t: f64, _ctx: &Ctx, _st: &mut NodeState) -> f32 {
            // Ne peut pas lire le graphe ici : la somme est vérifiée via Param
            // dans les tests d'intégration. Ce nœud sert au tri topologique.
            self.0.len() as f32
        }
    }

    #[test]
    fn un_noeud_partage_nest_evalue_quune_fois() {
        // Le test le plus important du graphe (§3) : sans cache, la régression
        // est invisible — le visuel reste correct, seul le coût explose.
        let count = Rc::new(Cell::new(0));
        let mut g = Graph::new();
        let lfo = g.add(Box::new(Counter(count.clone())));

        // Cinq destinations lisent le même nœud.
        let params: Vec<Param> = (0..5).map(|_| Param::modulated(0.0, 1.0, lfo)).collect();

        g.eval_frame(0.0, 1.0 / 60.0);
        for p in &params {
            assert_eq!(p.get(&g), 1.0);
        }

        assert_eq!(
            count.get(),
            1,
            "le nœud partagé doit être évalué une seule fois"
        );
    }

    #[test]
    fn le_cache_est_reevalue_a_chaque_frame() {
        let count = Rc::new(Cell::new(0));
        let mut g = Graph::new();
        g.add(Box::new(Counter(count.clone())));
        g.eval_frame(0.0, 0.016);
        g.eval_frame(0.016, 0.016);
        g.eval_frame(0.032, 0.016);
        assert_eq!(count.get(), 3);
    }

    #[test]
    fn letat_persiste_entre_les_frames() {
        let mut g = Graph::new();
        let n = g.add(Box::new(Accum));
        g.eval_frame(0.0, 0.5);
        assert!((g.value(n) - 0.5).abs() < 1e-6, "got {}", g.value(n));
        g.eval_frame(0.5, 0.5);
        assert!((g.value(n) - 1.0).abs() < 1e-6, "got {}", g.value(n));
    }

    #[test]
    fn reset_state_remet_letat_a_zero() {
        let mut g = Graph::new();
        let n = g.add(Box::new(Accum));
        g.eval_frame(0.0, 0.5);
        g.eval_frame(0.5, 0.5);
        g.reset_state();
        g.eval_frame(1.0, 0.5);
        assert!((g.value(n) - 0.5).abs() < 1e-6, "got {}", g.value(n));
    }

    #[test]
    fn le_nan_est_arrete_au_cache() {
        // §18.3 : un NaN ne doit jamais traverser le graphe.
        let mut g = Graph::new();
        let n = g.add(Box::new(AlwaysNan));
        g.eval_frame(0.0, 0.016);
        assert_eq!(g.value(n), 0.0, "le NaN doit être remplacé par 0.0");
        assert_eq!(g.diagnostics().non_finite, 1);
        assert!(!g.diagnostics().is_clean());
    }

    #[test]
    fn diagnostics_propre_sur_une_frame_nominale() {
        // §18.5 : c'est ainsi que le régime tolérant ne devient pas
        // « silencieusement faux partout ».
        let mut g = Graph::new();
        let a = g.add(Box::new(Constant(0.5)));
        let _b = g.add_with_deps(Box::new(Sum(vec![a])), vec![a]).unwrap();
        g.eval_frame(0.0, 0.016);
        assert!(g.diagnostics().is_clean(), "got {:?}", g.diagnostics());
    }

    #[test]
    fn le_cycle_est_refuse_au_cablage() {
        let mut g = Graph::new();
        let a = g.add(Box::new(Constant(1.0)));
        let b = g.add(Box::new(Constant(2.0)));
        g.connect(b, a).unwrap();
        assert_eq!(g.connect(a, b), Err(GraphError::WouldCycle));
        assert_eq!(
            g.connect(a, a),
            Err(GraphError::WouldCycle),
            "auto-référence"
        );
    }

    #[test]
    fn le_cycle_indirect_est_refuse() {
        let mut g = Graph::new();
        let a = g.add(Box::new(Constant(1.0)));
        let b = g.add(Box::new(Constant(2.0)));
        let c = g.add(Box::new(Constant(3.0)));
        g.connect(b, a).unwrap(); // b dépend de a
        g.connect(c, b).unwrap(); // c dépend de b
                                  // a dépendrait de c → cycle a -> c -> b -> a
        assert_eq!(g.connect(a, c), Err(GraphError::WouldCycle));
    }

    #[test]
    fn suppression_refusee_si_encore_reference() {
        let mut g = Graph::new();
        let a = g.add(Box::new(Constant(1.0)));
        let b = g.add_with_deps(Box::new(Constant(2.0)), vec![a]).unwrap();
        match g.remove(a) {
            Err(GraphError::StillReferenced(deps)) => assert_eq!(deps, vec![b]),
            other => panic!("attendu StillReferenced, got {other:?}"),
        }
    }

    #[test]
    fn un_id_perime_ne_pointe_pas_vers_le_nouveau_noeud() {
        // §16.5 : le compteur de génération protège de la réutilisation d'index.
        let mut g = Graph::new();
        let a = g.add(Box::new(Constant(1.0)));
        g.remove(a).unwrap();
        let b = g.add(Box::new(Constant(2.0))); // réutilise l'emplacement
        assert_eq!(a.index(), b.index(), "l'emplacement doit être réutilisé");
        assert_ne!(a.generation(), b.generation(), "la génération doit avancer");

        g.eval_frame(0.0, 0.016);
        assert!(!g.is_live(a));
        assert_eq!(g.value(a), 0.0, "un id périmé résout vers 0.0");
        assert_eq!(g.value(b), 2.0);
    }

    #[test]
    fn param_sur_id_perime_rend_zero_sans_paniquer() {
        // §18.2 : jamais de panique en évaluation.
        let mut g = Graph::new();
        let a = g.add(Box::new(Constant(5.0)));
        let p = Param::modulated(1.0, 2.0, a);
        g.remove(a).unwrap();
        g.eval_frame(0.0, 0.016);
        // base + depth * 0.0 == base
        assert_eq!(p.get(&g), 1.0);
    }

    #[test]
    fn dependance_vers_un_noeud_perime_est_refusee() {
        let mut g = Graph::new();
        let a = g.add(Box::new(Constant(1.0)));
        g.remove(a).unwrap();
        let r = g.add_with_deps(Box::new(Constant(2.0)), vec![a]);
        assert_eq!(r.err(), Some(GraphError::StaleNode));
    }

    #[test]
    fn ordre_topologique_evalue_les_dependances_dabord() {
        let mut g = Graph::new();
        let a = g.add(Box::new(Constant(1.0)));
        let b = g.add_with_deps(Box::new(Sum(vec![a])), vec![a]).unwrap();
        let c = g.add_with_deps(Box::new(Sum(vec![b])), vec![b]).unwrap();
        g.eval_frame(0.0, 0.016);

        let pos = |id: NodeId| g.order.iter().position(|&i| i == id.index()).unwrap();
        assert!(pos(a) < pos(b), "a doit précéder b");
        assert!(pos(b) < pos(c), "b doit précéder c");
    }

    #[test]
    fn graphe_vide_sevalue_sans_erreur() {
        let mut g = Graph::new();
        g.eval_frame(0.0, 0.016);
        assert!(g.is_empty());
        assert!(g.diagnostics().is_clean());
    }
}
