//! Splits encoded streams (video plus optional audio) into keyframe-aligned MPEG-TS segments.

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
    /// Time base of each stream in the file (chosen by the muxer).
    stream_time_bases: Vec<Rational>,
    first_pts: i64,
    end_pts: i64,
}

/// One stream of a segment: codec parameters and the time base of its packets.
#[derive(Clone)]
pub struct StreamConfig {
    pub params: codec::Parameters,
    pub time_base: Rational,
}

/// Writes packets into `seg_XXXXXXXX.ts` files of roughly `target` length, starting a new file
/// at the first video keyframe after `target` has elapsed.
///
/// Stream 0 is the video and decides where segments are cut; further streams (audio) go into
/// whichever segment is open when their packets arrive. Packets are written in arrival order
/// (not interleaved): MPEG-TS only needs increasing timestamps per stream, and waiting for
/// interleaving would hold video back whenever audio lags.
pub struct SegmentWriter {
    dir: PathBuf,
    streams: Vec<StreamConfig>,
    target: Duration,
    stream: u64,
    next_seq: u64,
    current: Option<OpenSegment>,
}

impl SegmentWriter {
    /// `streams[0]` is the video. `stream` is stored in every [`Segment`]; segments with the
    /// same value can be joined into one clip.
    pub fn new(
        dir: &Path,
        streams: Vec<StreamConfig>,
        target: Duration,
        stream: u64,
        first_seq: u64,
    ) -> Self {
        Self {
            dir: dir.to_owned(),
            streams,
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

    /// Write one packet of stream `index`. Returns the segment that was finished as a result,
    /// if any. Packets of other streams before the first video keyframe are dropped.
    pub fn write(&mut self, packet: Packet, index: usize) -> Result<Option<Segment>, EncodeError> {
        if index == 0 {
            return self.write_video(packet);
        }
        let Some(config) = self.streams.get(index) else {
            return Ok(None);
        };
        let time_base = config.time_base;
        if let Some(current) = &mut self.current {
            write_packet(current, packet, index, time_base)?;
        }
        Ok(None)
    }

    fn write_video(&mut self, packet: Packet) -> Result<Option<Segment>, EncodeError> {
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

        let time_base = self.video_time_base();
        if let Some(current) = &mut self.current {
            current.end_pts = pts + packet.duration().max(1);
            write_packet(current, packet, 0, time_base)?;
        }
        Ok(finished)
    }

    fn video_time_base(&self) -> Rational {
        self.streams
            .first()
            .map_or(Rational::new(1, 1), |s| s.time_base)
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
        for config in &self.streams {
            let mut stream = output.add_stream(ffmpeg_next::encoder::find(codec::Id::None))?;
            stream.set_parameters(config.params.clone());
            stream.set_time_base(config.time_base);
        }
        output.write_header()?;
        let stream_time_bases = self
            .streams
            .iter()
            .enumerate()
            .map(|(i, c)| output.stream(i).map_or(c.time_base, |s| s.time_base()))
            .collect();
        Ok(OpenSegment {
            seq,
            path,
            output,
            stream_time_bases,
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
        let time_base = self.video_time_base();
        let secs = ticks.max(0) as f64 * f64::from(time_base.numerator())
            / f64::from(time_base.denominator().max(1));
        Duration::from_secs_f64(secs)
    }
}

fn write_packet(
    segment: &mut OpenSegment,
    mut packet: Packet,
    index: usize,
    time_base: Rational,
) -> Result<(), EncodeError> {
    let Some(&stream_time_base) = segment.stream_time_bases.get(index) else {
        return Ok(());
    };
    packet.set_stream(index);
    packet.set_position(-1);
    packet.rescale_ts(time_base, stream_time_base);
    packet
        .write(&mut segment.output)
        .map(drop)
        .map_err(|e| EncodeError::Call {
            call: "av_write_frame",
            error: e,
        })
}
