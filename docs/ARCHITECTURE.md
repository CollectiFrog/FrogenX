# Architecture — FrogenX

> Mixeur vidéo et scène 3D live, organisé en grille de clips déclenchables, où
> chaque clip est piloté comme un synthétiseur modulaire.

**Statut du document :** vivant. Il décrit les décisions structurantes et le *pourquoi*
de chacune. Il doit être modifié quand une décision change — un document d'architecture
qui ment est pire que pas de document.

**Réorientation (voir §13 pour le détail des décisions) :** FrogenX part d'un générateur
de visuels procéduraux 2D vers un outil de mixage vidéo live avec scènes 3D, organisé
autour d'une **grille de clips** (inspirée de Resolume Arena) plutôt que d'un patch
modulaire unique. Le graphe `Signal`/`Param` ne disparaît pas : il devient le moteur de
pilotage *à l'intérieur* de chaque clip, layer et effet — exactement le rôle qu'il jouait
déjà, mais désormais invoqué depuis une structure d'organisation explicite plutôt que
comme unique niveau du logiciel.

---

## 1. L'idée en une phrase

Une **grille de clips** (decks, layers, colonnes) organise ce qui joue et quand. Chaque
**clip** contient une **source** (forme procédurale, scène 3D, vidéo) et une chaîne
d'**effets**, tous deux pilotés par des **signaux** qui modulent des **paramètres**.
Les layers se composent en **couches**, envoyées vers plusieurs **sorties** simultanées.

```
   Grille ──organise──▶ Clip ──contient──▶ Source + Effets ──module──▶ via Signal/Param
  (Deck,                (quoi joue,       (forme, scène 3D,          (le graphe existant,
   Layer,                 quand,           vidéo + chaîne             désormais local
   Column)                comment)         d'opérateurs)              à chaque clip)
                                                   │
                                                   ▼
                                    Couches ──compose──▶ Sorties
                                   (compositeur de         (écran, NDI,
                                    layers/groupes)          fichier, Art-Net)
```

Cinq étages : la grille qui organise, le clip qui contient, le graphe qui module, les
couches qui composent, les sorties qui diffusent. Tout ce qui suit les détaille.

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

**Un `Graph` par clip, pas un graphe global.** Chaque `Clip` (§3bis) possède son propre
`Graph` pour piloter sa source et ses effets. Deux clips ne partagent pas de nœuds : un
LFO réglé sur un clip n'existe pas pour les autres. C'est délibéré — un clip doit rester
déplaçable, dupliquable et sérialisable seul, sans graphe de dépendances externe à
résoudre. Un besoin de modulation inter-clips (un LFO qui pilote plusieurs clips à la
fois) se pose plus tard comme une extension explicite, pas comme le cas par défaut.

---

## 3bis. La grille de composition

**Décision structurante : FrogenX organise ce qui joue et quand autour d'une grille de
clips déclenchables, inspirée de Resolume Arena** — plutôt que d'exposer un unique patch
modulaire comme surface de premier niveau. Le graphe `Signal`/`Param`/`Graph` du §2 et
du §3 ne disparaît pas : il devient le moteur de pilotage interne de chaque clip, layer
et effet. La grille répond à « quoi joue, quand, sur quelle couche » ; le graphe répond
à « comment ce qui joue évolue dans le temps ».

### 3bis.1 La hiérarchie

```
Composition
 ├─ Deck[]            un jeu complet de layers, interchangeable à chaud
 │   └─ LayerGroup[]  pilote plusieurs layers comme un seul objet
 │       └─ Layer[]   piste verticale : vidéo + audio, blend, autopilot
 │           └─ Clip[]  source + effets + transport, pilotable par son Graph
 └─ Column[]          déclenche la même tranche sur tous les layers à la fois
```

**Pourquoi un `Deck` plutôt qu'un patch unique actif.** Un VJ change de répertoire
entier de visuels en plein set sans interrompre la diffusion — charger un nouveau deck
pendant qu'un autre continue de tourner sur une sortie. Un patch modulaire unique ne
permet pas ça : il faudrait décâbler puis recâbler en live. Le `Deck` rend ce changement
atomique et réversible.

**Pourquoi `Column` est transversal plutôt qu'une propriété de `Layer`.** Une colonne
déclenche le même instant dramaturgique (« scène 3 ») sur tous les layers simultanément
— un déclenchement par layer ne peut pas exprimer ça sans dupliquer la logique de
synchronisation sur chaque layer. C'est le même besoin qu'une scène sur un contrôleur
d'éclairage : une colonne, canaux multiples.

### 3bis.2 Le `Clip`, point de jonction avec le graphe modulaire

```rust
pub struct Clip {
    pub source: ClipSource,       // Shape(Graph) | Scene3d(Graph) | Video(VideoTrack)
    pub effects: Vec<ShapeOp>,    // ou son équivalent vidéo/3D — chaîne en place (§16.3 bis)
    pub transport: Transport,     // Timeline ou BpmSync (§3bis.3)
    pub transition: ClipTransition,
}
```

**Un clip n'est pas qu'un fichier vidéo.** Il peut contenir une forme procédurale
(héritage direct de `shape/`), une scène 3D, ou un flux vidéo décodé — la distinction
se fait sur `ClipSource`, pas sur le type `Clip` lui-même. C'est ce qui permet à FrogenX
de rester différent d'un simple lecteur de clips : un clip procédural module ses propres
paramètres via son `Graph`, exactement comme avant la réorientation.

**Chaque clip porte son `Graph`** (voir le changement au §3). Les effets d'un clip sont
une chaîne `ShapeOp`-like en place, cohérente avec la décision déjà prise en §16.3 bis —
elle s'étend aux effets vidéo et 3D sans remettre en cause le contrat.

### 3bis.3 Transport et tempo

```rust
pub enum Transport {
    Timeline { position: Param, speed: Param },
    BpmSync { position: Param, duration_beats: Param, play_mode: PlayMode },
}

pub struct TempoController {
    pub tempo: Param,              // BPM
    pub tap: EventSink,            // tempo tap
    pub resync: EventSink,
}
```

**Pourquoi un `TempoController` de premier niveau, pas juste `Clock::Fixed`/`Realtime`
(§4).** `Clock` répond à « quelle heure est-il » pour le rendu déterministe. Le tempo
répond à « combien de temps dure un beat maintenant », une donnée musicale que plusieurs
clips et layers doivent partager en même temps, et qui change en live (tap tempo, push/
pull de synchro avec un DJ). C'est une couche au-dessus de `Clock`, pas une redéfinition.

### 3bis.4 `CrossFader`

```rust
pub struct CrossFader {
    pub phase: Param,              // 0 = groupe A, 1 = groupe B
    pub curve: FadeCurve,
}
```

**Pourquoi un fader global plutôt que seulement `BlendParams` par couche (§22).**
`BlendParams::opacity` mélange une couche avec ce qu'il y a dessous, couche par couche.
Le `CrossFader` mélange deux *groupes* de layers entiers en une seule commande — le
geste de régie classique (main gauche sur A, main droite sur B). Les deux coexistent :
le fader est une commande de haut niveau qui finit par piloter des `Param` d'opacité
existants, pas un nouveau pipeline de composition.

### 3bis.5 Conséquence sur le format de patch (§19)

Le format de patch versionné (§19) s'étend pour porter la grille : un patch décrit
désormais une `Composition` (decks, layers, columns), chaque clip sérialisant son
`Graph` comme avant. C'est un changement de schéma, donc une migration (§19.2) au
moment de l'implémenter — pas une refonte du mécanisme de versionnage lui-même.

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
[Scene3dLayer]   ─┤
[VideoLayer]     ─┼──▶ compositeur ──▶ texture finale
[FeedbackLayer]  ─┘
```

**Les implémentations, dans l'ordre de difficulté :**

1. **`ShapeLayer`** — rasterise des formes tessellées. C'est déjà l'architecture.
2. **`FeedbackLayer`** — réinjecte la frame précédente, transformée. Presque gratuit
   (deux textures qu'on échange), et visuellement très rentable.
3. **`VideoLayer`** — le vrai morceau (§9.3).
4. **`Scene3dLayer`** — rend une scène 3D (`scene3d`, §10) dans sa propre texture ;
   rejoint le compositeur exactement comme les autres, le point de rencontre décrit
   plus haut ne change pas en passant de 2D à 3D. Arrive en dernier (§12, étape 13) :
   le culling/batching pour les hautes densités de polygones est un investissement
   propre, indépendant du reste de la chaîne de composition.

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
├── osc/       # oscillateurs, LFO, enveloppes, bruit ✅
├── shape/     # primitives + opérateurs              ✅ (core, osc, glam)
├── scene3d/   # maillages, caméra, lumières, culling (core, osc, glam)
├── audio/     # cpal, FFT, features → Signal
├── composition/ # Deck, LayerGroup, Layer, Column, Clip, CrossFader, TempoController (core)
├── layer/     # trait Layer : Shape / Scene3d / Feedback / Video
├── render/    # wgpu, compositeur, post-fx
├── output/    # OutputSink : preview / NDI / Spout / Syphon / Art-Net / recorder
└── ui/        # egui : grille de clips, rack par clip, monitoring
```

**`composition` ne dépend que de `core`.** La grille (§3bis) doit rester testable sans
GPU ni audio, pour la même raison que `core`/`osc`/`shape` : trier des decks, détecter
des conflits de connexion sur une colonne, valider une transition de clip — tout cela
se vérifie sur des structures de données pures. `composition` connaît l'existence d'un
`Graph` par clip (il en possède un par `Clip`), mais jamais wgpu, cpal ou le rendu.

**`scene3d` suit la même règle que `shape`.** Primitives 3D (maillages, caméra,
lumières) et opérateurs, sans connaître wgpu ni l'audio — exactement le rôle que
`shape` jouait pour le 2D, étendu à la 3D plutôt que remplacé.

**Règle de dépendance :** les flèches vont vers le haut de la liste. `core` ne dépend
de rien. `shape`/`scene3d` ne connaissent ni wgpu ni l'audio. `render` ne sait pas ce
qu'est un oscillateur.

**Pourquoi cela compte :** `core`, `osc` et `shape` restent testables sans GPU, sans
carte son et sans fenêtre — donc en CI, rapidement. C'est ce qui garde le projet
maintenable à mesure qu'il grossit.

---

## 11. Dépendances

| Domaine | Crate | Rôle |
|---|---|---|
| Maths | `glam` ✅ | vecteurs, matrices |
| Rendu | `wgpu`, `winit` | GPU multiplateforme, fenêtrage |
| UI | `egui` | grille de clips, rack de modules, câblage |
| Bruit | `noise` | Perlin, simplex |
| Tessellation | `lyon` | polylignes → triangles |
| Audio | `cpal` | capture multiplateforme |
| FFT | `rustfft` | analyse spectrale |
| Lock-free | `rtrb`, `triple_buffer` | passage audio → rendu |
| Sérialisation | `serde` | sauvegarde de patchs |
| Encodage | binaire `ffmpeg` (pipe) | enregistrement, décodage de clips (§9.3) |
| Éclairage | `artnet` ou équivalent | sortie Art-Net / DMX (§3bis, §8) |
| Scripting | `mlua` | logique de composition personnalisée (§23.4) |
| Contrôle live | `midir` (MIDI), `rosc` (OSC) | pilotage de l'interface et des effets par surface externe (§22bis.1) |

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
| 3b | ✅ Primitives + opérateurs de formes | polylignes testables — *fait* |
| 4 | `composition` : `Deck`/`LayerGroup`/`Layer`/`Column`/`Clip`, `CrossFader`, `TempoController` (§3bis) | **la grille existe, sans rendu** |
| 5 | `Layer` + `ShapeLayer` + compositeur, offscreen | **première image à l'écran** |
| 6 | `OutputSink` + preview | pipeline de sortie en place |
| 7 | LFO câblé sur une fréquence, depuis un clip de la grille | **le modulaire prend vie dans la grille** |
| 8 | Audio → features → `Signal` | **visuel audio-réactif** |
| 9 | `FeedbackLayer` | traînées, échos, complexité organique |
| 10 | Enregistrement (readback + ffmpeg) | fichier vidéo |
| 11 | NDI, puis Spout / Syphon | intégration VJ |
| 12 | `VideoLayer` | clips vidéo comme couches |
| 13 | `scene3d` : maillages, caméra, lumières, culling | scène 3D comme source de clip |
| 14 | Art-Net / DMX en sortie | pilotage d'éclairage physique |
| 15 | Scripting Lua (`mlua`) sur `Command` (§23.4) | logique de composition scriptable |
| 16 | Shaders d'effets custom en WGSL natif (§23.4) | effets utilisateur sans recompiler FrogenX |
| 17 | Entrées MIDI/OSC, `Param3d::Bind` sur `scene3d` (§22bis.1) | pilotage live de l'interface et des effets par surface de contrôle externe |

**Les étapes clés :** la 4 (la grille devient le squelette organisationnel, avant même
le rendu — tout ce qui suit s'y raccroche), la 5 (première image), la 7 (le concept
modulaire se prouve *dans* la grille), la 8 (l'audio-réactivité, cœur du projet).

**L'étape 4 passe devant le rendu, à rebours du vertical slice habituel**, parce que
la grille est désormais la structure que toutes les étapes suivantes organisent. Y
raccrocher `Layer`/`OutputSink`/`VideoLayer` après coup serait une réécriture ; les
construire directement au-dessus d'une grille déjà en place ne l'est pas.

**Les étapes 5 et 6 restent l'investissement anticipé** du plan : elles coûtent peu
maintenant et évitent une réécriture complète du rendu quand la vidéo et les sorties
multiples arrivent. La 3D (étape 13) et l'éclairage (étape 14) viennent après que le
pipeline vidéo/audio-réactif est prouvé, pour la même raison qu'avant la réorientation
la vidéo venait en dernier (§9.3) : travail conséquent, largement indépendant du reste.

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
| **Grille de clips (Deck/LayerGroup/Layer/Column) comme structure d'organisation** (§3bis) | répond à « quoi joue, quand » ; un patch modulaire unique ne permet pas de changer de répertoire de visuels à chaud en live |
| **Un `Graph` par `Clip`, pas un graphe global** (§3) | un clip reste déplaçable, dupliquable, sérialisable seul ; la modulation inter-clips est une extension explicite, pas le cas par défaut |
| **`Column` transversal**, pas une propriété de `Layer` (§3bis.1) | un déclenchement par layer ne peut pas exprimer une scène synchronisée sur tous les layers sans dupliquer la logique |
| **`TempoController` au-dessus de `Clock`**, pas une redéfinition (§3bis.3) | `Clock` répond à l'heure pour le déterminisme ; le tempo répond à la durée d'un beat, partagée et modifiable en live (tap, resync) |
| **`CrossFader` global en plus de `BlendParams` par couche** (§3bis.4) | le geste de régie mélange deux groupes entiers ; `BlendParams` mélange une couche avec celle du dessous — les deux répondent à des échelles différentes |
| **`composition` ne dépend que de `core`** (§10) | la grille doit rester testable sans GPU ni audio, pour la même raison que `core`/`osc`/`shape` |
| **La grille (étape 4) précède le rendu (étape 5)** (§12) | toutes les étapes suivantes s'y raccrochent ; construire par-dessus une grille déjà en place évite une réécriture |
| **Lua (`mlua`) pour le scripting de composition**, pas Rhai (§23.4) | maturité et vitesse l'emportent sur la pureté Rust pour un langage écrit en direct par l'utilisateur ; Rhai rapporté plus lent et sans LSP |
| **`egui`, pas `iced`, pour la crate `ui`** (§23) | intégration wgpu directe (texture native en quelques lignes), écosystème node-graph existant (`egui_node_graph`, `egui-snarl`), et mode immédiat qui lit/écrit l'état du `Graph` sans state parallèle — `iced` imposerait une machine `Message`/`update()` en plus, contre l'invariant « l'état vit dans le graphe » |
| **`scene3d` modulée en direct par `Signal`/`Param`**, pas un flux déjà joué (§22bis) | TouchDesigner — seule référence du projet qui fait du 3D node-based en live — recuit toujours sa scène depuis l'état courant des paramètres au moment du Render TOP ; aucun raccourci de pré-rendu documenté |
| **Instancing comme primitive de `scene3d`**, pas un système de particules séparé (§22bis) | modèle confirmé chez TouchDesigner (Geometry COMP, page Instance) pour le cas « beaucoup d'objets » visé à l'étape 13 |
| **WGSL natif pour les shaders d'effets custom**, pas ISF adapté (§23.4) | le crate `isf` ne parse que les métadonnées (abandonné depuis 2020), le frontend GLSL de `naga` est lui-même qualifié « support secondaire » et incompatible avec le dialecte ISF ; transpiler la syntaxe ne suffirait pas, il faudrait reconstruire le runtime ISF (buffers multi-passes, persistants) — un sous-projet de semaines, pas un adaptateur |
| **`Param::Bind` (bidirectionnel) ajouté dans `scene3d` seulement**, `core`/2D restent unidirectionnels (§22bis.1) | le pilotage live par MIDI/OSC et la manipulation directe d'une caméra 3D exigent une resynchronisation dans les deux sens (éviter le *parameter jump*) ; la génération 2D reste de la modulation pure sans ce geste de pilotage manuel, `Modulated` suffit |
| **Pool de textures scratch réutilisables par format** (§22) | évite l'allocation par frame pour les buffers temporaires (snapshots, masques), généralise le ping-pong déjà en place pour `FeedbackLayer` |
| **État des nœuds dans le graphe**, pas dans les modules (§16.1) | modules purs et testables, sérialisation et reset gratuits, pas d'emprunt mutable sur un nœud partagé |
| **`Ctx` porte le cache, pas `&Graph`** (§16.2) | le graphe mute l'état du nœud courant pendant `eval` ; et un `Signal` n'a pas à voir la topologie, seulement les valeurs de ses dépendances |
| **Phase accumulée**, pas `sin(2π·f·t)` (osc) | en FM, la formule directe fait sauter la phase à chaque changement de fréquence — discontinuité visible |
| **Bruit implémenté en interne**, pas la crate `noise` | le déterminisme à graine explicite (§20.3) doit être garanti, pas supposé à travers les versions d'une dépendance |
| **`ShapeOp` en place**, distinct de `ShapeGenerator` (§16.3 bis) | des opérateurs enveloppants exigeraient un buffer par maillon — l'allocation par frame que le §16.3 évite |
| **Contour non refermé par duplication** (§16.3) | un point répété dupliquerait un sommet à la tessellation et produirait des doublons sous répétition radiale |
| **Saturation de la géométrie chez chaque primitive** (§18.4) | une modulation dégénérée doit dégrader la forme, jamais allouer des millions de points ni figer l'application |
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
la résolution exige le graphe. Une forme est générée **après** `eval_frame` (§20.1),
donc elle utilise `Param::get(&Graph)` et non `Param::eval(&Ctx)` (§16.4).

**Convention de fermeture :** une forme fermée ne répète **pas** son premier point à la
fin. C'est au rasteriseur de refermer le contour. Répéter le point dupliquerait un
sommet à la tessellation et compliquerait tous les opérateurs — une répétition radiale
sur 6 branches produirait 6 doublons.

**Convention d'espace :** coordonnées normalisées, origine au centre, `[-1, 1]` sur le
petit côté (§17).

### 16.3 bis Le trait `ShapeOp` — les opérateurs

Décision prise en implémentant `shape`, que la partie I ne tranchait pas.

```rust
pub trait ShapeOp {
    fn apply(&self, t: f64, ctx: &Ctx, g: &Graph, pts: &mut Vec<Vec2>);
}
```

**Pourquoi un trait distinct plutôt qu'un opérateur enveloppant un générateur.**
L'enveloppement (`Deform::new(Circle)`) donnerait une composition naturelle, mais chaque
maillon de la chaîne aurait besoin de son **propre buffer** pour recevoir la sortie du
précédent — exactement l'allocation par frame que le §16.3 cherche à éviter.

Un opérateur travaille donc **en place**. Une chaîne de dix opérateurs n'alloue rien.
`Chain` assemble un générateur et une suite d'opérateurs, et implémente lui-même
`ShapeGenerator` — la composition reste donc transparente pour l'appelant.

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
  "version": 2,
  "meta": { "name": "...", "created": "2026-09-10T...", "app": "0.1.0" },
  "decks": [
    { "id": 0, "name": "Set principal",
      "layers": [
        { "id": 10, "clips": [
          { "id": 100, "source": "shape",
            "nodes": [
              { "id": 0, "gen": 0, "kind": "osc",
                "params": { "waveform": "sine",
                            "freq":  { "fixed": 0.5 },
                            "amp":   { "modulated": { "base": 1.0, "depth": 0.3, "source": 4 } } } }
            ],
            "effects": [ /* ... */ ]
          }
        ] }
      ]
    }
  ],
  "columns": [ /* ... */ ],
  "tempo": { "bpm": 120.0 },
  "state": null
}
```

**Chaque clip porte son propre tableau `nodes`** (§3) — pas un graphe global partagé.
Un clip copié-collé d'un deck à l'autre emporte son graphe avec lui, sans résoudre de
dépendances externes.

**JSON, pas un format binaire.** Un patch se relit, se diffe dans git, se corrige à la
main quand le logiciel refuse de l'ouvrir. La taille et la vitesse de chargement sont
sans enjeu ici.

**Migration v1 → v2 :** le format v1 (un seul graphe global, `layers` plat) se migre en
enveloppant le patch dans un deck unique et en rattachant le graphe existant au premier
clip — voir §19.2. La fixture `tests/fixtures/patch_v1.json` n'est jamais régénérée.

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

> Section d'intention : elle sera précisée à l'usage, à partir de l'étape 5 (§12).

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

**Relation avec `CrossFader` (§3bis.4).** `BlendParams` reste la seule donnée que le
compositeur lit pour mélanger une couche avec celle du dessous. Le `CrossFader` ne
contourne pas le compositeur : sa `phase` pilote les `opacity` des deux groupes de
layers qu'il relie, via leurs `Param::Modulated`. Le compositeur n'a donc jamais
conscience d'un fader global — seulement des `BlendParams` qui en résultent.

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

**Pool de textures scratch réutilisables, un pool par format.** Inspiré d'une lecture
du crate `darkly` (compositeur wgpu open source) : plutôt que d'allouer une texture
par besoin temporaire (snapshot de `FeedbackLayer`, buffer intermédiaire d'un effet),
un pool par format (ex. un pool `Rgba16Float` pour les couches, un pool dédié pour les
masques) prête et reprend des textures, avec une règle stricte — **une texture du pool
n'est jamais réutilisée tant qu'une référence active la cite**. Même logique que le
ping-pong déjà en place pour `FeedbackLayer`, généralisée : évite la pression mémoire
d'une allocation par frame tout en gardant la garantie lire/écrire différente.
**Limite de cette référence :** la lecture de `darkly` s'est arrêtée à sa documentation
publique, pas à son code source complet — son format de texture interne exact (il
pourrait être en `Rgba8`, pas en `Rgba16Float` comme §21 l'impose) n'est pas confirmé.
Le pattern du pool est repris ; son choix de format colorimétrique ne l'est pas sans
vérification.

---

## 22bis. `scene3d` — analyse TouchDesigner

> Section d'intention : précisée avant l'étape 13 (§12), pas à l'implémentation.

**Décision, enfin tranchée : la scène 3D est modulée en direct, frame par frame, par
le graphe `Signal`/`Param`** — pas un flux d'entrée déjà « joué » qu'on se contenterait
de mixer comme une vidéo. Cette question était restée ouverte depuis la réorientation ;
l'analyse de TouchDesigner (seul logiciel de référence du projet qui fait réellement du
3D node-based en live) la tranche.

**Ce que confirme la documentation officielle de TouchDesigner (derivative.ca) :**

- **Familles de nœuds** : SOP (géométrie historique, points/polygones), POP (successeur
  GPU — points, primitives, particules, compute), COMP (hiérarchie d'objets, caméras,
  lumières, conteneurs), TOP (image 2D), MAT (matériaux). Un **Geometry COMP** réunit
  une chaîne SOP/POP + un MAT en objet rendable.
- **Le pont 3D → 2D est le Render TOP** : il consomme Geometry COMP(s) + Camera COMP +
  Light COMP et produit une image 2D plate (un TOP), **toujours recuite en direct
  depuis l'état courant des paramètres** — aucun raccourci de pré-rendu documenté. C'est
  la confirmation directe : pas de « scène déjà jouée », la scène est réévaluée chaque
  fois qu'elle est tirée. C'est le même rôle que jouera `Scene3dLayer` pour FrogenX :
  produire une texture consommée par le compositeur 2D (§6, §22), sans que celui-ci ait
  besoin de connaître le contenu 3D.
- **Modèle de cook** : évaluation paresseuse tirée par la demande (une sortie regardée
  ou connectée) et déclenchée par une raison (un paramètre a changé, l'amont a recuit) —
  à la fois *pull* et *push*. **Diverge de FrogenX** : le `Graph` de FrogenX évalue tout
  le graphe topologiquement à chaque frame avec un cache plat (§13), plus simple et
  déterministe, moins efficace sur de larges sous-graphes inchangés. Pas de changement
  prévu pour `core` — mais `scene3d` pourra, à terme, avoir intérêt à un cache de
  sous-arbre si le culling (§12, étape 13) ne suffit pas ; à réévaluer à l'implémentation,
  pas maintenant.
- **Liaison de paramètres** : chaque paramètre TD a 4 modes — Constant, Expression,
  Export (canal → paramètre, à sens unique), **Bind (canal ↔ paramètre, à double sens)**.
  Le `Param::Modulated{base,depth,source}` de FrogenX (§2.2) couvre l'équivalent
  d'Export, pas de Bind. **Décision : architecture hybride, tranchée en §23.5.**

### 22bis.1 `Param` reste unidirectionnel en 2D, `scene3d` gagne `Param::Bind`

**Décision : le binding bidirectionnel est ajouté, mais seulement dans `scene3d`.**
`core`/`osc`/`shape` (génération 2D) gardent `Param` tel qu'il est — `Fixed` /
`Modulated{base,depth,source}`, strictement à sens unique. `scene3d` gagne une
troisième variante, propre à cette crate :

```rust
// dans scene3d uniquement — pas dans core::Param
pub enum Param3d {
    Fixed(f32),
    Modulated { base: f32, depth: f32, source: NodeId },
    Bind { source: NodeId },   // deux sens : lit ET réécrit le NodeState de source
}
```

**Pourquoi ici et pas en 2D.** Le déclencheur est le pilotage live par contrôleur
physique (MIDI, OSC) demandé pour l'interface et les effets : un fader logiciel à 50 %
et une molette MIDI encore à 0 % se désynchronisent au premier geste (le classique
*parameter jump* MIDI) si rien ne les resynchronise dans les deux sens. Une caméra 3D
pilotée à la souris pendant qu'un `Signal` la module (ex. un LFO sur la rotation) a le
même besoin : tourner la caméra à la main doit pouvoir re-caler la phase du LFO, pas
seulement être écrasé à la frame suivante. La génération 2D (formes, `ShapeOp`) n'a
jamais ce geste de pilotage direct d'un objet manipulable à la main — elle reste de la
modulation pure, `Modulated` suffit.

**Pourquoi ça ne casse pas « l'état vit dans le graphe » (§2.3).** `Param3d::Bind`
n'écrit jamais dans un module ni dans un champ mutable arbitraire — il écrit dans le
`NodeState` du nœud `source`, via le `Graph`, exactement le canal déjà prévu pour que
l'état d'un `Signal` persiste entre frames (§16.1). Un `Signal` utilisé derrière un
`Bind` doit lire sa position courante depuis son `NodeState` plutôt que la recalculer
uniquement depuis `t` — c'est une exigence sur les `Signal` admissibles derrière un
`Bind`, pas une exception à la règle : l'état reste dans le graphe, jamais dans le
module.

**Portée explicite :** `Param3d::Bind` peut cibler n'importe quel nœud du graphe de
`scene3d`, interne ou branché sur une entrée MIDI/OSC — pas seulement les entrées
physiques. La frontière est la crate, pas la nature de la source.
- **Instancing** : le Geometry COMP a une page Instance intégrée — transform/couleur par
  instance piloté par un canal CHOP, une instance par échantillon/ligne/pixel/point.
  Pas un système de particules séparé. C'est la réponse à « beaucoup d'objets » : une
  primitive d'instancing dans `scene3d`, pas un système ad hoc par cas d'usage.

**Non confirmé, à ne pas citer comme acquis** : le détail des particules GPU/compute de
TouchDesigner, et si des cycles sont permis ailleurs que dans les nœuds de feedback
explicites — ni confirmé ni contredit par la documentation consultée.

---

## 23. UI et graphe

> Section d'intention : elle sera précisée à l'étape où l'UI arrive.

**Décision : `egui`, pas `iced`.** Comparés explicitement avant l'implémentation de
l'étape UI, pour quatre raisons :

- **Intégration wgpu** : `egui-wgpu` expose `register_native_texture` pour dessiner une
  texture rendue en externe (le monitor live, la grille de clips) comme une simple
  image — quelques lignes. En `iced`, la même chose demande d'implémenter un
  `Shader`/`Primitive` personnalisé sur son `Renderer` bas niveau, peu documenté.
- **Écosystème node-graph** : `egui_node_graph`/`egui-snarl` existent déjà et sont
  directement réutilisables pour l'éditeur de patch par clip (§3bis, §2). `iced` n'a
  pas d'équivalent — il faudrait le construire depuis son `Canvas`.
- **Correspondance architecturale** : `egui` est **immédiat** — chaque frame lit et
  écrit directement l'état existant (`ui.slider(&mut param.value)`), sans état de
  widget parallèle. `iced` est Elm-style (`Message` + `update()`) : il faudrait faire
  vivre une machine à états en plus du `Graph`, ce qui va contre l'invariant « l'état
  vit dans le graphe » (§2.3).
- **Précédent** : `egui` a des usages réels dans l'outillage créatif/temps réel en Rust
  (visualisateurs wgpu, outils live-coding) ; aucun équivalent trouvé côté `iced` pour
  ce type de logiciel.

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

### 23.1 Charte graphique de référence — Resolume Arena

**Décision : l'UI de FrogenX part de la charte graphique et de la disposition de
Resolume Arena**, observées sur une capture de l'application réelle (v7.28.1) et
confirmées par ses fichiers de layout (`Preferences/recentLayout.xml`,
`Presets/Interface/*.xml`). Cette charte est un point de départ, amené à évoluer
avec les fonctionnalités développées — pas une contrainte figée.

**Palette (valeurs relatives, à affiner en hex pendant l'implémentation UI) :**

| Rôle | Ton | Usage |
|---|---|---|
| Fond principal | anthracite quasi noir | zone de grille, dashboards, monitors |
| Fond des en-têtes | gris sombre, légèrement plus clair que le fond principal | barres de colonnes, toolbar |
| **Accent unique** | cyan/menthe vif | sélection active — layer sélectionné, onglet actif, bouton play, clip/effet mis en avant |
| Accent secondaire | vert terne | toggles actifs mais non sélectionnés |
| Texte principal | blanc cassé | tous les labels |
| Texte secondaire | gris moyen | labels désactivés, placeholders |
| Bordures | noir quasi pur | séparateurs entre panneaux |

**Un seul accent porte toute la sélection**, plutôt qu'une couleur par catégorie
d'élément — layer actif, onglet actif, transport, effet actif partagent la même
teinte. C'est délibéré chez Resolume : dans un environnement de régie sombre, une
palette monochrome avec un accent unique reste lisible sous faible éclairage et ne
rivalise jamais avec les couleurs du contenu vidéo affiché dans les vignettes et les
monitors, qui doivent rester les éléments les plus saturés à l'écran. FrogenX reprend
ce principe plutôt que la palette dans l'absolu : fond sombre neutre, un seul accent
réservé à l'état actif, jamais de couleur UI qui concurrence le rendu.

### 23.2 Disposition de référence

```
┌──────────────────────────────────────────────────────┐
│ Barre de menu                                         │
├──────────────────────────────────────────────────────┤
│ Grille (Layers × Clips/Columns) — §3bis               │  zone haute,
│                                                        │  hauteur fixe desired
├──────────────────────────────────────────────────────┤
│ Toolbar (transport, tempo, tap, resync — §3bis.3)     │
├───────────┬────────────┬─────────────┬────────────────┤
│ Monitors  │ Dashboard  │ Dashboard   │ Navigateur      │
│ (sortie + │ contextuel │ du clip     │ (sources,       │
│  preview) │ (colonne/  │ sélectionné │  effets,        │
│           │  groupe/   │ + sa chaîne │  compositions,  │
│           │  layer)    │ d'effets    │  enregistrement)│
└───────────┴────────────┴─────────────┴────────────────┘
```

**Quatre zones flexibles sous la grille**, redimensionnables indépendamment — pas une
disposition figée mais un système de conteneurs flexibles, comme chez Resolume
(`FlexContainer`/`TabbedContainer` imbriqués dans leur layout). FrogenX adopte le même
principe plutôt qu'une mise en page statique : chaque panneau peut être masqué,
redimensionné ou déplacé, pour les mêmes raisons qui ont fait ce choix chez eux — un
poste de VJ tourne sur des configurations d'écran très variées (un moniteur, deux,
une sortie dédiée à la régie).

**Correspondance avec le modèle FrogenX :** la grille haute est `Deck`/`LayerGroup`/
`Layer`/`Column`/`Clip` (§3bis.1) ; le dashboard contextuel affiche les `Param` de
l'objet sélectionné à n'importe quel niveau de la hiérarchie ; le panneau Clip affiche
la chaîne `ShapeOp` (ou son équivalent vidéo/3D) du clip actif (§3bis.2) ; le
navigateur liste les types de nœuds disponibles — direct héritage du couple
`Effects`/`Sources` de l'API Resolume déjà documenté en §13.

### 23.3 Modèle d'effets — inspiré de Resolume Wire

**Decision : le catalogue d'effets et de sources de FrogenX s'inspire du moteur
interne "Wire" de Resolume** (`WireLib.dll`/`WireNodes.dll`, fichiers `.wired` dans
`plugins/wire/`) plutôt que d'être conçu dans l'abstrait. Wire est lui-même un
système d'effets node-based — chaque `.wired` est un graphe de nœuds sérialisé,
exactement le même genre d'objet qu'un `Clip` de FrogenX avec sa chaîne `ShapeOp` et
son `Graph` de modulation (§3bis.2).

**Ce que ça confirme pour FrogenX, sans copier le code :**
- Un effet n'est pas une fonction opaque mais un **petit graphe réutilisable et
  partageable** — cohérent avec `ShapeOp::Chain` (§16.3 bis) et avec le fait qu'un
  patch se sauvegarde en JSON lisible (§19).
- Les effets natifs (`plugins/afx/*.dll` côté audio) et les effets node-based
  (`plugins/wire/*.wired` côté vidéo) coexistent chez Resolume : un effet peut être
  soit un module compilé, soit un graphe composé d'autres modules. FrogenX a déjà
  cette distinction implicitement (`ShapeOp` natif vs. `Chain` de `ShapeOp`) — Wire
  confirme qu'il vaut la peine de l'exposer à l'utilisateur comme deux façons
  équivalentes d'ajouter un effet au catalogue, pas seulement comme un détail
  d'implémentation.
- Chaque effet déclare des **presets nommés réutilisables** (confirmé en §13) :
  FrogenX en manque aujourd'hui. Un preset par `ShapeOp`/chaîne d'effets rejoint la
  feuille de route (§12) comme extension du format de patch (§19), sans remettre en
  cause son schéma de base.

### 23.4 Effets custom — deux surfaces distinctes

**Décision : FrogenX ouvre deux surfaces de scripting séparées**, chacune pour un
besoin différent — écrire un effet visuel pixel par pixel n'est pas le même geste que
scripter la logique d'un patch. Les confondre dans un seul langage forcerait l'un des
deux usages à de mauvais compromis.

**Scripting de composition : Lua, via `mlua`.** Choisi plutôt que `Rhai` (pur Rust)
pour la maturité et la vitesse — trente ans d'usage en production, `mlua` se compile
statiquement sans dépendance système à installer, et c'est le langage qu'un non-
développeur a le plus de chances de déjà connaître dans le monde du live visuel. Rhai
reste plus proche du reste du codebase (zéro FFI C), mais les retours communautaires
le rapportent nettement plus lent que Lua et sans outillage (pas de LSP) — un mauvais
compromis pour un langage que l'utilisateur va écrire en direct, en répétition ou en
régie. Un script Lua peut émettre des `Command` (§23) pour piloter le graphe, mais ne
mute jamais le `Graph` directement — même contrat que l'UI.

**Format de shaders pour les effets pixel : tranché — WGSL natif, importeur ISF
plus tard.** Deux options avaient été posées (compatible ISF adapté en WGSL, vs
format WGSL propre à FrogenX) ; la décision a été vérifiée contre l'état réel de
l'écosystème Rust plutôt que devinée :

- Le crate `isf` (nannou-org) ne fait que **parser** le JSON de métadonnées — pas de
  transpilation, abandonné depuis 2020 (dernière publication v0.1.0).
- Le frontend GLSL de `naga` (seul transpileur GLSL→WGSL utilisable en Rust) est
  qualifié de **« support secondaire »** par son propre README, limité à un dialecte
  GLSL 440+/Vulkan — pas le GLSL ES 2.0/3.0 dans lequel ISF est écrit — avec un
  blocage connu sur les samplers combinés signalé par un mainteneur wgpu en 2024.
- Aucun projet Rust ISF-dans-wgpu trouvé, même partiel ou abandonné.
- Au-delà de la syntaxe, ISF a un vrai modèle d'exécution à reconstruire : buffers
  `TARGET` multi-passes, buffers `PERSISTENT` qui survivent entre frames, uniformes
  implicites (`isf_FragNormCoord`, `IMG_NORMPIXEL`, entrées FFT audio). Transpiler la
  syntaxe ne suffit pas ; il faudrait réimplémenter une partie du runtime ISF par-
  dessus un transpileur déjà fragile.

**Décision : WGSL natif maintenant.** La compatibilité ISF dépend d'une
infrastructure qui n'existe pas dans l'écosystème Rust — ce n'est pas un adaptateur
fin mais un sous-projet de plusieurs semaines avec un risque de correction (le
frontend GLSL de `naga` n'est pas fiable aujourd'hui), à ne pas laisser retarder ou
fragiliser le pipeline d'effets. **Ce choix ne ferme pas la porte à ISF plus tard** :
tant que le binding des paramètres d'effet reste le schéma explicite de FrogenX
(`Param`, §16.4) et non la convention d'uniformes implicites d'ISF, un futur
importeur ISF (métadonnées ISF → WGSL généré + bindings `Param`) peut s'ajouter comme
frontend de compilation isolé, sans toucher au pipeline d'effets natif.

**Contraintes du format WGSL natif, dès l'implémentation :**
- Jamais de panique en évaluation (§18.1) — un shader custom défaillant dégrade
  (image noire, effet ignoré, compteur `Diagnostics` incrémenté), ne plante jamais le
  rendu.
- Les paramètres exposés par un effet custom restent des `Param` (§16.4), pas des
  uniformes bruts — un effet custom doit être modulable exactement comme un effet
  natif, sinon il devient un citoyen de seconde classe dans le graphe. C'est aussi ce
  qui garde la porte ISF ouverte (ci-dessus).
- Compilation/validation du shader se fait en régime **édition** (§18.1), jamais en
  frame — un shader qui ne compile pas est refusé au chargement, pas découvert à
  l'exécution.

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
