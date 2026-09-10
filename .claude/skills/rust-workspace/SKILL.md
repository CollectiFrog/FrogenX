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
crates/core/    Signal, Param, Graph, Clock          ← aucune dépendance   ✅
crates/osc/     oscillateurs, LFO, bruit, enveloppes ← core                ✅
crates/audio/   cpal, FFT, features → Signal
crates/shape/   primitives + opérateurs
crates/layer/   trait Layer : Shape / Feedback / Video
crates/render/  wgpu, compositeur, post-fx
crates/output/  OutputSink : preview / NDI / Spout / Syphon / recorder
crates/ui/      egui : rack, câblage, monitoring
```

**Ajouter une crate** demande deux éditions du `Cargo.toml` racine — `members` **et**
`[workspace.dependencies]`. Oublier la seconde donne une erreur de résolution peu
parlante.

## La règle de dépendance

**Les flèches vont vers le haut de la liste.** `core` ne dépend de rien. `shape` ne
connaît ni wgpu ni l'audio. `render` ne sait pas ce qu'est un oscillateur.

Ce n'est pas une préférence esthétique : c'est ce qui garde `core`, `osc` et `shape`
testables sans GPU, sans carte son et sans fenêtre — donc en CI, rapidement. Une
dépendance qui fuit vers le bas rend soudain une couche entière impossible à tester.

**Avant d'ajouter une dépendance à un `Cargo.toml` :** vérifier le sens de la flèche. Si
`shape` a besoin de quelque chose de `render`, l'abstraction est au mauvais endroit —
c'est presque toujours un trait qui devrait descendre dans `core`.

## Dépendances communes

Déclarées dans `[workspace.dependencies]` à la racine, référencées par
`dep.workspace = true` dans chaque crate. Évite les versions divergentes de `glam` ou
`wgpu` entre crates, qui produisent des erreurs de types incompréhensibles.

| Domaine | Crate | Statut |
|---|---|---|
| Maths | `glam` | à venir (shape) |
| Rendu | `wgpu`, `winit` | à venir (render) |
| UI | `egui` | à venir |
| Tessellation | `lyon` | à venir |
| Audio | `cpal`, `rustfft` | à venir |
| Lock-free | `rtrb`, `triple_buffer` | à venir |
| Sérialisation | `serde` | à venir |

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
