//! Audio format and the timeline that keeps captured audio in sync with the video clock.

use std::collections::VecDeque;
use std::time::Duration;

/// Sample rate of all recorded audio (Hz). PipeWire/PulseAudio resample for us.
pub const SAMPLE_RATE: u32 = 48_000;
/// Channels of all recorded audio (stereo, interleaved `f32` while captured).
pub const CHANNELS: usize = 2;

/// Number of sample frames in `duration` at [`SAMPLE_RATE`].
pub fn frames_in(duration: Duration) -> i64 {
    i64::try_from(duration.as_micros() * u128::from(SAMPLE_RATE) / 1_000_000).unwrap_or(i64::MAX)
}

/// What to do with one captured chunk of audio.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Placement {
    /// Silent sample frames to write before the chunk (a gap in the capture).
    pub silence: u64,
    /// Sample frames to drop from the start of the chunk (they overlap audio already written).
    pub skip: usize,
}

/// Maps captured audio chunks onto a continuous track whose sample position follows the
/// session clock (the video timestamps).
///
/// Positions are sample frames since the session started. A chunk's position comes from the
/// time it arrived, while the audio device clock drifts slowly against the system clock
/// (hundreds of ppm). Arrival times are late by a varying amount (the sound server delivers
/// in quanta of ~21 ms, plus scheduling), never early. So the track is written continuously
/// and only corrected when it is clearly off by more than [`Self::TOLERANCE`]:
/// - a real gap (the game paused its stream) is filled with silence at once,
/// - audio that arrived earlier than the track position by more than the tolerance (the
///   device runs fast) is dropped; arrival delay can't cause that,
/// - the track lagging behind (the device runs slow) is only corrected when the *smallest*
///   offset over the last [`Self::RECENT`] chunks shows it, since single chunks look late
///   all the time.
///
/// When no audio arrives at all (stream corked, device gone) [`Self::fill_silence`] keeps the
/// track going so players don't lose sync and muxers don't wait for audio.
#[derive(Debug, Clone)]
pub struct AudioTimeline {
    /// Sample frames written so far.
    position: i64,
    started: bool,
    /// Offsets of recent chunks, to tell drift from arrival jitter.
    recent: VecDeque<i64>,
}

impl AudioTimeline {
    /// Offsets up to this are left alone (20 ms).
    pub const TOLERANCE: i64 = SAMPLE_RATE as i64 / 50;
    /// A chunk this late (60 ms) is a gap in the capture and is filled at once.
    pub const GAP: i64 = SAMPLE_RATE as i64 * 3 / 50;
    /// Chunks whose offsets must all be late before the lag is corrected (1 to 2 s of audio).
    pub const RECENT: usize = 100;
    /// With no audio for this long (200 ms), [`Self::fill_silence`] writes silence ...
    pub const STARVED: i64 = SAMPLE_RATE as i64 / 5;
    /// ... up to this far behind the clock (100 ms), so audio that arrives late still fits.
    pub const FILL_MARGIN: i64 = SAMPLE_RATE as i64 / 10;

    pub fn new() -> Self {
        Self {
            position: 0,
            started: false,
            recent: VecDeque::with_capacity(Self::RECENT),
        }
    }

    /// Sample frames written so far.
    pub fn position(&self) -> i64 {
        self.position
    }

    /// Place a chunk of `len` sample frames whose last frame was captured at position `end`.
    pub fn place(&mut self, end: i64, len: usize) -> Placement {
        let len_i = i64::try_from(len).unwrap_or(i64::MAX);
        let start = end - len_i;
        let mut placement = Placement::default();
        if !self.started {
            // The first chunk sets the start exactly: silence before it, or cut what was
            // captured before the session began.
            if end <= 0 {
                placement.skip = len;
                return placement;
            }
            self.started = true;
            if start > 0 {
                placement.silence = start.unsigned_abs();
            } else {
                placement.skip = start.unsigned_abs().try_into().unwrap_or(len);
            }
        } else {
            let offset = start - self.position;
            if offset >= Self::GAP {
                placement.silence = offset.unsigned_abs();
                self.recent.clear();
            } else if offset < -Self::TOLERANCE {
                placement.skip = usize::try_from(-offset).unwrap_or(len).min(len);
                self.recent.clear();
            } else {
                if self.recent.len() == Self::RECENT {
                    self.recent.pop_front();
                }
                self.recent.push_back(offset);
                let lag = self.recent.iter().copied().min().unwrap_or(0);
                if self.recent.len() == Self::RECENT && lag > Self::TOLERANCE {
                    placement.silence = lag.unsigned_abs();
                    self.recent.clear();
                }
            }
        }
        let written = len - placement.skip.min(len);
        self.position +=
            i64::try_from(placement.silence).unwrap_or(0) + i64::try_from(written).unwrap_or(0);
        placement
    }

    /// Called regularly with the current clock position `now`. Returns how many silent sample
    /// frames to write because no audio has arrived for [`Self::STARVED`].
    pub fn fill_silence(&mut self, now: i64) -> u64 {
        let behind = now - self.position;
        if behind <= Self::STARVED {
            return 0;
        }
        let silence = behind - Self::FILL_MARGIN;
        self.started = true;
        self.recent.clear();
        self.position += silence;
        silence.unsigned_abs()
    }
}

impl Default for AudioTimeline {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: i64 = SAMPLE_RATE as i64 / 1000;
    /// A 10 ms chunk.
    const CHUNK: usize = 480;

    fn place(t: &mut AudioTimeline, end_ms: i64) -> Placement {
        t.place(end_ms * MS, CHUNK)
    }

    #[test]
    fn first_chunk_is_placed_exactly() {
        let mut t = AudioTimeline::new();
        // Ends at 25 ms: starts at 15 ms, so 15 ms of silence first.
        assert_eq!(
            place(&mut t, 25),
            Placement {
                silence: 15 * MS as u64,
                skip: 0
            }
        );
        assert_eq!(t.position(), 25 * MS);

        // Captured partly before the session started: the early part is cut.
        let mut t = AudioTimeline::new();
        assert_eq!(
            place(&mut t, 4),
            Placement {
                silence: 0,
                skip: 6 * MS as usize
            }
        );
        assert_eq!(t.position(), 4 * MS);

        // Entirely before the session: dropped, timeline not started.
        let mut t = AudioTimeline::new();
        assert_eq!(place(&mut t, -5).skip, CHUNK);
        assert_eq!(t.position(), 0);
    }

    #[test]
    fn jitter_is_ignored() {
        let mut t = AudioTimeline::new();
        place(&mut t, 10);
        for (i, jitter) in [3, 0, 12, 1, 18, 0, 5, 9, 2, 15].into_iter().enumerate() {
            let end = 20 + 10 * i as i64 + jitter;
            assert_eq!(place(&mut t, end), Placement::default(), "chunk {i}");
        }
        assert_eq!(t.position(), 110 * MS);
    }

    #[test]
    fn gap_is_filled_with_silence() {
        let mut t = AudioTimeline::new();
        place(&mut t, 10);
        place(&mut t, 20);
        // The stream paused for 500 ms.
        let p = place(&mut t, 530);
        assert_eq!(p.silence, 500 * MS as u64);
        assert_eq!(t.position(), 530 * MS);
    }

    #[test]
    fn slow_device_clock_is_corrected_after_consistent_lateness() {
        let mut t = AudioTimeline::new();
        place(&mut t, 10);
        // Every chunk is 30 ms "late" (but less than a gap): corrected once a full window of
        // chunks shows it, by the smallest offset seen.
        let mut silence = 0;
        for i in 0..AudioTimeline::RECENT as i64 {
            let jitter = if i == 3 { 25 } else { 30 };
            let p = place(&mut t, 20 + 10 * i + jitter);
            if (i as usize) < AudioTimeline::RECENT - 1 {
                assert_eq!(p, Placement::default(), "chunk {i}");
            }
            silence += p.silence;
        }
        assert_eq!(silence, 25 * MS as u64);
        // One early chunk in the window (the real offset is small) prevents the correction.
        let mut t = AudioTimeline::new();
        place(&mut t, 10);
        let mut silence = 0;
        for i in 0..2 * AudioTimeline::RECENT as i64 {
            let jitter = if i % 50 == 7 { 2 } else { 30 };
            silence += place(&mut t, 20 + 10 * i + jitter).silence;
        }
        assert_eq!(silence, 0);
    }

    #[test]
    fn fast_device_clock_drops_overlap() {
        let mut t = AudioTimeline::new();
        place(&mut t, 10);
        place(&mut t, 20);
        // A 40 ms chunk that starts 25 ms before the track position (more than the 20 ms
        // tolerance): its first 25 ms are dropped.
        let p = t.place(35 * MS, 40 * MS as usize);
        assert_eq!(
            p,
            Placement {
                silence: 0,
                skip: 25 * MS as usize
            }
        );
        assert_eq!(t.position(), 35 * MS);
        // A chunk entirely in the past is dropped completely.
        let p = t.place(0, CHUNK);
        assert_eq!(p.skip, CHUNK);
    }

    #[test]
    fn starved_track_is_filled_up_to_the_margin() {
        let mut t = AudioTimeline::new();
        assert_eq!(t.fill_silence(150 * MS), 0);
        let filled = t.fill_silence(1000 * MS);
        assert_eq!(filled, 900 * MS as u64);
        assert_eq!(t.position(), 900 * MS);
        // Audio resumes "now": the 100 ms margin minus the chunk is a gap → silence.
        let p = place(&mut t, 1000);
        assert_eq!(p.silence, 90 * MS as u64);
        assert_eq!(t.position(), 1000 * MS);
    }

    #[test]
    fn frames_in_duration() {
        assert_eq!(frames_in(Duration::from_millis(20)), 960);
        assert_eq!(frames_in(Duration::from_secs(2)), 96_000);
    }
}
