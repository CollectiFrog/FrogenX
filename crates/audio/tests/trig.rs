//! Du son au trig, sans carte son — §3bis.6 de `docs/ARCHITECTURE.md`.
//!
//! Une boucle de batterie de synthèse traverse la vraie FFT, puis la vraie
//! zone et le vrai compteur, frame par frame, comme dans le moteur. C'est ce qui
//! vérifie que les constantes de l'analyse tiennent sur autre chose qu'une
//! sinusoïde : un coup doit rester un seul front.

use frogenx_audio::SpectrumAnalyzer;
use frogenx_core::{AudioFeatures, Graph, NodeId, Param};
use frogenx_osc::{Divider, SpectrumZone};
use std::f32::consts::TAU;

const SR: f32 = 48_000.0;
/// Échantillons par frame, à 60 fps.
const FRAME: usize = 800;
/// À 120 BPM, deux coups par seconde.
const BEAT: usize = 24_000;

/// Une grosse caisse : un balayage de 120 à 50 Hz qui s'éteint.
fn kick(i: usize) -> f32 {
    let t = i as f32 / SR;
    let phase = 50.0 * t + 70.0 * 0.03 * (1.0 - (-t / 0.03).exp());
    0.8 * (-t / 0.09).exp() * (TAU * phase).sin()
}

/// Un charleston : un bruit bref, à graine fixe (§20.3).
fn hat(i: usize, bruit: &mut u32) -> f32 {
    *bruit = bruit.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
    let blanc = (*bruit >> 16) as f32 / 32_768.0 - 1.0;
    0.3 * (-(i as f32 / SR) / 0.015).exp() * blanc
}

/// Une grosse caisse sur chaque temps, un charleston sur chaque contretemps.
fn boucle(temps: usize) -> Vec<f32> {
    let mut bruit = 1;
    let mut filtre = 0.0;
    (0..temps * BEAT)
        .map(|i| {
            let dans_le_temps = i % BEAT;
            let brut = hat((i + BEAT / 2) % BEAT, &mut bruit);
            // Une différence premier ordre pousse le bruit vers les aigus.
            let aigu = brut - filtre;
            filtre = brut;
            kick(dans_le_temps) + aigu
        })
        .collect()
}

/// Rend les frames où chaque nœud vaut 1, comptées en fronts montants.
fn fronts(samples: &[f32], g: &mut Graph, noeud: NodeId) -> u32 {
    let mut analyzer = SpectrumAnalyzer::new(SR);
    let (mut total, mut precedent) = (0, false);
    for bloc in samples.chunks(FRAME) {
        for s in bloc {
            analyzer.push(*s);
        }
        g.set_audio(AudioFeatures {
            spectrum: *analyzer.spectrum(),
            ..Default::default()
        });
        g.eval_frame(0.0, 1.0 / 60.0);
        let haut = g.value(noeud) > 0.5;
        total += (haut && !precedent) as u32;
        precedent = haut;
    }
    total
}

fn zone_grave() -> SpectrumZone {
    SpectrumZone::new((30.0, 120.0), (0.7, 1.0))
}

#[test]
fn chaque_coup_de_grosse_caisse_donne_un_seul_front() {
    let mut g = Graph::new();
    let porte = g.add(Box::new(zone_grave()));
    assert_eq!(fronts(&boucle(16), &mut g, porte), 16);
}

#[test]
fn le_compteur_laisse_passer_un_coup_sur_quatre_de_la_vraie_boucle() {
    for (every, attendus) in [(4.0, 4), (8.0, 2), (16.0, 1)] {
        let mut g = Graph::new();
        let porte = g.add(Box::new(zone_grave()));
        let compteur = g
            .add_with_deps(
                Box::new(Divider::new(Param::modulated(0.0, 1.0, porte), every)),
                vec![porte],
            )
            .unwrap();
        assert_eq!(fronts(&boucle(16), &mut g, compteur), attendus);
    }
}

#[test]
fn une_zone_d_aigus_suit_le_charleston_et_pas_la_grosse_caisse() {
    let mut g = Graph::new();
    // Le pic d'un bruit varie d'un coup à l'autre, ici de 0.49 à 0.55 : une zone
    // se pose sous ces fluctuations, pas dessus.
    let porte = g.add(Box::new(SpectrumZone::new((6000.0, 16_000.0), (0.4, 1.0))));
    assert_eq!(fronts(&boucle(16), &mut g, porte), 16);
}

#[test]
fn une_zone_au_dessus_du_signal_ne_s_ouvre_jamais() {
    let mut g = Graph::new();
    let porte = g.add(Box::new(SpectrumZone::new((30.0, 120.0), (0.995, 1.0))));
    assert_eq!(fronts(&boucle(8), &mut g, porte), 0);
}
