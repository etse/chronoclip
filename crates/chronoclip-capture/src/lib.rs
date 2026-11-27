//! Window capture for Chronoclip.
//!
//! X11 backend: the window is redirected with XComposite, its pixmap is bound as an OpenGL
//! texture (GLX_EXT_texture_from_pixmap), copied on the GPU into a texture shared with CUDA,
//! and from there into the encoder's CUDA frame. Frames never touch the CPU.
//!
//! The global hotkey is a passive key grab on the root window (see [`X11Hotkey`]).
//!
//! Audio comes from the PulseAudio API (pipewire-pulse on PipeWire systems), see
//! [`AudioCapture`].
//!
//! Nothing here touches the game process: we only read what the X server already composites.
//! X11, GLX, OpenGL, CUDA and libpulse are loaded at runtime, so no `-dev` packages are needed.

#![allow(unsafe_code)] // FFI to Xlib, GLX, OpenGL, CUDA and libpulse; every block has a SAFETY comment.

mod audio;
mod cuda;
mod error;
mod glx;
mod hotkey;
mod sound;
mod window;
mod xlib;

pub use audio::{AudioCapture, AudioSink, AudioSourceKind, AudioSourceState, GameWindow};
pub use error::CaptureError;
pub use glx::{CaptureStatus, X11WindowCapture};
pub use hotkey::{HotkeyError, X11Hotkey};
pub use sound::play_sound;
pub use window::{OpenWindow, WindowInfo, find_window, list_windows};

/// Destination for one captured frame: an `RGB0` image in CUDA device memory.
#[derive(Debug, Clone, Copy)]
pub struct CudaDestination {
    /// `CUcontext` that owns `device_ptr`.
    pub context: *mut std::ffi::c_void,
    /// `CUdeviceptr` of the first row.
    pub device_ptr: u64,
    /// Bytes per row.
    pub pitch: usize,
}
