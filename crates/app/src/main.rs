//! FrogenX — l'application : `cargo run`.
//!
//! Le binaire possède la fenêtre et la boucle d'événements ; tout le reste est
//! dans `Engine`, qui ne connaît pas de fenêtre, et dans `frogenx-ui`, qui
//! décrit l'interface (§8.3, §23.5 de `docs/ARCHITECTURE.md`). Le moteur se
//! cadence sur sa propre échéance : masquer la fenêtre n'arrête rien.
//!
//! ```bash
//! cargo run                  # la fenêtre, jusqu'à sa fermeture
//! cargo run -- --frames 120  # 120 frames, un bilan, puis quitte
//! ```
//!
//! Au clavier, en plus de l'interface : les touches 1 à 8 déclenchent les
//! colonnes de la grille, Espace tape le tempo.

mod gui;

use frogenx_app::{next_deadline, Engine, FrameRate};
use frogenx_composition::Command;
use frogenx_core::Clock;
use frogenx_layer::{Gpu, LAYER_FORMAT};
use frogenx_output::{MonitorSink, OutputSink};
use glam::UVec2;
use gui::Gui;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

/// La résolution de sortie, indépendante de la fenêtre (§8).
const OUTPUT_SIZE: UVec2 = UVec2::new(1920, 1080);
/// La taille du monitor dans l'interface : un quart de la sortie suffit.
const MONITOR_SIZE: UVec2 = UVec2::new(960, 540);
/// 60 fps cible.
const FRAME_PERIOD: Duration = Duration::from_nanos(16_666_667);

const USAGE: &str = "usage : frogenx [--frames N]";

#[derive(Debug, Default, PartialEq, Eq)]
struct Options {
    /// Quitter après N frames : vérifie l'ouverture et la présentation (§8.3).
    frames: Option<u64>,
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

/// Ce qui n'existe qu'une fois la fenêtre ouverte.
struct Running {
    window: Arc<Window>,
    engine: Engine,
    gui: Gui,
    rate: FrameRate,
    started: Instant,
    last: Instant,
    deadline: Instant,
    frames: u64,
}

impl Running {
    fn start(event_loop: &ActiveEventLoop) -> Result<Self, String> {
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

        let maintenant = Instant::now();
        Ok(Self {
            window,
            engine,
            gui,
            rate: FrameRate::new(FRAME_PERIOD),
            started: maintenant,
            last: maintenant,
            deadline: maintenant,
            frames: 0,
        })
    }

    fn frame(&mut self, now: Instant) {
        let dt = (now - self.last).as_secs_f64();
        self.rate.tick(dt);
        self.engine.frame(dt);
        self.gui
            .draw(&self.window, &mut self.engine, self.rate.fps());
        self.gui.recover(&self.window, self.engine.gpu());
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
        match Running::start(event_loop) {
            Ok(running) => self.running = Some(running),
            Err(e) => {
                self.failure = Some(e);
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let garde_par_egui = match &mut self.running {
            Some(running) => running.gui.on_event(&running.window, &event),
            None => false,
        };
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(taille) => {
                if let Some(running) = &mut self.running {
                    running.gui.resize(taille.width, taille.height);
                }
            }
            WindowEvent::KeyboardInput { event, .. } => {
                if let (true, false, PhysicalKey::Code(key), Some(running)) = (
                    event.state.is_pressed() && !garde_par_egui,
                    event.repeat,
                    event.physical_key,
                    &mut self.running,
                ) {
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
            running.frame(maintenant);
        }
        if self.options.frames.is_some_and(|n| running.frames >= n) {
            running.report();
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
        assert_eq!(parse(&[]), Ok(Options { frames: None }));
    }

    #[test]
    fn frames_fixe_le_nombre_de_frames() {
        assert_eq!(
            parse(&["--frames", "120"]),
            Ok(Options { frames: Some(120) })
        );
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
