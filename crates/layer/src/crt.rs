//! Le rendu des effets d'image : le CRT — §6.2 de `docs/ARCHITECTURE.md`.
//!
//! Lit la texture qu'un clip vient de rendre et écrit la sienne. Le halo passe par
//! un flou séparable en demi-résolution, seulement s'il n'est pas nul. La
//! rémanence garde l'image d'avant dans une texture à elle : lire et écrire la
//! même texture est interdit (§22).

use crate::gpu::{Gpu, LAYER_FORMAT};
use bytemuck::{Pod, Zeroable};
use frogenx_shape::{CrtStep, FilterStep};
use glam::UVec2;

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
struct Uniforms {
    size: [f32; 2],
    time: f32,
    lines: f32,
    scanlines: f32,
    mask: f32,
    curvature: f32,
    vignette: f32,
    halo: f32,
    aberration: f32,
    noise: f32,
    flicker: f32,
    band: f32,
    keep: f32,
    _pad: [f32; 2],
}

fn uniforms(step: &CrtStep, size: UVec2, t: f64) -> Uniforms {
    Uniforms {
        size: [size.x.max(1) as f32, size.y.max(1) as f32],
        // Replié : un `f32` perd sa précision au bout d'une heure (§17).
        time: (t % 3600.0) as f32,
        lines: step.lines,
        scanlines: step.scanlines,
        mask: step.mask,
        curvature: step.curvature,
        vignette: step.vignette,
        halo: step.halo,
        aberration: step.aberration,
        noise: step.noise,
        flicker: step.flicker,
        band: step.band,
        keep: step.keep,
        _pad: [0.0; 2],
    }
}

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Pod, Zeroable)]
struct Blur {
    direction: [f32; 2],
    size: [f32; 2],
}

fn texture(gpu: &Gpu, label: &str, size: UVec2, copy: bool) -> (wgpu::Texture, wgpu::TextureView) {
    let mut usage = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
    if copy {
        usage |= wgpu::TextureUsages::COPY_SRC | wgpu::TextureUsages::COPY_DST;
    }
    let t = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size.x.max(1),
            height: size.y.max(1),
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: LAYER_FORMAT,
        usage,
        view_formats: &[],
    });
    let v = t.create_view(&Default::default());
    (t, v)
}

fn pipeline(gpu: &Gpu, module: &wgpu::ShaderModule, fragment: &str) -> wgpu::RenderPipeline {
    gpu.device
        .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(fragment),
            layout: None,
            vertex: wgpu::VertexState {
                module,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module,
                entry_point: Some(fragment),
                compilation_options: Default::default(),
                targets: &[Some(LAYER_FORMAT.into())],
            }),
            primitive: Default::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        })
}

fn pass(
    encoder: &mut wgpu::CommandEncoder,
    target: &wgpu::TextureView,
    label: &str,
) -> wgpu::RenderPass<'static> {
    encoder
        .begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(label),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: target,
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
        })
        .forget_lifetime()
}

fn view_entry(binding: u32, view: &wgpu::TextureView) -> wgpu::BindGroupEntry<'_> {
    wgpu::BindGroupEntry {
        binding,
        resource: wgpu::BindingResource::TextureView(view),
    }
}

/// Un CRT : sa cible, sa mémoire, et ses deux demi-images de flou.
pub struct CrtRenderer {
    size: UVec2,
    main: wgpu::RenderPipeline,
    blur: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,
    uniforms: wgpu::Buffer,
    blur_uniforms: [wgpu::Buffer; 2],
    target: (wgpu::Texture, wgpu::TextureView),
    previous: (wgpu::Texture, wgpu::TextureView),
    half: [(wgpu::Texture, wgpu::TextureView); 2],
    /// Construits à chaque frame : la source peut changer d'une frame à l'autre.
    groups: Option<(wgpu::BindGroup, [wgpu::BindGroup; 2])>,
    halo: bool,
}

impl CrtRenderer {
    pub fn new(gpu: &Gpu, size: UVec2) -> Self {
        let crt = gpu
            .device
            .create_shader_module(wgpu::include_wgsl!("crt.wgsl"));
        let flou = gpu
            .device
            .create_shader_module(wgpu::include_wgsl!("crt_blur.wgsl"));
        let moitie = UVec2::new(size.x.div_ceil(2), size.y.div_ceil(2));
        let tampon = |label, taille| {
            gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: taille,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let blur_size = std::mem::size_of::<Blur>() as u64;
        Self {
            size,
            main: pipeline(gpu, &crt, "fs_main"),
            blur: pipeline(gpu, &flou, "fs_blur"),
            sampler: gpu.device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("crt.sampler"),
                mag_filter: wgpu::FilterMode::Linear,
                min_filter: wgpu::FilterMode::Linear,
                ..Default::default()
            }),
            uniforms: tampon("crt.params", std::mem::size_of::<Uniforms>() as u64),
            blur_uniforms: [
                tampon("crt.flou.h", blur_size),
                tampon("crt.flou.v", blur_size),
            ],
            target: texture(gpu, "crt.target", size, true),
            previous: texture(gpu, "crt.previous", size, true),
            half: [
                texture(gpu, "crt.half.a", moitie, false),
                texture(gpu, "crt.half.b", moitie, false),
            ],
            groups: None,
            halo: false,
        }
    }

    pub fn prepare(&mut self, gpu: &Gpu, source: &wgpu::TextureView, step: &CrtStep, t: f64) {
        let u = uniforms(step, self.size, t);
        gpu.queue
            .write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&u));
        let moitie = [
            self.size.x.div_ceil(2) as f32,
            self.size.y.div_ceil(2) as f32,
        ];
        for (buffer, direction) in self.blur_uniforms.iter().zip([[1.0, 0.0], [0.0, 1.0]]) {
            let b = Blur {
                direction,
                size: moitie,
            };
            gpu.queue.write_buffer(buffer, 0, bytemuck::bytes_of(&b));
        }
        self.halo = step.halo > 0.0;

        let vue = view_entry;
        let echantillon = |binding| wgpu::BindGroupEntry {
            binding,
            resource: wgpu::BindingResource::Sampler(&self.sampler),
        };
        let principal = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("crt"),
            layout: &self.main.get_bind_group_layout(0),
            entries: &[
                vue(0, source),
                vue(1, &self.half[1].1),
                vue(2, &self.previous.1),
                echantillon(3),
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: self.uniforms.as_entire_binding(),
                },
            ],
        });
        let flou = |entree: &wgpu::TextureView, buffer: &wgpu::Buffer| {
            gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("crt.flou"),
                layout: &self.blur.get_bind_group_layout(0),
                entries: &[
                    vue(0, entree),
                    echantillon(1),
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: buffer.as_entire_binding(),
                    },
                ],
            })
        };
        let flous = [
            flou(source, &self.blur_uniforms[0]),
            flou(&self.half[0].1, &self.blur_uniforms[1]),
        ];
        self.groups = Some((principal, flous));
    }

    pub fn encode(&self, encoder: &mut wgpu::CommandEncoder) {
        let Some((principal, flous)) = &self.groups else {
            return;
        };
        if self.halo {
            for (cible, groupe) in [(&self.half[0].1, &flous[0]), (&self.half[1].1, &flous[1])] {
                let mut p = pass(encoder, cible, "crt.flou");
                p.set_pipeline(&self.blur);
                p.set_bind_group(0, groupe, &[]);
                p.draw(0..3, 0..1);
            }
        }
        {
            let mut p = pass(encoder, &self.target.1, "crt");
            p.set_pipeline(&self.main);
            p.set_bind_group(0, principal, &[]);
            p.draw(0..3, 0..1);
        }
        encoder.copy_texture_to_texture(
            self.target.0.as_image_copy(),
            self.previous.0.as_image_copy(),
            self.target.0.size(),
        );
    }

    pub fn output(&self) -> &wgpu::TextureView {
        &self.target.1
    }
}

/// Les effets d'image d'une place de piste, dans l'ordre du clip. Un rendu par
/// effet : chacun garde sa mémoire tant que le clip les garde.
pub struct FilterChain {
    size: UVec2,
    renderers: Vec<CrtRenderer>,
}

impl FilterChain {
    pub fn new(size: UVec2) -> Self {
        Self {
            size,
            renderers: Vec::new(),
        }
    }

    /// Prépare chaque effet sur la sortie du précédent. Le premier lit `source`.
    pub fn prepare(&mut self, gpu: &Gpu, source: &wgpu::TextureView, steps: &[FilterStep], t: f64) {
        while self.renderers.len() < steps.len() {
            self.renderers.push(CrtRenderer::new(gpu, self.size));
        }
        self.renderers.truncate(steps.len());
        for (i, etape) in steps.iter().enumerate() {
            let (avant, reste) = self.renderers.split_at_mut(i);
            let entree = avant.last().map_or(source, |r| r.output());
            match etape {
                FilterStep::Crt(step) => reste[0].prepare(gpu, entree, step, t),
            }
        }
    }

    pub fn encode(&self, encoder: &mut wgpu::CommandEncoder) {
        for r in &self.renderers {
            r.encode(encoder);
        }
    }

    /// La sortie du dernier effet. `None` : la chaîne est vide.
    pub fn output(&self) -> Option<&wgpu::TextureView> {
        self.renderers.last().map(CrtRenderer::output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn les_uniformes_tiennent_en_blocs_de_16_octets() {
        assert_eq!(std::mem::size_of::<Uniforms>() % 16, 0);
        assert_eq!(std::mem::size_of::<Blur>() % 16, 0);
    }

    #[test]
    fn le_temps_se_replie_avant_de_perdre_sa_precision() {
        let step = CrtStep {
            lines: 240.0,
            scanlines: 0.0,
            mask: 0.0,
            curvature: 0.0,
            vignette: 0.0,
            halo: 0.0,
            aberration: 0.0,
            noise: 0.0,
            flicker: 0.0,
            band: 0.0,
            keep: 0.0,
        };
        let u = uniforms(&step, UVec2::new(100, 50), 3600.0 * 5.0 + 1.5);
        assert!((u.time - 1.5).abs() < 1e-3);
        assert_eq!(u.size, [100.0, 50.0]);
    }
}
