//! La grille des temps et la quantification — §3bis.3 de `docs/ARCHITECTURE.md`.
//!
//! Le tempo dit la durée d'un battement, la grille dit où il tombe. Un recalage
//! écrit `reference` d'un coup, et ce qui joue la rejoint par une poussée du
//! tempo : aucune position n'est réécrite, donc rien ne saute à l'écran.

use crate::hierarchy::Composition;
use crate::ids::{ClipId, ColumnId, LayerId};

/// La poussée la plus forte, en part du tempo.
const MAX_PUSH: f64 = 0.12;
/// Le temps de rattrapage, en battements, une fois sous la poussée maximale.
const CATCH_BEATS: f64 = 1.0;
const BAR_BEATS: f64 = 4.0;

/// Ce qu'un lancement attend (§3bis.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Quantize {
    /// Le clip part au clic.
    #[default]
    Off,
    Beat,
    Bar,
}

impl Quantize {
    pub const ALL: [Quantize; 3] = [Quantize::Off, Quantize::Beat, Quantize::Bar];

    /// Son nom dans le format de fichier : une traduction de l'interface ne doit
    /// pas casser les fichiers.
    pub fn word(self) -> &'static str {
        match self {
            Quantize::Off => "off",
            Quantize::Beat => "beat",
            Quantize::Bar => "bar",
        }
    }

    pub fn from_word(word: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|q| q.word() == word)
    }

    pub fn label(self) -> &'static str {
        match self {
            Quantize::Off => "Aucune",
            Quantize::Beat => "1 temps",
            Quantize::Bar => "4 temps",
        }
    }

    /// L'écart entre deux lancements possibles, en battements.
    fn beats(self) -> Option<f64> {
        match self {
            Quantize::Off => None,
            Quantize::Beat => Some(1.0),
            Quantize::Bar => Some(BAR_BEATS),
        }
    }
}

/// Où tombe le temps, et où en est ce qui joue, en battements depuis le dernier
/// premier temps.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct BeatGrid {
    reference: f64,
    played: f64,
    /// Un geste a posé le temps : le son ne le déplace plus.
    by_hand: bool,
}

impl BeatGrid {
    /// La position de ce qui joue, celle de la pastille et des lancements.
    pub fn played(&self) -> f64 {
        self.played
    }

    /// Ce qu'il reste à rattraper, le plus court chemin : `[-0,5, 0,5)` battement.
    pub fn error(&self) -> f64 {
        (self.reference - self.played + 0.5).rem_euclid(1.0) - 0.5
    }

    /// Avance d'une frame, et rend le tempo que doivent voir les clips.
    pub fn advance(&mut self, dt: f64, bpm: f32) -> f32 {
        let poussee = (self.error() / CATCH_BEATS).clamp(-MAX_PUSH, MAX_PUSH);
        let battements = dt * bpm as f64 / 60.0;
        self.reference += battements;
        self.played += battements * (1.0 + poussee);
        bpm * (1.0 + poussee) as f32
    }

    /// Cet instant est le premier temps de la mesure. Le compte repart de zéro
    /// sur les deux positions, qui gardent leur écart.
    pub fn first_beat(&mut self) {
        self.played -= self.played.round();
        self.reference = 0.0;
        self.by_hand = true;
    }

    /// Cet instant est un temps, le plus proche : le compte de la mesure reste.
    pub fn snap(&mut self) {
        self.reference = self.reference.round();
        self.by_hand = true;
    }

    /// Le son dit où on en est dans le battement. Il ne connaît pas la mesure :
    /// seule la place du temps le plus proche bouge.
    pub fn follow(&mut self, phase: f32) {
        if !self.by_hand {
            let phase = phase as f64;
            self.reference = (self.reference - phase).round() + phase;
        }
    }

    /// Le son reprend la main sur la phase.
    pub fn listen(&mut self) {
        self.by_hand = false;
    }
}

/// `played` a-t-il franchi un multiple de `every` ?
fn crossed(before: f64, now: f64, every: f64) -> bool {
    (before / every).floor() < (now / every).floor()
}

/// Un geste de lancement, en attente de son temps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Launch {
    Clip { layer: LayerId, clip: ClipId },
    Column(ColumnId),
}

impl Composition {
    pub fn beat(&self) -> &BeatGrid {
        &self.beat
    }

    fn fire(&mut self, launch: Launch) -> bool {
        match launch {
            Launch::Clip { layer, clip } => self.trigger(layer, clip),
            Launch::Column(column) => {
                let mut layers = Vec::new();
                self.layers_in_order(&mut layers);
                self.trigger_column(column, &layers)
            }
        }
    }

    /// Lance tout de suite, ou range le lancement jusqu'à son temps. Le dernier
    /// geste gagne : sur sa piste, et sur toutes pour une colonne.
    pub(crate) fn launch(&mut self, launch: Launch) -> bool {
        if self.quantize == Quantize::Off {
            return self.fire(launch);
        }
        match launch {
            Launch::Clip { layer, clip } => {
                if !self.layer(layer).is_some_and(|l| l.holds(clip)) {
                    return false;
                }
                self.waiting
                    .retain(|l| !matches!(l, Launch::Clip { layer: autre, .. } if *autre == layer));
            }
            Launch::Column(column) => {
                if !self.columns().contains(&column) {
                    return false;
                }
                self.waiting.clear();
            }
        }
        self.waiting.push(launch);
        true
    }

    /// Fait avancer la grille, lance ce qui attendait ce temps, et rend le tempo
    /// que doivent voir les clips. À appeler avant leur évaluation.
    pub fn advance_beat(&mut self, dt: f64, bpm: f32) -> f32 {
        let avant = self.beat.played();
        let tempo = self.beat.advance(dt, bpm);
        let arrive = match self.quantize.beats() {
            Some(every) => crossed(avant, self.beat.played(), every),
            None => true,
        };
        if arrive {
            for launch in std::mem::take(&mut self.waiting) {
                self.fire(launch);
            }
        }
        tempo
    }

    /// Ce clip attend-il son temps ?
    pub fn is_waiting(&self, layer: LayerId, clip: ClipId) -> bool {
        self.waiting.iter().any(|launch| match *launch {
            Launch::Clip { layer: l, clip: c } => l == layer && c == clip,
            Launch::Column(column) => {
                let rang = self.columns().iter().position(|c| *c == column);
                let case = rang.and_then(|i| self.layer(layer)?.cell(i));
                case == Some(clip)
            }
        })
    }

    /// Une piste qu'on vide n'attend plus rien.
    pub(crate) fn cancel_launch(&mut self, layer: LayerId) {
        self.waiting
            .retain(|l| !matches!(l, Launch::Clip { layer: autre, .. } if *autre == layer));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clip::Clip;
    use crate::command::Command;
    use crate::transport::{PlayMode, Transport};
    use frogenx_core::{AudioFeatures, Graph, Param};
    use frogenx_shape::Ellipse;

    const FRAME: f64 = 1.0 / 60.0;

    fn tourne(g: &mut BeatGrid, secondes: f64, bpm: f32) {
        for _ in 0..(secondes / FRAME).round() as usize {
            g.advance(FRAME, bpm);
        }
    }

    fn clip(nom: &str) -> Clip {
        let mut graph = Graph::new();
        let transport = Transport::bpm_sync(&mut graph, Param::fixed(4.0), PlayMode::Loop);
        let mut clip = Clip::shape(graph, Box::new(Ellipse::circle(0.5)), transport);
        clip.name = nom.to_string();
        clip
    }

    /// Deux pistes, deux colonnes, chaque case remplie.
    struct Grille {
        c: Composition,
        pistes: [LayerId; 2],
        colonnes: [ColumnId; 2],
        cases: [[ClipId; 2]; 2],
    }

    fn grille() -> Grille {
        let mut c = Composition::new();
        let deck = c.add_deck("A");
        let groupe = c.add_layer_group(deck).unwrap();
        let pistes = [c.add_layer(groupe).unwrap(), c.add_layer(groupe).unwrap()];
        let colonnes = [c.add_column(), c.add_column()];
        let mut cases = Vec::new();
        for p in pistes {
            let mut ligne = Vec::new();
            for k in colonnes {
                ligne.push(c.place_clip(p, k, clip("c")).unwrap());
            }
            cases.push([ligne[0], ligne[1]]);
        }
        Grille {
            c,
            pistes,
            colonnes,
            cases: [cases[0], cases[1]],
        }
    }

    fn actif(g: &Grille, piste: usize) -> Option<ClipId> {
        g.c.layer(g.pistes[piste]).and_then(|l| l.active_clip())
    }

    fn lance(g: &mut Grille, piste: usize, colonne: usize) -> bool {
        let commande = Command::LaunchClip {
            layer: g.pistes[piste],
            clip: g.cases[piste][colonne],
        };
        g.c.apply(commande, 0.0)
    }

    #[test]
    fn sans_recalage_rien_n_est_pousse() {
        let mut g = BeatGrid::default();
        assert_eq!(g.advance(FRAME, 120.0), 120.0);
        tourne(&mut g, 10.0, 120.0);
        assert_eq!(g.error(), 0.0);
        assert!((g.played() - (20.0 + FRAME * 2.0)).abs() < 1e-6);
    }

    #[test]
    fn l_ecart_prend_le_plus_court_chemin() {
        let mut g = BeatGrid {
            reference: 0.9,
            ..Default::default()
        };
        assert!((g.error() + 0.1).abs() < 1e-9, "{}", g.error());
        g.reference = 3.1;
        assert!((g.error() - 0.1).abs() < 1e-9, "{}", g.error());
    }

    #[test]
    fn un_quart_de_temps_se_rattrape_en_deux_secondes_sans_saut() {
        let mut g = BeatGrid {
            reference: 0.25,
            ..Default::default()
        };
        let mut avant = g.played();
        for _ in 0..120 {
            let tempo = g.advance(FRAME, 120.0);
            assert!((105.5..=134.5).contains(&tempo), "jamais plus de 12 %");
            let pas = g.played() - avant;
            assert!(pas > 0.0 && pas < 0.04, "aucun saut : {pas}");
            avant = g.played();
        }
        assert!(g.error().abs() < 0.03, "{}", g.error());
    }

    #[test]
    fn le_tempo_vu_par_les_clips_revient_au_tempo_affiche() {
        let mut g = BeatGrid {
            reference: 0.25,
            ..Default::default()
        };
        tourne(&mut g, 10.0, 120.0);
        assert!((g.advance(FRAME, 120.0) - 120.0).abs() < 0.01);
    }

    #[test]
    fn le_premier_temps_repart_de_zero_en_gardant_l_ecart() {
        let mut g = BeatGrid::default();
        tourne(&mut g, 2.6, 120.0);
        let avant = g.played();
        g.first_beat();
        assert_eq!(g.reference, 0.0);
        assert!((g.played() - (avant - avant.round())).abs() < 1e-9);
        assert!((g.error() + g.played()).abs() < 1e-9, "rien n'a sauté");
    }

    #[test]
    fn snap_garde_le_compte_de_la_mesure() {
        let mut g = BeatGrid::default();
        tourne(&mut g, 3.1, 120.0);
        g.snap();
        assert_eq!(g.reference, 6.0);
    }

    #[test]
    fn le_son_ne_deplace_que_le_temps_le_plus_proche() {
        let mut g = BeatGrid {
            reference: 6.1,
            played: 6.1,
            ..Default::default()
        };
        g.follow(0.25);
        assert!((g.reference - 6.25).abs() < 1e-6);
        g.follow(0.95);
        assert!((g.reference - 5.95).abs() < 1e-6, "{}", g.reference);
    }

    #[test]
    fn un_geste_de_phase_coupe_le_son_jusqu_a_reprendre_le_suivi() {
        let mut g = BeatGrid::default();
        g.first_beat();
        g.follow(0.25);
        assert_eq!(g.reference, 0.0);
        g.listen();
        g.follow(0.25);
        assert!((g.reference - 0.25).abs() < 1e-6);
    }

    #[test]
    fn les_limites_se_comptent_par_multiple() {
        assert!(crossed(0.99, 1.01, 1.0));
        assert!(!crossed(1.01, 1.5, 1.0));
        assert!(crossed(3.99, 4.0, 4.0));
        assert!(!crossed(2.0, 3.9, 4.0));
        assert!(crossed(-0.01, 0.01, 1.0));
    }

    #[test]
    fn sans_quantification_un_lancement_part_tout_de_suite() {
        let mut g = grille();
        assert!(lance(&mut g, 0, 0));
        assert_eq!(actif(&g, 0), Some(g.cases[0][0]));
        assert!(!g.c.is_waiting(g.pistes[0], g.cases[0][0]));
    }

    #[test]
    fn un_lancement_attend_le_prochain_temps() {
        let mut g = grille();
        g.c.quantize = Quantize::Beat;
        g.c.advance_beat(0.1, 120.0);
        assert!(lance(&mut g, 0, 0));
        assert_eq!(actif(&g, 0), None);
        assert!(g.c.is_waiting(g.pistes[0], g.cases[0][0]));

        g.c.advance_beat(0.1, 120.0);
        assert_eq!(actif(&g, 0), None, "0,4 temps");
        g.c.advance_beat(0.31, 120.0);
        assert_eq!(actif(&g, 0), Some(g.cases[0][0]), "le temps est franchi");
        assert!(!g.c.is_waiting(g.pistes[0], g.cases[0][0]));
    }

    #[test]
    fn quatre_temps_attendent_le_debut_de_la_mesure() {
        let mut g = grille();
        g.c.quantize = Quantize::Bar;
        g.c.advance_beat(0.1, 120.0);
        lance(&mut g, 0, 0);
        g.c.advance_beat(1.8, 120.0);
        assert_eq!(actif(&g, 0), None, "3,8 temps");
        g.c.advance_beat(0.2, 120.0);
        assert_eq!(actif(&g, 0), Some(g.cases[0][0]));
    }

    #[test]
    fn le_dernier_geste_gagne_sur_sa_piste_seulement() {
        let mut g = grille();
        g.c.quantize = Quantize::Beat;
        lance(&mut g, 0, 0);
        lance(&mut g, 1, 0);
        lance(&mut g, 0, 1);
        assert!(!g.c.is_waiting(g.pistes[0], g.cases[0][0]));
        assert!(g.c.is_waiting(g.pistes[0], g.cases[0][1]));
        assert!(g.c.is_waiting(g.pistes[1], g.cases[1][0]));
    }

    #[test]
    fn une_colonne_attend_sur_toutes_les_pistes_et_remplace_le_reste() {
        let mut g = grille();
        g.c.quantize = Quantize::Beat;
        lance(&mut g, 0, 0);
        let colonne = Command::LaunchColumn {
            column: g.colonnes[1],
        };
        assert!(g.c.apply(colonne, 0.0));
        assert!(!g.c.is_waiting(g.pistes[0], g.cases[0][0]));
        assert!(g.c.is_waiting(g.pistes[0], g.cases[0][1]));
        assert!(g.c.is_waiting(g.pistes[1], g.cases[1][1]));
        g.c.advance_beat(0.6, 120.0);
        assert_eq!(actif(&g, 0), Some(g.cases[0][1]));
        assert_eq!(actif(&g, 1), Some(g.cases[1][1]));
    }

    #[test]
    fn un_clip_etranger_est_refuse_et_n_attend_pas() {
        let mut g = grille();
        g.c.quantize = Quantize::Beat;
        let etranger = Command::LaunchClip {
            layer: g.pistes[0],
            clip: g.cases[1][0],
        };
        assert!(!g.c.apply(etranger, 0.0));
        assert!(g.c.waiting.is_empty());
    }

    #[test]
    fn vider_une_piste_annule_ce_qu_elle_attendait() {
        let mut g = grille();
        g.c.quantize = Quantize::Beat;
        lance(&mut g, 0, 0);
        let vide = Command::ClearLayer { layer: g.pistes[0] };
        g.c.apply(vide, 0.0);
        g.c.advance_beat(1.0, 120.0);
        assert_eq!(actif(&g, 0), None);
    }

    #[test]
    fn un_clip_retire_pendant_l_attente_ne_panique_pas() {
        let mut g = grille();
        g.c.quantize = Quantize::Beat;
        lance(&mut g, 0, 0);
        assert!(g.c.remove_clip(g.pistes[0], g.colonnes[0]));
        g.c.advance_beat(1.0, 120.0);
        assert_eq!(actif(&g, 0), None);
        assert!(g.c.waiting.is_empty());
    }

    #[test]
    fn couper_la_quantification_libere_ce_qui_attend() {
        let mut g = grille();
        g.c.quantize = Quantize::Bar;
        lance(&mut g, 0, 0);
        g.c.quantize = Quantize::Off;
        g.c.advance_beat(FRAME, 120.0);
        assert_eq!(actif(&g, 0), Some(g.cases[0][0]));
    }

    #[test]
    fn un_trig_ne_passe_pas_par_la_quantification() {
        let mut g = grille();
        g.c.quantize = Quantize::Bar;
        let trig = Command::TriggerClip {
            layer: g.pistes[0],
            clip: g.cases[0][0],
        };
        assert!(g.c.apply(trig, 0.0));
        assert_eq!(actif(&g, 0), Some(g.cases[0][0]));
    }

    #[test]
    fn la_premiere_frappe_d_une_serie_est_le_premier_temps() {
        let mut c = Composition::new();
        c.advance_beat(1.3, 120.0);
        c.apply(Command::TapTempo, 1.3);
        assert_eq!(c.beat.reference, 0.0);
        c.advance_beat(0.51, 120.0);
        c.apply(Command::TapTempo, 1.81);
        assert_eq!(c.beat.reference, 1.0, "la suivante garde le compte");
    }

    #[test]
    fn temps_1_coupe_la_phase_du_son_et_garde_son_bpm() {
        let mut c = Composition::new();
        c.tempo_follow = true;
        let son = AudioFeatures {
            bpm: 128.0,
            bpm_confidence: 0.8,
            beat_phase: 0.25,
            beat_confidence: 0.8,
            ..Default::default()
        };
        c.follow_tempo(&son);
        assert!((c.beat.reference - 0.25).abs() < 1e-6);

        assert!(c.apply(Command::ResyncBeat, 0.0));
        assert!(!Command::ResyncBeat.is_edit());
        c.follow_tempo(&AudioFeatures { bpm: 130.0, ..son });
        assert_eq!(c.beat.reference, 0.0, "la phase reste à la main");
        assert!(c.tempo_follow);
        assert_eq!(c.tempo.tempo.base(), 130.0, "le BPM suit encore");

        c.apply(Command::SetTempoFollow(true), 0.0);
        c.follow_tempo(&son);
        assert!((c.beat.reference - 0.25).abs() < 1e-6);
    }

    #[test]
    fn une_phase_peu_fiable_ne_deplace_rien() {
        let mut c = Composition::new();
        c.tempo_follow = true;
        c.follow_tempo(&AudioFeatures {
            bpm: 128.0,
            bpm_confidence: 0.8,
            beat_phase: 0.25,
            beat_confidence: 0.1,
            ..Default::default()
        });
        assert_eq!(c.beat.reference, 0.0);
    }

    #[test]
    fn chaque_quantification_a_son_mot_et_son_nom() {
        for q in Quantize::ALL {
            assert_eq!(Quantize::from_word(q.word()), Some(q));
            assert!(!q.label().is_empty());
        }
        assert_eq!(Quantize::from_word("demi"), None);
        assert_eq!(Quantize::default(), Quantize::Off);
    }
}
