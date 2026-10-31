//! La table des `Signal` — §19.1 de `docs/ARCHITECTURE.md`.
//!
//! Un `Signal` se reconstruit par le `from_spec` de son type. Le compilateur
//! impose `Signal::spec` à tout nouveau type, pas son entrée ici : le test du
//! parcours complet du catalogue, et celui de cette table, le rappellent.

use frogenx_composition::{BeatRate, PositionSignal};
use frogenx_core::{Constant, Signal, Spec, SpecError};
use frogenx_osc::{
    AudioFeature, Divider, Envelope, Lfo, Mix, Noise, Oscillator, Product, SampleHold,
    SpectrumZone, Sum, Trigger,
};

/// Le `Signal` que décrit `spec`.
pub fn signal_from_spec(spec: &Spec) -> Result<Box<dyn Signal>, SpecError> {
    Ok(match spec.kind.as_str() {
        "Constant" => Box::new(Constant::from_spec(spec)?),
        "Oscillator" => Box::new(Oscillator::from_spec(spec)?),
        "Lfo" => Box::new(Lfo::from_spec(spec)?),
        "Sum" => Box::new(Sum::from_spec(spec)?),
        "Product" => Box::new(Product::from_spec(spec)?),
        "Mix" => Box::new(Mix::from_spec(spec)?),
        "Noise" => Box::new(Noise::from_spec(spec)?),
        "Envelope" => Box::new(Envelope::from_spec(spec)?),
        "Trigger" => Box::new(Trigger::from_spec(spec)?),
        "SampleHold" => Box::new(SampleHold::from_spec(spec)?),
        "SpectrumZone" => Box::new(SpectrumZone::from_spec(spec)?),
        "Divider" => Box::new(Divider::from_spec(spec)?),
        "AudioFeature" => Box::new(AudioFeature::from_spec(spec)?),
        "BeatRate" => Box::new(BeatRate::from_spec(spec)?),
        "PositionSignal" => Box::new(PositionSignal::from_spec(spec)?),
        autre => return Err(SpecError::UnknownKind(autre.to_string())),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use frogenx_core::{FeatureKind, Param};
    use frogenx_osc::Waveform;

    /// Un `Signal` de chaque type, ses champs décalés de leur défaut.
    fn signaux() -> Vec<Box<dyn Signal>> {
        let p = |v| Param::fixed(v);
        vec![
            Box::new(Constant(0.25)),
            Box::new(
                Oscillator::new(Waveform::Saw, 2.0)
                    .with_amp(p(0.5))
                    .with_phase_offset(p(0.125)),
            ),
            Box::new(Lfo::new(Waveform::Square, 0.5).unipolar()),
            Box::new(Sum(vec![p(1.0), p(2.0)])),
            Box::new(Product(vec![p(0.5), p(0.25)])),
            Box::new(Mix {
                a: p(0.1),
                b: p(0.2),
                mix: p(0.3),
            }),
            Box::new(Noise::new(0.5, 42).fractal(3, 0.25)),
            Box::new(Envelope::new(p(1.0), 0.25, 0.5)),
            Box::new(Trigger::new(p(1.0), 0.75)),
            Box::new(SampleHold {
                input: p(0.5),
                trigger: p(1.0),
            }),
            Box::new(SpectrumZone::new((40.0, 90.0), (0.25, 0.75))),
            Box::new(Divider::new(p(1.0), 8.0)),
            Box::new(AudioFeature {
                attack: p(0.25),
                release: p(0.5),
                ..AudioFeature::new(FeatureKind::Chroma(9))
            }),
            Box::new(BeatRate::new(4.0)),
            Box::new(PositionSignal::new(p(1.5))),
        ]
    }

    #[test]
    fn chaque_signal_se_reconstruit_a_l_identique() {
        for signal in signaux() {
            let spec = signal.spec();
            let copie = signal_from_spec(&spec).expect(&spec.kind);
            assert_eq!(copie.spec(), spec);
            assert_eq!(copie.state_size(), signal.state_size(), "{}", spec.kind);
        }
    }

    #[test]
    fn la_table_couvre_tous_les_signaux_de_la_liste() {
        let mut noms: Vec<_> = signaux().iter().map(|s| s.spec().kind).collect();
        noms.sort();
        noms.dedup();
        assert_eq!(noms.len(), 15, "un type par ligne");
    }

    #[test]
    fn un_signal_inconnu_est_refuse_en_le_nommant() {
        let erreur = signal_from_spec(&Spec::new("Fantome")).err();
        assert_eq!(erreur, Some(SpecError::UnknownKind("Fantome".into())));
    }

    #[test]
    fn une_forme_d_onde_inconnue_est_refusee() {
        let spec = Lfo::new(Waveform::Sine, 1.0).spec();
        let mut spec = spec;
        spec.fields[0].1 = "bizarre".into();
        assert!(signal_from_spec(&spec).is_err());
    }
}
