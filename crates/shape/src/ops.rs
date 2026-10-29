//! Les opérateurs géométriques — §5.4.
//!
//! > **Où investir l'effort :** ajouter un opérateur multiplie les possibilités
//! > de toutes les primitives existantes. Ajouter une primitive n'ajoute
//! > qu'elle-même.
//!
//! ## Pourquoi un trait distinct de `ShapeGenerator`
//!
//! Un opérateur pourrait envelopper un générateur (`Deform::new(Circle)`), et
//! la composition s'écrirait naturellement. Mais chaque maillon de la chaîne
//! aurait alors besoin de son **propre buffer** pour recevoir la sortie du
//! précédent — exactement l'allocation par frame que le §16.3 cherche à éviter.
//!
//! Un opérateur travaille donc **en place** sur le `Path`. Une chaîne de dix
//! opérateurs n'alloue rien.
//!
//! ## Contours
//!
//! Un opérateur **ponctuel** (`Transform`) ne regarde qu'un point à la fois :
//! il boucle sur `points` sans connaître les contours. Un opérateur qui
//! regarde les **voisins** (`Deform`, `NoiseDisplace` via la normale, `Smooth`)
//! travaille contour par contour : un voisin ne traverse jamais la frontière
//! d'un contour, et un contour ouvert n'a pas de voisin au-delà de ses
//! extrémités (§16.3).

use crate::generator::{ShapeGenerator, MAX_POINTS};
use crate::path::Path;
use frogenx_core::{Ctx, Graph, Param};
use frogenx_osc::value_noise_1d;
use glam::Vec2;
use std::f32::consts::TAU;

/// Transforme une forme en place.
///
/// **Contrat (§16.3 bis) :** l'opérateur laisse le `Path` valide au sens de
/// [`Path::is_valid`]. Un opérateur ponctuel ne touche qu'à `points` ; un
/// opérateur qui ajoute ou retire des points met `contours` à jour.
pub trait ShapeOp {
    fn apply(&self, t: f64, ctx: &Ctx, g: &Graph, path: &mut Path);
}

/// Un générateur suivi d'une chaîne d'opérateurs.
///
/// C'est le composant qui rend le système réellement modulaire côté géométrie :
/// `Chain::new(Polygon).then(Deform).then(RadialRepeat)`.
pub struct Chain {
    source: Box<dyn ShapeGenerator>,
    ops: Vec<Box<dyn ShapeOp>>,
}

impl Chain {
    pub fn new(source: Box<dyn ShapeGenerator>) -> Self {
        Self {
            source,
            ops: Vec::new(),
        }
    }

    pub fn then(mut self, op: Box<dyn ShapeOp>) -> Self {
        self.ops.push(op);
        self
    }
}

impl ShapeGenerator for Chain {
    fn generate(&self, t: f64, ctx: &Ctx, g: &Graph, out: &mut Path) {
        self.source.generate(t, ctx, g, out);
        for op in &self.ops {
            op.apply(t, ctx, g, out);
        }
    }
}

/// Normale approchée au point `i` d'un contour.
///
/// Calculée depuis les voisins plutôt que depuis le centre : cela reste correct
/// pour une forme déjà déformée, là où `p.normalize()` supposerait un contour
/// encore centré sur l'origine. Sur un contour **ouvert**, les extrémités
/// n'ont qu'un voisin : la tangente y est unilatérale, jamais refermée sur
/// l'autre bout.
fn normale(pts: &[Vec2], i: usize, closed: bool) -> Vec2 {
    let n = pts.len();
    let (prev, next) = if closed {
        (pts[(i + n - 1) % n], pts[(i + 1) % n])
    } else {
        (pts[i.saturating_sub(1)], pts[(i + 1).min(n - 1)])
    };
    let tangente = next - prev;
    if tangente.length_squared() < 1e-12 {
        // Segment dégénéré : on retombe sur la direction radiale.
        return pts[i].normalize_or_zero();
    }
    // Perpendiculaire, orientée vers l'extérieur.
    let n = Vec2::new(tangente.y, -tangente.x).normalize_or_zero();
    if n.dot(pts[i]) < 0.0 {
        -n
    } else {
        n
    }
}

/// Abscisse du point `i` le long de son contour, dans `[0, 1]`.
///
/// Fermé : `i / n`, pour que la vague se raccorde au point de départ. Ouvert :
/// `i / (n - 1)`, pour qu'elle aille exactement d'une extrémité à l'autre.
fn abscisse(i: usize, n: usize, closed: bool) -> f32 {
    let d = if closed {
        n
    } else {
        n.saturating_sub(1).max(1)
    };
    i as f32 / d as f32
}

/// Applique `f(point, normale, abscisse)` à chaque point de chaque contour d'au
/// moins 3 points. Partagé par les opérateurs qui déplacent le long de la
/// normale ; `normales` est un buffer réutilisé d'un contour à l'autre.
fn deplacer_le_long_des_normales(path: &mut Path, f: impl Fn(f32) -> f32) {
    let Path { points, contours } = path;
    let mut normales: Vec<Vec2> = Vec::new();
    for c in contours.iter() {
        let pts = &mut points[c.range()];
        let n = pts.len();
        if n < 3 {
            continue;
        }
        // Snapshot des normales : les calculer au fil de la déformation ferait
        // dépendre chaque point des précédents déjà déplacés.
        normales.clear();
        normales.extend((0..n).map(|i| normale(pts, i, c.closed)));
        for i in 0..n {
            pts[i] += normales[i] * f(abscisse(i, n, c.closed));
        }
    }
}

/// Déforme le contour le long de sa normale, par une onde sinusoïdale.
///
/// C'est l'opérateur qui transforme un cercle en **blob organique** (§5.4). La
/// fréquence est en lobes par tour ; une fréquence non entière produit un
/// contour qui ne se referme pas sur lui-même, donc un aspect moins mécanique.
pub struct Deform {
    /// Amplitude de la déformation, en unités d'espace.
    pub amount: Param,
    /// Nombre de lobes sur le tour complet.
    pub frequency: Param,
    /// Décalage de phase, en tours. Modulable → le blob ondule.
    pub phase: Param,
}

impl Deform {
    pub fn new(amount: f32, frequency: f32) -> Self {
        Self {
            amount: Param::fixed(amount),
            frequency: Param::fixed(frequency),
            phase: Param::fixed(0.0),
        }
    }

    pub fn with_phase(mut self, p: Param) -> Self {
        self.phase = p;
        self
    }
}

impl ShapeOp for Deform {
    fn apply(&self, _t: f64, _ctx: &Ctx, g: &Graph, path: &mut Path) {
        let amount = self.amount.get(g);
        let amount = if amount.is_finite() { amount } else { 0.0 };
        let freq = self.frequency.get(g);
        let freq = if freq.is_finite() { freq } else { 1.0 };
        let phase = self.phase.get(g);
        let phase = if phase.is_finite() { phase } else { 0.0 };

        deplacer_le_long_des_normales(path, |u| ((u * freq + phase) * TAU).sin() * amount);
    }
}

/// Ajoute du bruit cohérent sur chaque sommet, le long de la normale.
///
/// Là où [`Deform`] reste périodique et régulier, le bruit produit de
/// l'irrégularité — c'est ce qui distingue le vivant du mécanique.
pub struct NoiseDisplace {
    pub amount: Param,
    /// Échelle spatiale : combien d'unités de bruit sur un tour de contour.
    pub scale: Param,
    /// Décalage dans l'espace du bruit. Modulable → l'irrégularité se déplace.
    pub offset: Param,
    /// Graine explicite (§20.3), jamais tirée au hasard.
    pub seed: u32,
}

impl NoiseDisplace {
    pub fn new(amount: f32, scale: f32, seed: u32) -> Self {
        Self {
            amount: Param::fixed(amount),
            scale: Param::fixed(scale),
            offset: Param::fixed(0.0),
            seed,
        }
    }

    pub fn with_offset(mut self, o: Param) -> Self {
        self.offset = o;
        self
    }
}

impl ShapeOp for NoiseDisplace {
    fn apply(&self, _t: f64, _ctx: &Ctx, g: &Graph, path: &mut Path) {
        let amount = self.amount.get(g);
        let amount = if amount.is_finite() { amount } else { 0.0 };
        let scale = self.scale.get(g);
        let scale = if scale.is_finite() { scale.abs() } else { 1.0 };
        let offset = self.offset.get(g);
        let offset = if offset.is_finite() { offset } else { 0.0 };

        let seed = self.seed;
        deplacer_le_long_des_normales(path, |u| value_noise_1d(u * scale + offset, seed) * amount);
    }
}

/// Répète la forme en rotation autour de l'origine.
///
/// Chaque copie de chaque contour devient **son propre contour**, avec le même
/// drapeau fermé/ouvert que l'original — jamais collée à la suite de la
/// précédente, ce qui ferait tracer au rasteriseur un segment d'une copie à
/// l'autre (§16.3).
///
/// Attention : cela **multiplie** le nombre de points. La borne `MAX_POINTS`
/// protège d'une modulation dégénérée.
pub struct RadialRepeat {
    /// Nombre de copies. Saturé dans `[1, 64]`.
    pub count: Param,
    /// Décalage angulaire supplémentaire par copie, en radians.
    pub offset: Param,
}

impl RadialRepeat {
    pub fn new(count: f32) -> Self {
        Self {
            count: Param::fixed(count),
            offset: Param::fixed(0.0),
        }
    }
}

impl ShapeOp for RadialRepeat {
    fn apply(&self, _t: f64, _ctx: &Ctx, g: &Graph, path: &mut Path) {
        let count = self.count.get(g);
        let count = if count.is_finite() {
            count.round().clamp(1.0, 64.0) as usize
        } else {
            1
        };
        if count <= 1 || path.points.is_empty() {
            return;
        }
        let offset = self.offset.get(g);
        let offset = if offset.is_finite() { offset } else { 0.0 };

        // Borne dure avant d'allouer : une répétition sur une forme dense
        // pourrait sinon exploser.
        let total = path.points.len().saturating_mul(count);
        if total > MAX_POINTS {
            return;
        }
        path.points.reserve(total - path.points.len());

        // Les contours d'origine seuls sont répétés, pas les copies ajoutées
        // au fil de la boucle.
        let originaux = path.contours.len();
        for k in 1..count {
            let a = k as f32 / count as f32 * TAU + offset * k as f32;
            let (s, c) = a.sin_cos();
            for ci in 0..originaux {
                let contour = path.contours[ci];
                for i in contour.range() {
                    let p = path.points[i];
                    path.points
                        .push(Vec2::new(p.x * c - p.y * s, p.x * s + p.y * c));
                }
                path.end_contour(contour.closed);
            }
        }
    }
}

/// Transformation affine simple : échelle, rotation, translation.
///
/// Opérateur ponctuel : il ne connaît pas les contours.
pub struct Transform {
    pub scale: Param,
    pub rotation: Param,
    pub translate_x: Param,
    pub translate_y: Param,
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            scale: Param::fixed(1.0),
            rotation: Param::fixed(0.0),
            translate_x: Param::fixed(0.0),
            translate_y: Param::fixed(0.0),
        }
    }
}

impl ShapeOp for Transform {
    fn apply(&self, _t: f64, _ctx: &Ctx, g: &Graph, path: &mut Path) {
        let s = self.scale.get(g);
        let s = if s.is_finite() { s } else { 1.0 };
        let r = self.rotation.get(g);
        let r = if r.is_finite() { r } else { 0.0 };
        let tx = self.translate_x.get(g);
        let tx = if tx.is_finite() { tx } else { 0.0 };
        let ty = self.translate_y.get(g);
        let ty = if ty.is_finite() { ty } else { 0.0 };

        let (sin, cos) = r.sin_cos();
        for p in path.points.iter_mut() {
            let x = p.x * s;
            let y = p.y * s;
            *p = Vec2::new(x * cos - y * sin + tx, x * sin + y * cos + ty);
        }
    }
}

/// Lisse le contour par moyenne glissante sur les voisins.
///
/// Adoucit ce que [`NoiseDisplace`] a rendu trop rugueux. Chaque passe réduit
/// le détail ; deux ou trois suffisent en général.
///
/// Contour par contour : un contour fermé boucle sur lui-même ; un contour
/// ouvert garde ses **extrémités fixes** — elles n'ont qu'un voisin, et les
/// moyenner avec l'autre bout rapprocherait les deux extrémités d'un arc.
pub struct Smooth {
    /// Nombre de passes. Saturé dans `[0, 16]`.
    pub passes: Param,
    /// Force du lissage, `[0, 1]`.
    pub strength: Param,
}

impl Smooth {
    pub fn new(passes: f32, strength: f32) -> Self {
        Self {
            passes: Param::fixed(passes),
            strength: Param::fixed(strength),
        }
    }
}

impl ShapeOp for Smooth {
    fn apply(&self, _t: f64, _ctx: &Ctx, g: &Graph, path: &mut Path) {
        let passes = self.passes.get(g);
        let passes = if passes.is_finite() {
            passes.round().clamp(0.0, 16.0) as usize
        } else {
            0
        };
        let strength = self.strength.get(g);
        // `clamp` laisse passer un NaN : on le neutralise avant.
        let strength = if strength.is_finite() {
            strength.clamp(0.0, 1.0)
        } else {
            0.0
        };

        let Path { points, contours } = path;
        let mut avant: Vec<Vec2> = Vec::new();
        for c in contours.iter() {
            let pts = &mut points[c.range()];
            let n = pts.len();
            if n < 3 {
                continue;
            }
            for _ in 0..passes {
                // Chaque passe lit l'état de la passe précédente, pas les
                // points déjà lissés de la passe en cours.
                avant.clear();
                avant.extend_from_slice(pts);
                for i in 0..n {
                    if !c.closed && (i == 0 || i == n - 1) {
                        continue;
                    }
                    let prev = avant[(i + n - 1) % n];
                    let next = avant[(i + 1) % n];
                    pts[i] = avant[i].lerp((prev + next) * 0.5, strength);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generator::test_util::{gen, gen_path, tous_finis};
    use crate::primitives::{Arc, Ellipse, Polygon};
    use frogenx_core::{Constant, Graph};

    const EPS: f32 = 1e-4;

    /// Vrai si deux formes ont les mêmes points, à `EPS` près, et les mêmes
    /// contours.
    fn memes_formes(a: &Path, b: &Path) -> bool {
        a.contours == b.contours
            && a.points.len() == b.points.len()
            && a.points
                .iter()
                .zip(b.points.iter())
                .all(|(p, q)| (*p - *q).length() < EPS)
    }

    #[test]
    fn la_chaine_applique_les_operateurs_dans_lordre() {
        let g = Graph::new();
        // Échelle 2 puis translation 1 → un point à x=1 finit à x=3.
        let chaine = Chain::new(Box::new(Ellipse::circle(1.0)))
            .then(Box::new(Transform {
                scale: Param::fixed(2.0),
                ..Default::default()
            }))
            .then(Box::new(Transform {
                translate_x: Param::fixed(1.0),
                ..Default::default()
            }));
        let pts = gen(&chaine, &g);
        let max_x = pts.iter().map(|p| p.x).fold(f32::MIN, f32::max);
        assert!((max_x - 3.0).abs() < EPS, "got {max_x}");
    }

    #[test]
    fn la_chaine_vide_ne_change_rien() {
        let g = Graph::new();
        let direct = gen_path(&Ellipse::circle(1.0), &g);
        let chaine = gen_path(&Chain::new(Box::new(Ellipse::circle(1.0))), &g);
        assert!(memes_formes(&direct, &chaine));
    }

    #[test]
    fn deform_preserve_le_nombre_de_points() {
        let g = Graph::new();
        let chaine =
            Chain::new(Box::new(Ellipse::circle(1.0))).then(Box::new(Deform::new(0.2, 5.0)));
        let pts = gen(&chaine, &g);
        assert_eq!(pts.len(), crate::generator::SMOOTH_SEGMENTS);
        assert!(tous_finis(&pts));
    }

    #[test]
    fn deform_fait_varier_le_rayon() {
        let g = Graph::new();
        let chaine =
            Chain::new(Box::new(Ellipse::circle(1.0))).then(Box::new(Deform::new(0.3, 5.0)));
        let pts = gen(&chaine, &g);
        let rmin = pts.iter().map(|p| p.length()).fold(f32::MAX, f32::min);
        let rmax = pts.iter().map(|p| p.length()).fold(0.0, f32::max);
        assert!(
            rmax - rmin > 0.3,
            "le contour doit onduler : {rmin}..{rmax}"
        );
    }

    #[test]
    fn deform_damplitude_nulle_ne_change_rien() {
        let g = Graph::new();
        let avant = gen_path(&Ellipse::circle(1.0), &g);
        let apres = gen_path(
            &Chain::new(Box::new(Ellipse::circle(1.0))).then(Box::new(Deform::new(0.0, 5.0))),
            &g,
        );
        assert!(memes_formes(&avant, &apres), "amplitude nulle = identité");
    }

    #[test]
    fn deform_commute_avec_la_repetition_radiale() {
        // Déformer puis répéter, ou répéter puis déformer : même résultat, si
        // la déformation travaille contour par contour. Avant §16.3, la
        // déformation courait le long de toutes les copies collées bout à
        // bout — chaque copie recevait une portion différente de l'onde.
        let g = Graph::new();
        let deformer_puis_repeter = Chain::new(Box::new(Polygon::new(5.0, 0.5)))
            .then(Box::new(Deform::new(0.1, 3.0)))
            .then(Box::new(RadialRepeat::new(6.0)));
        let repeter_puis_deformer = Chain::new(Box::new(Polygon::new(5.0, 0.5)))
            .then(Box::new(RadialRepeat::new(6.0)))
            .then(Box::new(Deform::new(0.1, 3.0)));
        assert!(memes_formes(
            &gen_path(&deformer_puis_repeter, &g),
            &gen_path(&repeter_puis_deformer, &g)
        ));
    }

    #[test]
    fn le_bruit_rend_le_contour_irregulier() {
        let g = Graph::new();
        let chaine = Chain::new(Box::new(Ellipse::circle(1.0)))
            .then(Box::new(NoiseDisplace::new(0.2, 4.0, 42)));
        let pts = gen(&chaine, &g);

        // Le bruit ne doit pas être périodique comme Deform : on vérifie que
        // les rayons ne se répètent pas à intervalle régulier.
        let rayons: Vec<f32> = pts.iter().map(|p| p.length()).collect();
        let moitie = rayons.len() / 2;
        let ecart: f32 = (0..moitie)
            .map(|i| (rayons[i] - rayons[i + moitie]).abs())
            .sum();
        assert!(ecart > 0.1, "le bruit doit décorréler les deux moitiés");
    }

    #[test]
    fn le_bruit_est_deterministe_a_graine_egale() {
        let g = Graph::new();
        let faire = || {
            gen(
                &Chain::new(Box::new(Ellipse::circle(1.0)))
                    .then(Box::new(NoiseDisplace::new(0.2, 4.0, 7))),
                &g,
            )
        };
        assert_eq!(faire(), faire(), "§20.3 : reproductible à l'identique");
    }

    #[test]
    fn deux_graines_donnent_des_contours_differents() {
        let g = Graph::new();
        let a = gen(
            &Chain::new(Box::new(Ellipse::circle(1.0)))
                .then(Box::new(NoiseDisplace::new(0.2, 4.0, 1))),
            &g,
        );
        let b = gen(
            &Chain::new(Box::new(Ellipse::circle(1.0)))
                .then(Box::new(NoiseDisplace::new(0.2, 4.0, 2))),
            &g,
        );
        assert_ne!(a, b);
    }

    #[test]
    fn la_repetition_radiale_multiplie_les_points() {
        let g = Graph::new();
        let base = gen(&Polygon::new(3.0, 0.3), &g);
        let repete = gen(
            &Chain::new(Box::new(Polygon::new(3.0, 0.3))).then(Box::new(RadialRepeat::new(6.0))),
            &g,
        );
        assert_eq!(repete.len(), base.len() * 6);
        assert!(tous_finis(&repete));
    }

    #[test]
    fn la_repetition_radiale_produit_un_contour_par_copie() {
        // Le défaut corrigé au §16.3 : les copies étaient collées bout à bout
        // dans une polyligne unique, et le rasteriseur aurait relié chaque
        // copie à la suivante.
        let g = Graph::new();
        let base = gen_path(&Polygon::new(3.0, 0.3), &g);
        let repete = gen_path(
            &Chain::new(Box::new(Polygon::new(3.0, 0.3))).then(Box::new(RadialRepeat::new(6.0))),
            &g,
        );

        assert_eq!(repete.contours.len(), 6, "une copie = un contour");
        let n = base.points.len();
        for (k, c) in repete.contours.iter().enumerate() {
            assert_eq!(c.len(), n, "copie {k} : même nombre de points");
            assert!(c.closed, "copie {k} : fermée comme l'original");

            // Chaque copie est l'original tourné de k/6 de tour.
            let a = k as f32 / 6.0 * TAU;
            let (s, co) = a.sin_cos();
            for (p, q) in base.points.iter().zip(repete.contour_points(c)) {
                let attendu = Vec2::new(p.x * co - p.y * s, p.x * s + p.y * co);
                assert!((attendu - *q).length() < EPS, "copie {k}");
            }
        }
    }

    #[test]
    fn la_repetition_conserve_le_drapeau_ouvert() {
        let g = Graph::new();
        let path = gen_path(
            &Chain::new(Box::new(Arc::new(0.0, 1.0, 0.5))).then(Box::new(RadialRepeat::new(4.0))),
            &g,
        );
        assert_eq!(path.contours.len(), 4);
        assert!(path.contours.iter().all(|c| !c.closed));
    }

    #[test]
    fn la_repetition_repete_chaque_contour_dune_forme_deja_multiple() {
        // Deux répétitions successives : 3 × 4 = 12 contours, pas 3 + 4.
        let g = Graph::new();
        let path = gen_path(
            &Chain::new(Box::new(Polygon::new(3.0, 0.2)))
                .then(Box::new(RadialRepeat::new(3.0)))
                .then(Box::new(RadialRepeat::new(4.0))),
            &g,
        );
        assert_eq!(path.contours.len(), 12);
    }

    #[test]
    fn la_repetition_a_une_copie_ne_change_rien() {
        let g = Graph::new();
        let avant = gen_path(&Polygon::new(4.0, 1.0), &g);
        let apres = gen_path(
            &Chain::new(Box::new(Polygon::new(4.0, 1.0))).then(Box::new(RadialRepeat::new(1.0))),
            &g,
        );
        assert!(memes_formes(&avant, &apres));
    }

    #[test]
    fn la_repetition_preserve_les_rayons() {
        // Une rotation ne change pas la distance à l'origine.
        let g = Graph::new();
        let pts = gen(
            &Chain::new(Box::new(Ellipse::circle(0.5))).then(Box::new(RadialRepeat::new(4.0))),
            &g,
        );
        for p in &pts {
            assert!((p.length() - 0.5).abs() < EPS, "got {}", p.length());
        }
    }

    #[test]
    fn la_repetition_respecte_la_borne_dure() {
        // §18.4 : une modulation dégénérée dégrade, elle ne fait pas exploser.
        let g = Graph::new();
        let dense = Ellipse {
            radius: Param::fixed(1.0),
            eccentricity: Param::fixed(1.0),
            segments: Param::fixed(2000.0),
        };
        let pts = gen(
            &Chain::new(Box::new(dense)).then(Box::new(RadialRepeat::new(64.0))),
            &g,
        );
        assert!(pts.len() <= MAX_POINTS, "got {} points", pts.len());
    }

    #[test]
    fn transform_par_defaut_est_lidentite() {
        let g = Graph::new();
        let avant = gen_path(&Ellipse::circle(1.0), &g);
        let apres = gen_path(
            &Chain::new(Box::new(Ellipse::circle(1.0))).then(Box::new(Transform::default())),
            &g,
        );
        assert!(memes_formes(&avant, &apres));
    }

    #[test]
    fn la_rotation_preserve_les_longueurs() {
        let g = Graph::new();
        let pts = gen(
            &Chain::new(Box::new(Polygon::new(5.0, 1.0))).then(Box::new(Transform {
                rotation: Param::fixed(0.7),
                ..Default::default()
            })),
            &g,
        );
        let rmax = pts.iter().map(|p| p.length()).fold(0.0, f32::max);
        assert!((rmax - 1.0).abs() < EPS, "got {rmax}");
    }

    #[test]
    fn le_lissage_reduit_les_asperites() {
        let g = Graph::new();
        let rugueux = Chain::new(Box::new(Ellipse::circle(1.0)))
            .then(Box::new(NoiseDisplace::new(0.3, 20.0, 5)));
        let lisse = Chain::new(Box::new(Ellipse::circle(1.0)))
            .then(Box::new(NoiseDisplace::new(0.3, 20.0, 5)))
            .then(Box::new(Smooth::new(4.0, 1.0)));

        let variation = |pts: &[Vec2]| -> f32 {
            let n = pts.len();
            (0..n)
                .map(|i| (pts[(i + 1) % n] - pts[i]).length())
                .sum::<f32>()
        };
        let a = variation(&gen(&rugueux, &g));
        let b = variation(&gen(&lisse, &g));
        assert!(b < a, "le lissage doit réduire le périmètre : {a} → {b}");
    }

    #[test]
    fn le_lissage_a_zero_passe_ne_change_rien() {
        let g = Graph::new();
        let avant = gen_path(&Polygon::new(6.0, 1.0), &g);
        let apres = gen_path(
            &Chain::new(Box::new(Polygon::new(6.0, 1.0))).then(Box::new(Smooth::new(0.0, 1.0))),
            &g,
        );
        assert!(memes_formes(&avant, &apres));
    }

    #[test]
    fn le_lissage_ne_touche_pas_les_extremites_dun_contour_ouvert() {
        // Le défaut corrigé au §16.3 : `Smooth` bouclait par modulo et
        // moyennait la première extrémité d'un arc avec la dernière.
        let g = Graph::new();
        let arc = gen_path(&Arc::new(0.0, TAU * 0.5, 1.0), &g);
        let lisse = gen_path(
            &Chain::new(Box::new(Arc::new(0.0, TAU * 0.5, 1.0)))
                .then(Box::new(Smooth::new(8.0, 1.0))),
            &g,
        );
        let (premier, dernier) = (arc.points[0], *arc.points.last().unwrap());
        assert!((lisse.points[0] - premier).length() < EPS);
        assert!((*lisse.points.last().unwrap() - dernier).length() < EPS);
    }

    #[test]
    fn le_lissage_commute_avec_la_repetition_radiale() {
        // Lisser puis répéter, ou répéter puis lisser : même résultat, parce
        // que le lissage ne déborde plus d'une copie sur la suivante.
        let g = Graph::new();
        let lisser_puis_repeter = Chain::new(Box::new(Polygon::new(5.0, 0.5)))
            .then(Box::new(Smooth::new(3.0, 0.8)))
            .then(Box::new(RadialRepeat::new(6.0)));
        let repeter_puis_lisser = Chain::new(Box::new(Polygon::new(5.0, 0.5)))
            .then(Box::new(RadialRepeat::new(6.0)))
            .then(Box::new(Smooth::new(3.0, 0.8)));
        assert!(memes_formes(
            &gen_path(&lisser_puis_repeter, &g),
            &gen_path(&repeter_puis_lisser, &g)
        ));
    }

    #[test]
    fn les_operateurs_sont_modulables() {
        // L'invariant : un Signal pilote la géométrie via un Param.
        let mut g = Graph::new();
        let src = g.add(Box::new(Constant(1.0)));
        g.eval_frame(0.0, 1.0 / 60.0);

        let chaine = Chain::new(Box::new(Ellipse::circle(1.0))).then(Box::new(Transform {
            scale: Param::modulated(0.0, 2.0, src),
            ..Default::default()
        }));
        let pts = gen(&chaine, &g);
        for p in &pts {
            assert!((p.length() - 2.0).abs() < EPS, "got {}", p.length());
        }
    }

    #[test]
    fn les_operateurs_ne_paniquent_pas_sur_valeurs_degenerees() {
        let g = Graph::new();
        let cas: Vec<Box<dyn ShapeOp>> = vec![
            Box::new(Deform::new(f32::NAN, f32::NAN)),
            Box::new(NoiseDisplace::new(f32::INFINITY, -5.0, 1)),
            Box::new(RadialRepeat::new(f32::NAN)),
            Box::new(RadialRepeat::new(-10.0)),
            Box::new(Smooth::new(f32::NAN, 50.0)),
            Box::new(Smooth::new(3.0, f32::NAN)),
            Box::new(Transform {
                scale: Param::fixed(f32::NAN),
                rotation: Param::fixed(f32::INFINITY),
                ..Default::default()
            }),
        ];
        for (i, op) in cas.into_iter().enumerate() {
            let chaine = Chain::new(Box::new(Ellipse::circle(1.0))).then(op);
            let pts = gen(&chaine, &g);
            assert!(tous_finis(&pts), "cas {i} : point non fini");
            assert!(pts.len() <= MAX_POINTS, "cas {i}");
        }
    }

    #[test]
    fn les_operateurs_tolerent_une_forme_degeneree() {
        // Moins de 3 points, ou aucun contour : les opérateurs doivent passer
        // sans rien casser et laisser un Path valide.
        let g = Graph::new();
        let audio = frogenx_core::AudioFeatures::default();
        let ctx = Ctx::new(1.0 / 60.0, 0, &audio);
        for n in 0..3 {
            for closed in [true, false] {
                let mut path = Path::new();
                path.points.extend(std::iter::repeat_n(Vec2::ZERO, n));
                path.end_contour(closed);
                Deform::new(0.5, 3.0).apply(0.0, &ctx, &g, &mut path);
                NoiseDisplace::new(0.5, 3.0, 1).apply(0.0, &ctx, &g, &mut path);
                Smooth::new(3.0, 1.0).apply(0.0, &ctx, &g, &mut path);
                RadialRepeat::new(4.0).apply(0.0, &ctx, &g, &mut path);
                assert!(tous_finis(&path.points), "n={n}");
                assert!(path.is_valid(), "n={n} closed={closed}");
            }
        }
    }
}
