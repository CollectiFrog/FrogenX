//! Le spectre d'une frame et les réglages de son analyse — §7 de
//! `docs/ARCHITECTURE.md`.
//!
//! Un nombre **fixe** de bandes, réparties selon une [`FreqScale`] : une taille
//! fixe voyage dans le triple buffer et dans `Ctx` sans jamais allouer. Le
//! spectre porte ses échelles avec lui : une zone en Hz se relit donc juste,
//! quelle que soit la répartition choisie. `core` ne sait pas d'où viennent ces
//! valeurs : un test les écrit à la main, et le rendu reste déterministe.

/// Nombre de bandes, de [`SPECTRUM_MIN_HZ`] à [`SPECTRUM_MAX_HZ`].
pub const SPECTRUM_BANDS: usize = 128;
pub const SPECTRUM_MIN_HZ: f32 = 20.0;
pub const SPECTRUM_MAX_HZ: f32 = 20_000.0;
/// Le niveau `0` en [`AmpScale::Decibel`], en dB. Le niveau `1` est une
/// sinusoïde à pleine échelle.
pub const FLOOR_DB: f32 = -72.0;
/// La durée d'analyse la plus courte, en secondes : quelques périodes d'aigu.
pub const MIN_WINDOW: f32 = 0.005;
/// La plus longue, en secondes : la mémoire que garde l'analyse.
pub const MAX_WINDOW: f32 = 1.0;
/// La durée par défaut : 2048 échantillons à 48 kHz.
pub const DEFAULT_WINDOW: f32 = 2048.0 / 48_000.0;

/// Comment l'axe des fréquences se découpe en bandes égales.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FreqScale {
    /// Autant de bandes par octave : la musique, et la lecture de Pro-Q.
    #[default]
    Log,
    /// L'oreille : linéaire dans les graves, logarithmique dans les aigus.
    Mel,
    /// Les bandes critiques de l'oreille, plus larges que Mel dans les graves.
    Bark,
    /// Autant de Hz par bande : la grille d'une FFT, les graves y tiennent peu.
    Linear,
}

impl FreqScale {
    pub const ALL: [FreqScale; 4] = [
        FreqScale::Log,
        FreqScale::Mel,
        FreqScale::Bark,
        FreqScale::Linear,
    ];

    /// Le mot du format de patch : stable, contrairement à [`Self::label`].
    pub fn word(self) -> &'static str {
        match self {
            FreqScale::Log => "log",
            FreqScale::Mel => "mel",
            FreqScale::Bark => "bark",
            FreqScale::Linear => "linear",
        }
    }

    pub fn from_word(word: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.word() == word)
    }

    pub fn label(self) -> &'static str {
        match self {
            FreqScale::Log => "Logarithmique",
            FreqScale::Mel => "Mel",
            FreqScale::Bark => "Bark",
            FreqScale::Linear => "Linéaire",
        }
    }

    fn warp(self, hz: f32) -> f32 {
        match self {
            FreqScale::Log => hz.ln(),
            FreqScale::Mel => 1127.0 * (1.0 + hz / 700.0).ln(),
            FreqScale::Bark => 26.81 * hz / (1960.0 + hz) - 0.53,
            FreqScale::Linear => hz,
        }
    }

    fn unwarp(self, v: f32) -> f32 {
        match self {
            FreqScale::Log => v.exp(),
            FreqScale::Mel => 700.0 * ((v / 1127.0).exp() - 1.0),
            FreqScale::Bark => 1960.0 * (v + 0.53) / (26.28 - v),
            FreqScale::Linear => v,
        }
    }

    /// La place d'une fréquence sur l'axe, dans `[0, 1]`. Hors de l'axe, elle
    /// rejoint le bord le plus proche ; `NaN` rejoint le bas (§18.4).
    pub fn position(self, hz: f32) -> f32 {
        if hz.is_nan() {
            return 0.0;
        }
        let (bas, haut) = (self.warp(SPECTRUM_MIN_HZ), self.warp(SPECTRUM_MAX_HZ));
        let hz = hz.clamp(SPECTRUM_MIN_HZ, SPECTRUM_MAX_HZ);
        ((self.warp(hz) - bas) / (haut - bas)).clamp(0.0, 1.0)
    }

    /// La fréquence à la place `position` de l'axe : l'inverse de
    /// [`FreqScale::position`]. Les bords sont exacts.
    pub fn frequency(self, position: f32) -> f32 {
        if position.is_nan() || position <= 0.0 {
            return SPECTRUM_MIN_HZ;
        }
        if position >= 1.0 {
            return SPECTRUM_MAX_HZ;
        }
        let (bas, haut) = (self.warp(SPECTRUM_MIN_HZ), self.warp(SPECTRUM_MAX_HZ));
        self.unwarp(bas + position * (haut - bas))
    }
}

/// Comment une magnitude devient un niveau `[0, 1]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AmpScale {
    /// Linéaire en dB : l'échelle de l'oreille, de [`FLOOR_DB`] à 0 dB.
    #[default]
    Decibel,
    /// La magnitude elle-même : seuls les sons forts émergent.
    Linear,
}

impl AmpScale {
    pub const ALL: [AmpScale; 2] = [AmpScale::Decibel, AmpScale::Linear];

    /// Le mot du format de patch : stable, contrairement à [`Self::label`].
    pub fn word(self) -> &'static str {
        match self {
            AmpScale::Decibel => "db",
            AmpScale::Linear => "linear",
        }
    }

    pub fn from_word(word: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.word() == word)
    }

    pub fn label(self) -> &'static str {
        match self {
            AmpScale::Decibel => "dB",
            AmpScale::Linear => "Linéaire",
        }
    }

    /// Une magnitude, 1 à pleine échelle, en niveau `[0, 1]`.
    pub fn level(self, magnitude: f32) -> f32 {
        match self {
            AmpScale::Decibel => {
                let db = 20.0 * magnitude.max(1e-9).log10();
                (1.0 - db / FLOOR_DB).clamp(0.0, 1.0)
            }
            AmpScale::Linear => magnitude.clamp(0.0, 1.0),
        }
    }
}

/// Comment une bande mesure ce qu'elle contient.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AnalysisMethod {
    /// La case de FFT la plus haute de la bande : les notes ressortent.
    #[default]
    Peak,
    /// L'énergie de toute la bande : le bruit et les charleys pèsent leur poids.
    Energy,
    /// Une fenêtre par bande, d'autant plus courte que la bande est large : les
    /// aigus réagissent vite, les graves restent séparés.
    ConstantQ,
}

impl AnalysisMethod {
    pub const ALL: [AnalysisMethod; 3] = [
        AnalysisMethod::Peak,
        AnalysisMethod::Energy,
        AnalysisMethod::ConstantQ,
    ];

    /// Le mot du format de patch : stable, contrairement à [`Self::label`].
    pub fn word(self) -> &'static str {
        match self {
            AnalysisMethod::Peak => "peak",
            AnalysisMethod::Energy => "energy",
            AnalysisMethod::ConstantQ => "constant_q",
        }
    }

    pub fn from_word(word: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.word() == word)
    }

    pub fn label(self) -> &'static str {
        match self {
            AnalysisMethod::Peak => "Crête FFT",
            AnalysisMethod::Energy => "Énergie FFT",
            AnalysisMethod::ConstantQ => "Multi-résolution",
        }
    }
}

/// Une durée d'analyse utilisable, en secondes, chez son consommateur (§18.4).
pub fn sanitize_window(seconds: f32) -> f32 {
    if seconds.is_nan() {
        return DEFAULT_WINDOW;
    }
    seconds.clamp(MIN_WINDOW, MAX_WINDOW)
}

/// Ce que l'analyse doit faire, résolu pour la frame. Cette version traverse les
/// threads : le réglable, avec son `Param`, est `composition::AudioAnalysis`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnalysisSettings {
    pub method: AnalysisMethod,
    pub freq_scale: FreqScale,
    pub amp_scale: AmpScale,
    /// La durée de signal analysée, en secondes.
    pub window: f32,
}

impl Default for AnalysisSettings {
    fn default() -> Self {
        Self {
            method: AnalysisMethod::default(),
            freq_scale: FreqScale::default(),
            amp_scale: AmpScale::default(),
            window: DEFAULT_WINDOW,
        }
    }
}

/// Le niveau de chaque bande, unipolaire `[0, 1]` (§17), et les échelles qui
/// permettent de le relire.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Spectrum {
    pub levels: [f32; SPECTRUM_BANDS],
    pub freq_scale: FreqScale,
    pub amp_scale: AmpScale,
}

impl Default for Spectrum {
    fn default() -> Self {
        Self::flat(0.0)
    }
}

impl Spectrum {
    /// Toutes les bandes au même niveau.
    pub fn flat(level: f32) -> Self {
        Self {
            levels: [level; SPECTRUM_BANDS],
            freq_scale: FreqScale::default(),
            amp_scale: AmpScale::default(),
        }
    }

    /// La bande qui contient une fréquence.
    pub fn band(&self, hz: f32) -> usize {
        let i = self.freq_scale.position(hz) * SPECTRUM_BANDS as f32;
        (i as usize).min(SPECTRUM_BANDS - 1)
    }

    /// Le niveau le plus haut entre deux fréquences, données dans n'importe
    /// quel ordre. Une plage plus étroite qu'une bande en lit une.
    pub fn peak(&self, a_hz: f32, b_hz: f32) -> f32 {
        let (a, b) = (
            self.freq_scale.position(a_hz),
            self.freq_scale.position(b_hz),
        );
        let n = SPECTRUM_BANDS as f32;
        let debut = (a.min(b) * n).floor() as usize;
        let fin = ((a.max(b) * n).ceil() as usize).min(SPECTRUM_BANDS);
        let debut = debut.min(SPECTRUM_BANDS - 1);
        self.levels[debut..fin.max(debut + 1)]
            .iter()
            .copied()
            .fold(0.0, f32::max)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chaque_echelle_a_sa_position_et_sa_frequence_inverses() {
        for scale in FreqScale::ALL {
            for hz in [20.0, 60.0, 440.0, 1000.0, 12_000.0, 20_000.0] {
                let retour = scale.frequency(scale.position(hz));
                assert!((retour - hz).abs() / hz < 1e-2, "{scale:?} {hz} → {retour}");
            }
        }
    }

    #[test]
    fn chaque_echelle_couvre_tout_l_axe_et_croit() {
        for scale in FreqScale::ALL {
            assert_eq!(scale.position(SPECTRUM_MIN_HZ), 0.0);
            assert_eq!(scale.position(SPECTRUM_MAX_HZ), 1.0);
            assert_eq!(scale.frequency(0.0), SPECTRUM_MIN_HZ);
            assert_eq!(scale.frequency(1.0), SPECTRUM_MAX_HZ);
            let mut precedent = 0.0;
            for i in 1..50 {
                let f = scale.frequency(i as f32 / 50.0);
                assert!(f > precedent, "{scale:?} doit croître");
                precedent = f;
            }
        }
    }

    #[test]
    fn l_echelle_log_donne_la_meme_largeur_a_chaque_decade() {
        let p = |hz| FreqScale::Log.position(hz);
        let (basse, haute) = (p(200.0) - p(20.0), p(2000.0) - p(200.0));
        assert!((basse - haute).abs() < 1e-5, "{basse} et {haute}");
    }

    #[test]
    fn l_echelle_lineaire_donne_la_meme_largeur_a_chaque_hz() {
        let p = |hz| FreqScale::Linear.position(hz);
        let (basse, haute) = (p(3000.0) - p(1000.0), p(12_000.0) - p(10_000.0));
        assert!((basse - haute).abs() < 1e-5, "{basse} et {haute}");
    }

    #[test]
    fn mel_et_bark_donnent_plus_de_place_aux_graves_que_le_lineaire() {
        // Et moins que le log, qui en est l'extrême.
        let sous_1k = |s: FreqScale| s.position(1000.0);
        assert!(sous_1k(FreqScale::Linear) < sous_1k(FreqScale::Mel));
        assert!(sous_1k(FreqScale::Mel) < sous_1k(FreqScale::Log));
        assert!(sous_1k(FreqScale::Linear) < sous_1k(FreqScale::Bark));
    }

    #[test]
    fn une_frequence_hors_axe_rejoint_le_bord() {
        for scale in FreqScale::ALL {
            assert_eq!(scale.position(1.0), 0.0);
            assert_eq!(scale.position(-5.0), 0.0);
            assert_eq!(scale.position(f32::NAN), 0.0);
            assert_eq!(scale.position(96_000.0), 1.0);
            assert_eq!(scale.frequency(f32::NAN), SPECTRUM_MIN_HZ);
        }
    }

    #[test]
    fn l_amplitude_se_lit_en_decibels_ou_telle_quelle() {
        assert_eq!(AmpScale::Decibel.level(1.0), 1.0);
        assert!((AmpScale::Decibel.level(0.5) - (1.0 - 6.02 / 72.0)).abs() < 1e-3);
        assert_eq!(AmpScale::Decibel.level(0.0), 0.0);
        assert_eq!(AmpScale::Linear.level(0.5), 0.5);
        for scale in AmpScale::ALL {
            assert_eq!(scale.level(4.0), 1.0, "au-dessus de la pleine échelle");
        }
    }

    #[test]
    fn une_duree_d_analyse_est_saturee_chez_le_consommateur() {
        assert_eq!(sanitize_window(0.0), MIN_WINDOW);
        assert_eq!(sanitize_window(90.0), MAX_WINDOW);
        assert_eq!(sanitize_window(0.1), 0.1);
        assert_eq!(sanitize_window(f32::NAN), DEFAULT_WINDOW);
    }

    #[test]
    fn chaque_choix_a_un_mot_stable() {
        for s in FreqScale::ALL {
            assert_eq!(FreqScale::from_word(s.word()), Some(s));
        }
        for s in AmpScale::ALL {
            assert_eq!(AmpScale::from_word(s.word()), Some(s));
        }
        for m in AnalysisMethod::ALL {
            assert_eq!(AnalysisMethod::from_word(m.word()), Some(m));
        }
        assert_eq!(FreqScale::from_word("cubique"), None);
    }

    #[test]
    fn chaque_choix_a_un_nom() {
        assert!(FreqScale::ALL.iter().all(|s| !s.label().is_empty()));
        assert!(AmpScale::ALL.iter().all(|s| !s.label().is_empty()));
        assert!(AnalysisMethod::ALL.iter().all(|m| !m.label().is_empty()));
    }

    fn un_seul_pic(hz: f32, niveau: f32) -> Spectrum {
        let mut s = Spectrum::default();
        s.levels[s.band(hz)] = niveau;
        s
    }

    #[test]
    fn le_pic_ne_voit_que_sa_plage() {
        let s = un_seul_pic(1000.0, 0.8);
        assert_eq!(s.peak(500.0, 2000.0), 0.8);
        assert_eq!(s.peak(2000.0, 500.0), 0.8, "bornes inversées");
        assert_eq!(s.peak(20.0, 100.0), 0.0);
        assert_eq!(s.peak(5000.0, 20_000.0), 0.0);
    }

    #[test]
    fn le_pic_suit_l_echelle_du_spectre() {
        // La même plage en Hz, sur un autre découpage : elle reste juste.
        let mut s = Spectrum {
            freq_scale: FreqScale::Linear,
            ..Spectrum::default()
        };
        s.levels[s.band(1000.0)] = 0.8;
        assert_eq!(s.peak(900.0, 1100.0), 0.8);
        assert_eq!(s.peak(5000.0, 20_000.0), 0.0);
    }

    #[test]
    fn une_plage_nulle_lit_une_bande_sans_paniquer() {
        let s = un_seul_pic(1000.0, 0.8);
        assert_eq!(s.peak(1000.0, 1000.0), 0.8);
        // Les deux bords de l'axe, où un découpage naïf sort du tableau.
        assert_eq!(un_seul_pic(20_000.0, 0.5).peak(20_000.0, 20_000.0), 0.5);
        assert_eq!(un_seul_pic(20.0, 0.5).peak(20.0, 20.0), 0.5);
        assert_eq!(s.peak(f32::NAN, f32::INFINITY), 0.8, "toute la plage");
    }
}
