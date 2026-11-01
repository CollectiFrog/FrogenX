//! # frogenx-composition
//!
//! La grille de composition de FrogenX : `Composition`, `Deck`, `LayerGroup`,
//! `Layer`, `Column`, `Clip`, `CrossFader`, `TempoController` — §3bis de
//! `docs/ARCHITECTURE.md`.
//!
//! **Décision structurante :** FrogenX organise ce qui joue et quand autour
//! d'une grille de clips déclenchables, plutôt que d'exposer un unique patch
//! modulaire comme surface de premier niveau. Le graphe `Signal`/`Param` de
//! `frogenx-core` ne disparaît pas : il devient le moteur de pilotage interne
//! de chaque clip, layer et effet. La grille répond à « quoi joue, quand, sur
//! quelle couche » ; le graphe répond à « comment ce qui joue évolue dans le
//! temps ».
//!
//! ## Dépendances (§10)
//!
//! Dépend de `core`, `osc` (les nœuds d'un `Trig`, §3bis.6) et `shape` — pas
//! seulement de `core` comme le sketch
//! initial le supposait. `Clip.effects` référence une chaîne `ShapeOp`
//! (§16.3 bis), dont le type vit dans `shape`. `shape` ne connaît ni GPU ni
//! carte son, donc cette dépendance ne casse pas la testabilité sans GPU qui
//! motive la règle de dépendance — elle déplace seulement la position de
//! `composition` dans la chaîne.
//!
//! ## Ce que `composition` ne fait pas
//!
//! Elle ne dessine rien et ne connaît pas wgpu. `frogenx-app` lit le clip actif
//! de chaque piste et le confie au renderer de la piste (§3bis.2).
//!
//! ## Ce qui n'est pas encore représenté
//!
//! - `ClipSource::Scene3d` (étape 13, §12) : non ajouté avant que `scene3d`
//!   n'existe.

pub mod analysis;
pub mod beat;
pub mod clip;
pub mod command;
pub mod continuity;
pub mod crossfader;
pub mod hierarchy;
pub mod ids;
pub mod position;
pub mod signals;
pub mod transport;
pub mod trig;
pub mod wiring;

pub use analysis::AudioAnalysis;
pub use beat::{BeatGrid, Quantize};
pub use clip::{
    sanitize_speed, Clip, ClipParam, ClipSource, ClipTransition, ParamGroup, ParamSlot,
    TransitionKind, WipeDirection, DEFAULT_TRANSITION_BEATS, MAX_CLIP_SPEED,
};
pub use command::Command;
pub use continuity::IdMap;
pub use crossfader::{CrossFader, FadeCurve};
pub use hierarchy::{
    Column, Composition, Deck, Layer, LayerGroup, TransitionView, MIN_TEMPO_CONFIDENCE,
};
pub use ids::{ClipId, ColumnId, DeckId, LayerGroupId, LayerId, TrigId};
pub use position::{BeatRate, PositionSignal, VideoRate};
pub use signals::signal_from_spec;
pub use transport::{sanitize_bpm, PlayMode, TempoController, Transport, MAX_BPM, MIN_BPM};
pub use trig::{RestoredTrig, Trig, TrigSlot, TrigTarget};
pub use wiring::{GraphScope, NodeKind, NodeValue, ParamTarget};
