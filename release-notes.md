# Kova Linux — Notes de version

## v1.0.0 — Port Linux (2026-03-04)

Fork du terminal macOS [Kova](https://github.com/ddaydd/kova) porté vers Linux.

### Changements majeurs

**Remplacement de la stack macOS par des équivalents Linux :**

| macOS | Linux |
|---|---|
| Metal | wgpu (Vulkan/OpenGL) |
| AppKit (NSWindow, NSView) | winit (X11/Wayland) |
| CoreText | FreeType + fontconfig |
| NSPasteboard | arboard (X11/Wayland clipboard) |
| objc2-* | supprimé |

### Fichiers réécrits

- `main.rs` — Event loop winit au lieu de NSApplication
- `app.rs` — `ApplicationHandler` winit au lieu de `NSApplicationDelegate`
- `window.rs` — Gestion fenêtre, events clavier/souris, tabs, splits via winit
- `input.rs` — Mapping `KeyEvent` winit → séquences PTY (au lieu de `NSEvent`)
- `keybindings.rs` — `KeyCombo::from_winit()` au lieu de `from_event(&NSEvent)`
- `renderer/mod.rs` — Pipeline wgpu au lieu de Metal (vertex buffers, render pass, texture atlas)
- `renderer/glyph_atlas.rs` — Rasterisation FreeType + fontconfig au lieu de CoreText
- `renderer/pipeline.rs` — Création pipeline wgpu
- `shaders/terminal.wgsl` — Shader WGSL au lieu de Metal Shading Language

### Fichiers conservés (Rust pur, pas de changement)

- `pane.rs` — Tab, Pane, SplitTree
- `terminal/mod.rs` — TerminalState
- `terminal/parser.rs` — VteHandler
- `session.rs` — Session JSON
- `config.rs` — Config TOML (font par défaut changée de "Hack" à "monospace")

### Fichiers modifiés légèrement

- `terminal/pty.rs` — `/proc/{pid}/cwd` au lieu de `proc_pidinfo`, `/proc/{pid}/comm` au lieu de `proc_name()`, shell par défaut `$SHELL` avec fallback `/bin/bash`

### Rendu texte

- **LCD subpixel rendering** via FreeType `TARGET_LCD` pour un texte net
- **LCD filter** (`LcdFilterDefault`) pour éviter les artefacts de couleur sur les bords des glyphes
- **Clamping atlas** pour empêcher le débordement de glyphes entre les lignes de l'atlas
- **Fallback font** intelligent : vérifie que fontconfig retourne bien la police demandée, sinon fallback sur "monospace"
- **Emoji couleur** supportés via `FT_LOAD_COLOR` (BGRA)
- **Box-drawing characters** rendus par code (pas de police nécessaire)

### Raccourcis clavier

- **Super** (touche Win) remplace **Cmd**
- **Alt** remplace **Option**
- Tous les raccourcis sont configurables via `~/.config/kova/config.toml`

### Prérequis système

```bash
sudo apt install build-essential pkg-config libfreetype-dev libfontconfig1-dev libxkbcommon-dev
```

### Build

```bash
cargo build --release
cargo run --release
```
