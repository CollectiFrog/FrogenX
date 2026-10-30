//! La preview — §8.3 de `docs/ARCHITECTURE.md`.
//!
//! Une sortie comme les autres : elle reçoit une `wgpu::Surface`, jamais une
//! fenêtre. Le moteur a sa propre échéance ; une preview indisponible saute sa
//! frame sans rien ralentir.

use crate::conversion::{ConversionMode, ConversionPass, Rect};
use crate::outputs::OutputError;
use crate::sink::{GpuFrame, GpuSink, SinkStatus};
use frogenx_layer::Gpu;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// `Mailbox` si la surface le propose : le moteur n'est pas ralenti, et l'image
/// ne se déchire pas. Sinon `Fifo`, qui existe partout. Jamais `Immediate`, qui
/// déchire l'image sur un projecteur.
pub fn pick_present_mode(modes: &[wgpu::PresentMode]) -> wgpu::PresentMode {
    if modes.contains(&wgpu::PresentMode::Mailbox) {
        wgpu::PresentMode::Mailbox
    } else {
        wgpu::PresentMode::Fifo
    }
}

/// Un format `*Srgb` si possible, le matériel encode alors ; sinon du 8 bits,
/// encodé par la passe. Rend `None` si la surface n'en propose aucun.
pub fn pick_format(formats: &[wgpu::TextureFormat]) -> Option<wgpu::TextureFormat> {
    use wgpu::TextureFormat as F;
    formats
        .iter()
        .find(|f| matches!(f, F::Bgra8UnormSrgb | F::Rgba8UnormSrgb))
        .or_else(|| {
            formats
                .iter()
                .find(|f| matches!(f, F::Bgra8Unorm | F::Rgba8Unorm))
        })
        .or(formats.first())
        .copied()
}

/// Le rectangle de `target` qui montre `source` sans la déformer, centré.
///
/// En pixels entiers : wgpu refuse un viewport qui dépasse la cible, et un
/// calcul en flottants pourrait la dépasser d'un arrondi.
pub fn letterbox(source: (u32, u32), target: (u32, u32)) -> Rect {
    let (sw, sh) = (source.0 as f64, source.1 as f64);
    let (tw, th) = (target.0 as f64, target.1 as f64);
    if sw <= 0.0 || sh <= 0.0 {
        return Rect {
            x: 0,
            y: 0,
            width: target.0,
            height: target.1,
        };
    }
    let echelle = (tw / sw).min(th / sh);
    let width = ((sw * echelle).round() as u32).min(target.0);
    let height = ((sh * echelle).round() as u32).min(target.1);
    Rect {
        x: (target.0 - width) / 2,
        y: (target.1 - height) / 2,
        width,
        height,
    }
}

#[derive(Default)]
struct Control {
    size: Option<(u32, u32)>,
    surface: Option<wgpu::Surface<'static>>,
    lost: bool,
    presented: u64,
}

/// Ce que `app` garde de la preview une fois celle-ci confiée à `Outputs`.
///
/// `app` possède la fenêtre : c'est lui qui connaît la nouvelle taille, et lui
/// seul peut recréer une surface perdue (§8.3).
#[derive(Clone)]
pub struct PreviewHandle(Arc<Mutex<Control>>);

impl PreviewHandle {
    fn lock(&self) -> MutexGuard<'_, Control> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// La fenêtre a changé de taille. Prise en compte à la frame suivante,
    /// jamais pendant une frame.
    pub fn resize(&self, width: u32, height: u32) {
        self.lock().size = Some((width, height));
    }

    /// La surface est perdue : `app` doit en recréer une.
    pub fn surface_lost(&self) -> bool {
        self.lock().lost
    }

    pub fn replace_surface(&self, surface: wgpu::Surface<'static>) {
        let mut control = self.lock();
        control.surface = Some(surface);
        control.lost = false;
    }

    /// Frames réellement présentées, depuis le début.
    pub fn presented(&self) -> u64 {
        self.lock().presented
    }
}

/// La sortie vers une fenêtre : le rendu en letterbox, sur du noir.
pub struct Preview {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    pass: ConversionPass,
    /// Acquise par `encode`, présentée par `after_submit`.
    pending: Option<wgpu::SurfaceTexture>,
    reconfigure: bool,
    /// Frames perdues parce que la surface n'a pas répondu. Une fenêtre
    /// simplement masquée n'en fait pas partie : rien n'y est anormal.
    skipped: u32,
    handle: PreviewHandle,
}

impl Preview {
    /// Rend aussi le [`PreviewHandle`], à garder par `app`.
    pub fn new(
        gpu: &Gpu,
        surface: wgpu::Surface<'static>,
        width: u32,
        height: u32,
    ) -> Result<(Self, PreviewHandle), OutputError> {
        let config = configuration(gpu, &surface, width, height)?;
        if config.width > 0 && config.height > 0 {
            surface.configure(&gpu.device, &config);
        }
        let pass = ConversionPass::new(gpu, config.format, ConversionMode::OverBlack);
        let handle = PreviewHandle(Arc::new(Mutex::new(Control::default())));
        let preview = Self {
            surface,
            config,
            pass,
            pending: None,
            reconfigure: false,
            skipped: 0,
            handle: handle.clone(),
        };
        Ok((preview, handle))
    }

    /// Applique ce que `app` a demandé depuis la dernière frame. Aucune texture
    /// n'est alors acquise : wgpu panique si l'on reconfigure avec une
    /// `SurfaceTexture` encore vivante.
    fn apply_control(&mut self, gpu: &Gpu) {
        let (size, surface) = {
            let mut control = self.handle.lock();
            (control.size.take(), control.surface.take())
        };
        if let Some(surface) = surface {
            if let Ok(config) = configuration(gpu, &surface, self.config.width, self.config.height)
            {
                if config.format != self.config.format {
                    self.pass = ConversionPass::new(gpu, config.format, ConversionMode::OverBlack);
                }
                self.config = config;
                self.surface = surface;
                self.reconfigure = true;
            }
        }
        if let Some((width, height)) = size {
            if (width, height) != (self.config.width, self.config.height) {
                self.config.width = width;
                self.config.height = height;
                self.reconfigure = true;
            }
        }
        if self.reconfigure {
            self.configure(gpu);
        }
    }

    /// wgpu panique sur une taille nulle : c'est celle d'une fenêtre réduite.
    fn configure(&mut self, gpu: &Gpu) {
        if self.config.width > 0 && self.config.height > 0 {
            self.surface.configure(&gpu.device, &self.config);
        }
        self.reconfigure = false;
    }
}

fn configuration(
    gpu: &Gpu,
    surface: &wgpu::Surface<'static>,
    width: u32,
    height: u32,
) -> Result<wgpu::SurfaceConfiguration, OutputError> {
    let capacites = surface.get_capabilities(&gpu.adapter);
    let mut config = surface
        .get_default_config(&gpu.adapter, width, height)
        .ok_or(OutputError::SurfaceNotCompatible)?;
    config.format = pick_format(&capacites.formats).ok_or(OutputError::SurfaceNotCompatible)?;
    config.present_mode = pick_present_mode(&capacites.present_modes);
    if capacites
        .alpha_modes
        .contains(&wgpu::CompositeAlphaMode::Opaque)
    {
        config.alpha_mode = wgpu::CompositeAlphaMode::Opaque;
    }
    Ok(config)
}

impl GpuSink for Preview {
    fn encode(&mut self, gpu: &Gpu, encoder: &mut wgpu::CommandEncoder, frame: &GpuFrame<'_>) {
        self.apply_control(gpu);
        if self.config.width == 0 || self.config.height == 0 || self.handle.surface_lost() {
            return;
        }
        let sortie = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(texture) => texture,
            wgpu::CurrentSurfaceTexture::Suboptimal(texture) => {
                self.reconfigure = true;
                texture
            }
            wgpu::CurrentSurfaceTexture::Occluded => return,
            wgpu::CurrentSurfaceTexture::Outdated => {
                self.configure(gpu);
                return;
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Validation => {
                self.skipped = self.skipped.saturating_add(1);
                return;
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                self.skipped = self.skipped.saturating_add(1);
                self.handle.lock().lost = true;
                return;
            }
        };
        let vue = sortie.texture.create_view(&Default::default());
        let rect = letterbox(
            (frame.texture.width(), frame.texture.height()),
            (self.config.width, self.config.height),
        );
        self.pass
            .encode(gpu, encoder, frame.texture, &vue, Some(rect));
        self.pending = Some(sortie);
    }

    fn after_submit(&mut self, gpu: &Gpu) {
        if let Some(sortie) = self.pending.take() {
            gpu.queue.present(sortie);
            self.handle.lock().presented += 1;
        }
    }

    fn status(&self) -> SinkStatus {
        if self.skipped > 0 {
            SinkStatus::Degraded {
                dropped: self.skipped,
            }
        } else {
            SinkStatus::Active
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wgpu::{PresentMode as P, TextureFormat as F};

    fn rect(x: u32, y: u32, width: u32, height: u32) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn une_sortie_16_9_dans_une_fenetre_4_3_a_des_bandes_en_haut_et_en_bas() {
        assert_eq!(
            letterbox((1920, 1080), (800, 600)),
            rect(0, 75, 800, 450),
            "bandes horizontales"
        );
    }

    #[test]
    fn une_sortie_4_3_dans_une_fenetre_16_9_a_des_bandes_laterales() {
        assert_eq!(
            letterbox((800, 600), (1920, 1080)),
            rect(240, 0, 1440, 1080)
        );
    }

    #[test]
    fn des_proportions_identiques_remplissent_la_cible() {
        assert_eq!(letterbox((1920, 1080), (1280, 720)), rect(0, 0, 1280, 720));
    }

    #[test]
    fn le_rectangle_ne_depasse_jamais_la_cible() {
        for (w, h) in [(1, 1), (3, 7), (1279, 719), (333, 1000), (4096, 11)] {
            let r = letterbox((1920, 1080), (w, h));
            assert!(r.x + r.width <= w && r.y + r.height <= h, "{w}×{h} : {r:?}");
        }
    }

    #[test]
    fn une_taille_nulle_ne_divise_pas_par_zero() {
        assert_eq!(letterbox((0, 0), (800, 600)), rect(0, 0, 800, 600));
        assert_eq!(letterbox((1920, 1080), (0, 0)), rect(0, 0, 0, 0));
    }

    #[test]
    fn mailbox_est_prefere_et_immediate_jamais_choisi() {
        assert_eq!(pick_present_mode(&[P::Fifo, P::Mailbox]), P::Mailbox);
        assert_eq!(pick_present_mode(&[P::Immediate, P::Fifo]), P::Fifo);
        assert_eq!(pick_present_mode(&[P::Immediate]), P::Fifo);
    }

    #[test]
    fn un_format_srgb_est_prefere_puis_un_format_8_bits() {
        assert_eq!(
            pick_format(&[F::Rgba16Float, F::Bgra8Unorm, F::Bgra8UnormSrgb]),
            Some(F::Bgra8UnormSrgb)
        );
        assert_eq!(
            pick_format(&[F::Rgba16Float, F::Rgba8Unorm]),
            Some(F::Rgba8Unorm)
        );
        assert_eq!(pick_format(&[F::Rgba16Float]), Some(F::Rgba16Float));
        assert_eq!(pick_format(&[]), None);
    }
}
