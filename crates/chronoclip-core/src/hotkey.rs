//! Parsing and formatting of global hotkeys such as `Ctrl+F9`.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Modifier keys held together with the main key.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub super_key: bool,
}

impl Modifiers {
    pub fn is_empty(self) -> bool {
        self == Self::default()
    }
}

/// The non-modifier key of a hotkey.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    /// Function key `F1`..=`F24`.
    F(u8),
    /// Letter `A`..=`Z` (stored upper case).
    Letter(char),
    /// Digit `0`..=`9`.
    Digit(char),
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Key::F(n) => write!(f, "F{n}"),
            Key::Letter(c) | Key::Digit(c) => write!(f, "{c}"),
        }
    }
}

/// A global hotkey, e.g. `Ctrl+Shift+F9`.
///
/// Serialized as its canonical string form.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Hotkey {
    pub modifiers: Modifiers,
    pub key: Key,
}

impl Default for Hotkey {
    /// `Ctrl+F9`
    fn default() -> Self {
        Self {
            modifiers: Modifiers {
                ctrl: true,
                ..Modifiers::default()
            },
            key: Key::F(9),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HotkeyError {
    #[error("hotkey is empty")]
    Empty,
    #[error("unknown modifier `{0}` (use Ctrl, Alt, Shift or Super)")]
    UnknownModifier(String),
    #[error("modifier `{0}` is listed twice")]
    DuplicateModifier(String),
    #[error("unsupported key `{0}` (use F1-F24, A-Z or 0-9)")]
    UnknownKey(String),
    #[error("`{0}` needs at least one modifier, otherwise it would block normal typing")]
    NeedsModifier(String),
}

impl FromStr for Hotkey {
    type Err = HotkeyError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let tokens: Vec<&str> = s.split('+').map(str::trim).collect();
        let Some((key_token, modifier_tokens)) = tokens.split_last() else {
            return Err(HotkeyError::Empty);
        };
        if tokens.iter().any(|t| t.is_empty()) {
            return Err(HotkeyError::Empty);
        }

        let mut modifiers = Modifiers::default();
        for token in modifier_tokens {
            let slot = match token.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => &mut modifiers.ctrl,
                "alt" => &mut modifiers.alt,
                "shift" => &mut modifiers.shift,
                "super" | "meta" | "win" => &mut modifiers.super_key,
                _ => return Err(HotkeyError::UnknownModifier((*token).to_owned())),
            };
            if *slot {
                return Err(HotkeyError::DuplicateModifier((*token).to_owned()));
            }
            *slot = true;
        }

        let key = parse_key(key_token)?;
        if modifiers.is_empty() && !matches!(key, Key::F(_)) {
            return Err(HotkeyError::NeedsModifier((*key_token).to_owned()));
        }
        Ok(Self { modifiers, key })
    }
}

fn parse_key(token: &str) -> Result<Key, HotkeyError> {
    let unknown = || HotkeyError::UnknownKey(token.to_owned());
    let upper = token.to_ascii_uppercase();
    let mut chars = upper.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if c.is_ascii_uppercase() => Ok(Key::Letter(c)),
        (Some(c), None) if c.is_ascii_digit() => Ok(Key::Digit(c)),
        (Some('F'), Some(_)) => match upper[1..].parse::<u8>() {
            Ok(n @ 1..=24) => Ok(Key::F(n)),
            _ => Err(unknown()),
        },
        _ => Err(unknown()),
    }
}

impl fmt::Display for Hotkey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let m = self.modifiers;
        for (held, name) in [
            (m.ctrl, "Ctrl"),
            (m.alt, "Alt"),
            (m.shift, "Shift"),
            (m.super_key, "Super"),
        ] {
            if held {
                write!(f, "{name}+")?;
            }
        }
        write!(f, "{}", self.key)
    }
}

impl TryFrom<String> for Hotkey {
    type Error = HotkeyError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl From<Hotkey> for String {
    fn from(value: Hotkey) -> Self {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Result<Hotkey, HotkeyError> {
        s.parse()
    }

    #[test]
    fn default_is_ctrl_f9() {
        assert_eq!(Hotkey::default().to_string(), "Ctrl+F9");
    }

    #[test]
    fn parses_and_normalizes() {
        assert_eq!(
            parse("ctrl+f9").map(|h| h.to_string()),
            Ok("Ctrl+F9".into())
        );
        assert_eq!(
            parse(" shift + Control + f12 ").map(|h| h.to_string()),
            Ok("Ctrl+Shift+F12".into())
        );
        assert_eq!(
            parse("Super+Alt+r").map(|h| h.to_string()),
            Ok("Alt+Super+R".into())
        );
        assert_eq!(parse("Ctrl+5").map(|h| h.key), Ok(Key::Digit('5')));
    }

    #[test]
    fn bare_function_key_is_allowed() {
        assert_eq!(parse("F10").map(|h| h.to_string()), Ok("F10".into()));
    }

    #[test]
    fn rejects_invalid_input() {
        assert_eq!(parse(""), Err(HotkeyError::Empty));
        assert_eq!(parse("Ctrl+"), Err(HotkeyError::Empty));
        assert_eq!(
            parse("Hyper+F9"),
            Err(HotkeyError::UnknownModifier("Hyper".into()))
        );
        assert_eq!(
            parse("Ctrl+ctrl+F9"),
            Err(HotkeyError::DuplicateModifier("ctrl".into()))
        );
        assert_eq!(
            parse("Ctrl+F25"),
            Err(HotkeyError::UnknownKey("F25".into()))
        );
        assert_eq!(parse("Ctrl+F0"), Err(HotkeyError::UnknownKey("F0".into())));
        assert_eq!(
            parse("Ctrl+Space"),
            Err(HotkeyError::UnknownKey("Space".into()))
        );
        assert_eq!(parse("A"), Err(HotkeyError::NeedsModifier("A".into())));
    }

    #[test]
    fn serde_roundtrip_as_string() {
        #[derive(Serialize, Deserialize)]
        struct Wrapper {
            hotkey: Hotkey,
        }
        let text = toml::to_string(&Wrapper {
            hotkey: Hotkey::default(),
        })
        .unwrap_or_default();
        assert_eq!(text.trim(), r#"hotkey = "Ctrl+F9""#);
        let back: Result<Wrapper, _> = toml::from_str(r#"hotkey = "alt+f1""#);
        assert_eq!(
            back.map(|w| w.hotkey.to_string()).ok(),
            Some("Alt+F1".into())
        );
    }
}
