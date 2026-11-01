//! Des échantillons aux features — §7.1 de `docs/ARCHITECTURE.md`.
//!
//! Pur, comme `analyzer` : un test y pousse du son de synthèse. **Une analyse à
//! part, fixe :** le réglage du spectre sert à lire et à poser des zones, et le
//! changer ne doit déplacer ni le tempo ni une modulation.

use crate::tempo::TempoTracker;
use frogenx_core::{AmpScale, AudioFeatures, FreqScale, CHROMA_NOTES};
use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};
use std::f32::consts::TAU;
use std::sync::Arc;

/// Analyses par seconde visées, quelle que soit la fréquence d'échantillonnage.
const ANALYSES_PER_SECOND: f32 = 86.0;
/// Le sommet des graves et le pied des aigus, en Hz.
const BASS_TOP: f32 = 250.0;
const TREBLE_BOTTOM: f32 = 4_000.0;
/// Le plancher de `rms`, en dB : au-dessous, c'est un silence.
const RMS_FLOOR_DB: f32 = -60.0;
/// Une fenêtre de Hann étale une sinusoïde sur 1.5 case d'énergie.
const HANN_NOISE_BANDWIDTH: f32 = 1.5;
/// La fenêtre de la chroma, en secondes : séparer deux demi-tons graves veut du
/// temps. Relue une fois sur quatre : une note dure plus qu'une analyse.
const CHROMA_WINDOW: f32 = 0.09;
const CHROMA_EVERY: u32 = 4;
const CHROMA_MIN_HZ: f32 = 110.0;
const CHROMA_MAX_HZ: f32 = 2_000.0;
/// La compression de la force des transitoires : un coup doux compte aussi.
const FLUX_COMPRESSION: f32 = 100.0;
/// Ce qu'on compare à la force courante, en secondes.
const ONSET_MEMORY: f32 = 0.4;
/// De combien la force doit dépasser sa moyenne récente.
const ONSET_RATIO: f32 = 1.8;
/// Sous cette force, ce n'est pas un coup, quelle que soit la moyenne.
const ONSET_FLOOR: f32 = 0.02;
/// Deux coups plus rapprochés n'en font qu'un.
const ONSET_GAP: f32 = 0.05;
/// La retombée de `onset`, en secondes.
const ONSET_RELEASE: f32 = 0.12;
/// De combien d'analyses la force d'un coup arrive après lui : mesuré sur des
/// boucles de synthèse (`tests/tempo.rs`). La latence de la carte son n'y est pas.
const ONSET_LAG: f32 = 1.5;

fn hann(length: usize) -> Vec<f32> {
    (0..length)
        .map(|i| 0.5 - 0.5 * (TAU * i as f32 / length as f32).cos())
        .collect()
}

/// Une FFT fenêtrée d'un flux réel. Rend des magnitudes : une sinusoïde à pleine
/// échelle mesure 1.
struct Spectrum {
    fft: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    gain: f32,
    buffer: Vec<Complex<f32>>,
    scratch: Vec<Complex<f32>>,
    magnitudes: Vec<f32>,
    /// Hz par case.
    bin_hz: f32,
}

impl Spectrum {
    fn new(planner: &mut FftPlanner<f32>, length: usize, sample_rate: f32) -> Self {
        let fft = planner.plan_fft_forward(length);
        let window = hann(length);
        let gain = 2.0 / window.iter().sum::<f32>();
        let scratch = vec![Complex::default(); fft.get_inplace_scratch_len()];
        Self {
            fft,
            window,
            gain,
            buffer: vec![Complex::default(); length],
            scratch,
            magnitudes: vec![0.0; length / 2 + 1],
            bin_hz: sample_rate / length as f32,
        }
    }

    /// `sample(age)` : l'échantillon `age` positions avant le plus récent.
    fn analyze(&mut self, sample: impl Fn(usize) -> f32) {
        let n = self.window.len();
        let moyenne = (0..n).map(&sample).sum::<f32>() / n as f32;
        for i in 0..n {
            let s = sample(n - 1 - i) - moyenne;
            self.buffer[i] = Complex::new(s * self.window[i], 0.0);
        }
        self.fft
            .process_with_scratch(&mut self.buffer, &mut self.scratch);
        for (m, case) in self.magnitudes.iter_mut().zip(&self.buffer) {
            *m = case.norm() * self.gain;
        }
    }

    /// La magnitude d'une sinusoïde qui porterait toute l'énergie de `bas..haut`.
    fn band(&self, bas: f32, haut: f32) -> f32 {
        let debut = ((bas / self.bin_hz).ceil() as usize).max(1);
        let fin = ((haut / self.bin_hz) as usize).min(self.magnitudes.len() - 1);
        let somme: f32 = self.magnitudes[debut..=fin.max(debut)]
            .iter()
            .map(|m| m * m)
            .sum();
        (somme / HANN_NOISE_BANDWIDTH).sqrt()
    }
}

/// Les features d'un flux mono.
pub struct FeatureAnalyzer {
    sample_rate: f32,
    samples: Vec<f32>,
    write: usize,
    hop: usize,
    since_analysis: usize,
    short: Spectrum,
    long: Spectrum,
    /// Les magnitudes compressées de l'analyse d'avant.
    previous: Vec<f32>,
    /// La force des dernières analyses, en anneau.
    recent: Vec<f32>,
    recent_write: usize,
    analyses: u32,
    /// Analyses depuis le dernier coup.
    since_onset: u32,
    onset_gap: u32,
    onset_release: f32,
    tempo: TempoTracker,
    rms: f32,
    bands: [f32; 3],
    centroid: f32,
    onset: f32,
    chroma: [f32; CHROMA_NOTES],
}

impl FeatureAnalyzer {
    pub fn new(sample_rate: f32) -> Self {
        let hop = (sample_rate / ANALYSES_PER_SECOND).round().max(1.0) as usize;
        let cadence = sample_rate / hop as f32;
        let court = (2 * hop).next_power_of_two();
        let long = ((CHROMA_WINDOW * sample_rate) as usize).next_power_of_two();
        let mut planner = FftPlanner::new();
        Self {
            sample_rate,
            samples: vec![0.0; long.max(court)],
            write: 0,
            hop,
            since_analysis: 0,
            short: Spectrum::new(&mut planner, court, sample_rate),
            long: Spectrum::new(&mut planner, long, sample_rate),
            previous: vec![0.0; court / 2 + 1],
            recent: vec![0.0; ((ONSET_MEMORY * cadence) as usize).max(1)],
            recent_write: 0,
            analyses: 0,
            since_onset: u32::MAX,
            onset_gap: (ONSET_GAP * cadence).ceil() as u32,
            onset_release: (-1.0 / (ONSET_RELEASE * cadence)).exp(),
            tempo: TempoTracker::new(cadence),
            rms: 0.0,
            bands: [0.0; 3],
            centroid: 0.0,
            onset: 0.0,
            chroma: [0.0; CHROMA_NOTES],
        }
    }

    /// Ajoute un échantillon. Rend `true` si les features viennent de changer.
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

    /// Écrit les features dans `out`, sans toucher à son spectre.
    pub fn write_to(&self, out: &mut AudioFeatures) {
        out.rms = self.rms;
        out.bands = self.bands;
        out.centroid = self.centroid;
        out.onset = self.onset;
        out.chroma = self.chroma;
        out.bpm = self.tempo.bpm();
        out.bpm_confidence = self.tempo.confidence();
        out.beat_phase = self.tempo.beat_phase(ONSET_LAG);
        out.beat_confidence = self.tempo.beat_confidence();
    }

    fn analyze(&mut self) {
        let (samples, write) = (&self.samples, self.write);
        let n = samples.len();
        let lire = |age: usize| samples[(write + n - 1 - age % n) % n];
        self.short.analyze(lire);
        self.analyses = self.analyses.wrapping_add(1);

        let taille = self.short.window.len();
        let energie = (0..taille).map(|a| lire(a) * lire(a)).sum::<f32>() / taille as f32;
        let db = 10.0 * energie.max(1e-12).log10();
        self.rms = ((db - RMS_FLOOR_DB) / -RMS_FLOOR_DB).clamp(0.0, 1.0);
        let niveau = |m: f32| AmpScale::Decibel.level(m);
        let nyquist = self.sample_rate * 0.5;
        self.bands = [
            niveau(self.short.band(0.0, BASS_TOP)),
            niveau(self.short.band(BASS_TOP, TREBLE_BOTTOM)),
            niveau(self.short.band(TREBLE_BOTTOM, nyquist)),
        ];

        let silence = self.rms <= 0.0;
        self.centroid = if silence {
            0.0
        } else {
            FreqScale::Log.position(self.mean_frequency())
        };
        if self.analyses.is_multiple_of(CHROMA_EVERY) {
            if silence {
                self.chroma = [0.0; CHROMA_NOTES];
            } else {
                self.long.analyze(lire);
                self.chroma = chroma(&self.long.magnitudes, self.long.bin_hz);
            }
        }

        let (force, graves) = self.flux();
        self.track_onset(force);
        self.tempo.push_with_bass(force, graves);
    }

    fn mean_frequency(&self) -> f32 {
        let hz = self.short.bin_hz;
        let (mut somme, mut poids) = (0.0, 0.0);
        for (i, m) in self.short.magnitudes.iter().enumerate().skip(1) {
            somme += m * i as f32 * hz;
            poids += m;
        }
        if poids > 0.0 {
            somme / poids
        } else {
            0.0
        }
    }

    /// La montée du spectre, case par case : ce qu'un coup a de propre, et ce
    /// qu'une note tenue n'a pas.
    /// La force de la transitoire, sur tout le spectre puis sur les graves seuls.
    fn flux(&mut self) -> (f32, f32) {
        let derniere_grave = (BASS_TOP / self.short.bin_hz) as usize;
        let (mut somme, mut graves) = (0.0, 0.0);
        for (i, (avant, m)) in self
            .previous
            .iter_mut()
            .zip(&self.short.magnitudes)
            .enumerate()
        {
            let comprime = (1.0 + FLUX_COMPRESSION * m).ln();
            let montee = (comprime - *avant).max(0.0);
            somme += montee;
            if i <= derniere_grave {
                graves += montee;
            }
            *avant = comprime;
        }
        (
            somme / self.previous.len() as f32,
            graves / (derniere_grave + 1) as f32,
        )
    }

    fn track_onset(&mut self, force: f32) {
        let moyenne = self.recent.iter().sum::<f32>() / self.recent.len() as f32;
        self.since_onset = self.since_onset.saturating_add(1);
        self.onset *= self.onset_release;
        if force > ONSET_FLOOR
            && force > moyenne * ONSET_RATIO
            && self.since_onset >= self.onset_gap
        {
            self.onset = 1.0;
            self.since_onset = 0;
        }
        self.recent[self.recent_write] = force;
        self.recent_write = (self.recent_write + 1) % self.recent.len();
    }
}

/// L'énergie de chaque note, la plus forte à 1. Do en tête.
fn chroma(magnitudes: &[f32], bin_hz: f32) -> [f32; CHROMA_NOTES] {
    let mut notes = [0.0f32; CHROMA_NOTES];
    let debut = (CHROMA_MIN_HZ / bin_hz).ceil() as usize;
    let fin = ((CHROMA_MAX_HZ / bin_hz) as usize).min(magnitudes.len() - 1);
    for (i, m) in magnitudes.iter().enumerate().take(fin + 1).skip(debut) {
        let hz = i as f32 * bin_hz;
        // 69 : le numéro MIDI du la à 440 Hz.
        let note = (12.0 * (hz / 440.0).log2()).round() as i32 + 69;
        notes[note.rem_euclid(CHROMA_NOTES as i32) as usize] += m * m;
    }
    let plus_forte = notes.iter().copied().fold(0.0, f32::max);
    if plus_forte > 0.0 {
        notes.iter_mut().for_each(|n| *n /= plus_forte);
    }
    notes
}

#[cfg(test)]
mod tests {
    use super::*;

    const SR: f32 = 44_100.0;

    fn sinus(hz: f32, niveau: f32, i: usize, sr: f32) -> f32 {
        niveau * (TAU * hz * i as f32 / sr).sin()
    }

    /// Les features après une seconde de ce son.
    fn apres(sr: f32, son: impl Fn(usize) -> f32) -> AudioFeatures {
        let mut a = FeatureAnalyzer::new(sr);
        for i in 0..sr as usize {
            a.push(son(i));
        }
        let mut f = AudioFeatures::default();
        a.write_to(&mut f);
        f
    }

    #[test]
    fn un_silence_ne_donne_rien() {
        let f = apres(SR, |_| 0.0);
        assert_eq!(f.rms, 0.0);
        assert_eq!(f.bands, [0.0; 3]);
        assert_eq!(f.centroid, 0.0);
        assert_eq!(f.onset, 0.0);
        assert_eq!(f.chroma, [0.0; CHROMA_NOTES]);
    }

    #[test]
    fn le_volume_suit_le_niveau() {
        // Une sinusoïde à pleine échelle : -3 dB, soit 0.95.
        let fort = apres(SR, |i| sinus(440.0, 1.0, i, SR));
        assert!((fort.rms - 0.95).abs() < 0.01, "{}", fort.rms);
        // 20 dB plus bas : 20 / 60 de moins.
        let bas = apres(SR, |i| sinus(440.0, 0.1, i, SR));
        assert!((fort.rms - bas.rms - 1.0 / 3.0).abs() < 0.01, "{}", bas.rms);
    }

    #[test]
    fn chaque_registre_se_lit_dans_sa_bande() {
        for (hz, attendue) in [(80.0, 0), (1_000.0, 1), (9_000.0, 2)] {
            let f = apres(SR, |i| sinus(hz, 0.5, i, SR));
            let forte = (0..3)
                .max_by(|a, b| f.bands[*a].total_cmp(&f.bands[*b]))
                .unwrap();
            assert_eq!(forte, attendue, "{hz} Hz : {:?}", f.bands);
            assert!(f.bands[forte] > 0.8, "{:?}", f.bands);
        }
    }

    #[test]
    fn une_sinusoide_mesure_son_niveau_dans_sa_bande() {
        let f = apres(SR, |i| sinus(1_000.0, 1.0, i, SR));
        assert!((f.bands[1] - 1.0).abs() < 0.03, "{:?}", f.bands);
    }

    #[test]
    fn le_centroide_monte_avec_la_hauteur() {
        let (grave, aigu) = (
            apres(SR, |i| sinus(200.0, 0.5, i, SR)),
            apres(SR, |i| sinus(5_000.0, 0.5, i, SR)),
        );
        assert!(grave.centroid < aigu.centroid);
        let attendu = FreqScale::Log.position(5_000.0);
        assert!((aigu.centroid - attendu).abs() < 0.05, "{}", aigu.centroid);
    }

    #[test]
    fn la_chroma_reconnait_une_note() {
        for (hz, note) in [(440.0, 9), (261.63, 0), (329.63, 4), (880.0, 9)] {
            let f = apres(SR, |i| sinus(hz, 0.5, i, SR));
            assert_eq!(f.chroma[note], 1.0, "{hz} Hz : {:?}", f.chroma);
            let autres = f.chroma.iter().enumerate().filter(|(n, _)| *n != note);
            assert!(
                autres.clone().all(|(_, v)| *v < 0.3),
                "{hz} : {:?}",
                f.chroma
            );
        }
    }

    #[test]
    fn un_coup_declenche_une_impulsion_qui_retombe() {
        let mut a = FeatureAnalyzer::new(SR);
        let mut f = AudioFeatures::default();
        for _ in 0..(SR as usize / 2) {
            a.push(0.0);
        }
        a.write_to(&mut f);
        assert_eq!(f.onset, 0.0);

        let mut plus_haut: f32 = 0.0;
        for i in 0..2_000 {
            a.push(sinus(200.0, 0.8, i, SR));
            a.write_to(&mut f);
            plus_haut = plus_haut.max(f.onset);
        }
        assert!(plus_haut > 0.8, "{plus_haut}");
        for _ in 0..(SR as usize) {
            a.push(0.0);
        }
        a.write_to(&mut f);
        assert!(f.onset < 0.01, "{}", f.onset);
    }

    #[test]
    fn une_note_tenue_ne_declenche_plus_rien() {
        let f = apres(SR, |i| sinus(440.0, 0.5, i, SR));
        assert!(f.onset < 0.05, "{}", f.onset);
    }

    #[test]
    fn un_echantillon_non_fini_ne_contamine_rien() {
        let f = apres(SR, |i| if i == 100 { f32::NAN } else { 0.0 });
        assert!(f.rms.is_finite() && f.centroid.is_finite());
    }

    #[test]
    fn les_mesures_ne_dependent_pas_de_la_frequence_d_echantillonnage() {
        for sr in [44_100.0, 48_000.0, 96_000.0] {
            let f = apres(sr, |i| sinus(1_000.0, 0.5, i, sr));
            assert!(f.bands[1] > 0.85, "{sr} : {:?}", f.bands);
            assert_eq!(f.chroma.iter().copied().fold(0.0, f32::max), 1.0);
        }
    }
}
