//! FFmpeg wrappers for Chronoclip: an NVENC encoder that takes CUDA frames, and a writer that
//! splits the encoded stream into keyframe-aligned MPEG-TS segments.
//!
//! All `unsafe` FFmpeg calls are confined to this crate behind safe types.

#![allow(unsafe_code)] // FFI to FFmpeg; every block has a SAFETY comment.

mod error;
mod nvenc;
mod segment_writer;

pub use error::EncodeError;
pub use ffmpeg_next::Packet;
pub use nvenc::{EncoderConfig, HwFrame, NvencEncoder};
pub use segment_writer::SegmentWriter;

/// Initialize FFmpeg (idempotent) and route its log output to `tracing` at warning level.
pub fn init() -> Result<(), EncodeError> {
    ffmpeg_next::init().map_err(EncodeError::Ffmpeg)?;
    ffmpeg_next::util::log::set_level(ffmpeg_next::util::log::Level::Warning);
    Ok(())
}
