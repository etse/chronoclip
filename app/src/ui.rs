//! Glue between the Slint UI and Rust: fills the form from settings and reads it back.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::rc::Rc;
use std::sync::Arc;

use chronoclip_core::Paths;
use chronoclip_core::disk::{
    estimate_buffer_bytes, format_gb, nominal_bitrate_mbps, tmpfs_warning,
};
use chronoclip_core::settings::{DEFAULT_FPS, EncoderPreference, SUPPORTED_FPS, Settings};
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

fn string_model(items: impl IntoIterator<Item = String>) -> ModelRc<SharedString> {
    let items: Vec<SharedString> = items.into_iter().map(Into::into).collect();
    ModelRc::new(VecModel::from(items))
}

pub fn init(window: &MainWindow, paths: Paths, settings: &Settings, controller: Arc<Controller>) {
    let paths = Rc::new(paths);

    window.set_version(env!("CARGO_PKG_VERSION").into());
    window.set_status_text("Starting…".into());
    window.set_fps_options(string_model(SUPPORTED_FPS.iter().map(u32::to_string)));
    window.set_encoder_options(string_model(
        EncoderPreference::ALL
            .iter()
            .map(|e| encoder_label(*e).to_owned()),
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

    window.on_apply({
        let weak = window.as_weak();
        let paths = Rc::clone(&paths);
        let controller = Arc::clone(&controller);
        move || {
            let Some(window) = weak.upgrade() else { return };
            let result = read_settings(&window, &paths).and_then(|s| {
                s.save(&paths.config_file)
                    .map(|()| s)
                    .map_err(|e| e.to_string())
            });
            match result {
                Ok(settings) => {
                    show_settings(&window, &settings, &paths);
                    set_message(&window, "Settings saved.", false);
                    tracing::info!(?settings, "settings saved");
                    controller.apply_settings(&settings);
                }
                Err(e) => set_message(&window, &e, true),
            }
        }
    });

    window.on_reset_defaults({
        let weak = window.as_weak();
        let paths = Rc::clone(&paths);
        move || {
            let Some(window) = weak.upgrade() else { return };
            show_settings(&window, &Settings::default(), &paths);
            set_message(
                &window,
                "Defaults restored. Click Save settings to keep them.",
                false,
            );
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
        controller.save_replay();
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
    window.set_microphone(settings.microphone);
    window.set_buffer_dir(settings.buffer_dir(paths).display().to_string().into());
    window.set_output_dir(settings.output_dir(paths).display().to_string().into());
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

    let settings = Settings {
        hotkey,
        buffer_minutes: window.get_buffer_minutes().round() as u32,
        fps,
        encoder,
        microphone: window.get_microphone(),
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

fn capitalize(s: &str) -> String {
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
