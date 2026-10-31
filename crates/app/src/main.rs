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
//! ```
//!
//! Au clavier, en plus de l'interface : les touches 1 à 8 déclenchent les
//! colonnes de la grille, Espace tape le tempo.

mod gui;
mod stage;

use frogenx_app::{next_deadline, Engine, FrameRate};
use frogenx_audio::AudioInput;
use frogenx_composition::Command;
use frogenx_core::Clock;
use frogenx_layer::{Gpu, LAYER_FORMAT};
use frogenx_output::{MonitorSink, OutputSink};
use frogenx_ui::OutputRequest;
use glam::UVec2;
use gui::Gui;
use stage::Stage;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

/// La résolution de sortie, indépendante de la fenêtre (§8).
const OUTPUT_SIZE: UVec2 = UVec2::new(1920, 1080);
/// La taille du monitor dans l'interface : un quart de la sortie suffit.
const MONITOR_SIZE: UVec2 = UVec2::new(960, 540);
/// 60 fps cible.
const FRAME_PERIOD: Duration = Duration::from_nanos(16_666_667);

const USAGE: &str = "usage : frogenx [--frames N] [--output ECRAN]";

#[derive(Debug, Default, PartialEq, Eq)]
struct Options {
    /// Quitter après N frames : vérifie l'ouverture et la présentation (§8.3).
    frames: Option<u64>,
    /// Ouvre la sortie sur cet écran, compté à partir de 1, dès le lancement :
    /// vérifie la sortie sans souris (§8.4).
    output: Option<usize>,
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
    rate: FrameRate,
    started: Instant,
    last: Instant,
    deadline: Instant,
    frames: u64,
}

impl Running {
    fn start(event_loop: &ActiveEventLoop, sortie: Option<usize>) -> Result<Self, String> {
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
        let grille = frogenx_library::starter();
        let premiere = grille.columns().first().copied();
        let mut engine = Engine::new(gpu, Clock::realtime(), OUTPUT_SIZE, grille);
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

        let maintenant = Instant::now();
        let mut stage = Stage::new(maintenant);
        stage.refresh(event_loop, &window, &mut engine, maintenant);
        if let Some(ecran) = sortie {
            stage.open(event_loop, ecran, &mut engine);
            println!("{}", stage.status());
        }

        Ok(Self {
            window,
            engine,
            gui,
            audio,
            audio_status,
            stage,
            rate: FrameRate::new(FRAME_PERIOD),
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
        let demande = self.gui.draw(
            &self.window,
            &mut self.engine,
            self.rate.fps(),
            &self.audio_status,
            self.stage.view(),
        );
        self.gui.recover(&self.window, self.engine.gpu());
        match demande {
            Some(OutputRequest::Open(ecran)) => {
                self.stage.open(event_loop, ecran, &mut self.engine);
            }
            Some(OutputRequest::Close) => self.stage.close(&mut self.engine),
            None => {}
        }
        self.stage
            .refresh(event_loop, &self.window, &mut self.engine, now);
        self.stage.recover(&self.engine);
        self.last = now;
        self.frames += 1;
        self.deadline = next_deadline(self.deadline, now, FRAME_PERIOD);
    }

    fn on_key(&mut self, key: KeyCode) {
        match key_action(key) {
            Some(Action::Column(i)) => {
                if let Some(column) = self.engine.composition().columns().get(i).copied() {
                    self.engine.push(Command::TriggerColumn { column });
                }
            }
            Some(Action::Tap) => self.engine.push(Command::TapTempo),
            None => {}
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
        match Running::start(event_loop, self.options.output) {
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
            }
            return;
        }
        let garde_par_egui = running.gui.on_event(&running.window, &event);
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
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
    fn frames_fixe_le_nombre_de_frames() {
        assert_eq!(
            parse(&["--frames", "120"]),
            Ok(Options {
                frames: Some(120),
                output: None
            })
        );
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
        assert_eq!(key_action(KeyCode::KeyA), None);
    }

    #[test]
    fn un_argument_inconnu_est_refuse() {
        assert!(parse(&["--plein-ecran"]).is_err());
    }
}
