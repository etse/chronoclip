//! Global hotkey: picks the backend for the session. X11 uses a key grab; Wayland gets the
//! GlobalShortcuts portal later (milestone 6). Without a backend, users bind `chronoclip save`
//! in their desktop's keyboard settings.

use chronoclip_capture::{HotkeyError, X11Hotkey};
use chronoclip_core::Hotkey;

pub enum GlobalHotkey {
    X11(X11Hotkey),
    /// No global hotkey available; the message explains why and what to do instead.
    Unavailable(String),
}

const USE_CLI: &str = "bind the command `chronoclip save` in your desktop's keyboard settings";

impl GlobalHotkey {
    /// Start the backend for this session. `on_press` runs on a background thread.
    pub fn start(on_press: impl Fn() + Send + 'static) -> Self {
        if is_wayland() {
            return Self::Unavailable(format!(
                "Global hotkeys on Wayland arrive in a later version. For now, {USE_CLI}."
            ));
        }
        match X11Hotkey::start(on_press) {
            Ok(hotkey) => Self::X11(hotkey),
            Err(e) => {
                tracing::warn!("global hotkey unavailable: {e}");
                Self::Unavailable(format!(
                    "Global hotkey unavailable ({e}). Instead, {USE_CLI}."
                ))
            }
        }
    }

    /// Register `hotkey`, replacing the previous one. Errors are messages for the user.
    pub fn set(&self, hotkey: Hotkey) -> Result<(), String> {
        match self {
            Self::X11(x11) => x11.set(Some(hotkey)).map_err(|e| match e {
                HotkeyError::Taken(_) => format!(
                    "{hotkey} is already used by another program or a desktop shortcut. Pick another hotkey."
                ),
                HotkeyError::KeyNotOnKeyboard(_) => format!(
                    "{hotkey} can't be used: that key is not on your keyboard layout."
                ),
                e => format!("Could not register {hotkey}: {e}."),
            }),
            Self::Unavailable(message) => Err(message.clone()),
        }
    }
}

fn is_wayland() -> bool {
    std::env::var("XDG_SESSION_TYPE").is_ok_and(|t| t == "wayland")
        || std::env::var_os("WAYLAND_DISPLAY").is_some_and(|d| !d.is_empty())
}
