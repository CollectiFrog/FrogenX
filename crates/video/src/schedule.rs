//! Quelle source pour l'instant `t` — §9.3 de `docs/ARCHITECTURE.md`.
//!
//! Une fonction pure : tout le comportement de lecture se teste ici sur des
//! nombres, sans ffmpeg.

/// Ce que le tampon de tête garde, en secondes : assez pour couvrir l'ouverture
/// d'un décodeur (environ 150 ms), avec de la marge.
pub const HEAD_SECONDS: f64 = 0.5;
/// Jusqu'où devant lui le flux peut rattraper en sautant des images, en secondes.
/// Au-delà, le relancer à la bonne place est plus court.
const MAX_AHEAD: f64 = 1.0;

/// Où en est le flux.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StreamView {
    /// L'instant de la prochaine image qu'il va rendre.
    pub next: f64,
}

/// La décision pour une frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Plan {
    /// L'image `n` du tampon. Le flux doit se préparer à prendre le relais.
    Head(usize),
    /// La suite du flux.
    Stream,
    /// Le flux n'y mène pas : il faut le relancer au début de cette image. Pas à
    /// `t` : au milieu de la dernière image, ffmpeg n'aurait plus rien à rendre.
    Seek(f64),
}

/// Décide d'où vient l'image de l'instant `t`.
pub fn plan(t: f64, fps: f32, head: usize, stream: Option<StreamView>) -> Plan {
    let t = t.max(0.0);
    let periode = 1.0 / fps.max(1.0) as f64;
    // 7 / 30 divisé par 1 / 30 vaut 6,9999… : sans cette marge, l'image d'avant.
    let image = (t / periode + 1e-6).floor() as usize;
    if image < head {
        return Plan::Head(image);
    }
    match stream {
        // Une image d'écart en arrière est la gigue du temps, pas un retour.
        Some(s) if t >= s.next - 2.0 * periode && t <= s.next + MAX_AHEAD => Plan::Stream,
        _ => Plan::Seek(image as f64 * periode),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FPS: f32 = 30.0;
    const HEAD: usize = 15;

    fn flux(next: f64) -> Option<StreamView> {
        Some(StreamView { next })
    }

    #[test]
    fn le_debut_vient_du_tampon_sans_attendre_le_flux() {
        assert_eq!(plan(0.0, FPS, HEAD, None), Plan::Head(0));
        assert_eq!(plan(0.1, FPS, HEAD, None), Plan::Head(3));
    }

    #[test]
    fn un_instant_exact_tombe_sur_son_image() {
        for n in 0..HEAD {
            let t = n as f64 / FPS as f64;
            assert_eq!(plan(t, FPS, HEAD, None), Plan::Head(n), "{t}");
        }
    }

    #[test]
    fn la_suite_vient_du_flux() {
        assert_eq!(plan(0.6, FPS, HEAD, flux(0.58)), Plan::Stream);
    }

    #[test]
    fn apres_le_tampon_sans_flux_il_faut_le_lancer() {
        assert!(matches!(plan(0.7, FPS, HEAD, None), Plan::Seek(s) if (s - 0.7).abs() < 1e-6));
    }

    #[test]
    fn sans_tampon_tout_vient_du_flux() {
        assert_eq!(plan(0.0, FPS, 0, None), Plan::Seek(0.0));
        assert_eq!(plan(0.0, FPS, 0, flux(0.0)), Plan::Stream);
    }

    #[test]
    fn une_petite_avance_se_rattrape_en_sautant_des_images() {
        assert_eq!(plan(1.4, FPS, HEAD, flux(1.0)), Plan::Stream);
    }

    #[test]
    fn un_saut_devant_relance_le_flux() {
        assert!(matches!(plan(5.0, FPS, HEAD, flux(1.0)), Plan::Seek(s) if (s - 5.0).abs() < 1e-6));
    }

    #[test]
    fn un_retour_en_arriere_relance_le_flux() {
        assert!(matches!(plan(2.0, FPS, HEAD, flux(3.0)), Plan::Seek(s) if (s - 2.0).abs() < 1e-6));
    }

    #[test]
    fn un_saut_part_du_debut_de_son_image() {
        let Plan::Seek(s) = plan(3.95, 15.0, HEAD, None) else {
            panic!("un saut attendu");
        };
        assert!((s - 59.0 / 15.0).abs() < 1e-6, "{s}");
    }

    #[test]
    fn la_gigue_du_temps_ne_relance_rien() {
        let periode = 1.0 / FPS as f64;
        assert_eq!(plan(2.0 - periode, FPS, HEAD, flux(2.0)), Plan::Stream);
    }

    #[test]
    fn un_temps_negatif_vaut_le_debut() {
        assert_eq!(plan(-1.0, FPS, HEAD, None), Plan::Head(0));
    }

    #[test]
    fn une_cadence_absurde_ne_divise_pas_par_zero() {
        assert_eq!(plan(20.0, 0.0, HEAD, None), Plan::Seek(20.0));
    }
}
