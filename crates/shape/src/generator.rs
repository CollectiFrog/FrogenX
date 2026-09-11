//! Le trait `ShapeGenerator` et le contrat des polylignes — §16.3.

use frogenx_core::{Ctx, Graph};
use glam::Vec2;

/// Tout ce qui produit une forme.
///
/// Une forme est une **polyligne** : une suite de points. Simple, uniforme, et
/// suffisant pour tout ce qui suit — un cercle est une polyligne assez dense
/// pour paraître lisse.
///
/// **Écriture dans un buffer fourni, pas de `Vec` retourné (§16.3).** Le rendu
/// tourne à 60 fps ; allouer un `Vec` par forme et par frame produit une
/// pression mémoire inutile. L'appelant possède le buffer et le réutilise.
///
/// **Convention d'espace (§17) :** coordonnées normalisées, origine au centre,
/// `[-1, 1]` sur le petit côté. Un patch rendu en 720p ou en 4K donne donc la
/// même image.
///
/// **Convention de fermeture :** une forme fermée ne répète **pas** son premier
/// point à la fin. C'est au rasteriseur de refermer le contour. Répéter le
/// point dupliquerait un sommet à la tessellation et compliquerait tous les
/// opérateurs (une répétition radiale sur 6 branches produirait 6 doublons).
pub trait ShapeGenerator {
    /// Remplit `out` avec les points de la forme.
    ///
    /// L'implémentation **doit** commencer par `out.clear()` — l'appelant
    /// fournit un buffer potentiellement déjà rempli.
    ///
    /// `g` sert à résoudre les `Param` via [`Param::get`](frogenx_core::Param::get) :
    /// les formes sont générées **après** `eval_frame` (§20.1), donc les valeurs
    /// sont déjà dans le cache. `ctx` donne accès à `dt` et aux features audio
    /// pour les générateurs qui en ont besoin.
    fn generate(&self, t: f64, ctx: &Ctx, g: &Graph, out: &mut Vec<Vec2>);
}

/// Nombre de segments utilisé pour discrétiser une courbe lisse.
///
/// 128 segments suffisent à ce qu'un cercle paraisse lisse en 4K sans charger
/// inutilement la tessellation. Les formes à faible nombre de côtés (polygone,
/// étoile) utilisent leur propre résolution, pas celle-ci.
pub const SMOOTH_SEGMENTS: usize = 128;

/// Borne dure sur le nombre de points d'une forme.
///
/// Une modulation dégénérée (un `depth` énorme sur un nombre de côtés) ne doit
/// pas faire allouer des millions de points et figer l'application. C'est la
/// saturation chez le consommateur du §18.4.
pub const MAX_POINTS: usize = 16_384;

#[cfg(test)]
pub(crate) mod test_util {
    use super::*;
    use frogenx_core::{AudioFeatures, Ctx, Graph};

    /// Génère une forme dans un contexte minimal, pour les tests.
    pub fn gen(shape: &dyn ShapeGenerator, g: &Graph) -> Vec<Vec2> {
        let audio = AudioFeatures::default();
        let ctx = Ctx::new(1.0 / 60.0, 0, &audio);
        let mut out = Vec::new();
        shape.generate(0.0, &ctx, g, &mut out);
        out
    }

    /// Vrai si tous les points sont finis — aucun NaN ne doit sortir d'une forme.
    pub fn tous_finis(pts: &[Vec2]) -> bool {
        pts.iter().all(|p| p.x.is_finite() && p.y.is_finite())
    }

    /// Distance maximale entre deux points consécutifs, contour refermé.
    ///
    /// Sert à vérifier la régularité d'un échantillonnage : un saut isolé
    /// trahit un point mal placé.
    pub fn ecart_max(pts: &[Vec2]) -> f32 {
        if pts.len() < 2 {
            return 0.0;
        }
        let mut max = 0.0f32;
        for i in 0..pts.len() {
            let a = pts[i];
            let b = pts[(i + 1) % pts.len()];
            max = max.max((b - a).length());
        }
        max
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn les_constantes_sont_coherentes() {
        const { assert!(SMOOTH_SEGMENTS >= 3, "il faut au moins un triangle") };
        const {
            assert!(
                SMOOTH_SEGMENTS < MAX_POINTS,
                "la résolution lisse doit tenir sous la borne dure"
            )
        };
    }
}
