# Spécifications du Projet : **Kymat** (anciennement *Omarchy-Sonora*)
> *Screensaver musical TUI/ASCII haute fréquence (120/144 FPS) en Rust pour Wayland/Hyprland, inspiré de l'esthétique Omarchy et du lecteur Sonora.*

---

## 1. Vision & Objectifs du Projet

L'objectif est de concevoir un économiseur d'écran (screensaver) contemplatif en **Rust** (`kymat`) capable de tourner dans un terminal Wayland (`foot`, `ghostty`, `kitty`, `alacritty`) avec une fluidité constante de **120 à 144 FPS** ($\Delta t \approx 6.94\text{ ms}$ par frame).

Le projet fusionne deux inspirations :
1. **L'esprit Omarchy (Arch + Hyprland)** : Intégration native au workflow clavier (`hypridle` / `omarchy-launch-screensaver`), esthétique ASCII/Pixel-art pure, animations organiques et consommation de ressources minimale.
2. **Le design de Sonora** : Une interface épurée sans boutons d'interaction, avec un fond lumineux dynamique (*Ambient Glow*) extrait de la pochette d'album, un visualiseur d'onde sonore fluide en bas d'écran, et une vue divisée automatique pour les paroles synchronisées (*Synced Lyrics*).

---

## 2. Analyse des Vues & Design Visuel (ASCII / Pixel-Art)

### 2.1. Fond Dynamique (*Ambient Mesh Gradient*)
L'arrière-plan n'est pas un noir uni, mais un dégradé diffus reprenant les 4 couleurs dominantes de la pochette d'album en cours :
* **Extraction** : Algorithme `color-thief` (Median Cut) au changement de piste, converti dans l'espace perceptuel **Oklab**.
* **Rendu Terminal Optimisé 144 FPS** :
  * Calcul d'un champ de couleurs par interpolation bilinéaire aux 4 coins appliqué à la couleur de fond ANSI 24-bit (`TrueColor`) de chaque cellule, avec un léger tramage spatial déterministe (matrice de Bayer $4 \times 4$) pour éviter tout *color banding*.
  * **Cache statique hors transition** : Pour éviter d'invalider les ~15 000 à 25 000 cellules du terminal à chaque frame (ce qui saturerait le PTY avec ~100 Mo/s de séquences ANSI), le fond reste **statique** pendant la lecture et n'est ré-interpolé qu'à ~30 Hz durant les $1.5\text{ s}$ de transition lors d'un changement de titre. Ainsi, seules les cellules de l'onde Braille, de la barre de temps et des paroles sont diffées par `ratatui` à 144 FPS.

### 2.2. Vue 1 : Mode Immersion (Cover Centrale & Visualiseur)
Mode affiché par défaut lorsqu'aucune parole synchronisée n'est disponible pour le titre en cours :
* **Pochette d'album (Centrée)** :
  * Rendu en **Pixel-Art Unicode** via les caractères demi-blocs (`▀` et `▄`), permettant d'afficher 2 pixels verticaux par cellule de terminal (couleurs de premier plan et d'arrière-plan distinctes).
  * Masque d'arrondi dans les coins (simulation de `border-radius` en fondant les pixels des coins extrêmes vers la couleur exacte du `AmbientBackground` sous-jacent).
* **Métadonnées & Temps (Sous la cover, 100% passif)** :
  * **Titre** en gras, blanc éclatant (`#FFFFFF`).
  * **Artiste** centré juste en dessous, avec une couleur atténuée (`#8899A6`).
  * **Barre de progression passive** : Fine ligne temporelle `0:15 ━━━●─────────── 2:56` (aucun bouton d'interaction ni icône de contrôle média).
* **Visualiseur d'Onde (Bas de l'écran)** :
  * Onde fluide occupant toute la largeur de l'écran.
  * Crête de l'onde dessinée avec les caractères **Braille Unicode** (`⠀` à `⣿`, offrant une grille de $2 \times 4$ sous-pixels par cellule) pour une courbe ultra-lisse.
  * Remplissage sous la courbe avec un dégradé vertical d'opacité vers le bas de l'écran.

```text
+-----------------------------------------------------------------------------+
|                                                                             |
|                             ▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄▄                             |
|                            █░░░░░░░░░░░░░░░░░░░█                            |
|                            █░░░ PIXEL COVER ░░░█                            |
|                            █░░░░░ (RGB 2x) ░░░░█                            |
|                            █░░░░░░░░░░░░░░░░░░░█                            |
|                             ▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀                             |
|                            Better When I'm Dancin'                          |
|                                Meghan Trainor                               |
|                            0:15 ━━━●─────────── 2:56                        |
|      _.~"~._.~"~._                                                          |
|  _.-"             "~._.~"~._                       _.~"~._                  |
|░░░░░░░░░░░░░░░░░░░░░░░░░░░░░"~._               _.-"░░░░░░░░"~._.~"~._.~"~._ |
|▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒"~._.~"~._.~"~▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒ |
|▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓▓ |
+-----------------------------------------------------------------------------+
```

### 2.3. Vue 2 : Mode Paroles Synchronisées (Split View)
Activée automatiquement dès que des paroles synchronisées sont disponibles :
* **Colonne Gauche ($40\%$ de la largeur)** :
  * La pochette d'album, le titre, l'artiste et la barre de temps passive glissent vers la gauche (animation fluide avec fonction d'easing *Cubic Bezier*).
* **Colonne Droite ($60\%$ de la largeur)** :
  * Liste verticale des paroles synchronisées au temps $t$ de la chanson.
  * **Ligne active** : Centrée verticalement, affichée en blanc pur (`#FFFFFF`) et en gras (avec support du karaoké mot-à-mot progressif lorsque fourni par le cache de Sonora).
  * **Effet de Flou / Profondeur (Interpolation Oklab)** : Les lignes passées (au-dessus) et futures (en dessous) subissent une atténuation progressive de contraste. Plus une ligne est éloignée du centre, plus sa couleur est interpolée dans l'espace **Oklab** vers la couleur du fond ambiant sous-jacent, imitant le flou gaussien optique de Sonora.

---

## 3. Architecture Technique & Pipeline 144 FPS

L'application est découpée en workers asynchrones / threads dédiés communiquant avec la boucle de rendu par canaux non-bloquants (`tokio::sync::watch` / `crossbeam-channel`).

### 3.1. Schéma d'Architecture

```text
+-------------------+       +-------------------+       +-------------------+
|   Thread MPRIS    |       |   Thread Audio    |       |   Worker Lyrics   |
|  (zbus / D-Bus)   |       | (Pulse Monitor +  |       | (Sonora SQLite +  |
|                   |       |     RustFFT)      |       |  Fallback LRCLIB) |
+---------+---------+       +---------+---------+       +---------+---------+
          |                           |                           |
          v (Metadata/Time)           v (Spectre 48 bandes)       v (SyncedLyrics)
+---------------------------------------------------------------------------+
|                        ÉTAT PARTAGÉ (Lock-free / Watch)                   |
+-------------------------------------+-------------------------------------+
                                      |
                                      v
+---------------------------------------------------------------------------+
|                     BOUCLE DE RENDU PRINCIPALE (144 FPS)                  |
|  1. Interpolation physique (Attack/Decay + Catmull-Rom) du spectre audio  |
|  2. Rendu du Mesh Gradient Oklab (mis en cache hors transitions)          |
|  3. Dessin des widgets (Cover ▀/▄, Temps passif, Wave Braille, Lyrics)    |
|  4. Diffing Cellule-par-Cellule & Flush ANSI atomique (Synchronized Out)  |
+---------------------------------------------------------------------------+
```

### 3.2. Le Secret du 144 FPS pour l'Audio (Interpolation Temporelle)
Les serveurs audio envoient des buffers à une fréquence inférieure ou désynchronisée de l'écran (~43 Hz à 86 Hz selon la taille du buffer). Pour chaque bande de fréquence $i$, la hauteur affichée $y_i(t)$ à la frame $t$ est calculée via un filtre de lissage exponentiel asymétrique indépendant du framerate :

$$y_i(t) = y_i(t - \Delta t) + \left( \text{target}_i - y_i(t - \Delta t) \right) \cdot \left( 1 - e^{-\lambda \Delta t} \right)$$

Où :
* $\text{target}_i$ est la dernière valeur FFT reçue du thread audio.
* $\Delta t \approx 0.00694\text{ s}$ ($1 / 144\text{ s}$).
* $\lambda_{\text{attack}} = 35.0$ (montée rapide sur les percussions/basses) et $\lambda_{\text{decay}} = 12.0$ (redescente fluide).

Les points discrets $y_i$ sont ensuite reliés spatialement par une **spline de Catmull-Rom** évaluée pour chaque sous-colonne Braille ($2 \times W_{\text{term}}$).

---

## 4. Stack Technique (Crates Rust)

| Module | Crate Rust | Rôle et Justification |
| :--- | :--- | :--- |
| **Moteur TUI & Rendu** | `ratatui` + `crossterm` | Double-buffering avec diffing cellule-par-cellule et *Synchronized Output* (`CSI ? 2026 h/l`) évitant tout tearing à 144 FPS. |
| **Espace Colorimétrique** | `palette` | Interpolation fluide des couleurs dans l'espace perceptuel **Oklab** (transitions de fond et effet de profondeur sur les paroles). |
| **Communication MPRIS** | `zbus` | Écoute événementielle asynchrone D-Bus (`PropertiesChanged`, `Seeked`) sans polling, avec priorité à `org.mpris.MediaPlayer2.sonora`. |
| **Capture Audio** | `libpulse-simple-binding` | Capture directe et sans configuration du flux `@DEFAULT_MONITOR@` via `pipewire-pulse` en `FLOAT32LE`. |
| **Analyse Spectrale** | `rustfft` | Calcul de la Transformée de Fourier Rapide (FFT) avec fenêtrage de Hann et regroupement logarithmique en bandes d'octaves. |
| **Traitement d'Image** | `image` + `color-thief` | Chargement local (`file://`) ou HTTP de la cover, redimensionnement Lanczos3, demi-blocs Unicode et extraction des couleurs dominantes. |
| **Paroles Synchronisées** | `rusqlite` + `reqwest` + `serde` | Lecture prioritaire en `READ_ONLY` du cache SQLite local de Sonora (`~/.cache/sonora/cache.sqlite`), puis fallback vers l'API **LRCLIB** (`https://lrclib.net/api/get`). |

---

## 5. Fonctionnalités Détaillées

### 5.1. Module MPRIS & Synchronisation Temporelle
* Détection automatique du lecteur actif (priorité à `sonora` ou au lecteur ayant le statut `Playing`).
* Écoute des signaux D-Bus `PropertiesChanged` et `Seeked`.
* **Horloge interne haute précision** : Maintien d'un `Instant::now()` local recalé sur la dernière position MPRIS connue pour faire avancer la barre de temps passive et les paroles à 144 Hz :
  $$t_{\text{current}} = t_{\text{mpris\_last}} + (t_{\text{now}} - t_{\text{sync}}) \times \text{PlaybackRate}$$

### 5.2. Module Lyrics Hybride (Cache SQLite Sonora + Fallback LRCLIB)
1. **Priorité 1 — Cache local Sonora (`~/.cache/sonora/cache.sqlite`)** :
   * Ouverture SQLite en mode `SQLITE_OPEN_READ_ONLY`.
   * Extraction de l'identifiant de piste depuis `xesam:url` (ex: `https://open.spotify.com/track/<id>` $\to$ clé `spotify:<id>:lyrics`).
   * Sélection du `hit` ayant le score `trust` maximal parmi Apple Music, Musixmatch, Spotify, YouTube Music et LrcLib, avec support des timestamps de lignes (`lines`) et de mots (`words`).
2. **Priorité 2 — Cache `.lrc` local & API LRCLIB** :
   * Si absent de Sonora, vérification dans `~/.cache/kymat/lyrics/` puis requête GET vers `https://lrclib.net/api/get`.
3. **Bascule de vue automatique** :
   * Si des paroles synchronisées sont disponibles $\to$ transition fluide ($600\text{ ms}$) vers la **Vue 2 (Split View)**.
   * Sinon $\to$ maintien de la **Vue 1 (Mode Immersion centré)**.

### 5.3. Mode Screensaver & Intégration Omarchy (Hyprland)
* **Comportement Screensaver (`--screensaver`)** :
  * Masquage automatique du curseur terminal et attente du redimensionnement initial du PTY plein écran.
  * **100% passif** : Toute pression de touche ou tout clic/mouvement de souris ferme proprement l'application avec un code de sortie `0` pour rendre la main instantanément.
* **Fallback "Idle Omarchy"** :
  * Si aucune musique n'est en cours de lecture (`Stopped` ou en pause prolongée), affichage d'une onde sinusoïdale lente au repos et d'une horloge minimale au centre jusqu'à la reprise de la lecture.

---

## 6. Architecture des Fichiers du Projet

```text
kymat/
├── Cargo.toml
├── config.toml.example         # Configuration (FPS cible, sensibilité audio, etc.)
└── src/
    ├── main.rs                 # Point d'entrée, initialisation CLI et boucle 144Hz
    ├── app.rs                  # Machine à états (Vue Centrale, Vue Lyrics, Mode Idle)
    ├── config.rs               # Parsing de ~/.config/kymat/config.toml & CLI
    ├── audio/
    │   ├── mod.rs
    │   ├── capture.rs          # Capture @DEFAULT_MONITOR@ via libpulse-simple
    │   ├── fft.rs              # Fenêtre de Hann, FFT, regroupement fréquentiel
    │   └── smoother.rs         # Interpolation temporelle (144 FPS) et Spline Catmull-Rom
    ├── media/
    │   ├── mod.rs
    │   ├── mpris_watcher.rs    # Client D-Bus asynchrone (métadonnées & position)
    │   ├── cover.rs            # Chargement file:///http, extraction palette Oklab & Pixel-Art
    │   └── lyrics.rs           # Lecteur SQLite Sonora + fallback API LRCLIB & parser LRC
    └── ui/
        ├── mod.rs
        ├── background.rs       # Rendu du Mesh Gradient ambiant Oklab avec cache statique
        ├── cover_widget.rs     # Rendu de la pochette en demi-blocs Unicode (▀/▄) + titre/temps passif
        ├── wave_widget.rs      # Rendu de l'onde sonore en caractères Braille (⠀-⣿) + dégradé
        ├── lyrics_widget.rs    # Rendu des paroles avec profondeur Oklab et défilement fluide
        └── transitions.rs      # Animations Cubic Bezier (Vue 1 <-> Vue 2)
```

---

## 7. Intégration dans Omarchy / Hyprland 0.56+

Compatible nativement avec `/usr/share/omarchy/bin/omarchy-launch-screensaver` (classe `org.omarchy.screensaver`) ou lançable directement avec `foot` / `alacritty` :

```bash
foot --app-id=org.omarchy.screensaver --fullscreen -e kymat --screensaver --fps 144
```
