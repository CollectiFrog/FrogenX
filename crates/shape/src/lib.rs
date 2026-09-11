//! # frogenx-shape
//!
//! Les formes de FrogenX : primitives, générateurs paramétriques et opérateurs.
//!
//! Une forme est une **polyligne** — une suite de `Vec2` en coordonnées
//! normalisées, origine au centre, `[-1, 1]` sur le petit côté (§17). Un patch
//! rendu en 720p ou en 4K donne donc la même image.
//!
//! Cette crate ne connaît **ni wgpu ni l'audio** (§10) : elle est testable sans
//! GPU ni carte son.
//!
//! ## Les deux traits
//!
//! - [`ShapeGenerator`] produit une polyligne dans un buffer fourni.
//! - [`ShapeOp`] la transforme **en place**.
//!
//! Pourquoi deux traits plutôt qu'un opérateur qui envelopperait un générateur :
//! l'enveloppement exigerait un buffer intermédiaire par maillon de la chaîne,
//! soit exactement l'allocation par frame que le §16.3 cherche à éviter. Avec
//! un opérateur en place, une chaîne de dix opérateurs n'alloue rien.
//!
//! ## Où investir l'effort
//!
//! Ajouter un **opérateur** multiplie les possibilités de toutes les primitives
//! existantes. Ajouter une **primitive** n'ajoute qu'elle-même. En cas de doute,
//! écrire un opérateur.
//!
//! ## Exemple : un blob organique modulé
//!
//! ```
//! use frogenx_core::{AudioFeatures, Ctx, Graph, Param};
//! use frogenx_osc::Lfo;
//! use frogenx_shape::{Chain, Deform, Ellipse, ShapeGenerator};
//!
//! let mut g = Graph::new();
//! let lfo = g.add(Box::new(Lfo::new(frogenx_osc::Waveform::Sine, 0.2)));
//! g.eval_frame(0.0, 1.0 / 60.0);
//!
//! // Un cercle dont le contour ondule, la phase pilotée par un LFO.
//! let blob = Chain::new(Box::new(Ellipse::circle(0.8)))
//!     .then(Box::new(Deform::new(0.15, 5.0).with_phase(Param::modulated(0.0, 1.0, lfo))));
//!
//! let audio = AudioFeatures::default();
//! let ctx = Ctx::new(1.0 / 60.0, 0, &audio);
//! let mut points = Vec::new();
//! blob.generate(0.0, &ctx, &g, &mut points);
//!
//! assert_eq!(points.len(), 128);
//! ```

pub mod generator;
pub mod ops;
pub mod parametric;
pub mod primitives;

pub use generator::{ShapeGenerator, MAX_POINTS, SMOOTH_SEGMENTS};
pub use ops::{Chain, Deform, NoiseDisplace, RadialRepeat, ShapeOp, Smooth, Transform};
pub use parametric::{Lissajous, Rose, Superformula};
pub use primitives::{Arc, Ellipse, Polygon, Rect, Star};
