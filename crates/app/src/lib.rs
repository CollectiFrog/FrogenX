//! # frogenx-app
//!
//! Le sommet de la chaîne : [`Engine`] déroule une frame complète dans l'ordre
//! du §20.1 de `docs/ARCHITECTURE.md`.
//!
//! `Engine` ne connaît pas de fenêtre (§8.3) : c'est ce qui le rend testable
//! et utilisable pour un rendu offline. Le binaire `winit` l'appellera ; il
//! lui fournira alors `wall_dt`, et c'est tout ce qu'`Engine` saura du temps
//! réel (§4).
//!
//! ## Une frame
//!
//! ```text
//! Clock::tick ─▶ eval_frame sur chaque graphe ─▶ Compositor::render ─▶ Outputs::submit
//! ```

pub mod demo;

use frogenx_core::{AudioFeatures, Clock, ClockMode, Ctx, Diagnostics, Graph};
use frogenx_layer::{Gpu, RenderLayer};
use frogenx_output::{DropPolicy, OutputError, OutputSink, Outputs, SinkStatus};
use frogenx_render::{Compositor, LayerInput};
use glam::UVec2;
use std::time::{Duration, Instant};

/// Un graphe et les couches qu'il pilote.
///
/// **Provisoire** : préfigure un clip (§3, §22.1). L'étape 7 remplacera la
/// liste de groupes par la grille de `composition`.
pub struct SceneGroup {
    pub graph: Graph,
    pub layers: Vec<Box<dyn RenderLayer>>,
}

/// La politique de perte suit l'horloge (§8.0) : `Realtime` perd et compte,
/// `Fixed` attend.
pub fn drop_policy(mode: ClockMode) -> DropPolicy {
    match mode {
        ClockMode::Realtime => DropPolicy::Lossy,
        ClockMode::Fixed { .. } => DropPolicy::Lossless,
    }
}

/// La prochaine échéance du moteur, qui se cadence lui-même (§8.3).
///
/// Une frame en retard ne se rattrape pas : accumuler le retard enchaînerait
/// des frames sans pause, et l'application ne répondrait plus.
pub fn next_deadline(deadline: Instant, now: Instant, period: Duration) -> Instant {
    let suivante = deadline + period;
    if suivante > now {
        suivante
    } else {
        now + period
    }
}

/// Une frame complète, de l'horloge aux sorties.
pub struct Engine {
    gpu: Gpu,
    clock: Clock,
    compositor: Compositor,
    outputs: Outputs,
    scene: Vec<SceneGroup>,
}

impl Engine {
    pub fn new(gpu: Gpu, clock: Clock, size: UVec2, scene: Vec<SceneGroup>) -> Self {
        let compositor = Compositor::new(&gpu, size);
        let outputs = Outputs::new(&gpu, drop_policy(clock.mode()));
        Self {
            gpu,
            clock,
            compositor,
            outputs,
            scene,
        }
    }

    /// Ajoute une sortie — régime d'édition, hors de la boucle (§18.1).
    pub fn add_output(&mut self, sink: OutputSink) -> Result<(), OutputError> {
        self.outputs.add(sink)
    }

    /// Une frame. `wall_dt` n'a d'effet qu'en `Realtime` (§4).
    pub fn frame(&mut self, wall_dt: f64) {
        self.clock.tick(wall_dt);
        let (t, dt) = (self.clock.t(), self.clock.dt());

        // Le graphe est entièrement évalué avant que la moindre couche ne
        // lise un `Param` (§20.1).
        for group in &mut self.scene {
            group.graph.eval_frame(t, dt);
        }

        let audio = AudioFeatures::default();
        let ctx = Ctx::new(dt, self.clock.frame(), &audio);
        let mut inputs: Vec<LayerInput> = Vec::new();
        for group in &mut self.scene {
            let graph = &group.graph;
            for layer in &mut group.layers {
                inputs.push(LayerInput {
                    layer: layer.as_mut(),
                    graph,
                });
            }
        }
        self.compositor.render(&self.gpu, t, &ctx, &mut inputs);
        self.outputs
            .submit(&self.gpu, self.compositor.output_texture(), t);
    }

    /// Attend que les sorties aient fini : à la fin d'un rendu offline, toutes
    /// les frames sont écrites.
    pub fn finish(&mut self) {
        self.outputs.finish();
    }

    pub fn gpu(&self) -> &Gpu {
        &self.gpu
    }

    pub fn clock(&self) -> &Clock {
        &self.clock
    }

    /// Les diagnostics de chaque graphe, pour la dernière frame (§18.1).
    pub fn diagnostics(&self) -> Vec<Diagnostics> {
        self.scene.iter().map(|g| g.graph.diagnostics()).collect()
    }

    pub fn output_statuses(&self) -> Vec<SinkStatus> {
        self.outputs.statuses()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn l_echeance_avance_d_une_periode_quand_le_moteur_est_a_l_heure() {
        let debut = Instant::now();
        let periode = Duration::from_millis(16);
        let maintenant = debut + Duration::from_millis(1);
        assert_eq!(next_deadline(debut, maintenant, periode), debut + periode);
    }

    #[test]
    fn un_retard_ne_se_rattrape_pas() {
        let debut = Instant::now();
        let periode = Duration::from_millis(16);
        let maintenant = debut + Duration::from_secs(5);
        assert_eq!(
            next_deadline(debut, maintenant, periode),
            maintenant + periode
        );
    }

    #[test]
    fn la_politique_de_perte_suit_l_horloge() {
        assert_eq!(drop_policy(ClockMode::Realtime), DropPolicy::Lossy);
        assert_eq!(
            drop_policy(ClockMode::Fixed { fps: 60.0 }),
            DropPolicy::Lossless
        );
    }
}
