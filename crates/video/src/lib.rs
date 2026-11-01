//! # frogenx-video
//!
//! Le décodage des clips vidéo — §9.3 de `docs/ARCHITECTURE.md`.
//!
//! ```text
//! tampon de tête, décodé d'avance ──┐
//!                                   ├──▶ VideoPlayer::frame_at(t) ──▶ `layer`
//! flux ffmpeg, lancé au départ ─────┘      ne bloque jamais
//! ```
//!
//! **Ne connaît ni wgpu ni `Graph`.** Elle dit quelle image montrer à l'instant
//! `t`, en YUV 4:2:0, et rien d'autre. Le choix entre le tampon et le flux est une
//! fonction pure, [`plan`] ; le reste se teste sur de vraies vidéos fabriquées par
//! ffmpeg.

pub mod info;
pub mod player;
pub mod schedule;

pub use info::{decode_size, parse_probe, probe, VideoInfo};
pub use player::{Frame, Loader, VideoPlayer};
pub use schedule::{plan, Plan, StreamView, HEAD_SECONDS};
