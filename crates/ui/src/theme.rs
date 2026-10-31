//! La charte graphique — §23.1 de `docs/ARCHITECTURE.md`.
//!
//! Un fond sombre neutre, un seul accent réservé à l'état actif : rien ne doit
//! concurrencer les couleurs du rendu affiché dans le monitor.

use egui::Color32;

/// Fond principal : grille, panneaux, monitor.
pub const BACKGROUND: Color32 = Color32::from_rgb(0x16, 0x17, 0x19);
/// En-têtes et barres, légèrement plus clairs que le fond.
pub const HEADER: Color32 = Color32::from_rgb(0x24, 0x26, 0x29);
/// Un état au repos : une cellule, un bouton.
pub const IDLE: Color32 = Color32::from_rgb(0x2e, 0x30, 0x34);
/// L'accent unique : porte toute la sélection.
pub const ACCENT: Color32 = Color32::from_rgb(0x2d, 0xe0, 0xc8);
/// Le texte posé sur l'accent.
pub const ON_ACCENT: Color32 = Color32::from_rgb(0x0c, 0x12, 0x12);
pub const TEXT: Color32 = Color32::from_rgb(0xe8, 0xe8, 0xe6);
pub const TEXT_DIM: Color32 = Color32::from_rgb(0x8c, 0x8e, 0x90);
pub const BORDER: Color32 = Color32::from_rgb(0x08, 0x08, 0x09);

/// À appliquer une fois, à la création du contexte egui.
pub fn apply(ctx: &egui::Context) {
    let mut style = (*ctx.global_style()).clone();
    let v = &mut style.visuals;
    v.dark_mode = true;
    v.panel_fill = BACKGROUND;
    v.window_fill = BACKGROUND;
    v.extreme_bg_color = BORDER;
    v.faint_bg_color = HEADER;
    v.override_text_color = Some(TEXT);
    v.selection.bg_fill = ACCENT;
    v.selection.stroke.color = ON_ACCENT;
    v.hyperlink_color = ACCENT;
    v.widgets.noninteractive.bg_stroke.color = BORDER;
    v.widgets.inactive.weak_bg_fill = IDLE;
    v.widgets.inactive.bg_fill = IDLE;
    v.widgets.hovered.weak_bg_fill = HEADER;
    v.widgets.hovered.bg_fill = HEADER;
    v.widgets.active.weak_bg_fill = ACCENT;
    v.widgets.active.bg_fill = ACCENT;
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
}
