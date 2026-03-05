# Kova (Linux)

Fork Linux du terminal [Kova](https://github.com/ddaydd/kova) (originalement macOS avec Metal + AppKit + CoreText), porté vers **winit + wgpu + FreeType + fontconfig**.

## Stack

- **Rust** (edition 2024) — langage unique
- **wgpu** — rendu GPU (Vulkan/OpenGL)
- **winit** — fenêtre et events (X11/Wayland)
- **FreeType + fontconfig** — rasterisation et découverte de polices
- **`vte`** — parsing séquences VT/ANSI
- **arboard** — clipboard X11/Wayland

## Architecture

- Un arbre binaire de splits par tab
- Un PTY par terminal pane
- Atlas de glyphes sur GPU

## Prérequis

```bash
# Ubuntu / Debian
sudo apt install build-essential pkg-config libfreetype-dev libfontconfig1-dev libxkbcommon-dev
```

## Build & Run

```bash
cd kova-ubuntu

cargo build --release        # binaire → target/release/kova
cargo run --release          # build + run

# Avec logs debug
RUST_LOG=info cargo run --release
```

## Installation

```bash
cargo build --release
sudo cp target/release/kova /usr/local/bin/
```

### Desktop integration

```bash
kova --install              # menu d'apps + "Ouvrir avec" pour les dossiers
kova --install --autostart  # idem + lancement au login
kova --uninstall            # supprime l'intégration
```

Code dans `install.rs`.

## Config

`~/.config/kova/config.toml` — voir `config.rs` pour les options disponibles.

## Logs

`~/.local/share/kova/kova.log` (level DEBUG par défaut, configurable via `RUST_LOG`).

## Raccourcis clavier

Voir `keyboard-shortcuts.md`. La touche **Super** (Win) remplace **Cmd** sur macOS.

## Pièges récurrents

- **Bytes vs chars** — Les cellules du terminal sont indexées par colonne (1 Cell = 1 char), mais les `String` Rust sont indexées par byte. Ne JAMAIS faire `&text[i..i+n]` sur du texte issu des cellules (contient des emoji, box-drawing, etc.). Toujours travailler avec `Vec<char>` ou itérateurs de chars.

## Principes

- Performance et RAM minimale avant tout
- Pas de feature creep : tabs, splits, config, c'est tout
