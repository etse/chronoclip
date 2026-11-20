//! FFmpeg wrappers for Chronoclip: an NVENC encoder that takes CUDA frames, an AAC encoder,
//! a writer that splits the encoded streams into keyframe-aligned MPEG-TS segments, and the
//! remux that joins segments into an MP4 clip.
//!
//! All `unsafe` FFmpeg calls are confined to this crate behind safe types.

#![allow(unsafe_code)] // FFI to FFmpeg; every block has a SAFETY comment.

mod aac;
mod error;
mod nvenc;
mod remux;
mod segment_writer;

pub use aac::{AAC_BITRATE, AacEncoder};
pub use error::EncodeError;
pub use ffmpeg_next::Packet;
pub use nvenc::{EncoderConfig, HwFrame, NvencEncoder};
pub use remux::{RemuxStats, remux_to_mp4};
pub use segment_writer::{SegmentWriter, StreamConfig};

/// Initialize FFmpeg (idempotent) and route its log output to `tracing` at warning level.
pub fn init() -> Result<(), EncodeError> {
    ffmpeg_next::init().map_err(EncodeError::Ffmpeg)?;
    ffmpeg_next::util::log::set_level(ffmpeg_next::util::log::Level::Warning);
    Ok(())
}
