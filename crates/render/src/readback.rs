//! Relecture GPU → CPU — §8.2 de `docs/ARCHITECTURE.md`.
//!
//! **Version synchrone** : la première image, les tests de pixels, le rendu
//! offline. Elle attend le GPU, ce qui est correct hors temps réel et
//! **interdit** dans la boucle live : là, l'enregistrement passera par un
//! anneau de 2 à 3 buffers de readback, la frame N-2 étant lue pendant que la
//! N se dessine (§8.2). Cette fonction n'est pas ce mécanisme-là.

use crate::convert::f16_to_f32;
use frogenx_layer::Gpu;

/// Octets par pixel d'une texture `Rgba16Float`.
const BYTES_PER_PIXEL_F16: u32 = 8;
/// Octets par pixel d'une texture `Rgba8*`.
const BYTES_PER_PIXEL_U8: u32 = 4;

/// Lit une texture `Rgba16Float` et rend ses pixels, ligne par ligne, de haut
/// en bas, en `f32` linéaire prémultiplié.
///
/// Rend `None` si le GPU ou la mise en mémoire échoue — jamais de panique
/// (§18.1). Bloque jusqu'à la fin du travail GPU en cours.
pub fn read_rgba16f(gpu: &Gpu, texture: &wgpu::Texture) -> Option<Vec<[f32; 4]>> {
    let octets = read_bytes(gpu, texture, BYTES_PER_PIXEL_F16)?;
    let pixels = octets
        .chunks_exact(BYTES_PER_PIXEL_F16 as usize)
        .map(|px| {
            let c = |i: usize| f16_to_f32(u16::from_le_bytes([px[2 * i], px[2 * i + 1]]));
            [c(0), c(1), c(2), c(3)]
        })
        .collect();
    Some(pixels)
}

/// Lit une texture `Rgba8*` et rend ses octets, ligne par ligne, de haut en
/// bas, sans remplissage entre les lignes.
///
/// Mêmes garanties que [`read_rgba16f`], et même limite : elle bloque, donc
/// elle n'a pas sa place dans la boucle live (§8.2).
pub fn read_rgba8(gpu: &Gpu, texture: &wgpu::Texture) -> Option<Vec<u8>> {
    read_bytes(gpu, texture, BYTES_PER_PIXEL_U8)
}

fn read_bytes(gpu: &Gpu, texture: &wgpu::Texture, bytes_per_pixel: u32) -> Option<Vec<u8>> {
    let (w, h) = (texture.width(), texture.height());
    // wgpu impose des lignes alignées sur 256 octets dans le buffer de copie.
    let ligne = w * bytes_per_pixel;
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

    let octets = {
        let data = slice.get_mapped_range().ok()?;
        let mut octets = Vec::with_capacity((ligne * h) as usize);
        for y in 0..h as usize {
            let debut = y * ligne_alignee as usize;
            octets.extend_from_slice(&data[debut..debut + ligne as usize]);
        }
        octets
    };
    buffer.unmap();
    Some(octets)
}
