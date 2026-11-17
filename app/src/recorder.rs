//! The recording pipeline: find the window → capture (GPU) → NVENC → segment files.
//!
//! Threads:
//! - `recorder`: finds the window, owns the GL context, captures and encodes at the target FPS.
//! - `writer`: writes packets into segments, keeps the segment index and deletes old segments.
//!
//! They are connected by a bounded channel; if the disk stalls, capture waits (and skips frames)
//! instead of buffering without limit.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use chronoclip_capture::{
    CaptureError, CaptureStatus, CudaDestination, WindowMatcher, X11WindowCapture, find_window,
};
use chronoclip_core::disk::bitrate_bps;
use chronoclip_core::segments::{SegmentIndex, clean_buffer_dir, retention_for_minutes};
use chronoclip_core::settings::EncoderPreference;
use chronoclip_encode::{EncoderConfig, NvencEncoder, Packet, SegmentWriter};
use ffmpeg_next::{Rational, codec};

/// Length of one buffer segment (and the keyframe interval).
pub const SEGMENT_SECONDS: u32 = 2;
/// Packets that may queue between encoder and writer (~4 s at 60 fps).
const CHANNEL_CAPACITY: usize = 256;
const WINDOW_POLL: Duration = Duration::from_secs(1);
const ERROR_RETRY: Duration = Duration::from_secs(3);
const STATUS_INTERVAL: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecorderConfig {
    pub buffer_dir: PathBuf,
    pub buffer_minutes: u32,
    pub fps: u32,
    pub encoder: EncoderPreference,
    pub matcher: WindowMatcher,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecorderStatus {
    WaitingForWindow,
    Recording {
        width: u32,
        height: u32,
        fps: u32,
        buffered: Duration,
        retention: Duration,
        bytes: u64,
        skipped_frames: u64,
    },
    Error(String),
    Stopped,
}

type StatusFn = dyn Fn(RecorderStatus) + Send + Sync;

/// Running recorder. Dropping it stops recording and deletes the buffer.
pub struct Recorder {
    stop: Arc<AtomicBool>,
    retention_secs: Arc<AtomicU64>,
    thread: Option<JoinHandle<()>>,
}

impl Recorder {
    pub fn start(
        config: RecorderConfig,
        on_status: impl Fn(RecorderStatus) + Send + Sync + 'static,
    ) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let retention_secs = Arc::new(AtomicU64::new(
            retention_for_minutes(config.buffer_minutes).as_secs(),
        ));
        let thread = {
            let stop = Arc::clone(&stop);
            let retention_secs = Arc::clone(&retention_secs);
            let on_status: Arc<StatusFn> = Arc::new(on_status);
            thread::Builder::new()
                .name("recorder".into())
                .spawn(move || run(config, &stop, retention_secs, on_status))
        };
        let thread = thread
            .inspect_err(|e| tracing::error!("could not start recorder thread: {e}"))
            .ok();
        Self {
            stop,
            retention_secs,
            thread,
        }
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

/// Shared between recorder and writer threads for status reporting.
#[derive(Default)]
struct BufferStats {
    buffered: Duration,
    bytes: u64,
}

enum WriterMessage {
    /// A new encoder session started (e.g. after a resize): start a new segment.
    Session {
        params: codec::Parameters,
        time_base: Rational,
    },
    Packet(Packet),
}

/// `codec::Parameters` is not `Send` in ffmpeg-next, but ours is a plain owned copy of
/// AVCodecParameters that is moved to the writer thread and never shared.
struct SendMessage(WriterMessage);
// SAFETY: see above; the message is moved, never aliased across threads.
#[allow(unsafe_code)]
unsafe impl Send for SendMessage {}

fn run(
    config: RecorderConfig,
    stop: &AtomicBool,
    retention_secs: Arc<AtomicU64>,
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
            "Only the NVENC encoder is supported so far (VAAPI and software come in milestone 6)."
                .into(),
        ));
        while !sleep_unless_stopped(stop, Duration::from_secs(1)) {}
        on_status(RecorderStatus::Stopped);
        return;
    }

    let stats = Arc::new(Mutex::new(BufferStats::default()));
    let (sender, receiver) = sync_channel::<SendMessage>(CHANNEL_CAPACITY);
    let writer = {
        let dir = config.buffer_dir.clone();
        let stats = Arc::clone(&stats);
        let retention_secs = Arc::clone(&retention_secs);
        thread::Builder::new()
            .name("writer".into())
            .spawn(move || write_segments(&dir, &receiver, &retention_secs, &stats))
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

    while !stop.load(Ordering::Relaxed) {
        let window = match find_window(&config.matcher) {
            Ok(Some(w)) if w.viewable => w,
            Ok(_) => {
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
            window = format!("0x{:x}", window.id),
            title = %window.title,
            pid = ?window.pid,
            width = window.width,
            height = window.height,
            "found window"
        );
        match record_window(
            &config,
            window.id,
            (window.width, window.height),
            stop,
            &sender,
            &stats,
            &retention_secs,
            on_status.as_ref(),
        ) {
            Ok(()) => {}
            Err(e) => {
                tracing::warn!("recording stopped: {e}");
                on_status(RecorderStatus::Error(e));
                sleep_unless_stopped(stop, ERROR_RETRY);
            }
        }
    }

    drop(sender); // writer finishes its segment and exits
    if writer.join().is_err() {
        tracing::error!("writer thread panicked");
    }
    match clean_buffer_dir(&config.buffer_dir) {
        Ok(n) => tracing::info!("recorder stopped, deleted {n} buffer segments"),
        Err(e) => tracing::warn!("could not clean buffer folder: {e}"),
    }
    on_status(RecorderStatus::Stopped);
}

/// Record one window until it closes, resizes, becomes invisible, or `stop` is set.
/// `Ok(())` means "look for the window again".
#[allow(clippy::too_many_arguments)]
fn record_window(
    config: &RecorderConfig,
    window: u64,
    (width, height): (u32, u32),
    stop: &AtomicBool,
    sender: &SyncSender<SendMessage>,
    stats: &Mutex<BufferStats>,
    retention_secs: &AtomicU64,
    on_status: &StatusFn,
) -> Result<(), String> {
    let fps = config.fps;
    let mut encoder = NvencEncoder::new(EncoderConfig {
        width,
        height,
        fps,
        bitrate_bps: bitrate_bps(width, height, fps),
        gop_frames: SEGMENT_SECONDS * fps,
    })
    .map_err(|e| format!("Cannot start the NVENC encoder: {e}"))?;
    let mut capture = match X11WindowCapture::new(window, encoder.cuda_context()) {
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
    send(WriterMessage::Session {
        params,
        time_base: encoder.time_base(),
    })?;

    let interval = Duration::from_secs(1) / fps;
    let start = Instant::now();
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
            send(WriterMessage::Packet(packet))?;
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
            let stats = stats.lock().unwrap_or_else(|e| e.into_inner());
            on_status(RecorderStatus::Recording {
                width,
                height,
                fps,
                buffered: stats.buffered,
                retention: Duration::from_secs(retention_secs.load(Ordering::Relaxed)),
                bytes: stats.bytes,
                skipped_frames: skipped,
            });
        }
    };

    // Flush the encoder so the last frames reach the buffer.
    if encoder.encode(None, &mut packets).is_ok() {
        for packet in packets.drain(..) {
            send(WriterMessage::Packet(packet))?;
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
    stats: &Mutex<BufferStats>,
) {
    let mut index = SegmentIndex::new(Duration::from_secs(retention_secs.load(Ordering::Relaxed)));
    let mut writer: Option<SegmentWriter> = None;
    let mut next_seq = 0;

    let finish_segment = |index: &mut SegmentIndex, segment| {
        let retention = Duration::from_secs(retention_secs.load(Ordering::Relaxed));
        let mut removed = if retention != index.retention() {
            index.set_retention(retention)
        } else {
            Vec::new()
        };
        removed.extend(index.push(segment));
        for old in removed {
            if let Err(e) = std::fs::remove_file(&old.path) {
                tracing::warn!("could not delete {}: {e}", old.path.display());
            }
        }
        let mut stats = stats.lock().unwrap_or_else(|e| e.into_inner());
        stats.buffered = index.total_duration();
        stats.bytes = index.total_bytes();
    };

    while let Ok(SendMessage(message)) = receiver.recv() {
        match message {
            WriterMessage::Session { params, time_base } => {
                if let Some(mut old) = writer.take() {
                    match old.finish() {
                        Ok(Some(segment)) => finish_segment(&mut index, segment),
                        Ok(None) => {}
                        Err(e) => tracing::warn!("could not finish segment: {e}"),
                    }
                    next_seq = old.next_seq();
                }
                writer = Some(SegmentWriter::new(
                    dir,
                    params,
                    time_base,
                    Duration::from_secs(SEGMENT_SECONDS.into()),
                    next_seq,
                ));
            }
            WriterMessage::Packet(packet) => {
                let Some(w) = writer.as_mut() else { continue };
                match w.write(packet) {
                    Ok(Some(segment)) => finish_segment(&mut index, segment),
                    Ok(None) => {}
                    Err(e) => tracing::warn!("could not write packet: {e}"),
                }
            }
        }
    }
    if let Some(mut w) = writer
        && let Ok(Some(segment)) = w.finish()
    {
        finish_segment(&mut index, segment);
    }
    // Files are deleted by the recorder thread (clean_buffer_dir) after we exit.
    let _ = index.clear();
}
