//! Formules de mélange, en alpha prémultiplié — §21, §22 de
//! `docs/ARCHITECTURE.md`.
//!
//! **Ce module ne connaît pas wgpu.** C'est la référence CPU de
//! `composite.wgsl` : les tests de pixels du compositeur comparent la sortie
//! du GPU à ces fonctions, ce qui garantit que shader et spécification ne
//! divergent pas en silence.
//!
//! Toutes les couleurs sont en **linéaire prémultiplié**. Pour les modes
//! séparables (`Multiply`, `Screen`, `Difference`), la formule est celle du
//! W3C Compositing (`co = cs·(1−αb) + cb·(1−αs) + αs·αb·B(Cs, Cb)`), écrite
//! directement sur les valeurs prémultipliées — sans jamais diviser par
//! alpha, donc sans cas particulier à alpha nul.

use frogenx_layer::BlendMode;

/// Saturation de l'opacité chez le consommateur (§18.4) : un `NaN` tombe à
/// 0 — `clamp` le laisserait passer.
pub fn saturate_opacity(v: f32) -> f32 {
    if v.is_finite() {
        v.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// Mélange `src` (la couche) sur `dst` (ce qu'il y a dessous), après avoir
/// appliqué l'`opacity` de la couche. Couleurs en linéaire prémultiplié.
pub fn blend_premul(mode: BlendMode, dst: [f32; 4], src: [f32; 4], opacity: f32) -> [f32; 4] {
    let o = saturate_opacity(opacity);
    let s = src.map(|c| c * o);
    let (sa, da) = (s[3], dst[3]);
    let canal = |i: usize| {
        let (sc, dc) = (s[i], dst[i]);
        match mode {
            BlendMode::Normal => sc + dc * (1.0 - sa),
            BlendMode::Add => sc + dc,
            BlendMode::Multiply => sc * dc + sc * (1.0 - da) + dc * (1.0 - sa),
            BlendMode::Screen => sc + dc - sc * dc,
            BlendMode::Difference => sc + dc - 2.0 * (sc * da).min(dc * sa),
        }
    };
    [canal(0), canal(1), canal(2), sa + da * (1.0 - sa)]
}

#[cfg(test)]
mod tests {
    use super::*;

    const EPS: f32 = 1e-6;
    const ROUGE: [f32; 4] = [1.0, 0.0, 0.0, 1.0];
    const GRIS: [f32; 4] = [0.5, 0.5, 0.5, 1.0];
    const VIDE: [f32; 4] = [0.0; 4];

    fn proche(a: [f32; 4], b: [f32; 4]) -> bool {
        a.iter().zip(b.iter()).all(|(x, y)| (x - y).abs() < EPS)
    }

    #[test]
    fn une_couche_a_opacite_nulle_ne_change_rien() {
        for mode in BlendMode::ALL {
            assert!(
                proche(blend_premul(mode, GRIS, ROUGE, 0.0), GRIS),
                "{mode:?}"
            );
        }
    }

    #[test]
    fn une_couche_transparente_ne_change_rien() {
        for mode in BlendMode::ALL {
            assert!(
                proche(blend_premul(mode, GRIS, VIDE, 1.0), GRIS),
                "{mode:?}"
            );
        }
    }

    #[test]
    fn sur_un_fond_vide_tous_les_modes_rendent_la_couche() {
        // Sans rien dessous, aucun mode n'a de quoi se mélanger.
        for mode in BlendMode::ALL {
            assert!(
                proche(blend_premul(mode, VIDE, ROUGE, 1.0), ROUGE),
                "{mode:?}"
            );
        }
    }

    #[test]
    fn normal_opaque_recouvre() {
        assert!(proche(
            blend_premul(BlendMode::Normal, GRIS, ROUGE, 1.0),
            ROUGE
        ));
    }

    #[test]
    fn normal_a_demi_opacite_moyenne_en_lineaire() {
        let r = blend_premul(BlendMode::Normal, GRIS, ROUGE, 0.5);
        assert!(proche(r, [0.75, 0.25, 0.25, 1.0]), "{r:?}");
    }

    #[test]
    fn add_somme_les_lumieres() {
        let r = blend_premul(BlendMode::Add, GRIS, [0.25, 0.25, 0.25, 1.0], 1.0);
        assert!(proche(r, [0.75, 0.75, 0.75, 1.0]), "{r:?}");
    }

    #[test]
    fn multiply_par_le_blanc_est_lidentite() {
        let blanc = [1.0, 1.0, 1.0, 1.0];
        assert!(proche(
            blend_premul(BlendMode::Multiply, GRIS, blanc, 1.0),
            GRIS
        ));
        let r = blend_premul(BlendMode::Multiply, GRIS, ROUGE, 1.0);
        assert!(proche(r, [0.5, 0.0, 0.0, 1.0]), "{r:?}");
    }

    #[test]
    fn screen_par_le_noir_est_lidentite() {
        let noir = [0.0, 0.0, 0.0, 1.0];
        assert!(proche(
            blend_premul(BlendMode::Screen, GRIS, noir, 1.0),
            GRIS
        ));
        let r = blend_premul(BlendMode::Screen, GRIS, GRIS, 1.0);
        assert!(proche(r, [0.75, 0.75, 0.75, 1.0]), "{r:?}");
    }

    #[test]
    fn difference_dune_couleur_avec_elle_meme_est_noire() {
        let r = blend_premul(BlendMode::Difference, GRIS, GRIS, 1.0);
        assert!(proche(r, [0.0, 0.0, 0.0, 1.0]), "{r:?}");
        let r = blend_premul(BlendMode::Difference, GRIS, ROUGE, 1.0);
        assert!(proche(r, [0.5, 0.5, 0.5, 1.0]), "{r:?}");
    }

    #[test]
    fn lopacite_degeneree_est_saturee_sans_paniquer() {
        assert_eq!(saturate_opacity(f32::NAN), 0.0);
        assert_eq!(saturate_opacity(f32::INFINITY), 0.0);
        assert_eq!(saturate_opacity(-1.0), 0.0);
        assert_eq!(saturate_opacity(5.0), 1.0);
        assert!(proche(
            blend_premul(BlendMode::Normal, GRIS, ROUGE, f32::NAN),
            GRIS
        ));
    }
}
