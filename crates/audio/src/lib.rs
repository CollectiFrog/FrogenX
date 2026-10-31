//! # frogenx-audio
//!
//! Du micro aux [`AudioFeatures`](frogenx_core::AudioFeatures) — §7 de
//! `docs/ARCHITECTURE.md`.
//!
//! ```text
//! [callback cpal] ──▶ rtrb ──▶ [Analysis : FFT] ──▶ triple buffer ──▶ [rendu]
//!    temps réel                                      lecture sans attente
//! ```
//!
//! **La carte son s'arrête à [`AudioInput`].** [`SpectrumAnalyzer`] est pur, et
//! [`Analysis`] ne connaît que l'anneau : tout se teste avec des sinusoïdes de
//! synthèse, sans matériel. Rien ici ne connaît la grille : `app` pose les
//! features dans le moteur, et le reste du système ne voit qu'un `Ctx`.

pub mod analysis;
pub mod analyzer;
pub mod capture;

pub use analysis::Analysis;
pub use analyzer::SpectrumAnalyzer;
pub use capture::{AudioError, AudioInput};
