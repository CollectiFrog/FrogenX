# FrogenX

Générateur de visuels procéduraux, organiques et géométriques — temps réel, piloté
comme un synthétiseur modulaire.

Des **signaux** (oscillateurs, LFO, features audio) modulent des **paramètres** de
**formes**, rendues en **couches** composées ensemble, puis envoyées vers plusieurs
**sorties** simultanées.

- Rendu temps réel GPU (wgpu) — Linux, macOS, Windows
- Modulation audio-réactive
- Sorties simultanées : écran, NDI / Spout / Syphon, enregistrement vidéo

## Documentation

- [Architecture](docs/ARCHITECTURE.md) — décisions structurantes, feuille de route,
  et le *pourquoi* de chaque choix.

## Contribuer

Deux règles non négociables, détaillées dans
[§15 de l'architecture](docs/ARCHITECTURE.md#15-contrat-de-contribution) :

1. **Toute fonctionnalité arrive avec ses tests unitaires.**
2. **Toute modification met la documentation à jour dans le même commit.**

## État

En cours de démarrage — voir la [feuille de route](docs/ARCHITECTURE.md#12-feuille-de-route).
