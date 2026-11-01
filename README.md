# FrogenX

Mixeur de visuels procéduraux en temps réel, organisé en grille de clips, où chaque clip
se pilote comme un synthétiseur modulaire et où le son module l'image.

Des **signaux** (oscillateurs, LFO, features audio) modulent des **paramètres** de
**formes**, rendues en **couches** composées ensemble, puis envoyées vers plusieurs
**sorties** simultanées.

- Rendu temps réel GPU (wgpu) — Linux, macOS, Windows
- Modulation audio-réactive
- Sortie plein écran sur un second écran, flux réseau par Spout et KyberFrog, enregistrement
  vidéo ; à venir : Syphon

## Documentation

- [Architecture](docs/ARCHITECTURE.md) — décisions structurantes, feuille de route,
  et le *pourquoi* de chaque choix.

## Contribuer

Deux règles non négociables, détaillées dans
[§15 de l'architecture](docs/ARCHITECTURE.md#15-contrat-de-contribution) :

1. **Toute fonctionnalité arrive avec ses tests unitaires.**
2. **Toute modification met la documentation à jour dans le même commit.**

## État

La première version fonctionne : `cargo run` ouvre l'application. Ce qui est livré et ce qui
reste à faire sont dans le [backlog](docs/ARCHITECTURE.md#121-le-backlog-des-features).

## Licence

[AGPL-3.0 ou ultérieure](LICENSE), comme KyberFrog. Sauf mention contraire, une contribution est
proposée sous cette licence.
