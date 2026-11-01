//! Le spectrogramme : l'historique du spectre, en bande sous le tracé — §23.7 de
//! `docs/ARCHITECTURE.md`.
//!
//! Une vue, pas une analyse : elle ne lit que le `Spectrum` de la frame, et rien
//! de ce qu'elle retient ne sort de l'interface. Les lignes s'écrivent dans une
//! texture en anneau, le plus récent en haut.

use crate::spectrum::{freq_marks, RIGHT_MARGIN};
use crate::theme;
use egui::epaint::Mesh;
use egui::{
    Align2, Color32, ColorImage, Context, FontId, Pos2, Rect, Role, Sense, Shape, Stroke,
    TextureHandle, TextureOptions, Ui, WidgetInfo,
};
use frogenx_core::{AmpScale, FreqScale, Spectrum, SPECTRUM_BANDS};
use std::fmt;

/// Les durées proposées, en secondes.
pub const DURATIONS: [u32; 4] = [2, 5, 10, 30];
const DEFAULT_DURATION: u32 = 5;
/// Le nombre de lignes de la texture, quelle que soit la durée.
const ROWS: usize = 256;
/// La part du panneau que prend la bande.
const SHARE: f32 = 0.4;
/// La hauteur gardée au spectre : c'est sur lui que se posent les zones.
const MIN_PLOT: f32 = 120.0;
/// Les écarts possibles entre deux repères de temps, en secondes.
const MARK_STEPS: [u32; 5] = [1, 2, 5, 10, 30];
/// La place, en pixels, qu'il faut entre deux repères de temps.
const MARK_GAP: f32 = 18.0;
/// Le niveau où la teinte atteint le gris de la courbe, avant de monter vers l'or.
const KNEE: f32 = 0.6;
/// Le centre de la première bande dans la texture : la courbe du spectre passe
/// par les centres, et le bord du tracé doit tomber sur le même son.
const FIRST_BAND: f32 = 0.5 / SPECTRUM_BANDS as f32;
const LABEL: &str = "Historique du spectre";

/// L'historique du spectre. Vide tant que la bande est fermée.
#[derive(Clone, PartialEq)]
pub struct Spectrogram {
    pub open: bool,
    seconds: u32,
    /// La ligne la plus récente de la texture. Les plus anciennes viennent après.
    head: usize,
    /// Le temps passé dans la ligne en cours, en secondes.
    waited: f32,
    /// La crête de la ligne en cours : aucun coup ne passe entre deux lignes.
    peak: [f32; SPECTRUM_BANDS],
    /// Les échelles des lignes écrites.
    scales: Option<(FreqScale, AmpScale)>,
    texture: Option<TextureHandle>,
}

impl Default for Spectrogram {
    fn default() -> Self {
        Self {
            open: false,
            seconds: DEFAULT_DURATION,
            head: 0,
            waited: 0.0,
            peak: [0.0; SPECTRUM_BANDS],
            scales: None,
            texture: None,
        }
    }
}

// À la main : une texture n'a pas de `Debug`.
impl fmt::Debug for Spectrogram {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Spectrogram")
            .field("open", &self.open)
            .field("seconds", &self.seconds)
            .field("head", &self.head)
            .finish_non_exhaustive()
    }
}

impl Spectrogram {
    pub fn seconds(&self) -> u32 {
        self.seconds
    }

    /// Ouvre la bande, ou la ferme et l'oublie.
    pub fn toggle(&mut self) {
        self.open = !self.open;
        if !self.open {
            self.forget();
        }
    }

    /// Change la durée. Les lignes écrites n'ont plus la bonne longueur : elles
    /// s'effacent.
    pub fn set_seconds(&mut self, seconds: u32) {
        if seconds != self.seconds {
            self.seconds = seconds;
            self.forget();
        }
    }

    /// Repart d'une bande vide, sans texture.
    pub fn forget(&mut self) {
        self.head = 0;
        self.waited = 0.0;
        self.peak = [0.0; SPECTRUM_BANDS];
        self.scales = None;
        self.texture = None;
    }

    /// Ajoute une frame à la ligne en cours, et dit combien de lignes sont dues.
    /// Une interface lente en doit plusieurs : le temps reste vrai.
    fn advance(&mut self, levels: &[f32; SPECTRUM_BANDS], dt: f32) -> usize {
        for (crete, niveau) in self.peak.iter_mut().zip(levels) {
            *crete = crete.max(*niveau);
        }
        let duree = self.seconds as f32 / ROWS as f32;
        self.waited += dt;
        let dues = (self.waited / duree).floor();
        self.waited -= dues * duree;
        (dues as usize).min(ROWS)
    }

    /// Écrit les lignes dues dans la texture, qui naît à la première.
    pub fn update(&mut self, ctx: &Context, spectrum: &Spectrum, dt: f32) {
        let echelles = (spectrum.freq_scale, spectrum.amp_scale);
        if self.scales != Some(echelles) {
            self.forget();
            self.scales = Some(echelles);
        }
        let dues = self.advance(&spectrum.levels, dt);
        if dues == 0 {
            return;
        }
        let ligne: Vec<Color32> = self.peak.iter().map(|v| heat(*v)).collect();
        let ligne = ColorImage::new([SPECTRUM_BANDS, 1], ligne);
        let texture = self.texture.get_or_insert_with(|| {
            let vide = ColorImage::filled([SPECTRUM_BANDS, ROWS], heat(0.0));
            ctx.load_texture("spectrogramme", vide, TextureOptions::LINEAR)
        });
        for _ in 0..dues {
            // À rebours : la ligne d'après dans la texture est la plus ancienne.
            self.head = (self.head + ROWS - 1) % ROWS;
            texture.set_partial([0, self.head], ligne.clone(), TextureOptions::LINEAR);
        }
        self.peak = [0.0; SPECTRUM_BANDS];
    }

    /// La bande, ses repères, et renvoie son tracé, sans la marge de droite.
    pub fn strip(&self, ui: &mut Ui, size: egui::Vec2, spectrum: &Spectrum) -> Rect {
        let (cadre, reponse) = ui.allocate_exact_size(size, Sense::hover());
        reponse.widget_info(|| WidgetInfo::labeled(Role::Image, true, LABEL));
        let mut trace = cadre;
        trace.set_right(cadre.right() - RIGHT_MARGIN);

        let painter = ui.painter_at(cadre);
        painter.rect_filled(trace, 0.0, heat(0.0));
        if let Some(texture) = &self.texture {
            let mut image = Mesh::with_texture(texture.id());
            for (rect, uv) in quads(self.head, trace) {
                image.add_rect_with_uv(rect, uv, Color32::WHITE);
            }
            painter.add(Shape::mesh(image));
        }
        for (x, _) in freq_marks(spectrum.freq_scale, trace.width()) {
            let trait_ = Stroke::new(1.0, theme::GRID_STRONG);
            painter.vline(trace.left() + x, trace.y_range(), trait_);
        }
        for (y, texte) in time_marks(self.seconds, trace.height()) {
            painter.text(
                Pos2::new(
                    trace.right() + 7.0,
                    // Les extrêmes sont sur le bord : centrés, ils seraient coupés.
                    (trace.top() + y * trace.height())
                        .clamp(trace.top() + 7.0, trace.bottom() - 7.0),
                ),
                Align2::LEFT_CENTER,
                texte,
                FontId::proportional(10.5),
                theme::TEXT_DIM,
            );
        }
        trace
    }
}

/// La hauteur de la bande dans `total` pixels, `0` s'il n'y a pas la place de
/// garder le spectre lisible.
pub fn strip_height(total: f32) -> f32 {
    let bande = total * SHARE;
    if total - bande >= MIN_PLOT {
        bande
    } else {
        0.0
    }
}

/// La teinte d'un niveau : du fond du tracé au gris de la courbe, puis à l'or
/// des crêtes. Aucune couleur de trig : elles restent les seules de l'interface.
pub fn heat(level: f32) -> Color32 {
    let v = level.clamp(0.0, 1.0);
    if v < KNEE {
        theme::PLOT_BOTTOM.lerp_to_gamma(theme::CURVE, v / KNEE)
    } else {
        theme::CURVE.lerp_to_gamma(theme::ACCENT, (v - KNEE) / (1.0 - KNEE))
    }
}

/// L'anneau déroulé : de `head` au bas de la texture, puis du haut à `head`.
/// Chaque morceau est un rectangle de l'écran et ses coordonnées de texture.
fn quads(head: usize, rect: Rect) -> [(Rect, Rect); 2] {
    let v = head as f32 / ROWS as f32;
    let milieu = rect.top() + (1.0 - v) * rect.height();
    let (u0, u1) = (FIRST_BAND, 1.0 - FIRST_BAND);
    [
        (
            Rect::from_x_y_ranges(rect.x_range(), rect.top()..=milieu),
            Rect::from_min_max(Pos2::new(u0, v), Pos2::new(u1, 1.0)),
        ),
        (
            Rect::from_x_y_ranges(rect.x_range(), milieu..=rect.bottom()),
            Rect::from_min_max(Pos2::new(u0, 0.0), Pos2::new(u1, v)),
        ),
    ]
}

/// Les repères de temps, de haut en bas : leur place dans la bande `[0, 1]` et
/// leur nom. Le plus petit écart que la hauteur laisse lisible.
pub fn time_marks(seconds: u32, height: f32) -> Vec<(f32, String)> {
    let lisible = |pas: &&u32| **pas as f32 / seconds as f32 * height >= MARK_GAP;
    let Some(pas) = MARK_STEPS.iter().find(lisible) else {
        return vec![(0.0, "0".to_string())];
    };
    (0..=seconds / pas)
        .map(|i| {
            let s = i * pas;
            let nom = if s == 0 {
                "0".to_string()
            } else {
                format!("-{s} s")
            };
            (s as f32 / seconds as f32, nom)
        })
        .collect()
}

/// Le nom d'une durée, tel que le liste son menu.
pub fn duration_label(seconds: u32) -> &'static str {
    match seconds {
        2 => "2 s",
        5 => "5 s",
        10 => "10 s",
        _ => "30 s",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DUREE: f32 = 5.0 / ROWS as f32;

    fn plat(niveau: f32) -> Spectrum {
        Spectrum::flat(niveau)
    }

    #[test]
    fn une_ligne_est_une_duree_pas_une_frame() {
        let mut s = Spectrogram::default();
        let niveaux = [0.0; SPECTRUM_BANDS];
        assert_eq!(s.advance(&niveaux, DUREE * 0.4), 0);
        assert_eq!(s.advance(&niveaux, DUREE * 0.7), 1, "le reste compte");
        assert_eq!(s.advance(&niveaux, DUREE * 3.0), 3, "elle se rattrape");
    }

    #[test]
    fn un_trou_ne_reecrit_jamais_plus_que_la_bande() {
        let mut s = Spectrogram::default();
        assert_eq!(s.advance(&[0.0; SPECTRUM_BANDS], 3600.0), ROWS);
    }

    #[test]
    fn une_ligne_garde_la_crete_de_ses_frames() {
        let ctx = Context::default();
        let mut s = Spectrogram::default();
        let mut coup = plat(0.0);
        coup.levels[3] = 0.9;
        s.update(&ctx, &coup, DUREE * 0.1);
        assert_eq!(s.peak[3], 0.9);
        s.update(&ctx, &plat(0.1), DUREE * 0.1);
        assert_eq!(s.peak[3], 0.9, "le coup reste");
        assert_eq!(s.peak[4], 0.1);
        s.update(&ctx, &plat(0.0), DUREE);
        assert_eq!(s.peak[3], 0.0, "la ligne écrite, on repart");
    }

    #[test]
    fn la_ligne_la_plus_recente_recule_dans_l_anneau() {
        let ctx = Context::default();
        let mut s = Spectrogram::default();
        s.update(&ctx, &plat(0.5), DUREE * 1.5);
        assert_eq!(s.head, ROWS - 1);
        s.update(&ctx, &plat(0.5), DUREE * 2.0);
        assert_eq!(s.head, ROWS - 3);
        assert!(s.texture.is_some());
    }

    #[test]
    fn la_texture_ne_nait_qu_a_la_premiere_ligne() {
        let ctx = Context::default();
        let mut s = Spectrogram::default();
        s.update(&ctx, &plat(0.5), DUREE * 0.5);
        assert!(s.texture.is_none());
    }

    #[test]
    fn changer_d_echelle_efface_l_historique() {
        let ctx = Context::default();
        let mut s = Spectrogram::default();
        s.update(&ctx, &plat(0.5), DUREE * 2.0);
        assert_eq!(s.head, ROWS - 2);

        let mut autre = plat(0.5);
        autre.freq_scale = FreqScale::Mel;
        s.update(&ctx, &autre, 0.0);
        assert_eq!(s.head, 0);
        assert!(s.texture.is_none());

        s.update(&ctx, &autre, DUREE);
        autre.amp_scale = AmpScale::Linear;
        s.update(&ctx, &autre, 0.0);
        assert!(s.texture.is_none(), "l'amplitude aussi");
    }

    #[test]
    fn changer_de_duree_efface_l_historique() {
        let ctx = Context::default();
        let mut s = Spectrogram::default();
        s.update(&ctx, &plat(0.5), DUREE);

        s.set_seconds(s.seconds());
        assert!(s.texture.is_some(), "la même durée ne change rien");

        s.set_seconds(10);
        assert_eq!(s.seconds(), 10);
        assert!(s.texture.is_none());
    }

    #[test]
    fn fermer_oublie_mais_garde_la_duree() {
        let ctx = Context::default();
        let mut s = Spectrogram::default();
        s.set_seconds(30);
        s.toggle();
        assert!(s.open);
        s.update(&ctx, &plat(0.5), 1.0);
        assert!(s.texture.is_some());
        s.toggle();
        assert!(!s.open && s.texture.is_none());
        assert_eq!(s.seconds(), 30);
    }

    #[test]
    fn la_bande_laisse_le_spectre_lisible_ou_ne_s_ouvre_pas() {
        assert!((strip_height(260.0) - 104.0).abs() < 1e-3);
        assert_eq!(strip_height(190.0), 0.0);
        for total in [200.0, 260.0, 900.0] {
            assert!(total - strip_height(total) >= MIN_PLOT, "{total}");
        }
    }

    #[test]
    fn la_teinte_va_du_fond_a_l_or_sans_s_assombrir() {
        assert_eq!(heat(0.0), theme::PLOT_BOTTOM);
        assert_eq!(heat(KNEE), theme::CURVE);
        assert_eq!(heat(1.0), theme::ACCENT);
        assert_eq!(
            heat(4.0),
            theme::ACCENT,
            "un niveau hors plage reste lisible"
        );
        let clarte = |c: Color32| c.r() as u32 + c.g() as u32;
        let teintes: Vec<u32> = (0..=20).map(|i| clarte(heat(i as f32 / 20.0))).collect();
        assert!(teintes.windows(2).all(|p| p[0] <= p[1]));
    }

    #[test]
    fn l_anneau_se_deroule_du_plus_recent_au_plus_ancien() {
        let r = Rect::from_min_size(Pos2::new(10.0, 20.0), egui::vec2(100.0, 200.0));
        let [haut, bas] = quads(ROWS / 4, r);
        assert_eq!(haut.0.top(), 20.0);
        assert_eq!(haut.0.bottom(), 170.0);
        assert_eq!((haut.1.top(), haut.1.bottom()), (0.25, 1.0));
        assert_eq!(bas.0.bottom(), 220.0);
        assert_eq!((bas.1.top(), bas.1.bottom()), (0.0, 0.25));
    }

    #[test]
    fn un_anneau_cale_sur_zero_tient_en_un_morceau() {
        let r = Rect::from_min_size(Pos2::ZERO, egui::vec2(100.0, 200.0));
        let [haut, bas] = quads(0, r);
        assert_eq!(haut.0.height(), 200.0);
        assert_eq!(bas.0.height(), 0.0);
    }

    #[test]
    fn une_bande_tombe_a_l_aplomb_de_la_courbe() {
        // La bande `i` est à `i / (n - 1)` du tracé, comme sur le spectre.
        let r = Rect::from_min_size(Pos2::ZERO, egui::vec2(100.0, 100.0));
        let uv = quads(0, r)[0].1;
        let n = SPECTRUM_BANDS as f32;
        for i in [0.0, 40.0, n - 1.0] {
            let x = i / (n - 1.0);
            let u = uv.left() + x * uv.width();
            assert!((u * n - (i + 0.5)).abs() < 1e-3, "bande {i}");
        }
    }

    #[test]
    fn les_reperes_de_temps_partent_de_maintenant() {
        let marks = time_marks(10, 200.0);
        assert_eq!(marks[0], (0.0, "0".to_string()));
        assert_eq!(marks[1], (0.1, "-1 s".to_string()));
        assert_eq!(marks.last().unwrap(), &(1.0, "-10 s".to_string()));
    }

    #[test]
    fn les_reperes_de_temps_s_espacent_quand_la_place_manque() {
        for seconds in DURATIONS {
            for hauteur in [20.0, 60.0, 106.0, 400.0] {
                let marks = time_marks(seconds, hauteur);
                assert!(!marks.is_empty());
                for paire in marks.windows(2) {
                    let ecart = (paire[1].0 - paire[0].0) * hauteur;
                    assert!(ecart >= MARK_GAP - 1e-3, "{seconds} s, {hauteur}");
                }
            }
        }
        assert_eq!(time_marks(30, 106.0)[1].1, "-10 s");
    }

    #[test]
    fn une_bande_trop_basse_garde_son_repere_d_origine() {
        assert_eq!(time_marks(5, 4.0), vec![(0.0, "0".to_string())]);
    }

    #[test]
    fn chaque_duree_a_son_nom() {
        let noms: Vec<&str> = DURATIONS.iter().map(|d| duration_label(*d)).collect();
        assert_eq!(noms, ["2 s", "5 s", "10 s", "30 s"]);
    }
}
