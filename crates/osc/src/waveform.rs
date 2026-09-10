//! Les formes d'onde — fonctions pures de la phase.
//!
//! **Convention (§17) :** la phase est en **tours**, `[0, 1)`. Cela évite les
//! `2π` disséminés dans le code ; la conversion en radians se fait au dernier
//! moment, ici seulement, pour la sinusoïde.

use frogenx_core::TAU;

/// Forme d'onde d'un oscillateur.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Waveform {
    #[default]
    Sine,
    Triangle,
    /// Dent de scie montante : `-1` à `+1` sur un tour.
    Saw,
    Square,
}

impl Waveform {
    /// Évalue la forme d'onde à une phase donnée, en tours.
    ///
    /// Sortie dans `[-1, 1]` — bipolaire, conformément au §17. La phase est
    /// repliée dans `[0, 1)` : appeler avec `1.25` ou `-0.75` donne le même
    /// résultat que `0.25`.
    pub fn eval(self, phase: f32) -> f32 {
        let p = wrap01(phase);
        match self {
            Waveform::Sine => (p * TAU).sin(),
            // Montée de -1 à 1 sur la première moitié, descente sur la seconde.
            Waveform::Triangle => 4.0 * (p - 0.5).abs() - 1.0,
            Waveform::Saw => 2.0 * p - 1.0,
            Waveform::Square => {
                if p < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
        }
    }
}

/// Replie une phase dans `[0, 1)`, y compris pour les valeurs négatives.
///
/// `rem_euclid` traite correctement le négatif, contrairement à `%` qui rendrait
/// `-0.25` pour `-0.25 % 1.0`. Un non-fini rend `0.0` : le §18.3 arrête le NaN
/// au cache du graphe, mais autant ne pas le propager jusque-là.
pub fn wrap01(phase: f32) -> f32 {
    if phase.is_finite() {
        phase.rem_euclid(1.0)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Tolérance des comparaisons trigonométriques.
    const EPS: f32 = 1e-6;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < EPS
    }

    #[test]
    fn sine_aux_quatre_quarts() {
        let w = Waveform::Sine;
        assert!(close(w.eval(0.0), 0.0), "got {}", w.eval(0.0));
        assert!(close(w.eval(0.25), 1.0), "got {}", w.eval(0.25));
        assert!(close(w.eval(0.5), 0.0), "got {}", w.eval(0.5));
        assert!(close(w.eval(0.75), -1.0), "got {}", w.eval(0.75));
    }

    #[test]
    fn saw_monte_lineairement() {
        let w = Waveform::Saw;
        assert!(close(w.eval(0.0), -1.0), "got {}", w.eval(0.0));
        assert!(close(w.eval(0.5), 0.0), "got {}", w.eval(0.5));
        // Juste avant le tour complet, on approche +1 sans l'atteindre.
        assert!(w.eval(0.999) > 0.99);
    }

    #[test]
    fn triangle_culmine_aux_extremites() {
        let w = Waveform::Triangle;
        assert!(close(w.eval(0.0), 1.0), "got {}", w.eval(0.0));
        assert!(close(w.eval(0.25), 0.0), "got {}", w.eval(0.25));
        assert!(close(w.eval(0.5), -1.0), "got {}", w.eval(0.5));
        assert!(close(w.eval(0.75), 0.0), "got {}", w.eval(0.75));
    }

    #[test]
    fn square_bascule_a_mi_course() {
        let w = Waveform::Square;
        assert_eq!(w.eval(0.0), 1.0);
        assert_eq!(w.eval(0.499), 1.0);
        assert_eq!(w.eval(0.5), -1.0);
        assert_eq!(w.eval(0.999), -1.0);
    }

    #[test]
    fn toutes_les_ondes_restent_bornees() {
        // §17 : un oscillateur seul reste dans [-1, 1]. C'est la somme de
        // plusieurs qui peut en sortir, et c'est voulu.
        let ondes = [
            Waveform::Sine,
            Waveform::Triangle,
            Waveform::Saw,
            Waveform::Square,
        ];
        for w in ondes {
            for i in 0..1000 {
                let v = w.eval(i as f32 / 1000.0);
                assert!((-1.0..=1.0).contains(&v), "{w:?} à {i}/1000 : got {v}");
            }
        }
    }

    #[test]
    fn la_phase_se_replie() {
        let w = Waveform::Saw;
        assert!(close(w.eval(0.25), w.eval(1.25)), "tour suivant");
        assert!(close(w.eval(0.25), w.eval(-0.75)), "phase négative");
        assert!(close(w.eval(0.25), w.eval(10.25)), "dix tours plus loin");
    }

    #[test]
    fn wrap01_traite_le_negatif_et_le_non_fini() {
        assert!(close(wrap01(-0.25), 0.75), "got {}", wrap01(-0.25));
        assert!(close(wrap01(2.5), 0.5), "got {}", wrap01(2.5));
        assert_eq!(wrap01(f32::NAN), 0.0);
        assert_eq!(wrap01(f32::INFINITY), 0.0);
    }
}
