//! Horloge — §4 de `docs/ARCHITECTURE.md`.
//!
//! Deux modes : `Realtime` pour le live, `Fixed` pour un rendu déterministe.
//! Aucun autre module ne lit l'horloge système ; le temps est toujours reçu
//! en paramètre. C'est la condition du déterminisme et des tests à valeur exacte.

/// Mode d'avancement du temps.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ClockMode {
    /// Le temps suit l'horloge murale : `dt` est fourni par la boucle de rendu.
    Realtime,
    /// Le temps avance d'un pas fixe. Aucune frame perdue, résultat reproductible.
    Fixed { fps: f64 },
}

/// Horloge de la session.
///
/// `t` est en **secondes**, en `f64` : il accumule, et un `f32` perdrait
/// la précision après ~1 h de session (§17).
#[derive(Debug, Clone, Copy)]
pub struct Clock {
    mode: ClockMode,
    t: f64,
    dt: f64,
    frame: u64,
}

impl Clock {
    pub fn new(mode: ClockMode) -> Self {
        let dt = match mode {
            ClockMode::Fixed { fps } if fps > 0.0 => 1.0 / fps,
            _ => 0.0,
        };
        Self {
            mode,
            t: 0.0,
            dt,
            frame: 0,
        }
    }

    pub fn realtime() -> Self {
        Self::new(ClockMode::Realtime)
    }

    pub fn fixed(fps: f64) -> Self {
        Self::new(ClockMode::Fixed { fps })
    }

    /// Avance d'une frame.
    ///
    /// En mode `Fixed`, `wall_dt` est **ignoré** — c'est précisément ce qui rend
    /// le rendu reproductible. En `Realtime`, il donne la durée écoulée.
    pub fn tick(&mut self, wall_dt: f64) {
        self.dt = match self.mode {
            ClockMode::Fixed { fps } if fps > 0.0 => 1.0 / fps,
            ClockMode::Fixed { .. } => 0.0,
            // Un `wall_dt` négatif ou non fini viendrait d'une horloge système
            // qui a reculé (NTP, veille). On ne recule jamais le temps.
            ClockMode::Realtime => {
                if wall_dt.is_finite() && wall_dt > 0.0 {
                    wall_dt
                } else {
                    0.0
                }
            }
        };
        self.t += self.dt;
        self.frame += 1;
    }

    pub fn t(&self) -> f64 {
        self.t
    }

    pub fn dt(&self) -> f64 {
        self.dt
    }

    pub fn frame(&self) -> u64 {
        self.frame
    }

    pub fn mode(&self) -> ClockMode {
        self.mode
    }

    /// Remet le temps à zéro sans changer de mode.
    pub fn reset(&mut self) {
        self.t = 0.0;
        self.frame = 0;
        self.dt = match self.mode {
            ClockMode::Fixed { fps } if fps > 0.0 => 1.0 / fps,
            _ => 0.0,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_avance_par_pas_constant() {
        let mut c = Clock::fixed(60.0);
        assert_eq!(c.frame(), 0);
        for i in 1..=60 {
            c.tick(0.0);
            assert_eq!(c.frame(), i);
        }
        // 60 frames à 60 fps == 1 seconde, à la précision f64 près.
        assert!((c.t() - 1.0).abs() < 1e-9, "t: got {}", c.t());
    }

    #[test]
    fn fixed_ignore_le_temps_mural() {
        // Le cœur du déterminisme : deux `wall_dt` différents, même résultat.
        let mut a = Clock::fixed(30.0);
        let mut b = Clock::fixed(30.0);
        for _ in 0..10 {
            a.tick(0.001);
            b.tick(0.900);
        }
        assert_eq!(a.t(), b.t(), "Fixed doit ignorer wall_dt");
        assert!((a.t() - 10.0 / 30.0).abs() < 1e-9);
    }

    #[test]
    fn realtime_suit_le_temps_mural() {
        let mut c = Clock::realtime();
        c.tick(0.016);
        c.tick(0.020);
        assert!((c.t() - 0.036).abs() < 1e-9, "t: got {}", c.t());
        assert!((c.dt() - 0.020).abs() < 1e-9, "dt: got {}", c.dt());
    }

    #[test]
    fn realtime_ne_recule_jamais() {
        // Horloge système qui recule (NTP, sortie de veille) : on ignore.
        let mut c = Clock::realtime();
        c.tick(0.016);
        let t_avant = c.t();
        c.tick(-1.0);
        c.tick(f64::NAN);
        assert_eq!(c.t(), t_avant, "le temps ne doit jamais reculer");
        assert!(c.t().is_finite());
    }

    #[test]
    fn fps_nul_ne_divise_pas_par_zero() {
        let mut c = Clock::fixed(0.0);
        c.tick(0.016);
        assert!(c.t().is_finite(), "t: got {}", c.t());
        assert_eq!(c.dt(), 0.0);
    }

    #[test]
    fn reset_remet_a_zero_sans_changer_de_mode() {
        let mut c = Clock::fixed(60.0);
        c.tick(0.0);
        c.tick(0.0);
        c.reset();
        assert_eq!(c.t(), 0.0);
        assert_eq!(c.frame(), 0);
        assert_eq!(c.mode(), ClockMode::Fixed { fps: 60.0 });
    }
}
