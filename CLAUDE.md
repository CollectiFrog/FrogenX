# FrogenX

Générateur de visuels procéduraux, organiques et géométriques — temps réel, piloté
comme un synthétiseur modulaire. Rust, wgpu, multiplateforme (Linux / macOS / Windows).

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

`core` → `osc` → `audio` → `shape` → `layer` → `render` → `output` → `ui`, les flèches
vers le haut. `core` ne dépend de rien ; `shape` ne connaît ni wgpu ni l'audio.

Ce n'est pas de l'esthétique : c'est ce qui rend `core`, `osc` et `shape` testables sans
GPU ni carte son. Une fonctionnalité soudain « impossible à tester » signale presque
toujours une dépendance qui a fuité vers le bas.

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

**Étapes 1 et 2 faites** — `crates/core` : `Signal`, `Param`, `Graph`, `Clock`, sans
aucune dépendance, 30 tests verts. Les contrats d'implémentation sont fixés en partie II
de l'architecture (§16–§24).

Prochaine étape : `crates/osc` — oscillateurs, LFO, enveloppes, bruit. Puis les formes
(étape 3), et la première image à l'écran (étape 4).

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
```
