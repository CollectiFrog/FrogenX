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

**S'y ajoute un seul graphe de composition, porté par `Composition`** (§3bis.1). Il
module ce qui appartient à la régie et non à un clip : l'opacité des pistes, le tempo,
plus tard le crossfader. Il ne rompt pas la règle ci-dessus : aucun clip n'y accède, et
il survit à tous les changements de clip.

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
 │           └─ cases[]  une par colonne, vide ou portant un Clip
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

**Une cellule est une case de sa piste (étape 7c).** `Layer` porte une case par colonne
(`Vec<Option<ClipId>>`), à l'index de la colonne dans la grille ; `Column` ne porte plus
rien et ne vaut que par sa position. Déclencher la colonne `k`, c'est déclencher la case
`k` de chaque piste du deck actif ; une case vide laisse sa piste inchangée. Cela remplace
`Column.slots`, qui visait les pistes par leur rang : ajouter ou réordonner une piste
aurait décalé toutes les colonnes, et une piste qui n'était qu'une liste de clips ne
savait pas représenter une case vide. Les colonnes restent partagées entre les decks,
comme les scènes d'un conducteur.

**La grille se modifie entre deux frames.** `place_clip` remplit une case ; si la case
remplacée jouait, le nouveau clip prend sa place et repart de zéro, pour que l'image ne
s'éteigne pas sous un geste d'édition. Retirer une colonne ou une piste libère ses clips,
et un clip retiré cesse de jouer ; les identifiants restants se périment sans danger
(§16.5). `add_clip` reste pour construire une grille par code : il prend la première case
libre et ajoute une colonne s'il n'y en a plus.

**Représentation : une arena par niveau, pas une arena unique pour toute la
hiérarchie.** Même principe que `core::Graph` (§3) — `Vec<T>` + identifiant à
génération — répété une fois par niveau plutôt que partagé :

```rust
pub struct DeckId(u32);
pub struct LayerGroupId(u32);
pub struct LayerId(u32);
pub struct ColumnId(u32);
pub struct ClipId(u32);
```

**Pourquoi pas une seule arena hétérogène.** Un identifiant typé par niveau rend une
confusion (passer un `ClipId` où un `DeckId` est attendu) **détectable à la
compilation**, pas à l'exécution — alors qu'un unique `NodeId` générique pour toute la
hiérarchie laisserait ce genre d'erreur passer silencieusement jusqu'à un panique ou un
comportement incohérent en régime édition (§18.1). Chaque `Vec` reste aussi petit et
contigu que son niveau le permet, et retirer un `Deck` ne renumérote jamais les `Clip`
d'un autre deck.

**Une piste porte son mélange.** `composition::Layer` possède ses `BlendParams`
(opacité et mode, §22), dont les `Param` se résolvent dans le **graphe de composition**.
Dans le clip, changer de clip ferait sauter le réglage que le VJ tient sur sa piste.
`BlendParams` et `ColorParam` vivent donc dans `shape`, sans GPU : `composition` ne peut
pas dépendre de `layer` sans perdre ses tests sans GPU (§10).

**L'ordre de rendu est celui de la hiérarchie.** `Composition` a un deck actif, le
premier ajouté par défaut ; ses groupes puis leurs pistes se composent de bas en haut.
Une piste sans clip actif ne contribue pas à l'image.

**Coût accepté :** cinq structs d'identifiant au lieu d'un seul générique, et une
référence croisée (un `Layer` vers son `Deck` parent) passe par un identifiant typé de
la relation plutôt qu'un simple entier. C'est du boilerplate mécanique, pas un risque —
contrairement au choix de représentation de `Transport.position` (§3bis.3), qui
engageait un vrai contrat.

### 3bis.2 Le `Clip`, point de jonction avec le graphe modulaire

```rust
pub struct Clip {
    pub name: String,             // ce qu'affiche sa cellule dans la grille (§23.5)
    pub source: ClipSource,       // Shape { graph, generator, style } | Scene3d(Graph) | Video(VideoTrack)
    pub effects: Vec<ShapeOp>,    // ou son équivalent vidéo/3D — chaîne en place (§16.3 bis)
    pub transport: Transport,     // Timeline ou BpmSync (§3bis.3)
    pub transition: ClipTransition,
    pub speed: Param,             // vitesse du clip, résolue dans le graphe de composition
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

**`ClipSource::Shape` porte un générateur, pas seulement un graphe.** Correction du
premier état de `composition`, où la variante était `Shape(Graph)` : un graphe de
modulation seul ne produit aucune forme, et `Clip.effects` était alors une chaîne
d'opérateurs sans rien à transformer. La variante devient `Shape { graph, generator }`
— le graphe module, le générateur (`Box<dyn ShapeGenerator>`, §16.3) produit le `Path`
que les effets transforment. S'y ajoute `style` (`ShapeStyle` : remplissage et trait,
§21) : sans lui, un clip saurait quelle forme produire, mais pas de quelle couleur.

**Le clip décrit, la piste dessine (étape 7).** Un `ShapeRenderer` par piste possède les
textures et les buffers ; à chaque frame, le clip actif lui prête son contenu
(`ShapeContent`, §22.1). Une `ShapeLayer` par clip coûterait environ 80 Mo de textures
par clip en 1080p avec le MSAA 4× — 8 Go pour une grille de 100 clips, dont un seul par
piste s'affiche.

**Un clip sait décrire ses réglages.** `Clip::params` réunit en groupes ceux de son
générateur, de chaque effet et de son style, et désigne chacun par un `ParamSlot` ;
`Clip::set_param` écrit par ce même `ParamSlot`. L'interface n'a ainsi besoin de
connaître aucun type de forme (§16.3 ter). Les `Param` du graphe de modulation interne
du clip n'y figurent pas : ils arriveront avec l'éditeur de nœuds.

**La vitesse d'un clip multiplie son `dt` (étape 7c).** `Engine` évalue le graphe du clip
avec `dt * speed` : un seul réglage accélère tout ce qui s'y accumule, LFO, bruit et
position de lecture, sans savoir ce que le clip contient. C'est possible parce qu'aucun
`Signal` ni aucune forme ne lit `t` : tout s'intègre sur `dt`. Un module qui lirait `t`
directement échapperait à ce réglage. `speed` est un `Param` résolu dans le **graphe de
composition**, comme le mélange d'une piste : il doit être connu avant d'évaluer le graphe
du clip, et la régie pourra le moduler. Saturé à la lecture dans `[0, 4]`, une valeur non
finie valant 1 (§18.4).

**Seul le clip actif vit.** Un clip inactif n'est pas évalué : le coût d'une frame suit
ce qui est à l'écran, pas la taille de la grille. **Déclencher un clip le fait repartir
de zéro** (`Graph::reset_state`), y compris s'il était déjà actif : c'est ce qu'on attend
d'un pad qu'on frappe. Reprendre là où il en était sera une option par clip, si l'usage
la demande.

**`ClipTransition` suit le même principe que `Transport.position` (§3bis.3) : une
progression qui avance seule est un accumulateur, pas un `Param` statique.**

```rust
pub enum ClipTransition {
    Cut,
    Fade { phase: PositionSignal, curve: FadeCurve },
    Wipe { phase: PositionSignal, curve: FadeCurve, direction: WipeDirection },
}
```

**Pourquoi pas une énum de données pures (`Fade(Param)`).** Un fondu qui dure N frames
a besoin de la même chose qu'une position de lecture : une valeur qui progresse de 0 à
1 toute seule, frame après frame, sans qu'un `Command` externe la pousse à chaque
frame. C'est exactement le rôle de `PositionSignal` (§3bis.3) — réutilisé ici plutôt
que réinventé, et l'état de progression vit dans le `NodeState` du `Graph` du clip,
pas dans `ClipTransition` lui-même, qui reste sans champ mutable (§2.3).

**Le rendu des transitions, avec la première version.** `Layer` retient, le temps d'une
transition, le clip sortant et sa progression. C'est la transition du clip **qui entre** qui
s'applique, et elle se déclenche quand un autre clip jouait déjà : redéclencher le même clip le
fait repartir sans transition, et vider une piste reste une coupure. Déclencher pendant une
transition en commence une nouvelle depuis le clip qui entrait.

**Une durée en battements.** `phase.speed` vaut `1 / battements` : une transition suit le tempo
comme tout le reste, sur l'horloge de la régie. La vitesse d'un clip ne la change pas : à zéro,
elle ne doit pas rester bloquée à mi-chemin.

**La progression n'est pas dans un graphe.** Elle vit dans `Layer`, comme le clip actif. Un nœud
ajouté puis retiré à chaque transition finirait dans un fichier enregistré au mauvais moment. Elle
ne se sauvegarde pas : annuler ou ouvrir un fichier la termine net.

**Mélanger avant le compositeur.** Pendant une transition, les deux clips sont évalués, et la piste
en dessine deux. `TransitionMix` les mélange en une seule texture, qui rejoint le compositeur avec le
mode et l'opacité de la piste. Deux couches d'opacités croisées laisseraient voir un creux à
mi-fondu : à 50 %, deux images opaques ne couvrent que 75 %. Un `Fade` mélange linéairement, en
prémultiplié (§21). Un `Wipe` découvre l'image selon sa direction, avec un bord adouci de 2 % pour
ne pas crénelier. `WipeDirection::Left` signifie que le clip entrant avance vers la gauche.

**Le coût.** Deux renderers de plus sur une piste au premier fondu, gardés ensuite : environ 80 Mo en
1080p pour une forme (§13), rien sur une piste qui n'en fait jamais.

### 3bis.3 Transport et tempo

```rust
pub enum Transport {
    Timeline { position: PositionSignal, speed: Param },
    BpmSync { position: PositionSignal, duration_beats: Param, play_mode: PlayMode },
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

**Le tempo arrive aux clips par `Ctx.bpm`, depuis une source unique.** `Composition`
résout `TempoController::tempo` une fois par frame, après avoir évalué son graphe, et le
moteur le pose dans le `Ctx` de chaque clip actif (§16.2). Aucun clip n'est câblé au
tempo un par un, et une nouvelle source n'a qu'une chose à faire : écrire le tempo. Le
le tap tempo le fait, et le BPM calculé depuis l'audio aussi quand on le suit (§7.1).
Saturé à `[20, 400]` chez ce consommateur (§18.4) ; un tempo non fini rend 120.

```rust
// Un Signal qui vaut bpm / 60 / beats : une fréquence, en Hz.
pub struct BeatRate { pub beats: Param }
```

**`BeatRate` rend un Signal calé sur le tempo sans rien lui apprendre de la musique.**
Branché sur la `freq` d'un `Lfo`, il donne un cycle tous les `beats` battements ; branché
sur la `speed` d'une `PositionSignal`, une position en battements (`Transport::bpm_sync`
fait ce câblage). **Piège : il se déclare en dépendance** (`add_with_deps`), sinon
l'ordre d'évaluation n'est pas garanti et le consommateur lit la valeur de la frame
précédente.

**Le tap tempo** garde au plus 8 instants, repart de zéro après 2 s de silence, et
écrit un `Param::Fixed`.

**Limite de l'étape 7 : `play_mode` n'a pas d'effet.** `position` avance, mais une forme
procédurale n'a pas de fin naturelle : rien ne lit encore `duration_beats` ni
`play_mode`. Ils prendront leur sens avec les clips vidéo (étape 12). Ne pas lire
`Once` ou `PingPong` comme un comportement livré.

**Pourquoi `position` n'est pas un `Param`.** `speed` et `duration_beats` sont bien
réglables par l'utilisateur ou modulables par un `Signal` — ce sont des entrées,
`Param` leur convient. `position` n'est pas réglable de la même façon : c'est une
valeur qui **avance toute seule**, frame après frame, dérivée du temps — exactement le
rôle que §16.1 attribue à un `Signal`, pas à un `Param`. Un `Param::Fixed`/`Modulated`
ne s'auto-incrémente jamais ; le faire avancer depuis l'extérieur à 60 fps via un
`Command::SetParam` (§23) détournerait un mécanisme pensé pour l'édition ponctuelle,
pas pour l'évaluation par frame — exactement le genre de raccourci que §15 interdit.

`position` est donc portée par un petit `Signal` interne au clip, un
**accumulateur de position** — le même principe que la phase d'un oscillateur
(§16.1, « corollaire du temps » §2.3), appliqué à une position de lecture au lieu
d'une phase `[0,1)` :

```rust
// slots[0] = position accumulée, en secondes (Timeline) ou en battements (BpmSync)
pub struct PositionSignal { speed: Param }

impl Signal for PositionSignal {
    fn eval(&self, _t: f64, ctx: &Ctx, state: &mut NodeState) -> f32 {
        let pos = state.get(0) + self.speed.eval(ctx) * ctx.dt as f32;
        state.set(0, pos);
        pos
    }
    fn state_size(&self) -> usize { 1 }
}
```

**Ce que ça donne, concrètement :**
- `position` se lit comme n'importe quel `Signal` — `p.get(&graph)` depuis le moteur
  de composition entre deux frames, `p.eval(ctx)` depuis un autre `Signal` du même
  clip (§16.4, tableau des deux voies de lecture).
- L'état (la position courante) vit dans le `NodeState` que le `Graph` du clip
  possède — pas dans `Transport` lui-même, qui reste une structure sans champ
  mutable. Ça respecte §2.3 sans exception, et ça rend `position` sérialisable et
  réinitialisable gratuitement, comme tout `NodeState` (§16.1).
- `BpmSync` branche la même `PositionSignal` mais avec `speed` dérivée du `tempo`
  du `TempoController` plutôt qu'un `Param` fixe — un `resync` (tap, re-sync DJ) se
  traduit par un `Command` qui réécrit `state.slots[0]` via le `Graph`, pas par une
  mutation directe de `Transport`.
- `ClipTransition` (§3bis.2) suit le même principe pour sa progression — un fondu
  qui dure N frames est aussi un accumulateur, pas un `Param` statique.

**Pourquoi documenter ce choix ici plutôt que de le découvrir en silence à
l'implémentation** : `feature-complete` demande explicitement de corriger le contrat
dans le document dès qu'une contrainte du compilateur révèle un couplage que le
contrat autorisait à tort (c'est arrivé une première fois pour `Ctx`, §16.2). Ici,
c'est anticipé plutôt que découvert : `Param` n'a jamais eu vocation à représenter une
valeur qui avance seule, et laisser le sketch initial (`position: Param`) tel quel
aurait forcé une réécriture à l'étape 7 de la feuille de route (BpmSync câblé en
direct, §12).

**`PlayMode` agit sur le temps du clip, sans vidéo.** Une forme procédurale n'a pas de fin
naturelle : c'est `duration_beats` qui lui en donne une.

| Mode | Au bout de la durée |
|---|---|
| `Loop` | rien ne change : le clip continue, la barre de sa cellule repart |
| `Once` | la piste se vide, comme par `ClearLayer`, en coupure |
| `PingPong` | le temps du clip s'inverse, jusqu'au retour à zéro, puis repart |

**À l'envers, c'est le `dt` qui change de signe.** `Engine` évalue le graphe du clip avec `-dt`. Les
phases et le bruit remontent donc d'eux-mêmes, sans code par type : c'est tout l'intérêt de les
intégrer (§2.3). Ce n'est pas une lecture à rebours exacte : une enveloppe ou une feature audio
continuent de suivre leur entrée vers l'avant, le son ne se rejoue pas à l'envers. Un écho reste
aussi à l'endroit : ses réglages s'exprimeraient à contresens, et sa mémoire grandirait au lieu de
s'effacer.

**Le sens vit dans `Layer`**, comme le clip actif et la transition (§3bis.2) : c'est un état de jeu,
absent des fichiers. Une position à la limite suffit à le lire, et une durée qui n'a pas de sens
(nulle, non finie) laisse le clip en boucle (§18.4).

**Le geste.** Le panneau du clip porte une ligne `Lecture` : le mode et la durée en temps. Elle ne
concerne que les clips calés sur le tempo. `Timeline` n'a ni durée ni mode.

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

**`serde` dès le premier commit de `composition`, pas ajouté après coup.** Les cinq
identifiants arena du §3bis.1 et la forme de `ClipTransition` (§3bis.2) doivent avoir
un schéma de sérialisation stable avant que quoi que ce soit ne s'appuie dessus —
celui du §19.1. Ajouter
`#[derive(Serialize, Deserialize)]` après coup, une fois la structure interne figée
par du code qui en dépend, coûterait une migration de schéma (§19.2) immédiate — un
coût strictement supérieur à celui de le faire dès le début, pour un `derive` qui ne
change rien au comportement. `serde` passe donc de « à venir » à dépendance active de
`composition` dans `[workspace.dependencies]` (§11), dès l'étape 4 du §12.

**Découvert à l'implémentation : `Param` et `NodeId` (`core`) ne dérivent pas
`serde`, donc `Transport`/`Clip` non plus, directement.** `core::Param` encode une
source de modulation (`NodeId`) ; sa forme sérialisée passe par le schéma JSON nommé
du format de patch (§19.1 : `{"fixed": ...}` / `{"modulated": {...}}`), pas par un
`derive` direct sur la structure Rust en mémoire. `Transport` (§3bis.3) et `Clip`
(§3bis.2), qui portent tous deux un `Param`/`NodeId`, suivent donc la même règle : pas
de `#[derive(Serialize, Deserialize)]` sur ces types eux-mêmes. Ce n'est pas une
régression par rapport à la décision ci-dessus — les cinq identifiants *arena*
(`DeckId`…) restent sérialisables directement, c'est seulement ce qui référence un
`Param`/`NodeId` de `core` qui doit passer par le format de patch. La traduction
`Transport`/`Clip` ↔ JSON vit dans la crate `patch` (§19.1).

### 3bis.6 Le trig

**Une zone dessinée sur le spectre déclenche la grille, une fois sur N.** C'est le modèle
de déclenchement que le §7 laissait ouvert. On trace un rectangle dans le plan fréquence ×
amplitude ; quand le pic du spectre y entre, la porte s'ouvre ; un compteur laisse passer
un front sur N ; et ce qui passe devient un `Signal` et, si on le veut, un `Command`.

```
Ctx.audio.spectrum ─▶ gate : SpectrumZone ─▶ pulse : Divider ─▶ flash : Envelope
                       porte 0 / 1            1 pendant une        retombe seule
                                              frame, 1 fois sur N
                                                     │
                                                     └─▶ Command, à la frame suivante
```

```rust
pub struct SpectrumZone {            // osc — un Signal, à état
    pub freq_min: Param, pub freq_max: Param,   // Hz, dans n'importe quel ordre
    pub level_min: Param, pub level_max: Param, // [0, 1]
    pub hysteresis: Param,
}
pub struct Divider { pub input: Param, pub every: Param }  // osc — un Signal, à état

pub struct Trig {                    // composition
    pub name: String,
    pub target: TrigTarget,          // None | Clip { layer, clip } | Column(ColumnId)
    // zone, divider et les trois NodeId : gate(), pulse(), flash()
}
```

**La zone se franchit par le bas, une seule fois.** Une attaque traverse le rectangle en
montant, le dépasse, puis le retraverse en descendant : une porte qui dirait seulement
« le pic est dans la zone » s'ouvrirait deux fois par coup. `SpectrumZone` a donc trois
états, dans son slot : sous la zone, ouverte, passée au-dessus. Ouverte, elle le reste
jusqu'à ce que le pic repasse sous `level_min - hysteresis`, même s'il dépasse le haut
entre-temps. Un son qui saute d'un coup au-dessus de `level_max` ne l'ouvre pas, ni en
montant, ni en redescendant : c'est ce qui donne un sens au haut de la zone. À
`level_max = 1`, c'est un seuil simple. L'hystérésis évite la rafale d'un son qui tremble
sur le bord.

**Le compteur compte des fronts, pas des battements.** « 1 sur 4 » veut dire : un coup
détecté sur quatre. Sur un kick régulier, cela tombe sur la mesure, mais `Divider` ne sait
rien de la musique ni de l'audio : il compte les fronts de n'importe quelle porte, et une
source calée sur le tempo pourra s'y brancher. **Le premier front passe, pas le dernier** :
le trig répond tout de suite et son compte se cale sur le temps fort où on l'a lancé.
`Recaler` (`Command::ResetTrigCount`) remet ce compte à zéro, parce qu'un coup manqué le
décale. Les valeurs proposées sont 1, 2, 4, 8, 16, 32 ; `every` reste un `Param`, arrondi
et saturé dans `[1, 1024]` par son consommateur (§18.4).

**Deux couches : deux `Signal` dans `osc`, un `Trig` dans `composition`.** `SpectrumZone`
et `Divider` ne lisent que `Ctx` : ils sont déterministes, testables à la main, et un
clip peut en mettre dans son propre graphe pour moduler n'importe lequel de ses `Param`,
car `Engine` pose le spectre dans le graphe de chaque clip actif. `Trig` est ce câblage
tout fait, avec une cible. Il appartient à la composition, comme le tempo : ses nœuds
vivent dans le graphe de composition, il survit aux changements de clip et de deck, et
son compte aussi. Le déclenchement remettant à zéro le graphe du clip (§3bis.2), un trig
placé dans un clip ne compterait pas à travers ses propres déclenchements. `flash` se
branche sur tout `Param` de la régie, l'opacité d'une piste par exemple.

**Une frame de retard, voulue.** `Composition::trig_commands` se lit après le graphe de
composition, et les commandes obtenues rejoignent la file : elles s'appliquent au début
de la frame suivante, jamais pendant l'évaluation (§23). Avec les 21 ms du centre de la
fenêtre FFT (§7), le saut d'analyse et les deux frames, un coup agit à l'écran environ
50 ms après le son, à 60 fps. C'est un calcul, pas une mesure.

**Régler un `Param` déjà dans le graphe : `Graph::replace`.** Un `Signal` est opaque
derrière son `Box<dyn Signal>`. `Trig` garde donc la version de référence de `zone` et
`divider`, écrit dedans (`set_base`, la modulation survit, §16.3 ter) et remplace le nœud.
L'identifiant, les dépendances et l'état sont conservés : déplacer une zone ne perd pas le
compte. Refusé si la mémoire du module change (`GraphError::StateSizeMismatch`).
`Graph::node_state` expose l'état d'un nœud en lecture, pour afficher le compte.

**Non couvert.** `flash` se branche sur un `Param` depuis l'éditeur de nœuds (§23.10). Le
trig d'un `Column` déclenche sur le deck actif, comme tout `TriggerColumn`.

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
    fn generate(&self, t: f64, ctx: &Ctx, g: &Graph, out: &mut Path);
}
```

Une forme est un **ensemble de contours** : un buffer plat de points, découpé en plages,
chacune fermée ou ouverte. Un cercle est un contour fermé ; un arc, un contour ouvert ;
une rosace produite par répétition radiale, plusieurs contours fermés. Le détail et la
raison de ce choix sont au §16.3.

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
trait RenderLayer {
    fn prepare(&mut self, t: f64, ctx: &Ctx, g: &Graph, gpu: &Gpu); // travail CPU
    fn encode(&self, encoder: &mut CommandEncoder);                  // passe GPU
    fn output(&self) -> &TextureView;                                // texture prêtée
    fn blend(&self) -> BlendParams;   // opacité, mode — modulables par Signal
}
```

Contrat complet et raison du découpage en trois appels : §22.

**Pourquoi `RenderLayer` et non `Layer`.** Depuis la grille de composition (§3bis),
`composition::Layer` désigne une **piste** de la grille : une suite de clips dont un
seul est actif. Le trait de cette section désigne autre chose : un objet qui **produit
une texture**. Les deux coexisteront dans `render` et dans `ui` — une piste de la grille
pilote la couche de rendu de son clip actif. Deux types homonymes dans les mêmes
fichiers obligeraient à qualifier chaque usage et rendraient chaque message d'erreur
ambigu. Les implémentations gardent leur nom (`ShapeLayer`, `FeedbackLayer`,
`VideoLayer`, `Scene3dLayer`) : le suffixe y est sans ambiguïté.

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

### 6.1 Le feedback (étape 9)

> Le principe est fixé au §22 ; ce qui suit le place dans la grille.

**Un feedback est un clip.** `ClipSource::Feedback`, à côté de `Shape` : il se glisse depuis
la bibliothèque, se déclenche, se règle au potentiomètre, se module par le son et se
sauvegarde comme les autres. Une couche d'effet attachée à une piste aurait demandé un
second chemin pour tout cela.

**Il réinjecte l'image finale de la frame d'avant**, transformée et affaiblie, pas seulement
sa propre piste : c'est ce qui donne des échos de toute la composition. `Engine` copie la
texture finale dans une `FrameHistory` à la fin de chaque frame. Le feedback lit cette copie
et écrit sa propre cible : lire et écrire la même texture est interdit (§22). Le délai est
d'une frame exactement, et le graphe reste un DAG (§13).

**Tout s'exprime par seconde, jamais par frame.** Une rotation réglée par frame tournerait
deux fois plus vite à 120 fps. `FeedbackParams` porte quatre `Param` :

| Paramètre | Unité | Par frame |
|---|---|---|
| `zoom` | facteur par seconde | `zoom^dt` |
| `rotation` | radians par seconde | `rotation × dt` |
| `dérive x`, `dérive y` | unités d'espace par seconde | `dérive × dt` |
| `mémoire` | secondes pour perdre la moitié | `0,5^(dt / mémoire)` |

La vitesse du clip multiplie ce `dt`, comme pour son graphe (§3bis.2) : à zéro, l'écho se fige.

**Où le poser.** Sur une piste **sous** les formes, en `Normal`, il laisse des traînées
derrière elles. Au-dessus, en `Add`, il ajoute des échos lumineux. La mémoire reste bornée :
au-delà de 30 s, le paramètre est saturé chez le consommateur (§18.4), pour qu'une image ne
s'accumule pas sans fin.

**Le coût.** Une copie de texture par frame, que le feedback joue ou non : elle évite qu'un
clip déclenché réinjecte une image vieille de plusieurs minutes. Une cible de plus, allouée à
la première utilisation d'un feedback sur la piste.

**Fichier.** Un clip porte `generator` et `style`, ou `feedback`. C'est la version 3 du format.

**Un flou léger s'accumule, et il fait partie de l'effet.** Chaque frame relit l'image entre
deux pixels : sur une dérive, les bords s'adoucissent peu à peu. Rien ne le compense.

**Ce qui est vérifié, et comment.** Les réglages se testent sans GPU, y compris l'indépendance
à la cadence, de 30 à 144 fps. Le calcul du shader a sa référence Rust, comparée aux
uniformes. Sur le GPU : la moitié de l'image perdue à une demi-vie près, une première frame
qui ne lit rien, le zoom, la dérive, le gel à vitesse nulle, un rejeu identique au pixel près,
et un écho tenu par les graves à travers une source audio. Les trois échos du catalogue ont
été rendus et regardés. **Pas encore essayés :** à la vraie souris dans l'application, et sur
un écran ou une cadence autres que 60 fps pour le rendu réel.

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

Le mécanisme concret, écrit à l'étape 8, est au §7.1.

```rust
struct AudioFeature { feature: FeatureKind, attack: Param, release: Param }
impl Signal for AudioFeature { /* lit ctx.audio */ }
```

**Une feature audio est un `Signal`.** Aucune nouvelle abstraction, aucun câblage
spécial : l'audio se branche sur n'importe quel paramètre exactement comme un LFO.
Le système modulaire absorbe l'audio gratuitement. Un `Signal` lit `ctx.audio`, jamais le
triple buffer : celui-ci se lit une fois par frame, à l'étape 2 du §20.1, ce qui donne à
tous les graphes le même instantané et laisse un rendu offline fabriquer le sien.

**Lissage :** obligatoire. Les features brutes sont bruitées et produisent un visuel
nerveux. Attaque et release réglables par feature.

**Source :** entrée live (`cpal`) d'abord. La lecture de fichier peut s'ajouter ensuite
et apporte un avantage propre — connaître le futur du signal permet l'anticipation et
une synchro parfaite.

**Décidé pour la première version :** l'audio capturé sert aussi à **calculer le BPM**,
qui alimente le `TempoController` (§3bis.3) en plus du tap tempo, et à **déclencher des
effets vidéo** à partir des transitoires. Le modèle de déclenchement est tranché : c'est
le **trig** (§3bis.6). Le BPM est détaillé au §7.1.

**Le spectre.** `AudioFeatures::spectrum` : 128 bandes de 20 Hz à 20 kHz, chacune dans
`[0, 1]`. Le type vit dans `core` (§16.2), de taille fixe : il voyage dans le triple buffer
et dans `Ctx` sans jamais allouer. Il **porte ses échelles** : une zone en Hz se relit donc
juste, quel que soit le découpage. La montée est immédiate, et la descente a 60 ms de
constante de temps : lisser l'attaque retarderait le trig (§3bis.6).

**Quatre réglages, tenus par la composition** (`Composition.analysis`, un `AudioAnalysis`).
Un fichier les garde avec ses zones : une zone n'a de sens que relue comme elle a été posée.

| Réglage | Choix | Ce que ça change |
|---|---|---|
| `AnalysisMethod` | `Peak` | la case de FFT la plus haute de chaque bande : les notes ressortent |
| | `Energy` | l'énergie de toute la bande : le bruit et les charleys pèsent leur poids |
| | `ConstantQ` | une fenêtre par bande, d'autant plus courte que la bande est large : les aigus réagissent vite, les graves restent séparés |
| `FreqScale` | `Log`, `Mel`, `Bark`, `Linear` | comment l'axe se découpe en bandes égales. `Linear` donne 157 Hz par bande : les graves y tiennent dans une seule |
| `AmpScale` | `Decibel` | de -72 dB (`0`) à une sinusoïde à pleine échelle (`1`) |
| | `Linear` | la magnitude elle-même : seuls les sons forts émergent |
| `window` | 5 ms à 1 s | la durée de signal analysée, le compromis temps/fréquence |

**La durée d'analyse.** Longue, elle sépare deux graves proches, mais répond tard : il faut
environ 250 ms pour distinguer 100 et 130 Hz. Courte, elle réagit tout de suite et confond
tout ce qui est plus proche que son inverse. Les 43 ms par défaut (2048 échantillons à
48 kHz) sont un compromis pour un trig. La FFT fait toujours au moins deux fois la fenêtre,
complétée de zéros : une sinusoïde tombée entre deux cases y perd 0,4 dB au lieu de 1,4,
écart qui se lisait en amplitude linéaire. Le saut entre deux analyses reste à 512
échantillons au plus, et jamais plus d'une demi-fenêtre : une attaque ne passe pas entre
deux analyses. En `ConstantQ`, la durée plafonne le noyau le plus long.

**La durée reste un `Param`.** La règle du §2.2 tient ici aussi : `AudioAnalysis::window`
se module, et son consommateur la sature (`sanitize_window`, §18.4). C'est possible parce
que l'analyseur garde 1 s d'historique quelle que soit la fenêtre : en changer ne perd aucun
échantillon. Les trois autres réglages sont des choix qu'on tranche, pas des valeurs.
Ce qui traverse les threads est la version résolue, `AnalysisSettings`, par un petit anneau
`rtrb` du thread principal vers l'analyse, qui peut allouer : il n'est pas temps réel.

**Une zone se lit dans l'échelle où elle a été posée.** Ses fréquences sont en Hz et
survivent à tout changement d'échelle. Ses niveaux, eux, sont en unités du spectre :
passer de `Decibel` à `Linear` ne les convertit pas, et une zone à `0.7` n'y signifie plus
la même chose. Ce n'est pas un oubli : c'est le réglage d'un trig qu'on est en train de
changer, et le dire vaut mieux qu'une conversion que personne ne relirait.

**Quatre pièges déjà payés.** Le continu d'un micro fuit dans les premières bandes, celles
où l'on pose un trig de grosse caisse, et y tient la zone ouverte : la moyenne est retirée
avant la fenêtre. Une carte multicanale n'a souvent qu'une entrée branchée : la moyenne de
tous ses canaux perdrait 15 dB sur six, donc le mono ne moyenne que les canaux au-dessus de
-60 dB, décidés par bloc pour rester linéaire. Le callback écrit par blocs entiers ou pas du tout : couper une frame
stéréo mélangerait les canaux jusqu'à la fin de la session. Une entrée perdue rend le
silence, pas le dernier spectre : figé, il tiendrait une zone ouverte indéfiniment.

**Seul `spectrum` est calculé.** `rms`, `bands`, `centroid` et `onset` restent à zéro
jusqu'à la fin de l'étape 8.

### 7.1 Les features et le BPM (étape 8)

> Tranché pour l'étape 8. Raisons au §13.

**Une analyse à part, fixe.** `FeatureAnalyzer` a sa propre FFT, sans aucun lien avec
`AudioAnalysis` : le réglage du spectre sert à lire et à poser des zones, et le changer ne
doit déplacer ni le tempo ni une modulation.

| Champ de `AudioFeatures` | Mesure | Plage |
|---|---|---|
| `rms` | niveau global, de -60 à 0 dB | `[0, 1]` |
| `bands` | graves (< 250 Hz), médiums, aigus (> 4 kHz), en dB | `[0, 1]` |
| `centroid` | la brillance : centre du spectre sur l'axe logarithmique | `[0, 1]` |
| `onset` | `1` au transitoire, puis retombe en 120 ms | `[0, 1]` |
| `chroma` | l'énergie de chaque note, do en tête, la plus forte à `1` | `[0, 1]` |
| `bpm`, `bpm_confidence` | le tempo détecté (`0` : aucun), et sa fiabilité | BPM, `[0, 1]` |

**`onset` est une valeur qui retombe, pas un booléen.** Le thread d'analyse tourne plus vite
que les frames : un drapeau levé entre deux lectures du triple buffer serait perdu. Une
impulsion qui retombe survit à une lecture manquée, et reste une feature comme les autres.

**Une feature est un `Signal` : `AudioFeature`, dans `osc`.** Elle lit `ctx.audio` et lisse
par une attaque et un relâchement, deux `Param`. Son état tient dans `slots[0]`.

**Moduler sans éditeur de nœuds.** Chaque potentiomètre d'un clip propose une source audio
dans son menu de clic droit. `Command::SetAudioModulation` ajoute un nœud `AudioFeature` au
graphe du clip et y branche le `Param`, avec une amplitude de départ d'une demi-plage ;
`None` le rend fixe à sa base. Une modulation existante, un LFO par exemple, est remplacée.
Le nœud se sauvegarde comme les autres (§19.1).

**Le BPM.** `TempoTracker` garde 12 s de la force des transitoires, et réestime chaque
seconde : une autocorrélation, puis un peigne qui additionne ses valeurs à une, deux, trois
et quatre périodes, sur une grille de 70 à 180 BPM au demi-BPM. Le peigne donne la précision
qu'un seul retard n'a pas : à 86 analyses par seconde, un retard vaut 3 BPM autour de 120. Un
tempo double ou moitié se distingue mal : une courbe en cloche autour de 120 BPM tranche, et
la détection peut donc rendre 87 pour un morceau à 174. Elle attend 4 s de son, n'accepte un
saut de tempo que confirmé deux fois de suite, et rend `0` sans signal rythmique.

**Ce qui distingue un rythme d'un son tenu : le contraste, pas le niveau.** Une note tenue
produit une force minuscule, mais régulière, qui donnait un faux tempo avec une fiabilité
de 0,84. Un seuil absolu perdait une boucle 30 dB plus basse. Les coups doivent donc dépasser
cinq fois la force moyenne : mesuré, 4 pour une note tenue, 19 pour une boucle de batterie.

**Suivre le tempo.** `Composition::tempo_follow`, désactivé par défaut. Actif, chaque frame
écrit le BPM détecté dans le `Param` du tempo, si sa fiabilité dépasse 0,3 : le tempo garde
sa source unique (§3bis.3), et le dernier reste en place dans un silence. Taper ou fixer le
tempo coupe le suivi : un geste explicite gagne. Ce mouvement n'est pas une modification du
fichier (§23.8), seul le choix de suivre en est une. Le tempo se cale, pas la phase : le
battement détecté ne recale pas la position des clips.

**Déterminisme.** Rien de tout cela ne lit l'horloge : un rendu offline écrit ses propres
`AudioFeatures`, BPM compris, et obtient la même image (§20.3).

**Ce qui est vérifié, et comment.** Des boucles de batterie de synthèse (grosse caisse et
charleston) traversent toute l'analyse : 90, 100, 110, 120, 128, 140 et 150 BPM, à 44,1 et
48 kHz, se trouvent à moins d'1 BPM, et encore 128 BPM à 30 dB plus bas. Une note tenue, un
accord tenu, un bruit et un silence ne donnent aucun tempo. Chaque feature se teste sur des
sinusoïdes : le niveau, le registre, la note de la chroma. Un test du moteur montre le son
changer l'image par une source audio, un autre que le tempo suivi ne marque pas le fichier
comme modifié. La source audio se choisit par clics simulés, et se relit d'un fichier. Une
fixture v1 s'ouvre par migration. L'analyse coûte environ 0,4 % d'un cœur en release : 196 s
de son en 0,77 s, synthèse comprise. L'application tourne à 60 fps, et la fenêtre réelle a été
regardée sur une capture.

**Pas encore essayés :** de la vraie musique, ni pour le BPM, ni pour les features, ni pour un
trig : l'entrée de cette machine était silencieuse. Les seuils viennent de sons de synthèse
et peuvent demander un réglage. Le menu de clic droit à la vraie souris non plus. Un tempo
au-delà de 150 BPM n'est pas garanti : 174 peut se lire 87.

**Non couvert.** La phase : le tempo se cale, pas le battement. Une source audio sur
l'opacité d'une piste ou la vitesse d'un clip au clic droit : elles vivent dans le graphe de
composition, et se branchent depuis l'éditeur de nœuds (§23.10), comme le lissage d'une
source et la chroma.

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

### 8.0 Le contrat des sorties

> Tranché avant l'étape 6 (dilemme A). Le croquis initial —
> `fn submit(&mut self, frame: &FrameTexture, t: f64) -> Result<()>` — laissait trois
> questions ouvertes : que fait une sortie qui échoue, que reçoit-elle (texture GPU ou
> pixels CPU), et qui décide qu'une frame peut être perdue. Les trois réponses
> ci-dessous ; leurs raisons au §13.

**Deux familles de sorties, deux traits.** Les sorties **GPU** (preview, Spout, Syphon)
copient la texture sans quitter la carte. Les sorties **CPU** (NDI, enregistreur,
Art-Net, séquence d'images) ont besoin d'octets en mémoire, donc d'une relecture
GPU → CPU (§8.2) — l'opération la plus chère du pipeline. Un trait par famille : une
sortie ne peut pas recevoir la mauvaise forme de frame, c'est le compilateur qui le
garantit, pas un `match` à l'exécution.

```rust
pub enum SinkStatus {
    Active,
    Degraded { dropped: u32 },   // fonctionne, mais perd des frames ou se reconfigure
    Failed(String),              // n'est plus alimentée ; reste visible dans l'UI
}

pub struct GpuFrame<'a> {
    pub texture: &'a wgpu::Texture,   // sortie du compositeur : Rgba16Float linéaire prémultiplié (§21)
    pub index: u64,                   // croissant, sans trou en Clock::Fixed
    pub t: f64,                       // le t de CETTE frame
}

pub struct CpuFrame {
    pub pixels: Arc<[u8]>,            // RGBA8 encodé sRGB, alpha droit, lignes contiguës ; partagé, jamais copié par sortie
    pub width: u32,
    pub height: u32,
    pub index: u64,
    pub t: f64,                       // le t de la frame relue, pas le t courant (§8.2 : 2 à 3 frames de retard)
}

pub trait GpuSink {
    /// Phase 1 : enregistre ses copies / passes dans l'encodeur de la frame.
    fn encode(&mut self, gpu: &Gpu, encoder: &mut wgpu::CommandEncoder, frame: &GpuFrame<'_>);
    /// Phase 2 : après `queue.submit` — présenter la surface, signaler le récepteur
    /// Spout, publier la frame Syphon.
    fn after_submit(&mut self, gpu: &Gpu);
    fn status(&self) -> SinkStatus;
}

pub trait CpuSink: Send {
    /// Appelée sur le thread propre à la sortie : peut bloquer (disque, réseau).
    fn submit(&mut self, frame: &CpuFrame);
    fn status(&self) -> SinkStatus;
}

pub enum OutputSink {
    Gpu(Box<dyn GpuSink>),
    Cpu(Box<dyn CpuSink>),
}
```

**Erreurs : deux régimes, comme partout (§18.1).** L'**ouverture** d'une sortie —
bibliothèque NDI absente, fichier impossible à créer, surface refusée, backend
incompatible avec Spout — est de l'édition : le constructeur rend un `Result`, le refus
est clair. **`submit` et `encode` ne rendent rien** : une erreur par frame à 60 fps est
un flot inexploitable. Une panne en cours de route se lit dans `status()`. Une sortie
`Failed` n'est plus alimentée mais **n'est pas retirée** — l'utilisateur la voit et
décide. Le silence serait pire que l'erreur : l'enregistrement d'un concert de deux
heures qui s'arrête à la dixième minute sans que personne ne le voie est le scénario
que ce statut existe pour empêcher. Une surface périmée (changement d'écran,
redimensionnement) se reconfigure seule ; une surface **perdue** demande à être recréée
par `app`, qui possède la fenêtre (§8.3). Dans les deux cas c'est `Degraded`, pas
`Failed`.

**Deux phases pour les sorties GPU.** `encode` puis `after_submit` : la preview ne peut
appeler `surface.present()` qu'après la soumission, Spout doit signaler son récepteur
une fois le rendu terminé, Syphon publie après la soumission. Une seule méthode aurait
obligé chacune à soumettre son propre encodeur. Même forme que le contrat des couches
en plusieurs appels (§22).

**Une seule relecture, partagée.** L'**étage de sortie** (`Outputs`) relit la frame
**une fois**, et seulement si au moins une sortie CPU est active ; toutes les sorties
CPU reçoivent le même `Arc<[u8]>`. Avec NDI et l'enregistrement actifs, on ne paie pas
deux relectures. L'anneau du §8.2 n'existe qu'à un endroit.

**Une seule passe de conversion vers 8 bits sRGB**, composant de `output` partagé par
trois consommateurs : la preview (vers sa surface), Spout/Syphon (vers la texture
partagée) et le chemin CPU (vers la texture relue). Elle convertit **avant** la
relecture : on lit 4 octets par pixel au lieu de 8, et NDI, ffmpeg et Art-Net
consomment tous du 8 bits. La règle du §21 tient : tout l'intermédiaire reste
`Rgba16Float` linéaire, seules les cibles **finales** sont 8 bits encodées sRGB.

**Une `CpuFrame` est en alpha droit.** Prévu pour l'étape NDI, ce choix a dû être
avancé par la sortie PNG : dé-prémultiplier se fait **en linéaire, avant** l'encodage
sRGB. Après la passe, c'est trop tard — diviser des octets sRGB par l'alpha donne une
autre image. L'ordre de `linear_premul_to_srgb8` s'impose donc à la passe, et cette
fonction lui sert de référence CPU dans les tests. Si un récepteur NDI veut du
prémultiplié, c'est sa sortie qui le recalculera. Les cibles opaques (preview) gardent
le RGB prémultiplié : une zone transparente s'y lit sur du noir (§8.3).

**La politique de perte est portée par l'étage de sortie, pas par chaque sortie.**
Chaque sortie CPU a son thread et une file bornée (2 à 3 frames) que `Outputs` tient :

| Mode d'horloge (§4) | Sortie lente | Garantie |
|---|---|---|
| `Clock::Realtime` | dépôt sans attente ; si la file est pleine, la frame est perdue et comptée (`Degraded { dropped }`) | la boucle de rendu ne ralentit jamais |
| `Clock::Fixed` | dépôt bloquant : la boucle attend la sortie la plus lente | aucune frame perdue — indices contigus |

`DropPolicy::Lossy` et `Lossless` portent ces deux lignes, dans `fanout`, un module qui
ne connaît pas wgpu.

La politique est écrite **une fois**, et elle se teste ; si chaque sortie la décidait,
une seule erreur dans l'une d'elles ferait perdre des frames d'un rendu offline. Une
sortie qui panique n'emporte que son thread : elle passe en `Failed`. Les
sorties GPU tournent sur le thread principal (la surface y vit) et ne bloquent pas.

**`index` rend la perte visible.** Une sortie détecte un trou en comparant deux indices
successifs ; le test du §15.1 devient exact : en `Fixed`, une sortie factice reçoit
`0, 1, 2, …` sans trou.

**`Gpu` expose son backend** (`wgpu::Backend`), et acceptera une préférence de
backend : Spout exige DX12 sous Windows (§8.1). Si l'adaptateur obtenu ne convient pas,
c'est l'ouverture de la sortie qui échoue — en édition, avec un message.

**Ce que l'étape 6 implémente :** le contrat complet, l'étage `Outputs` et sa
politique `Realtime`/`Fixed`, la passe de conversion, `Engine` (§8.3), la sortie
`PngSequenceSink`, `Preview` (`GpuSink`) et des sorties factices pour les tests.
Jusqu'à l'étape 10, le chemin CPU s'appuie sur la relecture **synchrone** (§8.2) : elle
n'est permise qu'en `Fixed`, donc **`Outputs` refuse une sortie CPU en `Realtime`** à
l'ajout — en édition, avec un message. L'anneau asynchrone lèvera ce refus. La
politique `Realtime` de la file est néanmoins écrite et testée dès maintenant.

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

**Ce qu'ils ne transportent pas : une image.** Ils partagent un handle vers une texture
GPU ; le récepteur (Resolume, TouchDesigner, OBS) lit la même mémoire vidéo. Ni
relecture, ni encodage, latence quasi nulle — c'est tout leur intérêt, et c'est
pourquoi ce sont des `GpuSink` (§8.0). Un repli par le chemin CPU (relire, puis
réenvoyer en mémoire) serait simple et sans `unsafe`, mais perdrait cet intérêt.

| | Spout (Windows) | Syphon (macOS) | Linux |
|---|---|---|---|
| Partagé | texture **D3D11**, handle partagé DXGI | texture Metal adossée à une **IOSurface** | pas de standard équivalent : DMA-BUF via PipeWire plus tard, NDI d'ici là |
| Backend wgpu | **DX12** (interop vers D3D11) ; Vulkan possible mais bien plus lourd | Metal (seul backend) | — |
| Liaison | bibliothèque C++ SpoutDX, ou protocole Spout réimplémenté sur la crate `windows` | `Syphon.framework` (BSD) via `objc2`, embarqué dans le bundle | — |

**Points d'accès vérifiés dans wgpu 30.0.1** (en `unsafe`) : `Texture::as_hal` rend la
ressource native (`ID3D12Resource`, `MTLTexture`) ; `Device::create_texture_from_hal`
fait l'inverse — allouer soi-même une texture partageable et la présenter à wgpu comme
une texture ordinaire ; `Queue::as_hal` et `Device::as_hal` donnent la file et le
device natifs pour la synchronisation. Pas de fork de wgpu nécessaire. Mais
`wgpu-hal` est **moins stable** que l'API publique : chaque montée de majeure peut
casser Spout et Syphon — une raison de plus à l'épinglage du §11.

**Impacts à l'étape 11, gradés :**

| Impact | Effort | Risque |
|---|---|---|
| **`unsafe` par backend**, confiné dans `output/spout.rs` et `output/syphon.rs` sous `cfg` ; aucune autre crate n'en contient | élevé | élevé — une faute de synchronisation donne des images déchirées ou un plantage pilote, pas une erreur Rust |
| **Synchronisation entre processus** : le récepteur ne lit qu'une fois notre rendu terminé (fence partagée, ou flush + attente, ou le verrou propre à Spout) — c'est le rôle de `after_submit` | élevé | élevé — intermittent, dépend du pilote |
| **Backend imposé** : sous Vulkan, Spout est indisponible ; l'ouverture rend `Err` (§8.0). Vulkan est ce que wgpu choisit aujourd'hui sous Windows sur une RTX : il faudra demander DX12 explicitement | faible | moyen |
| **Même GPU des deux côtés** : sur un portable à deux GPU (Optimus), émetteur et récepteur peuvent être sur deux adaptateurs ; se détecte, s'affiche en `status()` | faible | moyen |
| **Format 8 bits** (BGRA8 ou RGBA8 attendu par les récepteurs) : la passe de conversion unique du §8.0 écrit directement dans la texture partagée | faible | faible |
| **Tests en boucle locale** : le test ouvre lui-même le handle partagé (device D3D11 ou client Syphon dans le même processus), relit et compare ; sauté proprement hors Windows / macOS ou sans GPU (§24.3). Un vrai récepteur se vérifie à la main | moyen | moyen |
| **Dépendances natives** : bibliothèque Spout ou protocole réimplémenté ; framework Syphon embarqué | moyen | moyen |

### 8.2 Enregistrement

**Le piège :** la lecture GPU → CPU est **asynchrone**. Lire la frame immédiatement
bloque le pipeline et effondre le framerate.

**La solution :** un anneau de buffers de readback avec 2 à 3 frames de latence — on
mappe la frame N-2 pendant que la frame N se dessine. C'est ce détail qui sépare 60 fps
de 20 fps pendant un enregistrement.

**État actuel :** seule une relecture **synchrone** existe
(`frogenx_render::read_rgba16f`, `read_rgba8`) — elle attend le GPU, ce qui est correct
pour le rendu offline et les tests, et **interdit** dans la boucle live. L'anneau
ci-dessus arrive avec l'enregistrement (étape 10, §12) ; ne pas réutiliser la version
synchrone pour cela.

**L'anneau vit dans l'étage de sortie (`Outputs`, §8.0), une seule fois**, quel que
soit le nombre de sorties CPU actives. Il relit la texture **déjà convertie en 8 bits
sRGB** (4 octets par pixel, pas 8). Chaque `CpuFrame` porte le `t` et l'`index` de la
frame relue, pas ceux de la frame en cours de rendu.

**Deux modes, adossés à `Clock` (§4) :**

- **Temps réel** — encodage pendant la performance ; des frames peuvent être perdues si
  l'encodeur décroche. Nécessaire en live.
- **Offline / déterministe** — pas de temps fixe, aucune frame perdue, qualité garantie,
  mais pas temps réel. Pour un rendu final propre.

**Encodage :** un pipe vers le binaire `ffmpeg` plutôt que des bindings. Plus simple à
démarrer, et surtout ffmpeg sélectionne lui-même l'encodeur matériel de chaque
plateforme (NVENC, VideoToolbox, Media Foundation) — un problème de portabilité qu'on
n'a pas à porter.

**D'ici l'étape 10, `PngSequenceSink`** écrit une image par frame, nommée d'après son
`index` (`frame_000000.png`). Elle donne un rendu offline déterministe sans encodeur ;
`ffmpeg` assemble ensuite la séquence (skill `poc-video`). L'ouverture crée le dossier
et rend un `Result` ; un disque plein en cours de rendu la fait passer en `Failed`.

### 8.3 Le moteur, la fenêtre et le rythme

> Tranché avant l'étape 6 (choix B à F).

**`app`, au sommet de la chaîne, possède le processus.** La boucle d'événements de
`winit` prend le contrôle du thread principal : une bibliothèque ne peut pas se
l'approprier. `output` ne connaît donc **pas** `winit` : la preview reçoit une
`wgpu::Surface` et sa taille. `app` a deux faces :

- une **bibliothèque**, `Engine`, dont `frame(wall_dt)` déroule les étapes 1 à 10 du
  §20.1 — sans fenêtre, donc testable, et utilisable pour un rendu offline ;
- un **binaire**, qui possède `winit` et appelle `Engine`. egui s'y branchera.

**La scène d'`Engine` est provisoire.** Une liste de groupes `{ graph, layers }`, chaque
groupe préfigurant un clip (§3, §22.1). L'étape 7 la remplacera par la grille de
`composition`.

**`Gpu::with_surface(display, cible)`** complète `headless()`. L'ordre est imposé par wgpu :
instance, surface, puis adaptateur compatible avec cette surface ; `Gpu` conserve donc
son `Instance`, nécessaire pour recréer une surface perdue. Générique sur la cible,
`layer` ne connaît pas `winit`.

**Encodage de la preview.** La passe a deux modes (`ConversionMode`) : `Straight` pour
une `CpuFrame`, au pixel près, et `OverBlack` pour une cible opaque, filtré et mis à
l'échelle. Dans les deux cas, elle regarde le format de sa cible :
`is_srgb()` → le matériel encode, sinon le shader. Elle écrit ainsi dans une surface
`*Srgb`, dans une texture `Unorm` que le récepteur lit comme du sRGB (Spout), et dans
les deux cas sans double encodage. La surface est opaque : le RGB prémultiplié se lit
sur du noir.

**Letterbox.** La preview conserve les proportions de la sortie et remplit le reste en
noir. Étirer ferait mentir la preview sur ce qui part vers le projecteur ; recadrer
cacherait les bords sans le dire. Le calcul du rectangle est une fonction pure, testée
sans GPU. Filtrage bilinéaire : une forte réduction crénelle un peu, ce qui est
acceptable pour un contrôle.

**Le moteur a sa propre échéance ; la preview n'est qu'une consommatrice.** wgpu 30
répond `Occluded` quand une fenêtre est masquée ou réduite : si la synchronisation de
l'écran rythmait la boucle, réduire la fenêtre risquerait d'arrêter NDI et
l'enregistrement. `app` cadence donc `Engine` sur une échéance (`ControlFlow::WaitUntil`,
60 fps cible), et une preview indisponible saute sa frame sans rien ralentir.

Présentation : `Mailbox` si la surface le propose, sinon `Fifo` (fonction pure de
choix, testée). Jamais `Immediate`, qui déchire l'image sur un projecteur.

| `get_current_texture` rend | La preview |
|---|---|
| `Success` | affiche |
| `Suboptimal` | affiche, puis reconfigure |
| `Occluded` | saute la frame, sans la compter : une fenêtre masquée n'a rien d'anormal |
| `Timeout`, `Validation` | saute la frame et la compte (`Degraded`) |
| `Outdated` | reconfigure, saute la frame |
| `Lost` | compte la frame ; `app` recrée la surface et la lui rend |

**`PresentSurface` : le tableau ci-dessus, écrit une seule fois.** La preview et la
fenêtre de l'interface (§23.5) présentent toutes deux sur une surface : acquisition,
reconfiguration et perte vivent dans ce type de `output`, qui leur est commun. Il suit
une préférence de format : `*Srgb` pour la preview, `Unorm` pour l'interface.

**`MonitorSink` : l'image dans l'interface.** Un `GpuSink` qui écrit la frame dans une
texture `Rgba8Unorm` de taille réduite, par la passe de conversion en mode `OverBlack` ;
le shader encode le sRGB (§8.3, encodage). `Rgba8Unorm` est imposé par egui
(`register_native_texture`), qui lit cette texture sans la décoder. Donner à egui la
texture du compositeur serait faux deux fois : elle est linéaire en 16 bits, et change
d'une frame à l'autre (ping-pong, §22).

**`PreviewHandle`.** `Outputs` possède la preview, mais c'est `app` qui connaît la
fenêtre : `Preview::new` rend aussi une poignée, par laquelle `app` signale une nouvelle
taille, voit qu'une surface est perdue et en fournit une autre. Tout s'applique au début
d'une frame : wgpu **panique** s'il reconfigure une surface dont une texture est encore
vivante, ou si la taille est nulle — celle d'une fenêtre réduite, que la preview saute
donc sans configurer.

**Vérifier une fenêtre.** Tout ce qui peut s'extraire se teste sans elle : `Engine` avec
des sorties factices, la passe hors écran, les fonctions pures. Un mode `--frames N`
ouvre la fenêtre, présente N frames et quitte : il prouve l'ouverture et la présentation,
hors CI puisqu'il faut un écran. La fluidité reste une vérification à l'œil.

### 8.4 La sortie plein écran (étape 7d)

> Tranché avant l'étape 7d (choix A1, B1, C1, D1). Raisons au §13.

**Le geste.** Le menu `Sortie` liste les écrans branchés. Un clic ouvre la sortie sur
l'écran choisi, un clic sur un autre l'y déplace, `Fermer la sortie` la ferme. Rien ne
s'ouvre au lancement : une image qui part seule sur un mauvais écran serait pire que pas
d'image. Le statut se lit dans l'en-tête.

**Une fenêtre sans bordure, pas de plein écran exclusif.** Le mode vidéo de l'écran ne
change pas : pas de noir à l'ouverture, et l'interface garde la main sur l'autre écran.

**1920×1080 fixe.** La sortie ne suit pas l'écran : le compositeur ne réalloue rien, et
`Preview` applique son letterbox (§8.3). Un projecteur 16:10 reçoit des bandes noires, un
écran 4K une image agrandie. Choisir la résolution viendra après la première version.

**Sur l'écran de l'interface, une fenêtre normale.** Un plein écran y cacherait le menu qui
le ferme. Elle sert aussi à travailler avec un seul écran.

**Retirer une sortie.** `Failed` ne retire jamais une sortie (§8.0), parce que seul
l'utilisateur en décide : fermer est précisément ce geste. Seules les sorties GPU se
retirent pour l'instant ; une sortie CPU suppose d'arrêter son thread, ce que l'enregistreur
de l'étape 10 décidera. Cela se fait entre deux frames, en régime d'édition :

```rust
impl Outputs {
    pub fn add_gpu(&mut self, sink: Box<dyn GpuSink>) -> GpuSinkId;
    pub fn remove_gpu(&mut self, id: GpuSinkId) -> bool;   // false : déjà retirée
}
```

`Engine` les expose en `add_gpu_output` et `remove_gpu_output`, et ne sait toujours rien
d'une fenêtre. La `wgpu::Surface` tient un `Arc<Window>` : la fenêtre ne se ferme vraiment
que lorsque `Preview` a quitté `Outputs` **et** que `app` a lâché le sien.

**Deux fenêtres, une boucle.** `app` aiguille les événements par `WindowId`. Ceux de la
sortie n'atteignent pas egui, sauf le clavier : si la sortie prend le focus par accident, 1
à 8 et Espace doivent continuer de jouer.

**Le focus reste à l'interface.** Une fenêtre normale est créée non active. Un plein écran
active toujours sa fenêtre sous Windows, et pas seulement à la création comme le laissait
croire `force_window_active` : le `SetWindowPos` de `set_fullscreen` n'a pas
`SWP_NOACTIVATE`. Une fenêtre sans bordure, sans drapeau de plein écran, y échapperait mais
laisserait la barre des tâches passer devant l'image. Le plein écran rend donc le focus à
l'interface dès l'événement `Focused(true)`, ce qui couvre aussi un clic accidentel sur la
sortie. Le curseur y est masqué. La taille vient ensuite de l'événement `Resized`, car
winit positionne la fenêtre de façon asynchrone.

**Fermer ne tient qu'à un menu.** Aucune touche ne ferme la sortie, Échap compris. Alt+F4
envoie `CloseRequested` : en plein écran, `app` l'ignore. Sur une fenêtre normale, sa croix
ferme la sortie, jamais l'application.

**Un écran qui disparaît.** Windows déplace alors la fenêtre sur un écran restant, donc
potentiellement par-dessus l'interface. winit 0.30 n'a pas d'événement pour cela : `app`
relit la liste des écrans chaque seconde, et aussitôt si la sortie est déplacée ou
redimensionnée. Si l'écran manque, la sortie se ferme, le moteur continue, et l'en-tête dit
`Sortie fermée : écran débranché`. Elle ne se rouvre pas seule au retour de l'écran. Un
écran s'identifie par son nom (`\\.\DISPLAY2` sous Windows), plus stable qu'un `HMONITOR`.

**L'interface ne connaît pas winit.** `app` lui prête la liste des écrans déjà mise en mots
et reçoit un `OutputRequest`, qui n'est pas une `Command` : il concerne la fenêtre, pas la
grille.

```rust
pub struct Screen { pub label: String, pub interface: bool }

pub struct OutputView<'a> {
    pub screens: &'a [Screen],
    pub open: Option<usize>,   // indice dans `screens`
    pub status: &'a str,
}

pub enum OutputRequest { Open(usize), Close }
```

**Le coût.** Ouvrir ou fermer la sortie crée ou détruit une surface sur le thread du
moteur : l'intervalle maximal est monté à 46 ms sur un enchaînement d'ouvertures, soit deux
ou trois frames. Le geste reste rare, et le moteur ne prend pas de retard : une frame en
retard ne se rattrape pas (§8.3). Hors de ces gestes, la seconde surface ne se voit pas
dans les intervalles (16,2 à 17,4 ms en 1080p, interface 0,49 ms par frame).

**Ce qui est vérifié, et comment.** `--output N` ouvre la sortie sur l'écran N dès le
lancement : combiné à `--frames`, il prouve l'ouverture, la présentation et la fermeture
sans souris. Le retrait d'une sortie se teste sur des sorties factices, le menu par clics
simulés, les règles de `stage.rs` par fonctions pures. Sur trois écrans réels (Windows,
Vulkan, RTX 4070 SUPER), chaque fenêtre a été relevée pendant l'exécution : le bon écran à
sa taille exacte, 300 frames présentées sur 300, le focus resté à l'interface, et des
couleurs identiques au pixel près entre la sortie et le monitor. Un écran en portrait
(1050×1680) a confirmé le letterbox. `WM_CLOSE` est ignoré par le plein écran, et ferme une
fenêtre normale sans quitter l'application. Un binaire temporaire a enchaîné ouverture,
fermeture, déplacement sur chaque écran et perte d'écran simulée, le moteur tenant 59,8 fps
sur 1080 frames.

**Pas encore essayés :** le menu à la vraie souris (un clic posté n'atteint pas egui, §23.5),
un vrai câble débranché, un projecteur, un écran à une autre fréquence que 60 Hz, et Linux
et macOS. Sur ces deux systèmes, deux écrans de même nom se confondraient dans la détection
d'un écran perdu. Si une surface n'offrait pas `Mailbox`, `Fifo` pourrait ralentir le moteur
sur un écran lent : le cas ne s'est pas présenté ici.

---

## 9. Portabilité

Cible : **Linux, macOS, Windows**, à parité.

### 9.1 Graphique
wgpu abstrait Vulkan / Metal / DX12. C'est la raison même de ce choix.
**Discipline :** rester en WGSL, ne jamais supposer un backend, tester tôt sur les trois.

### 9.2 Audio
`cpal` couvre ALSA / CoreAudio / WASAPI. **Piège classique :** les tailles de buffer et
les latences diffèrent nettement selon la plateforme. Ne jamais coder en supposant une
taille de buffer fixe. Sur Linux, compiler `cpal` demande les en-têtes ALSA
(`libasound2-dev`), en CI comme sur un poste. Seul Windows a été essayé à ce jour.

### 9.3 Vidéo
Le décodage est le morceau le plus lourd du projet : thread de décodage dédié, buffer
de frames, upload GPU, synchronisation avec `Clock`, scrubbing et boucle, conversion
YUV → RGB en shader. Plusieurs jours de travail, **indépendants du reste** — d'où sa
place en dernier.

---

## 10. Structure des crates

Les crates vivent sous `crates/`, le workspace est à la racine. `core`, `osc`, `shape`,
`composition`, `audio`, `library`, `patch`, `layer`, `render`, `output`, `ui` et `app` existent à ce jour ; les suivantes sont créées au fil de la feuille de
route (§12).

```
frogenx/
├── core/      # Signal, Param, Graph, Clock          ← aucune dépendance ✅
├── osc/       # oscillateurs, LFO, enveloppes, bruit, feature audio ✅
├── shape/     # primitives, opérateurs, style (ColorParam, BlendParams) ✅ (core, osc, glam)
├── scene3d/   # maillages, caméra, lumières, culling (core, osc, glam)
├── audio/     # cpal, FFT, spectre, features, tempo → AudioFeatures ✅ (core, cpal, rustfft, rtrb, triple_buffer)
├── composition/ # Deck, LayerGroup, Layer, Column, Clip, CrossFader, TempoController, Trig, Command ✅ (core, osc, shape)
├── library/   # catalogue de clips prédéfinis, grille de départ ✅ (core, osc, shape, composition)
├── patch/     # format de fichier : Composition ↔ JSON, version, écriture sûre ✅ (core, osc, shape, composition, serde_json)
├── layer/     # Gpu, trait RenderLayer, ShapeRenderer, ShapeLayer, FeedbackRenderer, tessellation ✅ (core, shape, wgpu, lyon)
├── render/    # compositeur, readback GPU → CPU, PNG              ✅ (core, layer, wgpu, png)
├── output/    # GpuSink / CpuSink, étage Outputs, passe 8 bits sRGB, PresentSurface, Preview, MonitorSink, PngSequenceSink ✅ (layer, render, wgpu) ; à venir : NDI / Spout / Syphon / Art-Net / recorder
├── ui/        # egui : grille, bibliothèque, tempo, monitor, réglages, éditeur de nœuds ; rend des Command ✅ (library, composition, shape, osc, core, egui)
└── app/       # Engine (la boucle du §20.1, sans fenêtre) + binaire `frogenx`, seul à connaître winit, egui-wgpu, egui-winit ✅
```

**Découpage `layer` / `render` (étape 5).** `layer` porte le contexte `Gpu` (device et
queue), le trait `RenderLayer` (§22) et ses implémentations ; `render` porte ce qui
**consomme** les couches — le compositeur, la cible offscreen finale et la relecture
GPU → CPU (§8.2). La tessellation (polylignes → triangles, `lyon`) vit dans `layer`,
mais dans un **module qui ne connaît pas wgpu** : elle prend un `Path` (§16.3) et rend
des sommets et des indices en mémoire CPU. C'est le morceau de l'étape 5 où se
logent les cas limites (contour dégénéré, auto-intersection, contour ouvert), et il
reste ainsi testable en CI sans GPU, comme `shape` (§24.3). Seuls l'upload des buffers
et la passe de rendu exigent un adaptateur.

**`composition` dépend de `core` et `shape`, pas seulement de `core`.** Révision du
sketch initial de §3bis.2 : `Clip.effects` référence une chaîne `ShapeOp` (§16.3 bis),
dont le type vit dans `shape`. Deux routes existaient pour éviter cette dépendance —
un identifiant opaque résolu plus haut dans la pile (§5, §13), ou accepter la
dépendance — et c'est la seconde qui est retenue : `shape` elle-même ne dépend que de
`core`/`osc`/`glam` (aucun GPU, aucune carte son), donc `composition → shape` ne casse
**pas** la testabilité sans GPU qui motive la règle de dépendance (§10) — elle déplace
seulement la position de `composition` dans la chaîne, sans l'exposer à wgpu ni cpal.
Trier des decks, détecter des conflits de connexion sur une colonne, valider une
transition de clip restent vérifiables sur des structures de données pures. La
distinction qui compte n'est donc pas « combien de crates en amont », mais « est-ce
que l'une d'elles connaît le GPU, la carte son ou une fenêtre » —`shape` n'en connaît
aucun.

**`composition` dépend aussi de `osc`** pour les nœuds d'un `Trig` (§3bis.6), sans plus
de conséquence : `osc` ne connaît ni GPU ni carte son.

**`audio` s'arrête à `AudioFeatures`.** Elle ne connaît pas la grille : `app` lit l'entrée
et pose les features dans `Engine`. Seul `capture.rs` touche à `cpal` : l'analyse et le
thread d'analyse se testent avec des signaux de synthèse, sans carte son :
`audio/tests/trig.rs` fait passer une boucle de batterie par la vraie FFT, la vraie zone et
le vrai compteur (`osc` y est une dépendance de test). Les `Signal`
de l'audio vivent dans `osc`, car ils ne lisent que `Ctx` : `composition` peut donc les
câbler sans dépendre d'`audio`.

**Conséquence sur l'ordre de la chaîne de dépendance (§0, en-tête du document) :**
`composition` se place après `shape`, pas après `core` directement —
`core → osc → shape → composition → audio → library → layer → render → output → ui → app`. `audio`
reste indépendant de `composition` (une feature audio est un `Signal`, peu importe où
il est consommé) ; son déplacement dans la liste est seulement pour lire la chaîne de
gauche à droite dans l'ordre réel des flèches.

**`ColorParam`, `ShapeStyle`, `BlendParams` vivent dans `shape`, pas dans `layer`.**
Un clip et une piste les portent, et `composition` ne peut pas dépendre de `layer` sans
tirer wgpu. Ce sont quatre `Param` et une énumération : rien en eux ne touche au GPU.
`layer` les réexporte.

**`library` est du contenu, pas de la mécanique.** `composition` décrit ce qu'est une
grille ; un clip prédéfini est une œuvre qui s'en sert, en assemblant des LFO et des
formes. Le catalogue vit donc à part, au-dessus d'`osc` et de `composition`, sans GPU ni
carte son, et se teste entièrement en CI. Placée après `audio`, `library` pourra proposer
des clips audio-réactifs.

**`app` dépend de `composition`** : `Engine` tient la grille et la traduit en couches.
`composition` ne connaît toujours rien du rendu.

**`ui` ne connaît ni wgpu ni `winit`.** Elle lit la grille et rend des `Command` ;
`egui-wgpu` et `egui-winit` restent dans `app`, seul à posséder la fenêtre. C'est ce qui
permet de la tester par des clics simulés, sans GPU ni écran (`egui_kittest`).

**`scene3d` suit la même règle que `shape`.** Primitives 3D (maillages, caméra,
lumières) et opérateurs, sans connaître wgpu ni l'audio — exactement le rôle que
`shape` jouait pour le 2D, étendu à la 3D plutôt que remplacé. `Clip.effects` pourra,
selon `ClipSource`, référencer une chaîne de `shape::ShapeOp` ou de son équivalent
`scene3d` — même principe de dépendance que ci-dessus, appliqué symétriquement.

**`output` dépend de `layer` et `render`, et c'est la seule crate autorisée à contenir
de l'`unsafe`** — confiné aux modules Spout et Syphon, sous `cfg` (§8.1). L'étage de
sortie, la politique de perte et la passe de conversion n'en contiennent pas : la
politique `Realtime`/`Fixed` se teste avec des sorties factices, sans GPU ni fenêtre.
`output` ne connaît pas `winit` : c'est `app` qui possède la fenêtre (§8.3).

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
| Rendu | `wgpu` 30 | GPU multiplateforme (étape 5) |
| Fenêtrage | `winit` 0.30 ✅ | fenêtre et boucle d'événements, dans `app` seulement (§8.3) |
| UI | `egui` 0.37 ✅ | grille de clips, réglages (`ui`) ; `egui-wgpu`, `egui-winit` dans `app` ; `egui_kittest` en test ; feature `default_fonts` requise |
| Bruit | interne à `osc` ✅ | value noise et fBm à graine explicite — pas la crate `noise` (§13, §20.3) |
| Tessellation | `lyon_tessellation` | `Path` → triangles, remplissage et trait (étape 5) |
| GPU, utilitaires | `bytemuck`, `pollster` | octets des buffers de sommets ; attente synchrone de l'adaptateur (étape 5) |
| Image | `png` | écriture de la première image et des snapshots de test (étape 5) |
| Audio | `cpal` 0.18 ✅ | capture multiplateforme, dans `audio` seulement |
| FFT | `rustfft` 6 ✅ | analyse spectrale |
| Lock-free | `rtrb` 0.4, `triple_buffer` 9 ✅ | passage audio → rendu |
| Sérialisation | `serde`, `serde_json` ✅ | sauvegarde de patchs, dans `patch` seulement pour le JSON (§19) |
| Dialogues | `rfd` ✅ | boîtes de fichiers du système, dans `app` seulement (§23.8) |
| Encodage | binaire `ffmpeg` (pipe) | enregistrement, décodage de clips (§9.3) |
| Éclairage | `artnet` ou équivalent | sortie Art-Net / DMX (§3bis, §8) |
| Scripting | `mlua` | logique de composition personnalisée (§23.4) |
| Contrôle live | `midir` (MIDI), `rosc` (OSC) | pilotage de l'interface et des effets par surface externe (§22bis.1) |

**`wgpu` est épinglé à une version majeure précise (30), et `rust-version` du workspace
suit son minimum (1.87).** wgpu publie une version majeure environ tous les trois mois,
avec des changements d'API à chaque fois : une contrainte lâche ferait changer l'API
sous les pieds du projet à la première mise à jour. Une montée de version est un
changement délibéré, fait dans son propre commit, avec les tests de rendu (§24) comme
filet. Le `rust-version` du workspace passe de 1.80 à 1.87 en conséquence : déclarer un
minimum inférieur à celui d'une dépendance serait un mensonge que `cargo` ne détecte
qu'à la compilation chez celui qui a la vieille toolchain.

**`winit` est épinglé en 0.30 parce qu'`egui` l'impose** : `egui-wgpu` 0.36 et 0.37, les
seules versions pour wgpu 30, demandent `winit ^0.30.13`, et la 0.31 est encore en bêta
à la date de ce choix. **`rust-version` passe à 1.95**, le minimum d'egui 0.37, qui
dépasse désormais celui de wgpu. Vérifié sur crates.io.

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
| 4 | ✅ `composition` : `Deck`/`LayerGroup`/`Layer`/`Column`/`Clip`, `CrossFader`, `TempoController` (§3bis) | **la grille existe, sans rendu** — *fait* |
| 5 | ✅ `RenderLayer` + `ShapeLayer` + compositeur, offscreen | **première image** (écrite en PNG — la fenêtre arrive à l'étape 6) — *fait* : `cargo run -p frogenx-render --example first_image` |
| 6 | ✅ `OutputSink` (`GpuSink`/`CpuSink`, contrat au §8.0) + étage `Outputs` + preview | pipeline de sortie en place, **première fenêtre** — *fait* : `cargo run` ouvre l'application ; `cargo run -p frogenx-app --example sequence` rend le **premier clip** |
| 7 | ✅ La grille pilote le rendu : `ShapeRenderer` par piste, `Command`, tempo dans `Ctx` | **le modulaire prend vie dans la grille** — *fait* : `cargo run`, touches 1 à 4, Espace pour taper le tempo |
| 7b | ✅ Interface egui : grille cliquable, tempo, monitor, réglages du clip (§23.5) | **l'application se pilote à la souris** — *fait* : `cargo run` |
| 7c | ✅ Clips prédéfinis : grille à cases, `library`, glisser-déposer, vitesse et amplitude (§23.6) | **une grille qu'on remplit soi-même** — *fait* : `cargo run`, glisser un clip sur une case |
| 7d | ✅ Sortie plein écran sur un second écran (`Preview`, §8.4) | **usage sur scène** — *fait* : `cargo run`, menu `Sortie` |
| 7e | ✅ Sauvegarde des patchs (§19, §23.8) | **retrouver son travail** — *fait* : `cargo run`, menu `Fichier` |
| 8 | ✅ Audio → features → `Signal`, BPM calculé, déclenchements (§7, §7.1) | **visuel audio-réactif** — *fait* : `cargo run`, clic droit sur un potentiomètre, `Suivre` |
| 9 | ✅ Feedback : un clip qui relit l'image finale (§6.1) | **traînées, échos, complexité organique** — *fait* : `cargo run`, famille `Échos` de la bibliothèque |
| 10 | Enregistrement (readback + ffmpeg) | fichier vidéo |
| 11 | NDI, puis Spout / Syphon | intégration VJ |
| 12 | `VideoLayer` | clips vidéo comme couches |
| 13 | `scene3d` : maillages, caméra, lumières, culling | scène 3D comme source de clip |
| 14 | Art-Net / DMX en sortie | pilotage d'éclairage physique |
| 15 | Scripting Lua (`mlua`) sur `Command` (§23.4) | logique de composition scriptable |
| 16 | Shaders d'effets custom en WGSL natif (§23.4) | effets utilisateur sans recompiler FrogenX |
| 17 | Entrées MIDI/OSC, `Param3d::Bind` sur `scene3d` (§22bis.1) | pilotage live de l'interface et des effets par surface de contrôle externe |

**Première version utilisable : jusqu'à l'étape 8 incluse, atteinte.** Les étapes 7b à 7e ont été
insérées quand le plan a révélé qu'aucune étape ne couvrait l'interface. On y ouvre
l'application, on déclenche des clips dans une grille, on règle leurs paramètres, on
envoie l'image sur un projecteur, on retrouve son travail, et le son pilote l'image.
L'éditeur de nœuds, l'enregistrement, NDI, Spout, la vidéo, la 3D et le MIDI/OSC
viennent ensuite.

**Les étapes clés :** la 4 (la grille devient le squelette organisationnel, avant même
le rendu — tout ce qui suit s'y raccroche), la 5 (première image), la 7 (le concept
modulaire se prouve *dans* la grille), la 8 (l'audio-réactivité, cœur du projet).

**L'étape 4 passe devant le rendu, à rebours du vertical slice habituel**, parce que
la grille est désormais la structure que toutes les étapes suivantes organisent. Y
raccrocher `RenderLayer`/`OutputSink`/`VideoLayer` après coup serait une réécriture ; les
construire directement au-dessus d'une grille déjà en place ne l'est pas.

**Les étapes 5 et 6 restent l'investissement anticipé** du plan : elles coûtent peu
maintenant et évitent une réécriture complète du rendu quand la vidéo et les sorties
multiples arrivent. La 3D (étape 13) et l'éclairage (étape 14) viennent après que le
pipeline vidéo/audio-réactif est prouvé, pour la même raison qu'avant la réorientation
la vidéo venait en dernier (§9.3) : travail conséquent, largement indépendant du reste.

### 12.1 Le backlog des features

La feuille de route dit l'ordre ; ce tableau dit où en est chaque feature, y compris celles
ajoutées en cours de route et celles qui n'ont pas de numéro. À tenir à jour dans le commit
qui les livre (skill `doc-sync`).

**Livré**

| Feature | Détail |
|---|---|
| Signaux et graphe | oscillateurs, LFO, bruit à graine, enveloppes, opérateurs, 15 types de nœuds |
| Formes | 8 générateurs, 5 effets, style de remplissage et de trait |
| Grille | decks, pistes, colonnes, cases, tempo partagé, tap tempo, 5 modes de mélange |
| Bibliothèque | 27 clips calés sur le tempo, grille de départ (§23.6) |
| Interface | en-tête unique, potentiomètres, panneau de clip flottant, apparence façon Pro-Q (§23.9) |
| Sortie plein écran | menu `Sortie`, fenêtre sans bordure, écran perdu géré (§8.4) |
| Sauvegarde | format JSON versionné, migrations, dernier fichier rouvert, confirmation (§19, §23.8) |
| Spectre | 3 méthodes, 4 échelles de fréquence, 2 d'amplitude, durée réglable (§7) |
| Trig | zone tracée sur le spectre, compteur « 1 sur N », cible clip ou colonne (§3bis.6) |
| Features audio | volume, registres, brillance, transitoires, chroma, comme `Signal` (§7.1) |
| Modulation par le son | source audio au clic droit de chaque potentiomètre (§7.1) |
| BPM | détection et suivi du tempo (§7.1) |
| Feedback | clips d'écho : traînée, tunnel, tourbillon, réglés par seconde (§6.1) |
| Éditeur de nœuds | fenêtre à part, graphes de clip et de régie, effets, sources sur l'opacité et la vitesse (§23.10) |
| Annuler et rétablir | 100 pas, un par geste, sans faire sauter l'image, Ctrl+Z et menu `Édition` (§23.11) |
| Rendus sans fenêtre | séquence PNG déterministe, planche du catalogue |

**À faire, dans l'ordre conseillé**

| Priorité | Feature | Pourquoi | Taille |
|---|---|---|---|
| 1 | Transitions `Fade` et `Wipe` | seule la coupure franche s'applique (§3bis.2) | moyenne |
| 2 | `PlayMode` | `Loop`, `Once`, `PingPong` n'ont pas d'effet avant la vidéo (§3bis.3) | petite |
| 3 | Choix de la résolution de sortie | fixe en 1080p (§8.4) | moyenne |
| 4 | Sélecteur de couleur | les couleurs se règlent canal par canal (§23.5) | petite |
| 5 | Recalage de la phase du tempo |
| 6 | Éditeur de nœuds, suite | la place des nœuds dans le fichier, le zoom, l'ordre des effets (§23.10) | moyenne | le tempo se cale, pas le battement (§7.1) | moyenne |
| 7 | Enregistrement, étape 10 | un fichier vidéo ; lève le refus des sorties CPU en direct | grande |
| 8 | NDI, puis Spout et Syphon, étape 11 | l'image vers d'autres logiciels | grande |
| 9 | Clips vidéo, étape 12 | le vrai morceau (§9.3) | grande |
| 10 | Scène 3D, étape 13 | | très grande |
| 11 | Art-Net / DMX, étape 14 | | moyenne |
| 12 | Scripting Lua, étape 15 | | moyenne |
| 13 | Shaders personnalisés WGSL, étape 16 | | moyenne |
| 14 | MIDI et OSC, étape 17 | une surface de contrôle sur scène | moyenne |

**À vérifier de main d'homme**

| Point | État |
|---|---|
| BPM, features et trig sur de la vraie musique | jamais essayés : seuils venus de sons de synthèse (§7.1) |
| Menus `Sortie`, `Fichier` et `Édition`, clic droit des potentiomètres, fils de l'éditeur, Ctrl+Z, à la vraie souris et au vrai clavier | validés par clics simulés seulement |
| Projecteur, câble débranché, écran à une autre fréquence | jamais essayés (§8.4) |
| Linux et macOS | jamais essayés |

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
| **Le trig : deux `Signal` dans `osc` (`SpectrumZone`, `Divider`), câblés par un `Trig` de `composition`** (§3bis.6) | ils ne lisent que `Ctx`, donc déterministes et testés sans carte son ; un clip peut s'en servir pour moduler ses `Param` ; `Trig` porte seulement la cible et survit aux changements de clip |
| **Une zone se franchit par le bas, une seule fois** (§3bis.6) | une attaque traverse la zone deux fois : sans ce contrat, chaque coup compterait double et le compteur serait faux ; le haut de la zone garde un sens en excluant ce qui saute au-dessus |
| **`Divider` compte des fronts et laisse passer le premier** (§3bis.6) | il ignore la musique, donc toute porte s'y branche ; partir du premier répond tout de suite et cale le compte sur le temps fort |
| **Spectre : un nombre fixe de bandes, qui porte ses échelles, dans `core`** (§7) | une taille fixe traverse le triple buffer et `Ctx` sans allouer ; `core` n'en sait pas l'origine, un rendu offline écrit le sien ; les échelles voyagent avec lui, donc une zone en Hz se relit juste sur tout découpage |
| **L'analyse se règle dans la composition** (`AudioAnalysis`), pas dans l'interface ni dans `audio` (§7) | une zone n'a de sens que relue avec l'analyse qui l'a fait poser : un fichier doit garder les deux ; `composition` ne touche pas la carte son, elle résout, et `app` transmet |
| **La durée d'analyse est un `Param`, l'analyseur garde 1 s d'historique** (§7) | la règle du §2.2 n'a pas d'exception : changer de fenêtre ne perd aucun échantillon, donc la moduler reste possible ; les méthodes et échelles sont des choix, pas des valeurs |
| **Trois méthodes sur un même découpage**, pas un analyseur par usage (§7) | les bandes viennent de la `FreqScale` et la mesure de l'`AnalysisMethod` : toute méthode vaut sur toute échelle, et le `Spectrum` en sortie reste le même contrat pour le trig et l'interface |
| **Une zone garde ses niveaux quand l'échelle d'amplitude change** (§7) | c'est le réglage du trig qu'on change ; une conversion silencieuse cacherait qu'une zone à 0.7 ne veut plus dire la même chose |
| **L'image de sortie n'est jamais couverte par les réglages** (§23.9) | un spectre se lit sous un panneau, une image de sortie doit se voir entière ; c'est la seule entorse au modèle du spectre |
| **L'image ne dépend pas de la sélection** (§23.9) | choisir un clip ne doit pas la faire sauter en plein set |
| **Un seul potentiomètre pour tous les réglages** (`widgets::knob`), avec couronne de modulation (§23.9) | un seul geste à apprendre ; la modulation d'un `Param` se voit là où il se règle |
| **Le pliage des réglages est calculé** (§23.9) | egui ne replie que ce dont il connaît la taille d'avance : les derniers réglages sortaient du panneau sans le dire |
| **Un en-tête unique** au lieu d'un menu et d'une barre (§23.9) | une ligne gagnée pour l'image, et le tempo au plus près des menus |
| **Apparence de Pro-Q, disposition de Resolume** (§23.1) | la disposition répond à la conduite d'un set, l'apparence à la lecture d'un son ; le spectre est devenu le cœur de l'audio-réactivité |
| **Une couleur par trig, enfermée dans le spectre** (§23.1) | des zones qui se superposent se distinguent par leur couleur ; hors du panneau, l'or reste le seul ton saturé, et rien ne concurrence le monitor |
| **Les gestes sur le spectre se calculent en coordonnées du tracé** (§23.7) | ni Hz ni pixels : un même geste vaut sur toutes les échelles, et se teste sans egui |
| **Un `Signal` lit l'audio dans `Ctx`, pas dans le triple buffer** (§7, §20.1) | le buffer se lit une fois par frame, à l'étape 2 : deux lectures pendant l'évaluation verraient deux spectres, et le rendu ne serait plus déterministe |
| **`Graph::replace`, plutôt qu'un `Param` partagé** (§3bis.6) | un `Signal` est opaque dans le graphe ; le remplacer garde son identifiant, ses dépendances et son état, donc le compte d'un trig qu'on règle en jouant |
| NDI avant Spout / Syphon | seule techno des trois plateformes ; les autres sont `unsafe` et tardives |
| **Deux traits de sortie, `GpuSink` et `CpuSink`**, réunis dans `enum OutputSink` (§8.0) | une sortie ne peut pas recevoir la mauvaise forme de frame, garanti à la compilation ; la relecture GPU → CPU, l'opération la plus chère, n'est payée que si une sortie CPU est active |
| **Une relecture unique partagée** (`Arc<[u8]>`) entre toutes les sorties CPU (§8.0, §8.2) | NDI + enregistrement actifs ne paient pas deux relectures ; l'anneau du §8.2 n'existe qu'à un endroit |
| **Pas de `Result` par frame** : ouverture → `Result`, puis `status()` (`Active`/`Degraded`/`Failed`) (§8.0) | une erreur par frame contredit le §18.1 ; un échec silencieux perd un enregistrement sans que personne ne le voie — le statut le rend visible sans interrompre |
| **`GpuSink` en deux phases** (`encode` / `after_submit`) (§8.0) | `surface.present()`, le signal Spout et la publication Syphon viennent tous **après** `queue.submit` ; une seule méthode obligerait chaque sortie à soumettre son propre encodeur |
| **Politique de perte portée par l'étage `Outputs`**, pas par chaque sortie (§8.0) | `Realtime` perd et compte, `Fixed` attend : écrit une fois et testé ; décidé six fois, une erreur suffirait à trouer un rendu offline |
| **`index` de frame croissant** dans chaque frame de sortie (§8.0) | rend une perte détectable par la sortie elle-même, et le test « sans perte en `Fixed` » exact |
| **Passe de conversion 8 bits sRGB unique**, avant la relecture (§8.0, §21) | trois consommateurs (preview, Spout/Syphon, chemin CPU) ; convertir avant de relire divise par deux les octets lus |
| **`unsafe` confiné à `output`**, modules Spout/Syphon sous `cfg` (§8.1, §10) | l'interop de textures natives l'exige (`as_hal`, `create_texture_from_hal`) ; le confiner garde le reste du projet sûr et la surface de risque lisible |
| **`CpuFrame` en alpha droit**, avancé de l'étape NDI à la sortie PNG (§8.0) | dé-prémultiplier doit se faire en linéaire, avant l'encodage sRGB : après la passe de conversion, c'est trop tard |
| **Crate `app` : bibliothèque `Engine` + binaire `winit`**, `output` ne connaît pas `winit` (§8.3) | la boucle d'événements prend le contrôle du thread principal, ce qu'une bibliothèque ne peut pas s'approprier ; `Engine` sans fenêtre reste testable et sert le rendu offline |
| **`winit` 0.30** (§11) | imposé par `egui-wgpu` pour wgpu 30 ; la 0.31 est en bêta |
| **La passe de conversion suit le format de sa cible** (`is_srgb()`) (§8.3) | surface `*Srgb` et texture `Unorm` lue comme du sRGB coexistent : décider au cas par cas évite le double encodage, qui délave l'image |
| **Letterbox pour la preview** (§8.3) | étirer ment sur la sortie, recadrer cache les bords sans le dire |
| **Le moteur a sa propre échéance**, pas la synchro de l'écran (§8.3) | wgpu répond `Occluded` sur une fenêtre masquée : réduire la preview ne doit pas arrêter NDI ni l'enregistrement |
| **Tests GPU exécutés un par un** (§24.3) | des devices wgpu ouverts en parallèle bloquent environ 3 % des exécutions, défaut antérieur à l'étape 6 et de cause non identifiée ; un réglage unique évite qu'un futur test GPU l'oublie |
| **`Mailbox`, sinon `Fifo`, jamais `Immediate`** (§8.3) | `Immediate` déchire l'image sur un projecteur ; `Fifo` existe partout |
| **Sortie plein écran : fenêtre sans bordure** (§8.4) | le mode vidéo ne change pas : pas de noir à l'ouverture, et l'interface reste utilisable sur l'autre écran ; le plein écran exclusif ferait risquer une perte à chaque changement de fenêtre, sur scène |
| **Sortie ouverte depuis un menu, jamais au lancement** (§8.4) | on choisit l'écran en direct, et un branchement imprévu n'envoie pas l'image au mauvais endroit |
| **Résolution de sortie fixe en 1080p pour la première version** (§8.4) | suivre l'écran réallouerait toutes les textures à l'ouverture, avec une saccade ; `Preview` sait déjà adapter l'image sans la déformer |
| **Fenêtre normale sur l'écran de l'interface** (§8.4) | un plein écran y cacherait le menu qui le ferme ; avec un seul écran, on serait enfermé |
| **Aucune touche ne ferme la sortie, `CloseRequested` ignoré en plein écran** (§8.4) | sur scène, une touche accidentelle couperait l'image |
| **Le plein écran rend le focus à l'interface** (§8.4) | sous Windows, `set_fullscreen` active toujours la fenêtre, qui volerait le clavier en plein jeu ; rendre le focus couvre aussi un clic accidentel, sur toutes les plateformes |
| **Écran perdu : la sortie se ferme, le moteur continue** (§8.4) | un câble débranché ne doit couper ni l'interface ni le monitor ; laisser la fenêtre glisser sur l'écran restant pourrait recouvrir l'interface |
| **`OutputRequest` distinct de `Command`** (§8.4) | une fenêtre n'est pas la grille : `Composition` n'a rien à en savoir, et `Command` ne concerne que ce qui s'applique entre deux frames du moteur |
| **Seules les sorties GPU se retirent** (§8.4) | retirer une sortie CPU suppose d'arrêter son thread : l'enregistreur de l'étape 10 dira comment terminer un fichier proprement |
| **Sorties CPU refusées en `Realtime` avant l'étape 10** (§8.0) | sans l'anneau, seule la relecture synchrone existe : l'accepter en live effondrerait le framerate en silence ; un refus à l'ajout est clair |
| ffmpeg en pipe, pas en bindings | l'encodeur matériel par plateforme n'est plus notre problème |
| Vidéo en dernier | plusieurs jours de travail, indépendants du reste |
| **Grille de clips (Deck/LayerGroup/Layer/Column) comme structure d'organisation** (§3bis) | répond à « quoi joue, quand » ; un patch modulaire unique ne permet pas de changer de répertoire de visuels à chaud en live |
| **Un `Graph` par `Clip`, pas un graphe global** (§3) | un clip reste déplaçable, dupliquable, sérialisable seul ; la modulation inter-clips est une extension explicite, pas le cas par défaut |
| **`Column` transversal**, pas une propriété de `Layer` (§3bis.1) | un déclenchement par layer ne peut pas exprimer une scène synchronisée sur tous les layers sans dupliquer la logique |
| **Une case par colonne dans `Layer`**, plus de `Column.slots` (§3bis.1) | une colonne qui visait les pistes par leur rang se décalait dès qu'on ajoutait une piste ; une liste de clips ne représentait pas une case vide |
| **La vitesse d'un clip multiplie son `dt`** (§3bis.2) | un seul réglage accélère tout le clip sans connaître son contenu, puisque tout s'intègre sur `dt` |
| **Clips prédéfinis en code, dans une crate `library`** (§10, §23.6) | le format de fichier n'arrive qu'à l'étape 7e ; le contenu reste séparé de la mécanique de la grille et se teste sans GPU |
| **L'interface demande un clip prédéfini par identifiant, `app` le construit** (§23.6) | `Command` reste `Copy` et comparable, donc testable ; `Clip` porte des `Box<dyn ...>` et ne le peut pas |
| **`TempoController` au-dessus de `Clock`**, pas une redéfinition (§3bis.3) | `Clock` répond à l'heure pour le déterminisme ; le tempo répond à la durée d'un beat, partagée et modifiable en live (tap, resync) |
| **`Transport.position` est un `Signal` accumulateur**, pas un `Param` (§3bis.3) | une valeur qui avance seule à chaque frame est le rôle d'un `Signal` (§16.1), pas d'un `Param` — l'écrire via `Command::SetParam` à 60 fps détournerait un mécanisme pensé pour l'édition ponctuelle |
| **Une arena par niveau de la grille** (`DeckId`/`LayerGroupId`/`LayerId`/`ColumnId`/`ClipId`), pas une arena unique (§3bis.1) | un identifiant typé par niveau rend une confusion détectable à la compilation, pas à l'exécution ; même idiome que `core::Graph` |
| **`ClipTransition` réutilise `PositionSignal`** pour sa progression, `Fade`/`Wipe` non rendus avant l'étape 5 (§3bis.2) | une transition qui avance seule a le même besoin que `Transport.position` ; leur rendu dépend d'un compositeur de couches que `composition` ne doit pas connaître (§10) |
| **`serde` dès le premier commit de `composition`**, pas ajouté après (§3bis.5) | un schéma figé par du code qui en dépend coûte une migration immédiate (§19.2) ; le coût de l'ajouter dès le début est quasi nul |
| **`composition` dépend de `shape`**, pas seulement de `core` (§10) | `Clip.effects` référence une chaîne `ShapeOp` dont le type vit dans `shape` ; `shape` elle-même ne connaît ni GPU ni carte son, donc cette dépendance ne casse pas la testabilité sans GPU qui motive la règle de dépendance, elle déplace seulement la position de `composition` dans la chaîne |
| **`Transport`/`Clip` ne dérivent pas `serde` directement** (§3bis.5) | ils portent un `Param`/`NodeId` de `core`, qui ne dérive pas `serde` lui-même — leur forme sérialisée passe par le schéma JSON nommé du format de patch (§19.1), pas par un `derive` sur la structure en mémoire |
| **`CrossFader` global en plus de `BlendParams` par couche** (§3bis.4) | le geste de régie mélange deux groupes entiers ; `BlendParams` mélange une couche avec celle du dessous — les deux répondent à des échelles différentes |
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
| **Une forme est un `Path` : buffer plat + plages de contours fermés/ouverts**, pas une polyligne unique (§16.3) | une polyligne unique ne sait dire ni où un contour finit ni s'il est fermé — `RadialRepeat` et `Smooth` produisaient des défauts invisibles jusqu'au premier rendu ; le buffer plat évite l'allocation par contour d'un `Vec<Vec<Vec2>>` |
| **Contrat des couches en trois appels** (`prepare` / `encode` / `output`), pas `render -> &TextureView` (§22) | rendre une texture depuis un emprunt mutable interdisait au compositeur de tenir toutes les textures à la fois ; séparer le CPU du GPU permet une préparation parallèle future sans changer le contrat de chaque couche |
| **Trait `RenderLayer`**, pas `Layer` (§6) | `composition::Layer` est une piste de la grille ; deux types homonymes dans `render` et `ui` rendraient chaque usage et chaque erreur ambigus |
| **Tessellation dans `layer`, en module CPU pur** ; compositeur et readback dans `render` (§10) | les cas limites de l'étape 5 se logent dans la tessellation : elle reste testable en CI sans GPU |
| **Compositeur par passes shader en ping-pong**, pas `BlendState` natif (§22) | `Difference` exige de lire la destination, impossible en mélange matériel ; le calcul prémultiplié s'écrit en clair, une fois |
| **MSAA 4× avec repli 1× diagnostiqué** (§22) | le support varie selon le backend ; une capacité absente dégrade, ne panique pas (§18.1) |
| **Couleur = quatre `Param` saisis en sRGB**, convertis en linéaire prémultiplié pendant `prepare` (§21) | une couleur est un champ qu'on module ; un `[f32; 4]` nu serait un câble impossible à brancher |
| **`wgpu` épinglé à une majeure, `rust-version` alignée sur son minimum** (§11) | une majeure tous les trois mois, avec changements d'API ; une montée de version doit être délibérée |
| **Tests GPU qui se sautent sans adaptateur**, tout le reste testé sans GPU (§24.3) | un test qui échoue faute de GPU finit désactivé ; ce qui est pur ne doit jamais attendre un GPU |
| **`LayerInput` porte deux graphes** : `graph` pour le contenu, `blend_graph` pour le mélange (§22.1) | chaque clip porte son propre graphe (§3), mais l'opacité d'une piste appartient à la régie : elle se résout dans le graphe de composition |
| **Un `ShapeRenderer` par piste**, le clip actif lui prête son contenu (§3bis.2, §22.1) | une `ShapeLayer` par clip coûterait 80 Mo par clip en 1080p, 8 Go pour 100 clips, dont un seul par piste s'affiche |
| **`ColorParam`, `ShapeStyle`, `BlendParams` dans `shape`** (§10) | clips et pistes les portent ; `composition` ne peut pas dépendre de `layer` sans tirer wgpu |
| **Seul le clip actif est évalué, et il repart de zéro au déclenchement** (§3bis.2) | le coût d'une frame suit ce qui est affiché, pas la taille de la grille ; un pad frappé redémarre |
| **`PlayMode` agit par le signe du `dt` du clip** (§3bis.3) | tout ce qui s'intègre remonte seul, sans code par type ; une forme procédurale reçoit une fin de sa durée, sans attendre la vidéo |
| **Une transition mélange deux clips avant le compositeur** (§3bis.2) | deux couches d'opacités croisées creusent l'image à mi-fondu ; une texture mélangée rejoint le compositeur comme un seul clip, avec le mode de la piste |
| **La durée d'une transition est en battements, hors de la vitesse du clip** (§3bis.2) | elle suit le tempo, et un clip à vitesse nulle ne doit pas la bloquer |
| **Annuler par images de la grille, pas par commandes inverses** (§23.11) | une image coûte 1 ms et couvre d'office toute nouvelle commande ; un inverse faux corromprait la grille sans le dire |
| **Une grille restaurée reprend ce qui joue** (§23.11) | sans cela, annuler ferait repartir les LFO et les compteurs de trig, et l'image sauterait en plein jeu |
| **Un pas d'annulation est un geste** (§23.11) | un potentiomètre émet une commande par frame : il faudrait des dizaines de Ctrl+Z pour un seul geste |
| **Éditeur de nœuds maison** (§23.10) | `egui-snarl` demande egui 0.36, et un patch à maintenir à chaque version coûterait plus qu'un éditeur à nos conventions, testé par clics simulés |
| **L'éditeur a sa fenêtre et son contexte egui** (§23.10) | le câblage se fait à côté de l'image, et ne coûte rien fermé |
| **`Graph::rewire` plutôt que des `Param` exposés par le graphe** (§23.10) | un `Signal` reste opaque ; passer par sa `Spec` réutilise la table du format de fichier, et un seul endroit recalcule dépendances et cycles |
| **Une cible unique, `ParamTarget`, pour toute destination** (§23.10) | brancher un `Param` de clip, de nœud ou de régie est le même geste : une commande de chaque par type aurait multiplié les cas |
| **Les nœuds d'un trig se lisent mais ne se règlent pas** (§23.10) | `Trig` garde les versions de référence (§3bis.6) : les régler ailleurs les désynchroniserait |
| **La place des nœuds n'est pas dans le fichier** (§23.10) | un patch décrit ce qui joue ; une mise en page changerait le fichier à chaque glissé |
| **Un feedback est un clip, pas un effet de piste** (§6.1) | il hérite de la bibliothèque, des potentiomètres, des sources audio et de la sauvegarde ; un effet de piste aurait demandé un second chemin pour chacun |
| **Le feedback relit l'image finale, copiée en fin de frame** (§6.1) | il donne des échos de toute la composition, avec un délai d'une frame exactement, sans lire et écrire la même texture |
| **Réglages du feedback par seconde** (§6.1) | un réglage par frame changerait d'image avec la cadence, et casserait le rendu offline à une autre cadence |
| **Les features ont leur propre analyse, fixe** (§7.1) | régler le spectre pour poser une zone ne doit déplacer ni le tempo ni une modulation |
| **`onset` est une impulsion qui retombe, pas un booléen** (§7.1) | l'analyse tourne plus vite que les frames : un drapeau serait perdu entre deux lectures du triple buffer |
| **BPM par autocorrélation et peigne, avec une préférence autour de 120** (§7.1) | sans dépendance, et testé sur des boucles de synthèse ; un seul retard ne donne que 3 BPM de précision ; un tempo double ou moitié reste ambigu sans préférence |
| **Suivre le tempo est un choix, désactivé par défaut** (§7.1) | une détection fausse ne doit pas déformer un set ; taper ou fixer le tempo reprend la main |
| **Une source audio se choisit sur chaque potentiomètre, sans éditeur de nœuds** (§7.1) | sans cela, la première version n'offrirait la modulation audio qu'en code |
| **Le tempo est dans `Ctx`, d'une source unique** (§3bis.3, §16.2) | tap et BPM audio écrivent au même endroit, tous les clips lisent au même endroit ; câbler chaque clip serait à refaire pour tout nouveau clip |
| **Un graphe de composition, en plus du graphe de chaque clip** (§3) | l'opacité d'une piste et le tempo survivent aux changements de clip ; les mettre dans un clip les ferait sauter |
| **`ui` ne connaît ni wgpu ni `winit`** : elle lit la grille et rend des `Command` (§10, §23.5) | testable par clics simulés, sans GPU ni écran ; `egui-wgpu` et `egui-winit` restent dans `app` |
| **Panneaux egui, pas `egui_dock`** (§23.5) | `egui_dock` s'arrête à egui 0.36 ; garder egui à jour vaut mieux que des panneaux déplaçables pour la première version. Report du §23.2, pas abandon |
| **Monitor par un `GpuSink` en `Rgba8Unorm`** (§8.3) | imposé par egui ; la texture du compositeur est linéaire en 16 bits et change à chaque frame |
| **`PresentSurface` commun à la preview et à l'interface** (§8.3) | le tableau des réponses de la surface n'existe qu'une fois |
| **`params(&self)` + `param_mut(i)`**, pas une seule méthode en `&mut` (§16.3 ter) | l'interface lit à chaque frame : une méthode en `&mut` lui ferait emprunter toute la grille en écriture ; une macro écrit les deux depuis une seule liste |
| **Un réglage passe par `Command::SetParam`**, qui écrit la base d'un `Param` (§16.3 ter, §23.5) | §23 : l'interface ne mute pas pendant l'évaluation ; garder la modulation sous un curseur évite de la perdre sans le voir, et l'annulation restera possible |
| **Cellule de grille : nom et état, pas de vignette** (§23.5) | une vignette vivante évaluerait tous les clips, contre la décision « seul le clip actif vit » |
| **`Command` dès l'étape 7, avant l'interface** (§23) | déclencher depuis un clavier, une interface ou le MIDI passe par le même chemin, testé sans GPU ; des appels directs seraient à réécrire |
| **Remplissage en règle non nulle**, contours fermés seulement (§22.1) | les copies d'une répétition radiale qui se chevauchent restent pleines au lieu de se trouer ; un contour ouvert n'a pas d'intérieur |
| **Formules de mélange en double : WGSL + référence CPU testée** (§22.1) | un shader ne se teste pas seul ; la version Rust sert d'oracle aux tests de pixels, mode par mode |
| **`ClipSource::Shape { graph, generator }`**, pas `Shape(Graph)` (§3bis.2) | un graphe seul ne produit aucune forme ; les effets du clip n'avaient rien à transformer |
| **Contour non refermé par duplication** (§16.3) | un point répété dupliquerait un sommet à la tessellation et produirait des doublons sous répétition radiale |
| **Saturation de la géométrie chez chaque primitive** (§18.4) | une modulation dégénérée doit dégrader la forme, jamais allouer des millions de points ni figer l'application |
| **Écriture dans un buffer fourni** pour les formes (§16.3) | évite une allocation par forme et par frame à 60 fps |
| **Graphe évalué entièrement avant les formes** (§20.1) | `Param::get()` sans coût ni effet de bord ; deux formes lisant un LFO voient la même valeur |
| **Pas d'erreur en évaluation**, seulement en édition (§18.1) | un logiciel de scène ne panique pas ; les compteurs de diagnostic rendent les incohérences visibles sans interrompre |
| **Vérification du non-fini au cache**, un seul point (§18.3) | un `NaN` fait disparaître une forme sans message — le pire défaut : muet |
| **Saturation chez le consommateur**, jamais dans le `Signal` (§18.4) | saturer à la source détruirait la modulation pour tous les autres consommateurs |
| **Un `Signal` se décrit par une `Spec` de `core`, méthode obligatoire** (§19.1) | `core` reste sans dépendance, contrairement à `typetag` ; sans implémentation par défaut, un `Signal` ne peut pas rester insauvegardable sans qu'on le voie |
| **Le fichier décrit le clip, pas son entrée de catalogue** (§19.1, §23.6) | un clip réglé n'est plus celui du catalogue ; modifier le catalogue ne doit pas changer d'anciens fichiers, et l'éditeur de nœuds produira des clips sans entrée |
| **Le graphe s'écrit en ordre topologique, avec des positions** (§19.1) | index et génération racontent l'histoire de l'arena, pas le patch ; refuser une référence en avant garantit qu'un fichier ne crée aucun cycle |
| **Les clips actifs se sauvegardent, pas les phases** (§19.1) | la composition revient et joue, sans figer le format sur l'état interne de chaque `Signal` |
| **Un fichier se charge en entier ou pas du tout** (§19.1) | une `Composition` neuve se construit à côté : un fichier refusé ne touche pas à la grille qui joue |
| **Écriture par fichier temporaire puis renommage** (§19.1) | un plantage pendant l'écriture laisse l'ancienne version intacte |
| **Dialogues et disque hors du thread du moteur** (§23.8) | une boîte bloquante figerait l'image sur la sortie tant qu'elle reste ouverte |
| **`FileFlow` sans `rfd` ni `winit`** (§23.8) | enregistrer, renoncer et quitter forment un parcours à risque de perte : il se teste sans fenêtre |
| **Déclencher n'est pas modifier** (`Command::is_edit`, §23.8) | sinon toute session de jeu demanderait d'enregistrer ; le `match` exhaustif oblige à trancher pour chaque nouvelle commande |
| **Rouvrir le dernier fichier, avec repli sur la grille de départ** (§23.8) | on retrouve son travail sans geste, et un fichier disparu ne doit pas empêcher un lancement sur scène |
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
| **Tessellation** | sans GPU : un contour fermé produit des triangles qui couvrent son aire, un contour ouvert n'est jamais rempli, plusieurs contours restent disjoints (pas de triangle à cheval), un contour dégénéré (moins de 3 points, points confondus, non-fini) rend zéro triangle sans paniquer |
| **Couches** | par relecture de pixels, sur un GPU s'il existe : un carré plein au centre donne au pixel central la couleur **linéaire prémultipliée** attendue et laisse le coin transparent ; conversion sRGB → linéaire vérifiée sans GPU sur des valeurs de référence |
| **Sorties** | sans GPU ni fenêtre, avec des `CpuSink` factices : en `Fixed`, une sortie lente reçoit les indices `0, 1, 2, …` sans trou ; en `Realtime`, elle perd des frames, les compte dans `Degraded { dropped }`, et la boucle ne ralentit pas ; une relecture n'a lieu que si une sortie CPU est active ; une sortie `Failed` n'est plus alimentée mais reste listée. Sur GPU s'il existe : la passe de conversion 8 bits suit sa référence CPU (`linear_premul_to_srgb8`), à 1/255 près, sur cible `Unorm` comme `*Srgb` ; deux sorties CPU partagent le même `Arc` (une seule relecture) ; un `GpuSink` factice voit `after_submit` appelé une fois par frame, après `encode` |
| **Interface** | sans GPU ni écran, par `egui_kittest` : un clic sur une cellule, un en-tête de colonne, le bouton tap ou un curseur produit la bonne `Command`. Sans GPU : `set_param` écrit le bon champ et garde la modulation ; chaque type de forme expose ses `Param` et `param_mut` atteint le même que `params`. Sur GPU : `MonitorSink` suit la référence CPU |
| **Pilotage de la grille** | sans GPU : un déclenchement remet l'état du clip à zéro, un refus n'y touche pas ; l'ordre des pistes suit la hiérarchie ; `BeatRate` suit le tempo ; le tap tempo oublie après 2 s ; une `Command` invalide est refusée sans panique. Sur GPU : un clip inactif n'est pas évalué, changer de clip change l'image, une piste vide ne contribue pas, l'opacité d'une piste se résout dans le graphe de composition |
| **Grille de composition** | construction/suppression dans la hiérarchie sans invalider les identifiants des niveaux non touchés (§3bis.1) ; une `Column` déclenche bien la même tranche sur tous les `Layer` (§3bis.1) ; `TempoController::tap` calcule un BPM correct sur une séquence d'événements synthétiques ; `CrossFader.phase` à 0/0.5/1 produit les `BlendParams::opacity` attendus sur les deux groupes (§3bis.4) ; `PositionSignal` avance déterministiquement à un `t`/`dt` injecté, jamais l'horloge système (§3bis.3) |

**Ce qui rend cela possible :** la règle de dépendance du §10. `core`, `osc` et `shape`
ne connaissent ni GPU, ni carte son, ni fenêtre — ils sont donc testables en CI,
rapidement, sans matériel. **Cette règle n'est pas une élégance, c'est la condition de
la testabilité du projet.**

**Le déterminisme est un outil de test.** Le temps étant toujours passé en paramètre
(§4), un test fixe `t` et vérifie une valeur exacte. C'est la seconde raison d'être de
cette règle, après le rendu offline.

**Pour ce qui touche au GPU et au matériel :** injecter une abstraction (un `GpuSink` ou
`CpuSink` factice, une source audio synthétique) plutôt que renoncer au test. Les tests
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

    /// Sa description, pour le format de patch (§19.1).
    fn spec(&self) -> Spec;
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
    pub bpm: f32,                     // tempo de la composition (§3bis.3)
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

`audio` porte le spectre de la frame (`Spectrum`, §7) : c'est par là qu'une zone de trig
le lit (§3bis.6), sans savoir d'où il vient.

`dt` est nécessaire aux modules à état — une enveloppe doit savoir combien de temps
s'est écoulé, et ne peut pas le déduire de `t` seul si elle veut rester correcte quand
une frame est longue.

### 16.3 Le trait `ShapeGenerator`

```rust
pub struct Contour {
    pub start: u32,    // index du premier point dans `Path::points`
    pub end: u32,      // exclusif
    pub closed: bool,
}

pub struct Path {
    pub points: Vec<Vec2>,
    pub contours: Vec<Contour>,
}

pub trait ShapeGenerator {
    fn generate(&self, t: f64, ctx: &Ctx, g: &Graph, out: &mut Path);
}
```

**Écriture dans un buffer fourni, pas de `Vec` retourné.** Le rendu tourne à 60 fps ;
allouer un `Vec` par forme et par frame produit une pression mémoire inutile. Le
compositeur possède les buffers et les réutilise (`out.clear()` puis remplissage).

**Une forme est un ensemble de contours, pas une polyligne unique.** Correction du
contrat initial (`out: &mut Vec<Vec2>`), décidée avant l'étape 5 parce qu'une polyligne
unique ne sait dire ni où un contour finit, ni s'il est fermé. Deux défauts latents en
découlaient dans `shape`, invisibles tant que rien n'était dessiné — les tests
comptaient des points, ils ne traçaient rien :

- `RadialRepeat` ajoutait ses copies **à la suite dans le même buffer**. Le
  rasteriseur aurait tracé un segment parasite de la fin de chaque copie au début de
  la suivante.
- `Smooth` bouclait par modulo sur tout le buffer. Sur un `Arc` ouvert, il moyennait
  le premier point avec le dernier et rapprochait les deux extrémités ; après une
  répétition radiale, il lissait d'une copie à l'autre.

**Pourquoi un buffer plat découpé en plages, plutôt qu'un `Vec` par contour.** Un
`Vec<Vec<Vec2>>` serait plus lisible, mais coûterait une allocation par contour et par
frame — exactement ce que le paragraphe précédent interdit. Avec le buffer plat, les
opérateurs ponctuels (`Deform`, `NoiseDisplace`, `Transform`) restent une boucle sur
tous les points, sans connaître les contours ; seuls les opérateurs qui regardent les
voisins (`Smooth`) itèrent contour par contour. Un séparateur dans le buffer (un point
`NaN` entre deux contours) aurait évité de changer la signature, mais contredit le
§18.3 : aucun non-fini ne circule.

**Invariant, vérifié par les tests :** les plages de `contours` se suivent, ne se
chevauchent pas et couvrent exactement `points` ; aucune plage n'est vide. Le
générateur pose le drapeau `closed` (`Arc` ouvert, les autres primitives fermées), les
opérateurs le conservent, et `RadialRepeat` ajoute un contour par copie.

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
    fn apply(&self, t: f64, ctx: &Ctx, g: &Graph, path: &mut Path);
}
```

Un opérateur reçoit le `Path` entier (§16.3). Il **doit préserver l'invariant des
plages** : un opérateur ponctuel ne touche qu'à `points` ; un opérateur qui ajoute ou
retire des points met `contours` à jour dans le même appel.

**Pourquoi un trait distinct plutôt qu'un opérateur enveloppant un générateur.**
L'enveloppement (`Deform::new(Circle)`) donnerait une composition naturelle, mais chaque
maillon de la chaîne aurait besoin de son **propre buffer** pour recevoir la sortie du
précédent — exactement l'allocation par frame que le §16.3 cherche à éviter.

Un opérateur travaille donc **en place**. Une chaîne de dix opérateurs n'alloue rien.
`Chain` assemble un générateur et une suite d'opérateurs, et implémente lui-même
`ShapeGenerator` — la composition reste donc transparente pour l'appelant.

### 16.3 ter Décrire ses paramètres

```rust
pub struct ParamDesc {
    pub name: &'static str,
    pub min: f32,                // plage du curseur, pas une saturation
    pub max: f32,
    pub value: Param,            // copie : `Param` est `Copy`
}

// Sur `ShapeGenerator` et sur `ShapeOp` :
fn kind(&self) -> &'static str;                            // « Ellipse », « Deform »
fn params(&self) -> Vec<ParamDesc>;
fn param_mut(&mut self, index: usize) -> Option<&mut Param>;
fn spec(&self) -> Spec { /* déduite de kind et params */ }  // §19.1
```

Un clip tient ses formes dans des `Box<dyn ...>` : sans ceci, ses `Param` sont
inatteignables de l'extérieur. L'interface en a besoin, et le format de patch (§19) comme
le pilotage MIDI/OSC en auront besoin pour les mêmes raisons.

**Deux méthodes, pas une.** L'interface lit à chaque frame et n'écrit que rarement, entre
deux frames. Une seule méthode en `&mut` lui ferait emprunter toute la grille en
écriture pour afficher un curseur. La macro `params!` écrit les deux depuis une seule
liste, ce qui garantit que `param_mut(i)` désigne le `i`-ème de `params()`.

**La plage n'est pas une saturation.** `min` et `max` bornent le geste de l'utilisateur.
La saturation reste chez le consommateur (§18.4) : une modulation peut sortir de la
plage. L'interface règle donc ses curseurs en `SliderClamping::Never` : le défaut d'egui
réécrit toute valeur hors plage et signale un changement à chaque frame, si bien
qu'afficher un clip l'aurait modifié.

**`Param::set_base` règle un paramètre sans le débrancher.** Sur un `Fixed`, il remplace
la valeur ; sur un `Modulated`, il remplace `base` et garde `depth` et `source`. Un
curseur ne doit pas couper une modulation sans le dire.

**`Param::set_depth` règle l'amplitude d'une modulation (étape 7c).** Il rend `false` sur
un `Fixed` : il n'y a pas d'amplitude sans câble. C'est le seul accès au mouvement d'un
clip prédéfini tant que son graphe interne n'est pas exposé.

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

> Tranché avant l'étape 7e (choix A1, B2, C1, D2, E1). Raisons au §13. Geste et fichier au
> §23.8.

```jsonc
{
  "version": 2,
  "meta": { "app": "0.1.0" },
  "tempo": { "fixed": 120.0 },
  "tempo_follow": false,
  "analysis": { "method": "peak", "freq_scale": "log", "amp_scale": "db",
                "window": { "fixed": 0.0427 } },
  "columns": 8,
  "active_deck": 0,
  "graph": [ /* le graphe de composition, trigs compris */ ],
  "trigs": [
    { "name": "Trig 1", "target": { "column": 2 }, "gate": 0, "pulse": 1, "flash": 2 }
  ],
  "decks": [
    { "name": "Set principal", "groups": [ { "layers": [
      { "name": "Formes",
        "blend": { "opacity": { "fixed": 1.0 }, "mode": "normal" },
        "active": 0,                 // indice de colonne, null si rien ne joue
        "cells": [
          { "name": "Blob",
            "speed": { "fixed": 1.0 },
            "graph": [
              { "kind": "BeatRate", "deps": [], "fields": { "beats": { "fixed": 8.0 } } },
              { "kind": "Lfo", "deps": [0], "fields": {
                  "waveform": "sine", "unipolar": false,
                  "freq": { "modulated": { "base": 0.0, "depth": 1.0, "source": 0 } } } }
            ],
            "generator": { "kind": "Ellipse", "fields": { "rayon": { "fixed": 0.5 } } },
            "effects": [ { "kind": "Deform", "fields": { } } ],
            "style": { "fill": { }, "stroke": null },
            "transport": { "bpm_sync": { "position": 2, "duration_beats": { "fixed": 8.0 },
                                         "play_mode": "loop" } },
            "transition": "cut" },
          null                       // une case vide
        ] }
    ] } ] }
  ],
  "state": null
}
```

**Chaque clip porte son propre tableau `graph`** (§3), pas un graphe global partagé. Un
clip emporte son graphe avec lui, sans résoudre de dépendances externes. Le fichier décrit
le clip lui-même, jamais son entrée de catalogue (§23.6).

**Un nœud s'écrit par son `kind` et ses champs.** Le tableau suit l'ordre topologique, et
`source`, `deps` et `position` y désignent une **position** : le fichier ne connaît ni
index d'arena ni génération. Au chargement, chaque nœud s'ajoute après ses dépendances, et
`Graph::add_with_deps` refuse toute référence en avant. Un `Param` dont la source a été
retirée vaut sa base à la lecture : il s'écrit donc `fixed`.

**`core` reste sans dépendance.** Un `Signal` se décrit par une `Spec`, une valeur neutre
de `core` qui ne sait rien de JSON ; `patch` la traduit.

```rust
pub enum SpecValue {
    Param(Param), Params(Vec<Param>), Number(f32), Int(u32), Bool(bool),
    Word(String), Specs(Vec<Spec>),
}
pub struct Spec { pub kind: String, pub fields: Vec<(String, SpecValue)> }

pub trait Signal {
    fn spec(&self) -> Spec;   // obligatoire : un Signal oublié ne compile pas
}
```

Sans implémentation par défaut, un nouveau `Signal` ne peut pas rester insauvegardable sans
qu'on le voie. Les formes et les effets ont déjà `kind` et `params` (§16.3 ter) : leur
`spec` se déduit par défaut, et seuls `NoiseDisplace` (sa graine) et `Chain` la
surchargent. `shape::generator_from_spec` et `op_from_spec` reconstruisent une forme en
réglant ses `Param` par nom ; `patch` tient la table équivalente des `Signal`.

**`kind` et les noms de paramètres deviennent un contrat.** Les renommer casse les fichiers
existants, donc passe par une migration. La fixture du §19.2 le vérifie.

**JSON, pas un format binaire.** Un patch se relit, se diffe dans git, se corrige à la
main quand le logiciel refuse de l'ouvrir. La taille et la vitesse de chargement sont
sans enjeu ici.

**Ce qui est sauvegardé.** La grille, chaque clip, l'opacité et le mode des pistes, le
tempo et son suivi, le réglage de l'analyse audio (§7), les trigs, et le clip actif de chaque piste : la composition revient et joue tout de
suite, avec des phases reparties de zéro. `"state": null` demeure : un patch décrit une
*configuration*, pas un instant. Ni la disposition des panneaux ni l'écran de sortie ne
sont sauvegardés : les écrans changent d'une salle à l'autre (§8.4).

**Un fichier se charge en entier ou pas du tout.** `patch` construit une `Composition`
neuve. Toute erreur (JSON, version, `kind` inconnu, champ manquant, référence invalide)
rend un `Result` avec un message, et la grille en cours ne change pas (§18.1). Un fichier
trop volumineux pour une grille plausible est refusé avant d'allouer.

**Un fichier ne s'abîme pas en écrivant.** L'écriture passe par un fichier temporaire
synchronisé sur le disque, puis un renommage : un plantage laisse l'ancienne version
intacte. Une valeur non finie est refusée à l'enregistrement : JSON ne l'exprime pas, et le
fichier ne se rouvrirait plus.

### 19.2 Migrations

```rust
fn migrate(mut data: Value) -> Result<Value, PatchError> {
    let mut version = data["version"].as_u64().ok_or(PatchError::NoVersion)?;
    if version == 0 || version > CURRENT_VERSION {
        return Err(PatchError::UnknownVersion(version));
    }
    while version < CURRENT_VERSION {
        data = match version {
            // 1 => migrate_1_to_2(data)?,
            _ => return Err(PatchError::UnknownVersion(version)),
        };
        version += 1;
    }
    Ok(data)
}
```

Le format réel commence à la version 1. La version 2 ajoute `tempo_follow` (§7.1) : un
fichier v1 se migre en le mettant à `false`, ce qu'il faisait déjà sans le dire. La version
3 ajoute les clips de feedback (§6.1) : `generator` et `style` deviennent facultatifs, et un
fichier v2 n'a rien à changer.

Migrations **en chaîne**, une fonction par saut de version. Un patch v1 traverse
`1→2→3` — on n'écrit jamais de migration directe `1→3`, qui doublerait le travail à
chaque nouvelle version.

**Règle absolue :** tout changement du format incrémente `CURRENT_VERSION` **et**
ajoute sa fonction de migration **et** un test qui charge un patch de l'ancienne
version depuis `crates/patch/tests/fixtures/`. Les fixtures ne sont **jamais** régénérées — leur
valeur est précisément d'être anciennes.

Une version **supérieure** à `CURRENT_VERSION` est refusée avec un message clair : un
patch venu d'une version plus récente ne peut pas être deviné.

---

## 20. Cycle de frame et threads

### 20.1 L'ordre, strict

```
 0. Commandes      → appliquées entre deux frames (§23)
 1. Clock          → t, dt                     (Realtime ou Fixed)
 2. Audio          → lecture du triple buffer  (non bloquante)
 3. Ctx            → construction              (une fois)
 4. Graphes        → composition d'abord (tempo, vitesse des clips), puis clips actifs seulement
 5. Formes         → generate() dans les buffers réutilisés
 6. Tessellation   → polylignes → triangles (lyon)
 7. Couches        → chacune rend vers sa texture
 8. Composition    → mélange vers la texture finale
 9. Sorties        → a. GpuSink::encode, passe 8 bits + copie de relecture (si une sortie CPU)
                     b. queue.submit
                     c. GpuSink::after_submit (present, signal Spout, publication Syphon)
                     d. CpuFrame déposée dans la file de chaque CpuSink (§8.0)
10. Diagnostics    → publiés vers l'UI, puis remis à zéro
```

**L'étape 9 est en deux temps autour de la soumission** (§8.0) : ce qui doit
s'enregistrer dans l'encodeur avant `submit`, ce qui ne peut venir qu'après. `Outputs`
a son propre encodeur : le compositeur a déjà soumis le sien à l'étape 8, et
`GpuFrame` lui lit une texture finie. La
`CpuFrame` déposée en (d) est, avec l'anneau du §8.2, celle d'une frame antérieure ;
elle porte son propre `t`.

**Un trig agit à la frame suivante.** Ses commandes se lisent après le graphe de
composition (4) et rejoignent la file de l'étape 0 (§3bis.6).

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
┌─ un thread par CpuSink ─┐   NDI, pipe ffmpeg, Art-Net — file bornée tenue par Outputs
└─────────────────────────┘
```

**Règles de thread, chacune correspondant à un mode de défaillance réel :**

- Le callback audio **ne fait que copier** dans le ring buffer. Toute autre opération
  produit des clics audibles.
- Le thread principal **ne bloque jamais** sur l'audio. `triple_buffer` rend la
  dernière valeur disponible ; si l'analyse est en retard, on relit la précédente.
- Les sorties CPU ont **2 à 3 frames de retard** (§8.2) et ne doivent jamais faire
  attendre le rendu. Si l'une décroche en mode `Realtime`, on perd une frame — c'est le
  comportement voulu, compté dans son `status()`. En mode `Fixed`, le rendu attend :
  aucune frame n'est perdue. Cette politique appartient à l'étage `Outputs`, pas aux
  sorties (§8.0).
- Les sorties GPU (preview, Spout, Syphon) restent sur le thread principal, où vit la
  surface de la fenêtre.

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
linéaire ; seules les cibles finales sont 8 bits encodées sRGB** — la surface de
présentation, la texture partagée Spout/Syphon, la texture relue pour les sorties CPU.
Toutes trois sont écrites par **la même passe de conversion** (§8.0) : l'encodage sRGB
existe à un seul endroit, qu'il soit fait par le matériel (format `*Srgb`) ou dans le
shader (cible `Unorm` que le récepteur lit comme du sRGB, cas fréquent de Spout).

**Alpha prémultiplié** dans toute la chaîne de composition — sinon les bords des formes
tessellées présentent un liseré sombre au mélange.

**La couleur d'une forme est quatre `Param`, saisis en sRGB.** Remplissage et trait
d'une `ShapeLayer` portent chacun `r`, `g`, `b`, `a`, tous `Param` — jamais un
`[f32; 4]` en dur : une couleur est exactement le genre de champ qu'on voudra moduler
par un LFO ou une feature audio, et un tableau nu serait le « câble impossible à
brancher demain » du §16.4. Les valeurs sont **saisies et sérialisées en sRGB** (la
ligne « palettes, UI, patchs » du tableau ci-dessus : ce que l'humain lit et saisit),
puis converties en linéaire et prémultipliées **une fois, pendant `prepare`** (§22),
avant d'atteindre le GPU. Chaque canal est saturé dans `[0, 1]` chez la couche, son
consommateur — jamais dans le `Signal` qui le module (§18.4). La conversion
sRGB → linéaire est une fonction pure, testée sans GPU sur des valeurs de référence.
Le type vit dans `shape`, avec `ShapeStyle`, parce qu'un clip le porte (§10).

---

## 22. Contrat des couches

> Implémenté à l'étape 5 (`crates/layer`, `crates/render`) pour `ShapeLayer` et le
> compositeur ; ce qui a été fixé à l'implémentation est au §22.1. `FeedbackLayer`,
> `VideoLayer` et le pool de textures scratch restent des intentions.

```rust
pub trait RenderLayer {
    /// Travail CPU : génération des formes, tessellation, upload des buffers.
    fn prepare(&mut self, t: f64, ctx: &Ctx, g: &Graph, gpu: &Gpu);
    /// Enregistre la passe de rendu de la couche dans l'encodeur partagé.
    fn encode(&self, encoder: &mut CommandEncoder);
    /// Prête la texture de la couche au compositeur, en lecture.
    fn output(&self) -> &TextureView;
    fn blend(&self) -> BlendParams;
    fn resize(&mut self, size: UVec2, gpu: &Gpu);
}

pub struct BlendParams {
    pub opacity: Param,          // modulable
    pub mode: BlendMode,         // Normal, Add, Multiply, Screen, Difference...
}
```

**Pourquoi trois appels, et non `render(&mut self, …) -> &TextureView`.** Correction du
contrat initial, décidée avant l'étape 5. Le contrat d'origine rendait une référence
issue d'un emprunt **mutable** : la texture de la couche 1 restait empruntée tant
qu'on l'utilisait, ce qui interdisait d'appeler `render` sur la couche 2 — alors que le
compositeur a besoin des textures de **toutes** les couches en même temps. Il soumettait
aussi implicitement un travail GPU par couche, quand les couches doivent enregistrer
leurs passes dans un **seul** encodeur, soumis une fois par frame.

Le cycle du compositeur devient donc : `prepare` sur chaque couche (emprunt mutable,
l'une après l'autre), `encode` sur chaque couche, puis `output` sur toutes (emprunts
en lecture, qui coexistent).

**Pourquoi séparer `prepare` d'`encode`, quand deux appels auraient suffi à régler
l'emprunt.** `render(&mut self, …, encoder)` suivi d'`output(&self)` réglait déjà le
problème. La séparation est retenue pour isoler le travail CPU (générer, tessellater,
remplir les buffers) de l'enregistrement GPU : le jour où la préparation devient un
goulot, les couches pourront être préparées en parallèle sur plusieurs cœurs sans
changer le contrat que `FeedbackLayer`, `VideoLayer` et `Scene3dLayer` auront entretemps
implémenté. Le coût — un appel de plus par implémentation — est payé une fois ; une
modification du contrat de toutes les couches existantes, elle, se paierait à chaque
couche.

**`encode` prend `&self`.** Tout ce qui change d'une frame à l'autre (buffers de
sommets, uniformes) est écrit pendant `prepare`. `encode` ne fait que référencer des
ressources déjà prêtes : c'est ce qui garantit qu'aucun travail CPU coûteux ne se
glisse dans l'enregistrement des passes.

**Le compositeur mélange par passes shader en ping-pong, pas par `BlendState` natif.**
Il tient deux textures d'accumulation `Rgba16Float`. Pour chaque couche, une passe plein
écran lit l'accumulé et la texture de la couche, applique le mode de `BlendParams` et
l'opacité, et écrit dans l'autre texture d'accumulation ; les deux s'échangent à la
couche suivante — le même ping-pong que `FeedbackLayer`, pour la même raison : lire et
écrire la même texture est interdit. Les `BlendState` de wgpu auraient suffi pour
`Normal` et `Add`, mais `Difference` exige de **lire** la destination, ce que le
mélange matériel ne permet pas ; et le calcul prémultiplié (§21) y reste implicite,
réparti entre des facteurs de blend. En shader, tous les modes de `BlendMode` passent
par la même mécanique et le calcul prémultiplié s'écrit en clair, une fois, en WGSL —
approche également retenue par `darkly` (§13). Coût : une passe plein écran par couche,
négligeable face au rendu des couches elles-mêmes. Ce choix est **interne** au
compositeur : il ne touche pas le contrat `RenderLayer`.

**Antialiasing : MSAA 4×, avec repli.** Une `ShapeLayer` rend dans une cible
multiéchantillonnée `Rgba16Float`, résolue vers sa texture de sortie. Le support du
MSAA 4× pour ce format est **vérifié à l'initialisation** (les capacités varient selon
le backend et le matériel) ; s'il manque, la couche rend en 1× et le signale par un
diagnostic de rendu (propre à `layer`, pas un compteur de `core::Diagnostics`, qui ne
doit rien savoir du GPU) plutôt que d'échouer — le régime « jamais de panique » du
§18.1 vaut aussi pour une capacité matérielle absente. L'antialiasing analytique
(franges calculées par la tessellation) reste une piste si le MSAA s'avère insuffisant
sur les traits fins ; il n'est pas construit avant d'en avoir mesuré le besoin.

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

### 22.1 Ce que l'étape 5 a fixé

**Le compositeur reçoit deux graphes par couche.** `Compositor::render` prend des
`LayerInput { layer, graph, blend_graph }`, pas un `&Graph` unique : chaque clip porte
son propre graphe (§3), et un graphe partagé aurait fait résoudre le `Param` d'un clip
contre les nœuds d'un autre. `graph` résout le contenu ; `blend_graph` résout l'opacité,
qui appartient à la piste, donc au graphe de composition (§3bis.1).
`LayerInput::new(layer, graph)` utilise le même graphe pour les deux.

**`ShapeRenderer` et `ShapeContent` (étape 7).** `ShapeRenderer` possède ce qui coûte :
textures, pipeline, buffers, mémoire réutilisée. `ShapeContent` n'est qu'un emprunt d'une
frame : un générateur, des effets, un style. Un même renderer dessine donc n'importe
quel clip sans rien réallouer, et sans garder trace du précédent. `ShapeLayer`, pour un
contenu qu'elle possède, n'est plus qu'un renderer et son contenu.

**Piège : un buffer d'uniformes par couche, pas un seul réécrit.** Les écritures de
`queue.write_buffer` sont toutes appliquées **avant** la soumission de l'encodeur. Un
buffer d'uniformes unique, réécrit avant chaque passe de mélange, ne verrait que la
dernière valeur : toutes les couches se mélangeraient avec l'opacité et le mode de la
dernière. Le compositeur garde donc un buffer par couche, qui grandit avec leur
nombre.

**Deux implémentations du mélange, une référence.** Les formules vivent dans
`composite.wgsl` (GPU) et dans `frogenx_render::blend` (CPU, sans wgpu). La version CPU
est la référence : un test de pixels compare, mode par mode, la sortie du GPU à
`blend_premul`. Toute modification de l'une se reporte dans l'autre — sinon le test
casse, et c'est voulu. Les modes séparables suivent la formule du W3C Compositing,
écrite directement sur les valeurs prémultipliées, sans division par alpha.

**`ShapeLayer` : remplissage, puis trait par-dessus, dans une seule passe.** Le
remplissage ne concerne que les contours **fermés**, tous tessellés ensemble en règle
**non nulle** : là où deux copies d'une répétition radiale se chevauchent, la zone reste
pleine (union) au lieu de se trouer. Un contour ouvert n'est jamais rempli. Le trait
s'applique à tous les contours ; sa largeur est en unités de l'espace normalisé (§17),
donc proportionnelle à la résolution comme le reste de la forme. Positions (déjà en
NDC) et couleurs (déjà en linéaire prémultiplié) sont calculées pendant `prepare` : le
shader de la couche ne décide rien.

**Une couche vide est effacée.** Sans triangle à dessiner, `encode` ouvre quand même sa
passe, avec un `Clear` transparent : la couche doit sortir vide, pas garder l'image de
la frame précédente. De même, le compositeur part d'une accumulation transparente —
sans couche, la sortie est transparente.

**Les buffers de sommets grandissent, ne rétrécissent jamais.** Par puissances de
deux, à partir de 64 Kio : une forme qui pulse (une répétition radiale modulée, par
exemple) ne réalloue pas à chaque frame.

**La première image est déterministe à l'octet près.** En `Clock::Fixed`, deux
exécutions de l'exemple `first_image` produisent le même PNG (même somme de contrôle).
C'est le §20.3 vérifié de bout en bout, du graphe jusqu'au fichier — sur une même
machine ; d'un GPU à l'autre, l'antialiasing peut différer, d'où la tolérance des
snapshots (§24.2).

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

**Ce qui existe depuis l'étape 7.** Les commandes de **pilotage** de la grille, dans
`composition`, donc testées sans GPU : déclencher un clip ou une colonne, vider une
piste, régler son opacité, changer de deck, fixer le BPM, taper le tempo. L'étape 7c y
ajoute la structure de la grille : ajouter ou retirer une colonne ou une piste, retirer un
clip, et deux réglages de clip, son amplitude de modulation et sa vitesse.
`Composition::apply` rend un `bool` : une commande sur un identifiant périmé est refusée
sans panique. `Engine` les applique entre deux frames. Tout pilote passe par là : les
touches 1 à 8 aujourd'hui, l'interface, le MIDI et l'OSC ensuite. Les commandes de
câblage ci-dessus arriveront avec l'éditeur de nœuds.

### 23.1 Charte graphique — Resolume Arena pour la disposition, Pro-Q pour le signal

**Décision : la disposition de FrogenX vient de Resolume Arena** (§23.2), et **son
apparence de FabFilter Pro-Q** : un noir légèrement teinté, des surfaces arrondies, le
signal comme fond de l'écran, et les réglages qui flottent dessus seulement quand on les
demande. La disposition répond à la conduite d'un set, que Resolume a résolue ; l'apparence
répond à la lecture d'un son, que Pro-Q a résolue, et le spectre est devenu le cœur de
l'audio-réactivité (§3bis.6). Cette charte reste amenée à évoluer avec les fonctionnalités.

**Palette (`ui/src/theme.rs`) :**

| Rôle | Ton | Usage |
|---|---|---|
| Fond principal | noir teinté de violet | zone de grille, panneaux, monitor |
| Fond des en-têtes | gris sombre, légèrement plus clair | barres, cases vides |
| **Accent unique** | or vif | état actif — clip joué, valeur choisie, bouton pressé, crêtes du spectre |
| Couleurs de trig | six teintes saturées | **dans le spectre seulement** : une par trig |
| Texte principal | blanc cassé | tous les labels |
| Texte secondaire | gris moyen | labels secondaires, placeholders |
| Bordures | noir quasi pur | séparateurs entre panneaux |

**Un seul accent porte tout l'état actif**, plutôt qu'une couleur par catégorie d'élément.
Le principe vient de Resolume et il reste, même si la teinte a changé : dans une régie
sombre, une palette monochrome avec un accent unique reste lisible sous faible éclairage,
et ne rivalise jamais avec le contenu vidéo du monitor, qui doit rester l'élément le plus
saturé de l'écran.

**Une exception, et sa raison : les trigs.** Dans le spectre, la couleur ne dit pas un
état, elle distingue des objets qui se superposent, comme les bandes d'un égaliseur. Elle
y reste enfermée : hors du panneau audio, rien d'autre que l'or n'est saturé.

**Un survol ne gonfle pas une case.** Le défaut d'egui agrandit légèrement un bouton
survolé ; sur une grille, cela fait bouger ses voisines, et rien ne justifie ce mouvement
sur scène.

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

**Reporté :** la première interface a des panneaux redimensionnables et masquables, mais
pas déplaçables (§23.5). Ses quatre zones sont aujourd'hui trois : la scène réunit le
monitor et le panneau du clip (§23.9).

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

### 23.5 La première interface (étape 7b)

**`ui` est une fonction de l'état.** À chaque frame, `app` lui prête la grille en
lecture et une vue de ce qui se passe (tempo, fps, statut des sorties, texture du
monitor) ; elle dessine et remplit une liste de `Command`, que `Engine` appliquera entre
deux frames (§23). Seul le choix du clip affiché dans le panneau de réglages lui
appartient (`UiState`).

```
┌──────────────────────────────────────────────────────────┐
│ En-tête : menus, tempo │ fichier │ fps, sorties (§23.9)   │
├──────────────────────────────────────────────────────────┤
│ Grille : pistes × colonnes                               │
├─────────────────────────────────────────┬────────────────┤
│ Scène : image, réglages du clip choisi  │ Bibliothèque   │
├─────────────────────────────────────────┴────────────────┤
│ Audio : spectre, zones, analyse (§23.7)                  │
└──────────────────────────────────────────────────────────┘
```

**Grille.** Une ligne par piste, la plus haute en haut, comme dans la pile de
composition. Un clic sur une cellule déclenche le clip et l'affiche dans les réglages ;
un clic sur un en-tête déclenche la colonne. Le clip actif porte l'accent or (§23.1) ;
il n'y a **pas de vignette**, qui évaluerait tous les clips. Chaque ligne montre une case
par colonne, vide ou non (§3bis.1).

**Réglages.** La vitesse du clip, l'opacité de la piste, puis les groupes de
`Clip::params` (§3bis.2), dans le panneau flottant du clip (§23.9). Chaque potentiomètre
émet `Command::SetParam`, qui passe par `Param::set_base` (§16.3 ter). Un paramètre modulé
a une couronne, et une saisie pour sa profondeur.

**Le thread.** egui tourne sur le thread principal, dans la boucle du moteur (§20.2) :
la frame d'abord, l'interface ensuite, puis une présentation, à chaque échéance. Une
interface lente ralentirait le rendu ; son temps est mesuré et affiché dans le bilan de
`--frames`. Mesuré à 0,3 ms par frame en release, 1,5 ms en debug, avec des intervalles
de 15,8 à 17,6 ms : rien ne réclame un thread de rendu séparé.

**Mesurer une cadence.** Moyenner `dt`, puis inverser ; jamais moyenner `1 / dt`. Un seul
intervalle quasi nul, comme la toute première frame, affichait 420 fps sur un moteur à
60. Le bilan donne aussi l'intervalle minimal et maximal, hors échauffement : une
moyenne correcte peut cacher des saccades.

**Pièges d'egui 0.37.** `default_fonts` n'est plus activée par défaut : sans elle, le
premier texte panique. `TexturesDelta` vérifie, à sa destruction en build debug, qu'il a
été **vidé**, y compris sur une frame sautée : le parcourir avec `drain()`. Un curseur
expose deux nœuds de même nom : en test, viser le champ numérique par son rôle
(`SpinButton`), le seul qui accepte une valeur simulée. Une entrée glissable porte un
`Label` en `selectable(false)` : sinon la sélection de texte capte le geste.

**Le clavier** reste actif, sauf quand egui veut la saisie : Espace dans un champ ne doit
pas taper le tempo.

**La fenêtre change de rôle.** Elle montre l'interface, et l'image passe dans un panneau.
`Preview` n'y sert donc plus : elle présente dans la fenêtre de sortie (§8.4).

**Ce qui est vérifié, et comment.** `egui_kittest` couvre la logique des clics, et un
test d'`Engine` prouve qu'un `SetParam` change l'image à la frame suivante. L'affichage
de la vraie fenêtre a été regardé sur des captures de sa seule zone (`PrintWindow`). Un
clic réel sur une cellule y a été observé une fois, par messages postés à la fenêtre
(`PostMessage`) ; deux essais suivants n'ont rien produit, peut-être parce que Windows
signale la sortie du pointeur quand le vrai curseur est ailleurs, sans que cela soit
établi. **La souris réelle a été validée à la main** sur la fenêtre de l'application. Le
redimensionnement, la réduction et la récupération d'une surface perdue n'ont pas été
essayés.

**Non couvert.** Les onglets et l'ajout d'effets à un clip sont dans l'éditeur de
nœuds (§23.10). Les couleurs du style sont réglables par curseurs, canal par canal ; un sélecteur
de couleur viendra après.

### 23.6 La bibliothèque de clips (étape 7c)

**`library` tient un catalogue statique.** Chaque entrée porte un nom, une famille et une
fonction qui construit le clip : 27 clips, en cinq familles (fonds, formes, traits,
motifs, échos). Tous sont écrits en code, parce que le format de fichier n'arrive qu'à l'étape
7e. Une sauvegarde décrira ensuite le clip lui-même, jamais son entrée de catalogue : un
clip réglé n'est plus celui du catalogue.

**Tous sont calés sur le tempo et tous bougent**, sauf le fond uni `Nuit`. Sinon, ni
l'amplitude ni la vitesse n'auraient d'effet sur eux.

**Le geste.** Le panneau liste le catalogue par famille. On glisse une entrée sur une
case, vide ou occupée ; un clic droit sur une case retire le clip, un clic droit sur un
en-tête de colonne ou un nom de piste retire la colonne ou la piste. Retirer passe par un
menu, jamais par un bouton direct : sur scène, un clic raté ne doit pas supprimer un clip.

**L'interface ne construit pas de clip.** `Command` reste `Copy` et comparable, ce qui
rend les tests d'interface possibles, alors qu'un `Clip` porte des `Box<dyn ...>`. `draw`
remplit donc une seconde liste de `Placement { layer, column, preset }`, et `app` construit
le clip entre deux frames avant de le remettre à `Engine::place`. Un refus se compte comme
une commande refusée.

**Démarrage.** `library::starter` construit trois pistes et huit colonnes : le fond, les
formes, les traits. Elle sert aussi à l'exemple `sequence` et aux tests d'`Engine`. Sans
sauvegarde, c'est ce qu'on retrouve à chaque lancement jusqu'à l'étape 7e.

**Ce qui vérifie le catalogue.** Un test parcourt chaque entrée sans GPU : le clip se
construit, porte le nom du catalogue, dessine quelque chose et produit un `Path` valide et
fini sur 240 frames, avec des `Diagnostics` nuls. Un test d'`Engine` rend chaque entrée
sur le GPU. Un test d'interface vérifie qu'afficher chaque clip n'émet aucune commande.
L'exemple `catalogue` rend une planche de toutes les entrées, pour les regarder : un test
ne dit pas si une forme est belle. C'est elle qui a montré qu'un premier `Engrenage` à
dents trop larges ne dessinait qu'un anneau.

### 23.7 Le panneau audio

Le son est le fond de l'écran : c'est le principe repris de FabFilter Pro-Q (§23.1). Tout
le panneau (`ui/src/spectrum.rs`) est un tracé, sur lequel chaque trig pose sa zone.

```
┌─────────────────────────────────────────────────────────┬────┐
│  ┌─zone──●──┐                     ┌─zone──●──┐        -12 │ ▐  │
│  └──────────┘    courbe du son    └──────────┘        -24 │ ▐  │
│        ╭───panneau flottant du trig choisi───╮        -36 │ ▐  │
│ ▁▂▃▅▆▅ │ ◔ ◔ ◔ ◔ ◔   1/1 1/2 1/4 1/8 … Cible │ ▅▃▂▁   -48 │ ▐  │
│ 20  50 ╰─────────────────────────────────────╯   10k  20k │    │
├─────────────────────────────────────────────────────────┴────┤
│ Trig 1  Trig 2  Ajouter   source   Analyse  Échelle  Amplitude  Fenêtre │
└──────────────────────────────────────────────────────────────┘
```

**Le tracé.** Le son est une surface grise qui s'efface vers le bas, lissée entre les
bandes (Catmull-Rom). La courbe or est celle des crêtes : elle saute au niveau et retombe
doucement, pour montrer où poser une zone quand le son est déjà passé. Les repères suivent
les échelles (§7) : `freq_marks` efface ceux qui se chevauchent, si bien que l'échelle
linéaire perd ses graves plutôt que de les empiler. Les repères de fréquence se dessinent
après la courbe : avant, le son les recouvrait.

**Une couleur par trig.** Les zones sont les seuls éléments colorés de l'interface
(`theme::TRIGS`). Elles s'éclairent porte ouverte, et plus fort, avec un halo sur la
poignée, quand le compteur laisse passer le coup. Le trig choisi porte un contour plus
épais et une bague blanche.

**Les gestes.** Glisser dans le vide trace une nouvelle zone (`Command::AddTrigAt`) ; le
trig qui vient de naître devient le trig choisi. Attraper le corps déplace la zone, un bord
la redimensionne, et le curseur annonce lequel. Cliquer choisit une zone, cliquer dans le
vide referme le panneau : le spectre redevient lisible. Un geste se calcule en coordonnées
du tracé `[0, 1]²`, ni en Hz ni en pixels : il vaut donc sur toutes les échelles, et se teste
sans egui (`moved`, `grab_at`). Seules les bornes touchées émettent une commande.

**Le panneau flottant** n'existe que pour le trig choisi, et son fond retient la souris :
un clic manqué ne trace pas de zone dessous. Cinq potentiomètres (`BASSE`, `HAUTE`, `SEUIL`,
`PLAFOND`, `MARGE`), chacun avec sa valeur exacte en saisie ; le compteur en boutons, parce
que six valeurs se lisent mieux qu'une liste ; le coup attendu, la cible et `Recaler`.
Retirer un trig passe par un clic droit sur sa puce : sur scène, un clic raté ne
supprime rien (§23.6). La barre du bas règle l'analyse (§7), et la fenêtre annonce sa résolution au
survol. Sans entrée audio, elle dit pourquoi et l'application tourne quand même (§18.1).

**Pièges.** `drag_started` ne tombe qu'après quelques pixels : un geste part de
`pointer.press_origin()`, pas du curseur. `vertical_centered` prend toute la largeur
disponible : cinq potentiomètres y étiraient le panneau sur le son qu'il doit laisser voir.
`override_text_color` l'emporte sur tout : le texte d'un état choisi restait clair sur l'or.
`ComboBox::from_label` met son nom à droite ; il est donc ajouté après, dans une barre
remplie de droite à gauche. Un cadran tenu sans bouger ne doit rien émettre : sinon, le
fichier se marque comme modifié à chaque frame (§23.8).

**Ce qui est vérifié, et comment.** Les clics, les glissés et les potentiomètres par
`egui_kittest` ; le parcours complet, du son au trig, sur une boucle de batterie de synthèse
(16 coups, 16 fronts, puis 4, 2 et 1 passages à 1 sur 4, 8 et 16) ; le déclenchement dans
`Engine`, sans fenêtre. Trois captures de la vraie fenêtre ont été regardées, avec un
spectre simulé. Mesuré en release, avec le panneau ouvert et deux zones : 0,83 ms
d'interface par frame, 16,2 à 17,1 ms entre deux frames. Sur le thread d'analyse, à 48 kHz :
1 % d'un cœur au plus en FFT, 10,6 % au pire en `ConstantQ` (fenêtre de 250 ms).

L'entrée réelle s'ouvre et transmet du signal (M-Audio ProFire 610, 6 canaux, 44,1 kHz,
Windows), mais elle était silencieuse pendant l'essai. **Pas encore essayés :** un trig sur
de la vraie musique, le choix des méthodes à l'oreille, et les gestes à la vraie souris.

### 23.8 Le fichier (étape 7e)

**Le menu `Fichier`.** `Nouveau`, `Ouvrir…`, `Enregistrer` (Ctrl+S) et `Enregistrer sous…`.
La barre montre le nom du fichier, suivi de `*` s'il reste des modifications. `Nouveau`
revient à la grille de départ.

**La boîte de dialogue ne bloque pas le moteur.** `rfd` l'ouvre hors du thread du moteur :
une boîte bloquante figerait l'image sur la sortie tant qu'elle reste ouverte. Lecture et
écriture du fichier se font sur un thread à part. L'encodage et le décodage restent
synchrones : seul le disque peut être lent, et le remplacement de la grille a lieu entre
deux frames.

**Le parcours se teste sans fenêtre.** `FileFlow`, dans la bibliothèque d'`app`, est une
machine à états sans `rfd` ni `winit` : elle reçoit les demandes et les réponses, et rend
l'effet à produire. Le binaire se contente de produire ces effets.

**Modifications non enregistrées.** Avant `Nouveau`, `Ouvrir…` ou de quitter, une fenêtre
egui propose `Enregistrer`, `Ne pas enregistrer` et `Annuler`. Si `Enregistrer` échoue ou si
l'utilisateur renonce, l'action prévue est abandonnée : rien ne se perd en silence. Quitter
passe donc par `FileFlow`, plus directement par `event_loop.exit()`.

**Ce qui compte comme une modification.** `Command::is_edit`, un `match` exhaustif sans cas
par défaut : une nouvelle commande oblige à trancher. Déclencher un clip n'en est pas une :
c'est jouer, et un trig en déclenche des centaines. Régler, poser, retirer et changer de
tempo en sont. Le clip actif n'est donc enregistré qu'avec une vraie sauvegarde.

**Au lancement.** Priorité à `--patch fichier`, puis au dernier fichier ouvert ou
enregistré, puis à la grille de départ. Le chemin du dernier fichier se garde dans le
dossier de configuration du système, sans nouvelle dépendance. S'il a disparu ou se refuse,
l'application démarre sur la grille de départ et le dit dans l'en-tête : un lancement ne doit
jamais échouer à cause d'un fichier.

**L'ordre d'un graphe est stable.** Le tri topologique prend le plus petit index d'abord :
un graphe relu d'un fichier, déjà trié, garde son ordre. Sans cela, réenregistrer un fichier
inchangé réordonnait ses nœuds, et git aurait vu un fichier différent. Un test l'a révélé.

**Remplacer la grille.** `Engine::replace_composition`, entre deux frames, vide les
commandes et les poses en attente, qui visaient l'ancienne grille. Les identifiants de la
nouvelle grille peuvent coïncider avec les anciens : l'interface oublie sa sélection. La
sortie plein écran reste ouverte et montre la nouvelle image.

**Ce qui est vérifié, et comment.** `FileFlow` couvre le parcours sans fenêtre, y compris
les cas de perte : une écriture ratée ou un nom refusé ne quitte pas. Les 24 clips du
catalogue relus donnent la même forme, point par point, sur 240 frames ; une grille relue
donne les mêmes pixels. Les fichiers abîmés sont refusés un par un, sans toucher à la grille.
Une fixture de la version 1, jamais régénérée, doit se relire et se réécrire à l'identique :
un format changé sans nouvelle version fait échouer ce test.

Sur la vraie fenêtre (Windows, Vulkan), un binaire temporaire a enregistré sur le disque,
ouvert la boîte `Enregistrer sous`, y a renoncé, puis a demandé à quitter avec des
modifications. **La boîte ouverte ne ralentit pas le moteur** : 60 fps, intervalles de 15,9
à 17,5 ms sur tout l'essai. Enregistrer ne s'y voit pas non plus. La question a été regardée
sur une capture. Le lancement a été essayé avec le dernier fichier, `--patch`, puis avec un
fichier disparu, abîmé ou d'une version future : l'application démarre et le dit.

**Pas encore essayés :** le menu `Fichier` et la question à la vraie souris (un clic posté
n'atteint pas egui, §23.5), une ouverture réelle par la boîte `Ouvrir…`, Ctrl+S dans la vraie
fenêtre (couvert par un clic simulé), et Linux et macOS, où `rfd` a ses propres contraintes
de thread.

### 23.9 Toute l'interface sur le modèle du spectre

Le panneau audio (§23.7) a fixé un modèle ; le reste de l'application le reprend, parce
qu'une interface se lit mieux quand tout s'y manipule de la même façon.

| Principe | Panneau audio | Reste de l'interface |
|---|---|---|
| Le signal d'abord | le spectre remplit le panneau | l'image de sortie occupe la scène, les cellules actives montrent leur boucle |
| Un réglage flotte, et disparaît | le panneau du trig choisi | le panneau du clip choisi, à côté de l'image |
| Un seul geste de réglage | potentiomètres | potentiomètres pour tous les réglages de clip (`widgets::knob`) |
| Fermer se fait dans le vide | clic dans le vide du spectre | clic sur la scène, ou `Fermer` |
| Retirer passe par un menu | clic droit sur la puce d'un trig | clic droit sur une case, une colonne, une piste (§23.6) |

**L'en-tête** remplace la barre de menu et la barre d'outils : une seule ligne, comme le
haut de Pro-Q. À gauche le nom, les menus et le tempo, au plus près parce que c'est ce
qu'on touche le plus sur scène ; au milieu le fichier, centré sur la fenêtre et non sur la
place laissée par les menus ; à droite l'état de la machine. Le nom du fichier, ses refus,
et l'état de la sortie y restent toujours lisibles (§8.4, §23.8). La pastille du tempo
pulse à chaque temps : elle dit le tempo, **pas la place dans la mesure**, puisque rien ne
recale encore la phase (§3bis.3).

**Le tempo** a deux gestes de plus : `Suivre`, et à côté ce que le son en dit, `son : 128.0`
ou `son : —` (§7.1).

**La scène.** L'image est calée à gauche, entière. **Le panneau du clip ne la couvre pas :
c'est la seule entorse au modèle du spectre, et elle est voulue.** Un spectre se lit encore
sous un panneau ; une image de sortie doit se voir entière. Il flotte donc à côté, dans
440 pixels réservés, et ne se pose dessus que sous 800 pixels de large (`stage_layout`).
L'image ne dépend pas de la sélection : choisir un clip ne la fait pas sauter en plein set.

**La grille.** Une cellule active porte l'or, un halo qui ne déplace pas ses voisines, et
une barre d'avancement de sa boucle (`loop_progress`). Seul le clip actif est lu : c'est
le même coût qu'avant, puisqu'il est déjà évalué (§3bis.2). Une case vide n'est qu'un
contour : la grille se lit par ce qu'elle contient.

**Le panneau du clip.** Un potentiomètre par `Param`, regroupés en cartes : le clip et sa
piste, le générateur, chaque effet, le style. Un clic droit sur un cadran y branche une
source audio (§7.1). Un `Param` modulé porte **une couronne**,
reprise de l'anneau de gain de Pro-Q : l'arc montre ce que la modulation parcourt, le point
où elle en est à cette frame, et une ligne `±` règle sa profondeur.

**Pièges.** `horizontal_wrapped` ne replie que ce dont il connaît la taille d'avance : une
carte y restait sur sa ligne, coupée au bord, alors qu'il restait de la place dessous. Le
pliage est donc calculé (`wrap_cards`), et une carte trop large replie ses propres
potentiomètres. `DragValue` réécrit par défaut une valeur hors plage, ce qui émettrait une
commande à chaque frame : `clamp_existing_to_range(false)`, pour la même raison que les
curseurs d'avant (§16.3 ter). Sans cela, un rayon hors plage marquait le fichier comme
modifié sans que personne n'y touche.

**Ce qui est vérifié, et comment.** Les clics, glissés et saisies par `egui_kittest` ;
le pliage, le partage de la scène, l'avancement et la pulsation par des tests purs. Trois
captures de la vraie fenêtre ont été regardées, avec une grille de départ, un clip ouvert,
deux trigs et un spectre simulé. Mesuré en release, en 1440 × 900 : 1,2 ms d'interface par
frame, 16,0 à 17,2 ms entre deux frames. **Pas encore essayés :** les gestes à la vraie
souris, une scène sous 800 pixels à l'écran (elle n'est testée que par `stage_layout`), et
une fenêtre sur un écran 4K.

### 23.10 L'éditeur de nœuds

> Tranché (choix A1, B2, C2). Raisons au §13.

**Maison, sur egui.** `egui-snarl` 0.12 demande egui 0.36 et `egui_node_graph2` ne bouge plus
depuis 2024 : patcher une crate tierce à chaque version d'egui coûterait plus qu'un éditeur
qui suit nos conventions, s'affiche comme le reste et se teste par clics simulés.

**Dans sa fenêtre.** Le menu `Affichage` l'ouvre ; sa croix la ferme, sans toucher ni au
moteur ni à l'interface. Elle a son propre contexte egui, et ne coûte rien fermée. Le câblage
se fait donc à côté de l'image, éventuellement sur un autre écran. `--nodes` l'ouvre au
lancement, pour vérifier sans souris.

**Deux graphes.** L'onglet `Clip` montre le graphe du clip choisi dans l'interface, l'onglet
`Régie` celui de la composition, où vivent les trigs. Une source ne se branche que dans son
propre graphe : c'est ce qui garde chaque clip déplaçable (§3).

**Tout est une `Command`** (§23), donc testé sans fenêtre et prêt pour l'annulation :

```rust
pub enum GraphScope { Composition, Clip(ClipId) }

pub enum ParamTarget {
    ClipParam { clip: ClipId, slot: ParamSlot },
    NodeParam { scope: GraphScope, node: NodeId, input: u8 },
    LayerOpacity(LayerId),
    ClipSpeed(ClipId),
    Tempo,
    AnalysisWindow,
}

// Dans `Command` :
AddNode { scope: GraphScope, kind: NodeKind },
RemoveNode { scope: GraphScope, node: NodeId },
Connect { target: ParamTarget, source: NodeId },
Disconnect { target: ParamTarget },
SetTargetBase { target: ParamTarget, value: f32 },
SetTargetDepth { target: ParamTarget, value: f32 },
SetNodeValue { scope: GraphScope, node: NodeId, field: u8, value: NodeValue },
AddEffect { clip: ClipId, kind: EffectKind },
RemoveEffect { clip: ClipId, index: usize },
```

**Une entrée de nœud est un `Param` de sa `Spec`.** Un `Signal` reste opaque : pour changer un
réglage, `composition` lit sa `Spec`, la modifie, reconstruit le `Signal` par la table qui
quitte `patch` pour `composition`, et le pose par `Graph::rewire`. Celui-ci garde l'identifiant
et l'état, recalcule les dépendances depuis les sources des `Param`, et refuse un cycle : le
§3 tient donc aussi à l'édition. L'interface lit `Graph::can_connect` pour ne pas proposer un
branchement refusé.

**Retirer un nœud débranche d'abord ce qui le lit**, nœuds et destinations : chacun retombe
sur sa base. Les nœuds d'un trig et la position d'un clip ne se retirent pas, et un trig ne se
règle que depuis son panneau : le `Trig` garde les versions de référence (§3bis.6). Ses sorties
se branchent, c'est tout leur intérêt : `flash` sur une opacité.

**Le geste.** On tire d'une sortie vers une entrée ; un clic droit sur une entrée la débranche,
sur le fond ajoute un nœud ou un effet, sur un titre retire le nœud. Le fond se déplace au
glissé, chaque nœud aussi. Les destinations sont à droite : les `Param` du clip, ou le tempo,
la durée d'analyse, l'opacité des pistes et la vitesse des clips actifs.

**La place des nœuds n'est pas dans le fichier.** Elle se calcule par profondeur dans le graphe
et se garde pour la session : un patch décrit ce qui joue, pas sa mise en page.

**Brancher ne touche pas à la base.** Une source s'ajoute à la base du `Param` : une opacité à
`1` modulée par un `flash` reste à `1`. Pour qu'un coup allume une piste, on met sa base à `0`
puis on la branche. Débrancher rend cette base : mettre `0` en silence à sa place effacerait
la piste au débranchement.

**Valeurs affichées.** Seul le clip actif est évalué (§3bis.2) : le graphe d'un clip qui ne
joue pas montre ses dernières valeurs.

**Ce qui est vérifié, et comment.** Le câblage se teste sans fenêtre : cycles refusés sans
rien changer, source d'un autre graphe refusée, nœud retiré qui rend leur base à ce qui le lit,
nœuds tenus par le système, phase gardée pendant un réglage. L'éditeur se teste par clics
simulés : tirer un fil, lâcher dans le vide, menus, boucle non proposée. De bout en bout, un coup
dans le son traverse un trig et un câble posé par commandes et allume une piste à l'image, et un
câblage complet se relit d'un fichier, qui reste stable. Sur la vraie fenêtre : le moteur reste à
60 fps et 16,1 à 17,3 ms, l'éditeur présente chacune des 300 frames, et sa croix le ferme sans
toucher à l'application. L'affichage de l'onglet `Régie` a été regardé sur une capture.

**Pas encore essayés :** tirer un fil à la vraie souris, le menu du fond, et l'onglet `Clip` sur
une capture. Le fil s'y dessine par clics simulés seulement.

**Non couvert.** La place des nœuds se perd à la fermeture de l'application. Le déplacement d'un
effet dans la chaîne, le zoom du canevas et l'annulation ne sont pas faits. Une opacité ou une
vitesse ne reçoit qu'une source de la régie, jamais d'un clip : c'est le §3.

### 23.11 Annuler et rétablir

> Tranché (choix A2, B1, C1, D1). Raisons au §13.

**Une image complète de la grille, pas des commandes inverses.** `History` garde le texte du
format de fichier à chaque geste. Écrire un inverse pour chacune des commandes aurait été à
refaire pour chaque nouvelle, et un inverse faux corromprait la grille sans le dire : ici, une
nouvelle commande est couverte d'office. Mesuré sur la grille de départ : 90 Ko, 1,1 ms pour
l'écrire, 0,8 ms pour la relire, soit 9 Mo au plus pour 100 pas. L'historique vit en mémoire, et
se vide quand on ouvre un fichier ou qu'on fait `Nouveau`.

**L'image repart, sans sauter.** Une grille relue d'un fichier repartirait de zéro. Après une
restauration, `Composition::adopt_from` lui transmet donc ce qui joue :

| Ce qui passe de l'ancienne grille à la restaurée | Comment |
|---|---|
| Le clip actif de chaque piste | annuler ne déclenche ni n'éteint rien : jouer n'est pas modifier |
| L'état des nœuds d'un clip | copié si ses nœuds n'ont pas changé, type par type ; sinon, ce seul clip repart de zéro |
| L'état du graphe de composition | le compte des trigs tient de la même façon |

Les pistes et les clips se retrouvent par leur position, puis par leur nom : retirer une colonne
décale les cases. `adopt_from` rend aussi les correspondances d'identifiants (`IdMap`) : le clip
choisi et le trig choisi le restent.

**Un pas est un geste.** Un potentiomètre tourné émet une commande par frame. Tant qu'un bouton
de la souris est tenu, dans l'une ou l'autre fenêtre, les modifications s'accumulent ; le pas se
ferme au relâchement, après 200 ms sans modification, ce qui groupe aussi une saisie au clavier.
Annuler ferme d'abord le pas en cours.

**Seules les modifications comptent** (`Command::is_edit`, §23.8). Un déclenchement ne crée pas de
pas, et un trig ne remplit donc pas l'historique. Revenir à l'état enregistré retire le `*` du
fichier : chaque pas porte un numéro, comparé à celui de la dernière sauvegarde.

**Le geste.** Ctrl+Z annule, Ctrl+Y ou Ctrl+Maj+Z rétablit, dans l'interface comme dans l'éditeur
de nœuds, sauf pendant une saisie dans un champ, qui garde les siens. Le menu `Édition` nomme ce
qui va être annulé, et l'en-tête dit ce qui vient de l'être : un Ctrl+Z accidentel se voit.

**Ce qui est vérifié, et comment.** `History` se teste sans grille : gestes groupés, limite de 100
pas, retour à l'état enregistré. `adopt_from` se teste sans GPU : clip actif et phase repris, seul
le clip modifié qui repart, colonne et piste retirées retrouvées par leur nom. Sur le GPU, une
grille annulée puis rétablie rend la même image au pixel près que si on n'y avait pas touché, et
annuler ne change pas ce qui joue. Le menu et les raccourcis se testent par touches simulées, dans
les deux fenêtres. L'application tourne à 60 fps avec l'éditeur ouvert.

**Pas encore essayés :** Ctrl+Z dans la vraie fenêtre et à la vraie souris, sur un vrai geste.

**Non couvert.** Annuler une colonne retirée rend ses clips neufs, phases à zéro, puisqu'ils
n'existent plus dans la grille qui joue. Une grille qui contiendrait un `Signal` absent de la table
ne s'enregistre pas, donc ne s'annule pas : c'est sans effet, et sans panique. La place des nœuds
n'est pas concernée, puisqu'elle n'est pas dans le fichier.

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

**Mécanique, à partir de l'étape 5.** Les tests qui ont besoin du GPU demandent un
adaptateur par une fonction d'aide qui rend `Option<Gpu>`. Sans adaptateur, le test
s'arrête **avec succès** et le signale sur sa sortie (`GPU absent, test sauté`) —
jamais un échec, jamais une panique. Tout ce qui peut être testé sans GPU l'est sans
GPU : la tessellation, la conversion de couleur, le placement des formes dans l'espace
normalisé (§17) sont des fonctions pures. Les tests de pixels de l'étape 5 vérifient
des valeurs précises en quelques points (centre, coin) avec une petite tolérance ; les
snapshots d'image entière avec tolérance perceptuelle (§24.2) viennent plus tard, quand
il y aura des images assez riches pour que la comparaison point par point ne suffise
plus.

**Les tests tournent un par un** (`RUST_TEST_THREADS = "1"`, dans `.cargo/config.toml`).
Plusieurs devices wgpu ouverts en parallèle bloquaient environ 3 % des exécutions d'un
binaire de tests, indéfiniment, sur un test différent à chaque fois : mesuré à 3 sur
120 avant toute modification de `Gpu`, à 5 sur 120 après, et à 0 sur 300 une fois les
tests en série. **La cause exacte n'est pas identifiée** — pilote, Vulkan ou wgpu. L'application n'ouvre qu'un seul device et n'y
est donc a priori pas exposée. Ne pas retirer ce réglage sans refaire cette mesure : le
défaut n'apparaît pas sur quelques exécutions.

### 24.4 Fixtures de patchs

`crates/patch/tests/fixtures/` contient un patch par version de format (§19.2). **Jamais
régénérées** — un fichier v1 écrit aujourd'hui doit rester tel quel pour prouver dans
deux ans que la migration fonctionne encore.
