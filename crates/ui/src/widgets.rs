//! Les pièces communes de l'interface — §23.1 de `docs/ARCHITECTURE.md`.
//!
//! Un seul potentiomètre pour tout ce qui se règle, un seul panneau flottant
//! pour tout ce qui se pose sur un signal : le spectre et la scène se manipulent
//! de la même façon.

use crate::theme;
use egui::epaint::{Mesh, PathShape, PathStroke};
use egui::{
    Align, Color32, DragValue, Label, Layout, Pos2, Rect, RichText, Sense, Stroke, Ui, UiBuilder,
    WidgetInfo,
};

/// Le diamètre d'un potentiomètre, et la place d'une colonne.
const KNOB: f32 = 38.0;
const KNOB_WIDTH: f32 = 70.0;
/// Ce qu'occupe un potentiomètre dans une rangée, écart compris.
pub const KNOB_STEP: f32 = KNOB_WIDTH + 8.0;
/// La hauteur d'une colonne : le cadran, son nom, ses valeurs.
const KNOB_HEIGHT: f32 = 100.0;
/// La course d'un potentiomètre, de part et d'autre du haut, en radians.
const KNOB_SWEEP: f32 = 2.36;
/// Les pixels à parcourir pour une course complète.
const KNOB_TRAVEL: f32 = 160.0;
/// Le rayon de la couronne de modulation, au-delà du cadran.
const RING_OFFSET: f32 = 3.0;

/// Ce qu'une source fait à un réglage : la couronne autour du cadran.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Modulation {
    /// La profondeur réglée, de part et d'autre de la valeur.
    pub depth: f32,
    /// La valeur de cette frame, modulation comprise.
    pub live: f32,
}

/// Un potentiomètre à dessiner.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Knob<'a> {
    pub name: &'a str,
    pub value: f32,
    pub min: f32,
    pub max: f32,
    pub log: bool,
    pub suffix: &'a str,
    pub color: Color32,
    pub modulation: Option<Modulation>,
}

impl<'a> Knob<'a> {
    pub fn new(name: &'a str, value: f32, min: f32, max: f32, color: Color32) -> Self {
        Self {
            name,
            value,
            min,
            max,
            log: false,
            suffix: "",
            color,
            modulation: None,
        }
    }

    pub fn log(mut self, suffix: &'a str) -> Self {
        self.log = true;
        self.suffix = suffix;
        self
    }

    pub fn modulated(mut self, modulation: Option<Modulation>) -> Self {
        self.modulation = modulation;
        self
    }
}

/// Ce qu'un potentiomètre a changé pendant cette frame.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct KnobChange {
    pub value: Option<f32>,
    pub depth: Option<f32>,
}

/// La place d'un réglage sur la course d'un potentiomètre, dans `[0, 1]`. Une
/// plage nulle ou une valeur non finie donne le bas de la course.
pub fn knob_position(value: f32, min: f32, max: f32, log: bool) -> f32 {
    let t = if log {
        (value.max(min) / min).ln() / (max / min).ln()
    } else {
        (value - min) / (max - min)
    };
    if t.is_finite() {
        t.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

/// Le réglage à la place `t` : l'inverse de [`knob_position`].
pub fn knob_value(t: f32, min: f32, max: f32, log: bool) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if log {
        min * (max / min).powf(t)
    } else {
        min + (max - min) * t
    }
}

/// L'angle d'une place sur la course, en radians depuis le haut.
fn angle_of(t: f32) -> f32 {
    -KNOB_SWEEP + 2.0 * KNOB_SWEEP * t.clamp(0.0, 1.0)
}

/// Un potentiomètre avec son nom et sa valeur exacte, qu'on peut aussi saisir.
///
/// **Sa plage borne le geste, pas le paramètre** (§16.3 ter) : une valeur hors
/// plage s'affiche telle quelle. La réécrire émettrait une commande à chaque
/// frame, et marquerait le fichier comme modifié sans que personne n'ait touché.
pub fn knob(ui: &mut Ui, k: &Knob<'_>) -> KnobChange {
    let mut change = KnobChange::default();
    // Une largeur fixe : `vertical_centered` prendrait toute la place, et le
    // panneau s'étirerait sur le signal qu'il doit laisser voir.
    let colonne = Layout::top_down(Align::Center);
    ui.allocate_ui_with_layout(egui::vec2(KNOB_WIDTH, KNOB_HEIGHT), colonne, |ui| {
        ui.spacing_mut().item_spacing.y = 2.0;
        let (rect, cadran) =
            ui.allocate_exact_size(egui::vec2(KNOB, KNOB), Sense::click_and_drag());
        let t = knob_position(k.value, k.min, k.max, k.log);
        // Tenu sans bouger, un cadran n'émet rien : ce n'est pas une modification.
        let geste = cadran.drag_delta().y;
        let t = if geste != 0.0 {
            let t = t - geste / KNOB_TRAVEL;
            change.value = Some(knob_value(t, k.min, k.max, k.log));
            t
        } else {
            t
        };
        paint_knob(ui, rect, k, t);

        let etiquette = ui.add(
            Label::new(RichText::new(k.name).size(10.0).color(theme::TEXT_DIM))
                .selectable(false)
                .truncate(),
        );
        let mut v = k.value;
        let vitesse = if k.log {
            k.value.abs().max(1.0) * 0.01
        } else {
            (k.max - k.min) * 0.005
        };
        let saisie = DragValue::new(&mut v)
            .range(k.min..=k.max)
            .clamp_existing_to_range(false)
            .speed(vitesse)
            .suffix(k.suffix)
            .max_decimals(if k.log { 0 } else { 2 });
        if ui.add(saisie).labelled_by(etiquette.id).changed() {
            change.value = Some(v);
        }

        if let Some(modulation) = k.modulation {
            let etendue = k.max - k.min;
            let mut d = modulation.depth;
            let saisie = DragValue::new(&mut d)
                .range(-etendue..=etendue)
                .clamp_existing_to_range(false)
                .speed(etendue * 0.005)
                .prefix("± ")
                .max_decimals(2);
            let reponse = ui.add(saisie);
            let nom = format!("{} : modulation", k.name);
            reponse.widget_info(|| WidgetInfo::labeled(egui::Role::SpinButton, true, nom.clone()));
            if reponse.changed() {
                change.depth = Some(d);
            }
        }
    });
    change
}

fn paint_knob(ui: &Ui, rect: Rect, k: &Knob<'_>, t: f32) {
    let painter = ui.painter_at(rect.expand(RING_OFFSET + 2.0));
    let (centre, rayon) = (rect.center(), KNOB * 0.5 - 2.0);
    let point = |angle: f32, r: f32| centre + egui::vec2(angle.sin(), -angle.cos()) * r;
    let arc = |debut: f32, fin: f32, r: f32| -> Vec<Pos2> {
        let pas = ((fin - debut).abs() * 12.0).ceil().max(2.0) as usize;
        (0..=pas)
            .map(|i| point(debut + (fin - debut) * i as f32 / pas as f32, r))
            .collect()
    };
    let angle = angle_of(t);
    painter.circle_filled(centre, rayon - 3.0, theme::IDLE);
    painter.add(PathShape::line(
        arc(-KNOB_SWEEP, KNOB_SWEEP, rayon),
        PathStroke::new(2.0, theme::GRID_STRONG),
    ));
    painter.add(PathShape::line(
        arc(-KNOB_SWEEP, angle, rayon),
        PathStroke::new(2.0, k.color),
    ));
    painter.line_segment(
        [point(angle, rayon - 11.0), point(angle, rayon - 4.0)],
        Stroke::new(2.0, theme::TEXT),
    );

    // La couronne de Pro-Q : ce que la modulation parcourt, et où elle en est.
    if let Some(m) = k.modulation {
        let place = |v: f32| knob_position(v, k.min, k.max, k.log);
        let (bas, haut) = (
            angle_of(place(k.value - m.depth)),
            angle_of(place(k.value + m.depth)),
        );
        let r = rayon + RING_OFFSET;
        painter.add(PathShape::line(
            arc(bas, haut, r),
            PathStroke::new(1.5, k.color.gamma_multiply(0.55)),
        ));
        painter.circle_filled(point(angle_of(place(m.live)), r), 2.2, theme::TEXT);
    }
}

/// Un dégradé vertical.
pub fn gradient(rect: Rect, top: Color32, bottom: Color32) -> Mesh {
    let mut mesh = Mesh::default();
    mesh.colored_vertex(rect.left_top(), top);
    mesh.colored_vertex(rect.right_top(), top);
    mesh.colored_vertex(rect.right_bottom(), bottom);
    mesh.colored_vertex(rect.left_bottom(), bottom);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    mesh
}

/// La place d'un panneau flottant : centré au bas de `area`, sans en sortir.
pub fn floating_rect(area: Rect, size: egui::Vec2, bottom_gap: f32) -> Rect {
    let taille = egui::vec2(
        size.x.min(area.width() - 16.0).max(0.0),
        size.y.min(area.height() - 8.0).max(0.0),
    );
    Rect::from_center_size(
        Pos2::new(
            area.center().x,
            area.bottom() - taille.y * 0.5 - bottom_gap.min(area.height() - taille.y).max(0.0),
        ),
        taille,
    )
}

/// Un panneau posé sur un signal. Son fond retient la souris : un clic manqué ne
/// doit pas agir sur ce qu'il y a dessous.
pub fn floating_panel(ui: &mut Ui, rect: Rect, id: &str, add: impl FnOnce(&mut Ui)) {
    let _ = ui.interact(rect, ui.scope_id().with(id), Sense::click_and_drag());
    let mut fenetre = ui.new_child(
        UiBuilder::new()
            .max_rect(rect)
            .layout(Layout::left_to_right(Align::Center)),
    );
    egui::Frame::new()
        .fill(theme::GLASS)
        .stroke(Stroke::new(1.0, theme::GRID_STRONG))
        .corner_radius(14)
        .inner_margin(egui::Margin::symmetric(12, 8))
        .show(&mut fenetre, |ui| {
            ui.set_min_size(rect.size() - egui::vec2(24.0, 16.0));
            add(ui);
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn un_potentiometre_retrouve_sa_valeur() {
        for (v, min, max, log) in [(0.3, 0.0, 1.0, false), (440.0, 20.0, 20_000.0, true)] {
            let t = knob_position(v, min, max, log);
            assert!((knob_value(t, min, max, log) - v).abs() / v < 1e-3);
        }
    }

    #[test]
    fn la_course_d_un_potentiometre_est_bornee() {
        assert_eq!(knob_position(-1.0, 0.0, 1.0, false), 0.0);
        assert_eq!(knob_position(9.0, 0.0, 1.0, false), 1.0);
        assert_eq!(knob_position(5.0, 20.0, 20_000.0, true), 0.0);
        assert_eq!(knob_value(3.0, 0.0, 1.0, false), 1.0);
    }

    #[test]
    fn le_milieu_d_un_potentiometre_log_est_la_moyenne_geometrique() {
        let milieu = knob_value(0.5, 20.0, 20_000.0, true);
        assert!((milieu - 632.5).abs() < 1.0, "got {milieu}");
    }

    #[test]
    fn une_plage_nulle_ne_divise_pas_par_zero() {
        assert_eq!(knob_position(0.5, 1.0, 1.0, false), 0.0);
        assert_eq!(knob_position(f32::NAN, 0.0, 1.0, false), 0.0);
    }

    #[test]
    fn la_course_va_du_bas_gauche_au_bas_droit() {
        assert_eq!(angle_of(0.0), -KNOB_SWEEP);
        assert_eq!(angle_of(1.0), KNOB_SWEEP);
        assert!(angle_of(0.5).abs() < 1e-6, "le milieu est en haut");
    }

    #[test]
    fn un_panneau_flottant_reste_dans_sa_zone() {
        let zone = Rect::from_min_size(Pos2::new(10.0, 20.0), egui::vec2(400.0, 100.0));
        let r = floating_rect(zone, egui::vec2(900.0, 300.0), 22.0);
        assert!(zone.contains_rect(r), "{r:?}");
        assert_eq!(r.center().x, zone.center().x);
    }

    #[test]
    fn un_panneau_flottant_se_pose_au_dessus_de_l_ecart_demande() {
        let zone = Rect::from_min_size(Pos2::ZERO, egui::vec2(1000.0, 400.0));
        let r = floating_rect(zone, egui::vec2(300.0, 100.0), 22.0);
        assert_eq!(r.bottom(), 378.0);
    }
}
