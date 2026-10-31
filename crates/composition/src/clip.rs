//! Le `Clip` — point de jonction avec le graphe modulaire, §3bis.2 de
//! `docs/ARCHITECTURE.md`.

use frogenx_core::{Graph, Param};
use frogenx_shape::{ColorParam, ParamDesc, ShapeGenerator, ShapeOp, ShapeStyle, COLOR_CHANNELS};

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

/// Où vit un paramètre du clip. Reste valable d'une frame à l'autre, tant
/// que le clip ne change pas de structure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamSlot {
    Generator(usize),
    Effect {
        effect: usize,
        param: usize,
    },
    /// Un canal du remplissage, dans l'ordre de `COLOR_CHANNELS`.
    Fill(usize),
    /// 0 : l'épaisseur. 1 à 4 : les canaux de la couleur du trait.
    Stroke(usize),
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClipParam {
    pub slot: ParamSlot,
    pub desc: ParamDesc,
}

/// Les paramètres d'une partie du clip : son générateur, un effet, son style.
#[derive(Debug, Clone, PartialEq)]
pub struct ParamGroup {
    pub label: String,
    pub params: Vec<ClipParam>,
}

/// Les canaux d'une couleur, décrits pour un curseur.
fn color_descs(color: ColorParam) -> impl Iterator<Item = ParamDesc> {
    color
        .channels()
        .into_iter()
        .zip(COLOR_CHANNELS)
        .map(|(value, name)| ParamDesc {
            name,
            min: 0.0,
            max: 1.0,
            value,
        })
}

/// La vitesse maximale d'un clip, appliquée à la lecture (§18.4).
pub const MAX_CLIP_SPEED: f32 = 4.0;

/// Une vitesse utilisable : saturée, et 1 si elle n'est pas finie.
pub fn sanitize_speed(speed: f32) -> f32 {
    if speed.is_finite() {
        speed.clamp(0.0, MAX_CLIP_SPEED)
    } else {
        1.0
    }
}

/// Source + effets + transport, pilotable par son propre `Graph` — §3bis.2.
///
/// **Pas de `Serialize` ici.** `effects` et `source` portent des trait
/// objects (`Box<dyn ShapeOp>`, `Graph`) qui ne se sérialisent pas tels
/// quels — c'est le rôle du format de patch (§19.1) de les décrire par
/// `kind`/`params` nommés, pas de cette structure en mémoire.
pub struct Clip {
    /// Ce qu'affiche sa cellule dans la grille (§23.5).
    pub name: String,
    pub source: ClipSource,
    pub effects: Vec<Box<dyn ShapeOp>>,
    pub transport: Transport,
    pub transition: ClipTransition,
    /// Multiplie le `dt` de son graphe (§3bis.2). Se résout dans le graphe de
    /// composition : on doit la connaître avant d'évaluer celui du clip.
    pub speed: Param,
}

impl Clip {
    /// Un clip de forme procédurale, sans effet, coupure instantanée. Il porte
    /// le nom de sa forme.
    pub fn shape(graph: Graph, generator: Box<dyn ShapeGenerator>, transport: Transport) -> Self {
        Self {
            name: generator.kind().to_string(),
            source: ClipSource::Shape {
                graph,
                generator,
                style: ShapeStyle::default(),
            },
            effects: Vec::new(),
            transport,
            transition: ClipTransition::Cut,
            speed: Param::fixed(1.0),
        }
    }

    pub fn with_name(mut self, name: impl Into<String>) -> Self {
        self.name = name.into();
        self
    }

    /// Tout ce qui se règle dans ce clip, sans connaître aucun type de forme
    /// (§16.3 ter). Les `Param` de son graphe interne n'y figurent pas.
    pub fn params(&self) -> Vec<ParamGroup> {
        let ClipSource::Shape {
            generator, style, ..
        } = &self.source;

        let enumere = |descs: Vec<ParamDesc>, slot: &dyn Fn(usize) -> ParamSlot| {
            descs
                .into_iter()
                .enumerate()
                .map(|(i, desc)| ClipParam {
                    slot: slot(i),
                    desc,
                })
                .collect()
        };

        let mut groupes = vec![ParamGroup {
            label: generator.kind().to_string(),
            params: enumere(generator.params(), &ParamSlot::Generator),
        }];
        for (effect, op) in self.effects.iter().enumerate() {
            groupes.push(ParamGroup {
                label: op.kind().to_string(),
                params: enumere(op.params(), &|param| ParamSlot::Effect { effect, param }),
            });
        }
        if let Some(fill) = style.fill {
            groupes.push(ParamGroup {
                label: "Remplissage".to_string(),
                params: enumere(color_descs(fill).collect(), &ParamSlot::Fill),
            });
        }
        if let Some(stroke) = style.stroke {
            let epaisseur = ParamDesc {
                name: "épaisseur",
                min: 0.0,
                max: 0.2,
                value: stroke.width,
            };
            let descs = std::iter::once(epaisseur)
                .chain(color_descs(stroke.color))
                .collect();
            groupes.push(ParamGroup {
                label: "Trait".to_string(),
                params: enumere(descs, &ParamSlot::Stroke),
            });
        }
        groupes
    }

    /// Règle un paramètre en gardant sa modulation (§16.3 ter). Rend `false`
    /// si `slot` ne désigne rien ou si `value` n'est pas fini.
    pub fn set_param(&mut self, slot: ParamSlot, value: f32) -> bool {
        if !value.is_finite() {
            return false;
        }
        match self.param_mut(slot) {
            Some(p) => {
                p.set_base(value);
                true
            }
            None => false,
        }
    }

    /// Règle l'amplitude d'un paramètre modulé. Rend `false` si `slot` ne
    /// désigne rien, si le paramètre n'est pas modulé ou si `value` n'est pas
    /// fini.
    pub fn set_depth(&mut self, slot: ParamSlot, value: f32) -> bool {
        value.is_finite() && self.param_mut(slot).is_some_and(|p| p.set_depth(value))
    }

    fn param_mut(&mut self, slot: ParamSlot) -> Option<&mut Param> {
        let ClipSource::Shape {
            generator, style, ..
        } = &mut self.source;
        match slot {
            ParamSlot::Generator(i) => generator.param_mut(i),
            ParamSlot::Effect { effect, param } => self
                .effects
                .get_mut(effect)
                .and_then(|op| op.param_mut(param)),
            ParamSlot::Fill(i) => style.fill.as_mut().and_then(|c| c.channel_mut(i)),
            ParamSlot::Stroke(0) => style.stroke.as_mut().map(|s| &mut s.width),
            ParamSlot::Stroke(i) => style
                .stroke
                .as_mut()
                .and_then(|s| s.color.channel_mut(i - 1)),
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

    /// Un cercle répété, rouge et tracé.
    fn clip_complet() -> Clip {
        let rouge = frogenx_shape::ColorParam::srgb(1.0, 0.0, 0.0, 1.0);
        let transport = Transport::Timeline {
            position: dummy_position(),
            speed: Param::fixed(1.0),
        };
        Clip::shape(Graph::new(), Box::new(Ellipse::circle(0.5)), transport)
            .with_effect(Box::new(RadialRepeat::new(3.0)))
            .with_style(ShapeStyle {
                fill: Some(rouge),
                stroke: Some(frogenx_shape::StrokeStyle {
                    width: Param::fixed(0.01),
                    color: rouge,
                }),
            })
    }

    /// La valeur de base d'un paramètre, retrouvée par son emplacement.
    fn base(clip: &Clip, slot: ParamSlot) -> f32 {
        clip.params()
            .iter()
            .flat_map(|g| &g.params)
            .find(|p| p.slot == slot)
            .expect("emplacement listé")
            .desc
            .value
            .base()
    }

    #[test]
    fn un_clip_porte_le_nom_de_sa_forme_par_defaut() {
        let clip = clip_complet();
        assert_eq!(clip.name, "Ellipse");
        assert_eq!(clip.with_name("Lune").name, "Lune");
    }

    #[test]
    fn un_clip_groupe_ses_parametres_par_partie() {
        let labels: Vec<String> = clip_complet()
            .params()
            .into_iter()
            .map(|g| g.label)
            .collect();
        assert_eq!(
            labels,
            ["Ellipse", "Répétition radiale", "Remplissage", "Trait"]
        );
    }

    #[test]
    fn chaque_emplacement_liste_s_ecrit_et_se_relit() {
        let mut clip = clip_complet();
        let slots: Vec<ParamSlot> = clip
            .params()
            .iter()
            .flat_map(|g| g.params.iter().map(|p| p.slot))
            .collect();
        assert!(slots.len() > 10, "got {}", slots.len());
        for (i, slot) in slots.into_iter().enumerate() {
            let valeur = 0.25 + i as f32 * 0.01;
            assert!(clip.set_param(slot, valeur), "{slot:?}");
            assert_eq!(base(&clip, slot), valeur, "{slot:?}");
        }
    }

    #[test]
    fn un_reglage_garde_la_modulation() {
        let mut clip = clip_complet();
        let source = dummy_position();
        let ClipSource::Shape { style, .. } = &mut clip.source;
        style.fill.as_mut().unwrap().r = Param::modulated(0.1, 0.5, source);

        assert!(clip.set_param(ParamSlot::Fill(0), 0.8));
        let ClipSource::Shape { style, .. } = &clip.source;
        assert_eq!(
            style.fill.unwrap().r,
            Param::modulated(0.8, 0.5, source),
            "la base change, le câble reste"
        );
    }

    #[test]
    fn l_amplitude_se_regle_sur_un_parametre_module_seulement() {
        let mut clip = clip_complet();
        let source = dummy_position();
        let ClipSource::Shape { style, .. } = &mut clip.source;
        style.fill.as_mut().unwrap().r = Param::modulated(0.1, 0.5, source);

        assert!(clip.set_depth(ParamSlot::Fill(0), 0.2));
        let ClipSource::Shape { style, .. } = &clip.source;
        assert_eq!(style.fill.unwrap().r, Param::modulated(0.1, 0.2, source));

        assert!(!clip.set_depth(ParamSlot::Fill(1), 0.2), "pas de câble");
        assert!(!clip.set_depth(ParamSlot::Fill(0), f32::NAN));
        assert!(!clip.set_depth(ParamSlot::Generator(99), 0.2));
    }

    #[test]
    fn la_vitesse_est_saturee_a_la_lecture() {
        assert_eq!(sanitize_speed(2.0), 2.0);
        assert_eq!(sanitize_speed(-1.0), 0.0);
        assert_eq!(sanitize_speed(99.0), MAX_CLIP_SPEED);
        assert_eq!(sanitize_speed(f32::NAN), 1.0);
        assert_eq!(sanitize_speed(f32::INFINITY), 1.0);
    }

    #[test]
    fn un_emplacement_ou_une_valeur_invalide_est_refuse() {
        let mut clip = clip_complet();
        assert!(!clip.set_param(ParamSlot::Generator(99), 0.5));
        assert!(!clip.set_param(
            ParamSlot::Effect {
                effect: 9,
                param: 0
            },
            0.5
        ));
        assert!(!clip.set_param(ParamSlot::Fill(4), 0.5));
        assert!(!clip.set_param(ParamSlot::Stroke(5), 0.5));
        assert!(!clip.set_param(ParamSlot::Generator(0), f32::NAN));
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
