//! Le son d'une prise : un `.wav` écrit sur un thread à part — §8.2 de
//! `docs/ARCHITECTURE.md`.
//!
//! Le disque n'a rien à faire dans le callback (§7), ni même dans l'analyse, qui
//! décrocherait à chaque écriture lente. Ne connaît pas `cpal`.

use rtrb::Consumer;
use std::fs::File;
use std::io::{self, BufWriter, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// L'attente quand l'anneau est vide.
const IDLE: Duration = Duration::from_millis(5);
const HEADER_BYTES: u64 = 44;
const SAMPLE_BYTES: u64 = 4;
/// Le format IEEE float du RIFF.
const FLOAT_FORMAT: u16 = 3;

/// L'en-tête d'un `.wav` mono en flottants 32 bits.
pub fn wav_header(sample_rate: u32, samples: u64) -> [u8; HEADER_BYTES as usize] {
    let donnees = (samples * SAMPLE_BYTES).min(u32::MAX as u64 - HEADER_BYTES) as u32;
    let octets_par_seconde = sample_rate.wrapping_mul(SAMPLE_BYTES as u32);
    let mut h = Vec::with_capacity(HEADER_BYTES as usize);
    h.extend(b"RIFF");
    h.extend((donnees + 36).to_le_bytes());
    h.extend(b"WAVEfmt ");
    h.extend(16u32.to_le_bytes());
    h.extend(FLOAT_FORMAT.to_le_bytes());
    h.extend(1u16.to_le_bytes());
    h.extend(sample_rate.to_le_bytes());
    h.extend(octets_par_seconde.to_le_bytes());
    h.extend((SAMPLE_BYTES as u16).to_le_bytes());
    h.extend(32u16.to_le_bytes());
    h.extend(b"data");
    h.extend(donnees.to_le_bytes());
    let mut sortie = [0; HEADER_BYTES as usize];
    sortie.copy_from_slice(&h);
    sortie
}

/// Ce qu'une prise a produit.
#[derive(Debug, Clone, PartialEq)]
pub struct AudioTake {
    pub path: PathBuf,
    /// L'instant de son premier échantillon, s'il y en a eu un.
    pub started: Option<Instant>,
}

/// Le thread qui écrit le son d'une prise.
pub struct AudioRecording {
    stop: Arc<AtomicBool>,
    thread: JoinHandle<io::Result<AudioTake>>,
}

impl AudioRecording {
    /// Écrit dans `path` ce qui arrive sur `samples`, mono, à `sample_rate` Hz.
    pub fn start(samples: Consumer<f32>, sample_rate: u32, path: &Path) -> io::Result<Self> {
        let mut fichier = BufWriter::new(File::create(path)?);
        fichier.write_all(&wav_header(sample_rate, 0))?;
        let stop = Arc::new(AtomicBool::new(false));
        let arret = Arc::clone(&stop);
        let chemin = path.to_path_buf();
        let thread = std::thread::Builder::new()
            .name("son de la prise".into())
            .spawn(move || write_all(samples, sample_rate, fichier, chemin, &arret))?;
        Ok(Self { stop, thread })
    }

    /// Arrête la prise, ferme le fichier et le rend.
    pub fn stop(self) -> Result<AudioTake, String> {
        self.stop.store(true, Ordering::Relaxed);
        match self.thread.join() {
            Ok(fait) => fait.map_err(|e| e.to_string()),
            Err(_) => Err("le son de la prise s'est interrompu".to_string()),
        }
    }
}

fn write_all(
    mut samples: Consumer<f32>,
    sample_rate: u32,
    mut fichier: BufWriter<File>,
    path: PathBuf,
    stop: &AtomicBool,
) -> io::Result<AudioTake> {
    let mut ecrits = 0u64;
    let mut started = None;
    loop {
        // Lu avant l'anneau : ce qui y est déjà part encore dans le fichier.
        let fini = stop.load(Ordering::Relaxed);
        let disponibles = samples.slots();
        if disponibles > 0 {
            let bloc = samples
                .read_chunk(disponibles)
                .map_err(|e| io::Error::other(e.to_string()))?;
            if started.is_none() {
                // Le bloc a attendu sa durée dans l'anneau.
                let age = disponibles as f64 / sample_rate.max(1) as f64;
                started = Instant::now().checked_sub(Duration::from_secs_f64(age));
            }
            let (a, b) = bloc.as_slices();
            for s in a.iter().chain(b) {
                fichier.write_all(&s.to_le_bytes())?;
            }
            bloc.commit_all();
            ecrits += disponibles as u64;
        } else if fini {
            break;
        } else {
            std::thread::sleep(IDLE);
        }
    }
    fichier.seek(SeekFrom::Start(0))?;
    fichier.write_all(&wav_header(sample_rate, ecrits))?;
    fichier.flush()?;
    Ok(AudioTake { path, started })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rtrb::RingBuffer;

    fn le_u32(o: &[u8]) -> u32 {
        u32::from_le_bytes([o[0], o[1], o[2], o[3]])
    }

    #[test]
    fn l_en_tete_decrit_du_mono_flottant() {
        let h = wav_header(48_000, 100);
        assert_eq!(&h[..4], b"RIFF");
        assert_eq!(le_u32(&h[4..]), 436);
        assert_eq!(&h[8..16], b"WAVEfmt ");
        assert_eq!(u16::from_le_bytes([h[20], h[21]]), FLOAT_FORMAT);
        assert_eq!(u16::from_le_bytes([h[22], h[23]]), 1, "mono");
        assert_eq!(le_u32(&h[24..]), 48_000);
        assert_eq!(le_u32(&h[28..]), 192_000);
        assert_eq!(&h[36..40], b"data");
        assert_eq!(le_u32(&h[40..]), 400);
    }

    #[test]
    fn une_prise_tres_longue_ne_deborde_pas() {
        let h = wav_header(48_000, u64::MAX / 8);
        assert!(le_u32(&h[4..]) > 0);
    }

    #[test]
    fn la_prise_ecrit_tout_ce_qui_arrive_puis_corrige_son_en_tete() {
        let chemin = std::env::temp_dir().join(format!("frogenx-son-{}.wav", std::process::id()));
        let (mut entree, sortie) = RingBuffer::new(1024);
        let prise = AudioRecording::start(sortie, 48_000, &chemin).unwrap();
        for i in 0..500 {
            entree.push(i as f32).unwrap();
        }
        let fait = prise.stop().unwrap();
        assert!(fait.started.is_some());

        let octets = std::fs::read(&chemin).unwrap();
        let _ = std::fs::remove_file(&chemin);
        assert_eq!(octets.len(), 44 + 500 * 4);
        assert_eq!(&octets[..44], &wav_header(48_000, 500));
        let dernier = f32::from_le_bytes([octets[2040], octets[2041], octets[2042], octets[2043]]);
        assert_eq!(dernier, 499.0);
    }

    #[test]
    fn une_prise_silencieuse_n_a_pas_de_debut() {
        let chemin = std::env::temp_dir().join(format!("frogenx-muet-{}.wav", std::process::id()));
        let (_entree, sortie) = RingBuffer::<f32>::new(16);
        let prise = AudioRecording::start(sortie, 48_000, &chemin).unwrap();
        let fait = prise.stop().unwrap();
        let _ = std::fs::remove_file(&chemin);
        assert_eq!(fait.started, None);
    }
}
