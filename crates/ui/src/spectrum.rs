//! Le panneau audio : le spectre, ses zones, le réglage des trigs et de l'analyse
//! — §3bis.6, §7, §23.7 de `docs/ARCHITECTURE.md`.
//!
//! Le son est le fond de l'écran. Chaque trig y pose une zone colorée et sa
//! poignée ; le trig choisi ouvre un panneau flottant, et la barre du bas règle
//! l'analyse. Tout geste émet des `Command`, rien n'est écrit ici.
//!
//! Les gestes se calculent en **coordonnées du tracé** `[0, 1]²`, jamais en Hz ni
//! en pixels : une zone se déplace ainsi de la même façon sur toutes les échelles,
//! et ce calcul se teste sans egui.

use crate::widgets::{floating_panel, floating_rect, gradient, knob, Knob};
use crate::{remove_menu, theme, FrameView, UiState};
use egui::epaint::{Mesh, PathShape, PathStroke};
use egui::{
    Align, Align2, Color32, CursorIcon, DragValue, FontId, Label, Layout, Pos2, Rect, Response,
    RichText, Role, Sense, Shape, Stroke, Ui, WidgetInfo,
};
use frogenx_composition::{Command, Composition, TrigId, TrigSlot, TrigTarget};
use frogenx_core::{
    AmpScale, AnalysisMethod, FreqScale, FLOOR_DB, MAX_WINDOW, MIN_WINDOW, SPECTRUM_BANDS,
    SPECTRUM_MAX_HZ, SPECTRUM_MIN_HZ,
};
use frogenx_osc::SpectrumZone;

/// Les valeurs du compteur : « 1 fois sur N ».
pub const DIVISIONS: [u32; 6] = [1, 2, 4, 8, 16, 32];
const LABEL: &str = "Spectre";
/// Les fréquences qu'on peut repérer sur l'axe, les plus utiles d'abord : quand
/// la place manque, ce sont les dernières qui s'effacent.
const FREQ_MARKS: [(f32, &str); 10] = [
    (1000.0, "1k"),
    (100.0, "100"),
    (10_000.0, "10k"),
    (20.0, "20"),
    (20_000.0, "20k"),
    (200.0, "200"),
    (2000.0, "2k"),
    (500.0, "500"),
    (5000.0, "5k"),
    (50.0, "50"),
];
/// La place, en pixels, qu'il faut entre deux repères de fréquence.
const MARK_GAP: f32 = 34.0;
/// La distance à un bord de zone, en pixels, où il s'attrape encore.
const EDGE_GRAB: f32 = 6.0;
/// La plus petite zone qu'un tracé peut créer, en pixels.
const MIN_DRAWN: f32 = 6.0;
/// Le rayon de la poignée d'une zone, en pixels.
const HANDLE: f32 = 6.5;
/// À droite du tracé : l'échelle d'amplitude, puis le vumètre.
pub(crate) const RIGHT_MARGIN: f32 = 58.0;
const METER_WIDTH: f32 = 9.0;
/// La chute de la courbe de crêtes, en niveau par seconde.
const HOLD_FALL: f32 = 0.2;
/// Points de courbe par bande : assez pour qu'on n'y voie plus de segments.
const SUBDIVISION: usize = 3;
/// La taille du panneau flottant.
const FLOATING: egui::Vec2 = egui::vec2(690.0, 112.0);
/// L'écart avec le bas du tracé : les repères de fréquence restent lisibles.
const FLOATING_GAP: f32 = 22.0;
/// Une zone en coordonnées du tracé : `x` sur l'axe des fréquences, `y` le niveau.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ZoneBox {
    pub x0: f32,
    pub x1: f32,
    pub y0: f32,
    pub y1: f32,
}

impl ZoneBox {
    /// La boîte d'une zone sur une échelle. Toujours rangée : les bornes d'une
    /// zone peuvent être inversées (§3bis.6).
    fn of(zone: &SpectrumZone, scale: FreqScale) -> Self {
        let (a, b) = (
            scale.position(zone.freq_min.base()),
            scale.position(zone.freq_max.base()),
        );
        let (c, d) = (zone.level_min.base(), zone.level_max.base());
        Self {
            x0: a.min(b),
            x1: a.max(b),
            y0: c.min(d).clamp(0.0, 1.0),
            y1: c.max(d).clamp(0.0, 1.0),
        }
    }

    /// La même zone, en pixels.
    fn rect(&self, plot: Rect) -> Rect {
        Rect::from_two_pos(
            to_plot(plot, (self.x0, self.y0)),
            to_plot(plot, (self.x1, self.y1)),
        )
    }
}

/// La partie d'une zone qu'on attrape.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grab {
    Body,
    Left,
    Right,
    Bottom,
    Top,
}

/// Le geste en cours sur le spectre.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ZoneGesture {
    /// Trace une nouvelle zone, d'un coin à l'autre.
    Draw { from: (f32, f32), to: (f32, f32) },
    /// Déplace ou redimensionne une zone depuis son état de départ.
    Edit {
        trig: TrigId,
        grab: Grab,
        start: ZoneBox,
        origin: (f32, f32),
    },
}

/// Un point de l'écran en coordonnées du tracé. Hors du tracé, il rejoint le
/// bord : on dessine une zone jusqu'au bout de l'axe en dépassant.
pub fn from_plot(rect: Rect, pos: Pos2) -> (f32, f32) {
    // Un panneau replié a un tracé nul : pas de division par zéro.
    let part = |d: f32, total: f32| {
        if total > 0.0 {
            (d / total).clamp(0.0, 1.0)
        } else {
            0.0
        }
    };
    (
        part(pos.x - rect.left(), rect.width()),
        part(rect.bottom() - pos.y, rect.height()),
    )
}

/// L'inverse de [`from_plot`].
pub fn to_plot(rect: Rect, (x, y): (f32, f32)) -> Pos2 {
    Pos2::new(
        rect.left() + x.clamp(0.0, 1.0) * rect.width(),
        rect.bottom() - y.clamp(0.0, 1.0) * rect.height(),
    )
}

/// Ce que le pointeur attrape dans une zone. Les bords d'abord, y compris un
/// peu à l'extérieur : un trait de 1 pixel ne se viserait pas.
pub fn grab_at(zone: Rect, pos: Pos2) -> Option<Grab> {
    if !zone.expand(EDGE_GRAB).contains(pos) {
        return None;
    }
    let distance = [
        ((pos.x - zone.left()).abs(), Grab::Left),
        ((pos.x - zone.right()).abs(), Grab::Right),
        ((pos.y - zone.bottom()).abs(), Grab::Bottom),
        ((pos.y - zone.top()).abs(), Grab::Top),
    ];
    let proche = distance
        .iter()
        .filter(|(d, _)| *d <= EDGE_GRAB)
        .min_by(|a, b| a.0.total_cmp(&b.0));
    Some(proche.map_or(Grab::Body, |(_, grab)| *grab))
}

/// Une zone après un geste. Déplacée, elle garde sa taille au lieu de s'écraser
/// contre le bord du tracé.
pub fn moved(start: ZoneBox, grab: Grab, dx: f32, dy: f32) -> ZoneBox {
    let mut z = start;
    match grab {
        Grab::Body => {
            let dx = dx.clamp(-start.x0, 1.0 - start.x1);
            let dy = dy.clamp(-start.y0, 1.0 - start.y1);
            z.x0 += dx;
            z.x1 += dx;
            z.y0 += dy;
            z.y1 += dy;
        }
        Grab::Left => z.x0 = (start.x0 + dx).clamp(0.0, start.x1),
        Grab::Right => z.x1 = (start.x1 + dx).clamp(start.x0, 1.0),
        Grab::Bottom => z.y0 = (start.y0 + dy).clamp(0.0, start.y1),
        Grab::Top => z.y1 = (start.y1 + dy).clamp(start.y0, 1.0),
    }
    z
}

/// Les repères de fréquence, de gauche à droite, avec leur place en pixels. Sur
/// une échelle linéaire, les graves se tassent : leurs repères s'effacent.
pub fn freq_marks(scale: FreqScale, width: f32) -> Vec<(f32, &'static str)> {
    let mut gardes: Vec<(f32, &'static str)> = Vec::new();
    for (hz, texte) in FREQ_MARKS {
        let x = scale.position(hz) * width;
        if gardes.iter().all(|(g, _)| (g - x).abs() >= MARK_GAP) {
            gardes.push((x, texte));
        }
    }
    gardes.sort_by(|a, b| a.0.total_cmp(&b.0));
    gardes
}

/// Les repères d'amplitude : un niveau et son nom.
pub fn amp_marks(scale: AmpScale) -> Vec<(f32, String)> {
    match scale {
        AmpScale::Decibel => (0..=5)
            .map(|i| {
                let db = -12 * i;
                (1.0 - db as f32 / FLOOR_DB, db.to_string())
            })
            .collect(),
        AmpScale::Linear => (0..=4)
            .map(|i| {
                let v = 1.0 - 0.25 * i as f32;
                (v, format!("{v:.2}"))
            })
            .collect(),
    }
}

/// Une courbe sans angles par les niveaux donnés (Catmull-Rom), sans dépasser
/// `[0, 1]`. Écrit dans `out` pour ne pas allouer à chaque frame.
pub fn smooth(levels: &[f32], out: &mut Vec<f32>) {
    out.clear();
    let n = levels.len();
    let lu = |i: isize| levels[i.clamp(0, n as isize - 1) as usize];
    for i in 0..n.saturating_sub(1) as isize {
        let (a, b, c, d) = (lu(i - 1), lu(i), lu(i + 1), lu(i + 2));
        for k in 0..SUBDIVISION {
            let t = k as f32 / SUBDIVISION as f32;
            let v = 0.5
                * (2.0 * b
                    + (c - a) * t
                    + (2.0 * a - 5.0 * b + 4.0 * c - d) * t * t
                    + (3.0 * b - a - 3.0 * c + d) * t * t * t);
            out.push(v.clamp(0.0, 1.0));
        }
    }
    if let Some(dernier) = levels.last() {
        out.push(*dernier);
    }
}

/// La courbe de crêtes : elle saute au niveau, puis retombe doucement. Elle montre
/// où poser une zone quand le son est déjà passé.
pub fn hold_peaks(hold: &mut [f32; SPECTRUM_BANDS], levels: &[f32; SPECTRUM_BANDS], dt: f32) {
    for (h, l) in hold.iter_mut().zip(levels) {
        *h = (*h - HOLD_FALL * dt).max(*l);
    }
}

/// Le trig choisi, s'il existe encore. Aucun : le spectre reste dégagé.
fn selected(state: &UiState, c: &Composition) -> Option<TrigId> {
    state.selected_trig.filter(|id| c.trig(*id).is_some())
}

fn set(commands: &mut Vec<Command>, trig: TrigId, slot: TrigSlot, value: f32) {
    commands.push(Command::SetTrigParam { trig, slot, value });
}

/// La grille, ses repères et l'échelle d'amplitude à droite.
fn paint_grid(painter: &egui::Painter, plot: Rect, view: &FrameView<'_>) {
    let police = FontId::proportional(10.5);
    for (x, texte) in freq_marks(view.spectrum.freq_scale, plot.width()) {
        let x = plot.left() + x;
        let forte = matches!(texte, "100" | "1k" | "10k");
        let couleur = if forte {
            theme::GRID_STRONG
        } else {
            theme::GRID
        };
        painter.vline(x, plot.y_range(), Stroke::new(1.0, couleur));
    }
    for (niveau, _) in amp_marks(view.spectrum.amp_scale) {
        let y = plot.bottom() - niveau * plot.height();
        painter.hline(plot.x_range(), y, Stroke::new(1.0, theme::GRID));
    }
    for (niveau, texte) in amp_marks(view.spectrum.amp_scale) {
        painter.text(
            Pos2::new(
                plot.right() + 7.0,
                // Les extrêmes sont sur le bord : centrés, ils seraient coupés.
                (plot.bottom() - niveau * plot.height())
                    .clamp(plot.top() + 7.0, plot.bottom() - 7.0),
            ),
            Align2::LEFT_CENTER,
            texte,
            police.clone(),
            theme::TEXT_DIM,
        );
    }
}

/// Les repères de fréquence, à part : par-dessus la courbe, sinon le son les
/// recouvrirait.
fn paint_freq_labels(painter: &egui::Painter, plot: Rect, view: &FrameView<'_>) {
    for (x, texte) in freq_marks(view.spectrum.freq_scale, plot.width()) {
        painter.text(
            Pos2::new(
                (plot.left() + x).max(plot.left() + 10.0),
                plot.bottom() - 3.0,
            ),
            Align2::CENTER_BOTTOM,
            texte,
            FontId::proportional(10.5),
            theme::TEXT_DIM,
        );
    }
}

/// Une courbe de niveaux, posée sur le tracé.
fn curve_points(plot: Rect, heights: &[f32]) -> Vec<Pos2> {
    let pas = plot.width() / (heights.len().max(2) - 1) as f32;
    heights
        .iter()
        .enumerate()
        .map(|(i, h)| {
            Pos2::new(
                plot.left() + i as f32 * pas,
                plot.bottom() - h * plot.height(),
            )
        })
        .collect()
}

/// Le son : une surface qui s'efface vers le bas, son contour, et les crêtes.
fn paint_spectrum(painter: &egui::Painter, plot: Rect, view: &FrameView<'_>, state: &UiState) {
    let mut hauteurs = Vec::with_capacity(SPECTRUM_BANDS * SUBDIVISION);
    smooth(&view.spectrum.levels, &mut hauteurs);
    let points = curve_points(plot, &hauteurs);

    let mut surface = Mesh::default();
    for p in &points {
        surface.colored_vertex(*p, theme::CURVE.gamma_multiply(0.30));
        surface.colored_vertex(
            Pos2::new(p.x, plot.bottom()),
            theme::CURVE.gamma_multiply(0.04),
        );
    }
    for i in 0..points.len().saturating_sub(1) as u32 {
        let k = 2 * i;
        surface.add_triangle(k, k + 1, k + 2);
        surface.add_triangle(k + 1, k + 3, k + 2);
    }
    painter.add(Shape::mesh(surface));
    painter.add(PathShape::line(
        points,
        PathStroke::new(1.2, theme::CURVE.gamma_multiply(0.9)),
    ));

    smooth(&state.peak_hold, &mut hauteurs);
    painter.add(PathShape::line(
        curve_points(plot, &hauteurs),
        PathStroke::new(1.0, theme::ACCENT.gamma_multiply(0.55)),
    ));
}

/// Le niveau le plus haut de tout le spectre, sur le bord droit.
fn paint_meter(painter: &egui::Painter, plot: Rect, view: &FrameView<'_>) {
    let niveau = view.spectrum.levels.iter().copied().fold(0.0, f32::max);
    let cadre = Rect::from_min_max(
        Pos2::new(plot.right() + RIGHT_MARGIN - METER_WIDTH - 5.0, plot.top()),
        Pos2::new(plot.right() + RIGHT_MARGIN - 5.0, plot.bottom()),
    );
    painter.rect_filled(cadre, 2.0, theme::BORDER);
    let mut plein = cadre;
    plein.set_top(cadre.bottom() - niveau.clamp(0.0, 1.0) * cadre.height());
    painter.add(Shape::mesh(gradient(
        plein,
        theme::METER_HIGH,
        theme::METER_LOW,
    )));
}

/// Une zone et sa poignée. Elle s'éclaire porte ouverte, et plus fort quand le
/// compteur laisse passer le coup.
fn paint_zone(
    painter: &egui::Painter,
    rect: Rect,
    color: Color32,
    open: bool,
    flash: f32,
    on: bool,
) {
    let eclat = (if open { 0.22 } else { 0.0 }) + 0.5 * flash;
    let haut = color.gamma_multiply(0.20 + eclat);
    let bas = color.gamma_multiply(0.04 + eclat * 0.5);
    painter.add(Shape::mesh(gradient(rect, haut, bas)));
    let trait_ = Stroke::new(if on { 1.5 } else { 1.0 }, color);
    painter.rect_stroke(rect, 2.0, trait_, egui::StrokeKind::Inside);

    let centre = rect.center();
    if flash > 0.01 {
        let halo = HANDLE + 12.0 * flash;
        painter.circle_filled(centre, halo, color.gamma_multiply(0.35 * flash));
    }
    painter.circle_filled(centre, HANDLE, color);
    let bague = if on { theme::TEXT } else { theme::BORDER };
    painter.circle_stroke(
        centre,
        HANDLE,
        Stroke::new(if on { 2.0 } else { 1.0 }, bague),
    );
}

/// Le curseur qui annonce le geste sous le pointeur.
fn cursor_for(grab: Grab) -> CursorIcon {
    match grab {
        Grab::Body => CursorIcon::Grab,
        Grab::Left | Grab::Right => CursorIcon::ResizeHorizontal,
        Grab::Bottom | Grab::Top => CursorIcon::ResizeVertical,
    }
}

/// La zone sous le pointeur : celle du trig choisi d'abord, sinon la dernière
/// ajoutée, qui est dessinée au-dessus.
fn zone_under(
    c: &Composition,
    scale: FreqScale,
    plot: Rect,
    pos: Pos2,
    choisi: Option<TrigId>,
) -> Option<(TrigId, Grab, ZoneBox)> {
    let essai = |id: TrigId| {
        let boite = ZoneBox::of(c.trig(id)?.zone(), scale);
        Some((id, grab_at(boite.rect(plot), pos)?, boite))
    };
    choisi
        .and_then(essai)
        .or_else(|| c.trigs().iter().rev().copied().find_map(essai))
}

/// Applique un geste en cours. Seules les bornes touchées émettent une commande :
/// les autres resteraient au bit près, sans aller-retour par l'échelle.
fn apply_edit(
    commands: &mut Vec<Command>,
    scale: FreqScale,
    trig: TrigId,
    grab: Grab,
    zone: ZoneBox,
) {
    let (corps, gauche, droite, bas, haut) = (
        grab == Grab::Body,
        grab == Grab::Left,
        grab == Grab::Right,
        grab == Grab::Bottom,
        grab == Grab::Top,
    );
    if corps || gauche {
        set(commands, trig, TrigSlot::FreqMin, scale.frequency(zone.x0));
    }
    if corps || droite {
        set(commands, trig, TrigSlot::FreqMax, scale.frequency(zone.x1));
    }
    if corps || bas {
        set(commands, trig, TrigSlot::LevelMin, zone.y0);
    }
    if corps || haut {
        set(commands, trig, TrigSlot::LevelMax, zone.y1);
    }
}

/// Les gestes de la souris sur le tracé.
fn gestures(
    ui: &Ui,
    reponse: &Response,
    plot: Rect,
    state: &mut UiState,
    view: &FrameView<'_>,
    commands: &mut Vec<Command>,
    choisi: Option<TrigId>,
) {
    let c = view.composition;
    let scale = view.spectrum.freq_scale;
    let curseur = reponse.interact_pointer_pos();

    if let Some(p) = reponse.hover_pos().filter(|_| state.zone_gesture.is_none()) {
        if let Some((_, grab, _)) = zone_under(c, scale, plot, p, choisi) {
            ui.ctx().set_cursor_icon(cursor_for(grab));
        }
    }

    if reponse.clicked() {
        // Un clic dans le vide referme le panneau : le spectre redevient lisible.
        state.selected_trig =
            curseur.and_then(|p| zone_under(c, scale, plot, p, choisi).map(|(id, ..)| id));
    }
    if reponse.drag_started() {
        // L'origine de l'appui, pas le curseur : un glissé n'est reconnu qu'après
        // quelques pixels, et le geste doit partir de là où on a cliqué.
        if let Some(p) = ui.input(|i| i.pointer.press_origin()) {
            let origin = from_plot(plot, p);
            state.zone_gesture = Some(match zone_under(c, scale, plot, p, choisi) {
                Some((trig, grab, start)) => {
                    state.selected_trig = Some(trig);
                    ZoneGesture::Edit {
                        trig,
                        grab,
                        start,
                        origin,
                    }
                }
                None => ZoneGesture::Draw {
                    from: origin,
                    to: origin,
                },
            });
        }
    }
    if let (true, Some(p)) = (reponse.dragged(), curseur) {
        let ici = from_plot(plot, p);
        match &mut state.zone_gesture {
            Some(ZoneGesture::Draw { to, .. }) => *to = ici,
            Some(ZoneGesture::Edit {
                trig,
                grab,
                start,
                origin,
            }) => {
                let zone = moved(*start, *grab, ici.0 - origin.0, ici.1 - origin.1);
                apply_edit(commands, scale, *trig, *grab, zone);
            }
            None => {}
        }
    }
    if reponse.drag_stopped() {
        if let Some(ZoneGesture::Draw { from, to }) = state.zone_gesture {
            let trace = Rect::from_two_pos(to_plot(plot, from), to_plot(plot, to));
            if trace.width() >= MIN_DRAWN && trace.height() >= MIN_DRAWN {
                let (a, b) = (scale.frequency(from.0), scale.frequency(to.0));
                commands.push(Command::AddTrigAt {
                    freq: (a.min(b), a.max(b)),
                    level: (from.1.min(to.1), from.1.max(to.1)),
                });
            }
        }
        state.zone_gesture = None;
    }
}

/// Le spectre, ses zones, puis le panneau du trig choisi par-dessus.
fn plot(
    ui: &mut Ui,
    state: &mut UiState,
    view: &FrameView<'_>,
    commands: &mut Vec<Command>,
    choisi: Option<TrigId>,
) {
    let taille = egui::vec2(ui.available_width(), ui.available_height().max(120.0));
    let (cadre, reponse) = ui.allocate_exact_size(taille, Sense::click_and_drag());
    reponse.widget_info(|| WidgetInfo::labeled(Role::Image, true, LABEL));
    let c = view.composition;
    let mut trace = cadre;
    trace.set_right(cadre.right() - RIGHT_MARGIN);

    let painter = ui.painter_at(cadre);
    painter.add(Shape::mesh(gradient(
        cadre,
        theme::PLOT_TOP,
        theme::PLOT_BOTTOM,
    )));
    paint_grid(&painter, trace, view);
    paint_spectrum(&painter, trace, view, state);

    let scale = view.spectrum.freq_scale;
    for (i, id) in c.trigs().iter().enumerate() {
        let Some(t) = c.trig(*id) else { continue };
        let boite = ZoneBox::of(t.zone(), scale);
        paint_zone(
            &painter,
            boite.rect(trace),
            theme::trig_color(i),
            c.graph().value(t.gate()) > 0.5,
            c.graph().value(t.flash()).clamp(0.0, 1.0),
            Some(*id) == choisi,
        );
    }
    if let Some(ZoneGesture::Draw { from, to }) = state.zone_gesture {
        let couleur = theme::trig_color(c.trigs().len());
        let zone = Rect::from_two_pos(to_plot(trace, from), to_plot(trace, to));
        painter.add(Shape::mesh(gradient(
            zone,
            couleur.gamma_multiply(0.25),
            couleur.gamma_multiply(0.05),
        )));
        painter.rect_stroke(
            zone,
            2.0,
            Stroke::new(1.0, couleur),
            egui::StrokeKind::Inside,
        );
    }
    paint_freq_labels(&painter, trace, view);
    paint_meter(&painter, trace, view);

    gestures(ui, &reponse, trace, state, view, commands, choisi);
    if let Some(id) = choisi {
        floating(ui, trace, view, commands, id);
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

/// Le compteur en boutons : les valeurs se voient toutes, et une seule se clique.
fn counter(ui: &mut Ui, id: TrigId, every: u32, commands: &mut Vec<Command>) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        ui.spacing_mut().button_padding = egui::vec2(6.0, 3.0);
        for n in DIVISIONS {
            let texte = format!("1/{n}");
            if ui.selectable_label(n == every, texte).clicked() {
                set(commands, id, TrigSlot::Every, n as f32);
            }
        }
    });
}

/// Le panneau flottant du trig choisi : posé sur le spectre, comme celui d'une
/// bande de Pro-Q, et absent quand rien n'est choisi.
fn floating(
    ui: &mut Ui,
    plot: Rect,
    view: &FrameView<'_>,
    commands: &mut Vec<Command>,
    id: TrigId,
) {
    let c = view.composition;
    let Some(t) = c.trig(id) else { return };
    let rang = c.trigs().iter().position(|t| *t == id).unwrap_or(0);
    let couleur = theme::trig_color(rang);

    let cadre = floating_rect(plot, FLOATING, FLOATING_GAP);
    floating_panel(ui, cadre, "fond_trig", |ui| {
        ui.horizontal_top(|ui| {
            let z = t.zone();
            let (hz_min, hz_max) = (SPECTRUM_MIN_HZ, SPECTRUM_MAX_HZ);
            for (reglage, slot) in [
                (
                    Knob::new("BASSE", z.freq_min.base(), hz_min, hz_max, couleur).log(" Hz"),
                    TrigSlot::FreqMin,
                ),
                (
                    Knob::new("HAUTE", z.freq_max.base(), hz_min, hz_max, couleur).log(" Hz"),
                    TrigSlot::FreqMax,
                ),
                (
                    Knob::new("SEUIL", z.level_min.base(), 0.0, 1.0, couleur),
                    TrigSlot::LevelMin,
                ),
                (
                    Knob::new("PLAFOND", z.level_max.base(), 0.0, 1.0, couleur),
                    TrigSlot::LevelMax,
                ),
                (
                    Knob::new("MARGE", z.hysteresis.base(), 0.0, 1.0, couleur),
                    TrigSlot::Hysteresis,
                ),
            ] {
                if let Some(v) = knob(ui, &reglage).value {
                    set(commands, id, slot, v);
                }
            }

            ui.separator();
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 5.0;
                let every = t.every().base().round().max(1.0) as u32;
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&t.name).color(couleur).strong());
                    // Le coup qu'on attend : 1 est celui qui passe.
                    let suivant = t.count(c.graph()) + 1;
                    let compte = format!("coup {suivant} / {every}");
                    ui.label(RichText::new(compte).color(theme::TEXT_DIM).size(11.0));
                });
                counter(ui, id, every, commands);
                ui.horizontal(|ui| {
                    let nom = ui.label(RichText::new("Cible").color(theme::TEXT_DIM));
                    egui::ComboBox::from_id_salt("cible")
                        .width(116.0)
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
                        })
                        .response
                        .labelled_by(nom.id);
                    if ui.button("Recaler").clicked() {
                        commands.push(Command::ResetTrigCount(id));
                    }
                });
            });
        });
    });
}

/// Une liste de choix : une ligne par valeur, et une commande si elle change.
fn choice<T: Copy + PartialEq>(
    ui: &mut Ui,
    nom: &str,
    current: T,
    all: &[T],
    label: impl Fn(T) -> &'static str,
) -> Option<T> {
    let mut choisi = None;
    let liste = egui::ComboBox::from_id_salt(nom)
        .selected_text(label(current))
        .show_ui(ui, |ui| {
            for v in all {
                if ui.selectable_label(*v == current, label(*v)).clicked() && *v != current {
                    choisi = Some(*v);
                }
            }
        });
    // La barre se remplit de droite à gauche : ajouté après, le nom est à gauche.
    let etiquette = ui.add(Label::new(RichText::new(nom).color(theme::TEXT_DIM)).selectable(false));
    liste.response.labelled_by(etiquette.id);
    choisi
}

/// La barre du bas : les trigs à gauche, l'analyse à droite.
fn analysis_bar(
    ui: &mut Ui,
    state: &mut UiState,
    view: &FrameView<'_>,
    commands: &mut Vec<Command>,
    choisi: Option<TrigId>,
) {
    let c = view.composition;
    let reglage = c.analysis;
    ui.horizontal_centered(|ui| {
        for (i, id) in c.trigs().iter().enumerate() {
            let Some(t) = c.trig(*id) else { continue };
            let actif = Some(*id) == choisi;
            let texte = RichText::new(&t.name);
            // Sur l'or, la couleur du trig ne se lirait pas : l'accent la remplace.
            let texte = if actif {
                texte
            } else {
                texte.color(theme::trig_color(i))
            };
            let puce = ui.selectable_label(actif, texte);
            if puce.clicked() {
                state.selected_trig = Some(*id);
            }
            remove_menu(&puce, "Retirer le trig", Command::RemoveTrig(*id), commands);
        }
        if ui.button("Ajouter un trig").clicked() {
            commands.push(Command::AddTrig);
        }
        if c.trigs().is_empty() {
            ui.label(RichText::new("Glisse sur le spectre pour tracer une zone.").color(theme::TEXT_DIM));
        }

        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let mut ms = reglage.window.base() * 1000.0;
            // Proportionnelle : 1 ms compte à 10 ms, pas à 800.
            let vitesse = ms * 0.01;
            let saisie = DragValue::new(&mut ms)
                .range(MIN_WINDOW * 1000.0..=MAX_WINDOW * 1000.0)
                .speed(vitesse)
                .suffix(" ms")
                .max_decimals(0);
            let reponse = ui.add(saisie);
            let nom = ui.add(
                Label::new(RichText::new("Fenêtre").color(theme::TEXT_DIM)).selectable(false),
            );
            let reponse = reponse.labelled_by(nom.id);
            if reponse.changed() {
                commands.push(Command::SetAnalysisWindow(ms / 1000.0));
            }
            let resolution = 1.0 / reglage.window.base().max(MIN_WINDOW);
            reponse.on_hover_text(format!(
                "La durée de signal analysée. Longue : les graves se séparent (≈ {resolution:.0} Hz). \
                 Courte : le spectre réagit plus vite."
            ));

            if let Some(v) = choice(ui, "Amplitude", reglage.amp_scale, &AmpScale::ALL, AmpScale::label) {
                commands.push(Command::SetAmpScale(v));
            }
            if let Some(v) = choice(ui, "Échelle", reglage.freq_scale, &FreqScale::ALL, FreqScale::label) {
                commands.push(Command::SetFreqScale(v));
            }
            if let Some(v) = choice(ui, "Analyse", reglage.method, &AnalysisMethod::ALL, AnalysisMethod::label) {
                commands.push(Command::SetAnalysisMethod(v));
            }
            ui.add(
                Label::new(RichText::new(view.audio_status).color(theme::TEXT_DIM).size(11.0))
                    .truncate(),
            );
        });
    });
}

/// Le panneau : le spectre en grand, la barre d'analyse dessous.
pub fn panel(ui: &mut Ui, state: &mut UiState, view: &FrameView<'_>, commands: &mut Vec<Command>) {
    let c = view.composition;
    // Un trig qui vient de naître est celui qu'on veut régler.
    if state.known_trigs.is_some_and(|n| c.trigs().len() > n) {
        state.selected_trig = c.trigs().last().copied();
    }
    state.known_trigs = Some(c.trigs().len());

    let dt = ui.input(|i| i.stable_dt);
    hold_peaks(&mut state.peak_hold, &view.spectrum.levels, dt);

    let choisi = selected(state, c);
    egui::Panel::bottom("barre_analyse")
        .exact_size(34.0)
        .show(ui, |ui| analysis_bar(ui, state, view, commands, choisi));
    egui::CentralPanel::default()
        .frame(egui::Frame::new())
        .show(ui, |ui| plot(ui, state, view, commands, choisi));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn trace() -> Rect {
        Rect::from_min_size(Pos2::new(100.0, 50.0), egui::vec2(600.0, 200.0))
    }

    fn boite() -> ZoneBox {
        ZoneBox {
            x0: 0.2,
            x1: 0.4,
            y0: 0.5,
            y1: 0.8,
        }
    }

    #[test]
    fn un_point_du_trace_se_lit_en_position_et_en_niveau() {
        let r = trace();
        assert_eq!(from_plot(r, r.left_bottom()), (0.0, 0.0));
        assert_eq!(from_plot(r, r.right_top()), (1.0, 1.0));
    }

    #[test]
    fn un_point_hors_du_trace_rejoint_le_bord() {
        assert_eq!(from_plot(trace(), Pos2::new(5000.0, 5000.0)), (1.0, 0.0));
    }

    #[test]
    fn les_deux_conversions_sont_inverses() {
        let r = trace();
        for p in [(0.1, 0.75), (0.5, 0.5), (0.9, 0.1)] {
            let (x, y) = from_plot(r, to_plot(r, p));
            assert!((x - p.0).abs() < 1e-5 && (y - p.1).abs() < 1e-5);
        }
    }

    #[test]
    fn un_trace_nul_ne_divise_pas_par_zero() {
        let r = Rect::from_min_size(Pos2::ZERO, egui::Vec2::ZERO);
        assert_eq!(from_plot(r, Pos2::new(1.0, 1.0)), (0.0, 0.0));
    }

    #[test]
    fn une_zone_garde_ses_frequences_d_une_echelle_a_l_autre() {
        // Seule sa place à l'écran change : c'est ce qui rend l'échelle libre.
        let zone = SpectrumZone::new((100.0, 1000.0), (0.5, 0.8));
        for scale in FreqScale::ALL {
            let b = ZoneBox::of(&zone, scale);
            assert!((scale.frequency(b.x0) - 100.0).abs() < 1.0, "{scale:?}");
            assert!((scale.frequency(b.x1) - 1000.0).abs() < 2.0, "{scale:?}");
        }
    }

    #[test]
    fn une_zone_aux_bornes_inversees_reste_rangee() {
        let zone = SpectrumZone::new((1000.0, 100.0), (0.8, 0.5));
        let b = ZoneBox::of(&zone, FreqScale::Log);
        assert!(b.x0 < b.x1 && b.y0 < b.y1);
    }

    #[test]
    fn on_attrape_un_bord_ou_le_corps() {
        let z = Rect::from_min_max(Pos2::new(100.0, 100.0), Pos2::new(200.0, 200.0));
        assert_eq!(grab_at(z, Pos2::new(150.0, 150.0)), Some(Grab::Body));
        assert_eq!(grab_at(z, Pos2::new(98.0, 150.0)), Some(Grab::Left));
        assert_eq!(grab_at(z, Pos2::new(203.0, 150.0)), Some(Grab::Right));
        assert_eq!(grab_at(z, Pos2::new(150.0, 99.0)), Some(Grab::Top));
        assert_eq!(grab_at(z, Pos2::new(150.0, 204.0)), Some(Grab::Bottom));
        assert_eq!(grab_at(z, Pos2::new(50.0, 150.0)), None);
    }

    #[test]
    fn deplacer_une_zone_garde_sa_taille_contre_le_bord() {
        let z = moved(boite(), Grab::Body, -1.0, 1.0);
        assert!((z.x0 - 0.0).abs() < 1e-6 && (z.y1 - 1.0).abs() < 1e-6);
        assert!((z.x1 - z.x0 - 0.2).abs() < 1e-6, "la largeur reste");
        assert!((z.y1 - z.y0 - 0.3).abs() < 1e-6, "la hauteur reste");
    }

    #[test]
    fn un_bord_ne_bouge_que_lui() {
        let z = moved(boite(), Grab::Right, 0.1, 0.5);
        assert_eq!((z.x0, z.y0, z.y1), (0.2, 0.5, 0.8));
        assert!((z.x1 - 0.5).abs() < 1e-6);
    }

    #[test]
    fn un_bord_ne_traverse_pas_l_autre() {
        assert_eq!(moved(boite(), Grab::Left, 0.9, 0.0).x0, 0.4);
        assert_eq!(moved(boite(), Grab::Top, 0.0, -0.9).y1, 0.5);
    }

    #[test]
    fn les_reperes_de_frequence_ne_se_chevauchent_jamais() {
        for scale in FreqScale::ALL {
            for largeur in [200.0, 600.0, 1400.0] {
                let marks = freq_marks(scale, largeur);
                assert!(!marks.is_empty());
                for paire in marks.windows(2) {
                    assert!(paire[1].0 - paire[0].0 >= MARK_GAP, "{scale:?} {largeur}");
                }
            }
        }
    }

    #[test]
    fn l_echelle_log_montre_tous_ses_reperes_sur_un_grand_trace() {
        assert_eq!(freq_marks(FreqScale::Log, 1200.0).len(), 10);
    }

    #[test]
    fn l_echelle_lineaire_efface_les_graves_entasses() {
        let noms: Vec<&str> = freq_marks(FreqScale::Linear, 1000.0)
            .iter()
            .map(|m| m.1)
            .collect();
        assert!(noms.contains(&"1k") && noms.contains(&"10k"));
        assert!(!noms.contains(&"50") && !noms.contains(&"200"));
    }

    #[test]
    fn les_reperes_d_amplitude_suivent_l_echelle() {
        let db = amp_marks(AmpScale::Decibel);
        assert_eq!(db[0], (1.0, "0".to_string()));
        assert!((db[1].0 - (1.0 - 12.0 / 72.0)).abs() < 1e-6);
        assert_eq!(db[1].1, "-12");
        let lineaire = amp_marks(AmpScale::Linear);
        assert_eq!(lineaire[2], (0.5, "0.50".to_string()));
    }

    #[test]
    fn la_courbe_passe_par_les_niveaux_sans_depasser() {
        let niveaux = [0.0, 1.0, 0.0, 0.5, 0.5];
        let mut courbe = Vec::new();
        smooth(&niveaux, &mut courbe);
        assert_eq!(courbe.len(), (niveaux.len() - 1) * SUBDIVISION + 1);
        for (i, n) in niveaux.iter().enumerate() {
            assert!((courbe[i * SUBDIVISION] - n).abs() < 1e-6, "bande {i}");
        }
        assert!(courbe.iter().all(|v| (0.0..=1.0).contains(v)));
    }

    #[test]
    fn une_courbe_vide_ne_panique_pas() {
        let mut courbe = vec![1.0];
        smooth(&[], &mut courbe);
        assert!(courbe.is_empty());
    }

    #[test]
    fn les_cretes_sautent_au_niveau_puis_retombent() {
        let mut crete = [0.0; SPECTRUM_BANDS];
        let mut son = [0.0; SPECTRUM_BANDS];
        son[3] = 0.8;
        hold_peaks(&mut crete, &son, 0.016);
        assert_eq!(crete[3], 0.8);

        son[3] = 0.0;
        hold_peaks(&mut crete, &son, 1.0);
        assert!((crete[3] - (0.8 - HOLD_FALL)).abs() < 1e-6);
        hold_peaks(&mut crete, &son, 100.0);
        assert_eq!(crete[3], 0.0, "jamais sous le son");
    }
}
