//! Ce qu'on sait d'une vidéo avant de la lire — §9.3 de `docs/ARCHITECTURE.md`.
//!
//! Lu dans ce que ffmpeg écrit sur son fichier : `ffprobe` ne l'accompagne pas
//! toujours.

use std::path::Path;
use std::process::{Command, Stdio};

/// Le binaire, cherché dans le `PATH` : il n'est pas livré avec FrogenX.
pub(crate) const FFMPEG: &str = "ffmpeg";

/// Une vidéo, telle que ffmpeg la décrit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VideoInfo {
    pub width: u32,
    pub height: u32,
    pub fps: f32,
    /// En secondes.
    pub duration: f64,
}

/// `HH:MM:SS.cc`, en secondes.
fn parse_clock(texte: &str) -> Option<f64> {
    let mut parties = texte.trim().split(':');
    let h: f64 = parties.next()?.parse().ok()?;
    let m: f64 = parties.next()?.parse().ok()?;
    let s: f64 = parties.next()?.parse().ok()?;
    Some(h * 3600.0 + m * 60.0 + s)
}

/// `1280x720`, le premier mot d'un morceau de ligne.
fn parse_size(morceau: &str) -> Option<(u32, u32)> {
    let mot = morceau.split_whitespace().next()?;
    let (w, h) = mot.split_once('x')?;
    Some((w.parse().ok()?, h.parse().ok()?))
}

/// La cadence, dite `30 fps` ou à défaut `30 tbr`.
fn parse_rate(morceaux: &[&str]) -> Option<f32> {
    let cherche = |unite: &str| {
        morceaux.iter().find_map(|m| {
            let m = m.split('(').next()?.trim();
            m.strip_suffix(unite)?.trim().parse::<f32>().ok()
        })
    };
    cherche(" fps").or_else(|| cherche(" tbr"))
}

/// Lit ce que `ffmpeg -i` écrit. `None` : ce n'est pas une vidéo qui a une durée.
pub fn parse_probe(texte: &str) -> Option<VideoInfo> {
    let duree = texte.lines().find_map(|l| {
        let apres = l.trim().strip_prefix("Duration:")?;
        parse_clock(apres.split(',').next()?)
    })?;
    let ligne = texte.lines().find(|l| l.contains(": Video:"))?;
    let morceaux: Vec<&str> = ligne.split(", ").collect();
    let (width, height) = morceaux.iter().find_map(|m| parse_size(m))?;
    let fps = parse_rate(&morceaux)?;
    (duree > 0.0 && fps > 0.0 && width > 0 && height > 0).then_some(VideoInfo {
        width,
        height,
        fps,
        duration: duree,
    })
}

/// Interroge ffmpeg. Bloque quelques dizaines de millisecondes : à appeler sur un
/// thread.
pub fn probe(path: &Path) -> Result<VideoInfo, String> {
    let sortie = Command::new(FFMPEG)
        .arg("-hide_banner")
        .arg("-i")
        .arg(path)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("ffmpeg ne se lance pas : {e}"))?;
    // Sans sortie demandée, ffmpeg finit en erreur : seule sa description compte.
    let texte = String::from_utf8_lossy(&sortie.stderr);
    parse_probe(&texte).ok_or_else(|| {
        let nom = path.file_name().unwrap_or_default().to_string_lossy();
        format!("{nom} n'est pas une vidéo lisible")
    })
}

/// La taille décodée : jamais plus grande que le rendu, proportions gardées, et
/// des côtés pairs, comme l'exige le 4:2:0.
pub fn decode_size(info: &VideoInfo, max: (u32, u32)) -> (u32, u32) {
    let (w, h) = (info.width.max(1) as f64, info.height.max(1) as f64);
    let echelle = (max.0 as f64 / w).min(max.1 as f64 / h).min(1.0);
    let cote = |c: f64| ((c * echelle).round() as u32 & !1).max(2);
    (cote(w), cote(h))
}

#[cfg(test)]
mod tests {
    use super::*;

    const H264: &str = "Input #0, mov,mp4,m4a,3gp,3g2,mj2, from 'a.mp4':
  Metadata:
    major_brand     : isom
  Duration: 00:01:06.69, start: 0.000000, bitrate: 8389 kb/s
  Stream #0:0[0x1](und): Video: h264 (Main) (avc1 / 0x31637661), yuv420p(tv, bt709/unknown/unknown, progressive), 1280x720 [SAR 1:1 DAR 16:9], 8287 kb/s, 29.97 fps, 30 tbr, 15360 tbn (default)
  Stream #0:1[0x2](und): Audio: aac (LC) (mp4a / 0x6134706D), 44100 Hz, mono, fltp, 138 kb/s (default)
At least one output file must be specified";

    #[test]
    fn la_description_de_ffmpeg_se_lit() {
        let i = parse_probe(H264).unwrap();
        assert_eq!((i.width, i.height), (1280, 720));
        assert_eq!(i.fps, 29.97);
        assert!((i.duration - 66.69).abs() < 1e-9);
    }

    #[test]
    fn sans_fps_la_cadence_vient_de_tbr() {
        let texte = H264.replace("29.97 fps, ", "");
        assert_eq!(parse_probe(&texte).unwrap().fps, 30.0);
    }

    #[test]
    fn un_codec_aux_chiffres_trompeurs_ne_donne_pas_sa_taille() {
        let texte = "Duration: 00:00:02.00, start: 0
  Stream #0:0: Video: hap (Hap1 / 0x31706148), rgb0(pc, gbr/unknown/unknown), 3840x2160, 60 fps, 60 tbr";
        let i = parse_probe(texte).unwrap();
        assert_eq!((i.width, i.height), (3840, 2160));
    }

    #[test]
    fn un_son_seul_ou_une_duree_inconnue_n_est_pas_une_video() {
        let son = "Duration: 00:03:00.00, start: 0
  Stream #0:0: Audio: mp3, 44100 Hz, stereo";
        assert_eq!(parse_probe(son), None);
        let flux = H264.replace("00:01:06.69", "N/A");
        assert_eq!(parse_probe(&flux), None);
        assert_eq!(parse_probe("n'importe quoi"), None);
    }

    fn video(width: u32, height: u32) -> VideoInfo {
        VideoInfo {
            width,
            height,
            fps: 30.0,
            duration: 1.0,
        }
    }

    #[test]
    fn une_petite_video_garde_sa_taille() {
        assert_eq!(decode_size(&video(640, 360), (1920, 1080)), (640, 360));
    }

    #[test]
    fn une_grande_video_tient_dans_le_rendu() {
        assert_eq!(decode_size(&video(3840, 2160), (1920, 1080)), (1920, 1080));
        assert_eq!(decode_size(&video(3840, 2160), (1000, 1000)), (1000, 562));
    }

    #[test]
    fn les_cotes_restent_pairs() {
        let (w, h) = decode_size(&video(853, 481), (1920, 1080));
        assert_eq!((w % 2, h % 2), (0, 0));
    }
}
