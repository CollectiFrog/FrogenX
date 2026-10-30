//! La passe de conversion vers 8 bits sRGB — §8.0, §8.3, §21 de
//! `docs/ARCHITECTURE.md`.

use frogenx_layer::Gpu;

/// Ce que la passe fait de l'alpha et de la résolution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConversionMode {
    /// Pour une `CpuFrame` : alpha droit, lecture au pixel près (§8.0).
    Straight,
    /// Pour une cible opaque : le RGB prémultiplié se lit sur du noir, et la
    /// source est filtrée pour être mise à l'échelle (§8.3).
    OverBlack,
}

/// Un rectangle de la cible, en pixels entiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Convertit la sortie du compositeur vers une cible 8 bits.
///
/// Une seule passe pour tous les consommateurs : l'encodage sRGB n'existe qu'à
/// un endroit. Elle suit le format de sa cible : une cible `*Srgb` laisse le
/// matériel encoder, toute autre est encodée par le shader. Les deux donnent
/// la même image, sans double encodage.
pub struct ConversionPass {
    mode: ConversionMode,
    pipeline: wgpu::RenderPipeline,
    bind_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
}

impl ConversionPass {
    pub fn new(gpu: &Gpu, target: wgpu::TextureFormat, mode: ConversionMode) -> Self {
        let bind_layout = gpu
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("conversion"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });
        // Au plus proche, une lecture à la même taille rend chaque pixel tel
        // quel. Bilinéaire, une mise à l'échelle reste propre (§8.3).
        let filter = match mode {
            ConversionMode::Straight => wgpu::FilterMode::Nearest,
            ConversionMode::OverBlack => wgpu::FilterMode::Linear,
        };
        let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("conversion"),
            mag_filter: filter,
            min_filter: filter,
            ..Default::default()
        });
        let shader = gpu
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some("conversion.wgsl"),
                source: wgpu::ShaderSource::Wgsl(include_str!("conversion.wgsl").into()),
            });
        let layout = gpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("conversion"),
                bind_group_layouts: &[Some(&bind_layout)],
                immediate_size: 0,
            });
        let constantes = [
            ("ENCODE_SRGB", shader_encodes(target) as u32 as f64),
            (
                "STRAIGHT_ALPHA",
                (mode == ConversionMode::Straight) as u32 as f64,
            ),
        ];
        let pipeline = gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("conversion"),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    compilation_options: wgpu::PipelineCompilationOptions {
                        constants: &constantes,
                        ..Default::default()
                    },
                    targets: &[Some(wgpu::ColorTargetState {
                        format: target,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            });
        Self {
            mode,
            pipeline,
            bind_layout,
            sampler,
        }
    }

    /// Enregistre la passe. Sans `viewport`, l'image couvre toute la cible ;
    /// avec, le reste de la cible reste à sa couleur de fond.
    pub fn encode(
        &self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        source: &wgpu::Texture,
        target: &wgpu::TextureView,
        viewport: Option<Rect>,
    ) {
        let vue = source.create_view(&Default::default());
        let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("conversion"),
            layout: &self.bind_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&vue),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        let fond = match self.mode {
            ConversionMode::Straight => wgpu::Color::TRANSPARENT,
            ConversionMode::OverBlack => wgpu::Color::BLACK,
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("conversion"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(fond),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        if let Some(r) = viewport {
            pass.set_viewport(
                r.x as f32,
                r.y as f32,
                r.width as f32,
                r.height as f32,
                0.0,
                1.0,
            );
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &bind, &[]);
        pass.draw(0..3, 0..1);
    }
}

/// Le shader encode en sRGB si le matériel ne le fait pas pour cette cible.
fn shader_encodes(target: wgpu::TextureFormat) -> bool {
    !target.is_srgb()
}

#[cfg(test)]
mod tests {
    use super::*;
    use wgpu::TextureFormat as F;

    #[test]
    fn le_shader_n_encode_jamais_une_cible_que_le_materiel_encode() {
        assert!(!shader_encodes(F::Rgba8UnormSrgb));
        assert!(!shader_encodes(F::Bgra8UnormSrgb));
        assert!(shader_encodes(F::Rgba8Unorm));
        assert!(shader_encodes(F::Bgra8Unorm));
    }
}
