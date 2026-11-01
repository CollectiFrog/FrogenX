//! `Transport` et `TempoController` — §3bis.3 de `docs/ARCHITECTURE.md`.

use crate::position::{BeatRate, PositionSignal};
use frogenx_core::{Graph, NodeId, Param};

/// Bornes du tempo, appliquées à la lecture (§18.4).
pub const MIN_BPM: f32 = 20.0;
pub const MAX_BPM: f32 = 400.0;

/// Un tempo utilisable : saturé, et le défaut s'il n'est pas fini.
pub fn sanitize_bpm(bpm: f32) -> f32 {
    if bpm.is_finite() {
        bpm.clamp(MIN_BPM, MAX_BPM)
    } else {
        frogenx_core::DEFAULT_BPM
    }
}

/// Comment un `Clip` avance dans le temps.
///
/// `position` est l'identifiant d'un [`crate::position::PositionSignal`]
/// dans le `Graph` du clip (§3bis.3) — pas un `Param` : une position de
/// lecture avance seule, c'est le rôle d'un `Signal`.
///
/// **Pas de `Serialize`/`Deserialize` ici.** `NodeId` et `Param` (`core`) ne
/// dérivent pas `serde` — leur forme sérialisée passe par le schéma JSON
/// nommé du format de patch (§19.1 : `{"fixed": ...}` / `{"modulated": {...}}`),
/// pas par un `derive` direct sur les structures en mémoire. Même choix que
/// pour `Clip` (§3bis.2) : ce n'est pas un oubli, c'est cohérent avec une
/// décision déjà prise avant que `composition` n'existe.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Transport {
    /// Lecture en temps absolu (secondes). `speed` module la vitesse de
    /// `PositionSignal` — 1.0 = vitesse normale, négatif = marche arrière.
    Timeline { position: NodeId, speed: Param },
    /// Lecture synchronisée au tempo partagé (`TempoController::tempo`).
    /// `duration_beats` fixe la longueur d'une boucle, en battements.
    BpmSync {
        position: NodeId,
        duration_beats: Param,
        play_mode: PlayMode,
    },
}

impl Transport {
    /// Une lecture calée sur le tempo : ajoute à `graph` une position qui
    /// avance en battements, et rend le transport qui la désigne.
    pub fn bpm_sync(graph: &mut Graph, duration_beats: Param, play_mode: PlayMode) -> Self {
        let rate = graph.add(Box::new(BeatRate::new(1.0)));
        let speed = Param::modulated(0.0, 1.0, rate);
        let position = graph
            .add_with_deps(Box::new(PositionSignal::new(speed)), vec![rate])
            .expect("`rate` vient d'être créé");
        Self::BpmSync {
            position,
            duration_beats,
            play_mode,
        }
    }
}

/// Ce qu'un clip fait au bout de sa durée (§3bis.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayMode {
    Loop,
    /// La piste se vide.
    Once,
    /// Son temps s'inverse à chaque extrémité.
    PingPong,
}

impl PlayMode {
    pub const ALL: [PlayMode; 3] = [PlayMode::Loop, PlayMode::Once, PlayMode::PingPong];

    pub fn label(self) -> &'static str {
        match self {
            PlayMode::Loop => "Boucle",
            PlayMode::Once => "Une fois",
            PlayMode::PingPong => "Aller-retour",
        }
    }
}

/// Pilote le tempo partagé par plusieurs clips et layers — §3bis.3.
///
/// Une couche **au-dessus** de `Clock` (§4), pas une redéfinition : `Clock`
/// répond à « quelle heure est-il » pour le déterminisme, `TempoController`
/// répond à « combien de temps dure un beat maintenant », une donnée
/// musicale qui change en live (tap tempo, resync).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TempoController {
    /// Tempo courant, en BPM.
    pub tempo: Param,
}

impl TempoController {
    pub fn new(bpm: f32) -> Self {
        Self {
            tempo: Param::fixed(bpm),
        }
    }

    /// Durée d'un battement, en secondes, pour un tempo fixe donné.
    ///
    /// `tempo` à `0.0` ou négatif n'a pas de sens musical — saturé ici, chez
    /// le consommateur, jamais à la source (§18.4).
    pub fn beat_duration_secs(&self, bpm: f32) -> f32 {
        let bpm = bpm.max(1.0);
        60.0 / bpm
    }

    /// Calcule un BPM à partir d'une séquence d'instants de tap (secondes),
    /// par la moyenne des intervalles. Rend `None` si la séquence ne permet
    /// pas de conclure (moins de deux taps, ou intervalle non positif).
    pub fn bpm_from_taps(taps: &[f64]) -> Option<f32> {
        if taps.len() < 2 {
            return None;
        }
        let intervals: Vec<f64> = taps.windows(2).map(|w| w[1] - w[0]).collect();
        if intervals.iter().any(|&d| d <= 0.0) {
            return None;
        }
        let mean = intervals.iter().sum::<f64>() / intervals.len() as f64;
        Some((60.0 / mean) as f32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bpm_from_taps_calcule_un_bpm_correct() {
        // Quatre taps à exactement 0.5s d'intervalle → 120 BPM.
        let taps = [0.0, 0.5, 1.0, 1.5];
        let bpm = TempoController::bpm_from_taps(&taps).expect("bpm attendu");
        assert!((bpm - 120.0).abs() < 1e-3, "got {bpm}");
    }

    #[test]
    fn bpm_from_taps_tolere_une_legere_irregularite() {
        // Un VJ ne tape jamais parfaitement en rythme — la moyenne absorbe.
        let taps = [0.0, 0.49, 1.0, 1.52];
        let bpm = TempoController::bpm_from_taps(&taps).expect("bpm attendu");
        assert!((bpm - 118.1).abs() < 0.5, "got {bpm}");
    }

    #[test]
    fn bpm_from_taps_rend_none_si_pas_assez_de_taps() {
        assert_eq!(TempoController::bpm_from_taps(&[]), None);
        assert_eq!(TempoController::bpm_from_taps(&[0.0]), None);
    }

    #[test]
    fn bpm_from_taps_rend_none_si_intervalle_non_positif() {
        // Deux taps au même instant (ou dans le désordre) ne veut rien dire.
        assert_eq!(TempoController::bpm_from_taps(&[1.0, 1.0]), None);
        assert_eq!(TempoController::bpm_from_taps(&[1.0, 0.5]), None);
    }

    #[test]
    fn beat_duration_sature_un_tempo_non_positif_sans_paniquer() {
        let tc = TempoController::new(120.0);
        assert!(tc.beat_duration_secs(0.0).is_finite());
        assert!(tc.beat_duration_secs(-10.0).is_finite());
    }
}
