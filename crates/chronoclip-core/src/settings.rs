//! User settings, persisted as TOML in the XDG config directory.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::games::Game;
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

/// Which sound goes into the clip's main audio track.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioMode {
    /// Only the game's own audio stream (falls back to all sound while it isn't found).
    #[default]
    Game,
    /// Everything played on the default output: game, voice chat, music, …
    All,
    /// No game/desktop audio track.
    Off,
}

impl AudioMode {
    pub const ALL: [Self; 3] = [Self::Game, Self::All, Self::Off];
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
    pub audio: AudioMode,
    /// Record the default microphone as a separate audio track.
    pub microphone: bool,
    /// Play a chime when a clip is saved (and a different one when saving fails).
    pub save_sound: bool,
    /// Frameless window with Chronoclip's own title bar; off: the desktop's title bar.
    pub custom_title_bar: bool,
    /// `None` means [`Paths::default_buffer_dir`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub buffer_dir: Option<PathBuf>,
    /// `None` means [`Paths::default_output_dir`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_dir: Option<PathBuf>,
    /// Games added by the user, recorded besides the built-in World of Warcraft. Last field:
    /// TOML writes it as `[[games]]` tables, which must follow the plain values.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub games: Vec<Game>,
    /// Installed or built-in games the user switched off ("Don't record"). A game the user
    /// picks in the window picker is taken off this list again.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub never_record: Vec<Game>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            hotkey: Hotkey::default(),
            buffer_minutes: DEFAULT_BUFFER_MINUTES,
            fps: DEFAULT_FPS,
            encoder: EncoderPreference::default(),
            audio: AudioMode::default(),
            microphone: false,
            save_sound: true,
            custom_title_bar: true,
            buffer_dir: None,
            output_dir: None,
            games: Vec::new(),
            never_record: Vec::new(),
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
    #[error("game {0:?} needs a name and an executable or window title")]
    InvalidGame(String),
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
        if let Some(game) = self.games.iter().find(|g| !g.is_valid()) {
            return Err(SettingsError::InvalidGame(game.name.clone()));
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
        self.never_record.retain(Game::is_valid);
        self.games.retain(|game| {
            let valid = game.is_valid();
            if !valid {
                tracing::warn!(?game, "ignoring game without a name or anything to match");
            }
            valid
        });
        self
    }

    /// Add a game unless an existing one (yours, `installed` or built in) already covers it.
    /// Returns the name of the game that now records it: the new one, or the existing one.
    /// Either way it's taken off the "never record" list: the user asked for it.
    pub fn add_game(&mut self, game: Game, installed: &[Game]) -> String {
        self.never_record
            .retain(|n| !n.covers(&game) && !game.covers(n));
        if let Some(existing) = self
            .all_games(installed)
            .into_iter()
            .find(|g| g.covers(&game))
        {
            return existing.name;
        }
        let name = game.name.clone();
        self.games.push(game);
        name
    }

    /// True if `game` is on the "never record" list.
    pub fn is_never_recorded(&self, game: &Game) -> bool {
        self.never_record
            .iter()
            .any(|n| n.name.eq_ignore_ascii_case(&game.name) || n.covers(game) || game.covers(n))
    }

    /// Every game to look for, in priority order: the user's games (their names win where a
    /// window matches both), then the `installed` games, then the rest of the built-in
    /// catalog; without the ones on the "never record" list.
    pub fn all_games(&self, installed: &[Game]) -> Vec<Game> {
        let mut games = self.games.clone();
        for game in installed.iter().chain(crate::games::catalog()) {
            let known = games
                .iter()
                .any(|g| g.name.eq_ignore_ascii_case(&game.name) || g.covers(game));
            if !known && !self.is_never_recorded(game) {
                games.push(game.clone());
            }
        }
        games
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
    fn games_round_trip_and_invalid_ones_are_dropped() -> Result<(), Box<dyn std::error::Error>> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("settings.toml");
        let elden = Game {
            name: "Elden Ring".into(),
            steam_app_ids: Vec::new(),
            executables: vec!["eldenring.exe".into()],
            title_contains: None,
        };
        let s = Settings {
            output_dir: Some("/clips".into()),
            games: vec![elden.clone()],
            ..Settings::default()
        };
        s.save(&path)?;
        assert_eq!(Settings::load(&path)?, s);
        let names: Vec<_> = s.all_games(&[]).into_iter().map(|g| g.name).collect();
        // Yours first; it covers the catalog's ELDEN RING (same executable), so that's left out.
        assert_eq!(names[0], "Elden Ring");
        assert!(!names.contains(&"ELDEN RING".to_owned()));
        assert!(names.contains(&"World of Warcraft".to_owned()));
        assert_eq!(names.len(), crate::games::catalog().len());

        let broken = Game {
            name: "Broken".into(),
            steam_app_ids: Vec::new(),
            executables: Vec::new(),
            title_contains: None,
        };
        let s = Settings {
            games: vec![broken.clone(), elden.clone()],
            ..Settings::default()
        };
        assert!(matches!(s.validate(), Err(SettingsError::InvalidGame(n)) if n == "Broken"));
        assert_eq!(s.sanitized().games, std::slice::from_ref(&elden));

        let mut s = Settings::default();
        // Built in: not added again, the catalog's name is used.
        assert_eq!(s.add_game(elden, &[]), "ELDEN RING");
        assert_eq!(
            s.add_game(Game::world_of_warcraft(), &[]),
            "World of Warcraft"
        );
        assert!(s.games.is_empty());
        let hades = Game {
            name: "Hades".into(),
            steam_app_ids: Vec::new(),
            executables: vec!["Hades.exe".into()],
            title_contains: None,
        };
        assert_eq!(s.add_game(hades.clone(), &[]), "Hades");
        assert_eq!(s.add_game(hades.clone(), &[]), "Hades");
        assert_eq!(s.games, [hades]);

        // Installed and built-in games come after yours; "never record" leaves them out.
        let factorio = Game {
            name: "Factorio".into(),
            executables: Vec::new(),
            steam_app_ids: vec![427_520],
            title_contains: None,
        };
        let names = |s: &Settings| -> Vec<String> {
            s.all_games(std::slice::from_ref(&factorio))
                .into_iter()
                .map(|g| g.name)
                .collect()
        };
        assert_eq!(names(&s)[..2], ["Hades", "Factorio"]);
        assert!(names(&s).contains(&"World of Warcraft".to_owned()));
        s.never_record = vec![factorio.clone(), Game::world_of_warcraft()];
        assert!(!names(&s).contains(&"Factorio".to_owned()));
        assert!(!names(&s).contains(&"World of Warcraft".to_owned()));
        // Picking it again takes it off the list.
        assert_eq!(
            s.add_game(factorio.clone(), std::slice::from_ref(&factorio)),
            "Factorio"
        );
        assert_eq!(s.never_record, [Game::world_of_warcraft()]);
        Ok(())
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
            audio: AudioMode::All,
            microphone: true,
            save_sound: false,
            custom_title_bar: false,
            buffer_dir: Some("/mnt/ssd/buffer".into()),
            output_dir: None,
            games: vec![Game::by_title("Factorio")],
            never_record: vec![Game::world_of_warcraft()],
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
        assert_eq!(s.audio, AudioMode::Game);
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
