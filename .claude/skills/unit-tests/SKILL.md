---
name: unit-tests
description: Run and write FrogenX unit tests — what to assert per architectural layer (Signal, Graph, Param, Shape, audio, output), NodeState injection, Diagnostics assertions, and how to test without GPU or sound card
---

```bash
cargo test --workspace                 # tout
cargo test -p frogenx-core             # un crate
cargo test -p frogenx-osc -- --nocapture     # avec sortie
```

`core`, `osc`, `shape` et `composition` tournent **sans GPU ni carte son** — donc en
CI, rapidement. Dans `layer` et `render`, les modules `tessellate`, `color`, `space`,
`blend` et `convert` aussi. C'est la règle de dépendance du §10 de
`docs/ARCHITECTURE.md`, et elle existe pour ça.

## Quoi tester, par étage

| Étage | Assertions attendues |
|---|---|
| **Signaux** | valeurs exactes à des `t` connus, bornes, périodicité, cas limites (fréquence nulle, négative, non finie) |
| **Signaux à état** | sortie **et** `NodeState` résultant, sur plusieurs frames enchaînées |
| **Graphe** | tri topologique correct, détection de cycle, **cache : un nœud partagé n'est évalué qu'une fois** |
| **Paramètres** | `Fixed` constant ; `Modulated` == `base + depth * source` |
| **Formes** | nombre de points, drapeau fermé/ouvert de chaque contour, symétries, morphing continu du polygone |
| **Opérateurs** | composition, idempotence quand elle est attendue, préservation du nombre de points ; **commutation** avec `RadialRepeat` pour ceux qui regardent les voisins |
| **Tessellation** | aire couverte, contour ouvert jamais rempli, contours disjoints sans triangle à cheval, dégénérés sans panique |
| **Couches** | pixels relus en quelques points, comparés à une référence CPU (`srgb_to_linear`, `blend_premul`) |
| **Audio** | features sur signaux **synthétiques** : sinus pur → centroïde connu, silence → RMS nul |
| **Sorties** | un `OutputSink` factice reçoit les frames, dans l'ordre, sans perte en mode `Fixed` |
| **Patchs** | chaque fixture d'ancienne version se migre et se charge (§19.2) |

## L'état se teste en l'injectant

Les modules sont purs : l'état vit dans le graphe (§16.1). Un test fournit donc un
`NodeState` connu et vérifie **les deux** sorties — la valeur et l'état d'après.

```rust
let mut slots = [0.0f32; 1];
let mut st = NodeState::new(&mut slots);
let v = osc.eval(0.0, &ctx, &mut st);
assert!((v - 0.0).abs() < 1e-6, "sortie: got {v}");
assert!((slots[0] - expected_phase).abs() < 1e-6, "phase: got {}", slots[0]);
```

C'est le bénéfice principal du choix « état dans le graphe » : aucun module n'a besoin
d'être mis dans un état particulier avant d'être testé.

## Tester une propriété, pas seulement une valeur

Les tests les plus utiles d'`osc` vérifient une **propriété** sur une trajectoire :

- **Continuité** — sous modulation de fréquence, aucun écart entre deux frames
  consécutives n'excède ce qu'un pas de temps peut produire. C'est ce qui prouve que la
  phase est accumulée et non recalculée.
- **Bornage** — 10 000 points de bruit tous dans `[-1, 1]`.
- **Cohérence** — deux points de bruit proches donnent des valeurs proches (c'est ce
  qui distingue le bruit cohérent du bruit blanc).
- **Symétrie** — `sin(-x) == -sin(x)` : deux oscillateurs de fréquences opposées
  restent opposés.
- **Comptage** — 1 Hz pendant 3 s produit exactement 3 déclenchements.

## `Diagnostics` à zéro

Après une frame nominale, **tous** les compteurs sont nuls :

```rust
assert!(g.diagnostics().is_clean(), "got {:?}", g.diagnostics());
```

C'est ce qui empêche le régime tolérant du §18 de devenir « silencieusement faux
partout ». Tester aussi le contraire : un patch volontairement incohérent **doit**
incrémenter le bon compteur, sans paniquer.

## Flottants

Jamais d'égalité stricte sur des `f32` issus de trigonométrie :

```rust
assert!((got - want).abs() < 1e-6, "got {got}, want {want}");
```

Toujours afficher les deux valeurs — un `assert!` nu sur un flottant ne dit rien quand
il casse.

## Le cache du graphe

Le test le moins évident et le plus important : un LFO branché sur cinq destinations
doit être évalué **une fois** par frame. Le vérifier avec un `Signal` compteur qui
incrémente un `Cell<u32>` à chaque `eval`, câblé sur plusieurs destinations, puis
asserter `count == 1` après une frame. Sans ce test, la régression est invisible — le
résultat visuel reste correct, seul le coût explose.

## Fixtures de patchs

`tests/fixtures/` contient un patch par version de format. **Jamais régénérées** — leur
valeur est précisément d'être anciennes. Un changement de format ajoute une fixture, il
n'en modifie aucune.

## `gen()` vérifie l'invariant des plages

Dans `shape`, l'utilitaire de test `gen` / `gen_path` vérifie à **chaque appel** que le
`Path` produit est valide (`Path::is_valid`). Tout test qui génère une forme teste donc
gratuitement l'invariant des contours — ne pas contourner `gen` pour appeler
`generate` à la main, sauf pour tester justement un `Path` invalide.

## GPU et matériel

Injecter une abstraction plutôt que renoncer au test : `OutputSink` factice, source
audio synthétique. Les tests d'intégration réels **ne remplacent pas** les tests
unitaires.

**Tests de pixels (`crates/render/tests/pixels.rs`).** Ils ouvrent un GPU par
`Gpu::headless()`, qui rend `Option` :

```rust
let Some(gpu) = gpu() else { return };   // sans adaptateur : succès, message affiché
```

Sans GPU, le test **réussit** et l'écrit sur sa sortie — jamais d'échec ni de panique,
sinon il finit désactivé (§24.3). Ils vérifient quelques pixels précis (centre, coin)
contre une **référence CPU**, avec une tolérance de l'ordre de 3e-3 (textures en
demi-flottants). Pour savoir s'ils ont vraiment tourné sur un GPU, les lancer avec
`-- --nocapture` et chercher « GPU absent ».

Le mélange des couches a deux implémentations, `blend.rs` (CPU) et `composite.wgsl`
(GPU) : le test `chaque_mode_de_melange_du_gpu_suit_la_reference_cpu` les compare mode
par mode. Toute modification de l'une se reporte dans l'autre.

Les snapshots d'image entière avec **tolérance perceptuelle** (§24.2) viendront quand
les images seront trop riches pour quelques points — jamais d'égalité stricte, qui
échoue d'un GPU à l'autre.
