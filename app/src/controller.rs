//! Handles commands from the UI, the tray and the IPC socket, and owns the recorder.
//!
//! Lives on any thread; touches the UI only through `upgrade_in_event_loop`.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, mpsc};

use chronoclip_capture::WindowMatcher;
use chronoclip_core::disk::{format_duration, format_size};
use chronoclip_core::ipc::{Request, Response};
use chronoclip_core::settings::Settings;
use chronoclip_core::{Hotkey, Paths};

use crate::hotkey::GlobalHotkey;
use crate::notify::Alerts;
use crate::recorder::{Recorder, RecorderConfig, RecorderStatus};
use crate::save::{self, SavedClip};
use crate::ui::{MainWindow, Tray, TrayState};

/// Set to a window title (substring) to record that window instead of WoW. For testing.
const WINDOW_ENV: &str = "CHRONOCLIP_WINDOW";

#[derive(Default)]
struct RecorderSlot {
    recorder: Option<Recorder>,
    config: Option<RecorderConfig>,
}

struct HotkeySlot {
    backend: GlobalHotkey,
    /// The hotkey currently grabbed, if registering it worked.
    registered: Option<Hotkey>,
}

/// Save progress and result, shown in the window and the tray tooltip.
#[derive(Clone)]
struct SaveFeedback {
    window: slint::Weak<MainWindow>,
    /// Line for the tray tooltip (shown with the next status update) about the last save.
    last_save: Arc<Mutex<Option<String>>>,
    alerts: Arc<Alerts>,
    /// The "play a sound when saving" setting.
    sound: Arc<AtomicBool>,
}

impl SaveFeedback {
    fn started(&self) {
        let _ = self.window.upgrade_in_event_loop(|w| {
            w.set_saving(true);
            w.set_save_text("Saving replay…".into());
            w.set_save_is_error(false);
        });
    }

    fn finished(&self, result: &Result<SavedClip, String>) {
        let (text, tray_line, is_error) = match result {
            Ok(clip) => (
                format!(
                    "Saved {} in {:.1} s",
                    clip.summary(),
                    clip.elapsed.as_secs_f64()
                ),
                format!("Last clip: {}", clip.file_name()),
                false,
            ),
            Err(e) => (e.clone(), format!("Save failed: {e}"), true),
        };
        *self.last_save.lock().unwrap_or_else(|e| e.into_inner()) = Some(tray_line);
        let sound = self.sound.load(Ordering::Relaxed);
        match result {
            Ok(clip) => self.alerts.clip_saved(clip, sound),
            Err(e) => self.alerts.save_failed(e, sound),
        }
        let _ = self.window.upgrade_in_event_loop(move |w| {
            w.set_saving(false);
            w.set_save_text(text.into());
            w.set_save_is_error(is_error);
        });
    }

    /// A save that didn't start (busy, not recording).
    fn refused(&self, message: String) {
        self.alerts
            .save_failed(&message, self.sound.load(Ordering::Relaxed));
        let _ = self.window.upgrade_in_event_loop(move |w| {
            w.set_save_text(message.into());
            w.set_save_is_error(true);
        });
    }
}

pub struct Controller {
    window: slint::Weak<MainWindow>,
    tray: OnceLock<slint::Weak<Tray>>,
    paths: Paths,
    recorder: Mutex<RecorderSlot>,
    hotkey: Mutex<Option<HotkeySlot>>,
    output_dir: Mutex<PathBuf>,
    saving: Arc<AtomicBool>,
    last_save: Arc<Mutex<Option<String>>>,
    alerts: Arc<Alerts>,
    save_sound: Arc<AtomicBool>,
}

impl Controller {
    pub fn new(window: slint::Weak<MainWindow>, paths: Paths) -> Self {
        let output_dir = paths.default_output_dir.clone();
        let cache_dir = paths
            .default_buffer_dir
            .parent()
            .unwrap_or(&paths.default_buffer_dir)
            .to_owned();
        let alerts = Arc::new(Alerts::new(&paths.home_dir, &cache_dir));
        Self {
            window,
            tray: OnceLock::new(),
            paths,
            recorder: Mutex::new(RecorderSlot::default()),
            hotkey: Mutex::new(None),
            output_dir: Mutex::new(output_dir),
            saving: Arc::new(AtomicBool::new(false)),
            last_save: Arc::new(Mutex::new(None)),
            alerts,
            save_sound: Arc::new(AtomicBool::new(true)),
        }
    }

    pub fn set_tray(&self, tray: slint::Weak<Tray>) {
        let _ = self.tray.set(tray);
    }

    /// Use `backend` for the global hotkey. The hotkey itself is registered by `apply_settings`.
    pub fn set_hotkey_backend(&self, backend: GlobalHotkey) {
        *self.hotkey.lock().unwrap_or_else(|e| e.into_inner()) = Some(HotkeySlot {
            backend,
            registered: None,
        });
    }

    fn feedback(&self) -> SaveFeedback {
        SaveFeedback {
            window: self.window.clone(),
            last_save: Arc::clone(&self.last_save),
            alerts: Arc::clone(&self.alerts),
            sound: Arc::clone(&self.save_sound),
        }
    }

    pub fn handle(&self, request: Request) -> Response {
        match request {
            Request::Ping => Response::Ok("pong".into()),
            Request::Show => {
                self.show_window();
                Response::Ok(String::new())
            }
            Request::Save => self.save_and_wait(),
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

    /// Save the replay buffer in the background (window button, tray, hotkey). Progress and
    /// result are shown in the window and tray tooltip.
    pub fn save_replay(&self) {
        if let Err(message) = self.start_save(|_| {}) {
            tracing::info!("save not started: {message}");
            self.feedback().refused(message);
        }
    }

    /// Save and wait for the result (`chronoclip save`). Returns the clip path.
    fn save_and_wait(&self) -> Response {
        let (sender, receiver) = mpsc::sync_channel(1);
        if let Err(message) = self.start_save(move |result| {
            let _ = sender.send(result.clone());
        }) {
            return Response::Error(message);
        }
        match receiver.recv() {
            Ok(Ok(clip)) => Response::Ok(clip.path.display().to_string()),
            Ok(Err(message)) => Response::Error(message),
            Err(_) => Response::Error("The save stopped unexpectedly.".into()),
        }
    }

    /// Start a save on its own thread; `done` runs there with the result. Fails at once if a
    /// save is already running or nothing is being recorded.
    fn start_save(
        &self,
        done: impl FnOnce(&Result<SavedClip, String>) + Send + 'static,
    ) -> Result<(), String> {
        if self.saving.swap(true, Ordering::SeqCst) {
            return Err("Already saving a replay. Try again when it is done.".into());
        }
        let buffer = self
            .recorder
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .recorder
            .as_ref()
            .map(Recorder::buffer);
        let Some(buffer) = buffer else {
            self.saving.store(false, Ordering::SeqCst);
            return Err("Not recording, so there is nothing to save.".into());
        };
        let output_dir = self
            .output_dir
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let saving = Arc::clone(&self.saving);
        let feedback = self.feedback();
        feedback.started();
        let spawned = std::thread::Builder::new()
            .name("save".into())
            .spawn(move || {
                let result = save::save_replay(&buffer, &output_dir);
                if let Err(e) = &result {
                    tracing::warn!("save failed: {e}");
                }
                saving.store(false, Ordering::SeqCst);
                feedback.finished(&result);
                done(&result);
            });
        spawned.map(drop).map_err(|e| {
            self.saving.store(false, Ordering::SeqCst);
            let message = format!("Could not start saving: {e}");
            self.feedback().finished(&Err(message.clone()));
            message
        })
    }

    /// Register `hotkey` unless it is already active; shows a problem next to the setting.
    fn apply_hotkey(&self, hotkey: Hotkey) {
        let mut slot = self.hotkey.lock().unwrap_or_else(|e| e.into_inner());
        let Some(slot) = slot.as_mut() else { return };
        if slot.registered == Some(hotkey) {
            return;
        }
        let result = slot.backend.set(hotkey);
        slot.registered = result.as_ref().ok().map(|()| hotkey);
        let problem = result.err().unwrap_or_default();
        if !problem.is_empty() {
            tracing::warn!("hotkey: {problem}");
        }
        let _ = self
            .window
            .upgrade_in_event_loop(move |w| w.set_hotkey_error(problem.into()));
    }

    pub fn quit(&self) {
        if let Err(e) = slint::quit_event_loop() {
            tracing::warn!("could not quit event loop: {e}");
        }
    }

    /// Start the recorder, or update/restart it for changed settings. A change of buffer length
    /// alone is applied without interrupting the recording.
    pub fn apply_settings(&self, settings: &Settings) {
        self.apply_hotkey(settings.hotkey);
        self.save_sound
            .store(settings.save_sound, Ordering::Relaxed);
        *self.output_dir.lock().unwrap_or_else(|e| e.into_inner()) =
            settings.output_dir(&self.paths);

        let config = RecorderConfig {
            buffer_dir: settings.buffer_dir(&self.paths),
            buffer_minutes: settings.buffer_minutes,
            fps: settings.fps,
            encoder: settings.encoder,
            audio: settings.audio,
            microphone: settings.microphone,
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

        // Stops the old recorder and clears its buffer. A running save finishes first.
        slot.recorder = None;
        let window = self.window.clone();
        let tray = self.tray.get().cloned();
        let matcher = config.matcher.clone();
        let last_save = Arc::clone(&self.last_save);
        slot.recorder = Some(Recorder::start(config.clone(), move |status| {
            let (text, recording) = status_text(&status, &matcher);
            let state = match status {
                RecorderStatus::Recording { .. } => TrayState::Recording,
                RecorderStatus::Error(_) => TrayState::Error,
                RecorderStatus::WaitingForWindow | RecorderStatus::Stopped => TrayState::Waiting,
            };
            let mut tray_text = format!("Chronoclip: {text}");
            if let Some(line) = &*last_save.lock().unwrap_or_else(|e| e.into_inner()) {
                tray_text.push('\n');
                tray_text.push_str(line);
            }
            let _ = window.upgrade_in_event_loop(move |w| {
                w.set_status_text(text.into());
                w.set_recording(recording);
            });
            if let Some(tray) = &tray {
                let _ = tray.upgrade_in_event_loop(move |t| {
                    t.set_status(tray_text.into());
                    t.set_state(state);
                });
            }
        }));
        slot.config = Some(config);
    }

    /// Release the hotkey and stop recording (deletes the buffer once a running save is done).
    /// Called on exit.
    pub fn shutdown(&self) {
        drop(self.hotkey.lock().unwrap_or_else(|e| e.into_inner()).take());
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
            audio,
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
            if let Some(audio) = audio {
                text.push_str(" · ");
                text.push_str(audio);
            }
            (text, true)
        }
        RecorderStatus::Error(e) => (e.clone(), false),
        RecorderStatus::Stopped => ("Stopped".into(), false),
    }
}
