# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.0] - 2026-11-13

### Added
- 144 FPS TrueColor TUI/ASCII musical screensaver for Wayland & Hyprland (Omarchy).
- Synced Lyrics view with Oklab ambient cover palette, Gaussian blur on past/upcoming lines, and word-level timing support.
- Soundwave view with live FFT spectrum and double-crest Braille audio waveform via PipeWire/PulseAudio monitor capture.
- Automatic MPRIS player detection (Sonora, Spotify, Deezer, browsers, VLC, mpv) and lyrics resolution (Sonora local cache + `lrclib.net`).
- Multi-script lyrics & title support (Latin, Extended Latin, Cyrillic, Greek, and system fontconfig CJK/Hangul fallback).
- Dancing rat easter egg (`r` toggle or optional climax state machine).
- CLI subcommands (`kymat config edit|path|show`) and XDG desktop entry.
- Dual-monitor Omarchy screensaver integration scripts.
