//! Une vraie prise : le vrai ffmpeg, sans GPU — §8.2 de `docs/ARCHITECTURE.md`.
//!
//! Sans ffmpeg dans le `PATH`, chaque test s'arrête **avec succès** et le dit, comme
//! les tests GPU (§24.3).

use frogenx_output::{
    assemble, audio_part, probe_encoders, video_part, CpuFrame, CpuSink, Encoders, FfmpegSink,
    PixelLayout, RecordPreset, SinkStatus,
};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

/// NVENC refuse moins de 145 pixels de large.
const SIZE: (u32, u32) = (256, 144);
const FPS: u32 = 30;

fn encoders() -> Option<Encoders> {
    let encodeurs = probe_encoders();
    if encodeurs.is_none() {
        eprintln!("ffmpeg absent, test sauté (§8.2)");
    }
    encodeurs
}

fn dossier(nom: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("frogenx-prise-{nom}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn frame(index: u64, t: f64) -> CpuFrame {
    let gris = (index * 8 % 255) as u8;
    let pixels: Vec<u8> = (0..SIZE.0 * SIZE.1)
        .flat_map(|_| [gris, gris, gris, 255])
        .collect();
    CpuFrame {
        pixels: Arc::from(pixels),
        layout: PixelLayout::BgraOpaque,
        width: SIZE.0,
        height: SIZE.1,
        index,
        t,
    }
}

/// Ce que ffmpeg dit d'un fichier. `ffprobe` n'accompagne pas toujours ffmpeg.
fn description(path: &Path) -> String {
    let sortie = Command::new("ffmpeg")
        .arg("-hide_banner")
        .arg("-i")
        .arg(path)
        .output()
        .unwrap();
    String::from_utf8_lossy(&sortie.stderr).into_owned()
}

/// Le nombre d'images du fichier, comptées en le relisant.
fn images(path: &Path) -> u64 {
    let sortie = Command::new("ffmpeg")
        .args(["-hide_banner", "-i"])
        .arg(path)
        .args(["-map", "0:v", "-c", "copy", "-f", "null", "-"])
        .output()
        .unwrap();
    let texte = String::from_utf8_lossy(&sortie.stderr).replace('\r', "\n");
    let ligne = texte.lines().rev().find(|l| l.contains("frame="));
    let apres = ligne.unwrap().split("frame=").nth(1).unwrap().trim_start();
    apres.split_whitespace().next().unwrap().parse().unwrap()
}

/// Une prise de `n` frames, à `pas` périodes l'une de l'autre.
fn prise(preset: RecordPreset, e: Encoders, path: &Path, n: u64, pas: u64) -> SinkStatus {
    let encodeur = preset.encoder(e);
    let (mut sink, suivi) =
        FfmpegSink::open(encodeur, SIZE, FPS, PixelLayout::BgraOpaque, path).unwrap();
    for i in 0..n {
        sink.submit(&frame(i, (i * pas) as f64 / FPS as f64));
    }
    let statut = sink.status();
    drop(sink);
    let fin = suivi.finished.recv_timeout(Duration::from_secs(30));
    fin.expect("ffmpeg se ferme").expect("ffmpeg réussit");
    assert!(suivi.first_frame.get().is_some());
    statut
}

fn wav(path: &Path, secondes: f32) {
    const SR: u32 = 48_000;
    let n = (secondes * SR as f32) as u32;
    let mut octets = Vec::new();
    octets.extend(b"RIFF");
    octets.extend((36 + n * 2).to_le_bytes());
    octets.extend(b"WAVEfmt ");
    octets.extend(16u32.to_le_bytes());
    octets.extend(1u16.to_le_bytes());
    octets.extend(1u16.to_le_bytes());
    octets.extend(SR.to_le_bytes());
    octets.extend((SR * 2).to_le_bytes());
    octets.extend(2u16.to_le_bytes());
    octets.extend(16u16.to_le_bytes());
    octets.extend(b"data");
    octets.extend((n * 2).to_le_bytes());
    octets.resize(44 + (n * 2) as usize, 0);
    std::fs::write(path, octets).unwrap();
}

#[test]
fn une_prise_h264_est_un_fichier_lisible_et_lisible_partout() {
    let Some(e) = encoders() else { return };
    let d = dossier("h264");
    let f = d.join("a.mp4");
    assert_eq!(prise(RecordPreset::H264, e, &f, 30, 1), SinkStatus::Active);
    let lu = description(&f);
    assert!(lu.contains("Video: h264"), "{lu}");
    assert!(lu.contains("256x144"), "{lu}");
    assert!(
        lu.contains("yuv420p"),
        "sans lui, beaucoup de lecteurs refusent : {lu}"
    );
    assert_eq!(images(&f), 30);
    assert!(!f.with_extension("ffmpeg.log").exists(), "journal retiré");
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn une_prise_prores_garde_ses_dix_bits() {
    let Some(e) = encoders() else { return };
    let d = dossier("prores");
    let f = d.join("a.mov");
    prise(RecordPreset::ProRes, e, &f, 10, 1);
    let lu = description(&f);
    assert!(lu.contains("Video: prores"), "{lu}");
    assert!(lu.contains("yuv422p10le"), "{lu}");
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn des_frames_perdues_ne_raccourcissent_pas_la_prise() {
    let Some(e) = encoders() else { return };
    let d = dossier("perte");
    let f = d.join("a.mp4");
    // Une frame sur trois arrive : le fichier doit quand même durer 1 s.
    let statut = prise(RecordPreset::H264, e, &f, 10, 3);
    assert!(
        matches!(statut, SinkStatus::Degraded { dropped: 18 }),
        "{statut:?}"
    );
    assert_eq!(images(&f), 28);
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn une_taille_qui_change_arrete_la_prise_sans_perdre_le_fichier() {
    let Some(e) = encoders() else { return };
    let d = dossier("taille");
    let f = d.join("a.mp4");
    let encodeur = RecordPreset::H264.encoder(e);
    let (mut sink, suivi) =
        FfmpegSink::open(encodeur, SIZE, FPS, PixelLayout::BgraOpaque, &f).unwrap();
    for i in 0..5 {
        sink.submit(&frame(i, i as f64 / FPS as f64));
    }
    let mut autre = frame(5, 5.0 / FPS as f64);
    autre.width = 32;
    sink.submit(&autre);
    assert!(matches!(sink.status(), SinkStatus::Failed(_)));
    drop(sink);
    suivi
        .finished
        .recv_timeout(Duration::from_secs(30))
        .unwrap()
        .unwrap();
    assert_eq!(images(&f), 5);
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn l_assemblage_ajoute_le_son_et_retire_les_parties() {
    let Some(e) = encoders() else { return };
    let d = dossier("assemblage");
    let f = d.join("FrogenX 001.mp4");
    let (video, audio) = (video_part(&f), audio_part(&f));
    prise(RecordPreset::H264, e, &video, 30, 1);
    wav(&audio, 2.0);
    assemble(RecordPreset::H264, &video, Some((&audio, -0.5)), &f).unwrap();
    let lu = description(&f);
    assert!(lu.contains("Audio: aac"), "{lu}");
    assert!(lu.contains("Video: h264"), "{lu}");
    assert!(!video.exists() && !audio.exists());
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn sans_son_l_image_prend_le_nom_final() {
    let Some(e) = encoders() else { return };
    let d = dossier("muet");
    let f = d.join("a.mp4");
    let video = video_part(&f);
    prise(RecordPreset::H264, e, &video, 5, 1);
    assemble(RecordPreset::H264, &video, None, &f).unwrap();
    assert!(f.exists() && !video.exists());
    let _ = std::fs::remove_dir_all(d);
}

#[test]
fn un_assemblage_rate_garde_les_deux_parties() {
    let Some(_) = encoders() else { return };
    let d = dossier("rate");
    let f = d.join("a.mp4");
    let (video, audio) = (video_part(&f), audio_part(&f));
    std::fs::write(&video, b"ceci n'est pas une video").unwrap();
    std::fs::write(&audio, b"ni un son").unwrap();
    let erreur = assemble(RecordPreset::H264, &video, Some((&audio, 0.0)), &f);
    assert!(erreur.is_err());
    assert!(video.exists() && audio.exists(), "la prise reste");
    let _ = std::fs::remove_dir_all(d);
}
