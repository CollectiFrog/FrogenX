//! Le format et la cadence d'un flux — §8.5 de `docs/ARCHITECTURE.md`.
//!
//! **Ce module ne connaît pas wgpu.** Tout s'y teste sur des nombres.

/// La plus petite taille d'un flux, en pixels : un encodeur refuse en dessous.
const MIN_SIDE: u32 = 16;
/// La part de la période qu'une frame peut avoir d'avance : le `t` d'un moteur
/// temps réel tremble, et une frame refusée pour une milliseconde attendrait
/// la suivante.
const EARLY_SHARE: f64 = 0.25;

/// Ce qu'un flux demande. Se règle d'un geste, entre deux frames : ce n'est pas
/// un `Param` (§8.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamFormat {
    pub width: u32,
    pub height: u32,
    /// Images par seconde, bornées dans les faits par la cadence du moteur.
    pub fps: u32,
}

impl StreamFormat {
    /// Le format réel sous un rendu de cette taille : jamais d'agrandissement,
    /// et les proportions du flux restent les siennes. Les côtés sont pairs,
    /// comme l'exige un encodeur en 4:2:0.
    pub fn within(self, render: (u32, u32)) -> (u32, u32) {
        let (w, h) = (self.width.max(1) as f64, self.height.max(1) as f64);
        let echelle = (render.0 as f64 / w).min(render.1 as f64 / h).min(1.0);
        let cote = |c: f64| ((c * echelle).round() as u32 & !1).max(MIN_SIDE);
        (cote(w), cote(h))
    }
}

/// Décide quelles frames partent, d'après leur `t`.
///
/// Sur le `t` et non sur le numéro de frame : la cadence d'un flux ne dépend
/// pas de celle du moteur. L'échéance avance de sa période et non depuis la
/// frame reçue : à la longue, le débit reste exact, même sur un ratio qui ne
/// tombe pas juste.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cadence {
    period: f64,
    next: Option<f64>,
}

impl Cadence {
    pub fn new(fps: u32) -> Self {
        Self {
            period: 1.0 / fps.max(1) as f64,
            next: None,
        }
    }

    /// Cette frame part-elle ?
    pub fn due(&mut self, t: f64) -> bool {
        if !t.is_finite() {
            return false;
        }
        let Some(echeance) = self.next else {
            self.next = Some(t + self.period);
            return true;
        };
        if t < echeance - self.period * EARLY_SHARE {
            // Un temps qui recule (une horloge remise à zéro) ne doit pas
            // bloquer le flux jusqu'à rattraper l'ancienne échéance.
            if t < echeance - 2.0 * self.period {
                self.next = Some(t + self.period);
                return true;
            }
            return false;
        }
        // Après un trou, on repart d'ici au lieu d'envoyer tout le retard.
        let suite = echeance + self.period;
        self.next = Some(if suite <= t { t + self.period } else { suite });
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn format(width: u32, height: u32) -> StreamFormat {
        StreamFormat {
            width,
            height,
            fps: 60,
        }
    }

    /// Le nombre de frames parties en 60 s sur un moteur à `moteur` fps.
    fn parties(fps: u32, moteur: u32, tremble: f64) -> u32 {
        let mut c = Cadence::new(fps);
        (0..moteur * 60)
            .filter(|i| {
                // Un tremblement déterministe, sans dérive.
                let ecart = if i % 2 == 0 { tremble } else { -tremble };
                c.due(*i as f64 / moteur as f64 + ecart)
            })
            .count() as u32
    }

    #[test]
    fn le_debit_reste_exact_a_chaque_ratio() {
        for (fps, moteur) in [(60, 60), (30, 60), (20, 60), (15, 60), (25, 50), (24, 60)] {
            let n = parties(fps, moteur, 0.0);
            assert!(n.abs_diff(fps * 60) <= 1, "{fps} sur {moteur} : {n}");
        }
    }

    #[test]
    fn un_ratio_qui_ne_tombe_pas_juste_tient_quand_meme_son_debit() {
        let n = parties(50, 60, 0.0);
        assert!(n.abs_diff(3000) <= 1, "{n}");
    }

    #[test]
    fn un_temps_qui_tremble_ne_change_pas_le_debit() {
        for fps in [60, 30, 20] {
            let n = parties(fps, 60, 0.002);
            assert!(n.abs_diff(fps * 60) <= 2, "{fps} : {n}");
        }
    }

    #[test]
    fn la_cadence_demandee_plafonne_a_celle_du_moteur() {
        assert_eq!(parties(120, 60, 0.0), 3600);
    }

    #[test]
    fn la_premiere_frame_part_toujours() {
        assert!(Cadence::new(1).due(12.5));
    }

    #[test]
    fn un_trou_ne_fait_pas_rattraper_le_retard() {
        let mut c = Cadence::new(30);
        assert!(c.due(0.0));
        assert!(c.due(10.0));
        assert!(!c.due(10.0 + 1.0 / 60.0), "pas de rafale après le trou");
        assert!(c.due(10.0 + 1.0 / 30.0));
    }

    #[test]
    fn un_temps_remis_a_zero_ne_bloque_pas_le_flux() {
        let mut c = Cadence::new(30);
        assert!(c.due(100.0));
        assert!(c.due(0.0));
        assert!(c.due(1.0 / 30.0));
    }

    #[test]
    fn un_temps_non_fini_ne_part_pas_et_ne_casse_rien() {
        let mut c = Cadence::new(30);
        assert!(!c.due(f64::NAN));
        assert!(!c.due(f64::INFINITY));
        assert!(c.due(0.0));
    }

    #[test]
    fn une_cadence_nulle_reste_definie() {
        let mut c = Cadence::new(0);
        assert!(c.due(0.0));
        assert!(!c.due(0.5));
        assert!(c.due(1.0));
    }

    #[test]
    fn un_flux_plus_petit_que_le_rendu_garde_sa_taille() {
        assert_eq!(format(1280, 720).within((1920, 1080)), (1280, 720));
    }

    #[test]
    fn un_flux_plus_grand_que_le_rendu_n_est_jamais_agrandi() {
        assert_eq!(format(3840, 2160).within((1920, 1080)), (1920, 1080));
        assert_eq!(format(1920, 1080).within((1280, 720)), (1280, 720));
    }

    #[test]
    fn un_flux_reduit_garde_ses_proportions() {
        assert_eq!(format(1920, 1200).within((1280, 720)), (1152, 720));
    }

    #[test]
    fn les_cotes_sont_toujours_pairs() {
        let (w, h) = format(1279, 719).within((1920, 1080));
        assert_eq!((w % 2, h % 2), (0, 0));
    }

    #[test]
    fn un_format_absurde_reste_defini() {
        let (w, h) = format(0, 0).within((1920, 1080));
        assert!(w >= MIN_SIDE && h >= MIN_SIDE);
        let (w, h) = format(1920, 1080).within((0, 0));
        assert!(w >= MIN_SIDE && h >= MIN_SIDE);
    }
}
