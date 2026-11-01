//! FrogenX — l'application : `cargo run`.
//!
//! Le binaire possède les fenêtres et la boucle d'événements ; tout le reste est
//! dans `Engine`, qui ne connaît pas de fenêtre, et dans `frogenx-ui`, qui
//! décrit l'interface (§8.3, §8.4, §23.5 de `docs/ARCHITECTURE.md`). Le moteur se
//! cadence sur sa propre échéance : masquer une fenêtre n'arrête rien.
//!
//! ```bash
//! cargo run                              # la fenêtre, jusqu'à sa fermeture
//! cargo run -- --frames 120              # 120 frames, un bilan, puis quitte
//! cargo run -- --output 2 --frames 120   # la même chose, sortie sur l'écran 2
//! cargo run -- --patch concert.frogenx   # ouvre ce fichier
//! ```
//!
//! Au clavier, en plus de l'interface : les touches 1 à 8 déclenchent les
//! colonnes de la grille, Espace tape le tempo.

mod files;
mod gui;
mod node_window;
mod stage;

use files::{Files, Reaction};
use frogenx_app::last_patch::config_dir;
use frogenx_app::recording::{clock_label, Recording};
use frogenx_app::settings::{MachineSettings, DEFAULT_RENDER};
use frogenx_app::streams::{open_spout, Streams, STREAMS_AVAILABLE};
use frogenx_app::{next_deadline, Engine, FrameRate};
use frogenx_audio::AudioInput;
use frogenx_composition::Command;
use frogenx_core::Clock;
use frogenx_layer::{Gpu, LAYER_FORMAT};
use frogenx_output::{MonitorSink, OutputSink, RecordPreset, StreamFormat};
use frogenx_ui::{HistoryRequest, OutputRequest, OutputView, RecordingView};
use glam::UVec2;
use gui::Gui;
use node_window::NodeWindow;
use rfd::AsyncFileDialog;
use stage::Stage;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

/// La taille du monitor dans l'interface : un quart de la sortie suffit.
const MONITOR_SIZE: UVec2 = UVec2::new(960, 540);
const USAGE: &str = "usage : frogenx [--frames N] [--output ECRAN] [--patch FICHIER] [--nodes] \
                     [--stream NOM] [--render LxH] [--rate N] [--record FICHIER]";

/// La période d'une cadence. Le moteur a sa propre échéance (§8.3).
fn period_of(rate: u32) -> Duration {
    Duration::from_secs_f64(1.0 / rate.max(1) as f64)
}

/// `1920x1080` en deux nombres.
fn parse_size(texte: &str) -> Option<UVec2> {
    let (w, h) = texte.split_once('x')?;
    Some(UVec2::new(w.parse().ok()?, h.parse().ok()?))
}

#[derive(Debug, Default, PartialEq, Eq)]
struct Options {
    /// Quitter après N frames : vérifie l'ouverture et la présentation (§8.3).
    frames: Option<u64>,
    /// Ouvre la sortie sur cet écran, compté à partir de 1, dès le lancement :
    /// vérifie la sortie sans souris (§8.4).
    output: Option<usize>,
    /// Ouvre ce fichier, plutôt que le dernier (§23.8).
    patch: Option<PathBuf>,
    /// Ouvre l'éditeur de nœuds dès le lancement : le vérifie sans souris (§23.10).
    nodes: bool,
    /// Ouvre un flux de ce nom : le vérifie sans souris (§8.5).
    stream: Option<String>,
    /// La résolution de rendu, pour ce lancement seulement.
    render: Option<UVec2>,
    /// La cadence du moteur, pour ce lancement seulement.
    rate: Option<u32>,
    /// Lance une prise dès que ffmpeg a répondu : la vérifie sans souris (§8.2).
    record: Option<PathBuf>,
}

fn parse_args(args: impl Iterator<Item = String>) -> Result<Options, String> {
    let mut options = Options::default();
    let mut args = args;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--frames" => {
                let n = args.next().and_then(|n| n.parse().ok());
                options.frames = Some(n.ok_or(USAGE)?);
            }
            "--output" => {
                let ecran = args.next().and_then(|n| n.parse::<usize>().ok());
                let ecran = ecran.and_then(|n| n.checked_sub(1));
                options.output = Some(ecran.ok_or(USAGE)?);
            }
            "--patch" => options.patch = Some(args.next().ok_or(USAGE)?.into()),
            "--nodes" => options.nodes = true,
            "--stream" => options.stream = Some(args.next().ok_or(USAGE)?),
            "--record" => options.record = Some(args.next().ok_or(USAGE)?.into()),
            "--render" => {
                let taille = args.next().and_then(|t| parse_size(&t));
                options.render = Some(taille.ok_or(USAGE)?);
            }
            "--rate" => {
                let rate = args.next().and_then(|n| n.parse().ok());
                options.rate = Some(rate.filter(|r| *r > 0).ok_or(USAGE)?);
            }
            _ => return Err(format!("argument inconnu : {arg}\n{USAGE}")),
        }
    }
    Ok(options)
}

/// Ce qu'une touche demande. La touche physique, pas le caractère : la rangée
/// des chiffres reste la même en AZERTY, où elle produit `&`, `é`, `"`, `'`.
#[derive(Debug, PartialEq, Eq)]
enum Action {
    Column(usize),
    Tap,
    FirstBeat,
}

fn key_action(key: KeyCode) -> Option<Action> {
    match key {
        KeyCode::Digit1 | KeyCode::Numpad1 => Some(Action::Column(0)),
        KeyCode::Digit2 | KeyCode::Numpad2 => Some(Action::Column(1)),
        KeyCode::Digit3 | KeyCode::Numpad3 => Some(Action::Column(2)),
        KeyCode::Digit4 | KeyCode::Numpad4 => Some(Action::Column(3)),
        KeyCode::Digit5 | KeyCode::Numpad5 => Some(Action::Column(4)),
        KeyCode::Digit6 | KeyCode::Numpad6 => Some(Action::Column(5)),
        KeyCode::Digit7 | KeyCode::Numpad7 => Some(Action::Column(6)),
        KeyCode::Digit8 | KeyCode::Numpad8 => Some(Action::Column(7)),
        KeyCode::Space => Some(Action::Tap),
        KeyCode::KeyR => Some(Action::FirstBeat),
        _ => None,
    }
}

/// La touche d'un appui franc : ni un relâchement, ni une répétition.
fn pressed_key(event: &KeyEvent) -> Option<KeyCode> {
    match event.physical_key {
        PhysicalKey::Code(key) if event.state.is_pressed() && !event.repeat => Some(key),
        _ => None,
    }
}

/// Ouvre ou ferme un flux, sur l'adaptateur du moteur.
fn toggle_stream(flux: &mut Streams, i: usize, moteur: &mut Engine) {
    let info = moteur.gpu().adapter.get_info();
    flux.toggle(i, moteur, &mut |nom, taille| open_spout(&info, nom, taille));
}

/// Les noms des préréglages, dans l'ordre de `RecordPreset::ALL`.
const PRESETS: [&str; 2] = [RecordPreset::H264.label(), RecordPreset::ProRes.label()];

/// Ce qui n'existe qu'une fois la fenêtre ouverte.
struct Running {
    window: Arc<Window>,
    engine: Engine,
    gui: Gui,
    /// `None` sans entrée utilisable : l'application tourne quand même, et
    /// `audio_status` dit pourquoi (§18.1).
    audio: Option<AudioInput>,
    audio_status: String,
    stage: Stage,
    /// Fermé tant qu'on ne l'a pas demandé : il ne coûte rien (§23.10).
    nodes: Option<NodeWindow>,
    files: Files,
    streams: Streams,
    recording: Recording,
    /// Le fichier de `--record`, jusqu'à ce que ffmpeg ait répondu.
    pending_record: Option<PathBuf>,
    /// Le dossier choisi dans sa fenêtre, quand elle se ferme.
    folder: Option<Receiver<PathBuf>>,
    /// Le rendu, la cadence et les flux tels qu'ils seront retenus (§8.5).
    settings: MachineSettings,
    /// Où les retenir. `None` sans dossier de configuration.
    settings_dir: Option<PathBuf>,
    period: Duration,
    rate: FrameRate,
    started: Instant,
    last: Instant,
    deadline: Instant,
    frames: u64,
}

impl Running {
    fn start(event_loop: &ActiveEventLoop, options: &Options) -> Result<Self, String> {
        let attributs = Window::default_attributes()
            .with_title("FrogenX")
            .with_inner_size(LogicalSize::new(1440.0, 900.0));
        let window = Arc::new(
            event_loop
                .create_window(attributs)
                .map_err(|e| format!("ouverture de la fenêtre impossible : {e}"))?,
        );
        let display = Box::new(event_loop.owned_display_handle());
        let (gpu, surface) = Gpu::with_surface(display, Arc::clone(&window))
            .ok_or("aucun adaptateur GPU ne peut présenter dans cette fenêtre")?;

        let info = gpu.adapter.get_info();
        println!(
            "GPU : {} ({:?}), MSAA {}×",
            info.name,
            info.backend,
            gpu.msaa_samples(LAYER_FORMAT)
        );

        let monitor = MonitorSink::new(&gpu, MONITOR_SIZE.x, MONITOR_SIZE.y);
        let aspect = MONITOR_SIZE.x as f32 / MONITOR_SIZE.y as f32;
        let gui = Gui::new(&window, &gpu, surface, &monitor.view(), aspect)?;
        let (files, grille, usine) = Files::start(options.patch.clone());
        let premiere = grille.columns().first().copied().filter(|_| usine);
        let settings_dir = config_dir(|nom| std::env::var(nom).ok());
        let settings = settings_dir
            .as_deref()
            .map(MachineSettings::load)
            .unwrap_or_default();
        let rendu = options.render.unwrap_or(settings.render);
        let period = period_of(options.rate.unwrap_or(settings.rate));
        let mut engine = Engine::new(gpu, Clock::realtime(), DEFAULT_RENDER, grille);
        if let Err(e) = engine.set_render_size(rendu) {
            println!("{e}");
        }
        let mut streams = Streams::new(settings.streams.clone());
        engine
            .add_output(OutputSink::Gpu(Box::new(monitor)))
            .map_err(|e| e.to_string())?;
        if let Some(column) = premiere {
            engine.push(Command::TriggerColumn { column });
        }

        let (audio, audio_status) = match AudioInput::open_default() {
            Ok(entree) => {
                let status = entree.status();
                (Some(entree), status)
            }
            Err(e) => (None, e.to_string()),
        };
        println!("Audio : {audio_status}");

        let nodes = if options.nodes {
            Some(NodeWindow::open(event_loop, engine.gpu())?)
        } else {
            None
        };

        let maintenant = Instant::now();
        let mut stage = Stage::new(maintenant);
        stage.refresh(event_loop, &window, &mut engine, maintenant);
        if !files.status().is_empty() {
            println!("{}", files.status());
        }
        if let Some(ecran) = options.output {
            stage.open(event_loop, ecran, &mut engine);
            println!("{}", stage.status());
        }
        if let Some(nom) = &options.stream {
            let rendu = engine.render_size();
            let i = streams.add_named(
                nom.clone(),
                StreamFormat {
                    width: rendu.x,
                    height: rendu.y,
                    fps: options.rate.unwrap_or(settings.rate),
                },
            );
            toggle_stream(&mut streams, i, &mut engine);
            println!("Flux : {}", streams.views(&engine)[i].status);
        }

        Ok(Self {
            window,
            engine,
            gui,
            audio,
            audio_status,
            stage,
            nodes,
            files,
            streams,
            recording: Recording::new(),
            pending_record: options.record.clone(),
            folder: None,
            settings,
            settings_dir,
            period,
            rate: FrameRate::new(period),
            started: maintenant,
            last: maintenant,
            deadline: maintenant,
            frames: 0,
        })
    }

    fn frame(&mut self, event_loop: &ActiveEventLoop, now: Instant) {
        let dt = (now - self.last).as_secs_f64();
        self.rate.tick(dt);
        if let Some(entree) = &mut self.audio {
            entree.configure(self.engine.analysis());
            self.engine.set_audio(entree.read().clone());
            self.audio_status = entree.status();
        }
        self.engine.frame(dt);
        let titre = self.files.title(self.engine.is_dirty());
        let flux = self.streams.views(&self.engine);
        let rendu = self.engine.render_size();
        self.recording.poll();
        self.poll_folder();
        if self.recording.available() {
            if let Some(fichier) = self.pending_record.take() {
                let mut reglage = self.settings.recording.clone();
                reglage.preset = RecordPreset::of_path(&fichier);
                reglage.format.width = rendu.x;
                reglage.format.height = rendu.y;
                let audio = self.audio.as_mut();
                self.recording
                    .start(&reglage, Some(fichier), &mut self.engine, audio);
            }
        }
        let prise = self
            .recording
            .take_status(&self.engine, self.audio.as_ref());
        let etat_prise = prise.unwrap_or_else(|| self.recording.status().to_string());
        let horloge = clock_label(self.recording.elapsed());
        let dossier = self.settings.recording.folder.display().to_string();
        let reglage = &self.settings.recording;
        let sortie = OutputView {
            streams: &flux,
            streams_available: STREAMS_AVAILABLE,
            render: (rendu.x, rendu.y),
            rate: (1.0 / self.period.as_secs_f64()).round() as u32,
            recording: RecordingView {
                available: self.recording.available(),
                active: self.recording.active(),
                presets: &PRESETS,
                preset: RecordPreset::ALL
                    .iter()
                    .position(|p| *p == reglage.preset)
                    .unwrap_or(0),
                size: (reglage.format.width, reglage.format.height),
                fps: reglage.format.fps,
                folder: &dossier,
                status: &etat_prise,
                clock: &horloge,
            },
            ..self.stage.view()
        };
        let demandes = self.gui.draw(
            &self.window,
            &mut self.engine,
            self.rate.fps(),
            &self.audio_status,
            sortie,
            self.files.view(&titre),
        );
        self.gui.recover(&self.window, self.engine.gpu());
        let mut reaction = self.files.poll(&self.window, &mut self.engine);
        if let Some(demande) = demandes.file {
            let suite = self.files.request(demande, &self.window, &mut self.engine);
            reaction = if suite == Reaction::Nothing {
                reaction
            } else {
                suite
            };
        }
        self.react(reaction, event_loop);
        match demandes.output {
            Some(OutputRequest::Open(ecran)) => {
                self.stage.open(event_loop, ecran, &mut self.engine);
            }
            Some(OutputRequest::Close) => self.stage.close(&mut self.engine),
            Some(demande) => self.apply_output(demande),
            None => {}
        }
        self.stage
            .refresh(event_loop, &self.window, &mut self.engine, now);
        self.stage.recover(&self.engine);

        if self.gui.take_nodes_request() {
            self.open_nodes(event_loop);
        }
        if let Some(fenetre) = &mut self.nodes {
            fenetre.draw(&mut self.engine, self.gui.selected_clip());
        }
        self.apply_history();
        self.last = now;
        self.frames += 1;
        self.deadline = next_deadline(self.deadline, now, self.period);
    }

    /// La fenêtre du système sur un thread : elle ne doit pas figer l'image.
    fn choose_folder(&mut self) {
        let (reponse, recu) = channel();
        let boite = AsyncFileDialog::new()
            .set_directory(&self.settings.recording.folder)
            .set_parent(&*self.window)
            .pick_folder();
        std::thread::spawn(move || {
            if let Some(dossier) = pollster::block_on(boite) {
                let _ = reponse.send(dossier.path().to_path_buf());
            }
        });
        self.folder = Some(recu);
    }

    fn poll_folder(&mut self) {
        let choisi = self.folder.as_ref().and_then(|r| r.try_recv().ok());
        if let Some(dossier) = choisi {
            self.folder = None;
            self.settings.recording.folder = dossier;
            if let Some(dir) = &self.settings_dir {
                self.settings.save(dir);
            }
        }
    }

    /// Le rendu et les flux : ce que l'interface demande, puis ce qu'on retient.
    fn apply_output(&mut self, demande: OutputRequest) {
        let flux = &mut self.streams;
        let moteur = &mut self.engine;
        match demande {
            OutputRequest::SetRender((w, h)) => {
                let taille = UVec2::new(w, h);
                match moteur.set_render_size(taille) {
                    Ok(()) => self.settings.render = taille,
                    Err(e) => println!("{e}"),
                }
            }
            OutputRequest::SetRate(rate) => {
                self.settings.rate = rate;
                self.period = period_of(rate);
                self.rate = FrameRate::new(self.period);
            }
            OutputRequest::AddStream => {
                let rendu = moteur.render_size();
                flux.add(StreamFormat {
                    width: rendu.x,
                    height: rendu.y,
                    fps: self.settings.rate,
                });
            }
            OutputRequest::ToggleStream(i) => toggle_stream(flux, i, moteur),
            OutputRequest::SetStreamSize(i, (width, height)) => {
                if let Some(f) = flux.format(i) {
                    let suite = StreamFormat { width, height, ..f };
                    flux.set_format(i, suite, moteur);
                }
            }
            OutputRequest::SetStreamFps(i, fps) => {
                if let Some(f) = flux.format(i) {
                    flux.set_format(i, StreamFormat { fps, ..f }, moteur);
                }
            }
            OutputRequest::RemoveStream(i) => flux.remove(i, moteur),
            OutputRequest::ToggleRecording => {
                if self.recording.active() {
                    self.recording.stop(moteur);
                } else {
                    let reglage = &self.settings.recording;
                    self.recording
                        .start(reglage, None, moteur, self.audio.as_mut());
                }
                return;
            }
            OutputRequest::SetRecordPreset(i) => {
                if let Some(preset) = RecordPreset::ALL.get(i) {
                    self.settings.recording.preset = *preset;
                }
            }
            OutputRequest::SetRecordSize((width, height)) => {
                let f = &mut self.settings.recording.format;
                (f.width, f.height) = (width, height);
            }
            OutputRequest::SetRecordFps(fps) => self.settings.recording.format.fps = fps,
            OutputRequest::ChooseRecordFolder => {
                self.choose_folder();
                return;
            }
            OutputRequest::Open(_) | OutputRequest::Close => return,
        }
        self.settings.streams = self.streams.configs();
        if let Some(dir) = &self.settings_dir {
            self.settings.save(dir);
        }
    }

    fn on_key(&mut self, key: KeyCode) {
        match key_action(key) {
            Some(Action::Column(i)) => {
                if let Some(column) = self.engine.composition().columns().get(i).copied() {
                    self.engine.push(Command::LaunchColumn { column });
                }
            }
            Some(Action::Tap) => self.engine.push(Command::TapTempo),
            Some(Action::FirstBeat) => self.engine.push(Command::ResyncBeat),
            None => {}
        }
    }

    /// Annuler ou rétablir, demandé d'une fenêtre ou de l'autre. Un bouton tenu dans
    /// l'une ou l'autre garde le pas ouvert (§23.11).
    fn apply_history(&mut self) {
        let editeur = self.nodes.as_mut();
        let tenu = editeur.as_ref().is_some_and(|f| f.pointer_held());
        let demande = editeur.and_then(|f| f.take_history_request());
        let demande = self.gui.take_history_request().or(demande);
        self.engine.set_gesture(tenu || self.gui.pointer_held());
        let (fait, verbe) = match demande {
            Some(HistoryRequest::Undo) => (self.engine.undo(), "Annulé"),
            Some(HistoryRequest::Redo) => (self.engine.redo(), "Rétabli"),
            None => return,
        };
        if let Some(fait) = fait {
            self.gui.remap(&fait.map);
            if let Some(fenetre) = &mut self.nodes {
                fenetre.remap(&fait.map);
            }
            self.files.set_status(format!("{verbe} : {}", fait.label));
        }
    }

    /// Ouvre l'éditeur, ou le ramène devant s'il l'est déjà. Un échec se lit dans
    /// le bilan : il n'arrête rien (§18.1).
    fn open_nodes(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(fenetre) = &self.nodes {
            fenetre.focus();
            return;
        }
        match NodeWindow::open(event_loop, self.engine.gpu()) {
            Ok(fenetre) => self.nodes = Some(fenetre),
            Err(e) => eprintln!("Éditeur de nœuds impossible : {e}"),
        }
    }

    fn react(&mut self, reaction: Reaction, event_loop: &ActiveEventLoop) {
        match reaction {
            Reaction::Nothing => {}
            Reaction::GridReplaced => {
                self.gui.forget_grid();
                if let Some(fenetre) = &mut self.nodes {
                    fenetre.forget_grid();
                }
            }
            Reaction::Quit => {
                // Une prise en cours se referme avant de quitter : elle ne se perd pas.
                self.recording.finish(&mut self.engine);
                event_loop.exit();
            }
        }
    }

    /// Un événement de la fenêtre de sortie. egui n'en voit rien, mais le
    /// clavier joue encore si elle prend le focus par accident (§8.4).
    fn on_output_event(&mut self, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => self.stage.close_requested(&mut self.engine),
            WindowEvent::Focused(true) if self.stage.returns_focus() => {
                self.window.focus_window();
            }
            WindowEvent::Resized(taille) => {
                self.stage.resize(taille);
                self.stage.poll_soon();
            }
            WindowEvent::Moved(_) => self.stage.poll_soon(),
            WindowEvent::KeyboardInput { event, .. } => {
                if let Some(key) = pressed_key(&event) {
                    self.on_key(key);
                }
            }
            _ => {}
        }
    }

    fn report(&self) {
        let duree = self.started.elapsed().as_secs_f64();
        let (min, max) = self.rate.range_ms();
        println!(
            "{} frames en {duree:.2} s ({:.1} fps), {} présentées, intervalle {min:.1} à {max:.1} ms, interface {:.2} ms/frame \
             — sorties : {:?}",
            self.frames,
            self.frames as f64 / duree,
            self.gui.presented,
            self.gui.spent.as_secs_f64() * 1000.0 / self.frames.max(1) as f64,
            self.engine.output_statuses()
        );
        if let Some(fenetre) = &self.nodes {
            println!("Éditeur de nœuds : {} frames présentées", fenetre.presented);
        }
        if let Some(presentees) = self.stage.presented() {
            println!("{}, {presentees} frames présentées", self.stage.status());
        }
    }
}

struct App {
    options: Options,
    running: Option<Running>,
    failure: Option<String>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.running.is_some() {
            return;
        }
        match Running::start(event_loop, &self.options) {
            Ok(running) => self.running = Some(running),
            Err(e) => {
                self.failure = Some(e);
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let Some(running) = &mut self.running else {
            return;
        };
        // Une fenêtre de sortie déjà fermée peut encore envoyer un événement :
        // jamais il ne doit atteindre l'interface, où il quitterait l'application.
        if id != running.window.id() {
            if running.stage.owns(id) {
                running.on_output_event(event);
            } else if let Some(fenetre) = &mut running.nodes {
                if fenetre.owns(id) && fenetre.on_event(&event) {
                    running.nodes = None;
                }
            }
            return;
        }
        let garde_par_egui = running.gui.on_event(&running.window, &event);
        match event {
            // Pas de sortie directe : il reste peut-être de quoi enregistrer.
            WindowEvent::CloseRequested => {
                let reaction = running
                    .files
                    .close_requested(&running.window, &mut running.engine);
                running.react(reaction, event_loop);
            }
            WindowEvent::Resized(taille) => running.gui.resize(taille.width, taille.height),
            WindowEvent::KeyboardInput { event, .. } if !garde_par_egui => {
                if let Some(key) = pressed_key(&event) {
                    running.on_key(key);
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let Some(running) = &mut self.running else {
            return;
        };
        let maintenant = Instant::now();
        if maintenant >= running.deadline {
            running.frame(event_loop, maintenant);
        }
        if self.options.frames.is_some_and(|n| running.frames >= n) {
            running.report();
            running.recording.finish(&mut running.engine);
            running.stage.close(&mut running.engine);
            event_loop.exit();
            // `exit` n'arrête pas la boucle sur-le-champ : elle repasse ici.
            self.running = None;
            return;
        }
        event_loop.set_control_flow(ControlFlow::WaitUntil(running.deadline));
    }
}

fn main() -> ExitCode {
    let options = match parse_args(std::env::args().skip(1)) {
        Ok(options) => options,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::FAILURE;
        }
    };
    let mut app = App {
        options,
        running: None,
        failure: None,
    };
    let lancement = EventLoop::new().and_then(|event_loop| event_loop.run_app(&mut app));
    if let Err(e) = lancement {
        eprintln!("boucle d'événements impossible : {e}");
        return ExitCode::FAILURE;
    }
    if let Some(e) = app.failure {
        eprintln!("{e}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Options, String> {
        parse_args(args.iter().map(|a| a.to_string()))
    }

    #[test]
    fn sans_argument_la_fenetre_reste_ouverte() {
        assert_eq!(parse(&[]), Ok(Options::default()));
    }

    #[test]
    fn un_flux_le_rendu_et_la_cadence_se_choisissent_au_lancement() {
        assert_eq!(
            parse(&["--record", "a.mov"]).unwrap().record,
            Some(PathBuf::from("a.mov"))
        );
        assert!(parse(&["--record"]).is_err());
        let lu = parse(&["--stream", "Scène", "--render", "1280x720", "--rate", "50"]).unwrap();
        assert_eq!(lu.stream.as_deref(), Some("Scène"));
        assert_eq!(lu.render, Some(UVec2::new(1280, 720)));
        assert_eq!(lu.rate, Some(50));
    }

    #[test]
    fn un_rendu_ou_une_cadence_illisibles_sont_refuses() {
        assert!(parse(&["--render", "1280"]).is_err());
        assert!(parse(&["--render", "ax720"]).is_err());
        assert!(parse(&["--rate", "0"]).is_err());
        assert!(parse(&["--stream"]).is_err());
    }

    #[test]
    fn la_periode_suit_la_cadence() {
        assert_eq!(period_of(50), Duration::from_millis(20));
        assert_eq!(period_of(0), Duration::from_secs(1));
    }

    #[test]
    fn frames_fixe_le_nombre_de_frames() {
        assert_eq!(
            parse(&["--frames", "120"]),
            Ok(Options {
                frames: Some(120),
                ..Options::default()
            })
        );
    }

    #[test]
    fn nodes_ouvre_l_editeur_au_lancement() {
        assert!(parse(&["--nodes"]).unwrap().nodes);
        assert!(!parse(&[]).unwrap().nodes);
    }

    #[test]
    fn patch_designe_un_fichier() {
        assert_eq!(
            parse(&["--patch", "a.frogenx"]).unwrap().patch,
            Some(PathBuf::from("a.frogenx"))
        );
        assert!(parse(&["--patch"]).is_err());
    }

    #[test]
    fn output_compte_les_ecrans_a_partir_de_un() {
        assert_eq!(parse(&["--output", "2"]).unwrap().output, Some(1));
        assert!(parse(&["--output", "0"]).is_err());
        assert!(parse(&["--output"]).is_err());
        assert!(parse(&["--output", "x"]).is_err());
    }

    #[test]
    fn un_nombre_manquant_ou_invalide_est_refuse() {
        assert!(parse(&["--frames"]).is_err());
        assert!(parse(&["--frames", "abc"]).is_err());
        assert!(parse(&["--frames", "-1"]).is_err());
    }

    #[test]
    fn les_chiffres_declenchent_les_colonnes_et_espace_tape() {
        assert_eq!(key_action(KeyCode::Digit1), Some(Action::Column(0)));
        assert_eq!(key_action(KeyCode::Numpad4), Some(Action::Column(3)));
        assert_eq!(key_action(KeyCode::Digit8), Some(Action::Column(7)));
        assert_eq!(key_action(KeyCode::Numpad5), Some(Action::Column(4)));
        assert_eq!(key_action(KeyCode::Digit9), None);
        assert_eq!(key_action(KeyCode::Space), Some(Action::Tap));
        assert_eq!(key_action(KeyCode::KeyR), Some(Action::FirstBeat));
        assert_eq!(key_action(KeyCode::KeyA), None);
    }

    #[test]
    fn un_argument_inconnu_est_refuse() {
        assert!(parse(&["--plein-ecran"]).is_err());
    }
}
