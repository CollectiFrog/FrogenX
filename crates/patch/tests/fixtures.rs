//! Les fichiers d'hier s'ouvrent aujourd'hui — §19.2 de `docs/ARCHITECTURE.md`.
//!
//! **Les fixtures ne sont jamais régénérées** : leur valeur est d'être anciennes.
//! Si un test échoue ici, le format a changé : il faut une nouvelle version, sa
//! migration et sa fixture, pas une fixture réécrite.

use frogenx_composition::{ClipSource, Composition, PlayMode, Quantize, TrigTarget};
use frogenx_core::FeatureKind;
use frogenx_patch::{from_json, to_json, CURRENT_VERSION};
use serde_json::Value;

const V1: &str = include_str!("fixtures/patch_v1.json");
const V2: &str = include_str!("fixtures/patch_v2.json");
const V3: &str = include_str!("fixtures/patch_v3.json");
const V4: &str = include_str!("fixtures/patch_v4.json");
const V5: &str = include_str!("fixtures/patch_v5.json");
const V6: &str = include_str!("fixtures/patch_v6.json");
const V7: &str = include_str!("fixtures/patch_v7.json");

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

#[test]
fn la_fixture_v1_se_migre_sans_suivre_le_tempo() {
    assert!(!from_json(V1).unwrap().tempo_follow);
}

#[test]
fn la_fixture_v2_garde_le_suivi_et_sa_source_audio() {
    let c = from_json(V2).expect("un fichier v2 s'ouvre");
    assert!(c.tempo_follow);
    let mut ids = Vec::new();
    c.layers_in_order(&mut ids);
    let formes = c.layer(ids[1]).unwrap();
    let clip = c.clip(formes.cell(0).unwrap()).unwrap();
    assert_eq!(clip.params()[0].params[0].audio, Some(FeatureKind::Bass));
}

#[test]
fn la_fixture_v3_garde_son_echo() {
    let c = from_json(V3).expect("un fichier v3 s'ouvre");
    let mut ids = Vec::new();
    c.layers_in_order(&mut ids);
    let fond = c.layer(ids[0]).unwrap();
    let echo = c.clip(fond.cell(1).unwrap()).unwrap();
    assert!(matches!(echo.source, ClipSource::Feedback { .. }));
    assert_eq!(echo.params()[0].params[4].desc.value.base(), 0.75);
}

#[test]
fn un_fichier_v3_donne_a_ses_trigs_un_canal_par_rang() {
    let c = from_json(V3).expect("un fichier v3 s'ouvre");
    assert_eq!(c.trig(c.trigs()[0]).unwrap().channel(), 0);
}

#[test]
fn la_fixture_v4_garde_le_canal_et_le_trig_d_un_clip() {
    let c = from_json(V4).expect("un fichier v4 s'ouvre");
    assert_eq!(c.trig(c.trigs()[0]).unwrap().channel(), 0);
    let mut ids = Vec::new();
    c.layers_in_order(&mut ids);
    let formes = c.layer(ids[1]).unwrap();
    let clip = c.clip(formes.cell(0).unwrap()).unwrap();
    assert_eq!(clip.params()[0].params[0].trig, Some(0));
}

#[test]
fn un_fichier_v4_lance_au_clic() {
    assert_eq!(from_json(V4).unwrap().quantize, Quantize::Off);
}

#[test]
fn la_fixture_v5_garde_sa_quantification() {
    assert_eq!(from_json(V5).unwrap().quantize, Quantize::Bar);
}

#[test]
fn une_quantification_inconnue_est_refusee() {
    let texte = V5.replace("\"quantize\": \"bar\"", "\"quantize\": \"demi\"");
    assert!(from_json(&texte).is_err());
}

#[test]
fn les_noeuds_d_un_fichier_v5_restent_places_d_office() {
    let c = from_json(V5).unwrap();
    assert!(c.graph().nodes().iter().all(|n| n.place.is_none()));
}

#[test]
fn la_fixture_v6_garde_la_place_d_un_noeud() {
    let c = from_json(V6).unwrap();
    assert_eq!(c.graph().nodes()[0].place, Some([320.0, 48.0]));
}

#[test]
fn la_fixture_v7_garde_sa_video_son_mode_et_son_calage() {
    let c = from_json(V7).unwrap();
    let (_, params) = c.video_clips().next().expect("une vidéo");
    assert_eq!(params.path, "D:/prises/concert.mp4");
    assert_eq!(params.duration, 12.0);
    let id = c.video_clips().next().unwrap().0;
    let (_, mode, beats) = c.clip(id).unwrap().video_playback().unwrap();
    assert_eq!(mode, PlayMode::PingPong);
    assert_eq!(beats, 16.0);
}

/// Tant que `CURRENT_VERSION` vaut 7, relire puis réécrire la fixture rend le
/// même contenu. Sinon, le format a changé sans changer de version.
#[test]
fn le_format_n_a_pas_change_sans_changer_de_version() {
    if CURRENT_VERSION != 7 {
        return;
    }
    let sans_meta = |texte: &str| {
        let mut v: Value = serde_json::from_str(texte).unwrap();
        v.as_object_mut().unwrap().remove("meta");
        v
    };
    let reecrit = to_json(&from_json(V7).unwrap()).unwrap();
    assert_eq!(sans_meta(&reecrit), sans_meta(V7));
}
