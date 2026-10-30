//! Une frame complète, sortie factice — §24.1 (« Intégration ») de
//! `docs/ARCHITECTURE.md`.
//!
//! Sans adaptateur, chaque test s'arrête **avec succès** et le dit (§24.3).

use frogenx_app::{demo, Engine, SceneGroup};
use frogenx_core::{Clock, Graph, Param};
use frogenx_layer::{ColorParam, Gpu, ShapeLayer};
use frogenx_osc::{Lfo, Waveform};
use frogenx_output::{CpuFrame, CpuSink, OutputError, OutputSink, PngSequenceSink, SinkStatus};
use frogenx_shape::{Chain, Deform, Ellipse, Rect};
use glam::UVec2;
use std::sync::{Arc, Mutex};

const SIZE: UVec2 = UVec2::new(32, 32);

fn gpu() -> Option<Gpu> {
    let gpu = Gpu::headless();
    if gpu.is_none() {
        eprintln!("GPU absent, test sauté (§24.3)");
    }
    gpu
}

#[derive(Clone, Default)]
struct Recu(Arc<Mutex<Vec<CpuFrame>>>);

impl CpuSink for Recu {
    fn submit(&mut self, frame: &CpuFrame) {
        self.0.lock().unwrap().push(frame.clone());
    }

    fn status(&self) -> SinkStatus {
        SinkStatus::Active
    }
}

/// Un carré rouge opaque sur la moitié centrale, le reste transparent.
fn carre_rouge(gpu: &Gpu) -> Vec<SceneGroup> {
    vec![SceneGroup {
        graph: Graph::new(),
        layers: vec![Box::new(
            ShapeLayer::new(gpu, SIZE, Box::new(Rect::new(1.0, 1.0)))
                .with_fill(ColorParam::srgb(1.0, 0.0, 0.0, 1.0)),
        )],
    }]
}

/// Une forme animée par un LFO : deux frames successives diffèrent.
fn blob_anime(gpu: &Gpu) -> Vec<SceneGroup> {
    let mut graph = Graph::new();
    let lfo = graph.add(Box::new(Lfo::new(Waveform::Sine, 7.0)));
    let forme = Chain::new(Box::new(Ellipse::circle(0.6))).then(Box::new(
        Deform::new(0.3, 5.0).with_phase(Param::modulated(0.0, 1.0, lfo)),
    ));
    vec![SceneGroup {
        graph,
        layers: vec![Box::new(
            ShapeLayer::new(gpu, SIZE, Box::new(forme))
                .with_fill(ColorParam::srgb(0.2, 0.7, 1.0, 1.0)),
        )],
    }]
}

fn pixel(frame: &CpuFrame, x: u32, y: u32) -> [u8; 4] {
    let i = 4 * (y * frame.width + x) as usize;
    [
        frame.pixels[i],
        frame.pixels[i + 1],
        frame.pixels[i + 2],
        frame.pixels[i + 3],
    ]
}

#[test]
fn une_frame_traverse_tout_le_moteur_jusqu_a_une_sortie_cpu() {
    let Some(gpu) = gpu() else { return };
    let scene = carre_rouge(&gpu);
    let recu = Recu::default();
    let mut moteur = Engine::new(gpu, Clock::fixed(30.0), SIZE, scene);
    moteur
        .add_output(OutputSink::Cpu(Box::new(recu.clone())))
        .unwrap();

    for _ in 0..3 {
        // En `Fixed`, le temps mural est ignoré (§4).
        moteur.frame(99.0);
    }
    moteur.finish();

    let recu = recu.0.lock().unwrap();
    let indices: Vec<u64> = recu.iter().map(|f| f.index).collect();
    assert_eq!(indices, vec![0, 1, 2], "indices contigus en Fixed");
    for (i, frame) in recu.iter().enumerate() {
        let attendu = (i + 1) as f64 / 30.0;
        assert!((frame.t - attendu).abs() < 1e-9, "t : got {}", frame.t);
        assert_eq!((frame.width, frame.height), (SIZE.x, SIZE.y));
        assert_eq!(pixel(frame, 16, 16), [255, 0, 0, 255], "centre");
        assert_eq!(pixel(frame, 1, 1), [0, 0, 0, 0], "coin transparent");
    }
    assert!(
        moteur.diagnostics().iter().all(|d| d.is_clean()),
        "got {:?}",
        moteur.diagnostics()
    );
    assert_eq!(moteur.output_statuses(), vec![SinkStatus::Active]);
}

#[test]
fn le_rendu_fixe_est_reproductible_et_l_animation_bouge() {
    let Some(a) = gpu() else { return };
    let Some(b) = gpu() else { return };

    let rendre = |gpu: Gpu| {
        let scene = blob_anime(&gpu);
        let recu = Recu::default();
        let mut moteur = Engine::new(gpu, Clock::fixed(60.0), SIZE, scene);
        moteur
            .add_output(OutputSink::Cpu(Box::new(recu.clone())))
            .unwrap();
        for _ in 0..6 {
            moteur.frame(0.0);
        }
        moteur.finish();
        let frames = recu.0.lock().unwrap().clone();
        frames
    };
    let (premier, second) = (rendre(a), rendre(b));

    assert_eq!(premier.len(), 6);
    assert_eq!(second.len(), 6);
    for (x, y) in premier.iter().zip(&second) {
        assert_eq!(x.pixels, y.pixels, "frame {} différente", x.index);
    }
    assert_ne!(
        premier[0].pixels, premier[3].pixels,
        "l'animation est figée"
    );
}

#[test]
fn le_moteur_ecrit_une_sequence_png() {
    let Some(gpu) = gpu() else { return };
    let dossier = std::env::temp_dir().join(format!("frogenx-engine-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dossier);

    let scene = carre_rouge(&gpu);
    let mut moteur = Engine::new(gpu, Clock::fixed(60.0), SIZE, scene);
    moteur
        .add_output(OutputSink::Cpu(Box::new(
            PngSequenceSink::open(&dossier).unwrap(),
        )))
        .unwrap();
    for _ in 0..4 {
        moteur.frame(0.0);
    }
    // Sans ce `finish`, les dernières frames seraient encore en file.
    moteur.finish();

    let fichiers = std::fs::read_dir(&dossier).unwrap().count();
    let derniere = PngSequenceSink::path_for(&dossier, 3).exists();
    std::fs::remove_dir_all(&dossier).unwrap();
    assert_eq!(fichiers, 4);
    assert!(derniere);
}

#[test]
fn la_scene_de_demonstration_tourne_sans_diagnostic() {
    let Some(gpu) = gpu() else { return };
    let scene = demo::scene(&gpu, SIZE);
    let mut moteur = Engine::new(gpu, Clock::fixed(60.0), SIZE, vec![scene]);
    for _ in 0..3 {
        moteur.frame(0.0);
    }
    assert!(
        moteur.diagnostics().iter().all(|d| d.is_clean()),
        "got {:?}",
        moteur.diagnostics()
    );
}

#[test]
fn le_temps_reel_refuse_une_sortie_cpu() {
    let Some(gpu) = gpu() else { return };
    let scene = carre_rouge(&gpu);
    let mut moteur = Engine::new(gpu, Clock::realtime(), SIZE, scene);
    assert_eq!(
        moteur.add_output(OutputSink::Cpu(Box::new(Recu::default()))),
        Err(OutputError::CpuSinkRequiresFixedClock)
    );
    moteur.frame(0.016);
    assert!((moteur.clock().t() - 0.016).abs() < 1e-9);
}
