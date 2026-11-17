//! User settings, persisted as TOML in the XDG config directory.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::hotkey::Hotkey;
use crate::paths::Paths;

pub const MIN_BUFFER_MINUTES: u32 = 5;
pub const MAX_BUFFER_MINUTES: u32 = 45;
pub const DEFAULT_BUFFER_MINUTES: u32 = 10;
pub const SUPPORTED_FPS: [u32; 4] = [30, 60, 120, 144];
pub const DEFAULT_FPS: u32 = 60;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EncoderPreference {
    /// Pick the best available hardware encoder.
    #[default]
    Auto,
    Nvenc,
    Vaapi,
    Software,
}

impl EncoderPreference {
    pub const ALL: [Self; 4] = [Self::Auto, Self::Nvenc, Self::Vaapi, Self::Software];
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub hotkey: Hotkey,
    /// Length of the replay buffer, `MIN_BUFFER_MINUTES..=MAX_BUFFER_MINUTES`.
    pub buffer_minutes: u32,
    /// One of [`SUPPORTED_FPS`].
    pub fps: u32,
    pub encoder: EncoderPreference,
    pub microphone: bool,
    /// `None` means [`Paths::default_buffer_dir`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub buffer_dir: Option<PathBuf>,
    /// `None` means [`Paths::default_output_dir`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_dir: Option<PathBuf>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            hotkey: Hotkey::default(),
            buffer_minutes: DEFAULT_BUFFER_MINUTES,
            fps: DEFAULT_FPS,
            encoder: EncoderPreference::default(),
            microphone: false,
            buffer_dir: None,
            output_dir: None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("could not access {path}: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("invalid settings file {path}: {source}")]
    Parse {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("could not serialize settings: {0}")]
    Serialize(#[from] toml::ser::Error),
    #[error("buffer length must be {MIN_BUFFER_MINUTES}-{MAX_BUFFER_MINUTES} minutes, got {0}")]
    BufferMinutesOutOfRange(u32),
    #[error("unsupported frame rate {0} (supported: 30, 60, 120, 144)")]
    UnsupportedFps(u32),
    #[error("folder must be an absolute path: {0}")]
    RelativePath(PathBuf),
}

impl Settings {
    /// Load settings from `path`. A missing file yields the defaults. Out-of-range values in an
    /// otherwise valid file are corrected instead of rejected.
    pub fn load(path: &Path) -> Result<Self, SettingsError> {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(source) => {
                return Err(SettingsError::Io {
                    path: path.to_owned(),
                    source,
                });
            }
        };
        let settings: Self = toml::from_str(&text).map_err(|source| SettingsError::Parse {
            path: path.to_owned(),
            source,
        })?;
        Ok(settings.sanitized())
    }

    /// Validate and write settings to `path` atomically (write to a temp file, then rename).
    pub fn save(&self, path: &Path) -> Result<(), SettingsError> {
        self.validate()?;
        let io_err = |source| SettingsError::Io {
            path: path.to_owned(),
            source,
        };
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir).map_err(io_err)?;
        }
        let text = toml::to_string_pretty(self)?;
        let tmp = path.with_extension("toml.tmp");
        fs::write(&tmp, text).map_err(io_err)?;
        fs::rename(&tmp, path).map_err(io_err)
    }

    pub fn validate(&self) -> Result<(), SettingsError> {
        if !(MIN_BUFFER_MINUTES..=MAX_BUFFER_MINUTES).contains(&self.buffer_minutes) {
            return Err(SettingsError::BufferMinutesOutOfRange(self.buffer_minutes));
        }
        if !SUPPORTED_FPS.contains(&self.fps) {
            return Err(SettingsError::UnsupportedFps(self.fps));
        }
        for dir in [&self.buffer_dir, &self.output_dir].into_iter().flatten() {
            if dir.is_relative() {
                return Err(SettingsError::RelativePath(dir.clone()));
            }
        }
        Ok(())
    }

    /// Clamp or reset values that are out of range, e.g. from a hand-edited file.
    pub fn sanitized(mut self) -> Self {
        let clamped = self
            .buffer_minutes
            .clamp(MIN_BUFFER_MINUTES, MAX_BUFFER_MINUTES);
        if clamped != self.buffer_minutes {
            tracing::warn!(
                from = self.buffer_minutes,
                to = clamped,
                "buffer length out of range, clamped"
            );
            self.buffer_minutes = clamped;
        }
        if !SUPPORTED_FPS.contains(&self.fps) {
            tracing::warn!(
                fps = self.fps,
                "unsupported frame rate, using {DEFAULT_FPS}"
            );
            self.fps = DEFAULT_FPS;
        }
        for dir in [&mut self.buffer_dir, &mut self.output_dir] {
            if dir.as_ref().is_some_and(|d| d.is_relative()) {
                tracing::warn!(?dir, "ignoring relative folder, using default");
                *dir = None;
            }
        }
        self
    }

    pub fn buffer_dir(&self, paths: &Paths) -> PathBuf {
        self.buffer_dir
            .clone()
            .unwrap_or_else(|| paths.default_buffer_dir.clone())
    }

    pub fn output_dir(&self, paths: &Paths) -> PathBuf {
        self.output_dir
            .clone()
            .unwrap_or_else(|| paths.default_output_dir.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid() {
        let s = Settings::default();
        assert!(s.validate().is_ok());
        assert_eq!(s.buffer_minutes, 10);
        assert_eq!(s.hotkey.to_string(), "Ctrl+F9");
    }

    #[test]
    fn validate_rejects_out_of_range() {
        for minutes in [0, 4, 46, 1000] {
            let s = Settings {
                buffer_minutes: minutes,
                ..Settings::default()
            };
            assert!(matches!(
                s.validate(),
                Err(SettingsError::BufferMinutesOutOfRange(m)) if m == minutes
            ));
        }
        for minutes in [5, 45] {
            let s = Settings {
                buffer_minutes: minutes,
                ..Settings::default()
            };
            assert!(s.validate().is_ok());
        }
        let s = Settings {
            fps: 75,
            ..Settings::default()
        };
        assert!(matches!(
            s.validate(),
            Err(SettingsError::UnsupportedFps(75))
        ));
        let s = Settings {
            output_dir: Some("clips".into()),
            ..Settings::default()
        };
        assert!(matches!(s.validate(), Err(SettingsError::RelativePath(_))));
    }

    #[test]
    fn sanitized_clamps() {
        let s = Settings {
            buffer_minutes: 90,
            fps: 75,
            buffer_dir: Some("relative".into()),
            ..Settings::default()
        }
        .sanitized();
        assert_eq!(s.buffer_minutes, 45);
        assert_eq!(s.fps, 60);
        assert_eq!(s.buffer_dir, None);
        assert_eq!(
            Settings {
                buffer_minutes: 1,
                ..Settings::default()
            }
            .sanitized()
            .buffer_minutes,
            5
        );
    }

    #[test]
    fn missing_file_gives_defaults() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let s = Settings::load(&dir.path().join("nope.toml"))?;
        assert_eq!(s, Settings::default());
        Ok(())
    }

    #[test]
    fn save_and_load_roundtrip() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("nested/settings.toml");
        let s = Settings {
            hotkey: "Ctrl+Shift+F10".parse()?,
            buffer_minutes: 30,
            fps: 144,
            encoder: EncoderPreference::Nvenc,
            microphone: true,
            buffer_dir: Some("/mnt/ssd/buffer".into()),
            output_dir: None,
        };
        s.save(&path)?;
        assert_eq!(Settings::load(&path)?, s);
        assert!(!path.with_extension("toml.tmp").exists());
        Ok(())
    }

    #[test]
    fn save_refuses_invalid() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("settings.toml");
        let s = Settings {
            buffer_minutes: 99,
            ..Settings::default()
        };
        assert!(s.save(&path).is_err());
        assert!(!path.exists());
        Ok(())
    }

    #[test]
    fn partial_file_uses_defaults_for_missing_keys() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("settings.toml");
        fs::write(&path, "buffer_minutes = 20\nencoder = \"vaapi\"\n")?;
        let s = Settings::load(&path)?;
        assert_eq!(s.buffer_minutes, 20);
        assert_eq!(s.encoder, EncoderPreference::Vaapi);
        assert_eq!(s.fps, DEFAULT_FPS);
        Ok(())
    }

    #[test]
    fn invalid_file_is_an_error() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("settings.toml");
        fs::write(&path, "hotkey = \"Banana+F9\"\n")?;
        assert!(matches!(
            Settings::load(&path),
            Err(SettingsError::Parse { .. })
        ));
        Ok(())
    }
}
