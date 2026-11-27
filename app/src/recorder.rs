//! The recording pipeline: find the window → capture (GPU) → NVENC → segment files, plus
//! audio tracks → AAC into the same segments.
//!
//! Threads:
//! - `recorder`: finds the window, owns the GL context, captures and encodes at the target FPS.
//! - `audio` / `audio-mic`: capture sound and encode it (see [`crate::audio`]).
//! - `writer`: writes packets into segments, adds them to the [`ReplayBuffer`] (which deletes
//!   old segments).
//!
//! They are connected by a bounded channel; if the disk stalls, capture waits (and skips frames)
//! instead of buffering without limit. Saves read the shared [`ReplayBuffer`] from their own
//! thread.
//!
//! Every window session (from finding the window until it closes, hides or resizes) gets an
//! id. Video and audio timestamps count from the session start, and the writer drops packets
//! of a session that has already ended.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use chronoclip_capture::{
    AudioSourceKind, CaptureError, CaptureStatus, CudaDestination, GameWindow, X11WindowCapture,
    find_window,
};
use chronoclip_core::disk::bitrate_bps;
use chronoclip_core::segments::{clean_buffer_dir, retention_for_minutes};
use chronoclip_core::settings::EncoderPreference;
use chronoclip_core::{AudioMode, Game};
use chronoclip_encode::{
    AacEncoder, EncoderConfig, NvencEncoder, Packet, SegmentWriter, StreamConfig,
};

/// Length of one buffer segment (and the keyframe interval).
pub const SEGMENT_SECONDS: u32 = 2;
/// Packets that may queue between encoder and writer (~4 s at 60 fps).
const CHANNEL_CAPACITY: usize = 256;
const WINDOW_POLL: Duration = Duration::from_secs(1);
const ERROR_RETRY: Duration = Duration::from_secs(3);
const STATUS_INTERVAL: Duration = Duration::from_millis(500);
/// How long stopping the recorder waits for a running save before deleting the buffer.
const SAVE_WAIT_ON_STOP: Duration = Duration::from_secs(600);

use crate::audio::{AudioTrack, Session, SessionClock, describe};
use crate::buffer::ReplayBuffer;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecorderConfig {
    pub buffer_dir: PathBuf,
    pub buffer_minutes: u32,
    pub fps: u32,
    pub encoder: EncoderPreference,
    pub audio: AudioMode,
    pub microphone: bool,
}

impl RecorderConfig {
    /// The audio tracks to record, in track order.
    fn audio_sources(&self) -> Vec<AudioSourceKind> {
        let mut sources = Vec::new();
        match self.audio {
            AudioMode::Game => sources.push(AudioSourceKind::Game),
            AudioMode::All => sources.push(AudioSourceKind::AllSound),
            AudioMode::Off => {}
        }
        if self.microphone {
            sources.push(AudioSourceKind::Microphone);
        }
        sources
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecorderStatus {
    WaitingForWindow,
    Recording {
        /// Name of the game being recorded.
        game: String,
        width: u32,
        height: u32,
        fps: u32,
        buffered: Duration,
        retention: Duration,
        bytes: u64,
        skipped_frames: u64,
        /// What the audio tracks record, e.g. "game sound + mic"; `None` without audio.
        audio: Option<String>,
    },
    Error(String),
    /// Recording `game`'s window failed (it is tried again after a pause).
    Failed {
        game: String,
        error: String,
    },
    Stopped,
}

type StatusFn = dyn Fn(RecorderStatus) + Send + Sync;

/// Running recorder. Dropping it stops recording and deletes the buffer.
pub struct Recorder {
    stop: Arc<AtomicBool>,
    retention_secs: Arc<AtomicU64>,
    buffer: Arc<ReplayBuffer>,
    thread: Option<JoinHandle<()>>,
}

impl Recorder {
    /// `current` is the window to stay with: the recorder prefers it when looking for a game
    /// and sets it to the window it records (cleared when no game window is open). Shared, so
    /// it survives restarts and the window picker can point it elsewhere.
    /// `games` is the list of games to look for, in priority order; shared, so it can change
    /// (a game added, removed, newly installed) without restarting the recorder. A session ends
    /// when its game is no longer in the list.
    pub fn start(
        config: RecorderConfig,
        games: Arc<Mutex<Vec<Game>>>,
        current: Arc<Mutex<Option<u64>>>,
        on_status: impl Fn(RecorderStatus) + Send + Sync + 'static,
    ) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let retention = retention_for_minutes(config.buffer_minutes);
        let retention_secs = Arc::new(AtomicU64::new(retention.as_secs()));
        let buffer = Arc::new(ReplayBuffer::new(retention));
        let thread = {
            let stop = Arc::clone(&stop);
            let retention_secs = Arc::clone(&retention_secs);
            let buffer = Arc::clone(&buffer);
            let on_status: Arc<StatusFn> = Arc::new(on_status);
            thread::Builder::new()
                .name("recorder".into())
                .spawn(move || {
                    run(
                        config,
                        &games,
                        &current,
                        &stop,
                        retention_secs,
                        &buffer,
                        on_status,
                    )
                })
        };
        let thread = thread
            .inspect_err(|e| tracing::error!("could not start recorder thread: {e}"))
            .ok();
        Self {
            stop,
            retention_secs,
            buffer,
            thread,
        }
    }

    /// The replay buffer, for saving clips.
    pub fn buffer(&self) -> Arc<ReplayBuffer> {
        Arc::clone(&self.buffer)
    }

    /// Change the buffer length without interrupting the recording.
    pub fn set_buffer_minutes(&self, minutes: u32) {
        self.retention_secs
            .store(retention_for_minutes(minutes).as_secs(), Ordering::Relaxed);
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            tracing::error!("recorder thread panicked");
        }
    }
}

/// Sleep up to `duration`, waking early when `stop` is set. Returns true if stopped.
fn sleep_unless_stopped(stop: &AtomicBool, duration: Duration) -> bool {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        if stop.load(Ordering::Relaxed) {
            return true;
        }
        thread::sleep(Duration::from_millis(50).min(deadline - Instant::now()));
    }
    stop.load(Ordering::Relaxed)
}

pub enum WriterMessage {
    /// A new window session started (e.g. after a resize): start a new segment.
    Session {
        id: u64,
        /// Video first, then the audio tracks.
        streams: Vec<StreamConfig>,
        /// Frame size; segments of the same size can be joined into one clip.
        size: (u32, u32),
    },
    /// An encoded packet of `stream` (0 = video) in session `session`.
    Packet {
        session: u64,
        stream: usize,
        packet: Packet,
    },
    /// The session ended (window closed, hidden or resized): finish the segment.
    EndSession,
    /// A new game started: delete everything recorded before (after a running save).
    Discard,
}

/// `codec::Parameters` is not `Send` in ffmpeg-next, but ours are plain owned copies of
/// AVCodecParameters that are moved to the writer thread and never shared.
pub struct SendMessage(pub WriterMessage);
// SAFETY: see above; the message is moved, never aliased across threads.
#[allow(unsafe_code)]
unsafe impl Send for SendMessage {}

fn run(
    config: RecorderConfig,
    games: &Mutex<Vec<Game>>,
    current: &Mutex<Option<u64>>,
    stop: &AtomicBool,
    retention_secs: Arc<AtomicU64>,
    buffer: &Arc<ReplayBuffer>,
    on_status: Arc<StatusFn>,
) {
    match clean_buffer_dir(&config.buffer_dir) {
        Ok(0) => {}
        Ok(n) => tracing::info!("deleted {n} leftover segments"),
        Err(e) => {
            on_status(RecorderStatus::Error(format!(
                "Cannot use buffer folder {}: {e}",
                config.buffer_dir.display()
            )));
            return;
        }
    }

    if !matches!(
        config.encoder,
        EncoderPreference::Auto | EncoderPreference::Nvenc
    ) {
        on_status(RecorderStatus::Error(
            "Only the NVENC encoder is supported so far (VAAPI and software come in milestone 9)."
                .into(),
        ));
        while !sleep_unless_stopped(stop, Duration::from_secs(1)) {}
        on_status(RecorderStatus::Stopped);
        return;
    }

    let (sender, receiver) = sync_channel::<SendMessage>(CHANNEL_CAPACITY);
    let writer = {
        let dir = config.buffer_dir.clone();
        let buffer = Arc::clone(buffer);
        let retention_secs = Arc::clone(&retention_secs);
        thread::Builder::new()
            .name("writer".into())
            .spawn(move || write_segments(&dir, &receiver, &retention_secs, &buffer))
    };
    let writer = match writer {
        Ok(w) => w,
        Err(e) => {
            on_status(RecorderStatus::Error(format!(
                "could not start writer: {e}"
            )));
            return;
        }
    };

    // Audio tracks run for the whole recorder lifetime; they write only while a window
    // session is active.
    let clock = Arc::new(SessionClock::default());
    let tracks = start_audio(&config, &clock, &sender);
    buffer.set_audio_tracks(tracks.iter().map(|t| t.title.clone()).collect());
    let audio_streams: Vec<StreamConfig> = match tracks.first() {
        Some(_) => match AacEncoder::new() {
            Ok(aac) => vec![
                StreamConfig {
                    params: aac.parameters(),
                    time_base: aac.time_base(),
                };
                tracks.len()
            ],
            Err(_) => Vec::new(), // start_audio already checked this
        },
        None => Vec::new(),
    };

    let mut session_id = 0;
    // The game that was recorded last, and whether its window has disappeared since. A clip
    // only contains the current game: when the game is started again, the old recording is
    // dropped. (Not when it closes, so the end of a game can still be saved afterwards.)
    let mut last_game: Option<Option<u32>> = None;
    let mut window_gone = false;
    while !stop.load(Ordering::Relaxed) {
        let prefer = *current.lock().unwrap_or_else(|e| e.into_inner());
        let list = games.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let window = match find_window(&list, prefer) {
            Ok(Some(w)) if w.viewable => w,
            Ok(found) => {
                if found.is_none() {
                    *current.lock().unwrap_or_else(|e| e.into_inner()) = None;
                    if last_game.is_some() {
                        window_gone = true;
                    }
                }
                on_status(RecorderStatus::WaitingForWindow);
                sleep_unless_stopped(stop, WINDOW_POLL);
                continue;
            }
            Err(e) => {
                on_status(RecorderStatus::Error(format!(
                    "Cannot look for windows: {e}"
                )));
                sleep_unless_stopped(stop, ERROR_RETRY);
                continue;
            }
        };
        tracing::info!(
            game = %window.game.name,
            window = format!("0x{:x}", window.id),
            title = %window.title,
            pid = ?window.pid,
            width = window.width,
            height = window.height,
            "found window"
        );
        let restarted = window_gone || last_game.is_some_and(|pid| pid != window.pid);
        if restarted {
            tracing::info!("the game was restarted: dropping the recording of the previous one");
            let _ = sender.send(SendMessage(WriterMessage::Discard));
        }
        last_game = Some(window.pid);
        window_gone = false;
        for track in &tracks {
            track.set_game(Some(GameWindow {
                pid: window.pid,
                title: window.title.clone(),
                game: window.game.clone(),
            }));
        }
        *current.lock().unwrap_or_else(|e| e.into_inner()) = Some(window.id);
        session_id += 1;
        let game = window.game.name.clone();
        let result = record_window(
            &config,
            Window {
                id: window.id,
                size: (window.width, window.height),
                session: session_id,
                game: window.game.name.clone(),
            },
            stop,
            &sender,
            Audio {
                clock: &clock,
                streams: &audio_streams,
                tracks: &tracks,
            },
            buffer,
            &retention_secs,
            games,
            on_status.as_ref(),
        );
        // Close the open segment so it can be saved while no window is recorded.
        let _ = sender.send(SendMessage(WriterMessage::EndSession));
        match result {
            Ok(()) => {}
            Err(e) => {
                tracing::warn!(%game, "recording stopped: {e}");
                on_status(RecorderStatus::Failed { game, error: e });
                sleep_unless_stopped(stop, ERROR_RETRY);
            }
        }
    }

    drop(tracks); // stops the audio threads, which hold senders too
    drop(sender); // writer finishes its segment and exits
    if writer.join().is_err() {
        tracing::error!("writer thread panicked");
    }
    // A save in progress reads the segments; let it finish before deleting them.
    if !buffer.wait_until_unpinned(SAVE_WAIT_ON_STOP) {
        tracing::warn!("a save is still running; deleting the buffer anyway");
    }
    buffer.clear();
    match clean_buffer_dir(&config.buffer_dir) {
        Ok(n) => tracing::info!("recorder stopped, deleted {n} buffer segments"),
        Err(e) => tracing::warn!("could not clean buffer folder: {e}"),
    }
    on_status(RecorderStatus::Stopped);
}

/// Start the audio tracks the settings ask for. A track that can't start is left out (and
/// logged), so the recording still works without it.
fn start_audio(
    config: &RecorderConfig,
    clock: &Arc<SessionClock>,
    sender: &SyncSender<SendMessage>,
) -> Vec<AudioTrack> {
    let sources = config.audio_sources();
    if sources.is_empty() {
        return Vec::new();
    }
    if let Err(e) = AacEncoder::new() {
        tracing::error!("recording without sound: the AAC encoder is not available: {e}");
        return Vec::new();
    }
    let mut tracks = Vec::new();
    for kind in sources {
        let index = tracks.len() + 1; // stream 0 is the video
        match AudioTrack::start(kind, index, Arc::clone(clock), sender.clone()) {
            Ok(track) => tracks.push(track),
            Err(e) => tracing::error!(?kind, "could not start audio capture: {e}"),
        }
    }
    tracks
}

/// The window to record and the id of this recording session.
struct Window {
    id: u64,
    size: (u32, u32),
    session: u64,
    /// Name of the game it belongs to.
    game: String,
}

/// The audio side of a session.
struct Audio<'a> {
    clock: &'a SessionClock,
    streams: &'a [StreamConfig],
    tracks: &'a [AudioTrack],
}

/// Marks the session as active for the audio tracks; ends it when dropped.
struct ActiveSession<'a>(&'a SessionClock);

impl Drop for ActiveSession<'_> {
    fn drop(&mut self) {
        self.0.set(None);
    }
}

/// Record one window until it closes, resizes, becomes invisible, or `stop` is set.
/// `Ok(())` means "look for the window again".
#[allow(clippy::too_many_arguments)]
fn record_window(
    config: &RecorderConfig,
    window: Window,
    stop: &AtomicBool,
    sender: &SyncSender<SendMessage>,
    audio: Audio,
    buffer: &ReplayBuffer,
    retention_secs: &AtomicU64,
    games: &Mutex<Vec<Game>>,
    on_status: &StatusFn,
) -> Result<(), String> {
    let (width, height) = window.size;
    let fps = config.fps;
    let mut encoder = NvencEncoder::new(EncoderConfig {
        width,
        height,
        fps,
        bitrate_bps: bitrate_bps(width, height, fps),
        gop_frames: SEGMENT_SECONDS * fps,
    })
    .map_err(|e| format!("Cannot start the NVENC encoder: {e}"))?;
    let mut capture = match X11WindowCapture::new(window.id, encoder.cuda_context()) {
        Ok(c) => c,
        Err(CaptureError::WindowGone | CaptureError::WindowNotViewable) => return Ok(()),
        Err(e) => return Err(format!("Cannot capture the window: {e}")),
    };
    let params = encoder.parameters().map_err(|e| e.to_string())?;
    let send = |msg| {
        sender
            .send(SendMessage(msg))
            .map_err(|_| "writer stopped".to_owned())
    };
    let mut streams = vec![StreamConfig {
        params,
        time_base: encoder.time_base(),
    }];
    streams.extend_from_slice(audio.streams);
    send(WriterMessage::Session {
        id: window.session,
        streams,
        size: (width, height),
    })?;

    let interval = Duration::from_secs(1) / fps;
    let start = Instant::now();
    // From now on the audio tracks encode with timestamps relative to `start`, the instant of
    // video frame 0. Published after the Session message, so the writer knows the session
    // before the first audio packet arrives.
    audio.clock.set(Some(Session {
        id: window.session,
        start,
    }));
    let _active = ActiveSession(audio.clock);
    let video_packet = |packet| WriterMessage::Packet {
        session: window.session,
        stream: 0,
        packet,
    };
    let mut frame_number: u64 = 0;
    let mut skipped: u64 = 0;
    let mut packets = Vec::new();
    let mut last_status = Instant::now() - STATUS_INTERVAL;

    let result = loop {
        if stop.load(Ordering::Relaxed) {
            break Ok(());
        }
        let frame = encoder.alloc_frame().map_err(|e| e.to_string())?;
        let destination = CudaDestination {
            context: encoder.cuda_context(),
            device_ptr: frame.device_ptr(),
            pitch: frame.pitch(),
        };
        match capture.capture(destination) {
            Ok(CaptureStatus::Captured) => {}
            Ok(CaptureStatus::Resized { width, height }) => {
                tracing::info!(width, height, "window resized, restarting capture");
                break Ok(());
            }
            Err(CaptureError::WindowGone | CaptureError::WindowNotViewable) => {
                tracing::info!("window closed or hidden");
                break Ok(());
            }
            Err(e) => break Err(format!("Capture failed: {e}")),
        }
        let pts = i64::try_from(frame_number).unwrap_or(i64::MAX);
        encoder
            .encode(Some((frame, pts)), &mut packets)
            .map_err(|e| e.to_string())?;
        for packet in packets.drain(..) {
            send(video_packet(packet))?;
        }

        // Pace to wall-clock time. If we fell behind, skip frame numbers so timestamps stay real-time.
        frame_number += 1;
        let next = start + interval * u32::try_from(frame_number).unwrap_or(u32::MAX);
        let now = Instant::now();
        if let Some(wait) = next.checked_duration_since(now) {
            thread::sleep(wait);
        } else {
            let due = (now - start).as_nanos() / interval.as_nanos().max(1);
            let due = u64::try_from(due).unwrap_or(u64::MAX);
            if due > frame_number {
                skipped += due - frame_number;
                frame_number = due;
            }
        }

        if last_status.elapsed() >= STATUS_INTERVAL {
            last_status = Instant::now();
            // Removed from the games (or switched off): stop recording it.
            let listed = games
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
                .any(|g| g.name == window.game);
            if !listed {
                tracing::info!(game = %window.game, "no longer in the games list: stop recording it");
                break Ok(());
            }
            let (buffered, bytes) = buffer.stats();
            on_status(RecorderStatus::Recording {
                game: window.game.clone(),
                width,
                height,
                fps,
                buffered,
                retention: Duration::from_secs(retention_secs.load(Ordering::Relaxed)),
                bytes,
                skipped_frames: skipped,
                audio: describe(
                    &audio
                        .tracks
                        .iter()
                        .map(AudioTrack::state)
                        .collect::<Vec<_>>(),
                ),
            });
        }
    };

    // Flush the encoder so the last frames reach the buffer.
    if encoder.encode(None, &mut packets).is_ok() {
        for packet in packets.drain(..) {
            send(video_packet(packet))?;
        }
    }
    if skipped > 0 {
        tracing::info!(
            skipped,
            "frames skipped because capture or encoding fell behind"
        );
    }
    result
}

fn write_segments(
    dir: &std::path::Path,
    receiver: &Receiver<SendMessage>,
    retention_secs: &AtomicU64,
    buffer: &ReplayBuffer,
) {
    let mut writer: Option<SegmentWriter> = None;
    let mut next_seq = 0;
    // Stream id and frame size of the current session; a new size starts a new stream.
    let mut stream: Option<(u64, (u32, u32))> = None;
    // Packets of other sessions (audio arriving after its session ended) are dropped.
    let mut session: Option<u64> = None;

    let finish = |writer: &mut SegmentWriter| {
        match writer.finish() {
            Ok(Some(segment)) => buffer.add_segment(
                segment,
                Duration::from_secs(retention_secs.load(Ordering::Relaxed)),
            ),
            Ok(None) => {}
            Err(e) => tracing::warn!("could not finish segment: {e}"),
        }
        buffer.set_open_segment(None);
    };

    while let Ok(SendMessage(message)) = receiver.recv() {
        match message {
            WriterMessage::Session { id, streams, size } => {
                session = Some(id);
                if let Some(mut old) = writer.take() {
                    finish(&mut old);
                    next_seq = old.next_seq();
                }
                let id = match stream {
                    Some((id, old_size)) if old_size == size => id,
                    Some((id, _)) => id + 1,
                    None => 0,
                };
                stream = Some((id, size));
                writer = Some(SegmentWriter::new(
                    dir,
                    streams,
                    Duration::from_secs(SEGMENT_SECONDS.into()),
                    id,
                    next_seq,
                ));
            }
            WriterMessage::Packet {
                session: packet_session,
                stream: index,
                packet,
            } => {
                if session != Some(packet_session) {
                    continue;
                }
                let Some(w) = writer.as_mut() else { continue };
                match w.write(packet, index) {
                    Ok(Some(segment)) => buffer.add_segment(
                        segment,
                        Duration::from_secs(retention_secs.load(Ordering::Relaxed)),
                    ),
                    Ok(None) => {}
                    Err(e) => tracing::warn!("could not write packet: {e}"),
                }
                buffer.set_open_segment(w.current_seq());
            }
            WriterMessage::Discard => {
                if let Some(w) = writer.as_mut() {
                    finish(w);
                }
                // A save in progress is reading the old segments; let it finish first.
                if !buffer.wait_until_unpinned(SAVE_WAIT_ON_STOP) {
                    tracing::warn!("a save is still running; dropping the old recording anyway");
                }
                buffer.discard();
            }
            WriterMessage::EndSession => {
                session = None;
                if let Some(w) = writer.as_mut() {
                    finish(w);
                }
            }
        }
    }
    if let Some(mut w) = writer {
        finish(&mut w);
    }
    // The recorder thread deletes the files (clean_buffer_dir) after we exit.
}
