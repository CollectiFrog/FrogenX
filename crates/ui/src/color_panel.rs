//! Le sélecteur de couleur du panneau d'un clip — §23.5 de
//! `docs/ARCHITECTURE.md`.
//!
//! Il s'ajoute aux potentiomètres : une couleur reste quatre `Param`, et il
//! n'émet que des `SetParam`. Le RVB seul : l'alpha garde son potentiomètre.

use crate::theme;
use egui::{
    Color32, Mesh, Pos2, Rect, Response, RichText, Role, Sense, Shape, Stroke, StrokeKind, Ui,
    WidgetInfo,
};
use frogenx_composition::{Clip, ClipId, Command, ParamGroup, ParamSlot};

/// La taille d'une pastille.
const SWATCH_SIZE: egui::Vec2 = egui::vec2(22.0, 11.0);
/// Le côté du champ saturation-valeur.
const FIELD_SIZE: f32 = 132.0;
/// La largeur de la barre de teinte.
const HUE_WIDTH: f32 = 16.0;
/// Les cases du champ : un dégradé par sommets n'a pas besoin de plus.
const FIELD_CELLS: usize = 16;
/// Les tranches de la barre de teinte, une par couleur pure et ses transitions.
const HUE_CELLS: usize = 36;

/// Une couleur en teinte, saturation, valeur, chacune dans `[0, 1]`, calculée
/// sur les canaux sRGB tels qu'on les saisit (§21).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hsv {
    pub h: f32,
    pub s: f32,
    pub v: f32,
}

impl Hsv {
    pub fn from_rgb([r, g, b]: [f32; 3]) -> Self {
        let (max, min) = (r.max(g).max(b), r.min(g).min(b));
        let ecart = max - min;
        let h = if ecart <= 0.0 {
            0.0
        } else if max == r {
            ((g - b) / ecart).rem_euclid(6.0) / 6.0
        } else if max == g {
            ((b - r) / ecart + 2.0) / 6.0
        } else {
            ((r - g) / ecart + 4.0) / 6.0
        };
        let s = if max > 0.0 { ecart / max } else { 0.0 };
        Self { h, s, v: max }
    }

    pub fn to_rgb(self) -> [f32; 3] {
        let secteur = self.h.rem_euclid(1.0) * 6.0;
        let c = self.v * self.s;
        let x = c * (1.0 - (secteur.rem_euclid(2.0) - 1.0).abs());
        let (r, g, b) = match secteur as u32 {
            0 => (c, x, 0.0),
            1 => (x, c, 0.0),
            2 => (0.0, c, x),
            3 => (0.0, x, c),
            4 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        let m = self.v - c;
        [r + m, g + m, b + m]
    }
}

/// Ce que le sélecteur retient d'un geste à l'autre.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Held {
    hsv: Hsv,
    /// La couleur avant le dernier geste, et celle qu'il a choisie : la commande
    /// ne s'applique qu'à la frame suivante, les deux sont donc légitimes.
    before: [u8; 3],
    chosen: [u8; 3],
}

/// La teinte d'un gris ou d'un noir ne se lit pas dans ses canaux : sans cette
/// mémoire, passer par le noir ferait perdre la teinte choisie. Une couleur
/// venue d'ailleurs (un potentiomètre, annuler) reprend ses droits.
fn retained(held: Option<Held>, bases: [f32; 3]) -> Hsv {
    let octets = bases.map(to_byte);
    held.filter(|m| m.before == octets || m.chosen == octets)
        .map_or_else(|| Hsv::from_rgb(bases), |m| m.hsv)
}

/// Une position du pointeur, dans `[0, 1]` sur chaque axe de `cadre`.
fn normalized(cadre: Rect, p: Pos2) -> (f32, f32) {
    (
        ((p.x - cadre.left()) / cadre.width()).clamp(0.0, 1.0),
        ((p.y - cadre.top()) / cadre.height()).clamp(0.0, 1.0),
    )
}

/// Un quadrillage de `nx` par `ny` cases dont `couleur` donne chaque sommet.
fn gradient_mesh(
    cadre: Rect,
    nx: usize,
    ny: usize,
    couleur: impl Fn(f32, f32) -> [f32; 3],
) -> Mesh {
    let mut mesh = Mesh::default();
    for y in 0..=ny {
        for x in 0..=nx {
            let (u, v) = (x as f32 / nx as f32, y as f32 / ny as f32);
            let point = cadre.left_top() + egui::vec2(u * cadre.width(), v * cadre.height());
            mesh.colored_vertex(point, to_color(couleur(u, v)));
        }
    }
    let rang = |x: usize, y: usize| (y * (nx + 1) + x) as u32;
    for y in 0..ny {
        for x in 0..nx {
            let (a, b) = (rang(x, y), rang(x + 1, y));
            let (c, d) = (rang(x + 1, y + 1), rang(x, y + 1));
            mesh.add_triangle(a, b, c);
            mesh.add_triangle(a, c, d);
        }
    }
    mesh
}

/// Une surface que l'on règle à la souris, nommée pour l'accessibilité.
fn gesture_area(ui: &mut Ui, taille: egui::Vec2, nom: &str) -> (Rect, Response) {
    let (cadre, reponse) = ui.allocate_exact_size(taille, Sense::click_and_drag());
    let nom = nom.to_string();
    reponse.widget_info(|| WidgetInfo::labeled(Role::Slider, true, nom.clone()));
    (cadre, reponse)
}

/// Le champ saturation-valeur : la saturation à l'horizontale, la valeur vers le
/// haut. Rend `true` si le geste l'a changé.
fn value_field(ui: &mut Ui, hsv: &mut Hsv) -> bool {
    let (cadre, reponse) = gesture_area(ui, egui::Vec2::splat(FIELD_SIZE), "Saturation et valeur");
    let h = hsv.h;
    let mesh = gradient_mesh(cadre, FIELD_CELLS, FIELD_CELLS, |s, v| {
        Hsv { h, s, v: 1.0 - v }.to_rgb()
    });
    let peintre = ui.painter();
    peintre.add(Shape::mesh(mesh));
    let point =
        cadre.left_top() + egui::vec2(hsv.s * cadre.width(), (1.0 - hsv.v) * cadre.height());
    peintre.circle_stroke(point, 4.0, Stroke::new(1.5, Color32::WHITE));
    peintre.circle_stroke(point, 5.0, Stroke::new(1.0, Color32::BLACK));
    let Some(p) = reponse.interact_pointer_pos() else {
        return false;
    };
    let (s, v) = normalized(cadre, p);
    let suite = Hsv {
        s,
        v: 1.0 - v,
        ..*hsv
    };
    let change = suite != *hsv;
    *hsv = suite;
    change
}

/// La barre de teinte. Rend `true` si le geste l'a changée.
fn hue_bar(ui: &mut Ui, hsv: &mut Hsv) -> bool {
    let taille = egui::vec2(HUE_WIDTH, FIELD_SIZE);
    let (cadre, reponse) = gesture_area(ui, taille, "Teinte");
    let mesh = gradient_mesh(cadre, 1, HUE_CELLS, |_, h| {
        Hsv { h, s: 1.0, v: 1.0 }.to_rgb()
    });
    let peintre = ui.painter();
    peintre.add(Shape::mesh(mesh));
    let y = cadre.top() + hsv.h * cadre.height();
    peintre.hline(cadre.x_range(), y, Stroke::new(2.0, Color32::WHITE));
    let Some(p) = reponse.interact_pointer_pos() else {
        return false;
    };
    let h = normalized(cadre, p).1;
    let change = h != hsv.h;
    hsv.h = h;
    change
}

/// Une couleur du clip, vue par le panneau.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Swatch {
    /// Le rouge : ce qui désigne cette couleur quand le sélecteur est ouvert.
    pub key: ParamSlot,
    slots: [ParamSlot; 3],
    /// Ce que règle le sélecteur : les bases, comme un potentiomètre.
    bases: [f32; 3],
    /// Ce qui joue, modulation comprise.
    live: [f32; 3],
}

/// Un canal sur 8 bits : un `Param` modulé peut sortir de `[0, 1]` (§18.4).
fn to_byte(canal: f32) -> u8 {
    (canal.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn to_color(canaux: [f32; 3]) -> Color32 {
    let [r, g, b] = canaux.map(to_byte);
    Color32::from_rgb(r, g, b)
}

impl Swatch {
    /// La couleur de ce groupe de paramètres, s'il en est une.
    pub fn of(clip: &Clip, group: &ParamGroup) -> Option<Self> {
        let slots = match group.params.first()?.slot {
            ParamSlot::Fill(_) => [0, 1, 2].map(ParamSlot::Fill),
            ParamSlot::Stroke(_) => [1, 2, 3].map(ParamSlot::Stroke),
            _ => return None,
        };
        let (mut bases, mut live) = ([0.0; 3], [0.0; 3]);
        for (i, slot) in slots.iter().enumerate() {
            let desc = group.params.iter().find(|p| p.slot == *slot)?.desc;
            bases[i] = desc.value.base();
            live[i] = desc.value.get(clip.graph());
        }
        Some(Self {
            key: slots[0],
            slots,
            bases,
            live,
        })
    }

    /// Ce que le choix change. Un canal que le geste n'a pas touché n'émet
    /// rien : le choix se fait sur 8 bits, et le réécrire lui ferait perdre sa
    /// précision.
    fn changes(&self, choix: [u8; 3]) -> Vec<(ParamSlot, f32)> {
        let avant = self.bases.map(to_byte);
        (0..3)
            .filter(|i| choix[*i] != avant[*i])
            .map(|i| (self.slots[i], choix[i] as f32 / 255.0))
            .collect()
    }
}

/// La pastille sur le titre d'une carte : la couleur telle qu'elle joue. Rend
/// `true` au clic.
pub fn swatch_button(ui: &mut Ui, titre: &str, swatch: &Swatch, ouvert: bool) -> bool {
    let (cadre, reponse) = ui.allocate_exact_size(SWATCH_SIZE, Sense::click());
    reponse.widget_info(|| WidgetInfo::labeled(Role::Button, true, format!("Couleur : {titre}")));
    let bord = if ouvert {
        theme::ACCENT
    } else {
        theme::TEXT_DIM
    };
    let peintre = ui.painter();
    peintre.rect_filled(cadre, 2.0, to_color(swatch.live));
    peintre.rect_stroke(cadre, 2.0, Stroke::new(1.0, bord), StrokeKind::Outside);
    reponse.clicked()
}

/// Le sélecteur, sur une rangée du panneau : il ne couvre jamais l'image
/// (§23.9), et les cartes se plient sans lui. Il émet des commandes, n'écrit
/// rien (§23).
pub fn color_row(
    ui: &mut Ui,
    titre: &str,
    clip: ClipId,
    swatch: &Swatch,
    commands: &mut Vec<Command>,
) {
    let memoire = ui
        .scope_id()
        .with(format!("couleur {clip:?} {:?}", swatch.key));
    let held = ui.data(|d| d.get_temp::<Held>(memoire));
    let mut hsv = retained(held, swatch.bases);
    let mut change = false;
    ui.vertical(|ui| {
        ui.label(RichText::new(titre).size(10.5).color(theme::TEXT_DIM));
        ui.horizontal(|ui| {
            change |= value_field(ui, &mut hsv);
            change |= hue_bar(ui, &mut hsv);
        });
    });
    if change {
        let avant = swatch.bases.map(to_byte);
        let chosen = hsv.to_rgb().map(to_byte);
        for (slot, value) in swatch.changes(chosen) {
            commands.push(Command::SetParam { clip, slot, value });
        }
        ui.data_mut(|d| {
            d.insert_temp(
                memoire,
                Held {
                    hsv,
                    before: avant,
                    chosen,
                },
            )
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::{kittest::Queryable, Harness};
    use frogenx_composition::{Composition, PlayMode, Transport};
    use frogenx_core::{Constant, Graph, Param};
    use frogenx_shape::{ColorParam, Ellipse, ShapeStyle, StrokeStyle};

    fn clip() -> Clip {
        let mut g = Graph::new();
        let source = g.add(Box::new(Constant(1.0)));
        g.eval_frame(0.0, 1.0 / 60.0);
        let transport = Transport::bpm_sync(&mut g, Param::fixed(4.0), PlayMode::Loop);
        let mut remplissage = ColorParam::srgb(0.25, 0.5, 0.123, 1.0);
        remplissage.r = Param::modulated(0.25, 0.5, source);
        let style = ShapeStyle {
            fill: Some(remplissage),
            stroke: Some(StrokeStyle {
                width: Param::fixed(0.01),
                color: ColorParam::srgb(0.0, 1.0, 0.0, 0.5),
            }),
        };
        Clip::shape(g, Box::new(Ellipse::circle(0.5)), transport).with_style(style)
    }

    fn nuancier(clip: &Clip, titre: &str) -> Option<Swatch> {
        let groupes = clip.params();
        let groupe = groupes.iter().find(|g| g.label == titre)?;
        Swatch::of(clip, groupe)
    }

    #[test]
    fn le_remplissage_et_le_trait_sont_des_couleurs() {
        let c = clip();
        assert_eq!(nuancier(&c, "Remplissage").unwrap().key, ParamSlot::Fill(0));
        let trait_ = nuancier(&c, "Trait").unwrap();
        assert_eq!(trait_.key, ParamSlot::Stroke(1), "pas l'épaisseur");
        assert_eq!(trait_.bases, [0.0, 1.0, 0.0]);
    }

    #[test]
    fn un_generateur_n_est_pas_une_couleur() {
        let c = clip();
        let groupes = c.params();
        assert_eq!(Swatch::of(&c, &groupes[0]), None);
    }

    #[test]
    fn la_pastille_montre_ce_qui_joue_et_le_selecteur_regle_la_base() {
        let c = clip();
        let s = nuancier(&c, "Remplissage").unwrap();
        assert_eq!(s.bases[0], 0.25);
        assert_eq!(
            s.live[0],
            c.graph().value(c.graph().nodes()[0].id) * 0.5 + 0.25
        );
        assert_ne!(s.live[0], s.bases[0]);
    }

    #[test]
    fn un_canal_module_hors_de_l_intervalle_reste_une_couleur() {
        assert_eq!(to_byte(1.5), 255);
        assert_eq!(to_byte(-0.5), 0);
        assert_eq!(to_byte(f32::NAN), 0);
    }

    #[test]
    fn seul_le_canal_change_est_emis() {
        let s = nuancier(&clip(), "Remplissage").unwrap();
        let avant = s.bases.map(to_byte);
        assert!(s.changes(avant).is_empty(), "aucune écriture sans geste");
        let choix = [avant[0], avant[1], 200];
        assert_eq!(
            s.changes(choix),
            vec![(ParamSlot::Fill(2), 200.0 / 255.0)],
            "le rouge et le vert gardent leur précision"
        );
    }

    #[test]
    fn un_canal_du_trait_vise_son_propre_emplacement() {
        let s = nuancier(&clip(), "Trait").unwrap();
        assert_eq!(s.changes([0, 255, 255]), vec![(ParamSlot::Stroke(3), 1.0)]);
    }

    #[test]
    fn la_couleur_passe_en_srgb_sans_conversion() {
        assert_eq!(
            to_color([1.0, 0.5, 0.0]),
            Color32::from_rgb(255, 128, 0),
            "les canaux sont saisis en sRGB (§21)"
        );
    }

    fn proche(a: [f32; 3], b: [f32; 3]) -> bool {
        a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5)
    }

    #[test]
    fn les_couleurs_pures_tombent_sur_leur_teinte() {
        for (rgb, h) in [
            ([1.0, 0.0, 0.0], 0.0),
            ([1.0, 1.0, 0.0], 1.0 / 6.0),
            ([0.0, 1.0, 0.0], 2.0 / 6.0),
            ([0.0, 1.0, 1.0], 3.0 / 6.0),
            ([0.0, 0.0, 1.0], 4.0 / 6.0),
            ([1.0, 0.0, 1.0], 5.0 / 6.0),
        ] {
            let hsv = Hsv::from_rgb(rgb);
            assert!((hsv.h - h).abs() < 1e-6, "{rgb:?} : {}", hsv.h);
            assert_eq!((hsv.s, hsv.v), (1.0, 1.0));
        }
    }

    #[test]
    fn une_couleur_revient_d_un_aller_retour() {
        for rgb in [
            [0.25, 0.5, 0.125],
            [0.9, 0.1, 0.4],
            [0.1, 0.2, 0.95],
            [0.5, 0.5, 0.5],
            [0.0, 0.0, 0.0],
            [1.0, 1.0, 1.0],
        ] {
            let retour = Hsv::from_rgb(rgb).to_rgb();
            assert!(proche(retour, rgb), "{rgb:?} : {retour:?}");
        }
    }

    #[test]
    fn la_fin_de_la_barre_de_teinte_rejoint_son_debut() {
        let tour = Hsv {
            h: 1.0,
            s: 1.0,
            v: 1.0,
        }
        .to_rgb();
        assert!(proche(tour, [1.0, 0.0, 0.0]), "{tour:?}");
    }

    fn tenu(h: f32) -> Held {
        Held {
            hsv: Hsv { h, s: 0.0, v: 0.0 },
            before: [255, 0, 0],
            chosen: [0, 0, 0],
        }
    }

    #[test]
    fn passer_par_le_noir_garde_la_teinte() {
        assert_eq!(retained(Some(tenu(0.6)), [0.0, 0.0, 0.0]).h, 0.6);
    }

    #[test]
    fn la_couleur_d_avant_vaut_encore_tant_que_la_commande_n_a_pas_joue() {
        assert_eq!(retained(Some(tenu(0.6)), [1.0, 0.0, 0.0]).h, 0.6);
    }

    #[test]
    fn une_couleur_venue_d_ailleurs_reprend_ses_droits() {
        let lu = retained(Some(tenu(0.6)), [0.0, 1.0, 0.0]);
        assert!((lu.h - 2.0 / 6.0).abs() < 1e-6);
    }

    #[test]
    fn un_pointeur_hors_du_cadre_reste_dans_l_intervalle() {
        let cadre = Rect::from_min_size(Pos2::new(10.0, 10.0), egui::vec2(100.0, 100.0));
        assert_eq!(normalized(cadre, Pos2::new(60.0, 35.0)), (0.5, 0.25));
        assert_eq!(normalized(cadre, Pos2::new(-5.0, 500.0)), (0.0, 1.0));
    }

    #[test]
    fn un_quadrillage_a_un_sommet_par_coin_de_case() {
        let cadre = Rect::from_min_size(Pos2::ZERO, egui::vec2(10.0, 10.0));
        let mesh = gradient_mesh(cadre, 2, 3, |_, _| [1.0, 0.0, 0.0]);
        assert_eq!(mesh.vertices.len(), 12);
        assert_eq!(mesh.indices.len(), 2 * 3 * 6);
    }

    fn harness_selecteur<'a>(id: ClipId, swatch: Swatch) -> Harness<'a, Vec<Command>> {
        let mut h = Harness::builder()
            .with_size(egui::vec2(320.0, 240.0))
            .build_ui_state(
                move |ui, commands: &mut Vec<Command>| {
                    color_row(ui, "Remplissage", id, &swatch, commands);
                },
                Vec::new(),
            );
        h.run();
        h
    }

    fn clip_pose() -> (ClipId, Swatch) {
        let mut composition = Composition::new();
        let deck = composition.add_deck("test");
        let groupe = composition.add_layer_group(deck).unwrap();
        let piste = composition.add_layer(groupe).unwrap();
        let id = composition.add_clip(piste, clip()).unwrap();
        (
            id,
            nuancier(composition.clip(id).unwrap(), "Remplissage").unwrap(),
        )
    }

    #[test]
    fn un_clic_dans_le_champ_emet_les_canaux_du_clip() {
        let (id, swatch) = clip_pose();
        let mut h = harness_selecteur(id, swatch);
        assert!(h.state().is_empty(), "rien sans geste");
        h.get_by_label("Saturation et valeur").click();
        h.run();
        assert!(!h.state().is_empty());
        assert!(h
            .state()
            .iter()
            .all(|c| matches!(c, Command::SetParam { clip, .. } if *clip == id)));
    }

    #[test]
    fn un_clic_sur_la_barre_change_la_teinte() {
        let (id, swatch) = clip_pose();
        let mut h = harness_selecteur(id, swatch);
        h.get_by_label("Teinte").click();
        h.run();
        assert!(!h.state().is_empty());
    }
}
