//! "Show in the app menu" for the AppImage: a menu entry in `~/.local/share/applications` plus
//! the app icon in `~/.local/share/icons/hicolor`.
//!
//! Desktops match a window to its menu entry (`StartupWMClass=chronoclip`) to show its icon;
//! without one, Cinnamon's window list shows a generic cog (it ignores the window's own icon).
//! The .deb installs an entry; an AppImage has none unless a tool like AppImageLauncher adds
//! one. Only files carrying our marker are ever rewritten or removed.

use std::io;
use std::path::{Path, PathBuf};

use crate::APP_ID;
use crate::autostart::exec_argument;

/// Marks the entry written by Chronoclip.
const MARKER: &str = "X-Chronoclip-MenuEntry=true";

/// `<data dir>/applications/io.github.etse.chronoclip.desktop`
pub fn entry_file(data_dir: &Path) -> PathBuf {
    data_dir
        .join("applications")
        .join(format!("{APP_ID}.desktop"))
}

/// `<data home>/icons/hicolor/<size>x<size>/apps/io.github.etse.chronoclip.png`
pub fn icon_file(data_home: &Path, size: u32) -> PathBuf {
    data_home
        .join("icons/hicolor")
        .join(format!("{size}x{size}/apps/{APP_ID}.png"))
}

/// The menu entry that starts `exe` (the `.AppImage` file).
pub fn desktop_entry(exe: &Path) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Chronoclip\n\
         GenericName=Game Replay Recorder\n\
         Comment=Keep the last minutes of your game and save them with a hotkey\n\
         Exec={}\n\
         Icon={APP_ID}\n\
         Terminal=false\n\
         Categories=AudioVideo;Video;Recorder;\n\
         Keywords=replay;record;clip;game;gameplay;wow;world of warcraft;shadowplay;instant replay;\n\
         StartupWMClass=chronoclip\n\
         SingleMainWindow=true\n\
         {MARKER}\n",
        exec_argument(&exe.to_string_lossy())
    )
}

fn is_ours(file: &Path) -> bool {
    std::fs::read_to_string(file).is_ok_and(|text| text.lines().any(|l| l.trim() == MARKER))
}

/// True if a menu entry not written by us exists: installed with the .deb (a system data
/// dir) or by someone else in the user's folder. Then Chronoclip doesn't offer its own.
pub fn installed_elsewhere(data_dirs: &[PathBuf]) -> bool {
    data_dirs.iter().any(|dir| {
        let file = entry_file(dir);
        file.is_file() && !is_ours(&file)
    })
}

/// True while our entry exists in `data_home`.
pub fn is_enabled(data_home: &Path) -> bool {
    is_ours(&entry_file(data_home))
}

/// Write our entry for `exe` and the icons (`(size, png)` pairs). Doesn't overwrite an entry
/// that isn't ours.
pub fn enable(data_home: &Path, exe: &Path, icons: &[(u32, &[u8])]) -> io::Result<()> {
    let file = entry_file(data_home);
    if file.exists() && !is_ours(&file) {
        return Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("{} exists and wasn't written by Chronoclip", file.display()),
        ));
    }
    for &(size, png) in icons {
        let icon = icon_file(data_home, size);
        if let Some(dir) = icon.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(icon, png)?;
    }
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&file, desktop_entry(exe))
}

/// Remove our entry and the icons in `sizes`. An entry that isn't ours is left alone (and so
/// are the icons).
pub fn disable(data_home: &Path, sizes: &[u32]) -> io::Result<()> {
    let file = entry_file(data_home);
    if !is_ours(&file) {
        return Ok(());
    }
    std::fs::remove_file(&file)?;
    for &size in sizes {
        match std::fs::remove_file(icon_file(data_home, size)) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
    }
    Ok(())
}

/// If our entry starts a different file (the AppImage was moved or replaced by a newer one
/// elsewhere), point it at `exe`. Returns true if the file was rewritten.
pub fn refresh(data_home: &Path, exe: &Path) -> io::Result<bool> {
    let file = entry_file(data_home);
    if !is_ours(&file) {
        return Ok(false);
    }
    let wanted = desktop_entry(exe);
    if std::fs::read_to_string(&file)? == wanted {
        return Ok(false);
    }
    std::fs::write(&file, wanted)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_matches_the_window_and_starts_the_appimage() {
        let entry = desktop_entry(Path::new("/home/me/Apps/Chronoclip 0.3.0.AppImage"));
        assert!(entry.starts_with("[Desktop Entry]\n"));
        assert!(entry.contains("\nExec=\"/home/me/Apps/Chronoclip 0.3.0.AppImage\"\n"));
        assert!(entry.contains("\nStartupWMClass=chronoclip\n"));
        assert!(entry.contains(&format!("\nIcon={APP_ID}\n")));
        assert!(entry.contains(MARKER));
    }

    #[test]
    fn enable_refresh_disable() -> Result<(), Box<dyn std::error::Error>> {
        let home = tempfile::tempdir()?;
        let data = home.path().join(".local/share");
        let png: &[u8] = b"\x89PNG";
        let a = Path::new("/apps/Chronoclip-0.2.0.AppImage");
        let b = Path::new("/apps/Chronoclip-0.3.0.AppImage");

        assert!(!is_enabled(&data));
        enable(&data, a, &[(64, png), (256, png)])?;
        assert!(is_enabled(&data));
        assert!(icon_file(&data, 256).is_file());
        assert!(!refresh(&data, a)?);
        assert!(refresh(&data, b)?);
        assert!(std::fs::read_to_string(entry_file(&data))?.contains("0.3.0"));
        // Our own entry isn't "installed elsewhere".
        assert!(!installed_elsewhere(std::slice::from_ref(&data)));

        disable(&data, &[64, 256])?;
        assert!(!is_enabled(&data));
        assert!(!entry_file(&data).exists());
        assert!(!icon_file(&data, 64).exists());
        Ok(())
    }

    #[test]
    fn other_entries_are_left_alone() -> Result<(), Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        let data = root.path().join("home/.local/share");
        let system = root.path().join("usr/share");
        // Installed by the .deb.
        std::fs::create_dir_all(system.join("applications"))?;
        std::fs::write(entry_file(&system), "[Desktop Entry]\nName=Chronoclip\n")?;
        assert!(installed_elsewhere(&[data.clone(), system]));

        // Someone else's file in the user's folder: not overwritten, not removed.
        std::fs::create_dir_all(data.join("applications"))?;
        std::fs::write(entry_file(&data), "[Desktop Entry]\nName=Mine\n")?;
        assert!(enable(&data, Path::new("/a.AppImage"), &[]).is_err());
        assert!(!refresh(&data, Path::new("/a.AppImage"))?);
        disable(&data, &[64])?;
        assert_eq!(
            std::fs::read_to_string(entry_file(&data))?,
            "[Desktop Entry]\nName=Mine\n"
        );
        Ok(())
    }
}
