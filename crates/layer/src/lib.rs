//! # frogenx-layer
//!
//! Les couches de rendu de FrogenX : le contexte [`Gpu`], le contrat
//! [`RenderLayer`] et sa première implémentation, [`ShapeLayer`] — §6, §22 de
//! `docs/ARCHITECTURE.md`.
//!
//! C'est la **première crate qui dépend du GPU** (§10). Elle garde pourtant
//! deux modules qui ne connaissent pas wgpu, testables en CI sans GPU :
//!
//! - [`tessellate`] : `Path` → triangles. Les cas limites de l'étape 5 se
//!   logent là — contour dégénéré, auto-intersection, contour ouvert.
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

pub mod feedback;
pub mod gpu;
pub mod render_layer;
pub mod shape_layer;
pub mod space;
pub mod tessellate;
pub mod transition;

pub use feedback::{FeedbackRenderer, FrameHistory};
pub use frogenx_shape::{
    srgb_to_linear, BlendMode, BlendParams, ColorParam, FeedbackParams, FeedbackStep, MixKind,
    ShapeStyle, StrokeStyle,
};
pub use gpu::{Gpu, LAYER_FORMAT};
pub use render_layer::{RenderDiagnostics, RenderLayer};
pub use shape_layer::{ShapeContent, ShapeLayer, ShapeRenderer};
pub use space::normalized_to_ndc;
pub use tessellate::{Mesh, Tessellator};
pub use transition::{mix_coverage, MixRenderer};
