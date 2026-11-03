//! # frogenx-shape
//!
//! Les formes de FrogenX : primitives, générateurs paramétriques et opérateurs.
//!
//! Une forme est un [`Path`] — des `Vec2` en coordonnées normalisées, origine
//! au centre, `[-1, 1]` sur le petit côté (§17), découpés en **contours**
//! fermés ou ouverts. Un cercle est un contour fermé, un arc un contour ouvert,
//! une rosace répétée plusieurs contours fermés. Un patch rendu en 720p ou en
//! 4K donne la même image.
//!
//! Cette crate ne connaît **ni wgpu ni l'audio** (§10) : elle est testable sans
//! GPU ni carte son.
//!
//! ## Les deux traits
//!
//! - [`ShapeGenerator`] produit une forme dans un [`Path`] fourni.
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
//! use frogenx_shape::{Chain, Deform, Ellipse, Path, ShapeGenerator};
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
//! let mut forme = Path::new(); // réutilisé d'une frame à l'autre
//! blob.generate(0.0, &ctx, &g, &mut forme);
//!
//! assert_eq!(forme.points.len(), 128);
//! assert_eq!(forme.contours.len(), 1);
//! assert!(forme.contours[0].closed);
//! ```

pub mod crt;
pub mod feedback;
pub mod generator;
pub mod ops;
pub mod parametric;
pub mod path;
pub mod primitives;
pub mod spec;
pub mod style;
pub mod video;

pub use crt::{CrtParams, CrtStep, FilterKind, FilterStep, ImageFilter};
pub use feedback::{FeedbackParams, FeedbackStep};
pub use generator::{ParamDesc, ShapeGenerator, MAX_POINTS, SMOOTH_SEGMENTS};
pub use ops::{
    Chain, Dashes, Deform, GridRepeat, LinearWave, Mirror, NoiseDisplace, Quantize, RadialRepeat,
    ShapeEchoes, ShapeOp, Smooth, Transform, Trim, Twist,
};
pub use parametric::{Lissajous, Rose, Superformula};
pub use path::{Contour, Path};
pub use primitives::{Arc, Ellipse, Polygon, Rect, Star};
pub use spec::{generator_from_spec, op_from_spec, EffectKind};
pub use style::{
    srgb_to_linear, BlendMode, BlendParams, ColorParam, MixKind, ShapeStyle, StrokeStyle,
    COLOR_CHANNELS,
};
pub use video::{VideoParams, VideoStep};
