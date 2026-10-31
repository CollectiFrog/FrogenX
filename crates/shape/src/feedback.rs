//! Les réglages d'un feedback — §6.1 de `docs/ARCHITECTURE.md`.
//!
//! Ici, et non dans `layer` : un clip les porte, et `composition` ne connaît pas
//! wgpu. **Tout s'exprime par seconde** : un réglage par frame changerait d'image
//! avec la cadence.

use crate::generator::ParamDesc;
use frogenx_core::{Graph, Param, Spec, SpecError};

/// Le zoom le plus fort et le plus faible, par seconde (§18.4).
const MAX_ZOOM: f32 = 16.0;
/// La mémoire la plus longue, en secondes : au-delà, une image s'accumulerait
/// sans fin (§18.4).
const MAX_MEMORY: f32 = 30.0;

/// Ce que devient l'image d'une frame à l'autre.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FeedbackParams {
    /// Facteur d'agrandissement par seconde. `1` = taille constante.
    pub zoom: Param,
    /// Radians par seconde.
    pub rotation: Param,
    /// Unités d'espace (§17) par seconde.
    pub drift_x: Param,
    pub drift_y: Param,
    /// Secondes pour perdre la moitié de l'image.
    pub memory: Param,
}

impl Default for FeedbackParams {
    fn default() -> Self {
        Self {
            zoom: Param::fixed(1.0),
            rotation: Param::fixed(0.0),
            drift_x: Param::fixed(0.0),
            drift_y: Param::fixed(0.0),
            memory: Param::fixed(1.0),
        }
    }
}

/// Le déplacement d'une frame, résolu : tout ce que le rendu a besoin de savoir.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FeedbackStep {
    pub scale: f32,
    pub angle: f32,
    pub offset: [f32; 2],
    /// La part de l'image gardée, `[0, 1]`.
    pub keep: f32,
}

impl FeedbackStep {
    /// Où lire, dans l'image d'avant, le point `p` de l'image nouvelle, en
    /// coordonnées du §17. Le shader fait le même calcul : cette version lui sert
    /// de référence dans les tests.
    pub fn source_of(&self, p: [f32; 2]) -> [f32; 2] {
        let (x, y) = (p[0] - self.offset[0], p[1] - self.offset[1]);
        let (sin, cos) = (-self.angle).sin_cos();
        [
            (x * cos - y * sin) / self.scale,
            (x * sin + y * cos) / self.scale,
        ]
    }
}

impl FeedbackParams {
    /// Le nom du groupe de réglages.
    pub fn kind(&self) -> &'static str {
        "Écho"
    }

    /// Ses paramètres réglables (§16.3 ter).
    pub fn params(&self) -> Vec<ParamDesc> {
        let desc = |name, min, max, value| ParamDesc {
            name,
            min,
            max,
            value,
        };
        vec![
            desc("zoom", 0.25, 4.0, self.zoom),
            desc("rotation", -3.0, 3.0, self.rotation),
            desc("dérive x", -2.0, 2.0, self.drift_x),
            desc("dérive y", -2.0, 2.0, self.drift_y),
            desc("mémoire", 0.05, 10.0, self.memory),
        ]
    }

    /// Le `index`-ième paramètre de [`Self::params`], en écriture.
    pub fn param_mut(&mut self, index: usize) -> Option<&mut Param> {
        match index {
            0 => Some(&mut self.zoom),
            1 => Some(&mut self.rotation),
            2 => Some(&mut self.drift_x),
            3 => Some(&mut self.drift_y),
            4 => Some(&mut self.memory),
            _ => None,
        }
    }

    /// Sa description pour le format de patch (§19.1).
    pub fn spec(&self) -> Spec {
        self.params()
            .into_iter()
            .fold(Spec::new(self.kind()), |spec, p| spec.with(p.name, p.value))
    }

    pub fn from_spec(spec: &Spec) -> Result<Self, SpecError> {
        let mut params = Self::default();
        for (i, desc) in params.params().iter().enumerate() {
            let valeur = spec.param(desc.name)?;
            if let Some(param) = params.param_mut(i) {
                *param = valeur;
            }
        }
        Ok(params)
    }

    /// Le déplacement d'une frame de durée `dt`. Saturé ici, chez le consommateur
    /// (§18.4) : une modulation trop profonde dégrade l'image, sans plus.
    pub fn step(&self, g: &Graph, dt: f64) -> FeedbackStep {
        let dt = dt as f32;
        let fini = |v: f32, repli: f32| if v.is_finite() { v } else { repli };
        let zoom = fini(self.zoom.get(g), 1.0).clamp(1.0 / MAX_ZOOM, MAX_ZOOM);
        let memoire = fini(self.memory.get(g), 1.0).min(MAX_MEMORY);
        FeedbackStep {
            scale: zoom.powf(dt),
            angle: fini(self.rotation.get(g), 0.0) * dt,
            offset: [
                fini(self.drift_x.get(g), 0.0) * dt,
                fini(self.drift_y.get(g), 0.0) * dt,
            ],
            keep: if memoire > 0.0 {
                0.5f32.powf(dt / memoire)
            } else {
                0.0
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use frogenx_core::Constant;

    const DT: f64 = 1.0 / 60.0;

    fn reglage() -> FeedbackParams {
        FeedbackParams {
            zoom: Param::fixed(2.0),
            rotation: Param::fixed(1.5),
            drift_x: Param::fixed(0.25),
            drift_y: Param::fixed(-0.25),
            memory: Param::fixed(0.5),
        }
    }

    fn proche(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn une_seconde_vaut_les_reglages_quelle_que_soit_la_cadence() {
        let g = Graph::new();
        for fps in [30, 60, 120, 144] {
            let s = reglage().step(&g, 1.0 / fps as f64);
            let n = fps as f32;
            assert!(proche(s.scale.powf(n), 2.0), "{fps}");
            assert!(proche(s.angle * n, 1.5), "{fps}");
            assert!(proche(s.keep.powf(n), 0.25), "{fps} : deux demi-vies");
            assert!(proche(s.offset[0] * n, 0.25), "{fps}");
        }
    }

    #[test]
    fn un_temps_nul_ne_change_rien() {
        let s = reglage().step(&Graph::new(), 0.0);
        assert_eq!((s.scale, s.angle, s.keep), (1.0, 0.0, 1.0));
        assert_eq!(s.offset, [0.0, 0.0]);
        assert_eq!(s.source_of([0.5, -0.25]), [0.5, -0.25]);
    }

    #[test]
    fn par_defaut_l_image_reste_en_place_et_s_efface() {
        let s = FeedbackParams::default().step(&Graph::new(), DT);
        assert_eq!(s.scale, 1.0);
        assert_eq!(s.source_of([0.5, 0.5]), [0.5, 0.5]);
        assert!(s.keep < 1.0);
    }

    #[test]
    fn agrandir_lit_plus_pres_du_centre() {
        let s = FeedbackStep {
            scale: 2.0,
            angle: 0.0,
            offset: [0.0, 0.0],
            keep: 1.0,
        };
        assert_eq!(s.source_of([0.5, -0.5]), [0.25, -0.25]);
    }

    #[test]
    fn tourner_d_un_quart_de_tour_amene_ce_qui_etait_a_droite_en_haut() {
        let s = FeedbackStep {
            scale: 1.0,
            angle: std::f32::consts::FRAC_PI_2,
            offset: [0.0, 0.0],
            keep: 1.0,
        };
        // Le point en haut lit ce qui était à droite.
        let source = s.source_of([0.0, 1.0]);
        assert!(
            proche(source[0], 1.0) && proche(source[1], 0.0),
            "{source:?}"
        );
    }

    #[test]
    fn deriver_vers_la_droite_lit_a_gauche() {
        let s = FeedbackStep {
            scale: 1.0,
            angle: 0.0,
            offset: [0.25, 0.0],
            keep: 1.0,
        };
        assert_eq!(s.source_of([0.0, 0.0]), [-0.25, 0.0]);
    }

    #[test]
    fn une_memoire_nulle_ne_garde_rien() {
        let p = FeedbackParams {
            memory: Param::fixed(0.0),
            ..Default::default()
        };
        assert_eq!(p.step(&Graph::new(), DT).keep, 0.0);
    }

    #[test]
    fn une_modulation_trop_profonde_reste_bornee() {
        let g = Graph::new();
        let p = FeedbackParams {
            zoom: Param::fixed(1e9),
            memory: Param::fixed(1e9),
            ..Default::default()
        };
        let s = p.step(&g, 1.0);
        assert_eq!(s.scale, MAX_ZOOM);
        assert!(s.keep < 1.0, "l'image finit toujours par s'effacer");

        let p = FeedbackParams {
            zoom: Param::fixed(-3.0),
            ..Default::default()
        };
        assert!(p.step(&g, DT).scale > 0.0, "jamais de division par zéro");
    }

    #[test]
    fn une_valeur_non_finie_retombe_sur_le_neutre() {
        let p = FeedbackParams {
            zoom: Param::fixed(f32::NAN),
            rotation: Param::fixed(f32::INFINITY),
            drift_x: Param::fixed(f32::NAN),
            drift_y: Param::fixed(f32::NAN),
            memory: Param::fixed(f32::NAN),
        };
        let s = p.step(&Graph::new(), DT);
        assert!(s.scale.is_finite() && s.angle.is_finite() && s.keep.is_finite());
    }

    #[test]
    fn une_modulation_se_lit_dans_le_graphe() {
        let mut g = Graph::new();
        let source = g.add(Box::new(Constant(1.0)));
        let p = FeedbackParams {
            rotation: Param::modulated(0.5, 0.5, source),
            ..Default::default()
        };
        g.eval_frame(0.0, DT);
        assert_eq!(p.step(&g, 1.0).angle, 1.0);
    }

    #[test]
    fn chaque_parametre_est_atteignable_et_se_relit_de_sa_description() {
        let mut p = FeedbackParams::default();
        let n = p.params().len();
        for i in 0..n {
            p.param_mut(i).unwrap().set_base(i as f32 + 2.0);
        }
        assert!(p.param_mut(n).is_none());
        assert_eq!(FeedbackParams::from_spec(&p.spec()), Ok(p));
    }

    #[test]
    fn une_description_incomplete_est_refusee() {
        assert!(FeedbackParams::from_spec(&Spec::new("Écho")).is_err());
    }
}
