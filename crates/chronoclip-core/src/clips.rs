//! Naming of saved clips and the free-space check before a save.

use std::path::{Path, PathBuf};

use chrono::NaiveDateTime;

use crate::disk::format_size;

const PREFIX: &str = "Chronoclip_";
const EXTENSION: &str = "mp4";

/// File name for a clip saved at local time `saved_at`, e.g. `Chronoclip_2026-11-20_21-04-33.mp4`.
/// Sorts chronologically and contains no characters that need quoting in a shell.
pub fn clip_file_name(saved_at: NaiveDateTime) -> String {
    format!(
        "{PREFIX}{}.{EXTENSION}",
        saved_at.format("%Y-%m-%d_%H-%M-%S")
    )
}

/// A path in `dir` for a clip saved at `saved_at` that doesn't exist yet: the plain name, or
/// with `_2`, `_3`, … appended if two clips are saved in the same second.
pub fn unique_clip_path(dir: &Path, saved_at: NaiveDateTime) -> PathBuf {
    let name = clip_file_name(saved_at);
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
            clip_file_name(at(9, 4, 3)),
            "Chronoclip_2026-11-20_09-04-03.mp4"
        );
        assert!(clip_file_name(at(9, 59, 59)) < clip_file_name(at(10, 0, 0)));
    }

    #[test]
    fn unique_path_never_overwrites() -> std::io::Result<()> {
        let dir = tempfile::tempdir()?;
        let t = at(21, 4, 33);
        let first = unique_clip_path(dir.path(), t);
        assert_eq!(first, dir.path().join("Chronoclip_2026-11-20_21-04-33.mp4"));
        std::fs::write(&first, b"x")?;
        let second = unique_clip_path(dir.path(), t);
        assert_eq!(
            second,
            dir.path().join("Chronoclip_2026-11-20_21-04-33_2.mp4")
        );
        // A save in progress also reserves its name.
        std::fs::write(partial_path(&second), b"x")?;
        assert_eq!(
            unique_clip_path(dir.path(), t),
            dir.path().join("Chronoclip_2026-11-20_21-04-33_3.mp4")
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
