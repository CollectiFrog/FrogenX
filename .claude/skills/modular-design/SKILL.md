---
name: modular-design
description: The load-bearing invariants of FrogenX — everything is a Signal, every tunable parameter is a Param (never a bare f32), node state lives in the graph. Read before adding any module, oscillator, shape, or parameter.
---

Ces invariants portent tout le système. Les enfreindre ne casse rien immédiatement —
c'est ce qui les rend dangereux. Contrats détaillés : partie II de
`docs/ARCHITECTURE.md` (§16–§20).

## 1. Tout est un `Signal`

```rust
pub trait Signal {
    fn eval(&self, t: f64, ctx: &Ctx, state: &mut NodeState) -> f32;
    fn state_size(&self) -> usize { 0 }   // 0 = sans état
}
```

Oscillateur, LFO, enveloppe, constante, mixeur, **bande de fréquence audio** : tous
implémentent `Signal`, donc tous sont interchangeables.

On ne code jamais « le LFO peut moduler la fréquence » puis « l'audio peut moduler la
taille ». On code `Signal` une fois, et **n'importe quelle source module n'importe
quelle destination**. Une feature audio est un `Signal` — c'est pour ça que l'audio-
réactivité n'a demandé aucune abstraction nouvelle (§7).

## 2. L'état vit dans le graphe, pas dans le module

Un module n'a **aucun champ mutable**. Sa mémoire entre frames est dans le `NodeState`
que le graphe possède et lui prête.

```rust
// slots[0] = phase accumulée      ← documenter les slots, toujours
fn eval(&self, _t: f64, ctx: &Ctx, state: &mut NodeState) -> f32 {
    let phase = wrap01(state.get(0) + self.freq.eval(ctx) * ctx.dt as f32);
    state.set(0, phase);
    self.waveform.eval(phase)
}
```

Bénéfices : modules purs et testables en injectant un état connu, sérialisation et
reset gratuits, et un nœud partagé par cinq destinations reste trivial (pas d'emprunt
mutable). **Un slot non documenté est un bug de relecture.**

## 3. Tout paramètre réglable est un `Param`

```rust
pub enum Param {
    Fixed(f32),
    Modulated { base: f32, depth: f32, source: NodeId },
}
```

**La règle la plus importante du projet, et la plus facile à oublier :** aucun champ
numérique visible par l'utilisateur ne doit être déclaré `f32`. S'il est réglable, il
est `Param`.

> Un `f32` glissé aujourd'hui est un câble impossible à brancher demain.

Le piège : `f32` marche parfaitement au moment où on l'écrit. Le coût n'apparaît qu'au
moment où l'on veut y brancher un LFO — et il faut alors remonter toute la chaîne
d'appel.

**Deux voies pour le résoudre**, selon d'où on appelle :

| Appelant | Méthode |
|---|---|
| Forme, couche — **après** `eval_frame` | `p.get(&graph)` |
| `Signal` — **pendant** son `eval` | `p.eval(ctx)` |

Les deux lisent le cache et n'évaluent rien. La seconde existe parce que le graphe mute
le `NodeState` du nœud courant pendant `eval` et ne peut pas se prêter en entier
(§16.2) — un `Signal` voit les valeurs de ses dépendances, jamais la topologie.

## 4. Conventions numériques (§17)

| Grandeur | Convention |
|---|---|
| Sortie de `Signal` | nominalement `[-1,1]`, **non borné** (sommes, FM) |
| Enveloppes, features audio | `[0,1]` |
| Fréquence | Hz |
| Phase | tours `[0,1)` — pas de `2π` partout |
| Angle géométrique | radians |
| Espace des formes | normalisé `[-1,1]`, origine au centre |
| Temps `t` | secondes, **`f64`** — en `f32` ça bégaie après ~1 h |

## 5. Erreurs : jamais de panique en évaluation (§18)

- **Édition** (câblage, chargement) → `Result`, refus avec message clair.
- **Évaluation** (60 fps) → valeur de repli + compteur `Diagnostics`, jamais d'erreur.

**Saturer chez le consommateur, jamais dans le `Signal`** — saturer à la source
détruirait la modulation pour tous les autres consommateurs :

```rust
let sides  = self.sides.get(g).clamp(2.0, 64.0);
let radius = self.radius.get(g).max(0.0);
```

## 6. Le graphe

DAG en **arena** : `Vec<Node>` + `NodeId(u32)` avec compteur de génération. Pas de
`Rc<RefCell<Node>>` — idiome Rust, sérialisation triviale, pas d'emprunts imbriqués.

**Cache par frame obligatoire** : un LFO branché sur cinq destinations est évalué une
fois, pas cinq. **Cycles interdits**, détectés au câblage. Le feedback se fait au
niveau des couches (§6), où il a un délai d'une frame bien défini.

## 7. Le temps

Aucun module ne lit l'horloge système. `t` est **toujours** reçu en paramètre. C'est ce
qui rend possibles le rendu offline déterministe et les tests à valeur exacte. Un
`Instant::now()` dans `core`, `osc` ou `shape` est un bug.

**Corollaire pratique**, appris en écrivant les oscillateurs : ne jamais calculer
`sin(2π·f·t)` directement. Dès que `f` est modulée, la phase **saute** à chaque
changement. Il faut l'accumuler dans le `NodeState` — c'est la raison d'être du
mécanisme d'état.

Autre corollaire (§20.3) : pas de `HashMap` dans le chemin d'évaluation (ordre non
déterministe), et tout aléatoire passe par une graine explicite stockée dans le patch —
jamais `rand::random()`.

## Où investir

Ajouter un **opérateur** multiplie les possibilités de toutes les primitives
existantes. Ajouter une **primitive** n'ajoute qu'elle-même. En cas de doute, écrire un
opérateur.
