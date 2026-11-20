//! Platform-independent logic for Chronoclip.
//!
//! This crate must stay free of FFI, D-Bus, PipeWire and X11 so it compiles fast and is easy
//! to unit-test (see AGENTS.md).

#![forbid(unsafe_code)]

pub mod audio;
pub mod clips;
pub mod disk;
pub mod hotkey;
pub mod ipc;
pub mod paths;
pub mod segments;
pub mod settings;

pub use hotkey::Hotkey;
pub use paths::Paths;
pub use settings::{AudioMode, EncoderPreference, Settings};
