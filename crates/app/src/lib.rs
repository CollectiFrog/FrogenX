//! # frogenx-app
//!
//! Le sommet de la chaîne : [`Engine`] déroule une frame complète dans l'ordre
//! du §20.1 de `docs/ARCHITECTURE.md`.
//!
//! `Engine` ne connaît pas de fenêtre (§8.3) : c'est ce qui le rend testable
//! et utilisable pour un rendu offline. Le binaire `frogenx` lui fournit
//! `wall_dt`, et c'est tout ce qu'`Engine` sait du temps réel (§4).
//!
//! ## Une frame
//!
//! ```text
//! commandes ─▶ Clock::tick ─▶ audio ─▶ graphe de composition ─▶ tempo, trigs
//!           ─▶ graphe de chaque clip actif ─▶ Compositor::render ─▶ Outputs::submit
//! ```

pub mod file_flow;
pub mod history;
pub mod last_patch;

use frogenx_composition::{
    sanitize_speed, Clip, ClipSource, ColumnId, Command, Composition, IdMap, LayerId,
};
use frogenx_core::{AnalysisSettings, AudioFeatures, Clock, ClockMode, Ctx, Diagnostics, Graph};
use frogenx_layer::{
    BlendParams, FeedbackRenderer, FeedbackStep, FrameHistory, Gpu, RenderLayer, ShapeContent,
    ShapeRenderer,
};
use frogenx_output::{
    DropPolicy, GpuSink, GpuSinkId, OutputError, OutputSink, Outputs, SinkStatus,
};
use frogenx_render::{Compositor, LayerInput};
use glam::UVec2;
use history::{History, Restore};
use std::time::{Duration, Instant};

/// Ce qu'on vient d'annuler ou de rétablir.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Undone {
    pub label: &'static str,
    /// Les identifiants ont changé : l'interface garde sa sélection par cette table.
    pub map: IdMap,
}

/// La politique de perte suit l'horloge (§8.0) : `Realtime` perd et compte,
/// `Fixed` attend.
pub fn drop_policy(mode: ClockMode) -> DropPolicy {
    match mode {
        ClockMode::Realtime => DropPolicy::Lossy,
        ClockMode::Fixed { .. } => DropPolicy::Lossless,
    }
}

/// Le statut des sorties, en mots : ce que lit l'interface.
pub fn describe_outputs(statuses: &[SinkStatus]) -> String {
    let mut perdues = 0;
    let mut pannes = Vec::new();
    for status in statuses {
        match status {
            SinkStatus::Active => {}
            SinkStatus::Degraded { dropped } => perdues += dropped,
            SinkStatus::Failed(message) => pannes.push(message.as_str()),
        }
    }
    if !pannes.is_empty() {
        format!("Sortie en panne : {}", pannes.join(" ; "))
    } else if perdues > 0 {
        format!("{perdues} frames perdues")
    } else {
        format!("Sorties actives : {}", statuses.len())
    }
}

/// La cadence mesurée, lissée pour que le chiffre reste lisible.
///
/// C'est la durée qu'on moyenne, pas son inverse : quelques frames très
/// rapprochées suffiraient à fausser une moyenne de `1 / dt`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FrameRate {
    dt: f64,
    min: f64,
    max: f64,
    /// Frames vues : l'échauffement (compilation des shaders) ne compte pas
    /// dans les extrêmes.
    seen: u32,
}

/// Frames ignorées par les extrêmes de [`FrameRate`].
const WARM_UP: u32 = 30;

impl FrameRate {
    pub fn new(period: Duration) -> Self {
        Self {
            dt: period.as_secs_f64(),
            min: f64::INFINITY,
            max: 0.0,
            seen: 0,
        }
    }

    pub fn tick(&mut self, dt: f64) {
        if dt > 0.0 {
            self.dt += (dt - self.dt) * 0.05;
            self.seen = self.seen.saturating_add(1);
            if self.seen > WARM_UP {
                self.min = self.min.min(dt);
                self.max = self.max.max(dt);
            }
        }
    }

    pub fn fps(&self) -> f32 {
        (1.0 / self.dt) as f32
    }

    /// Le plus court et le plus long intervalle vus : la régularité, qu'une
    /// moyenne cache.
    pub fn range_ms(&self) -> (f64, f64) {
        (self.min * 1000.0, self.max * 1000.0)
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

/// Le renderer d'une piste : un seul, quel que soit son nombre de clips
/// (§3bis.2).
struct Track {
    layer: LayerId,
    renderer: ShapeRenderer,
    /// Créé au premier feedback de la piste : la plupart n'en auront jamais.
    feedback: Option<FeedbackRenderer>,
}

/// Le clip actif d'une piste, vu par le compositeur le temps d'une frame.
enum ClipLayer<'a> {
    Shape {
        renderer: &'a mut ShapeRenderer,
        content: ShapeContent<'a>,
        blend: BlendParams,
    },
    Feedback {
        renderer: &'a mut FeedbackRenderer,
        step: FeedbackStep,
        blend: BlendParams,
    },
}

impl RenderLayer for ClipLayer<'_> {
    fn prepare(&mut self, t: f64, ctx: &Ctx, g: &Graph, gpu: &Gpu) {
        match self {
            Self::Shape {
                renderer, content, ..
            } => renderer.prepare(content, t, ctx, g, gpu),
            Self::Feedback { renderer, step, .. } => renderer.prepare(step, gpu),
        }
    }

    fn encode(&self, encoder: &mut wgpu::CommandEncoder) {
        match self {
            Self::Shape { renderer, .. } => renderer.encode(encoder),
            Self::Feedback { renderer, .. } => renderer.encode(encoder),
        }
    }

    fn output(&self) -> &wgpu::TextureView {
        match self {
            Self::Shape { renderer, .. } => renderer.output(),
            Self::Feedback { renderer, .. } => renderer.output(),
        }
    }

    fn blend(&self) -> BlendParams {
        match self {
            Self::Shape { blend, .. } | Self::Feedback { blend, .. } => *blend,
        }
    }

    /// La taille d'un feedback ne change pas : `Engine` n'en a qu'une (§8).
    fn resize(&mut self, size: UVec2, gpu: &Gpu) {
        if let Self::Shape { renderer, .. } = self {
            renderer.resize(size, gpu);
        }
    }
}

/// Une frame complète, de la grille aux sorties.
pub struct Engine {
    gpu: Gpu,
    clock: Clock,
    compositor: Compositor,
    /// L'image finale de la frame d'avant, pour les feedbacks (§6.1).
    history: FrameHistory,
    outputs: Outputs,
    composition: Composition,
    size: UVec2,
    tracks: Vec<Track>,
    /// Les pistes du deck actif, réécrites à chaque frame sans allouer.
    order: Vec<LayerId>,
    commands: Vec<Command>,
    /// Les clips à poser : un `Clip` n'est pas une `Command` (§23.6).
    placements: Vec<(LayerId, ColumnId, Clip)>,
    refused_commands: u32,
    /// La grille diffère du fichier : jouer n'en fait pas partie (§23.8).
    dirty: bool,
    steps: History,
    /// Un bouton de souris est tenu : le pas d'annulation reste ouvert (§23.11).
    gesture_held: bool,
    bpm: f32,
    /// Les features de la prochaine frame. Vides sans entrée audio, écrites à
    /// la main par un rendu offline : c'est ce qui garde le déterminisme (§20.3).
    audio: AudioFeatures,
}

impl Engine {
    pub fn new(gpu: Gpu, clock: Clock, size: UVec2, composition: Composition) -> Self {
        let compositor = Compositor::new(&gpu, size);
        let history = FrameHistory::new(&gpu, size);
        let outputs = Outputs::new(&gpu, drop_policy(clock.mode()));
        let bpm = composition.bpm();
        let mut steps = History::default();
        steps.reset(frogenx_patch::to_json(&composition).ok());
        Self {
            gpu,
            clock,
            compositor,
            history,
            outputs,
            composition,
            size,
            tracks: Vec::new(),
            order: Vec::new(),
            commands: Vec::new(),
            placements: Vec::new(),
            refused_commands: 0,
            dirty: false,
            steps,
            gesture_held: false,
            bpm,
            audio: AudioFeatures::default(),
        }
    }

    /// Ajoute une sortie — régime d'édition, hors de la boucle (§18.1).
    pub fn add_output(&mut self, sink: OutputSink) -> Result<(), OutputError> {
        self.outputs.add(sink)
    }

    /// Ajoute une sortie GPU qu'on pourra retirer. `Engine` ne sait pas qu'elle
    /// présente dans une fenêtre (§8.4).
    pub fn add_gpu_output(&mut self, sink: Box<dyn GpuSink>) -> GpuSinkId {
        self.outputs.add_gpu(sink)
    }

    /// Retire une sortie GPU, entre deux frames. Rend `false` si elle est déjà
    /// retirée.
    pub fn remove_gpu_output(&mut self, id: GpuSinkId) -> bool {
        self.outputs.remove_gpu(id)
    }

    /// Met une commande en file. Elle s'applique au début de la frame suivante,
    /// jamais pendant l'évaluation (§23).
    pub fn push(&mut self, command: Command) {
        self.commands.push(command);
    }

    /// Pose un clip dans une case au début de la frame suivante, après les
    /// commandes (§23.6). Un refus se compte comme une commande refusée.
    pub fn place(&mut self, layer: LayerId, column: ColumnId, clip: Clip) {
        self.placements.push((layer, column, clip));
    }

    /// Pose les features audio de la prochaine frame. `Engine` ne connaît pas
    /// la carte son : le binaire lit l'entrée, un rendu offline les fabrique.
    pub fn set_audio(&mut self, audio: AudioFeatures) {
        self.audio = audio;
    }

    /// Les features de la dernière frame.
    pub fn audio(&self) -> &AudioFeatures {
        &self.audio
    }

    /// Ce que la composition demande à l'analyse audio (§7).
    pub fn analysis(&self) -> AnalysisSettings {
        self.composition.analysis.resolve(self.composition.graph())
    }

    /// Remplace toute la grille, entre deux frames. Ce qui attendait dans les
    /// files visait l'ancienne, et est oublié (§23.8).
    pub fn replace_composition(&mut self, composition: Composition) {
        self.commands.clear();
        self.placements.clear();
        self.bpm = composition.bpm();
        self.steps.reset(frogenx_patch::to_json(&composition).ok());
        self.composition = composition;
        self.dirty = false;
    }

    /// Un bouton de souris est-il tenu ? Tant qu'il l'est, les modifications font
    /// un seul pas d'annulation (§23.11).
    pub fn set_gesture(&mut self, held: bool) {
        self.gesture_held = held;
    }

    /// Ferme le pas ouvert sur l'état de la grille.
    fn commit_step(&mut self) {
        if self.steps.has_pending() {
            self.steps
                .commit(frogenx_patch::to_json(&self.composition).ok());
        }
    }

    /// Le nom de ce qu'annulerait [`Self::undo`].
    pub fn undo_label(&self) -> Option<&'static str> {
        self.steps.undo_label()
    }

    pub fn redo_label(&self) -> Option<&'static str> {
        self.steps.redo_label()
    }

    /// Annule le dernier pas. Rend son nom et les nouveaux identifiants.
    pub fn undo(&mut self) -> Option<Undone> {
        self.commit_step();
        let restaure = self.steps.undo()?;
        self.restore(restaure)
    }

    /// Rétablit le dernier pas annulé.
    pub fn redo(&mut self) -> Option<Undone> {
        self.commit_step();
        let restaure = self.steps.redo()?;
        self.restore(restaure)
    }

    /// Remplace la grille par une image, en reprenant ce qui joue. Ce qui
    /// attendait dans les files visait l'ancienne grille.
    fn restore(&mut self, restaure: Restore) -> Option<Undone> {
        let mut grille = frogenx_patch::from_json(&restaure.json).ok()?;
        let map = grille.adopt_from(&self.composition);
        self.commands.clear();
        self.placements.clear();
        self.composition = grille;
        self.dirty = !self.steps.at_saved();
        Some(Undone {
            label: restaure.label,
            map,
        })
    }

    /// Remplace la grille par une grille d'usine et joue sa première colonne.
    pub fn play_starter(&mut self, composition: Composition) {
        let premiere = composition.columns().first().copied();
        self.replace_composition(composition);
        if let Some(column) = premiere {
            self.push(Command::TriggerColumn { column });
        }
    }

    /// La grille a-t-elle changé depuis le dernier enregistrement ?
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Le fichier vient d'être écrit (`false`), ou l'écriture a échoué (`true`).
    pub fn set_dirty(&mut self, dirty: bool) {
        self.commit_step();
        if dirty {
            self.steps.mark_unsaved();
        } else {
            self.steps.mark_saved();
        }
        self.dirty = dirty;
    }

    /// Une frame. `wall_dt` n'a d'effet qu'en `Realtime` (§4).
    pub fn frame(&mut self, wall_dt: f64) {
        let instant = self.clock.t();
        for command in self.commands.drain(..) {
            if self.composition.apply(command, instant) {
                if command.is_edit() {
                    self.dirty = true;
                    self.steps.note(command.label());
                }
            } else {
                self.refused_commands = self.refused_commands.saturating_add(1);
            }
        }
        for (layer, column, clip) in self.placements.drain(..) {
            if self.composition.place_clip(layer, column, clip).is_some() {
                self.dirty = true;
                self.steps.note("Pose d'un clip");
            } else {
                self.refused_commands = self.refused_commands.saturating_add(1);
            }
        }

        if self.steps.tick(self.gesture_held) {
            self.commit_step();
        }

        // Pas une modification du fichier : le choix de suivre en est une (§7.1).
        self.composition.follow_tempo(&self.audio);

        self.clock.tick(wall_dt);
        let (t, dt) = (self.clock.t(), self.clock.dt());

        // Les graphes sont entièrement évalués avant que la moindre couche ne
        // lise un `Param` (§20.1). Celui de la composition d'abord : il donne
        // le tempo que voient les clips.
        self.composition.graph_mut().set_audio(self.audio.clone());
        self.composition.graph_mut().eval_frame(t, dt);
        self.bpm = self.composition.bpm();
        // Une commande ne s'applique qu'entre deux frames (§23) : un trig agit
        // donc à la frame suivante, une de retard sur le spectre.
        self.composition.trig_commands(&mut self.commands);
        self.composition.layers_in_order(&mut self.order);
        for layer in &self.order {
            let actif = self.composition.layer(*layer).and_then(|l| l.active_clip());
            let Some(id) = actif else { continue };
            // Lue dans le graphe de composition, déjà évalué (§3bis.2).
            let vitesse = self.composition.clip(id).map_or(1.0, |clip| {
                sanitize_speed(clip.speed.get(self.composition.graph()))
            });
            if let Some(clip) = self.composition.clip_mut(id) {
                let graph = clip.graph_mut();
                graph.set_bpm(self.bpm);
                graph.set_audio(self.audio.clone());
                graph.eval_frame(t, dt * vitesse as f64);
            }
        }
        self.sync_tracks();

        let ctx = Ctx::new(dt, self.clock.frame(), &self.audio).with_bpm(self.bpm);
        let composition = &self.composition;
        let mut couches: Vec<(ClipLayer, &Graph)> = Vec::with_capacity(self.tracks.len());
        for track in &mut self.tracks {
            let Some(piste) = composition.layer(track.layer) else {
                continue;
            };
            let Some(clip) = piste.active_clip().and_then(|id| composition.clip(id)) else {
                continue;
            };
            let couche = match &clip.source {
                ClipSource::Shape {
                    generator, style, ..
                } => ClipLayer::Shape {
                    renderer: &mut track.renderer,
                    content: ShapeContent {
                        generator: generator.as_ref(),
                        effects: &clip.effects,
                        style: *style,
                    },
                    blend: piste.blend,
                },
                ClipSource::Feedback { graph, params } => {
                    // Comme le graphe du clip : sa vitesse multiplie son temps.
                    let vitesse = sanitize_speed(clip.speed.get(composition.graph()));
                    let (gpu, taille, history) = (&self.gpu, self.size, &self.history);
                    ClipLayer::Feedback {
                        renderer: track
                            .feedback
                            .get_or_insert_with(|| FeedbackRenderer::new(gpu, taille, history)),
                        step: params.step(graph, dt * vitesse as f64),
                        blend: piste.blend,
                    }
                }
            };
            couches.push((couche, clip.graph()));
        }
        let mut inputs: Vec<LayerInput> = couches
            .iter_mut()
            .map(|(layer, graph)| LayerInput {
                layer,
                graph,
                blend_graph: composition.graph(),
            })
            .collect();
        self.compositor.render(&self.gpu, t, &ctx, &mut inputs);
        drop(inputs);
        drop(couches);

        let mut encoder = self
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("feedback.history"),
            });
        self.history
            .record(&mut encoder, self.compositor.output_texture());
        self.gpu.queue.submit(std::iter::once(encoder.finish()));

        self.outputs
            .submit(&self.gpu, self.compositor.output_texture(), t);
    }

    /// Un renderer par piste du deck actif, dans le même ordre. Un changement
    /// de deck réutilise ceux qui existent déjà.
    fn sync_tracks(&mut self) {
        for (i, layer) in self.order.iter().enumerate() {
            if self.tracks.get(i).is_some_and(|t| t.layer == *layer) {
                continue;
            }
            match self.tracks.iter().position(|t| t.layer == *layer) {
                Some(j) => self.tracks.swap(i, j),
                None => self.tracks.insert(
                    i,
                    Track {
                        layer: *layer,
                        renderer: ShapeRenderer::new(&self.gpu, self.size),
                        feedback: None,
                    },
                ),
            }
        }
        self.tracks.truncate(self.order.len());
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

    pub fn composition(&self) -> &Composition {
        &self.composition
    }

    /// Le tempo de la dernière frame.
    pub fn bpm(&self) -> f32 {
        self.bpm
    }

    /// Commandes refusées depuis le début : un identifiant périmé ne panique
    /// pas, mais il se voit (§18.1).
    pub fn refused_commands(&self) -> u32 {
        self.refused_commands
    }

    /// Les diagnostics de la dernière frame : le graphe de composition, puis
    /// chaque clip actif (§18.1).
    pub fn diagnostics(&self) -> Vec<Diagnostics> {
        let actifs = self.order.iter().filter_map(|layer| {
            let id = self.composition.layer(*layer)?.active_clip()?;
            Some(self.composition.clip(id)?.graph().diagnostics())
        });
        std::iter::once(self.composition.graph().diagnostics())
            .chain(actifs)
            .collect()
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
    fn la_cadence_n_est_pas_faussee_par_des_frames_rapprochees() {
        // Un intervalle sur deux très court : 30 fps... en moyenne de durées
        // (8.5 + 25 ms ne font pas une frame sur 16 ms).
        let mut r = FrameRate::new(Duration::from_millis(16));
        for _ in 0..1000 {
            r.tick(0.002);
            r.tick(0.0313);
        }
        assert!((r.fps() - 60.0).abs() < 4.0, "got {}", r.fps());
        let (min, max) = r.range_ms();
        assert!((min - 2.0).abs() < 1e-6 && (max - 31.3).abs() < 1e-6);
    }

    #[test]
    fn la_cadence_ignore_un_intervalle_nul() {
        let mut r = FrameRate::new(Duration::from_millis(16));
        let avant = r.fps();
        r.tick(0.0);
        assert_eq!(r.fps(), avant);
    }

    #[test]
    fn le_statut_des_sorties_dit_d_abord_le_plus_grave() {
        let actif = SinkStatus::Active;
        let lent = SinkStatus::Degraded { dropped: 3 };
        let panne = SinkStatus::Failed("disque plein".into());
        assert_eq!(
            describe_outputs(std::slice::from_ref(&actif)),
            "Sorties actives : 1"
        );
        assert_eq!(
            describe_outputs(&[actif.clone(), lent.clone()]),
            "3 frames perdues"
        );
        assert_eq!(
            describe_outputs(&[actif, lent, panne]),
            "Sortie en panne : disque plein"
        );
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
