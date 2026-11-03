//! Les effets d'image, et le premier : le CRT — §6.2 de `docs/ARCHITECTURE.md`.
//!
//! Ici pour que `composition` les porte sans connaître wgpu. Chaque calcul du
//! shader a sa version en Rust : elle lui sert de référence dans les tests, comme
//! pour le mélange (§22.1). Toute modification d'un côté doit être reportée de
//! l'autre.

use crate::generator::ParamDesc;
use frogenx_core::{Graph, Param, Spec, SpecError};

/// La plus longue rémanence, en secondes : au-delà, l'image ne s'efface plus.
const MAX_PERSISTENCE: f32 = 30.0;
const MIN_LINES: f32 = 16.0;
const MAX_LINES: f32 = 2160.0;

/// Ce que propose le menu : copiable, pour voyager dans une commande.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterKind {
    Crt,
}

impl FilterKind {
    pub const ALL: [FilterKind; 1] = [FilterKind::Crt];

    /// Un effet tel qu'on l'ajoute : visible, sans être violent.
    pub fn build(self) -> ImageFilter {
        match self {
            FilterKind::Crt => ImageFilter::Crt(CrtParams::default()),
        }
    }

    pub fn label(self) -> &'static str {
        self.build().kind()
    }
}

/// Un effet d'image : il agit sur la texture que le clip vient de rendre.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ImageFilter {
    Crt(CrtParams),
}

impl ImageFilter {
    pub fn kind(&self) -> &'static str {
        match self {
            ImageFilter::Crt(_) => "CRT",
        }
    }

    pub fn params(&self) -> Vec<ParamDesc> {
        match self {
            ImageFilter::Crt(p) => p.params(),
        }
    }

    pub fn param_mut(&mut self, index: usize) -> Option<&mut Param> {
        match self {
            ImageFilter::Crt(p) => p.param_mut(index),
        }
    }

    pub fn spec(&self) -> Spec {
        self.params()
            .into_iter()
            .fold(Spec::new(self.kind()), |spec, p| spec.with(p.name, p.value))
    }

    pub fn from_spec(spec: &Spec) -> Result<Self, SpecError> {
        let mut filtre = match spec.kind.as_str() {
            "CRT" => FilterKind::Crt.build(),
            autre => return Err(SpecError::UnknownKind(autre.to_string())),
        };
        for (i, desc) in filtre.params().iter().enumerate() {
            let valeur = spec.param(desc.name)?;
            if let Some(param) = filtre.param_mut(i) {
                *param = valeur;
            }
        }
        Ok(filtre)
    }

    /// Ses réglages pour cette frame, résolus et saturés (§18.4).
    pub fn step(&self, g: &Graph, dt: f64) -> FilterStep {
        match self {
            ImageFilter::Crt(p) => FilterStep::Crt(p.step(g, dt)),
        }
    }
}

/// Un effet d'image, résolu : ce que le rendu a besoin de savoir.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum FilterStep {
    Crt(CrtStep),
}

/// Les réglages du CRT. Une intensité nulle coupe son composant.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CrtParams {
    /// Le nombre de lignes de balayage sur la hauteur.
    pub lines: Param,
    /// La force des lignes, `[0, 1]`.
    pub scanlines: Param,
    /// La force du masque de phosphores, `[0, 1]`.
    pub mask: Param,
    pub curvature: Param,
    pub vignette: Param,
    pub halo: Param,
    pub aberration: Param,
    pub noise: Param,
    pub flicker: Param,
    pub band: Param,
    /// Secondes pour perdre la moitié de l'image ; `0` : aucune rémanence.
    pub persistence: Param,
}

impl Default for CrtParams {
    fn default() -> Self {
        let p = Param::fixed;
        Self {
            lines: p(240.0),
            scanlines: p(0.5),
            mask: p(0.3),
            curvature: p(0.15),
            vignette: p(0.3),
            halo: p(0.3),
            aberration: p(0.2),
            noise: p(0.05),
            flicker: p(0.03),
            band: p(0.0),
            persistence: p(0.0),
        }
    }
}

/// Le CRT d'une frame, résolu.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CrtStep {
    pub lines: f32,
    pub scanlines: f32,
    pub mask: f32,
    pub curvature: f32,
    pub vignette: f32,
    pub halo: f32,
    pub aberration: f32,
    pub noise: f32,
    pub flicker: f32,
    pub band: f32,
    /// La part de l'image d'avant gardée cette frame, `[0, 1)`.
    pub keep: f32,
}

impl CrtParams {
    pub fn params(&self) -> Vec<ParamDesc> {
        let desc = |name, min, max, value| ParamDesc {
            name,
            min,
            max,
            value,
        };
        vec![
            desc("lignes", 60.0, 1080.0, self.lines),
            desc("balayage", 0.0, 1.0, self.scanlines),
            desc("masque", 0.0, 1.0, self.mask),
            desc("courbure", 0.0, 1.0, self.curvature),
            desc("vignette", 0.0, 1.0, self.vignette),
            desc("halo", 0.0, 2.0, self.halo),
            desc("aberration", 0.0, 1.0, self.aberration),
            desc("bruit", 0.0, 1.0, self.noise),
            desc("scintillement", 0.0, 1.0, self.flicker),
            desc("bande", 0.0, 1.0, self.band),
            desc("rémanence", 0.0, 4.0, self.persistence),
        ]
    }

    pub fn param_mut(&mut self, index: usize) -> Option<&mut Param> {
        Some(match index {
            0 => &mut self.lines,
            1 => &mut self.scanlines,
            2 => &mut self.mask,
            3 => &mut self.curvature,
            4 => &mut self.vignette,
            5 => &mut self.halo,
            6 => &mut self.aberration,
            7 => &mut self.noise,
            8 => &mut self.flicker,
            9 => &mut self.band,
            10 => &mut self.persistence,
            _ => return None,
        })
    }

    /// Saturé ici, chez le consommateur (§18.4). `dt` règle la rémanence par
    /// seconde, jamais par frame.
    pub fn step(&self, g: &Graph, dt: f64) -> CrtStep {
        let lu = |p: Param| {
            let v = p.get(g);
            if v.is_finite() {
                v
            } else {
                0.0
            }
        };
        let unite = |p: Param| lu(p).clamp(0.0, 1.0);
        CrtStep {
            lines: lu(self.lines).clamp(MIN_LINES, MAX_LINES),
            scanlines: unite(self.scanlines),
            mask: unite(self.mask),
            curvature: unite(self.curvature),
            vignette: unite(self.vignette),
            halo: lu(self.halo).clamp(0.0, 4.0),
            aberration: unite(self.aberration),
            noise: unite(self.noise),
            flicker: unite(self.flicker),
            band: unite(self.band),
            keep: persistence_keep(dt, lu(self.persistence)),
        }
    }
}

/// La part de l'image d'avant gardée après `dt` secondes, pour une demi-vie donnée.
pub fn persistence_keep(dt: f64, half_life: f32) -> f32 {
    if half_life <= 0.0 || dt <= 0.0 {
        return 0.0;
    }
    0.5f32.powf(dt as f32 / half_life.min(MAX_PERSISTENCE))
}

/// Où lire l'image pour le point `uv` d'un écran bombé, `[0, 1]²`. Les bords
/// vont chercher plus loin que l'image : ils tombent hors de l'écran.
pub fn barrel(uv: [f32; 2], curvature: f32) -> [f32; 2] {
    let c = [uv[0] * 2.0 - 1.0, uv[1] * 2.0 - 1.0];
    let r2 = c[0] * c[0] + c[1] * c[1];
    let f = 1.0 + curvature * 0.25 * r2;
    [c[0] * f * 0.5 + 0.5, c[1] * f * 0.5 + 0.5]
}

/// Ce que laisse passer une ligne de balayage à la hauteur `y`, `[0, 1]`.
pub fn scanline(y: f32, lines: f32, strength: f32) -> f32 {
    let creux = 0.5 - 0.5 * (std::f32::consts::TAU * y * lines).cos();
    1.0 - strength * creux
}

/// Le poids de chaque couleur pour la colonne de pixels `x` : une triade de
/// phosphores, rouge, vert, bleu. À force nulle, rien ne change.
pub fn mask_weights(x: u32, strength: f32) -> [f32; 3] {
    let triade = match x % 3 {
        0 => [1.0, 0.25, 0.25],
        1 => [0.25, 1.0, 0.25],
        _ => [0.25, 0.25, 1.0],
    };
    triade.map(|w| 1.0 + strength * (w - 1.0))
}

/// L'assombrissement des coins, au point `uv`.
pub fn vignette(uv: [f32; 2], strength: f32) -> f32 {
    let c = [uv[0] * 2.0 - 1.0, uv[1] * 2.0 - 1.0];
    (1.0 - strength * 0.5 * (c[0] * c[0] + c[1] * c[1])).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chaque_reglage_se_retrouve_par_son_rang() {
        let mut p = CrtParams::default();
        let n = p.params().len();
        for i in 0..n {
            *p.param_mut(i).unwrap() = Param::fixed(i as f32);
        }
        assert_eq!(p.param_mut(n), None);
        let lus: Vec<f32> = p.params().iter().map(|d| d.value.base()).collect();
        assert_eq!(lus, (0..n).map(|i| i as f32).collect::<Vec<_>>());
    }

    #[test]
    fn la_description_rend_le_meme_effet() {
        let mut f = FilterKind::Crt.build();
        *f.param_mut(1).unwrap() = Param::fixed(0.9);
        assert_eq!(ImageFilter::from_spec(&f.spec()).unwrap(), f);
        assert_eq!(FilterKind::Crt.label(), "CRT");
    }

    #[test]
    fn un_type_inconnu_est_refuse() {
        assert!(ImageFilter::from_spec(&Spec::new("VHS")).is_err());
    }

    #[test]
    fn la_remanence_se_compte_en_secondes() {
        assert_eq!(persistence_keep(1.0 / 60.0, 0.0), 0.0, "coupée");
        let par_frame = persistence_keep(1.0 / 60.0, 0.5);
        assert!(
            (par_frame.powf(30.0) - 0.5).abs() < 1e-4,
            "la moitié en 0,5 s"
        );
        let a_30 = persistence_keep(1.0 / 30.0, 0.5);
        assert!(
            (a_30.powf(15.0) - 0.5).abs() < 1e-4,
            "quelle que soit la cadence"
        );
    }

    #[test]
    fn le_centre_d_un_ecran_bombe_ne_bouge_pas() {
        assert_eq!(barrel([0.5, 0.5], 1.0), [0.5, 0.5]);
        let plat = barrel([0.1, 0.9], 0.0);
        assert!((plat[0] - 0.1).abs() < 1e-6 && (plat[1] - 0.9).abs() < 1e-6);
    }

    #[test]
    fn les_coins_d_un_ecran_bombe_sortent_de_l_image() {
        let coin = barrel([0.0, 0.0], 0.5);
        assert!(coin[0] < 0.0 && coin[1] < 0.0, "{coin:?}");
    }

    #[test]
    fn une_ligne_assombrit_entre_deux_lignes_et_pas_sur_elles() {
        assert!((scanline(0.0, 240.0, 1.0) - 1.0).abs() < 1e-6);
        assert!(scanline(0.5 / 240.0, 240.0, 1.0).abs() < 1e-5);
        assert_eq!(scanline(0.3, 240.0, 0.0), 1.0, "à force nulle, rien");
    }

    #[test]
    fn le_masque_alterne_les_trois_phosphores() {
        assert_eq!(mask_weights(0, 1.0), [1.0, 0.25, 0.25]);
        assert_eq!(mask_weights(4, 1.0), [0.25, 1.0, 0.25]);
        assert_eq!(mask_weights(5, 1.0), [0.25, 0.25, 1.0]);
        assert_eq!(mask_weights(7, 0.0), [1.0; 3]);
    }

    #[test]
    fn la_vignette_epargne_le_centre() {
        assert_eq!(vignette([0.5, 0.5], 1.0), 1.0);
        assert!(vignette([0.0, 0.0], 1.0) < 0.1);
    }

    #[test]
    fn des_reglages_fous_restent_utilisables() {
        let mut p = CrtParams::default();
        let g = Graph::new();
        p.lines = Param::fixed(f32::NAN);
        p.scanlines = Param::fixed(7.0);
        p.persistence = Param::fixed(-2.0);
        let s = p.step(&g, 1.0 / 60.0);
        assert_eq!(s.lines, MIN_LINES);
        assert_eq!(s.scanlines, 1.0);
        assert_eq!(s.keep, 0.0);
    }
}
