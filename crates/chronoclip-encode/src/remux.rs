//! Joins buffer segments into one MP4 by stream copy (no re-encode).

use std::ffi::CString;
use std::path::{Path, PathBuf};
use std::ptr;
use std::time::{Duration, Instant};

use ffmpeg_next::format::context::Input;
use ffmpeg_next::{Dictionary, Packet, Rational, codec, ffi, media};

use crate::error::{EncodeError, check};

/// Result of a successful remux.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemuxStats {
    /// Length of the video in the clip.
    pub duration: Duration,
    pub bytes: u64,
    /// Segments that could not be read and were left out (normally 0).
    pub skipped_segments: usize,
    /// Wall-clock time the remux took.
    pub elapsed: Duration,
}

/// Join MPEG-TS `segments` (oldest first, same codec parameters) into an MP4 at `output`.
///
/// Packets are copied, never decoded. Timestamps are rebased onto one continuous timeline
/// starting at zero, so segments from a recording that was interrupted (window hidden, or
/// timestamps that restart) still play back without a gap. The `moov` atom is moved to the
/// front (`+faststart`) so the clip starts playing immediately in browsers and chat apps.
///
/// On error a partially written `output` may remain; the caller deletes it.
pub fn remux_to_mp4(segments: &[PathBuf], output: &Path) -> Result<RemuxStats, EncodeError> {
    let started = Instant::now();
    let (first, rest) = segments.split_first().ok_or(EncodeError::NoSegments)?;

    // The first segment is probed fully: the MP4 header needs the complete codec parameters
    // (SPS/PPS as extradata), which the MPEG-TS header alone doesn't give.
    let mut first_input = ffmpeg_next::format::input(utf8_path(first)?)?;
    let (in_index, in_time_base, params) = video_stream(&first_input)?;

    let mut out = ffmpeg_next::format::output_as(utf8_path(output)?, "mp4")?;
    {
        let mut stream = out.add_stream(ffmpeg_next::encoder::find(codec::Id::None))?;
        stream.set_parameters(params);
        // SAFETY: the stream's codec parameters are owned by the output context we hold
        // mutably. A zero tag lets the MP4 muxer pick its own (`avc1`); the MPEG-TS tag is
        // invalid in MP4.
        unsafe { (*stream.parameters().as_mut_ptr()).codec_tag = 0 };
        stream.set_time_base(in_time_base);
    }
    let mut options = Dictionary::new();
    options.set("movflags", "+faststart");
    out.write_header_with(options)?;
    let out_time_base = out.stream(0).map(|s| s.time_base()).unwrap_or(in_time_base);

    let mut timeline = Timeline::new(ticks_per_second(in_time_base));
    let mut skipped_segments = 0;
    let mut copy = |input: &mut Input, index: usize| -> Result<(), EncodeError> {
        timeline.start_file();
        let mut packet = Packet::empty();
        loop {
            match packet.read(input) {
                Ok(()) => {}
                Err(ffmpeg_next::Error::Eof) => return Ok(()),
                Err(e) => {
                    return Err(EncodeError::Call {
                        call: "av_read_frame",
                        error: e,
                    });
                }
            }
            if packet.stream() != index {
                continue;
            }
            let Some(dts) = packet.dts().or(packet.pts()) else {
                continue;
            };
            let Some(offset) = timeline.offset_for(dts) else {
                tracing::debug!(dts, "dropping packet with non-increasing timestamp");
                continue;
            };
            packet.set_dts(Some(dts + offset));
            packet.set_pts(Some(packet.pts().unwrap_or(dts) + offset));
            packet.set_duration(timeline.frame_ticks());
            packet.set_stream(0);
            packet.set_position(-1);
            packet.rescale_ts(in_time_base, out_time_base);
            packet
                .write_interleaved(&mut out)
                .map_err(|error| EncodeError::Call {
                    call: "av_interleaved_write_frame",
                    error,
                })?;
        }
    };

    copy(&mut first_input, in_index)?;
    drop(first_input);
    for path in rest {
        // The rest are opened without probing (that would decode a frame per segment and
        // make long saves slow). The MPEG-TS header is enough to find the video stream.
        let opened = open_input_unprobed(path).and_then(|mut input| {
            let index = input
                .streams()
                .find(|s| s.parameters().medium() == media::Type::Video)
                .map(|s| s.index())
                .ok_or(EncodeError::NoVideoStream)?;
            copy(&mut input, index)
        });
        if let Err(e) = opened {
            tracing::warn!("skipping unreadable segment {}: {e}", path.display());
            skipped_segments += 1;
        }
    }
    out.write_trailer()?;
    drop(out); // closes the file

    let bytes = std::fs::metadata(output)
        .map_err(|source| EncodeError::Io {
            path: output.to_owned(),
            source,
        })?
        .len();
    Ok(RemuxStats {
        duration: timeline.duration(),
        bytes,
        skipped_segments,
        elapsed: started.elapsed(),
    })
}

fn utf8_path(path: &Path) -> Result<&str, EncodeError> {
    // ffmpeg-next panics on paths that aren't valid UTF-8, so reject them here.
    path.to_str()
        .ok_or_else(|| EncodeError::NonUtf8Path(path.to_owned()))
}

fn video_stream(input: &Input) -> Result<(usize, Rational, codec::Parameters), EncodeError> {
    let stream = input
        .streams()
        .best(media::Type::Video)
        .ok_or(EncodeError::NoVideoStream)?;
    Ok((stream.index(), stream.time_base(), stream.parameters()))
}

fn ticks_per_second(time_base: Rational) -> i64 {
    let (num, den) = (time_base.numerator(), time_base.denominator());
    if num > 0 && den > 0 {
        i64::from(den / num).max(1)
    } else {
        90_000
    }
}

/// Open an input file without `avformat_find_stream_info`.
fn open_input_unprobed(path: &Path) -> Result<Input, EncodeError> {
    let c_path =
        CString::new(utf8_path(path)?).map_err(|_| EncodeError::NonUtf8Path(path.to_owned()))?;
    let mut ctx: *mut ffi::AVFormatContext = ptr::null_mut();
    // SAFETY: `ctx` is a valid out pointer, `c_path` a NUL-terminated string; format and
    // options are NULL (autodetect, no options). On failure FFmpeg frees the context and
    // leaves `ctx` NULL.
    let ret = unsafe {
        ffi::avformat_open_input(&mut ctx, c_path.as_ptr(), ptr::null(), ptr::null_mut())
    };
    check("avformat_open_input", ret)?;
    // SAFETY: `ctx` was just opened successfully; `Input` takes ownership and closes it on drop.
    Ok(unsafe { Input::wrap(ctx) })
}

/// Maps the timestamps of several input files onto one increasing timeline starting at zero.
///
/// Within a recording, segments continue each other's timestamps and are copied as they are.
/// When a file's timestamps jump backwards (the encoder restarted) or far forward (MPEG-TS
/// wrap-around or a pause), the file is attached one frame after the previous packet.
#[derive(Debug)]
struct Timeline {
    offset: Option<i64>,
    last_input_dts: i64,
    last_output_dts: i64,
    frame: i64,
    ticks_per_second: i64,
    new_file: bool,
}

impl Timeline {
    fn new(ticks_per_second: i64) -> Self {
        Self {
            offset: None,
            last_input_dts: 0,
            last_output_dts: 0,
            frame: ticks_per_second / 60,
            ticks_per_second,
            new_file: true,
        }
    }

    fn start_file(&mut self) {
        self.new_file = true;
    }

    /// Offset to add to a packet with decode timestamp `dts`, or `None` to drop the packet.
    fn offset_for(&mut self, dts: i64) -> Option<i64> {
        let new_file = std::mem::take(&mut self.new_file);
        let offset = match self.offset {
            None => -dts,
            Some(offset) => {
                let delta = dts - self.last_input_dts;
                // A gap of more than a second is not a skipped frame but a break.
                let max_gap = self.ticks_per_second;
                if new_file && (delta <= 0 || delta > max_gap) {
                    self.last_output_dts + self.frame - dts
                } else {
                    if delta > 0 && delta <= max_gap && !new_file {
                        self.frame = delta;
                    }
                    offset
                }
            }
        };
        let out = dts + offset;
        if self.offset.is_some() && out <= self.last_output_dts {
            return None;
        }
        self.offset = Some(offset);
        self.last_input_dts = dts;
        self.last_output_dts = out;
        Some(offset)
    }

    /// Length of one frame in input ticks (the most recent spacing between packets).
    fn frame_ticks(&self) -> i64 {
        self.frame
    }

    /// Duration of everything mapped so far, including the last frame.
    fn duration(&self) -> Duration {
        let ticks = if self.offset.is_some() {
            self.last_output_dts + self.frame
        } else {
            0
        };
        Duration::from_secs_f64(ticks.max(0) as f64 / self.ticks_per_second.max(1) as f64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feed `files` (lists of dts) through a timeline; returns the output dts per packet
    /// (`None` for dropped packets).
    fn run(files: &[&[i64]]) -> (Vec<Option<i64>>, Timeline) {
        let mut t = Timeline::new(90_000);
        let mut out = Vec::new();
        for file in files {
            t.start_file();
            for &dts in *file {
                out.push(t.offset_for(dts).map(|o| dts + o));
            }
        }
        (out, t)
    }

    #[test]
    fn continuous_segments_keep_their_spacing() {
        // 60 fps in 90 kHz ticks, starting at the MPEG-TS mux delay.
        let (out, t) = run(&[&[63_000, 64_500, 66_000], &[67_500, 69_000]]);
        assert_eq!(
            out,
            vec![Some(0), Some(1_500), Some(3_000), Some(4_500), Some(6_000)]
        );
        assert_eq!(t.frame_ticks(), 1_500);
        // 5 frames of 1/60 s.
        assert_eq!(t.duration().as_millis(), 83);
    }

    #[test]
    fn skipped_frames_keep_real_time() {
        let (out, _) = run(&[&[0, 1_500, 6_000, 7_500]]);
        assert_eq!(out, vec![Some(0), Some(1_500), Some(6_000), Some(7_500)]);
    }

    #[test]
    fn restarted_timestamps_continue_after_previous_file() {
        // Second file restarts at 63 000 (new encoder session): attach it one frame later.
        let (out, _) = run(&[&[63_000, 64_500, 66_000], &[63_000, 64_500]]);
        assert_eq!(
            out,
            vec![Some(0), Some(1_500), Some(3_000), Some(4_500), Some(6_000)]
        );
    }

    #[test]
    fn large_forward_jump_is_closed() {
        let (out, _) = run(&[&[0, 3_000], &[900_000, 903_000]]);
        assert_eq!(out, vec![Some(0), Some(3_000), Some(6_000), Some(9_000)]);
    }

    #[test]
    fn non_increasing_packets_inside_a_file_are_dropped() {
        let (out, _) = run(&[&[0, 1_500, 1_500, 1_000, 3_000]]);
        assert_eq!(out, vec![Some(0), Some(1_500), None, None, Some(3_000)]);
    }

    #[test]
    fn empty_timeline_has_zero_duration() {
        assert_eq!(Timeline::new(90_000).duration(), Duration::ZERO);
    }
}
