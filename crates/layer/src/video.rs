//! Le rendu d'un clip vidéo — §9.3 de `docs/ARCHITECTURE.md`.
//!
//! Reçoit des plans YUV 4:2:0, sans savoir d'où ils viennent : `app` les prend dans
//! `crates/video`. Trois textures et une conversion en shader : du RGBA coûterait
//! 2,7 fois plus à envoyer, à chaque image.

use crate::gpu::{Gpu, LAYER_FORMAT};
use bytemuck::{Pod, Zeroable};
use frogenx_shape::VideoStep;
use glam::UVec2;

/// Une image à afficher, le temps d'un appel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VideoImage<'a> {
    pub width: u32,
    pub height: u32,
    /// Les plans Y, U et V, à la suite.
    pub data: &'a [u8],
    /// Son instant : à image identique, rien n'est renvoyé au GPU.
    pub time: f64,
}

/// Du YUV BT.709 en plage limitée vers du RGB linéaire. Le shader fait le même
/// calcul : cette version lui sert de référence dans les tests.
pub fn yuv_to_linear(y: u8, u: u8, v: u8) -> [f32; 3] {
    let y = (y as f32 - 16.0) / 219.0;
    let u = (u as f32 - 128.0) / 224.0;
    let v = (v as f32 - 128.0) / 224.0;
    let lineaire = |c: f32| {
        let c = c.clamp(0.0, 1.0);
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    [
        lineaire(y + 1.5748 * v),
        lineaire(y - 0.1873 * u - 0.4681 * v),
        lineaire(y + 1.8556 * u),
    ]
}

/// La moitié d'une image de ces proportions, entière dans `extent` : elle garde
/// ses proportions, quitte à laisser des bandes.
pub fn fit_image(extent: [f32; 2], aspect: f32) -> [f32; 2] {
    if aspect >= extent[0] / extent[1] {
        [extent[0], extent[0] / aspect]
    } else {
        [extent[1] * aspect, extent[1]]
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
struct Uniforms {
    row0: [f32; 2],
    row1: [f32; 2],
    offset: [f32; 2],
    extent: [f32; 2],
    image: [f32; 2],
    size: [f32; 2],
}

/// Le demi-côté du rendu dans l'espace du §17 : `[-1, 1]` sur le petit côté.
fn half_extent(size: UVec2) -> [f32; 2] {
    let (w, h) = (size.x.max(1) as f32, size.y.max(1) as f32);
    if w >= h {
        [w / h, 1.0]
    } else {
        [1.0, h / w]
    }
}

fn uniforms(step: &VideoStep, size: UVec2, image: (u32, u32)) -> Uniforms {
    let (sin, cos) = (-step.angle).sin_cos();
    let extent = half_extent(size);
    let aspect = image.0.max(1) as f32 / image.1.max(1) as f32;
    Uniforms {
        row0: [cos / step.scale, -sin / step.scale],
        row1: [sin / step.scale, cos / step.scale],
        offset: step.offset,
        extent,
        image: fit_image(extent, aspect),
        size: [size.x.max(1) as f32, size.y.max(1) as f32],
    }
}

/// Les trois plans d'une taille donnée.
struct Planes {
    textures: [wgpu::Texture; 3],
    bind_group: wgpu::BindGroup,
    size: (u32, u32),
}

/// Un par piste, créé à sa première vidéo.
pub struct VideoRenderer {
    size: UVec2,
    pipeline: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    uniforms: wgpu::Buffer,
    #[allow(dead_code)] // tenue vivante : `view` en dépend
    target: wgpu::Texture,
    view: wgpu::TextureView,
    planes: Option<Planes>,
    /// L'image envoyée : son adresse et son instant.
    sent: Option<(usize, u64)>,
    visible: bool,
}

impl VideoRenderer {
    pub fn new(gpu: &Gpu, size: UVec2) -> Self {
        let shader = gpu
            .device
            .create_shader_module(wgpu::include_wgsl!("video.wgsl"));
        let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("video.target"),
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
            label: Some("video.params"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("video.sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let pipeline = gpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("video"),
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
            sampler,
            uniforms,
            target,
            view,
            planes: None,
            sent: None,
            visible: false,
        }
    }

    fn make_planes(&self, gpu: &Gpu, (w, h): (u32, u32)) -> Planes {
        let plan = |nom, width, height| {
            gpu.device.create_texture(&wgpu::TextureDescriptor {
                label: Some(nom),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            })
        };
        let textures = [
            plan("video.y", w, h),
            plan("video.u", w / 2, h / 2),
            plan("video.v", w / 2, h / 2),
        ];
        let vues: Vec<_> = textures
            .iter()
            .map(|t| t.create_view(&Default::default()))
            .collect();
        let bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("video"),
            layout: &self.pipeline.get_bind_group_layout(0),
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&vues[0]),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&vues[1]),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&vues[2]),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: self.uniforms.as_entire_binding(),
                },
            ],
        });
        Planes {
            textures,
            bind_group,
            size: (w, h),
        }
    }

    /// Envoie l'image si elle a changé. `None` : rien à montrer, la couche est
    /// transparente. Une image de la mauvaise taille est ignorée, sans panique.
    pub fn prepare(&mut self, image: Option<VideoImage<'_>>, step: &VideoStep, gpu: &Gpu) {
        self.visible = false;
        let Some(image) = image else { return };
        let (w, h) = (image.width, image.height);
        if w < 2 || h < 2 || image.data.len() < w as usize * h as usize * 3 / 2 {
            return;
        }
        if self.planes.as_ref().map(|p| p.size) != Some((w, h)) {
            self.planes = Some(self.make_planes(gpu, (w, h)));
            self.sent = None;
        }
        let u = uniforms(step, self.size, (w, h));
        gpu.queue
            .write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&u));
        self.visible = true;

        let etiquette = (image.data.as_ptr() as usize, image.time.to_bits());
        if self.sent == Some(etiquette) {
            return;
        }
        self.sent = Some(etiquette);
        let Some(plans) = &self.planes else { return };
        let (cw, ch) = (w / 2, h / 2);
        let luma = w as usize * h as usize;
        let chroma = cw as usize * ch as usize;
        let morceaux = [
            (&image.data[..luma], w, h),
            (&image.data[luma..luma + chroma], cw, ch),
            (&image.data[luma + chroma..luma + 2 * chroma], cw, ch),
        ];
        for (texture, (octets, largeur, hauteur)) in plans.textures.iter().zip(morceaux) {
            gpu.queue.write_texture(
                texture.as_image_copy(),
                octets,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(largeur),
                    rows_per_image: Some(hauteur),
                },
                wgpu::Extent3d {
                    width: largeur,
                    height: hauteur,
                    depth_or_array_layers: 1,
                },
            );
        }
    }

    pub fn encode(&self, encoder: &mut wgpu::CommandEncoder) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("video"),
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
        if let (true, Some(plans)) = (self.visible, &self.planes) {
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &plans.bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
    }

    pub fn output(&self) -> &wgpu::TextureView {
        &self.view
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn proche(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.02)
    }

    #[test]
    fn le_noir_et_le_blanc_de_la_plage_limitee_sont_les_extremes() {
        assert_eq!(yuv_to_linear(16, 128, 128), [0.0; 3]);
        let blanc = yuv_to_linear(235, 128, 128);
        assert!(blanc.iter().all(|c| (c - 1.0).abs() < 1e-3), "{blanc:?}");
    }

    #[test]
    fn un_rouge_bt709_reste_rouge() {
        assert!(proche(yuv_to_linear(63, 102, 240), [1.0, 0.0, 0.0]));
        assert!(proche(yuv_to_linear(32, 240, 118), [0.0, 0.0, 1.0]));
    }

    #[test]
    fn les_valeurs_hors_plage_restent_dans_0_1() {
        for c in yuv_to_linear(255, 255, 255)
            .into_iter()
            .chain(yuv_to_linear(0, 0, 0))
        {
            assert!((0.0..=1.0).contains(&c));
        }
    }

    #[test]
    fn une_image_16_9_remplit_un_rendu_16_9() {
        let e = half_extent(UVec2::new(1920, 1080));
        let i = fit_image(e, 16.0 / 9.0);
        assert!((i[0] - e[0]).abs() < 1e-5 && (i[1] - e[1]).abs() < 1e-5);
    }

    #[test]
    fn une_image_4_3_laisse_des_bandes_sur_les_cotes() {
        let e = half_extent(UVec2::new(1920, 1080));
        let i = fit_image(e, 4.0 / 3.0);
        assert_eq!(i[1], e[1]);
        assert!((i[0] - 4.0 / 3.0).abs() < 1e-5);
    }

    #[test]
    fn une_image_large_laisse_des_bandes_en_haut_et_en_bas() {
        let e = half_extent(UVec2::new(1920, 1080));
        let i = fit_image(e, 2.35);
        assert_eq!(i[0], e[0]);
        assert!(i[1] < e[1]);
    }

    #[test]
    fn la_pose_passe_dans_les_uniformes() {
        let step = VideoStep {
            scale: 2.0,
            angle: 0.0,
            offset: [0.25, -0.5],
        };
        let u = uniforms(&step, UVec2::new(1000, 1000), (100, 100));
        assert_eq!(u.row0, [0.5, 0.0]);
        assert_eq!(u.offset, [0.25, -0.5]);
        assert_eq!(u.image, [1.0, 1.0]);
    }
}
