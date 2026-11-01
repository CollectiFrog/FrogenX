//! Le BPM d'un flux — §7.1 de `docs/ARCHITECTURE.md`.
//!
//! Pur : il reçoit la force des transitoires, une valeur par analyse. Une
//! autocorrélation dit à quelles périodes le son se ressemble, puis un peigne
//! additionne ses valeurs à une, deux, trois et quatre périodes : un seul retard
//! ne vaudrait que 3 BPM de précision autour de 120.

/// Ce qu'on regarde du passé, en secondes.
const HISTORY: f32 = 12.0;
/// Le son qu'il faut avant de répondre.
const MIN_HISTORY: f32 = 4.0;
/// Une réestimation par seconde : le tempo d'un morceau ne bouge pas plus vite.
const ESTIMATE_EVERY: f32 = 1.0;
const MIN_BPM: f32 = 70.0;
const MAX_BPM: f32 = 180.0;
const GRID_STEP: f32 = 0.5;
/// Les multiples de la période que le peigne additionne.
const COMB: usize = 4;
/// Le centre de la préférence, là où se trouve l'essentiel de la musique.
const PREFERRED_BPM: f32 = 120.0;
/// Sa largeur, en octaves : elle départage un tempo double ou moitié, sans
/// écraser un tempo franc loin du centre.
const PREFERENCE_WIDTH: f32 = 1.0;
/// Sous cette fiabilité, il n'y a pas de rythme : mieux vaut ne rien dire.
const MIN_CONFIDENCE: f32 = 0.15;
/// Un écart plus petit suit le tempo ; plus grand, il doit se confirmer.
const SAME_TEMPO: f32 = 1.5;
/// La part d'une nouvelle mesure dans le tempo affiché.
const SMOOTHING: f32 = 0.4;
/// La force moyenne sous laquelle le signal est un silence.
const SILENCE: f32 = 1e-5;
/// De combien les coups doivent dépasser la moyenne. Un son tenu a une force
/// minuscule mais régulière, qui ressemblerait sinon à un rythme.
const MIN_CREST: f32 = 5.0;

/// Les battements repliés pour trouver le temps : assez pour qu'un coup isolé ne
/// décide pas, assez peu pour suivre un morceau qui bouge.
const PHASE_BEATS: usize = 8;
/// Le contraste du repli sous lequel le temps n'a pas de place franche.
const MIN_BEAT_CONTRAST: f32 = 0.4;

/// Le tempo détecté dans un flux de force de transitoires.
pub struct TempoTracker {
    /// Analyses par seconde.
    rate: f32,
    history: Vec<f32>,
    /// La même force, sur les graves seuls : c'est là que se lit le temps.
    bass: Vec<f32>,
    write: usize,
    filled: usize,
    since_estimate: usize,
    bpm: f32,
    confidence: f32,
    /// Un tempo très différent de `bpm`, en attente de confirmation.
    pending: Option<f32>,
    /// Analyses depuis le dernier temps.
    since_beat: f32,
    beat_confidence: f32,
}

impl TempoTracker {
    /// `rate` : le nombre de valeurs reçues par seconde.
    pub fn new(rate: f32) -> Self {
        Self {
            rate,
            history: vec![0.0; (HISTORY * rate).ceil() as usize],
            bass: vec![0.0; (HISTORY * rate).ceil() as usize],
            write: 0,
            filled: 0,
            since_estimate: 0,
            bpm: 0.0,
            confidence: 0.0,
            pending: None,
            since_beat: 0.0,
            beat_confidence: 0.0,
        }
    }

    /// Le tempo, `0` s'il n'y en a pas de fiable.
    pub fn bpm(&self) -> f32 {
        self.bpm
    }

    /// `[0, 1]`. `0` sans tempo.
    pub fn confidence(&self) -> f32 {
        self.confidence
    }

    /// La fiabilité de la phase, `[0, 1]`. `0` : le temps n'a pas de place franche.
    pub fn beat_confidence(&self) -> f32 {
        self.beat_confidence
    }

    /// Où on en est dans le battement, `lag` analyses plus tard, `[0, 1)`. La
    /// force d'un coup arrive après lui : `lag` rattrape ce retard.
    pub fn beat_phase(&self, lag: f32) -> f32 {
        if self.bpm <= 0.0 {
            return 0.0;
        }
        let periode = 60.0 * self.rate / self.bpm;
        ((self.since_beat + lag) / periode).rem_euclid(1.0)
    }

    pub fn push(&mut self, strength: f32) {
        self.push_with_bass(strength, 0.0);
    }

    /// `bass` : la force des graves seuls, qui place le temps quand elle est
    /// franche. Sinon, c'est `strength`.
    pub fn push_with_bass(&mut self, strength: f32, bass: f32) {
        let fini = |v: f32| if v.is_finite() { v } else { 0.0 };
        self.history[self.write] = fini(strength);
        self.bass[self.write] = fini(bass);
        self.write = (self.write + 1) % self.history.len();
        self.filled = (self.filled + 1).min(self.history.len());
        self.since_estimate += 1;
        self.since_beat += 1.0;
        if self.since_estimate as f32 >= ESTIMATE_EVERY * self.rate
            && self.filled as f32 >= MIN_HISTORY * self.rate
        {
            self.since_estimate = 0;
            self.estimate();
        }
    }

    /// La force, du plus ancien au plus récent, sans sa moyenne.
    fn envelope(&self) -> Option<Vec<f32>> {
        let n = self.filled;
        let debut = self.write + self.history.len() - n;
        let brut: Vec<f32> = (0..n)
            .map(|i| self.history[(debut + i) % self.history.len()])
            .collect();
        let moyenne = brut.iter().sum::<f32>() / n as f32;
        let plus_haut = brut.iter().copied().fold(0.0, f32::max);
        (moyenne >= SILENCE && plus_haut >= moyenne * MIN_CREST)
            .then(|| brut.iter().map(|v| v - moyenne).collect())
    }

    fn estimate(&mut self) {
        let Some(enveloppe) = self.envelope() else {
            self.forget();
            return;
        };
        let correlation = autocorrelation(&enveloppe, self.max_lag().min(enveloppe.len() - 1));
        if correlation[0] <= 0.0 {
            self.forget();
            return;
        }
        let peigne = |bpm: f32| comb(&correlation, 60.0 * self.rate / bpm);
        let note = |bpm: f32| peigne(bpm) * preference(bpm);

        let cases = ((MAX_BPM - MIN_BPM) / GRID_STEP) as usize;
        let tempo = |i: usize| MIN_BPM + i as f32 * GRID_STEP;
        let meilleur = (0..=cases)
            .max_by(|a, b| note(tempo(*a)).total_cmp(&note(tempo(*b))))
            .unwrap_or(0);
        let fiabilite = (peigne(tempo(meilleur)) / correlation[0]).clamp(0.0, 1.0);
        if fiabilite < MIN_CONFIDENCE {
            self.forget();
            return;
        }
        let bpm = refine(
            note(tempo(meilleur.saturating_sub(1))),
            note(tempo(meilleur)),
            note(tempo((meilleur + 1).min(cases))),
            tempo(meilleur),
        );
        self.report(bpm, fiabilite);
        self.locate_beat();
    }

    /// Replie les derniers battements de `force` sur la période : elle s'accumule
    /// là où tombe le temps. Rend sa place, en analyses, et le contraste du repli.
    fn fold(&self, force: &[f32], periode: f32) -> (usize, f32) {
        let n = force.len();
        let lire = |a: usize| force[(self.write + 2 * n - 1 - a % n) % n];
        let il_y_a = |age: f32| {
            let i = age as usize;
            lire(i) + (lire(i + 1) - lire(i)) * age.fract()
        };
        let plein = (self.filled as f32 / periode) as usize;
        let battements = PHASE_BEATS.min(plein.saturating_sub(1)).max(1);
        let notes: Vec<f32> = (0..periode.ceil() as usize)
            .map(|place| {
                (0..battements)
                    .map(|k| il_y_a(place as f32 + k as f32 * periode))
                    .sum()
            })
            .collect();
        let (mut meilleure, mut plus_haut) = (0, 0.0);
        for (i, note) in notes.iter().enumerate() {
            if *note > plus_haut {
                (meilleure, plus_haut) = (i, *note);
            }
        }
        let moyenne = notes.iter().sum::<f32>() / notes.len() as f32;
        let contraste = if plus_haut > 0.0 {
            1.0 - moyenne / plus_haut
        } else {
            0.0
        };
        (meilleure, contraste)
    }

    /// Les graves d'abord : un charleston à contretemps pèse plus lourd que la
    /// grosse caisse dans la force totale. Elle ne sert que s'ils se taisent.
    fn locate_beat(&mut self) {
        self.beat_confidence = 0.0;
        if self.bpm <= 0.0 {
            return;
        }
        let periode = 60.0 * self.rate / self.bpm;
        let graves = self.fold(&self.bass, periode);
        let (place, contraste) = if graves.1 >= MIN_BEAT_CONTRAST {
            graves
        } else {
            self.fold(&self.history, periode)
        };
        if contraste >= MIN_BEAT_CONTRAST {
            self.since_beat = place as f32;
            self.beat_confidence = contraste;
        }
    }

    /// Le plus long retard utile : quatre périodes du tempo le plus lent.
    fn max_lag(&self) -> usize {
        (COMB as f32 * 60.0 * self.rate / MIN_BPM).ceil() as usize + 1
    }

    /// Suit un tempo proche. Un saut doit se confirmer : un break ne doit pas
    /// faire changer tout le set de vitesse.
    fn report(&mut self, bpm: f32, confidence: f32) {
        self.confidence = confidence;
        if self.bpm == 0.0 {
            self.bpm = bpm;
        } else if (bpm - self.bpm).abs() <= SAME_TEMPO {
            self.bpm += (bpm - self.bpm) * SMOOTHING;
            self.pending = None;
        } else if self.pending.is_some_and(|p| (bpm - p).abs() <= SAME_TEMPO) {
            self.bpm = bpm;
            self.pending = None;
        } else {
            self.pending = Some(bpm);
        }
    }

    fn forget(&mut self) {
        self.bpm = 0.0;
        self.confidence = 0.0;
        self.pending = None;
        self.beat_confidence = 0.0;
    }
}

/// Corrélation de `signal` avec lui-même, de `0` à `max_lag`. Divisée par le
/// nombre de termes : un long retard ne doit pas perdre à cause de son recouvrement.
fn autocorrelation(signal: &[f32], max_lag: usize) -> Vec<f32> {
    (0..=max_lag)
        .map(|lag| {
            let termes = signal.len() - lag;
            let somme: f32 = signal[..termes]
                .iter()
                .zip(&signal[lag..])
                .map(|(a, b)| a * b)
                .sum();
            somme / termes as f32
        })
        .collect()
}

/// La moyenne de `correlation` aux multiples de `period`, retards fractionnaires
/// compris.
fn comb(correlation: &[f32], period: f32) -> f32 {
    let lire = |lag: f32| {
        let i = lag.floor() as usize;
        let a = correlation.get(i).copied().unwrap_or(0.0);
        let b = correlation.get(i + 1).copied().unwrap_or(0.0);
        a + (b - a) * lag.fract()
    };
    (1..=COMB).map(|k| lire(k as f32 * period)).sum::<f32>() / COMB as f32
}

fn preference(bpm: f32) -> f32 {
    let ecart = (bpm / PREFERRED_BPM).log2() / PREFERENCE_WIDTH;
    (-0.5 * ecart * ecart).exp()
}

/// Le sommet de la parabole passant par trois notes voisines de la grille.
fn refine(avant: f32, ici: f32, apres: f32, bpm: f32) -> f32 {
    let courbure = avant - 2.0 * ici + apres;
    if courbure >= 0.0 {
        return bpm;
    }
    bpm + (0.5 * (avant - apres) / courbure).clamp(-1.0, 1.0) * GRID_STEP
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f32 = 86.0;

    /// Une impulsion par battement, qui retombe en quelques analyses.
    fn impulsions(tracker: &mut TempoTracker, bpm: f32, secondes: f32) {
        let periode = 60.0 * RATE / bpm;
        let total = (secondes * RATE) as usize;
        for i in 0..total {
            let phase = (i as f32 % periode) / periode;
            tracker.push((1.0 - phase * 8.0).max(0.0));
        }
    }

    #[test]
    fn un_tempo_franc_se_trouve_a_moins_d_un_bpm() {
        for attendu in [90.0, 100.0, 120.0, 128.0, 140.0, 150.0] {
            let mut t = TempoTracker::new(RATE);
            impulsions(&mut t, attendu, 12.0);
            assert!((t.bpm() - attendu).abs() < 1.0, "{attendu} : {}", t.bpm());
            assert!(t.confidence() > 0.3, "{attendu} : {}", t.confidence());
        }
    }

    #[test]
    fn rien_avant_quatre_secondes() {
        let mut t = TempoTracker::new(RATE);
        impulsions(&mut t, 120.0, 3.5);
        assert_eq!(t.bpm(), 0.0);
        impulsions(&mut t, 120.0, 1.5);
        assert!(t.bpm() > 0.0);
    }

    #[test]
    fn un_silence_ne_donne_aucun_tempo() {
        let mut t = TempoTracker::new(RATE);
        for _ in 0..(10.0 * RATE) as usize {
            t.push(0.0);
        }
        assert_eq!(t.bpm(), 0.0);
        assert_eq!(t.confidence(), 0.0);
    }

    #[test]
    fn un_bruit_sans_rythme_ne_donne_aucun_tempo() {
        let mut t = TempoTracker::new(RATE);
        let mut x = 12345u32;
        for _ in 0..(12.0 * RATE) as usize {
            x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            t.push((x >> 16) as f32 / 65_536.0);
        }
        assert_eq!(t.bpm(), 0.0, "fiabilité {}", t.confidence());
    }

    #[test]
    fn le_tempo_s_oublie_quand_le_son_s_arrete() {
        let mut t = TempoTracker::new(RATE);
        impulsions(&mut t, 120.0, 12.0);
        assert!(t.bpm() > 0.0);
        for _ in 0..(13.0 * RATE) as usize {
            t.push(0.0);
        }
        assert_eq!(t.bpm(), 0.0);
    }

    #[test]
    fn un_changement_de_tempo_finit_par_passer() {
        let mut t = TempoTracker::new(RATE);
        impulsions(&mut t, 100.0, 12.0);
        impulsions(&mut t, 140.0, 13.0);
        assert!((t.bpm() - 140.0).abs() < 1.5, "{}", t.bpm());
    }

    #[test]
    fn un_saut_isole_ne_change_pas_le_tempo() {
        let mut t = TempoTracker::new(RATE);
        t.report(120.0, 0.5);
        t.report(95.0, 0.5);
        assert_eq!(t.bpm(), 120.0);
        t.report(120.0, 0.5);
        assert_eq!(t.pending, None);
        t.report(95.0, 0.5);
        t.report(95.2, 0.5);
        assert_eq!(t.bpm(), 95.2);
    }

    #[test]
    fn une_valeur_non_finie_compte_pour_rien() {
        let mut t = TempoTracker::new(RATE);
        for _ in 0..(6.0 * RATE) as usize {
            t.push(f32::NAN);
        }
        assert_eq!(t.bpm(), 0.0);
    }

    /// L'écart entre deux phases, le plus court, en battements.
    fn ecart(a: f32, b: f32) -> f32 {
        (a - b + 0.5).rem_euclid(1.0) - 0.5
    }

    #[test]
    fn le_temps_se_trouve_a_une_analyse_pres() {
        for attendu in [90.0, 120.0, 128.0, 150.0] {
            let mut t = TempoTracker::new(RATE);
            impulsions(&mut t, attendu, 12.0);
            // La dernière valeur reçue est la numéro `n - 1` ; la période vaut
            // `60 * RATE / bpm`.
            let periode = 60.0 * RATE / attendu;
            let n = (12.0 * RATE) as usize;
            let vraie = ((n - 1) as f32 % periode) / periode;
            let e = ecart(t.beat_phase(0.0), vraie).abs() * periode;
            assert!(e <= 2.0, "{attendu} : {e} analyses");
            assert!(t.beat_confidence() > 0.4, "{}", t.beat_confidence());
        }
    }

    #[test]
    fn les_graves_placent_le_temps_quand_le_contretemps_pese_plus_lourd() {
        let mut t = TempoTracker::new(RATE);
        let periode = 0.5 * RATE;
        let total = (12.0 * RATE) as usize;
        for i in 0..total {
            let phase = (i as f32 % periode) / periode;
            let temps = (1.0 - phase * 8.0).max(0.0);
            let contre = (1.0 - ((phase + 0.5) % 1.0) * 8.0).max(0.0);
            t.push_with_bass(temps + 3.0 * contre, temps);
        }
        let vraie = ((total - 1) as f32 % periode) / periode;
        assert!(ecart(t.beat_phase(0.0), vraie).abs() < 0.05);
    }

    #[test]
    fn la_phase_avance_entre_deux_estimations() {
        let mut t = TempoTracker::new(RATE);
        impulsions(&mut t, 120.0, 12.0);
        let avant = t.beat_phase(0.0);
        for _ in 0..10 {
            t.push(0.0);
        }
        let attendu = avant + 10.0 / (0.5 * RATE);
        assert!(ecart(t.beat_phase(0.0), attendu).abs() < 1e-3);
    }

    #[test]
    fn le_retard_d_un_coup_se_rattrape_en_analyses() {
        let mut t = TempoTracker::new(RATE);
        impulsions(&mut t, 120.0, 12.0);
        let attendu = t.beat_phase(0.0) + 4.3 / (0.5 * RATE);
        assert!(ecart(t.beat_phase(4.3), attendu).abs() < 1e-3);
    }

    #[test]
    fn sans_tempo_il_n_y_a_pas_de_phase() {
        let mut t = TempoTracker::new(RATE);
        impulsions(&mut t, 120.0, 12.0);
        for _ in 0..(13.0 * RATE) as usize {
            t.push(0.0);
        }
        assert_eq!(t.beat_phase(1.0), 0.0);
        assert_eq!(t.beat_confidence(), 0.0);
    }

    #[test]
    fn une_force_sans_place_franche_ne_donne_pas_de_phase() {
        let mut t = TempoTracker::new(RATE);
        t.bpm = 120.0;
        t.filled = t.history.len();
        t.history.iter_mut().for_each(|v| *v = 1.0);
        t.beat_confidence = 0.9;
        t.locate_beat();
        assert_eq!(t.beat_confidence(), 0.0);
    }

    #[test]
    fn la_preference_retombe_de_chaque_cote_de_son_centre() {
        assert_eq!(preference(120.0), 1.0);
        assert!(preference(90.0) < 1.0 && preference(160.0) < 1.0);
    }

    #[test]
    fn le_sommet_tombe_entre_deux_cases() {
        assert_eq!(refine(1.0, 2.0, 1.0, 120.0), 120.0);
        assert!(refine(1.5, 2.0, 1.0, 120.0) < 120.0);
        assert!(refine(1.0, 2.0, 1.5, 120.0) > 120.0);
        assert_eq!(refine(1.0, 1.0, 1.0, 120.0), 120.0, "pas de sommet");
    }
}
