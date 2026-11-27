//! Platform-independent logic for Chronoclip.
//!
//! This crate must stay free of FFI, D-Bus, PipeWire and X11 so it compiles fast and is easy
//! to unit-test (see AGENTS.md).

#![forbid(unsafe_code)]

/// Application ID: desktop entry, icon and AppStream metadata names (`packaging/linux/`).
pub const APP_ID: &str = "io.github.etse.chronoclip";

pub mod audio;
pub mod autostart;
pub mod chime;
pub mod clips;
pub mod disk;
pub mod games;
pub mod hotkey;
pub mod ipc;
pub mod launchers;
pub mod paths;
pub mod segments;
pub mod settings;

pub use games::Game;
pub use hotkey::Hotkey;
pub use paths::Paths;
pub use settings::{AudioMode, EncoderPreference, Settings};
