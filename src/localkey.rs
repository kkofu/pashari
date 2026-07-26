//! Local key bindings (not registered with the OS — simply compared
//! on-the-spot against `KeyboardInput` received by the app's own window).
//!
//! A separate system from global hotkeys (`hotkey.rs`, based on
//! `global_hotkey::hotkey::Code`, for OS registration). Since the targets
//! here are only things expressible as "a single character + Ctrl/Shift/
//! Alt" — Undo/Redo, region selection's R/X/Q, the menu's S/C/E/U/V, the
//! editor's Ctrl+V/S/C/0 — this sticks to plain character comparison
//! rather than a physical-key system like `Code`/`KeyCode`.

use std::fmt;

/// A local key binding. `ch` is stored/compared lowercased.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct LocalKey {
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
    pub ch: char,
}

impl LocalKey {
    pub fn new(ctrl: bool, shift: bool, alt: bool, ch: char) -> Self {
        Self {
            ctrl,
            shift,
            alt,
            ch: ch.to_ascii_lowercase(),
        }
    }
}

impl fmt::Display for LocalKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.ctrl {
            write!(f, "Ctrl+")?;
        }
        if self.shift {
            write!(f, "Shift+")?;
        }
        if self.alt {
            write!(f, "Alt+")?;
        }
        write!(f, "{}", self.ch.to_ascii_uppercase())
    }
}

/// Converts a string like "Ctrl+Shift+Z" into a `LocalKey`. Modifier
/// tokens are case-insensitive. The main key token must be exactly one character (`None` otherwise).
pub fn parse(spec: &str) -> Option<LocalKey> {
    let mut ctrl = false;
    let mut shift = false;
    let mut alt = false;
    let mut ch: Option<char> = None;
    for tok in spec.split('+') {
        let t = tok.trim();
        match t.to_ascii_lowercase().as_str() {
            "" => {}
            "ctrl" | "control" => ctrl = true,
            "shift" => shift = true,
            "alt" | "option" => alt = true,
            other => {
                let mut it = other.chars();
                let c = it.next()?;
                if it.next().is_some() {
                    return None; // Multi-character tokens aren't supported.
                }
                if ch.is_some() {
                    return None; // More than one main key is invalid.
                }
                ch = Some(c);
            }
        }
    }
    Some(LocalKey::new(ctrl, shift, alt, ch?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_display_roundtrip() {
        for spec in ["R", "X", "Q", "Ctrl+Z", "Ctrl+Shift+Z", "Ctrl+0", "Alt+S"] {
            let k = parse(spec).unwrap_or_else(|| panic!("{spec} should parse"));
            assert_eq!(k.to_string(), spec);
        }
    }

    #[test]
    fn parse_is_case_insensitive_for_modifiers_and_key() {
        assert_eq!(parse("ctrl+z"), parse("Ctrl+Z"));
        assert_eq!(parse("CTRL+SHIFT+z"), parse("Ctrl+Shift+Z"));
    }

    #[test]
    fn parse_rejects_multi_char_or_missing_key() {
        assert_eq!(parse("Ctrl+"), None);
        assert_eq!(parse("Ctrl+Foo"), None);
        assert_eq!(parse(""), None);
        assert_eq!(parse("Ctrl+Z+X"), None);
    }

    #[test]
    fn equality_requires_exact_modifier_match() {
        let plain = LocalKey::new(false, false, false, 'r');
        let ctrl = LocalKey::new(true, false, false, 'r');
        assert_ne!(plain, ctrl);
        assert_eq!(plain, LocalKey::new(false, false, false, 'R'));
    }
}
