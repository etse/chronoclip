//! Audio tracks of the recording: capture (PulseAudio/PipeWire) → timeline → AAC → writer.
//!
//! Each track runs on its capture thread (see `chronoclip_capture::AudioCapture`). Its
//! timestamps count from the start of the current video session ([`SessionClock`]), so audio
//! and video share one clock; between sessions (no window) nothing is written.

use std::sync::mpsc::SyncSender;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use chronoclip_capture::{AudioCapture, AudioSink, AudioSourceKind, AudioSourceState, GameWindow};
use chronoclip_core::audio::{AudioTimeline, CHANNELS, frames_in};
use chronoclip_encode::AacEncoder;

use crate::recorder::{SendMessage, WriterMessage};

/// The video session audio is aligned to: its id and the instant of video timestamp 0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Session {
    pub id: u64,
    pub start: Instant,
}

/// The current video session, shared by the recorder (sets it) and the audio tracks.
#[derive(Debug, Default)]
pub struct SessionClock(Mutex<Option<Session>>);

impl SessionClock {
    fn lock(&self) -> MutexGuard<'_, Option<Session>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn get(&self) -> Option<Session> {
        *self.lock()
    }

    pub fn set(&self, session: Option<Session>) {
        *self.lock() = session;
    }
}

/// One audio track being recorded. Dropping it stops the capture.
pub struct AudioTrack {
    pub title: String,
    capture: AudioCapture,
    state: Arc<Mutex<AudioSourceState>>,
}

impl AudioTrack {
    /// Start capturing `kind` into writer stream `index`.
    pub fn start(
        kind: AudioSourceKind,
        index: usize,
        clock: Arc<SessionClock>,
        sender: SyncSender<SendMessage>,
    ) -> std::io::Result<Self> {
        let state = Arc::new(Mutex::new(AudioSourceState::Connecting));
        let sink = TrackSink {
            index,
            clock,
            sender,
            state: Arc::clone(&state),
            session: None,
            packets: Vec::new(),
            failed: false,
        };
        let title = match kind {
            AudioSourceKind::Game => "Game audio",
            AudioSourceKind::AllSound => "All sound",
            AudioSourceKind::Microphone => "Microphone",
        };
        Ok(Self {
            title: title.into(),
            capture: AudioCapture::start(kind, sink)?,
            state,
        })
    }

    /// The recorded window, to find the game's audio stream.
    pub fn set_game(&self, game: Option<GameWindow>) {
        self.capture.set_game(game);
    }

    pub fn state(&self) -> AudioSourceState {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

/// Per-session encoding state.
struct TrackSession {
    session: Session,
    timeline: AudioTimeline,
    encoder: AacEncoder,
}

struct TrackSink {
    index: usize,
    clock: Arc<SessionClock>,
    sender: SyncSender<SendMessage>,
    state: Arc<Mutex<AudioSourceState>>,
    session: Option<TrackSession>,
    packets: Vec<chronoclip_encode::Packet>,
    /// The encoder could not be created; logged once.
    failed: bool,
}

impl TrackSink {
    /// The current session's encoding state, (re)created when the video session changed.
    fn current(&mut self) -> Option<&mut TrackSession> {
        let Some(session) = self.clock.get() else {
            self.session = None;
            return None;
        };
        if self.session.as_ref().map(|s| s.session.id) != Some(session.id) {
            self.session = None;
            match AacEncoder::new() {
                Ok(encoder) => {
                    self.session = Some(TrackSession {
                        session,
                        timeline: AudioTimeline::new(),
                        encoder,
                    });
                }
                Err(e) => {
                    if !std::mem::replace(&mut self.failed, true) {
                        tracing::error!("cannot start the AAC encoder: {e}");
                    }
                    return None;
                }
            }
        }
        self.session.as_mut()
    }

    /// Send the encoded packets to the writer, tagged with the session they belong to.
    fn flush(&mut self, session: u64) {
        let mut packets = std::mem::take(&mut self.packets);
        for packet in packets.drain(..) {
            let message = WriterMessage::Packet {
                session,
                stream: self.index,
                packet,
            };
            if self.sender.send(SendMessage(message)).is_err() {
                // The writer stopped (recorder shutting down); the rest are dropped.
                break;
            }
        }
        packets.clear();
        self.packets = packets; // keep the allocation
    }
}

/// Clock position of `instant` in sample frames since `start` (negative before it).
fn position(start: Instant, instant: Instant) -> i64 {
    match instant.checked_duration_since(start) {
        Some(after) => frames_in(after),
        None => -frames_in(start.duration_since(instant)),
    }
}

impl AudioSink for TrackSink {
    fn samples(&mut self, samples: &[f32], end: Instant) {
        let index = self.index;
        let mut packets = std::mem::take(&mut self.packets);
        let Some(track) = self.current() else {
            self.packets = packets;
            return;
        };
        let frames = samples.len() / CHANNELS;
        let placement = track
            .timeline
            .place(position(track.session.start, end), frames);
        if placement.silence > 0 || placement.skip > 0 {
            tracing::debug!(
                track = index,
                silence = placement.silence,
                skip = placement.skip,
                position = track.timeline.position(),
                "audio timeline corrected"
            );
        }
        let keep = &samples[placement.skip.min(frames) * CHANNELS..frames * CHANNELS];
        let result = track
            .encoder
            .push_silence(placement.silence, &mut packets)
            .and_then(|()| track.encoder.push(keep, &mut packets));
        let id = track.session.id;
        if let Err(e) = result {
            tracing::warn!("audio encoding failed: {e}");
        }
        self.packets = packets;
        self.flush(id);
    }

    fn tick(&mut self, now: Instant) {
        let mut packets = std::mem::take(&mut self.packets);
        let Some(track) = self.current() else {
            self.packets = packets;
            return;
        };
        let silence = track
            .timeline
            .fill_silence(position(track.session.start, now));
        if silence == 0 {
            self.packets = packets;
            return;
        }
        if let Err(e) = track.encoder.push_silence(silence, &mut packets) {
            tracing::warn!("audio encoding failed: {e}");
        }
        let id = track.session.id;
        self.packets = packets;
        self.flush(id);
    }

    fn state(&mut self, state: AudioSourceState) {
        *self.state.lock().unwrap_or_else(|e| e.into_inner()) = state;
    }
}

/// Short description of the audio tracks for the status line, e.g. "sound: game + mic".
pub fn describe(states: &[AudioSourceState]) -> Option<String> {
    let mut parts = Vec::new();
    for state in states {
        parts.push(match state {
            AudioSourceState::Connecting => "connecting sound…".to_owned(),
            AudioSourceState::Game => "game sound".to_owned(),
            AudioSourceState::AllSound {
                game_missing: false,
            } => "all sound".to_owned(),
            AudioSourceState::AllSound { game_missing: true } => {
                "all sound (game audio not found)".to_owned()
            }
            AudioSourceState::Microphone => "mic".to_owned(),
            AudioSourceState::Unavailable(_) => "no sound".to_owned(),
        });
    }
    (!parts.is_empty()).then(|| parts.join(" + "))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn position_counts_sample_frames_both_ways() {
        let start = Instant::now();
        assert_eq!(position(start, start + Duration::from_millis(10)), 480);
        assert_eq!(position(start + Duration::from_millis(10), start), -480);
    }

    #[test]
    fn describes_tracks() {
        assert_eq!(describe(&[]), None);
        assert_eq!(
            describe(&[AudioSourceState::Game, AudioSourceState::Microphone]).as_deref(),
            Some("game sound + mic")
        );
        assert_eq!(
            describe(&[AudioSourceState::AllSound { game_missing: true }]).as_deref(),
            Some("all sound (game audio not found)")
        );
    }
}
