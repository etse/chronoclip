//! Audio capture against the real sound server. Run with `cargo test -- --ignored`; needs
//! PipeWire (pipewire-pulse) or PulseAudio, and `paplay` + `ffmpeg` to play a test tone.
//! The tone is played quietly on the default output.

#![allow(clippy::expect_used)]

use std::process::{Child, Command};
use std::sync::mpsc::{Sender, channel};
use std::time::{Duration, Instant};

use chronoclip_capture::{AudioCapture, AudioSink, AudioSourceKind, AudioSourceState, GameWindow};
use chronoclip_core::Game;

enum Event {
    Samples { peak: f32, frames: usize },
    State(AudioSourceState),
}

struct TestSink(Sender<Event>);

impl AudioSink for TestSink {
    fn samples(&mut self, samples: &[f32], _end: Instant) {
        let peak = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
        let _ = self.0.send(Event::Samples {
            peak,
            frames: samples.len() / 2,
        });
    }
    fn tick(&mut self, _now: Instant) {}
    fn state(&mut self, state: AudioSourceState) {
        let _ = self.0.send(Event::State(state));
    }
}

struct Tone(Child);

impl Drop for Tone {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn play_tone(dir: &std::path::Path) -> Tone {
    let wav = dir.join("tone.wav");
    let status = Command::new("ffmpeg")
        .args(["-loglevel", "error", "-y", "-f", "lavfi", "-i"])
        .arg("sine=frequency=440:duration=10")
        .args(["-af", "volume=0.05"])
        .arg(&wav)
        .status()
        .expect("ffmpeg");
    assert!(status.success());
    Tone(Command::new("paplay").arg(&wav).spawn().expect("paplay"))
}

/// Collect events for `duration`; returns the states seen, captured frames and max peak.
fn collect(
    rx: &std::sync::mpsc::Receiver<Event>,
    duration: Duration,
) -> (Vec<AudioSourceState>, usize, f32) {
    let deadline = Instant::now() + duration;
    let (mut states, mut frames, mut peak) = (Vec::new(), 0, 0.0f32);
    while let Some(left) = deadline.checked_duration_since(Instant::now()) {
        match rx.recv_timeout(left) {
            Ok(Event::State(s)) => states.push(s),
            Ok(Event::Samples { peak: p, frames: n }) => {
                frames += n;
                peak = peak.max(p);
            }
            Err(_) => break,
        }
    }
    (states, frames, peak)
}

#[test]
#[ignore = "needs a sound server, paplay and ffmpeg"]
fn records_one_application_stream() {
    let dir = tempfile::tempdir().expect("tempdir");
    let tone = play_tone(dir.path());
    std::thread::sleep(Duration::from_millis(500));

    let (tx, rx) = channel();
    let capture = AudioCapture::start(AudioSourceKind::Game, TestSink(tx)).expect("start");
    capture.set_game(Some(GameWindow {
        pid: Some(tone.0.id()),
        title: "tone".into(),
        game: Game::by_title("tone"),
    }));
    let (states, frames, peak) = collect(&rx, Duration::from_secs(3));
    drop(capture);

    assert!(
        states.contains(&AudioSourceState::Game),
        "game stream not found: {states:?}"
    );
    // About 2.5 s of audio at 48 kHz once connected, with the tone in it (lavfi's sine is
    // 1/8 amplitude, times 0.05: 0.006).
    assert!(frames > 48_000, "only {frames} frames captured");
    assert!(peak > 0.005, "tone not captured, peak {peak}");
}

#[test]
#[ignore = "needs a sound server"]
fn falls_back_to_all_sound_without_game_stream() {
    let (tx, rx) = channel();
    let capture = AudioCapture::start(AudioSourceKind::Game, TestSink(tx)).expect("start");
    capture.set_game(Some(GameWindow {
        pid: Some(1), // init never plays audio
        title: "no such game".into(),
        game: Game::by_title("no such game"),
    }));
    let (states, frames, _) = collect(&rx, Duration::from_secs(2));
    drop(capture);
    assert!(
        states.contains(&AudioSourceState::AllSound { game_missing: true }),
        "{states:?}"
    );
    assert!(frames > 24_000, "only {frames} frames captured");
}

/// Run while World of Warcraft is running and playing sound: finds its stream from the
/// window, as the recorder does. Fails if WoW isn't running.
#[test]
#[ignore = "needs World of Warcraft running with sound"]
fn finds_running_wow_stream() {
    let window = chronoclip_capture::find_window(&[Game::world_of_warcraft()], None)
        .expect("X11")
        .expect("World of Warcraft is not running");
    let (tx, rx) = channel();
    let capture = AudioCapture::start(AudioSourceKind::Game, TestSink(tx)).expect("start");
    capture.set_game(Some(GameWindow {
        pid: window.pid,
        title: window.title.clone(),
        game: window.game.clone(),
    }));
    let (states, frames, peak) = collect(&rx, Duration::from_secs(3));
    drop(capture);
    eprintln!("{window:?}: states {states:?}, {frames} frames, peak {peak}");
    assert!(states.contains(&AudioSourceState::Game), "{states:?}");
}
