//! Écriture d'une image PNG — la « première image » de l'étape 5 (§12).

use crate::convert::linear_premul_to_srgb8;
use std::io;
use std::path::Path;

/// Écrit des pixels linéaires prémultipliés (relus du compositeur) en PNG
/// sRGB 8 bits, alpha droit — la seule conversion vers sRGB de la chaîne
/// (§21).
pub fn write_png(path: &Path, width: u32, height: u32, pixels: &[[f32; 4]]) -> io::Result<()> {
    if pixels.len() != (width as usize) * (height as usize) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} pixels pour une image {width}×{height}", pixels.len()),
        ));
    }
    let octets: Vec<u8> = pixels
        .iter()
        .flat_map(|px| linear_premul_to_srgb8(*px))
        .collect();

    let fichier = io::BufWriter::new(std::fs::File::create(path)?);
    let mut encoder = png::Encoder::new(fichier, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(io::Error::other)?;
    writer.write_image_data(&octets).map_err(io::Error::other)?;
    writer.finish().map_err(io::Error::other)
}
