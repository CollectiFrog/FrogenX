//! `Param` et `NodeId` — §16.4, §16.5 de `docs/ARCHITECTURE.md`.
//!
//! **La règle la plus importante du projet, et la plus facile à oublier :**
//! aucun champ numérique visible par l'utilisateur ne doit être déclaré `f32`.
//! S'il est réglable, il est `Param`.
//!
//! > Un `f32` glissé aujourd'hui est un câble impossible à brancher demain.
//!
//! Le piège : `f32` marche parfaitement au moment où on l'écrit. Le coût
//! n'apparaît qu'au moment où l'on veut y brancher un LFO — et il faut alors
//! remonter toute la chaîne d'appel.

use crate::graph::Graph;

/// Identifiant d'un nœud du graphe.
///
/// Type distinct, pas un `usize` nu — un `NodeId` et un index de couche ne
/// doivent jamais être confondus par le compilateur (§16.5).
///
/// Porte un **compteur de génération** pour que la suppression d'un nœud puis
/// la création d'un autre ne fasse pas pointer un ancien `NodeId` vers le
/// nouveau nœud. Une référence périmée résout vers `0.0` et incrémente
/// `missing_node` (§18.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeId {
    pub(crate) index: u32,
    pub(crate) gen: u32,
}

impl NodeId {
    pub(crate) fn new(index: u32, gen: u32) -> Self {
        Self { index, gen }
    }

    pub fn index(&self) -> u32 {
        self.index
    }

    pub fn generation(&self) -> u32 {
        self.gen
    }
}

/// Un paramètre réglable : fixe, ou modulé par un signal.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Param {
    Fixed(f32),
    Modulated {
        base: f32,
        depth: f32,
        source: NodeId,
    },
}

impl Param {
    pub fn fixed(v: f32) -> Self {
        Param::Fixed(v)
    }

    pub fn modulated(base: f32, depth: f32, source: NodeId) -> Self {
        Param::Modulated {
            base,
            depth,
            source,
        }
    }

    /// Résout la valeur du paramètre.
    ///
    /// **Lit le cache de la frame, n'évalue rien.** Le graphe est entièrement
    /// évalué avant que la moindre forme ne soit générée (§20.1) : deux formes
    /// lisant le même LFO voient donc exactement la même valeur, et aucun coût
    /// caché ne se déclenche ici.
    pub fn get(&self, g: &Graph) -> f32 {
        match *self {
            Param::Fixed(v) => v,
            Param::Modulated {
                base,
                depth,
                source,
            } => base + depth * g.value(source),
        }
    }

    /// La source de modulation, si ce paramètre est modulé.
    pub fn source(&self) -> Option<NodeId> {
        match *self {
            Param::Fixed(_) => None,
            Param::Modulated { source, .. } => Some(source),
        }
    }
}

impl From<f32> for Param {
    fn from(v: f32) -> Self {
        Param::Fixed(v)
    }
}

impl Default for Param {
    fn default() -> Self {
        Param::Fixed(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::Graph;
    use crate::signal::Constant;

    #[test]
    fn fixed_est_constant_quel_que_soit_le_graphe() {
        let g = Graph::new();
        assert_eq!(Param::fixed(1.5).get(&g), 1.5);
        assert_eq!(Param::from(-3.0).get(&g), -3.0);
        assert_eq!(Param::default().get(&g), 0.0);
    }

    #[test]
    fn modulated_vaut_base_plus_depth_fois_source() {
        let mut g = Graph::new();
        let src = g.add(Box::new(Constant(0.5)));
        g.eval_frame(0.0, 1.0 / 60.0);

        // 2.0 + 4.0 * 0.5 == 4.0
        let p = Param::modulated(2.0, 4.0, src);
        assert!((p.get(&g) - 4.0).abs() < 1e-6, "got {}", p.get(&g));
    }

    #[test]
    fn depth_negatif_inverse_la_modulation() {
        let mut g = Graph::new();
        let src = g.add(Box::new(Constant(1.0)));
        g.eval_frame(0.0, 1.0 / 60.0);

        let p = Param::modulated(1.0, -0.25, src);
        assert!((p.get(&g) - 0.75).abs() < 1e-6, "got {}", p.get(&g));
    }

    #[test]
    fn source_expose_la_dependance() {
        let mut g = Graph::new();
        let src = g.add(Box::new(Constant(0.0)));
        assert_eq!(Param::fixed(1.0).source(), None);
        assert_eq!(Param::modulated(0.0, 1.0, src).source(), Some(src));
    }
}
