//! Handles commands from the UI, the tray and the IPC socket, and owns the recorder.
//!
//! Lives on any thread; touches the UI only through `upgrade_in_event_loop`.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, mpsc};

use chronoclip_core::Game;
use chronoclip_core::disk::{format_duration, format_size};
use chronoclip_core::ipc::{Request, Response};
use chronoclip_core::settings::Settings;
use chronoclip_core::{Hotkey, Paths};

use crate::hotkey::GlobalHotkey;
use crate::notify::Alerts;
use crate::recorder::{Recorder, RecorderConfig, RecorderStatus};
use crate::save::{self, SavedClip};
use crate::ui::{MainWindow, RecState, Tray, TrayState};

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
    /// The clip of the last successful save (for Play / Show in folder on the Home page).
    last_clip: Arc<Mutex<Option<PathBuf>>>,
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
            Ok(clip) => {
                self.alerts.clip_saved(clip, sound);
                *self.last_clip.lock().unwrap_or_else(|e| e.into_inner()) = Some(clip.path.clone());
                let name = clip.file_name();
                let detail = format!(
                    "{} · {}",
                    format_duration(clip.duration),
                    format_size(clip.bytes)
                );
                let _ = self.window.upgrade_in_event_loop(move |w| {
                    w.set_has_last_clip(true);
                    w.set_last_clip_name(name.into());
                    w.set_last_clip_detail(detail.into());
                });
            }
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
    last_clip: Arc<Mutex<Option<PathBuf>>>,
    alerts: Arc<Alerts>,
    save_sound: Arc<AtomicBool>,
    /// A game just picked in the window picker: kept if it starts recording, removed again
    /// (through the window's `game-failed`) if recording it fails first.
    trial: Arc<Mutex<Option<String>>>,
    /// The window being recorded (or picked to be): the recorder stays with it until it
    /// closes, also across restarts for changed settings. See [`Recorder::start`].
    current_window: Arc<Mutex<Option<u64>>>,
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
            last_clip: Arc::new(Mutex::new(None)),
            alerts,
            save_sound: Arc::new(AtomicBool::new(true)),
            trial: Arc::new(Mutex::new(None)),
            current_window: Arc::new(Mutex::new(None)),
        }
    }

    /// Watch whether `game` (just added from the window picker) can be recorded. Call before
    /// applying the settings that add it.
    pub fn try_game(&self, game: &str) {
        *self.trial.lock().unwrap_or_else(|e| e.into_inner()) = Some(game.to_owned());
    }

    /// Record window `id` (picked by the user) instead of the current one. Returns false if
    /// it is already the one being recorded. The switch happens when the recorder restarts:
    /// through [`Self::apply_settings`] if the settings change too, else [`Self::restart_recorder`].
    pub fn prefer_window(&self, id: u64) -> bool {
        let mut current = self
            .current_window
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let changed = *current != Some(id);
        *current = Some(id);
        changed
    }

    /// The window being recorded (or about to be).
    pub fn current_window(&self) -> Option<u64> {
        *self
            .current_window
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    /// Restart the recorder with the same settings (drops the buffer: a clip never mixes two
    /// windows).
    pub fn restart_recorder(&self) {
        let mut slot = self.recorder.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(config) = slot.config.clone() {
            slot.recorder = None;
            self.start_recorder(&mut slot, config);
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
            last_clip: Arc::clone(&self.last_clip),
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

    /// Open the last saved clip in the default player.
    pub fn play_last_clip(&self) {
        if let Some(path) = self
            .last_clip
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
        {
            crate::notify::open(&path);
        }
    }

    /// Show the last saved clip in the file manager.
    pub fn show_last_clip(&self) {
        if let Some(path) = self
            .last_clip
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
        {
            crate::notify::show_in_folder(&path);
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
            games: match std::env::var(WINDOW_ENV) {
                Ok(title) if !title.is_empty() => vec![Game::by_title(&title)],
                _ => settings.all_games(),
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
        self.start_recorder(&mut slot, config);
    }

    fn start_recorder(&self, slot: &mut RecorderSlot, config: RecorderConfig) {
        let window = self.window.clone();
        let tray = self.tray.get().cloned();
        let waiting_for = waiting_for(&config.games);
        let last_save = Arc::clone(&self.last_save);
        let trial = Arc::clone(&self.trial);
        let current = Arc::clone(&self.current_window);
        slot.recorder = Some(Recorder::start(config.clone(), current, move |status| {
            check_trial(&trial, &status, &window);
            let (text, recording) = status_text(&status, &waiting_for);
            let state = match status {
                RecorderStatus::Recording { .. } => TrayState::Recording,
                RecorderStatus::Error(_) | RecorderStatus::Failed { .. } => TrayState::Error,
                RecorderStatus::WaitingForWindow | RecorderStatus::Stopped => TrayState::Waiting,
            };
            let mut tray_text = format!("Chronoclip: {text}");
            if let Some(line) = &*last_save.lock().unwrap_or_else(|e| e.into_inner()) {
                tray_text.push('\n');
                tray_text.push_str(line);
            }
            let view = status_view(&status, &waiting_for);
            let _ = window.upgrade_in_event_loop(move |w| {
                w.set_status_text(text.into());
                w.set_recording(recording);
                w.set_rec_state(view.state);
                w.set_rec_title(view.title.into());
                w.set_rec_detail(view.detail.into());
                w.set_buffer_fraction(view.fraction);
                w.set_buffer_label(view.buffer_label.into());
                w.set_buffer_size(view.buffer_size.into());
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

/// What the Home page shows for a recorder status.
struct StatusView {
    state: RecState,
    title: String,
    detail: String,
    fraction: f32,
    buffer_label: String,
    buffer_size: String,
}

/// Resolve a trial (see [`Controller::try_game`]): recording the game ends it; a failure
/// first tells the window, which removes the game again.
fn check_trial(
    trial: &Mutex<Option<String>>,
    status: &RecorderStatus,
    window: &slint::Weak<MainWindow>,
) {
    let mut trial = trial.lock().unwrap_or_else(|e| e.into_inner());
    let Some(name) = trial.as_deref() else { return };
    match status {
        RecorderStatus::Recording { game, .. } if game == name => {
            tracing::info!(%game, "picked game records fine");
            *trial = None;
        }
        RecorderStatus::Failed { game, error } if game == name => {
            *trial = None;
            let (game, error) = (game.clone(), error.clone());
            let _ = window.upgrade_in_event_loop(move |w| {
                w.invoke_game_failed(game.into(), error.into());
            });
        }
        _ => {}
    }
}

/// The games the app looks for, for the waiting text: "World of Warcraft",
/// "World of Warcraft and Satisfactory", "A, B, C and 2 more".
fn waiting_for(games: &[Game]) -> String {
    let names: Vec<&str> = games.iter().map(|g| g.name.as_str()).collect();
    match names.as_slice() {
        [] => String::new(),
        [one] => (*one).to_owned(),
        [first @ .., last] if names.len() <= 3 => format!("{} and {last}", first.join(", ")),
        _ => format!("{} and {} more", names[..3].join(", "), names.len() - 3),
    }
}

fn status_view(status: &RecorderStatus, waiting_for: &str) -> StatusView {
    let idle = |state, title: String, detail: String| StatusView {
        state,
        title,
        detail,
        fraction: 0.0,
        buffer_label: "Empty".into(),
        buffer_size: String::new(),
    };
    match status {
        RecorderStatus::WaitingForWindow => idle(
            RecState::Waiting,
            "Waiting for a game".into(),
            format!(
                "Looking for {waiting_for}: recording begins by itself when one starts. \
                 Another game? Start it and choose its window."
            ),
        ),
        RecorderStatus::Recording {
            game,
            width,
            height,
            fps,
            buffered,
            retention,
            bytes,
            skipped_frames,
            audio,
        } => {
            let mut detail = format!("{width}×{height} · {fps} fps");
            if let Some(audio) = audio {
                detail.push_str(" · ");
                detail.push_str(&capitalize(audio));
            }
            if *skipped_frames > 0 {
                let s = if *skipped_frames == 1 { "" } else { "s" };
                detail.push_str(&format!(" · {skipped_frames} frame{s} skipped"));
            }
            StatusView {
                state: RecState::Recording,
                title: game.clone(),
                detail,
                fraction: if retention.is_zero() {
                    0.0
                } else {
                    (buffered.as_secs_f32() / retention.as_secs_f32()).min(1.0)
                },
                buffer_label: format!(
                    "{} of {}",
                    format_duration(*buffered),
                    format_duration(*retention)
                ),
                buffer_size: format_size(*bytes),
            }
        }
        RecorderStatus::Error(e) => idle(RecState::Error, "Can't record".into(), e.clone()),
        RecorderStatus::Failed { game, error } => idle(
            RecState::Error,
            format!("Can't record {game}"),
            error.clone(),
        ),
        RecorderStatus::Stopped => idle(RecState::Waiting, "Stopped".into(), String::new()),
    }
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

fn status_text(status: &RecorderStatus, waiting_for: &str) -> (String, bool) {
    match status {
        RecorderStatus::WaitingForWindow => (format!("Waiting for a game ({waiting_for})…"), false),
        RecorderStatus::Recording {
            game,
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
                "Recording {game} {width}×{height} @ {fps} fps · buffer {} / {} · {}",
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
        RecorderStatus::Failed { game, error } => (format!("Can't record {game}: {error}"), false),
        RecorderStatus::Stopped => ("Stopped".into(), false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waiting_text_lists_the_games() {
        let games =
            |names: &[&str]| -> Vec<Game> { names.iter().map(|n| Game::by_title(n)).collect() };
        assert_eq!(
            waiting_for(&games(&["World of Warcraft"])),
            "World of Warcraft"
        );
        assert_eq!(
            waiting_for(&games(&["WoW", "Satisfactory"])),
            "WoW and Satisfactory"
        );
        assert_eq!(waiting_for(&games(&["A", "B", "C"])), "A, B and C");
        assert_eq!(
            waiting_for(&games(&["A", "B", "C", "D", "E"])),
            "A, B, C and 2 more"
        );
    }
}
