//! Converts between hotkey strings ("Ctrl+Shift+2" etc) and `global_hotkey::HotKey`.

use global_hotkey::hotkey::{Code, HotKey, Modifiers};

/// Converts a string like "Ctrl+Shift+2" into a `HotKey`. Modifiers are
/// case-insensitive; digit/letter keys are auto-expanded to Digit/Key,
/// anything else is interpreted as a `Code` name (F5 / NumpadSubtract etc).
pub fn parse(spec: &str) -> Option<HotKey> {
    let mut mods = Modifiers::empty();
    let mut code: Option<Code> = None;
    for tok in spec.split('+') {
        let t = tok.trim();
        match t.to_ascii_lowercase().as_str() {
            "" => {}
            "ctrl" | "control" => mods |= Modifiers::CONTROL,
            "shift" => mods |= Modifiers::SHIFT,
            "alt" | "option" => mods |= Modifiers::ALT,
            "super" | "win" | "windows" | "meta" | "cmd" | "command" => mods |= Modifiers::META,
            _ => code = parse_code(t),
        }
    }
    let code = code?;
    let mods = (!mods.is_empty()).then_some(mods);
    Some(HotKey::new(mods, code))
}

/// Converts a single key into a `Code` (digit -> DigitN, letter -> KeyX, otherwise the `Code` name).
fn parse_code(key: &str) -> Option<Code> {
    if key.len() == 1 {
        let c = key.chars().next().unwrap();
        if c.is_ascii_digit() {
            return format!("Digit{c}").parse().ok();
        }
        if c.is_ascii_alphabetic() {
            return format!("Key{}", c.to_ascii_uppercase()).parse().ok();
        }
    }
    key.parse::<Code>().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_friendly_hotkey_strings() {
        assert_eq!(
            parse("Ctrl+Shift+2"),
            Some(HotKey::new(
                Some(Modifiers::CONTROL | Modifiers::SHIFT),
                Code::Digit2
            ))
        );
        assert_eq!(
            parse("alt+a"),
            Some(HotKey::new(Some(Modifiers::ALT), Code::KeyA))
        );
        assert_eq!(parse("F5"), Some(HotKey::new(None, Code::F5)));
        assert_eq!(
            parse("Ctrl+NumpadSubtract"),
            Some(HotKey::new(Some(Modifiers::CONTROL), Code::NumpadSubtract))
        );
        assert_eq!(parse("Ctrl+"), None);
    }
}
