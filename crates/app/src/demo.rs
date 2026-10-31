//! Une grille de démonstration, en attendant la bibliothèque de clips.
//!
//! **Provisoire** : un fond, puis deux pistes de quatre clips, dont quatre
//! colonnes déclenchent une paire. Tout est calé sur le tempo : à 120 BPM, les
//! LFO bouclent en 8 battements, soit 4 s.

use frogenx_composition::{BeatRate, Clip, ColumnId, Composition, LayerId, PlayMode, Transport};
use frogenx_core::{Graph, NodeId, Param};
use frogenx_layer::{BlendMode, BlendParams, ColorParam, ShapeStyle};
use frogenx_osc::{Lfo, Waveform};
use frogenx_shape::{
    Deform, Ellipse, Lissajous, NoiseDisplace, Polygon, RadialRepeat, Rect, Rose, ShapeGenerator,
    Smooth, Star, Superformula, Transform,
};

/// Durée d'un cycle de modulation, en battements.
const CYCLE: f32 = 8.0;

pub struct DemoGrid {
    pub composition: Composition,
    pub columns: Vec<ColumnId>,
}

/// Un LFO qui fait un cycle tous les `beats` battements.
fn lfo_on_beat(graph: &mut Graph, beats: f32) -> NodeId {
    let rate = graph.add(Box::new(BeatRate::new(beats)));
    let lfo = Lfo {
        waveform: Waveform::Sine,
        freq: Param::modulated(0.0, 1.0, rate),
        unipolar: false,
    };
    graph
        .add_with_deps(Box::new(lfo), vec![rate])
        .expect("`rate` vient d'être créé")
}

fn clip(graph: Graph, generator: Box<dyn ShapeGenerator>, style: ShapeStyle) -> Clip {
    let mut graph = graph;
    let transport = Transport::bpm_sync(&mut graph, Param::fixed(CYCLE), PlayMode::Loop);
    Clip::shape(graph, generator, transport).with_style(style)
}

fn blob() -> Clip {
    let mut g = Graph::new();
    let lfo = lfo_on_beat(&mut g, CYCLE);
    let style = ShapeStyle::filled(ColorParam::srgb(0.08, 0.22, 0.55, 1.0));
    clip(g, Box::new(Ellipse::circle(0.72)), style)
        .with_effect(Box::new(
            Deform::new(0.08, 5.0).with_phase(Param::modulated(0.0, 1.0, lfo)),
        ))
        .with_effect(Box::new(NoiseDisplace::new(0.06, 6.0, 7)))
        .with_effect(Box::new(Smooth::new(2.0, 0.5)))
}

fn rosace() -> Clip {
    let mut g = Graph::new();
    let lfo = lfo_on_beat(&mut g, CYCLE);
    let style = ShapeStyle::filled(ColorParam::srgb(1.0, 0.55, 0.15, 0.9));
    clip(g, Box::new(Polygon::new(3.0, 0.2)), style)
        .with_effect(Box::new(Transform {
            translate_x: Param::fixed(0.46),
            ..Default::default()
        }))
        .with_effect(Box::new(RadialRepeat::new(8.0)))
        .with_effect(Box::new(Transform {
            rotation: Param::modulated(0.0, 0.6, lfo),
            ..Default::default()
        }))
}

fn polygone_qui_respire() -> Clip {
    let mut g = Graph::new();
    let lfo = lfo_on_beat(&mut g, CYCLE);
    let forme = Polygon {
        sides: Param::modulated(6.0, 3.0, lfo),
        ..Polygon::new(6.0, 0.7)
    };
    let style = ShapeStyle::filled(ColorParam::srgb(0.75, 0.12, 0.4, 1.0));
    clip(g, Box::new(forme), style)
}

fn fleur() -> Clip {
    let mut g = Graph::new();
    let lfo = lfo_on_beat(&mut g, CYCLE);
    let style = ShapeStyle::filled(ColorParam::srgb(0.1, 0.6, 0.45, 1.0));
    clip(g, Box::new(Rose::new(5.0, 0.8)), style).with_effect(Box::new(Transform {
        rotation: Param::modulated(0.0, 1.2, lfo),
        ..Default::default()
    }))
}

fn trait_de(graph: Graph, generator: Box<dyn ShapeGenerator>, epaisseur: f32) -> Clip {
    let style = ShapeStyle::stroked(
        Param::fixed(epaisseur),
        ColorParam::srgb(0.9, 0.95, 1.0, 1.0),
    );
    clip(graph, generator, style)
}

fn etoile() -> Clip {
    trait_de(Graph::new(), Box::new(Star::new(7.0, 0.95, 0.42)), 0.012)
}

fn lissajous() -> Clip {
    trait_de(Graph::new(), Box::new(Lissajous::new(3.0, 2.0)), 0.008)
}

fn anneau() -> Clip {
    let mut g = Graph::new();
    let lfo = lfo_on_beat(&mut g, CYCLE / 2.0);
    trait_de(g, Box::new(Ellipse::circle(0.85)), 0.01).with_effect(Box::new(
        Deform::new(0.12, 3.0).with_phase(Param::modulated(0.0, 1.0, lfo)),
    ))
}

fn superformule() -> Clip {
    trait_de(Graph::new(), Box::new(Superformula::new(6.0)), 0.008)
}

/// Le fond, sur sa propre piste : sans lui, les modes additifs n'ont rien sur
/// quoi agir. Un rectangle de 4 couvre l'espace normalisé même en 16:9.
fn fond() -> Clip {
    let style = ShapeStyle::filled(ColorParam::srgb(0.03, 0.03, 0.06, 1.0));
    clip(Graph::new(), Box::new(Rect::new(4.0, 4.0)), style)
}

/// Ajoute une colonne qui déclenche `clips`, une entrée par piste de `pistes`.
fn colonne(c: &mut Composition, pistes: &[LayerId], clips: [Clip; 2]) -> ColumnId {
    let id = c.add_column();
    for (i, clip) in clips.into_iter().enumerate() {
        let piste = pistes[i];
        let ajoute = c.add_clip(piste, clip);
        // La piste 0 de la colonne est celle du fond : elle n'en fait pas partie.
        c.set_column_slot(id, i + 1, ajoute);
    }
    id
}

/// La grille complète. Le fond joue déjà ; aucune colonne n'est déclenchée.
pub fn grid() -> DemoGrid {
    let mut c = Composition::new();
    let deck = c.add_deck("Démo");
    let groupe = c.add_layer_group(deck).expect("le deck vient d'être créé");
    let piste = |c: &mut Composition| c.add_layer(groupe).expect("le groupe vient d'être créé");
    let (arriere, formes, traits) = (piste(&mut c), piste(&mut c), piste(&mut c));

    let clip_fond = c
        .add_clip(arriere, fond())
        .expect("la piste vient d'être créée");
    c.trigger(arriere, clip_fond);

    // L'opacité des traits respire : elle appartient à la piste, pas au clip,
    // et continue donc de respirer d'un clip à l'autre (§3bis.1).
    let respiration = c
        .graph_mut()
        .add(Box::new(Lfo::new(Waveform::Sine, 0.25).unipolar()));
    if let Some(l) = c.layer_mut(traits) {
        l.blend = BlendParams {
            opacity: Param::modulated(0.45, 0.55, respiration),
            mode: BlendMode::Add,
        };
    }

    let pistes = [formes, traits];
    let columns = vec![
        colonne(&mut c, &pistes, [blob(), etoile()]),
        colonne(&mut c, &pistes, [rosace(), lissajous()]),
        colonne(&mut c, &pistes, [polygone_qui_respire(), anneau()]),
        colonne(&mut c, &pistes, [fleur(), superformule()]),
    ];
    DemoGrid {
        composition: c,
        columns,
    }
}
