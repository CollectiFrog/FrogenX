//! # frogenx-layer
//!
//! Les couches de rendu de FrogenX : le contexte [`Gpu`], le contrat
//! [`RenderLayer`] et sa première implémentation, [`ShapeLayer`] — §6, §22 de
//! `docs/ARCHITECTURE.md`.
//!
//! C'est la **première crate qui dépend du GPU** (§10). Elle garde pourtant
//! trois modules qui ne connaissent pas wgpu, testables en CI sans GPU :
//!
//! - [`tessellate`] : `Path` → triangles. Les cas limites de l'étape 5 se
//!   logent là — contour dégénéré, auto-intersection, contour ouvert.
//! - [`color`] : quatre `Param` saisis en sRGB, résolus en linéaire
//!   prémultiplié (§21).
//! - [`space`] : espace normalisé des formes → coordonnées GPU, avec la
//!   correction de proportion (§17).
//!
//! La relecture GPU → CPU et le compositeur vivent dans `frogenx-render`, qui
//! **consomme** les couches. Les tests de pixels d'une `ShapeLayer` y sont
//! donc aussi.
//!
//! ## Pourquoi `RenderLayer` et non `Layer`
//!
//! `composition::Layer` désigne déjà une piste de la grille. Le trait de cette
//! crate désigne un objet qui produit une texture ; les deux coexisteront dans
//! `render` et `ui` (§6).

pub mod color;
pub mod gpu;
pub mod render_layer;
pub mod shape_layer;
pub mod space;
pub mod tessellate;

pub use color::{srgb_to_linear, ColorParam};
pub use gpu::{Gpu, LAYER_FORMAT};
pub use render_layer::{BlendMode, BlendParams, RenderDiagnostics, RenderLayer};
pub use shape_layer::{ShapeLayer, StrokeStyle};
pub use space::normalized_to_ndc;
pub use tessellate::{Mesh, Tessellator};
