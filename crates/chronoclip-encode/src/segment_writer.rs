//! Splits an encoded stream into keyframe-aligned MPEG-TS segments.

use std::path::{Path, PathBuf};
use std::time::Duration;

use chronoclip_core::segments::{Segment, segment_file_name};
use ffmpeg_next::format::context::Output;
use ffmpeg_next::{Packet, Rational, codec};

use crate::error::EncodeError;

struct OpenSegment {
    seq: u64,
    path: PathBuf,
    output: Output,
    stream_time_base: Rational,
    first_pts: i64,
    end_pts: i64,
}

/// Writes packets into `seg_XXXXXXXX.ts` files of roughly `target` length, starting a new file
/// at the first keyframe after `target` has elapsed.
pub struct SegmentWriter {
    dir: PathBuf,
    params: codec::Parameters,
    time_base: Rational,
    target: Duration,
    stream: u64,
    next_seq: u64,
    current: Option<OpenSegment>,
}

impl SegmentWriter {
    /// `time_base` is the time base of the packets' timestamps (the encoder's). `stream` is
    /// stored in every [`Segment`]; segments with the same value can be joined into one clip.
    pub fn new(
        dir: &Path,
        params: codec::Parameters,
        time_base: Rational,
        target: Duration,
        stream: u64,
        first_seq: u64,
    ) -> Self {
        Self {
            dir: dir.to_owned(),
            params,
            time_base,
            target,
            stream,
            next_seq: first_seq,
            current: None,
        }
    }

    /// Sequence number the next segment will get.
    pub fn next_seq(&self) -> u64 {
        self.next_seq
    }

    /// Sequence number of the segment being written, if one is open.
    pub fn current_seq(&self) -> Option<u64> {
        self.current.as_ref().map(|c| c.seq)
    }

    /// Write one packet. Returns the segment that was finished as a result, if any.
    pub fn write(&mut self, mut packet: Packet) -> Result<Option<Segment>, EncodeError> {
        let pts = packet.pts().unwrap_or(0);
        let is_key = packet.is_key();

        let mut finished = None;
        if is_key
            && let Some(current) = &self.current
            && self.ticks_to_duration(pts - current.first_pts) >= self.target
        {
            finished = self.close(Some(pts))?;
        }

        if self.current.is_none() {
            if !is_key {
                // A segment must start with a keyframe; drop until the next one.
                return Ok(finished);
            }
            self.current = Some(self.open(pts)?);
        }

        if let Some(current) = &mut self.current {
            current.end_pts = pts + packet.duration().max(1);
            packet.set_stream(0);
            packet.set_position(-1);
            packet.rescale_ts(self.time_base, current.stream_time_base);
            packet
                .write_interleaved(&mut current.output)
                .map_err(|e| EncodeError::Call {
                    call: "av_interleaved_write_frame",
                    error: e,
                })?;
        }
        Ok(finished)
    }

    /// Close the current segment (e.g. when recording stops).
    pub fn finish(&mut self) -> Result<Option<Segment>, EncodeError> {
        self.close(None)
    }

    fn open(&mut self, first_pts: i64) -> Result<OpenSegment, EncodeError> {
        let seq = self.next_seq;
        self.next_seq += 1;
        let path = self.dir.join(segment_file_name(seq));
        let mut output = ffmpeg_next::format::output_as(&path, "mpegts")?;
        {
            let mut stream = output.add_stream(ffmpeg_next::encoder::find(codec::Id::None))?;
            stream.set_parameters(self.params.clone());
            stream.set_time_base(self.time_base);
        }
        output.write_header()?;
        let stream_time_base = output
            .stream(0)
            .map(|s| s.time_base())
            .unwrap_or(self.time_base);
        Ok(OpenSegment {
            seq,
            path,
            output,
            stream_time_base,
            first_pts,
            end_pts: first_pts,
        })
    }

    fn close(&mut self, next_pts: Option<i64>) -> Result<Option<Segment>, EncodeError> {
        let Some(mut current) = self.current.take() else {
            return Ok(None);
        };
        current.output.write_trailer()?;
        drop(current.output); // closes the file
        let bytes = std::fs::metadata(&current.path)
            .map_err(|source| EncodeError::Io {
                path: current.path.clone(),
                source,
            })?
            .len();
        let end = next_pts.unwrap_or(current.end_pts);
        Ok(Some(Segment {
            seq: current.seq,
            stream: self.stream,
            path: current.path,
            duration: self.ticks_to_duration(end - current.first_pts),
            bytes,
        }))
    }

    fn ticks_to_duration(&self, ticks: i64) -> Duration {
        let secs = ticks.max(0) as f64 * f64::from(self.time_base.numerator())
            / f64::from(self.time_base.denominator());
        Duration::from_secs_f64(secs)
    }
}
