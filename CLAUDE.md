# FrogenX

Mixeur vidéo et scène 3D live, organisé en grille de clips déclenchables, où chaque
clip est piloté comme un synthétiseur modulaire. Rust, wgpu, multiplateforme
(Linux / macOS / Windows).

**Lire [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) avant toute intervention.** Les
décisions structurantes y sont accompagnées de leur raison (§13, tableau
décision → raison) — ne pas les redécouvrir ni les défaire par inadvertance.

## Deux règles non négociables

1. **Toute fonctionnalité arrive avec ses tests unitaires.**
2. **Toute modification met la documentation à jour — dans le même commit.**

Aucune exception de taille : un « petit changement » suit la même règle. Détail au §15
de l'architecture, et dans la skill `feature-complete`.

## Les invariants qui portent le système

- **Tout est un `Signal`** — oscillateur, LFO, enveloppe, feature audio : tous
  interchangeables, donc n'importe quelle source module n'importe quelle destination.
- **Tout paramètre réglable est un `Param`, jamais un `f32` nu.** La règle la plus facile
  à oublier du projet : un `f32` glissé aujourd'hui est un câble impossible à brancher
  demain.
- **L'état vit dans le graphe**, pas dans les modules — aucun champ mutable, la mémoire
  entre frames passe par un `NodeState` prêté.
- **Jamais de panique en évaluation.** Erreurs en édition seulement ; en frame, valeur
  de repli + compteur `Diagnostics`.

Détail dans la skill `modular-design`, contrats complets en partie II de l'architecture.

## Règle de dépendance des crates

`core` → `osc` → `shape` → `composition` → `audio` → `layer` → `render` → `output` →
`ui` → `app`, les flèches vers le haut. `core` ne dépend de rien ; `shape` et `composition` ne
connaissent ni wgpu ni l'audio.

Ce n'est pas de l'esthétique : c'est ce qui rend `core`, `osc`, `shape` et
`composition` testables sans GPU ni carte son. Une fonctionnalité soudain « impossible
à tester » signale presque toujours une dépendance qui a fuité vers le bas — la
question qui compte n'est pas « combien de crates en amont », mais « est-ce que l'une
d'elles connaît le GPU, la carte son ou une fenêtre » (docs/ARCHITECTURE.md §10).

## Skills

| Skill | Quand |
|---|---|
| `feature-complete` | avant de commencer **et** de terminer toute fonctionnalité |
| `modular-design` | avant d'ajouter un module, un oscillateur, une forme, un paramètre |
| `unit-tests` | écrire ou lancer des tests — quoi asserter par étage |
| `doc-sync` | quelle section de l'architecture suit quel type de changement |
| `rust-workspace` | ajouter une crate ou une dépendance |
| `poc-video` | produire un clip de démonstration |

## État

**Étapes 1 à 7 faites.** `cargo run` ouvre l'application : une grille de démonstration
provisoire à 60 fps, touches 1 à 4 pour les colonnes, Espace pour taper le tempo. Rendus sans fenêtre : `cargo run -p frogenx-render
--example first_image` (une image) ; `cargo run --release -p frogenx-app --example
sequence` (un clip déterministe, skill `poc-video`). Les tests GPU se sautent proprement
sans adaptateur, et tournent un par un (`.cargo/config.toml`, §24.3).

- `crates/core` : `Signal`, `Param`, `Graph`, `Clock`. Aucune dépendance.
- `crates/osc` : oscillateurs (phase accumulée), LFO, bruit cohérent à graine
  explicite, enveloppes attaque/relâchement, opérateurs (`Sum`, `Product`, `Mix`,
  `Trigger`, `SampleHold`).
- `crates/shape` : une forme est un `Path` (buffer plat de points découpé en contours
  fermés ou ouverts, §16.3). Primitives (`Ellipse`, `Polygon` à côtés flottants, `Star`,
  `Rect`, `Arc`), générateurs paramétriques (`Lissajous`, `Rose`, `Superformula`) et
  opérateurs en place (`Deform`, `NoiseDisplace`, `RadialRepeat`, `Transform`, `Smooth`,
  `Chain`). Le style y vit aussi (`ColorParam`, `ShapeStyle`, `BlendParams`) : un clip le
  porte, et `composition` ne connaît pas wgpu.
- `crates/composition` : la grille (§3bis) — `Deck`/`LayerGroup`/`Layer`/`Column`/
  `Clip`, `CrossFader`, `TempoController`, `PositionSignal`, `BeatRate`, `Command`, graphe
  de composition. Dépend de `core` et `shape`. Seul `Cut` s'applique, et `PlayMode` n'a
  pas encore d'effet (§3bis.3).
- `crates/layer` : `Gpu`, trait `RenderLayer` (`prepare`/`encode`/`output`),
  `ShapeRenderer` (un par piste, le clip lui prête son `ShapeContent`), `ShapeLayer`,
  tessellation lyon (module sans wgpu).
- `crates/render` : `Compositor` (passes shader en ping-pong, 5 modes), relecture
  GPU → CPU, PNG. `blend.rs` est la référence CPU du shader de mélange.
- `crates/output` : `GpuSink` (`encode`/`after_submit`), `CpuSink`, étage `Outputs`,
  passe de conversion 8 bits sRGB, `Preview`, `PngSequenceSink`. `fanout` (files bornées, politique
  de perte) ne connaît pas wgpu. `output` ne connaît pas `winit`.
- `crates/app` : `Engine`, qui traduit la grille en couches selon le §20.1, sans
  fenêtre, et le binaire `frogenx`, seul à connaître `winit`. La grille de `demo` est
  provisoire : la bibliothèque de clips la remplacera.

**Prochaine étape : l'interface egui** (grille, monitor, paramètres). Le §12 ne contient
pas encore cette étape. Première version visée : interface, clips prédéfinis, sortie
plein écran, sauvegarde, puis audio (capture, BPM calculé, déclenchements, §7).
Les sorties CPU restent refusées en `Realtime` jusqu'à l'anneau de l'étape 10.

wgpu est épinglé en **30** : lire ses signatures dans les sources de la version
épinglée, pas de mémoire (skill `rust-workspace`).

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
```
