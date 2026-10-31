//! Le contrat des couches — §6, §22 de `docs/ARCHITECTURE.md`.

use crate::gpu::Gpu;
use frogenx_core::{Ctx, Graph};
use frogenx_shape::BlendParams;
use glam::UVec2;

/// Un objet qui produit une texture — §22.
///
/// Trois appels plutôt que `render(&mut self) -> &TextureView` : le
/// compositeur doit tenir les textures de **toutes** les couches en même
/// temps, ce qu'un emprunt mutable par couche interdirait ; et toutes les
/// couches enregistrent leurs passes dans **un seul** encodeur.
///
/// Cycle par frame : `prepare` sur chaque couche, puis `encode` sur chaque
/// couche, puis `output` sur toutes.
pub trait RenderLayer {
    /// Travail CPU : génération, tessellation, upload des buffers. Tout ce
    /// qui change d'une frame à l'autre est écrit ici.
    fn prepare(&mut self, t: f64, ctx: &Ctx, g: &Graph, gpu: &Gpu);

    /// Enregistre la passe de rendu dans l'encodeur partagé. Ne fait que
    /// référencer des ressources déjà prêtes : aucun travail CPU coûteux.
    fn encode(&self, encoder: &mut wgpu::CommandEncoder);

    /// Prête la texture de la couche, en lecture.
    fn output(&self) -> &wgpu::TextureView;

    fn blend(&self) -> BlendParams;

    /// Change la résolution. Appelé hors de la boucle de rendu, jamais
    /// pendant une frame (§22).
    fn resize(&mut self, size: UVec2, gpu: &Gpu);
}

/// Diagnostics propres au rendu — distincts de `core::Diagnostics`, qui ne
/// doit rien savoir du GPU (§22).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RenderDiagnostics {
    /// Le MSAA 4× n'est pas disponible pour ce format : la couche rend en 1×.
    pub msaa_fallback: bool,
    /// Échecs de tessellation sur la dernière frame (forme ignorée).
    pub tessellation_failures: u32,
}
