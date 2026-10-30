//! Conversions de sortie — §21 de `docs/ARCHITECTURE.md`.
//!
//! **Ce module ne connaît pas wgpu.** Toute la chaîne travaille en linéaire
//! prémultiplié, en demi-flottants (`Rgba16Float`). Ce n'est qu'à la sortie
//! — un fichier, un écran — que l'image repasse en sRGB 8 bits, alpha droit.
//! Une image ne subit cette conversion **qu'une fois** : la faire deux fois
//! délave l'image, l'oublier l'assombrit. [`linear_premul_to_srgb8`] est aussi
//! la référence CPU de la passe de conversion de `frogenx-output` : toute
//! modification ici se reporte dans `conversion.wgsl`, et inversement.

/// Demi-flottant IEEE 754 binary16 → `f32`.
///
/// Écrit à la main plutôt que de prendre une dépendance pour quinze lignes ;
/// les valeurs de référence des tests couvrent normaux, sous-normaux, zéros
/// signés, infinis et NaN.
pub fn f16_to_f32(bits: u16) -> f32 {
    let signe = if bits & 0x8000 != 0 { -1.0 } else { 1.0 };
    let exposant = (bits >> 10) & 0x1f;
    let mantisse = (bits & 0x3ff) as f32;
    match exposant {
        0 => signe * mantisse * 2f32.powi(-24),
        0x1f if mantisse == 0.0 => signe * f32::INFINITY,
        0x1f => f32::NAN,
        e => signe * (1.0 + mantisse / 1024.0) * 2f32.powi(e as i32 - 15),
    }
}

/// Linéaire → sRGB, pour un canal dans `[0, 1]` (courbe IEC 61966-2-1).
pub fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

fn sature(v: f32) -> f32 {
    if v.is_finite() {
        v.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// Un pixel linéaire prémultiplié → sRGB 8 bits, alpha droit (ce qu'attend
/// un PNG).
///
/// Un pixel transparent rend `[0, 0, 0, 0]` : dé-prémultiplier diviserait
/// par zéro. Les canaux hors domaine (un mode `Add` peut dépasser 1) sont
/// saturés ici, chez le consommateur final (§18.4).
pub fn linear_premul_to_srgb8(px: [f32; 4]) -> [u8; 4] {
    let a = sature(px[3]);
    if a <= 0.0 {
        return [0; 4];
    }
    let q = |v: f32| (sature(v) * 255.0 + 0.5).floor() as u8;
    let c = |i: usize| q(linear_to_srgb(sature(px[i] / a)));
    [c(0), c(1), c(2), q(a)]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f16_aux_valeurs_de_reference() {
        assert_eq!(f16_to_f32(0x0000), 0.0);
        assert_eq!(f16_to_f32(0x3c00), 1.0);
        assert_eq!(f16_to_f32(0x3800), 0.5);
        assert_eq!(f16_to_f32(0xc000), -2.0);
        assert_eq!(f16_to_f32(0x7bff), 65504.0, "plus grand normal");
        assert_eq!(f16_to_f32(0x0001), 2f32.powi(-24), "plus petit sous-normal");
        assert_eq!(f16_to_f32(0x7c00), f32::INFINITY);
        assert_eq!(f16_to_f32(0xfc00), f32::NEG_INFINITY);
        assert!(f16_to_f32(0x7e00).is_nan());
        assert!(f16_to_f32(0x8000) == 0.0 && f16_to_f32(0x8000).is_sign_negative());
    }

    #[test]
    fn linear_to_srgb_inverse_srgb_to_linear() {
        for i in 0..=100 {
            let c = i as f32 / 100.0;
            let aller_retour = linear_to_srgb(frogenx_layer::srgb_to_linear(c));
            assert!((aller_retour - c).abs() < 1e-5, "{c} → {aller_retour}");
        }
    }

    #[test]
    fn un_pixel_opaque_est_converti_en_srgb() {
        // Le gris moyen sRGB (128) vaut environ 0.216 en linéaire, pas 0.5.
        let l = frogenx_layer::srgb_to_linear(128.0 / 255.0);
        assert!((l - 0.2158).abs() < 1e-3);
        assert_eq!(linear_premul_to_srgb8([l, l, l, 1.0]), [128, 128, 128, 255]);
        assert_eq!(
            linear_premul_to_srgb8([1.0, 0.0, 0.0, 1.0]),
            [255, 0, 0, 255]
        );
    }

    #[test]
    fn un_pixel_semi_transparent_est_de_premultiplie() {
        // Rouge pur à 50 % : prémultiplié (0.5, 0, 0, 0.5) → (255, 0, 0, 128).
        assert_eq!(
            linear_premul_to_srgb8([0.5, 0.0, 0.0, 0.5]),
            [255, 0, 0, 128]
        );
    }

    #[test]
    fn un_pixel_transparent_ne_divise_pas_par_zero() {
        assert_eq!(linear_premul_to_srgb8([0.3, 0.3, 0.3, 0.0]), [0; 4]);
    }

    #[test]
    fn les_valeurs_hors_domaine_sont_saturees() {
        assert_eq!(
            linear_premul_to_srgb8([4.0, -1.0, f32::NAN, 1.0]),
            [255, 0, 0, 255]
        );
        assert_eq!(linear_premul_to_srgb8([1.0, 1.0, 1.0, f32::NAN]), [0; 4]);
    }
}
