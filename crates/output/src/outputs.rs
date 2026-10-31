//! L'étage de sortie — étape 9 du §20.1, §8.0 de `docs/ARCHITECTURE.md`.

use crate::conversion::{ConversionMode, ConversionPass};
use crate::fanout::{CpuFanout, DropPolicy};
use crate::sink::{CpuFrame, CpuSink, GpuFrame, GpuSink, OutputSink, SinkStatus};
use frogenx_layer::Gpu;
use frogenx_render::read_rgba8;
use std::fmt;
use std::sync::Arc;

/// Le format de la texture relue : le matériel encode le sRGB (§8.3).
const CPU_TARGET_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// Refus d'ajouter une sortie — régime d'édition (§18.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputError {
    /// Seule la relecture synchrone existe avant l'étape 10 : en `Realtime`,
    /// elle effondrerait le framerate (§8.2).
    CpuSinkRequiresFixedClock,
    /// La surface ne propose aucun format avec cet adaptateur.
    SurfaceNotCompatible,
}

impl fmt::Display for OutputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CpuSinkRequiresFixedClock => write!(
                f,
                "sortie CPU refusée en temps réel : il faut l'anneau de relecture (étape 10)"
            ),
            Self::SurfaceNotCompatible => write!(f, "surface incompatible avec cet adaptateur"),
        }
    }
}

impl std::error::Error for OutputError {}

/// Désigne une sortie GPU, pour la retirer (§8.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GpuSinkId(u32);

struct CpuTarget {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

impl CpuTarget {
    fn new(gpu: &Gpu, width: u32, height: u32) -> Self {
        let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("outputs.cpu_target"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: CPU_TARGET_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = texture.create_view(&Default::default());
        Self { texture, view }
    }
}

/// Alimente toutes les sorties, une fois par frame.
///
/// Numérote les frames, relit **une seule fois** pour toutes les sorties CPU,
/// et seulement si l'une d'elles est active. La politique de perte vit dans
/// [`CpuFanout`], écrite une fois.
pub struct Outputs {
    conversion: ConversionPass,
    gpu_sinks: Vec<(GpuSinkId, Box<dyn GpuSink>)>,
    next_gpu_id: u32,
    fanout: CpuFanout,
    /// Créée à la première relecture, recréée si la résolution change.
    cpu_target: Option<CpuTarget>,
    next_index: u64,
}

impl Outputs {
    pub fn new(gpu: &Gpu, policy: DropPolicy) -> Self {
        Self {
            conversion: ConversionPass::new(gpu, CPU_TARGET_FORMAT, ConversionMode::Straight),
            gpu_sinks: Vec::new(),
            next_gpu_id: 0,
            fanout: CpuFanout::new(policy),
            cpu_target: None,
            next_index: 0,
        }
    }

    pub fn add(&mut self, sink: OutputSink) -> Result<(), OutputError> {
        match sink {
            OutputSink::Gpu(sink) => {
                self.add_gpu(sink);
            }
            OutputSink::Cpu(sink) => self.add_cpu(sink)?,
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

    fn add_cpu(&mut self, sink: Box<dyn CpuSink>) -> Result<(), OutputError> {
        if self.fanout.policy() == DropPolicy::Lossy {
            return Err(OutputError::CpuSinkRequiresFixedClock);
        }
        self.fanout.add(sink);
        Ok(())
    }

    /// Étape 9 du §20.1, à partir de la texture finale du compositeur.
    pub fn submit(&mut self, gpu: &Gpu, texture: &wgpu::Texture, t: f64) {
        let index = self.next_index;
        self.next_index += 1;

        let (width, height) = (texture.width(), texture.height());
        let lecture = self.fanout.has_active();
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
        if lecture {
            self.ensure_cpu_target(gpu, width, height);
            if let Some(cible) = &self.cpu_target {
                self.conversion
                    .encode(gpu, &mut encoder, texture, &cible.view, None);
            }
        }
        gpu.queue.submit(std::iter::once(encoder.finish()));

        for (_, sink) in self.gpu_sinks.iter_mut().filter(actives) {
            sink.after_submit(gpu);
        }

        if lecture {
            self.read_and_publish(gpu, width, height, index, t);
        }
    }

    fn ensure_cpu_target(&mut self, gpu: &Gpu, width: u32, height: u32) {
        let a_jour = self
            .cpu_target
            .as_ref()
            .is_some_and(|c| c.texture.width() == width && c.texture.height() == height);
        if !a_jour {
            self.cpu_target = Some(CpuTarget::new(gpu, width, height));
        }
    }

    fn read_and_publish(&mut self, gpu: &Gpu, width: u32, height: u32, index: u64, t: f64) {
        let octets = self
            .cpu_target
            .as_ref()
            .and_then(|cible| read_rgba8(gpu, &cible.texture));
        match octets {
            Some(octets) => self.fanout.publish(CpuFrame {
                pixels: Arc::from(octets),
                width,
                height,
                index,
                t,
            }),
            None => self.fanout.count_lost(),
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
        self.fanout.finish();
    }
}
