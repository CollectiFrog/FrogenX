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

`core` → `osc` → `shape` → `composition` → `audio` → `library` → `patch` → `layer` → `render` → `output` →
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

**Étapes 1 à 9 faites : la première version est là, et le feedback avec.** `cargo run` ouvre l'application : une interface egui sur la
grille de départ, à 60 fps. On y déclenche des clips, on en glisse d'autres depuis la
bibliothèque, on règle leurs paramètres, leur vitesse et le tempo ; au clavier, 1 à 8 pour
les colonnes et Espace pour taper. Le menu `Sortie` envoie l'image en plein écran sur un
autre écran (§8.4), le menu `Fichier` enregistre et rouvre son travail (§23.8), et le
dernier fichier revient au lancement. Rendus sans fenêtre : `cargo run -p frogenx-render
--example first_image` (une image) ; `cargo run --release -p frogenx-app --example
sequence` (un clip déterministe, skill `poc-video`) ; `cargo run --release -p frogenx-app
--example catalogue` (une planche du catalogue). Les tests GPU se sautent proprement
sans adaptateur, et tournent un par un (`.cargo/config.toml`, §24.3).

Le panneau audio montre le spectre de l'entrée par défaut, dans l'analyse choisie : trois
méthodes, quatre échelles de fréquence dont la linéaire, deux d'amplitude, et une durée
d'analyse de 5 ms à 1 s (§7). On y trace à la souris la zone d'un **trig**, qui déclenche
une colonne ou un clip une fois sur 1, 2, 4, 8, 16 ou 32 (§3bis.6). Le design suit §23.1 :
disposition de Resolume, apparence de Pro-Q, et toute l'interface sur le modèle du spectre
(§23.9) : le signal d'abord, un potentiomètre pour tout réglage, l'image jamais couverte.

- `crates/core` : `Signal`, `Param`, `Graph`, `Clock`, `Spectrum`. Aucune dépendance.
- `crates/osc` : oscillateurs (phase accumulée), LFO, bruit cohérent à graine
  explicite, enveloppes attaque/relâchement, opérateurs (`Sum`, `Product`, `Mix`,
  `Trigger`, `SampleHold`), et les deux `Signal` du trig (`SpectrumZone`, `Divider`).
- `crates/shape` : une forme est un `Path` (buffer plat de points découpé en contours
  fermés ou ouverts, §16.3). Primitives (`Ellipse`, `Polygon` à côtés flottants, `Star`,
  `Rect`, `Arc`), générateurs paramétriques (`Lissajous`, `Rose`, `Superformula`) et
  opérateurs en place (`Deform`, `NoiseDisplace`, `RadialRepeat`, `Transform`, `Smooth`,
  `Chain`). Le style y vit aussi (`ColorParam`, `ShapeStyle`, `BlendParams`) : un clip le
  porte, et `composition` ne connaît pas wgpu.
- `crates/composition` : la grille (§3bis) — `Deck`/`LayerGroup`/`Layer`/`Column`/
  `Clip`, `CrossFader`, `TempoController`, `Trig`, `PositionSignal`, `BeatRate`, `Command`, graphe
  de composition. Dépend de `core` et `shape`. `PlayMode` n'a
  pas encore d'effet (§3bis.3).
- `crates/audio` : capture `cpal`, analyse en 128 bandes (crête, énergie, multi-résolution),
  features fixes et BPM (`features.rs`, `tempo.rs`, §7.1). Seul
  `capture.rs` touche à la carte son ; le reste se teste sur des signaux de synthèse. Dépend
  de `core` seule : `app` pose les features dans `Engine` (`set_audio`).
- `crates/library` : les 27 clips prédéfinis et la grille de départ (§23.6), en code
  jusqu'au format de l'étape 7e. Sans GPU ni carte son.
- `crates/patch` : le format de fichier (§19), `Composition` ↔ JSON, version et fixture,
  écriture par fichier temporaire. Seule crate à connaître le JSON ; `core` décrit un nœud
  par une `Spec` neutre. Un nouveau `Signal` doit entrer dans `patch/src/signals.rs`.
- Un clip est une forme ou un écho (`ClipSource::Feedback`, §6.1) : l'écho relit l'image
  finale de la frame d'avant, et ses réglages s'expriment par seconde, jamais par frame.
- `crates/layer` : `Gpu`, trait `RenderLayer` (`prepare`/`encode`/`output`),
  `ShapeRenderer` (un par piste, le clip lui prête son `ShapeContent`), `ShapeLayer`,
  tessellation lyon (module sans wgpu).
- `crates/render` : `Compositor` (passes shader en ping-pong, 5 modes), relecture
  GPU → CPU, PNG. `blend.rs` est la référence CPU du shader de mélange.
- `crates/output` : `GpuSink` (`encode`/`after_submit`), `CpuSink`, étage `Outputs`,
  passe de conversion 8 bits sRGB, `PresentSurface`, `Preview`, `MonitorSink`,
  `PngSequenceSink`. `fanout` (files bornées, politique
  de perte) ne connaît pas wgpu. `output` ne connaît pas `winit`.
- `crates/ui` : l'interface, fonction de l'état, et l'éditeur de nœuds (`nodes.rs`). Lit la grille, rend des `Command` et des
  `Placement` pour les clips glissés depuis la bibliothèque. Ne
  connaît ni wgpu ni `winit` : testée par clics simulés (`egui_kittest`).
- `crates/app` : `Engine`, qui traduit la grille en couches selon le §20.1, sans
  fenêtre, et le binaire `frogenx`, seul à connaître `winit`, `egui-wgpu`, `egui-winit`.
  `stage.rs` y possède la fenêtre de sortie et suit les écrans branchés, `node_window.rs`
  celle de l'éditeur de nœuds. `file_flow.rs`
  décide du parcours du fichier sans fenêtre, `files.rs` produit ses effets.

**Prochaine étape : à choisir au §12.1**, le backlog à tenir à jour. L'éditeur de nœuds est fait
(§23.10) : tout câblage passe par `Composition::connect` et ses commandes. Annuler passe par
`Engine::undo` et des images de la grille (§23.11) : une nouvelle commande est couverte d'office,
mais elle doit recevoir son nom dans `Command::label`.
Le BPM et les features n'ont pas encore été essayés sur de la vraie musique (§7.1).
Les sorties CPU restent refusées en `Realtime` jusqu'à l'anneau de l'étape 10.

wgpu est épinglé en **30** : lire ses signatures dans les sources de la version
épinglée, pas de mémoire (skill `rust-workspace`).

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
```
