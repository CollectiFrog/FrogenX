//! # frogenx-output
//!
//! Ce qui **quitte** le moteur : le contrat des sorties, l'étage [`Outputs`]
//! et la passe de conversion vers 8 bits sRGB — §8, §20.1 de
//! `docs/ARCHITECTURE.md`.
//!
//! Le rendu ne va jamais directement à l'écran : il va dans une texture
//! offscreen, que plusieurs sorties consomment. Deux familles, deux traits :
//! [`GpuSink`] reste sur la carte, [`CpuSink`] reçoit des octets.
//!
//! Le module [`fanout`] ne connaît pas wgpu : la politique de perte s'y teste
//! sans GPU. Cette crate ne connaît pas `winit` : c'est `app` qui possède la
//! fenêtre, et [`Preview`] ne reçoit qu'une surface (§8.3).
//!
//! ## L'étape 9 d'une frame
//!
//! ```text
//! texture du compositeur ─▶ Outputs::submit
//!                              ├─ GpuSink::encode
//!                              ├─ passe 8 bits (si une sortie CPU est active)
//!                              ├─ queue.submit
//!                              ├─ GpuSink::after_submit
//!                              └─ relecture, puis une CpuFrame par file
//! ```

pub mod conversion;
pub mod fanout;
pub mod outputs;
pub mod png_sequence;
pub mod preview;
pub mod sink;

pub use conversion::{ConversionMode, ConversionPass, Rect};
pub use fanout::{CpuFanout, DropPolicy, QUEUE_DEPTH};
pub use outputs::{OutputError, Outputs};
pub use png_sequence::PngSequenceSink;
pub use preview::{letterbox, pick_format, pick_present_mode, Preview, PreviewHandle};
pub use sink::{CpuFrame, CpuSink, GpuFrame, GpuSink, OutputSink, SinkStatus};
