//! Une scène de démonstration, en attendant que la grille pilote le rendu.
//!
//! **Provisoire**, comme [`SceneGroup`] : l'étape 7 la remplacera par un clip
//! de `composition`. Son LFO à 0.25 Hz boucle en 4 s.

use crate::SceneGroup;
use frogenx_core::{Graph, Param};
use frogenx_layer::{BlendMode, BlendParams, ColorParam, Gpu, RenderLayer, ShapeLayer};
use frogenx_osc::{Lfo, Waveform};
use frogenx_shape::{
    Chain, Deform, Ellipse, Lissajous, NoiseDisplace, Polygon, RadialRepeat, Rect, Smooth, Star,
    Transform,
};
use glam::UVec2;

/// Un fond, un blob organique, une rosace, une étoile et une courbe de
/// Lissajous, composés avec quatre modes de mélange.
pub fn scene(gpu: &Gpu, size: UVec2) -> SceneGroup {
    let mut graph = Graph::new();
    let lfo = graph.add(Box::new(Lfo::new(Waveform::Sine, 0.25)));

    let blob = Chain::new(Box::new(Ellipse::circle(0.72)))
        .then(Box::new(
            Deform::new(0.08, 5.0).with_phase(Param::modulated(0.0, 1.0, lfo)),
        ))
        .then(Box::new(NoiseDisplace::new(0.06, 6.0, 7)))
        .then(Box::new(Smooth::new(2.0, 0.5)));

    let rosace = Chain::new(Box::new(Polygon::new(3.0, 0.2)))
        .then(Box::new(Transform {
            translate_x: Param::fixed(0.46),
            ..Default::default()
        }))
        .then(Box::new(RadialRepeat::new(8.0)))
        .then(Box::new(Transform {
            rotation: Param::modulated(0.0, 0.6, lfo),
            ..Default::default()
        }));

    let layers: Vec<Box<dyn RenderLayer>> = vec![
        // Un rectangle de 4 couvre l'espace normalisé même en 16:9.
        Box::new(
            ShapeLayer::new(gpu, size, Box::new(Rect::new(4.0, 4.0)))
                .with_fill(ColorParam::srgb(0.03, 0.03, 0.06, 1.0)),
        ),
        Box::new(
            ShapeLayer::new(gpu, size, Box::new(blob))
                .with_fill(ColorParam::srgb(0.08, 0.22, 0.55, 1.0)),
        ),
        Box::new(
            ShapeLayer::new(gpu, size, Box::new(rosace))
                .with_fill(ColorParam::srgb(1.0, 0.55, 0.15, 0.9))
                .with_blend(BlendParams {
                    opacity: Param::fixed(1.0),
                    mode: BlendMode::Screen,
                }),
        ),
        Box::new(
            ShapeLayer::new(gpu, size, Box::new(Star::new(7.0, 0.95, 0.42)))
                .with_stroke(Param::fixed(0.012), ColorParam::srgb(0.95, 0.95, 1.0, 1.0))
                .with_blend(BlendParams {
                    opacity: Param::fixed(0.8),
                    mode: BlendMode::Add,
                }),
        ),
        Box::new(
            ShapeLayer::new(gpu, size, Box::new(Lissajous::new(3.0, 2.0)))
                .with_stroke(Param::fixed(0.006), ColorParam::srgb(0.55, 1.0, 0.8, 1.0))
                .with_blend(BlendParams {
                    opacity: Param::fixed(0.9),
                    mode: BlendMode::Difference,
                }),
        ),
    ];
    SceneGroup { graph, layers }
}
