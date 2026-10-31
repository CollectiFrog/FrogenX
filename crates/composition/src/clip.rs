//! Le `Clip` — point de jonction avec le graphe modulaire, §3bis.2 de
//! `docs/ARCHITECTURE.md`.

use frogenx_core::Graph;
use frogenx_shape::{ShapeGenerator, ShapeOp, ShapeStyle};

use crate::crossfader::FadeCurve;
use crate::position::PositionSignal;
use crate::transport::Transport;

/// Ce qu'un clip joue. La distinction se fait ici, pas sur le type `Clip`
/// lui-même — un clip procédural module ses propres paramètres via son
/// `Graph`, exactement comme avant la réorientation (§3bis.2).
pub enum ClipSource {
    /// Forme procédurale : le générateur produit le `Path`, le graphe module
    /// ses paramètres. Un graphe seul ne produirait aucune forme — les effets
    /// du clip n'auraient rien à transformer (§3bis.2).
    Shape {
        graph: Graph,
        generator: Box<dyn ShapeGenerator>,
        style: ShapeStyle,
    },
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
    /// Coupure instantanée. `composition` ne rend rien : le mélange de deux
    /// clips pendant un `Fade`/`Wipe` est le travail du compositeur de
    /// couches (étape 5 et suivantes, §12), que `composition` ne doit pas
    /// connaître (§10). Ce type fixe seulement la forme du contrat.
    Cut,
    Fade {
        phase: PositionSignal,
        curve: FadeCurve,
    },
    Wipe {
        phase: PositionSignal,
        curve: FadeCurve,
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
    pub fn shape(graph: Graph, generator: Box<dyn ShapeGenerator>, transport: Transport) -> Self {
        Self {
            source: ClipSource::Shape {
                graph,
                generator,
                style: ShapeStyle::default(),
            },
            effects: Vec::new(),
            transport,
            transition: ClipTransition::Cut,
        }
    }

    pub fn with_style(mut self, new_style: ShapeStyle) -> Self {
        let ClipSource::Shape { style, .. } = &mut self.source;
        *style = new_style;
        self
    }

    /// Le graphe qui pilote ce clip.
    pub fn graph(&self) -> &Graph {
        let ClipSource::Shape { graph, .. } = &self.source;
        graph
    }

    pub fn graph_mut(&mut self) -> &mut Graph {
        let ClipSource::Shape { graph, .. } = &mut self.source;
        graph
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
    use frogenx_core::{AudioFeatures, Ctx, NodeId, Param};
    use frogenx_shape::{Ellipse, Path, RadialRepeat};

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
            Box::new(Ellipse::circle(0.5)),
            Transport::Timeline {
                position: dummy_position(),
                speed: Param::fixed(1.0),
            },
        );
        assert!(clip.effects.is_empty());
        assert!(matches!(clip.transition, ClipTransition::Cut));
    }

    #[test]
    fn la_source_shape_produit_une_forme_que_les_effets_transforment() {
        // Le manque corrigé au §3bis.2 : avec `Shape(Graph)`, le clip n'avait
        // aucun moyen de produire une forme, et ses effets rien à transformer.
        let clip = Clip::shape(
            Graph::new(),
            Box::new(Ellipse::circle(0.5)),
            Transport::Timeline {
                position: dummy_position(),
                speed: Param::fixed(1.0),
            },
        )
        .with_effect(Box::new(RadialRepeat::new(3.0)));

        let ClipSource::Shape {
            graph, generator, ..
        } = &clip.source;
        let audio = AudioFeatures::default();
        let ctx = Ctx::new(1.0 / 60.0, 0, &audio);
        let mut path = Path::new();
        generator.generate(0.0, &ctx, graph, &mut path);
        for op in &clip.effects {
            op.apply(0.0, &ctx, graph, &mut path);
        }
        assert_eq!(path.contours.len(), 3, "le cercle répété trois fois");
        assert!(path.is_valid());
    }

    #[test]
    fn un_clip_ne_dessine_rien_tant_qu_il_n_a_pas_de_style() {
        let transport = Transport::Timeline {
            position: dummy_position(),
            speed: Param::fixed(1.0),
        };
        let nu = Clip::shape(Graph::new(), Box::new(Ellipse::circle(0.5)), transport);
        let ClipSource::Shape { style, .. } = &nu.source;
        assert_eq!(*style, ShapeStyle::default());

        let rouge = frogenx_shape::ColorParam::srgb(1.0, 0.0, 0.0, 1.0);
        let style_voulu = ShapeStyle::filled(rouge);
        let colore = nu.with_style(style_voulu);
        let ClipSource::Shape { style, .. } = &colore.source;
        assert_eq!(*style, style_voulu);
    }

    #[test]
    fn with_transition_remplace_la_coupure_par_un_fondu() {
        let clip = Clip::shape(
            Graph::new(),
            Box::new(Ellipse::circle(0.5)),
            Transport::BpmSync {
                position: dummy_position(),
                duration_beats: Param::fixed(4.0),
                play_mode: PlayMode::Loop,
            },
        )
        .with_transition(ClipTransition::Fade {
            phase: PositionSignal::fixed_speed(1.0),
            curve: FadeCurve::SmoothStep,
        });
        assert!(matches!(clip.transition, ClipTransition::Fade { .. }));
    }
}
