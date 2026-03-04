use winit::event::Modifiers;
use winit::keyboard::{Key, NamedKey};

use crate::keybindings::{KeyCombo, Keybindings, TerminalAction};
use crate::terminal::pty::Pty;

/// Write raw UTF-8 text to PTY.
pub fn write_text(text: &str, pty: &Pty) {
    if !text.is_empty() {
        pty.write(text.as_bytes());
    }
}

pub fn handle_key_event(
    key: &Key,
    modifiers: &Modifiers,
    pty: &Pty,
    cursor_keys_app: bool,
    keybindings: &Keybindings,
) {
    let combo = KeyCombo::from_winit(key, modifiers);
    let state = modifiers.state();

    // Check configurable terminal keybindings first
    if let Some(action) = keybindings.terminal_map.get(&combo) {
        match action {
            TerminalAction::KillLine => pty.write(b"\x15"),
            TerminalAction::Home => pty.write(b"\x1b[H"),
            TerminalAction::End => pty.write(b"\x1b[F"),
            TerminalAction::WordBack => pty.write(b"\x1bb"),
            TerminalAction::WordForward => pty.write(b"\x1bf"),
            TerminalAction::ShiftEnter => pty.write(b"\x1b[13;2u"),
        }
        return;
    }

    let has_ctrl = state.control_key();
    let has_alt = state.alt_key();
    let has_cmd = state.super_key();

    // Super key with no matching terminal binding — ignore
    if has_cmd {
        return;
    }

    // Ctrl + letter → control byte
    if has_ctrl {
        if let Key::Character(s) = key {
            let c = s.chars().next().unwrap_or('\0');
            if c.is_ascii_alphabetic() {
                let ctrl_byte = (c.to_ascii_lowercase() as u8) - b'a' + 1;
                pty.write(&[ctrl_byte]);
                return;
            }
            match c {
                '[' | '\\' | ']' | '^' | '_' => {
                    let ctrl_byte = (c as u8) - b'@';
                    pty.write(&[ctrl_byte]);
                    return;
                }
                _ => {}
            }
        }
    }

    // Special keys (arrows, function keys, etc.)
    match key {
        Key::Named(named) => {
            match named {
                NamedKey::ArrowUp => { pty.write(if cursor_keys_app { b"\x1bOA" } else { b"\x1b[A" }); return; }
                NamedKey::ArrowDown => { pty.write(if cursor_keys_app { b"\x1bOB" } else { b"\x1b[B" }); return; }
                NamedKey::ArrowLeft => { pty.write(if cursor_keys_app { b"\x1bOD" } else { b"\x1b[D" }); return; }
                NamedKey::ArrowRight => { pty.write(if cursor_keys_app { b"\x1bOC" } else { b"\x1b[C" }); return; }
                NamedKey::Insert => { pty.write(b"\x1b[2~"); return; }
                NamedKey::Delete => { pty.write(b"\x1b[3~"); return; }
                NamedKey::Home => { pty.write(b"\x1b[H"); return; }
                NamedKey::End => { pty.write(b"\x1b[F"); return; }
                NamedKey::PageUp => { pty.write(b"\x1b[5~"); return; }
                NamedKey::PageDown => { pty.write(b"\x1b[6~"); return; }
                NamedKey::Backspace => { pty.write(b"\x7f"); return; }
                NamedKey::Enter => { pty.write(b"\r"); return; }
                NamedKey::Tab => {
                    if state.shift_key() {
                        pty.write(b"\x1b[Z");
                    } else {
                        pty.write(b"\t");
                    }
                    return;
                }
                NamedKey::Escape => { pty.write(b"\x1b"); return; }
                NamedKey::F1 => { pty.write(b"\x1bOP"); return; }
                NamedKey::F2 => { pty.write(b"\x1bOQ"); return; }
                NamedKey::F3 => { pty.write(b"\x1bOR"); return; }
                NamedKey::F4 => { pty.write(b"\x1bOS"); return; }
                _ => {}
            }
        }
        _ => {}
    }

    // Alt + character → ESC prefix
    if has_alt {
        if let Key::Character(s) = key {
            let mut bytes = vec![0x1b];
            bytes.extend_from_slice(s.as_bytes());
            pty.write(&bytes);
            return;
        }
    }

    // Regular character input
    if let Key::Character(s) = key {
        pty.write(s.as_bytes());
    }
}
