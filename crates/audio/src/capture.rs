//! L'entrée audio — §7, §9.2 de `docs/ARCHITECTURE.md`. Seul module à toucher
//! à `cpal`, donc à la carte son.

use crate::analysis::Analysis;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{ErrorKind, FromSample, SampleFormat, SizedSample};
use frogenx_core::{AnalysisSettings, AudioFeatures};
use rtrb::{Producer, RingBuffer};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

/// Durée de l'anneau entre le callback et l'analyse, en secondes. Large : il
/// n'est plein que si le thread d'analyse décroche.
const RING_SECONDS: f32 = 0.5;

/// Une entrée qui n'a pas pu s'ouvrir. Régime d'édition (§18.1) : l'application
/// continue sans audio, et l'interface le dit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioError {
    NoDevice,
    Unsupported(String),
    Stream(String),
}

impl std::fmt::Display for AudioError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AudioError::NoDevice => write!(f, "aucune entrée audio"),
            AudioError::Unsupported(format) => write!(f, "format audio non géré : {format}"),
            AudioError::Stream(message) => write!(f, "entrée audio impossible : {message}"),
        }
    }
}

impl std::error::Error for AudioError {}

/// Ce que le callback remonte, sans verrou.
#[derive(Default)]
struct Health {
    /// Blocs jetés faute de place : l'analyse a décroché.
    dropped: AtomicU64,
    /// L'entrée a disparu en cours de route.
    lost: AtomicBool,
}

/// L'entrée par défaut, capturée et analysée. Le flux `cpal` n'est pas `Send`
/// partout : ce type reste sur le thread qui l'a ouverte.
pub struct AudioInput {
    // Détruit avant `analysis` : plus d'écriture dans un anneau abandonné.
    _stream: cpal::Stream,
    analysis: Analysis,
    health: Arc<Health>,
    silence: AudioFeatures,
    device: String,
    channels: usize,
    sample_rate: u32,
}

impl AudioInput {
    pub fn open_default() -> Result<Self, AudioError> {
        let host = cpal::default_host();
        let device = host.default_input_device().ok_or(AudioError::NoDevice)?;
        let supported = device
            .default_input_config()
            .map_err(|e| AudioError::Stream(e.to_string()))?;
        let config = supported.config();
        let (channels, sample_rate) = (config.channels as usize, config.sample_rate);

        let capacite = (sample_rate as f32 * RING_SECONDS) as usize * channels;
        let (entree, sortie) = RingBuffer::new(capacite.max(1));
        let analysis = Analysis::spawn(sortie, channels, sample_rate as f32)
            .map_err(|e| AudioError::Stream(e.to_string()))?;
        let health = Arc::new(Health::default());

        let sante = Arc::clone(&health);
        let stream = match supported.sample_format() {
            SampleFormat::F32 => capture::<f32>(&device, config, entree, sante),
            SampleFormat::I16 => capture::<i16>(&device, config, entree, sante),
            SampleFormat::U16 => capture::<u16>(&device, config, entree, sante),
            SampleFormat::I32 => capture::<i32>(&device, config, entree, sante),
            autre => return Err(AudioError::Unsupported(autre.to_string())),
        }
        .map_err(|e| AudioError::Stream(e.to_string()))?;
        stream
            .play()
            .map_err(|e| AudioError::Stream(e.to_string()))?;

        Ok(Self {
            _stream: stream,
            analysis,
            health,
            silence: AudioFeatures::default(),
            device: device.to_string(),
            channels,
            sample_rate,
        })
    }

    /// Les dernières features, sans bloquer (§20.2). Une entrée perdue rend le
    /// silence : un spectre figé tiendrait une zone ouverte indéfiniment.
    pub fn read(&mut self) -> &AudioFeatures {
        if self.health.lost.load(Ordering::Relaxed) {
            return &self.silence;
        }
        self.analysis.read()
    }

    /// Change l'analyse, sans bloquer : à appeler à chaque frame (§7).
    pub fn configure(&mut self, settings: AnalysisSettings) {
        self.analysis.configure(settings);
    }

    /// L'état de l'entrée, en mots : ce que lit l'interface.
    pub fn status(&self) -> String {
        let perdus = self.health.dropped.load(Ordering::Relaxed);
        if self.health.lost.load(Ordering::Relaxed) {
            format!("Entrée perdue : {}", self.device)
        } else if perdus > 0 {
            format!("{} — {perdus} blocs perdus", self.device)
        } else {
            let (canaux, frequence) = (self.channels, self.sample_rate);
            format!("{} — {canaux} canaux, {frequence} Hz", self.device)
        }
    }
}

fn capture<T>(
    device: &cpal::Device,
    config: cpal::StreamConfig,
    mut samples: Producer<f32>,
    health: Arc<Health>,
) -> Result<cpal::Stream, cpal::Error>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let sante = Arc::clone(&health);
    device.build_input_stream(
        config,
        // Temps réel dur : ni allocation, ni verrou, ni I/O (§7). Le bloc
        // passe en entier ou pas du tout : couper une frame mélangerait les
        // canaux jusqu'à la fin de la session.
        move |data: &[T], _| match samples.write_chunk_uninit(data.len()) {
            Ok(bloc) => {
                bloc.fill_from_iter(data.iter().map(|s| s.to_sample::<f32>()));
            }
            Err(_) => {
                sante.dropped.fetch_add(1, Ordering::Relaxed);
            }
        },
        move |erreur| {
            // Un décrochage est passager ; le reste ne revient pas tout seul.
            if erreur.kind() != ErrorKind::Xrun {
                health.lost.store(true, Ordering::Relaxed);
            }
        },
        None,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn une_erreur_d_ouverture_se_lit_en_francais() {
        assert_eq!(AudioError::NoDevice.to_string(), "aucune entrée audio");
        assert_eq!(
            AudioError::Unsupported("u8".into()).to_string(),
            "format audio non géré : u8"
        );
    }
}
