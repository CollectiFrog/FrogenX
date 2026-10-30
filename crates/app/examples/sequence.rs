//! Une séquence PNG déterministe — le premier clip de FrogenX (§8.2, §12).
//!
//! La scène de `demo` : son LFO à 0.25 Hz boucle en 4 s, soit 240 frames à
//! 60 fps — d'où la valeur par défaut, qui raccorde sans saut.
//!
//! ```bash
//! cargo run --release -p frogenx-app --example sequence -- [dossier] [frames]
//! ffmpeg -framerate 60 -i target/sequence/frame_%06d.png \
//!        -c:v libx264 -pix_fmt yuv420p -crf 15 target/sequence.mp4
//! ```

use frogenx_app::{demo, Engine};
use frogenx_core::Clock;
use frogenx_layer::Gpu;
use frogenx_output::{OutputSink, PngSequenceSink};
use glam::UVec2;
use std::path::PathBuf;
use std::time::Instant;

const SIZE: UVec2 = UVec2::new(1280, 720);
const FPS: f64 = 60.0;
/// Une période du LFO : la séquence boucle sans saut.
const FRAMES: u64 = 240;

fn main() {
    let mut args = std::env::args().skip(1);
    let dossier = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("target/sequence"));
    let frames = args.next().and_then(|f| f.parse().ok()).unwrap_or(FRAMES);

    let Some(gpu) = Gpu::headless() else {
        eprintln!("Aucun adaptateur GPU disponible : impossible de rendre la séquence.");
        std::process::exit(1);
    };
    let sortie = match PngSequenceSink::open(&dossier) {
        Ok(sortie) => sortie,
        Err(e) => {
            eprintln!("Ouverture de {} impossible : {e}", dossier.display());
            std::process::exit(1);
        }
    };

    let groupe = demo::scene(&gpu, SIZE);
    let mut moteur = Engine::new(gpu, Clock::fixed(FPS), SIZE, vec![groupe]);
    if let Err(e) = moteur.add_output(OutputSink::Cpu(Box::new(sortie))) {
        eprintln!("{e}");
        std::process::exit(1);
    }

    let debut = Instant::now();
    for _ in 0..frames {
        moteur.frame(0.0);
    }
    moteur.finish();

    let duree = debut.elapsed().as_secs_f64();
    println!(
        "{frames} frames dans {} en {duree:.1} s ({:.1} fps) — sortie : {:?}",
        dossier.display(),
        frames as f64 / duree,
        moteur.output_statuses()
    );
}
