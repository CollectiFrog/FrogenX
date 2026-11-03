//! Le catalogue : 31 clips calés sur le tempo (§23.6).
//!
//! Un clip assemble un graphe de LFO, une forme, des effets et un style. Les
//! durées sont en battements, jamais en secondes : un clip suit le tempo de
//! la régie, et sa vitesse propre multiplie son `dt` (§3bis.2).

use crate::{Family, Preset, CYCLE};
use frogenx_composition::{BeatRate, Clip, PlayMode, Transport};
use frogenx_core::{Graph, NodeId, Param};
use frogenx_osc::{Lfo, Noise, Waveform};
use frogenx_shape::{
    Arc, ColorParam, Deform, Ellipse, FeedbackParams, GridRepeat, LinearWave, Lissajous, Mirror,
    NoiseDisplace, Polygon, Quantize, RadialRepeat, Rect, Rose, ShapeEchoes, ShapeGenerator,
    ShapeStyle, Smooth, Star, Superformula, Transform, Trim, Twist,
};
use std::f32::consts::PI;

pub(crate) static CATALOG: [Preset; 31] = [
    Preset::new("Nuit", Family::Backgrounds, nuit),
    Preset::new("Aurore", Family::Backgrounds, aurore),
    Preset::new("Braise", Family::Backgrounds, braise),
    Preset::new("Marée", Family::Backgrounds, maree),
    Preset::new("Blob", Family::Shapes, blob),
    Preset::new("Polygone", Family::Shapes, polygone),
    Preset::new("Fleur", Family::Shapes, fleur),
    Preset::new("Disque", Family::Shapes, disque),
    Preset::new("Cristal", Family::Shapes, cristal),
    Preset::new("Goutte", Family::Shapes, goutte),
    Preset::new("Galet", Family::Shapes, galet),
    Preset::new("Feuille", Family::Shapes, feuille),
    Preset::new("Étoile", Family::Lines, etoile),
    Preset::new("Lissajous", Family::Lines, lissajous),
    Preset::new("Anneau", Family::Lines, anneau),
    Preset::new("Superformule", Family::Lines, superformule),
    Preset::new("Arcs", Family::Lines, arcs),
    Preset::new("Spirographe", Family::Lines, spirographe),
    Preset::new("Tracé", Family::Lines, trace),
    Preset::new("Spirale", Family::Lines, spirale),
    Preset::new("Rosace", Family::Patterns, rosace),
    Preset::new("Mandala", Family::Patterns, mandala),
    Preset::new("Constellation", Family::Patterns, constellation),
    Preset::new("Engrenage", Family::Patterns, engrenage),
    Preset::new("Kaléidoscope", Family::Patterns, kaleidoscope),
    Preset::new("Pétales", Family::Patterns, petales),
    Preset::new("Miroir", Family::Patterns, miroir),
    Preset::new("Damier", Family::Patterns, damier),
    Preset::new("Traînée", Family::Echoes, trainee),
    Preset::new("Tunnel", Family::Echoes, tunnel),
    Preset::new("Tourbillon", Family::Echoes, tourbillon),
];

/// Un écho : à poser sur une piste sous les formes (§6.1).
fn echo(mut g: Graph, nom: &str, params: FeedbackParams) -> Clip {
    let transport = Transport::bpm_sync(&mut g, Param::fixed(CYCLE), PlayMode::Loop);
    Clip::feedback(g, params, transport).with_name(nom)
}

/// Les formes laissent une trace, plus ou moins longue selon le tempo.
fn trainee() -> Clip {
    let mut g = Graph::new();
    let souffle = sinus(&mut g, CYCLE);
    let params = FeedbackParams {
        memory: Param::modulated(0.5, 0.25, souffle),
        ..Default::default()
    };
    echo(g, "Traînée", params)
}

/// L'image s'enfonce vers le spectateur, et respire sur le tempo.
fn tunnel() -> Clip {
    let mut g = Graph::new();
    let souffle = sinus(&mut g, CYCLE);
    let params = FeedbackParams {
        zoom: Param::modulated(1.5, 0.25, souffle),
        memory: Param::fixed(1.0),
        ..Default::default()
    };
    echo(g, "Tunnel", params)
}

/// L'image tourne en s'enfonçant, et change de sens sur le tempo.
fn tourbillon() -> Clip {
    let mut g = Graph::new();
    let sens = sinus(&mut g, 2.0 * CYCLE);
    let params = FeedbackParams {
        zoom: Param::fixed(1.3),
        rotation: Param::modulated(0.0, 0.8, sens),
        memory: Param::fixed(1.5),
        ..Default::default()
    };
    echo(g, "Tourbillon", params)
}

/// Un LFO qui fait un cycle tous les `beats` battements.
fn lfo(graph: &mut Graph, waveform: Waveform, beats: f32) -> NodeId {
    let rate = graph.add(Box::new(BeatRate::new(beats)));
    let lfo = Lfo {
        waveform,
        freq: Param::modulated(0.0, 1.0, rate),
        unipolar: false,
    };
    graph
        .add_with_deps(Box::new(lfo), vec![rate])
        .expect("`rate` vient d'être créé")
}

fn sinus(graph: &mut Graph, beats: f32) -> NodeId {
    lfo(graph, Waveform::Sine, beats)
}

/// Un bruit lent, à graine explicite (§20.3), qui défile sur le tempo.
fn bruit(graph: &mut Graph, beats: f32, seed: u32) -> NodeId {
    let rate = graph.add(Box::new(BeatRate::new(beats)));
    let noise = Noise {
        rate: Param::modulated(0.0, 1.0, rate),
        ..Noise::new(0.0, seed)
    };
    graph
        .add_with_deps(Box::new(noise), vec![rate])
        .expect("`rate` vient d'être créé")
}

/// Une rotation continue, un tour tous les `beats` battements. La dent de scie
/// va de -π à π : pas de saut visible, ces deux angles sont le même.
fn tour(graph: &mut Graph, beats: f32) -> Transform {
    let scie = lfo(graph, Waveform::Saw, beats);
    Transform {
        rotation: Param::modulated(0.0, PI, scie),
        ..Default::default()
    }
}

fn clip(graph: Graph, generator: impl ShapeGenerator + 'static, style: ShapeStyle) -> Clip {
    let mut graph = graph;
    let transport = Transport::bpm_sync(&mut graph, Param::fixed(CYCLE), PlayMode::Loop);
    Clip::shape(graph, Box::new(generator), transport).with_style(style)
}

fn plein(r: f32, g: f32, b: f32) -> ShapeStyle {
    ShapeStyle::filled(ColorParam::srgb(r, g, b, 1.0))
}

fn voile(r: f32, g: f32, b: f32, alpha: f32) -> ShapeStyle {
    ShapeStyle::filled(ColorParam::srgb(r, g, b, alpha))
}

fn trait_blanc(epaisseur: f32) -> ShapeStyle {
    ShapeStyle::stroked(
        Param::fixed(epaisseur),
        ColorParam::srgb(0.9, 0.95, 1.0, 1.0),
    )
}

/// Un rectangle de 4 couvre l'espace normalisé même en 16:9.
fn fond(graph: Graph, couleur: ColorParam) -> Clip {
    clip(graph, Rect::new(4.0, 4.0), ShapeStyle::filled(couleur))
}

// Fonds

fn nuit() -> Clip {
    fond(Graph::new(), ColorParam::srgb(0.03, 0.03, 0.06, 1.0))
}

fn aurore() -> Clip {
    let mut g = Graph::new();
    let lent = sinus(&mut g, CYCLE * 2.0);
    let moyen = sinus(&mut g, CYCLE);
    let couleur = ColorParam {
        r: Param::modulated(0.08, 0.05, moyen),
        g: Param::modulated(0.16, 0.08, lent),
        b: Param::modulated(0.26, 0.08, moyen),
        ..ColorParam::srgb(0.0, 0.0, 0.0, 1.0)
    };
    fond(g, couleur)
}

fn braise() -> Clip {
    let mut g = Graph::new();
    let flamme = bruit(&mut g, 2.0, 11);
    let couleur = ColorParam {
        r: Param::modulated(0.26, 0.1, flamme),
        g: Param::modulated(0.07, 0.03, flamme),
        ..ColorParam::srgb(0.0, 0.0, 0.03, 1.0)
    };
    fond(g, couleur)
}

fn maree() -> Clip {
    let mut g = Graph::new();
    let houle = sinus(&mut g, CYCLE);
    let couleur = ColorParam {
        g: Param::modulated(0.17, 0.07, houle),
        b: Param::modulated(0.24, 0.08, houle),
        ..ColorParam::srgb(0.02, 0.0, 0.0, 1.0)
    };
    fond(g, couleur)
}

// Formes

fn blob() -> Clip {
    let mut g = Graph::new();
    let onde = sinus(&mut g, CYCLE);
    clip(g, Ellipse::circle(0.72), plein(0.08, 0.22, 0.55))
        .with_effect(Box::new(
            Deform::new(0.08, 5.0).with_phase(Param::modulated(0.0, 1.0, onde)),
        ))
        .with_effect(Box::new(NoiseDisplace::new(0.06, 6.0, 7)))
        .with_effect(Box::new(Smooth::new(2.0, 0.5)))
}

fn polygone() -> Clip {
    let mut g = Graph::new();
    let onde = sinus(&mut g, CYCLE);
    let forme = Polygon {
        sides: Param::modulated(6.0, 3.0, onde),
        ..Polygon::new(6.0, 0.7)
    };
    clip(g, forme, plein(0.75, 0.12, 0.4))
}

fn fleur() -> Clip {
    let mut g = Graph::new();
    let onde = sinus(&mut g, CYCLE);
    clip(g, Rose::new(5.0, 0.8), plein(0.1, 0.6, 0.45)).with_effect(Box::new(Transform {
        rotation: Param::modulated(0.0, 1.2, onde),
        ..Default::default()
    }))
}

fn disque() -> Clip {
    let mut g = Graph::new();
    let pouls = sinus(&mut g, 2.0);
    let forme = Ellipse::circle(0.55).with_radius(Param::modulated(0.55, 0.1, pouls));
    clip(g, forme, plein(0.95, 0.75, 0.2))
}

fn cristal() -> Clip {
    let mut g = Graph::new();
    let onde = sinus(&mut g, CYCLE / 2.0);
    let rotation = tour(&mut g, CYCLE * 2.0);
    let forme = Star::new(6.0, 0.8, 0.5).with_inner_ratio(Param::modulated(0.5, 0.18, onde));
    clip(g, forme, plein(0.35, 0.75, 0.95)).with_effect(Box::new(rotation))
}

fn goutte() -> Clip {
    let mut g = Graph::new();
    let onde = sinus(&mut g, CYCLE);
    let forme = Superformula {
        radius: Param::fixed(0.65),
        ..Superformula::new(3.0).with_n(
            Param::modulated(2.0, 0.8, onde),
            Param::fixed(4.0),
            Param::fixed(4.0),
        )
    };
    clip(g, forme, plein(0.55, 0.2, 0.8)).with_effect(Box::new(Smooth::new(1.0, 0.5)))
}

fn galet() -> Clip {
    let mut g = Graph::new();
    let onde = sinus(&mut g, CYCLE);
    let rotation = tour(&mut g, CYCLE * 2.0);
    let forme = Rect::new(1.0, 1.0).with_corner_radius(Param::modulated(0.3, 0.2, onde));
    clip(g, forme, plein(0.85, 0.35, 0.25)).with_effect(Box::new(rotation))
}

fn feuille() -> Clip {
    let mut g = Graph::new();
    let onde = sinus(&mut g, CYCLE);
    let rotation = Transform {
        rotation: Param::modulated(0.0, 0.8, onde),
        ..Default::default()
    };
    clip(g, Rose::new(2.0, 0.85), plein(0.3, 0.7, 0.2)).with_effect(Box::new(rotation))
}

// Traits

fn etoile() -> Clip {
    let mut g = Graph::new();
    let rotation = tour(&mut g, CYCLE * 2.0);
    clip(g, Star::new(7.0, 0.95, 0.42), trait_blanc(0.012)).with_effect(Box::new(rotation))
}

fn lissajous() -> Clip {
    let mut g = Graph::new();
    let scie = lfo(&mut g, Waveform::Saw, CYCLE);
    // Une phase en tours : de -0,5 à 0,5, soit un tour complet sans saut.
    let forme = Lissajous::new(3.0, 2.0).with_phase(Param::modulated(0.0, 0.5, scie));
    clip(g, forme, trait_blanc(0.008))
}

fn anneau() -> Clip {
    let mut g = Graph::new();
    let onde = sinus(&mut g, CYCLE / 2.0);
    clip(g, Ellipse::circle(0.85), trait_blanc(0.01)).with_effect(Box::new(
        Deform::new(0.12, 3.0).with_phase(Param::modulated(0.0, 1.0, onde)),
    ))
}

fn superformule() -> Clip {
    let mut g = Graph::new();
    let onde = sinus(&mut g, CYCLE);
    let forme = Superformula::new(6.0).with_n(
        Param::modulated(1.0, 0.4, onde),
        Param::fixed(1.0),
        Param::fixed(1.0),
    );
    clip(g, forme, trait_blanc(0.008))
}

fn arcs() -> Clip {
    let mut g = Graph::new();
    let onde = sinus(&mut g, CYCLE / 2.0);
    let rotation = tour(&mut g, CYCLE);
    let forme = Arc::new(0.0, 2.0, 0.8).with_sweep(Param::modulated(2.5, 1.5, onde));
    clip(g, forme, trait_blanc(0.016))
        .with_effect(Box::new(RadialRepeat::new(3.0)))
        .with_effect(Box::new(rotation))
}

fn spirographe() -> Clip {
    let mut g = Graph::new();
    let onde = sinus(&mut g, CYCLE * 2.0);
    let forme = Rose::new(7.0, 0.9).with_k(Param::modulated(7.0, 2.0, onde));
    clip(g, forme, trait_blanc(0.006))
}

// Motifs

/// Décale la forme vers la droite, avant qu'une répétition radiale l'emporte.
fn decale(x: f32) -> Transform {
    Transform {
        translate_x: Param::fixed(x),
        ..Default::default()
    }
}

fn rosace() -> Clip {
    let mut g = Graph::new();
    let onde = sinus(&mut g, CYCLE);
    clip(g, Polygon::new(3.0, 0.2), voile(1.0, 0.55, 0.15, 0.9))
        .with_effect(Box::new(decale(0.46)))
        .with_effect(Box::new(RadialRepeat::new(8.0)))
        .with_effect(Box::new(Transform {
            rotation: Param::modulated(0.0, 0.6, onde),
            ..Default::default()
        }))
}

fn mandala() -> Clip {
    let mut g = Graph::new();
    let onde = sinus(&mut g, CYCLE / 2.0);
    let rotation = tour(&mut g, CYCLE * 2.0);
    let forme = Star::new(5.0, 0.2, 0.45).with_inner_ratio(Param::modulated(0.45, 0.2, onde));
    clip(g, forme, plein(0.9, 0.3, 0.6))
        .with_effect(Box::new(decale(0.6)))
        .with_effect(Box::new(RadialRepeat::new(12.0)))
        .with_effect(Box::new(rotation))
}

fn constellation() -> Clip {
    let mut g = Graph::new();
    let pouls = sinus(&mut g, 2.0);
    let rotation = tour(&mut g, CYCLE * 4.0);
    let forme = Ellipse::circle(0.04).with_radius(Param::modulated(0.04, 0.02, pouls));
    clip(g, forme, plein(0.9, 0.95, 1.0))
        .with_effect(Box::new(decale(0.75)))
        .with_effect(Box::new(RadialRepeat::new(24.0)))
        .with_effect(Box::new(rotation))
}

fn engrenage() -> Clip {
    let mut g = Graph::new();
    let rotation = tour(&mut g, CYCLE);
    clip(g, Rect::new(0.24, 0.12), plein(0.7, 0.72, 0.78))
        .with_effect(Box::new(decale(0.62)))
        .with_effect(Box::new(RadialRepeat::new(14.0)))
        .with_effect(Box::new(rotation))
}

fn kaleidoscope() -> Clip {
    let mut g = Graph::new();
    let flot = bruit(&mut g, 4.0, 23);
    let rotation = tour(&mut g, CYCLE * 2.0);
    clip(g, Polygon::new(4.0, 0.3), plein(0.2, 0.8, 0.75))
        .with_effect(Box::new(
            NoiseDisplace::new(0.1, 3.0, 23).with_offset(Param::modulated(0.0, 2.0, flot)),
        ))
        .with_effect(Box::new(decale(0.4)))
        .with_effect(Box::new(RadialRepeat::new(6.0)))
        .with_effect(Box::new(rotation))
}

fn petales() -> Clip {
    let mut g = Graph::new();
    let pouls = sinus(&mut g, CYCLE / 2.0);
    let forme = Ellipse::circle(0.34).with_eccentricity(Param::fixed(0.35));
    clip(g, forme, voile(0.95, 0.5, 0.65, 0.8))
        .with_effect(Box::new(decale(0.42)))
        .with_effect(Box::new(RadialRepeat::new(10.0)))
        .with_effect(Box::new(Transform {
            scale: Param::modulated(0.9, 0.12, pouls),
            ..Default::default()
        }))
}

/// Une dent de scie de 0 à 1, sur `beats` battements : le LFO va de -1 à 1.
fn rampe(graph: &mut Graph, beats: f32) -> Param {
    Param::modulated(0.5, 0.5, lfo(graph, Waveform::Saw, beats))
}

/// Le tracé se dessine sur un cycle, puis repart : la `fin` d'une découpe suit
/// une rampe, et la fenêtre tourne doucement. Elle part de 15 % : à 0, le clip
/// serait vide à son lancement.
fn trace() -> Clip {
    let mut g = Graph::new();
    let scie = lfo(&mut g, Waveform::Saw, CYCLE);
    let derive = rampe(&mut g, CYCLE * 4.0);
    let mut decoupe = Trim::new(0.0, 1.0);
    decoupe.end = Param::modulated(0.575, 0.425, scie);
    decoupe.offset = derive;
    clip(g, Rose::new(5.0, 0.85), trait_blanc(0.01)).with_effect(Box::new(decoupe))
}

/// Des étoiles emboîtées qui tournent, traversées par une onde qui défile.
fn spirale() -> Clip {
    let mut g = Graph::new();
    let onde = sinus(&mut g, CYCLE);
    let defile = rampe(&mut g, CYCLE / 2.0);
    let mut echos = ShapeEchoes::new(8.0, 0.82, 0.25);
    echos.rotation = Param::modulated(0.25, 0.15, onde);
    let mut vague = LinearWave::new(0.04, 0.5);
    vague.phase = defile;
    clip(g, Star::new(7.0, 0.9, 0.5), trait_blanc(0.008))
        .with_effect(Box::new(echos))
        .with_effect(Box::new(vague))
}

/// Une étoile tordue, mise en miroir quatre fois : la torsion respire sur le tempo.
fn miroir() -> Clip {
    let mut g = Graph::new();
    let souffle = sinus(&mut g, CYCLE);
    let rotation = tour(&mut g, CYCLE * 4.0);
    let mut torsion = Twist::new(0.0, 0.8);
    torsion.strength = Param::modulated(0.0, 1.6, souffle);
    clip(g, Star::new(5.0, 0.22, 0.5), plein(0.35, 0.6, 0.95))
        .with_effect(Box::new(decale(0.42)))
        .with_effect(Box::new(torsion))
        .with_effect(Box::new(Mirror::new(3.0, 0.0)))
        .with_effect(Box::new(rotation))
}

/// Des disques en quinconce qui respire, accrochés à une grille qui pulse.
fn damier() -> Clip {
    let mut g = Graph::new();
    let quinconce = sinus(&mut g, CYCLE);
    let pouls = sinus(&mut g, 2.0);
    let mut grille = GridRepeat::new(7.0, 4.0, 0.32);
    grille.offset = Param::modulated(0.5, 0.5, quinconce);
    let mut pixel = Quantize::new(0.0);
    pixel.step = Param::modulated(0.035, 0.025, pouls);
    clip(g, Ellipse::circle(0.11), plein(0.95, 0.75, 0.2))
        .with_effect(Box::new(grille))
        .with_effect(Box::new(pixel))
}
