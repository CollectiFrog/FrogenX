//! # frogenx-render
//!
//! Ce qui **consomme** les couches : le [`Compositor`], la relecture GPU →
//! CPU et l'écriture d'image — §8, §10, §22 de `docs/ARCHITECTURE.md`.
//!
//! Deux modules ne connaissent pas wgpu et se testent sans GPU :
//!
//! - [`blend`] : les formules de mélange en alpha prémultiplié, référence
//!   CPU du shader du compositeur ;
//! - [`convert`] : demi-flottants, linéaire → sRGB, dé-prémultiplication.
//!
//! Les tests de pixels (`tests/pixels.rs`) comparent le GPU à ces deux
//! modules, et se sautent proprement sans adaptateur (§24.3).
//!
//! ## Une frame
//!
//! ```text
//! graphes évalués (eval_frame) ─▶ Compositor::render
//!                                    ├─ prepare  sur chaque couche (CPU)
//!                                    ├─ encode   sur chaque couche (GPU)
//!                                    ├─ une passe de mélange par couche
//!                                    └─ un seul submit
//! ```

pub mod blend;
pub mod compositor;
pub mod convert;
pub mod image;
pub mod readback;

pub use blend::{blend_premul, saturate_opacity};
pub use compositor::{Compositor, LayerInput};
pub use convert::{f16_to_f32, linear_premul_to_srgb8, linear_to_srgb};
pub use image::write_png;
pub use readback::read_rgba16f;
