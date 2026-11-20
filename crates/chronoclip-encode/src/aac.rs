//! AAC encoding with FFmpeg's built-in encoder (LGPL, no extra library).

use chronoclip_core::audio::{CHANNELS, SAMPLE_RATE};
use ffmpeg_next::format::Sample;
use ffmpeg_next::format::sample::Type as SampleType;
use ffmpeg_next::{ChannelLayout, Packet, Rational, codec, encoder, ffi, frame};

use crate::error::{EncodeError, check};

/// AAC bitrate: transparent enough for game audio and voice.
pub const AAC_BITRATE: usize = 160_000;

/// Encodes interleaved stereo `f32` samples at [`SAMPLE_RATE`] to AAC-LC.
///
/// Packet timestamps count sample frames from the first sample pushed (time base
/// 1/[`SAMPLE_RATE`]). The encoder's priming packet (timestamp below zero) is dropped:
/// MPEG-TS can't store negative timestamps, and it only costs the first 21 ms of a session.
pub struct AacEncoder {
    encoder: encoder::Audio,
    frame: frame::Audio,
    /// Sample frames already in `frame`.
    filled: usize,
    frame_size: usize,
    /// Timestamp of the first sample in `frame`.
    pts: i64,
}

impl AacEncoder {
    pub fn new() -> Result<Self, EncodeError> {
        crate::init()?;
        let codec = encoder::find(codec::Id::AAC).ok_or(EncodeError::EncoderUnavailable("aac"))?;
        let mut ctx = codec::context::Context::new_with_codec(codec)
            .encoder()
            .audio()?;
        ctx.set_rate(SAMPLE_RATE as i32);
        ctx.set_channel_layout(ChannelLayout::STEREO);
        ctx.set_format(Sample::F32(SampleType::Planar));
        ctx.set_bit_rate(AAC_BITRATE);
        ctx.set_time_base(Rational::new(1, SAMPLE_RATE as i32));
        let encoder = ctx.open_as(codec)?;
        let frame_size = usize::try_from(encoder.frame_size())
            .ok()
            .filter(|&n| n > 0)
            .unwrap_or(1024);
        let mut frame = frame::Audio::new(
            Sample::F32(SampleType::Planar),
            frame_size,
            ChannelLayout::STEREO,
        );
        frame.set_rate(SAMPLE_RATE);
        Ok(Self {
            encoder,
            frame,
            filled: 0,
            frame_size,
            pts: 0,
        })
    }

    /// Time base of the packets.
    pub fn time_base(&self) -> Rational {
        Rational::new(1, SAMPLE_RATE as i32)
    }

    /// Codec parameters for muxers.
    pub fn parameters(&self) -> codec::Parameters {
        codec::Parameters::from(&*self.encoder)
    }

    /// Encode interleaved stereo samples; finished packets are appended to `out`.
    pub fn push(&mut self, samples: &[f32], out: &mut Vec<Packet>) -> Result<(), EncodeError> {
        let mut rest = samples;
        while rest.len() >= CHANNELS {
            let n = self.fill(Some(rest), rest.len() / CHANNELS)?;
            rest = &rest[n * CHANNELS..];
            self.send_if_full(out)?;
        }
        Ok(())
    }

    /// Encode `frames` sample frames of silence.
    pub fn push_silence(&mut self, frames: u64, out: &mut Vec<Packet>) -> Result<(), EncodeError> {
        let mut left = usize::try_from(frames).unwrap_or(usize::MAX);
        while left > 0 {
            left -= self.fill(None, left)?;
            self.send_if_full(out)?;
        }
        Ok(())
    }

    /// Copy up to `frames` sample frames (from interleaved `source`, or silence) into the
    /// pending frame. Returns how many were taken.
    fn fill(&mut self, source: Option<&[f32]>, frames: usize) -> Result<usize, EncodeError> {
        if self.filled == 0 {
            // SAFETY: the frame is ours; this copies its buffers if the encoder still holds a
            // reference to them, so we never write into data the encoder reads.
            check("av_frame_make_writable", unsafe {
                ffi::av_frame_make_writable(self.frame.as_mut_ptr())
            })?;
        }
        let n = frames.min(self.frame_size - self.filled);
        let range = self.filled..self.filled + n;
        for channel in 0..CHANNELS {
            let plane = &mut self.frame.plane_mut::<f32>(channel)[range.clone()];
            match source {
                Some(src) => {
                    for (dst, frame) in plane.iter_mut().zip(src.as_chunks::<CHANNELS>().0) {
                        *dst = frame[channel];
                    }
                }
                None => plane.fill(0.0),
            }
        }
        self.filled += n;
        Ok(n)
    }

    fn send_if_full(&mut self, out: &mut Vec<Packet>) -> Result<(), EncodeError> {
        if self.filled < self.frame_size {
            return Ok(());
        }
        self.frame.set_pts(Some(self.pts));
        self.pts += i64::try_from(self.frame_size).unwrap_or(1024);
        self.filled = 0;
        self.encoder.send_frame(&self.frame)?;
        self.receive(out)
    }

    fn receive(&mut self, out: &mut Vec<Packet>) -> Result<(), EncodeError> {
        loop {
            let mut packet = Packet::empty();
            match self.encoder.receive_packet(&mut packet) {
                Ok(()) if packet.pts().is_some_and(|pts| pts < 0) => {}
                Ok(()) => out.push(packet),
                Err(ffmpeg_next::Error::Other { errno }) if errno == ffi::EAGAIN => return Ok(()),
                Err(ffmpeg_next::Error::Eof) => return Ok(()),
                Err(e) => {
                    return Err(EncodeError::Call {
                        call: "avcodec_receive_packet",
                        error: e,
                    });
                }
            }
        }
    }
}

impl Drop for AacEncoder {
    fn drop(&mut self) {
        // Drain the encoder so FFmpeg doesn't warn about queued frames; the packets are
        // dropped (the session they belong to has ended).
        if self.encoder.send_eof().is_ok() {
            let _ = self.receive(&mut Vec::new());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_continuous_packets() -> Result<(), EncodeError> {
        let mut enc = AacEncoder::new()?;
        assert_eq!(enc.time_base(), Rational::new(1, 48_000));
        let params = enc.parameters();
        assert_eq!(params.id(), codec::Id::AAC);

        let mut packets = Vec::new();
        // 1 s of a 440 Hz tone, then 1 s of silence.
        let tone: Vec<f32> = (0..48_000)
            .flat_map(|i| {
                let s = (i as f32 * 440.0 * std::f32::consts::TAU / 48_000.0).sin() * 0.2;
                [s, s]
            })
            .collect();
        enc.push(&tone, &mut packets)?;
        enc.push_silence(48_000, &mut packets)?;
        // 2 s = 93.75 frames of 1024; the encoder holds back a frame or two.
        assert!(
            (88..=94).contains(&packets.len()),
            "{} packets",
            packets.len()
        );
        let pts: Vec<i64> = packets.iter().filter_map(|p| p.pts()).collect();
        assert_eq!(pts.len(), packets.len());
        assert!(pts.windows(2).all(|w| w[1] - w[0] == 1024), "{pts:?}");
        // The priming packet before zero is dropped.
        assert_eq!(pts[0], 0);
        Ok(())
    }
}
