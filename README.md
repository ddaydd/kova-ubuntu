# Kova (Linux)

> Fork Linux de [Kova](https://github.com/ddaydd/kova), originalement un terminal macOS (Metal + AppKit + CoreText), porté vers **winit + wgpu + FreeType + fontconfig**.

A blazing-fast terminal built from scratch with Rust and GPU rendering. No Electron, no cross-platform compromises — just native GPU rendering on Linux.

## Features

### GPU-rendered with wgpu

Every frame is drawn on the GPU via wgpu (Vulkan/OpenGL). Glyph atlas with on-demand rasterization via FreeType. Dirty-flag rendering — the GPU only redraws when terminal state actually changes. Synchronized output (mode 2026) eliminates tearing during fast updates.

### Splits and tabs

- Binary tree splits — horizontal and vertical, nested arbitrarily
- Drag-to-resize separators or use keyboard shortcuts (Super+Ctrl+Arrows)
- Auto-equalize: splits rebalance to equal sizes when adding/removing panes
- Horizontal scroll — when splits exceed the screen width, horizontal scroll navigates the virtual viewport. Configurable minimum split width.
- Tabs with colored tab bar, drag-to-reorder, and rename (Super+Shift+R)
- Cross-tab split navigation (Super+Alt+Arrows)
- Swap panes between splits (Super+Shift+Arrows)
- New splits and tabs inherit the CWD of the focused pane

### Session persistence

Layout (tabs, splits, CWD) is saved on quit and restored on launch. Window position is remembered automatically.

### Clickable URLs

Super+hover highlights URLs with an underline and pointer cursor. Super+click opens them in your browser. The hovered URL is shown in the status bar.

### Scrollback search

Super+F opens an inline search overlay with match highlighting. Click a match to jump to it.

### Status bar

Displays CWD, git branch (auto-polling every ~2s), scroll position indicator, and time. Each element's color is independently configurable.

### Wide characters

Full support for emoji and CJK characters with proper 2-column rendering.

### Input shortcuts

| Shortcut | Action |
|---|---|
| Alt+Left/Right | Word jump |
| Super+Left/Right | Beginning/end of line |
| Super+Backspace | Kill line |
| Shift+Enter | Newline without executing |

### Configuration

TOML config at `~/.config/kova/config.toml`. All settings have sensible defaults — the file is entirely optional.

```toml
[font]
family = "Hack"
size = 13.0

[colors]
foreground = [1.0, 1.0, 1.0]
background = [0.1, 0.1, 0.12]
cursor = [0.8, 0.8, 0.8]

[terminal]
scrollback = 10000
fps = 60

[status_bar]
branch_color = [0.4, 0.7, 0.5]

[tab_bar]
active_bg = [0.22, 0.22, 0.26]

[splits]
min_width = 300.0  # minimum pane width in points before horizontal scroll activates
```

### Keyboard shortcuts

| Shortcut | Action |
|---|---|
| Super+T | New tab |
| Super+W | Close pane/tab |
| Super+D | Vertical split (side by side) |
| Super+Shift+D | Horizontal split (stacked) |
| Super+E | Vertical split at root (full-height column) |
| Super+Shift+E | Horizontal split at root (full-width row) |
| Super+Shift+[ / ] | Previous/next tab |
| Super+1..9 | Jump to tab |
| Super+Alt+Arrows | Navigate between splits (cross-tab) |
| Super+Shift+Arrows | Swap pane with neighbor |
| Super+Ctrl+Arrows | Resize split |
| Super+Shift+R | Rename tab |
| Super+F | Search scrollback |
| Super+K | Clear scrollback and screen |
| Super+C | Copy selection |
| Super+V | Paste |

## Prerequisites

```bash
# Ubuntu / Debian
sudo apt install build-essential pkg-config libfreetype-dev libfontconfig1-dev libxkbcommon-dev
```

## Build

Requires Linux with Vulkan or OpenGL support and Rust (edition 2024).

```bash
cargo build --release
```

The binary lands in `target/release/kova`.

### Install

```bash
cargo build --release
sudo cp target/release/kova /usr/local/bin/
```

### Run

```bash
cargo run --release

# With debug logs
RUST_LOG=info cargo run --release
```

## Logs

`~/.local/share/kova/kova.log` (level DEBUG par défaut, configurable via `RUST_LOG`).

## Non-goals

- Plugin system
- Network multiplexing (ssh tunneling, etc.)
- Built-in AI (Claude runs *in* the terminal, not *as* the terminal)

## License

MIT
