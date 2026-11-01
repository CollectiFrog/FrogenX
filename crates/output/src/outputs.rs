//! L'étage de sortie — étape 9 du §20.1, §8.0 et §8.5 de `docs/ARCHITECTURE.md`.

use crate::conversion::{ConversionMode, ConversionPass, Rect};
use crate::fanout::{CpuFanout, CpuSinkId, DropPolicy, FrameKey, Group};
use crate::live::{pool_take, LiveReader, Pool, Readback, READBACK_SLOTS};
use crate::preview::letterbox;
use crate::sink::{CpuFrame, CpuSink, GpuFrame, GpuSink, OutputSink, PixelLayout, SinkStatus};
use crate::stream::StreamFormat;
use frogenx_layer::Gpu;
use frogenx_render::read_rgba8;
use std::fmt;
use std::sync::{Arc, Mutex};

/// Le format de la texture relue pour une sortie CPU : le matériel encode le
/// sRGB (§8.3).
const CPU_TARGET_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
/// Celui d'un flux, dans l'ordre que Spout attend : le GPU range les octets,
/// la sortie n'a rien à recopier (§8.5).
const STREAM_TARGET_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Bgra8UnormSrgb;
const BYTES_PER_PIXEL: u32 = 4;

/// Refus d'ajouter une sortie — régime d'édition (§18.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputError {
    /// La surface ne propose aucun format avec cet adaptateur.
    SurfaceNotCompatible,
}

impl fmt::Display for OutputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SurfaceNotCompatible => write!(f, "surface incompatible avec cet adaptateur"),
        }
    }
}

impl std::error::Error for OutputError {}

/// Désigne une sortie GPU, pour la retirer (§8.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GpuSinkId(u32);

/// La cible d'un format : une réduction et ses buffers de relecture.
struct CpuTarget {
    key: FrameKey,
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    /// Les buffers libres, et combien existent déjà.
    pool: Pool,
    buffers: usize,
    /// La ligne utile et sa place dans un buffer, en octets.
    row: u32,
    stride: u32,
}

impl CpuTarget {
    fn new(gpu: &Gpu, key: FrameKey) -> Self {
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("outputs.cpu_target"),
            size: wgpu::Extent3d {
                width: key.width,
                height: key.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: match key.layout {
                PixelLayout::RgbaStraight => CPU_TARGET_FORMAT,
                PixelLayout::BgraOpaque => STREAM_TARGET_FORMAT,
            },
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        let row = key.width * BYTES_PER_PIXEL;
        Self {
            key,
            texture,
            view,
            pool: Arc::new(Mutex::new(Vec::new())),
            buffers: 0,
            row,
            stride: row.next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT),
        }
    }

    /// Un buffer libre : aucun ne reste en attente tant que l'anneau n'a pas
    /// sa taille. `None` : toute l'avance est prise, la frame sera perdue.
    fn buffer(&mut self, gpu: &Gpu) -> Option<wgpu::Buffer> {
        if let Some(buffer) = pool_take(&self.pool) {
            return Some(buffer);
        }
        (self.buffers < READBACK_SLOTS).then(|| {
            self.buffers += 1;
            gpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("outputs.readback"),
                size: (self.stride * self.key.height) as u64,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            })
        })
    }

    fn copy_into(&self, encoder: &mut wgpu::CommandEncoder, buffer: &wgpu::Buffer) {
        encoder.copy_texture_to_buffer(
            self.texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(self.stride),
                    rows_per_image: Some(self.key.height),
                },
            },
            self.texture.size(),
        );
    }
}

/// Une image attendue à cette frame.
struct Pending {
    group: Group,
    /// Sa cible dans `Outputs::targets`.
    target: usize,
    /// `None` en `Fixed`, où la relecture reste synchrone.
    buffer: Option<wgpu::Buffer>,
}

/// Alimente toutes les sorties, une fois par frame.
///
/// Numérote les frames et relit **une seule fois par format**, seulement ce que
/// quelqu'un attend. La politique de perte vit dans [`CpuFanout`], écrite une
/// fois. En `Lossy`, la relecture a lieu sur un thread : la boucle n'attend
/// jamais le GPU (§8.5).
pub struct Outputs {
    conversion: ConversionPass,
    /// La passe des flux, créée au premier.
    stream_pass: Option<ConversionPass>,
    gpu_sinks: Vec<(GpuSinkId, Box<dyn GpuSink>)>,
    next_gpu_id: u32,
    fanout: CpuFanout,
    /// Une cible par format en usage.
    targets: Vec<CpuTarget>,
    reader: Option<LiveReader>,
    /// La taille de rendu des cibles, pour oublier celles d'avant.
    render: (u32, u32),
    next_index: u64,
}

impl Outputs {
    pub fn new(gpu: &Gpu, policy: DropPolicy) -> Self {
        Self {
            conversion: ConversionPass::new(gpu, CPU_TARGET_FORMAT, ConversionMode::Straight),
            stream_pass: None,
            gpu_sinks: Vec::new(),
            next_gpu_id: 0,
            fanout: CpuFanout::new(policy),
            targets: Vec::new(),
            reader: None,
            render: (0, 0),
            next_index: 0,
        }
    }

    pub fn add(&mut self, sink: OutputSink) -> Result<(), OutputError> {
        match sink {
            OutputSink::Gpu(sink) => {
                self.add_gpu(sink);
            }
            OutputSink::Cpu(sink) => {
                self.fanout.add(sink);
            }
        }
        Ok(())
    }

    /// Ajoute une sortie GPU qu'on pourra retirer.
    pub fn add_gpu(&mut self, sink: Box<dyn GpuSink>) -> GpuSinkId {
        let id = GpuSinkId(self.next_gpu_id);
        self.next_gpu_id += 1;
        self.gpu_sinks.push((id, sink));
        id
    }

    /// Retire une sortie GPU, entre deux frames : un geste de l'utilisateur,
    /// jamais une réaction à `Failed` (§8.0, §8.4). Rend `false` si elle est
    /// déjà retirée.
    pub fn remove_gpu(&mut self, id: GpuSinkId) -> bool {
        let avant = self.gpu_sinks.len();
        self.gpu_sinks.retain(|(sink, _)| *sink != id);
        self.gpu_sinks.len() != avant
    }

    /// Ajoute un flux : une sortie CPU qui a son format et sa cadence (§8.5).
    pub fn add_stream(&mut self, sink: Box<dyn CpuSink>, format: StreamFormat) -> CpuSinkId {
        self.fanout.add_stream(sink, format)
    }

    /// Change le format d'un flux, entre deux frames. Rend `false` s'il est
    /// retiré.
    pub fn set_stream_format(&mut self, id: CpuSinkId, format: StreamFormat) -> bool {
        let fait = self.fanout.set_stream_format(id, format);
        self.forget_unused_targets();
        fait
    }

    /// Retire un flux : les relectures en cours se terminent d'abord. Rend
    /// `false` s'il est déjà retiré.
    pub fn remove_stream(&mut self, id: CpuSinkId) -> bool {
        if let Some(reader) = &self.reader {
            reader.flush();
        }
        let fait = self.fanout.remove(id);
        self.forget_unused_targets();
        fait
    }

    /// Le statut d'un flux.
    pub fn stream_status(&self, id: CpuSinkId) -> Option<SinkStatus> {
        self.fanout.status(id)
    }

    fn forget_unused_targets(&mut self) {
        let utiles = self.fanout.keys(self.render);
        self.targets.retain(|t| utiles.contains(&t.key));
    }

    /// Étape 9 du §20.1, à partir de la texture finale du compositeur.
    pub fn submit(&mut self, gpu: &Gpu, texture: &wgpu::Texture, t: f64) {
        let index = self.next_index;
        self.next_index += 1;

        let render = (texture.width(), texture.height());
        if render != self.render {
            self.render = render;
            self.forget_unused_targets();
        }
        let groupes = self.fanout.due(t, render);
        let actives =
            |s: &&mut (GpuSinkId, Box<dyn GpuSink>)| !matches!(s.1.status(), SinkStatus::Failed(_));

        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("outputs"),
            });
        let frame = GpuFrame { texture, index, t };
        for (_, sink) in self.gpu_sinks.iter_mut().filter(actives) {
            sink.encode(gpu, &mut encoder, &frame);
        }
        let attendues: Vec<Pending> = groupes
            .into_iter()
            .filter_map(|group| self.encode_group(gpu, &mut encoder, texture, group))
            .collect();
        let soumission = gpu.queue.submit(std::iter::once(encoder.finish()));

        for (_, sink) in self.gpu_sinks.iter_mut().filter(actives) {
            sink.after_submit(gpu);
        }

        for attendue in attendues {
            self.finish_group(gpu, attendue, &soumission, index, t);
        }
    }

    /// Réduit l'image vers la cible du groupe, et la copie vers un buffer en
    /// direct. Sans buffer libre, la frame est perdue avant de coûter quoi
    /// que ce soit.
    fn encode_group(
        &mut self,
        gpu: &Gpu,
        encoder: &mut wgpu::CommandEncoder,
        texture: &wgpu::Texture,
        group: Group,
    ) -> Option<Pending> {
        let render = (texture.width(), texture.height());
        let i = self.target(gpu, group.key);
        let direct = self.fanout.policy() == DropPolicy::Lossy;
        let buffer = if direct {
            let Some(buffer) = self.targets[i].buffer(gpu) else {
                group.count_lost();
                return None;
            };
            Some(buffer)
        } else {
            None
        };
        let cible = &self.targets[i];
        match group.key.layout {
            PixelLayout::RgbaStraight => {
                self.conversion
                    .encode(gpu, encoder, texture, &cible.view, None);
            }
            PixelLayout::BgraOpaque => {
                let passe = self.stream_pass.get_or_insert_with(|| {
                    ConversionPass::new(gpu, STREAM_TARGET_FORMAT, ConversionMode::OverBlack)
                });
                let cadre: Rect = letterbox(render, (group.key.width, group.key.height));
                passe.encode(gpu, encoder, texture, &cible.view, Some(cadre));
            }
        }
        if let Some(buffer) = &buffer {
            cible.copy_into(encoder, buffer);
        }
        Some(Pending {
            group,
            target: i,
            buffer,
        })
    }

    fn target(&mut self, gpu: &Gpu, key: FrameKey) -> usize {
        match self.targets.iter().position(|t| t.key == key) {
            Some(i) => i,
            None => {
                self.targets.push(CpuTarget::new(gpu, key));
                self.targets.len() - 1
            }
        }
    }

    fn finish_group(
        &mut self,
        gpu: &Gpu,
        attendue: Pending,
        soumission: &wgpu::SubmissionIndex,
        index: u64,
        t: f64,
    ) {
        let Pending {
            group,
            target,
            buffer,
        } = attendue;
        let cible = &self.targets[target];
        let frame = CpuFrame {
            pixels: Arc::from(Vec::new()),
            layout: group.key.layout,
            width: group.key.width,
            height: group.key.height,
            index,
            t,
        };
        match buffer {
            Some(buffer) => {
                let lecture = Readback {
                    buffer,
                    pool: Arc::clone(&cible.pool),
                    group,
                    frame,
                    submission: soumission.clone(),
                    row: cible.row as usize,
                    stride: cible.stride as usize,
                };
                self.reader
                    .get_or_insert_with(|| LiveReader::new(gpu.device.clone()))
                    .read(lecture);
            }
            None => match read_rgba8(gpu, &cible.texture) {
                Some(octets) => group.deliver(CpuFrame {
                    pixels: Arc::from(octets),
                    ..frame
                }),
                None => group.count_lost(),
            },
        }
    }

    /// Le statut de chaque sortie : les sorties GPU, puis les sorties CPU.
    pub fn statuses(&self) -> Vec<SinkStatus> {
        self.gpu_sinks
            .iter()
            .map(|(_, s)| s.status())
            .chain(self.fanout.statuses())
            .collect()
    }

    /// Attend que les sorties CPU aient fini : à la fin d'un rendu offline,
    /// toutes les frames sont écrites.
    pub fn finish(&mut self) {
        if let Some(reader) = &self.reader {
            reader.flush();
        }
        self.fanout.finish();
    }
}

impl Drop for Outputs {
    fn drop(&mut self) {
        self.finish();
    }
}
