//! Le spectre d'une frame — §7 de `docs/ARCHITECTURE.md`.
//!
//! Des **bandes logarithmiques** de taille fixe, et non les cases brutes de la
//! FFT : l'oreille entend en octaves, et une taille fixe voyage dans le triple
//! buffer et dans `Ctx` sans jamais allouer. `core` ne sait pas d'où viennent
//! ces valeurs : un test les écrit à la main, et le rendu reste déterministe.

/// Nombre de bandes, de [`SPECTRUM_MIN_HZ`] à [`SPECTRUM_MAX_HZ`].
pub const SPECTRUM_BANDS: usize = 128;
pub const SPECTRUM_MIN_HZ: f32 = 20.0;
pub const SPECTRUM_MAX_HZ: f32 = 20_000.0;

/// Le niveau de chaque bande, unipolaire `[0, 1]` (§17).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spectrum {
    pub levels: [f32; SPECTRUM_BANDS],
}

impl Default for Spectrum {
    fn default() -> Self {
        Self {
            levels: [0.0; SPECTRUM_BANDS],
        }
    }
}

impl Spectrum {
    /// La place d'une fréquence sur l'axe logarithmique, dans `[0, 1]`. Hors
    /// de l'axe, elle rejoint le bord le plus proche ; `NaN` rejoint le bas
    /// (§18.4).
    pub fn position(hz: f32) -> f32 {
        if hz.is_nan() {
            return 0.0;
        }
        let etendue = (SPECTRUM_MAX_HZ / SPECTRUM_MIN_HZ).ln();
        ((hz.max(SPECTRUM_MIN_HZ) / SPECTRUM_MIN_HZ).ln() / etendue).clamp(0.0, 1.0)
    }

    /// La fréquence à la place `position` de l'axe : l'inverse de
    /// [`Spectrum::position`].
    pub fn frequency(position: f32) -> f32 {
        SPECTRUM_MIN_HZ * (SPECTRUM_MAX_HZ / SPECTRUM_MIN_HZ).powf(position.clamp(0.0, 1.0))
    }

    /// Le niveau le plus haut entre deux fréquences, données dans n'importe
    /// quel ordre. Une plage plus étroite qu'une bande en lit une.
    pub fn peak(&self, a_hz: f32, b_hz: f32) -> f32 {
        let (a, b) = (Self::position(a_hz), Self::position(b_hz));
        let n = SPECTRUM_BANDS as f32;
        let debut = (a.min(b) * n).floor() as usize;
        let fin = ((a.max(b) * n).ceil() as usize).min(SPECTRUM_BANDS);
        let debut = debut.min(SPECTRUM_BANDS - 1);
        self.levels[debut..fin.max(debut + 1)]
            .iter()
            .copied()
            .fold(0.0, f32::max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_position_et_la_frequence_sont_inverses() {
        for hz in [20.0, 60.0, 440.0, 1000.0, 12_000.0, 20_000.0] {
            let retour = Spectrum::frequency(Spectrum::position(hz));
            assert!((retour - hz).abs() / hz < 1e-3, "{hz} → {retour}");
        }
    }

    #[test]
    fn l_axe_est_logarithmique() {
        // Une décade occupe la même largeur où qu'elle soit.
        let basse = Spectrum::position(200.0) - Spectrum::position(20.0);
        let haute = Spectrum::position(2000.0) - Spectrum::position(200.0);
        assert!((basse - haute).abs() < 1e-5, "{basse} et {haute}");
    }

    #[test]
    fn une_frequence_hors_axe_rejoint_le_bord() {
        assert_eq!(Spectrum::position(1.0), 0.0);
        assert_eq!(Spectrum::position(-5.0), 0.0);
        assert_eq!(Spectrum::position(f32::NAN), 0.0);
        assert_eq!(Spectrum::position(96_000.0), 1.0);
    }

    fn un_seul_pic(hz: f32, niveau: f32) -> Spectrum {
        let mut s = Spectrum::default();
        let bande = (Spectrum::position(hz) * SPECTRUM_BANDS as f32) as usize;
        s.levels[bande.min(SPECTRUM_BANDS - 1)] = niveau;
        s
    }

    #[test]
    fn le_pic_ne_voit_que_sa_plage() {
        let s = un_seul_pic(1000.0, 0.8);
        assert_eq!(s.peak(500.0, 2000.0), 0.8);
        assert_eq!(s.peak(2000.0, 500.0), 0.8, "bornes inversées");
        assert_eq!(s.peak(20.0, 100.0), 0.0);
        assert_eq!(s.peak(5000.0, 20_000.0), 0.0);
    }

    #[test]
    fn une_plage_nulle_lit_une_bande_sans_paniquer() {
        let s = un_seul_pic(1000.0, 0.8);
        assert_eq!(s.peak(1000.0, 1000.0), 0.8);
        // Les deux bords de l'axe, où un découpage naïf sort du tableau.
        assert_eq!(un_seul_pic(20_000.0, 0.5).peak(20_000.0, 20_000.0), 0.5);
        assert_eq!(un_seul_pic(20.0, 0.5).peak(20.0, 20.0), 0.5);
        assert_eq!(s.peak(f32::NAN, f32::INFINITY), 0.8, "toute la plage");
    }
}
