//! egui dans la fenêtre — §23.5 de `docs/ARCHITECTURE.md`.
//!
//! `frogenx-ui` décrit l'interface sans connaître la fenêtre ni le GPU. Ce module
//! fait le lien : les événements `winit` vers egui, le dessin vers la surface.

use frogenx_app::{describe_outputs, Engine};
use frogenx_layer::Gpu;
use frogenx_output::{FormatPreference, PresentSurface};
use frogenx_ui::{theme, FrameView, Monitor, OutputRequest, OutputView, UiState};
use std::sync::Arc;
use std::time::{Duration, Instant};
use winit::event::WindowEvent;
use winit::window::Window;

pub struct Gui {
    ctx: egui::Context,
    winit: egui_winit::State,
    renderer: egui_wgpu::Renderer,
    surface: PresentSurface,
    state: UiState,
    monitor: Monitor,
    pub presented: u64,
    /// Temps passé dans l'interface : elle partage le thread du rendu (§23.5).
    pub spent: Duration,
}

impl Gui {
    pub fn new(
        window: &Window,
        gpu: &Gpu,
        surface: wgpu::Surface<'static>,
        monitor: &wgpu::TextureView,
        aspect: f32,
    ) -> Result<Self, String> {
        let taille = window.inner_size();
        let surface = PresentSurface::new(
            gpu,
            surface,
            taille.width,
            taille.height,
            FormatPreference::Unorm,
        )
        .map_err(|e| format!("surface de l'interface impossible : {e}"))?;

        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let winit = egui_winit::State::new(
            ctx.clone(),
            egui::ViewportId::ROOT,
            window,
            Some(window.scale_factor() as f32),
            window.theme(),
            Some(gpu.device.limits().max_texture_dimension_2d as usize),
        );
        let mut renderer =
            egui_wgpu::Renderer::new(&gpu.device, surface.format(), Default::default());
        let texture =
            renderer.register_native_texture(&gpu.device, monitor, wgpu::FilterMode::Linear);
        Ok(Self {
            ctx,
            winit,
            renderer,
            surface,
            state: UiState::default(),
            monitor: Monitor { texture, aspect },
            presented: 0,
            spent: Duration::ZERO,
        })
    }

    /// Transmet un événement à egui. Rend `true` s'il le garde pour lui :
    /// Espace dans un champ ne doit pas taper le tempo.
    pub fn on_event(&mut self, window: &Window, event: &WindowEvent) -> bool {
        self.winit.on_window_event(window, event).consumed
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.surface.resize(width, height);
    }

    /// Dessine l'interface d'après la dernière frame du moteur, et lui transmet
    /// les commandes émises. Elles s'appliqueront à la frame suivante (§23).
    /// Rend ce que l'interface demande à la fenêtre de sortie (§8.4).
    pub fn draw(
        &mut self,
        window: &Window,
        engine: &mut Engine,
        fps: f32,
        audio_status: &str,
        sortie: OutputView<'_>,
    ) -> Option<OutputRequest> {
        let debut = Instant::now();
        let input = self.winit.take_egui_input(window);
        let sorties = describe_outputs(&engine.output_statuses());
        let view = FrameView {
            composition: engine.composition(),
            bpm: engine.bpm(),
            fps,
            outputs: &sorties,
            monitor: Some(self.monitor),
            spectrum: &engine.audio().spectrum,
            audio_status,
            output: sortie,
        };
        let mut commands = Vec::new();
        let mut placements = Vec::new();
        let mut demande = None;
        let state = &mut self.state;
        let output = self.ctx.run_ui(input, |ui| {
            // egui peut rejouer une passe : seule la dernière compte, sinon
            // un clic se déclencherait deux fois.
            commands.clear();
            placements.clear();
            demande = None;
            frogenx_ui::draw(
                ui,
                state,
                &view,
                &mut commands,
                &mut placements,
                &mut demande,
            );
        });
        self.winit
            .handle_platform_output(window, output.platform_output);
        for command in commands {
            engine.push(command);
        }
        // L'interface désigne, `app` construit (§23.6).
        for p in placements {
            if let Some(preset) = frogenx_library::preset(p.preset) {
                engine.place(p.layer, p.column, preset.build());
            }
        }

        let gpu = engine.gpu();
        // Vidés, pas seulement lus : egui vérifie qu'ils ont été traités, y
        // compris sur une frame sautée.
        let mut textures = output.textures_delta;
        for (id, deltas) in textures.set.drain() {
            for delta in &deltas {
                self.renderer
                    .update_texture(&gpu.device, &gpu.queue, id, delta);
            }
        }
        let primitives = self.ctx.tessellate(output.shapes, output.pixels_per_point);
        if let Some(sortie) = self.surface.acquire(gpu) {
            let (width, height) = self.surface.size();
            let ecran = egui_wgpu::ScreenDescriptor {
                size_in_pixels: [width, height],
                pixels_per_point: output.pixels_per_point,
            };
            let mut encoder = gpu
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("interface"),
                });
            let tampons = self.renderer.update_buffers(
                &gpu.device,
                &gpu.queue,
                &mut encoder,
                &primitives,
                &ecran,
            );
            let vue = sortie.texture.create_view(&Default::default());
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("interface"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &vue,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            let mut pass = pass.forget_lifetime();
            self.renderer.render(&mut pass, &primitives, &ecran);
            drop(pass);
            gpu.queue
                .submit(tampons.into_iter().chain([encoder.finish()]));
            gpu.queue.present(sortie);
            self.presented += 1;
        }
        for id in textures.free.drain() {
            self.renderer.free_texture(&id);
        }
        self.spent += debut.elapsed();
        demande
    }

    /// Seul `app` a la fenêtre : c'est donc lui qui recrée une surface perdue.
    pub fn recover(&mut self, window: &Arc<Window>, gpu: &Gpu) {
        if !self.surface.is_lost() {
            return;
        }
        if let Ok(surface) = gpu.instance.create_surface(Arc::clone(window)) {
            // Le renderer d'egui reste valable tant que le format ne change
            // pas ; un refus laisse la surface perdue, retentée à la frame
            // suivante.
            let _ = self.surface.replace(gpu, surface);
        }
    }
}
