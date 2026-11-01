//! # frogenx-ui
//!
//! L'interface : une fonction de l'état — §23.5 de `docs/ARCHITECTURE.md`.
//!
//! [`draw`] lit la grille, dessine avec egui et remplit une liste de `Command`
//! qu'`Engine` applique entre deux frames (§23), une liste de [`Placement`] pour
//! les clips glissés depuis la bibliothèque (§23.6), et au plus un
//! [`OutputRequest`] pour la sortie plein écran (§8.4) et un [`FileRequest`] pour
//! le fichier (§23.8). Elle ne mute rien d'autre que son [`UiState`].
//!
//! **Ne connaît ni wgpu ni `winit` :** `egui-wgpu` et `egui-winit` restent dans
//! `app`. Les tests simulent des clics, sans GPU ni écran.

pub mod clip_panel;
pub mod nodes;
pub mod spectrum;
pub mod theme;
pub mod widgets;

use egui::{
    Align, Button, CornerRadius, Label, Layout, Margin, Pos2, Rect, Response, RichText, Sense,
    Stroke, Ui, UiBuilder,
};
use frogenx_composition::IdMap;
use frogenx_composition::{
    Clip, ClipId, ColumnId, Command, Composition, LayerId, Transport, TrigId,
};
use frogenx_core::{Spectrum, SPECTRUM_BANDS};
use frogenx_library::{presets, Family, PresetId};
use spectrum::ZoneGesture;

/// Ce qu'affiche une case vide, pour la retrouver et la viser.
const EMPTY_CELL: &str = "·";
/// La taille d'une case de la grille.
const CELL_SIZE: egui::Vec2 = egui::vec2(110.0, 34.0);
/// La hauteur de la barre d'avancement d'un clip actif.
const PROGRESS_HEIGHT: f32 = 3.0;
/// La largeur laissée au nom du fichier, au milieu de l'en-tête.
const TITLE_WIDTH: f32 = 420.0;
/// Les proportions de la scène quand aucune image ne les donne.
const STAGE_ASPECT: f32 = 16.0 / 9.0;
/// La taille du texte d'état, à côté du nom du fichier.
const STATUS_SIZE: f32 = 11.0;

/// Un clip du catalogue à poser dans une case. `Command` ne peut pas porter de
/// `Clip` : `app` le construit entre deux frames (§23.6).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub layer: LayerId,
    pub column: ColumnId,
    pub preset: PresetId,
}

/// Un écran branché, déjà mis en mots.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Screen {
    pub label: String,
    /// Celui de l'interface : la sortie s'y ouvre en fenêtre normale, un plein
    /// écran y cacherait le menu qui le ferme (§8.4).
    pub interface: bool,
}

/// L'état de la sortie plein écran.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputView<'a> {
    pub screens: &'a [Screen],
    /// L'écran de la sortie ouverte : un indice dans `screens`.
    pub open: Option<usize>,
    /// Déjà mis en mots : ce qui s'affiche dans la barre.
    pub status: &'a str,
}

/// Ce que l'interface demande à la fenêtre de sortie. Ce n'est pas une
/// `Command` : la grille n'a rien à en savoir, et seul `app` possède les
/// fenêtres (§8.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputRequest {
    /// Ouvre la sortie sur cet écran, en la déplaçant si elle est déjà ouverte.
    Open(usize),
    Close,
}

/// Annuler ou rétablir : `app` tient l'historique (§23.11).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HistoryRequest {
    Undo,
    Redo,
}

/// Ce que l'historique permet, en mots.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HistoryView {
    pub undo: Option<&'static str>,
    pub redo: Option<&'static str>,
}

/// Ctrl+Z, puis Ctrl+Y ou Ctrl+Maj+Z. Hors d'un champ, qui garde les siens.
pub fn history_shortcut(ui: &mut Ui) -> Option<HistoryRequest> {
    if ui.ctx().egui_wants_keyboard_input() {
        return None;
    }
    let ctrl = egui::Modifiers::COMMAND;
    let raccourci = |m, k| egui::KeyboardShortcut::new(m, k);
    ui.input_mut(|i| {
        // Le plus précis d'abord : Ctrl+Z accepterait aussi Ctrl+Maj+Z.
        if i.consume_shortcut(&raccourci(ctrl | egui::Modifiers::SHIFT, egui::Key::Z))
            || i.consume_shortcut(&raccourci(ctrl, egui::Key::Y))
        {
            Some(HistoryRequest::Redo)
        } else if i.consume_shortcut(&raccourci(ctrl, egui::Key::Z)) {
            Some(HistoryRequest::Undo)
        } else {
            None
        }
    })
}

/// L'état du fichier, déjà mis en mots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileView<'a> {
    /// Le nom du fichier, suivi de `*` s'il reste des modifications.
    pub title: &'a str,
    /// Ce qui vient de se passer : un enregistrement, un refus.
    pub status: &'a str,
    /// Il reste des modifications : la question attend sa réponse.
    pub confirming: bool,
}

/// La réponse à la question des modifications non enregistrées.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Confirmation {
    Save,
    Discard,
    Cancel,
}

/// Ce que l'interface demande au fichier. Le parcours est décidé par `app`
/// (§23.8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileRequest {
    New,
    Open,
    Save,
    SaveAs,
    Confirm(Confirmation),
}

/// Enregistrer : le seul raccourci du fichier. Ouvrir et remplacer la grille
/// restent dans le menu : sur scène, une touche ne doit pas changer l'image.
const SAVE_SHORTCUT: egui::KeyboardShortcut =
    egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::S);

/// Ce que l'interface se rappelle d'une frame à l'autre.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UiState {
    /// Le clip affiché dans les réglages, et sa piste.
    pub selected: Option<(LayerId, ClipId)>,
    pub show_grid: bool,
    pub show_monitor: bool,
    pub show_settings: bool,
    pub show_library: bool,
    pub show_audio: bool,
    /// L'utilisateur a demandé l'éditeur de nœuds : `app` ouvre sa fenêtre.
    pub open_nodes: bool,
    /// Annuler ou rétablir, demandé cette frame.
    pub history: Option<HistoryRequest>,
    /// Le trig réglé dans le panneau flottant ; aucun : le spectre reste dégagé.
    pub selected_trig: Option<TrigId>,
    /// Le geste en cours sur le spectre.
    pub zone_gesture: Option<ZoneGesture>,
    /// Les trigs vus à la frame d'avant : un de plus, et il devient le trig
    /// choisi. `None` avant la première frame d'une grille, dont les trigs ne
    /// viennent pas de naître.
    pub known_trigs: Option<usize>,
    /// La courbe de crêtes du spectre.
    pub peak_hold: [f32; SPECTRUM_BANDS],
}

impl UiState {
    /// La grille a été remplacée : ses identifiants visaient l'ancienne, et
    /// pourraient coïncider avec ceux de la nouvelle (§23.8).
    /// La grille a été restaurée : la sélection suit, ou disparaît avec son clip.
    pub fn remap(&mut self, map: &IdMap) {
        self.selected = self
            .selected
            .and_then(|(layer, clip)| Some((map.layer(layer)?, map.clip(clip)?)));
        self.selected_trig = self.selected_trig.and_then(|t| map.trig(t));
        self.zone_gesture = None;
    }

    pub fn forget_grid(&mut self) {
        self.selected = None;
        self.selected_trig = None;
        self.zone_gesture = None;
        self.known_trigs = None;
    }
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            selected: None,
            show_grid: true,
            show_monitor: true,
            show_settings: true,
            show_library: true,
            show_audio: true,
            open_nodes: false,
            history: None,
            selected_trig: None,
            zone_gesture: None,
            known_trigs: None,
            peak_hold: [0.0; SPECTRUM_BANDS],
        }
    }
}

/// Le monitor : une texture déjà connue d'egui, et ses proportions.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Monitor {
    pub texture: egui::TextureId,
    pub aspect: f32,
}

/// Ce qu'`app` prête à l'interface pour une frame.
pub struct FrameView<'a> {
    pub composition: &'a Composition,
    pub bpm: f32,
    pub fps: f32,
    /// Le statut des sorties, déjà mis en mots.
    pub outputs: &'a str,
    pub monitor: Option<Monitor>,
    /// Le spectre de la dernière frame.
    pub spectrum: &'a Spectrum,
    /// L'état de l'entrée audio, déjà mis en mots.
    pub audio_status: &'a str,
    pub output: OutputView<'a>,
    pub file: FileView<'a>,
    /// L'instant de la dernière frame, en secondes.
    pub time: f64,
    /// Le tempo détecté dans le son, `0` s'il n'y en a pas (§7.1).
    pub detected_bpm: f32,
    pub history: HistoryView,
}

/// Dessine toute l'interface et remplit `commands`, `placements`, `output` et
/// `file`.
pub fn draw(
    ui: &mut Ui,
    state: &mut UiState,
    view: &FrameView<'_>,
    commands: &mut Vec<Command>,
    placements: &mut Vec<Placement>,
    output: &mut Option<OutputRequest>,
    file: &mut Option<FileRequest>,
) {
    if view.file.confirming {
        confirmation(ui, view.file.title, file);
    } else if ui.input_mut(|i| i.consume_shortcut(&SAVE_SHORTCUT)) {
        *file = Some(FileRequest::Save);
    } else if let Some(demande) = history_shortcut(ui) {
        state.history = Some(demande);
    }
    let entete = egui::Frame::new()
        .fill(theme::HEADER)
        .inner_margin(Margin::symmetric(10, 5));
    egui::Panel::top("menu")
        .frame(entete)
        .show(ui, |ui| header(ui, state, view, commands, output, file));
    if state.show_grid {
        egui::Panel::top("grille")
            .resizable(true)
            .default_size(226.0)
            .show(ui, |ui| {
                grid(ui, state, view.composition, commands, placements)
            });
    }
    if state.show_audio {
        egui::Panel::bottom("audio")
            .resizable(true)
            .default_size(300.0)
            .show(ui, |ui| spectrum::panel(ui, state, view, commands));
    }
    if state.show_library {
        egui::Panel::right("bibliotheque")
            .resizable(true)
            .default_size(170.0)
            .show(ui, library);
    }
    egui::CentralPanel::default()
        .frame(egui::Frame::new())
        .show(ui, |ui| stage(ui, state, view, commands));
}

/// La pulsation du tempo : 1 sur le temps, puis elle retombe. Elle dit le tempo,
/// pas la place dans la mesure : rien ne recale encore la phase (§3bis.3).
pub fn beat_pulse(time: f64, bpm: f32) -> f32 {
    let battements = time * bpm.max(0.0) as f64 / 60.0;
    let phase = battements - battements.floor();
    (1.0 - phase as f32).powi(3)
}

/// Le nom, comme le haut de Pro-Q : d'un seul trait, l'accent à la fin.
fn wordmark(ui: &mut Ui) {
    ui.spacing_mut().item_spacing.x = 0.0;
    ui.add(Label::new(RichText::new("Frogen").size(15.0).color(theme::TEXT)).selectable(false));
    ui.add(
        Label::new(RichText::new("X").size(15.0).color(theme::ACCENT).strong()).selectable(false),
    );
    ui.spacing_mut().item_spacing.x = 8.0;
}

/// Le tempo, au plus près des menus : c'est ce qu'on touche le plus sur scène.
/// Ce que le son dit du tempo, à côté de `Suivre`.
fn detected_label(bpm: f32) -> String {
    if bpm > 0.0 {
        format!("son : {bpm:.1}")
    } else {
        "son : —".to_string()
    }
}

fn tempo(ui: &mut Ui, view: &FrameView<'_>, commands: &mut Vec<Command>) {
    let mut bpm = view.bpm;
    let reglage = egui::DragValue::new(&mut bpm)
        .range(20.0..=400.0)
        .speed(0.5)
        .suffix(" BPM");
    if ui.add(reglage).changed() {
        commands.push(Command::SetBpm(bpm));
    }
    if ui.button("Tap").clicked() {
        commands.push(Command::TapTempo);
    }
    let suit = view.composition.tempo_follow;
    if ui.selectable_label(suit, "Suivre").clicked() {
        commands.push(Command::SetTempoFollow(!suit));
    }
    ui.label(
        RichText::new(detected_label(view.detected_bpm))
            .color(theme::TEXT_DIM)
            .size(11.5),
    );
    let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), Sense::hover());
    let pulsation = beat_pulse(view.time, view.bpm);
    ui.painter().circle_filled(rect.center(), 4.0, theme::IDLE);
    ui.painter()
        .circle_filled(rect.center(), 4.0, theme::ACCENT.gamma_multiply(pulsation));
}

/// Une ligne, comme l'en-tête de Pro-Q : le nom et les menus à gauche, le fichier
/// au milieu, l'état de la machine à droite.
fn header(
    ui: &mut Ui,
    state: &mut UiState,
    view: &FrameView<'_>,
    commands: &mut Vec<Command>,
    output: &mut Option<OutputRequest>,
    file: &mut Option<FileRequest>,
) {
    let entete = ui.max_rect();
    egui::MenuBar::new().ui(ui, |ui| {
        wordmark(ui);
        ui.add_space(14.0);
        ui.menu_button("Fichier", |ui| file_menu(ui, file));
        ui.menu_button("Édition", |ui| history_menu(ui, state, view.history));
        ui.menu_button("Affichage", |ui| {
            ui.checkbox(&mut state.show_grid, "Grille");
            ui.checkbox(&mut state.show_monitor, "Monitor");
            ui.checkbox(&mut state.show_settings, "Réglages");
            ui.checkbox(&mut state.show_audio, "Audio");
            ui.checkbox(&mut state.show_library, "Bibliothèque");
            ui.separator();
            if ui.button("Éditeur de nœuds").clicked() {
                state.open_nodes = true;
                ui.close();
            }
        });
        ui.menu_button("Sortie", |ui| output_menu(ui, &view.output, output));
        ui.add_space(14.0);
        tempo(ui, view, commands);

        // Centré sur toute la fenêtre, pas sur la place laissée par les menus.
        let police = egui::TextStyle::Body.resolve(ui.style());
        let largeur = |texte: &str, police: egui::FontId| {
            ui.painter()
                .layout_no_wrap(texte.to_string(), police, theme::TEXT)
                .size()
                .x
        };
        let mut total = largeur(view.file.title, police);
        if !view.file.status.is_empty() {
            let petit = egui::FontId::proportional(STATUS_SIZE);
            total += ui.spacing().item_spacing.x + largeur(view.file.status, petit);
        }
        let centre = Rect::from_center_size(
            Pos2::new(entete.center().x, ui.max_rect().center().y),
            egui::vec2(total.clamp(1.0, TITLE_WIDTH), ui.max_rect().height()),
        );
        let ligne = Layout::left_to_right(Align::Center);
        ui.scope_builder(UiBuilder::new().max_rect(centre).layout(ligne), |ui| {
            ui.label(RichText::new(view.file.title).color(theme::TEXT));
            if !view.file.status.is_empty() {
                let etat = RichText::new(view.file.status)
                    .color(theme::TEXT_DIM)
                    .size(STATUS_SIZE);
                ui.label(etat);
            }
        });

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let discret = |texte: &str| RichText::new(texte).color(theme::TEXT_DIM).size(11.5);
            ui.label(discret(&format!("{:.0} fps", view.fps)));
            ui.label(discret(view.outputs));
            ui.label(discret(view.output.status));
        });
    });
}

/// Nomme ce qui va changer : un geste à l'aveugle ne se rattrape pas sur scène.
fn history_menu(ui: &mut Ui, state: &mut UiState, view: HistoryView) {
    let entrees = [
        ("Annuler", view.undo, HistoryRequest::Undo),
        ("Rétablir", view.redo, HistoryRequest::Redo),
    ];
    for (verbe, nom, demande) in entrees {
        let libelle = nom.map_or(verbe.to_string(), |n| format!("{verbe} : {n}"));
        if ui
            .add_enabled(nom.is_some(), Button::new(libelle))
            .clicked()
        {
            state.history = Some(demande);
            ui.close();
        }
    }
}

fn file_menu(ui: &mut Ui, file: &mut Option<FileRequest>) {
    let items = [
        ("Nouveau", FileRequest::New),
        ("Ouvrir…", FileRequest::Open),
        ("Enregistrer", FileRequest::Save),
        ("Enregistrer sous…", FileRequest::SaveAs),
    ];
    for (libelle, demande) in items {
        if ui.button(libelle).clicked() {
            *file = Some(demande);
            ui.close();
        }
    }
}

/// Avant de perdre des modifications. Modale : rien d'autre ne se clique, et
/// un clic à côté ne vaut pas réponse.
fn confirmation(ui: &Ui, title: &str, file: &mut Option<FileRequest>) {
    egui::Modal::new(ui.scope_id().with("confirmation")).show(ui.ctx(), |ui| {
        ui.label(format!(
            "« {} » a des modifications non enregistrées.",
            title.trim_end_matches(" *")
        ));
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            let choix = [
                ("Enregistrer", Confirmation::Save),
                ("Ne pas enregistrer", Confirmation::Discard),
                ("Annuler", Confirmation::Cancel),
            ];
            for (libelle, reponse) in choix {
                if ui.button(libelle).clicked() {
                    *file = Some(FileRequest::Confirm(reponse));
                }
            }
        });
    });
}

/// Un écran par ligne. Celui de la sortie porte la sélection, et le recliquer
/// ne fait rien : rouvrir la fenêtre couperait l'image pour rien.
fn output_menu(ui: &mut Ui, view: &OutputView<'_>, output: &mut Option<OutputRequest>) {
    if view.screens.is_empty() {
        ui.label(RichText::new("Aucun écran détecté").color(theme::TEXT_DIM));
    }
    for (i, screen) in view.screens.iter().enumerate() {
        let ouverte = view.open == Some(i);
        let libelle = if screen.interface {
            format!("{} : en fenêtre", screen.label)
        } else {
            screen.label.clone()
        };
        if ui.selectable_label(ouverte, libelle).clicked() {
            if !ouverte {
                *output = Some(OutputRequest::Open(i));
            }
            ui.close();
        }
    }
    ui.separator();
    let fermer = Button::new("Fermer la sortie");
    if ui.add_enabled(view.open.is_some(), fermer).clicked() {
        *output = Some(OutputRequest::Close);
        ui.close();
    }
}

/// L'avancement de la boucle d'un clip, dans `[0, 1)`. Un clip en temps absolu
/// n'a pas de boucle : `None`.
pub fn loop_progress(clip: &Clip) -> Option<f32> {
    let Transport::BpmSync {
        position,
        duration_beats,
        ..
    } = clip.transport
    else {
        return None;
    };
    let graphe = clip.graph();
    let duree = duration_beats.get(graphe);
    (duree > 0.0).then(|| (graphe.value(position) / duree).rem_euclid(1.0))
}

/// Une cellule : l'accent n'y désigne que le clip actif (§23.1), et son bas
/// montre où en est sa boucle, comme le spectre montre le son.
fn cell(ui: &mut Ui, name: &str, active: bool, progress: Option<f32>) -> Response {
    let couleur = if active {
        theme::ON_ACCENT
    } else {
        theme::TEXT
    };
    let fond = ui.painter().add(egui::Shape::Noop);
    let bouton = Button::new(RichText::new(name).color(couleur))
        .fill(egui::Color32::TRANSPARENT)
        .min_size(CELL_SIZE);
    let reponse = ui.add(bouton);
    let rect = reponse.rect;
    let rayon = CornerRadius::same(6);
    let survol = reponse.hovered();
    let surface = match (active, survol) {
        (true, _) => theme::ACCENT,
        (false, true) => theme::HOVER,
        (false, false) => theme::IDLE,
    };
    ui.painter()
        .set(fond, egui::Shape::rect_filled(rect, rayon, surface));
    if active {
        // La lumière d'un pad allumé, sans bouger ses voisins.
        ui.painter().rect_stroke(
            rect.expand(1.5),
            7.0,
            Stroke::new(2.0, theme::ACCENT.gamma_multiply(0.3)),
            egui::StrokeKind::Outside,
        );
        if let Some(t) = progress {
            let mut barre = rect.shrink2(egui::vec2(6.0, 4.0));
            barre.set_top(barre.bottom() - PROGRESS_HEIGHT);
            ui.painter()
                .rect_filled(barre, 1.5, theme::ON_ACCENT.gamma_multiply(0.25));
            barre.set_right(barre.left() + t.clamp(0.0, 1.0) * barre.width());
            ui.painter().rect_filled(barre, 1.5, theme::ON_ACCENT);
        }
    }
    reponse
}

/// Une case vide : elle ne déclenche rien, elle reçoit. Un simple contour : la
/// grille se lit par ce qu'elle contient.
fn empty_cell(ui: &mut Ui) -> Response {
    let bouton = Button::new(RichText::new(EMPTY_CELL).color(theme::TEXT_DIM))
        .fill(egui::Color32::TRANSPARENT)
        .stroke(Stroke::new(1.0, theme::GRID))
        .min_size(CELL_SIZE);
    ui.add(bouton)
}

/// Une case reçoit un clip glissé depuis la bibliothèque : le contour blanc
/// indique où il tombera, l'accent restant réservé au clip actif.
fn drop_target(ui: &Ui, reponse: &Response) -> Option<PresetId> {
    if reponse.dnd_hover_payload::<PresetId>().is_some() {
        ui.painter().rect_stroke(
            reponse.rect,
            2.0,
            Stroke::new(2.0, theme::TEXT),
            egui::StrokeKind::Inside,
        );
    }
    reponse.dnd_release_payload::<PresetId>().map(|id| *id)
}

/// Retirer passe par un menu : sur scène, un clic raté ne supprime rien (§23.6).
fn remove_menu(reponse: &Response, libelle: &str, command: Command, commands: &mut Vec<Command>) {
    reponse.context_menu(|ui| {
        if ui.button(libelle).clicked() {
            commands.push(command);
            ui.close();
        }
    });
}

/// Les pistes, la plus haute en haut, comme dans la pile de composition. Une
/// case par colonne : un clip reste sous son en-tête (§3bis.1).
fn grid(
    ui: &mut Ui,
    state: &mut UiState,
    c: &Composition,
    commands: &mut Vec<Command>,
    placements: &mut Vec<Placement>,
) {
    let mut pistes = Vec::new();
    c.layers_in_order(&mut pistes);
    egui::ScrollArea::both().show(ui, |ui| {
        egui::Grid::new("grille_clips").show(ui, |ui| {
            ui.label("");
            for (i, column) in c.columns().iter().enumerate() {
                let titre = RichText::new(format!("Colonne {}", i + 1))
                    .size(11.5)
                    .color(theme::TEXT_DIM);
                let bouton = Button::new(titre)
                    .fill(theme::HEADER)
                    .min_size(egui::vec2(CELL_SIZE.x, 0.0));
                let entete = ui.add(bouton);
                if entete.clicked() {
                    commands.push(Command::TriggerColumn { column: *column });
                }
                let retire = Command::RemoveColumn { column: *column };
                remove_menu(&entete, "Retirer la colonne", retire, commands);
            }
            if ui.button("+ colonne").clicked() {
                commands.push(Command::AddColumn);
            }
            ui.end_row();

            for layer in pistes.iter().rev() {
                let Some(piste) = c.layer(*layer) else {
                    continue;
                };
                let nom = Label::new(RichText::new(&piste.name).color(theme::TEXT_DIM))
                    .selectable(false)
                    .sense(Sense::click());
                let retire = Command::RemoveLayer { layer: *layer };
                remove_menu(&ui.add(nom), "Retirer la piste", retire, commands);

                for (i, column) in c.columns().iter().enumerate() {
                    let clip = piste.cell(i).and_then(|id| Some((id, c.clip(id)?)));
                    let reponse = match clip {
                        Some((id, clip)) => {
                            let actif = piste.active_clip() == Some(id);
                            let boucle = loop_progress(clip).filter(|_| actif);
                            let reponse = cell(ui, &clip.name, actif, boucle);
                            if reponse.clicked() {
                                commands.push(Command::TriggerClip {
                                    layer: *layer,
                                    clip: id,
                                });
                                state.selected = Some((*layer, id));
                            }
                            let retire = Command::RemoveClip {
                                layer: *layer,
                                column: *column,
                            };
                            remove_menu(&reponse, "Retirer le clip", retire, commands);
                            reponse
                        }
                        None => empty_cell(ui),
                    };
                    if let Some(preset) = drop_target(ui, &reponse) {
                        placements.push(Placement {
                            layer: *layer,
                            column: *column,
                            preset,
                        });
                    }
                }
                ui.end_row();
            }
            if ui.button("+ piste").clicked() {
                commands.push(Command::AddLayer);
            }
            ui.end_row();
        });
    });
}

/// Le catalogue, par famille. Une entrée se glisse sur une case.
fn library(ui: &mut Ui) {
    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        let aide = RichText::new("Glisse un clip sur une case.")
            .size(11.0)
            .color(theme::TEXT_DIM);
        ui.label(aide);
        for family in Family::ALL {
            ui.add_space(8.0);
            ui.label(
                RichText::new(family.label())
                    .size(11.0)
                    .color(theme::ACCENT),
            );
            for (id, preset) in presets().filter(|(_, p)| p.family == family) {
                ui.dnd_drag_source(ui.scope_id().with(preset.name), id, |ui| {
                    let ligne = egui::Frame::new()
                        .fill(theme::HEADER)
                        .corner_radius(4)
                        .inner_margin(Margin::symmetric(8, 1));
                    ligne.show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.add(Label::new(preset.name).selectable(false));
                    });
                });
            }
        }
    });
}

/// Le plus grand rectangle aux proportions `aspect` dans `available`.
fn fit(available: egui::Vec2, aspect: f32) -> egui::Vec2 {
    if aspect <= 0.0 || available.x <= 0.0 || available.y <= 0.0 {
        return egui::Vec2::ZERO;
    }
    let largeur = available.x.min(available.y * aspect);
    egui::vec2(largeur, largeur / aspect)
}

/// Le partage de la scène : l'image, puis ce qui reste pour le panneau du clip.
/// L'image est calée à gauche et ne dépend pas de la sélection : choisir un clip ne
/// doit pas la faire sauter en plein set. Trop étroite, la scène lui laisse tout,
/// et le panneau se pose dessus.
pub fn stage_layout(cadre: Rect, aspect: f32) -> (Rect, Rect) {
    let marge = cadre.shrink(10.0);
    let cote_a_cote = cadre.width() >= clip_panel::MIN_SIDE_BY_SIDE;
    let reserve = if cote_a_cote {
        clip_panel::RESERVED_WIDTH + 10.0
    } else {
        0.0
    };
    let place = egui::vec2((marge.width() - reserve).max(0.0), marge.height());
    let taille = fit(place, aspect);
    let image = Rect::from_min_size(
        Pos2::new(marge.left(), marge.center().y - taille.y * 0.5),
        taille,
    );
    let mut panneau = marge;
    if cote_a_cote {
        panneau.set_left(image.right() + 10.0);
    }
    (image, panneau)
}

/// La scène : l'image de sortie, entière, et les réglages du clip choisi à côté.
/// Un clic ailleurs les referme, comme un clic dans le vide du spectre.
fn stage(ui: &mut Ui, state: &mut UiState, view: &FrameView<'_>, commands: &mut Vec<Command>) {
    let (cadre, fond) = ui.allocate_exact_size(ui.available_size(), Sense::click());
    ui.painter().add(egui::Shape::mesh(widgets::gradient(
        cadre,
        theme::PLOT_TOP,
        theme::PLOT_BOTTOM,
    )));
    if fond.clicked() {
        state.selected = None;
    }

    let aspect = view.monitor.map_or(STAGE_ASPECT, |m| m.aspect);
    let (image, panneau) = stage_layout(cadre, aspect);
    if state.show_monitor {
        match view.monitor {
            Some(m) => {
                egui::Image::new((m.texture, image.size()))
                    .corner_radius(6)
                    .paint_at(ui, image);
                ui.painter().rect_stroke(
                    image,
                    6.0,
                    Stroke::new(1.0, theme::GRID_STRONG),
                    egui::StrokeKind::Outside,
                );
            }
            None => {
                ui.put(image, Label::new("Monitor indisponible").selectable(false));
            }
        }
    }

    if !state.show_settings {
        return;
    }
    let selection = state.selected.filter(|(layer, id)| {
        view.composition.layer(*layer).is_some() && view.composition.clip(*id).is_some()
    });
    match selection {
        Some(choix) => clip_panel::floating(ui, panneau, state, view, commands, choix),
        None => {
            let aide = RichText::new("Clique un clip pour le régler.")
                .size(11.5)
                .color(theme::TEXT_DIM);
            ui.put(panneau, Label::new(aide).selectable(false));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::kittest::Queryable;
    use egui_kittest::Harness;
    use frogenx_composition::{Clip, ParamSlot, PlayMode, Transport, TrigSlot, TrigTarget};
    use frogenx_core::{AmpScale, AnalysisMethod, Constant, FreqScale, Graph, Param};
    use frogenx_library::find;
    use frogenx_shape::{ColorParam, Ellipse, ShapeStyle};

    struct Grille {
        composition: Composition,
        piste: LayerId,
        clips: [ClipId; 2],
        colonnes: [ColumnId; 3],
    }

    /// Une piste : Alpha, Beta, puis une case vide.
    fn grille() -> Grille {
        let mut c = Composition::new();
        let deck = c.add_deck("test");
        let groupe = c.add_layer_group(deck).unwrap();
        let piste = c.add_layer(groupe).unwrap();
        let clip = |c: &mut Composition, nom: &str| {
            let mut g = Graph::new();
            let transport = Transport::bpm_sync(&mut g, Param::fixed(4.0), PlayMode::Loop);
            let clip = Clip::shape(g, Box::new(Ellipse::circle(0.5)), transport)
                .with_name(nom)
                .with_style(ShapeStyle::filled(ColorParam::srgb(1.0, 0.0, 0.0, 1.0)));
            c.add_clip(piste, clip).unwrap()
        };
        let (a, b) = (clip(&mut c, "Alpha"), clip(&mut c, "Beta"));
        c.add_column();
        let colonnes = [c.columns()[0], c.columns()[1], c.columns()[2]];
        Grille {
            composition: c,
            piste,
            clips: [a, b],
            colonnes,
        }
    }

    struct Etat {
        ui: UiState,
        commands: Vec<Command>,
        placements: Vec<Placement>,
        output: Option<OutputRequest>,
        file: Option<FileRequest>,
    }

    /// Une interface sans écran, qui garde les commandes émises.
    fn harness(composition: &Composition) -> Harness<'_, Etat> {
        harness_sortie(composition, Vec::new(), None, "Sortie fermée")
    }

    /// Le même, la question des modifications posée ou non.
    fn harness_fichier(composition: &Composition, confirming: bool) -> Harness<'_, Etat> {
        harness_complet(composition, Vec::new(), None, "Sortie fermée", confirming)
    }

    /// Le même, avec un état de la sortie.
    fn harness_sortie<'a>(
        composition: &'a Composition,
        ecrans: Vec<Screen>,
        ouverte: Option<usize>,
        statut: &'a str,
    ) -> Harness<'a, Etat> {
        harness_complet(composition, ecrans, ouverte, statut, false)
    }

    fn harness_complet<'a>(
        composition: &'a Composition,
        ecrans: Vec<Screen>,
        ouverte: Option<usize>,
        statut: &'a str,
        confirming: bool,
    ) -> Harness<'a, Etat> {
        let etat = Etat {
            ui: UiState::default(),
            commands: Vec::new(),
            placements: Vec::new(),
            output: None,
            file: None,
        };
        let spectre = Spectrum::default();
        Harness::builder()
            .with_size(egui::vec2(1400.0, 1100.0))
            .build_ui_state(
                move |ui, etat: &mut Etat| {
                    let view = FrameView {
                        composition,
                        bpm: 120.0,
                        fps: 60.0,
                        outputs: "Monitor : actif",
                        monitor: None,
                        time: 0.0,
                        spectrum: &spectre,
                        audio_status: "Micro — 48000 Hz",
                        output: OutputView {
                            screens: &ecrans,
                            open: ouverte,
                            status: statut,
                        },
                        file: FileView {
                            title: "concert.frogenx *",
                            status: "Enregistré : concert.frogenx",
                            confirming,
                        },
                        detected_bpm: 127.5,
                        history: HistoryView {
                            undo: Some("Câblage"),
                            redo: None,
                        },
                    };
                    draw(
                        ui,
                        &mut etat.ui,
                        &view,
                        &mut etat.commands,
                        &mut etat.placements,
                        &mut etat.output,
                        &mut etat.file,
                    );
                },
                etat,
            )
    }

    /// Le champ numérique d'un curseur, qui accepte une valeur simulée. Le
    /// curseur lui-même porte le même nom.
    fn champ<'h, 'a>(h: &'h Harness<'a, Etat>, nom: &'h str) -> egui_kittest::Node<'h> {
        h.get_by_role_and_label(egui::accesskit::Role::SpinButton, nom)
    }

    #[test]
    fn un_clic_sur_une_cellule_declenche_le_clip_et_le_selectionne() {
        let g = grille();
        let mut h = harness(&g.composition);
        h.get_by_label("Beta").click();
        h.run();

        assert_eq!(
            h.state().commands,
            vec![Command::TriggerClip {
                layer: g.piste,
                clip: g.clips[1]
            }]
        );
        assert_eq!(h.state().ui.selected, Some((g.piste, g.clips[1])));
    }

    #[test]
    fn un_clic_sur_un_en_tete_declenche_la_colonne() {
        let g = grille();
        let mut h = harness(&g.composition);
        h.get_by_label("Colonne 1").click();
        h.run();
        assert_eq!(
            h.state().commands,
            vec![Command::TriggerColumn {
                column: g.colonnes[0]
            }]
        );
        assert_eq!(
            h.state().ui.selected,
            None,
            "une colonne ne choisit aucun clip"
        );
    }

    #[test]
    fn le_bouton_tap_tape_le_tempo() {
        let g = grille();
        let mut h = harness(&g.composition);
        h.get_by_label("Tap").click();
        h.run();
        assert_eq!(h.state().commands, vec![Command::TapTempo]);
    }

    #[test]
    fn un_clip_selectionne_montre_ses_reglages_et_un_curseur_emet_une_commande() {
        let g = grille();
        let mut h = harness(&g.composition);
        h.get_by_label("Alpha").click();
        h.run();
        h.state_mut().commands.clear();

        champ(&h, "RAYON").set_value_accesskit(0.8);
        h.run();
        assert_eq!(
            h.state().commands,
            vec![Command::SetParam {
                clip: g.clips[0],
                slot: ParamSlot::Generator(0),
                value: 0.8
            }]
        );
    }

    #[test]
    fn le_curseur_d_opacite_regle_la_piste() {
        let g = grille();
        let mut h = harness(&g.composition);
        h.get_by_label("Alpha").click();
        h.run();
        h.state_mut().commands.clear();

        champ(&h, "OPACITÉ").set_value_accesskit(0.5);
        h.run();
        assert_eq!(
            h.state().commands,
            vec![Command::SetLayerOpacity {
                layer: g.piste,
                opacity: Param::fixed(0.5)
            }]
        );
    }

    /// L'interface sur son écran, et un projecteur.
    fn deux_ecrans() -> Vec<Screen> {
        vec![
            Screen {
                label: "Écran 1".into(),
                interface: true,
            },
            Screen {
                label: "Écran 2".into(),
                interface: false,
            },
        ]
    }

    #[test]
    fn le_menu_sortie_ouvre_la_sortie_sur_l_ecran_choisi() {
        let g = grille();
        let mut h = harness_sortie(&g.composition, deux_ecrans(), None, "Sortie fermée");
        h.get_by_label("Sortie").click();
        h.run();
        h.get_by_label("Écran 2").click();
        h.run();

        assert_eq!(h.state().output, Some(OutputRequest::Open(1)));
        assert!(h.state().commands.is_empty(), "la grille n'en sait rien");
    }

    #[test]
    fn le_menu_dit_que_l_ecran_de_l_interface_ouvre_une_fenetre() {
        let g = grille();
        let mut h = harness_sortie(&g.composition, deux_ecrans(), None, "Sortie fermée");
        h.get_by_label("Sortie").click();
        h.run();
        h.get_by_label("Écran 1 : en fenêtre").click();
        h.run();
        assert_eq!(h.state().output, Some(OutputRequest::Open(0)));
    }

    #[test]
    fn un_autre_ecran_deplace_la_sortie_et_le_meme_ne_la_rouvre_pas() {
        let g = grille();
        let mut h = harness_sortie(&g.composition, deux_ecrans(), Some(1), "Sortie : Écran 2");
        h.get_by_label("Sortie").click();
        h.run();
        h.get_by_label("Écran 2").click();
        h.run();
        assert_eq!(h.state().output, None, "le rouvrir couperait l'image");

        h.get_by_label("Sortie").click();
        h.run();
        h.get_by_label("Écran 1 : en fenêtre").click();
        h.run();
        assert_eq!(h.state().output, Some(OutputRequest::Open(0)));
    }

    #[test]
    fn fermer_la_sortie_passe_par_le_menu() {
        let g = grille();
        let mut h = harness_sortie(&g.composition, deux_ecrans(), Some(1), "Sortie : Écran 2");
        h.get_by_label("Sortie").click();
        h.run();
        h.get_by_label("Fermer la sortie").click();
        h.run();
        assert_eq!(h.state().output, Some(OutputRequest::Close));
    }

    #[test]
    fn une_sortie_fermee_ne_se_ferme_pas_une_seconde_fois() {
        let g = grille();
        let mut h = harness_sortie(&g.composition, deux_ecrans(), None, "Sortie fermée");
        h.get_by_label("Sortie").click();
        h.run();
        h.get_by_label("Fermer la sortie").click();
        h.run();
        assert_eq!(h.state().output, None);
    }

    #[test]
    fn sans_ecran_le_menu_le_dit() {
        let g = grille();
        let mut h = harness(&g.composition);
        h.get_by_label("Sortie").click();
        h.run();
        assert!(h.query_by_label("Aucun écran détecté").is_some());
    }

    #[test]
    fn la_barre_dit_ce_que_devient_la_sortie() {
        let g = grille();
        let statut = "Sortie fermée : écran débranché";
        let h = harness_sortie(&g.composition, deux_ecrans(), None, statut);
        assert!(h.query_by_label(statut).is_some());
    }

    fn clique_fichier(libelle: &str) -> Option<FileRequest> {
        let g = grille();
        let mut h = harness(&g.composition);
        h.get_by_label("Fichier").click();
        h.run();
        h.get_by_label(libelle).click();
        h.run();
        assert!(h.state().commands.is_empty(), "la grille n'en sait rien");
        h.state().file
    }

    #[test]
    fn le_menu_fichier_emet_chaque_demande() {
        assert_eq!(clique_fichier("Nouveau"), Some(FileRequest::New));
        assert_eq!(clique_fichier("Ouvrir…"), Some(FileRequest::Open));
        assert_eq!(clique_fichier("Enregistrer"), Some(FileRequest::Save));
        assert_eq!(
            clique_fichier("Enregistrer sous…"),
            Some(FileRequest::SaveAs)
        );
    }

    #[test]
    fn ctrl_s_enregistre() {
        let g = grille();
        let mut h = harness(&g.composition);
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::S);
        h.run();
        assert_eq!(h.state().file, Some(FileRequest::Save));
    }

    #[test]
    fn la_barre_dit_le_fichier_et_ce_qui_vient_de_se_passer() {
        let g = grille();
        let h = harness(&g.composition);
        assert!(h.query_by_label("concert.frogenx *").is_some());
        assert!(h.query_by_label("Enregistré : concert.frogenx").is_some());
    }

    #[test]
    fn sans_question_la_fenetre_n_apparait_pas() {
        let g = grille();
        let h = harness_fichier(&g.composition, false);
        assert!(h.query_by_label("Ne pas enregistrer").is_none());
    }

    #[test]
    fn la_question_nomme_le_fichier_et_emet_la_reponse() {
        for (libelle, reponse) in [
            ("Enregistrer", Confirmation::Save),
            ("Ne pas enregistrer", Confirmation::Discard),
            ("Annuler", Confirmation::Cancel),
        ] {
            let g = grille();
            let mut h = harness_fichier(&g.composition, true);
            h.run();
            assert!(h
                .query_by_label("« concert.frogenx » a des modifications non enregistrées.")
                .is_some());
            h.get_by_label(libelle).click();
            h.run();
            assert_eq!(h.state().file, Some(FileRequest::Confirm(reponse)));
        }
    }

    #[test]
    fn la_question_empeche_de_cliquer_la_grille() {
        let g = grille();
        let mut h = harness_fichier(&g.composition, true);
        h.run();
        h.get_by_label("Alpha").click();
        h.run();
        assert!(h.state().commands.is_empty());
    }

    #[test]
    fn oublier_la_grille_oublie_la_selection() {
        let g = grille();
        let mut etat = UiState {
            selected: Some((g.piste, g.clips[0])),
            zone_gesture: Some(ZoneGesture::Draw {
                from: (0.1, 0.5),
                to: (0.2, 0.6),
            }),
            known_trigs: Some(3),
            ..UiState::default()
        };
        etat.forget_grid();
        assert_eq!(etat.selected, None);
        assert_eq!(etat.zone_gesture, None);
        assert_eq!(etat.known_trigs, None);
    }

    #[test]
    fn le_menu_edition_nomme_ce_qu_il_annule() {
        let g = grille();
        let mut h = harness(&g.composition);
        h.get_by_label("Édition").click();
        h.run();
        h.get_by_label("Annuler : Câblage").click();
        h.run();
        assert_eq!(h.state().ui.history, Some(HistoryRequest::Undo));
        assert!(h.state().commands.is_empty());
    }

    #[test]
    fn rien_a_retablir_laisse_le_menu_inactif() {
        let g = grille();
        let mut h = harness(&g.composition);
        h.get_by_label("Édition").click();
        h.run();
        h.get_by_label("Rétablir").click();
        h.run();
        assert_eq!(h.state().ui.history, None);
    }

    #[test]
    fn les_raccourcis_annulent_et_retablissent() {
        let ctrl = egui::Modifiers::COMMAND;
        for (mods, touche, attendu) in [
            (ctrl, egui::Key::Z, HistoryRequest::Undo),
            (ctrl, egui::Key::Y, HistoryRequest::Redo),
            (
                ctrl | egui::Modifiers::SHIFT,
                egui::Key::Z,
                HistoryRequest::Redo,
            ),
        ] {
            let g = grille();
            let mut h = harness(&g.composition);
            h.key_press_modifiers(mods, touche);
            h.run();
            assert_eq!(h.state().ui.history, Some(attendu), "{mods:?} {touche:?}");
        }
    }

    #[test]
    fn la_question_du_fichier_bloque_les_raccourcis() {
        let g = grille();
        let mut h = harness_fichier(&g.composition, true);
        h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::Z);
        h.run();
        assert_eq!(h.state().ui.history, None);
    }

    #[test]
    fn la_selection_suit_la_grille_restauree() {
        let g = grille();
        let mut etat = UiState {
            selected: Some((g.piste, g.clips[0])),
            ..UiState::default()
        };
        let map = IdMap {
            layers: vec![(g.piste, g.piste)],
            clips: vec![(g.clips[0], g.clips[1])],
            trigs: Vec::new(),
        };
        etat.remap(&map);
        assert_eq!(etat.selected, Some((g.piste, g.clips[1])));
        etat.remap(&IdMap::default());
        assert_eq!(etat.selected, None, "le clip a disparu");
    }

    #[test]
    fn le_menu_demande_l_editeur_de_noeuds() {
        let g = grille();
        let mut h = harness(&g.composition);
        h.get_by_label("Affichage").click();
        h.run();
        h.get_by_label("Éditeur de nœuds").click();
        h.run();
        assert!(h.state().ui.open_nodes);
        assert!(h.state().commands.is_empty());
    }

    #[test]
    fn suivre_le_tempo_se_choisit_dans_l_en_tete() {
        let g = grille();
        let mut h = harness(&g.composition);
        assert!(h.query_by_label("son : 127.5").is_some());
        h.get_by_label("Suivre").click();
        h.run();
        assert_eq!(h.state().commands, vec![Command::SetTempoFollow(true)]);
    }

    #[test]
    fn suivre_une_seconde_fois_arrete_de_suivre() {
        let mut g = grille();
        g.composition.tempo_follow = true;
        let mut h = harness(&g.composition);
        h.get_by_label("Suivre").click();
        h.run();
        assert_eq!(h.state().commands, vec![Command::SetTempoFollow(false)]);
    }

    #[test]
    fn sans_tempo_detecte_l_en_tete_le_dit() {
        assert_eq!(detected_label(0.0), "son : —");
        assert_eq!(detected_label(128.04), "son : 128.0");
    }

    /// Le menu d'un potentiomètre du clip Alpha, ouvert.
    fn menu_du_rayon<'a>(composition: &'a Composition) -> Harness<'a, Etat> {
        let mut h = harness(composition);
        h.get_by_label("Alpha").click();
        h.run();
        h.state_mut().commands.clear();
        h.get_by_label("RAYON : cadran").click_secondary();
        h.run();
        h
    }

    #[test]
    fn une_source_audio_se_choisit_au_clic_droit() {
        let g = grille();
        let mut h = menu_du_rayon(&g.composition);
        h.get_by_label("Graves").click();
        h.run();
        assert_eq!(
            h.state().commands,
            vec![Command::SetAudioModulation {
                clip: g.clips[0],
                slot: ParamSlot::Generator(0),
                feature: Some(frogenx_core::FeatureKind::Bass),
            }]
        );
    }

    #[test]
    fn le_menu_propose_chaque_feature() {
        let g = grille();
        let h = menu_du_rayon(&g.composition);
        for feature in frogenx_core::FeatureKind::MENU {
            assert!(h.query_by_label(feature.label()).is_some(), "{feature:?}");
        }
    }

    #[test]
    fn sans_source_il_n_y_a_rien_a_debrancher() {
        let g = grille();
        let mut h = menu_du_rayon(&g.composition);
        h.get_by_label("Sans source audio").click();
        h.run();
        assert!(h.state().commands.is_empty());
    }

    #[test]
    fn une_source_branchee_se_debranche() {
        let mut g = grille();
        let clip = g.composition.clip_mut(g.clips[0]).unwrap();
        clip.set_audio_modulation(
            ParamSlot::Generator(0),
            Some(frogenx_core::FeatureKind::Onset),
        );
        let mut h = menu_du_rayon(&g.composition);
        h.get_by_label("Sans source audio").click();
        h.run();
        assert_eq!(
            h.state().commands,
            vec![Command::SetAudioModulation {
                clip: g.clips[0],
                slot: ParamSlot::Generator(0),
                feature: None,
            }]
        );
    }

    #[test]
    fn un_panneau_se_masque() {
        let g = grille();
        let mut h = harness(&g.composition);
        h.state_mut().ui.show_grid = false;
        h.run();
        assert!(h.query_by_label("Alpha").is_none());
    }

    #[test]
    fn sans_selection_les_reglages_invitent_a_choisir_un_clip() {
        let g = grille();
        let h = harness(&g.composition);
        assert!(h.query_by_label("Clique un clip pour le régler.").is_some());
        assert!(h.state().commands.is_empty());
    }

    #[test]
    fn une_selection_perimee_ne_panique_pas() {
        let g = grille();
        let mut h = harness(&g.composition);
        // Un identifiant valable ailleurs, qui ne désigne rien ici.
        let mut autre = grille();
        let clip = Clip::shape(
            Graph::new(),
            Box::new(Ellipse::circle(0.5)),
            Transport::bpm_sync(&mut Graph::new(), Param::fixed(4.0), PlayMode::Loop),
        );
        let intrus = autre.composition.add_clip(autre.piste, clip).unwrap();
        h.state_mut().ui.selected = Some((g.piste, intrus));
        h.run();
        assert!(h.query_by_label("Clique un clip pour le régler.").is_some());
    }

    /// Le centre d'un élément de l'interface.
    fn centre(h: &Harness<'_, Etat>, nom: &str) -> egui::Pos2 {
        h.get_by_label(nom).rect().center()
    }

    /// Glisse depuis `depuis` jusqu'à `vers`, en trois gestes distincts.
    fn glisse(h: &mut Harness<'_, Etat>, depuis: egui::Pos2, vers: egui::Pos2) {
        h.drag_at(depuis);
        h.run();
        h.hover_at(depuis + egui::vec2(4.0, 4.0));
        h.run();
        h.hover_at(vers);
        h.run();
        h.drop_at(vers);
        h.run();
    }

    #[test]
    fn une_cellule_reste_sous_l_en_tete_de_sa_colonne() {
        let g = grille();
        let mut c = g.composition;
        // Une case vide avant le clip : c'était le défaut d'avant les cases.
        let troisieme = c.columns()[2];
        c.place_clip(
            g.piste,
            troisieme,
            Clip::shape(
                Graph::new(),
                Box::new(Ellipse::circle(0.5)),
                Transport::bpm_sync(&mut Graph::new(), Param::fixed(4.0), PlayMode::Loop),
            )
            .with_name("Gamma"),
        )
        .unwrap();
        c.remove_clip(g.piste, c.columns()[1]);

        let h = harness(&c);
        let (entete, clip) = (centre(&h, "Colonne 3"), centre(&h, "Gamma"));
        assert!((entete.x - clip.x).abs() < 30.0, "{entete:?} / {clip:?}");
        assert!(h.query_by_label("Beta").is_none());
    }

    #[test]
    fn un_clic_sur_une_case_vide_ne_fait_rien() {
        let g = grille();
        let mut h = harness(&g.composition);
        h.get_by_label(EMPTY_CELL).click();
        h.run();
        assert!(h.state().commands.is_empty());
        assert_eq!(h.state().ui.selected, None);
    }

    #[test]
    fn glisser_un_clip_sur_une_case_vide_le_pose() {
        let g = grille();
        let mut h = harness(&g.composition);
        let (depuis, vers) = (centre(&h, "Aurore"), centre(&h, EMPTY_CELL));
        glisse(&mut h, depuis, vers);
        assert_eq!(
            h.state().placements,
            vec![Placement {
                layer: g.piste,
                column: g.colonnes[2],
                preset: find("Aurore").unwrap()
            }]
        );
        assert!(h.state().commands.is_empty(), "rien n'est déclenché");
    }

    #[test]
    fn glisser_un_clip_sur_une_case_occupee_le_remplace() {
        let g = grille();
        let mut h = harness(&g.composition);
        let (depuis, vers) = (centre(&h, "Mandala"), centre(&h, "Beta"));
        glisse(&mut h, depuis, vers);
        assert_eq!(
            h.state().placements,
            vec![Placement {
                layer: g.piste,
                column: g.colonnes[1],
                preset: find("Mandala").unwrap()
            }]
        );
    }

    #[test]
    fn relacher_hors_de_la_grille_ne_pose_rien() {
        let g = grille();
        let mut h = harness(&g.composition);
        let (depuis, vers) = (centre(&h, "Aurore"), centre(&h, "Tap"));
        glisse(&mut h, depuis, vers);
        assert!(h.state().placements.is_empty());
    }

    /// Ouvre le menu d'un élément par clic droit, et choisit `action`.
    fn menu_de(h: &mut Harness<'_, Etat>, nom: &str, action: &str) {
        h.get_by_label(nom).click_secondary();
        h.run();
        h.get_by_label(action).click();
        h.run();
    }

    #[test]
    fn un_clic_droit_retire_un_clip() {
        let g = grille();
        let mut h = harness(&g.composition);
        menu_de(&mut h, "Beta", "Retirer le clip");
        assert_eq!(
            h.state().commands,
            vec![Command::RemoveClip {
                layer: g.piste,
                column: g.colonnes[1]
            }]
        );
    }

    #[test]
    fn un_clic_droit_retire_une_colonne_sans_la_declencher() {
        let g = grille();
        let mut h = harness(&g.composition);
        menu_de(&mut h, "Colonne 2", "Retirer la colonne");
        assert_eq!(
            h.state().commands,
            vec![Command::RemoveColumn {
                column: g.colonnes[1]
            }]
        );
    }

    #[test]
    fn un_clic_droit_retire_une_piste() {
        let g = grille();
        let mut h = harness(&g.composition);
        menu_de(&mut h, "Piste 1", "Retirer la piste");
        assert_eq!(
            h.state().commands,
            vec![Command::RemoveLayer { layer: g.piste }]
        );
    }

    #[test]
    fn les_boutons_ajoutent_une_colonne_et_une_piste() {
        let g = grille();
        let mut h = harness(&g.composition);
        h.get_by_label("+ colonne").click();
        h.run();
        h.get_by_label("+ piste").click();
        h.run();
        assert_eq!(
            h.state().commands,
            vec![Command::AddColumn, Command::AddLayer]
        );
    }

    #[test]
    fn le_curseur_de_vitesse_regle_le_clip() {
        let g = grille();
        let mut h = harness(&g.composition);
        h.get_by_label("Alpha").click();
        h.run();
        h.state_mut().commands.clear();

        champ(&h, "VITESSE").set_value_accesskit(2.0);
        h.run();
        assert_eq!(
            h.state().commands,
            vec![Command::SetClipSpeed {
                clip: g.clips[0],
                value: 2.0
            }]
        );
    }

    /// Une grille dont le rayon du premier clip est modulé.
    fn grille_modulee() -> Grille {
        let mut g = grille();
        let clip = g.composition.clip_mut(g.clips[0]).unwrap();
        let source = clip.graph_mut().add(Box::new(Constant(0.5)));
        let frogenx_composition::ClipSource::Shape { generator, .. } = &mut clip.source else {
            panic!("un clip de forme");
        };
        *generator.param_mut(0).unwrap() = Param::modulated(0.5, 0.25, source);
        g
    }

    #[test]
    fn un_parametre_module_a_un_curseur_d_amplitude() {
        let g = grille_modulee();
        let mut h = harness(&g.composition);
        h.get_by_label("Alpha").click();
        h.run();
        h.state_mut().commands.clear();

        champ(&h, "RAYON : modulation").set_value_accesskit(0.1);
        h.run();
        assert_eq!(
            h.state().commands,
            vec![Command::SetDepth {
                clip: g.clips[0],
                slot: ParamSlot::Generator(0),
                value: 0.1
            }]
        );
    }

    #[test]
    fn un_parametre_fixe_n_a_pas_de_curseur_d_amplitude() {
        let g = grille();
        let mut h = harness(&g.composition);
        h.get_by_label("Alpha").click();
        h.run();
        assert!(h.query_by_label_contains("modulation").is_none());
    }

    #[test]
    fn afficher_un_clip_du_catalogue_n_emet_aucune_commande() {
        // Une valeur hors de la plage d'un curseur ne doit pas être réécrite :
        // la plage borne le geste, pas le paramètre (§16.3 ter).
        let mut c = Composition::new();
        let deck = c.add_deck("test");
        let groupe = c.add_layer_group(deck).unwrap();
        let piste = c.add_layer(groupe).unwrap();
        let clips: Vec<ClipId> = frogenx_library::catalog()
            .iter()
            .map(|p| c.add_clip(piste, p.build()).unwrap())
            .collect();

        let mut h = harness(&c);
        for clip in clips {
            h.state_mut().ui.selected = Some((piste, clip));
            h.run();
            assert_eq!(h.state().commands, vec![], "clip {clip:?}");
        }
    }

    #[test]
    fn la_bibliotheque_liste_le_catalogue_et_se_masque() {
        let g = grille();
        let mut h = harness(&g.composition);
        assert!(h.query_by_label("Motifs").is_some());
        assert!(h.query_by_label("Kaléidoscope").is_some());
        h.state_mut().ui.show_library = false;
        h.run();
        assert!(h.query_by_label("Kaléidoscope").is_none());
    }

    #[test]
    fn le_bouton_ajoute_un_trig() {
        let g = grille();
        let mut h = harness(&g.composition);
        assert!(h
            .query_by_label("Glisse sur le spectre pour tracer une zone.")
            .is_some());
        h.get_by_label("Ajouter un trig").click();
        h.run();
        assert_eq!(h.state().commands, vec![Command::AddTrig]);
    }

    /// La partie du panneau où se tracent les zones, sans les échelles de droite.
    fn zone_de_trace(h: &Harness<'_, Etat>) -> egui::Rect {
        let mut r = h.get_by_label("Spectre").rect();
        r.set_right(r.right() - spectrum::RIGHT_MARGIN);
        r
    }

    /// Le point `(x, y)` du tracé, `y` vers le haut.
    fn point(r: egui::Rect, x: f32, y: f32) -> egui::Pos2 {
        spectrum::to_plot(r, (x, y))
    }

    /// Une grille avec un trig posé à mi-hauteur, loin du panneau flottant.
    fn grille_avec_zone() -> (Grille, TrigId) {
        let mut g = grille();
        let trig = g.composition.add_default_trig().unwrap();
        for (slot, value) in [
            (TrigSlot::FreqMin, 200.0),
            (TrigSlot::FreqMax, 2000.0),
            (TrigSlot::LevelMin, 0.70),
            (TrigSlot::LevelMax, 0.95),
        ] {
            g.composition.set_trig_param(trig, slot, value);
        }
        (g, trig)
    }

    /// Le centre de la zone de [`grille_avec_zone`], en coordonnées du tracé.
    fn centre_de_zone() -> (f32, f32) {
        let p = |hz| FreqScale::Log.position(hz);
        ((p(200.0) + p(2000.0)) * 0.5, 0.825)
    }

    /// Un glissé complet, en plusieurs pas : un geste réel n'est pas instantané.
    fn geste(h: &mut Harness<'_, Etat>, depuis: egui::Pos2, vers: egui::Pos2) {
        h.hover_at(depuis);
        h.run();
        h.drag_at(depuis);
        h.run();
        h.hover_at(depuis + (vers - depuis) * 0.5);
        h.run();
        h.hover_at(vers);
        h.run();
        h.drop_at(vers);
        h.run();
    }

    /// Un clic sans déplacement.
    fn clique(h: &mut Harness<'_, Etat>, p: egui::Pos2) {
        h.hover_at(p);
        h.run();
        h.drag_at(p);
        h.run();
        h.drop_at(p);
        h.run();
    }

    /// Les valeurs émises pour une borne d'un trig, la dernière d'abord.
    fn borne(h: &Harness<'_, Etat>, slot: TrigSlot) -> Option<f32> {
        h.state().commands.iter().rev().find_map(|c| match c {
            Command::SetTrigParam { slot: s, value, .. } if *s == slot => Some(*value),
            _ => None,
        })
    }

    #[test]
    fn glisser_dans_le_vide_cree_un_trig_sur_la_zone_tracee() {
        let g = grille();
        let mut h = harness(&g.composition);
        let r = zone_de_trace(&h);
        geste(&mut h, point(r, 0.25, 0.5), point(r, 0.5, 0.9));

        assert_eq!(h.state().ui.zone_gesture, None, "le glissé est fini");
        let [Command::AddTrigAt { freq, level }] = h.state().commands[..] else {
            panic!("attendu un AddTrigAt : {:?}", h.state().commands);
        };
        let proche = |a: f32, b: f32| (a / b - 1.0).abs() < 0.02;
        assert!(proche(freq.0, FreqScale::Log.frequency(0.25)));
        assert!(proche(freq.1, FreqScale::Log.frequency(0.5)));
        assert!(proche(level.0, 0.5) && proche(level.1, 0.9));
    }

    #[test]
    fn un_trace_trop_petit_ne_cree_rien() {
        let g = grille();
        let mut h = harness(&g.composition);
        let r = zone_de_trace(&h);
        let depuis = point(r, 0.5, 0.5);
        geste(&mut h, depuis, depuis + egui::vec2(20.0, 1.0));
        assert!(h.state().commands.is_empty());
    }

    #[test]
    fn le_corps_d_une_zone_la_deplace_et_la_choisit() {
        let (g, trig) = grille_avec_zone();
        let mut h = harness(&g.composition);
        let r = zone_de_trace(&h);
        let (x, y) = centre_de_zone();
        geste(&mut h, point(r, x, y), point(r, x + 0.1, y - 0.1));

        assert_eq!(h.state().ui.selected_trig, Some(trig));
        // Toute la zone glisse d'autant sur l'axe.
        let p = |hz| FreqScale::Log.position(hz);
        let bas = p(borne(&h, TrigSlot::FreqMin).unwrap()) - p(200.0);
        let haut = p(borne(&h, TrigSlot::FreqMax).unwrap()) - p(2000.0);
        assert!(
            (bas - 0.1).abs() < 0.01 && (haut - 0.1).abs() < 0.01,
            "{bas} {haut}"
        );
        assert!((borne(&h, TrigSlot::LevelMin).unwrap() - 0.6).abs() < 0.02);
    }

    #[test]
    fn un_bord_de_zone_ne_regle_que_sa_borne() {
        let (g, _) = grille_avec_zone();
        let mut h = harness(&g.composition);
        let r = zone_de_trace(&h);
        let (_, y) = centre_de_zone();
        let droite = FreqScale::Log.position(2000.0);
        geste(&mut h, point(r, droite, y), point(r, droite + 0.1, y));

        assert!(borne(&h, TrigSlot::FreqMax).unwrap() > 2000.0);
        assert_eq!(borne(&h, TrigSlot::FreqMin), None);
        assert_eq!(borne(&h, TrigSlot::LevelMin), None);
    }

    #[test]
    fn un_clic_choisit_une_zone_et_un_clic_dans_le_vide_referme() {
        let (g, trig) = grille_avec_zone();
        let mut h = harness(&g.composition);
        assert!(h.query_by_label("Recaler").is_none(), "rien de choisi");

        let r = zone_de_trace(&h);
        let (x, y) = centre_de_zone();
        clique(&mut h, point(r, x, y));
        assert_eq!(h.state().ui.selected_trig, Some(trig));
        assert!(h.query_by_label("Recaler").is_some());

        clique(&mut h, point(r, 0.95, 0.98));
        assert_eq!(h.state().ui.selected_trig, None);
    }

    #[test]
    fn le_panneau_flottant_retient_la_souris() {
        // Un glissé commencé sur son fond ne trace pas de zone dessous.
        let (g, trig) = grille_avec_zone();
        let mut h = harness(&g.composition);
        h.state_mut().ui.selected_trig = Some(trig);
        h.run();
        // Juste à droite du dernier bouton : sur le fond du panneau.
        let fond = h.get_by_label("Recaler").rect().right_center() + egui::vec2(5.0, 0.0);
        geste(&mut h, fond, fond + egui::vec2(40.0, -60.0));
        assert!(h.state().commands.is_empty());
    }

    #[test]
    fn un_trig_qui_vient_de_naitre_est_choisi() {
        let (g, trig) = grille_avec_zone();
        let mut h = harness(&g.composition);
        h.state_mut().ui.known_trigs = Some(0);
        h.run();
        assert_eq!(h.state().ui.selected_trig, Some(trig));
    }

    #[test]
    fn les_trigs_d_une_grille_ouverte_ne_sont_pas_choisis_d_office() {
        let (g, _) = grille_avec_zone();
        let h = harness(&g.composition);
        assert_eq!(h.state().ui.selected_trig, None);
    }

    /// Le harnais, avec un trig déjà choisi.
    fn harness_choisi(g: &Grille, trig: TrigId) -> Harness<'_, Etat> {
        let mut h = harness(&g.composition);
        h.state_mut().ui.selected_trig = Some(trig);
        h.run();
        h
    }

    #[test]
    fn le_compteur_montre_ses_divisions_et_emet_une_commande() {
        let (g, trig) = grille_avec_zone();
        let mut h = harness_choisi(&g, trig);
        for n in spectrum::DIVISIONS {
            assert!(h.query_by_label(&format!("1/{n}")).is_some());
        }
        h.get_by_label("1/16").click();
        h.run();
        assert_eq!(borne(&h, TrigSlot::Every), Some(16.0));
    }

    #[test]
    fn la_cible_se_choisit_parmi_les_colonnes_et_les_clips() {
        let (g, trig) = grille_avec_zone();
        let mut h = harness_choisi(&g, trig);
        h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "Cible")
            .click();
        h.run();
        assert!(h.query_by_label("Piste 1 / Beta").is_some());
        // L'en-tête de la grille porte le même nom : le menu vient après.
        h.query_all_by_label("Colonne 1").last().unwrap().click();
        h.run();
        assert_eq!(
            h.state().commands,
            vec![Command::SetTrigTarget {
                trig,
                target: TrigTarget::Column(g.composition.columns()[0])
            }]
        );
    }

    #[test]
    fn la_valeur_d_un_potentiometre_se_saisit() {
        let (g, trig) = grille_avec_zone();
        let mut h = harness_choisi(&g, trig);
        champ(&h, "SEUIL").set_value_accesskit(0.4);
        h.run();
        assert_eq!(
            h.state().commands,
            vec![Command::SetTrigParam {
                trig,
                slot: TrigSlot::LevelMin,
                value: 0.4
            }]
        );
    }

    #[test]
    fn afficher_un_potentiometre_n_emet_rien() {
        // Le piège de `clamp_existing_to_range` : rien ne change sans geste.
        let (g, trig) = grille_avec_zone();
        let h = harness_choisi(&g, trig);
        assert!(h.state().commands.is_empty());
    }

    #[test]
    fn tourner_un_potentiometre_regle_sa_borne() {
        let (g, trig) = grille_avec_zone();
        let mut h = harness_choisi(&g, trig);
        let valeur = champ(&h, "PLAFOND").rect();
        // Le cadran est juste au-dessus de son nom et de sa valeur.
        let cadran = valeur.center_top() - egui::vec2(0.0, 40.0);
        geste(&mut h, cadran, cadran + egui::vec2(0.0, 32.0));
        let v = borne(&h, TrigSlot::LevelMax).expect("le cadran a émis");
        assert!(v < 0.95, "vers le bas, la valeur descend : {v}");
    }

    #[test]
    fn recaler_emet_sa_commande() {
        let (g, trig) = grille_avec_zone();
        let mut h = harness_choisi(&g, trig);
        h.get_by_label("Recaler").click();
        h.run();
        assert_eq!(h.state().commands, vec![Command::ResetTrigCount(trig)]);
    }

    #[test]
    fn un_trig_se_retire_par_un_menu_jamais_par_un_bouton_direct() {
        // Sur scène, un clic raté ne supprime rien (§23.6).
        let (g, trig) = grille_avec_zone();
        let mut h = harness_choisi(&g, trig);
        assert!(h.query_by_label("Retirer").is_none());
        assert!(h.query_by_label("Retirer le trig").is_none());
        // La puce de la barre vient avant le titre du panneau, du même nom.
        let puce = h.query_all_by_label("Trig 1").next().unwrap();
        puce.click_secondary();
        h.run();
        h.get_by_label("Retirer le trig").click();
        h.run();
        assert_eq!(h.state().commands, vec![Command::RemoveTrig(trig)]);
    }

    /// Ouvre une liste de la barre d'analyse et y choisit une valeur.
    fn choisis(h: &mut Harness<'_, Etat>, liste: &str, valeur: &str) {
        h.get_by_role_and_label(egui::accesskit::Role::ComboBox, liste)
            .click();
        h.run();
        h.get_by_label(valeur).click();
        h.run();
    }

    #[test]
    fn la_barre_regle_l_analyse() {
        let g = grille();
        let mut h = harness(&g.composition);
        choisis(&mut h, "Analyse", "Multi-résolution");
        choisis(&mut h, "Échelle", "Mel");
        choisis(&mut h, "Amplitude", "Linéaire");
        assert_eq!(
            h.state().commands,
            vec![
                Command::SetAnalysisMethod(AnalysisMethod::ConstantQ),
                Command::SetFreqScale(FreqScale::Mel),
                Command::SetAmpScale(AmpScale::Linear),
            ]
        );
    }

    #[test]
    fn reprendre_le_choix_en_cours_n_emet_rien() {
        let g = grille();
        let mut h = harness(&g.composition);
        h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "Échelle")
            .click();
        h.run();
        h.get_by_label("Logarithmique").click();
        h.run();
        assert!(h.state().commands.is_empty());
    }

    #[test]
    fn la_duree_d_analyse_se_regle_en_millisecondes() {
        let g = grille();
        let mut h = harness(&g.composition);
        champ(&h, "Fenêtre").set_value_accesskit(250.0);
        h.run();
        assert_eq!(h.state().commands, vec![Command::SetAnalysisWindow(0.25)]);
    }

    #[test]
    fn le_panneau_dit_d_ou_vient_le_son_et_se_masque() {
        let g = grille();
        let mut h = harness(&g.composition);
        assert!(h.query_by_label("Micro — 48000 Hz").is_some());
        h.state_mut().ui.show_audio = false;
        h.run();
        assert!(h.query_by_label("Spectre").is_none());
    }

    #[test]
    fn un_trig_retire_ne_reste_pas_selectionne() {
        let (mut g, a) = grille_avec_zone();
        let b = g.composition.add_default_trig().unwrap();
        g.composition.remove_trig(b);
        let mut h = harness(&g.composition);
        h.state_mut().ui.selected_trig = Some(b);
        h.run();
        assert!(
            h.query_by_label("Recaler").is_none(),
            "{a:?} n'est pas choisi"
        );
    }

    #[test]
    fn un_clic_sur_la_scene_referme_les_reglages_sans_toucher_au_clip() {
        let g = grille();
        let mut h = harness(&g.composition);
        h.get_by_label("Alpha").click();
        h.run();
        assert!(h.query_by_label("VITESSE").is_some());
        h.state_mut().commands.clear();

        // Au-dessus du spectre, à gauche : là où est l'image.
        let image = h.get_by_label("Spectre").rect().left_top() + egui::vec2(60.0, -60.0);
        clique(&mut h, image);
        assert_eq!(h.state().ui.selected, None);
        assert!(h.query_by_label("VITESSE").is_none());
        assert!(h.state().commands.is_empty(), "rien ne s'arrête");
    }

    #[test]
    fn le_bouton_fermer_referme_les_reglages() {
        let g = grille();
        let mut h = harness(&g.composition);
        h.get_by_label("Alpha").click();
        h.run();
        h.get_by_label("Fermer").click();
        h.run();
        assert_eq!(h.state().ui.selected, None);
    }

    #[test]
    fn le_fond_du_panneau_de_clip_ne_le_referme_pas() {
        let g = grille();
        let mut h = harness(&g.composition);
        h.get_by_label("Alpha").click();
        h.run();
        // Dans la marge du panneau, à gauche du bouton.
        let vide = h.get_by_label("Fermer").rect().left_center() - egui::vec2(30.0, 0.0);
        clique(&mut h, vide);
        assert_eq!(h.state().ui.selected, Some((g.piste, g.clips[0])));
    }

    #[test]
    fn tous_les_reglages_d_un_clip_sont_atteignables() {
        // Le défaut d'un pliage confié à egui : les derniers sortaient du panneau.
        let g = grille();
        let mut h = harness(&g.composition);
        h.get_by_label("Alpha").click();
        h.run();
        let panneau = h.get_by_label("Fermer").rect();
        for nom in ["VITESSE", "OPACITÉ", "RAYON"] {
            let r = champ(&h, nom).rect();
            assert!(r.right() <= panneau.right() + 1.0, "{nom} coupé : {r:?}");
        }
    }

    #[test]
    fn afficher_les_reglages_d_un_clip_n_emet_rien() {
        let g = grille();
        let mut h = harness(&g.composition);
        h.get_by_label("Alpha").click();
        h.run();
        h.state_mut().commands.clear();
        h.run();
        assert!(h.state().commands.is_empty());
    }

    #[test]
    fn une_valeur_hors_plage_s_affiche_sans_etre_reecrite() {
        // La plage borne le geste, pas le paramètre : le rayon est saturé par sa
        // forme, pas par l'interface qui l'affiche (§16.3 ter).
        let mut g = grille();
        let slot = ParamSlot::Generator(0);
        assert!(g
            .composition
            .clip_mut(g.clips[0])
            .unwrap()
            .set_param(slot, 50.0));
        let mut h = harness(&g.composition);
        h.get_by_label("Alpha").click();
        h.run();
        h.state_mut().commands.clear();
        h.run();
        assert!(h.state().commands.is_empty());
    }

    #[test]
    fn tourner_un_potentiometre_de_clip_regle_son_parametre() {
        let g = grille();
        let mut h = harness(&g.composition);
        h.get_by_label("Alpha").click();
        h.run();
        h.state_mut().commands.clear();
        let cadran = champ(&h, "RAYON").rect().center_top() - egui::vec2(0.0, 40.0);
        geste(&mut h, cadran, cadran + egui::vec2(0.0, -32.0));
        assert!(matches!(
            h.state().commands.last(),
            Some(Command::SetParam { .. })
        ));
    }

    #[test]
    fn le_tempo_est_dans_l_en_tete_avec_le_nom_du_fichier() {
        let g = grille();
        let h = harness(&g.composition);
        let tap = h.get_by_label("Tap").rect();
        let fichier = h.get_by_label("Fichier").rect();
        assert!((tap.center().y - fichier.center().y).abs() < 4.0);
        assert!(h.query_by_label("60 fps").is_some());
    }

    #[test]
    fn la_pulsation_tombe_sur_le_temps_et_retombe() {
        // À 120 BPM, un temps toutes les 0.5 s.
        assert_eq!(beat_pulse(0.0, 120.0), 1.0);
        assert_eq!(beat_pulse(1.0, 120.0), 1.0);
        assert!(beat_pulse(0.25, 120.0) < 0.2);
        assert!(beat_pulse(0.05, 120.0) > beat_pulse(0.1, 120.0));
    }

    #[test]
    fn la_pulsation_ne_panique_pas_sans_tempo() {
        assert!(beat_pulse(3.0, 0.0).is_finite());
        assert!(beat_pulse(3.0, -5.0).is_finite());
    }

    #[test]
    fn une_boucle_avance_avec_le_clip_et_revient_a_zero() {
        let g = grille();
        let mut c = g.composition;
        let clip = c.clip_mut(g.clips[0]).unwrap();
        assert_eq!(loop_progress(clip), Some(0.0));
        // À 120 BPM, 2 battements par seconde : un quart des 4 battements.
        clip.graph_mut().set_bpm(120.0);
        for _ in 0..10 {
            clip.graph_mut().eval_frame(0.0, 0.05);
        }
        let t = loop_progress(clip).unwrap();
        assert!((t - 0.25).abs() < 1e-3, "got {t}");
        // 2 s de plus : un tour complet, et elle retrouve sa place.
        for _ in 0..40 {
            clip.graph_mut().eval_frame(0.0, 0.05);
        }
        let t = loop_progress(clip).unwrap();
        assert!((t - 0.25).abs() < 1e-3, "got {t}");
    }

    #[test]
    fn un_clip_en_temps_absolu_n_a_pas_de_boucle() {
        let mut g = Graph::new();
        let position = g.add(Box::new(Constant(1.0)));
        let transport = Transport::Timeline {
            position,
            speed: Param::fixed(1.0),
        };
        let clip = Clip::shape(g, Box::new(Ellipse::circle(0.5)), transport);
        assert_eq!(loop_progress(&clip), None);
    }

    #[test]
    fn une_boucle_de_duree_nulle_ne_divise_pas_par_zero() {
        let mut g = Graph::new();
        let transport = Transport::bpm_sync(&mut g, Param::fixed(0.0), PlayMode::Loop);
        let clip = Clip::shape(g, Box::new(Ellipse::circle(0.5)), transport);
        assert_eq!(loop_progress(&clip), None);
    }

    fn scene(largeur: f32, hauteur: f32) -> Rect {
        Rect::from_min_size(Pos2::new(10.0, 20.0), egui::vec2(largeur, hauteur))
    }

    #[test]
    fn l_image_garde_ses_proportions_et_laisse_la_place_au_panneau() {
        let (image, panneau) = stage_layout(scene(1270.0, 390.0), 16.0 / 9.0);
        assert!((image.width() / image.height() - 16.0 / 9.0).abs() < 1e-3);
        assert!(!image.intersects(panneau), "elle reste entière");
        assert!(panneau.width() >= clip_panel::RESERVED_WIDTH);
    }

    #[test]
    fn une_scene_large_garde_sa_reserve_meme_haute() {
        // Une image limitée par sa largeur ne doit pas manger le panneau.
        let (image, panneau) = stage_layout(scene(1000.0, 900.0), 16.0 / 9.0);
        assert!(!image.intersects(panneau));
        assert!(panneau.width() >= clip_panel::RESERVED_WIDTH);
    }

    #[test]
    fn une_scene_etroite_laisse_le_panneau_se_poser_sur_l_image() {
        let (image, panneau) = stage_layout(scene(600.0, 400.0), 16.0 / 9.0);
        assert!(image.width() > 500.0, "l'image prend tout");
        assert!(image.intersects(panneau));
    }

    #[test]
    fn une_scene_nulle_ne_divise_pas_par_zero() {
        let (image, _) = stage_layout(scene(0.0, 0.0), 16.0 / 9.0);
        assert_eq!(image.size(), egui::Vec2::ZERO);
    }

    #[test]
    fn l_image_est_calee_a_gauche_de_la_scene() {
        // `stage_layout` ne reçoit pas la sélection : choisir un clip ne peut
        // pas la déplacer.
        let (image, _) = stage_layout(scene(1270.0, 390.0), 2.0);
        assert_eq!(image.left(), scene(1270.0, 390.0).left() + 10.0);
    }

    #[test]
    fn le_nom_du_fichier_est_au_milieu_de_la_fenetre() {
        let g = grille();
        let h = harness(&g.composition);
        // Le nom et l'état du fichier, ensemble.
        let debut = h.get_by_label("concert.frogenx *").rect().left();
        let fin = h
            .get_by_label("Enregistré : concert.frogenx")
            .rect()
            .right();
        let milieu = (debut + fin) * 0.5;
        assert!((milieu - 700.0).abs() < 6.0, "got {milieu}");
    }

    #[test]
    fn le_monitor_garde_ses_proportions() {
        assert_eq!(
            fit(egui::vec2(1600.0, 400.0), 2.0),
            egui::vec2(800.0, 400.0)
        );
        assert_eq!(fit(egui::vec2(400.0, 900.0), 2.0), egui::vec2(400.0, 200.0));
    }

    #[test]
    fn un_monitor_sans_place_ne_divise_pas_par_zero() {
        assert_eq!(fit(egui::Vec2::ZERO, 2.0), egui::Vec2::ZERO);
        assert_eq!(fit(egui::vec2(100.0, 100.0), 0.0), egui::Vec2::ZERO);
    }
}
