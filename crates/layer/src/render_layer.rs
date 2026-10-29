//! Le contrat des couches — §6, §22 de `docs/ARCHITECTURE.md`.

use crate::gpu::Gpu;
use frogenx_core::{Ctx, Graph, Param};
use glam::UVec2;

/// Comment une couche se mélange à ce qu'il y a dessous (§22).
///
/// L'indice de chaque variante est celui que lit le shader du compositeur :
/// ne pas réordonner sans mettre à jour `composite.wgsl`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlendMode {
    Normal = 0,
    Add = 1,
    Multiply = 2,
    Screen = 3,
    Difference = 4,
}

impl BlendMode {
    pub const ALL: [BlendMode; 5] = [
        BlendMode::Normal,
        BlendMode::Add,
        BlendMode::Multiply,
        BlendMode::Screen,
        BlendMode::Difference,
    ];
}

/// La seule donnée que le compositeur lit pour mélanger une couche (§22).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlendParams {
    /// Modulable — c'est par ce `Param` que le `CrossFader` agit (§3bis.4).
    pub opacity: Param,
    pub mode: BlendMode,
}

impl Default for BlendParams {
    fn default() -> Self {
        Self {
            opacity: Param::fixed(1.0),
            mode: BlendMode::Normal,
        }
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn les_indices_des_modes_sont_ceux_du_shader() {
        // composite.wgsl compare `mode` à ces entiers.
        let indices: Vec<u32> = BlendMode::ALL.iter().map(|m| *m as u32).collect();
        assert_eq!(indices, vec![0, 1, 2, 3, 4]);
    }

    #[test]
    fn le_melange_par_defaut_est_normal_et_opaque() {
        let b = BlendParams::default();
        assert_eq!(b.mode, BlendMode::Normal);
        assert_eq!(b.opacity, Param::fixed(1.0));
    }
}
