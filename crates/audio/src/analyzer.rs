//! Des échantillons au spectre — §7 de `docs/ARCHITECTURE.md`.
//!
//! Pur : aucun thread, aucune carte son. Un test y pousse une sinusoïde de
//! fréquence connue et lit la bande attendue.
//!
//! **Trois méthodes, un même découpage.** Les bandes viennent de la `FreqScale`
//! choisie ; seule change la façon de mesurer ce que contient chacune :
//!
//! | Méthode | Mesure | Coût par analyse |
//! |---|---|---|
//! | `Peak` | la case de FFT la plus haute | une FFT |
//! | `Energy` | l'énergie de toutes les cases | une FFT |
//! | `ConstantQ` | une fenêtre par bande, plus courte dans les bandes larges | une somme par bande |

use frogenx_core::{
    sanitize_window, AnalysisMethod, AnalysisSettings, Spectrum, MAX_WINDOW, SPECTRUM_BANDS,
};
use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};
use std::f32::consts::TAU;
use std::sync::Arc;

/// La plus petite FFT. Elle fait toujours au moins deux fois la fenêtre,
/// complétée par des zéros : une sinusoïde tombée entre deux cases y perd
/// 0.4 dB au lieu de 1.4, et la courbe est plus lisse.
const MIN_FFT_SIZE: usize = 2048;
/// Échantillons entre deux analyses, au plus pour une fenêtre usuelle : 94
/// spectres par seconde à 48 kHz, plus qu'il n'y a de frames.
const MAX_HOP: usize = 512;
/// Une fenêtre très longue n'a pas besoin d'être relue si souvent.
const LONG_WINDOW_OVERLAP: usize = 16;
/// Une fenêtre de Hann étale une sinusoïde sur 1.5 case d'énergie : c'est ce qui
/// laisse une sinusoïde au même niveau en `Peak` et en `Energy`.
const HANN_NOISE_BANDWIDTH: f32 = 1.5;
/// Le plus petit noyau multi-résolution, en échantillons.
const MIN_KERNEL: usize = 32;
/// Constante de temps de la descente, en secondes. La montée est immédiate :
/// lisser une attaque retarderait le trig.
const RELEASE: f32 = 0.06;

/// Le spectre glissant d'un flux mono.
pub struct SpectrumAnalyzer {
    sample_rate: f32,
    settings: AnalysisSettings,
    planner: FftPlanner<f32>,
    fft: Arc<dyn Fft<f32>>,
    /// Les derniers échantillons, en anneau. Toujours `MAX_WINDOW` : changer de
    /// fenêtre ne perd rien, même si un `Param` la module.
    samples: Vec<f32>,
    write: usize,
    since_analysis: usize,
    /// La durée de la fenêtre, en échantillons.
    length: usize,
    hop: usize,
    window: Vec<f32>,
    buffer: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    magnitudes: Vec<f32>,
    /// Les bords de chaque bande, en cases de FFT (fractionnaires).
    bands: Vec<(f32, f32)>,
    /// Un noyau par bande, en `ConstantQ` seulement : vide sinon.
    kernels: Vec<Vec<Complex<f32>>>,
    /// Une sinusoïde à pleine échelle mesure 1.
    gain: f32,
    /// Divise une somme d'énergie pour qu'une sinusoïde y mesure son amplitude.
    energy_norm: f32,
    /// Part de l'écart parcourue à chaque analyse pendant une descente.
    release: f32,
    spectrum: Spectrum,
}

impl SpectrumAnalyzer {
    pub fn new(sample_rate: f32) -> Self {
        Self::with_settings(sample_rate, AnalysisSettings::default())
    }

    pub fn with_settings(sample_rate: f32, settings: AnalysisSettings) -> Self {
        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_forward(MIN_FFT_SIZE);
        let memoire = (MAX_WINDOW * sample_rate).ceil() as usize + 1;
        let mut analyzer = Self {
            sample_rate,
            settings,
            planner,
            fft,
            samples: vec![0.0; memoire],
            write: 0,
            since_analysis: 0,
            length: 0,
            hop: MAX_HOP,
            window: Vec::new(),
            buffer: Vec::new(),
            scratch: Vec::new(),
            magnitudes: Vec::new(),
            bands: Vec::new(),
            kernels: Vec::new(),
            gain: 1.0,
            energy_norm: 1.0,
            release: 1.0,
            spectrum: Spectrum::default(),
        };
        analyzer.rebuild();
        analyzer
    }

    /// Change de réglages. L'historique reste : la prochaine analyse relit le
    /// même son. Sans effet si rien ne change.
    pub fn configure(&mut self, settings: AnalysisSettings) {
        if settings != self.settings {
            self.settings = settings;
            self.rebuild();
        }
    }

    pub fn settings(&self) -> AnalysisSettings {
        self.settings
    }

    /// La durée de la fenêtre en cours, en échantillons.
    pub fn window_length(&self) -> usize {
        self.length
    }

    fn rebuild(&mut self) {
        let sr = self.sample_rate;
        let duree = sanitize_window(self.settings.window);
        let longueur = ((duree * sr).round() as usize).clamp(MIN_KERNEL, self.samples.len());
        let taille = (2 * longueur).next_power_of_two().max(MIN_FFT_SIZE);

        self.length = longueur;
        // Une moitié de fenêtre au plus : un saut plus long laisserait passer
        // une attaque entre deux analyses.
        self.hop = (longueur / 2)
            .min(MAX_HOP)
            .max(longueur / LONG_WINDOW_OVERLAP);
        self.fft = self.planner.plan_fft_forward(taille);
        self.scratch
            .resize(self.fft.get_inplace_scratch_len(), Complex::default());
        self.buffer.resize(taille, Complex::default());
        self.magnitudes.resize(taille / 2 + 1, 0.0);
        self.window = hann(longueur);
        self.gain = 2.0 / self.window.iter().sum::<f32>();
        self.energy_norm = HANN_NOISE_BANDWIDTH * taille as f32 / longueur as f32;
        self.release = 1.0 - (-(self.hop as f32 / sr) / RELEASE).exp();

        let echelle = self.settings.freq_scale;
        let bord = |b: usize| echelle.frequency(b as f32 / SPECTRUM_BANDS as f32);
        let par_case = sr / taille as f32;
        self.bands = (0..SPECTRUM_BANDS)
            .map(|i| (bord(i) / par_case, bord(i + 1) / par_case))
            .collect();
        self.kernels = if self.settings.method == AnalysisMethod::ConstantQ {
            (0..SPECTRUM_BANDS)
                .map(|i| kernel(bord(i), bord(i + 1), sr, longueur))
                .collect()
        } else {
            Vec::new()
        };

        // Les niveaux d'avant ne se relisent pas sur les nouvelles échelles.
        self.spectrum.freq_scale = echelle;
        self.spectrum.amp_scale = self.settings.amp_scale;
        self.spectrum.levels = [0.0; SPECTRUM_BANDS];
    }

    /// Ajoute un échantillon. Rend `true` si le spectre vient de changer.
    pub fn push(&mut self, sample: f32) -> bool {
        // Un échantillon non fini ne doit pas empoisonner toute une fenêtre.
        self.samples[self.write] = if sample.is_finite() { sample } else { 0.0 };
        self.write = (self.write + 1) % self.samples.len();
        self.since_analysis += 1;
        if self.since_analysis < self.hop {
            return false;
        }
        self.since_analysis = 0;
        self.analyze();
        true
    }

    pub fn spectrum(&self) -> &Spectrum {
        &self.spectrum
    }

    /// L'échantillon `age` positions avant le plus récent.
    fn sample(&self, age: usize) -> f32 {
        let n = self.samples.len();
        self.samples[(self.write + n - 1 - age % n) % n]
    }

    fn analyze(&mut self) {
        // Le continu d'un micro fuirait dans les premières bandes, celles où
        // l'on pose un trig de grosse caisse : il tiendrait la zone ouverte.
        let moyenne = (0..self.length).map(|a| self.sample(a)).sum::<f32>() / self.length as f32;
        if self.settings.method == AnalysisMethod::ConstantQ {
            for i in 0..SPECTRUM_BANDS {
                let mut somme = Complex::default();
                for (age, coefficient) in self.kernels[i].iter().enumerate() {
                    somme += *coefficient * (self.sample(age) - moyenne);
                }
                self.track(i, somme.norm());
            }
            return;
        }

        for i in 0..self.length {
            let s = self.sample(self.length - 1 - i);
            self.buffer[i] = Complex::new((s - moyenne) * self.window[i], 0.0);
        }
        self.buffer[self.length..].fill(Complex::default());
        self.fft
            .process_with_scratch(&mut self.buffer, &mut self.scratch);
        for (m, case) in self.magnitudes.iter_mut().zip(&self.buffer) {
            *m = case.norm() * self.gain;
        }

        for i in 0..SPECTRUM_BANDS {
            let (bas, haut) = self.bands[i];
            let magnitude = match self.settings.method {
                AnalysisMethod::Energy => {
                    band_energy(&self.magnitudes, bas, haut, self.energy_norm)
                }
                _ => band_magnitude(&self.magnitudes, bas, haut),
            };
            self.track(i, magnitude);
        }
    }

    /// Fait suivre à une bande sa nouvelle magnitude.
    fn track(&mut self, bande: usize, magnitude: f32) {
        let cible = self.settings.amp_scale.level(magnitude);
        let courant = &mut self.spectrum.levels[bande];
        *courant = if cible > *courant {
            cible
        } else {
            *courant + (cible - *courant) * self.release
        };
    }
}

/// Hann périodique : la fenêtre usuelle pour une FFT glissante.
fn hann(length: usize) -> Vec<f32> {
    (0..length)
        .map(|i| 0.5 - 0.5 * (TAU * i as f32 / length as f32).cos())
        .collect()
}

/// Le noyau d'une bande, du plus récent au plus ancien échantillon : une
/// sinusoïde complexe sous une fenêtre deux fois plus longue que l'inverse de la
/// largeur de la bande. Plafonné par la fenêtre d'analyse, qui garde donc son
/// sens ici aussi : c'est la durée du noyau le plus long.
fn kernel(bas: f32, haut: f32, sample_rate: f32, plafond: usize) -> Vec<Complex<f32>> {
    let centre = (bas + haut) * 0.5;
    if centre >= sample_rate * 0.5 {
        return Vec::new(); // au-dessus de Nyquist : rien à lire
    }
    let longueur = ((2.0 * sample_rate / (haut - bas)) as usize).clamp(MIN_KERNEL, plafond);
    let fenetre = hann(longueur);
    let gain = 2.0 / fenetre.iter().sum::<f32>();
    let pas = TAU * centre / sample_rate;
    fenetre
        .iter()
        .enumerate()
        .map(|(i, w)| Complex::from_polar(w * gain, -pas * i as f32))
        .collect()
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

/// La magnitude de toute l'énergie d'une bande. Plus étroite que la tache d'une
/// sinusoïde, une bande n'en contient qu'un morceau : elle se lit alors comme
/// en `Peak`.
fn band_energy(magnitudes: &[f32], bas: f32, haut: f32, norm: f32) -> f32 {
    let derniere = magnitudes.len() - 1;
    if bas >= derniere as f32 {
        return 0.0;
    }
    if haut - bas < norm {
        return band_magnitude(magnitudes, bas, haut);
    }
    let fin = (haut.floor() as usize).min(derniere);
    let somme: f32 = magnitudes[bas.ceil() as usize..=fin]
        .iter()
        .map(|m| m * m)
        .sum();
    (somme / norm).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;
    use frogenx_core::{AmpScale, FreqScale};

    const SR: f32 = 48_000.0;
    /// La fenêtre par défaut, en échantillons.
    const FENETRE: usize = 2048;

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
        s.freq_scale
            .frequency((i as f32 + 0.5) / SPECTRUM_BANDS as f32)
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
        for s in sinus(1000.0, 0.5, FENETRE, SR) {
            a.push(s);
        }
        let attaque = a.spectrum().peak(900.0, 1100.0);
        assert!(attaque > 0.85, "got {attaque}");

        // Une fenêtre de silence plus 20 ms : la fenêtre est vide, le niveau
        // n'est pas encore retombé.
        for _ in 0..FENETRE + 960 {
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
        let analyses = (0..MAX_HOP * 3).filter(|_| a.push(0.0)).count();
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

    /// Réglages à partir du défaut.
    fn reglages(method: AnalysisMethod, window: f32) -> AnalysisSettings {
        AnalysisSettings {
            method,
            window,
            ..Default::default()
        }
    }

    fn analyse_avec(settings: AnalysisSettings, samples: impl Iterator<Item = f32>) -> Spectrum {
        let mut a = SpectrumAnalyzer::with_settings(SR, settings);
        for s in samples {
            a.push(s);
        }
        *a.spectrum()
    }

    /// Un bruit blanc d'amplitude 0.5, à graine fixe (§20.3).
    fn bruit(n: usize) -> impl Iterator<Item = f32> {
        let mut etat = 1u32;
        (0..n).map(move |_| {
            etat = etat.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            ((etat >> 16) as f32 / 32_768.0 - 1.0) * 0.5
        })
    }

    #[test]
    fn la_fenetre_suit_la_duree_demandee() {
        for (duree, attendu) in [(0.01, 480), (0.1, 4800), (0.5, 24_000)] {
            let a = SpectrumAnalyzer::with_settings(SR, reglages(AnalysisMethod::Peak, duree));
            assert_eq!(a.window_length(), attendu);
        }
    }

    #[test]
    fn une_fenetre_hors_plage_est_saturee_sans_paniquer() {
        for duree in [0.0, -3.0, 500.0, f32::NAN] {
            let mut a = SpectrumAnalyzer::with_settings(SR, reglages(AnalysisMethod::Peak, duree));
            for s in sinus(1000.0, 0.5, 96_000, SR) {
                a.push(s);
            }
            assert!(a.spectrum().levels.iter().all(|l| l.is_finite()));
            assert!(a.window_length() >= 240 && a.window_length() <= 48_000);
        }
    }

    #[test]
    fn une_sinusoide_garde_son_niveau_quelle_que_soit_la_methode_et_la_fenetre() {
        let attendu = 1.0 - 6.02 / 72.0;
        for method in AnalysisMethod::ALL {
            for duree in [0.02, 0.043, 0.2] {
                let s = analyse_avec(reglages(method, duree), sinus(1000.0, 0.5, 48_000, SR));
                let n = s.peak(900.0, 1100.0);
                assert!((n - attendu).abs() < 0.03, "{method:?} {duree} s : {n}");
            }
        }
    }

    #[test]
    fn une_fenetre_longue_separe_deux_graves_qu_une_courte_confond() {
        // 100 et 130 Hz, séparés de 30 Hz : il faut bien plus de 33 ms pour les
        // distinguer. C'est tout l'enjeu du réglage.
        let accord = || {
            (0..48_000).map(|i| {
                let t = i as f32 / SR;
                0.4 * (TAU * 100.0 * t).sin() + 0.4 * (TAU * 130.0 * t).sin()
            })
        };
        let creux =
            |s: Spectrum| s.peak(100.0, 100.0).min(s.peak(130.0, 130.0)) - s.peak(114.0, 114.0);
        let longue = creux(analyse_avec(reglages(AnalysisMethod::Peak, 0.25), accord()));
        let courte = creux(analyse_avec(reglages(AnalysisMethod::Peak, 0.02), accord()));
        assert!(longue > 0.15, "un creux entre les deux notes : {longue}");
        assert!(courte < 0.03, "une seule bosse : {courte}");
    }

    #[test]
    fn une_fenetre_courte_reagit_plus_vite() {
        // 10 ms après le début d'une note : une fenêtre courte la voit déjà
        // entière, une longue encore à peine.
        let apres = |duree| {
            let mut a = SpectrumAnalyzer::with_settings(SR, reglages(AnalysisMethod::Peak, duree));
            for s in sinus(1000.0, 0.5, 480, SR) {
                a.push(s);
            }
            a.spectrum().peak(900.0, 1100.0)
        };
        assert!(apres(0.01) > apres(0.2) + 0.2);
    }

    #[test]
    fn l_energie_fait_peser_le_bruit_large_plus_que_la_crete() {
        // Dans une bande large, le bruit s'additionne : la crête n'en voit
        // qu'une case. Une sinusoïde, elle, reste au même niveau.
        let (crete, energie) = (
            analyse_avec(reglages(AnalysisMethod::Peak, 0.043), bruit(48_000)),
            analyse_avec(reglages(AnalysisMethod::Energy, 0.043), bruit(48_000)),
        );
        let (c, e) = (crete.peak(8000.0, 9000.0), energie.peak(8000.0, 9000.0));
        assert!(e > c + 0.03, "énergie {e}, crête {c}");
    }

    #[test]
    fn le_multi_resolution_voit_une_attaque_aigue_avant_la_fft() {
        // 3 ms de 8 kHz : la bande large a déjà sa fenêtre pleine, pas la FFT.
        let reglage = |method| reglages(method, 0.1);
        let mut fft = SpectrumAnalyzer::with_settings(SR, reglage(AnalysisMethod::Peak));
        let mut cq = SpectrumAnalyzer::with_settings(SR, reglage(AnalysisMethod::ConstantQ));
        for _ in 0..4096 {
            fft.push(0.0);
            cq.push(0.0);
        }
        // Une analyse juste après la note, au même instant pour les deux.
        fft.since_analysis = fft.hop - 144;
        cq.since_analysis = cq.hop - 144;
        for s in sinus(8000.0, 0.5, 144, SR) {
            fft.push(s);
            cq.push(s);
        }
        let (f, c) = (
            fft.spectrum().peak(7500.0, 8500.0),
            cq.spectrum().peak(7500.0, 8500.0),
        );
        assert!(c > f + 0.1, "multi-résolution {c}, FFT {f}");
    }

    #[test]
    fn les_noyaux_raccourcissent_avec_la_largeur_de_bande() {
        let etroit = kernel(100.0, 106.0, SR, 48_000);
        let large = kernel(10_000.0, 10_600.0, SR, 48_000);
        assert!(etroit.len() > 50 * large.len());
        assert_eq!(kernel(100.0, 101.0, SR, 2048).len(), 2048, "plafonné");
        assert!(kernel(30_000.0, 31_000.0, SR, 2048).is_empty(), "Nyquist");
    }

    #[test]
    fn chaque_echelle_place_une_sinusoide_a_sa_frequence() {
        for freq_scale in FreqScale::ALL {
            let settings = AnalysisSettings {
                freq_scale,
                ..Default::default()
            };
            let s = analyse_avec(settings, sinus(1000.0, 0.5, 8192, SR));
            assert_eq!(s.freq_scale, freq_scale);
            assert!(s.peak(800.0, 1250.0) > 0.8, "{freq_scale:?}");
            assert!(s.peak(4000.0, 20_000.0) < 0.3, "{freq_scale:?}");
        }
    }

    #[test]
    fn l_echelle_d_amplitude_passe_dans_le_spectre() {
        let settings = AnalysisSettings {
            amp_scale: AmpScale::Linear,
            ..Default::default()
        };
        let s = analyse_avec(settings, sinus(1000.0, 0.5, 8192, SR));
        assert_eq!(s.amp_scale, AmpScale::Linear);
        assert!((s.peak(900.0, 1100.0) - 0.5).abs() < 0.03);
    }

    #[test]
    fn changer_de_reglage_garde_l_historique() {
        // La sinusoïde est passée avant le changement : la première analyse
        // d'après la voit quand même.
        let mut a = SpectrumAnalyzer::new(SR);
        for s in sinus(1000.0, 0.5, 8192, SR) {
            a.push(s);
        }
        a.configure(reglages(AnalysisMethod::ConstantQ, 0.1));
        assert_eq!(a.spectrum().peak(900.0, 1100.0), 0.0, "niveaux effacés");
        for _ in 0..a.hop {
            a.push(0.0);
        }
        assert!(a.spectrum().peak(900.0, 1100.0) > 0.5);
    }

    #[test]
    fn reconfigurer_a_l_identique_ne_touche_a_rien() {
        let mut a = SpectrumAnalyzer::new(SR);
        for s in sinus(1000.0, 0.5, 8192, SR) {
            a.push(s);
        }
        let avant = *a.spectrum();
        a.configure(AnalysisSettings::default());
        assert_eq!(*a.spectrum(), avant);
    }

    #[test]
    fn une_fenetre_courte_ne_laisse_pas_de_trou_entre_deux_analyses() {
        // Le saut ne dépasse jamais la moitié de la fenêtre.
        let a = SpectrumAnalyzer::with_settings(SR, reglages(AnalysisMethod::Peak, 0.005));
        assert!(a.hop * 2 <= a.window_length());
    }
}
