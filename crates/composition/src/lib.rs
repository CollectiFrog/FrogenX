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
//! Dépend de `core` et `shape` — pas seulement de `core` comme le sketch
//! initial le supposait. `Clip.effects` référence une chaîne `ShapeOp`
//! (§16.3 bis), dont le type vit dans `shape`. `shape` ne connaît ni GPU ni
//! carte son, donc cette dépendance ne casse pas la testabilité sans GPU qui
//! motive la règle de dépendance — elle déplace seulement la position de
//! `composition` dans la chaîne.
//!
//! ## Ce qui n'est pas encore représenté
//!
//! - `ClipSource::Scene3d`/`Video` (étapes 12-13, §12) : non ajoutés avant
//!   que `scene3d` et le décodage vidéo n'existent.
//! - Le rendu de `ClipTransition::Fade`/`Wipe` (étape 5, §12) : le type
//!   existe pour fixer le contrat, mais `composition` ne doit pas connaître
//!   le compositeur de couches qui en a besoin.

pub mod clip;
pub mod crossfader;
pub mod hierarchy;
pub mod ids;
pub mod position;
pub mod transport;

pub use clip::{Clip, ClipSource, ClipTransition, WipeDirection};
pub use crossfader::{CrossFader, FadeCurve};
pub use hierarchy::{Column, Composition, Deck, Layer, LayerGroup};
pub use ids::{ClipId, ColumnId, DeckId, LayerGroupId, LayerId};
pub use position::PositionSignal;
pub use transport::{PlayMode, TempoController, Transport};
