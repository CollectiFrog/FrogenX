//! # frogenx-ui
//!
//! L'interface : une fonction de l'état — §23.5 de `docs/ARCHITECTURE.md`.
//!
//! [`draw`] lit la grille, dessine avec egui et remplit une liste de `Command`
//! qu'`Engine` applique entre deux frames (§23). Elle ne mute rien d'autre que
//! son [`UiState`].
//!
//! **Ne connaît ni wgpu ni `winit` :** `egui-wgpu` et `egui-winit` restent dans
//! `app`. Les tests simulent des clics, sans GPU ni écran.

pub mod theme;

use egui::{Button, Color32, RichText, Slider, Ui};
use frogenx_composition::{ClipId, ClipParam, Command, Composition, LayerId};

/// Ce que l'interface se rappelle d'une frame à l'autre.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UiState {
    /// Le clip affiché dans les réglages, et sa piste.
    pub selected: Option<(LayerId, ClipId)>,
    pub show_grid: bool,
    pub show_monitor: bool,
    pub show_settings: bool,
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            selected: None,
            show_grid: true,
            show_monitor: true,
            show_settings: true,
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
}

/// Dessine toute l'interface et remplit `commands`.
pub fn draw(ui: &mut Ui, state: &mut UiState, view: &FrameView<'_>, commands: &mut Vec<Command>) {
    egui::Panel::top("menu").show(ui, |ui| menu(ui, state));
    if state.show_grid {
        egui::Panel::top("grille")
            .resizable(true)
            .default_size(190.0)
            .show(ui, |ui| grid(ui, state, view.composition, commands));
    }
    egui::Panel::top("barre").show(ui, |ui| toolbar(ui, view, commands));
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
        });
    });
}

/// Une cellule : l'accent n'y désigne que le clip actif (§23.1).
fn cell(ui: &mut Ui, name: &str, active: bool) -> bool {
    let (fond, texte) = if active {
        (theme::ACCENT, theme::ON_ACCENT)
    } else {
        (theme::IDLE, theme::TEXT)
    };
    let bouton = Button::new(RichText::new(name).color(texte))
        .fill(fond)
        .min_size(egui::vec2(110.0, 34.0));
    ui.add(bouton).clicked()
}

/// Les pistes, la plus haute en haut, comme dans la pile de composition.
fn grid(ui: &mut Ui, state: &mut UiState, c: &Composition, commands: &mut Vec<Command>) {
    let mut pistes = Vec::new();
    c.layers_in_order(&mut pistes);
    egui::ScrollArea::both().show(ui, |ui| {
        egui::Grid::new("grille_clips").show(ui, |ui| {
            ui.label("");
            for (i, column) in c.columns().iter().enumerate() {
                if ui.button(format!("Colonne {}", i + 1)).clicked() {
                    commands.push(Command::TriggerColumn { column: *column });
                }
            }
            ui.end_row();

            for layer in pistes.iter().rev() {
                let Some(piste) = c.layer(*layer) else {
                    continue;
                };
                ui.label(RichText::new(&piste.name).color(theme::TEXT_DIM));
                for id in piste.clips() {
                    let Some(clip) = c.clip(*id) else { continue };
                    if cell(ui, &clip.name, piste.active_clip() == Some(*id)) {
                        commands.push(Command::TriggerClip {
                            layer: *layer,
                            clip: *id,
                        });
                        state.selected = Some((*layer, *id));
                    }
                }
                ui.end_row();
            }
        });
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
    let curseur = Slider::new(&mut value, desc.min..=desc.max).text(nom);
    if ui.add(curseur).changed() {
        commands.push(Command::SetParam {
            clip,
            slot: param.slot,
            value,
        });
    }
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
        let curseur = Slider::new(&mut valeur, 0.0..=1.0).text("opacité de la piste");
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
    use frogenx_composition::{Clip, ColumnId, ParamSlot, PlayMode, Transport};
    use frogenx_core::{Graph, Param};
    use frogenx_shape::{ColorParam, Ellipse, ShapeStyle};

    struct Grille {
        composition: Composition,
        piste: LayerId,
        clips: [ClipId; 2],
        colonne: ColumnId,
    }

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
        let colonne = c.add_column();
        c.set_column_slot(colonne, 0, Some(a));
        Grille {
            composition: c,
            piste,
            clips: [a, b],
            colonne,
        }
    }

    struct Etat {
        ui: UiState,
        commands: Vec<Command>,
    }

    /// Une interface sans écran, qui garde les commandes émises.
    fn harness(composition: &Composition) -> Harness<'_, Etat> {
        let etat = Etat {
            ui: UiState::default(),
            commands: Vec::new(),
        };
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
                    };
                    draw(ui, &mut etat.ui, &view, &mut etat.commands);
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
            vec![Command::TriggerColumn { column: g.colonne }]
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
