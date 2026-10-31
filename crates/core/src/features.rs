//! Nommer une feature audio — §7.1 de `docs/ARCHITECTURE.md`.
//!
//! Pure donnée : `osc` en fait un `Signal`, une commande la désigne, un fichier
//! l'écrit par son mot.

use crate::signal::AudioFeatures;

/// Les notes de [`AudioFeatures::chroma`], do en tête.
pub const CHROMA_NOTES: usize = 12;

const NOTE_LABELS: [&str; CHROMA_NOTES] = [
    "Do", "Do♯", "Ré", "Ré♯", "Mi", "Fa", "Fa♯", "Sol", "Sol♯", "La", "La♯", "Si",
];
const NOTE_WORDS: [&str; CHROMA_NOTES] = [
    "chroma_0",
    "chroma_1",
    "chroma_2",
    "chroma_3",
    "chroma_4",
    "chroma_5",
    "chroma_6",
    "chroma_7",
    "chroma_8",
    "chroma_9",
    "chroma_10",
    "chroma_11",
];

/// Ce qu'on lit dans [`AudioFeatures`], toujours dans `[0, 1]` (§17).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FeatureKind {
    #[default]
    Rms,
    Bass,
    Mid,
    Treble,
    Centroid,
    Onset,
    /// Une note, de 0 (do) à 11 (si). Au-delà, elle vaut 0 (§18.4).
    Chroma(u8),
}

impl FeatureKind {
    /// Ce que propose un menu. Les notes n'y figurent pas toutes : une seule
    /// suffit à colorer, et douze lignes noieraient les autres.
    pub const MENU: [FeatureKind; 6] = [
        FeatureKind::Rms,
        FeatureKind::Bass,
        FeatureKind::Mid,
        FeatureKind::Treble,
        FeatureKind::Centroid,
        FeatureKind::Onset,
    ];

    /// Sa valeur dans `audio`.
    pub fn read(self, audio: &AudioFeatures) -> f32 {
        match self {
            Self::Rms => audio.rms,
            Self::Bass => audio.bands[0],
            Self::Mid => audio.bands[1],
            Self::Treble => audio.bands[2],
            Self::Centroid => audio.centroid,
            Self::Onset => audio.onset,
            Self::Chroma(note) => audio.chroma.get(note as usize).copied().unwrap_or(0.0),
        }
    }

    /// Son nom à l'écran.
    pub fn label(self) -> &'static str {
        match self {
            Self::Rms => "Volume",
            Self::Bass => "Graves",
            Self::Mid => "Médiums",
            Self::Treble => "Aigus",
            Self::Centroid => "Brillance",
            Self::Onset => "Transitoires",
            Self::Chroma(note) => NOTE_LABELS.get(note as usize).copied().unwrap_or("?"),
        }
    }

    /// Son nom dans un fichier (§19.1).
    pub fn word(self) -> &'static str {
        match self {
            Self::Rms => "rms",
            Self::Bass => "bass",
            Self::Mid => "mid",
            Self::Treble => "treble",
            Self::Centroid => "centroid",
            Self::Onset => "onset",
            Self::Chroma(note) => NOTE_WORDS.get(note as usize).copied().unwrap_or("chroma_0"),
        }
    }

    pub fn from_word(word: &str) -> Option<Self> {
        match word {
            "rms" => Some(Self::Rms),
            "bass" => Some(Self::Bass),
            "mid" => Some(Self::Mid),
            "treble" => Some(Self::Treble),
            "centroid" => Some(Self::Centroid),
            "onset" => Some(Self::Onset),
            autre => NOTE_WORDS
                .iter()
                .position(|w| *w == autre)
                .map(|note| Self::Chroma(note as u8)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn audio() -> AudioFeatures {
        let mut chroma = [0.0; CHROMA_NOTES];
        chroma[9] = 0.9;
        AudioFeatures {
            rms: 0.1,
            bands: [0.2, 0.3, 0.4],
            centroid: 0.5,
            onset: 0.6,
            chroma,
            ..Default::default()
        }
    }

    #[test]
    fn chaque_feature_lit_son_champ() {
        let a = audio();
        let attendu = [0.1, 0.2, 0.3, 0.4, 0.5, 0.6];
        for (kind, valeur) in FeatureKind::MENU.iter().zip(attendu) {
            assert_eq!(kind.read(&a), valeur, "{kind:?}");
        }
        assert_eq!(FeatureKind::Chroma(9).read(&a), 0.9);
    }

    #[test]
    fn une_note_hors_de_la_gamme_vaut_zero_sans_paniquer() {
        assert_eq!(FeatureKind::Chroma(12).read(&audio()), 0.0);
        assert_eq!(FeatureKind::Chroma(200).label(), "?");
    }

    #[test]
    fn chaque_mot_se_relit() {
        let notes = (0..CHROMA_NOTES as u8).map(FeatureKind::Chroma);
        for kind in FeatureKind::MENU.into_iter().chain(notes) {
            assert_eq!(FeatureKind::from_word(kind.word()), Some(kind));
        }
        assert_eq!(FeatureKind::from_word("magie"), None);
        assert_eq!(FeatureKind::from_word("chroma_12"), None);
    }
}
