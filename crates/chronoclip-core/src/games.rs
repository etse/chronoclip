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

    /// A new game entry for a window the user picked: named after the window title (or the
    /// executable), matched by its executable when it has a specific one, else by title.
    pub fn from_window(window: &WindowFacts) -> Self {
        let title = window.title.trim();
        let exe = window.executable();
        let name = if title.is_empty() {
            exe.as_deref()
                .map(|e| e.strip_suffix(".exe").unwrap_or(e).to_owned())
                .unwrap_or_else(|| "Game".to_owned())
        } else {
            title.to_owned()
        };
        match exe {
            Some(exe) => Self {
                name,
                executables: vec![exe],
                title_contains: None,
            },
            None => Self {
                name,
                executables: Vec::new(),
                title_contains: Some(title.to_owned()),
            },
        }
    }

    /// How the game is recognized, for the games list, e.g. `eldenring.exe` or
    /// `Window title contains “Factorio”`.
    pub fn describe(&self) -> String {
        let exes: Vec<&str> = self
            .executables
            .iter()
            .map(|e| e.trim())
            .filter(|e| !e.is_empty())
            .collect();
        let title = self
            .title_contains
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(|t| format!("Window title contains “{t}”"));
        let exes = match exes.len() {
            0 => None,
            1..=3 => Some(exes.join(", ")),
            n => Some(format!("{} and {} more", exes[..2].join(", "), n - 2)),
        };
        [exes, title]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ")
    }

    /// True if this entry already covers `other` (same executable or same title rule), so
    /// adding `other` would be a duplicate.
    pub fn covers(&self, other: &Game) -> bool {
        other.executables.iter().any(|e| self.has_executable(e))
            || other.title_contains.as_deref().is_some_and(|t| {
                self.title_contains
                    .as_deref()
                    .is_some_and(|own| own.trim().eq_ignore_ascii_case(t.trim()))
            })
    }
}

/// Programs that run many different games or apps: their name doesn't identify a game, so a
/// picked window running one of them is matched by its title instead.
const GENERIC_EXECUTABLES: &[&str] = &[
    "wine",
    "wine64",
    "wine-preloader",
    "wine64-preloader",
    "wineserver",
    "start.exe",
    "explorer.exe",
    "python",
    "python3",
    "java",
    "mono",
    "love",
    "godot",
    "bash",
    "sh",
];

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

impl WindowFacts {
    /// The executable that identifies the window's program: a Windows `.exe` (Wine/Proton),
    /// else the program file of the command line, else the process name. `None` for generic
    /// runners (Wine itself, Python, Java, …).
    pub fn executable(&self) -> Option<String> {
        // Some programs (Chrome, Electron apps) rewrite their command line into one string
        // with the options: cut those off.
        let names: Vec<&str> = self
            .process_names
            .iter()
            .map(|n| {
                let file = file_name(n);
                file.split_once(" -").map_or(file, |(f, _)| f).trim()
            })
            .filter(|n| !n.is_empty())
            .collect();
        let is_generic = |n: &&str| {
            GENERIC_EXECUTABLES
                .iter()
                .any(|g| g.eq_ignore_ascii_case(n))
                || n.to_ascii_lowercase().starts_with("python")
        };
        let exe = names
            .iter()
            .find(|n| n.to_ascii_lowercase().ends_with(".exe") && !is_generic(n))
            .or_else(|| names.get(1).or(names.first()))
            .filter(|n| !is_generic(n))?;
        Some((*exe).to_owned())
    }
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
    fn picked_window_becomes_a_game() {
        // Proton game: the .exe from the command line, named after the window.
        let elden = Game::from_window(&facts(
            "ELDEN RING™",
            &["steam_app_1245620"],
            &[
                "eldenring.exe",
                r"Z:\home\me\Games\ELDEN RING\Game\eldenring.exe",
            ],
        ));
        assert_eq!(elden.name, "ELDEN RING™");
        assert_eq!(elden.executables, ["eldenring.exe"]);
        assert_eq!(elden.title_contains, None);
        assert_eq!(elden.describe(), "eldenring.exe");
        // Native game: the program file.
        let factorio = Game::from_window(&facts(
            "Factorio 2.0.55",
            &["factorio", "factorio"],
            &[
                "factorio",
                "/home/me/.steam/steamapps/common/Factorio/bin/x64/factorio",
            ],
        ));
        assert_eq!(factorio.executables, ["factorio"]);
        let chrome = facts(
            "YouTube",
            &[],
            &[
                "chrome",
                "/opt/google/chrome/chrome --ozone-platform=x11 --restart",
            ],
        );
        assert_eq!(chrome.executable().as_deref(), Some("chrome"));
        // A generic runner: matched by title instead.
        let py = Game::from_window(&facts(
            "My Pygame",
            &[],
            &["python3", "/usr/bin/python3", "game.py"],
        ));
        assert_eq!(py.executables, Vec::<String>::new());
        assert_eq!(py.title_contains.as_deref(), Some("My Pygame"));
        assert_eq!(py.describe(), "Window title contains “My Pygame”");
        // No title: named after the executable.
        assert_eq!(
            Game::from_window(&facts(" ", &[], &["Game.exe"])).name,
            "Game"
        );
    }

    #[test]
    fn duplicates_are_detected() {
        let wow = Game::world_of_warcraft();
        let picked = Game::from_window(&facts("World of Warcraft", &[], &["Wow.exe"]));
        assert!(wow.covers(&picked));
        let elden = Game::from_window(&facts("ELDEN RING", &[], &["eldenring.exe"]));
        assert!(!wow.covers(&elden));
        assert!(elden.covers(&Game::from_window(&facts(
            "Other title",
            &[],
            &["EldenRing.exe"]
        ))));
        let t = Game::by_title("Pygame");
        assert!(t.covers(&Game::by_title("pygame ")));
        assert!(!t.covers(&Game::by_title("Other")));
        assert!(
            wow.describe()
                .starts_with("Wow.exe, WowClassic.exe and 4 more")
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
