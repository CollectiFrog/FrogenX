//! `CrossFader` — §3bis.4 de `docs/ARCHITECTURE.md`.
//!
//! Mélange deux *groupes* de layers entiers en une seule commande — le geste
//! de régie classique (main gauche sur A, main droite sur B). Coexiste avec
//! `BlendParams::opacity` par couche (§22) : le fader est une commande de
//! haut niveau qui finit par piloter des `Param` d'opacité existants, pas un
//! nouveau pipeline de composition. `BlendParams` vit dans `layer` (§22),
//! qui n'existe pas encore (étape 5, §12) — `CrossFader` expose donc ses
//! opacités résolues en `Param` nus, que `layer` branchera quand il existera,
//! plutôt que de référencer un type qui n'existe pas.

use frogenx_core::Param;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FadeCurve {
    Linear,
    /// Monte/descend plus lentement aux extrémités — évite le saut perçu
    /// d'un fondu linéaire en début/fin de geste.
    SmoothStep,
}

impl FadeCurve {
    /// Applique la courbe à une phase déjà bornée dans `[0, 1]`.
    fn apply(&self, phase: f32) -> f32 {
        match self {
            FadeCurve::Linear => phase,
            FadeCurve::SmoothStep => phase * phase * (3.0 - 2.0 * phase),
        }
    }
}

pub struct CrossFader {
    /// 0 = groupe A à pleine opacité, 1 = groupe B à pleine opacité.
    pub phase: Param,
    pub curve: FadeCurve,
}

impl CrossFader {
    pub fn new(curve: FadeCurve) -> Self {
        Self {
            phase: Param::fixed(0.0),
            curve,
        }
    }

    /// Résout l'opacité du groupe A pour une phase donnée, déjà saturée dans
    /// `[0, 1]` — saturation chez le consommateur, jamais à la source (§18.4).
    pub fn opacity_a(&self, phase: f32) -> f32 {
        1.0 - self.curve.apply(phase.clamp(0.0, 1.0))
    }

    /// Résout l'opacité du groupe B — toujours `1.0 - opacity_a` pour la même
    /// phase, ce qui garantit qu'aucune frame n'a les deux groupes à 0 % ou
    /// 100 % simultanément par un arrondi divergent.
    pub fn opacity_b(&self, phase: f32) -> f32 {
        self.curve.apply(phase.clamp(0.0, 1.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase_zero_est_tout_au_groupe_a() {
        let cf = CrossFader::new(FadeCurve::Linear);
        assert_eq!(cf.opacity_a(0.0), 1.0);
        assert_eq!(cf.opacity_b(0.0), 0.0);
    }

    #[test]
    fn phase_un_est_tout_au_groupe_b() {
        let cf = CrossFader::new(FadeCurve::Linear);
        assert_eq!(cf.opacity_a(1.0), 0.0);
        assert_eq!(cf.opacity_b(1.0), 1.0);
    }

    #[test]
    fn phase_milieu_partage_a_parts_egales_en_lineaire() {
        let cf = CrossFader::new(FadeCurve::Linear);
        assert!((cf.opacity_a(0.5) - 0.5).abs() < 1e-6);
        assert!((cf.opacity_b(0.5) - 0.5).abs() < 1e-6);
    }

    #[test]
    fn les_deux_opacites_somment_toujours_a_un() {
        // Propriété invariante quelle que soit la courbe, pour éviter un
        // "trou" ou un double 100% par divergence d'arrondi.
        for curve in [FadeCurve::Linear, FadeCurve::SmoothStep] {
            let cf = CrossFader::new(curve);
            for i in 0..=10 {
                let phase = i as f32 / 10.0;
                let sum = cf.opacity_a(phase) + cf.opacity_b(phase);
                assert!(
                    (sum - 1.0).abs() < 1e-6,
                    "curve={curve:?} phase={phase} sum={sum}"
                );
            }
        }
    }

    #[test]
    fn phase_hors_bornes_est_saturee_sans_paniquer() {
        // §18.4 : saturation chez le consommateur, jamais dans le Signal.
        let cf = CrossFader::new(FadeCurve::Linear);
        assert_eq!(cf.opacity_a(-5.0), 1.0);
        assert_eq!(cf.opacity_a(5.0), 0.0);
        assert_eq!(cf.opacity_b(-5.0), 0.0);
        assert_eq!(cf.opacity_b(5.0), 1.0);
    }

    #[test]
    fn smoothstep_est_plus_lent_aux_extremites_que_lineaire() {
        let linear = CrossFader::new(FadeCurve::Linear);
        let smooth = CrossFader::new(FadeCurve::SmoothStep);
        // À phase=0.1, smoothstep doit avoir moins progressé que linéaire.
        assert!(smooth.opacity_b(0.1) < linear.opacity_b(0.1));
    }
}
