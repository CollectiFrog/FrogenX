//! Un flux Spout sans fenêtre, pour KyberFrog — §8.5 de `docs/ARCHITECTURE.md`.
//!
//! Publie la grille de départ sous un nom de sender, mesure ce que FrogenX
//! ajoute (du début de la frame à l'annonce Spout), et change de résolution en
//! cours de route : c'est ce qu'il faut vérifier côté récepteur, NVENC compris.
//!
//! ```bash
//! cargo run --release -p frogenx-app --example flux -- [nom] [secondes] [LxH] [fps] [rendu LxH]
//! ```

use frogenx_app::streams::open_spout;
use frogenx_app::Engine;
use frogenx_composition::Command;
use frogenx_core::Clock;
use frogenx_layer::Gpu;
use frogenx_output::{CpuFrame, CpuSink, SinkStatus, StreamFormat};
use glam::UVec2;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const SORTIE: Duration = Duration::from_nanos(16_666_667);

/// Le début de chaque frame, par indice, et les retards mesurés.
#[derive(Default)]
struct Mesures {
    debuts: BTreeMap<u64, Instant>,
    retards_ms: Vec<f64>,
}

/// Enveloppe une sortie, et note quand chaque frame en est sortie.
struct Mesuree {
    sortie: Box<dyn CpuSink>,
    mesures: Arc<Mutex<Mesures>>,
}

impl CpuSink for Mesuree {
    fn submit(&mut self, frame: &CpuFrame) {
        self.sortie.submit(frame);
        let fin = Instant::now();
        let mut m = self.mesures.lock().unwrap();
        if let Some(debut) = m.debuts.remove(&frame.index) {
            m.retards_ms.push((fin - debut).as_secs_f64() * 1000.0);
        }
        m.debuts.retain(|i, _| *i > frame.index);
    }

    fn status(&self) -> SinkStatus {
        self.sortie.status()
    }
}

fn taille(texte: Option<String>, repli: (u32, u32)) -> (u32, u32) {
    texte
        .and_then(|t| {
            let (w, h) = t.split_once('x')?;
            Some((w.parse().ok()?, h.parse().ok()?))
        })
        .unwrap_or(repli)
}

fn resume(nom: &str, retards: &mut [f64]) {
    if retards.is_empty() {
        println!("{nom} : aucune frame");
        return;
    }
    retards.sort_by(f64::total_cmp);
    let rang = |q: f64| retards[((retards.len() - 1) as f64 * q) as usize];
    println!(
        "{nom} : {} frames, médiane {:.1} ms, 99 sur 100 sous {:.1} ms, pire {:.1} ms",
        retards.len(),
        rang(0.5),
        rang(0.99),
        rang(1.0)
    );
}

fn main() {
    let mut args = std::env::args().skip(1);
    let nom = args.next().unwrap_or_else(|| "FrogenX essai".into());
    let secondes: u64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(30);
    let (width, height) = taille(args.next(), (1920, 1080));
    let fps: u32 = args.next().and_then(|f| f.parse().ok()).unwrap_or(60);
    let rendu = taille(args.next(), (1920, 1080));

    let Some(gpu) = Gpu::headless() else {
        eprintln!("Aucun adaptateur GPU disponible.");
        std::process::exit(1);
    };
    let info = gpu.adapter.get_info();
    println!("GPU : {} ({:?})", info.name, info.backend);

    let grille = frogenx_library::starter();
    let colonne = grille.columns().first().copied();
    let taille_rendu = UVec2::new(rendu.0, rendu.1);
    let mut moteur = Engine::new(gpu, Clock::realtime(), taille_rendu, grille);
    if let Some(column) = colonne {
        moteur.push(Command::TriggerColumn { column });
    }

    let format = StreamFormat { width, height, fps };
    let sortie = match open_spout(&info, &nom, format.within(rendu)) {
        Ok(sortie) => sortie,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    let mesures = Arc::new(Mutex::new(Mesures::default()));
    let flux = moteur.add_stream(
        Box::new(Mesuree {
            sortie,
            mesures: Arc::clone(&mesures),
        }),
        format,
    );
    println!("Sender « {nom} » publié pour {secondes} s : {width}×{height} à {fps} i/s.");

    let debut = Instant::now();
    let fin = debut + Duration::from_secs(secondes);
    // Le tiers médian de la durée passe à la moitié de la taille.
    let reduction = debut + Duration::from_secs(secondes / 3);
    let retour = debut + Duration::from_secs(2 * secondes / 3);
    let mut palier = 0;
    let (mut frame, mut echeance, mut dernier) = (0u64, debut, debut);
    while Instant::now() < fin {
        let maintenant = Instant::now();
        if maintenant < echeance {
            std::thread::sleep(echeance - maintenant);
        }
        let maintenant = Instant::now();
        if palier == 0 && maintenant >= reduction {
            palier = 1;
            moteur.set_stream_format(
                flux,
                StreamFormat {
                    width: width / 2,
                    height: height / 2,
                    fps,
                },
            );
            println!("→ {}×{}", width / 2, height / 2);
        } else if palier == 1 && maintenant >= retour {
            palier = 2;
            moteur.set_stream_format(flux, format);
            println!("→ {width}×{height}");
        }
        mesures.lock().unwrap().debuts.insert(frame, maintenant);
        moteur.frame((maintenant - dernier).as_secs_f64());
        dernier = maintenant;
        frame += 1;
        echeance += SORTIE;
    }
    println!("État : {:?}", moteur.stream_status(flux));
    moteur.remove_stream(flux);

    let mut m = mesures.lock().unwrap();
    println!("{frame} frames en {:.1} s.", debut.elapsed().as_secs_f64());
    resume("Début de frame → annonce Spout", &mut m.retards_ms);
}
