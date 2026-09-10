---
name: feature-complete
description: The definition-of-done gate for FrogenX — code + unit tests + doc update in the same commit. Read before starting ANY feature, and again before claiming one is finished.
---

Une fonctionnalité est terminée quand le code marche, **que les tests le prouvent**, et
**que la documentation le décrit**. Les trois, ou rien. Règle posée par l'utilisateur au
cadrage du projet, inscrite au §15 de `docs/ARCHITECTURE.md`. Aucune exception de
taille : un « petit changement » suit la même règle.

## Les trois livrables, dans le même commit

1. **Le code** de la fonctionnalité.
2. **Les tests unitaires** qui la couvrent — voir `unit-tests` pour quoi tester par étage.
3. **La doc à jour** — voir `doc-sync` pour quelles sections suivent quoi.

## Avant de dire « c'est fait »

```bash
cargo test --workspace          # vert, sans exception
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
```

Puis vérifier que `docs/ARCHITECTURE.md` ne ment pas sur ce qui vient de changer.

## Le piège récurrent

L'ordre naturel — coder, puis tester, puis documenter — fait que la doc saute quand le
code marche et qu'on passe à la suite. **Écrire le test dans le même mouvement que le
code**, et corriger la doc avant de lancer la suite de tests, pas après : à ce
moment-là on a encore en tête *pourquoi* on a fait ce choix, ce qui est précisément ce
que la doc doit capturer (§13, tableau décision → raison).

## Quand le compilateur contredit le contrat

Cela s'est produit à l'étape 3 : `Param::get(&Graph)` était inapplicable depuis
`Signal::eval` (le graphe mute l'état du nœud courant, il ne peut pas se prêter en
entier). **Ce n'est pas un détail d'implémentation à contourner en silence.** Le
contrat était faux ; il se corrige dans le document, avec sa raison, dans le même
commit. Une contrainte du compilateur révèle souvent un couplage que le contrat
autorisait à tort.

## Ce qui rend tout ceci possible

La règle de dépendance des crates (§10) : `core`, `osc`, `shape` ne connaissent ni GPU,
ni carte son, ni fenêtre. **Ne jamais l'enfreindre pour un raccourci** — c'est la
condition de la testabilité du projet, pas une élégance. Une fonctionnalité soudain
« impossible à tester » signale presque toujours qu'une dépendance a fuité vers le bas.
