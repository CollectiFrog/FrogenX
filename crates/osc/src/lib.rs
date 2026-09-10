//! # frogenx-osc
//!
//! Les sources de modulation de FrogenX : oscillateurs, LFO, bruit cohérent,
//! enveloppes et opérateurs.
//!
//! Tous implémentent [`Signal`](frogenx_core::Signal) — donc tous sont
//! interchangeables, et n'importe lequel module n'importe quel
//! [`Param`](frogenx_core::Param). C'est l'invariant du §2.1 de
//! `docs/ARCHITECTURE.md`.
//!
//! ## Conventions (§17)
//!
//! - Fréquences en **Hz**, phases en **tours** `[0, 1)` — pas de `2π` disséminé.
//! - Sortie nominalement `[-1, 1]`, mais **non bornée** : une somme sort de
//!   l'intervalle, et c'est voulu. C'est au consommateur de saturer (§18.4).
//! - Aucun module ne lit l'horloge système ; le temps arrive par `ctx.dt`.
//!
//! ## Pourquoi la phase est accumulée
//!
//! Un oscillateur pourrait calculer `sin(2π·f·t)` directement. Mais dès que `f`
//! est modulée, cette formule fait **sauter** la phase à chaque changement —
//! discontinuité visible. L'accumulation garde la continuité, qui est
//! précisément ce qu'on veut en FM. C'est la raison d'être du `NodeState`.
//!
//! ## Exemple : un LFO module la fréquence d'un oscillateur
//!
//! ```
//! use frogenx_core::{Graph, Param, Signal};
//! use frogenx_osc::{Lfo, Oscillator, Waveform};
//!
//! let mut g = Graph::new();
//!
//! let lfo = g.add(Box::new(Lfo::new(Waveform::Sine, 0.5)));
//! let osc = g.add_with_deps(
//!     Box::new(Oscillator::sine(2.0).with_freq(Param::modulated(2.0, 1.5, lfo))),
//!     vec![lfo],
//! ).unwrap();
//!
//! for i in 0..60 {
//!     g.eval_frame(i as f64 / 60.0, 1.0 / 60.0);
//! }
//!
//! assert!(g.value(osc).is_finite());
//! assert!(g.diagnostics().is_clean());
//! ```

pub mod envelope;
pub mod noise;
pub mod oscillator;
pub mod waveform;

pub use envelope::{Envelope, SampleHold, Trigger};
pub use noise::{fbm_1d, value_noise_1d, Noise};
pub use oscillator::{Lfo, Mix, Oscillator, Product, Sum};
pub use waveform::{wrap01, Waveform};
