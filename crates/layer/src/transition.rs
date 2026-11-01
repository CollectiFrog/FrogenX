//! Mélanger deux clips d'une piste — §3bis.2 de `docs/ARCHITECTURE.md`.
//!
//! Avant le compositeur : deux couches d'opacités croisées creuseraient l'image à
//! mi-fondu. La texture mélangée le rejoint comme un seul clip.

use crate::gpu::{Gpu, LAYER_FORMAT};
use bytemuck::{Pod, Zeroable};
use frogenx_shape::MixKind;
use glam::UVec2;

/// La demi-largeur du bord d'un volet, en part de l'image : assez pour ne pas
/// créneler, trop peu pour se voir comme un dégradé.
pub const WIPE_EDGE: f32 = 0.02;

/// Ce que le clip entrant couvre au point `uv` (`[0, 1]`, l'origine en haut à
/// gauche), pour une transition avancée de `amount`. Le shader fait le même
/// calcul : cette version lui sert de référence dans les tests.
pub fn mix_coverage(kind: MixKind, uv: [f32; 2], amount: f32) -> f32 {
    let position = match kind {
        MixKind::Fade => return amount,
        MixKind::WipeLeft => 1.0 - uv[0],
        MixKind::WipeRight => uv[0],
        MixKind::WipeUp => 1.0 - uv[1],
        MixKind::WipeDown => uv[1],
    };
    let front = amount * (1.0 + 2.0 * WIPE_EDGE) - WIPE_EDGE;
    1.0 - ((position - front + WIPE_EDGE) / (2.0 * WIPE_EDGE)).clamp(0.0, 1.0)
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
struct Uniforms {
    amount: f32,
    kind: u32,
    size: [f32; 2],
}

/// Un par piste, créé à sa première transition.
pub struct MixRenderer {
    size: UVec2,
    pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    #[allow(dead_code)] // tenue vivante : `view` en dépend
    target: wgpu::Texture,
    view: wgpu::TextureView,
    bind_group: Option<wgpu::BindGroup>,
}

impl MixRenderer {
    pub fn new(gpu: &Gpu, size: UVec2) -> Self {
        let shader = gpu
            .device
            .create_shader_module(wgpu::include_wgsl!("transition.wgsl"));
        let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("transition.target"),
            size: wgpu::Extent3d {
                width: size.x.max(1),
                height: size.y.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: LAYER_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });
        let view = target.create_view(&Default::default());
        let uniforms = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("transition.params"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let pipeline = gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("transition"),
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
        Self {
            size,
            pipeline,
            uniforms,
            target,
            view,
            bind_group: None,
        }
    }

    pub fn prepare(
        &mut self,
        gpu: &Gpu,
        incoming: &wgpu::TextureView,
        outgoing: &wgpu::TextureView,
        amount: f32,
        kind: MixKind,
    ) {
        let u = Uniforms {
            amount,
            kind: kind as u32,
            size: [self.size.x.max(1) as f32, self.size.y.max(1) as f32],
        };
        gpu.queue
            .write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&u));
        self.bind_group = Some(gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("transition"),
            layout: &self.pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(incoming),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(outgoing),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: self.uniforms.as_entire_binding(),
                },
            ],
        }));
    }

    pub fn encode(&self, encoder: &mut wgpu::CommandEncoder) {
        let Some(bind_group) = &self.bind_group else {
            return;
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("transition"),
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
        pass.set_bind_group(0, bind_group, &[]);
        pass.draw(0..3, 0..1);
    }

    pub fn output(&self) -> &wgpu::TextureView {
        &self.view
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VOLETS: [MixKind; 4] = [
        MixKind::WipeLeft,
        MixKind::WipeRight,
        MixKind::WipeUp,
        MixKind::WipeDown,
    ];

    #[test]
    fn un_fondu_couvre_partout_pareil() {
        for uv in [[0.0, 0.0], [0.5, 0.25], [1.0, 1.0]] {
            assert_eq!(mix_coverage(MixKind::Fade, uv, 0.3), 0.3);
        }
    }

    #[test]
    fn un_volet_part_de_rien_et_finit_sur_tout() {
        for kind in VOLETS {
            for uv in [[0.0, 0.0], [0.5, 0.5], [1.0, 1.0], [0.0, 1.0]] {
                assert_eq!(mix_coverage(kind, uv, 0.0), 0.0, "{kind:?} {uv:?}");
                let fin = mix_coverage(kind, uv, 1.0);
                assert!((fin - 1.0).abs() < 1e-5, "{kind:?} {uv:?}");
            }
        }
    }

    #[test]
    fn un_volet_entre_du_cote_oppose_a_sa_direction() {
        let a = 0.25;
        assert_eq!(
            mix_coverage(MixKind::WipeLeft, [0.9, 0.5], a),
            1.0,
            "par la droite"
        );
        assert_eq!(mix_coverage(MixKind::WipeLeft, [0.1, 0.5], a), 0.0);
        assert_eq!(
            mix_coverage(MixKind::WipeRight, [0.1, 0.5], a),
            1.0,
            "par la gauche"
        );
        assert_eq!(mix_coverage(MixKind::WipeRight, [0.9, 0.5], a), 0.0);
        assert_eq!(
            mix_coverage(MixKind::WipeUp, [0.5, 0.9], a),
            1.0,
            "par le bas"
        );
        assert_eq!(mix_coverage(MixKind::WipeUp, [0.5, 0.1], a), 0.0);
        assert_eq!(
            mix_coverage(MixKind::WipeDown, [0.5, 0.1], a),
            1.0,
            "par le haut"
        );
        assert_eq!(mix_coverage(MixKind::WipeDown, [0.5, 0.9], a), 0.0);
    }

    #[test]
    fn le_bord_d_un_volet_est_adouci_et_etroit() {
        let milieu = mix_coverage(MixKind::WipeRight, [0.5, 0.5], 0.5);
        assert!((milieu - 0.5).abs() < 1e-5, "{milieu}");
        let loin = 0.5 + 2.0 * WIPE_EDGE;
        assert_eq!(mix_coverage(MixKind::WipeRight, [loin, 0.5], 0.5), 0.0);
    }

    #[test]
    fn la_structure_respecte_l_alignement_du_shader() {
        assert_eq!(std::mem::size_of::<Uniforms>(), 16);
    }

    #[test]
    fn les_indices_sont_ceux_du_shader() {
        assert_eq!(MixKind::Fade as u32, 0);
        assert_eq!(MixKind::WipeLeft as u32, 1);
        assert_eq!(MixKind::WipeRight as u32, 2);
        assert_eq!(MixKind::WipeUp as u32, 3);
        assert_eq!(MixKind::WipeDown as u32, 4);
    }
}
