//! La lecture d'une vidéo : le tampon de tête et le flux — §9.3 de
//! `docs/ARCHITECTURE.md`.
//!
//! Un seul thread décode les têtes, l'une après l'autre : ouvrir une grille de
//! cinquante clips ne doit pas lancer cinquante ffmpeg. Chaque flux a le sien, et
//! rien de ce qui peut attendre ne tourne sur le thread du moteur : même le
//! lancement du processus se fait à côté.

use crate::info::{decode_size, probe, VideoInfo, FFMPEG};
use crate::schedule::{plan, Plan, StreamView, HEAD_SECONDS};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, sync_channel, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Les images qu'un flux prépare d'avance : au-delà, ffmpeg attend sur son pipe.
const QUEUE: usize = 6;
/// L'écart le plus court entre deux relances : la marche arrière se fait par
/// à-coups (§9.3), et chaque relance est un processus.
const MIN_SEEK: Duration = Duration::from_millis(250);
/// Ce qu'un rendu offline attend une image avant de renoncer.
const OFFLINE_TIMEOUT: Duration = Duration::from_secs(30);

/// Une image décodée, en YUV 4:2:0 : ses trois plans, à la suite.
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    /// Son instant dans la vidéo, en secondes.
    pub time: f64,
    pub width: u32,
    pub height: u32,
    pub data: Arc<[u8]>,
}

fn frame_bytes((w, h): (u32, u32)) -> usize {
    w as usize * h as usize * 3 / 2
}

/// La commande qui écrit des images brutes sur sa sortie, à partir de `start`.
/// `-fps` les rend régulières : le numéro d'une image dit alors son instant.
fn decode_args(
    path: &Path,
    start: f64,
    fps: f32,
    (w, h): (u32, u32),
    frames: Option<usize>,
) -> Vec<std::ffi::OsString> {
    let mot = |m: &str| m.into();
    let mut args: Vec<std::ffi::OsString> = vec![
        mot("-hide_banner"),
        mot("-loglevel"),
        mot("error"),
        mot("-nostdin"),
        mot("-hwaccel"),
        mot("auto"),
    ];
    if start > 0.0 {
        args.extend([mot("-ss"), format!("{start:.4}").into()]);
    }
    args.extend([mot("-i"), path.into()]);
    args.extend([mot("-map"), mot("0:v:0"), mot("-an"), mot("-sn")]);
    let filtre =
        format!("fps={fps},scale={w}:{h}:out_color_matrix=bt709:out_range=tv,format=yuv420p");
    args.extend([mot("-vf"), filtre.into()]);
    if let Some(n) = frames {
        args.extend([mot("-frames:v"), n.to_string().into()]);
    }
    args.extend([mot("-f"), mot("rawvideo"), mot("-")]);
    args
}

fn spawn_decoder(args: &[std::ffi::OsString]) -> std::io::Result<Child> {
    Command::new(FFMPEG)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
}

/// La tête d'une vidéo, prête à servir.
struct Head {
    info: VideoInfo,
    frames: Vec<Arc<Frame>>,
}

/// Ce qu'on sait d'un chargement : rien encore, ou son résultat, une seule fois.
type Loaded = Arc<OnceLock<Result<Head, String>>>;

struct Job {
    path: PathBuf,
    max: (u32, u32),
    loaded: Loaded,
}

fn read_head(path: &Path, max: (u32, u32)) -> Result<Head, String> {
    let info = probe(path)?;
    let taille = decode_size(&info, max);
    let n = ((HEAD_SECONDS * info.fps as f64).ceil() as usize).max(1);
    let args = decode_args(path, 0.0, info.fps, taille, Some(n));
    let mut child = spawn_decoder(&args).map_err(|e| e.to_string())?;
    let mut sortie = child.stdout.take().ok_or("ffmpeg sans sortie")?;
    let mut frames = Vec::with_capacity(n);
    let mut octets = vec![0; frame_bytes(taille)];
    while frames.len() < n && sortie.read_exact(&mut octets).is_ok() {
        frames.push(Arc::new(Frame {
            time: frames.len() as f64 / info.fps as f64,
            width: taille.0,
            height: taille.1,
            data: Arc::from(octets.as_slice()),
        }));
    }
    let _ = child.kill();
    let _ = child.wait();
    if frames.is_empty() {
        let nom = path.file_name().unwrap_or_default().to_string_lossy();
        return Err(format!("{nom} ne se décode pas"));
    }
    Ok(Head { info, frames })
}

/// Le thread qui décode les têtes, dans l'ordre des demandes. S'arrête avec lui.
pub struct Loader {
    jobs: Sender<Job>,
}

impl Default for Loader {
    fn default() -> Self {
        Self::new()
    }
}

impl Loader {
    pub fn new() -> Self {
        let (jobs, recus) = channel::<Job>();
        let travail = move || {
            for job in recus {
                let _ = job.loaded.set(read_head(&job.path, job.max));
            }
        };
        // Sans thread, `open` échoue à `send` : le clip dit qu'il ne se charge pas.
        let _ = std::thread::Builder::new()
            .name("têtes vidéo".into())
            .spawn(travail);
        Self { jobs }
    }
}

/// Un flux ffmpeg en cours. Le lâcher l'arrête.
struct Stream {
    next: f64,
    period: f64,
    frames: Receiver<Frame>,
    child: Arc<Mutex<Option<Child>>>,
    stop: Arc<AtomicBool>,
}

impl Stream {
    fn open(path: &Path, start: f64, fps: f32, size: (u32, u32)) -> Self {
        let period = 1.0 / fps.max(1.0) as f64;
        let args = decode_args(path, start, fps, size, None);
        let (envoi, frames) = sync_channel(QUEUE);
        let child: Arc<Mutex<Option<Child>>> = Arc::new(Mutex::new(None));
        let stop = Arc::new(AtomicBool::new(false));
        let (partage, arret) = (Arc::clone(&child), Arc::clone(&stop));
        let travail = move || {
            let Ok(mut processus) = spawn_decoder(&args) else {
                return;
            };
            let sortie = processus.stdout.take();
            if let Ok(mut place) = partage.lock() {
                *place = Some(processus);
            }
            let Some(mut sortie) = sortie else { return };
            let mut octets = vec![0; frame_bytes(size)];
            let mut n = 0u64;
            while !arret.load(Ordering::Relaxed) && sortie.read_exact(&mut octets).is_ok() {
                let frame = Frame {
                    time: start + n as f64 * period,
                    width: size.0,
                    height: size.1,
                    data: Arc::from(octets.as_slice()),
                };
                n += 1;
                if envoi.send(frame).is_err() {
                    break;
                }
            }
            reap(&partage);
        };
        let _ = std::thread::Builder::new()
            .name("flux vidéo".into())
            .spawn(travail);
        Self {
            next: start,
            period,
            frames,
            child,
            stop,
        }
    }
}

fn reap(child: &Mutex<Option<Child>>) {
    if let Ok(mut place) = child.lock() {
        if let Some(mut processus) = place.take() {
            let _ = processus.kill();
            let _ = processus.wait();
        }
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // Le thread peut attendre sur le pipe ou sur la file : tuer ffmpeg le
        // libère, et il reprendra le processus. Sur un thread : ne pas retenir la frame.
        let child = Arc::clone(&self.child);
        let _ = std::thread::Builder::new()
            .name("fin de flux vidéo".into())
            .spawn(move || reap(&child));
    }
}

/// La lecture d'un fichier.
pub struct VideoPlayer {
    path: PathBuf,
    loaded: Loaded,
    stream: Option<Stream>,
    last: Option<Arc<Frame>>,
    last_seek: Option<Instant>,
}

impl VideoPlayer {
    /// Demande le chargement de la tête, sans attendre. Le décodage ne dépassera
    /// pas `max`.
    pub fn open(path: &Path, max: (u32, u32), loader: &Loader) -> Self {
        let loaded: Loaded = Arc::new(OnceLock::new());
        let job = Job {
            path: path.to_path_buf(),
            max,
            loaded: Arc::clone(&loaded),
        };
        if loader.jobs.send(job).is_err() {
            let _ = loaded.set(Err("le chargement vidéo est arrêté".to_string()));
        }
        Self {
            path: path.to_path_buf(),
            loaded,
            stream: None,
            last: None,
            last_seek: None,
        }
    }

    /// Le chargement a échoué : pourquoi, en mots.
    pub fn error(&self) -> Option<&str> {
        self.loaded.get()?.as_ref().err().map(String::as_str)
    }

    pub fn info(&self) -> Option<VideoInfo> {
        Some(self.head()?.info)
    }

    fn head(&self) -> Option<&Head> {
        self.loaded.get()?.as_ref().ok()
    }

    /// Le clip ne joue plus : son flux s'arrête, sa tête reste.
    pub fn pause(&mut self) {
        self.stream = None;
    }

    /// Lance le flux qui prendra le relais à la fin de la tête.
    fn relay(&mut self) {
        let Some(tete) = self.head() else { return };
        let fin = tete.frames.len() as f64 / tete.info.fps as f64;
        let (fps, taille) = (tete.info.fps, frame_size(tete));
        let pret = self.stream.as_ref().is_some_and(|s| s.next <= fin + 1e-6);
        if !pret {
            self.stream = Some(Stream::open(&self.path, fin, fps, taille));
        }
    }

    fn seek(&mut self, t: f64) {
        let Some(tete) = self.head() else { return };
        let (fps, taille) = (tete.info.fps, frame_size(tete));
        if self.last_seek.is_some_and(|d| d.elapsed() < MIN_SEEK) {
            return;
        }
        self.last_seek = Some(Instant::now());
        self.stream = Some(Stream::open(&self.path, t, fps, taille));
    }

    /// Sert ce que le flux a déjà préparé, jusqu'à l'instant `t`.
    fn drain(&mut self, t: f64) {
        let Some(flux) = &mut self.stream else { return };
        while flux.next <= t + flux.period * 0.5 {
            match flux.frames.try_recv() {
                Ok(frame) => {
                    flux.next = frame.time + flux.period;
                    self.last = Some(Arc::new(frame));
                }
                Err(TryRecvError::Empty | TryRecvError::Disconnected) => break,
            }
        }
    }

    /// Un instant dans la vidéo : un temps au-delà de la fin vaut sa dernière image.
    fn inside(&self, t: f64) -> Option<f64> {
        let info = self.head()?.info;
        let derniere = info.duration - 1.0 / info.fps.max(1.0) as f64;
        Some(t.min(derniere.max(0.0)))
    }

    fn decide(&self, t: f64) -> Option<Plan> {
        let tete = self.head()?;
        let flux = self.stream.as_ref().map(|s| StreamView { next: s.next });
        Some(plan(t, tete.info.fps, tete.frames.len(), flux))
    }

    /// L'image de l'instant `t`, ou la précédente si elle n'est pas prête. Ne
    /// bloque jamais (§9.3). `None` tant que rien n'a pu être décodé.
    pub fn frame_at(&mut self, t: f64) -> Option<Arc<Frame>> {
        let t = self.inside(t)?;
        match self.decide(t)? {
            Plan::Head(n) => {
                self.relay();
                self.last = Some(Arc::clone(self.head()?.frames.get(n)?));
            }
            Plan::Stream => self.drain(t),
            Plan::Seek(cible) => {
                self.seek(cible);
                self.drain(t);
            }
        }
        self.last.clone()
    }

    /// L'image exacte de l'instant `t`, en attendant ce qu'il faut : un rendu
    /// offline ne perd rien (§4, §20.3).
    pub fn frame_at_blocking(&mut self, t: f64) -> Option<Arc<Frame>> {
        let debut = Instant::now();
        while self.loaded.get().is_none() && debut.elapsed() < OFFLINE_TIMEOUT {
            std::thread::sleep(Duration::from_millis(1));
        }
        let t = self.inside(t)?;
        match self.decide(t)? {
            Plan::Head(n) => {
                self.last = Some(Arc::clone(self.head()?.frames.get(n)?));
                return self.last.clone();
            }
            Plan::Stream => {}
            Plan::Seek(cible) => {
                let tete = self.head()?;
                let (fps, taille) = (tete.info.fps, frame_size(tete));
                self.stream = Some(Stream::open(&self.path, cible, fps, taille));
            }
        }
        let flux = self.stream.as_mut()?;
        while flux.next <= t + flux.period * 0.5 {
            match flux.frames.recv_timeout(OFFLINE_TIMEOUT) {
                Ok(frame) => {
                    flux.next = frame.time + flux.period;
                    self.last = Some(Arc::new(frame));
                }
                // La fin du fichier : la dernière image reste.
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => break,
            }
        }
        self.last.clone()
    }
}

fn frame_size(tete: &Head) -> (u32, u32) {
    let image = &tete.frames[0];
    (image.width, image.height)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_commande_de_decodage_dit_tout() {
        let args = decode_args(Path::new("a.mp4"), 1.5, 30.0, (640, 360), Some(15));
        let mots: Vec<String> = args.iter().map(|a| a.to_string_lossy().into()).collect();
        let suite = |a: &str, b: &str| mots.windows(2).any(|w| w[0] == a && w[1] == b);
        assert!(suite("-ss", "1.5000"));
        assert!(suite("-i", "a.mp4"));
        assert!(suite("-frames:v", "15"));
        assert!(suite("-f", "rawvideo"));
        assert!(mots.contains(&"-an".to_string()), "le son est ignoré");
        let ss = mots.iter().position(|m| m == "-ss").unwrap();
        let i = mots.iter().position(|m| m == "-i").unwrap();
        assert!(ss < i, "avant `-i` : ffmpeg saute au lieu de tout décoder");
        let filtre = mots.iter().find(|m| m.starts_with("fps=")).unwrap();
        assert!(filtre.contains("scale=640:360"));
        assert!(filtre.ends_with("format=yuv420p"));
    }

    #[test]
    fn sans_depart_ni_fin_la_commande_lit_tout() {
        let args = decode_args(Path::new("a.mp4"), 0.0, 30.0, (640, 360), None);
        assert!(!args.iter().any(|a| a == "-ss" || a == "-frames:v"));
    }

    #[test]
    fn une_image_420_pese_un_octet_et_demi_par_pixel() {
        assert_eq!(frame_bytes((1920, 1080)), 3_110_400);
    }
}
