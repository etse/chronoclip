//! Game names from game launchers, so a game is called "ELDEN RING" rather than after its
//! window title or `eldenring.exe`.
//!
//! - **Steam**: the window's app id (WM_CLASS `steam_app_<id>` set by Proton, or `SteamAppId`
//!   in the game's environment) → `steamapps/appmanifest_<id>.acf` in any Steam library.
//! - **Heroic** (Epic, GOG, Amazon, sideloaded, …): `HEROIC_APP_NAME` in the environment → the
//!   title in Heroic's library caches (`store_cache/*_library.json`, `sideload_apps/library.json`).
//! - **Lutris**: `GAME_NAME` in the environment (Lutris sets it for every game it starts).
//! - **Faugus Launcher**: `games.json`; an entry counts only if the window runs the entry's own
//!   executable in the entry's prefix.
//!
//! A game started from a store launcher (WoW from Battle.net) inherits the launcher entry's
//! environment: Lutris's `GAME_NAME` is then "Battle.net". Names of known store launchers are
//! therefore ignored, and Faugus entries must match the executable.
//!
//! Only the launchers' own files are read; nothing about the game process beyond what the
//! capture backend already collects in [`WindowFacts`]. Lutris and Heroic follow their source
//! code (Lutris `game.py` `configure_game`, Heroic `launcher.ts` `setupWrapperEnvVars` and its
//! backup tests); the maintainer has neither installed.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::games::WindowFacts;

/// Environment variables of a game process the backend passes on (all others are ignored,
/// never read into memory beyond the scan): Steam/Proton/UMU ids and the Wine prefix.
pub const ENV_KEYS: &[&str] = &[
    "SteamAppId",
    "SteamGameId",
    "STEAM_COMPAT_APP_ID",
    "WINEPREFIX",
    "GAME_NAME",
    "HEROIC_APP_NAME",
];

/// Store launchers: a game named like one of these by a launcher entry is probably a game the
/// store launcher started (WoW from Battle.net), so the name is not used.
fn is_store_launcher(name: &str) -> bool {
    const LAUNCHERS: &[&str] = &[
        "battle.net",
        "blizzard",
        "ea app",
        "origin",
        "ubisoft connect",
        "uplay",
        "epic games",
        "rockstar games launcher",
        "gog galaxy",
        "amazon games",
        "itch",
        "steam",
    ];
    let name = name.trim().to_lowercase();
    LAUNCHERS
        .iter()
        .any(|l| name == *l || name.starts_with(&format!("{l} ")))
}

/// Steam "apps" that are tools, not games.
fn is_steam_tool(name: &str) -> bool {
    name.starts_with("Proton")
        || name.starts_with("Steam Linux Runtime")
        || name == "Steamworks Common Redistributables"
}

/// Installed games known to the launchers on this computer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Launchers {
    /// Steam app id → name.
    steam: HashMap<u32, String>,
    /// Heroic app name → title.
    heroic: HashMap<String, String>,
    faugus: Vec<FaugusGame>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FaugusGame {
    title: String,
    /// File name of the game's executable, e.g. `Battle.net.exe`.
    executable: String,
    /// Wine prefix (without a trailing `/pfx`).
    prefix: PathBuf,
}

impl Launchers {
    /// Read the launchers' files. `home` is the home folder, `data_home` `$XDG_DATA_HOME`,
    /// `config_home` `$XDG_CONFIG_HOME`. Missing or unreadable files just mean no names from
    /// that launcher.
    pub fn load(home: &Path, data_home: &Path, config_home: &Path) -> Self {
        let mut steam = HashMap::new();
        for library in steam_libraries(home, data_home) {
            let Ok(entries) = fs::read_dir(library.join("steamapps")) else {
                continue;
            };
            for entry in entries.flatten() {
                let file = entry.file_name();
                let Some(id) = file
                    .to_str()
                    .and_then(|f| f.strip_prefix("appmanifest_")?.strip_suffix(".acf"))
                    .and_then(|id| id.parse::<u32>().ok())
                else {
                    continue;
                };
                if let Some(name) = fs::read_to_string(entry.path())
                    .ok()
                    .and_then(|text| vdf_value(&text, "name"))
                    .filter(|name| !is_steam_tool(name))
                {
                    steam.insert(id, name);
                }
            }
        }
        let faugus = fs::read_to_string(data_home.join("faugus-launcher/games.json"))
            .ok()
            .map(|text| parse_faugus(&text))
            .unwrap_or_default();
        let mut heroic = HashMap::new();
        for dir in [
            config_home.join("heroic"),
            home.join(".var/app/com.heroicgameslauncher.hgl/config/heroic"),
        ] {
            let mut files: Vec<PathBuf> = fs::read_dir(dir.join("store_cache"))
                .map(|entries| {
                    entries
                        .flatten()
                        .map(|e| e.path())
                        .filter(|p| {
                            p.file_name()
                                .and_then(|f| f.to_str())
                                .is_some_and(|f| f.ends_with("_library.json"))
                        })
                        .collect()
                })
                .unwrap_or_default();
            files.push(dir.join("sideload_apps/library.json"));
            for file in files {
                if let Ok(text) = fs::read_to_string(&file) {
                    for (app, title) in heroic_titles(&text) {
                        heroic.entry(app).or_insert(title);
                    }
                }
            }
        }
        Self {
            steam,
            heroic,
            faugus,
        }
    }

    /// Installed Steam games as `(app id, name)`, sorted by name (tools such as Proton
    /// excluded).
    pub fn steam_games(&self) -> Vec<(u32, &str)> {
        let mut games: Vec<(u32, &str)> = self
            .steam
            .iter()
            .map(|(&id, name)| (id, name.as_str()))
            .collect();
        games.sort_by_key(|(_, name)| name.to_lowercase());
        games
    }

    /// Wine prefixes of the Faugus Launcher entries.
    pub fn faugus_prefixes(&self) -> Vec<PathBuf> {
        self.faugus.iter().map(|g| g.prefix.clone()).collect()
    }

    /// Name of installed Steam game `id` (tools such as Proton excluded).
    pub fn steam_name(&self, id: u32) -> Option<&str> {
        self.steam.get(&id).map(String::as_str)
    }

    /// The launcher's name for the game in `window`, if a launcher knows it.
    pub fn name_for(&self, window: &WindowFacts) -> Option<String> {
        if let Some(name) = steam_app_id(window).and_then(|id| self.steam.get(&id)) {
            return Some(name.clone());
        }
        if let Some(name) = window
            .env("HEROIC_APP_NAME")
            .and_then(|app| self.heroic.get(app))
        {
            return Some(name.clone());
        }
        if let Some(name) = window
            .env("GAME_NAME")
            .map(str::trim)
            .filter(|n| !n.is_empty() && !is_store_launcher(n))
        {
            return Some(name.to_owned());
        }
        let prefix = window.env("WINEPREFIX").map(wine_prefix_root)?;
        let exe = window.executable()?;
        self.faugus
            .iter()
            .find(|g| g.prefix == prefix && g.executable.eq_ignore_ascii_case(&exe))
            .map(|g| g.title.clone())
    }
}

/// The Steam app id of a window: WM_CLASS `steam_app_<id>` (Proton), else `SteamAppId` or
/// `SteamGameId` in the environment. Non-Steam launchers that run through UMU set these to
/// non-numeric ids (e.g. `faugus5dd8f7d6`): ignored.
pub fn steam_app_id(window: &WindowFacts) -> Option<u32> {
    window
        .class
        .iter()
        .filter_map(|c| c.strip_prefix("steam_app_"))
        .chain(
            ["SteamAppId", "SteamGameId"]
                .iter()
                .filter_map(|k| window.env(k)),
        )
        .filter_map(|id| id.parse::<u32>().ok())
        .find(|&id| id != 0)
}

/// `/home/me/Faugus/battlenet/pfx/` → `/home/me/Faugus/battlenet` (Proton keeps the Wine
/// prefix in `pfx/` inside the folder launchers call the prefix).
pub(crate) fn wine_prefix_root(prefix: &str) -> PathBuf {
    let path = Path::new(prefix.trim_end_matches('/'));
    if path.file_name().is_some_and(|f| f == "pfx") {
        path.parent().unwrap_or(path).to_owned()
    } else {
        path.to_owned()
    }
}

/// Steam library folders: each Steam install plus the libraries its `libraryfolders.vdf`
/// lists (other drives). Native, `~/.steam` symlink and Flatpak installs.
pub(crate) fn steam_libraries(home: &Path, data_home: &Path) -> Vec<PathBuf> {
    let roots = [
        data_home.join("Steam"),
        home.join(".steam/steam"),
        home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"),
    ];
    let mut libraries: Vec<PathBuf> = Vec::new();
    let mut add = |path: PathBuf| {
        let path = path.canonicalize().unwrap_or(path);
        if path.is_dir() && !libraries.contains(&path) {
            libraries.push(path);
        }
    };
    for root in roots {
        let folders = fs::read_to_string(root.join("steamapps/libraryfolders.vdf"))
            .map(|text| vdf_values(&text, "path"))
            .unwrap_or_default();
        add(root);
        for folder in folders {
            add(PathBuf::from(folder));
        }
    }
    libraries
}

/// Values of `"key"  "value"` lines in a Valve KeyValues (VDF/ACF) file, in order. Enough for
/// the flat fields we read (`name`, `path`); escapes other than `\\` don't occur in them.
fn vdf_values(text: &str, key: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            let mut parts = line.split('"').skip(1).step_by(2);
            let k = parts.next()?;
            let v = parts.next()?;
            k.eq_ignore_ascii_case(key).then(|| v.replace("\\\\", "\\"))
        })
        .collect()
}

fn vdf_value(text: &str, key: &str) -> Option<String> {
    vdf_values(text, key)
        .into_iter()
        .next()
        .filter(|v| !v.trim().is_empty())
}

/// `(app_name, title)` of every game in a Heroic library file. The files differ per store
/// (`{"games": [...]}`, `{"library": [...]}`, …), so any object with both fields counts.
fn heroic_titles(text: &str) -> Vec<(String, String)> {
    fn walk(value: &serde_json::Value, out: &mut Vec<(String, String)>) {
        match value {
            serde_json::Value::Object(map) => {
                let field = |k: &str| map.get(k).and_then(|v| v.as_str()).map(str::trim);
                if let (Some(app), Some(title)) = (field("app_name"), field("title"))
                    && !app.is_empty()
                    && !title.is_empty()
                {
                    out.push((app.to_owned(), title.to_owned()));
                }
                map.values().for_each(|v| walk(v, out));
            }
            serde_json::Value::Array(items) => items.iter().for_each(|v| walk(v, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    match serde_json::from_str(text) {
        Ok(value) => walk(&value, &mut out),
        Err(e) => tracing::debug!("cannot read a Heroic library file: {e}"),
    }
    out
}

fn parse_faugus(text: &str) -> Vec<FaugusGame> {
    #[derive(Deserialize)]
    struct Entry {
        #[serde(default)]
        title: String,
        #[serde(default)]
        path: String,
        #[serde(default)]
        prefix: String,
    }
    let entries: Vec<Entry> = match serde_json::from_str(text) {
        Ok(entries) => entries,
        Err(e) => {
            tracing::debug!("cannot read Faugus games.json: {e}");
            return Vec::new();
        }
    };
    entries
        .into_iter()
        .filter_map(|e| {
            let executable = e.path.rsplit(['/', '\\']).next()?.trim().to_owned();
            (!e.title.trim().is_empty() && !executable.is_empty() && !e.prefix.is_empty()).then(
                || FaugusGame {
                    title: e.title.trim().to_owned(),
                    executable,
                    prefix: wine_prefix_root(&e.prefix),
                },
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window(class: &[&str], process: &[&str], env: &[(&str, &str)]) -> WindowFacts {
        WindowFacts {
            title: "window".into(),
            class: class.iter().map(|&s| s.into()).collect(),
            process_names: process.iter().map(|&s| s.into()).collect(),
            env: env.iter().map(|&(k, v)| (k.into(), v.into())).collect(),
        }
    }

    #[test]
    fn steam_app_id_from_class_or_environment() {
        assert_eq!(
            steam_app_id(&window(&["steam_app_1245620"], &[], &[])),
            Some(1_245_620)
        );
        assert_eq!(
            steam_app_id(&window(&[], &[], &[("SteamAppId", "526870")])),
            Some(526_870)
        );
        // Faugus/UMU: non-numeric ids.
        let faugus = window(
            &["steam_app_faugus5dd8f7d6"],
            &[],
            &[
                ("SteamAppId", "faugus5dd8f7d6"),
                ("SteamGameId", "faugus5dd8f7d6"),
            ],
        );
        assert_eq!(steam_app_id(&faugus), None);
        assert_eq!(
            steam_app_id(&window(&[], &[], &[("SteamAppId", "0")])),
            None
        );
    }

    #[test]
    fn reads_steam_manifests_and_libraries() {
        let acf = "\"AppState\"\n{\n\t\"appid\"\t\t\"1245620\"\n\t\"universe\"\t\t\"1\"\n\t\"name\"\t\t\"ELDEN RING\"\n}\n";
        assert_eq!(vdf_value(acf, "name").as_deref(), Some("ELDEN RING"));
        assert_eq!(vdf_value(acf, "missing"), None);
        let vdf = "\"libraryfolders\"\n{\n\t\"0\"\n\t{\n\t\t\"path\"\t\t\"/home/me/.local/share/Steam\"\n\t}\n\t\"1\"\n\t{\n\t\t\"path\"\t\t\"/mnt/games/SteamLibrary\"\n\t}\n}\n";
        assert_eq!(
            vdf_values(vdf, "path"),
            ["/home/me/.local/share/Steam", "/mnt/games/SteamLibrary"]
        );
    }

    #[test]
    fn loads_steam_and_faugus_from_disk() -> Result<(), Box<dyn std::error::Error>> {
        let home = tempfile::tempdir()?;
        let data = home.path().join(".local/share");
        let lib = home.path().join("games/SteamLibrary");
        fs::create_dir_all(data.join("Steam/steamapps"))?;
        fs::create_dir_all(lib.join("steamapps"))?;
        fs::create_dir_all(data.join("faugus-launcher"))?;
        fs::write(
            data.join("Steam/steamapps/libraryfolders.vdf"),
            format!(
                "\"libraryfolders\"\n{{\n\t\"1\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n}}\n",
                lib.display()
            ),
        )?;
        let manifest = |id: u32, name: &str| {
            format!("\"AppState\"\n{{\n\t\"appid\"\t\t\"{id}\"\n\t\"name\"\t\t\"{name}\"\n}}\n")
        };
        fs::write(
            data.join("Steam/steamapps/appmanifest_1245620.acf"),
            manifest(1245620, "ELDEN RING"),
        )?;
        fs::write(
            lib.join("steamapps/appmanifest_526870.acf"),
            manifest(526870, "Satisfactory"),
        )?;
        fs::write(
            lib.join("steamapps/appmanifest_1493710.acf"),
            manifest(1493710, "Proton Experimental"),
        )?;
        fs::write(
            data.join("faugus-launcher/games.json"),
            r#"[{"gameid": "battlenet", "title": "Battle.net",
                 "path": "/home/me/Faugus/battlenet/drive_c/Program Files (x86)/Battle.net/Battle.net.exe",
                 "prefix": "/home/me/Faugus/battlenet", "runner": "Proton"}]"#,
        )?;

        let launchers = Launchers::load(home.path(), &data, &home.path().join(".config"));
        let steam = |id: &str| window(&[&format!("steam_app_{id}")], &[], &[]);
        assert_eq!(
            launchers.name_for(&steam("1245620")).as_deref(),
            Some("ELDEN RING")
        );
        assert_eq!(
            launchers.name_for(&steam("526870")).as_deref(),
            Some("Satisfactory")
        );
        assert_eq!(launchers.name_for(&steam("1493710")), None); // a tool
        assert_eq!(launchers.name_for(&steam("999")), None); // not installed

        let prefix = [("WINEPREFIX", "/home/me/Faugus/battlenet/pfx/")];
        let battlenet = window(&[], &["Battle.net.exe"], &prefix);
        assert_eq!(
            launchers.name_for(&battlenet).as_deref(),
            Some("Battle.net")
        );
        // WoW started from Battle.net: same prefix and environment, different executable.
        let wow = window(
            &[],
            &[
                "Wow.exe",
                r"C:\Program Files (x86)\World of Warcraft\_retail_\Wow.exe",
            ],
            &prefix,
        );
        assert_eq!(launchers.name_for(&wow), None);
        Ok(())
    }

    #[test]
    fn lutris_names_but_not_store_launchers() {
        let launchers = Launchers::default();
        let lutris = |name: &str| window(&[], &["game.exe"], &[("GAME_NAME", name)]);
        assert_eq!(
            launchers.name_for(&lutris("Hades")).as_deref(),
            Some("Hades")
        );
        // Diablo IV started from Lutris's "Battle.net" entry inherits its GAME_NAME.
        assert_eq!(launchers.name_for(&lutris("Battle.net")), None);
        assert_eq!(launchers.name_for(&lutris("EA App")), None);
        assert_eq!(launchers.name_for(&lutris("Ubisoft Connect")), None);
        assert_eq!(launchers.name_for(&lutris(" ")), None);
        // Not a launcher, despite the prefix.
        assert_eq!(
            launchers.name_for(&lutris("Steamworld Dig")).as_deref(),
            Some("Steamworld Dig")
        );
    }

    #[test]
    fn loads_heroic_titles() -> Result<(), Box<dyn std::error::Error>> {
        // Shapes from Heroic's own tests (e2e/backup.spec.ts), native and Flatpak.
        let home = tempfile::tempdir()?;
        let config = home.path().join(".config");
        let native = config.join("heroic");
        let flatpak = home
            .path()
            .join(".var/app/com.heroicgameslauncher.hgl/config/heroic");
        fs::create_dir_all(native.join("store_cache"))?;
        fs::create_dir_all(native.join("sideload_apps"))?;
        fs::create_dir_all(flatpak.join("store_cache"))?;
        fs::write(
            native.join("store_cache/gog_library.json"),
            r#"{"games": [{"app_name": "1207658924", "title": "Unreal Tournament 2004"}]}"#,
        )?;
        fs::write(
            native.join("store_cache/itchio_library.json"),
            r#"{"library": [{"app_name": "itchio-1", "title": "Itch Test", "is_installed": true}]}"#,
        )?;
        fs::write(
            native.join("sideload_apps/library.json"),
            r#"{"games": [{"app_name": "sideload-test", "title": "Sideload Test", "art_cover": ""}]}"#,
        )?;
        fs::write(
            flatpak.join("store_cache/legendary_library.json"),
            r#"{"library": [{"app_name": "Fortnite", "title": "Fortnite", "runner": "legendary"}]}"#,
        )?;
        fs::write(native.join("store_cache/nile_library.json"), "not json")?;

        let launchers = Launchers::load(home.path(), &home.path().join(".local/share"), &config);
        let heroic = |app: &str| window(&[], &["game.exe"], &[("HEROIC_APP_NAME", app)]);
        assert_eq!(
            launchers.name_for(&heroic("1207658924")).as_deref(),
            Some("Unreal Tournament 2004")
        );
        assert_eq!(
            launchers.name_for(&heroic("itchio-1")).as_deref(),
            Some("Itch Test")
        );
        assert_eq!(
            launchers.name_for(&heroic("sideload-test")).as_deref(),
            Some("Sideload Test")
        );
        assert_eq!(
            launchers.name_for(&heroic("Fortnite")).as_deref(),
            Some("Fortnite")
        );
        assert_eq!(launchers.name_for(&heroic("unknown")), None);
        Ok(())
    }

    #[test]
    fn broken_faugus_file_gives_nothing() {
        assert!(parse_faugus("not json").is_empty());
        assert!(parse_faugus(r#"[{"title": "", "path": "x.exe", "prefix": "/p"}]"#).is_empty());
    }
}
