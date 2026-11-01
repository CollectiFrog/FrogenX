//! La fenêtre de l'éditeur de nœuds — §23.10 de `docs/ARCHITECTURE.md`.
//!
//! Son propre contexte egui, sa propre surface : fermée, elle ne coûte rien.

use crate::gui::paint;
use frogenx_app::Engine;
use frogenx_composition::{ClipId, IdMap};
use frogenx_layer::Gpu;
use frogenx_output::{FormatPreference, PresentSurface};
use frogenx_ui::nodes::{editor, EditorState, EditorView};
use frogenx_ui::{theme, HistoryRequest};
use std::sync::Arc;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowId};

/// La taille d'ouverture : assez large pour une colonne de destinations.
const SIZE: LogicalSize<f64> = LogicalSize::new(1280.0, 800.0);

/// L'éditeur ouvert.
pub struct NodeWindow {
    window: Arc<Window>,
    ctx: egui::Context,
    winit: egui_winit::State,
    renderer: egui_wgpu::Renderer,
    surface: PresentSurface,
    state: EditorState,
    pub presented: u64,
}

impl NodeWindow {
    pub fn open(event_loop: &ActiveEventLoop, gpu: &Gpu) -> Result<Self, String> {
        let attributs = Window::default_attributes()
            .with_title("FrogenX — Nœuds")
            .with_inner_size(SIZE);
        let window = Arc::new(
            event_loop
                .create_window(attributs)
                .map_err(|e| e.to_string())?,
        );
        let taille = window.inner_size();
        let surface = gpu
            .instance
            .create_surface(Arc::clone(&window))
            .map_err(|e| e.to_string())?;
        let surface = PresentSurface::new(
            gpu,
            surface,
            taille.width,
            taille.height,
            FormatPreference::Unorm,
        )
        .map_err(|e| e.to_string())?;

        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let winit = egui_winit::State::new(
            ctx.clone(),
            egui::ViewportId::ROOT,
            &window,
            Some(window.scale_factor() as f32),
            window.theme(),
            Some(gpu.device.limits().max_texture_dimension_2d as usize),
        );
        let renderer = egui_wgpu::Renderer::new(&gpu.device, surface.format(), Default::default());
        Ok(Self {
            window,
            ctx,
            winit,
            renderer,
            surface,
            state: EditorState::default(),
            presented: 0,
        })
    }

    pub fn owns(&self, id: WindowId) -> bool {
        self.window.id() == id
    }

    pub fn focus(&self) {
        self.window.focus_window();
    }

    /// Rend `true` si la fenêtre demande à se fermer.
    pub fn on_event(&mut self, event: &WindowEvent) -> bool {
        let _ = self.winit.on_window_event(&self.window, event);
        match event {
            WindowEvent::CloseRequested => return true,
            WindowEvent::Resized(taille) => self.surface.resize(taille.width, taille.height),
            _ => {}
        }
        false
    }

    pub fn take_history_request(&mut self) -> Option<HistoryRequest> {
        self.state.history.take()
    }

    pub fn pointer_held(&self) -> bool {
        self.ctx.input(|i| i.pointer.any_down())
    }

    /// La grille a été restaurée : la place des nœuds suit.
    pub fn remap(&mut self, map: &IdMap) {
        self.state.remap(map);
    }

    pub fn forget_grid(&mut self) {
        self.state.forget_grid();
    }

    /// Dessine l'éditeur d'après la dernière frame, et transmet ses commandes.
    pub fn draw(&mut self, engine: &mut Engine, selected: Option<ClipId>) {
        let input = self.winit.take_egui_input(&self.window);
        let view = EditorView {
            composition: engine.composition(),
            selected,
        };
        let mut commands = Vec::new();
        let state = &mut self.state;
        let mut output = self.ctx.run_ui(input, |ui| {
            // Comme l'interface : seule la dernière passe compte.
            commands.clear();
            editor(ui, state, &view, &mut commands);
        });
        self.winit
            .handle_platform_output(&self.window, std::mem::take(&mut output.platform_output));
        for command in commands {
            engine.push(command);
        }
        let presente = paint(
            &self.ctx,
            &mut self.renderer,
            &mut self.surface,
            engine.gpu(),
            output,
        );
        self.presented += u64::from(presente);
        if self.surface.is_lost() {
            let gpu = engine.gpu();
            if let Ok(surface) = gpu.instance.create_surface(Arc::clone(&self.window)) {
                let _ = self.surface.replace(gpu, surface);
            }
        }
    }
}
