---
name: doc-sync
description: Keep docs/ARCHITECTURE.md truthful — which section to update for which kind of change, and the §13 decision table. Required in the same commit as any code change.
---

`docs/ARCHITECTURE.md` est un document **vivant**. Une doc qui ment est pire que pas de
doc. Mise à jour **dans le même commit** que le code — « plus tard » n'arrive jamais.

## Quoi suit quoi

| Ce qui change dans le code | Sections à mettre à jour |
|---|---|
| Une **décision structurante** | la section concernée **et** la ligne du **§13** |
| Un **module** ajouté | §10 (crates), et §11 si une dépendance apparaît |
| Une **étape** de feuille de route franchie | §12 |
| Un **piège** rencontré à l'usage | la section concernée (voir §14) |
| Un **comportement observable** modifié | partout où il est décrit |
| Un **trait public** (`Signal`, `Layer`, `OutputSink`, `ShapeGenerator`) | son extrait de code dans la section correspondante |

## Le §13 est le cœur

Le tableau « décision → raison » est ce qu'on lit en reprenant le projet dans six mois.
Quand une décision change, **remplacer la ligne**, ne pas en ajouter une seconde qui la
contredit. Ne pas empiler les couches d'obsolescence.

## Quand l'implémentation corrige le contrat

Cas réel de l'étape 3 : le §16.4 spécifiait `Param::get(&Graph)`, inapplicable depuis
`Signal::eval` — le graphe mute l'état du nœud courant et ne peut pas se prêter en
entier. Le contrat était faux.

**Ce qu'il faut faire :** corriger la section, ajouter la ligne au §13, et surtout
**écrire pourquoi**. Ici : « un `Signal` n'a pas à voir la topologie du graphe,
seulement les valeurs de ses dépendances » — la contrainte du compilateur avait révélé
un couplage que le contrat autorisait à tort. C'est cette phrase qui a de la valeur
dans six mois, pas le fait que la signature ait changé.

## Le test de cohérence

> Si quelqu'un lit le document et est surpris par le code, le document a un bug.
> Il se corrige comme un bug.

## Les extraits de code

Le document contient des `trait` et des `enum` en Rust. Ils doivent correspondre au code
réel — signatures comprises. Un extrait périmé est plus nuisible qu'absent, parce qu'on
lui fait confiance. Après avoir touché un trait public, `grep` son nom dans le document.

## Signal d'alarme

Une nouvelle couche difficile à documenter dans les quatre étages du §1 (signaux →
formes → couches → sorties) indique un problème d'**architecture**, pas de rédaction. Si
un module n'y rentre pas, c'est que l'architecture doit bouger — et cela mérite d'être
écrit, discuté, pas contourné.
