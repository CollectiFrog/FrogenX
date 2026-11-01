//! L'enregistreur : un pipe vers le binaire `ffmpeg` — §8.2 de
//! `docs/ARCHITECTURE.md`.
//!
//! **Ce module ne connaît pas wgpu.** `FfmpegSink` est un `CpuSink` comme un autre :
//! la relecture, la cadence et la perte sont celles d'un flux (§8.5). Les lignes de
//! commande sont des fonctions pures : elles se testent sans ffmpeg.

use crate::sink::{CpuFrame, CpuSink, PixelLayout, SinkStatus};
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, OnceLock};
use std::time::Instant;

/// Le binaire, cherché dans le `PATH` : il n'est pas livré avec FrogenX.
const FFMPEG: &str = "ffmpeg";
/// Ce qu'une frame peut rattraper d'un coup, en secondes. Au-delà, c'est une panne
/// plutôt qu'un retard, et un fichier ne doit pas grossir pour la masquer.
const MAX_CATCH_UP: u64 = 10;

/// Le format d'une prise (§8.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RecordPreset {
    /// Léger et lisible partout : pour archiver un set.
    #[default]
    H264,
    /// Lourd et sans perte visible : pour le montage.
    ProRes,
}

impl RecordPreset {
    pub const ALL: [RecordPreset; 2] = [RecordPreset::H264, RecordPreset::ProRes];

    /// Son nom dans `sorties.txt`.
    pub fn word(self) -> &'static str {
        match self {
            RecordPreset::H264 => "h264",
            RecordPreset::ProRes => "prores",
        }
    }

    pub fn from_word(word: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.word() == word)
    }

    pub const fn label(self) -> &'static str {
        match self {
            RecordPreset::H264 => "H.264",
            RecordPreset::ProRes => "ProRes 422 HQ",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            RecordPreset::H264 => "mp4",
            RecordPreset::ProRes => "mov",
        }
    }

    /// Le préréglage d'un fichier, d'après son extension.
    pub fn of_path(path: &Path) -> Self {
        let mov = path.extension().is_some_and(|e| e == "mov");
        if mov {
            RecordPreset::ProRes
        } else {
            RecordPreset::H264
        }
    }

    /// L'encodeur de ce préréglage, sur cette machine.
    pub fn encoder(self, encoders: Encoders) -> VideoEncoder {
        match self {
            RecordPreset::ProRes => VideoEncoder::ProRes,
            RecordPreset::H264 if encoders.nvenc => VideoEncoder::Nvenc,
            RecordPreset::H264 => VideoEncoder::X264,
        }
    }

    fn audio_codec(self) -> [&'static str; 4] {
        match self {
            RecordPreset::H264 => ["-c:a", "aac", "-b:a", "256k"],
            RecordPreset::ProRes => ["-c:a", "pcm_s24le", "-ar", "48000"],
        }
    }
}

/// Un encodeur vidéo de ffmpeg.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoEncoder {
    Nvenc,
    X264,
    ProRes,
}

impl VideoEncoder {
    fn args(self) -> &'static [&'static str] {
        match self {
            VideoEncoder::Nvenc => &[
                "-c:v",
                "h264_nvenc",
                "-preset",
                "p5",
                "-rc",
                "vbr",
                "-cq",
                "19",
                "-b:v",
                "0",
            ],
            VideoEncoder::X264 => &["-c:v", "libx264", "-preset", "veryfast", "-crf", "18"],
            VideoEncoder::ProRes => &["-c:v", "prores_ks", "-profile:v", "3", "-vendor", "apl0"],
        }
    }

    fn pixel_format(self) -> &'static str {
        match self {
            VideoEncoder::Nvenc | VideoEncoder::X264 => "yuv420p",
            VideoEncoder::ProRes => "yuv422p10le",
        }
    }
}

fn words(mots: &[&str]) -> Vec<String> {
    mots.iter().map(|m| m.to_string()).collect()
}

/// La commande qui reçoit des images brutes sur son entrée et écrit `path`.
pub fn video_args(
    encoder: VideoEncoder,
    (width, height): (u32, u32),
    fps: u32,
    layout: PixelLayout,
    path: &Path,
) -> Vec<String> {
    let pixels = match layout {
        PixelLayout::RgbaStraight => "rgba",
        PixelLayout::BgraOpaque => "bgra",
    };
    let mut args = words(&["-hide_banner", "-loglevel", "error", "-y"]);
    args.extend(words(&["-f", "rawvideo", "-pix_fmt", pixels]));
    args.extend(["-s".into(), format!("{width}x{height}")]);
    args.extend(["-r".into(), fps.to_string()]);
    args.extend(words(&["-i", "-"]));
    args.extend(words(encoder.args()));
    // Le RGB du rendu est sRGB : sans le dire, un lecteur suppose du BT.601 et
    // les couleurs glissent.
    let couleurs = format!(
        "scale=out_color_matrix=bt709:out_range=tv,format={}",
        encoder.pixel_format()
    );
    args.extend(["-vf".into(), couleurs]);
    args.extend(words(&[
        "-colorspace",
        "bt709",
        "-color_primaries",
        "bt709",
        "-color_trc",
        "bt709",
        "-color_range",
        "tv",
    ]));
    args.push(path.display().to_string());
    args
}

/// La commande qui ajoute le son à l'image, sans la réencoder. `offset` : de
/// combien de secondes le son a démarré après l'image.
pub fn mux_args(
    preset: RecordPreset,
    video: &Path,
    audio: &Path,
    offset: f64,
    out: &Path,
) -> Vec<String> {
    let mut args = words(&["-hide_banner", "-loglevel", "error", "-y"]);
    args.extend(["-i".into(), video.display().to_string()]);
    let decalage = format!("{:.3}", offset.abs());
    // Parti avant l'image : on coupe son début. Parti après : on le retarde.
    args.push(if offset < 0.0 { "-ss" } else { "-itsoffset" }.into());
    args.push(decalage);
    args.extend(["-i".into(), audio.display().to_string()]);
    args.extend(words(&["-map", "0:v", "-map", "1:a", "-c:v", "copy"]));
    args.extend(words(&preset.audio_codec()));
    args.push("-shortest".into());
    args.push(out.display().to_string());
    args
}

/// Combien d'images écrire pour une frame arrivée à `elapsed` secondes de la
/// première, quand `written` le sont déjà. Le flux brut n'a pas d'horloge : une
/// frame perdue raccourcirait la vidéo, on répète donc pour rattraper.
///
/// Rien ne bouge à moins d'une image d'écart : le `t` d'un moteur temps réel
/// tremble, et un arrondi répéterait une image pour en sauter une autre.
pub fn frames_due(elapsed: f64, fps: u32, written: u64) -> u64 {
    let ecart = elapsed.max(0.0) * fps as f64 - written as f64;
    if ecart <= -1.0 {
        return 0;
    }
    (1 + ecart.max(0.0) as u64).min(MAX_CATCH_UP * fps as u64)
}

/// Le premier nom libre : sur scène, on ne tape pas de nom.
pub fn next_take(folder: &Path, preset: RecordPreset, free: impl Fn(&Path) -> bool) -> PathBuf {
    let extension = preset.extension();
    (1..)
        .map(|n| folder.join(format!("FrogenX {n:03}.{extension}")))
        .find(|p| free(p))
        .unwrap_or_else(|| folder.join(format!("FrogenX.{extension}")))
}

/// Le fichier d'image d'une prise, avant l'assemblage.
pub fn video_part(path: &Path) -> PathBuf {
    let extension = path.extension().unwrap_or_default().to_string_lossy();
    path.with_extension(format!("video.{extension}"))
}

/// Le fichier de son d'une prise, avant l'assemblage.
pub fn audio_part(path: &Path) -> PathBuf {
    path.with_extension("audio.wav")
}

/// Ce que la machine sait encoder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Encoders {
    pub nvenc: bool,
}

fn runs(args: &[&str]) -> bool {
    Command::new(FFMPEG)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Cherche ffmpeg, et essaie NVENC sur quelques images : `-encoders` le liste
/// même sans carte NVIDIA. `None` : ffmpeg est introuvable. Bloque : à appeler
/// sur un thread.
pub fn probe_encoders() -> Option<Encoders> {
    if !runs(&["-hide_banner", "-version"]) {
        return None;
    }
    let nvenc = runs(&[
        "-hide_banner",
        "-f",
        "lavfi",
        "-i",
        "color=size=256x256:rate=30",
        "-frames:v",
        "3",
        "-c:v",
        "h264_nvenc",
        "-f",
        "null",
        "-",
    ]);
    Some(Encoders { nvenc })
}

/// La dernière ligne d'un journal, ce qu'on peut en dire à l'utilisateur.
fn last_line(log: &Path) -> String {
    let texte = fs::read_to_string(log).unwrap_or_default();
    let ligne = texte.lines().rev().find(|l| !l.trim().is_empty());
    ligne.unwrap_or("ffmpeg s'est arrêté").to_string()
}

fn wait_ffmpeg(mut child: Child, log: &Path) -> Result<(), String> {
    let fin = child.wait().map_err(|e| e.to_string())?;
    if fin.success() {
        let _ = fs::remove_file(log);
        Ok(())
    } else {
        Err(last_line(log))
    }
}

fn log_of(path: &Path) -> PathBuf {
    path.with_extension("ffmpeg.log")
}

/// Assemble l'image et le son dans `out`, puis les retire. Sans son, l'image
/// prend simplement le nom final. Si ça échoue, les deux restent : une prise de
/// concert ne se jette pas (§8.2). Bloque : à appeler sur un thread.
pub fn assemble(
    preset: RecordPreset,
    video: &Path,
    audio: Option<(&Path, f64)>,
    out: &Path,
) -> Result<(), String> {
    let Some((son, offset)) = audio else {
        return fs::rename(video, out).map_err(|e| e.to_string());
    };
    let log = log_of(out);
    let journal = File::create(&log).map_err(|e| e.to_string())?;
    let child = Command::new(FFMPEG)
        .args(mux_args(preset, video, son, offset, out))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(journal)
        .spawn()
        .map_err(|e| e.to_string())?;
    wait_ffmpeg(child, &log)?;
    let _ = fs::remove_file(video);
    let _ = fs::remove_file(son);
    Ok(())
}

/// Ce qu'on suit d'une prise depuis l'extérieur.
pub struct RecordHandles {
    /// Reçoit le résultat de ffmpeg, une fois le fichier fermé.
    pub finished: Receiver<Result<(), String>>,
    /// L'instant de la première image : il recale le son.
    pub first_frame: Arc<OnceLock<Instant>>,
}

/// Écrit les frames reçues dans un ffmpeg. Le détruire ferme le fichier, sur un
/// thread à part : un encodeur qui vide ses tampons ne retient pas le rendu.
pub struct FfmpegSink {
    stdin: Option<ChildStdin>,
    child: Option<Child>,
    log: PathBuf,
    size: (u32, u32),
    layout: PixelLayout,
    fps: u32,
    first_t: Option<f64>,
    written: u64,
    repeated: u64,
    failure: Option<String>,
    first_frame: Arc<OnceLock<Instant>>,
    finished: Sender<Result<(), String>>,
}

impl FfmpegSink {
    /// Lance ffmpeg vers `path`. `layout` et `size` sont ceux des frames à venir.
    pub fn open(
        encoder: VideoEncoder,
        size: (u32, u32),
        fps: u32,
        layout: PixelLayout,
        path: &Path,
    ) -> Result<(Self, RecordHandles), String> {
        let log = log_of(path);
        let journal = File::create(&log).map_err(|e| format!("{}: {e}", log.display()))?;
        let mut child = Command::new(FFMPEG)
            .args(video_args(encoder, size, fps, layout, path))
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(journal)
            .spawn()
            .map_err(|e| format!("ffmpeg ne se lance pas : {e}"))?;
        let stdin = child.stdin.take();
        let (finished, recu) = channel();
        let first_frame = Arc::new(OnceLock::new());
        let sink = Self {
            stdin,
            child: Some(child),
            log,
            size,
            layout,
            fps,
            first_t: None,
            written: 0,
            repeated: 0,
            failure: None,
            first_frame: Arc::clone(&first_frame),
            finished,
        };
        let handles = RecordHandles {
            finished: recu,
            first_frame,
        };
        Ok((sink, handles))
    }

    fn write(&mut self, frame: &CpuFrame) -> Result<(), String> {
        if (frame.width, frame.height) != self.size || frame.layout != self.layout {
            return Err("le rendu a changé de taille pendant la prise".to_string());
        }
        let debut = *self.first_t.get_or_insert(frame.t);
        let _ = self.first_frame.set(Instant::now());
        let dues = frames_due(frame.t - debut, self.fps, self.written);
        let stdin = self.stdin.as_mut().ok_or("ffmpeg est fermé")?;
        for _ in 0..dues {
            stdin.write_all(&frame.pixels).map_err(|e| e.to_string())?;
        }
        self.written += dues;
        self.repeated += dues.saturating_sub(1);
        Ok(())
    }
}

impl CpuSink for FfmpegSink {
    fn submit(&mut self, frame: &CpuFrame) {
        if self.failure.is_none() {
            if let Err(e) = self.write(frame) {
                // Ferme l'entrée : ce qui est écrit reste un fichier lisible.
                self.stdin = None;
                self.failure = Some(e);
            }
        }
    }

    fn status(&self) -> SinkStatus {
        match &self.failure {
            Some(e) => SinkStatus::Failed(e.clone()),
            None if self.repeated > 0 => SinkStatus::Degraded {
                dropped: self.repeated.min(u32::MAX as u64) as u32,
            },
            None => SinkStatus::Active,
        }
    }
}

impl Drop for FfmpegSink {
    fn drop(&mut self) {
        self.stdin = None;
        let Some(child) = self.child.take() else {
            return;
        };
        let (log, finished) = (self.log.clone(), self.finished.clone());
        let attente = move || {
            let _ = finished.send(wait_ffmpeg(child, &log));
        };
        let _ = std::thread::Builder::new()
            .name("fin de prise".into())
            .spawn(attente);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contient(args: &[String], paire: [&str; 2]) -> bool {
        args.windows(2)
            .any(|w| w[0] == paire[0] && w[1] == paire[1])
    }

    #[test]
    fn la_commande_video_dit_tout_ce_que_le_flux_brut_ne_porte_pas() {
        let args = video_args(
            VideoEncoder::Nvenc,
            (1920, 1080),
            60,
            PixelLayout::BgraOpaque,
            Path::new("a.video.mp4"),
        );
        assert!(contient(&args, ["-pix_fmt", "bgra"]));
        assert!(contient(&args, ["-s", "1920x1080"]));
        assert!(contient(&args, ["-r", "60"]));
        assert!(contient(&args, ["-i", "-"]));
        assert!(contient(&args, ["-c:v", "h264_nvenc"]));
        assert!(contient(&args, ["-colorspace", "bt709"]));
        assert_eq!(args.last().unwrap(), "a.video.mp4");
    }

    #[test]
    fn chaque_encodeur_a_son_format_de_pixels() {
        let chemin = Path::new("a");
        let filtre = |e| {
            let args = video_args(e, (64, 64), 30, PixelLayout::RgbaStraight, chemin);
            let i = args.iter().position(|a| a == "-vf").unwrap();
            args[i + 1].clone()
        };
        assert!(filtre(VideoEncoder::X264).ends_with("format=yuv420p"));
        assert!(filtre(VideoEncoder::ProRes).ends_with("format=yuv422p10le"));
    }

    #[test]
    fn h264_prend_nvenc_seulement_s_il_repond() {
        let oui = Encoders { nvenc: true };
        let non = Encoders { nvenc: false };
        assert_eq!(RecordPreset::H264.encoder(oui), VideoEncoder::Nvenc);
        assert_eq!(RecordPreset::H264.encoder(non), VideoEncoder::X264);
        assert_eq!(RecordPreset::ProRes.encoder(oui), VideoEncoder::ProRes);
    }

    #[test]
    fn le_son_parti_avant_l_image_perd_son_debut() {
        let args = mux_args(
            RecordPreset::H264,
            Path::new("v.mp4"),
            Path::new("a.wav"),
            -0.25,
            Path::new("f.mp4"),
        );
        assert!(contient(&args, ["-ss", "0.250"]));
        assert!(contient(&args, ["-c:v", "copy"]));
        assert!(contient(&args, ["-c:a", "aac"]));
        let ss = args.iter().position(|a| a == "-ss").unwrap();
        let son = args.iter().position(|a| a == "a.wav").unwrap();
        assert!(ss < son, "une option d'entrée précède son fichier");
    }

    #[test]
    fn le_son_parti_apres_l_image_est_retarde() {
        let args = mux_args(
            RecordPreset::ProRes,
            Path::new("v.mov"),
            Path::new("a.wav"),
            0.1,
            Path::new("f.mov"),
        );
        assert!(contient(&args, ["-itsoffset", "0.100"]));
        assert!(contient(&args, ["-c:a", "pcm_s24le"]));
    }

    #[test]
    fn une_frame_a_l_heure_s_ecrit_une_fois() {
        assert_eq!(frames_due(0.0, 60, 0), 1);
        assert_eq!(frames_due(1.0 / 60.0, 60, 1), 1);
        assert_eq!(frames_due(1.0, 60, 60), 1);
    }

    #[test]
    fn une_frame_perdue_se_rattrape() {
        assert_eq!(frames_due(3.0 / 60.0, 60, 1), 3);
    }

    #[test]
    fn une_frame_tres_en_avance_attend_son_tour() {
        assert_eq!(frames_due(0.0, 60, 2), 0);
    }

    #[test]
    fn la_gigue_du_moteur_ne_repete_rien() {
        // Le pire cas d'un arrondi : le `t` oscille autour d'une demi-période.
        let mut ecrites = 0;
        let mut repetees = 0;
        for i in 0..600u64 {
            let t = (i as f64 + 0.5 + 0.4 * (i as f64).sin()) / 60.0;
            let dues = frames_due(t, 60, ecrites);
            ecrites += dues;
            repetees += dues.saturating_sub(1);
        }
        assert_eq!(repetees, 0);
        assert_eq!(ecrites, 600);
    }

    #[test]
    fn un_trou_enorme_ne_gonfle_pas_le_fichier() {
        assert_eq!(frames_due(3600.0, 30, 1), 300);
    }

    #[test]
    fn un_temps_qui_recule_n_ecrit_rien_de_plus() {
        assert_eq!(frames_due(-1.0, 60, 5), 0);
    }

    #[test]
    fn le_premier_nom_libre_est_choisi() {
        let dossier = Path::new("prises");
        let pris = dossier.join("FrogenX 001.mp4");
        let nom = next_take(dossier, RecordPreset::H264, |p| p != pris);
        assert_eq!(nom, dossier.join("FrogenX 002.mp4"));
        let nom = next_take(dossier, RecordPreset::ProRes, |_| true);
        assert_eq!(nom, dossier.join("FrogenX 001.mov"));
    }

    #[test]
    fn les_parties_d_une_prise_sont_a_cote_d_elle() {
        let p = Path::new("d/FrogenX 001.mp4");
        assert_eq!(video_part(p), Path::new("d/FrogenX 001.video.mp4"));
        assert_eq!(audio_part(p), Path::new("d/FrogenX 001.audio.wav"));
    }

    #[test]
    fn un_prereglage_se_relit_et_se_deduit_du_fichier() {
        for p in RecordPreset::ALL {
            assert_eq!(RecordPreset::from_word(p.word()), Some(p));
            assert!(!p.label().is_empty());
        }
        assert_eq!(RecordPreset::from_word("av1"), None);
        assert_eq!(
            RecordPreset::of_path(Path::new("x.mov")),
            RecordPreset::ProRes
        );
        assert_eq!(
            RecordPreset::of_path(Path::new("x.mp4")),
            RecordPreset::H264
        );
    }
}
