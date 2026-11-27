//! Game names from game launchers, so a game is called "ELDEN RING" rather than after its
//! window title or `eldenring.exe`.
//!
//! - **Steam**: the window's app id (WM_CLASS `steam_app_<id>` set by Proton, or `SteamAppId`
//!   in the game's environment) → `steamapps/appmanifest_<id>.acf` in any Steam library.
//! - **Faugus Launcher**: `games.json`; an entry counts only if the window runs the entry's own
//!   executable in the entry's prefix. (WoW runs inside the Battle.net entry's prefix, with
//!   Battle.net's environment, and must not be called "Battle.net".)
//!
//! Only the launchers' own files are read; nothing about the game process beyond what the
//! capture backend already collects in [`WindowFacts`].

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
];

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
    /// Read the launchers' files. `home` is the home folder, `data_home` `$XDG_DATA_HOME`.
    /// Missing or unreadable files just mean no names from that launcher.
    pub fn load(home: &Path, data_home: &Path) -> Self {
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
        Self { steam, faugus }
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
fn wine_prefix_root(prefix: &str) -> PathBuf {
    let path = Path::new(prefix.trim_end_matches('/'));
    if path.file_name().is_some_and(|f| f == "pfx") {
        path.parent().unwrap_or(path).to_owned()
    } else {
        path.to_owned()
    }
}

/// Steam library folders: each Steam install plus the libraries its `libraryfolders.vdf`
/// lists (other drives). Native, `~/.steam` symlink and Flatpak installs.
fn steam_libraries(home: &Path, data_home: &Path) -> Vec<PathBuf> {
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

        let launchers = Launchers::load(home.path(), &data);
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
    fn broken_faugus_file_gives_nothing() {
        assert!(parse_faugus("not json").is_empty());
        assert!(parse_faugus(r#"[{"title": "", "path": "x.exe", "prefix": "/p"}]"#).is_empty());
    }
}
