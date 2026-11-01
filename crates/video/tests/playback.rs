//! La lecture sur de vraies vidéos — §9.3 de `docs/ARCHITECTURE.md`.
//!
//! Chaque image porte son numéro dans sa luminosité : environ `31 + 3 × n`, à ±1 de
//! bruit après l'encodage. Une image servie dit donc d'elle-même si c'est la bonne. Sans ffmpeg dans le `PATH`, chaque
//! test s'arrête **avec succès** et le dit, comme les tests GPU (§24.3).

use frogenx_video::{probe, Frame, Loader, VideoPlayer, HEAD_SECONDS};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

const FPS: f64 = 15.0;
const SIZE: (u32, u32) = (64, 36);

/// La vidéo de test, fabriquée une fois : 4 s, une image intra par seconde.
fn sample() -> Option<&'static Path> {
    static CHEMIN: OnceLock<Option<PathBuf>> = OnceLock::new();
    let chemin = CHEMIN.get_or_init(|| {
        let f = std::env::temp_dir().join(format!("frogenx-video-{}.mp4", std::process::id()));
        let fait = Command::new("ffmpeg")
            .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi"])
            .args(["-i", "nullsrc=s=64x36:r=15:d=4,format=yuv420p"])
            .args(["-vf", "geq=lum='32+3*N':cb=128:cr=128,format=yuv420p"])
            .args([
                "-c:v",
                "libx264",
                "-qp",
                "4",
                "-g",
                "15",
                "-preset",
                "ultrafast",
            ])
            .arg(&f)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        fait.then_some(f)
    });
    if chemin.is_none() {
        eprintln!("ffmpeg absent, test sauté (§9.3)");
    }
    chemin.as_deref()
}

/// Le numéro de l'image, relu dans sa luminosité.
fn number(frame: &Frame) -> i32 {
    let centre = (frame.height / 2 * frame.width + frame.width / 2) as usize;
    ((frame.data[centre] as f32 - 31.0) / 3.0).round() as i32
}

fn player(path: &Path) -> VideoPlayer {
    let loader = Box::leak(Box::new(Loader::new()));
    let mut p = VideoPlayer::open(path, (1920, 1080), loader);
    p.frame_at_blocking(0.0);
    p
}

#[test]
fn la_vraie_description_se_lit() {
    let Some(f) = sample() else { return };
    let info = probe(f).unwrap();
    assert_eq!((info.width, info.height), SIZE);
    assert_eq!(info.fps, 15.0);
    assert!((info.duration - 4.0).abs() < 0.1, "{}", info.duration);
}

#[test]
fn ce_qui_n_est_pas_une_video_est_refuse_en_mots() {
    let Some(_) = sample() else { return };
    let faux = std::env::temp_dir().join(format!("frogenx-faux-{}.mp4", std::process::id()));
    std::fs::write(&faux, b"ceci n'est pas une video").unwrap();
    let erreur = probe(&faux).unwrap_err();
    let _ = std::fs::remove_file(&faux);
    assert!(erreur.contains("pas une vidéo"), "{erreur}");
}

#[test]
fn le_tampon_sert_la_bonne_image_sans_attendre() {
    let Some(f) = sample() else { return };
    let mut p = player(f);
    let debut = Instant::now();
    let image = p.frame_at(0.2).expect("la tête est prête");
    assert!(
        debut.elapsed() < Duration::from_millis(20),
        "{:?}",
        debut.elapsed()
    );
    assert_eq!(number(&image), 3);
}

#[test]
fn un_rendu_offline_lit_chaque_image_a_la_bonne_place() {
    let Some(f) = sample() else { return };
    let mut p = player(f);
    for n in (0..45).step_by(4) {
        let image = p.frame_at_blocking(n as f64 / FPS).unwrap();
        assert_eq!(number(&image), n, "t = {}", n as f64 / FPS);
    }
}

#[test]
fn un_saut_arrive_a_la_bonne_image() {
    let Some(f) = sample() else { return };
    let mut p = player(f);
    let image = p.frame_at_blocking(3.0).unwrap();
    assert_eq!(number(&image), 45);
    let image = p.frame_at_blocking(1.0).unwrap();
    assert_eq!(number(&image), 15, "en arrière aussi");
}

/// Le coeur du contrat : joué en temps réel, jamais bloquant, le tampon couvre
/// l'ouverture du flux et rien ne saute au raccord.
#[test]
fn en_temps_reel_le_flux_prend_le_relais_du_tampon() {
    let Some(f) = sample() else { return };
    let mut p = player(f);
    let debut = Instant::now();
    let mut pire_appel = Duration::ZERO;
    let mut retard_max = 0.0_f64;
    while debut.elapsed().as_secs_f64() < 2.0 {
        let t = debut.elapsed().as_secs_f64();
        let avant = Instant::now();
        let image = p.frame_at(t).expect("une image");
        pire_appel = pire_appel.max(avant.elapsed());
        if t > HEAD_SECONDS * 0.5 {
            retard_max = retard_max.max(t - number(&image) as f64 / FPS);
        }
        std::thread::sleep(Duration::from_millis(16));
    }
    assert!(pire_appel < Duration::from_millis(30), "{pire_appel:?}");
    assert!(retard_max < 0.25, "retard de {retard_max:.3} s au raccord");
}

#[test]
fn une_boucle_repart_du_debut_sans_attendre() {
    let Some(f) = sample() else { return };
    let mut p = player(f);
    let avance = p.frame_at_blocking(3.9).unwrap();
    assert!(number(&avance) >= 57, "{}", number(&avance));
    let debut = Instant::now();
    let image = p.frame_at(0.0).unwrap();
    assert!(debut.elapsed() < Duration::from_millis(20));
    assert_eq!(number(&image), 0);
}

#[test]
fn apres_la_fin_la_derniere_image_reste() {
    let Some(f) = sample() else { return };
    let mut p = player(f);
    let image = p.frame_at_blocking(3.95).unwrap();
    let plus_loin = p.frame_at_blocking(3.99).unwrap();
    assert_eq!(number(&image), number(&plus_loin));
    assert!(number(&image) >= 58, "{}", number(&image));
}

#[test]
fn un_fichier_absent_dit_pourquoi_et_ne_rend_rien() {
    let Some(_) = sample() else { return };
    let loader = Loader::new();
    let mut p = VideoPlayer::open(Path::new("n-existe-pas.mp4"), (640, 360), &loader);
    assert_eq!(p.frame_at_blocking(0.0), None);
    assert!(p.error().unwrap().contains("n-existe-pas.mp4"));
}

#[test]
fn une_grande_video_se_decode_a_la_taille_du_rendu() {
    let Some(f) = sample() else { return };
    let loader = Loader::new();
    let mut p = VideoPlayer::open(f, (32, 32), &loader);
    let image = p.frame_at_blocking(0.0).unwrap();
    assert_eq!((image.width, image.height), (32, 18));
    assert_eq!(image.data.len(), 32 * 18 * 3 / 2);
}

#[test]
fn en_pause_le_flux_s_arrete_et_la_tete_reste() {
    let Some(f) = sample() else { return };
    let mut p = player(f);
    p.frame_at_blocking(2.0);
    p.pause();
    let debut = Instant::now();
    let image = p.frame_at(0.1).unwrap();
    assert!(debut.elapsed() < Duration::from_millis(20));
    assert_eq!(number(&image), 1);
}
