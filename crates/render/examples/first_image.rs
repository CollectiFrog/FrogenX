//! La première image de FrogenX — le résultat visible de l'étape 5 (§12).
//!
//! Un fond et quatre `ShapeLayer` composées avec quatre modes de mélange, une
//! frame rendue en `Clock::Fixed` (déterministe, §20.3), relue et écrite en
//! PNG. La fenêtre de preview arrive à l'étape 6.
//!
//! ```bash
//! cargo run -p frogenx-render --example first_image [chemin.png]
//! ```

use frogenx_core::{AudioFeatures, Clock, Ctx, Graph, Param};
use frogenx_layer::{BlendMode, BlendParams, ColorParam, Gpu, ShapeLayer, LAYER_FORMAT};
use frogenx_osc::{Lfo, Waveform};
use frogenx_render::{read_rgba16f, write_png, Compositor, LayerInput};
use frogenx_shape::{
    Chain, Deform, Ellipse, Lissajous, NoiseDisplace, Polygon, RadialRepeat, Rect, Smooth, Star,
    Transform,
};
use glam::UVec2;
use std::path::PathBuf;

const SIZE: UVec2 = UVec2::new(1280, 720);
/// L'instant rendu : le LFO a eu le temps de déformer le blob.
const FRAMES: u64 = 80;

fn main() {
    let sortie = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/first_image.png"));

    let Some(gpu) = Gpu::headless() else {
        eprintln!("Aucun adaptateur GPU disponible : impossible de rendre l'image.");
        std::process::exit(1);
    };
    let info = gpu.adapter.get_info();
    println!(
        "GPU : {} ({:?}), MSAA {}×",
        info.name,
        info.backend,
        gpu.msaa_samples(LAYER_FORMAT)
    );

    // Le graphe de modulation : un LFO lent fait onduler le blob et tourner
    // la rosace — le même câblage que tout le reste du système (§2).
    let mut graph = Graph::new();
    let lfo = graph.add(Box::new(Lfo::new(Waveform::Sine, 0.25)));
    let rotation = Param::modulated(0.0, 0.6, lfo);

    // 1. Un blob organique : cercle déformé, bruité, lissé (§5.4).
    let blob = Chain::new(Box::new(Ellipse::circle(0.72)))
        .then(Box::new(
            Deform::new(0.08, 5.0).with_phase(Param::modulated(0.0, 1.0, lfo)),
        ))
        .then(Box::new(NoiseDisplace::new(0.06, 6.0, 7)))
        .then(Box::new(Smooth::new(2.0, 0.5)));

    // 2. Une rosace : un triangle décalé, répété huit fois — huit contours
    //    distincts, aucun segment parasite entre eux (§16.3).
    let rosace = Chain::new(Box::new(Polygon::new(3.0, 0.2)))
        .then(Box::new(Transform {
            translate_x: Param::fixed(0.46),
            ..Default::default()
        }))
        .then(Box::new(RadialRepeat::new(8.0)))
        .then(Box::new(Transform {
            rotation,
            ..Default::default()
        }));

    let mut couches = [
        // 0. Un fond opaque : sans lui, la sortie est transparente là où rien
        //    n'est dessiné, et les modes additifs n'ont rien sur quoi agir.
        //    Un rectangle de 4 couvre l'espace normalisé même en 16:9.
        ShapeLayer::new(&gpu, SIZE, Box::new(Rect::new(4.0, 4.0)))
            .with_fill(ColorParam::srgb(0.03, 0.03, 0.06, 1.0)),
        ShapeLayer::new(&gpu, SIZE, Box::new(blob))
            .with_fill(ColorParam::srgb(0.08, 0.22, 0.55, 1.0)),
        ShapeLayer::new(&gpu, SIZE, Box::new(rosace))
            .with_fill(ColorParam::srgb(1.0, 0.55, 0.15, 0.9))
            .with_blend(BlendParams {
                opacity: Param::fixed(1.0),
                mode: BlendMode::Screen,
            }),
        ShapeLayer::new(&gpu, SIZE, Box::new(Star::new(7.0, 0.95, 0.42)))
            .with_stroke(Param::fixed(0.012), ColorParam::srgb(0.95, 0.95, 1.0, 1.0))
            .with_blend(BlendParams {
                opacity: Param::fixed(0.8),
                mode: BlendMode::Add,
            }),
        ShapeLayer::new(&gpu, SIZE, Box::new(Lissajous::new(3.0, 2.0)))
            .with_stroke(Param::fixed(0.006), ColorParam::srgb(0.55, 1.0, 0.8, 1.0))
            .with_blend(BlendParams {
                opacity: Param::fixed(0.9),
                mode: BlendMode::Difference,
            }),
    ];

    // Temps fixe : la même commande produit la même image, à l'octet près.
    let mut clock = Clock::fixed(60.0);
    for _ in 0..FRAMES {
        clock.tick(0.0);
        graph.eval_frame(clock.t(), clock.dt());
    }

    let audio = AudioFeatures::default();
    let ctx = Ctx::new(clock.dt(), clock.frame(), &audio);
    let mut compositeur = Compositor::new(&gpu, SIZE);
    let mut entrees: Vec<LayerInput> = couches
        .iter_mut()
        .map(|c| LayerInput::new(c, &graph))
        .collect();
    compositeur.render(&gpu, clock.t(), &ctx, &mut entrees);

    let Some(pixels) = read_rgba16f(&gpu, compositeur.output_texture()) else {
        eprintln!("Relecture GPU impossible.");
        std::process::exit(1);
    };
    if let Err(e) = write_png(&sortie, SIZE.x, SIZE.y, &pixels) {
        eprintln!("Écriture de {} impossible : {e}", sortie.display());
        std::process::exit(1);
    }
    println!("Image écrite : {}", sortie.display());
}
