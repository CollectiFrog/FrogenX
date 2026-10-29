//! `Path` — une forme comme ensemble de contours, §16.3 de `docs/ARCHITECTURE.md`.
//!
//! Un buffer plat de points, découpé en plages. Chaque plage est un contour,
//! fermé (cercle, polygone) ou ouvert (arc). Une répétition radiale produit
//! plusieurs contours fermés dans le même `Path`.
//!
//! **Pourquoi pas une polyligne unique :** elle ne sait dire ni où un contour
//! finit, ni s'il est fermé. `RadialRepeat` collait ses copies bout à bout — le
//! rasteriseur aurait relié chaque copie à la suivante — et `Smooth` moyennait
//! les deux extrémités d'un arc ouvert.
//!
//! **Pourquoi pas un `Vec` par contour :** une allocation par contour et par
//! frame, ce que le §16.3 interdit. Ici, le `Path` est réutilisé d'une frame à
//! l'autre et `clear()` garde ses capacités.

use glam::Vec2;
use std::ops::Range;

/// Une plage de `Path::points`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Contour {
    /// Index du premier point.
    pub start: u32,
    /// Index de fin, **exclusif**.
    pub end: u32,
    /// Vrai si le dernier point se relie au premier. Le premier point n'est
    /// jamais répété en fin de contour (§16.3).
    pub closed: bool,
}

impl Contour {
    pub fn range(&self) -> Range<usize> {
        self.start as usize..self.end as usize
    }

    pub fn len(&self) -> usize {
        self.end.saturating_sub(self.start) as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Une forme : des points, et les contours qui les découpent.
///
/// **Invariant** (vérifié par [`Path::is_valid`]) : les contours se suivent
/// dans l'ordre, sans trou ni chevauchement, couvrent exactement `points`, et
/// aucun n'est vide.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Path {
    pub points: Vec<Vec2>,
    pub contours: Vec<Contour>,
}

impl Path {
    pub fn new() -> Self {
        Self::default()
    }

    /// Vide la forme en gardant les capacités allouées — c'est ce qui permet
    /// de réutiliser le même `Path` à chaque frame sans allouer.
    pub fn clear(&mut self) {
        self.points.clear();
        self.contours.clear();
    }

    /// Clôt le contour en cours : tous les points ajoutés depuis la fin du
    /// contour précédent. Ne fait rien si aucun point n'a été ajouté — un
    /// contour vide violerait l'invariant.
    ///
    /// Usage dans un générateur : `out.points.push(...)` autant que nécessaire,
    /// puis `out.end_contour(closed)`.
    pub fn end_contour(&mut self, closed: bool) {
        let start = self.contours.last().map_or(0, |c| c.end);
        let end = self.points.len() as u32;
        if end > start {
            self.contours.push(Contour { start, end, closed });
        }
    }

    /// Les points d'un contour.
    pub fn contour_points(&self, c: &Contour) -> &[Vec2] {
        self.points.get(c.range()).unwrap_or(&[])
    }

    /// Vérifie l'invariant des plages. Sert aux tests et aux assertions de
    /// débogage ; le rendu ne s'y fie pas pour éviter une panique (§18.1).
    pub fn is_valid(&self) -> bool {
        let mut attendu = 0u32;
        for c in &self.contours {
            if c.start != attendu || c.end <= c.start {
                return false;
            }
            attendu = c.end;
        }
        attendu as usize == self.points.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(x: f32) -> Vec2 {
        Vec2::new(x, 0.0)
    }

    #[test]
    fn un_path_vide_est_valide() {
        assert!(Path::new().is_valid());
    }

    #[test]
    fn end_contour_decoupe_les_points_ajoutes_depuis_le_contour_precedent() {
        let mut path = Path::new();
        path.points.extend([p(0.0), p(1.0), p(2.0)]);
        path.end_contour(true);
        path.points.extend([p(3.0), p(4.0)]);
        path.end_contour(false);

        assert!(path.is_valid());
        assert_eq!(
            path.contours,
            vec![
                Contour {
                    start: 0,
                    end: 3,
                    closed: true
                },
                Contour {
                    start: 3,
                    end: 5,
                    closed: false
                },
            ]
        );
        assert_eq!(path.contour_points(&path.contours[1]), &[p(3.0), p(4.0)]);
    }

    #[test]
    fn end_contour_sans_point_nouveau_ne_cree_pas_de_contour_vide() {
        let mut path = Path::new();
        path.end_contour(true);
        path.points.push(p(0.0));
        path.end_contour(true);
        path.end_contour(true);
        assert_eq!(path.contours.len(), 1);
        assert!(path.is_valid());
    }

    #[test]
    fn des_points_hors_contour_rendent_le_path_invalide() {
        let mut path = Path::new();
        path.points.extend([p(0.0), p(1.0)]);
        path.end_contour(true);
        path.points.push(p(2.0)); // jamais rattaché à un contour
        assert!(!path.is_valid());
    }

    #[test]
    fn un_trou_ou_un_chevauchement_rend_le_path_invalide() {
        let points = vec![p(0.0), p(1.0), p(2.0), p(3.0)];
        let trou = Path {
            points: points.clone(),
            contours: vec![
                Contour {
                    start: 0,
                    end: 1,
                    closed: true,
                },
                Contour {
                    start: 2,
                    end: 4,
                    closed: true,
                },
            ],
        };
        let chevauchement = Path {
            points: points.clone(),
            contours: vec![
                Contour {
                    start: 0,
                    end: 3,
                    closed: true,
                },
                Contour {
                    start: 2,
                    end: 4,
                    closed: true,
                },
            ],
        };
        let vide = Path {
            points,
            contours: vec![
                Contour {
                    start: 0,
                    end: 0,
                    closed: true,
                },
                Contour {
                    start: 0,
                    end: 4,
                    closed: true,
                },
            ],
        };
        assert!(!trou.is_valid());
        assert!(!chevauchement.is_valid());
        assert!(!vide.is_valid());
    }

    #[test]
    fn clear_garde_la_capacite() {
        // §16.3 : réutiliser le Path d'une frame à l'autre sans réallouer.
        let mut path = Path::new();
        path.points.extend((0..100).map(|i| p(i as f32)));
        path.end_contour(true);
        let capacite = path.points.capacity();
        path.clear();
        assert!(path.points.is_empty() && path.contours.is_empty());
        assert_eq!(path.points.capacity(), capacite);
    }

    #[test]
    fn contour_points_hors_bornes_rend_une_tranche_vide_sans_paniquer() {
        let path = Path::new();
        let fantome = Contour {
            start: 5,
            end: 9,
            closed: true,
        };
        assert!(path.contour_points(&fantome).is_empty());
    }
}
