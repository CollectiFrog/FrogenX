//! Des échantillons au spectre — §7 de `docs/ARCHITECTURE.md`.
//!
//! Pur : aucun thread, aucune carte son. Un test y pousse une sinusoïde de
//! fréquence connue et lit la bande attendue.

use frogenx_core::{Spectrum, SPECTRUM_BANDS};
use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};
use std::f32::consts::TAU;
use std::sync::Arc;

/// Taille de la FFT : 43 ms à 48 kHz, soit des cases de 23 Hz. Le double
/// séparerait mieux les graves, mais doublerait le retard d'un déclenchement.
pub const FFT_SIZE: usize = 2048;
/// Échantillons entre deux analyses : 94 spectres par seconde à 48 kHz, plus
/// qu'il n'y a de frames.
pub const HOP: usize = 512;
/// Le niveau `0` du spectre, en dB. Le niveau `1` est une sinusoïde à pleine
/// échelle.
pub const FLOOR_DB: f32 = -72.0;
/// Constante de temps de la descente, en secondes. La montée est immédiate :
/// lisser une attaque retarderait le trig.
const RELEASE: f32 = 0.06;

/// Le spectre glissant d'un flux mono.
pub struct SpectrumAnalyzer {
    fft: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    /// Les `FFT_SIZE` derniers échantillons, en anneau.
    samples: Vec<f32>,
    write: usize,
    since_analysis: usize,
    buffer: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    magnitudes: Vec<f32>,
    /// Les bords de chaque bande, en cases de FFT (fractionnaires).
    bands: Vec<(f32, f32)>,
    /// Une sinusoïde à pleine échelle mesure 1.
    gain: f32,
    /// Part de l'écart parcourue à chaque analyse pendant une descente.
    release: f32,
    spectrum: Spectrum,
}

impl SpectrumAnalyzer {
    pub fn new(sample_rate: f32) -> Self {
        let fft = FftPlanner::new().plan_fft_forward(FFT_SIZE);
        let scratch = vec![Complex::default(); fft.get_inplace_scratch_len()];
        // Hann périodique : la fenêtre usuelle pour une FFT glissante.
        let window: Vec<f32> = (0..FFT_SIZE)
            .map(|i| 0.5 - 0.5 * (TAU * i as f32 / FFT_SIZE as f32).cos())
            .collect();
        let par_case = sample_rate / FFT_SIZE as f32;
        let bands = (0..SPECTRUM_BANDS)
            .map(|i| {
                let bord = |b: usize| Spectrum::frequency(b as f32 / SPECTRUM_BANDS as f32);
                (bord(i) / par_case, bord(i + 1) / par_case)
            })
            .collect();
        Self {
            gain: 2.0 / window.iter().sum::<f32>(),
            release: 1.0 - (-(HOP as f32 / sample_rate) / RELEASE).exp(),
            fft,
            window,
            samples: vec![0.0; FFT_SIZE],
            write: 0,
            since_analysis: 0,
            buffer: vec![Complex::default(); FFT_SIZE],
            scratch,
            magnitudes: vec![0.0; FFT_SIZE / 2 + 1],
            bands,
            spectrum: Spectrum::default(),
        }
    }

    /// Ajoute un échantillon. Rend `true` si le spectre vient de changer.
    pub fn push(&mut self, sample: f32) -> bool {
        // Un échantillon non fini ne doit pas empoisonner toute une fenêtre.
        self.samples[self.write] = if sample.is_finite() { sample } else { 0.0 };
        self.write = (self.write + 1) % FFT_SIZE;
        self.since_analysis += 1;
        if self.since_analysis < HOP {
            return false;
        }
        self.since_analysis = 0;
        self.analyze();
        true
    }

    pub fn spectrum(&self) -> &Spectrum {
        &self.spectrum
    }

    fn analyze(&mut self) {
        // Le continu d'un micro fuirait dans les premières bandes, celles où
        // l'on pose un trig de grosse caisse : il tiendrait la zone ouverte.
        let moyenne = self.samples.iter().sum::<f32>() / FFT_SIZE as f32;
        for (i, case) in self.buffer.iter_mut().enumerate() {
            let s = self.samples[(self.write + i) % FFT_SIZE];
            *case = Complex::new((s - moyenne) * self.window[i], 0.0);
        }
        self.fft
            .process_with_scratch(&mut self.buffer, &mut self.scratch);
        for (m, case) in self.magnitudes.iter_mut().zip(&self.buffer) {
            *m = case.norm() * self.gain;
        }

        for (i, (bas, haut)) in self.bands.iter().enumerate() {
            let cible = level(band_magnitude(&self.magnitudes, *bas, *haut));
            let courant = &mut self.spectrum.levels[i];
            *courant = if cible > *courant {
                cible
            } else {
                *courant + (cible - *courant) * self.release
            };
        }
    }
}

/// La magnitude d'une bande : sa case la plus haute. Une bande plus étroite
/// qu'une case, dans les graves, s'interpole entre ses deux voisines.
fn band_magnitude(magnitudes: &[f32], bas: f32, haut: f32) -> f32 {
    let derniere = magnitudes.len() - 1;
    if bas >= derniere as f32 {
        return 0.0; // au-dessus de Nyquist : rien à lire
    }
    if haut - bas < 1.0 {
        let centre = (bas + haut) * 0.5;
        let i = centre.floor() as usize;
        let suivant = magnitudes[(i + 1).min(derniere)];
        return magnitudes[i] + (suivant - magnitudes[i]) * centre.fract();
    }
    let fin = (haut.floor() as usize).min(derniere);
    magnitudes[bas.ceil() as usize..=fin]
        .iter()
        .copied()
        .fold(0.0, f32::max)
}

/// Une magnitude en niveau `[0, 1]`, linéaire en dB : l'échelle de l'oreille,
/// et celle dans laquelle on dessine une zone.
fn level(magnitude: f32) -> f32 {
    let db = 20.0 * magnitude.max(1e-9).log10();
    (1.0 - db / FLOOR_DB).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 48_000.0;

    fn sinus(hz: f32, amplitude: f32, n: usize, sr: f32) -> impl Iterator<Item = f32> {
        (0..n).map(move |i| amplitude * (TAU * hz * i as f32 / sr).sin())
    }

    fn analyse(samples: impl Iterator<Item = f32>, sr: f32) -> Spectrum {
        let mut a = SpectrumAnalyzer::new(sr);
        for s in samples {
            a.push(s);
        }
        *a.spectrum()
    }

    fn bande_du_pic(s: &Spectrum) -> f32 {
        let i = (0..SPECTRUM_BANDS)
            .max_by(|a, b| s.levels[*a].total_cmp(&s.levels[*b]))
            .unwrap();
        Spectrum::frequency((i as f32 + 0.5) / SPECTRUM_BANDS as f32)
    }

    #[test]
    fn une_sinusoide_tombe_dans_sa_bande() {
        for hz in [100.0, 440.0, 1000.0, 8000.0] {
            let s = analyse(sinus(hz, 0.5, 8192, SR), SR);
            let pic = bande_du_pic(&s);
            assert!((pic / hz).log2().abs() < 0.2, "{hz} Hz lu à {pic} Hz");
        }
    }

    #[test]
    fn une_sinusoide_ne_touche_pas_les_bandes_lointaines() {
        let s = analyse(sinus(100.0, 0.5, 8192, SR), SR);
        assert!(s.peak(90.0, 110.0) > 0.8);
        assert!(
            s.peak(2000.0, 20_000.0) < 0.2,
            "got {}",
            s.peak(2000.0, 20_000.0)
        );
    }

    #[test]
    fn le_niveau_suit_l_amplitude_en_decibels() {
        // Pleine échelle : 1. Moitié (-6 dB) : 1 - 6 / 72.
        let plein = analyse(sinus(1000.0, 1.0, 8192, SR), SR).peak(900.0, 1100.0);
        let moitie = analyse(sinus(1000.0, 0.5, 8192, SR), SR).peak(900.0, 1100.0);
        assert!((plein - 1.0).abs() < 0.02, "got {plein}");
        assert!((moitie - (1.0 - 6.02 / 72.0)).abs() < 0.02, "got {moitie}");
    }

    #[test]
    fn le_silence_donne_un_spectre_nul() {
        let s = analyse(std::iter::repeat_n(0.0, 8192), SR);
        assert_eq!(s, Spectrum::default());
    }

    #[test]
    fn le_continu_ne_monte_pas_dans_les_graves() {
        // Un micro décalé de 0.2 : la zone d'une grosse caisse reste fermée.
        // Une seconde entière : le choc du premier échantillon doit retomber.
        let s = analyse(std::iter::repeat_n(0.2, 48_000), SR);
        assert!(s.peak(20.0, 200.0) < 0.05, "got {}", s.peak(20.0, 200.0));
    }

    #[test]
    fn un_echantillon_non_fini_n_empoisonne_pas_le_spectre() {
        let avec_nan =
            sinus(1000.0, 0.5, 8192, SR)
                .enumerate()
                .map(|(i, s)| if i == 6000 { f32::NAN } else { s });
        let s = analyse(avec_nan, SR);
        assert!(s.levels.iter().all(|l| l.is_finite()));
    }

    #[test]
    fn l_attaque_est_immediate_et_la_descente_progressive() {
        let mut a = SpectrumAnalyzer::new(SR);
        for s in sinus(1000.0, 0.5, FFT_SIZE, SR) {
            a.push(s);
        }
        let attaque = a.spectrum().peak(900.0, 1100.0);
        assert!(attaque > 0.85, "got {attaque}");

        // Une fenêtre de silence plus 20 ms : la fenêtre est vide, le niveau
        // n'est pas encore retombé.
        for _ in 0..FFT_SIZE + 960 {
            a.push(0.0);
        }
        let descente = a.spectrum().peak(900.0, 1100.0);
        assert!(descente > 0.05 && descente < attaque, "got {descente}");

        for _ in 0..48_000 {
            a.push(0.0);
        }
        assert!(a.spectrum().peak(900.0, 1100.0) < 1e-3);
    }

    #[test]
    fn une_analyse_tombe_tous_les_hop_echantillons() {
        let mut a = SpectrumAnalyzer::new(SR);
        let analyses = (0..HOP * 3).filter(|_| a.push(0.0)).count();
        assert_eq!(analyses, 3);
    }

    #[test]
    fn au_dessus_de_nyquist_les_bandes_restent_a_zero() {
        // Un micro à 16 kHz ne voit rien au-delà de 8 kHz.
        let sr = 16_000.0;
        let s = analyse(sinus(1000.0, 0.5, 8192, sr), sr);
        assert_eq!(s.peak(9000.0, 20_000.0), 0.0);
        assert!(s.peak(900.0, 1100.0) > 0.8);
    }

    #[test]
    fn un_niveau_reste_dans_zero_un() {
        assert_eq!(level(0.0), 0.0);
        assert_eq!(level(1.0), 1.0);
        assert_eq!(level(4.0), 1.0, "au-dessus de la pleine échelle");
    }
}
