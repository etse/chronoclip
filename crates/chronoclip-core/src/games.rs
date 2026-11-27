//! Games Chronoclip records, and how a window is recognized as one of them.
//!
//! The backend collects [`WindowFacts`] about each top-level window (title, WM_CLASS, the
//! owning process's names); [`find_game`] then picks the game it belongs to. Pure logic, so
//! matching rules are unit-tested here.

use serde::{Deserialize, Serialize};

/// Name of the built-in World of Warcraft entry.
pub const WORLD_OF_WARCRAFT: &str = "World of Warcraft";

/// Executable names of World of Warcraft clients (Retail, Classic, PTR, Beta).
const WOW_EXECUTABLES: &[&str] = &[
    "Wow.exe",
    "WowClassic.exe",
    "WowT.exe",
    "WowB.exe",
    "WowClassicT.exe",
    "WowClassicB.exe",
];

/// A game and how to recognize its window. At least one way to match must be set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Game {
    /// Display name, also used for clip folders and file names.
    pub name: String,
    /// Executable file names (no path), compared case-insensitively against the window's
    /// WM_CLASS and its process name and command line. Wine/Proton games show up under their
    /// Windows executable name, e.g. `eldenring.exe`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub executables: Vec<String>,
    /// Text the window title contains (case-insensitive). Least reliable; for windows without
    /// a usable executable name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_contains: Option<String>,
}

impl Game {
    /// World of Warcraft, always in the games list.
    pub fn world_of_warcraft() -> Self {
        Self {
            name: WORLD_OF_WARCRAFT.to_owned(),
            executables: WOW_EXECUTABLES.iter().map(|&e| e.to_owned()).collect(),
            title_contains: None,
        }
    }

    /// A game matched by window title only (the `CHRONOCLIP_WINDOW` override and tests).
    pub fn by_title(title: &str) -> Self {
        Self {
            name: title.to_owned(),
            executables: Vec::new(),
            title_contains: Some(title.to_owned()),
        }
    }

    /// True if the entry can match anything: a name and at least one non-empty rule.
    pub fn is_valid(&self) -> bool {
        !self.name.trim().is_empty()
            && (self.executables.iter().any(|e| !e.trim().is_empty())
                || self
                    .title_contains
                    .as_deref()
                    .is_some_and(|t| !t.trim().is_empty()))
    }

    /// True if `name` (a file name or a Unix/Windows path) is one of this game's executables.
    pub fn has_executable(&self, name: &str) -> bool {
        let file = file_name(name);
        !file.is_empty()
            && self
                .executables
                .iter()
                .any(|e| !e.trim().is_empty() && file.eq_ignore_ascii_case(e.trim()))
    }

    pub fn matches(&self, window: &WindowFacts) -> bool {
        let by_exe = window
            .class
            .iter()
            .chain(&window.process_names)
            .any(|n| self.has_executable(n));
        let by_title = self
            .title_contains
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .is_some_and(|t| window.title.to_lowercase().contains(&t.to_lowercase()));
        by_exe || by_title
    }
}

/// What is known about a top-level window, gathered by the capture backend.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WindowFacts {
    pub title: String,
    /// WM_CLASS instance and class names.
    pub class: Vec<String>,
    /// The owning process's name (`/proc/<pid>/comm`) and command-line arguments. Wine sets
    /// these to the Windows executable (path).
    pub process_names: Vec<String>,
}

/// The game `window` belongs to: the first entry in `games` that matches.
pub fn find_game<'a>(games: &'a [Game], window: &WindowFacts) -> Option<&'a Game> {
    games.iter().find(|g| g.matches(window))
}

/// Last component of a Unix or Windows path.
fn file_name(path: &str) -> &str {
    path.trim().rsplit(['/', '\\']).next().unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(title: &str, class: &[&str], process: &[&str]) -> WindowFacts {
        WindowFacts {
            title: title.into(),
            class: class.iter().map(|&s| s.into()).collect(),
            process_names: process.iter().map(|&s| s.into()).collect(),
        }
    }

    #[test]
    fn recognizes_wow_executables() {
        let wow = Game::world_of_warcraft();
        for name in [
            "Wow.exe",
            "WowClassic.exe",
            "wow.exe",
            r"C:\Program Files (x86)\World of Warcraft\_retail_\Wow.exe",
            "/home/me/Games/wow/drive_c/World of Warcraft/_retail_/Wow.exe",
        ] {
            assert!(wow.has_executable(name), "{name}");
        }
        for name in ["Battle.net.exe", "WowVoiceProxy.exe", "glxgears", "", "  "] {
            assert!(!wow.has_executable(name), "{name}");
        }
    }

    #[test]
    fn matches_by_class_process_or_title() {
        let wow = Game::world_of_warcraft();
        assert!(wow.matches(&facts("World of Warcraft", &["wow.exe", "wow.exe"], &[])));
        assert!(wow.matches(&facts(
            "",
            &["steam_app_faugus5dd8f7d6"],
            &["Wow.exe", r"Z:\games\_retail_\Wow.exe"]
        )));
        // The Battle.net launcher: same Proton class, different executable.
        assert!(!wow.matches(&facts(
            "Battle.net",
            &["steam_app_faugus5dd8f7d6"],
            &["Battle.net.exe"]
        )));
        // WoW has no title rule: a browser tab about WoW is not the game.
        assert!(!wow.matches(&facts(
            "World of Warcraft - Wowhead",
            &["google-chrome"],
            &[]
        )));

        let gears = Game::by_title("glxgears");
        assert!(gears.matches(&facts("GLXGears", &[], &[])));
        assert!(!gears.matches(&facts("xterm", &[], &["glxgears"])));
    }

    #[test]
    fn first_matching_game_wins() {
        let games = [
            Game::world_of_warcraft(),
            Game {
                name: "Elden Ring".into(),
                executables: vec!["eldenring.exe".into()],
                title_contains: None,
            },
        ];
        let elden = facts("ELDEN RING™", &["steam_app_1245620"], &["eldenring.exe"]);
        assert_eq!(
            find_game(&games, &elden).map(|g| g.name.as_str()),
            Some("Elden Ring")
        );
        assert_eq!(
            find_game(&games, &facts("xterm", &["xterm"], &["bash"])),
            None
        );
    }

    #[test]
    fn entries_without_rules_are_invalid() {
        assert!(Game::world_of_warcraft().is_valid());
        assert!(Game::by_title("x").is_valid());
        let empty = Game {
            name: "Nothing".into(),
            executables: vec![" ".into()],
            title_contains: Some(String::new()),
        };
        assert!(!empty.is_valid());
        assert!(!empty.matches(&facts("Nothing", &[" "], &[""])));
        assert!(
            !Game {
                name: " ".into(),
                ..Game::world_of_warcraft()
            }
            .is_valid()
        );
    }
}
