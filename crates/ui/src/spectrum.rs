//! Le panneau audio : le spectre, ses zones et le réglage des trigs — §3bis.6,
//! §23.5 de `docs/ARCHITECTURE.md`.
//!
//! Le tracé est une fonction pure de la vue : glisser sur le spectre dessine la
//! zone du trig choisi, en émettant des `Command`, sans rien écrire soi-même.

use crate::{slider, theme, FrameView, UiState};
use egui::{Pos2, Rect, RichText, Role, Sense, Stroke, Ui, WidgetInfo};
use frogenx_composition::{Command, Composition, Trig, TrigId, TrigSlot, TrigTarget};
use frogenx_core::{Spectrum, SPECTRUM_BANDS, SPECTRUM_MAX_HZ, SPECTRUM_MIN_HZ};

/// Les valeurs du compteur : « 1 fois sur N ».
pub const DIVISIONS: [u32; 6] = [1, 2, 4, 8, 16, 32];
/// Les repères de fréquence sur le tracé.
const MARKS: [(f32, &str); 4] = [
    (100.0, "100"),
    (1000.0, "1k"),
    (5000.0, "5k"),
    (10_000.0, "10k"),
];
const LABEL: &str = "Spectre";

/// Un point du tracé en fréquence (Hz) et niveau `[0, 1]`. Hors du tracé, il
/// rejoint le bord : on dessine une zone jusqu'au bout de l'axe en dépassant.
pub fn plot_to_zone(rect: Rect, pos: Pos2) -> (f32, f32) {
    // Un panneau replié a un tracé nul : pas de division par zéro.
    let part = |d: f32, total: f32| {
        if total > 0.0 {
            (d / total).clamp(0.0, 1.0)
        } else {
            0.0
        }
    };
    let x = part(pos.x - rect.left(), rect.width());
    let y = part(rect.bottom() - pos.y, rect.height());
    (Spectrum::frequency(x), y)
}

/// L'inverse de [`plot_to_zone`].
pub fn zone_to_plot(rect: Rect, hz: f32, level: f32) -> Pos2 {
    Pos2::new(
        rect.left() + Spectrum::position(hz) * rect.width(),
        rect.bottom() - level.clamp(0.0, 1.0) * rect.height(),
    )
}

/// Le rectangle d'un trig sur le tracé.
fn zone_rect(rect: Rect, trig: &Trig) -> Rect {
    let z = trig.zone();
    Rect::from_two_pos(
        zone_to_plot(rect, z.freq_min.base(), z.level_min.base()),
        zone_to_plot(rect, z.freq_max.base(), z.level_max.base()),
    )
}

/// Le trig affiché : celui qu'on a choisi s'il existe encore, sinon le premier.
fn selected(state: &UiState, c: &Composition) -> Option<TrigId> {
    state
        .selected_trig
        .filter(|id| c.trig(*id).is_some())
        .or_else(|| c.trigs().first().copied())
}

fn set(commands: &mut Vec<Command>, trig: TrigId, slot: TrigSlot, value: f32) {
    commands.push(Command::SetTrigParam { trig, slot, value });
}

/// Le spectre et les zones, puis les gestes de la souris.
fn plot(
    ui: &mut Ui,
    state: &mut UiState,
    view: &FrameView<'_>,
    commands: &mut Vec<Command>,
    trig: Option<TrigId>,
) {
    let taille = egui::vec2(ui.available_width(), ui.available_height().max(120.0));
    let (rect, reponse) = ui.allocate_exact_size(taille, Sense::click_and_drag());
    reponse.widget_info(|| WidgetInfo::labeled(Role::Image, true, LABEL));
    let c = view.composition;

    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, theme::BORDER);
    for (hz, _) in MARKS {
        let x = zone_to_plot(rect, hz, 0.0).x;
        painter.vline(x, rect.y_range(), Stroke::new(1.0, theme::HEADER));
    }

    let largeur = rect.width() / SPECTRUM_BANDS as f32;
    for (i, niveau) in view.spectrum.levels.iter().enumerate() {
        let x = rect.left() + i as f32 * largeur;
        let haut = rect.bottom() - niveau.clamp(0.0, 1.0) * rect.height();
        let barre = Rect::from_min_max(
            Pos2::new(x, haut),
            Pos2::new(x + (largeur - 1.0).max(1.0), rect.bottom()),
        );
        painter.rect_filled(barre, 0.0, theme::TEXT_DIM);
    }

    // En haut, et après les barres : en bas, le son les recouvrirait.
    for (hz, texte) in MARKS {
        painter.text(
            Pos2::new(zone_to_plot(rect, hz, 0.0).x + 3.0, rect.top() + 2.0),
            egui::Align2::LEFT_TOP,
            texte,
            egui::FontId::proportional(10.0),
            theme::TEXT_DIM,
        );
    }

    for id in c.trigs() {
        let Some(t) = c.trig(*id) else { continue };
        let zone = zone_rect(rect, t);
        let flash = c.graph().value(t.flash()).clamp(0.0, 1.0);
        let ouverte = c.graph().value(t.gate()) > 0.5;
        // L'accent désigne le trig choisi, et ne s'allume que s'il se passe
        // quelque chose : il reste réservé à l'état actif (§23.1).
        let opacite = if ouverte { 0.2 } else { 0.0 } + 0.5 * flash;
        painter.rect_filled(zone, 0.0, theme::ACCENT.gamma_multiply(opacite));
        let couleur = if Some(*id) == trig {
            theme::ACCENT
        } else {
            theme::TEXT
        };
        painter.rect_stroke(
            zone,
            0.0,
            Stroke::new(1.0, couleur),
            egui::StrokeKind::Inside,
        );
    }

    let curseur = reponse.interact_pointer_pos();
    if reponse.clicked() {
        // Le dernier ajouté est dessiné au-dessus : c'est lui qu'on attrape.
        let sous_le_curseur = c.trigs().iter().rev().copied().find(|id| {
            let zone = c.trig(*id).map(|t| zone_rect(rect, t));
            zone.zip(curseur).is_some_and(|(z, p)| z.contains(p))
        });
        if sous_le_curseur.is_some() {
            state.selected_trig = sous_le_curseur;
        }
    }
    if reponse.drag_started() {
        // L'origine de l'appui, pas le curseur : un glissé n'est reconnu qu'après
        // quelques pixels, et le coin de la zone doit rester là où on a cliqué.
        let origine = ui.input(|i| i.pointer.press_origin());
        state.zone_anchor = origine.map(|p| plot_to_zone(rect, p));
    }
    if let (true, Some(trig), Some(ancre), Some(p)) =
        (reponse.dragged(), trig, state.zone_anchor, curseur)
    {
        let (hz, niveau) = plot_to_zone(rect, p);
        let (f0, f1) = (ancre.0.min(hz), ancre.0.max(hz));
        let (l0, l1) = (ancre.1.min(niveau), ancre.1.max(niveau));
        set(commands, trig, TrigSlot::FreqMin, f0);
        set(commands, trig, TrigSlot::FreqMax, f1);
        set(commands, trig, TrigSlot::LevelMin, l0);
        set(commands, trig, TrigSlot::LevelMax, l1);
    }
    if reponse.drag_stopped() {
        state.zone_anchor = None;
    }
}

/// Le nom d'une cible, tel que le liste son menu.
fn target_name(c: &Composition, target: TrigTarget) -> String {
    match target {
        TrigTarget::None => "Aucune".to_string(),
        TrigTarget::Column(column) => match c.columns().iter().position(|id| *id == column) {
            Some(i) => format!("Colonne {}", i + 1),
            None => "Colonne retirée".to_string(),
        },
        TrigTarget::Clip { layer, clip } => match (c.layer(layer), c.clip(clip)) {
            (Some(piste), Some(clip)) => format!("{} / {}", piste.name, clip.name),
            _ => "Clip retiré".to_string(),
        },
    }
}

/// Toutes les cibles possibles : rien, chaque colonne, chaque clip.
fn targets(c: &Composition) -> Vec<TrigTarget> {
    let mut cibles = vec![TrigTarget::None];
    cibles.extend(c.columns().iter().map(|id| TrigTarget::Column(*id)));
    let mut pistes = Vec::new();
    c.layers_in_order(&mut pistes);
    for layer in pistes.iter().rev() {
        if let Some(piste) = c.layer(*layer) {
            let cellules = (0..c.columns().len()).filter_map(|i| piste.cell(i));
            cibles.extend(cellules.map(|clip| TrigTarget::Clip {
                layer: *layer,
                clip,
            }));
        }
    }
    cibles
}

/// Le choix du trig, puis ses réglages.
fn controls(
    ui: &mut Ui,
    state: &mut UiState,
    view: &FrameView<'_>,
    commands: &mut Vec<Command>,
    choisi: Option<TrigId>,
) {
    let c = view.composition;
    ui.label(RichText::new(view.audio_status).color(theme::TEXT_DIM));
    ui.horizontal_wrapped(|ui| {
        for id in c.trigs() {
            let Some(t) = c.trig(*id) else { continue };
            if ui.selectable_label(Some(*id) == choisi, &t.name).clicked() {
                state.selected_trig = Some(*id);
            }
        }
        if ui.button("Ajouter un trig").clicked() {
            commands.push(Command::AddTrig);
        }
    });

    let Some((id, t)) = choisi.and_then(|id| Some((id, c.trig(id)?))) else {
        ui.label(RichText::new("Ajoute un trig pour dessiner sa zone.").color(theme::TEXT_DIM));
        return;
    };

    let every = t.every().base().round().max(1.0) as u32;
    ui.horizontal(|ui| {
        egui::ComboBox::from_label("Compteur")
            .selected_text(format!("1 sur {every}"))
            .show_ui(ui, |ui| {
                for n in DIVISIONS {
                    if ui
                        .selectable_label(n == every, format!("1 sur {n}"))
                        .clicked()
                    {
                        set(commands, id, TrigSlot::Every, n as f32);
                    }
                }
            });
        // Le coup qu'on attend : 1 est celui qui passe.
        let suivant = t.count(c.graph()) + 1;
        ui.label(RichText::new(format!("{suivant} / {every}")).color(theme::TEXT_DIM));
        if ui.button("Recaler").clicked() {
            commands.push(Command::ResetTrigCount(id));
        }
    });

    ui.horizontal(|ui| {
        egui::ComboBox::from_label("Cible")
            .selected_text(target_name(c, t.target))
            .show_ui(ui, |ui| {
                for cible in targets(c) {
                    let nom = target_name(c, cible);
                    if ui.selectable_label(cible == t.target, nom).clicked() {
                        commands.push(Command::SetTrigTarget {
                            trig: id,
                            target: cible,
                        });
                    }
                }
            });
        if ui.button("Retirer").clicked() {
            commands.push(Command::RemoveTrig(id));
        }
    });

    let z = t.zone();
    for (nom, slot, valeur) in [
        ("fréquence basse", TrigSlot::FreqMin, z.freq_min.base()),
        ("fréquence haute", TrigSlot::FreqMax, z.freq_max.base()),
    ] {
        let mut v = valeur;
        let curseur = slider(&mut v, SPECTRUM_MIN_HZ, SPECTRUM_MAX_HZ)
            .logarithmic(true)
            .suffix(" Hz")
            .text(nom);
        if ui.add(curseur).changed() {
            set(commands, id, slot, v);
        }
    }
    for (nom, slot, valeur) in [
        ("niveau bas", TrigSlot::LevelMin, z.level_min.base()),
        ("niveau haut", TrigSlot::LevelMax, z.level_max.base()),
        ("hystérésis", TrigSlot::Hysteresis, z.hysteresis.base()),
    ] {
        let mut v = valeur;
        if ui.add(slider(&mut v, 0.0, 1.0).text(nom)).changed() {
            set(commands, id, slot, v);
        }
    }
}

/// Le panneau : le tracé à gauche, les réglages à droite.
pub fn panel(ui: &mut Ui, state: &mut UiState, view: &FrameView<'_>, commands: &mut Vec<Command>) {
    let choisi = selected(state, view.composition);
    egui::Panel::right("reglages_trig")
        .resizable(false)
        .exact_size(340.0)
        .show(ui, |ui| controls(ui, state, view, commands, choisi));
    egui::CentralPanel::default().show(ui, |ui| plot(ui, state, view, commands, choisi));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trace() -> Rect {
        Rect::from_min_size(Pos2::new(100.0, 50.0), egui::vec2(600.0, 200.0))
    }

    #[test]
    fn un_point_du_trace_se_lit_en_frequence_et_en_niveau() {
        let r = trace();
        let (hz, niveau) = plot_to_zone(r, r.left_bottom());
        assert!((hz - SPECTRUM_MIN_HZ).abs() < 0.01);
        assert_eq!(niveau, 0.0);
        let (hz, niveau) = plot_to_zone(r, r.right_top());
        assert!((hz - SPECTRUM_MAX_HZ).abs() < 1.0);
        assert_eq!(niveau, 1.0);
    }

    #[test]
    fn un_point_hors_du_trace_rejoint_le_bord() {
        let r = trace();
        let (hz, niveau) = plot_to_zone(r, Pos2::new(5000.0, 5000.0));
        assert!((hz - SPECTRUM_MAX_HZ).abs() < 1.0);
        assert_eq!(niveau, 0.0);
    }

    #[test]
    fn les_deux_conversions_sont_inverses() {
        let r = trace();
        for (hz, niveau) in [(60.0, 0.75), (440.0, 0.5), (12_000.0, 0.1)] {
            let (h, n) = plot_to_zone(r, zone_to_plot(r, hz, niveau));
            assert!((h - hz).abs() / hz < 1e-3, "{hz} → {h}");
            assert!((n - niveau).abs() < 1e-5);
        }
    }

    #[test]
    fn un_trace_nul_ne_divise_pas_par_zero() {
        let r = Rect::from_min_size(Pos2::ZERO, egui::Vec2::ZERO);
        let (hz, niveau) = plot_to_zone(r, Pos2::new(1.0, 1.0));
        assert_eq!(hz, SPECTRUM_MIN_HZ);
        assert_eq!(niveau, 0.0);
    }
}
