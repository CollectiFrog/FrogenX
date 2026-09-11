//! Les primitives géométriques — §5.2.
//!
//! Chaque paramètre est un [`Param`], donc modulable. Les primitives sont
//! volontairement ennuyeuses seules : c'est la composition avec les opérateurs
//! (§5.4) qui produit la richesse.

use crate::generator::{ShapeGenerator, MAX_POINTS, SMOOTH_SEGMENTS};
use frogenx_core::{Ctx, Graph, Param};
use glam::Vec2;
use std::f32::consts::TAU;

/// Nombre de points effectif d'une forme, saturé dans un domaine sain.
///
/// Saturation **chez le consommateur** (§18.4) : une modulation dégénérée
/// produit une forme dégradée, jamais un gel ni une panique.
fn resolution(n: f32, min: usize) -> usize {
    if !n.is_finite() {
        return min;
    }
    (n.round().max(min as f32) as usize).min(MAX_POINTS)
}

/// Cercle ou ellipse.
///
/// L'excentricité écrase l'axe Y : `1.0` donne un cercle, `0.5` une ellipse
/// deux fois plus plate.
pub struct Ellipse {
    pub radius: Param,
    /// Rapport hauteur/largeur. `1.0` = cercle.
    pub eccentricity: Param,
    pub segments: Param,
}

impl Ellipse {
    pub fn circle(radius: f32) -> Self {
        Self {
            radius: Param::fixed(radius),
            eccentricity: Param::fixed(1.0),
            segments: Param::fixed(SMOOTH_SEGMENTS as f32),
        }
    }

    pub fn with_radius(mut self, r: Param) -> Self {
        self.radius = r;
        self
    }

    pub fn with_eccentricity(mut self, e: Param) -> Self {
        self.eccentricity = e;
        self
    }
}

impl ShapeGenerator for Ellipse {
    fn generate(&self, _t: f64, _ctx: &Ctx, g: &Graph, out: &mut Vec<Vec2>) {
        out.clear();
        // Un rayon négatif viendrait d'une modulation trop profonde : on le
        // sature plutôt que de produire une forme retournée.
        let r = self.radius.get(g).max(0.0);
        let e = self.eccentricity.get(g);
        let e = if e.is_finite() { e } else { 1.0 };
        let n = resolution(self.segments.get(g), 3);

        for i in 0..n {
            let a = i as f32 / n as f32 * TAU;
            out.push(Vec2::new(a.cos() * r, a.sin() * r * e));
        }
    }
}

/// Polygone régulier à **nombre de côtés flottant**.
///
/// C'est la primitive la plus rentable du système (§5.2). Le nombre de côtés
/// étant un `Param` continu, un polygone morphe sans discontinuité de triangle
/// à carré à pentagone — et au-delà de ~32 côtés, il devient visuellement un
/// cercle.
///
/// **Comment le morphing continu fonctionne :** on échantillonne le contour à
/// résolution fixe et on calcule, pour chaque angle, le rayon du polygone
/// régulier correspondant. La partie fractionnaire du nombre de côtés déplace
/// donc les sommets progressivement au lieu d'en ajouter un d'un coup.
pub struct Polygon {
    /// Nombre de côtés, **flottant et modulable**. Saturé dans `[2, 64]`.
    pub sides: Param,
    pub radius: Param,
    /// Rotation en **radians** (§17).
    pub rotation: Param,
    /// Arrondi des angles, `[0, 1]`. `0` = anguleux, `1` = cercle.
    pub corner_radius: Param,
    pub segments: Param,
}

impl Polygon {
    pub fn new(sides: f32, radius: f32) -> Self {
        Self {
            sides: Param::fixed(sides),
            radius: Param::fixed(radius),
            rotation: Param::fixed(0.0),
            corner_radius: Param::fixed(0.0),
            segments: Param::fixed(SMOOTH_SEGMENTS as f32),
        }
    }

    pub fn with_sides(mut self, s: Param) -> Self {
        self.sides = s;
        self
    }

    pub fn with_rotation(mut self, r: Param) -> Self {
        self.rotation = r;
        self
    }

    pub fn with_corner_radius(mut self, c: Param) -> Self {
        self.corner_radius = c;
        self
    }
}

/// Rayon du polygone régulier à `sides` côtés, à l'angle `a`.
///
/// Formule classique : le contour d'un polygone régulier inscrit est
/// `cos(π/n) / cos((a mod 2π/n) - π/n)`. Elle est continue en `n`, ce qui est
/// exactement ce qui rend le morphing possible.
fn polygon_radius(a: f32, sides: f32) -> f32 {
    let seg = TAU / sides;
    let half = seg * 0.5;
    // Angle replié dans un secteur, centré sur le milieu du côté.
    let local = a.rem_euclid(seg) - half;
    (half.cos() / local.cos()).clamp(0.0, 4.0)
}

impl ShapeGenerator for Polygon {
    fn generate(&self, _t: f64, _ctx: &Ctx, g: &Graph, out: &mut Vec<Vec2>) {
        out.clear();
        let sides = self.sides.get(g);
        // Sous 2 côtés la formule dégénère ; au-delà de 64 c'est un cercle.
        let sides = if sides.is_finite() {
            sides.clamp(2.0, 64.0)
        } else {
            3.0
        };
        let r = self.radius.get(g).max(0.0);
        let rot = self.rotation.get(g);
        let rot = if rot.is_finite() { rot } else { 0.0 };
        let round = self.corner_radius.get(g).clamp(0.0, 1.0);
        let n = resolution(self.segments.get(g), 3);

        for i in 0..n {
            let a = i as f32 / n as f32 * TAU;
            // L'arrondi interpole vers le cercle (rayon constant 1.0).
            let pr = polygon_radius(a, sides);
            let radius = (pr * (1.0 - round) + round) * r;
            let angle = a + rot;
            out.push(Vec2::new(angle.cos() * radius, angle.sin() * radius));
        }
    }
}

/// Étoile à branches.
///
/// `points` est le nombre de branches, `inner_ratio` le rapport entre le rayon
/// creux et le rayon des pointes. Un ratio de `1.0` donne un polygone régulier.
pub struct Star {
    pub points: Param,
    pub radius: Param,
    /// Rayon intérieur relatif, `[0, 1]`.
    pub inner_ratio: Param,
    pub rotation: Param,
}

impl Star {
    pub fn new(points: f32, radius: f32, inner_ratio: f32) -> Self {
        Self {
            points: Param::fixed(points),
            radius: Param::fixed(radius),
            inner_ratio: Param::fixed(inner_ratio),
            rotation: Param::fixed(0.0),
        }
    }

    pub fn with_inner_ratio(mut self, r: Param) -> Self {
        self.inner_ratio = r;
        self
    }
}

impl ShapeGenerator for Star {
    fn generate(&self, _t: f64, _ctx: &Ctx, g: &Graph, out: &mut Vec<Vec2>) {
        out.clear();
        let p = self.points.get(g);
        let p = if p.is_finite() {
            p.clamp(2.0, 64.0)
        } else {
            5.0
        };
        let branches = p.round() as usize;
        let r = self.radius.get(g).max(0.0);
        let inner = self.inner_ratio.get(g).clamp(0.0, 1.0);
        let rot = self.rotation.get(g);
        let rot = if rot.is_finite() { rot } else { 0.0 };

        // Deux sommets par branche : une pointe, un creux.
        for i in 0..branches * 2 {
            let a = i as f32 / (branches * 2) as f32 * TAU + rot;
            let radius = if i % 2 == 0 { r } else { r * inner };
            out.push(Vec2::new(a.cos() * radius, a.sin() * radius));
        }
    }
}

/// Rectangle à coins arrondis.
pub struct Rect {
    pub width: Param,
    pub height: Param,
    /// Rayon des coins, en unités d'espace. Saturé à la moitié du petit côté.
    pub corner_radius: Param,
    /// Segments par coin arrondi.
    pub corner_segments: Param,
}

impl Rect {
    pub fn new(width: f32, height: f32) -> Self {
        Self {
            width: Param::fixed(width),
            height: Param::fixed(height),
            corner_radius: Param::fixed(0.0),
            corner_segments: Param::fixed(8.0),
        }
    }

    pub fn with_corner_radius(mut self, r: Param) -> Self {
        self.corner_radius = r;
        self
    }
}

impl ShapeGenerator for Rect {
    fn generate(&self, _t: f64, _ctx: &Ctx, g: &Graph, out: &mut Vec<Vec2>) {
        out.clear();
        let w = self.width.get(g).max(0.0) * 0.5;
        let h = self.height.get(g).max(0.0) * 0.5;
        // Un rayon supérieur à la moitié du petit côté produirait des coins
        // qui se croisent : saturation chez le consommateur (§18.4).
        let cr = self.corner_radius.get(g).clamp(0.0, w.min(h));
        let cs = resolution(self.corner_segments.get(g), 1);

        if cr <= f32::EPSILON {
            out.extend_from_slice(&[
                Vec2::new(w, h),
                Vec2::new(-w, h),
                Vec2::new(-w, -h),
                Vec2::new(w, -h),
            ]);
            return;
        }

        // Quatre coins, sens antihoraire depuis le coin haut-droit.
        let centres = [
            (Vec2::new(w - cr, h - cr), 0.0),
            (Vec2::new(-w + cr, h - cr), TAU * 0.25),
            (Vec2::new(-w + cr, -h + cr), TAU * 0.5),
            (Vec2::new(w - cr, -h + cr), TAU * 0.75),
        ];
        for (centre, base) in centres {
            for i in 0..=cs {
                let a = base + (i as f32 / cs as f32) * TAU * 0.25;
                out.push(centre + Vec2::new(a.cos() * cr, a.sin() * cr));
            }
        }
    }
}

/// Arc de cercle — ou ligne droite quand la courbure est nulle.
///
/// Contrairement aux autres primitives, un arc est une polyligne **ouverte**.
pub struct Arc {
    pub start: Param,
    /// Angle balayé, en **radians**.
    pub sweep: Param,
    pub radius: Param,
    pub segments: Param,
}

impl Arc {
    pub fn new(start: f32, sweep: f32, radius: f32) -> Self {
        Self {
            start: Param::fixed(start),
            sweep: Param::fixed(sweep),
            radius: Param::fixed(radius),
            segments: Param::fixed(SMOOTH_SEGMENTS as f32),
        }
    }

    pub fn with_sweep(mut self, s: Param) -> Self {
        self.sweep = s;
        self
    }
}

impl ShapeGenerator for Arc {
    fn generate(&self, _t: f64, _ctx: &Ctx, g: &Graph, out: &mut Vec<Vec2>) {
        out.clear();
        let start = self.start.get(g);
        let start = if start.is_finite() { start } else { 0.0 };
        let sweep = self.sweep.get(g);
        let sweep = if sweep.is_finite() { sweep } else { 0.0 };
        let r = self.radius.get(g).max(0.0);
        let n = resolution(self.segments.get(g), 2);

        // Polyligne ouverte : n+1 points pour n segments.
        for i in 0..=n {
            let a = start + sweep * (i as f32 / n as f32);
            out.push(Vec2::new(a.cos() * r, a.sin() * r));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generator::test_util::{ecart_max, gen, tous_finis};
    use frogenx_core::{Constant, Graph};

    const EPS: f32 = 1e-4;

    #[test]
    fn le_cercle_a_tous_ses_points_a_bonne_distance() {
        let g = Graph::new();
        let pts = gen(&Ellipse::circle(0.5), &g);
        assert_eq!(pts.len(), SMOOTH_SEGMENTS);
        for p in &pts {
            assert!((p.length() - 0.5).abs() < EPS, "rayon: got {}", p.length());
        }
    }

    #[test]
    fn le_cercle_est_centre_sur_lorigine() {
        let g = Graph::new();
        let pts = gen(&Ellipse::circle(1.0), &g);
        let centre: Vec2 = pts.iter().copied().sum::<Vec2>() / pts.len() as f32;
        assert!(centre.length() < EPS, "centre: got {centre:?}");
    }

    #[test]
    fn lellipse_ecrase_laxe_y() {
        let g = Graph::new();
        let e = Ellipse::circle(1.0).with_eccentricity(Param::fixed(0.5));
        let pts = gen(&e, &g);
        let max_x = pts.iter().map(|p| p.x.abs()).fold(0.0, f32::max);
        let max_y = pts.iter().map(|p| p.y.abs()).fold(0.0, f32::max);
        assert!((max_x - 1.0).abs() < EPS, "x: got {max_x}");
        assert!((max_y - 0.5).abs() < EPS, "y: got {max_y}");
    }

    #[test]
    fn le_contour_nest_pas_referme_par_duplication() {
        // Convention du §16.3 : le premier point n'est pas répété à la fin.
        let g = Graph::new();
        let pts = gen(&Ellipse::circle(1.0), &g);
        assert!(
            (pts[0] - pts[pts.len() - 1]).length() > EPS,
            "le premier point ne doit pas être dupliqué en fin"
        );
    }

    #[test]
    fn le_polygone_a_le_bon_nombre_de_sommets() {
        // Un triangle a 3 sommets à distance maximale du centre.
        //
        // On compte les **maxima locaux** du rayon, pas les points proches du
        // maximum : avec 128 échantillons, les sommets d'un triangle tombent
        // aux indices 0, 42.67 et 85.33 — un seul échantillon atterrit
        // exactement dessus, les deux autres l'encadrent.
        let g = Graph::new();
        let pts = gen(&Polygon::new(3.0, 1.0), &g);
        let rayons: Vec<f32> = pts.iter().map(|p| p.length()).collect();
        let n = rayons.len();
        let sommets = (0..n)
            .filter(|&i| {
                let prev = rayons[(i + n - 1) % n];
                let next = rayons[(i + 1) % n];
                rayons[i] >= prev && rayons[i] > next
            })
            .count();
        assert_eq!(sommets, 3, "un triangle a 3 sommets");
    }

    #[test]
    fn le_polygone_morphe_continument() {
        // LA propriété qui justifie le nombre de côtés flottant (§5.2).
        //
        // Ce qu'on teste est l'absence de **discontinuité**, pas l'absence de
        // mouvement : entre 3 et 3.01 côtés, les sommets se déplacent
        // légitimement (un sommet à 120° bouge de ~0.4°, soit ~0.03 unité sur
        // un contour de rayon 1). Un vrai saut — un sommet qui apparaît d'un
        // coup — produirait un déplacement proche du rayon entier.
        //
        // On vérifie donc que le déplacement reste **proportionnel au pas** :
        // en divisant le pas par 10, le saut doit diminuer d'autant.
        let g = Graph::new();
        let saut_pour = |pas: f32| -> f32 {
            let mut precedent: Option<Vec<Vec2>> = None;
            let mut saut_max = 0.0f32;
            for i in 0..=100 {
                let sides = 3.0 + i as f32 * pas;
                let pts = gen(&Polygon::new(sides, 1.0), &g);
                if let Some(prev) = &precedent {
                    for (a, b) in prev.iter().zip(pts.iter()) {
                        saut_max = saut_max.max((*b - *a).length());
                    }
                }
                precedent = Some(pts);
            }
            saut_max
        };

        let gros = saut_pour(0.01);
        let fin = saut_pour(0.001);
        assert!(gros < 0.1, "déplacement anormalement grand : {gros}");
        assert!(
            fin < gros * 0.3,
            "morphing discontinu : un pas 10× plus fin devrait donner un saut \
             bien plus petit, or {fin} vs {gros}"
        );
    }

    #[test]
    fn le_polygone_a_beaucoup_de_cotes_tend_vers_le_cercle() {
        let g = Graph::new();
        let pts = gen(&Polygon::new(64.0, 1.0), &g);
        for p in &pts {
            assert!(
                (p.length() - 1.0).abs() < 0.01,
                "à 64 côtés on doit approcher le cercle : got {}",
                p.length()
            );
        }
    }

    #[test]
    fn larrondi_transforme_le_polygone_en_cercle() {
        let g = Graph::new();
        let p = Polygon::new(3.0, 1.0).with_corner_radius(Param::fixed(1.0));
        let pts = gen(&p, &g);
        for pt in &pts {
            assert!(
                (pt.length() - 1.0).abs() < EPS,
                "arrondi maximal = cercle : got {}",
                pt.length()
            );
        }
    }

    #[test]
    fn la_rotation_fait_tourner_sans_deformer() {
        let g = Graph::new();
        let a = gen(&Polygon::new(4.0, 1.0), &g);
        let b = gen(
            &Polygon::new(4.0, 1.0).with_rotation(Param::fixed(TAU * 0.25)),
            &g,
        );
        // Même ensemble de rayons : la rotation ne déforme pas.
        let ra: f32 = a.iter().map(|p| p.length()).sum();
        let rb: f32 = b.iter().map(|p| p.length()).sum();
        assert!((ra - rb).abs() < EPS, "la rotation ne doit pas déformer");
    }

    #[test]
    fn letoile_alterne_pointes_et_creux() {
        let g = Graph::new();
        let pts = gen(&Star::new(5.0, 1.0, 0.4), &g);
        assert_eq!(pts.len(), 10, "5 branches = 10 sommets");
        for (i, p) in pts.iter().enumerate() {
            let attendu = if i % 2 == 0 { 1.0 } else { 0.4 };
            assert!(
                (p.length() - attendu).abs() < EPS,
                "sommet {i} : got {}, want {attendu}",
                p.length()
            );
        }
    }

    #[test]
    fn letoile_a_ratio_un_est_un_polygone() {
        let g = Graph::new();
        let pts = gen(&Star::new(6.0, 1.0, 1.0), &g);
        for p in &pts {
            assert!((p.length() - 1.0).abs() < EPS, "got {}", p.length());
        }
    }

    #[test]
    fn le_rectangle_sans_arrondi_a_quatre_coins() {
        let g = Graph::new();
        let pts = gen(&Rect::new(2.0, 1.0), &g);
        assert_eq!(pts.len(), 4);
        let max_x = pts.iter().map(|p| p.x.abs()).fold(0.0, f32::max);
        let max_y = pts.iter().map(|p| p.y.abs()).fold(0.0, f32::max);
        assert!((max_x - 1.0).abs() < EPS, "demi-largeur: got {max_x}");
        assert!((max_y - 0.5).abs() < EPS, "demi-hauteur: got {max_y}");
    }

    #[test]
    fn larrondi_du_rectangle_est_sature_au_petit_cote() {
        // Un rayon démesuré ne doit pas produire de coins qui se croisent.
        let g = Graph::new();
        let r = Rect::new(2.0, 1.0).with_corner_radius(Param::fixed(100.0));
        let pts = gen(&r, &g);
        assert!(tous_finis(&pts));
        let max_y = pts.iter().map(|p| p.y.abs()).fold(0.0, f32::max);
        assert!(max_y <= 0.5 + EPS, "débordement vertical : {max_y}");
    }

    #[test]
    fn larc_est_une_polyligne_ouverte() {
        let g = Graph::new();
        let pts = gen(&Arc::new(0.0, TAU * 0.5, 1.0), &g);
        // Demi-cercle : les extrémités sont diamétralement opposées.
        let d = (pts[0] - pts[pts.len() - 1]).length();
        assert!((d - 2.0).abs() < EPS, "diamètre: got {d}");
    }

    #[test]
    fn larc_a_balayage_nul_est_degenere_mais_fini() {
        let g = Graph::new();
        let pts = gen(&Arc::new(0.0, 0.0, 1.0), &g);
        assert!(tous_finis(&pts));
        assert!(!pts.is_empty());
    }

    #[test]
    fn lechantillonnage_du_cercle_est_regulier() {
        let g = Graph::new();
        let pts = gen(&Ellipse::circle(1.0), &g);
        let ecart = ecart_max(&pts);
        let attendu = TAU / SMOOTH_SEGMENTS as f32;
        assert!(
            (ecart - attendu).abs() < 0.01,
            "écart irrégulier : {ecart} vs {attendu}"
        );
    }

    #[test]
    fn les_parametres_sont_modulables() {
        // L'invariant central : un Param modulé pilote bien la géométrie.
        let mut g = Graph::new();
        let src = g.add(Box::new(Constant(0.5)));
        g.eval_frame(0.0, 1.0 / 60.0);

        let e = Ellipse::circle(0.0).with_radius(Param::modulated(0.0, 2.0, src));
        let pts = gen(&e, &g);
        // base 0 + depth 2 * 0.5 == 1.0
        for p in &pts {
            assert!((p.length() - 1.0).abs() < EPS, "got {}", p.length());
        }
    }

    #[test]
    fn les_valeurs_degenerees_ne_paniquent_pas() {
        // §18.1 : jamais de panique. Une modulation folle dégrade, elle ne tue pas.
        let g = Graph::new();
        let cas: Vec<Box<dyn ShapeGenerator>> = vec![
            Box::new(Ellipse::circle(-5.0)),
            Box::new(Ellipse::circle(f32::NAN)),
            Box::new(Polygon::new(f32::NAN, 1.0)),
            Box::new(Polygon::new(-10.0, 1.0)),
            Box::new(Polygon::new(1e9, 1.0)),
            Box::new(Star::new(f32::INFINITY, 1.0, 0.5)),
            Box::new(Star::new(5.0, 1.0, -3.0)),
            Box::new(Rect::new(-1.0, f32::NAN)),
            Box::new(Arc::new(f32::NAN, f32::NAN, 1.0)),
        ];
        for (i, forme) in cas.iter().enumerate() {
            let pts = gen(forme.as_ref(), &g);
            assert!(tous_finis(&pts), "cas {i} : point non fini");
            assert!(pts.len() <= MAX_POINTS, "cas {i} : {} points", pts.len());
        }
    }

    #[test]
    fn le_buffer_est_bien_vide_avant_remplissage() {
        // Contrat du §16.3 : generate() commence par out.clear().
        let g = Graph::new();
        let audio = frogenx_core::AudioFeatures::default();
        let ctx = Ctx::new(1.0 / 60.0, 0, &audio);
        let mut out = vec![Vec2::new(99.0, 99.0); 50];
        Ellipse::circle(1.0).generate(0.0, &ctx, &g, &mut out);
        assert_eq!(out.len(), SMOOTH_SEGMENTS);
        assert!(!out.contains(&Vec2::new(99.0, 99.0)), "buffer non nettoyé");
    }
}
