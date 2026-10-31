//! Les fichiers d'hier s'ouvrent aujourd'hui — §19.2 de `docs/ARCHITECTURE.md`.
//!
//! **Les fixtures ne sont jamais régénérées** : leur valeur est d'être anciennes.
//! Si un test échoue ici, le format a changé : il faut une nouvelle version, sa
//! migration et sa fixture, pas une fixture réécrite.

use frogenx_composition::{Composition, TrigTarget};
use frogenx_patch::{from_json, to_json, CURRENT_VERSION};
use serde_json::Value;

const V1: &str = include_str!("fixtures/patch_v1.json");

fn pistes(c: &Composition) -> Vec<String> {
    let mut ids = Vec::new();
    c.layers_in_order(&mut ids);
    ids.iter()
        .filter_map(|id| c.layer(*id))
        .map(|l| l.name.clone())
        .collect()
}

#[test]
fn la_fixture_v1_s_ouvre() {
    let c = from_json(V1).expect("un fichier v1 s'ouvre");
    assert_eq!(c.columns().len(), 8);
    assert_eq!(pistes(&c), ["Fond", "Formes", "Traits"]);
    assert_eq!(c.tempo.tempo.base(), 128.0);

    let trig = c.trig(c.trigs()[0]).expect("un trig");
    assert_eq!(trig.name, "Grosse caisse");
    assert_eq!(trig.every().base(), 4.0);
    assert_eq!(trig.target, TrigTarget::Column(c.columns()[2]));
}

#[test]
fn la_fixture_v1_garde_ses_clips_actifs() {
    let c = from_json(V1).unwrap();
    let mut ids = Vec::new();
    c.layers_in_order(&mut ids);
    for id in ids {
        assert!(c.layer(id).unwrap().active_clip().is_some());
    }
}

/// Tant que `CURRENT_VERSION` vaut 1, relire puis réécrire la fixture rend le
/// même contenu. Sinon, le format a changé sans changer de version.
#[test]
fn le_format_n_a_pas_change_sans_changer_de_version() {
    if CURRENT_VERSION != 1 {
        return;
    }
    let sans_meta = |texte: &str| {
        let mut v: Value = serde_json::from_str(texte).unwrap();
        v.as_object_mut().unwrap().remove("meta");
        v
    };
    let reecrit = to_json(&from_json(V1).unwrap()).unwrap();
    assert_eq!(sans_meta(&reecrit), sans_meta(V1));
}
