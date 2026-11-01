//! La sortie plein écran — §8.4 de `docs/ARCHITECTURE.md`.
//!
//! `Preview` ne connaît qu'une surface : ce module possède la seconde fenêtre,
//! suit les écrans branchés, et traduit les demandes de l'interface. Le moteur
//! ne fait que gagner ou perdre une sortie.

use frogenx_app::Engine;
use frogenx_output::{GpuSinkId, Preview, PreviewHandle};
use frogenx_ui::{OutputView, Screen};
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::dpi::{LogicalSize, PhysicalSize};
use winit::event_loop::ActiveEventLoop;
use winit::monitor::MonitorHandle;
use winit::window::{Fullscreen, Window, WindowId};

/// winit n'annonce pas les écrans qui disparaissent : on relit la liste.
const SCREEN_POLL: Duration = Duration::from_secs(1);
/// La fenêtre de sortie sur l'écran de l'interface.
const WINDOWED_SIZE: LogicalSize<f64> = LogicalSize::new(1280.0, 720.0);

const CLOSED: &str = "Sortie fermée";
const SCREEN_LOST: &str = "Sortie fermée : écran débranché";

/// Ce qu'une sortie est sur son écran.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Fullscreen,
    /// Sur l'écran de l'interface : un plein écran y cacherait le menu.
    Windowed,
}

impl Mode {
    fn of(interface: bool) -> Self {
        if interface {
            Self::Windowed
        } else {
            Self::Fullscreen
        }
    }

    /// Une fenêtre normale obéit à l'utilisateur : sa croix la ferme, et elle
    /// garde le focus. Un plein écran ne dépend que du menu : Alt+F4 ne coupe
    /// pas l'image, et le clavier reste à l'interface (§8.4).
    fn is_user_window(self) -> bool {
        self == Self::Windowed
    }
}

/// L'identité d'un écran : son nom s'il en a un, plus stable qu'un handle.
fn identity(name: Option<String>, position: (i32, i32), size: (u32, u32)) -> String {
    name.unwrap_or_else(|| format!("{position:?} {size:?}"))
}

fn monitor_identity(monitor: &MonitorHandle) -> String {
    let position = monitor.position();
    let size = monitor.size();
    identity(
        monitor.name(),
        (position.x, position.y),
        (size.width, size.height),
    )
}

/// Où est cet écran dans la liste. `None` : il a été débranché.
fn screen_index(identities: &[String], screen: &str) -> Option<usize> {
    identities.iter().position(|i| i == screen)
}

fn screen_label(index: usize, size: (u32, u32)) -> String {
    format!("Écran {} ({}×{})", index + 1, size.0, size.1)
}

fn open_status(label: &str) -> String {
    format!("Sortie : {label}")
}

/// La sortie ouverte.
struct Open {
    window: Arc<Window>,
    preview: PreviewHandle,
    sink: GpuSinkId,
    screen: String,
    mode: Mode,
}

/// La seconde fenêtre et les écrans. Tout se passe entre deux frames.
pub struct Stage {
    monitors: Vec<MonitorHandle>,
    identities: Vec<String>,
    screens: Vec<Screen>,
    open: Option<Open>,
    status: String,
    next_poll: Instant,
}

impl Stage {
    pub fn new(now: Instant) -> Self {
        Self {
            monitors: Vec::new(),
            identities: Vec::new(),
            screens: Vec::new(),
            open: None,
            status: CLOSED.into(),
            next_poll: now,
        }
    }

    /// Ce qu'en lit l'interface.
    pub fn view(&self) -> OutputView<'_> {
        let open = self
            .open
            .as_ref()
            .and_then(|o| screen_index(&self.identities, &o.screen));
        OutputView {
            screens: &self.screens,
            open,
            status: &self.status,
            streams: &[],
            streams_available: false,
            render: (0, 0),
            rate: 0,
        }
    }

    pub fn status(&self) -> &str {
        &self.status
    }

    /// Frames présentées par la sortie ouverte.
    pub fn presented(&self) -> Option<u64> {
        self.open.as_ref().map(|o| o.preview.presented())
    }

    pub fn owns(&self, id: WindowId) -> bool {
        self.open.as_ref().is_some_and(|o| o.window.id() == id)
    }

    /// Relit les écrans à l'échéance. Celui de la sortie a disparu : le système
    /// a déplacé la fenêtre, peut-être sur l'interface, donc elle se ferme.
    pub fn refresh(
        &mut self,
        event_loop: &ActiveEventLoop,
        interface: &Window,
        engine: &mut Engine,
        now: Instant,
    ) {
        if now < self.next_poll {
            return;
        }
        self.next_poll = now + SCREEN_POLL;
        let ici = interface.current_monitor().map(|m| monitor_identity(&m));
        self.monitors = event_loop.available_monitors().collect();
        self.identities = self.monitors.iter().map(monitor_identity).collect();
        self.screens = self
            .monitors
            .iter()
            .zip(&self.identities)
            .enumerate()
            .map(|(i, (monitor, identity))| {
                let taille = monitor.size();
                Screen {
                    label: screen_label(i, (taille.width, taille.height)),
                    interface: ici.as_ref() == Some(identity),
                }
            })
            .collect();
        if self.view().open.is_none() && self.open.is_some() {
            self.close(engine);
            self.status = SCREEN_LOST.into();
        }
    }

    /// La fenêtre a bougé : un écran vient peut-être de disparaître.
    pub fn poll_soon(&mut self) {
        self.next_poll = Instant::now();
    }

    /// Ouvre la sortie sur cet écran, en fermant d'abord celle qui l'est. Un
    /// refus reste affiché : le moteur continue (§18.1).
    pub fn open(&mut self, event_loop: &ActiveEventLoop, index: usize, engine: &mut Engine) {
        self.close(engine);
        let (Some(monitor), Some(screen), Some(identity)) = (
            self.monitors.get(index),
            self.screens.get(index),
            self.identities.get(index),
        ) else {
            self.status = format!("Sortie impossible : aucun écran {}", index + 1);
            return;
        };
        let mode = Mode::of(screen.interface);
        match open_window(event_loop, monitor, mode, engine) {
            Ok((window, preview, sink)) => {
                self.status = open_status(&screen.label);
                self.open = Some(Open {
                    window,
                    preview,
                    sink,
                    screen: identity.clone(),
                    mode,
                });
            }
            Err(e) => self.status = format!("Sortie impossible : {e}"),
        }
    }

    /// Retire la sortie du moteur, ce qui lâche sa surface, puis la fenêtre :
    /// celle-ci ne se ferme qu'une fois les deux relâchées (§8.4).
    pub fn close(&mut self, engine: &mut Engine) {
        if let Some(open) = self.open.take() {
            engine.remove_gpu_output(open.sink);
            self.status = CLOSED.into();
        }
    }

    /// `CloseRequested` sur la fenêtre de sortie.
    pub fn close_requested(&mut self, engine: &mut Engine) {
        if self.open.as_ref().is_some_and(|o| o.mode.is_user_window()) {
            self.close(engine);
        }
    }

    /// La sortie ouverte doit-elle rendre le focus qu'elle vient de recevoir ?
    pub fn returns_focus(&self) -> bool {
        self.open.as_ref().is_some_and(|o| !o.mode.is_user_window())
    }

    pub fn resize(&self, taille: PhysicalSize<u32>) {
        if let Some(open) = &self.open {
            open.preview.resize(taille.width, taille.height);
        }
    }

    /// Seul `app` a la fenêtre : c'est lui qui recrée une surface perdue (§8.3).
    pub fn recover(&self, engine: &Engine) {
        let Some(open) = self.open.as_ref().filter(|o| o.preview.surface_lost()) else {
            return;
        };
        let surface = engine
            .gpu()
            .instance
            .create_surface(Arc::clone(&open.window));
        if let Ok(surface) = surface {
            open.preview.replace_surface(surface);
        }
    }
}

/// Une fenêtre, sa surface, et une sortie de plus pour le moteur.
///
/// Non active pour qu'une fenêtre normale laisse le clavier à l'interface. Sous
/// Windows, `set_fullscreen` active quand même la fenêtre : `app` rend le focus
/// à l'interface à l'événement `Focused` (§8.4).
fn open_window(
    event_loop: &ActiveEventLoop,
    monitor: &MonitorHandle,
    mode: Mode,
    engine: &mut Engine,
) -> Result<(Arc<Window>, PreviewHandle, GpuSinkId), String> {
    let attributs = Window::default_attributes()
        .with_title("FrogenX — Sortie")
        .with_active(false);
    let attributs = match mode {
        Mode::Windowed => attributs.with_inner_size(WINDOWED_SIZE),
        Mode::Fullscreen => attributs
            .with_decorations(false)
            .with_position(monitor.position())
            .with_inner_size(monitor.size()),
    };
    let window = Arc::new(
        event_loop
            .create_window(attributs)
            .map_err(|e| e.to_string())?,
    );
    if mode == Mode::Fullscreen {
        window.set_fullscreen(Some(Fullscreen::Borderless(Some(monitor.clone()))));
        window.set_cursor_visible(false);
    }
    let taille = window.inner_size();
    let surface = engine
        .gpu()
        .instance
        .create_surface(Arc::clone(&window))
        .map_err(|e| e.to_string())?;
    let (preview, handle) = Preview::new(engine.gpu(), surface, taille.width, taille.height)
        .map_err(|e| e.to_string())?;
    let sink = engine.add_gpu_output(Box::new(preview));
    Ok((window, handle, sink))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn un_ecran_se_reconnait_a_son_nom_meme_deplace() {
        let nom = || Some(r"\\.\DISPLAY2".to_string());
        assert_eq!(
            identity(nom(), (1920, 0), (1280, 720)),
            identity(nom(), (0, 1080), (1920, 1080))
        );
    }

    #[test]
    fn un_ecran_sans_nom_se_reconnait_a_sa_place() {
        let a = identity(None, (0, 0), (1920, 1080));
        assert_ne!(a, identity(None, (1920, 0), (1920, 1080)));
        assert_eq!(a, identity(None, (0, 0), (1920, 1080)));
    }

    #[test]
    fn un_ecran_debranche_n_est_plus_trouve() {
        let ecrans = [r"\\.\DISPLAY1".to_string(), r"\\.\DISPLAY3".to_string()];
        assert_eq!(screen_index(&ecrans, r"\\.\DISPLAY3"), Some(1));
        assert_eq!(screen_index(&ecrans, r"\\.\DISPLAY2"), None);
    }

    #[test]
    fn le_libelle_numerote_a_partir_de_un_et_donne_la_taille() {
        assert_eq!(screen_label(1, (1050, 1680)), "Écran 2 (1050×1680)");
        assert_eq!(open_status("Écran 2"), "Sortie : Écran 2");
    }

    #[test]
    fn seul_l_ecran_de_l_interface_garde_une_fenetre_normale() {
        assert_eq!(Mode::of(true), Mode::Windowed);
        assert_eq!(Mode::of(false), Mode::Fullscreen);
    }

    #[test]
    fn alt_f4_et_le_focus_n_atteignent_pas_le_plein_ecran() {
        assert!(!Mode::Fullscreen.is_user_window());
        assert!(Mode::Windowed.is_user_window());
    }
}
