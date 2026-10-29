//! Le contexte GPU — §10, §22 de `docs/ARCHITECTURE.md`.

/// Format de toutes les textures intermédiaires : couches, accumulation du
/// compositeur. Linéaire, demi-flottant — `Rgba8` produirait du banding dans
/// les tons sombres, là où se jouent les dégradés lents (§21). Seule la
/// sortie finale (écran, fichier) est convertie en sRGB.
pub const LAYER_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// Device et queue wgpu, plus l'adaptateur pour interroger ses capacités.
pub struct Gpu {
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
}

impl Gpu {
    /// Ouvre un GPU sans fenêtre (rendu offscreen, tests, enregistrement).
    ///
    /// Rend `None` si aucun adaptateur n'est disponible — une machine de CI
    /// sans GPU, par exemple. Jamais de panique : c'est à l'appelant de
    /// décider si l'absence de GPU est une erreur (§24.3).
    pub fn headless() -> Option<Self> {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            ..Default::default()
        }))
        .ok()?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("frogenx"),
            ..Default::default()
        }))
        .ok()?;
        Some(Self {
            adapter,
            device,
            queue,
        })
    }

    /// Nombre d'échantillons MSAA utilisable pour `format` : 4 si l'adaptateur
    /// sait à la fois rendre en 4× et résoudre ce format, 1 sinon (§22).
    ///
    /// Les capacités varient selon le backend et le matériel : on les
    /// interroge plutôt que de supposer.
    pub fn msaa_samples(&self, format: wgpu::TextureFormat) -> u32 {
        let flags = self.adapter.get_texture_format_features(format).flags;
        if flags.sample_count_supported(4)
            && flags.contains(wgpu::TextureFormatFeatureFlags::MULTISAMPLE_RESOLVE)
        {
            4
        } else {
            1
        }
    }
}
