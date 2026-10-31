//! Une planche de tout le catalogue, pour le regarder — §23.6.
//!
//! Un test dit qu'un clip produit une forme valide, pas qu'elle est réussie.
//! Chaque clip est rendu seul sur le fond `Nuit`, après quelques battements,
//! dans l'ordre du catalogue.
//!
//! ```bash
//! cargo run --release -p frogenx-app --example catalogue -- [planche.png]
//! ```

use frogenx_app::Engine;
use frogenx_composition::{Command, Composition};
use frogenx_core::Clock;
use frogenx_layer::Gpu;
use frogenx_library::{catalog, find, preset};
use frogenx_output::{CpuFrame, CpuSink, OutputSink, SinkStatus};
use frogenx_render::write_png_rgba8;
use glam::UVec2;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

const CELL: UVec2 = UVec2::new(320, 180);
const COLUMNS: u32 = 6;
/// Le temps qu'un clip s'anime avant d'être photographié, en frames.
const FRAMES: u64 = 75;

/// Garde les frames dont l'indice clôt un clip.
#[derive(Clone, Default)]
struct Planche(Arc<Mutex<Vec<CpuFrame>>>);

impl CpuSink for Planche {
    fn submit(&mut self, frame: &CpuFrame) {
        if (frame.index + 1).is_multiple_of(FRAMES) {
            self.0.lock().unwrap().push(frame.clone());
        }
    }

    fn status(&self) -> SinkStatus {
        SinkStatus::Active
    }
}

fn main() {
    let sortie = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/catalogue.png"));
    let Some(gpu) = Gpu::headless() else {
        eprintln!("Aucun adaptateur GPU disponible : impossible de rendre la planche.");
        std::process::exit(1);
    };

    let mut grille = Composition::new();
    let deck = grille.add_deck("Catalogue");
    let groupe = grille
        .add_layer_group(deck)
        .expect("le deck vient d'être créé");
    let fond = grille
        .add_layer(groupe)
        .expect("le groupe vient d'être créé");
    let piste = grille
        .add_layer(groupe)
        .expect("le groupe vient d'être créé");
    let nuit = find("Nuit")
        .and_then(preset)
        .expect("`Nuit` est au catalogue");
    let fond_clip = grille
        .add_clip(fond, nuit.build())
        .expect("la piste existe");
    grille.trigger(fond, fond_clip);
    let clips: Vec<_> = catalog()
        .iter()
        .map(|p| grille.add_clip(piste, p.build()).expect("la piste existe"))
        .collect();

    let planche = Planche::default();
    let mut moteur = Engine::new(gpu, Clock::fixed(60.0), CELL, grille);
    if let Err(e) = moteur.add_output(OutputSink::Cpu(Box::new(planche.clone()))) {
        eprintln!("{e}");
        std::process::exit(1);
    }
    for clip in clips {
        moteur.push(Command::TriggerClip { layer: piste, clip });
        for _ in 0..FRAMES {
            moteur.frame(0.0);
        }
    }
    moteur.finish();

    let frames = planche.0.lock().unwrap();
    let lignes = (frames.len() as u32).div_ceil(COLUMNS);
    let (largeur, hauteur) = (CELL.x * COLUMNS, CELL.y * lignes);
    let mut octets = vec![0u8; 4 * (largeur * hauteur) as usize];
    for (i, frame) in frames.iter().enumerate() {
        let (cx, cy) = (i as u32 % COLUMNS * CELL.x, i as u32 / COLUMNS * CELL.y);
        for y in 0..CELL.y {
            let debut = (4 * y * CELL.x) as usize;
            let cible = 4 * ((cy + y) * largeur + cx) as usize;
            let n = 4 * CELL.x as usize;
            octets[cible..cible + n].copy_from_slice(&frame.pixels[debut..debut + n]);
        }
    }
    if let Err(e) = write_png_rgba8(&sortie, largeur, hauteur, &octets) {
        eprintln!("Écriture de {} impossible : {e}", sortie.display());
        std::process::exit(1);
    }
    let noms: Vec<_> = catalog().iter().map(|p| p.name).collect();
    println!("{} clips dans {}", frames.len(), sortie.display());
    for ligne in noms.chunks(COLUMNS as usize) {
        println!("{}", ligne.join(" | "));
    }
}
