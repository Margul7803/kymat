# Kymat

[![CI](https://github.com/Margul7803/kymat/actions/workflows/ci.yml/badge.svg)](https://github.com/Margul7803/kymat/actions/workflows/ci.yml)
[![License: GPL-3.0-or-later](https://img.shields.io/badge/License-GPL--3.0--or--later-blue.svg)](LICENSE)

A smooth **144 FPS ASCII / terminal music screensaver** for **Wayland & Hyprland** ([Omarchy](https://omarchy.org/)), inspired by [Sonora](https://github.com/sonora)'s fullscreen player aesthetic.

Kymat reacts in real time to whatever music is playing on your desktop — displaying pixel-art album covers, ambient colors extracted from the artwork, synchronized lyrics with depth-of-field blur, and a live audio soundwave.

---

## Preview

### Synced Lyrics View
![Kymat Synced Lyrics View](assets/lyrics-view.png)

### Soundwave View
![Kymat Soundwave View](assets/wave-view.png)

---

## Highlights

- **Two Visual Modes (`Tab`)**
  - **Lyrics View**: Split-screen layout with large pixel-art lyrics (Latin, Extended Latin, Cyrillic, Greek, plus Japanese/Korean/Chinese via system CJK font fallback) and smooth Gaussian blur on past and upcoming lines.
  - **Soundwave View**: Centered album artwork over a live, double-crest audio wave that reacts to your system audio.
- **Works with Any Music Player**
  - Automatically detects **Sonora**, **Spotify**, **Deezer**, **YouTube Music**, **YouTube** (Brave, Chromium, Firefox), **VLC**, **mpv**, and any MPRIS-enabled player.
  - Fetches synced lyrics from Sonora's local cache or automatically via **LRCLIB** (`lrclib.net`).
- **Adaptive Ambient Theme**
  - Extracts dominant colors from the current album cover to tint the dark radial background glow, heart icon, progress bar, and soundwave.
- **Built for Omarchy & Dual-Monitor Setups**
  - Can run standalone in any terminal or automatically launch on a secondary monitor when Omarchy enters screensaver mode (keeping the classic Omarchy ASCII screensaver on your primary screen).

---

## Dependencies

### Build Dependencies
- Rust toolchain (`cargo` / `rustc`, stable)
- `libpulse` (PulseAudio/PipeWire client headers)
- `dbus`
- `pkg-config`

### Runtime Dependencies
- **Required**: `libpulse` (or `pipewire-pulse`), `dbus`, `fontconfig`
- **Recommended**:
  - `ttf-jetbrains-mono-nerd` (terminal font & UI icons)
  - `noto-fonts-cjk` (Japanese/Korean/Chinese lyrics and titles)
  - `jq`, `socat`, `hyprland` (for Omarchy dual-monitor screensaver scripts)

---

## Installation

### Arch Linux (AUR)

```bash
yay -S kymat
# or
paru -S kymat
```

### From Source (Arch Linux / Omarchy)

```bash
git clone https://github.com/Margul7803/kymat.git
cd kymat
cargo build --release --locked

install -Dm755 target/release/kymat ~/.local/bin/kymat
install -Dm644 foot-kymat.ini ~/.config/kymat/foot-kymat.ini
install -Dm644 config.toml.example ~/.config/kymat/config.toml
install -Dm644 assets/kymat.desktop ~/.local/share/applications/kymat.desktop
```

> **Optional CJK Lyrics & Titles Support (Japanese, Korean, Chinese):**  
> Install `noto-fonts-cjk` (e.g. `sudo pacman -S noto-fonts-cjk` on Arch/Omarchy) so both pixel-art lyrics and terminal titles render Kana, Kanji, Hangul, and Hanzi cleanly.

---

## Usage

Run `kymat` in any TrueColor terminal (`foot`, `ghostty`, `alacritty`, `kitty`):

```bash
kymat
```

### Controls

| Key | Action |
| --- | --- |
| `Tab` | Switch between **Lyrics View** and **Soundwave View** |
| `r` | Toggle the **Dancing Rat Easter Egg** in the center of the screen |
| `q` / `Esc` | Quit (in normal mode) |
| Any key / Mouse move | Exit screensaver (when launched with `--screensaver`) |

---

## Omarchy Dual-Monitor Screensaver Setup

If you use **Omarchy** with a second monitor connected, you can display the classic Omarchy screensaver on your primary display and **Kymat** on your secondary display.

If installed from **AUR**, link the helper launcher into `~/.local/bin/` (which takes precedence over `/usr/share/omarchy/bin/` without conflicting with Omarchy's system files):

```bash
ln -sf /usr/share/kymat/scripts/omarchy-launch-screensaver ~/.local/bin/omarchy-launch-screensaver
```

If installed **from source**:

```bash
install -Dm755 scripts/omarchy-launch-screensaver ~/.local/bin/omarchy-launch-screensaver
install -Dm755 scripts/kymat-screensaver-runner ~/.local/bin/kymat-screensaver-runner
```

Both screensavers wake up and close together as soon as you move the mouse or press a key on either screen.

---

## Configuration

Settings can be managed directly via the CLI or edited in `~/.config/kymat/config.toml`:

```bash
# Open configuration in your default editor ($VISUAL, $EDITOR, or nano/micro/vim)
kymat config edit

# Display the absolute path to your config file
kymat config path

# View current configuration contents
kymat config show
```

### Config File Options (`~/.config/kymat/config.toml`)

```toml
fps = 144
font_size = 12
default_mode = "lyrics" # "lyrics" or "soundwave"
rat_easter_egg = false
audio_sensitivity = 1.15
attack_lambda = 35.0
decay_lambda = 12.0
background_transition_secs = 1.5
layout_transition_secs = 0.6
```

---

## Uninstall

### AUR
```bash
yay -Rns kymat
rm -f ~/.local/bin/omarchy-launch-screensaver
rm -rf ~/.config/kymat
```

### Source Install
```bash
rm -f ~/.local/bin/kymat \
      ~/.local/bin/kymat-screensaver-runner \
      ~/.local/bin/omarchy-launch-screensaver \
      ~/.local/share/applications/kymat.desktop
rm -rf ~/.config/kymat
```

---

## License

[GPL-3.0-or-later](LICENSE)
