//! Générateurs paramétriques — §5.3.
//!
//! Ceux qui viennent directement du monde des oscillateurs : deux signaux
//! pilotent `(x, y)`, ou un signal pilote le rayon en fonction de l'angle.

use crate::generator::{ShapeGenerator, MAX_POINTS, SMOOTH_SEGMENTS};
use frogenx_core::{Ctx, Graph, Param};
use glam::Vec2;
use std::f32::consts::TAU;

fn resolution(n: f32, min: usize) -> usize {
    if !n.is_finite() {
        return min;
    }
    (n.round().max(min as f32) as usize).min(MAX_POINTS)
}

/// Courbe de Lissajous — deux oscillations perpendiculaires.
///
/// `x = sin(a·θ + δ)`, `y = sin(b·θ)`. Des rapports **entiers** donnent des
/// figures fermées et géométriques ; des rapports **non entiers** donnent des
/// courbes qui ne se referment pas, d'aspect organique (§5.3).
pub struct Lissajous {
    pub freq_x: Param,
    pub freq_y: Param,
    /// Déphasage entre les deux axes, en tours.
    pub phase: Param,
    pub amplitude: Param,
    pub segments: Param,
}

impl Lissajous {
    pub fn new(freq_x: f32, freq_y: f32) -> Self {
        Self {
            freq_x: Param::fixed(freq_x),
            freq_y: Param::fixed(freq_y),
            phase: Param::fixed(0.25),
            amplitude: Param::fixed(1.0),
            segments: Param::fixed(512.0),
        }
    }

    pub fn with_phase(mut self, p: Param) -> Self {
        self.phase = p;
        self
    }
}

impl ShapeGenerator for Lissajous {
    fn generate(&self, _t: f64, _ctx: &Ctx, g: &Graph, out: &mut Vec<Vec2>) {
        out.clear();
        let fx = self.freq_x.get(g);
        let fx = if fx.is_finite() { fx } else { 1.0 };
        let fy = self.freq_y.get(g);
        let fy = if fy.is_finite() { fy } else { 1.0 };
        let phase = self.phase.get(g);
        let phase = if phase.is_finite() { phase } else { 0.0 };
        let amp = self.amplitude.get(g);
        let amp = if amp.is_finite() { amp } else { 1.0 };
        let n = resolution(self.segments.get(g), 3);

        for i in 0..n {
            let th = i as f32 / n as f32 * TAU;
            out.push(Vec2::new(
                ((th * fx) + phase * TAU).sin() * amp,
                (th * fy).sin() * amp,
            ));
        }
    }
}

/// Rose polaire — `r = cos(k·θ)`.
///
/// Le nombre de pétales dépend de `k` : impair donne `k` pétales, pair en donne
/// `2k`. Un `k` non entier produit des figures ouvertes et irrégulières.
pub struct Rose {
    /// Paramètre `k` de la rose. Modulable → les pétales se transforment.
    pub k: Param,
    pub radius: Param,
    pub segments: Param,
}

impl Rose {
    pub fn new(k: f32, radius: f32) -> Self {
        Self {
            k: Param::fixed(k),
            radius: Param::fixed(radius),
            segments: Param::fixed(512.0),
        }
    }

    pub fn with_k(mut self, k: Param) -> Self {
        self.k = k;
        self
    }
}

impl ShapeGenerator for Rose {
    fn generate(&self, _t: f64, _ctx: &Ctx, g: &Graph, out: &mut Vec<Vec2>) {
        out.clear();
        let k = self.k.get(g);
        let k = if k.is_finite() {
            k.clamp(-32.0, 32.0)
        } else {
            3.0
        };
        let r = self.radius.get(g).max(0.0);
        let n = resolution(self.segments.get(g), 3);

        for i in 0..n {
            let th = i as f32 / n as f32 * TAU;
            // Le rayon peut être négatif : c'est ce qui dessine les pétales
            // opposés, et c'est voulu.
            let rad = (k * th).cos() * r;
            out.push(Vec2::new(th.cos() * rad, th.sin() * rad));
        }
    }
}

/// Superformule de Gielis — une équation, une grande part du règne végétal et
/// minéral (§5.3).
///
/// `r(θ) = (|cos(mθ/4)/a|^n2 + |sin(mθ/4)/b|^n3)^(-1/n1)`
///
/// Les six paramètres sont modulables, ce qui en fait le générateur le plus
/// expressif du système — et le plus difficile à régler.
pub struct Superformula {
    /// Symétrie. Entier → figure régulière à `m` lobes.
    pub m: Param,
    pub n1: Param,
    pub n2: Param,
    pub n3: Param,
    pub a: Param,
    pub b: Param,
    pub radius: Param,
    pub segments: Param,
}

impl Superformula {
    /// Réglage de départ : une forme arrondie à 5 lobes.
    pub fn new(m: f32) -> Self {
        Self {
            m: Param::fixed(m),
            n1: Param::fixed(1.0),
            n2: Param::fixed(1.0),
            n3: Param::fixed(1.0),
            a: Param::fixed(1.0),
            b: Param::fixed(1.0),
            radius: Param::fixed(1.0),
            segments: Param::fixed(SMOOTH_SEGMENTS as f32 * 2.0),
        }
    }

    pub fn with_m(mut self, m: Param) -> Self {
        self.m = m;
        self
    }

    pub fn with_n(mut self, n1: Param, n2: Param, n3: Param) -> Self {
        self.n1 = n1;
        self.n2 = n2;
        self.n3 = n3;
        self
    }
}

impl ShapeGenerator for Superformula {
    fn generate(&self, _t: f64, _ctx: &Ctx, g: &Graph, out: &mut Vec<Vec2>) {
        out.clear();
        let fini = |v: f32, defaut: f32| if v.is_finite() { v } else { defaut };
        let m = fini(self.m.get(g), 5.0).clamp(-64.0, 64.0);
        // n1 nul ferait diverger l'exposant -1/n1.
        let n1 = fini(self.n1.get(g), 1.0);
        let n1 = if n1.abs() < 1e-3 { 1e-3 } else { n1 };
        let n2 = fini(self.n2.get(g), 1.0);
        let n3 = fini(self.n3.get(g), 1.0);
        // a ou b nul diviserait par zéro.
        let a = fini(self.a.get(g), 1.0);
        let a = if a.abs() < 1e-3 { 1e-3 } else { a };
        let b = fini(self.b.get(g), 1.0);
        let b = if b.abs() < 1e-3 { 1e-3 } else { b };
        let radius = self.radius.get(g).max(0.0);
        let n = resolution(self.segments.get(g), 3);

        for i in 0..n {
            let th = i as f32 / n as f32 * TAU;
            let t1 = ((m * th / 4.0).cos() / a).abs().powf(n2);
            let t2 = ((m * th / 4.0).sin() / b).abs().powf(n3);
            let somme = t1 + t2;
            // La somme peut tomber à zéro ou déborder : on sature plutôt que
            // de laisser sortir un inf (§18.3, le NaN ne doit pas se propager).
            let r = if somme > 1e-6 {
                somme.powf(-1.0 / n1).clamp(0.0, 16.0)
            } else {
                0.0
            };
            let r = if r.is_finite() { r * radius } else { 0.0 };
            out.push(Vec2::new(th.cos() * r, th.sin() * r));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generator::test_util::{gen, tous_finis};
    use frogenx_core::{Constant, Graph};

    const EPS: f32 = 1e-3;

    #[test]
    fn lissajous_un_pour_un_donne_un_cercle() {
        // fx=fy=1 avec un quart de tour de déphasage : c'est un cercle.
        let g = Graph::new();
        let pts = gen(&Lissajous::new(1.0, 1.0), &g);
        for p in &pts {
            assert!((p.length() - 1.0).abs() < EPS, "got {}", p.length());
        }
    }

    #[test]
    fn lissajous_reste_dans_lamplitude() {
        let g = Graph::new();
        let pts = gen(&Lissajous::new(3.0, 2.0), &g);
        for p in &pts {
            assert!(p.x.abs() <= 1.0 + EPS && p.y.abs() <= 1.0 + EPS, "{p:?}");
        }
    }

    #[test]
    fn lissajous_a_rapport_entier_se_referme() {
        // La propriété qui distingue les rapports entiers (§5.3) : la courbe
        // revient à son point de départ.
        let g = Graph::new();
        let pts = gen(&Lissajous::new(3.0, 2.0), &g);
        let boucle = (pts[0] - pts[pts.len() - 1]).length();
        let pas = (pts[1] - pts[0]).length();
        assert!(
            boucle < pas * 2.0,
            "rapport entier : la courbe doit se refermer ({boucle} vs pas {pas})"
        );
    }

    #[test]
    fn la_rose_a_le_bon_nombre_de_petales() {
        // `r = cos(kθ)` balayé sur un tour complet passe 2k fois par un
        // extrême de |r| — le rayon **négatif** retrace les pétales opposés.
        // Pour k impair, ces 2k lobes se superposent deux à deux et l'œil ne
        // voit que k pétales ; la polyligne, elle, en parcourt bien 2k.
        //
        // On teste donc ce que la géométrie fait réellement (2k extrêmes),
        // puis la superposition qui produit les k pétales visibles.
        let g = Graph::new();
        let pts = gen(&Rose::new(5.0, 1.0), &g);
        let rayons: Vec<f32> = pts.iter().map(|p| p.length()).collect();
        let n = rayons.len();
        let extremes = (0..n)
            .filter(|&i| {
                let prev = rayons[(i + n - 1) % n];
                let next = rayons[(i + 1) % n];
                rayons[i] >= prev && rayons[i] > next && rayons[i] > 0.9
            })
            .count();
        assert_eq!(extremes, 10, "k=5 : la courbe parcourt 2k=10 lobes");

        // Les pétales visibles : chaque pointe a une jumelle diamétralement
        // opposée, donc les 10 lobes ne dessinent que 5 directions.
        //
        // On mesure l'**espacement angulaire** plutôt que de dédoublonner une
        // liste triée : replier sur [0, π) coupe en deux le paquet à cheval
        // sur la frontière 0/π, et un `dedup` linéaire ne peut pas refermer le
        // cercle — il compterait 6 paquets au lieu de 5.
        let mut directions: Vec<f32> = (0..n)
            .filter(|&i| rayons[i] > 0.99)
            .map(|i| pts[i].y.atan2(pts[i].x).rem_euclid(std::f32::consts::PI))
            .collect();
        directions.sort_by(|a, b| a.partial_cmp(b).unwrap());

        // Écarts entre pointes consécutives, la boucle refermée sur π.
        let mut ecarts: Vec<f32> = directions
            .windows(2)
            .map(|w| w[1] - w[0])
            .filter(|e| *e > 0.1)
            .collect();
        ecarts.push(directions[0] + std::f32::consts::PI - directions[directions.len() - 1]);
        let ecarts: Vec<f32> = ecarts.into_iter().filter(|e| *e > 0.1).collect();

        assert_eq!(ecarts.len(), 5, "k=5 impair → 5 directions de pétales");
        for e in &ecarts {
            assert!(
                (e - std::f32::consts::PI / 5.0).abs() < 0.1,
                "les pétales doivent être équirépartis : écart {e}, attendu {}",
                std::f32::consts::PI / 5.0
            );
        }
    }

    #[test]
    fn la_rose_reste_dans_son_rayon() {
        let g = Graph::new();
        let pts = gen(&Rose::new(4.0, 0.7), &g);
        for p in &pts {
            assert!(p.length() <= 0.7 + EPS, "got {}", p.length());
        }
    }

    #[test]
    fn la_superformule_a_m_quatre_est_symetrique() {
        // m=4 → symétrie d'ordre 4 : une rotation d'un quart de tour laisse
        // l'ensemble des rayons inchangé.
        let g = Graph::new();
        let pts = gen(&Superformula::new(4.0), &g);
        let n = pts.len();
        let quart = n / 4;
        for i in 0..quart {
            let a = pts[i].length();
            let b = pts[(i + quart) % n].length();
            assert!((a - b).abs() < 0.05, "asymétrie à {i} : {a} vs {b}");
        }
    }

    #[test]
    fn la_superformule_reste_bornee_et_finie() {
        let g = Graph::new();
        for m in [3.0, 5.0, 7.0, 12.0] {
            let pts = gen(&Superformula::new(m), &g);
            assert!(tous_finis(&pts), "m={m}");
            for p in &pts {
                assert!(p.length() < 20.0, "m={m} : rayon {}", p.length());
            }
        }
    }

    #[test]
    fn la_superformule_ne_divise_pas_par_zero() {
        // n1, a et b nuls feraient diverger la formule.
        let g = Graph::new();
        let s = Superformula {
            n1: Param::fixed(0.0),
            a: Param::fixed(0.0),
            b: Param::fixed(0.0),
            ..Superformula::new(5.0)
        };
        let pts = gen(&s, &g);
        assert!(tous_finis(&pts), "aucun NaN ne doit sortir");
    }

    #[test]
    fn les_generateurs_parametriques_sont_modulables() {
        let mut g = Graph::new();
        let src = g.add(Box::new(Constant(0.5)));
        g.eval_frame(0.0, 1.0 / 60.0);

        let r = Rose::new(0.0, 1.0).with_k(Param::modulated(0.0, 6.0, src));
        let pts = gen(&r, &g);
        assert!(tous_finis(&pts));
        // k = 0 + 6 * 0.5 = 3 → rose à 3 pétales, donc rayon variable.
        let rmin = pts.iter().map(|p| p.length()).fold(f32::MAX, f32::min);
        let rmax = pts.iter().map(|p| p.length()).fold(0.0, f32::max);
        assert!(rmax - rmin > 0.5, "la modulation doit dessiner des pétales");
    }

    #[test]
    fn les_valeurs_degenerees_ne_paniquent_pas() {
        let g = Graph::new();
        let cas: Vec<Box<dyn ShapeGenerator>> = vec![
            Box::new(Lissajous::new(f32::NAN, f32::INFINITY)),
            Box::new(Rose::new(f32::NAN, -1.0)),
            Box::new(Rose::new(1e9, 1.0)),
            Box::new(Superformula::new(f32::NAN)),
            Box::new(Superformula {
                n1: Param::fixed(f32::NAN),
                n2: Param::fixed(f32::INFINITY),
                n3: Param::fixed(-1e9),
                ..Superformula::new(5.0)
            }),
        ];
        for (i, forme) in cas.iter().enumerate() {
            let pts = gen(forme.as_ref(), &g);
            assert!(tous_finis(&pts), "cas {i} : point non fini");
            assert!(pts.len() <= MAX_POINTS, "cas {i}");
        }
    }
}
