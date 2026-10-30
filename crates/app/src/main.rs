//! FrogenX — l'application : `cargo run`.
//!
//! Le binaire possède la fenêtre et la boucle d'événements ; tout le reste est
//! dans `Engine`, qui ne connaît pas de fenêtre (§8.3 de
//! `docs/ARCHITECTURE.md`). Le moteur se cadence sur sa propre échéance : la
//! preview n'est qu'une sortie, et la masquer n'arrête rien.
//!
//! ```bash
//! cargo run                  # la fenêtre, jusqu'à sa fermeture
//! cargo run -- --frames 120  # 120 frames, un bilan, puis quitte
//! ```

use frogenx_app::{demo, next_deadline, Engine};
use frogenx_core::Clock;
use frogenx_layer::{Gpu, LAYER_FORMAT};
use frogenx_output::{OutputSink, Preview, PreviewHandle};
use glam::UVec2;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};

/// La résolution de sortie, indépendante de la fenêtre (§8).
const OUTPUT_SIZE: UVec2 = UVec2::new(1920, 1080);
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

/// Ce qui n'existe qu'une fois la fenêtre ouverte.
struct Running {
    window: Arc<Window>,
    engine: Engine,
    preview: PreviewHandle,
    started: Instant,
    last: Instant,
    deadline: Instant,
    frames: u64,
}

impl Running {
    fn start(event_loop: &ActiveEventLoop) -> Result<Self, String> {
        let attributs = Window::default_attributes()
            .with_title("FrogenX")
            .with_inner_size(LogicalSize::new(1280.0, 720.0));
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

        let taille = window.inner_size();
        let (preview, handle) = Preview::new(&gpu, surface, taille.width, taille.height)
            .map_err(|e| format!("preview impossible : {e}"))?;
        let scene = demo::scene(&gpu, OUTPUT_SIZE);
        let mut engine = Engine::new(gpu, Clock::realtime(), OUTPUT_SIZE, vec![scene]);
        engine
            .add_output(OutputSink::Gpu(Box::new(preview)))
            .map_err(|e| e.to_string())?;

        let maintenant = Instant::now();
        Ok(Self {
            window,
            engine,
            preview: handle,
            started: maintenant,
            last: maintenant,
            deadline: maintenant,
            frames: 0,
        })
    }

    fn frame(&mut self, now: Instant) {
        self.engine.frame((now - self.last).as_secs_f64());
        self.last = now;
        self.frames += 1;
        self.deadline = next_deadline(self.deadline, now, FRAME_PERIOD);

        // Seul `app` a la fenêtre : c'est donc lui qui recrée une surface perdue.
        if self.preview.surface_lost() {
            let instance = &self.engine.gpu().instance;
            if let Ok(surface) = instance.create_surface(Arc::clone(&self.window)) {
                self.preview.replace_surface(surface);
            }
        }
    }

    fn report(&self) {
        let duree = self.started.elapsed().as_secs_f64();
        println!(
            "{} frames en {duree:.2} s ({:.1} fps), {} présentées — sorties : {:?}",
            self.frames,
            self.frames as f64 / duree,
            self.preview.presented(),
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
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(taille) => {
                if let Some(running) = &self.running {
                    running.preview.resize(taille.width, taille.height);
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
    fn un_argument_inconnu_est_refuse() {
        assert!(parse(&["--plein-ecran"]).is_err());
    }
}
