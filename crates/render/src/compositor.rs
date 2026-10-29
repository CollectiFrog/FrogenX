//! Le compositeur — §20.1 (étapes 7-8), §22 de `docs/ARCHITECTURE.md`.

use crate::blend::saturate_opacity;
use frogenx_core::{Ctx, Graph};
use frogenx_layer::{Gpu, RenderLayer, LAYER_FORMAT};
use glam::UVec2;

/// Une couche à composer, avec le graphe qui résout ses `Param`.
///
/// Chaque clip porte son propre `Graph` (§3) : le compositeur ne suppose pas
/// un graphe unique pour toutes les couches.
pub struct LayerInput<'a> {
    pub layer: &'a mut dyn RenderLayer,
    pub graph: &'a Graph,
}

/// Uniformes d'une passe de composition — miroir de `Params` dans
/// `composite.wgsl` (16 octets).
#[repr(C)]
#[derive(Debug, Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct CompositeParams {
    opacity: f32,
    mode: u32,
    _pad: [u32; 2],
}

struct Accum {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

impl Accum {
    fn new(gpu: &Gpu, size: UVec2, label: &str) -> Self {
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
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
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        Self { texture, view }
    }
}

/// Mélange les couches, de bas en haut, dans une texture finale linéaire
/// prémultipliée.
///
/// Deux textures d'accumulation en ping-pong : lire et écrire la même
/// texture est interdit (§22). Le compositeur ne libère jamais une texture
/// qu'il n'a pas créée — il ne fait qu'emprunter celles des couches.
pub struct Compositor {
    size: UVec2,
    pipeline: wgpu::RenderPipeline,
    bind_layout: wgpu::BindGroupLayout,
    accum: [Accum; 2],
    /// Un buffer d'uniformes par couche : les écritures de la queue sont
    /// toutes appliquées avant la soumission, un buffer partagé ne verrait
    /// que la dernière.
    uniforms: Vec<wgpu::Buffer>,
    /// Indice de l'accumulation qui tient le résultat de la dernière frame.
    current: usize,
}

impl Compositor {
    pub fn new(gpu: &Gpu, size: UVec2) -> Self {
        let bind_layout = gpu
            .device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("compositor"),
                entries: &[
                    texture_entry(0),
                    texture_entry(1),
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });
        let pipeline = create_pipeline(gpu, &bind_layout);
        Self {
            size,
            pipeline,
            bind_layout,
            accum: [
                Accum::new(gpu, size, "compositor.accum0"),
                Accum::new(gpu, size, "compositor.accum1"),
            ],
            uniforms: Vec::new(),
            current: 0,
        }
    }

    pub fn size(&self) -> UVec2 {
        self.size
    }

    /// Change la résolution de la composition. Les couches sont
    /// redimensionnées séparément, par leur propre `resize` (§22).
    pub fn resize(&mut self, gpu: &Gpu, size: UVec2) {
        if size != self.size {
            self.size = size;
            self.accum = [
                Accum::new(gpu, size, "compositor.accum0"),
                Accum::new(gpu, size, "compositor.accum1"),
            ];
        }
    }

    /// La texture finale de la dernière frame, en linéaire prémultiplié.
    pub fn output_texture(&self) -> &wgpu::Texture {
        &self.accum[self.current].texture
    }

    pub fn output_view(&self) -> &wgpu::TextureView {
        &self.accum[self.current].view
    }

    /// Une frame complète, dans l'ordre strict du §20.1 et du §22 :
    /// `prepare` sur chaque couche, `encode` sur chaque couche, puis une
    /// passe de mélange par couche, de la première (en bas) à la dernière
    /// (en haut). Un seul encodeur, soumis une fois.
    ///
    /// Le graphe de chaque couche doit avoir été évalué pour la frame
    /// (`eval_frame`) avant l'appel.
    pub fn render(&mut self, gpu: &Gpu, t: f64, ctx: &Ctx, inputs: &mut [LayerInput<'_>]) {
        for input in inputs.iter_mut() {
            input.layer.prepare(t, ctx, input.graph, gpu);
        }

        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            });
        for input in inputs.iter() {
            input.layer.encode(&mut encoder);
        }

        // Point de départ : une accumulation transparente. Sans couche, la
        // sortie est donc transparente, jamais l'image d'une frame passée.
        clear_pass(&mut encoder, &self.accum[0].view);
        let mut current = 0;

        while self.uniforms.len() < inputs.len() {
            self.uniforms
                .push(gpu.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("compositor.params"),
                    size: std::mem::size_of::<CompositeParams>() as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }));
        }

        for (i, input) in inputs.iter().enumerate() {
            let blend = input.layer.blend();
            let params = CompositeParams {
                opacity: saturate_opacity(blend.opacity.get(input.graph)),
                mode: blend.mode as u32,
                _pad: [0; 2],
            };
            gpu.queue
                .write_buffer(&self.uniforms[i], 0, bytemuck::bytes_of(&params));

            let bind = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("compositor"),
                layout: &self.bind_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&self.accum[current].view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::TextureView(input.layer.output()),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: self.uniforms[i].as_entire_binding(),
                    },
                ],
            });

            let next = 1 - current;
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("compositor"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.accum[next].view,
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
            pass.set_bind_group(0, &bind, &[]);
            pass.draw(0..3, 0..1);
            drop(pass);
            current = next;
        }

        self.current = current;
        gpu.queue.submit(std::iter::once(encoder.finish()));
    }
}

fn texture_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            // Lue par `textureLoad`, au pixel près : pas d'échantillonneur.
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

/// Une passe sans dessin : seul son `LoadOp::Clear` compte. La passe se
/// termine à son abandon, explicite ici.
fn clear_pass(encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView) {
    let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("compositor.clear"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view,
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
    drop(pass);
}

fn create_pipeline(gpu: &Gpu, bind_layout: &wgpu::BindGroupLayout) -> wgpu::RenderPipeline {
    let shader = gpu
        .device
        .create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("composite.wgsl"),
            source: wgpu::ShaderSource::Wgsl(include_str!("composite.wgsl").into()),
        });
    let layout = gpu
        .device
        .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("compositor"),
            bind_group_layouts: &[Some(bind_layout)],
            immediate_size: 0,
        });
    gpu.device
        .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("compositor"),
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
                compilation_options: Default::default(),
                // Pas de mélange matériel : le shader calcule le résultat
                // complet et le remplace.
                targets: &[Some(wgpu::ColorTargetState {
                    format: LAYER_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        })
}
