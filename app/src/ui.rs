//! Glue between the Slint UI and Rust: fills the form from settings and reads it back.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;
use std::sync::Arc;

use chronoclip_core::Paths;
use chronoclip_core::disk::{
    estimate_buffer_bytes, format_gb, nominal_bitrate_mbps, tmpfs_warning,
};
use chronoclip_core::settings::{
    AudioMode, DEFAULT_FPS, EncoderPreference, SUPPORTED_FPS, Settings,
};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};

use crate::controller::Controller;

/// Code generated from `ui/*.slint`. It uses `unwrap()` internally, which our lints forbid.
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod generated {
    slint::include_modules!();
}
pub use generated::{MainWindow, Tray};

fn encoder_label(encoder: EncoderPreference) -> &'static str {
    match encoder {
        EncoderPreference::Auto => "Auto",
        EncoderPreference::Nvenc => "NVENC (NVIDIA)",
        EncoderPreference::Vaapi => "VAAPI (AMD / Intel)",
        EncoderPreference::Software => "Software (slow)",
    }
}

fn encoder_from_label(label: &str) -> Option<EncoderPreference> {
    EncoderPreference::ALL
        .into_iter()
        .find(|e| encoder_label(*e) == label)
}

fn audio_label(audio: AudioMode) -> &'static str {
    match audio {
        AudioMode::Game => "Game only",
        AudioMode::All => "All sound",
        AudioMode::Off => "Off",
    }
}

fn audio_from_label(label: &str) -> Option<AudioMode> {
    AudioMode::ALL
        .into_iter()
        .find(|a| audio_label(*a) == label)
}

fn string_model(items: impl IntoIterator<Item = String>) -> ModelRc<SharedString> {
    let items: Vec<SharedString> = items.into_iter().map(Into::into).collect();
    ModelRc::new(VecModel::from(items))
}

pub fn init(window: &MainWindow, paths: Paths, settings: &Settings, controller: Arc<Controller>) {
    let paths = Rc::new(paths);
    let save_controller = Arc::clone(&controller);

    window.set_version(env!("CARGO_PKG_VERSION").into());
    window.set_status_text("Starting…".into());
    window.set_fps_options(string_model(SUPPORTED_FPS.iter().map(u32::to_string)));
    window.set_encoder_options(string_model(
        EncoderPreference::ALL
            .iter()
            .map(|e| encoder_label(*e).to_owned()),
    ));
    window.set_audio_options(string_model(
        AudioMode::ALL.iter().map(|a| audio_label(*a).to_owned()),
    ));
    show_settings(window, settings, &paths);

    window.on_estimate_disk(|minutes, fps| {
        let fps = fps.parse().unwrap_or(DEFAULT_FPS);
        let minutes = u32::try_from(minutes).unwrap_or(0);
        let bytes = estimate_buffer_bytes(minutes, nominal_bitrate_mbps(fps));
        format!("About {} on disk at 1080p{fps}.", format_gb(bytes)).into()
    });

    window.on_buffer_dir_warning({
        let paths = Rc::clone(&paths);
        move |dir| {
            let dir = paths.expand_home(dir.trim());
            tmpfs_warning(&dir, paths.runtime_dir.as_deref())
                .unwrap_or_default()
                .into()
        }
    });

    // Settings are saved and applied as soon as a field changes (text fields when editing
    // is done). `current` is what is saved, so unchanged fields don't restart anything.
    let current = Rc::new(RefCell::new(settings.clone()));
    let apply = {
        let weak = window.as_weak();
        let paths = Rc::clone(&paths);
        move |done: &str| {
            let Some(window) = weak.upgrade() else { return };
            let settings = match read_settings(&window, &paths) {
                Ok(s) => s,
                Err(e) => return set_message(&window, &e, true),
            };
            if settings == *current.borrow() {
                // Nothing changed; clear an error from an earlier invalid entry.
                if window.get_message_is_error() {
                    set_message(&window, "", false);
                }
                return;
            }
            if let Err(e) = settings.save(&paths.config_file) {
                return set_message(&window, &capitalize(&e.to_string()), true);
            }
            show_settings(&window, &settings, &paths);
            set_message(&window, done, false);
            tracing::info!(?settings, "settings saved");
            *current.borrow_mut() = settings.clone();
            controller.apply_settings(&settings);
        }
    };
    let apply = Rc::new(apply);

    window.on_apply({
        let apply = Rc::clone(&apply);
        move || apply("Settings saved.")
    });

    window.on_reset_defaults({
        let weak = window.as_weak();
        let paths = Rc::clone(&paths);
        move || {
            let Some(window) = weak.upgrade() else { return };
            show_settings(&window, &Settings::default(), &paths);
            apply("Defaults restored and saved.");
        }
    });

    window.on_open_folder({
        let weak = window.as_weak();
        let paths = Rc::clone(&paths);
        move |dir| {
            let dir = paths.expand_home(dir.trim());
            if let Err(e) = open_folder(&dir)
                && let Some(window) = weak.upgrade()
            {
                set_message(
                    &window,
                    &format!("Could not open {}: {e}", dir.display()),
                    true,
                );
            }
        }
    });

    window.on_save_replay(move || {
        save_controller.save_replay();
    });
}

pub fn set_message(window: &MainWindow, text: &str, is_error: bool) {
    window.set_message(text.into());
    window.set_message_is_error(is_error);
}

fn show_settings(window: &MainWindow, settings: &Settings, paths: &Paths) {
    window.set_hotkey(settings.hotkey.to_string().into());
    window.set_buffer_minutes(settings.buffer_minutes as f32);
    window.set_fps(settings.fps.to_string().into());
    window.set_encoder(encoder_label(settings.encoder).into());
    window.set_audio(audio_label(settings.audio).into());
    window.set_microphone(settings.microphone);
    window.set_save_sound(settings.save_sound);
    window.set_buffer_dir(paths.contract_home(&settings.buffer_dir(paths)).into());
    window.set_output_dir(paths.contract_home(&settings.output_dir(paths)).into());
}

fn read_settings(window: &MainWindow, paths: &Paths) -> Result<Settings, String> {
    let hotkey = window
        .get_hotkey()
        .trim()
        .parse()
        .map_err(|e| format!("Hotkey: {e}."))?;
    let fps = window
        .get_fps()
        .parse()
        .map_err(|_| format!("Invalid frame rate `{}`.", window.get_fps()))?;
    let encoder = encoder_from_label(&window.get_encoder())
        .ok_or_else(|| format!("Unknown encoder `{}`.", window.get_encoder()))?;
    let audio = audio_from_label(&window.get_audio())
        .ok_or_else(|| format!("Unknown sound setting `{}`.", window.get_audio()))?;

    let settings = Settings {
        hotkey,
        buffer_minutes: window.get_buffer_minutes().round() as u32,
        fps,
        encoder,
        audio,
        microphone: window.get_microphone(),
        save_sound: window.get_save_sound(),
        buffer_dir: dir_setting(&window.get_buffer_dir(), &paths.default_buffer_dir, paths),
        output_dir: dir_setting(&window.get_output_dir(), &paths.default_output_dir, paths),
    };
    settings
        .validate()
        .map_err(|e| capitalize(&e.to_string()))?;
    Ok(settings)
}

/// Empty or equal to the default means "use the default" (stored as `None`).
fn dir_setting(text: &str, default: &Path, paths: &Paths) -> Option<PathBuf> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    let path = paths.expand_home(text);
    (path != default).then_some(path)
}

pub fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    chars
        .next()
        .map(|c| c.to_uppercase().chain(chars).collect())
        .unwrap_or_default()
}

fn open_folder(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    // Reap the child in the background so it doesn't linger as a zombie.
    let mut child = Command::new("xdg-open").arg(dir).spawn()?;
    std::thread::spawn(move || child.wait());
    Ok(())
}
