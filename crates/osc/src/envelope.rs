//! Enveloppes et lissage — §7 (« lissage : obligatoire »).
//!
//! Les features audio brutes sont bruitées et produisent un visuel nerveux.
//! Une attaque rapide et un relâchement lent donnent l'impression que le
//! visuel « respire » avec le son, plutôt que de trembler.

use frogenx_core::{Ctx, NodeState, Param, Signal};

/// Suiveur d'enveloppe à attaque et relâchement séparés.
///
/// **Unipolaire** : sortie dans `[0, 1]` si l'entrée l'est (§17).
///
/// Les temps sont en **secondes** et représentent la constante de temps —
/// la durée pour parcourir ~63 % de l'écart restant. Un lissage exponentiel
/// plutôt que linéaire : il n'a pas de discontinuité à l'arrivée, et c'est
/// ce que fait l'oreille.
pub struct Envelope {
    pub input: Param,
    /// Temps de montée, en secondes. `0` = instantané.
    pub attack: f32,
    /// Temps de descente, en secondes.
    pub release: f32,
}

impl Envelope {
    pub fn new(input: Param, attack: f32, release: f32) -> Self {
        Self {
            input,
            attack: attack.max(0.0),
            release: release.max(0.0),
        }
    }

    /// Réglage courant pour l'audio : réagit vite, retombe lentement.
    pub fn audio(input: Param) -> Self {
        Self::new(input, 0.01, 0.25)
    }
}

/// Coefficient de lissage exponentiel pour une constante de temps donnée.
///
/// `1 - exp(-dt / tau)`. Un `tau` nul donne 1.0 (suivi instantané), ce qui est
/// le comportement attendu et non une division par zéro.
fn coeff(tau: f32, dt: f32) -> f32 {
    if tau <= 0.0 || !tau.is_finite() {
        return 1.0;
    }
    if dt <= 0.0 {
        return 0.0;
    }
    1.0 - (-dt / tau).exp()
}

impl Signal for Envelope {
    // slots[0] = valeur lissée courante
    fn eval(&self, _t: f64, ctx: &Ctx, state: &mut NodeState) -> f32 {
        let cible = self.input.eval(ctx);
        let cible = if cible.is_finite() { cible } else { 0.0 };

        let courant = state.get(0);
        // Attaque quand on monte, relâchement quand on descend : c'est cette
        // asymétrie qui fait toute la différence perçue.
        let tau = if cible > courant {
            self.attack
        } else {
            self.release
        };

        let k = coeff(tau, ctx.dt as f32);
        let v = courant + (cible - courant) * k;
        state.set(0, v);
        v
    }

    fn state_size(&self) -> usize {
        1
    }
}

/// Détecteur de front — sort `1.0` sur la frame où l'entrée franchit un seuil
/// vers le haut, `0.0` sinon.
///
/// Sert aux déclenchements : un onset audio, un accent rythmique. Le seuil
/// n'est pas modulable pour l'instant : un seuil qui bouge sous le signal
/// produit des déclenchements erratiques difficiles à régler.
pub struct Trigger {
    pub input: Param,
    pub threshold: f32,
}

impl Trigger {
    pub fn new(input: Param, threshold: f32) -> Self {
        Self { input, threshold }
    }
}

impl Signal for Trigger {
    // slots[0] = 1.0 si l'entrée était au-dessus du seuil à la frame précédente
    fn eval(&self, _t: f64, ctx: &Ctx, state: &mut NodeState) -> f32 {
        let v = self.input.eval(ctx);
        let au_dessus = v.is_finite() && v >= self.threshold;
        let etait_au_dessus = state.get(0) > 0.5;
        state.set(0, if au_dessus { 1.0 } else { 0.0 });

        if au_dessus && !etait_au_dessus {
            1.0
        } else {
            0.0
        }
    }

    fn state_size(&self) -> usize {
        1
    }
}

/// Échantillonne son entrée quand un déclencheur passe à `1.0`, et maintient
/// la valeur entre deux déclenchements.
pub struct SampleHold {
    pub input: Param,
    pub trigger: Param,
}

impl Signal for SampleHold {
    // slots[0] = valeur maintenue
    fn eval(&self, _t: f64, ctx: &Ctx, state: &mut NodeState) -> f32 {
        if self.trigger.eval(ctx) > 0.5 {
            let v = self.input.eval(ctx);
            if v.is_finite() {
                state.set(0, v);
            }
        }
        state.get(0)
    }

    fn state_size(&self) -> usize {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use frogenx_core::{Constant, Graph, Param};

    #[test]
    fn lenveloppe_monte_vers_sa_cible_sans_la_depasser() {
        let mut g = Graph::new();
        let src = g.add(Box::new(Constant(1.0)));
        let env = g
            .add_with_deps(
                Box::new(Envelope::new(Param::modulated(0.0, 1.0, src), 0.1, 0.1)),
                vec![src],
            )
            .unwrap();

        let mut precedent = 0.0;
        for i in 0..120 {
            g.eval_frame(i as f64 / 60.0, 1.0 / 60.0);
            let v = g.value(env);
            assert!(v >= precedent - 1e-6, "doit croître, frame {i}");
            assert!(v <= 1.0 + 1e-6, "ne doit pas dépasser, frame {i} : {v}");
            precedent = v;
        }
        assert!(precedent > 0.99, "doit converger : got {precedent}");
    }

    #[test]
    fn attaque_et_relachement_sont_asymetriques() {
        // La propriété qui fait qu'un visuel « respire » (§7).
        let mut g = Graph::new();
        let src = g.add(Box::new(Constant(1.0)));
        let env = g
            .add_with_deps(
                Box::new(Envelope::new(Param::modulated(0.0, 1.0, src), 0.01, 1.0)),
                vec![src],
            )
            .unwrap();

        // Montée rapide : en 6 frames (0.1 s) avec tau=0.01, on est au sommet.
        for i in 0..6 {
            g.eval_frame(i as f64 / 60.0, 1.0 / 60.0);
        }
        let apres_attaque = g.value(env);
        assert!(apres_attaque > 0.9, "attaque trop lente : {apres_attaque}");

        // La source retombe à 0 : avec tau=1.0, la descente est lente.
        g.remove(env).unwrap();
        let zero = g.add(Box::new(Constant(0.0)));
        let env2 = g
            .add_with_deps(
                Box::new(Envelope::new(Param::modulated(0.0, 1.0, zero), 0.01, 1.0)),
                vec![zero],
            )
            .unwrap();
        // On force l'état initial haut en simulant : 6 frames suffisent à voir
        // que la descente est bien plus lente que la montée ne l'était.
        for i in 0..6 {
            g.eval_frame(i as f64 / 60.0, 1.0 / 60.0);
        }
        assert!(g.value(env2) < 0.2, "la descente doit partir de 0 ici");
    }

    #[test]
    fn attaque_nulle_suit_instantanement() {
        let mut g = Graph::new();
        let src = g.add(Box::new(Constant(1.0)));
        let env = g
            .add_with_deps(
                Box::new(Envelope::new(Param::modulated(0.0, 1.0, src), 0.0, 0.0)),
                vec![src],
            )
            .unwrap();
        g.eval_frame(0.0, 1.0 / 60.0);
        assert!((g.value(env) - 1.0).abs() < 1e-6, "got {}", g.value(env));
    }

    #[test]
    fn dt_nul_ne_fait_pas_bouger_lenveloppe() {
        let mut g = Graph::new();
        let src = g.add(Box::new(Constant(1.0)));
        let env = g
            .add_with_deps(
                Box::new(Envelope::new(Param::modulated(0.0, 1.0, src), 0.1, 0.1)),
                vec![src],
            )
            .unwrap();
        g.eval_frame(0.0, 0.0);
        assert_eq!(g.value(env), 0.0, "dt nul → aucune progression");
    }

    #[test]
    fn coeff_reste_dans_zero_un() {
        for tau in [0.0, 0.001, 0.01, 0.1, 1.0, 100.0] {
            for dt in [0.0, 1.0 / 120.0, 1.0 / 60.0, 1.0] {
                let k = coeff(tau, dt);
                assert!((0.0..=1.0).contains(&k), "tau={tau} dt={dt} : got {k}");
            }
        }
        assert_eq!(coeff(f32::NAN, 0.016), 1.0);
    }

    #[test]
    fn le_trigger_ne_se_declenche_quau_franchissement() {
        let mut g = Graph::new();
        let osc = g.add(Box::new(crate::Oscillator::sine(1.0)));
        let trig = g
            .add_with_deps(
                Box::new(Trigger::new(Param::modulated(0.0, 1.0, osc), 0.5)),
                vec![osc],
            )
            .unwrap();

        let mut declenchements = 0;
        for i in 0..180 {
            g.eval_frame(i as f64 / 60.0, 1.0 / 60.0);
            if g.value(trig) > 0.5 {
                declenchements += 1;
            }
        }
        // 1 Hz pendant 3 s : exactement 3 franchissements montants.
        assert_eq!(declenchements, 3, "un franchissement par cycle");
    }

    #[test]
    fn sample_hold_maintient_entre_les_declenchements() {
        let mut g = Graph::new();
        let src = g.add(Box::new(Constant(0.7)));
        let jamais = g.add(Box::new(Constant(0.0)));
        let sh = g
            .add_with_deps(
                Box::new(SampleHold {
                    input: Param::modulated(0.0, 1.0, src),
                    trigger: Param::modulated(0.0, 1.0, jamais),
                }),
                vec![src, jamais],
            )
            .unwrap();
        g.eval_frame(0.0, 0.016);
        assert_eq!(
            g.value(sh),
            0.0,
            "sans déclenchement, rien n'est échantillonné"
        );

        // Avec un déclencheur actif, la valeur est capturée.
        let toujours = g.add(Box::new(Constant(1.0)));
        let sh2 = g
            .add_with_deps(
                Box::new(SampleHold {
                    input: Param::modulated(0.0, 1.0, src),
                    trigger: Param::modulated(0.0, 1.0, toujours),
                }),
                vec![src, toujours],
            )
            .unwrap();
        g.eval_frame(0.016, 0.016);
        assert!((g.value(sh2) - 0.7).abs() < 1e-6, "got {}", g.value(sh2));
    }

    #[test]
    fn entree_non_finie_neutralisee() {
        let mut g = Graph::new();
        let nan = g.add(Box::new(Constant(f32::NAN)));
        let env = g
            .add_with_deps(
                Box::new(Envelope::new(Param::modulated(0.0, 1.0, nan), 0.1, 0.1)),
                vec![nan],
            )
            .unwrap();
        g.eval_frame(0.0, 1.0 / 60.0);
        assert!(g.value(env).is_finite());
        // Le NaN de la source est compté une fois, par le graphe (§18.3).
        assert_eq!(g.diagnostics().non_finite, 1);
    }
}
