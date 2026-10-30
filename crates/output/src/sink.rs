//! Le contrat des sorties — §8.0 de `docs/ARCHITECTURE.md`.

use frogenx_layer::Gpu;
use std::sync::Arc;

/// L'état d'une sortie, lu par l'UI sans interrompre la performance.
///
/// C'est ce qui remplace un `Result` par frame (§18.1) : une panne reste
/// visible, sans produire 60 erreurs par seconde.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SinkStatus {
    Active,
    /// Fonctionne, mais perd des frames ou se reconfigure.
    Degraded {
        dropped: u32,
    },
    /// N'est plus alimentée. Reste listée : c'est l'utilisateur qui décide.
    Failed(String),
}

/// Une frame pour les sorties GPU.
pub struct GpuFrame<'a> {
    /// Sortie du compositeur : `Rgba16Float` linéaire prémultiplié (§21).
    pub texture: &'a wgpu::Texture,
    /// Croissant, sans trou en `Clock::Fixed`.
    pub index: u64,
    pub t: f64,
}

/// Une frame pour les sorties CPU, partagée entre elles sans copie.
#[derive(Debug, Clone)]
pub struct CpuFrame {
    /// RGBA8 encodé sRGB, alpha droit, lignes contiguës (§8.0).
    pub pixels: Arc<[u8]>,
    pub width: u32,
    pub height: u32,
    pub index: u64,
    /// Le `t` de la frame relue, pas le `t` courant (§8.2).
    pub t: f64,
}

/// Une sortie qui reste sur le GPU : preview, Spout, Syphon.
///
/// Deux phases parce que `surface.present()`, le signal Spout et la
/// publication Syphon viennent tous **après** `queue.submit`.
pub trait GpuSink {
    /// Enregistre ses copies et ses passes dans l'encodeur de l'étage.
    fn encode(&mut self, gpu: &Gpu, encoder: &mut wgpu::CommandEncoder, frame: &GpuFrame<'_>);

    /// Appelée une fois la frame soumise.
    fn after_submit(&mut self, gpu: &Gpu);

    fn status(&self) -> SinkStatus;
}

/// Une sortie qui a besoin d'octets en mémoire : NDI, enregistreur, Art-Net,
/// séquence d'images.
///
/// `submit` est appelée sur le thread propre à la sortie : elle peut bloquer
/// sur le disque ou le réseau. La politique de perte n'est pas son affaire,
/// elle appartient à l'étage `Outputs` (§8.0).
pub trait CpuSink: Send {
    fn submit(&mut self, frame: &CpuFrame);

    fn status(&self) -> SinkStatus;
}

/// N'importe quelle sortie. Un trait par famille : une sortie ne peut pas
/// recevoir la mauvaise forme de frame, et le compilateur le garantit.
pub enum OutputSink {
    Gpu(Box<dyn GpuSink>),
    Cpu(Box<dyn CpuSink>),
}
