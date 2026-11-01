//! Le tempo d'une vraie forme sonore : des boucles de batterie de synthèse, à
//! travers toute l'analyse — §7.1 de `docs/ARCHITECTURE.md`.

use frogenx_audio::FeatureAnalyzer;
use frogenx_core::AudioFeatures;
use std::f32::consts::TAU;

/// Une grosse caisse : une sinusoïde qui descend et s'éteint en 150 ms.
fn kick(age: f32) -> f32 {
    if age > 0.15 {
        return 0.0;
    }
    let hz = 60.0 + 80.0 * (-age * 40.0).exp();
    (TAU * hz * age).sin() * (-age * 18.0).exp() * 0.8
}

/// Une boucle : grosse caisse sur chaque temps, charleston à contretemps.
fn boucle(bpm: f32, sr: f32, secondes: f32) -> Vec<f32> {
    let battement = 60.0 / bpm;
    let mut bruit = 12345u32;
    (0..(secondes * sr) as usize)
        .map(|i| {
            let t = i as f32 / sr;
            let caisse = kick(t % battement);
            let charleston = (t + battement * 0.5) % battement;
            bruit = bruit.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let b = (bruit >> 16) as f32 / 32_768.0 - 1.0;
            caisse + b * 0.15 * (-charleston * 60.0).exp()
        })
        .collect()
}

fn analyse(samples: &[f32], sr: f32) -> AudioFeatures {
    let mut a = FeatureAnalyzer::new(sr);
    for s in samples {
        a.push(*s);
    }
    let mut f = AudioFeatures::default();
    a.write_to(&mut f);
    f
}

#[test]
fn une_boucle_de_batterie_donne_son_tempo() {
    for sr in [44_100.0, 48_000.0] {
        for bpm in [90.0, 100.0, 110.0, 120.0, 128.0, 140.0, 150.0] {
            let f = analyse(&boucle(bpm, sr, 14.0), sr);
            assert!(
                (f.bpm - bpm).abs() < 1.0,
                "{bpm} à {sr} Hz : {} (fiabilité {})",
                f.bpm,
                f.bpm_confidence
            );
            assert!(f.bpm_confidence > 0.3, "{bpm} : {}", f.bpm_confidence);
        }
    }
}

#[test]
fn un_son_tenu_n_a_pas_de_tempo() {
    let sr = 44_100.0;
    let tenu: Vec<f32> = (0..(14.0 * sr) as usize)
        .map(|i| 0.5 * (TAU * 220.0 * i as f32 / sr).sin())
        .collect();
    assert_eq!(analyse(&tenu, sr).bpm, 0.0);
}

#[test]
fn un_accord_tenu_n_a_pas_de_tempo() {
    let sr = 44_100.0;
    let notes = [220.0, 277.18, 329.63, 440.0, 554.37];
    let accord: Vec<f32> = (0..(14.0 * sr) as usize)
        .map(|i| {
            let t = i as f32 / sr;
            notes.iter().map(|hz| (TAU * hz * t).sin()).sum::<f32>() * 0.15
        })
        .collect();
    assert_eq!(analyse(&accord, sr).bpm, 0.0);
}

#[test]
fn une_boucle_discrete_donne_encore_son_tempo() {
    // 30 dB plus bas : un micro d'ambiance, pas une entrée de table de mixage.
    let sr = 44_100.0;
    let discret: Vec<f32> = boucle(128.0, sr, 14.0).iter().map(|s| s * 0.03).collect();
    let f = analyse(&discret, sr);
    assert!((f.bpm - 128.0).abs() < 1.0, "{}", f.bpm);
}

#[test]
fn chaque_coup_passe_dans_la_feature_de_transitoires() {
    let sr = 44_100.0;
    let bpm = 120.0;
    let son = boucle(bpm, sr, 8.0);
    let mut a = FeatureAnalyzer::new(sr);
    let mut f = AudioFeatures::default();
    let (mut coups, mut avant) = (0, 0.0);
    for s in &son {
        if a.push(*s) {
            a.write_to(&mut f);
            if f.onset > avant + 0.5 {
                coups += 1;
            }
            avant = f.onset;
        }
    }
    // 16 temps et leurs contretemps : au moins chaque temps, jamais une rafale.
    assert!((14..=34).contains(&coups), "{coups} coups");
}

/// La phase d'une boucle dont on connaît les temps : le premier tombe à zéro.
#[test]
fn une_boucle_de_batterie_donne_la_place_de_son_temps() {
    for sr in [44_100.0, 48_000.0] {
        for bpm in [100.0, 120.0, 128.0, 140.0] {
            let secondes = 13.37;
            let f = analyse(&boucle(bpm, sr, secondes), sr);
            let battement = 60.0 / bpm;
            let vraie = (secondes % battement) / battement;
            let ecart = (f.beat_phase - vraie + 0.5).rem_euclid(1.0) - 0.5;
            let ms = ecart * battement * 1000.0;
            assert!(ms.abs() < 15.0, "{sr} Hz, {bpm} BPM : {ms:.1} ms");
            assert!(f.beat_confidence > 0.3, "{}", f.beat_confidence);
        }
    }
}

/// Reproductible : la même entrée donne les mêmes valeurs, au bit près (§20.3).
#[test]
fn la_meme_entree_donne_le_meme_tempo() {
    let sr = 44_100.0;
    let son = boucle(128.0, sr, 10.0);
    let (a, b) = (analyse(&son, sr), analyse(&son, sr));
    assert_eq!(a.bpm, b.bpm);
    assert_eq!(a.bands, b.bands);
    assert_eq!(a.chroma, b.chroma);
}
