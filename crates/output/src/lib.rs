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
//!                              ├─ passe 8 bits, une par format attendu
//!                              ├─ queue.submit
//!                              ├─ GpuSink::after_submit
//!                              └─ relecture, puis une CpuFrame par file
//! ```

pub mod conversion;
pub mod fanout;
mod live;
pub mod monitor;
pub mod outputs;
pub mod png_sequence;
pub mod preview;
pub mod record;
pub mod sink;
#[cfg(windows)]
pub mod spout;
pub mod stream;

pub use conversion::{ConversionMode, ConversionPass, Rect};
pub use fanout::{CpuFanout, CpuSinkId, DropPolicy, FrameKey, Group, QUEUE_DEPTH};
pub use live::READBACK_SLOTS;
pub use monitor::{MonitorSink, MONITOR_FORMAT};
pub use outputs::{GpuSinkId, OutputError, Outputs};
pub use png_sequence::PngSequenceSink;
pub use preview::{
    letterbox, pick_format, pick_present_mode, FormatPreference, PresentSurface, Preview,
    PreviewHandle,
};
pub use record::{
    assemble, audio_part, frames_due, mux_args, next_take, probe_encoders, video_args, video_part,
    Encoders, FfmpegSink, RecordHandles, RecordPreset, VideoEncoder,
};
pub use sink::{CpuFrame, CpuSink, GpuFrame, GpuSink, OutputSink, PixelLayout, SinkStatus};
pub use stream::{Cadence, StreamFormat};
