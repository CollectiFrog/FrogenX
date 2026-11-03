//! `Composition` → fichier.
//!
//! Le fichier ne connaît que des positions : chaque graphe s'écrit en ordre
//! topologique, et un `NodeId` devient sa place dans ce tableau.

use crate::file::*;
use crate::{PatchError, CURRENT_VERSION};
use frogenx_composition::{
    Clip, ClipSource, ClipTransition, Composition, FadeCurve, Layer, LayerId, PlayMode, Transport,
    TrigTarget, WipeDirection,
};
use frogenx_core::{Graph, NodeId, Param, Spec, SpecValue};
use frogenx_shape::{BlendMode, ColorParam, ShapeStyle};
use std::collections::BTreeMap;

fn finite(value: f32, what: &str) -> Result<f32, PatchError> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(PatchError::NotFinite(what.to_string()))
    }
}

/// L'ordre d'écriture d'un graphe : la place d'un nœud y sert de référence.
struct Positions(Vec<NodeId>);

impl Positions {
    fn of(&self, id: NodeId) -> Option<u32> {
        self.0.iter().position(|n| *n == id).map(|i| i as u32)
    }

    fn require(&self, id: NodeId, what: &str) -> Result<u32, PatchError> {
        self.of(id)
            .ok_or_else(|| PatchError::Invalid(format!("{what} désigne un nœud retiré")))
    }
}

/// Une source retirée se lit comme sa base (§18.2) : le fichier l'écrit ainsi.
fn param(p: Param, positions: &Positions) -> Result<FileParam, PatchError> {
    Ok(match p {
        Param::Fixed(v) => FileParam::Fixed(finite(v, "paramètre")?),
        Param::Modulated {
            base,
            depth,
            source,
        } => match positions.of(source) {
            Some(source) => FileParam::Modulated {
                base: finite(base, "base d'un paramètre")?,
                depth: finite(depth, "amplitude d'un paramètre")?,
                source,
            },
            None => FileParam::Fixed(finite(base, "base d'un paramètre")?),
        },
    })
}

fn value(v: &SpecValue, positions: &Positions) -> Result<FileValue, PatchError> {
    Ok(match v {
        SpecValue::Param(p) => FileValue::Param(param(*p, positions)?),
        SpecValue::Params(list) => FileValue::Params(
            list.iter()
                .map(|p| param(*p, positions))
                .collect::<Result<_, _>>()?,
        ),
        SpecValue::Number(n) => FileValue::Number(finite(*n, "nombre")?),
        SpecValue::Int(n) => FileValue::Int(*n),
        SpecValue::Bool(b) => FileValue::Bool(*b),
        SpecValue::Word(w) => FileValue::Word(w.clone()),
        SpecValue::Specs(list) => FileValue::Specs(
            list.iter()
                .map(|s| spec(s, positions))
                .collect::<Result<_, _>>()?,
        ),
    })
}

fn fields(s: &Spec, positions: &Positions) -> Result<BTreeMap<String, FileValue>, PatchError> {
    s.fields
        .iter()
        .map(|(name, v)| Ok((name.clone(), value(v, positions)?)))
        .collect()
}

fn spec(s: &Spec, positions: &Positions) -> Result<FileSpec, PatchError> {
    Ok(FileSpec {
        kind: s.kind.clone(),
        fields: fields(s, positions)?,
    })
}

fn graph(g: &Graph) -> Result<(Vec<FileNode>, Positions), PatchError> {
    let vues = g.nodes();
    let positions = Positions(vues.iter().map(|n| n.id).collect());
    let noeuds = vues
        .iter()
        .map(|n| {
            Ok(FileNode {
                kind: n.spec.kind.clone(),
                // Une dépendance vers un nœud retiré n'attend plus rien.
                deps: n.deps.iter().filter_map(|d| positions.of(*d)).collect(),
                fields: fields(&n.spec, &positions)?,
                place: n.place,
            })
        })
        .collect::<Result<_, PatchError>>()?;
    Ok((noeuds, positions))
}

fn color(c: &ColorParam, positions: &Positions) -> Result<FileColor, PatchError> {
    Ok(FileColor {
        r: param(c.r, positions)?,
        g: param(c.g, positions)?,
        b: param(c.b, positions)?,
        a: param(c.a, positions)?,
    })
}

fn style(s: &ShapeStyle, positions: &Positions) -> Result<FileStyle, PatchError> {
    Ok(FileStyle {
        fill: s.fill.as_ref().map(|c| color(c, positions)).transpose()?,
        stroke: s
            .stroke
            .as_ref()
            .map(|t| {
                Ok::<_, PatchError>(FileStroke {
                    width: param(t.width, positions)?,
                    color: color(&t.color, positions)?,
                })
            })
            .transpose()?,
    })
}

pub(crate) fn play_mode(mode: PlayMode) -> &'static str {
    match mode {
        PlayMode::Loop => "loop",
        PlayMode::Once => "once",
        PlayMode::PingPong => "ping_pong",
    }
}

pub(crate) fn curve(curve: FadeCurve) -> &'static str {
    match curve {
        FadeCurve::Linear => "linear",
        FadeCurve::SmoothStep => "smooth_step",
    }
}

pub(crate) fn direction(direction: WipeDirection) -> &'static str {
    match direction {
        WipeDirection::Left => "left",
        WipeDirection::Right => "right",
        WipeDirection::Up => "up",
        WipeDirection::Down => "down",
    }
}

pub(crate) fn blend_mode(mode: BlendMode) -> &'static str {
    match mode {
        BlendMode::Normal => "normal",
        BlendMode::Add => "add",
        BlendMode::Multiply => "multiply",
        BlendMode::Screen => "screen",
        BlendMode::Difference => "difference",
    }
}

fn transport(t: &Transport, positions: &Positions) -> Result<FileTransport, PatchError> {
    Ok(match t {
        Transport::Timeline { position, speed } => FileTransport::Timeline {
            position: positions.require(*position, "le transport")?,
            speed: param(*speed, positions)?,
        },
        Transport::BpmSync {
            position,
            duration_beats,
            play_mode: mode,
        } => FileTransport::BpmSync {
            position: positions.require(*position, "le transport")?,
            duration_beats: param(*duration_beats, positions)?,
            play_mode: play_mode(*mode).to_string(),
        },
    })
}

/// La phase d'une transition ne vit pas dans un graphe : `speed` n'y module rien.
fn transition(t: &ClipTransition, positions: &Positions) -> Result<FileTransition, PatchError> {
    Ok(match t {
        ClipTransition::Cut => FileTransition::Cut,
        ClipTransition::Fade { phase, curve: c } => FileTransition::Fade {
            speed: param(phase.speed, positions)?,
            curve: curve(*c).to_string(),
        },
        ClipTransition::Wipe {
            phase,
            curve: c,
            direction: d,
        } => FileTransition::Wipe {
            speed: param(phase.speed, positions)?,
            curve: curve(*c).to_string(),
            direction: direction(*d).to_string(),
        },
    })
}

fn clip(c: &Clip, composition: &Positions) -> Result<FileClip, PatchError> {
    let (noeuds, positions) = graph(c.graph())?;
    let video_ = match &c.source {
        ClipSource::Video { params, mode, .. } => Some(FileVideo {
            path: params.path.clone(),
            duration: params.duration,
            mode: play_mode(*mode).to_string(),
            params: spec(&params.spec(), &positions)?,
        }),
        _ => None,
    };
    let (generator, style_, feedback) = match &c.source {
        ClipSource::Shape {
            generator,
            style: s,
            ..
        } => (
            Some(spec(&generator.spec(), &positions)?),
            Some(style(s, &positions)?),
            None,
        ),
        ClipSource::Feedback { params, .. } => {
            (None, None, Some(spec(&params.spec(), &positions)?))
        }
        ClipSource::Video { .. } => (None, None, None),
    };
    Ok(FileClip {
        name: c.name.clone(),
        speed: param(c.speed, composition)?,
        graph: noeuds,
        generator,
        effects: c
            .effects
            .iter()
            .map(|e| spec(&e.spec(), &positions))
            .collect::<Result<_, _>>()?,
        filters: c
            .filters
            .iter()
            .map(|f| spec(&f.spec(), &positions))
            .collect::<Result<_, _>>()?,
        style: style_,
        feedback,
        video: video_,
        transport: transport(&c.transport, &positions)?,
        transition: transition(&c.transition, &positions)?,
    })
}

fn layer(
    c: &Composition,
    l: &Layer,
    columns: usize,
    positions: &Positions,
) -> Result<FileLayer, PatchError> {
    let cells = (0..columns)
        .map(|i| {
            l.cell(i)
                .and_then(|id| c.clip(id))
                .map(|clip_| clip(clip_, positions))
                .transpose()
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(FileLayer {
        name: l.name.clone(),
        blend: FileBlend {
            opacity: param(l.blend.opacity, positions)?,
            mode: blend_mode(l.blend.mode).to_string(),
        },
        active: l
            .active_clip()
            .and_then(|id| (0..columns).find(|i| l.cell(*i) == Some(id))),
        cells,
    })
}

/// Les pistes de toute la grille, dans l'ordre d'écriture : un trig désigne la
/// sienne par sa place dans cette liste.
fn all_layers(c: &Composition) -> Vec<LayerId> {
    let mut pistes = Vec::new();
    for deck in c.decks() {
        for groupe in c.deck(deck).map_or(&[][..], |d| d.groups()) {
            pistes.extend_from_slice(c.layer_group(*groupe).map_or(&[][..], |g| g.layers()));
        }
    }
    pistes
}

/// Une cible disparue ne déclenche rien : le fichier écrit `none`.
fn target(c: &Composition, t: TrigTarget, pistes: &[LayerId]) -> FileTarget {
    match t {
        TrigTarget::None => FileTarget::None,
        TrigTarget::Column(column) => c
            .column_index(column)
            .map_or(FileTarget::None, FileTarget::Column),
        TrigTarget::Clip { layer, clip } => {
            let piste = pistes.iter().position(|l| *l == layer);
            let colonne = c
                .layer(layer)
                .and_then(|l| (0..c.columns().len()).find(|i| l.cell(*i) == Some(clip)));
            match (piste, colonne) {
                (Some(layer), Some(column)) => FileTarget::Clip { layer, column },
                _ => FileTarget::None,
            }
        }
    }
}

pub(crate) fn patch(c: &Composition) -> Result<FilePatch, PatchError> {
    let (noeuds, positions) = graph(c.graph())?;
    let columns = c.columns().len();
    let pistes = all_layers(c);
    let trigs = c
        .trigs()
        .iter()
        .filter_map(|id| c.trig(*id))
        .map(|t| {
            Ok(FileTrig {
                name: t.name.clone(),
                target: target(c, t.target, &pistes),
                channel: t.channel(),
                gate: positions.require(t.gate(), "un trig")?,
                pulse: positions.require(t.pulse(), "un trig")?,
                flash: positions.require(t.flash(), "un trig")?,
            })
        })
        .collect::<Result<_, PatchError>>()?;

    let ids = c.decks();
    let mut decks = Vec::with_capacity(ids.len());
    for deck in &ids {
        let Some(d) = c.deck(*deck) else { continue };
        let mut groups = Vec::with_capacity(d.groups().len());
        for groupe in d.groups() {
            let Some(g) = c.layer_group(*groupe) else {
                continue;
            };
            let layers = g
                .layers()
                .iter()
                .filter_map(|id| c.layer(*id))
                .map(|l| layer(c, l, columns, &positions))
                .collect::<Result<_, _>>()?;
            groups.push(FileGroup { layers });
        }
        decks.push(FileDeck {
            name: d.name.clone(),
            groups,
        });
    }

    Ok(FilePatch {
        version: CURRENT_VERSION,
        meta: FileMeta {
            app: env!("CARGO_PKG_VERSION").to_string(),
        },
        tempo: param(c.tempo.tempo, &positions)?,
        tempo_follow: c.tempo_follow,
        quantize: c.quantize.word().to_string(),
        analysis: FileAnalysis {
            method: c.analysis.method.word().to_string(),
            freq_scale: c.analysis.freq_scale.word().to_string(),
            amp_scale: c.analysis.amp_scale.word().to_string(),
            window: param(c.analysis.window, &positions)?,
        },
        columns,
        active_deck: c
            .active_deck()
            .and_then(|active| ids.iter().position(|d| *d == active)),
        graph: noeuds,
        trigs,
        decks,
        state: None,
    })
}
