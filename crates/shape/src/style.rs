//! Le style d'une forme et le mélange d'une piste — §10, §21 de
//! `docs/ARCHITECTURE.md`.
//!
//! Ces types vivent ici, pas dans `layer` : un clip et une piste de
//! `composition` les portent, et `composition` ne peut pas dépendre de wgpu.
//! Quatre `Param` et une énumération : rien en eux ne touche au GPU.
//!
//! Une couleur est **quatre `Param`**, jamais un `[f32; 4]` en dur — c'est un
//! champ qu'on voudra moduler par un LFO ou une feature audio (§16.4). Les
//! valeurs sont saisies en **sRGB** (ce que l'humain lit et saisit), puis
//! converties **une fois, pendant `prepare`** (§22) en linéaire prémultiplié,
//! l'espace de toute la chaîne de composition.

use frogenx_core::{Graph, Param};

/// sRGB → linéaire, pour un canal dans `[0, 1]` (courbe IEC 61966-2-1).
pub fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// Saturation chez le consommateur (§18.4) : un canal modulé hors de `[0, 1]`
/// est ramené dans le domaine, un non-fini tombe à 0 — `clamp` laisserait
/// passer un `NaN`.
fn canal(v: f32) -> f32 {
    if v.is_finite() {
        v.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// Une couleur modulable, saisie en sRGB, alpha droit (non prémultiplié).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorParam {
    pub r: Param,
    pub g: Param,
    pub b: Param,
    pub a: Param,
}

/// Le nom des canaux, dans l'ordre de [`ColorParam::channel_mut`].
pub const COLOR_CHANNELS: [&str; 4] = ["rouge", "vert", "bleu", "alpha"];

impl ColorParam {
    pub fn channels(&self) -> [Param; 4] {
        [self.r, self.g, self.b, self.a]
    }

    pub fn channel_mut(&mut self, index: usize) -> Option<&mut Param> {
        match index {
            0 => Some(&mut self.r),
            1 => Some(&mut self.g),
            2 => Some(&mut self.b),
            3 => Some(&mut self.a),
            _ => None,
        }
    }

    /// Couleur fixe, composantes sRGB dans `[0, 1]`.
    pub fn srgb(r: f32, g: f32, b: f32, a: f32) -> Self {
        Self {
            r: Param::fixed(r),
            g: Param::fixed(g),
            b: Param::fixed(b),
            a: Param::fixed(a),
        }
    }

    /// Résout la couleur en **linéaire prémultiplié**, après `eval_frame`.
    pub fn resolve(&self, g: &Graph) -> [f32; 4] {
        let a = canal(self.a.get(g));
        [
            srgb_to_linear(canal(self.r.get(g))) * a,
            srgb_to_linear(canal(self.g.get(g))) * a,
            srgb_to_linear(canal(self.b.get(g))) * a,
            a,
        ]
    }
}

/// Le trait d'une forme : largeur en unités de l'espace normalisé (§17).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StrokeStyle {
    pub width: Param,
    pub color: ColorParam,
}

/// Comment une forme se dessine : remplissage, trait, ou les deux.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct ShapeStyle {
    pub fill: Option<ColorParam>,
    pub stroke: Option<StrokeStyle>,
}

impl ShapeStyle {
    pub fn filled(color: ColorParam) -> Self {
        Self {
            fill: Some(color),
            stroke: None,
        }
    }

    pub fn stroked(width: Param, color: ColorParam) -> Self {
        Self {
            fill: None,
            stroke: Some(StrokeStyle { width, color }),
        }
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use frogenx_core::Constant;

    const EPS: f32 = 1e-4;

    #[test]
    fn srgb_to_linear_aux_valeurs_de_reference() {
        assert_eq!(srgb_to_linear(0.0), 0.0);
        assert!((srgb_to_linear(1.0) - 1.0).abs() < EPS);
        // Le gris moyen sRGB n'est pas à 50 % de lumière : c'est toute la
        // raison du §21.
        assert!((srgb_to_linear(0.5) - 0.214_041).abs() < EPS);
        // Segment linéaire sous le seuil.
        assert!((srgb_to_linear(0.04) - 0.04 / 12.92).abs() < 1e-7);
    }

    #[test]
    fn srgb_to_linear_est_continue_au_seuil() {
        let a = srgb_to_linear(0.04045 - 1e-6);
        let b = srgb_to_linear(0.04045 + 1e-6);
        assert!((a - b).abs() < 1e-5);
    }

    #[test]
    fn la_couleur_est_resolue_en_lineaire_premultiplie() {
        let g = Graph::new();
        let c = ColorParam::srgb(1.0, 0.5, 0.0, 0.5).resolve(&g);
        assert!((c[0] - 0.5).abs() < EPS);
        assert!((c[1] - 0.214_041 * 0.5).abs() < EPS);
        assert_eq!(c[2], 0.0);
        assert_eq!(c[3], 0.5);
    }

    #[test]
    fn les_canaux_hors_domaine_sont_satures_sans_paniquer() {
        let g = Graph::new();
        let c = ColorParam::srgb(2.0, -1.0, f32::NAN, f32::INFINITY).resolve(&g);
        assert_eq!(c, [0.0, 0.0, 0.0, 0.0], "alpha non fini → 0, donc tout à 0");

        let c = ColorParam::srgb(2.0, -1.0, f32::NAN, 1.0).resolve(&g);
        assert_eq!(c, [1.0, 0.0, 0.0, 1.0]);
    }

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

    #[test]
    fn un_style_par_defaut_ne_dessine_rien() {
        assert_eq!(ShapeStyle::default().fill, None);
        assert_eq!(ShapeStyle::default().stroke, None);
    }

    #[test]
    fn un_style_rempli_n_a_pas_de_trait_et_inversement() {
        let rouge = ColorParam::srgb(1.0, 0.0, 0.0, 1.0);
        assert_eq!(ShapeStyle::filled(rouge).stroke, None);
        assert_eq!(ShapeStyle::stroked(Param::fixed(0.1), rouge).fill, None);
    }

    #[test]
    fn un_canal_est_modulable() {
        // §16.4 : la couleur est un câble qu'on peut brancher.
        let mut g = Graph::new();
        let src = g.add(Box::new(Constant(0.5)));
        g.eval_frame(0.0, 1.0 / 60.0);
        let c = ColorParam {
            a: Param::modulated(0.0, 2.0, src),
            ..ColorParam::srgb(1.0, 1.0, 1.0, 0.0)
        };
        assert_eq!(c.resolve(&g), [1.0, 1.0, 1.0, 1.0]);
    }
}
