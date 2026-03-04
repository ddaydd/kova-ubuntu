use std::collections::HashMap;
use winit::event::{ElementState, Modifiers};
use winit::keyboard::{Key, NamedKey, SmolStr};

use crate::pane::{NavDirection, SplitAxis};

/// A hashable key combination (modifiers + key).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KeyCombo {
    pub cmd: bool,   // Super/Win key on Linux
    pub ctrl: bool,
    pub option: bool, // Alt key on Linux
    pub shift: bool,
    pub key: KeyType,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum KeyType {
    Char(char),
    Up,
    Down,
    Left,
    Right,
    Backspace,
    Enter,
}

/// Window/tab/split actions dispatched from key events.
#[derive(Debug, Clone)]
pub enum Action {
    NewTab,
    ClosePaneOrTab,
    VSplit,
    HSplit,
    VSplitRoot,
    HSplitRoot,
    NewWindow,
    CloseWindow,
    KillWindow,
    Copy,
    Paste,
    ToggleFilter,
    ClearScrollback,
    PrevTab,
    NextTab,
    RenameTab,
    RenamePane,
    DetachTab,
    MergeWindow,
    SwitchTab(usize),
    Navigate(NavDirection),
    SwapPane(NavDirection),
    Resize(SplitAxis, f32),
}

/// Terminal-level actions dispatched from handle_key_event.
#[derive(Debug, Clone)]
pub enum TerminalAction {
    KillLine,
    Home,
    End,
    WordBack,
    WordForward,
    ShiftEnter,
}

pub struct Keybindings {
    pub window_map: HashMap<KeyCombo, Action>,
    pub terminal_map: HashMap<KeyCombo, TerminalAction>,
}

impl KeyCombo {
    pub fn from_winit(key: &Key, modifiers: &Modifiers) -> Self {
        let state = modifiers.state();
        let cmd = state.super_key();
        let ctrl = state.control_key();
        let option = state.alt_key();
        let shift = state.shift_key();

        let key_type = match key {
            Key::Named(NamedKey::ArrowUp) => KeyType::Up,
            Key::Named(NamedKey::ArrowDown) => KeyType::Down,
            Key::Named(NamedKey::ArrowLeft) => KeyType::Left,
            Key::Named(NamedKey::ArrowRight) => KeyType::Right,
            Key::Named(NamedKey::Backspace) => KeyType::Backspace,
            Key::Named(NamedKey::Enter) => KeyType::Enter,
            Key::Character(s) => {
                let c = s.chars().next().unwrap_or('\0').to_ascii_lowercase();
                KeyType::Char(c)
            }
            _ => KeyType::Char('\0'),
        };

        KeyCombo {
            cmd,
            ctrl,
            option,
            shift,
            key: key_type,
        }
    }
}

/// Parse a string like "cmd+shift+d" into a KeyCombo.
fn parse_key_combo(s: &str) -> KeyCombo {
    let mut combo = KeyCombo {
        cmd: false,
        ctrl: false,
        option: false,
        shift: false,
        key: KeyType::Char('\0'),
    };

    let num_parts = s.split('+').count();
    for (i, part) in s.split('+').enumerate() {
        let trimmed = part.trim();
        if i < num_parts - 1 {
            if trimmed.eq_ignore_ascii_case("cmd") || trimmed.eq_ignore_ascii_case("command") {
                combo.cmd = true;
            } else if trimmed.eq_ignore_ascii_case("ctrl") || trimmed.eq_ignore_ascii_case("control") {
                combo.ctrl = true;
            } else if trimmed.eq_ignore_ascii_case("option") || trimmed.eq_ignore_ascii_case("alt") || trimmed.eq_ignore_ascii_case("opt") {
                combo.option = true;
            } else if trimmed.eq_ignore_ascii_case("shift") {
                combo.shift = true;
            } else {
                log::warn!("Unknown modifier in keybinding: {}", trimmed);
            }
        } else {
            let lower = trimmed.to_ascii_lowercase();
            combo.key = match lower.as_str() {
                "up" => KeyType::Up,
                "down" => KeyType::Down,
                "left" => KeyType::Left,
                "right" => KeyType::Right,
                "backspace" | "delete" => KeyType::Backspace,
                "enter" | "return" => KeyType::Enter,
                "[" => KeyType::Char('['),
                "]" => KeyType::Char(']'),
                s if s.len() == 1 => KeyType::Char(s.chars().next().unwrap()),
                _ => {
                    log::warn!("Unknown key in keybinding: {}", trimmed);
                    KeyType::Char('\0')
                }
            };
        }
    }

    combo
}

use crate::config::KeysConfig;

impl Keybindings {
    pub fn from_config(keys: &KeysConfig) -> Self {
        let mut window_map = HashMap::new();
        let mut terminal_map = HashMap::new();

        let mut bind = |s: &str, action: Action| {
            let combo = parse_key_combo(s);
            window_map.insert(combo, action);
        };

        bind(&keys.new_tab, Action::NewTab);
        bind(&keys.close_pane_or_tab, Action::ClosePaneOrTab);
        bind(&keys.vsplit, Action::VSplit);
        bind(&keys.hsplit, Action::HSplit);
        bind(&keys.vsplit_root, Action::VSplitRoot);
        bind(&keys.hsplit_root, Action::HSplitRoot);
        bind(&keys.new_window, Action::NewWindow);
        bind(&keys.close_window, Action::CloseWindow);
        bind(&keys.kill_window, Action::KillWindow);
        bind(&keys.copy, Action::Copy);
        bind(&keys.paste, Action::Paste);
        bind(&keys.toggle_filter, Action::ToggleFilter);
        bind(&keys.clear_scrollback, Action::ClearScrollback);
        bind(&keys.prev_tab, Action::PrevTab);
        bind(&keys.next_tab, Action::NextTab);
        bind(&keys.rename_tab, Action::RenameTab);
        bind(&keys.rename_pane, Action::RenamePane);
        bind(&keys.detach_tab, Action::DetachTab);
        bind(&keys.merge_window, Action::MergeWindow);

        for (i, s) in [
            &keys.switch_tab_1, &keys.switch_tab_2, &keys.switch_tab_3,
            &keys.switch_tab_4, &keys.switch_tab_5, &keys.switch_tab_6,
            &keys.switch_tab_7, &keys.switch_tab_8, &keys.switch_tab_9,
        ]
        .iter()
        .enumerate()
        {
            bind(s, Action::SwitchTab(i));
        }

        bind(&keys.navigate_up, Action::Navigate(NavDirection::Up));
        bind(&keys.navigate_down, Action::Navigate(NavDirection::Down));
        bind(&keys.navigate_left, Action::Navigate(NavDirection::Left));
        bind(&keys.navigate_right, Action::Navigate(NavDirection::Right));

        bind(&keys.swap_up, Action::SwapPane(NavDirection::Up));
        bind(&keys.swap_down, Action::SwapPane(NavDirection::Down));
        bind(&keys.swap_left, Action::SwapPane(NavDirection::Left));
        bind(&keys.swap_right, Action::SwapPane(NavDirection::Right));

        bind(&keys.resize_left, Action::Resize(SplitAxis::Horizontal, -0.05));
        bind(&keys.resize_right, Action::Resize(SplitAxis::Horizontal, 0.05));
        bind(&keys.resize_up, Action::Resize(SplitAxis::Vertical, -0.05));
        bind(&keys.resize_down, Action::Resize(SplitAxis::Vertical, 0.05));

        let term = &keys.terminal;
        let mut tbind = |s: &str, action: TerminalAction| {
            let combo = parse_key_combo(s);
            terminal_map.insert(combo, action);
        };

        tbind(&term.kill_line, TerminalAction::KillLine);
        tbind(&term.home, TerminalAction::Home);
        tbind(&term.end, TerminalAction::End);
        tbind(&term.word_back, TerminalAction::WordBack);
        tbind(&term.word_forward, TerminalAction::WordForward);
        tbind(&term.shift_enter, TerminalAction::ShiftEnter);

        Keybindings {
            window_map,
            terminal_map,
        }
    }
}
