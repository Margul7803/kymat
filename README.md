# Kymat — Screensaver Musical ASCII/TUI 144 FPS (Omarchy × Sonora)

Économiseur d'écran musical haute fréquence (**144 FPS**) écrit en Rust pour **Wayland / Hyprland** (Omarchy), inspiré de l'esthétique plein écran de **Sonora**.

## Fonctionnalités

- **Moteur TUI 144 FPS sans tearing** (`ratatui` + `crossterm` avec *Synchronized Output* `CSI ? 2026 h/l` et 0 allocation mémoire par frame).
- **Fond dynamique Oklab** : extraction chromatique de la pochette (`color-thief`), fond quasi-noir (`#050508`) et double halo radial gaussien interpolé dans l'espace perceptuel Oklab.
- **Pochette Pixel-Art Quadrants Unicode $2 \times 2$** (`▘▝▀▖▌▞▛█`) avec coins arrondis SDF et cœur plein Nerd Font (`\u{f004}`).
- **Double vue dynamique (`Tab`)** :
  - **Vue 1 (Mode Sans Paroles)** : pochette centrée + double onde sonore Braille ($2 \times 4$ sous-pixels, interpolation spline de Catmull-Rom, Auto-Gain Control PipeWire/PulseAudio `@DEFAULT_MONITOR@`).
  - **Vue 2 (Mode Paroles Synchronisées)** : split-screen avec typographie sous-pixel $5 \times 6$ (3x plus grande que le titre) et **vrai flou gaussien 2D** sur les lignes passées et futures.
- **Compatibilité universelle MPRIS & Paroles** :
  - Compatible **Sonora**, **Spotify**, **Deezer**, **YouTube**, **YouTube Music**, navigateurs (Brave/Chrome/Firefox), VLC, mpv.
  - Lecture prioritaire dans `~/.cache/sonora/cache.sqlite` (`READ_ONLY`) + fallback universel **LRCLIB** (`lrclib.net`) avec nettoyage automatique des titres YouTube.

## Compilation & Installation

```bash
cargo build --release
install -Dm755 target/release/kymat ~/.local/bin/kymat
install -Dm644 foot-kymat.ini ~/.config/kymat/foot-kymat.ini
```

## Utilisation

- **Mode interactif** (`Tab` pour switcher Paroles / Onde sonore, `q` ou `Esc` pour quitter) :
  ```bash
  kymat --fps 144
  ```
- **Mode screensaver** (`Tab` pour switcher de vue, toute autre touche ou mouvement souris quitte immédiatement) :
  ```bash
  kymat --screensaver --fps 144
  ```

## Intégration Omarchy (Double Écran : Omarchy sur l'écran principal + Kymat sur le second)

Les scripts dans [`scripts/`](scripts/) permettent de surcharger proprement `omarchy-launch-screensaver` dans `~/.local/bin/` sans toucher à `/usr/share/omarchy/` :

```bash
install -Dm755 scripts/omarchy-launch-screensaver ~/.local/bin/omarchy-launch-screensaver
install -Dm755 scripts/kymat-screensaver-runner ~/.local/bin/kymat-screensaver-runner
```
