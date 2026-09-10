//! # frogenx-core
//!
//! Le cœur de FrogenX : `Signal`, `Param`, `Graph`, `Clock`.
//!
//! **Cette crate ne dépend de rien** — ni GPU, ni carte son, ni fenêtre. C'est
//! la règle de dépendance du §10 de `docs/ARCHITECTURE.md`, et ce n'est pas une
//! élégance : c'est la condition de la testabilité du projet.
//!
//! ## Les invariants
//!
//! - **Tout est un [`Signal`]** — oscillateur, LFO, enveloppe, feature audio :
//!   tous interchangeables, donc n'importe quelle source module n'importe
//!   quelle destination.
//! - **Tout paramètre réglable est un [`Param`], jamais un `f32` nu.** Un `f32`
//!   glissé aujourd'hui est un câble impossible à brancher demain.
//! - **L'état vit dans le graphe** ([`NodeState`]), pas dans les modules.
//! - **Jamais de panique en évaluation** : erreurs en édition seulement ; en
//!   frame, valeur de repli et compteur [`Diagnostics`].
//! - **Le temps est toujours reçu en paramètre** — aucun module ne lit
//!   l'horloge système.
//!
//! ## Exemple
//!
//! ```
//! use frogenx_core::{Clock, Graph, Param, Constant};
//!
//! let mut clock = Clock::fixed(60.0);
//! let mut graph = Graph::new();
//!
//! let lfo = graph.add(Box::new(Constant(0.5)));
//! let rayon = Param::modulated(1.0, 0.4, lfo); // 1.0 + 0.4 * 0.5
//!
//! clock.tick(0.0);
//! graph.eval_frame(clock.t(), clock.dt());
//!
//! assert!((rayon.get(&graph) - 1.2).abs() < 1e-6);
//! assert!(graph.diagnostics().is_clean());
//! ```

pub mod clock;
pub mod graph;
pub mod param;
pub mod signal;

pub use clock::{Clock, ClockMode};
pub use graph::{Graph, GraphError};
pub use param::{NodeId, Param};
pub use signal::{AudioFeatures, Constant, Ctx, Diagnostics, NodeState, Signal};

/// Deux fois pi — la conversion tours → radians (§17 : la phase est en tours).
pub const TAU: f32 = std::f32::consts::TAU;
