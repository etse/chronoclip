//! Handles commands from the UI, the tray and the IPC socket, and owns the recorder.
//!
//! Lives on any thread; touches the UI only through `upgrade_in_event_loop`.

use std::sync::{Mutex, OnceLock};

use chronoclip_capture::WindowMatcher;
use chronoclip_core::Paths;
use chronoclip_core::disk::{format_duration, format_size};
use chronoclip_core::ipc::{Request, Response};
use chronoclip_core::settings::Settings;

use crate::recorder::{Recorder, RecorderConfig, RecorderStatus};
use crate::ui::{self, MainWindow, Tray};

/// Set to a window title (substring) to record that window instead of WoW. For testing.
const WINDOW_ENV: &str = "CHRONOCLIP_WINDOW";

#[derive(Default)]
struct RecorderSlot {
    recorder: Option<Recorder>,
    config: Option<RecorderConfig>,
}

pub struct Controller {
    window: slint::Weak<MainWindow>,
    tray: OnceLock<slint::Weak<Tray>>,
    paths: Paths,
    recorder: Mutex<RecorderSlot>,
}

impl Controller {
    pub fn new(window: slint::Weak<MainWindow>, paths: Paths) -> Self {
        Self {
            window,
            tray: OnceLock::new(),
            paths,
            recorder: Mutex::new(RecorderSlot::default()),
        }
    }

    pub fn set_tray(&self, tray: slint::Weak<Tray>) {
        let _ = self.tray.set(tray);
    }

    pub fn handle(&self, request: Request) -> Response {
        match request {
            Request::Ping => Response::Ok("pong".into()),
            Request::Show => {
                self.show_window();
                Response::Ok(String::new())
            }
            Request::Save => self.save_replay(),
            Request::Quit => {
                self.quit();
                Response::Ok("quitting".into())
            }
        }
    }

    pub fn show_window(&self) {
        let result = self.window.upgrade_in_event_loop(|window| {
            if let Err(e) = slint::ComponentHandle::show(&window) {
                tracing::warn!("could not show window: {e}");
            }
        });
        if let Err(e) = result {
            tracing::warn!("event loop is gone: {e}");
        }
    }

    /// Save the replay buffer. Saving arrives in milestone 3, so this reports that for now.
    pub fn save_replay(&self) -> Response {
        let msg = "Saving clips is not implemented yet (milestone 3).";
        tracing::info!("save requested: {msg}");
        let _ = self
            .window
            .upgrade_in_event_loop(move |window| ui::set_message(&window, msg, true));
        Response::Error(msg.into())
    }

    pub fn quit(&self) {
        if let Err(e) = slint::quit_event_loop() {
            tracing::warn!("could not quit event loop: {e}");
        }
    }

    /// Start the recorder, or update/restart it for changed settings. A change of buffer length
    /// alone is applied without interrupting the recording.
    pub fn apply_settings(&self, settings: &Settings) {
        let config = RecorderConfig {
            buffer_dir: settings.buffer_dir(&self.paths),
            buffer_minutes: settings.buffer_minutes,
            fps: settings.fps,
            encoder: settings.encoder,
            matcher: match std::env::var(WINDOW_ENV) {
                Ok(title) if !title.is_empty() => WindowMatcher::TitleContains(title),
                _ => WindowMatcher::WorldOfWarcraft,
            },
        };
        let mut slot = self.recorder.lock().unwrap_or_else(|e| e.into_inner());
        if let (Some(recorder), Some(old)) = (&slot.recorder, &slot.config)
            && (RecorderConfig {
                buffer_minutes: config.buffer_minutes,
                ..old.clone()
            }) == config
        {
            recorder.set_buffer_minutes(config.buffer_minutes);
            slot.config = Some(config);
            return;
        }

        slot.recorder = None; // stops the old recorder and clears its buffer
        let window = self.window.clone();
        let tray = self.tray.get().cloned();
        let matcher = config.matcher.clone();
        slot.recorder = Some(Recorder::start(config.clone(), move |status| {
            let (text, recording) = status_text(&status, &matcher);
            let tray_text = format!("Chronoclip: {text}");
            let _ = window.upgrade_in_event_loop(move |w| {
                w.set_status_text(text.into());
                w.set_recording(recording);
            });
            if let Some(tray) = &tray {
                let _ = tray.upgrade_in_event_loop(move |t| t.set_status(tray_text.into()));
            }
        }));
        slot.config = Some(config);
    }

    /// Stop recording (deletes the buffer). Called on exit.
    pub fn shutdown(&self) {
        let mut slot = self.recorder.lock().unwrap_or_else(|e| e.into_inner());
        slot.recorder = None;
    }
}

fn status_text(status: &RecorderStatus, matcher: &WindowMatcher) -> (String, bool) {
    match status {
        RecorderStatus::WaitingForWindow => match matcher {
            WindowMatcher::WorldOfWarcraft => ("Waiting for World of Warcraft…".into(), false),
            WindowMatcher::TitleContains(t) => {
                (format!("Waiting for a window named “{t}”…"), false)
            }
        },
        RecorderStatus::Recording {
            width,
            height,
            fps,
            buffered,
            retention,
            bytes,
            skipped_frames,
        } => {
            let mut text = format!(
                "Recording {width}×{height} @ {fps} fps · buffer {} / {} · {}",
                format_duration(*buffered),
                format_duration(*retention),
                format_size(*bytes),
            );
            if *skipped_frames > 0 {
                let s = if *skipped_frames == 1 { "" } else { "s" };
                text.push_str(&format!(" · {skipped_frames} frame{s} skipped"));
            }
            (text, true)
        }
        RecorderStatus::Error(e) => (e.clone(), false),
        RecorderStatus::Stopped => ("Stopped".into(), false),
    }
}
