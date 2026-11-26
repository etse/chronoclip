//! Locations of config, buffer, output and IPC socket, following the XDG base directory spec.

use std::path::{Path, PathBuf};

use directories::{BaseDirs, ProjectDirs, UserDirs};

use crate::ipc::SOCKET_NAME;

const APP_NAME: &str = "chronoclip";

#[derive(Debug, thiserror::Error)]
pub enum PathsError {
    #[error("could not determine the home directory")]
    NoHome,
}

/// Resolved default locations for the current user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    pub home_dir: PathBuf,
    /// `$XDG_CONFIG_HOME/chronoclip/settings.toml`
    pub config_file: PathBuf,
    /// `$XDG_CACHE_HOME/chronoclip/buffer`. On disk, unlike `/tmp` or the runtime dir on many distros.
    pub default_buffer_dir: PathBuf,
    /// `$XDG_VIDEOS_DIR/Chronoclip`
    pub default_output_dir: PathBuf,
    /// `$XDG_RUNTIME_DIR`, usually a RAM-backed tmpfs.
    pub runtime_dir: Option<PathBuf>,
    /// Unix socket used by `chronoclip save` and single-instance detection.
    pub socket: PathBuf,
    /// `$XDG_CONFIG_HOME/autostart/chronoclip.desktop`, present while "Start with the desktop" is on.
    pub autostart_file: PathBuf,
}

impl Paths {
    pub fn detect() -> Result<Self, PathsError> {
        let base = BaseDirs::new().ok_or(PathsError::NoHome)?;
        let project = ProjectDirs::from("", "", APP_NAME).ok_or(PathsError::NoHome)?;
        let videos = UserDirs::new()
            .and_then(|u| u.video_dir().map(Path::to_path_buf))
            .unwrap_or_else(|| base.home_dir().join("Videos"));
        let runtime_dir = base.runtime_dir().map(Path::to_path_buf);
        let socket = runtime_dir
            .as_deref()
            .unwrap_or_else(|| project.cache_dir())
            .join(SOCKET_NAME);

        Ok(Self {
            home_dir: base.home_dir().to_path_buf(),
            config_file: project.config_dir().join("settings.toml"),
            default_buffer_dir: project.cache_dir().join("buffer"),
            default_output_dir: videos.join("Chronoclip"),
            runtime_dir,
            socket,
            autostart_file: base
                .config_dir()
                .join("autostart")
                .join("chronoclip.desktop"),
        })
    }

    /// Display form of `path`: inside the home directory it starts with `~`, which is shorter
    /// and reads back through [`expand_home`](Self::expand_home).
    pub fn contract_home(&self, path: &Path) -> String {
        match path.strip_prefix(&self.home_dir) {
            Ok(rest) if rest.as_os_str().is_empty() => "~".into(),
            Ok(rest) => format!("~/{}", rest.display()),
            Err(_) => path.display().to_string(),
        }
    }

    /// Expand a leading `~` to the home directory.
    pub fn expand_home(&self, path: &str) -> PathBuf {
        match path.strip_prefix('~') {
            Some("") => self.home_dir.clone(),
            Some(rest) if rest.starts_with('/') => self.home_dir.join(&rest[1..]),
            _ => PathBuf::from(path),
        }
    }
}

/// Data directories in lookup order: `$XDG_DATA_HOME` (default `~/.local/share`), then
/// `$XDG_DATA_DIRS` (default `/usr/local/share:/usr/share`).
pub fn data_dirs(home: &Path) -> Vec<PathBuf> {
    let env_dirs = |name: &str| {
        std::env::var_os(name)
            .map(|v| {
                std::env::split_paths(&v)
                    .filter(|p| p.is_absolute())
                    .collect::<Vec<_>>()
            })
            .filter(|dirs| !dirs.is_empty())
    };
    let mut dirs = env_dirs("XDG_DATA_HOME").unwrap_or_else(|| vec![home.join(".local/share")]);
    dirs.extend(
        env_dirs("XDG_DATA_DIRS")
            .unwrap_or_else(|| vec!["/usr/local/share".into(), "/usr/share".into()]),
    );
    dirs
}

/// The largest installed hicolor PNG icon named `name` in `data_dirs`, if any (e.g. the
/// app's own icon when installed from the .deb; not inside an AppImage).
pub fn find_icon(data_dirs: &[PathBuf], name: &str) -> Option<PathBuf> {
    const SIZES: [&str; 6] = ["512x512", "256x256", "128x128", "64x64", "48x48", "32x32"];
    SIZES.iter().find_map(|size| {
        data_dirs
            .iter()
            .map(|dir| {
                dir.join("icons/hicolor")
                    .join(size)
                    .join("apps")
                    .join(format!("{name}.png"))
            })
            .find(|path| path.is_file())
    })
}

/// `file://` URI for an absolute path, percent-encoding everything except unreserved
/// characters and `/` (for D-Bus calls such as `org.freedesktop.FileManager1.ShowItems`).
pub fn file_uri(path: &Path) -> String {
    use std::os::unix::ffi::OsStrExt;
    let mut uri = String::from("file://");
    for &b in path.as_os_str().as_bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/".contains(&b) {
            uri.push(char::from(b));
        } else {
            uri.push_str(&format!("%{b:02X}"));
        }
    }
    uri
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths() -> Paths {
        Paths {
            home_dir: "/home/me".into(),
            config_file: "/home/me/.config/chronoclip/settings.toml".into(),
            default_buffer_dir: "/home/me/.cache/chronoclip/buffer".into(),
            default_output_dir: "/home/me/Videos/Chronoclip".into(),
            runtime_dir: Some("/run/user/1000".into()),
            socket: "/run/user/1000/chronoclip.sock".into(),
            autostart_file: "/home/me/.config/autostart/chronoclip.desktop".into(),
        }
    }

    #[test]
    fn expands_home() {
        let p = paths();
        assert_eq!(p.expand_home("~"), PathBuf::from("/home/me"));
        assert_eq!(p.expand_home("~/clips"), PathBuf::from("/home/me/clips"));
        assert_eq!(p.expand_home("/mnt/ssd"), PathBuf::from("/mnt/ssd"));
        assert_eq!(p.expand_home("~other/x"), PathBuf::from("~other/x"));
    }

    #[test]
    fn contracts_home_for_display() {
        let p = paths();
        assert_eq!(
            p.contract_home(&p.default_output_dir),
            "~/Videos/Chronoclip"
        );
        assert_eq!(p.contract_home(Path::new("/home/me")), "~");
        assert_eq!(
            p.contract_home(Path::new("/mnt/ssd/clips")),
            "/mnt/ssd/clips"
        );
        // A sibling that only shares the prefix is not inside home.
        assert_eq!(p.contract_home(Path::new("/home/meow")), "/home/meow");
        // Round trip.
        let shown = p.contract_home(&p.default_buffer_dir);
        assert_eq!(p.expand_home(&shown), p.default_buffer_dir);
    }

    #[test]
    fn file_uris_are_percent_encoded() {
        assert_eq!(
            file_uri(Path::new(
                "/home/me/Videos/Chronoclip/Chronoclip_2026-11-26_21-48-40.mp4"
            )),
            "file:///home/me/Videos/Chronoclip/Chronoclip_2026-11-26_21-48-40.mp4"
        );
        assert_eq!(
            file_uri(Path::new("/home/me/My Clips/æ#.mp4")),
            "file:///home/me/My%20Clips/%C3%A6%23.mp4"
        );
    }

    #[test]
    fn finds_the_largest_installed_icon() -> std::io::Result<()> {
        let tmp = tempfile::tempdir()?;
        let (local, system) = (tmp.path().join("local"), tmp.path().join("system"));
        let dirs = vec![local.clone(), system.clone()];
        assert_eq!(find_icon(&dirs, "app"), None);
        for (dir, size) in [(&system, "256x256"), (&system, "48x48"), (&local, "64x64")] {
            let apps = dir.join("icons/hicolor").join(size).join("apps");
            std::fs::create_dir_all(&apps)?;
            std::fs::write(apps.join("app.png"), b"png")?;
        }
        assert_eq!(
            find_icon(&dirs, "app"),
            Some(system.join("icons/hicolor/256x256/apps/app.png"))
        );
        assert_eq!(find_icon(&dirs, "other"), None);
        Ok(())
    }
}
