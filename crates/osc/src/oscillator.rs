//! L'oscillateur — la source de modulation de base.
//!
//! Trois entrées modulables : fréquence (FM), amplitude (AM), décalage de
//! phase (PM). C'est ce trio qui donne le chaos harmonique recherché pour
//! l'organique (§5.3).

use crate::waveform::{wrap01, Waveform};
use frogenx_core::{Ctx, NodeState, Param, Signal};

/// Oscillateur à phase accumulée.
///
/// **Pourquoi accumuler la phase plutôt que calculer `sin(2π·f·t)` :** quand la
/// fréquence est modulée, la formule directe produit une discontinuité audible
/// et visible à chaque changement — la phase saute. L'accumulation garde la
/// continuité, qui est précisément ce qu'on veut en FM.
pub struct Oscillator {
    pub waveform: Waveform,
    /// Fréquence en **Hz** (§17). Modulable → FM.
    pub freq: Param,
    /// Amplitude. Modulable → AM.
    pub amp: Param,
    /// Décalage de phase en **tours**. Modulable → PM.
    pub phase_offset: Param,
}

impl Oscillator {
    pub fn new(waveform: Waveform, freq: f32) -> Self {
        Self {
            waveform,
            freq: Param::fixed(freq),
            amp: Param::fixed(1.0),
            phase_offset: Param::fixed(0.0),
        }
    }

    pub fn sine(freq: f32) -> Self {
        Self::new(Waveform::Sine, freq)
    }

    pub fn with_amp(mut self, amp: Param) -> Self {
        self.amp = amp;
        self
    }

    pub fn with_freq(mut self, freq: Param) -> Self {
        self.freq = freq;
        self
    }

    pub fn with_phase_offset(mut self, phase: Param) -> Self {
        self.phase_offset = phase;
        self
    }
}

impl Signal for Oscillator {
    // slots[0] = phase accumulée, en tours, repliée dans [0, 1)
    fn eval(&self, _t: f64, ctx: &Ctx, state: &mut NodeState) -> f32 {
        let freq = self.freq.eval(ctx);
        let amp = self.amp.eval(ctx);
        let offset = self.phase_offset.eval(ctx);

        // Une fréquence non finie viendrait d'une modulation dégénérée : on
        // n'avance pas la phase plutôt que de la corrompre définitivement.
        let advance = if freq.is_finite() {
            freq * ctx.dt as f32
        } else {
            0.0
        };

        let phase = wrap01(state.get(0) + advance);
        state.set(0, phase);

        self.waveform.eval(phase + offset) * amp
    }

    fn state_size(&self) -> usize {
        1
    }
}

/// LFO — un oscillateur lent, bipolaire ou unipolaire.
///
/// Techniquement identique à [`Oscillator`], mais l'usage diffère : un LFO
/// module, il ne se voit pas. Le mode unipolaire (`[0, 1]`) évite d'écrire
/// `base + depth * (x * 0.5 + 0.5)` à chaque câblage vers une grandeur
/// naturellement positive — un rayon, une échelle.
pub struct Lfo {
    pub waveform: Waveform,
    pub freq: Param,
    /// Si vrai, la sortie est repliée dans `[0, 1]` au lieu de `[-1, 1]`.
    pub unipolar: bool,
}

impl Lfo {
    pub fn new(waveform: Waveform, freq: f32) -> Self {
        Self {
            waveform,
            freq: Param::fixed(freq),
            unipolar: false,
        }
    }

    pub fn unipolar(mut self) -> Self {
        self.unipolar = true;
        self
    }
}

impl Signal for Lfo {
    // slots[0] = phase accumulée, en tours
    fn eval(&self, _t: f64, ctx: &Ctx, state: &mut NodeState) -> f32 {
        let freq = self.freq.eval(ctx);
        let advance = if freq.is_finite() {
            freq * ctx.dt as f32
        } else {
            0.0
        };
        let phase = wrap01(state.get(0) + advance);
        state.set(0, phase);

        let v = self.waveform.eval(phase);
        if self.unipolar {
            v * 0.5 + 0.5
        } else {
            v
        }
    }

    fn state_size(&self) -> usize {
        1
    }
}

/// Somme de plusieurs signaux.
///
/// **Ne sature pas** — la sortie peut dépasser `[-1, 1]`, conformément au §17.
/// C'est au consommateur de saturer son domaine (§18.4) ; saturer ici
/// détruirait la FM pour tous les autres consommateurs.
pub struct Sum(pub Vec<Param>);

impl Signal for Sum {
    fn eval(&self, _t: f64, ctx: &Ctx, _state: &mut NodeState) -> f32 {
        self.0.iter().map(|p| p.eval(ctx)).sum()
    }
}

/// Produit de plusieurs signaux — modulation en anneau quand les deux sont
/// bipolaires.
pub struct Product(pub Vec<Param>);

impl Signal for Product {
    fn eval(&self, _t: f64, ctx: &Ctx, _state: &mut NodeState) -> f32 {
        self.0.iter().map(|p| p.eval(ctx)).product()
    }
}

/// Interpolation linéaire entre deux signaux, pilotée par un troisième.
///
/// `mix` est saturé dans `[0, 1]` : c'est un consommateur, il connaît son
/// domaine (§18.4).
pub struct Mix {
    pub a: Param,
    pub b: Param,
    pub mix: Param,
}

impl Signal for Mix {
    fn eval(&self, _t: f64, ctx: &Ctx, _state: &mut NodeState) -> f32 {
        let k = self.mix.eval(ctx).clamp(0.0, 1.0);
        self.a.eval(ctx) * (1.0 - k) + self.b.eval(ctx) * k
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use frogenx_core::{Constant, Graph};

    const EPS: f32 = 1e-5;

    #[test]
    fn un_tour_complet_ramene_a_la_phase_initiale() {
        // 1 Hz pendant 1 s à 60 fps : la phase revient à ~0.
        let mut g = Graph::new();
        let osc = g.add(Box::new(Oscillator::sine(1.0)));
        for i in 0..60 {
            g.eval_frame(i as f64 / 60.0, 1.0 / 60.0);
        }
        assert!(g.value(osc).abs() < 1e-3, "got {}", g.value(osc));
    }

    #[test]
    fn la_sinusoide_atteint_son_maximum_au_quart() {
        // 1 Hz, 15 frames à 60 fps == un quart de tour.
        let mut g = Graph::new();
        let osc = g.add(Box::new(Oscillator::sine(1.0)));
        for i in 0..15 {
            g.eval_frame(i as f64 / 60.0, 1.0 / 60.0);
        }
        assert!((g.value(osc) - 1.0).abs() < EPS, "got {}", g.value(osc));
    }

    #[test]
    fn frequence_nulle_fige_loscillateur() {
        let mut g = Graph::new();
        let osc = g.add(Box::new(Oscillator::sine(0.0)));
        for _ in 0..10 {
            g.eval_frame(0.0, 1.0 / 60.0);
        }
        assert_eq!(g.value(osc), 0.0, "phase 0 → sin(0) == 0");
        assert!(g.diagnostics().is_clean());
    }

    #[test]
    fn frequence_negative_tourne_a_lenvers() {
        let mut g = Graph::new();
        let a = g.add(Box::new(Oscillator::sine(1.0)));
        let b = g.add(Box::new(Oscillator::sine(-1.0)));
        for i in 0..7 {
            g.eval_frame(i as f64 / 60.0, 1.0 / 60.0);
        }
        // sin(-x) == -sin(x) : les deux doivent être opposés.
        assert!(
            (g.value(a) + g.value(b)).abs() < EPS,
            "a={} b={}",
            g.value(a),
            g.value(b)
        );
    }

    #[test]
    fn lamplitude_module_la_sortie() {
        let mut g = Graph::new();
        let demi = g.add(Box::new(Constant(0.5)));
        let osc = g
            .add_with_deps(
                Box::new(Oscillator::sine(1.0).with_amp(Param::modulated(0.0, 1.0, demi))),
                vec![demi],
            )
            .unwrap();
        for i in 0..15 {
            g.eval_frame(i as f64 / 60.0, 1.0 / 60.0);
        }
        // Maximum au quart de tour, réduit de moitié.
        assert!((g.value(osc) - 0.5).abs() < EPS, "got {}", g.value(osc));
    }

    #[test]
    fn le_decalage_de_phase_avance_londe() {
        // Un quart de tour d'offset transforme sin en cos : sortie 1.0 à t=0.
        let mut g = Graph::new();
        let quart = g.add(Box::new(Constant(0.25)));
        let osc = g
            .add_with_deps(
                Box::new(
                    Oscillator::sine(1.0).with_phase_offset(Param::modulated(0.0, 1.0, quart)),
                ),
                vec![quart],
            )
            .unwrap();
        g.eval_frame(0.0, 0.0);
        assert!((g.value(osc) - 1.0).abs() < EPS, "got {}", g.value(osc));
    }

    #[test]
    fn la_phase_est_continue_sous_modulation_de_frequence() {
        // Le cœur du choix « phase accumulée » : en FM, la sortie ne doit pas
        // sauter. On vérifie qu'aucun écart entre frames consécutives n'excède
        // ce qu'un pas de temps peut produire.
        let mut g = Graph::new();
        let lfo = g.add(Box::new(Lfo::new(Waveform::Sine, 3.0)));
        let osc = g
            .add_with_deps(
                Box::new(Oscillator::sine(2.0).with_freq(Param::modulated(2.0, 8.0, lfo))),
                vec![lfo],
            )
            .unwrap();

        let mut prev: Option<f32> = None;
        for i in 0..600 {
            g.eval_frame(i as f64 / 60.0, 1.0 / 60.0);
            let v = g.value(osc);
            if let Some(p) = prev {
                // Fréquence max 10 Hz à 60 fps → au plus ~1/6 de tour par frame.
                assert!(
                    (v - p).abs() < 1.0,
                    "saut de phase à la frame {i} : {p} → {v}"
                );
            }
            prev = Some(v);
        }
        assert!(g.diagnostics().is_clean());
    }

    #[test]
    fn lfo_unipolaire_reste_dans_zero_un() {
        let mut g = Graph::new();
        let lfo = g.add(Box::new(Lfo::new(Waveform::Sine, 5.0).unipolar()));
        for i in 0..120 {
            g.eval_frame(i as f64 / 60.0, 1.0 / 60.0);
            let v = g.value(lfo);
            assert!((0.0..=1.0).contains(&v), "frame {i} : got {v}");
        }
    }

    #[test]
    fn la_somme_ne_sature_pas() {
        // §17/§18.4 : une somme sort de [-1,1], et c'est voulu.
        let mut g = Graph::new();
        let a = g.add(Box::new(Constant(0.8)));
        let b = g.add(Box::new(Constant(0.7)));
        let s = g
            .add_with_deps(
                Box::new(Sum(vec![
                    Param::modulated(0.0, 1.0, a),
                    Param::modulated(0.0, 1.0, b),
                ])),
                vec![a, b],
            )
            .unwrap();
        g.eval_frame(0.0, 0.016);
        assert!((g.value(s) - 1.5).abs() < EPS, "got {}", g.value(s));
    }

    #[test]
    fn le_produit_fait_la_modulation_en_anneau() {
        let mut g = Graph::new();
        let a = g.add(Box::new(Constant(0.5)));
        let b = g.add(Box::new(Constant(-0.4)));
        let p = g
            .add_with_deps(
                Box::new(Product(vec![
                    Param::modulated(0.0, 1.0, a),
                    Param::modulated(0.0, 1.0, b),
                ])),
                vec![a, b],
            )
            .unwrap();
        g.eval_frame(0.0, 0.016);
        assert!((g.value(p) - (-0.2)).abs() < EPS, "got {}", g.value(p));
    }

    #[test]
    fn mix_interpole_et_sature_son_domaine() {
        let mut g = Graph::new();
        let a = g.add(Box::new(Constant(0.0)));
        let b = g.add(Box::new(Constant(10.0)));

        // mix hors domaine : doit être saturé à 1.0, pas extrapoler.
        let m = g
            .add_with_deps(
                Box::new(Mix {
                    a: Param::modulated(0.0, 1.0, a),
                    b: Param::modulated(0.0, 1.0, b),
                    mix: Param::fixed(5.0),
                }),
                vec![a, b],
            )
            .unwrap();
        g.eval_frame(0.0, 0.016);
        assert!(
            (g.value(m) - 10.0).abs() < EPS,
            "saturation: got {}",
            g.value(m)
        );
    }

    #[test]
    fn frequence_non_finie_ne_corrompt_pas_la_phase() {
        // Une modulation dégénérée ne doit pas figer l'oscillateur pour de bon.
        let mut g = Graph::new();
        let osc = g.add(Box::new(Oscillator::sine(f32::NAN)));
        g.eval_frame(0.0, 1.0 / 60.0);
        assert_eq!(g.value(osc), 0.0);
        assert!(g.value(osc).is_finite());
        assert!(g.diagnostics().is_clean(), "la phase est protégée en amont");
    }

    #[test]
    fn deux_oscillateurs_identiques_restent_synchrones() {
        // Le déterminisme du §20.3, vu depuis osc : même config, même sortie.
        let mut g = Graph::new();
        let a = g.add(Box::new(Oscillator::new(Waveform::Triangle, 2.5)));
        let b = g.add(Box::new(Oscillator::new(Waveform::Triangle, 2.5)));
        for i in 0..200 {
            g.eval_frame(i as f64 / 60.0, 1.0 / 60.0);
            assert_eq!(g.value(a), g.value(b), "divergence à la frame {i}");
        }
    }
}
