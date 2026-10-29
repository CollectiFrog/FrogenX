//! Espace normalisé → espace écran — §17 de `docs/ARCHITECTURE.md`.
//!
//! **Ce module ne connaît pas wgpu.**
//!
//! Les formes vivent dans un espace normalisé : origine au centre, `[-1, 1]`
//! sur le **petit côté** de la cible. Les coordonnées de clip du GPU (NDC),
//! elles, vont de -1 à 1 sur **chaque** axe, quelle que soit la proportion.
//! Sans correction, un cercle deviendrait une ellipse en 16:9.

use glam::{UVec2, Vec2};

/// Facteur par lequel multiplier un point de l'espace normalisé pour obtenir
/// ses coordonnées NDC sur une cible de `size` pixels.
///
/// Le petit côté garde l'échelle 1 ; le grand côté est réduit dans la
/// proportion `petit / grand`. Une taille nulle est traitée comme 1 pixel :
/// jamais de division par zéro (§18.1).
pub fn normalized_to_ndc(size: UVec2) -> Vec2 {
    let w = size.x.max(1) as f32;
    let h = size.y.max(1) as f32;
    if w >= h {
        Vec2::new(h / w, 1.0)
    } else {
        Vec2::new(1.0, w / h)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn une_cible_carree_ne_corrige_rien() {
        assert_eq!(normalized_to_ndc(UVec2::new(512, 512)), Vec2::ONE);
    }

    #[test]
    fn en_paysage_seul_x_est_reduit() {
        let s = normalized_to_ndc(UVec2::new(1920, 1080));
        assert!((s.x - 0.5625).abs() < 1e-6);
        assert_eq!(s.y, 1.0);
    }

    #[test]
    fn en_portrait_seul_y_est_reduit() {
        let s = normalized_to_ndc(UVec2::new(1080, 1920));
        assert_eq!(s.x, 1.0);
        assert!((s.y - 0.5625).abs() < 1e-6);
    }

    #[test]
    fn un_cercle_unite_reste_rond_en_pixels() {
        // Le rayon 1 doit couvrir la même distance en pixels sur les deux axes.
        let size = UVec2::new(1920, 1080);
        let s = normalized_to_ndc(size);
        let rayon_x_px = s.x * size.x as f32 * 0.5;
        let rayon_y_px = s.y * size.y as f32 * 0.5;
        assert!((rayon_x_px - rayon_y_px).abs() < 1e-3);
    }

    #[test]
    fn une_taille_nulle_ne_divise_pas_par_zero() {
        let s = normalized_to_ndc(UVec2::new(0, 0));
        assert!(s.x.is_finite() && s.y.is_finite());
    }
}
