//! Le thread d'analyse — §7, §20.2 de `docs/ARCHITECTURE.md`.
//!
//! ```text
//! [callback audio] ──▶ rtrb ──▶ [ce thread] ──▶ triple buffer ──▶ [rendu]
//! ```
//!
//! Ne connaît pas `cpal` : un test remplit l'anneau à la main.

use crate::analyzer::SpectrumAnalyzer;
use frogenx_core::{AnalysisSettings, AudioFeatures};
use rtrb::{Consumer, Producer, RingBuffer};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;
use triple_buffer::{triple_buffer, Input, Output};

/// L'attente quand l'anneau est vide. Courte devant les 10 ms entre analyses.
const IDLE: Duration = Duration::from_millis(1);
/// L'énergie moyenne (carré du RMS, -60 dB) sous laquelle un canal ne compte
/// pas dans le mixage.
const SILENT_CHANNEL: f32 = 1e-6;
/// Réglages en attente. Le thread les lit à chaque tour : quelques-uns suffisent.
const PENDING_SETTINGS: usize = 4;

/// Le thread d'analyse, arrêté et attendu à la destruction.
pub struct Analysis {
    output: Output<AudioFeatures>,
    settings: Producer<AnalysisSettings>,
    /// Les derniers réglages transmis.
    current: AnalysisSettings,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Analysis {
    /// Lance l'analyse d'un flux entrelacé de `channels` canaux.
    pub fn spawn(
        samples: Consumer<f32>,
        channels: usize,
        sample_rate: f32,
    ) -> std::io::Result<Self> {
        let (input, output) = triple_buffer(&AudioFeatures::default());
        let (reglages, lus) = RingBuffer::new(PENDING_SETTINGS);
        let stop = Arc::new(AtomicBool::new(false));
        let arret = Arc::clone(&stop);
        let thread = std::thread::Builder::new()
            .name("analyse audio".into())
            .spawn(move || run(samples, lus, channels.max(1), sample_rate, input, &arret))?;
        Ok(Self {
            output,
            settings: reglages,
            current: AnalysisSettings::default(),
            stop,
            thread: Some(thread),
        })
    }

    /// Les dernières features : ne bloque jamais (§20.2). Si l'analyse est en
    /// retard, c'est la lecture précédente.
    pub fn read(&mut self) -> &AudioFeatures {
        self.output.read()
    }

    /// Transmet des réglages au thread d'analyse, sans bloquer (§20.2). File
    /// pleine : ils restent différents de `current`, donc retentés à la frame
    /// suivante. Sans effet si rien ne change.
    pub fn configure(&mut self, settings: AnalysisSettings) {
        if settings != self.current && self.settings.push(settings).is_ok() {
            self.current = settings;
        }
    }
}

impl Drop for Analysis {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Ramène un bloc entrelacé en mono : la moyenne des canaux **qui jouent**.
///
/// Une moyenne de tous les canaux diluerait le son d'une carte à six entrées
/// dont une seule est branchée : -15 dB, assez pour ne plus atteindre une zone.
/// La décision se prend par bloc, pas par échantillon : le mélange reste linéaire
/// et n'ajoute pas d'harmoniques au spectre.
fn downmix(a: &[f32], b: &[f32], energies: &mut [f32], mut push: impl FnMut(f32)) {
    let channels = energies.len();
    energies.fill(0.0);
    for (i, s) in a.iter().chain(b).enumerate() {
        energies[i % channels] += s * s;
    }
    let frames = (a.len() + b.len()) / channels;
    let seuil = SILENT_CHANNEL * frames as f32;
    let actifs = energies.iter().filter(|e| **e >= seuil).count();
    // `energies` sert ici de masque, le temps de ce bloc. Tous muets : on les
    // garde tous, le silence reste un silence.
    for e in energies.iter_mut() {
        *e = if actifs == 0 || *e >= seuil { 1.0 } else { 0.0 };
    }
    let diviseur = if actifs == 0 { channels } else { actifs } as f32;

    let mut somme = 0.0;
    for (i, s) in a.iter().chain(b).enumerate() {
        somme += s * energies[i % channels];
        if (i + 1) % channels == 0 {
            push(somme / diviseur);
            somme = 0.0;
        }
    }
}

fn run(
    mut samples: Consumer<f32>,
    mut settings: Consumer<AnalysisSettings>,
    channels: usize,
    sample_rate: f32,
    mut output: Input<AudioFeatures>,
    stop: &AtomicBool,
) {
    let mut analyzer = SpectrumAnalyzer::new(sample_rate);
    let mut energies = vec![0.0; channels];
    while !stop.load(Ordering::Relaxed) {
        // L'allocation d'un changement de fenêtre est permise ici : ce thread
        // n'est pas temps réel, et le callback n'attend jamais après lui.
        while let Ok(reglages) = settings.pop() {
            analyzer.configure(reglages);
        }
        // Des frames entières seulement : le callback écrit par blocs entiers,
        // les canaux restent donc alignés.
        let disponibles = samples.slots() - samples.slots() % channels;
        let Ok(bloc) = samples.read_chunk(disponibles) else {
            continue;
        };
        if disponibles == 0 {
            std::thread::sleep(IDLE);
            continue;
        }

        let (a, b) = bloc.as_slices();
        let mut nouveau = false;
        downmix(a, b, &mut energies, |mono| {
            nouveau |= analyzer.push(mono);
        });
        bloc.commit_all();

        if nouveau {
            output.write(AudioFeatures {
                spectrum: *analyzer.spectrum(),
                ..Default::default()
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use frogenx_core::{AmpScale, FreqScale};
    use std::f32::consts::TAU;
    use std::time::Instant;

    const SR: f32 = 48_000.0;

    /// Lit jusqu'à ce que `condition` soit vraie, ou échoue après 5 s.
    fn attend(analysis: &mut Analysis, condition: impl Fn(&AudioFeatures) -> bool) {
        let debut = Instant::now();
        while !condition(analysis.read()) {
            assert!(debut.elapsed().as_secs() < 5, "rien reçu de l'analyse");
            std::thread::sleep(IDLE);
        }
    }

    #[test]
    fn un_flux_stereo_ressort_en_spectre_de_l_autre_cote() {
        let (mut entree, sortie) = RingBuffer::new(32_768);
        let mut analysis = Analysis::spawn(sortie, 2, SR).unwrap();
        assert_eq!(analysis.read().spectrum.peak(900.0, 1100.0), 0.0);

        for i in 0..8192 {
            let s = 0.5 * (TAU * 1000.0 * i as f32 / SR).sin();
            entree.push(s).unwrap();
            entree.push(s).unwrap();
        }
        attend(&mut analysis, |a| a.spectrum.peak(900.0, 1100.0) > 0.8);
    }

    /// Le mono d'un bloc.
    fn mixe(bloc: &[f32], channels: usize) -> Vec<f32> {
        let mut mono = Vec::new();
        downmix(bloc, &[], &mut vec![0.0; channels], |s| mono.push(s));
        mono
    }

    #[test]
    fn deux_canaux_identiques_ne_gagnent_pas_de_niveau() {
        assert_eq!(mixe(&[0.5, 0.5, -0.25, -0.25], 2), [0.5, -0.25]);
    }

    #[test]
    fn un_canal_muet_ne_dilue_pas_les_autres() {
        // Une carte à six entrées, une seule branchée : le son reste entier.
        let mut bloc = vec![0.0; 12];
        bloc[3] = 0.5;
        bloc[9] = -0.5;
        assert_eq!(mixe(&bloc, 6), [0.5, -0.5]);
    }

    #[test]
    fn deux_canaux_actifs_sur_six_font_leur_moyenne() {
        let mut bloc = vec![0.0; 6];
        (bloc[0], bloc[1]) = (0.75, 0.25);
        assert_eq!(mixe(&bloc, 6), [0.5]);
    }

    #[test]
    fn un_souffle_sous_le_seuil_ne_compte_pas_comme_un_canal() {
        assert_eq!(mixe(&[0.5, 1e-4], 2), [0.5]);
    }

    #[test]
    fn tous_les_canaux_muets_restent_un_silence_sans_division_par_zero() {
        assert_eq!(mixe(&[0.0; 4], 2), [0.0, 0.0]);
    }

    #[test]
    fn un_bloc_coupe_en_deux_tranches_se_mixe_pareil() {
        // L'anneau rend parfois deux tranches : une frame peut être à cheval.
        let mut mono = Vec::new();
        downmix(&[0.5], &[0.5, -0.5, -0.5], &mut [0.0; 2], |s| mono.push(s));
        assert_eq!(mono, [0.5, -0.5]);
    }

    #[test]
    fn des_reglages_transmis_changent_le_spectre_produit() {
        let (mut entree, sortie) = RingBuffer::new(32_768);
        let mut analysis = Analysis::spawn(sortie, 1, SR).unwrap();
        analysis.configure(AnalysisSettings {
            freq_scale: FreqScale::Linear,
            amp_scale: AmpScale::Linear,
            ..Default::default()
        });
        for i in 0..8192 {
            entree
                .push(0.5 * (TAU * 1000.0 * i as f32 / SR).sin())
                .unwrap();
        }
        attend(&mut analysis, |a| {
            a.spectrum.freq_scale == FreqScale::Linear
                && a.spectrum.amp_scale == AmpScale::Linear
                && (a.spectrum.peak(900.0, 1100.0) - 0.5).abs() < 0.05
        });
    }

    #[test]
    fn des_reglages_identiques_ne_remplissent_pas_la_file() {
        let (_entree, sortie) = RingBuffer::<f32>::new(16);
        let mut analysis = Analysis::spawn(sortie, 1, SR).unwrap();
        for _ in 0..100 {
            analysis.configure(AnalysisSettings::default());
        }
        assert_eq!(analysis.settings.slots(), PENDING_SETTINGS);
    }

    #[test]
    fn arreter_l_analyse_rend_la_main() {
        let (entree, sortie) = RingBuffer::<f32>::new(16);
        let analysis = Analysis::spawn(sortie, 1, SR).unwrap();
        drop(analysis); // ne doit pas bloquer, même sans aucun échantillon
        assert!(entree.is_abandoned(), "le thread a rendu le consommateur");
    }
}
