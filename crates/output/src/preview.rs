//! Présenter sur une surface — §8.3 de `docs/ARCHITECTURE.md`.
//!
//! [`PresentSurface`] porte ce qui est commun à toute fenêtre : acquisition,
//! reconfiguration, perte. [`Preview`] en fait une sortie comme les autres :
//! elle reçoit une `wgpu::Surface`, jamais une fenêtre. Le moteur a sa propre
//! échéance ; une surface indisponible saute sa frame sans rien ralentir.

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

/// Quel format préférer pour une surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormatPreference {
    /// Le matériel encode le sRGB : ce que veut la preview.
    Srgb,
    /// L'application écrit des valeurs déjà encodées : ce que veut egui.
    Unorm,
}

/// Un format 8 bits selon `preference`, sinon l'autre famille 8 bits, sinon le
/// premier proposé. Rend `None` si la surface n'en propose aucun.
pub fn pick_format(
    formats: &[wgpu::TextureFormat],
    preference: FormatPreference,
) -> Option<wgpu::TextureFormat> {
    use wgpu::TextureFormat as F;
    let srgb = || {
        formats
            .iter()
            .find(|f| matches!(f, F::Bgra8UnormSrgb | F::Rgba8UnormSrgb))
    };
    let unorm = || {
        formats
            .iter()
            .find(|f| matches!(f, F::Bgra8Unorm | F::Rgba8Unorm))
    };
    match preference {
        FormatPreference::Srgb => srgb().or_else(unorm),
        FormatPreference::Unorm => unorm().or_else(srgb),
    }
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

fn configuration(
    gpu: &Gpu,
    surface: &wgpu::Surface<'static>,
    width: u32,
    height: u32,
    preference: FormatPreference,
) -> Result<wgpu::SurfaceConfiguration, OutputError> {
    let capacites = surface.get_capabilities(&gpu.adapter);
    let mut config = surface
        .get_default_config(&gpu.adapter, width, height)
        .ok_or(OutputError::SurfaceNotCompatible)?;
    config.format =
        pick_format(&capacites.formats, preference).ok_or(OutputError::SurfaceNotCompatible)?;
    config.present_mode = pick_present_mode(&capacites.present_modes);
    if capacites
        .alpha_modes
        .contains(&wgpu::CompositeAlphaMode::Opaque)
    {
        config.alpha_mode = wgpu::CompositeAlphaMode::Opaque;
    }
    Ok(config)
}

/// Une surface et sa configuration : le tableau du §8.3, écrit une seule fois.
pub struct PresentSurface {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    preference: FormatPreference,
    reconfigure: bool,
    lost: bool,
    /// Frames perdues parce que la surface n'a pas répondu. Une fenêtre
    /// simplement masquée n'en fait pas partie : rien n'y est anormal.
    skipped: u32,
}

impl PresentSurface {
    pub fn new(
        gpu: &Gpu,
        surface: wgpu::Surface<'static>,
        width: u32,
        height: u32,
        preference: FormatPreference,
    ) -> Result<Self, OutputError> {
        let config = configuration(gpu, &surface, width, height, preference)?;
        let mut present = Self {
            surface,
            config,
            preference,
            reconfigure: true,
            lost: false,
            skipped: 0,
        };
        present.configure(gpu);
        Ok(present)
    }

    pub fn format(&self) -> wgpu::TextureFormat {
        self.config.format
    }

    pub fn size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    pub fn skipped(&self) -> u32 {
        self.skipped
    }

    /// La surface est perdue : seul qui possède la fenêtre peut la recréer.
    pub fn is_lost(&self) -> bool {
        self.lost
    }

    /// Prise en compte à la prochaine acquisition, jamais pendant une frame.
    pub fn resize(&mut self, width: u32, height: u32) {
        if (width, height) != self.size() {
            self.config.width = width;
            self.config.height = height;
            self.reconfigure = true;
        }
    }

    /// Remplace une surface perdue. Le format peut changer avec elle.
    pub fn replace(
        &mut self,
        gpu: &Gpu,
        surface: wgpu::Surface<'static>,
    ) -> Result<(), OutputError> {
        let (width, height) = self.size();
        self.config = configuration(gpu, &surface, width, height, self.preference)?;
        self.surface = surface;
        self.lost = false;
        self.reconfigure = true;
        Ok(())
    }

    /// wgpu panique sur une taille nulle, qui est celle d'une fenêtre réduite,
    /// et si une `SurfaceTexture` est encore vivante : d'où un seul point
    /// d'appel, avant d'acquérir.
    fn configure(&mut self, gpu: &Gpu) {
        if self.config.width > 0 && self.config.height > 0 {
            self.surface.configure(&gpu.device, &self.config);
            self.reconfigure = false;
        }
    }

    /// La texture de la frame, ou `None` s'il faut la sauter. L'appelant la
    /// présente par `gpu.queue.present`, après soumission.
    pub fn acquire(&mut self, gpu: &Gpu) -> Option<wgpu::SurfaceTexture> {
        if self.reconfigure {
            self.configure(gpu);
        }
        if self.reconfigure || self.lost || self.config.width == 0 || self.config.height == 0 {
            return None;
        }
        match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(texture) => Some(texture),
            wgpu::CurrentSurfaceTexture::Suboptimal(texture) => {
                self.reconfigure = true;
                Some(texture)
            }
            wgpu::CurrentSurfaceTexture::Occluded => None,
            wgpu::CurrentSurfaceTexture::Outdated => {
                self.reconfigure = true;
                None
            }
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Validation => {
                self.skipped = self.skipped.saturating_add(1);
                None
            }
            wgpu::CurrentSurfaceTexture::Lost => {
                self.skipped = self.skipped.saturating_add(1);
                self.lost = true;
                None
            }
        }
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
    surface: PresentSurface,
    pass: ConversionPass,
    /// Acquise par `encode`, présentée par `after_submit`.
    pending: Option<wgpu::SurfaceTexture>,
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
        let surface = PresentSurface::new(gpu, surface, width, height, FormatPreference::Srgb)?;
        let pass = ConversionPass::new(gpu, surface.format(), ConversionMode::OverBlack);
        let handle = PreviewHandle(Arc::new(Mutex::new(Control::default())));
        let preview = Self {
            surface,
            pass,
            pending: None,
            handle: handle.clone(),
        };
        Ok((preview, handle))
    }

    /// Applique ce que `app` a demandé depuis la dernière frame.
    fn apply_control(&mut self, gpu: &Gpu) {
        let (size, surface) = {
            let mut control = self.handle.lock();
            (control.size.take(), control.surface.take())
        };
        if let Some(surface) = surface {
            let format = self.surface.format();
            if self.surface.replace(gpu, surface).is_ok() && self.surface.format() != format {
                self.pass =
                    ConversionPass::new(gpu, self.surface.format(), ConversionMode::OverBlack);
            }
        }
        if let Some((width, height)) = size {
            self.surface.resize(width, height);
        }
    }
}

impl GpuSink for Preview {
    fn encode(&mut self, gpu: &Gpu, encoder: &mut wgpu::CommandEncoder, frame: &GpuFrame<'_>) {
        self.apply_control(gpu);
        let Some(sortie) = self.surface.acquire(gpu) else {
            if self.surface.is_lost() {
                self.handle.lock().lost = true;
            }
            return;
        };
        let vue = sortie.texture.create_view(&Default::default());
        let rect = letterbox(
            (frame.texture.width(), frame.texture.height()),
            self.surface.size(),
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
        match self.surface.skipped() {
            0 => SinkStatus::Active,
            dropped => SinkStatus::Degraded { dropped },
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
    fn la_preview_prefere_srgb_et_l_interface_unorm() {
        let tous = [F::Rgba16Float, F::Bgra8Unorm, F::Bgra8UnormSrgb];
        assert_eq!(
            pick_format(&tous, FormatPreference::Srgb),
            Some(F::Bgra8UnormSrgb)
        );
        assert_eq!(
            pick_format(&tous, FormatPreference::Unorm),
            Some(F::Bgra8Unorm)
        );
    }

    #[test]
    fn faute_de_mieux_on_prend_l_autre_famille_puis_le_premier_format() {
        assert_eq!(
            pick_format(&[F::Rgba16Float, F::Rgba8Unorm], FormatPreference::Srgb),
            Some(F::Rgba8Unorm)
        );
        assert_eq!(
            pick_format(&[F::Bgra8UnormSrgb], FormatPreference::Unorm),
            Some(F::Bgra8UnormSrgb)
        );
        assert_eq!(
            pick_format(&[F::Rgba16Float], FormatPreference::Srgb),
            Some(F::Rgba16Float)
        );
        assert_eq!(pick_format(&[], FormatPreference::Unorm), None);
    }
}
