---
name: rust-workspace
description: FrogenX cargo workspace layout and dependency rule — which crate may depend on which, plus build/lint/test commands. Read before adding a crate or a dependency.
---

```bash
cargo build --workspace
cargo test  --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
```

## Les crates

Elles vivent sous `crates/`, le workspace est à la racine.

```
crates/core/         Signal, Param, Graph, Clock               ← aucune dépendance  ✅
crates/osc/          oscillateurs, LFO, bruit, enveloppes      ← core               ✅
crates/shape/        Path, primitives + opérateurs             ← core, osc, glam    ✅
crates/composition/  grille : Deck/Layer/Column/Clip, tempo    ← core, shape        ✅
crates/audio/        cpal, FFT, features → Signal
crates/layer/        Gpu, trait RenderLayer, ShapeLayer, tessellation ← core, shape, wgpu ✅
crates/render/       compositeur, readback, PNG                      ← core, layer, wgpu  ✅
crates/output/       GpuSink / CpuSink, Outputs, passe 8 bits, Preview, PngSequenceSink ← layer, render, wgpu ✅
                     à venir : NDI / Spout / Syphon / recorder
crates/ui/           egui : grille de clips, rack, câblage, monitoring
crates/app/          Engine (sans fenêtre) + binaire `frogenx` (`cargo run`) ← tout le reste, winit ✅
```

**Ajouter une crate** demande deux éditions du `Cargo.toml` racine — `members` **et**
`[workspace.dependencies]`. Oublier la seconde donne une erreur de résolution peu
parlante.

## La règle de dépendance

**Les flèches vont vers le haut de la liste.** `core` ne dépend de rien. `shape` et
`composition` ne connaissent ni wgpu ni l'audio. `render` ne sait pas ce qu'est un
oscillateur.

Ce n'est pas une préférence esthétique : c'est ce qui garde `core`, `osc`, `shape` et
`composition` testables sans GPU, sans carte son et sans fenêtre — donc en CI,
rapidement. Une dépendance qui fuit vers le bas rend soudain une couche entière
impossible à tester. La question qui compte n'est pas « combien de crates en amont »,
mais « est-ce que l'une d'elles connaît le GPU, la carte son ou une fenêtre » — c'est
ce qui autorise `composition → shape` (§10).

Dans `layer`, la tessellation reste un **module qui ne connaît pas wgpu** : la crate
dépend du GPU, mais ce module-là reste testable sans (§10, §24.3).

**`unsafe` : seulement dans `output`**, confiné aux modules Spout et Syphon sous `cfg`
(interop de textures natives via `as_hal`, §8.1). Partout ailleurs, aucun.

**Avant d'ajouter une dépendance à un `Cargo.toml` :** vérifier le sens de la flèche. Si
`shape` a besoin de quelque chose de `render`, l'abstraction est au mauvais endroit —
c'est presque toujours un trait qui devrait descendre dans `core`.

## Dépendances communes

Déclarées dans `[workspace.dependencies]` à la racine, référencées par
`dep.workspace = true` dans chaque crate. Évite les versions divergentes de `glam` ou
`wgpu` entre crates, qui produisent des erreurs de types incompréhensibles.

| Domaine | Crate | Statut |
|---|---|---|
| Maths | `glam` | ✅ (shape) |
| Sérialisation | `serde` | ✅ (composition) |
| Rendu | `wgpu` **30** | ✅ (layer, render) — majeure épinglée |
| Tessellation | `lyon_tessellation` | ✅ (layer) |
| GPU, utilitaires | `bytemuck`, `pollster` | ✅ (layer, render) |
| Image | `png` | ✅ (render) |
| Fenêtrage | `winit` **0.30** | ✅ (app seulement) — version imposée par `egui-wgpu` (§11) |
| UI | `egui`, `egui-wgpu` 0.36+ | à venir — demandera Rust 1.95 (§11) |
| Audio | `cpal`, `rustfft` | à venir |
| Lock-free | `rtrb`, `triple_buffer` | à venir |

**Avant d'écrire du code wgpu, lire les signatures dans les sources de la version
épinglée** (`~/.cargo/registry/src/*/wgpu-30*`, `wgpu-types-30*`), pas de mémoire :
d'une majeure à l'autre, `PipelineLayoutDescriptor` a pris des `Option` et
`immediate_size`, `multiview` est devenu `multiview_mask`, `get_mapped_range` rend un
`Result`, `request_adapter` aussi. Même règle pour `winit` et pour l'interop `as_hal`.

**`wgpu` est épinglé à une majeure, et `rust-version` suit son minimum** (§11). wgpu
change d'API à chaque majeure, environ tous les trois mois : une montée de version est
un commit délibéré, jamais un effet de bord d'un `cargo update`. Relever la majeure
oblige à relever `rust-version` du workspace si son minimum augmente.

**Le bruit ne prend pas la crate `noise`** : implémenté dans `osc` pour garantir le
déterminisme à graine explicite (§20.3) plutôt que le supposer à travers les versions
d'une dépendance. Avant d'ajouter une crate, se demander si le déterminisme en dépend.

## Multiplateforme

Cible : Linux, macOS, Windows à parité. Le code spécifique passe par `cfg` :

```rust
mod ndi;                                     // partout
#[cfg(windows)]             mod spout;
#[cfg(target_os = "macos")] mod syphon;
```

`cargo check` ne compile que la plateforme courante — un `cfg` d'une autre plateforme
peut être cassé sans qu'on le voie. Vérifier en CI sur les trois.
