//! La forme du fichier — §19.1 de `docs/ARCHITECTURE.md`.
//!
//! Ces types ne servent qu'à lire et écrire : ils ne contiennent aucun
//! identifiant d'arena, seulement des positions dans le fichier.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Serialize, Deserialize)]
pub struct FilePatch {
    pub version: u64,
    pub meta: FileMeta,
    pub tempo: FileParam,
    /// Le tempo suit le BPM détecté (§7.1).
    pub tempo_follow: bool,
    pub analysis: FileAnalysis,
    pub columns: usize,
    pub active_deck: Option<usize>,
    /// Le graphe de composition, trigs compris.
    pub graph: Vec<FileNode>,
    pub trigs: Vec<FileTrig>,
    pub decks: Vec<FileDeck>,
    /// Toujours `null` : un patch décrit une configuration, pas un instant.
    pub state: Option<serde_json::Value>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FileMeta {
    pub app: String,
}

/// Une zone de trig n'a de sens que relue avec l'analyse qui l'a fait poser.
#[derive(Debug, Serialize, Deserialize)]
pub struct FileAnalysis {
    pub method: String,
    pub freq_scale: String,
    pub amp_scale: String,
    /// La durée analysée, en secondes. Se résout dans le graphe de composition.
    pub window: FileParam,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileParam {
    Fixed(f32),
    Modulated { base: f32, depth: f32, source: u32 },
}

/// JSON ne distingue pas les listes vides : `Params` les prend toutes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FileValue {
    Bool(bool),
    Int(u32),
    Number(f32),
    Word(String),
    Param(FileParam),
    Params(Vec<FileParam>),
    Specs(Vec<FileSpec>),
}

/// Une forme ou un effet.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileSpec {
    pub kind: String,
    #[serde(default)]
    pub fields: BTreeMap<String, FileValue>,
}

/// Un nœud : ses dépendances sont des positions dans le même tableau.
#[derive(Debug, Serialize, Deserialize)]
pub struct FileNode {
    pub kind: String,
    #[serde(default)]
    pub deps: Vec<u32>,
    #[serde(default)]
    pub fields: BTreeMap<String, FileValue>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileTarget {
    None,
    /// La piste, comptée sur toute la grille, et la colonne de sa case.
    Clip {
        layer: usize,
        column: usize,
    },
    Column(usize),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FileTrig {
    pub name: String,
    pub target: FileTarget,
    pub gate: u32,
    pub pulse: u32,
    pub flash: u32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FileDeck {
    pub name: String,
    pub groups: Vec<FileGroup>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FileGroup {
    pub layers: Vec<FileLayer>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FileLayer {
    pub name: String,
    pub blend: FileBlend,
    /// La colonne du clip qui joue.
    pub active: Option<usize>,
    pub cells: Vec<Option<FileClip>>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FileBlend {
    pub opacity: FileParam,
    pub mode: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FileClip {
    pub name: String,
    /// Se résout dans le graphe de composition (§3bis.2).
    pub speed: FileParam,
    pub graph: Vec<FileNode>,
    pub generator: FileSpec,
    pub effects: Vec<FileSpec>,
    pub style: FileStyle,
    pub transport: FileTransport,
    pub transition: FileTransition,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct FileColor {
    pub r: FileParam,
    pub g: FileParam,
    pub b: FileParam,
    pub a: FileParam,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FileStroke {
    pub width: FileParam,
    pub color: FileColor,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FileStyle {
    pub fill: Option<FileColor>,
    pub stroke: Option<FileStroke>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileTransport {
    Timeline {
        position: u32,
        speed: FileParam,
    },
    BpmSync {
        position: u32,
        duration_beats: FileParam,
        play_mode: String,
    },
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileTransition {
    Cut,
    Fade {
        speed: FileParam,
        curve: String,
    },
    Wipe {
        speed: FileParam,
        curve: String,
        direction: String,
    },
}
