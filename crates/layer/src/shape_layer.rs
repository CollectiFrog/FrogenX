//! `ShapeLayer` — une forme rasterisée dans sa propre texture, §6, §22 de
//! `docs/ARCHITECTURE.md`.

use crate::gpu::{Gpu, LAYER_FORMAT};
use crate::render_layer::{RenderDiagnostics, RenderLayer};
use crate::space::normalized_to_ndc;
use crate::tessellate::{Mesh, Tessellator};
use frogenx_core::{Ctx, Graph, Param};
use frogenx_shape::{
    BlendParams, ColorParam, Path, ShapeGenerator, ShapeOp, ShapeStyle, StrokeStyle,
};
use glam::{UVec2, Vec2};

/// Sommet envoyé au GPU : position déjà en NDC, couleur déjà en linéaire
/// prémultiplié (`shape.wgsl` ne décide rien).
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
struct Vertex {
    position: [f32; 2],
    color: [f32; 4],
}

const VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 2] =
    wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x4];

/// Capacité initiale des buffers GPU, en octets. Ils grandissent par
/// puissances de deux, jamais ne rétrécissent : une forme qui pulse ne
/// réalloue pas à chaque frame.
const BUFFER_INITIAL: u64 = 64 * 1024;

/// Les textures de rendu d'une couche, recréées par `resize`.
struct Targets {
    output: wgpu::Texture,
    output_view: wgpu::TextureView,
    /// Cible multiéchantillonnée, résolue vers `output`. `None` en 1×.
    msaa_view: Option<wgpu::TextureView>,
}

impl Targets {
    fn new(gpu: &Gpu, size: UVec2, samples: u32) -> Self {
        let extent = wgpu::Extent3d {
            width: size.x.max(1),
            height: size.y.max(1),
            depth_or_array_layers: 1,
        };
        let output = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("shape_layer.output"),
            size: extent,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: LAYER_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let output_view = output.create_view(&Default::default());
        let msaa_view = (samples > 1).then(|| {
            gpu.device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some("shape_layer.msaa"),
                    size: extent,
                    mip_level_count: 1,
                    sample_count: samples,
                    dimension: wgpu::TextureDimension::D2,
                    format: LAYER_FORMAT,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        });
        Self {
            output,
            output_view,
            msaa_view,
        }
    }
}

/// Le contenu d'une frame : ce qu'il faut dessiner, emprunté à qui le possède.
///
/// Un clip de la grille n'a pas de textures : il prête ceci au renderer de sa
/// piste, le temps d'une frame (§3bis.2).
pub struct ShapeContent<'a> {
    pub generator: &'a dyn ShapeGenerator,
    /// Appliqués dans l'ordre au `Path` produit par le générateur (§16.3 bis).
    pub effects: &'a [Box<dyn ShapeOp>],
    pub style: ShapeStyle,
}

/// Ce qui coûte : textures, pipeline, buffers, mémoire réutilisée (§22.1).
///
/// Un renderer ne garde rien de son contenu d'une frame à l'autre : il dessine
/// n'importe quel clip sans rien réallouer. C'est ce qui permet d'en avoir un
/// par piste, et non un par clip.
pub struct ShapeRenderer {
    // Mémoire CPU réutilisée d'une frame à l'autre (§16.3).
    path: Path,
    tess: Tessellator,
    fill_mesh: Mesh,
    stroke_mesh: Mesh,
    vertices: Vec<Vertex>,
    indices: Vec<u32>,

    // Ressources GPU.
    size: UVec2,
    samples: u32,
    pipeline: wgpu::RenderPipeline,
    targets: Targets,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    index_count: u32,
    diagnostics: RenderDiagnostics,
}

impl ShapeRenderer {
    pub fn new(gpu: &Gpu, size: UVec2) -> Self {
        let samples = gpu.msaa_samples(LAYER_FORMAT);
        let pipeline = create_pipeline(gpu, samples);
        Self {
            path: Path::new(),
            tess: Tessellator::new(),
            fill_mesh: Mesh::new(),
            stroke_mesh: Mesh::new(),
            vertices: Vec::new(),
            indices: Vec::new(),
            size,
            samples,
            pipeline,
            targets: Targets::new(gpu, size, samples),
            vertex_buffer: create_buffer(gpu, BUFFER_INITIAL, wgpu::BufferUsages::VERTEX),
            index_buffer: create_buffer(gpu, BUFFER_INITIAL, wgpu::BufferUsages::INDEX),
            index_count: 0,
            diagnostics: RenderDiagnostics {
                msaa_fallback: samples == 1,
                tessellation_failures: 0,
            },
        }
    }

    pub fn size(&self) -> UVec2 {
        self.size
    }

    /// La texture de sortie — pour une relecture (§8.2), là où `output`
    /// ne prête qu'une vue.
    pub fn output_texture(&self) -> &wgpu::Texture {
        &self.targets.output
    }

    pub fn diagnostics(&self) -> RenderDiagnostics {
        self.diagnostics
    }

    /// Ajoute un maillage aux sommets GPU, avec sa couleur et la correction
    /// de proportion vers les NDC (§17).
    fn push_mesh(&mut self, which: MeshKind, color: [f32; 4], scale: Vec2) {
        let mesh = match which {
            MeshKind::Fill => &self.fill_mesh,
            MeshKind::Stroke => &self.stroke_mesh,
        };
        let base = self.vertices.len() as u32;
        self.vertices.extend(mesh.vertices.iter().map(|v| Vertex {
            position: (Vec2::from(*v) * scale).to_array(),
            color,
        }));
        self.indices.extend(mesh.indices.iter().map(|i| i + base));
    }

    /// Travail CPU de la frame : voir [`RenderLayer::prepare`].
    pub fn prepare(&mut self, content: &ShapeContent<'_>, t: f64, ctx: &Ctx, g: &Graph, gpu: &Gpu) {
        self.diagnostics.tessellation_failures = 0;
        content.generator.generate(t, ctx, g, &mut self.path);
        for op in content.effects {
            op.apply(t, ctx, g, &mut self.path);
        }

        let ShapeStyle { fill, stroke } = content.style;
        self.fill_mesh.clear();
        self.stroke_mesh.clear();
        if fill.is_some() && !self.tess.fill(&self.path, &mut self.fill_mesh) {
            self.diagnostics.tessellation_failures += 1;
        }
        if let Some(stroke) = stroke {
            let width = stroke.width.get(g);
            if !self.tess.stroke(&self.path, width, &mut self.stroke_mesh) {
                self.diagnostics.tessellation_failures += 1;
            }
        }

        // Le trait passe par-dessus le remplissage.
        let scale = normalized_to_ndc(self.size);
        self.vertices.clear();
        self.indices.clear();
        if let Some(fill) = fill {
            self.push_mesh(MeshKind::Fill, fill.resolve(g), scale);
        }
        if let Some(stroke) = stroke {
            self.push_mesh(MeshKind::Stroke, stroke.color.resolve(g), scale);
        }

        self.index_count = self.indices.len() as u32;
        if self.index_count == 0 {
            return;
        }
        let vbytes: &[u8] = bytemuck::cast_slice(&self.vertices);
        let ibytes: &[u8] = bytemuck::cast_slice(&self.indices);
        ensure_capacity(
            gpu,
            &mut self.vertex_buffer,
            vbytes.len() as u64,
            wgpu::BufferUsages::VERTEX,
        );
        ensure_capacity(
            gpu,
            &mut self.index_buffer,
            ibytes.len() as u64,
            wgpu::BufferUsages::INDEX,
        );
        gpu.queue.write_buffer(&self.vertex_buffer, 0, vbytes);
        gpu.queue.write_buffer(&self.index_buffer, 0, ibytes);
    }

    /// Voir [`RenderLayer::encode`].
    pub fn encode(&self, encoder: &mut wgpu::CommandEncoder) {
        let (view, resolve_target, store) = match &self.targets.msaa_view {
            // L'image multiéchantillonnée n'est utile que le temps de la
            // résolution : inutile de la conserver.
            Some(msaa) => (
                msaa,
                Some(&self.targets.output_view),
                wgpu::StoreOp::Discard,
            ),
            None => (&self.targets.output_view, None, wgpu::StoreOp::Store),
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("shape_layer"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store,
                },
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        // Une couche vide est quand même effacée : elle doit sortir
        // transparente, pas garder l'image de la frame précédente.
        if self.index_count > 0 {
            pass.set_pipeline(&self.pipeline);
            pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
            pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..self.index_count, 0, 0..1);
        }
    }

    pub fn output(&self) -> &wgpu::TextureView {
        &self.targets.output_view
    }

    /// Voir [`RenderLayer::resize`].
    pub fn resize(&mut self, size: UVec2, gpu: &Gpu) {
        if size != self.size {
            self.size = size;
            self.targets = Targets::new(gpu, size, self.samples);
        }
    }
}

#[derive(Clone, Copy)]
enum MeshKind {
    Fill,
    Stroke,
}

/// Une forme procédurale rasterisée, qui possède son contenu : générateur,
/// remplissage, trait. Pour un clip de la grille, voir [`ShapeRenderer`].
pub struct ShapeLayer {
    pub generator: Box<dyn ShapeGenerator>,
    pub fill: Option<ColorParam>,
    pub stroke: Option<StrokeStyle>,
    pub blend: BlendParams,
    renderer: ShapeRenderer,
}

impl ShapeLayer {
    /// Une couche sans remplissage ni trait : à compléter par `with_fill` /
    /// `with_stroke`.
    pub fn new(gpu: &Gpu, size: UVec2, generator: Box<dyn ShapeGenerator>) -> Self {
        Self {
            generator,
            fill: None,
            stroke: None,
            blend: BlendParams::default(),
            renderer: ShapeRenderer::new(gpu, size),
        }
    }

    pub fn with_fill(mut self, color: ColorParam) -> Self {
        self.fill = Some(color);
        self
    }

    pub fn with_stroke(mut self, width: Param, color: ColorParam) -> Self {
        self.stroke = Some(StrokeStyle { width, color });
        self
    }

    pub fn with_blend(mut self, blend: BlendParams) -> Self {
        self.blend = blend;
        self
    }

    pub fn size(&self) -> UVec2 {
        self.renderer.size()
    }

    /// La texture de sortie — pour une relecture (§8.2), là où `output`
    /// ne prête qu'une vue.
    pub fn output_texture(&self) -> &wgpu::Texture {
        self.renderer.output_texture()
    }

    pub fn diagnostics(&self) -> RenderDiagnostics {
        self.renderer.diagnostics()
    }
}

impl RenderLayer for ShapeLayer {
    fn prepare(&mut self, t: f64, ctx: &Ctx, g: &Graph, gpu: &Gpu) {
        let content = ShapeContent {
            generator: self.generator.as_ref(),
            effects: &[],
            style: ShapeStyle {
                fill: self.fill,
                stroke: self.stroke,
            },
        };
        self.renderer.prepare(&content, t, ctx, g, gpu);
    }

    fn encode(&self, encoder: &mut wgpu::CommandEncoder) {
        self.renderer.encode(encoder);
    }

    fn output(&self) -> &wgpu::TextureView {
        self.renderer.output()
    }

    fn blend(&self) -> BlendParams {
        self.blend
    }

    fn resize(&mut self, size: UVec2, gpu: &Gpu) {
        self.renderer.resize(size, gpu);
    }
}

fn create_buffer(gpu: &Gpu, size: u64, usage: wgpu::BufferUsages) -> wgpu::Buffer {
    gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("shape_layer.buffer"),
        size,
        usage: usage | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

/// Agrandit `buffer` s'il ne peut pas contenir `needed` octets.
fn ensure_capacity(gpu: &Gpu, buffer: &mut wgpu::Buffer, needed: u64, usage: wgpu::BufferUsages) {
    if needed > buffer.size() {
        *buffer = create_buffer(gpu, needed.next_power_of_two(), usage);
    }
}

fn create_pipeline(gpu: &Gpu, samples: u32) -> wgpu::RenderPipeline {
    let shader = gpu
        .device
        .create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("shape.wgsl"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shape.wgsl").into()),
        });
    let layout = gpu
        .device
        .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("shape_layer"),
            bind_group_layouts: &[],
            immediate_size: 0,
        });
    gpu.device
        .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("shape_layer"),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                compilation_options: Default::default(),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: std::mem::size_of::<Vertex>() as u64,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &VERTEX_ATTRIBUTES,
                })],
            },
            // Pas d'élimination des faces arrière : l'orientation des
            // triangles produits par lyon n'est pas garantie.
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState {
                count: samples,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                compilation_options: Default::default(),
                // Le trait se pose sur le remplissage dans la même passe :
                // mélange « par-dessus » en alpha prémultiplié (§21).
                targets: &[Some(wgpu::ColorTargetState {
                    format: LAYER_FORMAT,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            multiview_mask: None,
            cache: None,
        })
}
