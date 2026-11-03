//! Fichier → `Composition`.
//!
//! Tout se construit à côté de la grille qui joue. Un nœud s'ajoute après ses
//! dépendances : une référence en avant ne désigne encore rien, et se refuse.

use crate::encode;
use crate::file::*;
use crate::{PatchError, MAX_COLUMNS};
use frogenx_composition::{
    signal_from_spec, AudioAnalysis, Clip, ClipTransition, ColumnId, Composition, FadeCurve,
    LayerId, PlayMode, PositionSignal, Quantize, RestoredTrig, Transport, TrigTarget,
    WipeDirection,
};
use frogenx_core::{AmpScale, AnalysisMethod, FreqScale, Graph, NodeId, Param, Spec, SpecValue};
use frogenx_osc::{Divider, SpectrumZone};
use frogenx_shape::{
    generator_from_spec, op_from_spec, BlendMode, BlendParams, ColorParam, FeedbackParams,
    ImageFilter, ShapeStyle, StrokeStyle, VideoParams,
};
use std::collections::BTreeMap;

fn invalid(message: impl Into<String>) -> PatchError {
    PatchError::Invalid(message.into())
}

fn node(nodes: &[NodeId], position: u32) -> Result<NodeId, PatchError> {
    nodes
        .get(position as usize)
        .copied()
        .ok_or_else(|| invalid(format!("le nœud {position} n'existe pas à cet endroit")))
}

fn param(p: FileParam, nodes: &[NodeId]) -> Result<Param, PatchError> {
    Ok(match p {
        FileParam::Fixed(v) => Param::fixed(v),
        FileParam::Modulated {
            base,
            depth,
            source,
        } => Param::modulated(base, depth, node(nodes, source)?),
    })
}

fn value(v: &FileValue, nodes: &[NodeId]) -> Result<SpecValue, PatchError> {
    Ok(match v {
        FileValue::Param(p) => SpecValue::Param(param(*p, nodes)?),
        FileValue::Params(list) => SpecValue::Params(
            list.iter()
                .map(|p| param(*p, nodes))
                .collect::<Result<_, _>>()?,
        ),
        FileValue::Number(n) => SpecValue::Number(*n),
        FileValue::Int(n) => SpecValue::Int(*n),
        FileValue::Bool(b) => SpecValue::Bool(*b),
        FileValue::Word(w) => SpecValue::Word(w.clone()),
        FileValue::Specs(list) => SpecValue::Specs(
            list.iter()
                .map(|s| spec(&s.kind, &s.fields, nodes))
                .collect::<Result<_, _>>()?,
        ),
    })
}

fn spec(
    kind: &str,
    fields: &BTreeMap<String, FileValue>,
    nodes: &[NodeId],
) -> Result<Spec, PatchError> {
    Ok(Spec {
        kind: kind.to_string(),
        fields: fields
            .iter()
            .map(|(name, v)| Ok((name.clone(), value(v, nodes)?)))
            .collect::<Result<_, PatchError>>()?,
    })
}

/// Rend aussi la description de chaque nœud : un trig reconstruit ses copies de
/// référence depuis les siennes.
fn graph(file: &[FileNode], g: &mut Graph) -> Result<(Vec<NodeId>, Vec<Spec>), PatchError> {
    let mut nodes = Vec::with_capacity(file.len());
    let mut specs = Vec::with_capacity(file.len());
    for n in file {
        let description = spec(&n.kind, &n.fields, &nodes)?;
        let signal = signal_from_spec(&description)?;
        let deps = n
            .deps
            .iter()
            .map(|d| node(&nodes, *d))
            .collect::<Result<_, _>>()?;
        let id = g.add_with_deps(signal, deps)?;
        if let Some(place) = n.place {
            if !g.set_place(id, place) {
                return Err(invalid(format!("place {place:?}")));
            }
        }
        nodes.push(id);
        specs.push(description);
    }
    Ok((nodes, specs))
}

fn color(c: &FileColor, nodes: &[NodeId]) -> Result<ColorParam, PatchError> {
    Ok(ColorParam {
        r: param(c.r, nodes)?,
        g: param(c.g, nodes)?,
        b: param(c.b, nodes)?,
        a: param(c.a, nodes)?,
    })
}

fn style(s: &FileStyle, nodes: &[NodeId]) -> Result<ShapeStyle, PatchError> {
    Ok(ShapeStyle {
        fill: s.fill.as_ref().map(|c| color(c, nodes)).transpose()?,
        stroke: s
            .stroke
            .as_ref()
            .map(|t| {
                Ok::<_, PatchError>(StrokeStyle {
                    width: param(t.width, nodes)?,
                    color: color(&t.color, nodes)?,
                })
            })
            .transpose()?,
    })
}

/// Le mot de `word` parmi `choices`, écrits par la même fonction qu'à l'encodage.
fn word<T: Copy>(word: &str, choices: &[T], name: fn(T) -> &'static str) -> Result<T, PatchError> {
    choices
        .iter()
        .copied()
        .find(|c| name(*c) == word)
        .ok_or_else(|| invalid(format!("mot inconnu : « {word} »")))
}

fn play_mode(w: &str) -> Result<PlayMode, PatchError> {
    use PlayMode::*;
    word(w, &[Loop, Once, PingPong], encode::play_mode)
}

fn curve(w: &str) -> Result<FadeCurve, PatchError> {
    use FadeCurve::*;
    word(w, &[Linear, SmoothStep], encode::curve)
}

fn direction(w: &str) -> Result<WipeDirection, PatchError> {
    use WipeDirection::*;
    word(w, &[Left, Right, Up, Down], encode::direction)
}

fn blend_mode(w: &str) -> Result<BlendMode, PatchError> {
    use BlendMode::*;
    word(
        w,
        &[Normal, Add, Multiply, Screen, Difference],
        encode::blend_mode,
    )
}

fn analysis(a: &FileAnalysis, nodes: &[NodeId]) -> Result<AudioAnalysis, PatchError> {
    let unknown = |w: &str| invalid(format!("mot inconnu : « {w} »"));
    Ok(AudioAnalysis {
        method: AnalysisMethod::from_word(&a.method).ok_or_else(|| unknown(&a.method))?,
        freq_scale: FreqScale::from_word(&a.freq_scale).ok_or_else(|| unknown(&a.freq_scale))?,
        amp_scale: AmpScale::from_word(&a.amp_scale).ok_or_else(|| unknown(&a.amp_scale))?,
        window: param(a.window, nodes)?,
    })
}

fn transport(t: &FileTransport, nodes: &[NodeId]) -> Result<Transport, PatchError> {
    Ok(match t {
        FileTransport::Timeline { position, speed } => Transport::Timeline {
            position: node(nodes, *position)?,
            speed: param(*speed, nodes)?,
        },
        FileTransport::BpmSync {
            position,
            duration_beats,
            play_mode: mode,
        } => Transport::BpmSync {
            position: node(nodes, *position)?,
            duration_beats: param(*duration_beats, nodes)?,
            play_mode: play_mode(mode)?,
        },
    })
}

fn transition(t: &FileTransition, nodes: &[NodeId]) -> Result<ClipTransition, PatchError> {
    Ok(match t {
        FileTransition::Cut => ClipTransition::Cut,
        FileTransition::Fade { speed, curve: c } => ClipTransition::Fade {
            phase: PositionSignal::new(param(*speed, nodes)?),
            curve: curve(c)?,
        },
        FileTransition::Wipe {
            speed,
            curve: c,
            direction: d,
        } => ClipTransition::Wipe {
            phase: PositionSignal::new(param(*speed, nodes)?),
            curve: curve(c)?,
            direction: direction(d)?,
        },
    })
}

fn clip(file: &FileClip, composition: &[NodeId]) -> Result<Clip, PatchError> {
    let mut g = Graph::new();
    let (nodes, _) = graph(&file.graph, &mut g)?;
    let transport_ = transport(&file.transport, &nodes)?;
    let c = match (&file.feedback, &file.generator, &file.style) {
        (None, None, None) if file.video.is_some() => {
            let Some(v) = &file.video else {
                return Err(invalid("vidéo absente".to_string()));
            };
            let params = spec(&v.params.kind, &v.params.fields, &nodes)?;
            let params = VideoParams::from_spec(&params, v.path.clone(), v.duration)?;
            Clip::video_with(g, params, play_mode(&v.mode)?, transport_)
        }
        (Some(feedback), None, None) => {
            let params = spec(&feedback.kind, &feedback.fields, &nodes)?;
            Clip::feedback(g, FeedbackParams::from_spec(&params)?, transport_)
        }
        (None, Some(generator), Some(s)) => {
            let generator = spec(&generator.kind, &generator.fields, &nodes)?;
            Clip::shape(g, generator_from_spec(&generator)?, transport_)
                .with_style(style(s, &nodes)?)
        }
        _ => {
            let nom = &file.name;
            return Err(invalid(format!(
                "« {nom} » n'est ni une forme, ni un écho, ni une vidéo"
            )));
        }
    };
    let mut c = c
        .with_name(file.name.clone())
        .with_transition(transition(&file.transition, &nodes)?);
    c.speed = param(file.speed, composition)?;
    for effect in &file.effects {
        c = c.with_effect(op_from_spec(&spec(&effect.kind, &effect.fields, &nodes)?)?);
    }
    for filter in &file.filters {
        c = c.with_filter(ImageFilter::from_spec(&spec(
            &filter.kind,
            &filter.fields,
            &nodes,
        )?)?);
    }
    Ok(c)
}

/// Un trig s'appuie sur les nœuds déjà chargés : il doit désigner les bons types.
fn trig(
    t: &FileTrig,
    nodes: &[NodeId],
    specs: &[Spec],
    target: TrigTarget,
) -> Result<RestoredTrig, PatchError> {
    let description = |position: u32| {
        specs
            .get(position as usize)
            .ok_or_else(|| invalid(format!("le nœud {position} n'existe pas")))
    };
    Ok(RestoredTrig {
        name: t.name.clone(),
        target,
        channel: t.channel,
        zone: SpectrumZone::from_spec(description(t.gate)?)?,
        divider: Divider::from_spec(description(t.pulse)?)?,
        gate: node(nodes, t.gate)?,
        pulse: node(nodes, t.pulse)?,
        flash: node(nodes, t.flash)?,
    })
}

fn target(
    c: &Composition,
    t: &FileTarget,
    pistes: &[LayerId],
    colonnes: &[ColumnId],
) -> Result<TrigTarget, PatchError> {
    let colonne = |i: usize| {
        colonnes
            .get(i)
            .copied()
            .ok_or_else(|| invalid(format!("la colonne {i} n'existe pas")))
    };
    Ok(match t {
        FileTarget::None => TrigTarget::None,
        FileTarget::Column(i) => TrigTarget::Column(colonne(*i)?),
        FileTarget::Clip { layer, column } => {
            let id = *pistes
                .get(*layer)
                .ok_or_else(|| invalid(format!("la piste {layer} n'existe pas")))?;
            colonne(*column)?;
            let clip = c
                .layer(id)
                .and_then(|l| l.cell(*column))
                .ok_or_else(|| invalid(format!("la case {layer}, {column} est vide")))?;
            TrigTarget::Clip { layer: id, clip }
        }
    })
}

pub(crate) fn patch(file: FilePatch) -> Result<Composition, PatchError> {
    if file.columns > MAX_COLUMNS {
        return Err(invalid(format!("{} colonnes", file.columns)));
    }
    let mut c = Composition::new();
    let (nodes, specs) = graph(&file.graph, c.graph_mut())?;
    c.tempo.tempo = param(file.tempo, &nodes)?;
    c.tempo_follow = file.tempo_follow;
    c.quantize = Quantize::from_word(&file.quantize)
        .ok_or_else(|| invalid(format!("quantification {}", file.quantize)))?;
    c.analysis = analysis(&file.analysis, &nodes)?;
    let colonnes: Vec<_> = (0..file.columns).map(|_| c.add_column()).collect();

    let mut decks = Vec::with_capacity(file.decks.len());
    let mut pistes = Vec::new();
    let mut actifs = Vec::new();
    for fichier in &file.decks {
        let deck = c.add_deck(fichier.name.clone());
        decks.push(deck);
        for groupe in &fichier.groups {
            let g = c
                .add_layer_group(deck)
                .ok_or_else(|| invalid("deck introuvable"))?;
            for piste in &groupe.layers {
                let id = c
                    .add_layer(g)
                    .ok_or_else(|| invalid("groupe introuvable"))?;
                if piste.cells.len() > file.columns {
                    return Err(invalid(format!("« {} » dépasse la grille", piste.name)));
                }
                let l = c
                    .layer_mut(id)
                    .ok_or_else(|| invalid("piste introuvable"))?;
                l.name = piste.name.clone();
                l.blend = BlendParams {
                    opacity: param(piste.blend.opacity, &nodes)?,
                    mode: blend_mode(&piste.blend.mode)?,
                };
                for (i, case) in piste.cells.iter().enumerate() {
                    if let Some(fichier) = case {
                        let nouveau = clip(fichier, &nodes)?;
                        c.place_clip(id, colonnes[i], nouveau)
                            .ok_or_else(|| invalid("case introuvable"))?;
                    }
                }
                pistes.push(id);
                if let Some(active) = piste.active {
                    actifs.push((id, active));
                }
            }
        }
    }

    for t in &file.trigs {
        let cible = target(&c, &t.target, &pistes, &colonnes)?;
        c.restore_trig(trig(t, &nodes, &specs, cible)?);
    }
    for (id, colonne) in actifs {
        let actif = c.layer(id).and_then(|l| l.cell(colonne));
        let actif = actif.ok_or_else(|| invalid(format!("la case active {colonne} est vide")))?;
        c.trigger(id, actif);
    }
    if let Some(deck) = file.active_deck {
        let id = decks
            .get(deck)
            .ok_or_else(|| invalid(format!("le deck {deck} n'existe pas")))?;
        c.set_active_deck(*id);
    } else if let Some(premier) = c.active_deck() {
        // Le premier deck ajouté devient actif : le fichier dit qu'aucun ne l'est.
        c.remove_active_deck(premier);
    }
    Ok(c)
}
