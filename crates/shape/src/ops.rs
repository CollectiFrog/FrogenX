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
//! Un opérateur travaille donc **en place** sur le buffer. Une chaîne de dix
//! opérateurs n'alloue rien.

use crate::generator::{ShapeGenerator, MAX_POINTS};
use frogenx_core::{Ctx, Graph, Param};
use frogenx_osc::value_noise_1d;
use glam::Vec2;
use std::f32::consts::TAU;

/// Transforme une polyligne en place.
pub trait ShapeOp {
    fn apply(&self, t: f64, ctx: &Ctx, g: &Graph, pts: &mut Vec<Vec2>);
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
    fn generate(&self, t: f64, ctx: &Ctx, g: &Graph, out: &mut Vec<Vec2>) {
        self.source.generate(t, ctx, g, out);
        for op in &self.ops {
            op.apply(t, ctx, g, out);
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

/// Normale approchée en un point d'une polyligne fermée.
///
/// Calculée depuis les voisins plutôt que depuis le centre : cela reste correct
/// pour une forme déjà déformée, là où `p.normalize()` supposerait un contour
/// encore centré sur l'origine.
fn normale(pts: &[Vec2], i: usize) -> Vec2 {
    let n = pts.len();
    let prev = pts[(i + n - 1) % n];
    let next = pts[(i + 1) % n];
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

impl ShapeOp for Deform {
    fn apply(&self, _t: f64, _ctx: &Ctx, g: &Graph, pts: &mut Vec<Vec2>) {
        if pts.len() < 3 {
            return;
        }
        let amount = self.amount.get(g);
        let amount = if amount.is_finite() { amount } else { 0.0 };
        let freq = self.frequency.get(g);
        let freq = if freq.is_finite() { freq } else { 1.0 };
        let phase = self.phase.get(g);
        let phase = if phase.is_finite() { phase } else { 0.0 };

        let n = pts.len();
        // Snapshot des normales : les calculer au fil de la déformation ferait
        // dépendre chaque point des précédents déjà déplacés.
        let normales: Vec<Vec2> = (0..n).map(|i| normale(pts, i)).collect();

        for i in 0..n {
            let u = i as f32 / n as f32;
            let onde = ((u * freq + phase) * TAU).sin();
            pts[i] += normales[i] * onde * amount;
        }
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
    fn apply(&self, _t: f64, _ctx: &Ctx, g: &Graph, pts: &mut Vec<Vec2>) {
        if pts.len() < 3 {
            return;
        }
        let amount = self.amount.get(g);
        let amount = if amount.is_finite() { amount } else { 0.0 };
        let scale = self.scale.get(g);
        let scale = if scale.is_finite() { scale.abs() } else { 1.0 };
        let offset = self.offset.get(g);
        let offset = if offset.is_finite() { offset } else { 0.0 };

        let n = pts.len();
        let normales: Vec<Vec2> = (0..n).map(|i| normale(pts, i)).collect();

        for i in 0..n {
            let u = i as f32 / n as f32;
            let v = value_noise_1d(u * scale + offset, self.seed);
            pts[i] += normales[i] * v * amount;
        }
    }
}

/// Répète la forme en rotation autour de l'origine.
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
    fn apply(&self, _t: f64, _ctx: &Ctx, g: &Graph, pts: &mut Vec<Vec2>) {
        let count = self.count.get(g);
        let count = if count.is_finite() {
            count.round().clamp(1.0, 64.0) as usize
        } else {
            1
        };
        if count <= 1 || pts.is_empty() {
            return;
        }
        let offset = self.offset.get(g);
        let offset = if offset.is_finite() { offset } else { 0.0 };

        // Borne dure avant d'allouer : une répétition sur une forme dense
        // pourrait sinon exploser.
        let total = pts.len().saturating_mul(count);
        if total > MAX_POINTS {
            return;
        }

        let original = pts.clone();
        for k in 1..count {
            let a = k as f32 / count as f32 * TAU + offset * k as f32;
            let (s, c) = a.sin_cos();
            for p in &original {
                pts.push(Vec2::new(p.x * c - p.y * s, p.x * s + p.y * c));
            }
        }
    }
}

/// Transformation affine simple : échelle, rotation, translation.
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
    fn apply(&self, _t: f64, _ctx: &Ctx, g: &Graph, pts: &mut Vec<Vec2>) {
        let s = self.scale.get(g);
        let s = if s.is_finite() { s } else { 1.0 };
        let r = self.rotation.get(g);
        let r = if r.is_finite() { r } else { 0.0 };
        let tx = self.translate_x.get(g);
        let tx = if tx.is_finite() { tx } else { 0.0 };
        let ty = self.translate_y.get(g);
        let ty = if ty.is_finite() { ty } else { 0.0 };

        let (sin, cos) = r.sin_cos();
        for p in pts.iter_mut() {
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
    fn apply(&self, _t: f64, _ctx: &Ctx, g: &Graph, pts: &mut Vec<Vec2>) {
        if pts.len() < 3 {
            return;
        }
        let passes = self.passes.get(g);
        let passes = if passes.is_finite() {
            passes.round().clamp(0.0, 16.0) as usize
        } else {
            0
        };
        let strength = self.strength.get(g).clamp(0.0, 1.0);

        let n = pts.len();
        let mut buf = pts.clone();
        for _ in 0..passes {
            for i in 0..n {
                let prev = pts[(i + n - 1) % n];
                let next = pts[(i + 1) % n];
                let moyenne = (prev + next) * 0.5;
                buf[i] = pts[i].lerp(moyenne, strength);
            }
            std::mem::swap(pts, &mut buf);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generator::test_util::{gen, tous_finis};
    use crate::primitives::{Ellipse, Polygon};
    use frogenx_core::{Constant, Graph};

    const EPS: f32 = 1e-4;

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
        let direct = gen(&Ellipse::circle(1.0), &g);
        let chaine = gen(&Chain::new(Box::new(Ellipse::circle(1.0))), &g);
        assert_eq!(direct.len(), chaine.len());
        for (a, b) in direct.iter().zip(chaine.iter()) {
            assert!((*a - *b).length() < EPS);
        }
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
        let avant = gen(&Ellipse::circle(1.0), &g);
        let apres = gen(
            &Chain::new(Box::new(Ellipse::circle(1.0))).then(Box::new(Deform::new(0.0, 5.0))),
            &g,
        );
        for (a, b) in avant.iter().zip(apres.iter()) {
            assert!((*a - *b).length() < EPS, "amplitude nulle = identité");
        }
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
    fn la_repetition_a_une_copie_ne_change_rien() {
        let g = Graph::new();
        let avant = gen(&Polygon::new(4.0, 1.0), &g);
        let apres = gen(
            &Chain::new(Box::new(Polygon::new(4.0, 1.0))).then(Box::new(RadialRepeat::new(1.0))),
            &g,
        );
        assert_eq!(avant.len(), apres.len());
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
        let avant = gen(&Ellipse::circle(1.0), &g);
        let apres = gen(
            &Chain::new(Box::new(Ellipse::circle(1.0))).then(Box::new(Transform::default())),
            &g,
        );
        for (a, b) in avant.iter().zip(apres.iter()) {
            assert!((*a - *b).length() < EPS);
        }
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
        let avant = gen(&Polygon::new(6.0, 1.0), &g);
        let apres = gen(
            &Chain::new(Box::new(Polygon::new(6.0, 1.0))).then(Box::new(Smooth::new(0.0, 1.0))),
            &g,
        );
        for (a, b) in avant.iter().zip(apres.iter()) {
            assert!((*a - *b).length() < EPS);
        }
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
        // Moins de 3 points : les opérateurs doivent passer sans rien casser.
        let g = Graph::new();
        let audio = frogenx_core::AudioFeatures::default();
        let ctx = Ctx::new(1.0 / 60.0, 0, &audio);
        for n in 0..3 {
            let mut pts = vec![Vec2::ZERO; n];
            Deform::new(0.5, 3.0).apply(0.0, &ctx, &g, &mut pts);
            NoiseDisplace::new(0.5, 3.0, 1).apply(0.0, &ctx, &g, &mut pts);
            Smooth::new(3.0, 1.0).apply(0.0, &ctx, &g, &mut pts);
            RadialRepeat::new(4.0).apply(0.0, &ctx, &g, &mut pts);
            assert!(tous_finis(&pts), "n={n}");
        }
    }
}
