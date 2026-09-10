# Architecture — FrogenX

> Générateur de visuels procéduraux, organiques et géométriques, temps réel,
> piloté comme un synthétiseur modulaire.

**Statut du document :** vivant. Il décrit les décisions structurantes et le *pourquoi*
de chacune. Il doit être modifié quand une décision change — un document d'architecture
qui ment est pire que pas de document.

---

## 1. L'idée en une phrase

Des **signaux** (oscillateurs, LFO, enveloppes, features audio) modulent des
**paramètres** de **formes**, rendues en **couches** composées ensemble, puis
envoyées vers plusieurs **sorties** simultanées.

```
   Signaux  ──module──▶  Formes  ──rasterise──▶  Couches  ──compose──▶  Sorties
  (osc, LFO,            (primitives            (shape,               (écran, NDI,
   audio)                + opérateurs)          feedback, vidéo)      fichier)
```

Ces quatre étages sont l'ossature du logiciel. Tout ce qui suit les détaille.

---

## 2. Les deux idées qui portent tout le reste

Si vous ne lisez que deux sections de ce document, lisez celle-ci.

### 2.1 Tout est un signal

```rust
/// Tout module produit une valeur à un instant t.
trait Signal {
    fn eval(&self, t: f64, ctx: &Ctx) -> f32;
}
```

Un oscillateur, un LFO, une enveloppe, une constante, un mixeur, **une bande de
fréquence audio** : tous implémentent `Signal`. Ils sont donc interchangeables.

C'est ce qui rend le système modulaire au sens propre : on ne code pas « le LFO peut
moduler la fréquence » puis « l'audio peut moduler la taille ». On code une fois
`Signal`, et **n'importe quelle source module n'importe quelle destination**.

### 2.2 Tout paramètre est modulable

```rust
enum Param {
    Fixed(f32),
    Modulated { base: f32, depth: f32, source: NodeId },
}
```

Un paramètre n'est pas un `f32`. C'est soit une valeur fixe, soit une valeur de base
plus la sortie d'un signal, mise à l'échelle.

**Conséquence de conception :** aucun champ numérique visible par l'utilisateur ne
doit être déclaré `f32`. S'il est réglable, il est `Param`. C'est la règle la plus
importante du projet, et la plus facile à oublier — un `f32` glissé aujourd'hui est
un câble impossible à brancher demain.

---

## 3. Le graphe de patch

Les signaux forment un **graphe orienté acyclique** (DAG), évalué à chaque frame.

**Représentation : une arena.** Les nœuds vivent dans un `Vec<Node>` et se référencent
par index (`NodeId`), pas par pointeur.

```rust
pub struct NodeId(u32);

pub struct Graph {
    nodes: Vec<Node>,
    order: Vec<NodeId>,      // tri topologique, recalculé au recâblage
    cache: Vec<f32>,         // une valeur par nœud, invalidée à chaque frame
}
```

**Pourquoi une arena plutôt que `Rc<RefCell<Node>>` :** c'est l'idiome Rust pour les
graphes. Pas de comptage de références, pas d'emprunts imbriqués à l'exécution, la
sérialisation d'un patch devient triviale, et le tri topologique est direct.

**Pourquoi un cache par frame :** un LFO branché sur cinq destinations doit être évalué
**une fois** par frame, pas cinq. Sans cache, un graphe profond explose en coût
exponentiel. Le cache est invalidé au début de chaque frame, pas plus.

**Cycles :** interdits, détectés au câblage et refusés avec un message clair. Le
feedback ne se fait pas dans le graphe de signaux — il se fait au niveau des couches
(§6), où il a un sens visuel et un délai d'une frame bien défini.

---

## 4. Le temps

Une seule horloge, qui a deux modes :

```rust
enum Clock {
    Realtime,          // le temps suit l'horloge murale
    Fixed { fps: f64 },// le temps avance d'un pas fixe par frame
}
```

**Pourquoi les deux dès le départ.** `Realtime` est nécessaire pour jouer en live et
pour l'audio. `Fixed` est nécessaire pour un rendu final déterministe : chaque frame
est calculée entièrement, aucune n'est perdue, le résultat est reproductible à
l'identique.

Poser cette distinction maintenant coûte trois lignes. La rétrofitter plus tard
oblige à traquer chaque appel à « quelle heure est-il » dans tout le code.

**Règle :** aucun module ne lit l'horloge système. Le temps est **toujours** reçu en
paramètre (`t`). C'est ce qui rend le rendu déterministe possible.

**Transport (optionnel, plus tard).** BPM, boucles et quantification se posent
au-dessus de `Clock` sans la modifier — un `Transport` qui traduit le temps en
mesures et temps musicaux.

---

## 5. Formes

### 5.1 Le trait

```rust
trait ShapeGenerator {
    fn generate(&self, t: f64, ctx: &Ctx) -> Vec<Vec2>;
}
```

Une forme est une **polyligne** : une suite de points. Simple, uniforme, et suffisant
pour tout ce qui suit.

### 5.2 Primitives

Chaque paramètre est un `Param`, donc modulable.

| Primitive | Paramètres |
|---|---|
| Cercle / ellipse | rayon, excentricité |
| Polygone | **nombre de côtés (flottant)**, rotation, arrondi des angles |
| Étoile | branches, ratio intérieur/extérieur |
| Rectangle | largeur, hauteur, rayon des coins |
| Ligne / arc | extrémités, courbure |

**Le détail qui compte :** le nombre de côtés du polygone est un **flottant modulable**.
Un polygone morphe alors continûment de triangle à carré à cercle. Cette seule
primitive couvre une grande partie du besoin.

### 5.3 Générateurs paramétriques

Ceux qui viennent directement du monde des oscillateurs :

| Générateur | Principe |
|---|---|
| Lissajous / harmonographe | deux oscillateurs → `(x, y)` ; rapports non entiers = organique |
| Rose polaire | un oscillateur module le rayon selon l'angle → pétales, étoiles |
| Superformule de Gielis | une équation, une grande part du règne végétal et minéral |
| Champ de flux | oscillateurs 2D → champ d'angles → traces de particules |

### 5.4 Opérateurs — c'est ici qu'est la richesse

Les primitives sont ennuyeuses seules. Les opérateurs les transforment, et se
composent en chaîne :

- **Déformation** par oscillateur le long de la normale → le cercle devient un blob organique
- **Répétition** radiale ou en grille, avec décalage modulable
- **Bruit** sur les sommets (Perlin, simplex)
- **Subdivision** et lissage

> **Où investir l'effort :** ajouter un opérateur multiplie les possibilités de toutes
> les primitives existantes. Ajouter une primitive n'ajoute qu'elle-même. En cas de
> doute, écrire un opérateur.

---

## 6. Couches et composition

C'est le point de rencontre entre deux natures incompatibles.

**Le problème.** Les formes sont **vectorielles** (des points). Les clips vidéo sont
**matriciels** (des pixels, à leur propre framerate). On ne peut pas les mettre dans
le même pipeline naïvement.

**La solution.** Ils se rejoignent à un seul endroit : **la texture**. Une forme
rasterisée est une texture. Une frame vidéo décodée est une texture.

```rust
trait Layer {
    fn render(&mut self, t: f64, ctx: &Ctx) -> &TextureView;
    fn blend(&self) -> BlendParams;   // opacité, mode — modulables par Signal
}
```

```
[ShapeLayer]     ─┐
[VideoLayer]     ─┼──▶ compositeur ──▶ texture finale
[FeedbackLayer]  ─┘
```

**Les trois implémentations, dans l'ordre de difficulté :**

1. **`ShapeLayer`** — rasterise des formes tessellées. C'est déjà l'architecture.
2. **`FeedbackLayer`** — réinjecte la frame précédente, transformée. Presque gratuit
   (deux textures qu'on échange), et visuellement très rentable.
3. **`VideoLayer`** — le vrai morceau (§9.3).

**Ce que les couches débloquent :** une forme peut servir de **masque** ou de
**déplacement UV** sur la vidéo. C'est là que vectoriel et matriciel deviennent
réellement intéressants ensemble, plutôt que simplement superposés.

---

## 7. Audio

L'audio n'est pas un module de plus : il impose une **contrainte de threading**.

**La contrainte.** Le callback audio est temps réel dur : pas d'allocation, pas de
mutex, pas de blocage. Le violer produit des clics audibles. Il ne peut donc pas
parler directement au thread de rendu.

```
[callback audio]  ──▶  ring buffer  ──▶  [analyse]  ──▶  triple buffer  ──▶  [rendu 60fps]
   temps réel dur       lock-free           FFT            lock-free          lecture seule
```

Le thread audio **ne fait que copier des échantillons**. L'analyse tourne à côté.

### Features extraites

| Feature | Usage visuel typique |
|---|---|
| RMS / loudness | échelle globale, épaisseur de trait |
| Bandes spectrales (grave / médium / aigu) | modulation par registre |
| Onset / transitoire | déclenchement, flash, saut de forme |
| Centroïde spectral | « brillance » → couleur, complexité |
| Chroma | mapping harmonique → palette |

### L'élégance du système

```rust
struct AudioFeature { feature: FeatureKind, smoothing: f32 }
impl Signal for AudioFeature { /* lit le triple buffer */ }
```

**Une feature audio est un `Signal`.** Aucune nouvelle abstraction, aucun câblage
spécial : l'audio se branche sur n'importe quel paramètre exactement comme un LFO.
Le système modulaire absorbe l'audio gratuitement.

**Lissage :** obligatoire. Les features brutes sont bruitées et produisent un visuel
nerveux. Attaque et release réglables par feature.

**Source :** entrée live (`cpal`) d'abord. La lecture de fichier peut s'ajouter ensuite
et apporte un avantage propre — connaître le futur du signal permet l'anticipation et
une synchro parfaite.

---

## 8. Sorties

**Décision structurante : le rendu ne va jamais directement à l'écran.** Il va dans une
**texture offscreen**, consommée ensuite par plusieurs sorties simultanées.

```
scène ──▶ texture RGBA offscreen ─┬──▶ fenêtre (preview)
                                  ├──▶ NDI / Spout / Syphon
                                  └──▶ encodeur vidéo (fichier)
```

**Bénéfice immédiat :** la résolution de sortie est découplée de la taille de fenêtre.
Enregistrer en 4K dans une fenêtre de 800 px devient naturel.

```rust
trait OutputSink {
    fn submit(&mut self, frame: &FrameTexture, t: f64) -> Result<()>;
}
```

### 8.1 Partage inter-applications

Aucune techno n'est multiplateforme, sauf NDI :

| Techno | Plateforme | Mécanisme |
|---|---|---|
| **NDI** | Linux / macOS / Windows | réseau, coût CPU d'encodage |
| **Spout** | Windows uniquement | partage GPU local, latence nulle |
| **Syphon** | macOS uniquement | partage GPU local, latence nulle |

```rust
mod ndi;                                    // partout
#[cfg(windows)]            mod spout;
#[cfg(target_os = "macos")] mod syphon;
```

**Stratégie : NDI d'abord.** Il fonctionne sur les trois plateformes, donne une sortie
utilisable partout immédiatement, et c'est ce que la plupart des logiciels VJ
consomment déjà. Spout et Syphon sont une **optimisation tardive** — ils exposent des
handles de textures natives (D3D11, IOSurface) dont la récupération depuis wgpu exige
de descendre dans `wgpu-hal`, en `unsafe` spécifique à chaque backend.

### 8.2 Enregistrement

**Le piège :** la lecture GPU → CPU est **asynchrone**. Lire la frame immédiatement
bloque le pipeline et effondre le framerate.

**La solution :** un anneau de buffers de readback avec 2 à 3 frames de latence — on
mappe la frame N-2 pendant que la frame N se dessine. C'est ce détail qui sépare 60 fps
de 20 fps pendant un enregistrement.

**Deux modes, adossés à `Clock` (§4) :**

- **Temps réel** — encodage pendant la performance ; des frames peuvent être perdues si
  l'encodeur décroche. Nécessaire en live.
- **Offline / déterministe** — pas de temps fixe, aucune frame perdue, qualité garantie,
  mais pas temps réel. Pour un rendu final propre.

**Encodage :** un pipe vers le binaire `ffmpeg` plutôt que des bindings. Plus simple à
démarrer, et surtout ffmpeg sélectionne lui-même l'encodeur matériel de chaque
plateforme (NVENC, VideoToolbox, Media Foundation) — un problème de portabilité qu'on
n'a pas à porter.

---

## 9. Portabilité

Cible : **Linux, macOS, Windows**, à parité.

### 9.1 Graphique
wgpu abstrait Vulkan / Metal / DX12. C'est la raison même de ce choix.
**Discipline :** rester en WGSL, ne jamais supposer un backend, tester tôt sur les trois.

### 9.2 Audio
`cpal` couvre ALSA / CoreAudio / WASAPI. **Piège classique :** les tailles de buffer et
les latences diffèrent nettement selon la plateforme. Ne jamais coder en supposant une
taille de buffer fixe.

### 9.3 Vidéo
Le décodage est le morceau le plus lourd du projet : thread de décodage dédié, buffer
de frames, upload GPU, synchronisation avec `Clock`, scrubbing et boucle, conversion
YUV → RGB en shader. Plusieurs jours de travail, **indépendants du reste** — d'où sa
place en dernier.

---

## 10. Structure des crates

Les crates vivent sous `crates/`, le workspace est à la racine. Seule `core` existe
à ce jour ; les suivantes sont créées au fil de la feuille de route (§12).

```
frogenx/
├── core/      # Signal, Param, Graph, Clock          ← aucune dépendance ✅
├── osc/       # oscillateurs, LFO, enveloppes, bruit
├── audio/     # cpal, FFT, features → Signal
├── shape/     # primitives + opérateurs
├── layer/     # trait Layer : Shape / Feedback / Video
├── render/    # wgpu, compositeur, post-fx
├── output/    # OutputSink : preview / NDI / Spout / Syphon / recorder
└── ui/        # egui : rack, câblage, monitoring
```

**Règle de dépendance :** les flèches vont vers le haut de la liste. `core` ne dépend
de rien. `shape` ne connaît ni wgpu ni l'audio. `render` ne sait pas ce qu'est un
oscillateur.

**Pourquoi cela compte :** `core`, `osc` et `shape` restent testables sans GPU, sans
carte son et sans fenêtre — donc en CI, rapidement. C'est ce qui garde le projet
maintenable à mesure qu'il grossit.

---

## 11. Dépendances

| Domaine | Crate | Rôle |
|---|---|---|
| Maths | `glam` | vecteurs, matrices |
| Rendu | `wgpu`, `winit` | GPU multiplateforme, fenêtrage |
| UI | `egui` | rack de modules, câblage |
| Bruit | `noise` | Perlin, simplex |
| Tessellation | `lyon` | polylignes → triangles |
| Audio | `cpal` | capture multiplateforme |
| FFT | `rustfft` | analyse spectrale |
| Lock-free | `rtrb`, `triple_buffer` | passage audio → rendu |
| Sérialisation | `serde` | sauvegarde de patchs |
| Encodage | binaire `ffmpeg` (pipe) | enregistrement |

---

## 12. Feuille de route

Chaque étape produit quelque chose de **visible et fonctionnel**. La stratégie est un
*vertical slice* : un chemin complet et fin d'abord, épaissi ensuite — plutôt que des
couches complètes empilées.

| # | Étape | Résultat visible |
|---|---|---|
| 1 | ✅ `Signal`, `Param`, `Clock` | valeurs testables, sans GPU — *fait* |
| 2 | ✅ Graphe de patch (arena, tri topo, cache) | modulation évaluable — *fait* |
| 3a | ✅ Oscillateurs, LFO, bruit, enveloppes | sources de modulation — *fait* |
| 3b | Primitives + opérateurs de formes | polylignes testables |
| 4 | `Layer` + `ShapeLayer` + compositeur, offscreen | **première image à l'écran** |
| 5 | `OutputSink` + preview | pipeline de sortie en place |
| 6 | LFO câblé sur une fréquence | **le modulaire prend vie** |
| 7 | Audio → features → `Signal` | **visuel audio-réactif** |
| 8 | `FeedbackLayer` | traînées, échos, complexité organique |
| 9 | Enregistrement (readback + ffmpeg) | fichier vidéo |
| 10 | NDI, puis Spout / Syphon | intégration VJ |
| 11 | `VideoLayer` | clips vidéo comme couches |

**Les étapes clés :** la 4 (première image), la 6 (le concept se prouve), la 7
(l'audio-réactivité, cœur du projet).

**Les étapes 4 et 5 sont l'investissement anticipé** du plan : elles coûtent peu
maintenant et évitent une réécriture complète du rendu quand la vidéo et les sorties
multiples arrivent.

---

## 13. Décisions et leur raison

Un résumé pour qui reprend le projet — ou pour vous-même dans six mois.

| Décision | Raison |
|---|---|
| Tout est `Signal` | rend n'importe quelle source capable de moduler n'importe quoi |
| Tout paramètre réglable est `Param` | un `f32` aujourd'hui est un câble impossible demain |
| Graphe en arena, pas de `Rc<RefCell>` | idiome Rust, sérialisation triviale, pas d'emprunts imbriqués |
| Cache d'évaluation par frame | évite le coût exponentiel des graphes profonds |
| `Clock` à deux modes dès le début | le rendu déterministe est impossible à rétrofitter |
| Le temps est toujours passé en paramètre | condition du déterminisme |
| Rendu offscreen, jamais direct à l'écran | permet sorties multiples et résolution découplée |
| Couches texturées | seul point de rencontre entre vectoriel et matriciel |
| Feedback au niveau couche, pas signal | délai d'une frame bien défini, DAG préservé |
| Audio via buffers lock-free | le callback temps réel ne peut ni allouer ni bloquer |
| NDI avant Spout / Syphon | seule techno des trois plateformes ; les autres sont `unsafe` et tardives |
| ffmpeg en pipe, pas en bindings | l'encodeur matériel par plateforme n'est plus notre problème |
| Vidéo en dernier | plusieurs jours de travail, indépendants du reste |
| **État des nœuds dans le graphe**, pas dans les modules (§16.1) | modules purs et testables, sérialisation et reset gratuits, pas d'emprunt mutable sur un nœud partagé |
| **`Ctx` porte le cache, pas `&Graph`** (§16.2) | le graphe mute l'état du nœud courant pendant `eval` ; et un `Signal` n'a pas à voir la topologie, seulement les valeurs de ses dépendances |
| **Phase accumulée**, pas `sin(2π·f·t)` (osc) | en FM, la formule directe fait sauter la phase à chaque changement de fréquence — discontinuité visible |
| **Bruit implémenté en interne**, pas la crate `noise` | le déterminisme à graine explicite (§20.3) doit être garanti, pas supposé à travers les versions d'une dépendance |
| **Écriture dans un buffer fourni** pour les formes (§16.3) | évite une allocation par forme et par frame à 60 fps |
| **Graphe évalué entièrement avant les formes** (§20.1) | `Param::get()` sans coût ni effet de bord ; deux formes lisant un LFO voient la même valeur |
| **Pas d'erreur en évaluation**, seulement en édition (§18.1) | un logiciel de scène ne panique pas ; les compteurs de diagnostic rendent les incohérences visibles sans interrompre |
| **Vérification du non-fini au cache**, un seul point (§18.3) | un `NaN` fait disparaître une forme sans message — le pire défaut : muet |
| **Saturation chez le consommateur**, jamais dans le `Signal` (§18.4) | saturer à la source détruirait la modulation pour tous les autres consommateurs |
| **Patchs versionnés avec migrations en chaîne** (§19) | un patch sauvegardé aujourd'hui s'ouvre dans six mois ; fixtures jamais régénérées |
| **JSON pour les patchs**, pas de binaire | se diffe dans git et se corrige à la main quand le chargement échoue |
| **`f64` pour `t`, `f32` ailleurs** (§17) | `t` accumule : en `f32`, les oscillateurs bégaient après ~1 h de session |
| **Espace des formes normalisé** `[-1, 1]` (§17) | un patch rendu en 720p ou 4K donne la même image |
| **Calcul en couleur linéaire**, sRGB à la sortie seule (§21) | mélanger en sRGB est faux ; sur un logiciel de dégradés et de superpositions, l'erreur est visible partout |
| **`Rgba16Float` en intermédiaire** (§21) | `Rgba8` linéaire produit du banding dans les tons sombres, là où les dégradés lents se jouent |
| **UI par commandes**, pas de mutation directe (§23) | donne l'annulation et le rejeu sans coopération du reste du code ; impossible à rétrofitter |
| **Snapshots avec tolérance perceptuelle** (§24.2) | un snapshot strict échoue chez le voisin et finit désactivé, donc inutile |

---

## 14. Comment faire évoluer ce document

- **Une décision change ?** Modifier la section *et* la ligne du §13. Ne pas empiler
  les couches d'obsolescence.
- **Un nouveau module ?** Il doit trouver sa place dans les quatre étages du §1. S'il
  n'y rentre pas, c'est que l'architecture doit bouger — et cela mérite d'être écrit ici.
- **Un piège découvert à l'usage ?** L'ajouter. Les §7, §8.2 et §9.2 existent parce que
  ces pièges coûtent cher quand on les rencontre sans avertissement.
- **Une nouvelle couche difficile à tester ?** C'est un signal d'alarme sur
  l'architecture, pas sur les tests. Voir §15.2.

---

## 15. Contrat de contribution

Deux règles **non négociables** sur ce projet. Elles s'appliquent à chaque
modification, sans exception de taille : un « petit changement » non documenté et non
testé est exactement la façon dont un projet devient illisible.

### 15.1 Toute fonctionnalité arrive avec ses tests unitaires

Aucune fonctionnalité n'est considérée terminée sans tests. Ce que cela signifie
concrètement, par étage du §1 :

| Étage | Ce qu'on teste |
|---|---|
| **Signaux** | valeurs exactes à des `t` connus, bornes, périodicité, cas limites (fréquence nulle, amplitude négative) |
| **Graphe** | tri topologique correct, détection de cycle, **cache : un nœud partagé n'est évalué qu'une fois** |
| **Paramètres** | `Fixed` constant, `Modulated` = `base + depth * source` |
| **Formes** | nombre de points, fermeture du contour, symétries, morphing continu du polygone |
| **Opérateurs** | composition, idempotence quand elle est attendue, préservation du nombre de points |
| **Audio** | features sur signaux **synthétiques** (sinus pur → centroïde connu ; silence → RMS nul) |
| **Sorties** | un `OutputSink` factice reçoit bien les frames, dans l'ordre, sans perte en mode `Fixed` |

**Ce qui rend cela possible :** la règle de dépendance du §10. `core`, `osc` et `shape`
ne connaissent ni GPU, ni carte son, ni fenêtre — ils sont donc testables en CI,
rapidement, sans matériel. **Cette règle n'est pas une élégance, c'est la condition de
la testabilité du projet.**

**Le déterminisme est un outil de test.** Le temps étant toujours passé en paramètre
(§4), un test fixe `t` et vérifie une valeur exacte. C'est la seconde raison d'être de
cette règle, après le rendu offline.

**Pour ce qui touche au GPU et au matériel :** injecter une abstraction (un `OutputSink`
factice, une source audio synthétique) plutôt que renoncer au test. Les tests
d'intégration réels restent utiles, mais ils ne remplacent pas les tests unitaires.

### 15.2 Toute modification met la documentation à jour

**Dans le même commit.** Une documentation mise à jour « plus tard » ne l'est jamais.

Ce qui doit suivre le code :

- Une **décision structurante** change → la section concernée **et** la ligne du §13
- Un **module** ajouté → §10, et §11 si une dépendance apparaît
- Une **étape** de la feuille de route franchie → §12
- Un **piège** rencontré → la section concernée (voir §14)
- Un **comportement observable** modifié → partout où il est décrit

**Le test de cohérence :** si quelqu'un lit ce document et est surpris par le code, le
document a un bug. Il se corrige comme un bug.

### 15.3 En résumé

> Une fonctionnalité est terminée quand le code marche, **que les tests le prouvent**,
> et **que la documentation le décrit**. Les trois, ou rien.

---
---

# Partie II — Contrats d'implémentation

> La partie I décide **quoi** et **pourquoi**. Cette partie décide **comment**, au
> niveau des signatures et des conventions. Elle existe pour qu'il n'y ait pas deux
> lectures possibles du même paragraphe au moment de coder.
>
> **Elle est plus volatile que la partie I.** Les §16 à §20 sont des contrats fermes
> — les changer casse du code. Les §22 et §23 sont des intentions raisonnées : ils
> seront revus à l'usage, et c'est normal.

---

## 16. Signatures et types fondamentaux

### 16.1 Le trait `Signal`

**Décision : l'état vit dans le graphe, pas dans le module.**

```rust
pub trait Signal {
    fn eval(&self, t: f64, ctx: &Ctx, state: &mut NodeState) -> f32;

    /// Combien de f32 de mémoire ce nœud réclame. 0 = sans état.
    fn state_size(&self) -> usize { 0 }
}
```

**Pourquoi.** Une enveloppe, un filtre, un détecteur d'onset ont de la mémoire entre
les frames. Trois options existaient ; celle-ci est retenue parce qu'elle donne :

- des **modules purs** — aucun champ mutable, donc testables en injectant un `NodeState`
  connu et en vérifiant la sortie *et* l'état résultant ;
- une **sérialisation gratuite** — l'état est un bloc de `f32` que le graphe possède ;
- un **reset gratuit** — remettre l'état à zéro, sans coopération des modules ;
- pas d'emprunt mutable sur le module pendant l'évaluation, donc un nœud partagé par
  cinq destinations reste trivial.

Le coût : `eval` prend un paramètre de plus, et un module à état doit indexer son
`NodeState` plutôt que lire `self.foo`. C'est un prix faible.

```rust
/// Tranche de mémoire persistante d'un nœud. Le graphe la possède.
pub struct NodeState<'a> { pub slots: &'a mut [f32] }
```

**Convention :** un module documente ses slots en tête d'implémentation
(`// slots[0] = phase accumulée`). Un slot non documenté est un bug de relecture.

### 16.2 Le contexte

```rust
pub struct Ctx<'a> {
    pub dt: f64,                      // durée de la frame, en secondes
    pub frame: u64,                   // compteur de frames depuis le début
    pub sample_rate: f64,             // fréquence d'échantillonnage audio
    pub audio: &'a AudioFeatures,     // instantané, lecture seule
    pub cache: &'a [f32],             // valeurs déjà calculées de la frame
}
```

`Ctx` est **construit une fois par frame** par le moteur, et passé en lecture seule.
Il ne contient jamais l'horloge système : `t` est un paramètre séparé, précisément
pour qu'un test puisse le fixer (§4).

**Pourquoi `cache: &[f32]` et non `&Graph`** — découvert en implémentant `osc` :
pendant `eval_frame`, le graphe mute le `NodeState` du nœud courant. Se prêter
lui-même en `&Graph` au même moment viole l'emprunteur. Le cache et l'état sont deux
champs **distincts** de `Graph`, donc les deux emprunts coexistent.

C'est aussi plus honnête sur le plan de la conception : un `Signal` n'a **pas** à voir
la topologie du graphe, seulement les valeurs déjà calculées de ses dépendances. La
contrainte du compilateur a révélé un couplage que le contrat initial autorisait à
tort.

`dt` est nécessaire aux modules à état — une enveloppe doit savoir combien de temps
s'est écoulé, et ne peut pas le déduire de `t` seul si elle veut rester correcte quand
une frame est longue.

### 16.3 Le trait `ShapeGenerator`

```rust
pub trait ShapeGenerator {
    fn generate(&self, t: f64, ctx: &Ctx, g: &Graph, out: &mut Vec<Vec2>);
}
```

**Écriture dans un buffer fourni, pas de `Vec` retourné.** Le rendu tourne à 60 fps ;
allouer un `Vec` par forme et par frame produit une pression mémoire inutile. Le
compositeur possède les buffers et les réutilise (`out.clear()` puis remplissage).

Le `&Graph` est nécessaire parce que les paramètres d'une forme sont des `Param`, dont
la résolution exige le graphe.

### 16.4 `Param`

```rust
pub enum Param {
    Fixed(f32),
    Modulated { base: f32, depth: f32, source: NodeId },
}

impl Param {
    /// Pour les consommateurs (formes, couches), APRÈS eval_frame.
    pub fn get(&self, g: &Graph) -> f32 { /* base + depth * g.value(source) */ }

    /// Pour un Signal, PENDANT son évaluation. Lit `ctx.cache`.
    pub fn eval(&self, ctx: &Ctx) -> f32 { /* base + depth * ctx.cache[source] */ }
}
```

**Deux voies, une seule sémantique.** Les deux lisent le **cache de la frame** (§3) et
n'évaluent rien ; elles diffèrent seulement par ce à quoi l'appelant a accès :

| Appelant | Méthode | Raison |
|---|---|---|
| Forme, couche — après `eval_frame` | `get(&Graph)` | a le graphe entier sous la main |
| `Signal` — pendant `eval_frame` | `eval(&Ctx)` | le graphe est partiellement emprunté (§16.2) |

Le graphe est entièrement évalué avant que les formes ne soient générées (§20). Un
`Param` ne peut donc pas déclencher une évaluation en cascade, ce qui garantit
l'absence de coût caché. Une source hors cache rend `0.0` — jamais de panique (§18.1).

### 16.5 Identifiants

```rust
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeId(u32);
```

Type distinct, pas un `usize` nu — un `NodeId` et un index de couche ne doivent jamais
être confondus par le compilateur. Même chose pour `LayerId`.

**Génération.** Un `NodeId` est un index dans l'arena **plus un compteur de
génération** stocké à part, pour que la suppression d'un nœud puis la création d'un
autre ne fasse pas pointer un ancien `NodeId` vers le nouveau nœud (§18.2).

---

## 17. Conventions numériques

Ces conventions sont **globales et non négociables**. Une seule violation rend les
modulations incohérentes entre modules, ce qui se manifeste par des visuels
inexplicables plutôt que par une erreur.

| Grandeur | Convention | Raison |
|---|---|---|
| **Sortie de `Signal`** | nominalement `[-1, 1]`, **non borné en pratique** | permet `base + depth * source` uniforme ; le non-bornage autorise sommes et FM |
| **Sortie unipolaire** | `[0, 1]` pour enveloppes et features audio | ce qui est naturellement positif le reste ; un `Bipolar`/`Unipolar` explicite existe pour convertir |
| **Fréquence** | **Hz** (cycles par seconde) | intuitif, cohérent avec l'audio |
| **Phase** | tours, `[0, 1)` | évite les `2π` partout ; conversion au dernier moment |
| **Angle géométrique** | **radians** | c'est ce qu'attendent `sin`/`cos` et `glam` |
| **Espace des formes** | **normalisé**, `[-1, 1]` sur le petit côté, origine au centre | indépendant de la résolution ; un patch rendu en 720p ou en 4K donne la même image |
| **Temps `t`** | secondes, `f64` | `f32` perd la précision après ~1 h de session |
| **Couleur** | linéaire, `f32` par canal | voir §21 |

**Sur le non-bornage.** Un `Signal` n'est pas contraint à `[-1, 1]` : une somme de deux
oscillateurs sort naturellement de l'intervalle, et l'écrêter par défaut détruirait la
FM. C'est au **consommateur** de saturer si son domaine l'exige (§18.3).

**Sur `f64` pour le temps et `f32` pour le reste.** `t` accumule ; les valeurs de
signal non. Un `f32` a ~7 chiffres significatifs : après une heure (3600 s), le pas de
temps d'une frame à 60 fps devient comparable à la précision disponible, et les
oscillateurs se mettent à bégayer. C'est un bug classique et difficile à diagnostiquer.

---

## 18. Modèle d'erreur

**Principe directeur : le logiciel est destiné à la scène. Il ne panique jamais en
lecture.** Mais un test doit détecter l'incohérence, sinon les bugs se cachent.

### 18.1 Deux régimes

| Régime | Comportement |
|---|---|
| **Édition** (câblage, chargement de patch, UI) | `Result<T, Error>`, message clair, l'opération est refusée |
| **Évaluation** (par frame, temps réel) | jamais d'erreur, jamais de panique, valeur de repli + compteur |

L'évaluation ne renvoie pas de `Result` : à 60 fps, une erreur par frame est un flot
inexploitable, et propager un `Result` dans tout le chemin chaud coûte en lisibilité
sans rien apporter.

```rust
pub struct Diagnostics {
    pub missing_node: u32,      // NodeId résolu vers rien
    pub non_finite: u32,        // NaN ou inf produit par un module
    pub clamped: u32,           // valeur saturée par son consommateur
}
```

Ces compteurs sont **remis à zéro à chaque frame** et affichés dans l'UI. Un compteur
non nul est visible sans interrompre la performance.

### 18.2 Référence pendante

Un `NodeId` dont la génération ne correspond plus (§16.5) résout vers **`0.0`**,
incrémente `missing_node`, et ne panique pas.

En **édition**, supprimer un nœud encore référencé est refusé avec la liste des
dépendants — c'est là qu'on traite le problème, pas en lecture.

### 18.3 Valeurs non finies

Un `NaN` se propage silencieusement et fait disparaître une forme entière sans
explication. C'est le pire mode de défaillance possible : muet.

**Règle : la sortie de chaque nœud est vérifiée une fois, à l'écriture dans le cache.**

```rust
let v = node.eval(t, ctx, state);
cache[i] = if v.is_finite() { v } else { diag.non_finite += 1; 0.0 };
```

Un seul point de contrôle, coût négligeable, et le `NaN` ne peut pas traverser le
graphe.

### 18.4 Domaines invalides

Un rayon négatif, un nombre de côtés inférieur à 3, une opacité hors `[0, 1]` : ce sont
des **conséquences légitimes d'une modulation trop profonde**, pas des bugs.

**Chaque consommateur satura son domaine** et incrémente `clamped` :

```rust
let sides = self.sides.get(g).clamp(2.0, 64.0);
let radius = self.radius.get(g).max(0.0);
```

**Ne jamais saturer dans le `Signal`** — seulement chez le consommateur qui connaît son
domaine. Saturer à la source détruirait la modulation pour tous les autres
consommateurs.

### 18.5 En test

Les tests unitaires assertent que `Diagnostics` est **entièrement à zéro** après une
frame nominale. Un compteur non nul dans un test nominal est un échec : c'est ainsi que
le mode « tolérant en scène » ne devient pas « silencieusement faux partout ».

---

## 19. Format de patch

**Décision : versionné, avec migrations.** Un patch sauvegardé aujourd'hui doit
s'ouvrir dans six mois.

### 19.1 Forme

```jsonc
{
  "version": 1,
  "meta": { "name": "...", "created": "2026-09-10T...", "app": "0.1.0" },
  "nodes": [
    { "id": 0, "gen": 0, "kind": "osc",
      "params": { "waveform": "sine",
                  "freq":  { "fixed": 0.5 },
                  "amp":   { "modulated": { "base": 1.0, "depth": 0.3, "source": 4 } } } }
  ],
  "layers": [ /* ... */ ],
  "state": null
}
```

**JSON, pas un format binaire.** Un patch se relit, se diffe dans git, se corrige à la
main quand le logiciel refuse de l'ouvrir. La taille et la vitesse de chargement sont
sans enjeu ici.

**L'état n'est pas sauvegardé par défaut** (`"state": null`). Un patch décrit une
*configuration*, pas un instant. Le sauvegarder est optionnel et explicite — utile pour
reprendre une performance exactement où elle s'est arrêtée.

### 19.2 Migrations

```rust
fn migrate(raw: Value) -> Result<Patch, Error> {
    let mut v = raw["version"].as_u64().ok_or(Error::NoVersion)?;
    let mut data = raw;
    while v < CURRENT_VERSION {
        data = match v {
            1 => migrate_1_to_2(data)?,
            2 => migrate_2_to_3(data)?,
            _ => return Err(Error::UnknownVersion(v)),
        };
        v += 1;
    }
    serde_json::from_value(data).map_err(Into::into)
}
```

Migrations **en chaîne**, une fonction par saut de version. Un patch v1 traverse
`1→2→3` — on n'écrit jamais de migration directe `1→3`, qui doublerait le travail à
chaque nouvelle version.

**Règle absolue :** tout changement du format incrémente `CURRENT_VERSION` **et**
ajoute sa fonction de migration **et** un test qui charge un patch de l'ancienne
version depuis `tests/fixtures/`. Les fixtures ne sont **jamais** régénérées — leur
valeur est précisément d'être anciennes.

Une version **supérieure** à `CURRENT_VERSION` est refusée avec un message clair : un
patch venu d'une version plus récente ne peut pas être deviné.

---

## 20. Cycle de frame et threads

### 20.1 L'ordre, strict

```
 1. Clock          → t, dt                     (Realtime ou Fixed)
 2. Audio          → lecture du triple buffer  (non bloquante)
 3. Ctx            → construction              (une fois)
 4. Graphe         → cache invalidé, évalué dans l'ordre topologique
 5. Formes         → generate() dans les buffers réutilisés
 6. Tessellation   → polylignes → triangles (lyon)
 7. Couches        → chacune rend vers sa texture
 8. Composition    → mélange vers la texture finale
 9. Sorties        → chaque OutputSink reçoit la frame
10. Diagnostics    → publiés vers l'UI, puis remis à zéro
```

**Le graphe est entièrement évalué (4) avant que la moindre forme ne soit générée
(5).** C'est ce qui rend `Param::get()` sans coût et sans effet de bord, et ce qui
garantit que deux formes lisant le même LFO voient **exactement** la même valeur. Sans
cette séparation, l'ordre de génération influencerait le résultat.

### 20.2 Les threads

```
┌─ thread audio ──────┐   temps réel dur : ni allocation, ni mutex, ni I/O
│  callback cpal      │
│  → rtrb (SPSC)      │
└──────────┬──────────┘
           ▼
┌─ thread analyse ────┐   FFT, features, lissage
│  → triple_buffer    │
└──────────┬──────────┘
           ▼
┌─ thread principal ──┐   étapes 1 à 9, UI egui
│  lecture seule      │
└──────────┬──────────┘
           ▼
┌─ thread encodeur ───┐   readback GPU → pipe ffmpeg
└─────────────────────┘
```

**Règles de thread, chacune correspondant à un mode de défaillance réel :**

- Le callback audio **ne fait que copier** dans le ring buffer. Toute autre opération
  produit des clics audibles.
- Le thread principal **ne bloque jamais** sur l'audio. `triple_buffer` rend la
  dernière valeur disponible ; si l'analyse est en retard, on relit la précédente.
- L'encodeur a **2 à 3 frames de retard** (§8.2) et ne doit jamais faire attendre le
  rendu. S'il décroche en mode `Realtime`, on perd une frame — c'est le comportement
  voulu. En mode `Fixed`, le rendu attend : aucune frame n'est perdue.

### 20.3 Déterminisme

En mode `Clock::Fixed`, le rendu doit être **reproductible bit à bit** entre deux
exécutions. Cela impose :

- aucune lecture de l'horloge système hors de `Clock` ;
- aucun parcours de `HashMap` dans le chemin d'évaluation (ordre non déterministe) —
  utiliser `Vec` ou `BTreeMap` ;
- toute génération aléatoire passe par un PRNG **à graine explicite** stockée dans le
  patch, jamais `rand::random()`.

Ce déterminisme sert deux fois : le rendu offline propre, et les tests à valeur exacte.

---

## 21. Couleur

**Décision : tout le calcul en linéaire, conversion sRGB uniquement à la sortie.**

Le mélange de couleurs en sRGB est mathématiquement faux — un dégradé entre deux
couleurs vives passe par une zone terne, et deux couches à 50 % ne donnent pas la
moyenne perçue. Sur un logiciel dont la production principale **est** faite de dégradés
et de superpositions, l'erreur est visible partout.

| Étage | Espace |
|---|---|
| Palettes, UI, patchs | sRGB (ce que l'humain lit et saisit) |
| Textures de couches, composition, post-fx | **linéaire**, `Rgba16Float` |
| Sortie écran / NDI / fichier | sRGB (conversion finale) |

**Format des cibles offscreen : `Rgba16Float`.** Le `Rgba8Unorm` en linéaire produit du
banding dans les tons sombres, précisément là où les dégradés lents de ce logiciel se
jouent. Le coût mémoire est le double — acceptable.

**Le piège wgpu :** un format de texture `*Srgb` fait convertir le matériel
automatiquement à la lecture et à l'écriture. Mélanger une cible `Rgba8UnormSrgb` et
une `Rgba16Float` dans la même chaîne produit une double conversion — image délavée ou
trop sombre. **Une seule règle : tout ce qui est intermédiaire est `Rgba16Float`
linéaire ; seule la surface de présentation est `*Srgb`.**

**Alpha prémultiplié** dans toute la chaîne de composition — sinon les bords des formes
tessellées présentent un liseré sombre au mélange.

---

## 22. Contrat des couches

> Section d'intention : elle sera précisée à l'usage, à partir de l'étape 4.

```rust
pub trait Layer {
    fn render(&mut self, t: f64, ctx: &Ctx, g: &Graph, gpu: &Gpu) -> &TextureView;
    fn blend(&self) -> BlendParams;
    fn resize(&mut self, size: UVec2, gpu: &Gpu);
}

pub struct BlendParams {
    pub opacity: Param,          // modulable
    pub mode: BlendMode,         // Normal, Add, Multiply, Screen, Difference...
}
```

**Propriété des textures.** Chaque couche possède la sienne et la prête en lecture au
compositeur (`&TextureView`). Le compositeur ne libère jamais une texture qu'il n'a pas
créée.

**Résolution.** Toutes les couches partagent la résolution de la composition, fixée par
la sortie et non par la fenêtre (§8). `resize` est appelé sur changement, hors de la
boucle de rendu — jamais pendant une frame.

**`FeedbackLayer`** possède **deux** textures et les échange à chaque frame : on lit la
frame N-1 pendant qu'on écrit la N. Lire et écrire la même texture est interdit et
produit un résultat indéfini selon le backend.

**`VideoLayer`** ne décode pas dans le thread principal. Le décodeur pousse des frames
prêtes dans une file ; `render` prend la plus proche de `t` et **ne bloque jamais**. Si
aucune frame n'est prête, on réaffiche la précédente.

---

## 23. UI et graphe

> Section d'intention : elle sera précisée à l'étape où l'UI arrive.

egui est **immédiat** et tourne dans le thread principal, dans la même boucle que le
rendu. Il n'y a donc pas de concurrence à gérer — mais il y a un ordre à respecter.

**L'UI ne modifie jamais le graphe pendant son évaluation.** Elle produit des
**commandes** appliquées entre deux frames :

```rust
pub enum Command {
    AddNode(NodeKind),
    RemoveNode(NodeId),
    Connect { from: NodeId, to: NodeId, param: ParamRef },
    SetParam { node: NodeId, param: ParamRef, value: Param },
    LoadPatch(PathBuf),
}
```

**Pourquoi ce détour** plutôt qu'une mutation directe : c'est ce qui donne
l'annulation (undo) — une pile de commandes inversées — et le rejeu, sans coopération
du reste du code. Poser ce modèle tard obligerait à réécrire toute l'UI.

Les commandes de **câblage** sont validées (détection de cycle, §3) et peuvent être
**refusées** avec un message — c'est le régime « édition » du §18.1.

---

## 24. Stratégie de test

Le §15.1 dit *quoi* tester par étage. Cette section dit *comment*, pour ce que les
tests unitaires purs ne couvrent pas.

### 24.1 Les trois niveaux

| Niveau | Portée | Où |
|---|---|---|
| **Unitaire** | un module, sans GPU ni audio | `core`, `osc`, `shape` — la majorité |
| **Intégration** | une frame complète, sortie factice | moteur, sans fenêtre |
| **Snapshot d'image** | rendu réel comparé à une image de référence | nécessite un GPU |

### 24.2 Tests de snapshot

Un patch de référence est rendu en `Clock::Fixed`, à une frame précise, et comparé à un
PNG stocké dans `tests/snapshots/`.

**Tolérance obligatoire.** Les GPU ne sont pas identiques au bit près entre pilotes et
plateformes. La comparaison se fait sur une **différence perceptuelle moyenne** avec un
seuil, jamais sur une égalité stricte. Un test de snapshot strict échoue sur la machine
du voisin et finit désactivé — donc inutile.

À l'échec, écrire les trois images (référence, obtenue, différence amplifiée) dans un
répertoire d'artefacts : un test de snapshot qui dit seulement « différent » ne sert à
rien.

### 24.3 CI

Les tests unitaires tournent sur **les trois plateformes** — c'est là qu'on attrape les
`cfg` cassés que `cargo check` local ne compile pas (§9). Les tests de snapshot ne
tournent qu'où un GPU est disponible, et leur absence ailleurs ne fait pas échouer la
CI.

### 24.4 Fixtures de patchs

`tests/fixtures/` contient un patch par version de format (§19.2). **Jamais
régénérées** — un fichier v1 écrit aujourd'hui doit rester tel quel pour prouver dans
deux ans que la migration fonctionne encore.
