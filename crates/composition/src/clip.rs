//! Le `Clip` — point de jonction avec le graphe modulaire, §3bis.2 de
//! `docs/ARCHITECTURE.md`.

use frogenx_core::{FeatureKind, Graph, NodeId, Param, Signal};
use frogenx_osc::{AudioFeature, TrigInput};
use frogenx_shape::{
    ColorParam, FeedbackParams, MixKind, ParamDesc, ShapeGenerator, ShapeOp, ShapeStyle,
    VideoParams, COLOR_CHANNELS,
};
use std::path::Path;

use crate::crossfader::FadeCurve;
use crate::position::{PositionSignal, VideoRate};
use crate::transport::{PlayMode, Transport};

/// Ce qu'un clip joue. La distinction se fait ici, pas sur le type `Clip`
/// lui-même — un clip procédural module ses propres paramètres via son
/// `Graph`, exactement comme avant la réorientation (§3bis.2).
pub enum ClipSource {
    /// Forme procédurale : le générateur produit le `Path`, le graphe module
    /// ses paramètres. Un graphe seul ne produirait aucune forme — les effets
    /// du clip n'auraient rien à transformer (§3bis.2).
    Shape {
        graph: Graph,
        generator: Box<dyn ShapeGenerator>,
        style: ShapeStyle,
    },
    /// L'image finale de la frame d'avant, déplacée et affaiblie (§6.1).
    Feedback {
        graph: Graph,
        params: FeedbackParams,
    },
    /// Un fichier vidéo. `composition` ne décode rien : elle dit lequel, et quel
    /// instant montrer (§9.3).
    Video {
        graph: Graph,
        params: VideoParams,
        /// Ce qu'il fait au bout du fichier.
        mode: PlayMode,
    },
    // `Scene3d(Graph)` arrive à l'étape 13 de la feuille de route (§12), pour ne
    // pas figer un contrat deviné.
}

/// Comment un clip entre en scène ou en sort quand un autre clip prend sa
/// place sur le même `Layer` — §3bis.2.
///
/// `Fade`/`Wipe` réutilisent [`PositionSignal`] pour leur progression — une
/// transition qui avance seule a le même besoin que `Transport.position`
/// (§3bis.3) : ce n'est pas un `Param` statique, c'est un accumulateur.
pub enum ClipTransition {
    /// Coupure instantanée. `composition` ne rend rien : le mélange de deux
    /// clips pendant un `Fade`/`Wipe` est le travail du compositeur de
    /// couches (étape 5 et suivantes, §12), que `composition` ne doit pas
    /// connaître (§10). Ce type fixe seulement la forme du contrat.
    Cut,
    Fade {
        phase: PositionSignal,
        curve: FadeCurve,
    },
    Wipe {
        phase: PositionSignal,
        curve: FadeCurve,
        direction: WipeDirection,
    },
}

/// Le choix d'une transition, copiable : ce qu'une commande transporte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionKind {
    Cut,
    Fade,
    Wipe(WipeDirection),
}

impl TransitionKind {
    pub const ALL: [TransitionKind; 6] = [
        TransitionKind::Cut,
        TransitionKind::Fade,
        TransitionKind::Wipe(WipeDirection::Left),
        TransitionKind::Wipe(WipeDirection::Right),
        TransitionKind::Wipe(WipeDirection::Up),
        TransitionKind::Wipe(WipeDirection::Down),
    ];

    pub fn label(self) -> &'static str {
        match self {
            TransitionKind::Cut => "Coupure",
            TransitionKind::Fade => "Fondu",
            TransitionKind::Wipe(WipeDirection::Left) => "Volet vers la gauche",
            TransitionKind::Wipe(WipeDirection::Right) => "Volet vers la droite",
            TransitionKind::Wipe(WipeDirection::Up) => "Volet vers le haut",
            TransitionKind::Wipe(WipeDirection::Down) => "Volet vers le bas",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WipeDirection {
    Left,
    Right,
    Up,
    Down,
}

/// Où vit un paramètre du clip. Reste valable d'une frame à l'autre, tant
/// que le clip ne change pas de structure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamSlot {
    Generator(usize),
    Effect {
        effect: usize,
        param: usize,
    },
    /// Un canal du remplissage, dans l'ordre de `COLOR_CHANNELS`.
    Fill(usize),
    /// 0 : l'épaisseur. 1 à 4 : les canaux de la couleur du trait.
    Stroke(usize),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClipParam {
    pub slot: ParamSlot,
    pub desc: ParamDesc,
    /// La feature audio qui le module, s'il y en a une (§7.1).
    pub audio: Option<FeatureKind>,
    /// Le canal du trig qui le module, s'il y en a un (§3bis.6).
    pub trig: Option<u32>,
}

/// Les paramètres d'une partie du clip : son générateur, un effet, son style.
#[derive(Debug, Clone, PartialEq)]
pub struct ParamGroup {
    pub label: String,
    pub params: Vec<ClipParam>,
}

/// La feature audio que lit `param`, si sa source en est une.
fn audio_source(graph: &Graph, param: Param) -> Option<FeatureKind> {
    let spec = graph.node_spec(param.source()?)?;
    (spec.kind == "AudioFeature")
        .then(|| FeatureKind::from_word(spec.word("feature").ok()?))
        .flatten()
}

/// Le canal du trig que lit `param`, si sa source en est un.
fn trig_source(graph: &Graph, param: Param) -> Option<u32> {
    let spec = graph.node_spec(param.source()?)?;
    (spec.kind == "TrigInput")
        .then(|| spec.int("channel").ok())
        .flatten()
}

/// Les canaux d'une couleur, décrits pour un curseur.
fn color_descs(color: ColorParam) -> impl Iterator<Item = ParamDesc> {
    color
        .channels()
        .into_iter()
        .zip(COLOR_CHANNELS)
        .map(|(value, name)| ParamDesc {
            name,
            min: 0.0,
            max: 1.0,
            value,
        })
}

/// La durée proposée pour une nouvelle transition, en battements.
pub const DEFAULT_TRANSITION_BEATS: f32 = 2.0;

/// La vitesse maximale d'un clip, appliquée à la lecture (§18.4).
pub const MAX_CLIP_SPEED: f32 = 4.0;

/// Une vitesse utilisable : saturée, et 1 si elle n'est pas finie.
pub fn sanitize_speed(speed: f32) -> f32 {
    if speed.is_finite() {
        speed.clamp(0.0, MAX_CLIP_SPEED)
    } else {
        1.0
    }
}

/// Source + effets + transport, pilotable par son propre `Graph` — §3bis.2.
///
/// **Pas de `Serialize` ici.** `effects` et `source` portent des trait
/// objects (`Box<dyn ShapeOp>`, `Graph`) qui ne se sérialisent pas tels
/// quels — c'est le rôle du format de patch (§19.1) de les décrire par
/// `kind`/`params` nommés, pas de cette structure en mémoire.
pub struct Clip {
    /// Ce qu'affiche sa cellule dans la grille (§23.5).
    pub name: String,
    pub source: ClipSource,
    pub effects: Vec<Box<dyn ShapeOp>>,
    pub transport: Transport,
    pub transition: ClipTransition,
    /// Multiplie le `dt` de son graphe (§3bis.2). Se résout dans le graphe de
    /// composition : on doit la connaître avant d'évaluer celui du clip.
    pub speed: Param,
}

impl Clip {
    /// Un clip de forme procédurale, sans effet, coupure instantanée. Il porte
    /// le nom de sa forme.
    pub fn shape(graph: Graph, generator: Box<dyn ShapeGenerator>, transport: Transport) -> Self {
        Self {
            name: generator.kind().to_string(),
            source: ClipSource::Shape {
                graph,
                generator,
                style: ShapeStyle::default(),
            },
            effects: Vec::new(),
            transport,
            transition: ClipTransition::Cut,
            speed: Param::fixed(1.0),
        }
    }

    /// Un clip de feedback (§6.1). Il porte le nom de son type.
    pub fn feedback(graph: Graph, params: FeedbackParams, transport: Transport) -> Self {
        Self {
            name: params.kind().to_string(),
            source: ClipSource::Feedback { graph, params },
            effects: Vec::new(),
            transport,
            transition: ClipTransition::Cut,
            speed: Param::fixed(1.0),
        }
    }

    /// Un clip vidéo, à sa vitesse réelle. Il porte le nom du fichier.
    pub fn video(params: VideoParams) -> Self {
        let mut graph = Graph::new();
        let rate = graph.add(Box::new(VideoRate::new(params.duration, 0.0)));
        let speed = Param::modulated(0.0, 1.0, rate);
        let position = graph
            .add_with_deps(Box::new(PositionSignal::new(speed)), vec![rate])
            .expect("`rate` vient d'être créé");
        Self::video_with(
            graph,
            params,
            PlayMode::Loop,
            Transport::Timeline { position, speed },
        )
    }

    /// Un clip vidéo dont le graphe existe déjà : celui d'un fichier.
    pub fn video_with(
        graph: Graph,
        params: VideoParams,
        mode: PlayMode,
        transport: Transport,
    ) -> Self {
        let nom = Path::new(&params.path).file_stem().map_or_else(
            || params.kind().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        Self {
            name: nom,
            source: ClipSource::Video {
                graph,
                params,
                mode,
            },
            effects: Vec::new(),
            transport,
            transition: ClipTransition::Cut,
            speed: Param::fixed(1.0),
        }
    }

    /// Ce que dit un clip vidéo : son fichier, son mode, et ses temps de boucle
    /// (`0` : vitesse réelle).
    pub fn video_playback(&self) -> Option<(&VideoParams, PlayMode, f32)> {
        let ClipSource::Video { params, mode, .. } = &self.source else {
            return None;
        };
        let beats = self
            .video_rate()
            .and_then(|rate| self.graph().node_spec(rate))
            .and_then(|spec| spec.param("beats").ok())
            .map_or(0.0, |p| p.base());
        Some((params, *mode, beats))
    }

    fn video_rate(&self) -> Option<NodeId> {
        match self.transport {
            Transport::Timeline { speed, .. } => speed.source(),
            Transport::BpmSync { .. } => None,
        }
    }

    /// Choisit sa lecture. `beats` à `0` : sa vitesse réelle. Rend `false` si le
    /// clip n'est pas une vidéo.
    pub fn set_video_playback(&mut self, mode: PlayMode, beats: f32) -> bool {
        let Some(rate) = self.video_rate() else {
            return false;
        };
        let ClipSource::Video {
            graph,
            params,
            mode: actuel,
        } = &mut self.source
        else {
            return false;
        };
        if !beats.is_finite() || beats < 0.0 {
            return false;
        }
        *actuel = mode;
        let signal = VideoRate::new(params.duration, beats);
        graph.replace(rate, Box::new(signal)).is_ok()
    }

    /// L'instant de la vidéo à montrer, en secondes. La position avance sans fin :
    /// en boucle, elle se replie sur la durée.
    pub fn video_time(&self) -> Option<f64> {
        let (mode, position, duree) = self.position_beats()?;
        let ClipSource::Video { .. } = self.source else {
            return None;
        };
        let (position, duree) = (position as f64, duree as f64);
        Some(match mode {
            PlayMode::Loop => position.rem_euclid(duree),
            PlayMode::Once | PlayMode::PingPong => position.clamp(0.0, duree),
        })
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// Tout ce qui se règle dans ce clip, sans connaître aucun type de forme
    /// (§16.3 ter). Les `Param` de son graphe interne n'y figurent pas.
    pub fn params(&self) -> Vec<ParamGroup> {
        let graph = self.graph();
        let enumere = |descs: Vec<ParamDesc>, slot: &dyn Fn(usize) -> ParamSlot| {
            descs
                .into_iter()
                .enumerate()
                .map(|(i, desc)| ClipParam {
                    slot: slot(i),
                    desc,
                    audio: audio_source(graph, desc.value),
                    trig: trig_source(graph, desc.value),
                })
                .collect()
        };

        let (generator, style) = match &self.source {
            ClipSource::Shape {
                generator, style, ..
            } => (generator, style),
            ClipSource::Feedback { params, .. } => {
                return vec![ParamGroup {
                    label: params.kind().to_string(),
                    params: enumere(params.params(), &ParamSlot::Generator),
                }];
            }
            ClipSource::Video { params, .. } => {
                return vec![ParamGroup {
                    label: params.kind().to_string(),
                    params: enumere(params.params(), &ParamSlot::Generator),
                }];
            }
        };
        let mut groupes = vec![ParamGroup {
            label: generator.kind().to_string(),
            params: enumere(generator.params(), &ParamSlot::Generator),
        }];
        for (effect, op) in self.effects.iter().enumerate() {
            groupes.push(ParamGroup {
                label: op.kind().to_string(),
                params: enumere(op.params(), &|param| ParamSlot::Effect { effect, param }),
            });
        }
        if let Some(fill) = style.fill {
            groupes.push(ParamGroup {
                label: "Remplissage".to_string(),
                params: enumere(color_descs(fill).collect(), &ParamSlot::Fill),
            });
        }
        if let Some(stroke) = style.stroke {
            let epaisseur = ParamDesc {
                name: "épaisseur",
                min: 0.0,
                max: 0.2,
                value: stroke.width,
            };
            let descs = std::iter::once(epaisseur)
                .chain(color_descs(stroke.color))
                .collect();
            groupes.push(ParamGroup {
                label: "Trait".to_string(),
                params: enumere(descs, &ParamSlot::Stroke),
            });
        }
        groupes
    }

    /// Règle un paramètre en gardant sa modulation (§16.3 ter). Rend `false`
    /// si `slot` ne désigne rien ou si `value` n'est pas fini.
    pub fn set_param(&mut self, slot: ParamSlot, value: f32) -> bool {
        if !value.is_finite() {
            return false;
        }
        match self.param_mut(slot) {
            Some(p) => {
                p.set_base(value);
                true
            }
            None => false,
        }
    }

    /// Règle l'amplitude d'un paramètre modulé. Rend `false` si `slot` ne
    /// désigne rien, si le paramètre n'est pas modulé ou si `value` n'est pas
    /// fini.
    pub fn set_depth(&mut self, slot: ParamSlot, value: f32) -> bool {
        value.is_finite() && self.param_mut(slot).is_some_and(|p| p.set_depth(value))
    }

    /// Branche `feature` sur un paramètre, ou l'en débranche (§7.1). Une nouvelle
    /// source part d'une demi-plage d'amplitude ; changer de feature garde celle
    /// qui était réglée. `None` n'efface qu'une source audio : le LFO d'un clip du
    /// catalogue reste en place. Rend `false` si `slot` ne désigne rien.
    pub fn set_audio_modulation(&mut self, slot: ParamSlot, feature: Option<FeatureKind>) -> bool {
        let Some(param) = self.clip_param(slot) else {
            return false;
        };
        if param.audio == feature && param.trig.is_none() {
            return true;
        }
        let source = feature.map(|f| Box::new(AudioFeature::new(f)) as Box<dyn Signal>);
        self.replace_source(param, source);
        true
    }

    /// Branche le trig de ce canal sur un paramètre (§3bis.6). Remplace la source
    /// d'un menu, comme [`Self::set_audio_modulation`].
    pub fn set_trig_modulation(&mut self, slot: ParamSlot, channel: u32) -> bool {
        let Some(param) = self.clip_param(slot) else {
            return false;
        };
        if param.trig != Some(channel) {
            self.replace_source(param, Some(Box::new(TrigInput::new(channel))));
        }
        true
    }

    fn clip_param(&self, slot: ParamSlot) -> Option<ClipParam> {
        let groupes = self.params();
        let params = groupes.iter().flat_map(|g| &g.params);
        params.into_iter().find(|p| p.slot == slot).copied()
    }

    /// Pose une nouvelle source sur `param`, ou aucune. Seule une source posée
    /// par un menu se retire : un câble tiré dans l'éditeur reste à l'éditeur.
    fn replace_source(&mut self, param: ClipParam, source: Option<Box<dyn Signal>>) {
        let valeur = param.desc.value;
        let du_menu = param.audio.is_some() || param.trig.is_some();
        let ancienne = valeur.source().filter(|_| du_menu);
        let amplitude = valeur
            .depth()
            .filter(|_| ancienne.is_some())
            .unwrap_or((param.desc.max - param.desc.min) * 0.5);
        let nouvelle = match source {
            Some(source) => {
                let source = self.graph_mut().add(source);
                Param::modulated(valeur.base(), amplitude, source)
            }
            None => Param::fixed(valeur.base()),
        };
        if let Some(p) = self.param_mut(param.slot) {
            *p = nouvelle;
        }
        if let Some(source) = ancienne {
            let _ = self.graph_mut().remove(source);
        }
    }

    /// Remplace un paramètre entier, source comprise (§23.10). Rend `false` si
    /// `slot` ne désigne rien.
    pub fn replace_param(&mut self, slot: ParamSlot, param: Param) -> bool {
        match self.param_mut(slot) {
            Some(p) => {
                *p = param;
                true
            }
            None => false,
        }
    }

    fn param_mut(&mut self, slot: ParamSlot) -> Option<&mut Param> {
        let (generator, style) = match &mut self.source {
            ClipSource::Shape {
                generator, style, ..
            } => (generator, style),
            ClipSource::Feedback { params, .. } => {
                return match slot {
                    ParamSlot::Generator(i) => params.param_mut(i),
                    _ => None,
                };
            }
            ClipSource::Video { params, .. } => {
                return match slot {
                    ParamSlot::Generator(i) => params.param_mut(i),
                    _ => None,
                };
            }
        };
        match slot {
            ParamSlot::Generator(i) => generator.param_mut(i),
            ParamSlot::Effect { effect, param } => self
                .effects
                .get_mut(effect)
                .and_then(|op| op.param_mut(param)),
            ParamSlot::Fill(i) => style.fill.as_mut().and_then(|c| c.channel_mut(i)),
            ParamSlot::Stroke(0) => style.stroke.as_mut().map(|s| &mut s.width),
            ParamSlot::Stroke(i) => style
                .stroke
                .as_mut()
                .and_then(|s| s.color.channel_mut(i - 1)),
        }
    }

    pub fn with_style(mut self, new_style: ShapeStyle) -> Self {
        if let ClipSource::Shape { style, .. } = &mut self.source {
            *style = new_style;
        }
        self
    }

    /// Le graphe qui pilote ce clip.
    pub fn graph(&self) -> &Graph {
        match &self.source {
            ClipSource::Shape { graph, .. }
            | ClipSource::Feedback { graph, .. }
            | ClipSource::Video { graph, .. } => graph,
        }
    }

    pub fn graph_mut(&mut self) -> &mut Graph {
        match &mut self.source {
            ClipSource::Shape { graph, .. }
            | ClipSource::Feedback { graph, .. }
            | ClipSource::Video { graph, .. } => graph,
        }
    }

    pub fn with_effect(mut self, op: Box<dyn ShapeOp>) -> Self {
        self.effects.push(op);
        self
    }

    pub fn transition_kind(&self) -> TransitionKind {
        match &self.transition {
            ClipTransition::Cut => TransitionKind::Cut,
            ClipTransition::Fade { .. } => TransitionKind::Fade,
            ClipTransition::Wipe { direction, .. } => TransitionKind::Wipe(*direction),
        }
    }

    /// La part de transition parcourue par battement. `None` pour une coupure, ou
    /// si elle n'a pas de sens : la transition se termine alors net (§18.4).
    pub(crate) fn transition_rate(&self) -> Option<f32> {
        let phase = match &self.transition {
            ClipTransition::Cut => return None,
            ClipTransition::Fade { phase, .. } | ClipTransition::Wipe { phase, .. } => phase,
        };
        Some(phase.speed.get(self.graph())).filter(|v| v.is_finite() && *v > 0.0)
    }

    /// La durée de sa transition, en battements.
    pub fn transition_beats(&self) -> f32 {
        self.transition_rate()
            .map_or(DEFAULT_TRANSITION_BEATS, |rate| 1.0 / rate)
    }

    /// La courbe et le type de mélange de sa transition, si elle en a une.
    pub(crate) fn transition_mix(&self) -> Option<(FadeCurve, MixKind)> {
        Some(match &self.transition {
            ClipTransition::Cut => return None,
            ClipTransition::Fade { curve, .. } => (*curve, MixKind::Fade),
            ClipTransition::Wipe {
                curve, direction, ..
            } => (
                *curve,
                match direction {
                    WipeDirection::Left => MixKind::WipeLeft,
                    WipeDirection::Right => MixKind::WipeRight,
                    WipeDirection::Up => MixKind::WipeUp,
                    WipeDirection::Down => MixKind::WipeDown,
                },
            ),
        })
    }

    /// Choisit sa transition, en battements. Rend `false` si la durée n'a pas de
    /// sens.
    pub fn set_transition(&mut self, kind: TransitionKind, beats: f32) -> bool {
        if !beats.is_finite() || beats <= 0.0 {
            return false;
        }
        let phase = PositionSignal::fixed_speed(1.0 / beats);
        let curve = FadeCurve::SmoothStep;
        self.transition = match kind {
            TransitionKind::Cut => ClipTransition::Cut,
            TransitionKind::Fade => ClipTransition::Fade { phase, curve },
            TransitionKind::Wipe(direction) => ClipTransition::Wipe {
                phase,
                curve,
                direction,
            },
        };
        true
    }

    /// Son mode de lecture et sa durée en battements, s'il est calé sur le tempo.
    pub fn playback(&self) -> Option<(PlayMode, f32)> {
        match self.transport {
            Transport::BpmSync {
                duration_beats,
                play_mode,
                ..
            } => Some((play_mode, duration_beats.get(self.graph()))),
            Transport::Timeline { .. } => None,
        }
    }

    /// Ce que sa position impose à la piste cette frame : `None` si sa durée n'a
    /// pas de sens, et le clip reste en boucle (§18.4).
    pub(crate) fn position_beats(&self) -> Option<(PlayMode, f32, f32)> {
        // Une vidéo compte en secondes : sa durée remplace `duration_beats` (§9.3).
        if let (ClipSource::Video { params, mode, .. }, Transport::Timeline { position, .. }) =
            (&self.source, self.transport)
        {
            let duree = params.duration;
            return (duree.is_finite() && duree > 0.0)
                .then(|| (*mode, self.graph().value(position), duree));
        }
        let Transport::BpmSync {
            position,
            duration_beats,
            play_mode,
        } = self.transport
        else {
            return None;
        };
        let duree = duration_beats.get(self.graph());
        (duree.is_finite() && duree > 0.0).then(|| (play_mode, self.graph().value(position), duree))
    }

    /// Choisit sa lecture. Rend `false` si la durée n'a pas de sens, ou si le clip
    /// n'est pas calé sur le tempo.
    pub fn set_playback(&mut self, mode: PlayMode, beats: f32) -> bool {
        if !beats.is_finite() || beats <= 0.0 {
            return false;
        }
        match &mut self.transport {
            Transport::BpmSync {
                duration_beats,
                play_mode,
                ..
            } => {
                duration_beats.set_base(beats);
                *play_mode = mode;
                true
            }
            Transport::Timeline { .. } => false,
        }
    }

    pub fn with_transition(mut self, transition: ClipTransition) -> Self {
        self.transition = transition;
        self
    }
}

#[cfg(test)]
mod tests_video {
    use super::*;

    fn video() -> Clip {
        Clip::video(VideoParams::new("D:/prises/concert.mp4", 4.0))
    }

    fn avance(c: &mut Clip, secondes: f64) {
        for _ in 0..(secondes * 60.0).round() as usize {
            c.graph_mut().set_bpm(120.0);
            c.graph_mut().eval_frame(0.0, 1.0 / 60.0);
        }
    }

    #[test]
    fn un_clip_video_porte_le_nom_de_son_fichier() {
        assert_eq!(video().name, "concert");
    }

    #[test]
    fn son_temps_avance_a_sa_vitesse_reelle() {
        let mut c = video();
        avance(&mut c, 1.5);
        assert!((c.video_time().unwrap() - 1.5).abs() < 0.02);
    }

    #[test]
    fn en_boucle_son_temps_se_replie_sur_sa_duree() {
        let mut c = video();
        avance(&mut c, 5.0);
        assert!((c.video_time().unwrap() - 1.0).abs() < 0.02);
    }

    #[test]
    fn une_fois_son_temps_s_arrete_a_la_fin() {
        let mut c = video();
        assert!(c.set_video_playback(PlayMode::Once, 0.0));
        avance(&mut c, 5.0);
        assert_eq!(c.video_time(), Some(4.0));
    }

    #[test]
    fn calee_sur_le_tempo_sa_boucle_dure_ses_temps() {
        let mut c = video();
        // 2 temps à 120 BPM : les 4 s tiennent en 1 s.
        assert!(c.set_video_playback(PlayMode::Loop, 2.0));
        avance(&mut c, 0.25);
        assert!((c.video_time().unwrap() - 1.0).abs() < 0.05);
        assert_eq!(c.video_playback().unwrap().2, 2.0);
    }

    #[test]
    fn revenir_a_la_vitesse_reelle_efface_le_calage() {
        let mut c = video();
        c.set_video_playback(PlayMode::Loop, 2.0);
        c.set_video_playback(PlayMode::Loop, 0.0);
        assert_eq!(c.video_playback().unwrap().2, 0.0);
    }

    #[test]
    fn un_calage_absurde_est_refuse() {
        let mut c = video();
        assert!(!c.set_video_playback(PlayMode::Loop, -1.0));
        assert!(!c.set_video_playback(PlayMode::Loop, f32::NAN));
    }

    #[test]
    fn une_forme_n_a_pas_de_lecture_video() {
        let mut g = Graph::new();
        let transport = Transport::bpm_sync(&mut g, Param::fixed(4.0), PlayMode::Loop);
        let mut c = Clip::shape(g, Box::new(frogenx_shape::Ellipse::circle(0.5)), transport);
        assert_eq!(c.video_playback().map(|v| v.1), None);
        assert_eq!(c.video_time(), None);
        assert!(!c.set_video_playback(PlayMode::Loop, 0.0));
    }

    #[test]
    fn ses_reglages_passent_par_les_slots_du_generateur() {
        let mut c = video();
        let groupes = c.params();
        assert_eq!(groupes.len(), 1);
        assert_eq!(groupes[0].params.len(), 4);
        assert!(c.set_param(ParamSlot::Generator(0), 2.0));
        assert_eq!(c.params()[0].params[0].desc.value.base(), 2.0);
    }

    #[test]
    fn une_duree_nulle_ne_lit_rien() {
        let c = Clip::video(VideoParams::new("a.mp4", 0.0));
        assert_eq!(c.video_time(), None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::{PlayMode, Transport};
    use frogenx_core::{AudioFeatures, Ctx, NodeId, Param};
    use frogenx_shape::{Ellipse, Path, RadialRepeat};

    fn un_cercle() -> Clip {
        let mut g = Graph::new();
        let transport = Transport::bpm_sync(&mut g, Param::fixed(4.0), PlayMode::Loop);
        Clip::shape(g, Box::new(Ellipse::circle(0.5)), transport)
    }

    fn rayon(c: &Clip) -> ClipParam {
        c.params()[0].params[0]
    }

    #[test]
    fn une_feature_se_branche_sur_un_parametre_et_garde_sa_base() {
        let mut c = un_cercle();
        let slot = ParamSlot::Generator(0);
        let noeuds = c.graph().len();
        assert!(c.set_audio_modulation(slot, Some(FeatureKind::Bass)));

        let p = rayon(&c);
        assert_eq!(p.audio, Some(FeatureKind::Bass));
        assert_eq!(p.desc.value.base(), 0.5);
        assert_eq!(p.desc.value.depth(), Some(1.0), "une demi-plage");
        assert_eq!(c.graph().len(), noeuds + 1);

        c.graph_mut().set_audio(AudioFeatures {
            bands: [1.0, 0.0, 0.0],
            ..Default::default()
        });
        for _ in 0..60 {
            c.graph_mut().eval_frame(0.0, 1.0 / 60.0);
        }
        assert!(rayon(&c).desc.value.get(c.graph()) > 1.4);
    }

    #[test]
    fn un_trig_se_branche_sur_un_parametre_et_se_lit_dans_son_graphe() {
        let mut c = un_cercle();
        let slot = ParamSlot::Generator(0);
        let noeuds = c.graph().len();
        assert!(c.set_trig_modulation(slot, 3));
        assert_eq!(rayon(&c).trig, Some(3));
        assert_eq!(c.graph().len(), noeuds + 1);
        let trigs = [frogenx_core::TrigValues {
            gate: 0.0,
            pulse: 0.0,
            flash: 1.0,
        }; 4];
        let base = rayon(&c).desc.value.base();
        c.graph_mut().set_trigs(&trigs);
        c.graph_mut().eval_frame(0.0, 1.0 / 60.0);
        assert!(rayon(&c).desc.value.get(c.graph()) > base);
    }

    #[test]
    fn rebrancher_le_meme_trig_ne_change_rien() {
        let mut c = un_cercle();
        let slot = ParamSlot::Generator(0);
        c.set_trig_modulation(slot, 1);
        let noeuds = c.graph().len();
        assert!(c.set_trig_modulation(slot, 1));
        assert_eq!(c.graph().len(), noeuds);
    }

    #[test]
    fn un_trig_remplace_une_feature_et_inversement_sans_laisser_de_noeud() {
        let mut c = un_cercle();
        let slot = ParamSlot::Generator(0);
        c.set_audio_modulation(slot, Some(FeatureKind::Bass));
        c.set_depth(slot, 0.25);
        let noeuds = c.graph().len();
        c.set_trig_modulation(slot, 0);
        assert_eq!(rayon(&c).audio, None);
        assert_eq!(rayon(&c).desc.value.depth(), Some(0.25));
        assert_eq!(c.graph().len(), noeuds);
        c.set_audio_modulation(slot, Some(FeatureKind::Onset));
        assert_eq!(rayon(&c).trig, None);
        assert_eq!(c.graph().len(), noeuds);
    }

    #[test]
    fn sans_source_debranche_aussi_un_trig() {
        let mut c = un_cercle();
        let slot = ParamSlot::Generator(0);
        let noeuds = c.graph().len();
        c.set_trig_modulation(slot, 0);
        assert!(c.set_audio_modulation(slot, None));
        assert_eq!(rayon(&c).trig, None);
        assert_eq!(rayon(&c).desc.value.source(), None);
        assert_eq!(c.graph().len(), noeuds);
    }

    #[test]
    fn un_trig_sur_un_parametre_inconnu_est_refuse() {
        assert!(!un_cercle().set_trig_modulation(ParamSlot::Generator(99), 0));
    }

    #[test]
    fn changer_de_feature_garde_l_amplitude_et_ne_laisse_aucun_noeud() {
        let mut c = un_cercle();
        let slot = ParamSlot::Generator(0);
        c.set_audio_modulation(slot, Some(FeatureKind::Bass));
        c.set_depth(slot, 0.25);
        let noeuds = c.graph().len();
        c.set_audio_modulation(slot, Some(FeatureKind::Onset));
        assert_eq!(rayon(&c).audio, Some(FeatureKind::Onset));
        assert_eq!(rayon(&c).desc.value.depth(), Some(0.25));
        assert_eq!(c.graph().len(), noeuds);
    }

    #[test]
    fn debrancher_rend_le_parametre_fixe_a_sa_base() {
        let mut c = un_cercle();
        let slot = ParamSlot::Generator(0);
        let noeuds = c.graph().len();
        c.set_audio_modulation(slot, Some(FeatureKind::Rms));
        assert!(c.set_audio_modulation(slot, None));
        assert_eq!(rayon(&c).desc.value, Param::fixed(0.5));
        assert_eq!(rayon(&c).audio, None);
        assert_eq!(c.graph().len(), noeuds);
    }

    #[test]
    fn debrancher_ne_coupe_pas_un_lfo() {
        let mut c = un_cercle();
        let lfo = c.graph_mut().add(Box::new(frogenx_core::Constant(1.0)));
        *c.param_mut(ParamSlot::Generator(0)).unwrap() = Param::modulated(0.5, 0.2, lfo);
        assert!(c.set_audio_modulation(ParamSlot::Generator(0), None));
        assert_eq!(rayon(&c).desc.value.source(), Some(lfo));
    }

    #[test]
    fn une_feature_remplace_un_lfo() {
        let mut c = un_cercle();
        let lfo = c.graph_mut().add(Box::new(frogenx_core::Constant(1.0)));
        *c.param_mut(ParamSlot::Generator(0)).unwrap() = Param::modulated(0.5, 0.2, lfo);
        c.set_audio_modulation(ParamSlot::Generator(0), Some(FeatureKind::Treble));
        assert_eq!(rayon(&c).audio, Some(FeatureKind::Treble));
    }

    #[test]
    fn un_emplacement_inconnu_est_refuse() {
        let mut c = un_cercle();
        let slot = ParamSlot::Generator(99);
        assert!(!c.set_audio_modulation(slot, Some(FeatureKind::Rms)));
    }

    fn un_echo() -> Clip {
        let mut g = Graph::new();
        let transport = Transport::bpm_sync(&mut g, Param::fixed(4.0), PlayMode::Loop);
        Clip::feedback(g, FeedbackParams::default(), transport)
    }

    #[test]
    fn un_feedback_expose_ses_reglages_et_rien_d_autre() {
        let c = un_echo();
        let groupes = c.params();
        assert_eq!(groupes.len(), 1);
        assert_eq!(groupes[0].label, "Écho");
        assert_eq!(groupes[0].params.len(), 5);
        assert_eq!(c.name, "Écho");
    }

    #[test]
    fn un_feedback_se_regle_et_se_module_comme_une_forme() {
        let mut c = un_echo();
        let slot = ParamSlot::Generator(0);
        assert!(c.set_param(slot, 2.0));
        assert!(c.set_audio_modulation(slot, Some(FeatureKind::Onset)));
        let ClipSource::Feedback { params, .. } = &c.source else {
            panic!("un feedback");
        };
        assert_eq!(params.zoom.base(), 2.0);
        assert!(params.zoom.source().is_some());
        assert_eq!(c.params()[0].params[0].audio, Some(FeatureKind::Onset));
    }

    #[test]
    fn un_feedback_refuse_ce_qu_il_n_a_pas() {
        let mut c = un_echo();
        assert!(!c.set_param(ParamSlot::Fill(0), 0.5));
        assert!(!c.set_param(ParamSlot::Generator(99), 0.5));
    }

    #[test]
    fn un_style_n_a_pas_de_sens_sur_un_feedback() {
        let c = un_echo().with_style(ShapeStyle::default());
        assert!(matches!(c.source, ClipSource::Feedback { .. }));
    }

    fn dummy_position() -> NodeId {
        // Un Clip de test n'a pas besoin que `position` résolve réellement
        // dans ce graphe — seule la forme de Transport est exercée ici.
        let mut g = Graph::new();
        g.add(Box::new(PositionSignal::fixed_speed(1.0)))
    }

    #[test]
    fn un_clip_shape_sans_effet_a_une_chaine_vide() {
        let clip = Clip::shape(
            Graph::new(),
            Box::new(Ellipse::circle(0.5)),
            Transport::Timeline {
                position: dummy_position(),
                speed: Param::fixed(1.0),
            },
        );
        assert!(clip.effects.is_empty());
        assert!(matches!(clip.transition, ClipTransition::Cut));
    }

    #[test]
    fn la_source_shape_produit_une_forme_que_les_effets_transforment() {
        // Le manque corrigé au §3bis.2 : avec `Shape(Graph)`, le clip n'avait
        // aucun moyen de produire une forme, et ses effets rien à transformer.
        let clip = Clip::shape(
            Graph::new(),
            Box::new(Ellipse::circle(0.5)),
            Transport::Timeline {
                position: dummy_position(),
                speed: Param::fixed(1.0),
            },
        )
        .with_effect(Box::new(RadialRepeat::new(3.0)));

        let ClipSource::Shape {
            graph, generator, ..
        } = &clip.source
        else {
            panic!("un clip de forme");
        };
        let audio = AudioFeatures::default();
        let ctx = Ctx::new(1.0 / 60.0, 0, &audio);
        let mut path = Path::new();
        generator.generate(0.0, &ctx, graph, &mut path);
        for op in &clip.effects {
            op.apply(0.0, &ctx, graph, &mut path);
        }
        assert_eq!(path.contours.len(), 3, "le cercle répété trois fois");
        assert!(path.is_valid());
    }

    /// Un cercle répété, rouge et tracé.
    fn clip_complet() -> Clip {
        let rouge = frogenx_shape::ColorParam::srgb(1.0, 0.0, 0.0, 1.0);
        let transport = Transport::Timeline {
            position: dummy_position(),
            speed: Param::fixed(1.0),
        };
        Clip::shape(Graph::new(), Box::new(Ellipse::circle(0.5)), transport)
            .with_effect(Box::new(RadialRepeat::new(3.0)))
            .with_style(ShapeStyle {
                fill: Some(rouge),
                stroke: Some(frogenx_shape::StrokeStyle {
                    width: Param::fixed(0.01),
                    color: rouge,
                }),
            })
    }

    /// La valeur de base d'un paramètre, retrouvée par son emplacement.
    fn base(clip: &Clip, slot: ParamSlot) -> f32 {
        clip.params()
            .iter()
            .flat_map(|g| &g.params)
            .find(|p| p.slot == slot)
            .expect("emplacement listé")
            .desc
            .value
            .base()
    }

    #[test]
    fn un_clip_porte_le_nom_de_sa_forme_par_defaut() {
        let clip = clip_complet();
        assert_eq!(clip.name, "Ellipse");
        assert_eq!(clip.with_name("Lune").name, "Lune");
    }

    #[test]
    fn un_clip_groupe_ses_parametres_par_partie() {
        let labels: Vec<String> = clip_complet()
            .params()
            .into_iter()
            .map(|g| g.label)
            .collect();
        assert_eq!(
            labels,
            ["Ellipse", "Répétition radiale", "Remplissage", "Trait"]
        );
    }

    #[test]
    fn chaque_emplacement_liste_s_ecrit_et_se_relit() {
        let mut clip = clip_complet();
        let slots: Vec<ParamSlot> = clip
            .params()
            .iter()
            .flat_map(|g| g.params.iter().map(|p| p.slot))
            .collect();
        assert!(slots.len() > 10, "got {}", slots.len());
        for (i, slot) in slots.into_iter().enumerate() {
            let valeur = 0.25 + i as f32 * 0.01;
            assert!(clip.set_param(slot, valeur), "{slot:?}");
            assert_eq!(base(&clip, slot), valeur, "{slot:?}");
        }
    }

    #[test]
    fn un_reglage_garde_la_modulation() {
        let mut clip = clip_complet();
        let source = dummy_position();
        let ClipSource::Shape { style, .. } = &mut clip.source else {
            panic!("un clip de forme");
        };
        style.fill.as_mut().unwrap().r = Param::modulated(0.1, 0.5, source);

        assert!(clip.set_param(ParamSlot::Fill(0), 0.8));
        let ClipSource::Shape { style, .. } = &clip.source else {
            panic!("un clip de forme");
        };
        assert_eq!(
            style.fill.unwrap().r,
            Param::modulated(0.8, 0.5, source),
            "la base change, le câble reste"
        );
    }

    #[test]
    fn l_amplitude_se_regle_sur_un_parametre_module_seulement() {
        let mut clip = clip_complet();
        let source = dummy_position();
        let ClipSource::Shape { style, .. } = &mut clip.source else {
            panic!("un clip de forme");
        };
        style.fill.as_mut().unwrap().r = Param::modulated(0.1, 0.5, source);

        assert!(clip.set_depth(ParamSlot::Fill(0), 0.2));
        let ClipSource::Shape { style, .. } = &clip.source else {
            panic!("un clip de forme");
        };
        assert_eq!(style.fill.unwrap().r, Param::modulated(0.1, 0.2, source));

        assert!(!clip.set_depth(ParamSlot::Fill(1), 0.2), "pas de câble");
        assert!(!clip.set_depth(ParamSlot::Fill(0), f32::NAN));
        assert!(!clip.set_depth(ParamSlot::Generator(99), 0.2));
    }

    #[test]
    fn la_vitesse_est_saturee_a_la_lecture() {
        assert_eq!(sanitize_speed(2.0), 2.0);
        assert_eq!(sanitize_speed(-1.0), 0.0);
        assert_eq!(sanitize_speed(99.0), MAX_CLIP_SPEED);
        assert_eq!(sanitize_speed(f32::NAN), 1.0);
        assert_eq!(sanitize_speed(f32::INFINITY), 1.0);
    }

    #[test]
    fn un_emplacement_ou_une_valeur_invalide_est_refuse() {
        let mut clip = clip_complet();
        assert!(!clip.set_param(ParamSlot::Generator(99), 0.5));
        assert!(!clip.set_param(
            ParamSlot::Effect {
                effect: 9,
                param: 0
            },
            0.5
        ));
        assert!(!clip.set_param(ParamSlot::Fill(4), 0.5));
        assert!(!clip.set_param(ParamSlot::Stroke(5), 0.5));
        assert!(!clip.set_param(ParamSlot::Generator(0), f32::NAN));
    }

    #[test]
    fn un_clip_ne_dessine_rien_tant_qu_il_n_a_pas_de_style() {
        let transport = Transport::Timeline {
            position: dummy_position(),
            speed: Param::fixed(1.0),
        };
        let nu = Clip::shape(Graph::new(), Box::new(Ellipse::circle(0.5)), transport);
        let ClipSource::Shape { style, .. } = &nu.source else {
            panic!("un clip de forme");
        };
        assert_eq!(*style, ShapeStyle::default());

        let rouge = frogenx_shape::ColorParam::srgb(1.0, 0.0, 0.0, 1.0);
        let style_voulu = ShapeStyle::filled(rouge);
        let colore = nu.with_style(style_voulu);
        let ClipSource::Shape { style, .. } = &colore.source else {
            panic!("un clip de forme");
        };
        assert_eq!(*style, style_voulu);
    }

    #[test]
    fn with_transition_remplace_la_coupure_par_un_fondu() {
        let clip = Clip::shape(
            Graph::new(),
            Box::new(Ellipse::circle(0.5)),
            Transport::BpmSync {
                position: dummy_position(),
                duration_beats: Param::fixed(4.0),
                play_mode: PlayMode::Loop,
            },
        )
        .with_transition(ClipTransition::Fade {
            phase: PositionSignal::fixed_speed(1.0),
            curve: FadeCurve::SmoothStep,
        });
        assert!(matches!(clip.transition, ClipTransition::Fade { .. }));
    }
}
