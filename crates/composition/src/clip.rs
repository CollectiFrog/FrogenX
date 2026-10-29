//! Le `Clip` — point de jonction avec le graphe modulaire, §3bis.2 de
//! `docs/ARCHITECTURE.md`.

use frogenx_core::Graph;
use frogenx_shape::ShapeOp;

use crate::position::PositionSignal;
use crate::transport::Transport;

/// Ce qu'un clip joue. La distinction se fait ici, pas sur le type `Clip`
/// lui-même — un clip procédural module ses propres paramètres via son
/// `Graph`, exactement comme avant la réorientation (§3bis.2).
pub enum ClipSource {
    /// Forme procédurale : un `Graph` de modulation propre au clip.
    Shape(Graph),
    // `Scene3d(Graph)` et `Video(VideoTrack)` arrivent aux étapes 13 et 12
    // de la feuille de route (§12) — non représentés avant que `scene3d` et
    // le décodage vidéo n'existent, pour ne pas figer un contrat deviné.
}

/// Comment un clip entre en scène ou en sort quand un autre clip prend sa
/// place sur le même `Layer` — §3bis.2.
///
/// `Fade`/`Wipe` réutilisent [`PositionSignal`] pour leur progression — une
/// transition qui avance seule a le même besoin que `Transport.position`
/// (§3bis.3) : ce n'est pas un `Param` statique, c'est un accumulateur.
pub enum ClipTransition {
    /// Coupure instantanée — le seul mode dont le rendu est spécifié avant
    /// l'étape 5 (§12) : `Fade`/`Wipe` dépendent du compositeur de couches,
    /// que `composition` ne doit pas connaître (§10).
    Cut,
    Fade {
        phase: PositionSignal,
    },
    Wipe {
        phase: PositionSignal,
        direction: WipeDirection,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WipeDirection {
    Left,
    Right,
    Up,
    Down,
}

/// Source + effets + transport, pilotable par son propre `Graph` — §3bis.2.
///
/// **Pas de `Serialize` ici.** `effects` et `source` portent des trait
/// objects (`Box<dyn ShapeOp>`, `Graph`) qui ne se sérialisent pas tels
/// quels — c'est le rôle du format de patch (§19.1) de les décrire par
/// `kind`/`params` nommés, pas de cette structure en mémoire.
pub struct Clip {
    pub source: ClipSource,
    pub effects: Vec<Box<dyn ShapeOp>>,
    pub transport: Transport,
    pub transition: ClipTransition,
}

impl Clip {
    /// Un clip de forme procédurale, sans effet, coupure instantanée.
    pub fn shape(graph: Graph, transport: Transport) -> Self {
        Self {
            source: ClipSource::Shape(graph),
            effects: Vec::new(),
            transport,
            transition: ClipTransition::Cut,
        }
    }

    pub fn with_effect(mut self, op: Box<dyn ShapeOp>) -> Self {
        self.effects.push(op);
        self
    }

    pub fn with_transition(mut self, transition: ClipTransition) -> Self {
        self.transition = transition;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transport::{PlayMode, Transport};
    use frogenx_core::{NodeId, Param};

    fn dummy_position() -> NodeId {
        // Un Clip de test n'a pas besoin que `position` résolve réellement
        // dans ce graphe — seule la forme de Transport est exercée ici.
        let mut g = Graph::new();
        g.add(Box::new(PositionSignal::fixed_speed(1.0)))
    }

    #[test]
    fn un_clip_shape_sans_effet_a_une_chaine_vide() {
        let clip = Clip::shape(
            Graph::new(),
            Transport::Timeline {
                position: dummy_position(),
                speed: Param::fixed(1.0),
            },
        );
        assert!(clip.effects.is_empty());
        assert!(matches!(clip.transition, ClipTransition::Cut));
    }

    #[test]
    fn with_transition_remplace_la_coupure_par_un_fondu() {
        let clip = Clip::shape(
            Graph::new(),
            Transport::BpmSync {
                position: dummy_position(),
                duration_beats: Param::fixed(4.0),
                play_mode: PlayMode::Loop,
            },
        )
        .with_transition(ClipTransition::Fade {
            phase: PositionSignal::fixed_speed(1.0),
        });
        assert!(matches!(clip.transition, ClipTransition::Fade { .. }));
    }
}
