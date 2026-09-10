---
name: poc-video
description: Produce a video POC from FrogenX — deterministic offline render, ffmpeg encoding settings, and the patch/audio choices that make a demo read well. Use when asked for a demo clip, a rendered sequence, or a visual proof.
---

> **Statut : amorce.** Les commandes ci-dessous sont la cible, pas du vécu — le
> pipeline de rendu n'existe pas encore (étapes 4, 9 de §12). Corriger ce fichier avec
> les vraies invocations dès le premier POC produit, et retirer cet avertissement.

## Toujours rendre en mode déterministe

```rust
Clock::fixed(60.0)
```

`Realtime` peut perdre des frames si l'encodeur décroche — acceptable en live,
**jamais** pour un POC. En `Fixed`, `wall_dt` est ignoré : chaque frame est calculée
entièrement, le résultat est reproductible à l'identique. Un POC qu'on ne peut pas
re-rendre exactement n'est pas un POC, c'est un accident.

## Encodage

Pipe vers le binaire `ffmpeg` (§8.2) — pas de bindings, et ffmpeg choisit lui-même
l'encodeur matériel de la plateforme.

Pour une diffusion générale (H.264, compatible partout) :
```bash
ffmpeg -f rawvideo -pix_fmt rgba -s 1920x1080 -r 60 -i - \
       -c:v libx264 -pix_fmt yuv420p -crf 18 -preset slow out.mp4
```

`-pix_fmt yuv420p` est **obligatoire** pour la lecture hors ffmpeg — sans lui, beaucoup
de lecteurs et la plupart des navigateurs refusent le fichier. `-crf 18` est
visuellement sans perte ; descendre à 15 pour des dégradés fins, qui sont exactement ce
que ce logiciel produit.

Pour une boucle GIF/web courte, préférer un `.webm` VP9 ou un `.mp4` court — un GIF
détruit les dégradés.

## Ce qui fait qu'un POC se lit bien

- **Une idée par clip.** Un POC démontre *une* chose : le morphing du polygone, ou
  l'audio-réactivité, ou le feedback. Trois à la fois ne démontrent rien.
- **10 à 20 secondes.** Assez pour voir la respiration du système, assez court pour être
  regardé en boucle.
- **Boucle parfaite** quand c'est possible : choisir des fréquences d'oscillateurs en
  rapports **entiers** sur la durée du clip, et le raccord est invisible. Un oscillateur
  à 1 Hz sur 10 s fait exactement 10 tours.
- **Commencer posé.** Un visuel qui part à fond ne montre pas d'évolution. Laisser la
  modulation monter — une `Envelope` à attaque lente sur l'amplitude globale suffit.

## Formes qui rendent bien, par ordre de rentabilité

1. **Polygone à nombre de côtés flottant** modulé lentement — le morph triangle → carré
   → cercle est le plus lisible des arguments pour le système paramétrique.
2. **Cercle déformé par oscillateur** sur la normale → blob organique. Deux oscillateurs
   de fréquences **non harmoniques** (0.3 Hz et 0.7 Hz, pas 0.5 et 1.0) donnent un
   mouvement qui ne se répète pas visiblement.
3. **Lissajous** à rapport non entier — dessine sa propre trajectoire, très lisible.
4. **Feedback** avec une légère rotation + zoom par frame : coût quasi nul, rendu
   spectaculaire. À garder pour un POC dédié tant il domine visuellement.

Le **bruit fractal** (`Noise::fractal(4, 0.5)`) sur un rayon donne un organique
immédiat, là où un LFO seul reste mécanique. C'est le module le plus rentable d'`osc`
pour un POC.

## Audio

Pour un POC audio-réactif, un fichier vaut mieux qu'une entrée live : reproductible, et
on peut re-rendre exactement.

**Lisser les features** — brutes, elles produisent un visuel nerveux qui se lit mal.
`Envelope::audio()` (attaque 0.01 s, release 0.25 s) est le réglage de départ : c'est
l'asymétrie attaque/release qui donne l'impression que le visuel « respire » avec le
son plutôt qu'il ne tremble.

Mapper de préférence : grave → échelle, aigu → détail/complexité, onset → accent
ponctuel (via `Trigger` + `SampleHold`). Éviter de tout brancher sur tout — un POC
lisible a deux ou trois liaisons, pas dix.

## Livrer

Après rendu, envoyer le fichier à l'utilisateur avec `SendUserFile` — il peut suivre la
conversation depuis un autre appareil, où un chemin local ne lui sert à rien.
