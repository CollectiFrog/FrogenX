//! La charte graphique — §23.1 de `docs/ARCHITECTURE.md`.
//!
//! Un noir légèrement teinté, des surfaces arrondies, un or réservé à l'état
//! actif. La couleur n'apparaît que dans le spectre, où elle distingue les trigs :
//! partout ailleurs, rien ne doit concurrencer le rendu affiché dans le monitor.

use egui::{Color32, CornerRadius, Margin, Shadow, Stroke};

/// Fond principal : grille, panneaux, monitor.
pub const BACKGROUND: Color32 = Color32::from_rgb(0x15, 0x14, 0x18);
/// En-têtes et barres, légèrement plus clairs que le fond.
pub const HEADER: Color32 = Color32::from_rgb(0x21, 0x20, 0x26);
/// Un état au repos : une cellule, un bouton.
pub const IDLE: Color32 = Color32::from_rgb(0x2c, 0x2b, 0x33);
/// Un état survolé.
pub const HOVER: Color32 = Color32::from_rgb(0x38, 0x37, 0x41);
/// L'accent unique de l'interface : il porte toute la sélection.
pub const ACCENT: Color32 = Color32::from_rgb(0xf5, 0xb9, 0x31);
/// Le texte posé sur l'accent.
pub const ON_ACCENT: Color32 = Color32::from_rgb(0x1c, 0x15, 0x05);
pub const TEXT: Color32 = Color32::from_rgb(0xe9, 0xe7, 0xee);
pub const TEXT_DIM: Color32 = Color32::from_rgb(0x8d, 0x8b, 0x96);
pub const BORDER: Color32 = Color32::from_rgb(0x0b, 0x0a, 0x0d);

/// Le haut et le bas du fond du spectre.
pub const PLOT_TOP: Color32 = Color32::from_rgb(0x1e, 0x1c, 0x24);
pub const PLOT_BOTTOM: Color32 = Color32::from_rgb(0x10, 0x0f, 0x13);
/// Les repères du spectre.
pub const GRID: Color32 = Color32::from_rgb(0x28, 0x27, 0x2f);
pub const GRID_STRONG: Color32 = Color32::from_rgb(0x3a, 0x38, 0x43);
/// La courbe du son.
pub const CURVE: Color32 = Color32::from_rgb(0xa8, 0xa6, 0xb2);
/// Un panneau posé sur le spectre : il le laisse deviner.
pub const GLASS: Color32 = Color32::from_rgba_premultiplied(0x1a, 0x19, 0x1f, 0xe6);
/// Le vumètre, du bas vers le haut.
pub const METER_LOW: Color32 = Color32::from_rgb(0x3f, 0x8f, 0x3a);
pub const METER_HIGH: Color32 = Color32::from_rgb(0xd8, 0xc2, 0x2e);

/// La couleur de chaque trig, dans l'ordre : assez éloignées pour se distinguer
/// d'un coup d'œil, comme les bandes d'un égaliseur.
pub const TRIGS: [Color32; 6] = [
    Color32::from_rgb(0xd0, 0x4f, 0xe0),
    Color32::from_rgb(0x3f, 0xa9, 0xf5),
    Color32::from_rgb(0x2f, 0xd8, 0xa8),
    Color32::from_rgb(0xf0, 0x55, 0x5c),
    Color32::from_rgb(0x85, 0x62, 0xf5),
    Color32::from_rgb(0xb8, 0xe0, 0x3a),
];

/// La couleur du trig de rang `index`. Elle revient après la dernière.
pub fn trig_color(index: usize) -> Color32 {
    TRIGS[index % TRIGS.len()]
}

/// À appliquer une fois, à la création du contexte egui.
pub fn apply(ctx: &egui::Context) {
    let mut style = (*ctx.global_style()).clone();

    let s = &mut style.spacing;
    s.item_spacing = egui::vec2(8.0, 6.0);
    s.button_padding = egui::vec2(10.0, 4.0);
    s.window_margin = Margin::same(12);
    s.menu_margin = Margin::same(6);
    s.slider_rail_height = 4.0;

    let v = &mut style.visuals;
    v.dark_mode = true;
    v.panel_fill = BACKGROUND;
    v.window_fill = HEADER;
    v.extreme_bg_color = BORDER;
    v.faint_bg_color = HEADER;
    // Pas de `override_text_color` : il l'emporterait sur tout, et le texte
    // d'un état choisi resterait clair sur l'or.
    v.selection.bg_fill = ACCENT;
    v.selection.stroke.color = ON_ACCENT;
    v.hyperlink_color = ACCENT;
    v.slider_trailing_fill = true;
    v.window_corner_radius = CornerRadius::same(10);
    v.menu_corner_radius = CornerRadius::same(8);
    v.window_stroke = Stroke::new(1.0, GRID_STRONG);
    v.window_shadow = Shadow {
        offset: [0, 6],
        blur: 18,
        spread: 0,
        color: Color32::from_black_alpha(120),
    };
    v.popup_shadow = v.window_shadow;

    let bords = CornerRadius::same(6);
    let w = &mut v.widgets;
    w.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    w.noninteractive.fg_stroke.color = TEXT;
    w.noninteractive.corner_radius = bords;
    for (etat, fond) in [
        (&mut w.inactive, IDLE),
        (&mut w.hovered, HOVER),
        (&mut w.open, HOVER),
        (&mut w.active, ACCENT),
    ] {
        etat.weak_bg_fill = fond;
        etat.bg_fill = fond;
        etat.corner_radius = bords;
        etat.bg_stroke = Stroke::NONE;
        etat.fg_stroke.color = TEXT;
        // egui gonfle un bouton survolé : sur une grille, il pousse ses voisins
        // à l'œil et rien ne justifie ce mouvement sur scène.
        etat.expansion = 0.0;
    }
    w.active.fg_stroke.color = ON_ACCENT;
    ctx.set_global_style(style);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn l_accent_est_applique_a_la_selection() {
        let ctx = egui::Context::default();
        apply(&ctx);
        assert_eq!(ctx.global_style().visuals.selection.bg_fill, ACCENT);
        assert_eq!(ctx.global_style().visuals.panel_fill, BACKGROUND);
    }

    #[test]
    fn le_texte_d_un_etat_choisi_reste_lisible_sur_l_or() {
        let ctx = egui::Context::default();
        apply(&ctx);
        let v = &ctx.global_style().visuals;
        assert_eq!(v.override_text_color, None);
        assert_eq!(v.selection.stroke.color, ON_ACCENT);
        assert_eq!(v.widgets.active.fg_stroke.color, ON_ACCENT);
        assert_eq!(v.widgets.inactive.fg_stroke.color, TEXT);
    }

    #[test]
    fn un_survol_ne_gonfle_pas_les_cases() {
        let ctx = egui::Context::default();
        apply(&ctx);
        assert_eq!(ctx.global_style().visuals.widgets.hovered.expansion, 0.0);
    }

    #[test]
    fn la_couleur_d_un_trig_revient_sans_sortir_de_la_palette() {
        assert_eq!(trig_color(0), TRIGS[0]);
        assert_eq!(trig_color(TRIGS.len()), TRIGS[0]);
        assert_eq!(trig_color(TRIGS.len() + 2), TRIGS[2]);
    }

    #[test]
    fn deux_trigs_voisins_n_ont_pas_la_meme_couleur() {
        for (i, a) in TRIGS.iter().enumerate() {
            assert!(TRIGS.iter().skip(i + 1).all(|b| a != b));
        }
    }
}
