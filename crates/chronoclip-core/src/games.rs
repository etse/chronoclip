//! Games Chronoclip records, and how a window is recognized as one of them.
//!
//! The backend collects [`WindowFacts`] about each top-level window (title, WM_CLASS, the
//! owning process's names); [`find_game`] then picks the game it belongs to. Pure logic, so
//! matching rules are unit-tested here.

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

/// Name of the built-in World of Warcraft entry.
pub const WORLD_OF_WARCRAFT: &str = "World of Warcraft";

/// The built-in games catalog (`data/games.toml`), compiled in.
const CATALOG: &str = include_str!("../data/games.toml");

/// The built-in games, in the file's order. The file is checked by the `catalog_*` tests, so
/// it always parses; if it somehow doesn't, there are no built-in games (logged).
pub fn catalog() -> &'static [Game] {
    static GAMES: OnceLock<Vec<Game>> = OnceLock::new();
    GAMES.get_or_init(|| match parse_catalog(CATALOG) {
        Ok(games) => games,
        Err(e) => {
            tracing::error!("the built-in games catalog is broken: {e}");
            Vec::new()
        }
    })
}

/// Strict: a misspelled key (`executable = …`) is an error in the catalog, unlike in the
/// settings file, which must keep loading when a newer version wrote fields we don't know.
fn parse_catalog(text: &str) -> Result<Vec<Game>, toml::de::Error> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Catalog {
        game: Vec<Entry>,
    }
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Entry {
        name: String,
        #[serde(default)]
        executables: Vec<String>,
        #[serde(default)]
        steam_app_ids: Vec<u32>,
        title_contains: Option<String>,
    }
    Ok(toml::from_str::<Catalog>(text)?
        .game
        .into_iter()
        .map(|e| Game {
            name: e.name,
            executables: e.executables,
            steam_app_ids: e.steam_app_ids,
            title_contains: e.title_contains,
        })
        .collect())
}

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
    /// Steam app ids: Proton tags the game's window with it (WM_CLASS `steam_app_<id>`) and
    /// Steam sets `SteamAppId` for the game's processes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steam_app_ids: Vec<u32>,
    /// Text the window title contains (case-insensitive). Least reliable; for windows without
    /// a usable executable name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_contains: Option<String>,
}

impl Game {
    /// World of Warcraft from the catalog.
    pub fn world_of_warcraft() -> Self {
        catalog()
            .iter()
            .find(|g| g.name == WORLD_OF_WARCRAFT)
            .cloned()
            // Only if the catalog were broken (the tests rule that out).
            .unwrap_or_else(|| Self {
                executables: vec!["Wow.exe".to_owned()],
                ..Self::by_title(WORLD_OF_WARCRAFT)
            })
    }

    /// A game matched by window title only (the `CHRONOCLIP_WINDOW` override and tests).
    pub fn by_title(title: &str) -> Self {
        Self {
            name: title.to_owned(),
            executables: Vec::new(),
            steam_app_ids: Vec::new(),
            title_contains: Some(title.to_owned()),
        }
    }

    /// True if the entry can match anything: a name and at least one non-empty rule.
    pub fn is_valid(&self) -> bool {
        !self.name.trim().is_empty()
            && (self.executables.iter().any(|e| !e.trim().is_empty())
                || self.steam_app_ids.iter().any(|&id| id != 0)
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
        let by_steam = crate::launchers::steam_app_id(window)
            .is_some_and(|id| self.steam_app_ids.contains(&id));
        let by_title = self
            .title_contains
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .is_some_and(|t| window.title.to_lowercase().contains(&t.to_lowercase()));
        by_exe || by_steam || by_title
    }

    /// A new game entry for a window the user picked: named by its launcher (`launcher_name`,
    /// see [`crate::launchers::Launchers::name_for`]), else after the window title or the
    /// executable; matched by its executable when it has a specific one, else by title.
    pub fn from_window(window: &WindowFacts, launcher_name: Option<String>) -> Self {
        let title = window.title.trim();
        let exe = window.executable();
        let name = if let Some(name) = launcher_name.filter(|n| !n.trim().is_empty()) {
            name.trim().to_owned()
        } else if title.is_empty() {
            exe.as_deref()
                .map(|e| e.strip_suffix(".exe").unwrap_or(e).to_owned())
                .unwrap_or_else(|| "Game".to_owned())
        } else {
            title.to_owned()
        };
        let steam_app_ids: Vec<u32> = crate::launchers::steam_app_id(window).into_iter().collect();
        match exe {
            Some(exe) => Self {
                name,
                executables: vec![exe],
                steam_app_ids,
                title_contains: None,
            },
            // A Steam id is enough to recognize it; the title only when there's nothing else.
            None => Self {
                name,
                executables: Vec::new(),
                title_contains: steam_app_ids.is_empty().then(|| title.to_owned()),
                steam_app_ids,
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
        let steam = match self.steam_app_ids.as_slice() {
            [] => None,
            [id] => Some(format!("Steam app {id}")),
            ids => Some(format!(
                "Steam apps {}",
                ids.iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
        };
        [exes, steam, title]
            .into_iter()
            .flatten()
            .collect::<Vec<_>>()
            .join(" · ")
    }

    /// True if this entry already covers `other` (same executable or same title rule), so
    /// adding `other` would be a duplicate.
    pub fn covers(&self, other: &Game) -> bool {
        other.executables.iter().any(|e| self.has_executable(e))
            || other
                .steam_app_ids
                .iter()
                .any(|id| self.steam_app_ids.contains(id))
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
    /// The owning process's launcher variables ([`crate::launchers::ENV_KEYS`] only).
    pub env: Vec<(String, String)>,
}

impl WindowFacts {
    /// Value of launcher variable `key`, if set.
    pub fn env(&self, key: &str) -> Option<&str> {
        self.env
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

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
            env: Vec::new(),
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
                steam_app_ids: Vec::new(),
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
        let elden = Game::from_window(
            &facts(
                "ELDEN RING™",
                &["steam_app_1245620"],
                &[
                    "eldenring.exe",
                    r"Z:\home\me\Games\ELDEN RING\Game\eldenring.exe",
                ],
            ),
            None,
        );
        assert_eq!(elden.name, "ELDEN RING™");
        assert_eq!(elden.executables, ["eldenring.exe"]);
        assert_eq!(elden.steam_app_ids, [1_245_620]);
        assert_eq!(elden.title_contains, None);
        assert_eq!(elden.describe(), "eldenring.exe · Steam app 1245620");
        // Native game: the program file.
        let factorio = Game::from_window(
            &facts(
                "Factorio 2.0.55",
                &["factorio", "factorio"],
                &[
                    "factorio",
                    "/home/me/.steam/steamapps/common/Factorio/bin/x64/factorio",
                ],
            ),
            None,
        );
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
        let py = Game::from_window(
            &facts(
                "My Pygame",
                &[],
                &["python3", "/usr/bin/python3", "game.py"],
            ),
            None,
        );
        assert_eq!(py.executables, Vec::<String>::new());
        assert_eq!(py.title_contains.as_deref(), Some("My Pygame"));
        assert_eq!(py.describe(), "Window title contains “My Pygame”");
        // No title: named after the executable.
        assert_eq!(
            Game::from_window(&facts(" ", &[], &["Game.exe"]), None).name,
            "Game"
        );
        // The launcher's name wins over the window title.
        let named = Game::from_window(
            &facts("ELDEN RING™", &[], &["eldenring.exe"]),
            Some("ELDEN RING".into()),
        );
        assert_eq!(named.name, "ELDEN RING");
        assert_eq!(named.executables, ["eldenring.exe"]);
    }

    #[test]
    fn duplicates_are_detected() {
        let wow = Game::world_of_warcraft();
        let picked = Game::from_window(&facts("World of Warcraft", &[], &["Wow.exe"]), None);
        assert!(wow.covers(&picked));
        let elden = Game::from_window(&facts("ELDEN RING", &[], &["eldenring.exe"]), None);
        assert!(!wow.covers(&elden));
        assert!(elden.covers(&Game::from_window(
            &facts("Other title", &[], &["EldenRing.exe"]),
            None
        )));
        let t = Game::by_title("Pygame");
        assert!(t.covers(&Game::by_title("pygame ")));
        assert!(!t.covers(&Game::by_title("Other")));
        assert!(
            wow.describe()
                .starts_with("Wow.exe, WowClassic.exe and 4 more")
        );
    }

    #[test]
    fn matches_by_steam_app_id() {
        let elden = Game {
            name: "ELDEN RING".into(),
            executables: Vec::new(),
            steam_app_ids: vec![1_245_620],
            title_contains: None,
        };
        assert!(elden.is_valid());
        assert!(elden.matches(&facts("ELDEN RING", &["steam_app_1245620"], &[])));
        let mut env = facts("x", &[], &["start.exe"]);
        env.env = vec![("SteamAppId".into(), "1245620".into())];
        assert!(elden.matches(&env));
        assert!(!elden.matches(&facts("ELDEN RING", &["steam_app_526870"], &[])));
        // A picked Proton game without a usable executable: the Steam id instead of the title.
        let picked = Game::from_window(
            &facts("Some Game", &["steam_app_42"], &["wine64-preloader"]),
            None,
        );
        assert_eq!(picked.steam_app_ids, [42]);
        assert_eq!(picked.title_contains, None);
        assert!(elden.covers(&Game {
            name: "x".into(),
            ..elden.clone()
        }));
    }

    /// The catalog file parses, and every entry is usable and unique (checked on every pull
    /// request by CI).
    #[test]
    fn catalog_entries_are_valid_and_unique() {
        let games = parse_catalog(CATALOG).unwrap_or_else(|e| panic!("data/games.toml: {e}"));
        assert_eq!(games.len(), catalog().len());
        assert!(games.iter().any(|g| g.name == WORLD_OF_WARCRAFT));
        let mut names = std::collections::HashSet::new();
        let mut exes = std::collections::HashMap::new();
        let mut ids = std::collections::HashMap::new();
        for game in &games {
            assert!(
                game.is_valid(),
                "{:?}: needs a name and a way to match",
                game.name
            );
            assert_eq!(
                game.name,
                game.name.trim(),
                "{:?}: spaces around the name",
                game.name
            );
            assert!(
                names.insert(game.name.to_lowercase()),
                "{:?} is listed twice",
                game.name
            );
            for exe in &game.executables {
                assert_eq!(exe, exe.trim(), "{:?}: spaces around {exe:?}", game.name);
                assert!(
                    !exe.contains(['/', '\\']),
                    "{:?}: {exe:?} must be a file name, not a path",
                    game.name
                );
                let generic = WindowFacts {
                    process_names: vec![exe.clone()],
                    ..WindowFacts::default()
                };
                assert!(
                    generic.executable().is_some(),
                    "{:?}: {exe:?} is a generic program (Wine, Python, …)",
                    game.name
                );
                if let Some(other) = exes.insert(exe.to_lowercase(), &game.name) {
                    panic!("{exe:?} is used by both {other:?} and {:?}", game.name);
                }
            }
            for &id in &game.steam_app_ids {
                assert_ne!(id, 0, "{:?}: Steam app id 0", game.name);
                if let Some(other) = ids.insert(id, &game.name) {
                    panic!(
                        "Steam app {id} is used by both {other:?} and {:?}",
                        game.name
                    );
                }
            }
        }
    }

    #[test]
    fn catalog_rejects_mistakes() {
        assert!(parse_catalog("[[game]]\nname = \"X\"\nexecutable = [\"x.exe\"]\n").is_err()); // typo
        assert!(parse_catalog("[[game]]\nname = \"X\"\nsteam_app_ids = [\"12\"]\n").is_err());
    }

    #[test]
    fn entries_without_rules_are_invalid() {
        assert!(Game::world_of_warcraft().is_valid());
        assert!(Game::by_title("x").is_valid());
        let empty = Game {
            name: "Nothing".into(),
            steam_app_ids: Vec::new(),
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
