//! Une séquence d'images PNG — §8.2 de `docs/ARCHITECTURE.md`.

use crate::sink::{CpuFrame, CpuSink, SinkStatus};
use frogenx_render::write_png_rgba8;
use std::io;
use std::path::{Path, PathBuf};

/// Écrit une image par frame, nommée d'après son `index`.
///
/// Le rendu offline déterministe d'ici l'enregistreur de l'étape 10 : `ffmpeg`
/// assemble ensuite la séquence (skill `poc-video`).
pub struct PngSequenceSink {
    dir: PathBuf,
    failure: Option<String>,
}

impl PngSequenceSink {
    /// Crée le dossier. Un échec ici est de l'édition : il se dit tout de
    /// suite, pas après une heure de rendu (§18.1).
    pub fn open(dir: impl Into<PathBuf>) -> io::Result<Self> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir)?;
        Ok(Self { dir, failure: None })
    }

    /// Le fichier d'une frame. Six chiffres : plus de 4 h 30 à 60 fps, et
    /// l'ordre alphabétique reste celui des frames.
    pub fn path_for(dir: &Path, index: u64) -> PathBuf {
        dir.join(format!("frame_{index:06}.png"))
    }
}

impl CpuSink for PngSequenceSink {
    fn submit(&mut self, frame: &CpuFrame) {
        let chemin = Self::path_for(&self.dir, frame.index);
        if let Err(e) = write_png_rgba8(&chemin, frame.width, frame.height, &frame.pixels) {
            self.failure = Some(format!("{} : {e}", chemin.display()));
        }
    }

    fn status(&self) -> SinkStatus {
        match &self.failure {
            Some(message) => SinkStatus::Failed(message.clone()),
            None => SinkStatus::Active,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::sync::Arc;

    /// Un dossier temporaire propre à ce test, retiré à la fin.
    struct Dossier(PathBuf);

    impl Dossier {
        fn new(nom: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("frogenx-{nom}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            Self(dir)
        }
    }

    impl Drop for Dossier {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn frame(index: u64, pixels: Vec<u8>, width: u32, height: u32) -> CpuFrame {
        CpuFrame {
            pixels: Arc::from(pixels),
            width,
            height,
            index,
            t: 0.0,
        }
    }

    fn lire(chemin: &Path) -> (u32, u32, Vec<u8>) {
        let mut lecteur = png::Decoder::new(std::io::BufReader::new(File::open(chemin).unwrap()))
            .read_info()
            .unwrap();
        let mut octets = vec![0; lecteur.output_buffer_size().unwrap()];
        let info = lecteur.next_frame(&mut octets).unwrap();
        octets.truncate(info.buffer_size());
        (info.width, info.height, octets)
    }

    #[test]
    fn les_noms_suivent_l_index_et_l_ordre_alphabetique() {
        let dir = Path::new("x");
        assert_eq!(
            PngSequenceSink::path_for(dir, 7),
            dir.join("frame_000007.png")
        );
        let (a, b) = (
            PngSequenceSink::path_for(dir, 9),
            PngSequenceSink::path_for(dir, 10),
        );
        assert!(a < b, "{a:?} < {b:?}");
    }

    #[test]
    fn une_frame_est_ecrite_a_l_identique() {
        let dossier = Dossier::new("png-identique");
        let mut sink = PngSequenceSink::open(&dossier.0).unwrap();
        let pixels = vec![255, 0, 0, 255, 0, 128, 0, 64];
        sink.submit(&frame(3, pixels.clone(), 2, 1));

        assert_eq!(sink.status(), SinkStatus::Active);
        let (w, h, octets) = lire(&PngSequenceSink::path_for(&dossier.0, 3));
        assert_eq!((w, h), (2, 1));
        assert_eq!(octets, pixels);
    }

    #[test]
    fn ouvrir_un_chemin_impossible_est_une_erreur_d_edition() {
        let dossier = Dossier::new("png-fichier");
        std::fs::create_dir_all(&dossier.0).unwrap();
        let fichier = dossier.0.join("pas-un-dossier");
        std::fs::write(&fichier, b"x").unwrap();
        assert!(PngSequenceSink::open(fichier.join("sous")).is_err());
    }

    #[test]
    fn une_ecriture_impossible_passe_en_echec_sans_paniquer() {
        let dossier = Dossier::new("png-echec");
        let mut sink = PngSequenceSink::open(&dossier.0).unwrap();
        std::fs::remove_dir_all(&dossier.0).unwrap();
        sink.submit(&frame(0, vec![0; 4], 1, 1));
        assert!(
            matches!(sink.status(), SinkStatus::Failed(_)),
            "got {:?}",
            sink.status()
        );
    }

    #[test]
    fn des_pixels_de_taille_incoherente_passent_en_echec() {
        let dossier = Dossier::new("png-taille");
        let mut sink = PngSequenceSink::open(&dossier.0).unwrap();
        sink.submit(&frame(0, vec![0; 3], 1, 1));
        assert!(matches!(sink.status(), SinkStatus::Failed(_)));
    }
}
