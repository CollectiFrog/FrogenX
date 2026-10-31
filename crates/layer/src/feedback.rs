//! Le rendu d'un feedback — §6.1, §22 de `docs/ARCHITECTURE.md`.
//!
//! Il lit [`FrameHistory`], une copie de l'image finale de la frame d'avant, et
//! écrit sa propre cible : lire et écrire la même texture est interdit (§22).

use crate::gpu::{Gpu, LAYER_FORMAT};
use bytemuck::{Pod, Zeroable};
use frogenx_shape::FeedbackStep;
use glam::UVec2;

/// La copie de l'image finale. `Engine` l'écrit à la fin de chaque frame.
pub struct FrameHistory {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

impl FrameHistory {
    pub fn new(gpu: &Gpu, size: UVec2) -> Self {
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("feedback.history"),
            size: extent(size),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: LAYER_FORMAT,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        Self { texture, view }
    }

    pub fn view(&self) -> &wgpu::TextureView {
        &self.view
    }

    /// Enregistre la copie de `source`, l'image finale de cette frame.
    pub fn record(&self, encoder: &mut wgpu::CommandEncoder, source: &wgpu::Texture) {
        encoder.copy_texture_to_texture(
            source.as_image_copy(),
            self.texture.as_image_copy(),
            self.texture.size(),
        );
    }
}

fn extent(size: UVec2) -> wgpu::Extent3d {
    wgpu::Extent3d {
        width: size.x.max(1),
        height: size.y.max(1),
        depth_or_array_layers: 1,
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
struct Uniforms {
    row0: [f32; 2],
    row1: [f32; 2],
    offset: [f32; 2],
    extent: [f32; 2],
    size: [f32; 2],
    keep: f32,
    _pad: f32,
}

/// Le demi-côté de l'image dans l'espace du §17 : `[-1, 1]` sur le petit côté.
fn half_extent(size: UVec2) -> [f32; 2] {
    let (w, h) = (size.x.max(1) as f32, size.y.max(1) as f32);
    if w >= h {
        [w / h, 1.0]
    } else {
        [1.0, h / w]
    }
}

fn uniforms(step: &FeedbackStep, size: UVec2) -> Uniforms {
    let (sin, cos) = (-step.angle).sin_cos();
    Uniforms {
        row0: [cos / step.scale, -sin / step.scale],
        row1: [sin / step.scale, cos / step.scale],
        offset: step.offset,
        extent: half_extent(size),
        size: [size.x.max(1) as f32, size.y.max(1) as f32],
        keep: step.keep,
        _pad: 0.0,
    }
}

/// Un par piste, créé à son premier feedback : comme [`crate::ShapeRenderer`], il
/// ne garde rien de son clip d'une frame à l'autre.
pub struct FeedbackRenderer {
    size: UVec2,
    pipeline: wgpu::RenderPipeline,
    bind_group: wgpu::BindGroup,
    uniforms: wgpu::Buffer,
    #[allow(dead_code)] // tenue vivante : `view` en dépend
    target: wgpu::Texture,
    view: wgpu::TextureView,
}

impl FeedbackRenderer {
    pub fn new(gpu: &Gpu, size: UVec2, history: &FrameHistory) -> Self {
        let shader = gpu
            .device
            .create_shader_module(wgpu::include_wgsl!("feedback.wgsl"));
        let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("feedback.target"),
            size: extent(size),
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: LAYER_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = target.create_view(&Default::default());
        let uniforms = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("feedback.params"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("feedback.sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let pipeline = gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("feedback"),
                layout: None,
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    compilation_options: Default::default(),
                    targets: &[Some(LAYER_FORMAT.into())],
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            });
        let bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("feedback"),
            layout: &pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(history.view()),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: uniforms.as_entire_binding(),
                },
            ],
        });
        Self {
            size,
            pipeline,
            bind_group,
            uniforms,
            target,
            view,
        }
    }

    pub fn prepare(&mut self, step: &FeedbackStep, gpu: &Gpu) {
        let u = uniforms(step, self.size);
        gpu.queue
            .write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&u));
    }

    pub fn encode(&self, encoder: &mut wgpu::CommandEncoder) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("feedback"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &self.view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    pub fn output(&self) -> &wgpu::TextureView {
        &self.view
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STEP: FeedbackStep = FeedbackStep {
        scale: 2.0,
        angle: 0.5,
        offset: [0.25, -0.125],
        keep: 0.75,
    };

    /// Le calcul du shader, en Rust.
    fn par_le_shader(step: &FeedbackStep, p: [f32; 2]) -> [f32; 2] {
        let u = uniforms(step, UVec2::new(1920, 1080));
        let d = [p[0] - u.offset[0], p[1] - u.offset[1]];
        [
            u.row0[0] * d[0] + u.row0[1] * d[1],
            u.row1[0] * d[0] + u.row1[1] * d[1],
        ]
    }

    #[test]
    fn les_uniformes_donnent_le_calcul_de_reference() {
        for p in [[0.0, 0.0], [1.0, 0.5], [-0.75, 0.25]] {
            let (a, b) = (par_le_shader(&STEP, p), STEP.source_of(p));
            assert!((a[0] - b[0]).abs() < 1e-5 && (a[1] - b[1]).abs() < 1e-5);
        }
    }

    #[test]
    fn le_petit_cote_vaut_un() {
        assert_eq!(half_extent(UVec2::new(1920, 1080))[1], 1.0);
        assert_eq!(half_extent(UVec2::new(1080, 1920))[0], 1.0);
        assert_eq!(half_extent(UVec2::new(0, 0)), [1.0, 1.0]);
    }

    #[test]
    fn la_structure_respecte_l_alignement_du_shader() {
        assert_eq!(std::mem::size_of::<Uniforms>(), 48);
    }
}
