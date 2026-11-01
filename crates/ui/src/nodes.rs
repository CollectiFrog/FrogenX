//! L'éditeur de nœuds — §23.10 de `docs/ARCHITECTURE.md`.
//!
//! Dans sa propre fenêtre, comme le reste de l'interface une fonction de l'état :
//! [`editor`] lit la grille et remplit une liste de `Command`. Ce qu'il montre se
//! calcule à part ([`boxes`], [`auto_positions`]), sans egui : ça se teste seul.

use crate::{theme, HistoryRequest};
use egui::{
    Align2, Color32, DragValue, FontId, Pos2, Rect, Response, RichText, Sense, Shape, Stroke, Ui,
    UiBuilder, Vec2, WidgetInfo,
};
use frogenx_composition::{
    ClipId, ClipParam, ClipSource, Command, Composition, GraphScope, IdMap, NodeKind, NodeValue,
    ParamTarget,
};
use frogenx_core::{FeatureKind, NodeId, Param, SpecValue, CHROMA_NOTES};
use frogenx_shape::EffectKind;
use std::collections::HashMap;

/// La largeur d'une boîte, ses lignes et l'écart entre deux colonnes.
pub const BOX_WIDTH: f32 = 236.0;
pub const TITLE_HEIGHT: f32 = 26.0;
pub const ROW_HEIGHT: f32 = 24.0;
pub const COLUMN_STEP: f32 = 290.0;
const BOX_GAP: f32 = 18.0;
const MARGIN: f32 = 20.0;
/// Le rayon d'un port, et la distance où un fil s'y accroche.
const PORT_RADIUS: f32 = 5.0;
pub const PORT_REACH: f32 = 14.0;

const WAVEFORMS: [(&str, &str); 4] = [
    ("sine", "Sinus"),
    ("triangle", "Triangle"),
    ("saw", "Dent de scie"),
    ("square", "Carré"),
];

/// Le graphe affiché.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Tab {
    #[default]
    Clip,
    Regie,
}

/// Ce que l'éditeur se rappelle. La place des nœuds n'est pas dans le fichier
/// (§23.10) : un patch décrit ce qui joue, pas sa mise en page.
#[derive(Debug, Clone, Default)]
pub struct EditorState {
    pub tab: Tab,
    pub pan: Vec2,
    positions: HashMap<(GraphScope, NodeId), Pos2>,
    /// Le fil en cours de tirage : la sortie d'où il part.
    dragging: Option<NodeId>,
    /// Annuler ou rétablir, demandé cette frame (§23.11).
    pub history: Option<HistoryRequest>,
}

impl EditorState {
    /// La grille a été restaurée : les nœuds d'un clip gardent leur place.
    pub fn remap(&mut self, map: &IdMap) {
        let places = std::mem::take(&mut self.positions);
        self.positions = places
            .into_iter()
            .filter_map(|((scope, node), p)| {
                let scope = match scope {
                    GraphScope::Clip(clip) => GraphScope::Clip(map.clip(clip)?),
                    regie => regie,
                };
                Some(((scope, node), p))
            })
            .collect();
        self.dragging = None;
    }

    /// La grille a été remplacée : les identifiants ne désignent plus rien.
    pub fn forget_grid(&mut self) {
        self.positions.clear();
        self.dragging = None;
    }
}

/// Ce qu'`app` prête à l'éditeur pour une frame.
pub struct EditorView<'a> {
    pub composition: &'a Composition,
    /// Le clip choisi dans l'interface.
    pub selected: Option<ClipId>,
}

/// Une entrée : un `Param`, donc un port.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeInput {
    pub label: String,
    pub target: ParamTarget,
    pub param: Param,
    /// Le pas d'un glissé sur sa valeur.
    pub speed: f32,
}

/// Un réglage qui n'est pas un `Param`.
#[derive(Debug, Clone, PartialEq)]
pub enum Setting {
    Number {
        field: u8,
        label: String,
        value: f32,
    },
    Int {
        field: u8,
        label: String,
        value: u32,
    },
    Bool {
        field: u8,
        label: String,
        value: bool,
    },
    Word {
        field: u8,
        label: String,
        value: String,
    },
}

/// Ce que retire le clic droit sur un titre.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Removal {
    None,
    Node(NodeId),
    Effect(usize),
}

/// Une boîte : un nœud, ou un groupe de destinations.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeBox {
    pub title: String,
    /// Le nœud, s'il en est un : il a alors une sortie.
    pub node: Option<NodeId>,
    pub value: Option<f32>,
    pub inputs: Vec<NodeInput>,
    pub settings: Vec<Setting>,
    pub removal: Removal,
    /// Un trig garde ses réglages : ils ne se lisent ici qu'en consultation.
    pub read_only: bool,
    /// La profondeur dans le graphe : sa colonne par défaut.
    pub column: usize,
}

impl NodeBox {
    fn height(&self) -> f32 {
        TITLE_HEIGHT + ROW_HEIGHT * (self.inputs.len() + self.settings.len()) as f32 + 6.0
    }
}

/// Le nom d'un type de nœud à l'écran.
pub fn node_label(kind: &str) -> &str {
    match kind {
        "Constant" => "Constante",
        "Lfo" => "LFO",
        "Oscillator" => "Oscillateur",
        "Noise" => "Bruit",
        "Envelope" => "Enveloppe",
        "Trigger" => "Seuil",
        "SampleHold" => "Échantillonneur",
        "Sum" => "Somme",
        "Product" => "Produit",
        "Mix" => "Mélange",
        "BeatRate" => "Tempo",
        "PositionSignal" => "Position",
        "AudioFeature" => "Audio",
        "SpectrumZone" => "Zone",
        "Divider" => "Compteur",
        autre => autre,
    }
}

fn field_label(name: &str) -> String {
    let francais = match name {
        "freq" => "fréquence",
        "amp" => "amplitude",
        "phase_offset" => "phase",
        "rate" => "vitesse",
        "input" => "entrée",
        "trigger" => "déclencheur",
        "attack" => "attaque",
        "release" => "relâchement",
        "threshold" => "seuil",
        "seed" => "graine",
        "octaves" => "octaves",
        "persistence" => "persistance",
        "waveform" => "forme",
        "unipolar" => "unipolaire",
        "feature" => "feature",
        "beats" => "battements",
        "speed" => "vitesse",
        "value" => "valeur",
        "mix" => "mélange",
        "every" => "1 sur",
        "freq_min" => "fréquence basse",
        "freq_max" => "fréquence haute",
        "level_min" => "niveau bas",
        "level_max" => "niveau haut",
        "hysteresis" => "hystérésis",
        autre => autre,
    };
    francais.to_string()
}

/// Les choix d'un champ de mots.
pub fn word_choices(field: &str) -> Vec<(&'static str, &'static str)> {
    match field {
        "waveform" => WAVEFORMS.to_vec(),
        "feature" => FeatureKind::MENU
            .into_iter()
            .chain((0..CHROMA_NOTES as u8).map(FeatureKind::Chroma))
            .map(|f| (f.word(), f.label()))
            .collect(),
        _ => Vec::new(),
    }
}

/// Le pas d'un réglage dont on ne connaît pas la plage.
const FREE_SPEED: f32 = 0.01;

fn clip_input(clip: ClipId, p: &ClipParam) -> NodeInput {
    NodeInput {
        label: p.desc.name.to_string(),
        target: ParamTarget::ClipParam { clip, slot: p.slot },
        param: p.desc.value,
        speed: (p.desc.max - p.desc.min) * 0.005,
    }
}

/// Le nom d'un nœud de trig, s'il en est un : c'est ce qu'on cherche à brancher.
fn trig_title(c: &Composition, node: NodeId) -> Option<String> {
    c.trigs().iter().filter_map(|id| c.trig(*id)).find_map(|t| {
        let role = if node == t.gate() {
            "porte"
        } else if node == t.pulse() {
            "compteur"
        } else if node == t.flash() {
            "flash"
        } else {
            return None;
        };
        Some(format!("{} : {role}", t.name))
    })
}

/// Les boîtes des nœuds de `scope`, chacune dans la colonne de sa profondeur.
fn node_boxes(c: &Composition, scope: GraphScope) -> Vec<NodeBox> {
    let Some(graph) = c.graph_of(scope) else {
        return Vec::new();
    };
    let vues = graph.nodes();
    let mut profondeurs: HashMap<NodeId, usize> = HashMap::new();
    let mut boites = Vec::with_capacity(vues.len());
    for vue in &vues {
        let profondeur = vue
            .deps
            .iter()
            .filter_map(|d| profondeurs.get(d))
            .map(|p| p + 1)
            .max()
            .unwrap_or(0);
        profondeurs.insert(vue.id, profondeur);

        let trig = trig_title(c, vue.id);
        let lecture_seule = trig.is_some();
        let inputs = vue
            .spec
            .inputs()
            .into_iter()
            .zip(vue.spec.params_flat())
            .enumerate()
            .map(|(i, (nom, param))| NodeInput {
                label: field_label(&nom),
                target: ParamTarget::NodeParam {
                    scope,
                    node: vue.id,
                    input: i as u8,
                },
                param,
                speed: FREE_SPEED,
            })
            .collect();
        let settings = vue
            .spec
            .fields
            .iter()
            .enumerate()
            .filter_map(|(i, (nom, valeur))| {
                let (field, label) = (i as u8, field_label(nom));
                match valeur {
                    SpecValue::Number(value) => Some(Setting::Number {
                        field,
                        label,
                        value: *value,
                    }),
                    SpecValue::Int(value) => Some(Setting::Int {
                        field,
                        label,
                        value: *value,
                    }),
                    SpecValue::Bool(value) => Some(Setting::Bool {
                        field,
                        label,
                        value: *value,
                    }),
                    SpecValue::Word(value) => Some(Setting::Word {
                        field,
                        label: nom.clone(),
                        value: value.clone(),
                    }),
                    _ => None,
                }
            })
            .collect();
        let verrouille = c.is_locked_node(scope, vue.id);
        boites.push(NodeBox {
            title: trig.unwrap_or_else(|| node_label(&vue.spec.kind).to_string()),
            node: Some(vue.id),
            value: Some(graph.value(vue.id)),
            inputs,
            settings,
            removal: if verrouille {
                Removal::None
            } else {
                Removal::Node(vue.id)
            },
            read_only: lecture_seule,
            column: profondeur,
        });
    }
    boites
}

fn sink(title: String, inputs: Vec<NodeInput>, removal: Removal, column: usize) -> NodeBox {
    NodeBox {
        title,
        node: None,
        value: None,
        inputs,
        settings: Vec::new(),
        removal,
        read_only: false,
        column,
    }
}

/// Les destinations d'un clip : ses parties, et chaque effet se retire.
fn clip_sinks(c: &Composition, id: ClipId, column: usize) -> Vec<NodeBox> {
    let Some(clip) = c.clip(id) else {
        return Vec::new();
    };
    let effets = match clip.source {
        ClipSource::Shape { .. } => clip.effects.len(),
        ClipSource::Feedback { .. } => 0,
    };
    clip.params()
        .iter()
        .enumerate()
        .map(|(i, groupe)| {
            let inputs = groupe.params.iter().map(|p| clip_input(id, p)).collect();
            let removal = if (1..=effets).contains(&i) {
                Removal::Effect(i - 1)
            } else {
                Removal::None
            };
            sink(groupe.label.clone(), inputs, removal, column)
        })
        .collect()
}

/// Les destinations de la régie : le tempo, l'analyse, chaque piste du deck actif.
fn composition_sinks(c: &Composition, column: usize) -> Vec<NodeBox> {
    let entree = |label: &str, target, param, speed| NodeInput {
        label: label.to_string(),
        target,
        param,
        speed,
    };
    let mut boites = vec![sink(
        "Régie".to_string(),
        vec![
            entree("tempo", ParamTarget::Tempo, c.tempo.tempo, 0.5),
            entree(
                "durée d'analyse",
                ParamTarget::AnalysisWindow,
                c.analysis.window,
                0.002,
            ),
        ],
        Removal::None,
        column,
    )];
    let mut pistes = Vec::new();
    c.layers_in_order(&mut pistes);
    for id in pistes.iter().rev() {
        let Some(piste) = c.layer(*id) else { continue };
        let mut inputs = vec![entree(
            "opacité",
            ParamTarget::LayerOpacity(*id),
            piste.blend.opacity,
            0.005,
        )];
        if let Some((clip_id, clip)) = piste.active_clip().and_then(|a| Some((a, c.clip(a)?))) {
            inputs.push(entree(
                "vitesse du clip",
                ParamTarget::ClipSpeed(clip_id),
                clip.speed,
                0.01,
            ));
        }
        boites.push(sink(piste.name.clone(), inputs, Removal::None, column));
    }
    boites
}

/// Tout ce qu'affiche un onglet : les nœuds, puis les destinations à leur droite.
pub fn boxes(c: &Composition, scope: GraphScope) -> Vec<NodeBox> {
    let mut boites = node_boxes(c, scope);
    let colonne = boites.iter().map(|b| b.column + 1).max().unwrap_or(0);
    boites.extend(match scope {
        GraphScope::Composition => composition_sinks(c, colonne),
        GraphScope::Clip(id) => clip_sinks(c, id, colonne),
    });
    boites
}

/// La place de chaque boîte, empilée dans sa colonne.
pub fn auto_positions(boites: &[NodeBox]) -> Vec<Pos2> {
    let mut hauteurs: HashMap<usize, f32> = HashMap::new();
    boites
        .iter()
        .map(|b| {
            let y = hauteurs.entry(b.column).or_insert(MARGIN);
            let position = Pos2::new(MARGIN + b.column as f32 * COLUMN_STEP, *y);
            *y += b.height() + BOX_GAP;
            position
        })
        .collect()
}

/// L'entrée la plus proche de `pos`, à portée.
pub fn input_at(ports: &[(Pos2, ParamTarget)], pos: Pos2) -> Option<ParamTarget> {
    ports
        .iter()
        .filter(|(p, _)| p.distance(pos) <= PORT_REACH)
        .min_by(|a, b| a.0.distance(pos).total_cmp(&b.0.distance(pos)))
        .map(|(_, t)| *t)
}

/// Un fil, de la sortie vers l'entrée : une courbe qui sort à l'horizontale.
pub fn wire_points(from: Pos2, to: Pos2) -> [Pos2; 4] {
    let tension = ((to.x - from.x).abs() * 0.5).max(40.0);
    [
        from,
        from + Vec2::new(tension, 0.0),
        to - Vec2::new(tension, 0.0),
        to,
    ]
}

fn wire(ui: &Ui, from: Pos2, to: Pos2, color: Color32) {
    ui.painter().add(Shape::CubicBezier(
        egui::epaint::CubicBezierShape::from_points_stroke(
            wire_points(from, to),
            false,
            Color32::TRANSPARENT,
            Stroke::new(1.5, color),
        ),
    ));
}

/// Un port, accessible par son nom : les tests le visent, un lecteur d'écran aussi.
fn port(ui: &mut Ui, centre: Pos2, id: egui::Id, nom: String) -> Response {
    let reponse = ui.interact(
        Rect::from_center_size(centre, Vec2::splat(PORT_REACH * 2.0)),
        id,
        Sense::click_and_drag(),
    );
    reponse.widget_info(|| WidgetInfo::labeled(egui::Role::Button, true, nom.clone()));
    let couleur = if reponse.hovered() {
        theme::ACCENT
    } else {
        theme::CURVE
    };
    ui.painter().circle_filled(centre, PORT_RADIUS, couleur);
    reponse
}

fn input_row(ui: &mut Ui, input: &NodeInput, read_only: bool, commands: &mut Vec<Command>) {
    let target = input.target;
    let nom = ui.label(
        RichText::new(&input.label)
            .size(11.0)
            .color(theme::TEXT_DIM),
    );
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.add_enabled_ui(!read_only, |ui| {
            if let Some(mut depth) = input.param.depth() {
                let reglage = DragValue::new(&mut depth)
                    .speed(input.speed)
                    .prefix("± ")
                    .max_decimals(2);
                let reponse = ui.add(reglage);
                let titre = format!("{} : modulation", input.label);
                reponse.widget_info(|| {
                    WidgetInfo::labeled(egui::Role::SpinButton, true, titre.clone())
                });
                if reponse.changed() {
                    commands.push(Command::SetTargetDepth {
                        target,
                        value: depth,
                    });
                }
            }
            let mut base = input.param.base();
            let reglage = DragValue::new(&mut base).speed(input.speed).max_decimals(2);
            if ui.add(reglage).labelled_by(nom.id).changed() {
                commands.push(Command::SetTargetBase {
                    target,
                    value: base,
                });
            }
        });
    });
}

fn setting_row(
    ui: &mut Ui,
    scope: GraphScope,
    node: NodeId,
    setting: &Setting,
    commands: &mut Vec<Command>,
) {
    let mut emet = |field: u8, value: NodeValue| {
        commands.push(Command::SetNodeValue {
            scope,
            node,
            field,
            value,
        });
    };
    match setting {
        Setting::Number {
            field,
            label,
            value,
        } => {
            let nom = ui.label(RichText::new(label).size(11.0).color(theme::TEXT_DIM));
            let mut v = *value;
            if ui
                .add(DragValue::new(&mut v).speed(FREE_SPEED).max_decimals(3))
                .labelled_by(nom.id)
                .changed()
            {
                emet(*field, NodeValue::Number(v));
            }
        }
        Setting::Int {
            field,
            label,
            value,
        } => {
            let nom = ui.label(RichText::new(label).size(11.0).color(theme::TEXT_DIM));
            let mut v = *value;
            if ui.add(DragValue::new(&mut v)).labelled_by(nom.id).changed() {
                emet(*field, NodeValue::Int(v));
            }
        }
        Setting::Bool {
            field,
            label,
            value,
        } => {
            let mut v = *value;
            if ui.checkbox(&mut v, label.as_str()).changed() {
                emet(*field, NodeValue::Bool(v));
            }
        }
        Setting::Word {
            field,
            label,
            value,
        } => {
            let choix = word_choices(label);
            let affiche = choix
                .iter()
                .find(|(mot, _)| mot == value)
                .map_or(value.as_str(), |(_, nom)| nom);
            let titre = field_label(label);
            let nom = ui.label(RichText::new(titre).size(11.0).color(theme::TEXT_DIM));
            egui::ComboBox::from_id_salt((node, *field))
                .selected_text(affiche)
                .width(120.0)
                .show_ui(ui, |ui| {
                    for (mot, nom) in choix.iter().copied() {
                        if ui.selectable_label(mot == value, nom).clicked() {
                            emet(*field, NodeValue::Word(mot));
                        }
                    }
                })
                .response
                .labelled_by(nom.id);
        }
    }
}

/// Le contenu d'une boîte, déjà placée. Rend la position des ports.
struct Drawn {
    output: Option<(NodeId, Pos2)>,
    inputs: Vec<(Pos2, ParamTarget, Option<NodeId>)>,
}

fn draw_box(
    ui: &mut Ui,
    state: &mut EditorState,
    scope: GraphScope,
    boite: &NodeBox,
    origine: Pos2,
    commands: &mut Vec<Command>,
) -> Drawn {
    let rect = Rect::from_min_size(origine, egui::vec2(BOX_WIDTH, boite.height()));
    ui.painter().rect_filled(rect, 6.0, theme::GLASS);
    ui.painter().rect_stroke(
        rect,
        6.0,
        Stroke::new(1.0, theme::GRID_STRONG),
        egui::StrokeKind::Inside,
    );

    let titre = Rect::from_min_size(origine, egui::vec2(BOX_WIDTH, TITLE_HEIGHT));
    let cle = ui
        .scope_id()
        .with("boite")
        .with(boite.node)
        .with(&boite.title);
    let barre = ui.interact(titre, cle, Sense::click_and_drag());
    barre.widget_info(|| WidgetInfo::labeled(egui::Role::Button, true, boite.title.clone()));
    if let Some(node) = boite.node {
        if barre.dragged() {
            state
                .positions
                .entry((scope, node))
                .or_insert(origine - state.pan);
            if let Some(p) = state.positions.get_mut(&(scope, node)) {
                *p += barre.drag_delta();
            }
        }
    }
    ui.painter().text(
        titre.left_center() + Vec2::new(10.0, 0.0),
        Align2::LEFT_CENTER,
        &boite.title,
        FontId::proportional(12.5),
        theme::TEXT,
    );
    if let Some(valeur) = boite.value {
        ui.painter().text(
            titre.right_center() - Vec2::new(16.0, 0.0),
            Align2::RIGHT_CENTER,
            format!("{valeur:.2}"),
            FontId::proportional(10.5),
            theme::TEXT_DIM,
        );
    }
    match boite.removal {
        Removal::Node(node) => {
            barre.context_menu(|ui| {
                if ui.button("Retirer le nœud").clicked() {
                    commands.push(Command::RemoveNode { scope, node });
                    ui.close();
                }
            });
        }
        Removal::Effect(index) => {
            if let GraphScope::Clip(clip) = scope {
                barre.context_menu(|ui| {
                    if ui.button("Retirer l'effet").clicked() {
                        commands.push(Command::RemoveEffect { clip, index });
                        ui.close();
                    }
                });
            }
        }
        Removal::None => {}
    }

    let ligne = |i: usize| {
        Rect::from_min_size(
            origine + Vec2::new(14.0, TITLE_HEIGHT + i as f32 * ROW_HEIGHT),
            egui::vec2(BOX_WIDTH - 24.0, ROW_HEIGHT),
        )
    };
    let centre = egui::Layout::left_to_right(egui::Align::Center);
    let mut entrees = Vec::with_capacity(boite.inputs.len());
    for (i, input) in boite.inputs.iter().enumerate() {
        let zone = ligne(i);
        ui.scope_builder(UiBuilder::new().max_rect(zone).layout(centre), |ui| {
            input_row(ui, input, boite.read_only, commands);
        });
        let position = Pos2::new(rect.left(), zone.center().y);
        let nom = format!("entrée {} {}", boite.title, input.label);
        let id = ui.scope_id().with("entree").with(boite.node).with(&nom);
        let reponse = port(ui, position, id, nom);
        if input.param.source().is_some() {
            let target = input.target;
            reponse.context_menu(|ui| {
                if ui.button("Débrancher").clicked() {
                    commands.push(Command::Disconnect { target });
                    ui.close();
                }
            });
        }
        entrees.push((position, input.target, input.param.source()));
    }
    if let Some(node) = boite.node {
        for (i, reglage) in boite.settings.iter().enumerate() {
            let zone = ligne(boite.inputs.len() + i);
            ui.scope_builder(UiBuilder::new().max_rect(zone).layout(centre), |ui| {
                ui.add_enabled_ui(!boite.read_only, |ui| {
                    setting_row(ui, scope, node, reglage, commands);
                });
            });
        }
    }

    let sortie = boite.node.map(|node| {
        let position = titre.right_center();
        let nom = format!("sortie {}", boite.title);
        let id = ui.scope_id().with("sortie").with(node);
        let reponse = port(ui, position, id, nom);
        if reponse.drag_started() {
            state.dragging = Some(node);
        }
        (node, position)
    });
    Drawn {
        output: sortie,
        inputs: entrees,
    }
}

/// Le menu du fond : ajouter un nœud, ou un effet au clip.
fn add_menu(ui: &mut Ui, scope: GraphScope, effets: bool, commands: &mut Vec<Command>) {
    ui.label(
        RichText::new("Ajouter un nœud")
            .color(theme::TEXT_DIM)
            .size(11.0),
    );
    for kind in NodeKind::ALL {
        if ui.button(kind.label()).clicked() {
            commands.push(Command::AddNode { scope, kind });
            ui.close();
        }
    }
    if let (true, GraphScope::Clip(clip)) = (effets, scope) {
        ui.separator();
        ui.label(
            RichText::new("Ajouter un effet")
                .color(theme::TEXT_DIM)
                .size(11.0),
        );
        for kind in EffectKind::ALL {
            if ui.button(kind.label()).clicked() {
                commands.push(Command::AddEffect { clip, kind });
                ui.close();
            }
        }
    }
}

fn canvas(
    ui: &mut Ui,
    state: &mut EditorState,
    c: &Composition,
    scope: GraphScope,
    commands: &mut Vec<Command>,
) {
    let zone = ui.available_rect_before_wrap();
    let fond = ui.allocate_rect(zone, Sense::click_and_drag());
    ui.painter().rect_filled(zone, 0.0, theme::PLOT_BOTTOM);
    if fond.dragged() {
        state.pan += fond.drag_delta();
    }
    let effets = matches!(
        scope,
        GraphScope::Clip(id) if c.clip(id).is_some_and(|clip| matches!(clip.source, ClipSource::Shape { .. }))
    );
    fond.context_menu(|ui| add_menu(ui, scope, effets, commands));

    let boites = boxes(c, scope);
    let auto = auto_positions(&boites);
    let origine = zone.min.to_vec2() + state.pan;
    let mut sorties = Vec::new();
    let mut entrees = Vec::new();
    ui.set_clip_rect(zone);
    for (boite, position) in boites.iter().zip(&auto) {
        let cle = boite.node.map(|n| (scope, n));
        let place = cle
            .and_then(|k| state.positions.get(&k))
            .copied()
            .unwrap_or(*position)
            + origine;
        let dessin = draw_box(ui, state, scope, boite, place, commands);
        sorties.extend(dessin.output);
        entrees.extend(dessin.inputs);
    }

    for (position, _, source) in &entrees {
        let depart = source.and_then(|s| sorties.iter().find(|(n, _)| *n == s));
        if let Some((_, depart)) = depart {
            wire(ui, *depart, *position, theme::ACCENT.gamma_multiply(0.7));
        }
    }

    let Some(source) = state.dragging else {
        return;
    };
    let curseur = ui.input(|i| i.pointer.latest_pos());
    if let (Some(pos), Some((_, depart))) = (curseur, sorties.iter().find(|(n, _)| *n == source)) {
        wire(ui, *depart, pos, theme::ACCENT);
    }
    if ui.input(|i| i.pointer.any_released()) {
        state.dragging = None;
        let ports: Vec<_> = entrees.iter().map(|(p, t, _)| (*p, *t)).collect();
        if let Some(target) = curseur.and_then(|pos| input_at(&ports, pos)) {
            let reliable = match target {
                ParamTarget::NodeParam { node, .. } => c
                    .graph_of(scope)
                    .is_some_and(|g| g.can_connect(node, source)),
                _ => true,
            };
            if reliable {
                commands.push(Command::Connect { target, source });
            }
        }
    }
}

/// L'éditeur entier, dans sa fenêtre.
pub fn editor(
    ui: &mut Ui,
    state: &mut EditorState,
    view: &EditorView<'_>,
    commands: &mut Vec<Command>,
) {
    if let Some(demande) = crate::history_shortcut(ui) {
        state.history = Some(demande);
    }
    egui::Panel::top("onglets").show(ui, |ui| {
        ui.horizontal(|ui| {
            let titre = match view.selected.and_then(|id| view.composition.clip(id)) {
                Some(clip) => format!("Clip : {}", clip.name),
                None => "Clip".to_string(),
            };
            ui.selectable_value(&mut state.tab, Tab::Clip, titre);
            ui.selectable_value(&mut state.tab, Tab::Regie, "Régie");
            ui.separator();
            ui.label(
                RichText::new(
                    "Clic droit sur le fond pour ajouter. Tire d'une sortie vers une entrée.",
                )
                .size(11.0)
                .color(theme::TEXT_DIM),
            );
        });
    });
    let scope = match state.tab {
        Tab::Regie => Some(GraphScope::Composition),
        Tab::Clip => view
            .selected
            .filter(|id| view.composition.clip(*id).is_some())
            .map(GraphScope::Clip),
    };
    egui::CentralPanel::default()
        .frame(egui::Frame::new())
        .show(ui, |ui| match scope {
            Some(scope) => canvas(ui, state, view.composition, scope, commands),
            None => {
                ui.centered_and_justified(|ui| {
                    ui.label(
                        RichText::new("Clique un clip dans l'interface pour éditer son graphe.")
                            .color(theme::TEXT_DIM),
                    );
                });
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::kittest::Queryable;
    use egui_kittest::Harness;
    use frogenx_composition::{Clip, LayerId, ParamSlot, PlayMode, Transport};
    use frogenx_core::Graph;
    use frogenx_shape::Ellipse;

    struct Grille {
        c: Composition,
        piste: LayerId,
        clip: ClipId,
    }

    fn grille() -> Grille {
        let mut c = Composition::new();
        let deck = c.add_deck("test");
        let groupe = c.add_layer_group(deck).unwrap();
        let piste = c.add_layer(groupe).unwrap();
        let mut g = Graph::new();
        let transport = Transport::bpm_sync(&mut g, Param::fixed(4.0), PlayMode::Loop);
        let clip = Clip::shape(g, Box::new(Ellipse::circle(0.5)), transport).with_name("Alpha");
        let clip = c.add_clip(piste, clip).unwrap();
        c.trigger(piste, clip);
        Grille { c, piste, clip }
    }

    struct Etat {
        editeur: EditorState,
        commands: Vec<Command>,
    }

    fn harness<'a>(c: &'a Composition, selected: Option<ClipId>, tab: Tab) -> Harness<'a, Etat> {
        let etat = Etat {
            editeur: EditorState {
                tab,
                ..Default::default()
            },
            commands: Vec::new(),
        };
        Harness::builder()
            .with_size(egui::vec2(1500.0, 1000.0))
            .build_ui_state(
                move |ui, etat: &mut Etat| {
                    let view = EditorView {
                        composition: c,
                        selected,
                    };
                    editor(ui, &mut etat.editeur, &view, &mut etat.commands);
                },
                etat,
            )
    }

    /// Tire un fil de `depuis` à `vers`, deux éléments de l'éditeur.
    fn tire(h: &mut Harness<'_, Etat>, depuis: &str, vers: &str) {
        let (a, b) = (
            h.get_by_label(depuis).rect().center(),
            h.get_by_label(vers).rect().center(),
        );
        h.drag_at(a);
        h.run();
        h.hover_at(a + egui::vec2(6.0, 0.0));
        h.run();
        h.hover_at(b);
        h.run();
        h.drop_at(b);
        h.run();
    }

    fn scope_du_clip(g: &Grille) -> GraphScope {
        GraphScope::Clip(g.clip)
    }

    #[test]
    fn un_clip_montre_ses_noeuds_et_ses_destinations_a_droite() {
        let g = grille();
        let b = boxes(&g.c, scope_du_clip(&g));
        let titres: Vec<_> = b.iter().map(|b| b.title.as_str()).collect();
        assert_eq!(titres, ["Tempo", "Position", "Ellipse"]);
        assert!(b[2].column > b[1].column && b[1].column > b[0].column);
        assert!(b[2].node.is_none());
        assert_eq!(b[2].inputs[0].label, "rayon");
    }

    #[test]
    fn la_regie_montre_le_tempo_et_chaque_piste() {
        let g = grille();
        let b = boxes(&g.c, GraphScope::Composition);
        let piste = b.iter().find(|b| b.title == "Piste 1").unwrap();
        assert_eq!(piste.inputs[0].target, ParamTarget::LayerOpacity(g.piste));
        assert_eq!(piste.inputs[1].target, ParamTarget::ClipSpeed(g.clip));
        assert_eq!(b[0].inputs[0].target, ParamTarget::Tempo);
    }

    #[test]
    fn les_noeuds_d_un_trig_portent_son_nom_et_ne_se_retirent_pas() {
        let mut g = grille();
        g.c.add_default_trig().unwrap();
        let b = boxes(&g.c, GraphScope::Composition);
        let flash = b.iter().find(|b| b.title == "Trig 1 : flash").unwrap();
        assert_eq!(flash.removal, Removal::None);
        assert!(flash.read_only);
    }

    #[test]
    fn un_effet_ajoute_se_retire_par_son_index() {
        let mut g = grille();
        assert!(g.c.add_effect(g.clip, EffectKind::Deform));
        let b = boxes(&g.c, scope_du_clip(&g));
        let effet = b.iter().find(|b| b.title == "Déformation").unwrap();
        assert_eq!(effet.removal, Removal::Effect(0));
        assert_eq!(
            b.iter().find(|b| b.title == "Ellipse").unwrap().removal,
            Removal::None
        );
    }

    #[test]
    fn deux_boites_d_une_colonne_ne_se_chevauchent_pas() {
        let g = grille();
        let b = boxes(&g.c, GraphScope::Composition);
        let p = auto_positions(&b);
        for i in 1..b.len() {
            assert!(p[i].y >= p[i - 1].y + b[i - 1].height(), "{}", b[i].title);
        }
    }

    #[test]
    fn un_port_s_atteint_de_pres_et_le_plus_proche_gagne() {
        let cible = |n| ParamTarget::LayerOpacity(n);
        let g = grille();
        let ports = [
            (Pos2::new(0.0, 0.0), ParamTarget::Tempo),
            (Pos2::new(20.0, 0.0), cible(g.piste)),
        ];
        assert_eq!(
            input_at(&ports, Pos2::new(3.0, 0.0)),
            Some(ParamTarget::Tempo)
        );
        assert_eq!(input_at(&ports, Pos2::new(16.0, 0.0)), Some(cible(g.piste)));
        assert_eq!(input_at(&ports, Pos2::new(100.0, 0.0)), None);
    }

    #[test]
    fn un_fil_sort_a_l_horizontale_meme_vers_l_arriere() {
        let [a, b, c, d] = wire_points(Pos2::new(100.0, 0.0), Pos2::new(0.0, 50.0));
        assert!(b.x > a.x && c.x < d.x);
        assert_eq!((a.y, b.y), (0.0, 0.0));
    }

    #[test]
    fn chaque_champ_de_mots_propose_des_choix() {
        assert_eq!(word_choices("waveform").len(), 4);
        assert_eq!(word_choices("feature").len(), 18);
        assert!(word_choices("autre").is_empty());
    }

    #[test]
    fn afficher_n_emet_aucune_commande() {
        let mut g = grille();
        g.c.add_default_trig().unwrap();
        for tab in [Tab::Clip, Tab::Regie] {
            let h = harness(&g.c, Some(g.clip), tab);
            assert!(h.state().commands.is_empty());
        }
    }

    #[test]
    fn sans_clip_choisi_l_editeur_le_dit() {
        let g = grille();
        let h = harness(&g.c, None, Tab::Clip);
        assert!(h
            .query_by_label("Clique un clip dans l'interface pour éditer son graphe.")
            .is_some());
    }

    #[test]
    fn un_onglet_change_de_graphe() {
        let g = grille();
        let mut h = harness(&g.c, Some(g.clip), Tab::Clip);
        assert!(h.query_by_label("Clip : Alpha").is_some());
        h.get_by_label("Régie").click();
        h.run();
        assert_eq!(h.state().editeur.tab, Tab::Regie);
        assert!(h.query_by_label("entrée Régie tempo").is_some());
    }

    #[test]
    fn tirer_une_sortie_sur_une_entree_branche() {
        let g = grille();
        let mut h = harness(&g.c, Some(g.clip), Tab::Clip);
        tire(&mut h, "sortie Position", "entrée Ellipse rayon");
        let source = g.c.graph_of(scope_du_clip(&g)).unwrap().nodes()[1].id;
        assert_eq!(
            h.state().commands,
            vec![Command::Connect {
                target: ParamTarget::ClipParam {
                    clip: g.clip,
                    slot: ParamSlot::Generator(0)
                },
                source
            }]
        );
    }

    #[test]
    fn un_flash_de_trig_se_branche_sur_une_opacite() {
        let mut g = grille();
        let trig = g.c.add_default_trig().unwrap();
        let flash = g.c.trig(trig).unwrap().flash();
        let mut h = harness(&g.c, Some(g.clip), Tab::Regie);
        tire(&mut h, "sortie Trig 1 : flash", "entrée Piste 1 opacité");
        assert_eq!(
            h.state().commands,
            vec![Command::Connect {
                target: ParamTarget::LayerOpacity(g.piste),
                source: flash
            }]
        );
    }

    #[test]
    fn une_boucle_n_est_meme_pas_proposee() {
        let mut g = grille();
        let scope = scope_du_clip(&g);
        let a = g.c.add_node(scope, NodeKind::Lfo).unwrap();
        let b = g.c.add_node(scope, NodeKind::Noise).unwrap();
        let cible = ParamTarget::NodeParam {
            scope,
            node: a,
            input: 0,
        };
        assert!(g.c.connect(cible, b));
        let mut h = harness(&g.c, Some(g.clip), Tab::Clip);
        tire(&mut h, "sortie LFO", "entrée Bruit vitesse");
        assert!(h.state().commands.is_empty());
    }

    #[test]
    fn lacher_dans_le_vide_ne_branche_rien() {
        let g = grille();
        let mut h = harness(&g.c, Some(g.clip), Tab::Clip);
        let a = h.get_by_label("sortie Position").rect().center();
        h.drag_at(a);
        h.run();
        h.hover_at(a + egui::vec2(20.0, 400.0));
        h.run();
        h.drop_at(a + egui::vec2(20.0, 400.0));
        h.run();
        assert!(h.state().commands.is_empty());
        assert_eq!(h.state().editeur.dragging, None);
    }

    #[test]
    fn un_clic_droit_sur_une_entree_branchee_la_debranche() {
        let mut g = grille();
        let source =
            g.c.add_node(GraphScope::Composition, NodeKind::Lfo)
                .unwrap();
        let cible = ParamTarget::LayerOpacity(g.piste);
        g.c.connect(cible, source);
        let mut h = harness(&g.c, Some(g.clip), Tab::Regie);
        h.get_by_label("entrée Piste 1 opacité").click_secondary();
        h.run();
        h.get_by_label("Débrancher").click();
        h.run();
        assert_eq!(
            h.state().commands,
            vec![Command::Disconnect { target: cible }]
        );
    }

    #[test]
    fn un_clic_droit_sur_un_titre_retire_le_noeud() {
        let mut g = grille();
        let scope = scope_du_clip(&g);
        let node = g.c.add_node(scope, NodeKind::Noise).unwrap();
        let mut h = harness(&g.c, Some(g.clip), Tab::Clip);
        h.get_by_label("Bruit").click_secondary();
        h.run();
        h.get_by_label("Retirer le nœud").click();
        h.run();
        assert_eq!(
            h.state().commands,
            vec![Command::RemoveNode { scope, node }]
        );
    }

    #[test]
    fn un_noeud_tenu_par_le_systeme_n_a_pas_de_menu() {
        let g = grille();
        let mut h = harness(&g.c, Some(g.clip), Tab::Clip);
        h.get_by_label("Position").click_secondary();
        h.run();
        assert!(h.query_by_label("Retirer le nœud").is_none());
    }

    #[test]
    fn un_effet_se_retire_au_clic_droit() {
        let mut g = grille();
        g.c.add_effect(g.clip, EffectKind::Smooth);
        let mut h = harness(&g.c, Some(g.clip), Tab::Clip);
        h.get_by_label("Lissage").click_secondary();
        h.run();
        h.get_by_label("Retirer l'effet").click();
        h.run();
        assert_eq!(
            h.state().commands,
            vec![Command::RemoveEffect {
                clip: g.clip,
                index: 0
            }]
        );
    }

    #[test]
    fn deplacer_un_noeud_garde_sa_place_et_n_emet_rien() {
        let g = grille();
        let mut h = harness(&g.c, Some(g.clip), Tab::Clip);
        let a = h.get_by_label("Tempo").rect().center();
        h.drag_at(a);
        h.run();
        h.hover_at(a + egui::vec2(40.0, 300.0));
        h.run();
        h.drop_at(a + egui::vec2(40.0, 300.0));
        h.run();
        let apres = h.get_by_label("Tempo").rect().center();
        assert!(apres.y > a.y + 100.0, "{a:?} → {apres:?}");
        assert!(h.state().commands.is_empty());
    }

    #[test]
    fn un_reglage_de_noeud_emet_sa_valeur() {
        let mut g = grille();
        let scope = scope_du_clip(&g);
        let node = g.c.add_node(scope, NodeKind::Lfo).unwrap();
        let mut h = harness(&g.c, Some(g.clip), Tab::Clip);
        h.get_by_label("unipolaire").click();
        h.run();
        assert_eq!(
            h.state().commands,
            vec![Command::SetNodeValue {
                scope,
                node,
                field: 2,
                value: NodeValue::Bool(true),
            }]
        );
    }

    #[test]
    fn la_valeur_d_une_entree_se_regle_sans_la_debrancher() {
        let g = grille();
        let mut h = harness(&g.c, Some(g.clip), Tab::Regie);
        h.get_by_role_and_label(egui::accesskit::Role::SpinButton, "tempo")
            .set_value_accesskit(90.0);
        h.run();
        assert_eq!(
            h.state().commands,
            vec![Command::SetTargetBase {
                target: ParamTarget::Tempo,
                value: 90.0,
            }]
        );
    }

    #[test]
    fn le_menu_du_fond_ajoute_un_noeud_et_un_effet() {
        let g = grille();
        let (scope, clip) = (scope_du_clip(&g), g.clip);
        let mut h = harness(&g.c, Some(clip), Tab::Clip);
        let vide = egui::pos2(700.0, 900.0);
        h.hover_at(vide);
        h.run();
        h.event(egui::Event::PointerButton {
            pos: vide,
            button: egui::PointerButton::Secondary,
            pressed: true,
            modifiers: Default::default(),
        });
        h.event(egui::Event::PointerButton {
            pos: vide,
            button: egui::PointerButton::Secondary,
            pressed: false,
            modifiers: Default::default(),
        });
        h.run();
        h.get_by_label("LFO").click();
        h.run();
        assert_eq!(
            h.state().commands,
            vec![Command::AddNode {
                scope,
                kind: NodeKind::Lfo
            }]
        );
    }

    #[test]
    fn ctrl_z_fonctionne_aussi_dans_l_editeur() {
        let g = grille();
        let mut h = harness(&g.c, Some(g.clip), Tab::Clip);
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
        h.run();
        assert_eq!(h.state().editeur.history, Some(HistoryRequest::Undo));
    }

    #[test]
    fn une_grille_restauree_garde_la_place_des_noeuds_de_ses_clips() {
        let g = grille();
        let noeud = Graph::new().add(Box::new(frogenx_core::Constant(0.0)));
        let mut e = EditorState::default();
        e.positions
            .insert((GraphScope::Clip(g.clip), noeud), Pos2::ZERO);
        e.remap(&IdMap {
            clips: vec![(g.clip, g.clip)],
            ..Default::default()
        });
        assert_eq!(e.positions.len(), 1);
        e.remap(&IdMap::default());
        assert!(e.positions.is_empty(), "le clip a disparu");
    }

    #[test]
    fn oublier_la_grille_oublie_les_places() {
        let mut e = EditorState::default();
        e.positions.insert(
            (
                GraphScope::Composition,
                Graph::new().add(Box::new(frogenx_core::Constant(0.0))),
            ),
            Pos2::ZERO,
        );
        e.forget_grid();
        assert!(e.positions.is_empty());
    }
}
