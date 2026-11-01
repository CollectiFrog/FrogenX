//! Une prise : l'image, le son, puis l'assemblage — §8.2 de `docs/ARCHITECTURE.md`.
//!
//! L'image part par un flux d'`Engine`, le son par le fil de l'analyse. À l'arrêt,
//! un thread attend ffmpeg et assemble : l'interface continue, et fermer
//! l'application l'attend.

use crate::Engine;
use frogenx_audio::{AudioInput, AudioRecording, AudioTake};
use frogenx_output::{
    assemble, audio_part, next_take, probe_encoders, video_part, CpuSinkId, Encoders, FfmpegSink,
    PixelLayout, RecordHandles, RecordPreset, SinkStatus, StreamFormat,
};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver};
use std::thread::JoinHandle;
use std::time::Instant;

/// Le temps laissé à ffmpeg pour fermer l'image.
const CLOSE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// Ce qui appartient à la machine : un dossier ne voyage pas avec un patch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecordConfig {
    pub preset: RecordPreset,
    pub format: StreamFormat,
    pub folder: PathBuf,
}

/// Le dossier des vidéos de l'utilisateur, ou celui où l'on est.
pub fn default_folder(env: impl Fn(&str) -> Option<String>) -> PathBuf {
    let maison = env("USERPROFILE").or_else(|| env("HOME"));
    let videos = if cfg!(target_os = "macos") {
        "Movies"
    } else {
        "Videos"
    };
    maison.map_or_else(|| PathBuf::from("."), |m| Path::new(&m).join(videos))
}

/// L'écart entre le départ du son et celui de l'image, en secondes.
fn audio_offset(video: Option<Instant>, audio: Option<Instant>) -> f64 {
    match (video, audio) {
        (Some(v), Some(a)) if a >= v => (a - v).as_secs_f64(),
        (Some(v), Some(a)) => -(v - a).as_secs_f64(),
        _ => 0.0,
    }
}

/// La durée d'une prise, comme on la lit dans l'en-tête.
pub fn clock_label(seconds: u64) -> String {
    format!("{:02}:{:02}", seconds / 60, seconds % 60)
}

/// Ce qu'une prise en cours retient.
struct Take {
    id: CpuSinkId,
    preset: RecordPreset,
    handles: RecordHandles,
    audio: Option<AudioRecording>,
    video: PathBuf,
    path: PathBuf,
}

/// Le travail de fin de prise, sur son thread.
struct Finishing {
    preset: RecordPreset,
    closed: Receiver<Result<(), String>>,
    video: PathBuf,
    audio: Option<(AudioTake, f64)>,
    path: PathBuf,
}

impl Finishing {
    fn run(self) -> Result<PathBuf, String> {
        let ferme = self.closed.recv_timeout(CLOSE_TIMEOUT);
        ferme.map_err(|_| "ffmpeg ne se ferme pas".to_string())??;
        let son = self
            .audio
            .as_ref()
            .filter(|(prise, _)| prise.started.is_some())
            .map(|(prise, ecart)| (prise.path.as_path(), *ecart));
        assemble(self.preset, &self.video, son, &self.path)?;
        Ok(self.path)
    }
}

/// L'enregistreur de la session.
pub struct Recording {
    probe: Option<Receiver<Option<Encoders>>>,
    encoders: Option<Encoders>,
    take: Option<Take>,
    finishing: Option<JoinHandle<Result<PathBuf, String>>>,
    status: String,
}

impl Default for Recording {
    fn default() -> Self {
        Self::new()
    }
}

impl Recording {
    /// Cherche ffmpeg sur un thread : la sonde dure quelques dixièmes de seconde.
    pub fn new() -> Self {
        let (reponse, recu) = channel();
        let sonde = move || {
            let _ = reponse.send(probe_encoders());
        };
        let probe = std::thread::Builder::new()
            .name("sonde ffmpeg".into())
            .spawn(sonde)
            .ok()
            .map(|_| recu);
        Self {
            probe,
            encoders: None,
            take: None,
            finishing: None,
            status: String::new(),
        }
    }

    /// Une prise peut-elle partir ?
    pub fn available(&self) -> bool {
        self.encoders.is_some()
    }

    pub fn active(&self) -> bool {
        self.take.is_some()
    }

    /// Les secondes de la prise en cours.
    pub fn elapsed(&self) -> u64 {
        let debut = self.take.as_ref().and_then(|t| t.handles.first_frame.get());
        debut.map_or(0, |d| d.elapsed().as_secs())
    }

    /// Ce que dit l'en-tête quand rien ne tourne : le dernier résultat.
    pub fn status(&self) -> &str {
        &self.status
    }

    /// L'état de la prise en cours, en mots.
    pub fn take_status(&self, engine: &Engine, audio: Option<&AudioInput>) -> Option<String> {
        let prise = self.take.as_ref()?;
        let perdus = audio.map_or(0, AudioInput::tap_lost);
        Some(match engine.stream_status(prise.id) {
            Some(SinkStatus::Failed(e)) => e,
            Some(SinkStatus::Degraded { dropped }) => format!("{dropped} images répétées"),
            _ if perdus > 0 => format!("{perdus} échantillons perdus"),
            _ => String::new(),
        })
    }

    /// À chaque frame : la sonde et la fin de prise.
    pub fn poll(&mut self) {
        if let Some(sonde) = &self.probe {
            if let Ok(trouve) = sonde.try_recv() {
                self.encoders = trouve;
                self.probe = None;
            }
        }
        if self.finishing.as_ref().is_some_and(|f| f.is_finished()) {
            self.collect();
        }
    }

    fn collect(&mut self) {
        let Some(fin) = self.finishing.take() else {
            return;
        };
        self.status = match fin.join() {
            Ok(Ok(fichier)) => format!("Enregistré : {}", name_of(&fichier)),
            Ok(Err(e)) => format!("Prise conservée en deux fichiers : {e}"),
            Err(_) => "Prise conservée en deux fichiers".to_string(),
        };
        println!("{}", self.status);
    }

    /// Lance une prise. `path` : un nom imposé, sinon le premier libre.
    pub fn start(
        &mut self,
        config: &RecordConfig,
        path: Option<PathBuf>,
        engine: &mut Engine,
        audio: Option<&mut AudioInput>,
    ) {
        let Some(encoders) = self.encoders else {
            self.status = "ffmpeg introuvable dans le PATH".to_string();
            return;
        };
        if self.take.is_some() || self.finishing.is_some() {
            return;
        }
        match self.open(config, encoders, path, engine, audio) {
            Ok(prise) => {
                self.take = Some(prise);
                self.status.clear();
            }
            Err(e) => self.status = format!("Enregistrement impossible : {e}"),
        }
    }

    fn open(
        &self,
        config: &RecordConfig,
        encoders: Encoders,
        path: Option<PathBuf>,
        engine: &mut Engine,
        audio: Option<&mut AudioInput>,
    ) -> Result<Take, String> {
        std::fs::create_dir_all(&config.folder).map_err(|e| e.to_string())?;
        let path = path.unwrap_or_else(|| {
            next_take(&config.folder, config.preset, |p| {
                !p.exists() && !video_part(p).exists()
            })
        });
        let preset = config.preset;
        let rendu = engine.render_size();
        let taille = config.format.within((rendu.x, rendu.y));
        let video = video_part(&path);
        let (sink, handles) = FfmpegSink::open(
            preset.encoder(encoders),
            taille,
            config.format.fps,
            PixelLayout::BgraOpaque,
            &video,
        )?;
        // Le son d'abord : il part avant l'image, et l'assemblage coupe l'écart.
        let son = audio.and_then(|entree| {
            let fil = entree.tap()?;
            AudioRecording::start(fil, entree.sample_rate(), &audio_part(&path)).ok()
        });
        let id = engine.add_stream(Box::new(sink), config.format);
        Ok(Take {
            id,
            preset,
            handles,
            audio: son,
            video,
            path,
        })
    }

    /// Arrête la prise. Le son s'arrête avant l'image : il peut la dépasser,
    /// l'assemblage s'arrête au plus court.
    pub fn stop(&mut self, engine: &mut Engine) {
        let Some(prise) = self.take.take() else {
            return;
        };
        let premier = prise.handles.first_frame.get().copied();
        let son = prise.audio.and_then(|a| a.stop().ok()).map(|fait| {
            let ecart = audio_offset(premier, fait.started);
            (fait, ecart)
        });
        engine.remove_stream(prise.id);
        let fin = Finishing {
            preset: prise.preset,
            closed: prise.handles.finished,
            video: prise.video,
            audio: son,
            path: prise.path,
        };
        self.status = "Assemblage…".to_string();
        self.finishing = std::thread::Builder::new()
            .name("assemblage".into())
            .spawn(move || fin.run())
            .ok();
    }

    /// Fermer l'application attend le fichier : une prise de concert ne se perd pas.
    pub fn finish(&mut self, engine: &mut Engine) {
        self.stop(engine);
        self.collect();
    }
}

fn name_of(path: &Path) -> String {
    let nom = path.file_name().unwrap_or_default();
    nom.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn le_dossier_par_defaut_est_celui_des_videos() {
        let env = |nom: &str| (nom == "USERPROFILE").then(|| "C:/moi".to_string());
        if !cfg!(target_os = "macos") {
            assert_eq!(default_folder(env), Path::new("C:/moi").join("Videos"));
        }
        assert_eq!(default_folder(|_| None), PathBuf::from("."));
    }

    #[test]
    fn l_ecart_est_negatif_quand_le_son_part_avant() {
        let t0 = Instant::now();
        let t1 = t0 + Duration::from_millis(250);
        assert!((audio_offset(Some(t1), Some(t0)) + 0.25).abs() < 1e-9);
        assert!((audio_offset(Some(t0), Some(t1)) - 0.25).abs() < 1e-9);
        assert_eq!(audio_offset(None, Some(t0)), 0.0);
    }

    #[test]
    fn la_duree_se_lit_en_minutes() {
        assert_eq!(clock_label(0), "00:00");
        assert_eq!(clock_label(83), "01:23");
        assert_eq!(clock_label(3725), "62:05");
    }

    #[test]
    fn sans_prise_il_n_y_a_rien_a_suivre() {
        let r = Recording::new();
        assert!(!r.active());
        assert_eq!(r.elapsed(), 0);
        assert_eq!(r.status(), "");
    }
}
