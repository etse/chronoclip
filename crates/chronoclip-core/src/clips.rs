//! Naming of saved clips (a folder and file name per game) and the free-space check before a
//! save.

use std::path::{Path, PathBuf};

use chrono::NaiveDateTime;

use crate::disk::format_size;

const FALLBACK: &str = "Chronoclip";
const EXTENSION: &str = "mp4";
/// Longest folder/file name prefix taken from a game name (in characters).
const MAX_NAME_CHARS: usize = 80;

/// `name` as a folder or file name: without `/ \ : * ? " < > |` and control characters (not
/// allowed or awkward on Linux, exFAT and NTFS drives), single spaces, no leading or trailing
/// dots or spaces, at most 80 characters. Empty results become "Chronoclip".
pub fn safe_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => ' ',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    let joined = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let short: String = joined.chars().take(MAX_NAME_CHARS).collect();
    let trimmed = short.trim_matches(|c: char| c == '.' || c.is_whitespace());
    if trimmed.is_empty() {
        FALLBACK.to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// The folder for `game`'s clips inside the clips folder `output_dir`, e.g.
/// `~/Videos/Chronoclip/ELDEN RING`. Without a game: `output_dir` itself.
pub fn game_folder(output_dir: &Path, game: Option<&str>) -> PathBuf {
    match game {
        Some(game) => output_dir.join(safe_file_name(game)),
        None => output_dir.to_owned(),
    }
}

/// File name for a clip of `game` saved at local time `saved_at`, e.g.
/// `ELDEN RING_2026-11-20_21-04-33.mp4` (`Chronoclip_…` without a game). Sorts
/// chronologically within a game's folder.
pub fn clip_file_name(game: Option<&str>, saved_at: NaiveDateTime) -> String {
    format!(
        "{}_{}.{EXTENSION}",
        safe_file_name(game.unwrap_or(FALLBACK)),
        saved_at.format("%Y-%m-%d_%H-%M-%S")
    )
}

/// A path in `dir` for a clip of `game` saved at `saved_at` that doesn't exist yet: the plain
/// name, or with `_2`, `_3`, … appended if two clips are saved in the same second.
pub fn unique_clip_path(dir: &Path, game: Option<&str>, saved_at: NaiveDateTime) -> PathBuf {
    let name = clip_file_name(game, saved_at);
    let stem = name.trim_end_matches(&format!(".{EXTENSION}"));
    let taken = |path: &Path| path.exists() || partial_path(path).exists();
    let mut path = dir.join(&name);
    let mut n = 2;
    while taken(&path) {
        path = dir.join(format!("{stem}_{n}.{EXTENSION}"));
        n += 1;
    }
    path
}

/// Where a clip is written while it is being saved: a hidden file next to `clip`, renamed to
/// `clip` once complete, so the output folder never shows half-written clips.
pub fn partial_path(clip: &Path) -> PathBuf {
    let name = clip
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    clip.with_file_name(format!(".{name}.part"))
}

/// Space kept free on top of the clip size, so a save never fills the disk completely.
const MIN_HEADROOM: u64 = 256 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error(
    "not enough free space in the clips folder: the clip needs about {} but only {} is free",
    format_size(*.needed),
    format_size(*.available)
)]
pub struct NotEnoughSpace {
    pub needed: u64,
    pub available: u64,
}

/// Check that a clip of about `clip_bytes` fits into `available_bytes`, with some headroom.
/// A remuxed clip is about as large as the segments it is made from.
pub fn check_free_space(clip_bytes: u64, available_bytes: u64) -> Result<(), NotEnoughSpace> {
    let needed = clip_bytes.saturating_add((clip_bytes / 20).max(MIN_HEADROOM));
    if available_bytes >= needed {
        Ok(())
    } else {
        Err(NotEnoughSpace {
            needed,
            available: available_bytes,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn at(h: u32, m: u32, s: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 11, 20)
            .and_then(|d| d.and_hms_opt(h, m, s))
            .unwrap_or_default()
    }

    #[test]
    fn names_sort_by_time() {
        assert_eq!(
            clip_file_name(None, at(9, 4, 3)),
            "Chronoclip_2026-11-20_09-04-03.mp4"
        );
        assert_eq!(
            clip_file_name(Some("ELDEN RING"), at(21, 4, 33)),
            "ELDEN RING_2026-11-20_21-04-33.mp4"
        );
        assert!(clip_file_name(Some("x"), at(9, 59, 59)) < clip_file_name(Some("x"), at(10, 0, 0)));
    }

    #[test]
    fn game_names_become_safe_file_names() {
        assert_eq!(safe_file_name("World of Warcraft"), "World of Warcraft");
        assert_eq!(
            safe_file_name("Hollow Knight: Silksong"),
            "Hollow Knight Silksong"
        );
        assert_eq!(
            safe_file_name("Marvel’s Spider-Man: Miles Morales"),
            "Marvel’s Spider-Man Miles Morales"
        );
        assert_eq!(safe_file_name("R.E.P.O."), "R.E.P.O");
        assert_eq!(safe_file_name("AC/DC <Live>?"), "AC DC Live");
        assert_eq!(safe_file_name("../../etc"), "etc");
        assert_eq!(safe_file_name(" \t\n"), "Chronoclip");
        assert_eq!(safe_file_name("..."), "Chronoclip");
        assert_eq!(safe_file_name(&"x".repeat(200)).chars().count(), 80);
        assert_eq!(
            game_folder(Path::new("/v"), Some("Hollow Knight: Silksong")),
            PathBuf::from("/v/Hollow Knight Silksong")
        );
        assert_eq!(game_folder(Path::new("/v"), None), PathBuf::from("/v"));
    }

    #[test]
    fn unique_path_never_overwrites() -> std::io::Result<()> {
        let dir = tempfile::tempdir()?;
        let t = at(21, 4, 33);
        let game = Some("Factorio");
        let first = unique_clip_path(dir.path(), game, t);
        assert_eq!(first, dir.path().join("Factorio_2026-11-20_21-04-33.mp4"));
        std::fs::write(&first, b"x")?;
        let second = unique_clip_path(dir.path(), game, t);
        assert_eq!(
            second,
            dir.path().join("Factorio_2026-11-20_21-04-33_2.mp4")
        );
        // A save in progress also reserves its name.
        std::fs::write(partial_path(&second), b"x")?;
        assert_eq!(
            unique_clip_path(dir.path(), game, t),
            dir.path().join("Factorio_2026-11-20_21-04-33_3.mp4")
        );
        Ok(())
    }

    #[test]
    fn partial_file_is_hidden_next_to_clip() {
        assert_eq!(
            partial_path(Path::new("/v/Chronoclip_x.mp4")),
            PathBuf::from("/v/.Chronoclip_x.mp4.part")
        );
    }

    #[test]
    fn free_space_needs_headroom() {
        const GB: u64 = 1_000_000_000;
        assert!(check_free_space(GB, 10 * GB).is_ok());
        // Small clips still need the minimum headroom.
        assert!(check_free_space(1_000_000, MIN_HEADROOM).is_err());
        assert!(check_free_space(1_000_000, MIN_HEADROOM + 1_000_000).is_ok());
        // Large clips need 5 % extra.
        let err = check_free_space(10 * GB, 10 * GB + GB / 4);
        assert_eq!(
            err,
            Err(NotEnoughSpace {
                needed: 10 * GB + GB / 2,
                available: 10 * GB + GB / 4
            })
        );
        assert!(err.is_err_and(|e| e.to_string().contains("10.5 GB")));
    }
}
