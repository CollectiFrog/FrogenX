//! Le monitor : l'image dans l'interface — §8.3 de `docs/ARCHITECTURE.md`.

use crate::conversion::{ConversionMode, ConversionPass};
use crate::preview::letterbox;
use crate::sink::{GpuFrame, GpuSink, SinkStatus};
use frogenx_layer::Gpu;

/// Imposé par egui (`register_native_texture`), qui lit cette texture sans la
/// décoder : le shader de la passe y écrit donc des valeurs encodées en sRGB.
pub const MONITOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Écrit chaque frame dans une texture de taille réduite, que l'interface
/// affiche comme une image.
///
/// La texture du compositeur ne convient pas : elle est linéaire en 16 bits,
/// et elle change d'une frame à l'autre (ping-pong, §22).
pub struct MonitorSink {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    pass: ConversionPass,
}

impl MonitorSink {
    pub fn new(gpu: &Gpu, width: u32, height: u32) -> Self {
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("monitor"),
            size: wgpu::Extent3d {
                width: width.max(1),
                height: height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: MONITOR_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let pass = ConversionPass::new(gpu, MONITOR_FORMAT, ConversionMode::OverBlack);
        Self {
            texture,
            view,
            pass,
        }
    }

    /// La vue à enregistrer auprès d'egui. La texture ne change jamais.
    pub fn view(&self) -> wgpu::TextureView {
        self.view.clone()
    }

    pub fn texture(&self) -> wgpu::Texture {
        self.texture.clone()
    }
}

impl GpuSink for MonitorSink {
    fn encode(&mut self, gpu: &Gpu, encoder: &mut wgpu::CommandEncoder, frame: &GpuFrame<'_>) {
        let rect = letterbox(
            (frame.texture.width(), frame.texture.height()),
            (self.texture.width(), self.texture.height()),
        );
        self.pass
            .encode(gpu, encoder, frame.texture, &self.view, Some(rect));
    }

    fn after_submit(&mut self, _gpu: &Gpu) {}

    fn status(&self) -> SinkStatus {
        SinkStatus::Active
    }
}
