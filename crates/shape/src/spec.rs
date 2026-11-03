//! Reconstruire une forme ou un effet depuis sa description — §19.1 de
//! `docs/ARCHITECTURE.md`.
//!
//! Une forme neuve de son type, puis ses `Param` réglés par nom : `kind` et les
//! noms de [`params!`](crate::generator) suffisent, sans code par type.

use crate::generator::ShapeGenerator;
use crate::ops::{
    Chain, Deform, GridRepeat, Mirror, NoiseDisplace, RadialRepeat, ShapeOp, Smooth, Transform,
    Twist,
};
use crate::parametric::{Lissajous, Rose, Superformula};
use crate::primitives::{Arc, Ellipse, Polygon, Rect, Star};
use frogenx_core::{Spec, SpecError};

/// Ce qu'on peut ajouter à un clip (§23.10). Copiable, pour voyager dans une
/// commande.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EffectKind {
    Deform,
    Noise,
    RadialRepeat,
    Transform,
    Smooth,
    Mirror,
    GridRepeat,
    Twist,
}

impl EffectKind {
    pub const ALL: [EffectKind; 8] = [
        EffectKind::Deform,
        EffectKind::Noise,
        EffectKind::RadialRepeat,
        EffectKind::Transform,
        EffectKind::Smooth,
        EffectKind::Mirror,
        EffectKind::GridRepeat,
        EffectKind::Twist,
    ];

    /// Un effet neutre : l'ajouter ne change pas brutalement l'image.
    pub fn build(self) -> Box<dyn ShapeOp> {
        match self {
            EffectKind::Deform => Box::new(Deform::new(0.0, 5.0)),
            EffectKind::Noise => Box::new(NoiseDisplace::new(0.0, 3.0, 1)),
            EffectKind::RadialRepeat => Box::new(RadialRepeat::new(1.0)),
            EffectKind::Transform => Box::new(Transform::default()),
            EffectKind::Smooth => Box::new(Smooth::new(0.0, 0.5)),
            EffectKind::Mirror => Box::new(Mirror::default()),
            EffectKind::GridRepeat => Box::new(GridRepeat::default()),
            EffectKind::Twist => Box::new(Twist::new(0.0, 1.0)),
        }
    }

    pub fn label(self) -> &'static str {
        self.build().kind()
    }
}

/// Le générateur que décrit `spec`.
pub fn generator_from_spec(spec: &Spec) -> Result<Box<dyn ShapeGenerator>, SpecError> {
    let mut forme: Box<dyn ShapeGenerator> = match spec.kind.as_str() {
        "Ellipse" => Box::new(Ellipse::circle(0.5)),
        "Polygone" => Box::new(Polygon::new(3.0, 0.5)),
        "Étoile" => Box::new(Star::new(5.0, 0.5, 0.5)),
        "Rectangle" => Box::new(Rect::new(1.0, 1.0)),
        "Arc" => Box::new(Arc::new(0.0, 1.0, 0.5)),
        "Lissajous" => Box::new(Lissajous::new(1.0, 1.0)),
        "Rose" => Box::new(Rose::new(1.0, 0.5)),
        "Superformule" => Box::new(Superformula::new(1.0)),
        "Chaîne" => return chain_from_spec(spec),
        autre => return Err(SpecError::UnknownKind(autre.to_string())),
    };
    for (i, desc) in forme.params().iter().enumerate() {
        let valeur = spec.param(desc.name)?;
        if let Some(param) = forme.param_mut(i) {
            *param = valeur;
        }
    }
    Ok(forme)
}

/// L'effet que décrit `spec`.
pub fn op_from_spec(spec: &Spec) -> Result<Box<dyn ShapeOp>, SpecError> {
    let mut effet: Box<dyn ShapeOp> = match spec.kind.as_str() {
        "Déformation" => Box::new(Deform::new(0.0, 1.0)),
        "Bruit" => Box::new(NoiseDisplace::new(0.0, 1.0, spec.int("graine")?)),
        "Répétition radiale" => Box::new(RadialRepeat::new(1.0)),
        "Transformation" => Box::new(Transform::default()),
        "Lissage" => Box::new(Smooth::new(1.0, 0.5)),
        "Miroir" => Box::new(Mirror::default()),
        "Répétition en grille" => Box::new(GridRepeat::default()),
        "Torsion" => Box::new(Twist::new(0.0, 1.0)),
        autre => return Err(SpecError::UnknownKind(autre.to_string())),
    };
    for (i, desc) in effet.params().iter().enumerate() {
        let valeur = spec.param(desc.name)?;
        if let Some(param) = effet.param_mut(i) {
            *param = valeur;
        }
    }
    Ok(effet)
}

fn chain_from_spec(spec: &Spec) -> Result<Box<dyn ShapeGenerator>, SpecError> {
    let source = spec
        .specs("source")?
        .first()
        .ok_or_else(|| spec.field_error("source"))?;
    let mut chaine = Chain::new(generator_from_spec(source)?);
    for op in spec.specs("ops")? {
        chaine = chaine.then(op_from_spec(op)?);
    }
    Ok(Box::new(chaine))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generator::test_util::gen_path;
    use crate::ops::Chain;
    use frogenx_core::{Graph, Param};

    /// Une forme de chaque type, tous ses paramètres décalés de leur défaut.
    fn formes() -> Vec<Box<dyn ShapeGenerator>> {
        let mut formes: Vec<Box<dyn ShapeGenerator>> = vec![
            Box::new(Ellipse::circle(0.3)),
            Box::new(Polygon::new(7.0, 0.3)),
            Box::new(Star::new(9.0, 0.3, 0.2)),
            Box::new(Rect::new(0.3, 0.4)),
            Box::new(Arc::new(0.3, 2.0, 0.4)),
            Box::new(Lissajous::new(3.0, 2.0)),
            Box::new(Rose::new(5.0, 0.3)),
            Box::new(Superformula::new(6.0)),
        ];
        for forme in &mut formes {
            for i in 0..forme.params().len() {
                let p = forme.param_mut(i).expect("dans la liste");
                p.set_base(p.base() + 0.125);
            }
        }
        formes
    }

    fn effets() -> Vec<Box<dyn ShapeOp>> {
        vec![
            Box::new(Deform::new(0.1, 4.0)),
            Box::new(NoiseDisplace::new(0.1, 3.0, 77)),
            Box::new(RadialRepeat::new(5.0)),
            Box::new(Transform::default()),
            Box::new(Smooth::new(2.0, 0.25)),
            Box::new(Twist::new(1.5, 0.75)),
            Box::new(GridRepeat::new(3.0, 2.0, 0.4).with_offset(0.5)),
            Box::new(Mirror::new(3.0, 0.25)),
        ]
    }

    #[test]
    fn chaque_forme_se_reconstruit_a_l_identique() {
        let g = Graph::new();
        for forme in formes() {
            let copie = generator_from_spec(&forme.spec()).unwrap();
            assert_eq!(copie.spec(), forme.spec());
            let (a, b) = (gen_path(forme.as_ref(), &g), gen_path(copie.as_ref(), &g));
            assert_eq!(a.points, b.points, "{}", forme.kind());
        }
    }

    #[test]
    fn chaque_effet_se_reconstruit_a_l_identique() {
        for effet in effets() {
            let copie = op_from_spec(&effet.spec()).unwrap();
            assert_eq!(copie.spec(), effet.spec());
        }
    }

    #[test]
    fn les_noms_de_parametres_sont_uniques_dans_chaque_type() {
        // Ils servent de clés dans le fichier : un doublon perdrait un réglage.
        let mut listes: Vec<_> = formes().iter().map(|f| f.params()).collect();
        listes.extend(effets().iter().map(|e| e.params()));
        for params in listes {
            let mut noms: Vec<_> = params.iter().map(|p| p.name).collect();
            noms.sort_unstable();
            noms.dedup();
            assert_eq!(noms.len(), params.len());
        }
    }

    #[test]
    fn chaque_effet_s_ajoute_et_se_decrit() {
        for kind in EffectKind::ALL {
            let effet = kind.build();
            assert!(op_from_spec(&effet.spec()).is_ok(), "{}", kind.label());
            assert!(!kind.label().is_empty());
        }
    }

    #[test]
    fn la_graine_d_un_bruit_survit() {
        let effet = NoiseDisplace::new(0.1, 3.0, 77);
        assert_eq!(effet.spec().int("graine"), Ok(77));
        let copie = op_from_spec(&effet.spec()).unwrap();
        assert_eq!(copie.spec().int("graine"), Ok(77));
    }

    #[test]
    fn une_modulation_survit() {
        let mut g = Graph::new();
        let source = g.add(Box::new(frogenx_core::Constant(1.0)));
        let module = Ellipse::circle(0.5).with_radius(Param::modulated(0.5, 0.25, source));
        let copie = generator_from_spec(&module.spec()).unwrap();
        assert_eq!(copie.spec(), module.spec());
    }

    #[test]
    fn une_chaine_garde_sa_source_et_ses_operateurs_dans_l_ordre() {
        let g = Graph::new();
        let chaine = Chain::new(Box::new(Rose::new(5.0, 0.5)))
            .then(Box::new(Deform::new(0.1, 4.0)))
            .then(Box::new(RadialRepeat::new(3.0)));
        let copie = generator_from_spec(&chaine.spec()).unwrap();
        assert_eq!(copie.spec(), chaine.spec());
        assert_eq!(
            gen_path(&chaine, &g).points,
            gen_path(copie.as_ref(), &g).points
        );
    }

    #[test]
    fn un_type_inconnu_est_refuse_en_le_nommant() {
        let spec = Spec::new("Spirale");
        assert_eq!(
            generator_from_spec(&spec).err(),
            Some(SpecError::UnknownKind("Spirale".into()))
        );
        assert!(op_from_spec(&spec).is_err());
    }

    #[test]
    fn un_parametre_manquant_est_refuse() {
        let incomplet = Spec::new("Ellipse").with("rayon", Param::fixed(0.5));
        assert!(generator_from_spec(&incomplet).is_err());
    }
}
