//! Relecture GPU → CPU — §8.2 de `docs/ARCHITECTURE.md`.
//!
//! **Version synchrone, pour l'étape 5** : la première image, les tests de
//! pixels. Elle attend le GPU, ce qui est correct hors temps réel et
//! **interdit** dans la boucle live : là, l'enregistrement passera par un
//! anneau de 2 à 3 buffers de readback, la frame N-2 étant lue pendant que la
//! N se dessine (§8.2). Cette fonction n'est pas ce mécanisme-là.

use crate::convert::f16_to_f32;
use frogenx_layer::Gpu;

/// Octets par pixel d'une texture `Rgba16Float`.
const BYTES_PER_PIXEL: u32 = 8;

/// Lit une texture `Rgba16Float` et rend ses pixels, ligne par ligne, de haut
/// en bas, en `f32` linéaire prémultiplié.
///
/// Rend `None` si le GPU ou la mise en mémoire échoue — jamais de panique
/// (§18.1). Bloque jusqu'à la fin du travail GPU en cours.
pub fn read_rgba16f(gpu: &Gpu, texture: &wgpu::Texture) -> Option<Vec<[f32; 4]>> {
    let (w, h) = (texture.width(), texture.height());
    // wgpu impose des lignes alignées sur 256 octets dans le buffer de copie.
    let ligne = w * BYTES_PER_PIXEL;
    let ligne_alignee =
        ligne.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;

    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("readback"),
        size: (ligne_alignee * h) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("readback"),
        });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(ligne_alignee),
                rows_per_image: Some(h),
            },
        },
        wgpu::Extent3d {
            width: w,
            height: h,
            depth_or_array_layers: 1,
        },
    );
    gpu.queue.submit(std::iter::once(encoder.finish()));

    let (tx, rx) = std::sync::mpsc::channel();
    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, move |r| {
        let _ = tx.send(r);
    });
    gpu.device.poll(wgpu::PollType::wait_indefinitely()).ok()?;
    rx.recv().ok()?.ok()?;

    let pixels = {
        let data = slice.get_mapped_range().ok()?;
        let mut pixels = Vec::with_capacity((w * h) as usize);
        for y in 0..h as usize {
            let debut = y * ligne_alignee as usize;
            let rangee = &data[debut..debut + ligne as usize];
            pixels.extend(rangee.chunks_exact(BYTES_PER_PIXEL as usize).map(|px| {
                let c = |i: usize| f16_to_f32(u16::from_le_bytes([px[2 * i], px[2 * i + 1]]));
                [c(0), c(1), c(2), c(3)]
            }));
        }
        pixels
    };
    buffer.unmap();
    Some(pixels)
}
