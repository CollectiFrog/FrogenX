//! # frogenx-ui
//!
//! L'interface : une fonction de l'état — §23.5 de `docs/ARCHITECTURE.md`.
//!
//! [`draw`] lit la grille, dessine avec egui et remplit une liste de `Command`
//! qu'`Engine` applique entre deux frames (§23), et une liste de [`Placement`]
//! pour les clips glissés depuis la bibliothèque (§23.6). Elle ne mute rien
//! d'autre que son [`UiState`].
//!
//! **Ne connaît ni wgpu ni `winit` :** `egui-wgpu` et `egui-winit` restent dans
//! `app`. Les tests simulent des clics, sans GPU ni écran.

pub mod spectrum;
pub mod theme;

use egui::{Button, Color32, Label, Response, RichText, Sense, Slider, SliderClamping, Stroke, Ui};
use frogenx_composition::{
    ClipId, ClipParam, ColumnId, Command, Composition, LayerId, TrigId, MAX_CLIP_SPEED,
};
use frogenx_core::Spectrum;
use frogenx_library::{presets, Family, PresetId};

/// Ce qu'affiche une case vide, pour la retrouver et la viser.
const EMPTY_CELL: &str = "·";
/// La taille d'une case de la grille.
const CELL_SIZE: egui::Vec2 = egui::vec2(110.0, 34.0);

/// Un clip du catalogue à poser dans une case. `Command` ne peut pas porter de
/// `Clip` : `app` le construit entre deux frames (§23.6).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub layer: LayerId,
    pub column: ColumnId,
    pub preset: PresetId,
}

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
    /// Le trig réglé dans le panneau audio ; le premier s'il n'y en a pas.
    pub selected_trig: Option<TrigId>,
    /// Pendant un glissé sur le spectre, le coin où il a commencé : fréquence
    /// et niveau, en coordonnées du spectre et non en pixels.
    pub zone_anchor: Option<(f32, f32)>,
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
            selected_trig: None,
            zone_anchor: None,
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
}

/// Dessine toute l'interface et remplit `commands` et `placements`.
pub fn draw(
    ui: &mut Ui,
    state: &mut UiState,
    view: &FrameView<'_>,
    commands: &mut Vec<Command>,
    placements: &mut Vec<Placement>,
) {
    egui::Panel::top("menu").show(ui, |ui| menu(ui, state));
    if state.show_grid {
        egui::Panel::top("grille")
            .resizable(true)
            .default_size(240.0)
            .show(ui, |ui| {
                grid(ui, state, view.composition, commands, placements)
            });
    }
    egui::Panel::top("barre").show(ui, |ui| toolbar(ui, view, commands));
    if state.show_audio {
        egui::Panel::bottom("audio")
            .resizable(true)
            .default_size(240.0)
            .show(ui, |ui| spectrum::panel(ui, state, view, commands));
    }
    if state.show_library {
        egui::Panel::right("bibliotheque")
            .resizable(true)
            .default_size(170.0)
            .show(ui, library);
    }
    if state.show_monitor {
        egui::Panel::left("monitor")
            .resizable(true)
            .default_size(640.0)
            .show(ui, |ui| monitor(ui, view.monitor));
    }
    if state.show_settings {
        egui::CentralPanel::default().show(ui, |ui| settings(ui, state, view, commands));
    }
}

fn menu(ui: &mut Ui, state: &mut UiState) {
    egui::MenuBar::new().ui(ui, |ui| {
        ui.menu_button("Affichage", |ui| {
            ui.checkbox(&mut state.show_grid, "Grille");
            ui.checkbox(&mut state.show_monitor, "Monitor");
            ui.checkbox(&mut state.show_settings, "Réglages");
            ui.checkbox(&mut state.show_audio, "Audio");
            ui.checkbox(&mut state.show_library, "Bibliothèque");
        });
    });
}

/// Une cellule : l'accent n'y désigne que le clip actif (§23.1).
fn cell(ui: &mut Ui, name: &str, active: bool) -> Response {
    let (fond, texte) = if active {
        (theme::ACCENT, theme::ON_ACCENT)
    } else {
        (theme::IDLE, theme::TEXT)
    };
    let bouton = Button::new(RichText::new(name).color(texte))
        .fill(fond)
        .min_size(CELL_SIZE);
    ui.add(bouton)
}

/// Une case vide : elle ne déclenche rien, elle reçoit.
fn empty_cell(ui: &mut Ui) -> Response {
    let bouton = Button::new(RichText::new(EMPTY_CELL).color(theme::TEXT_DIM))
        .fill(theme::HEADER)
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
                let entete = ui.button(format!("Colonne {}", i + 1));
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
                            let reponse = cell(ui, &clip.name, piste.active_clip() == Some(id));
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
        ui.label(RichText::new("Glisse un clip sur une case.").color(theme::TEXT_DIM));
        for family in Family::ALL {
            ui.add_space(6.0);
            ui.label(RichText::new(family.label()).color(section_color()));
            for (id, preset) in presets().filter(|(_, p)| p.family == family) {
                ui.dnd_drag_source(ui.scope_id().with(preset.name), id, |ui| {
                    ui.add(Label::new(preset.name).selectable(false));
                });
            }
        }
    });
}

fn toolbar(ui: &mut Ui, view: &FrameView<'_>, commands: &mut Vec<Command>) {
    ui.horizontal(|ui| {
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
        ui.separator();
        ui.label(RichText::new(format!("{:.0} fps", view.fps)).color(theme::TEXT_DIM));
        ui.separator();
        ui.label(RichText::new(view.outputs).color(theme::TEXT_DIM));
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

fn monitor(ui: &mut Ui, monitor: Option<Monitor>) {
    match monitor {
        Some(m) => {
            let taille = fit(ui.available_size(), m.aspect);
            ui.image((m.texture, taille));
        }
        None => {
            ui.label("Monitor indisponible");
        }
    }
}

/// Un curseur : émet une commande, n'écrit rien lui-même (§23).
fn param_slider(ui: &mut Ui, clip: ClipId, param: &ClipParam, commands: &mut Vec<Command>) {
    let desc = param.desc;
    let mut value = desc.value.base();
    let mut nom = RichText::new(desc.name);
    if desc.value.source().is_some() {
        // Modulé : le curseur règle la base, la modulation continue.
        nom = nom.color(theme::ACCENT);
    }
    if ui
        .add(slider(&mut value, desc.min, desc.max).text(nom))
        .changed()
    {
        commands.push(Command::SetParam {
            clip,
            slot: param.slot,
            value,
        });
    }

    // Le mouvement d'un clip se règle ici, tant que son graphe n'est pas exposé.
    if let Some(mut value) = desc.value.depth() {
        let etendue = desc.max - desc.min;
        let nom = format!("{} : modulation", desc.name);
        if ui
            .add(slider(&mut value, -etendue, etendue).text(nom))
            .changed()
        {
            commands.push(Command::SetDepth {
                clip,
                slot: param.slot,
                value,
            });
        }
    }
}

/// Un curseur qui ne réécrit jamais une valeur hors de sa plage : la plage borne
/// le geste, pas le paramètre (§16.3 ter). Sans cela, afficher un réglage
/// émettrait une commande à chaque frame.
fn slider(value: &mut f32, min: f32, max: f32) -> Slider<'_> {
    Slider::new(value, min..=max).clamping(SliderClamping::Never)
}

fn settings(ui: &mut Ui, state: &UiState, view: &FrameView<'_>, commands: &mut Vec<Command>) {
    let selection = state.selected.and_then(|(layer, id)| {
        let piste = view.composition.layer(layer)?;
        Some((layer, piste, id, view.composition.clip(id)?))
    });
    let Some((layer, piste, id, clip)) = selection else {
        ui.label(RichText::new("Clique un clip pour le régler.").color(theme::TEXT_DIM));
        return;
    };

    egui::ScrollArea::vertical().show(ui, |ui| {
        ui.heading(&clip.name);
        let mut vitesse = clip.speed.base();
        let curseur = slider(&mut vitesse, 0.0, MAX_CLIP_SPEED).text("vitesse");
        if ui.add(curseur).changed() {
            commands.push(Command::SetClipSpeed {
                clip: id,
                value: vitesse,
            });
        }

        for groupe in clip.params() {
            ui.add_space(6.0);
            ui.label(RichText::new(&groupe.label).color(section_color()));
            for param in &groupe.params {
                param_slider(ui, id, param, commands);
            }
        }

        ui.add_space(10.0);
        ui.separator();
        ui.label(RichText::new(&piste.name).color(section_color()));
        let mut opacity = piste.blend.opacity;
        let mut valeur = opacity.base();
        let curseur = slider(&mut valeur, 0.0, 1.0).text("opacité de la piste");
        if ui.add(curseur).changed() {
            opacity.set_base(valeur);
            commands.push(Command::SetLayerOpacity { layer, opacity });
        }
    });
}

fn section_color() -> Color32 {
    theme::TEXT_DIM
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::kittest::Queryable;
    use egui_kittest::Harness;
    use frogenx_composition::{Clip, ParamSlot, PlayMode, Transport, TrigSlot, TrigTarget};
    use frogenx_core::{Constant, Graph, Param};
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
    }

    /// Une interface sans écran, qui garde les commandes émises.
    fn harness(composition: &Composition) -> Harness<'_, Etat> {
        let etat = Etat {
            ui: UiState::default(),
            commands: Vec::new(),
            placements: Vec::new(),
        };
        let spectre = Spectrum::default();
        Harness::builder()
            .with_size(egui::vec2(1400.0, 900.0))
            .build_ui_state(
                move |ui, etat: &mut Etat| {
                    let view = FrameView {
                        composition,
                        bpm: 120.0,
                        fps: 60.0,
                        outputs: "Monitor : actif",
                        monitor: None,
                        spectrum: &spectre,
                        audio_status: "Micro — 48000 Hz",
                    };
                    draw(
                        ui,
                        &mut etat.ui,
                        &view,
                        &mut etat.commands,
                        &mut etat.placements,
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

        champ(&h, "rayon").set_value_accesskit(0.8);
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

        champ(&h, "opacité de la piste").set_value_accesskit(0.5);
        h.run();
        assert_eq!(
            h.state().commands,
            vec![Command::SetLayerOpacity {
                layer: g.piste,
                opacity: Param::fixed(0.5)
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

        champ(&h, "vitesse").set_value_accesskit(2.0);
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
        let frogenx_composition::ClipSource::Shape { generator, .. } = &mut clip.source;
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

        champ(&h, "rayon : modulation").set_value_accesskit(0.1);
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
            .query_by_label("Ajoute un trig pour dessiner sa zone.")
            .is_some());
        h.get_by_label("Ajouter un trig").click();
        h.run();
        assert_eq!(h.state().commands, vec![Command::AddTrig]);
    }

    #[test]
    fn glisser_sur_le_spectre_dessine_la_zone_du_trig() {
        let mut g = grille();
        let trig = g.composition.add_default_trig().unwrap();
        let mut h = harness(&g.composition);

        let r = h.get_by_label("Spectre").rect();
        let debut = r.lerp_inside(egui::vec2(0.25, 0.75));
        let fin = r.lerp_inside(egui::vec2(0.5, 0.25));
        h.drag_at(debut);
        h.run();
        h.hover_at(fin);
        h.run();
        assert!(h.state().ui.zone_anchor.is_some(), "le glissé a commencé");
        h.state_mut().commands.clear();
        h.hover_at(r.lerp_inside(egui::vec2(0.5, 0.2)));
        h.run();
        h.hover_at(fin);
        h.run();
        h.drop_at(fin);
        h.run();
        assert_eq!(h.state().ui.zone_anchor, None, "le glissé est fini");

        // Les quatre bords, dans l'ordre : deux fréquences, deux niveaux.
        let bord = |slot: TrigSlot| {
            h.state().commands.iter().rev().find_map(|c| match c {
                Command::SetTrigParam {
                    slot: s,
                    value,
                    trig: t,
                } if *s == slot && *t == trig => Some(*value),
                _ => None,
            })
        };
        let attendu = |x: f32| Spectrum::frequency(x);
        let proche = |a: f32, b: f32| (a / b - 1.0).abs() < 0.02;
        assert!(proche(bord(TrigSlot::FreqMin).unwrap(), attendu(0.25)));
        assert!(proche(bord(TrigSlot::FreqMax).unwrap(), attendu(0.5)));
        assert!(proche(bord(TrigSlot::LevelMin).unwrap(), 0.25));
        assert!(proche(bord(TrigSlot::LevelMax).unwrap(), 0.75));
    }

    #[test]
    fn le_compteur_propose_les_divisions_et_emet_une_commande() {
        let mut g = grille();
        let trig = g.composition.add_default_trig().unwrap();
        let mut h = harness(&g.composition);
        h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "Compteur")
            .click();
        h.run();
        for n in [2, 4, 8, 16, 32] {
            assert!(h.query_by_label(&format!("1 sur {n}")).is_some());
        }
        h.get_by_label("1 sur 16").click();
        h.run();
        assert_eq!(
            h.state().commands,
            vec![Command::SetTrigParam {
                trig,
                slot: TrigSlot::Every,
                value: 16.0
            }]
        );
    }

    #[test]
    fn la_cible_se_choisit_parmi_les_colonnes_et_les_clips() {
        let mut g = grille();
        let trig = g.composition.add_default_trig().unwrap();
        let mut h = harness(&g.composition);
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
    fn un_curseur_regle_la_zone() {
        let mut g = grille();
        let trig = g.composition.add_default_trig().unwrap();
        let mut h = harness(&g.composition);
        champ(&h, "niveau bas").set_value_accesskit(0.4);
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
    fn recaler_et_retirer_emettent_leurs_commandes() {
        let mut g = grille();
        let trig = g.composition.add_default_trig().unwrap();
        let mut h = harness(&g.composition);
        h.get_by_label("Recaler").click();
        h.run();
        h.get_by_label("Retirer").click();
        h.run();
        assert_eq!(
            h.state().commands,
            vec![Command::ResetTrigCount(trig), Command::RemoveTrig(trig)]
        );
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
        let mut g = grille();
        let (a, b) = (
            g.composition.add_default_trig().unwrap(),
            g.composition.add_default_trig().unwrap(),
        );
        g.composition.remove_trig(b);
        let mut h = harness(&g.composition);
        h.state_mut().ui.selected_trig = Some(b);
        h.run();
        assert!(h.query_by_label("Recaler").is_some(), "retombe sur {a:?}");
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
