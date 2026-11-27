//! Games installed on this computer, found without the user adding them:
//!
//! - **Steam**: every installed Steam game (its catalog entry if there is one, else a game
//!   matched by its Steam app id).
//! - **Wine prefixes**: catalog games whose executable is in a Wine prefix's
//!   `Program Files` (Battle.net games, but also anything else from the catalog). Prefixes come
//!   from Faugus, Lutris, Heroic, Bottles, Steam (`compatdata`, e.g. non-Steam shortcuts),
//!   `~/.wine` and `~/Games/*`.
//!
//! Read at startup and when the Games tab opens; never stored, so an uninstalled game
//! disappears by itself.

use std::fs;
use std::path::{Path, PathBuf};

use crate::games::{Game, catalog};
use crate::launchers::{Launchers, steam_libraries, wine_prefix_root};

/// How deep below `Program Files` an executable may be: e.g.
/// `Heroes of the Storm/Versions/Base98785/HeroesOfTheStorm_x64.exe` is 4.
const SCAN_DEPTH: usize = 4;

/// A game found on this computer, and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstalledGame {
    pub game: Game,
    /// Where it was found, for the Games tab: "Steam" or the Wine prefix (`~/…`).
    pub source: String,
}

/// Find the installed games. `launchers` gives the Steam games and Faugus prefixes; `home`,
/// `data_home` and `config_home` are the home folder and `$XDG_DATA_HOME`/`$XDG_CONFIG_HOME`.
/// Sorted by name; each game once.
pub fn find_installed(
    launchers: &Launchers,
    home: &Path,
    data_home: &Path,
    config_home: &Path,
) -> Vec<InstalledGame> {
    let mut found: Vec<InstalledGame> = Vec::new();
    let mut add = |game: Game, source: String| {
        if !found
            .iter()
            .any(|f| f.game.name.eq_ignore_ascii_case(&game.name) || f.game.covers(&game))
        {
            found.push(InstalledGame { game, source });
        }
    };

    for (id, name) in launchers.steam_games() {
        let game = catalog()
            .iter()
            .find(|g| g.steam_app_ids.contains(&id))
            .cloned()
            .unwrap_or_else(|| Game {
                name: name.to_owned(),
                executables: Vec::new(),
                steam_app_ids: vec![id],
                title_contains: None,
            });
        add(game, "Steam".to_owned());
    }

    let catalog_exes: Vec<(String, &Game)> = catalog()
        .iter()
        .flat_map(|g| g.executables.iter().map(move |e| (e.to_lowercase(), g)))
        .collect();
    for prefix in wine_prefixes(launchers, home, data_home, config_home) {
        let Some(drive_c) = drive_c(&prefix) else {
            continue;
        };
        let source = match prefix.strip_prefix(home) {
            Ok(rel) => format!("~/{}", rel.display()),
            Err(_) => prefix.display().to_string(),
        };
        for dir in ["Program Files", "Program Files (x86)"] {
            let mut exes = Vec::new();
            collect_exe_names(&drive_c.join(dir), SCAN_DEPTH, &mut exes);
            for exe in exes {
                if let Some((_, game)) = catalog_exes.iter().find(|(e, _)| *e == exe) {
                    add((*game).clone(), source.clone());
                }
            }
        }
    }
    found.sort_by_key(|f| f.game.name.to_lowercase());
    found
}

/// Wine prefix folders the launchers know about (existing ones, each once).
fn wine_prefixes(
    launchers: &Launchers,
    home: &Path,
    data_home: &Path,
    config_home: &Path,
) -> Vec<PathBuf> {
    let mut candidates = launchers.faugus_prefixes();
    candidates.push(home.join(".wine"));
    candidates.extend(subdirs(&home.join("Games")));
    // Lutris: `prefix: /path` in each game's YAML config.
    for dir in [
        data_home.join("lutris/games"),
        config_home.join("lutris/games"),
    ] {
        for file in files(&dir) {
            if let Ok(text) = fs::read_to_string(&file) {
                candidates.extend(text.lines().filter_map(|line| {
                    let value = line.trim().strip_prefix("prefix:")?.trim();
                    let value = value.trim_matches(['"', '\'']);
                    (!value.is_empty()).then(|| expand_home(value, home))
                }));
            }
        }
    }
    // Heroic: "winePrefix" in GamesConfig/*.json (native and Flatpak).
    for dir in [
        config_home.join("heroic"),
        home.join(".var/app/com.heroicgameslauncher.hgl/config/heroic"),
    ] {
        let mut jsons = files(&dir.join("GamesConfig"));
        jsons.push(dir.join("config.json"));
        for file in jsons {
            if let Ok(text) = fs::read_to_string(&file) {
                candidates.extend(json_strings(&text, "winePrefix").map(|p| expand_home(&p, home)));
            }
        }
    }
    // Bottles (native and Flatpak).
    candidates.extend(subdirs(&data_home.join("bottles/bottles")));
    candidates.extend(subdirs(
        &home.join(".var/app/com.usebottles.bottles/data/bottles/bottles"),
    ));
    // Steam's Proton prefixes (non-Steam shortcuts such as Battle.net live here too).
    for library in steam_libraries(home, data_home) {
        candidates.extend(subdirs(&library.join("steamapps/compatdata")));
    }

    let mut prefixes: Vec<PathBuf> = Vec::new();
    for candidate in candidates {
        let root = wine_prefix_root(&candidate.to_string_lossy());
        let root = root.canonicalize().unwrap_or(root);
        if drive_c(&root).is_some() && !prefixes.contains(&root) {
            prefixes.push(root);
        }
    }
    prefixes
}

/// `drive_c` of a prefix: directly inside it (Wine) or in `pfx/` (Proton).
fn drive_c(prefix: &Path) -> Option<PathBuf> {
    [prefix.join("drive_c"), prefix.join("pfx/drive_c")]
        .into_iter()
        .find(|d| d.is_dir())
}

/// Lower-case names of `.exe` files up to `depth` levels below `dir` (not following links).
fn collect_exe_names(dir: &Path, depth: usize, out: &mut Vec<String>) {
    if depth == 0 {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let name = entry.file_name().to_string_lossy().to_lowercase();
        if kind.is_dir() {
            collect_exe_names(&entry.path(), depth - 1, out);
        } else if kind.is_file() && name.ends_with(".exe") {
            out.push(name);
        }
    }
}

fn subdirs(dir: &Path) -> Vec<PathBuf> {
    fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
                .map(|e| e.path())
                .collect()
        })
        .unwrap_or_default()
}

fn files(dir: &Path) -> Vec<PathBuf> {
    fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
                .map(|e| e.path())
                .collect()
        })
        .unwrap_or_default()
}

fn expand_home(path: &str, home: &Path) -> PathBuf {
    match path.strip_prefix("~/") {
        Some(rel) => home.join(rel),
        None => PathBuf::from(path),
    }
}

/// Every string value of `key` anywhere in a JSON document.
fn json_strings(text: &str, key: &str) -> impl Iterator<Item = String> {
    fn walk(value: &serde_json::Value, key: &str, out: &mut Vec<String>) {
        match value {
            serde_json::Value::Object(map) => {
                for (k, v) in map {
                    match v {
                        serde_json::Value::String(s) if k == key && !s.is_empty() => {
                            out.push(s.clone());
                        }
                        _ => walk(v, key, out),
                    }
                }
            }
            serde_json::Value::Array(items) => items.iter().for_each(|v| walk(v, key, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(text) {
        walk(&value, key, &mut out);
    }
    out.into_iter()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        fs::write(path, b"MZ")
    }

    #[test]
    fn finds_steam_and_wine_prefix_games() -> Result<(), Box<dyn std::error::Error>> {
        let home = tempfile::tempdir()?;
        let home = home.path();
        let data = home.join(".local/share");
        let config = home.join(".config");

        // Steam: a catalog game (by id) and one that isn't in the catalog.
        let apps = data.join("Steam/steamapps");
        fs::create_dir_all(&apps)?;
        let manifest = |id: u32, name: &str| {
            format!("\"AppState\"\n{{\n\t\"appid\"\t\t\"{id}\"\n\t\"name\"\t\t\"{name}\"\n}}\n")
        };
        fs::write(
            apps.join("appmanifest_1245620.acf"),
            manifest(1245620, "ELDEN RING"),
        )?;
        fs::write(
            apps.join("appmanifest_427520.acf"),
            manifest(427520, "Factorio"),
        )?;

        // Faugus Battle.net prefix (Proton layout) with WoW and StarCraft II.
        let faugus = home.join("Faugus/battlenet");
        fs::create_dir_all(data.join("faugus-launcher"))?;
        fs::write(
            data.join("faugus-launcher/games.json"),
            format!(
                r#"[{{"title": "Battle.net", "path": "{}/drive_c/Program Files (x86)/Battle.net/Battle.net.exe", "prefix": "{}"}}]"#,
                faugus.display(),
                faugus.display()
            ),
        )?;
        let x86 = faugus.join("drive_c/Program Files (x86)");
        touch(&x86.join("Battle.net/Battle.net.exe"))?;
        touch(&x86.join("World of Warcraft/_retail_/Wow.exe"))?;
        touch(&x86.join("StarCraft II/Versions/Base98745/SC2_x64.exe"))?;
        // Too deep to be scanned.
        touch(&x86.join("a/b/c/d/e/Warcraft III.exe"))?;

        // Lutris prefix (~ path) with Heroes of the Storm, Proton-style pfx/.
        fs::create_dir_all(data.join("lutris/games"))?;
        fs::write(
            data.join("lutris/games/heroes-123.yml"),
            "game:\n  exe: whatever\n  prefix: ~/Games/heroes\nwine:\n  version: x\n",
        )?;
        touch(&home.join(
            "Games/heroes/pfx/drive_c/Program Files (x86)/Heroes of the Storm/Versions/Base1/HeroesOfTheStorm_x64.exe",
        ))?;

        let launchers = Launchers::load(home, &data, &config);
        let found = find_installed(&launchers, home, &data, &config);
        let summary: Vec<(String, String)> = found
            .iter()
            .map(|f| (f.game.name.clone(), f.source.clone()))
            .collect();
        assert_eq!(
            summary,
            [
                ("ELDEN RING".to_owned(), "Steam".to_owned()),
                ("Factorio".to_owned(), "Steam".to_owned()),
                (
                    "Heroes of the Storm".to_owned(),
                    "~/Games/heroes".to_owned()
                ),
                ("StarCraft II".to_owned(), "~/Faugus/battlenet".to_owned()),
                (
                    "World of Warcraft".to_owned(),
                    "~/Faugus/battlenet".to_owned()
                ),
            ]
        );
        // The catalog entry, with its executables, for a catalog game on Steam.
        assert_eq!(found[0].game.executables, ["eldenring.exe"]);
        assert_eq!(found[1].game.steam_app_ids, [427520]);
        Ok(())
    }

    #[test]
    fn nothing_installed_finds_nothing() -> Result<(), Box<dyn std::error::Error>> {
        let home = tempfile::tempdir()?;
        let data = home.path().join(".local/share");
        let config = home.path().join(".config");
        let launchers = Launchers::load(home.path(), &data, &config);
        assert!(find_installed(&launchers, home.path(), &data, &config).is_empty());
        Ok(())
    }
}
