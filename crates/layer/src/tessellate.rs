//! Tessellation : `Path` → triangles — §10, §16.3 de `docs/ARCHITECTURE.md`.
//!
//! **Ce module ne connaît pas wgpu.** C'est le morceau de l'étape 5 où se
//! logent les cas limites (contour dégénéré, auto-intersection, contour
//! ouvert) : il reste testable en CI sans GPU (§24.3).
//!
//! ## Règles
//!
//! - **Remplissage** : seuls les contours **fermés** d'au moins 3 points sont
//!   remplis. Un contour ouvert (un arc) n'a pas d'intérieur ; le refermer
//!   d'office tracerait une corde qui n'existe pas dans la forme.
//! - Tous les contours fermés d'un `Path` sont remplis **ensemble**, en règle
//!   **non nulle** : là où deux copies d'une répétition radiale se
//!   chevauchent, la zone reste pleine (union) au lieu de se trouer.
//! - **Trait** : tout contour d'au moins 2 points, fermé ou ouvert selon son
//!   drapeau.
//! - Un contour contenant un point non fini est **ignoré** : un `NaN` ne doit
//!   jamais atteindre la tessellation (§18.3).
//! - Un échec de lyon ne panique pas : la sortie partielle est annulée et
//!   l'appel rend `false` — le régime « jamais de panique » du §18.1.
//!
//! ## Pas d'allocation par frame
//!
//! Le `Path` est converti en événements lyon à la volée, sans `Path` lyon
//! intermédiaire ; les tessellateurs et le [`Mesh`] sont réutilisés d'une
//! frame à l'autre, comme le `Path` lui-même (§16.3).

use frogenx_shape::{Contour, Path};
use glam::Vec2;
use lyon_tessellation::math::point;
use lyon_tessellation::path::PathEvent;
use lyon_tessellation::{
    BuffersBuilder, FillOptions, FillRule, FillTessellator, FillVertex, StrokeOptions,
    StrokeTessellator, StrokeVertex, VertexBuffers,
};

/// Triangles en mémoire CPU, dans l'espace normalisé des formes (§17).
///
/// Sommets en `[f32; 2]` plutôt qu'en `Vec2` : c'est le format que lyon
/// remplit, et il se prête à lyon sans conversion (voir `avec_buffers`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Mesh {
    pub vertices: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}

impl Mesh {
    pub fn new() -> Self {
        Self::default()
    }

    /// Vide le maillage en gardant les capacités.
    pub fn clear(&mut self) {
        self.vertices.clear();
        self.indices.clear();
    }

    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// Les triangles, sommet par sommet.
    pub fn triangles(&self) -> impl Iterator<Item = [Vec2; 3]> + '_ {
        self.indices.as_chunks::<3>().0.iter().map(|t| {
            [
                Vec2::from(self.vertices[t[0] as usize]),
                Vec2::from(self.vertices[t[1] as usize]),
                Vec2::from(self.vertices[t[2] as usize]),
            ]
        })
    }
}

fn tous_finis(pts: &[Vec2]) -> bool {
    pts.iter().all(|p| p.x.is_finite() && p.y.is_finite())
}

/// Événements lyon d'un ensemble de contours, sans allocation.
fn evenements<'a>(
    path: &'a Path,
    garder: impl Fn(&Contour, &[Vec2]) -> bool + 'a,
) -> impl Iterator<Item = PathEvent> + 'a {
    path.contours
        .iter()
        .map(move |c| (c, path.contour_points(c)))
        .filter(move |(c, pts)| garder(c, pts))
        .flat_map(|(c, pts)| {
            let p = |i: usize| point(pts[i].x, pts[i].y);
            let n = pts.len();
            (0..=n).map(move |k| match k {
                0 => PathEvent::Begin { at: p(0) },
                k if k < n => PathEvent::Line {
                    from: p(k - 1),
                    to: p(k),
                },
                _ => PathEvent::End {
                    last: p(n - 1),
                    first: p(0),
                    close: c.closed,
                },
            })
        })
}

/// Tessellateurs lyon réutilisés d'une frame à l'autre.
pub struct Tessellator {
    fill: FillTessellator,
    stroke: StrokeTessellator,
}

impl Default for Tessellator {
    fn default() -> Self {
        Self::new()
    }
}

impl Tessellator {
    pub fn new() -> Self {
        Self {
            fill: FillTessellator::new(),
            stroke: StrokeTessellator::new(),
        }
    }

    /// Ajoute à `out` le remplissage des contours fermés de `path`.
    ///
    /// Rend `false` si lyon a échoué ; `out` est alors laissé tel qu'avant
    /// l'appel.
    pub fn fill(&mut self, path: &Path, out: &mut Mesh) -> bool {
        let options = FillOptions::default().with_fill_rule(FillRule::NonZero);
        let events = evenements(path, |c, pts| c.closed && pts.len() >= 3 && tous_finis(pts));
        let fill = &mut self.fill;
        avec_buffers(out, |buffers| {
            let mut builder = BuffersBuilder::new(buffers, |v: FillVertex| v.position().to_array());
            fill.tessellate(events, &options, &mut builder).is_ok()
        })
    }

    /// Ajoute à `out` le trait de tous les contours de `path`.
    ///
    /// `width` est en unités de l'espace normalisé (§17). Une largeur nulle,
    /// négative ou non finie ne trace rien et n'est pas un échec.
    pub fn stroke(&mut self, path: &Path, width: f32, out: &mut Mesh) -> bool {
        if !(width.is_finite() && width > 0.0) {
            return true;
        }
        let options = StrokeOptions::default().with_line_width(width);
        let events = evenements(path, |_, pts| pts.len() >= 2 && tous_finis(pts));
        let stroke = &mut self.stroke;
        avec_buffers(out, |buffers| {
            let mut builder =
                BuffersBuilder::new(buffers, |v: StrokeVertex| v.position().to_array());
            stroke.tessellate(events, &options, &mut builder).is_ok()
        })
    }
}

/// Prête les vecteurs de `out` à lyon sous forme de `VertexBuffers`, sans
/// copie ni allocation (les `Vec` sont déplacés, puis rendus), et annule la
/// sortie partielle si `f` échoue.
fn avec_buffers(out: &mut Mesh, f: impl FnOnce(&mut VertexBuffers<[f32; 2], u32>) -> bool) -> bool {
    let (nv, ni) = (out.vertices.len(), out.indices.len());
    let mut buffers = VertexBuffers {
        vertices: std::mem::take(&mut out.vertices),
        indices: std::mem::take(&mut out.indices),
    };
    let ok = f(&mut buffers);
    out.vertices = buffers.vertices;
    out.indices = buffers.indices;
    if !ok {
        out.vertices.truncate(nv);
        out.indices.truncate(ni);
    }
    ok
}

#[cfg(test)]
mod tests {
    use super::*;

    fn carre(path: &mut Path, centre: Vec2, demi: f32) {
        path.points.extend([
            centre + Vec2::new(demi, demi),
            centre + Vec2::new(-demi, demi),
            centre + Vec2::new(-demi, -demi),
            centre + Vec2::new(demi, -demi),
        ]);
        path.end_contour(true);
    }

    /// Aire signée totale des triangles, en valeur absolue par triangle.
    fn aire(mesh: &Mesh) -> f32 {
        mesh.triangles()
            .map(|[a, b, c]| ((b - a).perp_dot(c - a) * 0.5).abs())
            .sum()
    }

    fn dans_le_carre(p: Vec2, centre: Vec2, demi: f32) -> bool {
        let d = (p - centre).abs();
        d.x <= demi + 1e-4 && d.y <= demi + 1e-4
    }

    #[test]
    fn un_contour_ferme_est_rempli_sur_toute_son_aire() {
        let mut path = Path::new();
        carre(&mut path, Vec2::ZERO, 0.5);
        let mut mesh = Mesh::new();
        assert!(Tessellator::new().fill(&path, &mut mesh));
        assert!(mesh.triangle_count() >= 2);
        assert!((aire(&mesh) - 1.0).abs() < 1e-4, "aire {}", aire(&mesh));
    }

    #[test]
    fn un_contour_ouvert_nest_jamais_rempli() {
        let mut path = Path::new();
        path.points.extend([
            Vec2::new(0.0, 0.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(1.0, 1.0),
        ]);
        path.end_contour(false);
        let mut mesh = Mesh::new();
        assert!(Tessellator::new().fill(&path, &mut mesh));
        assert!(mesh.vertices.is_empty() && mesh.indices.is_empty());
    }

    #[test]
    fn un_contour_ouvert_est_trace() {
        let mut path = Path::new();
        path.points
            .extend([Vec2::new(-0.5, 0.0), Vec2::new(0.5, 0.0)]);
        path.end_contour(false);
        let mut mesh = Mesh::new();
        assert!(Tessellator::new().stroke(&path, 0.1, &mut mesh));
        // Un segment d'un de long et 0.1 de large : aire 0.1, sans capuchon.
        assert!((aire(&mesh) - 0.1).abs() < 1e-3, "aire {}", aire(&mesh));
    }

    #[test]
    fn deux_contours_disjoints_ne_produisent_aucun_triangle_a_cheval() {
        // Le défaut que §16.3 corrige côté `shape` aurait reparu ici si la
        // tessellation reliait les contours entre eux.
        let (a, b) = (Vec2::new(-0.6, 0.0), Vec2::new(0.6, 0.0));
        let mut path = Path::new();
        carre(&mut path, a, 0.2);
        carre(&mut path, b, 0.2);
        let mut mesh = Mesh::new();
        assert!(Tessellator::new().fill(&path, &mut mesh));
        for t in mesh.triangles() {
            let centre = (t[0] + t[1] + t[2]) / 3.0;
            assert!(
                dans_le_carre(centre, a, 0.2) || dans_le_carre(centre, b, 0.2),
                "triangle hors des deux carrés : {t:?}"
            );
        }
        assert!((aire(&mesh) - 2.0 * 0.16).abs() < 1e-4);
    }

    #[test]
    fn deux_contours_qui_se_chevauchent_remplissent_leur_union() {
        // Règle non nulle : la zone commune reste pleine, pas trouée.
        let mut path = Path::new();
        carre(&mut path, Vec2::new(-0.1, 0.0), 0.2);
        carre(&mut path, Vec2::new(0.1, 0.0), 0.2);
        let mut mesh = Mesh::new();
        assert!(Tessellator::new().fill(&path, &mut mesh));
        // Union de deux carrés 0.4×0.4 décalés de 0.2 : 0.6 × 0.4.
        assert!((aire(&mesh) - 0.24).abs() < 1e-4, "aire {}", aire(&mesh));
    }

    #[test]
    fn les_contours_degeneres_ne_produisent_rien_sans_paniquer() {
        let mut path = Path::new();
        path.points.push(Vec2::ZERO); // un seul point
        path.end_contour(true);
        path.points.extend([Vec2::ZERO, Vec2::X]); // deux points fermés
        path.end_contour(true);
        path.points.extend([Vec2::ZERO; 4]); // points confondus
        path.end_contour(true);
        path.points
            .extend([Vec2::ZERO, Vec2::new(f32::NAN, 0.0), Vec2::Y]); // non fini
        path.end_contour(true);

        let mut t = Tessellator::new();
        let mut mesh = Mesh::new();
        assert!(t.fill(&path, &mut mesh));
        assert!(aire(&mesh) < 1e-6, "aucune aire attendue");
        assert!(mesh.vertices.iter().flatten().all(|c| c.is_finite()));
    }

    #[test]
    fn un_trait_de_largeur_invalide_ne_trace_rien() {
        let mut path = Path::new();
        carre(&mut path, Vec2::ZERO, 0.5);
        let mut t = Tessellator::new();
        for w in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            let mut mesh = Mesh::new();
            assert!(t.stroke(&path, w, &mut mesh), "largeur {w}");
            assert!(mesh.indices.is_empty(), "largeur {w}");
        }
    }

    #[test]
    fn remplir_puis_tracer_ajoute_au_meme_maillage() {
        // Les indices ajoutés par le trait doivent pointer vers ses propres
        // sommets, pas vers ceux du remplissage.
        let mut path = Path::new();
        carre(&mut path, Vec2::ZERO, 0.5);
        let mut t = Tessellator::new();
        let mut mesh = Mesh::new();
        t.fill(&path, &mut mesh);
        let apres_fill = mesh.vertices.len() as u32;
        t.stroke(&path, 0.05, &mut mesh);
        let trait_indices = &mesh.indices[mesh.indices.len() - 6..];
        assert!(trait_indices.iter().all(|&i| i >= apres_fill));
        assert!(mesh
            .indices
            .iter()
            .all(|&i| (i as usize) < mesh.vertices.len()));
    }

    #[test]
    fn le_tessellateur_est_reutilisable_dune_frame_a_lautre() {
        let mut path = Path::new();
        carre(&mut path, Vec2::ZERO, 0.5);
        let mut t = Tessellator::new();
        let mut mesh = Mesh::new();
        t.fill(&path, &mut mesh);
        let premier = mesh.clone();
        mesh.clear();
        t.fill(&path, &mut mesh);
        assert_eq!(mesh, premier, "même entrée, même sortie (§20.3)");
    }
}
