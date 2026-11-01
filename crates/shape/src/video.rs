//! Les réglages d'un clip vidéo — §9.3 de `docs/ARCHITECTURE.md`.
//!
//! Comme `FeedbackParams`, ici pour que `composition` les porte sans connaître
//! wgpu ni le décodage.

use crate::generator::ParamDesc;
use frogenx_core::{Graph, Param, Spec, SpecError};

const MIN_SCALE: f32 = 0.01;
const MAX_SCALE: f32 = 20.0;

/// Un fichier, et comment le poser dans l'image.
#[derive(Debug, Clone, PartialEq)]
pub struct VideoParams {
    pub path: String,
    /// En secondes : le calage sur le tempo a besoin de la connaître (§9.3).
    pub duration: f32,
    /// `1` : l'image tient entière dans le rendu.
    pub scale: Param,
    /// Radians.
    pub rotation: Param,
    /// Unités d'espace (§17).
    pub offset_x: Param,
    pub offset_y: Param,
}

/// La pose d'une frame, résolue.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VideoStep {
    pub scale: f32,
    pub angle: f32,
    pub offset: [f32; 2],
}

impl VideoParams {
    pub fn new(path: impl Into<String>, duration: f32) -> Self {
        Self {
            path: path.into(),
            duration,
            scale: Param::fixed(1.0),
            rotation: Param::fixed(0.0),
            offset_x: Param::fixed(0.0),
            offset_y: Param::fixed(0.0),
        }
    }

    pub fn kind(&self) -> &'static str {
        "Vidéo"
    }

    /// Ses paramètres réglables (§16.3 ter).
    pub fn params(&self) -> Vec<ParamDesc> {
        let desc = |name, min, max, value| ParamDesc {
            name,
            min,
            max,
            value,
        };
        vec![
            desc("échelle", 0.1, 4.0, self.scale),
            desc("rotation", -3.15, 3.15, self.rotation),
            desc("position x", -2.0, 2.0, self.offset_x),
            desc("position y", -2.0, 2.0, self.offset_y),
        ]
    }

    pub fn param_mut(&mut self, index: usize) -> Option<&mut Param> {
        match index {
            0 => Some(&mut self.scale),
            1 => Some(&mut self.rotation),
            2 => Some(&mut self.offset_x),
            3 => Some(&mut self.offset_y),
            _ => None,
        }
    }

    pub fn spec(&self) -> Spec {
        self.params()
            .into_iter()
            .fold(Spec::new(self.kind()), |spec, p| spec.with(p.name, p.value))
    }

    /// Le fichier et sa durée voyagent à part : `Spec` ne porte que des nombres.
    pub fn from_spec(spec: &Spec, path: String, duration: f32) -> Result<Self, SpecError> {
        let mut params = Self::new(path, duration);
        for (i, desc) in params.params().iter().enumerate() {
            let valeur = spec.param(desc.name)?;
            if let Some(param) = params.param_mut(i) {
                *param = valeur;
            }
        }
        Ok(params)
    }

    /// La pose de cette frame. Saturée ici, chez le consommateur (§18.4).
    pub fn step(&self, g: &Graph) -> VideoStep {
        let fini = |v: f32, repli: f32| if v.is_finite() { v } else { repli };
        VideoStep {
            scale: fini(self.scale.get(g), 1.0).clamp(MIN_SCALE, MAX_SCALE),
            angle: fini(self.rotation.get(g), 0.0),
            offset: [
                fini(self.offset_x.get(g), 0.0),
                fini(self.offset_y.get(g), 0.0),
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chaque_reglage_se_retrouve_par_son_rang() {
        let mut v = VideoParams::new("a.mp4", 2.0);
        for i in 0..v.params().len() {
            *v.param_mut(i).unwrap() = Param::fixed(i as f32 + 1.0);
        }
        assert_eq!(v.param_mut(4), None);
        let lues: Vec<f32> = v.params().iter().map(|p| p.value.base()).collect();
        assert_eq!(lues, [1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn la_description_rend_les_memes_reglages() {
        let mut v = VideoParams::new("a.mp4", 2.0);
        v.scale = Param::fixed(1.5);
        v.offset_y = Param::fixed(-0.25);
        let relu = VideoParams::from_spec(&v.spec(), "a.mp4".into(), 2.0).unwrap();
        assert_eq!(relu, v);
    }

    #[test]
    fn une_echelle_folle_reste_utilisable() {
        let mut v = VideoParams::new("a.mp4", 2.0);
        let g = Graph::new();
        v.scale = Param::fixed(f32::NAN);
        assert_eq!(v.step(&g).scale, 1.0);
        v.scale = Param::fixed(-3.0);
        assert_eq!(v.step(&g).scale, MIN_SCALE);
        v.scale = Param::fixed(1e6);
        assert_eq!(v.step(&g).scale, MAX_SCALE);
    }
}
