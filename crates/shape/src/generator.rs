//! Le trait `ShapeGenerator` et le contrat des formes — §16.3.

use crate::path::Path;
use frogenx_core::{Ctx, Graph, Param, Spec};

/// Tout ce qui produit une forme.
///
/// Une forme est un [`Path`] : des points découpés en contours, chacun fermé
/// ou ouvert. Un cercle est un contour fermé assez dense pour paraître lisse.
///
/// **Écriture dans un buffer fourni, pas de `Path` retourné (§16.3).** Le rendu
/// tourne à 60 fps ; allouer par forme et par frame produit une pression
/// mémoire inutile. L'appelant possède le `Path` et le réutilise.
///
/// **Convention d'espace (§17) :** coordonnées normalisées, origine au centre,
/// `[-1, 1]` sur le petit côté. Un patch rendu en 720p ou en 4K donne donc la
/// même image.
///
/// **Convention de fermeture :** un contour fermé ne répète **pas** son premier
/// point à la fin — c'est le drapeau `closed` qui dit au rasteriseur de le
/// refermer. Répéter le point dupliquerait un sommet à la tessellation.
pub trait ShapeGenerator {
    /// Remplit `out` avec la forme.
    ///
    /// L'implémentation **doit** commencer par `out.clear()` — l'appelant
    /// fournit un `Path` potentiellement déjà rempli — et laisser `out`
    /// valide au sens de [`Path::is_valid`].
    ///
    /// `g` sert à résoudre les `Param` via [`Param::get`](frogenx_core::Param::get) :
    /// les formes sont générées **après** `eval_frame` (§20.1), donc les valeurs
    /// sont déjà dans le cache. `ctx` donne accès à `dt` et aux features audio
    /// pour les générateurs qui en ont besoin.
    fn generate(&self, t: f64, ctx: &Ctx, g: &Graph, out: &mut Path);

    /// Le nom de ce type de forme, tel qu'affiché.
    fn kind(&self) -> &'static str;

    /// Ses paramètres réglables (§16.3 ter). Déclarés par [`params!`].
    fn params(&self) -> Vec<ParamDesc>;

    /// Le `index`-ième paramètre de [`Self::params`], en écriture.
    fn param_mut(&mut self, index: usize) -> Option<&mut Param>;

    /// Sa description pour le format de patch (§19.1), déduite de ses `Param`.
    fn spec(&self) -> Spec {
        describe(self.kind(), self.params())
    }
}

/// Une forme ou un effet, décrit par ses `Param` nommés.
pub(crate) fn describe(kind: &str, params: Vec<ParamDesc>) -> Spec {
    params
        .into_iter()
        .fold(Spec::new(kind), |spec, p| spec.with(p.name, p.value))
}

/// Un paramètre réglable, décrit pour qui ne connaît pas le type de la forme.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ParamDesc {
    pub name: &'static str,
    /// Plage du curseur. Ce n'est pas une saturation : une modulation peut en
    /// sortir, et c'est le consommateur qui sature (§18.4).
    pub min: f32,
    pub max: f32,
    pub value: Param,
}

/// Écrit `kind`, `params` et `param_mut` depuis une seule liste : `param_mut(i)`
/// désigne ainsi toujours le `i`-ème de `params()`.
macro_rules! params {
    ($kind:literal; $($field:ident: $name:literal, $min:expr, $max:expr;)*) => {
        fn kind(&self) -> &'static str {
            $kind
        }

        fn params(&self) -> Vec<$crate::generator::ParamDesc> {
            vec![$($crate::generator::ParamDesc {
                name: $name,
                min: $min,
                max: $max,
                value: self.$field,
            }),*]
        }

        fn param_mut(&mut self, index: usize) -> Option<&mut frogenx_core::Param> {
            let mut i = 0;
            $(
                if i == index {
                    return Some(&mut self.$field);
                }
                i += 1;
            )*
            let _ = (i, index);
            None
        }
    };
}
pub(crate) use params;

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
    use glam::Vec2;

    /// Génère une forme dans un contexte minimal, pour les tests.
    ///
    /// **Vérifie l'invariant des plages à chaque appel** : tout test qui génère
    /// une forme teste du même coup que générateurs et opérateurs laissent un
    /// `Path` valide.
    pub fn gen_path(shape: &dyn ShapeGenerator, g: &Graph) -> Path {
        let audio = AudioFeatures::default();
        let ctx = Ctx::new(1.0 / 60.0, 0, &audio);
        let mut out = Path::new();
        shape.generate(0.0, &ctx, g, &mut out);
        assert!(out.is_valid(), "Path invalide : {:?}", out.contours);
        out
    }

    /// Les points seuls, pour les tests qui ne regardent pas les contours.
    pub fn gen(shape: &dyn ShapeGenerator, g: &Graph) -> Vec<Vec2> {
        gen_path(shape, g).points
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

#[cfg(test)]
mod params_tests {
    use super::ParamDesc;
    use crate::*;

    /// Une liste cohérente, et `param_mut(i)` qui atteint bien le `i`-ème.
    fn verifie(
        kind: &str,
        attendus: usize,
        params: impl Fn() -> Vec<ParamDesc>,
        mut ecrit: impl FnMut(usize, f32) -> bool,
    ) {
        assert_eq!(params().len(), attendus, "{kind}");
        for p in params().iter() {
            assert!(p.min < p.max, "{kind}.{}", p.name);
            assert!(!p.name.is_empty(), "{kind}");
        }
        for i in 0..attendus {
            let valeur = 1000.0 + i as f32;
            assert!(ecrit(i, valeur), "{kind} : param_mut({i})");
        }
        for (i, p) in params().iter().enumerate() {
            assert_eq!(p.value.base(), 1000.0 + i as f32, "{kind}.{}", p.name);
        }
        assert!(!ecrit(attendus, 0.0), "{kind} : au-delà de la liste");
    }

    /// Le compte de chaque ligne est celui des champs `Param` de la structure :
    /// un champ oublié dans `params!` serait invisible pour l'interface, la
    /// sauvegarde et le MIDI.
    #[test]
    fn chaque_forme_declare_tous_ses_param() {
        let generateurs: Vec<(Box<dyn ShapeGenerator>, usize)> = vec![
            (Box::new(Ellipse::circle(0.5)), 3),
            (Box::new(Polygon::new(5.0, 0.5)), 5),
            (Box::new(Star::new(5.0, 0.5, 0.5)), 4),
            (Box::new(Rect::new(1.0, 1.0)), 4),
            (Box::new(Arc::new(0.0, 1.0, 0.5)), 4),
            (Box::new(Lissajous::new(3.0, 2.0)), 5),
            (Box::new(Rose::new(5.0, 0.5)), 3),
            (Box::new(Superformula::new(5.0)), 8),
        ];
        for (forme, n) in generateurs {
            let forme = std::cell::RefCell::new(forme);
            let kind = forme.borrow().kind();
            verifie(
                kind,
                n,
                || forme.borrow().params(),
                |i, v| {
                    let mut f = forme.borrow_mut();
                    match f.param_mut(i) {
                        Some(p) => {
                            p.set_base(v);
                            true
                        }
                        None => false,
                    }
                },
            );
        }

        let operateurs: Vec<(Box<dyn ShapeOp>, usize)> = vec![
            (Box::new(Deform::new(0.1, 3.0)), 3),
            (Box::new(NoiseDisplace::new(0.1, 3.0, 1)), 3),
            (Box::new(RadialRepeat::new(3.0)), 2),
            (Box::new(Transform::default()), 4),
            (Box::new(Smooth::new(1.0, 0.5)), 2),
            (Box::new(LinearWave::new(0.0, 0.5)), 4),
            (Box::new(Dashes::new(0.1, 0.0)), 3),
            (Box::new(Trim::default()), 3),
            (Box::new(ShapeEchoes::new(1.0, 0.8, 0.2)), 3),
            (Box::new(Twist::new(0.0, 1.0)), 2),
            (Box::new(GridRepeat::default()), 4),
            (Box::new(Mirror::default()), 2),
        ];
        for (op, n) in operateurs {
            let op = std::cell::RefCell::new(op);
            let kind = op.borrow().kind();
            verifie(
                kind,
                n,
                || op.borrow().params(),
                |i, v| {
                    let mut o = op.borrow_mut();
                    match o.param_mut(i) {
                        Some(p) => {
                            p.set_base(v);
                            true
                        }
                        None => false,
                    }
                },
            );
        }
    }

    #[test]
    fn une_chaine_expose_sa_source_puis_ses_operateurs() {
        let mut chaine = Chain::new(Box::new(Rose::new(5.0, 0.5)))
            .then(Box::new(Deform::new(0.1, 3.0)))
            .then(Box::new(RadialRepeat::new(3.0)));
        assert_eq!(chaine.params().len(), 3 + 3 + 2);

        chaine
            .param_mut(3)
            .expect("force de la déformation")
            .set_base(0.4);
        chaine
            .param_mut(6)
            .expect("copies de la répétition")
            .set_base(7.0);
        let params = chaine.params();
        assert_eq!(params[3].value.base(), 0.4);
        assert_eq!(params[6].value.base(), 7.0);
        assert!(chaine.param_mut(8).is_none());
    }
}
