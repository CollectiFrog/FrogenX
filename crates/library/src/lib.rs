//! # frogenx-library
//!
//! Les clips prédéfinis et la grille de départ — §23.6 de
//! `docs/ARCHITECTURE.md`.
//!
//! Le [catalogue](catalog) est statique : chaque entrée sait construire un
//! clip neuf. Écrit en code, parce que le format de fichier n'arrive qu'à
//! l'étape 7e. L'interface désigne une entrée par son [`PresetId`] : `Command`
//! reste `Copy`, et c'est `app` qui construit le clip entre deux frames.

mod clips;
mod starter;

use frogenx_composition::Clip;

pub use starter::{starter, STARTER_COLUMNS};

/// Durée d'un cycle de modulation, en battements. À 120 BPM, les LFO bouclent
/// en 8 battements, soit 4 s.
pub const CYCLE: f32 = 8.0;

/// Les familles du catalogue, dans l'ordre du panneau.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Backgrounds,
    Shapes,
    Lines,
    Patterns,
}

impl Family {
    pub const ALL: [Family; 4] = [
        Family::Backgrounds,
        Family::Shapes,
        Family::Lines,
        Family::Patterns,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Family::Backgrounds => "Fonds",
            Family::Shapes => "Formes",
            Family::Lines => "Traits",
            Family::Patterns => "Motifs",
        }
    }
}

/// Désigne une entrée du catalogue, sans la construire.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PresetId(usize);

/// Une entrée du catalogue.
#[derive(Debug, Clone, Copy)]
pub struct Preset {
    pub name: &'static str,
    pub family: Family,
    make: fn() -> Clip,
}

impl Preset {
    const fn new(name: &'static str, family: Family, make: fn() -> Clip) -> Self {
        Self { name, family, make }
    }

    /// Un clip neuf, qui porte le nom de l'entrée.
    pub fn build(&self) -> Clip {
        (self.make)().with_name(self.name)
    }
}

/// Tout le catalogue, dans l'ordre du panneau.
pub fn catalog() -> &'static [Preset] {
    &clips::CATALOG
}

/// Les entrées du catalogue et leur identifiant.
pub fn presets() -> impl Iterator<Item = (PresetId, &'static Preset)> {
    catalog().iter().enumerate().map(|(i, p)| (PresetId(i), p))
}

/// L'entrée désignée, ou `None` pour un identifiant hors du catalogue.
pub fn preset(id: PresetId) -> Option<&'static Preset> {
    catalog().get(id.0)
}

/// L'entrée de ce nom.
pub fn find(name: &str) -> Option<PresetId> {
    presets().find(|(_, p)| p.name == name).map(|(id, _)| id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use frogenx_composition::ClipSource;
    use frogenx_core::{AudioFeatures, Ctx};
    use frogenx_shape::Path;

    const BPM: f32 = 120.0;
    const DT: f64 = 1.0 / 60.0;

    /// Une période d'un LFO à 120 BPM, plus une marge.
    const FRAMES: u64 = 240;

    #[test]
    fn le_catalogue_compte_24_clips_en_quatre_familles() {
        assert_eq!(catalog().len(), 24);
        let compte = |f| catalog().iter().filter(|p| p.family == f).count();
        assert_eq!(compte(Family::Backgrounds), 4);
        assert_eq!(compte(Family::Shapes), 8);
        assert_eq!(compte(Family::Lines), 6);
        assert_eq!(compte(Family::Patterns), 6);
    }

    #[test]
    fn deux_entrees_ne_portent_jamais_le_meme_nom() {
        for (i, a) in catalog().iter().enumerate() {
            for b in &catalog()[i + 1..] {
                assert_ne!(a.name, b.name);
            }
        }
    }

    #[test]
    fn chaque_clip_porte_le_nom_du_catalogue_et_dessine() {
        for preset in catalog() {
            let clip = preset.build();
            assert_eq!(clip.name, preset.name);
            let ClipSource::Shape { style, .. } = &clip.source;
            assert!(
                style.fill.is_some() || style.stroke.is_some(),
                "{} ne dessine rien",
                preset.name
            );
        }
    }

    #[test]
    fn chaque_clip_produit_une_forme_valide_sans_diagnostic() {
        let audio = AudioFeatures::default();
        for preset in catalog() {
            let mut clip = preset.build();
            for frame in 0..FRAMES {
                let t = frame as f64 * DT;
                let graph = clip.graph_mut();
                graph.set_bpm(BPM);
                graph.eval_frame(t, DT);

                let ctx = Ctx::new(DT, frame, &audio).with_bpm(BPM);
                let ClipSource::Shape {
                    graph, generator, ..
                } = &clip.source;
                let mut path = Path::new();
                generator.generate(t, &ctx, graph, &mut path);
                for op in &clip.effects {
                    op.apply(t, &ctx, graph, &mut path);
                }
                let nom = preset.name;
                assert!(path.is_valid(), "{nom}, frame {frame}");
                assert!(!path.points.is_empty(), "{nom}, frame {frame}");
                assert!(
                    path.points.iter().all(|p| p.is_finite()),
                    "{nom}, frame {frame}"
                );
            }
            let diagnostics = clip.graph().diagnostics();
            assert!(diagnostics.is_clean(), "{} : {diagnostics:?}", preset.name);
        }
    }

    #[test]
    fn chaque_clip_bouge_sauf_le_fond_uni() {
        // Sans modulation, ni l'amplitude ni la vitesse n'auraient d'effet (§23.6).
        for preset in catalog().iter().filter(|p| p.name != "Nuit") {
            let module = preset
                .build()
                .params()
                .iter()
                .flat_map(|g| &g.params)
                .any(|p| p.desc.value.source().is_some());
            assert!(module, "{} ne bouge pas", preset.name);
        }
    }

    #[test]
    fn deux_constructions_sont_independantes() {
        // Un clip réglé n'est plus celui du catalogue (§23.6).
        let preset = preset(find("Blob").unwrap()).unwrap();
        let mut a = preset.build();
        let slot = a.params()[0].params[0].slot;
        assert!(a.set_param(slot, 0.1));
        let neuf = preset.build().params()[0].params[0].desc.value.base();
        assert_eq!(neuf, 0.72);
    }

    #[test]
    fn un_identifiant_hors_du_catalogue_ne_designe_rien() {
        assert!(preset(PresetId(catalog().len())).is_none());
        assert_eq!(find("Inconnu"), None);
    }

    #[test]
    fn un_identifiant_retrouve_son_entree() {
        for (id, entree) in presets() {
            assert_eq!(preset(id).unwrap().name, entree.name);
            assert_eq!(find(entree.name), Some(id));
        }
    }

    #[test]
    fn chaque_famille_a_un_libelle() {
        let libelles: Vec<_> = Family::ALL.iter().map(|f| f.label()).collect();
        assert_eq!(libelles, ["Fonds", "Formes", "Traits", "Motifs"]);
    }
}
