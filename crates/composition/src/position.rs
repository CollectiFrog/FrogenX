//! `PositionSignal` — l'accumulateur de position, §3bis.3 de
//! `docs/ARCHITECTURE.md`.
//!
//! Une position de lecture (`Transport::position`) ou une progression de
//! transition (`ClipTransition::Fade`/`Wipe`) n'est pas un `Param` : elle
//! avance **seule**, frame après frame, dérivée du temps — c'est le rôle
//! d'un `Signal` (§16.1 de core), pas d'une entrée réglée depuis l'extérieur.
//! `Param::Fixed`/`Modulated` ne s'auto-incrémentent jamais ; les faire
//! avancer via `Command::SetParam` à 60 fps détournerait un mécanisme pensé
//! pour l'édition ponctuelle, pas l'évaluation par frame.
//!
//! Même principe que l'accumulation de phase d'un oscillateur (§16.1,
//! `frogenx_osc::Oscillator`), appliqué à une position au lieu d'une phase
//! `[0, 1)` : non repliée, non bornée — c'est au consommateur (`Transport`,
//! `ClipTransition`) d'interpréter l'unité (secondes, battements, `[0,1]`).

use frogenx_core::{Ctx, NodeState, Param, Signal};

/// Accumulateur : `position += speed * dt` à chaque frame.
///
/// `speed` est un `Param` — c'est bien une entrée réglable (vitesse de
/// lecture, modulable par un LFO pour un scrub organique) — mais la
/// position résultante ne l'est pas, elle est la sortie accumulée.
pub struct PositionSignal {
    pub speed: Param,
}

impl PositionSignal {
    pub fn new(speed: Param) -> Self {
        Self { speed }
    }

    /// Vitesse fixe — le cas le plus courant (lecture à vitesse normale).
    pub fn fixed_speed(speed: f32) -> Self {
        Self {
            speed: Param::fixed(speed),
        }
    }
}

impl Signal for PositionSignal {
    // slots[0] = position accumulée
    fn eval(&self, _t: f64, ctx: &Ctx, state: &mut NodeState) -> f32 {
        let pos = state.get(0) + self.speed.eval(ctx) * ctx.dt as f32;
        state.set(0, pos);
        pos
    }

    fn state_size(&self) -> usize {
        1
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use frogenx_core::{AudioFeatures, Graph};

    #[test]
    fn avance_deterministement_a_vitesse_fixe() {
        let mut g = Graph::new();
        let p = g.add(Box::new(PositionSignal::fixed_speed(1.0)));
        g.eval_frame(0.0, 0.5);
        assert!((g.value(p) - 0.5).abs() < 1e-6, "got {}", g.value(p));
        g.eval_frame(0.5, 0.5);
        assert!((g.value(p) - 1.0).abs() < 1e-6, "got {}", g.value(p));
    }

    #[test]
    fn vitesse_nulle_ne_fait_pas_avancer() {
        let mut g = Graph::new();
        let p = g.add(Box::new(PositionSignal::fixed_speed(0.0)));
        g.eval_frame(0.0, 0.5);
        g.eval_frame(0.5, 0.5);
        assert_eq!(g.value(p), 0.0);
    }

    #[test]
    fn vitesse_negative_recule() {
        let mut g = Graph::new();
        let p = g.add(Box::new(PositionSignal::fixed_speed(-2.0)));
        g.eval_frame(0.0, 0.5);
        assert!((g.value(p) - -1.0).abs() < 1e-6, "got {}", g.value(p));
    }

    #[test]
    fn ignore_lhorloge_systeme_seul_t_et_dt_comptent() {
        // §4 : aucun module ne lit l'horloge système. Deux graphes identiques
        // évalués avec les mêmes (t, dt) doivent produire la même position,
        // quel que soit l'instant réel d'exécution du test.
        let mut g1 = Graph::new();
        let mut g2 = Graph::new();
        let p1 = g1.add(Box::new(PositionSignal::fixed_speed(3.0)));
        let p2 = g2.add(Box::new(PositionSignal::fixed_speed(3.0)));

        for i in 0..10 {
            let t = i as f64 / 60.0;
            g1.eval_frame(t, 1.0 / 60.0);
        }
        // g2 exécuté "plus tard" (simulé en ne faisant rien entre les deux
        // boucles) — le résultat ne doit dépendre que de (t, dt) injectés.
        for i in 0..10 {
            let t = i as f64 / 60.0;
            g2.eval_frame(t, 1.0 / 60.0);
        }

        assert_eq!(g1.value(p1), g2.value(p2));
    }

    #[test]
    fn letat_vit_dans_le_graphe_pas_dans_le_signal() {
        // §2.3 : PositionSignal n'a aucun champ mutable, uniquement `speed`
        // (un Param, en lecture). La position vit dans le NodeState du graphe.
        let audio = AudioFeatures::default();
        let ctx = Ctx::new(1.0 / 60.0, 0, &audio);
        let mut slots = [0.0f32; 1];
        let mut st = NodeState::new(&mut slots);

        let sig = PositionSignal::fixed_speed(1.0);
        sig.eval(0.0, &ctx, &mut st);
        sig.eval(0.0, &ctx, &mut st);
        // Deux appels sur le même NodeState prêté accumulent — la mémoire
        // est bien externe au `Signal`, pas un champ caché dans `sig`.
        assert!((st.get(0) - 2.0 / 60.0).abs() < 1e-6);
    }
}
