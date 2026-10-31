//! Une feature audio comme `Signal` — §7.1 de `docs/ARCHITECTURE.md`.
//!
//! Aucune abstraction de plus : l'audio se branche sur un `Param` comme un LFO.
//! Elle lit `ctx.audio`, jamais le triple buffer : tous les graphes voient donc le
//! même instantané, et un rendu offline écrit le sien.

use crate::envelope::coeff;
use frogenx_core::{Ctx, FeatureKind, NodeState, Param, Signal, Spec, SpecError};

/// Le lissage de départ : assez vif pour suivre une attaque, assez lent pour ne
/// pas faire trembler l'image.
const DEFAULT_ATTACK: f32 = 0.01;
const DEFAULT_RELEASE: f32 = 0.15;

/// Une feature lissée. Sort dans `[0, 1]`, comme ce qu'elle lit (§17).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AudioFeature {
    pub feature: FeatureKind,
    /// Temps de montée, en secondes. `0` = instantané.
    pub attack: Param,
    /// Temps de descente, en secondes.
    pub release: Param,
}

impl AudioFeature {
    pub fn new(feature: FeatureKind) -> Self {
        Self {
            feature,
            attack: Param::fixed(DEFAULT_ATTACK),
            release: Param::fixed(DEFAULT_RELEASE),
        }
    }

    pub fn from_spec(spec: &Spec) -> Result<Self, SpecError> {
        Ok(Self {
            feature: FeatureKind::from_word(spec.word("feature")?)
                .ok_or_else(|| spec.field_error("feature"))?,
            attack: spec.param("attack")?,
            release: spec.param("release")?,
        })
    }
}

impl Signal for AudioFeature {
    fn spec(&self) -> Spec {
        Spec::new("AudioFeature")
            .with("feature", self.feature.word())
            .with("attack", self.attack)
            .with("release", self.release)
    }

    // slots[0] = valeur lissée courante
    fn eval(&self, _t: f64, ctx: &Ctx, state: &mut NodeState) -> f32 {
        let cible = self.feature.read(ctx.audio);
        let cible = if cible.is_finite() { cible } else { 0.0 };
        let courant = state.get(0);
        let tau = if cible > courant {
            self.attack.eval(ctx)
        } else {
            self.release.eval(ctx)
        };
        let v = courant + (cible - courant) * coeff(tau, ctx.dt as f32);
        state.set(0, v);
        v
    }

    fn state_size(&self) -> usize {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use frogenx_core::{AudioFeatures, Graph};

    const DT: f64 = 1.0 / 60.0;

    fn graine(rms: f32) -> AudioFeatures {
        AudioFeatures {
            rms,
            ..Default::default()
        }
    }

    #[test]
    fn la_feature_monte_vite_et_redescend_lentement() {
        let mut g = Graph::new();
        let id = g.add(Box::new(AudioFeature::new(FeatureKind::Rms)));
        g.set_audio(graine(1.0));
        g.eval_frame(0.0, DT);
        let apres_attaque = g.value(id);
        assert!(apres_attaque > 0.7, "{apres_attaque}");

        g.set_audio(graine(0.0));
        g.eval_frame(0.0, DT);
        let apres_une_frame = g.value(id);
        assert!(apres_une_frame > apres_attaque * 0.8, "{apres_une_frame}");
        for _ in 0..120 {
            g.eval_frame(0.0, DT);
        }
        assert!(g.value(id) < 0.01);
    }

    #[test]
    fn sans_lissage_la_feature_suit_exactement() {
        let mut g = Graph::new();
        let id = g.add(Box::new(AudioFeature {
            attack: Param::fixed(0.0),
            release: Param::fixed(0.0),
            ..AudioFeature::new(FeatureKind::Onset)
        }));
        let audio = AudioFeatures {
            onset: 0.75,
            ..Default::default()
        };
        g.set_audio(audio);
        g.eval_frame(0.0, DT);
        assert_eq!(g.value(id), 0.75);
    }

    #[test]
    fn une_valeur_non_finie_ne_passe_pas() {
        let mut g = Graph::new();
        let id = g.add(Box::new(AudioFeature::new(FeatureKind::Rms)));
        g.set_audio(graine(f32::NAN));
        g.eval_frame(0.0, DT);
        assert_eq!(g.value(id), 0.0);
        assert!(g.diagnostics().is_clean());
    }

    #[test]
    fn un_lissage_module_lit_sa_source() {
        let mut g = Graph::new();
        let lent = g.add(Box::new(frogenx_core::Constant(100.0)));
        let id = g
            .add_with_deps(
                Box::new(AudioFeature {
                    attack: Param::modulated(0.0, 1.0, lent),
                    ..AudioFeature::new(FeatureKind::Rms)
                }),
                vec![lent],
            )
            .unwrap();
        g.set_audio(graine(1.0));
        g.eval_frame(0.0, DT);
        assert!(g.value(id) < 0.01, "une attaque de 100 s monte à peine");
    }
}
