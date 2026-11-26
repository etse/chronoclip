//! Feedback after a save that you notice even in-game: a chime and a desktop notification.
//!
//! Cinnamon hides notifications over fullscreen windows by default, and borderless WoW counts
//! as fullscreen, so the chime is what tells you a save worked while playing.

use std::path::{Path, PathBuf};
use std::process::Command;

use chronoclip_core::APP_ID;
use chronoclip_core::chime;
use chronoclip_core::disk::{format_duration, format_size};
use chronoclip_core::paths::{data_dirs, file_uri, find_icon};
use notify_rust::{Hint, Notification, Timeout};

use crate::save::SavedClip;

const APP_NAME: &str = "Chronoclip";
const ICON_PNG: &[u8] = include_bytes!("../ui/icons/chronoclip-256.png");
const TIMEOUT_MS: u32 = 8_000;

pub struct Alerts {
    /// Notification icon: the installed app icon, else a copy in the cache folder (AppImage,
    /// `cargo run`), else a theme icon name.
    icon: String,
}

impl Alerts {
    /// `cache_dir` is where the notification icon is written if the app isn't installed.
    pub fn new(home: &Path, cache_dir: &Path) -> Self {
        if let Some(installed) = find_icon(&data_dirs(home), APP_ID) {
            return Self {
                icon: installed.display().to_string(),
            };
        }
        let path = cache_dir.join("chronoclip.png");
        let current = std::fs::metadata(&path).is_ok_and(|m| m.len() == ICON_PNG.len() as u64);
        let written = current
            || std::fs::create_dir_all(cache_dir)
                .and_then(|()| std::fs::write(&path, ICON_PNG))
                .inspect_err(|e| tracing::debug!("could not write notification icon: {e}"))
                .is_ok();
        let icon = if written {
            path.display().to_string()
        } else {
            "media-record".into()
        };
        Self { icon }
    }

    pub fn clip_saved(&self, clip: &SavedClip, sound: bool) {
        if sound {
            play(chime::saved());
        }
        let body = format!(
            "{} · {}\n{}",
            format_duration(clip.duration),
            format_size(clip.bytes),
            clip.file_name()
        );
        let path = clip.path.clone();
        self.notify("Clip saved", body, Some(path));
    }

    pub fn save_failed(&self, message: &str, sound: bool) {
        if sound {
            play(chime::failed());
        }
        self.notify("Clip not saved", message.to_owned(), None);
    }

    /// Show a notification on its own thread; with a clip, offer Play and Show in folder.
    fn notify(&self, summary: &str, body: String, clip: Option<PathBuf>) {
        let mut notification = Notification::new();
        notification
            .appname(APP_NAME)
            .hint(Hint::DesktopEntry(APP_ID.into()))
            .summary(summary)
            .body(&body)
            .icon(&self.icon)
            .timeout(Timeout::Milliseconds(TIMEOUT_MS));
        if clip.is_some() {
            notification
                .hint(Hint::Category("transfer.complete".into()))
                .action("default", "Play")
                .action("play", "Play")
                .action("folder", "Show in folder");
        }
        let spawned = std::thread::Builder::new()
            .name("notification".into())
            .spawn(move || {
                let handle = match notification.show() {
                    Ok(handle) => handle,
                    Err(e) => {
                        tracing::debug!("could not show a notification: {e}");
                        return;
                    }
                };
                let Some(clip) = clip else { return };
                // Returns when the notification is clicked, closed or expires.
                handle.wait_for_action(|action| match action {
                    "default" | "play" => open(&clip),
                    "folder" => show_in_folder(&clip),
                    _ => {}
                });
            });
        if let Err(e) = spawned {
            tracing::debug!("could not start notification thread: {e}");
        }
    }
}

/// Play a chime on its own thread (it blocks for its length).
fn play(samples: Vec<i16>) {
    let spawned = std::thread::Builder::new()
        .name("chime".into())
        .spawn(move || {
            if let Err(e) = chronoclip_capture::play_sound(&samples, chime::SAMPLE_RATE) {
                tracing::debug!("could not play the save sound: {e}");
            }
        });
    if let Err(e) = spawned {
        tracing::debug!("could not start chime thread: {e}");
    }
}

/// Open a file or folder with the desktop's default application.
pub fn open(path: &Path) {
    match Command::new("xdg-open").arg(path).spawn() {
        // Reap the child in the background so it doesn't linger as a zombie.
        Ok(mut child) => drop(std::thread::spawn(move || child.wait())),
        Err(e) => tracing::warn!("could not open {}: {e}", path.display()),
    }
}

/// Open a web page in the default browser.
pub fn open_url(url: &str) {
    match Command::new("xdg-open").arg(url).spawn() {
        Ok(mut child) => drop(std::thread::spawn(move || child.wait())),
        Err(e) => tracing::warn!("could not open {url}: {e}"),
    }
}

/// Open the file manager with `path` selected (`org.freedesktop.FileManager1`, supported by
/// Nemo, Nautilus, Dolphin, …); fall back to opening its folder.
pub fn show_in_folder(path: &Path) {
    let uri = file_uri(path);
    let shown = zbus::blocking::Connection::session().and_then(|connection| {
        connection
            .call_method(
                Some("org.freedesktop.FileManager1"),
                "/org/freedesktop/FileManager1",
                Some("org.freedesktop.FileManager1"),
                "ShowItems",
                &(vec![uri.as_str()], ""),
            )
            .map(drop)
    });
    if let Err(e) = shown {
        tracing::debug!("file manager can't select the clip ({e}); opening its folder");
        if let Some(dir) = path.parent() {
            open(dir);
        }
    }
}
