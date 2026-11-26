//! "Start with the desktop": an XDG autostart entry (`~/.config/autostart/chronoclip.desktop`)
//! that starts Chronoclip in the tray when the user logs in.

use std::io;
use std::path::Path;

use crate::APP_ID;

/// Marks entries written by Chronoclip, so we only ever rewrite our own file.
const MARKER: &str = "X-Chronoclip-Autostart=true";

/// The autostart entry for the executable at `exe`.
pub fn desktop_entry(exe: &Path) -> String {
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Chronoclip\n\
         Comment=Replay buffer for World of Warcraft\n\
         Exec={} --hidden\n\
         Icon={APP_ID}\n\
         Terminal=false\n\
         X-GNOME-Autostart-enabled=true\n\
         {MARKER}\n",
        exec_argument(&exe.to_string_lossy())
    )
}

/// Quote one argument for an `Exec=` key (Desktop Entry spec): wrap it in double quotes,
/// backslash-escape `"` `` ` `` `$` `\`, then apply the string-value escaping (`\` → `\\`)
/// and double `%` (field codes).
fn exec_argument(arg: &str) -> String {
    let mut quoted = String::from("\"");
    for c in arg.chars() {
        if matches!(c, '"' | '`' | '$' | '\\') {
            quoted.push('\\');
        }
        quoted.push(c);
    }
    quoted.push('"');
    quoted.replace('\\', "\\\\").replace('%', "%%")
}

/// True while the autostart entry exists.
pub fn is_enabled(file: &Path) -> bool {
    file.is_file()
}

/// Write the entry (creating `~/.config/autostart` if needed).
pub fn enable(file: &Path, exe: &Path) -> io::Result<()> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(file, desktop_entry(exe))
}

/// Remove the entry if it exists.
pub fn disable(file: &Path) -> io::Result<()> {
    match std::fs::remove_file(file) {
        Err(e) if e.kind() != io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

/// If our entry exists but starts a different executable (the app was moved or rebuilt
/// elsewhere), point it at `exe`. Entries not written by Chronoclip are left alone.
/// Returns true if the file was rewritten.
pub fn refresh(file: &Path, exe: &Path) -> io::Result<bool> {
    let current = match std::fs::read_to_string(file) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(e) => return Err(e),
    };
    let wanted = desktop_entry(exe);
    if !current.contains(MARKER) || current == wanted {
        return Ok(false);
    }
    std::fs::write(file, wanted)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entry_starts_hidden_in_the_tray() {
        let entry = desktop_entry(Path::new("/usr/bin/chronoclip"));
        assert!(entry.starts_with("[Desktop Entry]\n"));
        assert!(entry.contains("\nExec=\"/usr/bin/chronoclip\" --hidden\n"));
        assert!(entry.contains("\nType=Application\n"));
        assert!(entry.contains(MARKER));
    }

    #[test]
    fn exec_paths_are_quoted_and_escaped() {
        assert_eq!(
            exec_argument("/home/me/My Games/chronoclip"),
            r#""/home/me/My Games/chronoclip""#
        );
        // `$` and `"` get an argument-level backslash, which the value escaping then doubles.
        assert_eq!(exec_argument("/a$b"), r#""/a\\$b""#);
        assert_eq!(exec_argument("/a\"b"), r#""/a\\"b""#);
        assert_eq!(exec_argument(r"/a\b"), r#""/a\\\\b""#);
        assert_eq!(exec_argument("/100%/x"), r#""/100%%/x""#);
    }

    #[test]
    fn enable_disable_refresh() -> io::Result<()> {
        let dir = tempfile::tempdir()?;
        let file = dir.path().join("autostart/chronoclip.desktop");
        assert!(!is_enabled(&file));
        disable(&file)?; // no-op when absent
        assert!(!refresh(&file, Path::new("/x"))?);

        enable(&file, Path::new("/old/chronoclip"))?;
        assert!(is_enabled(&file));
        assert!(
            !refresh(&file, Path::new("/old/chronoclip"))?,
            "unchanged path"
        );
        assert!(refresh(&file, Path::new("/new/chronoclip"))?);
        assert!(std::fs::read_to_string(&file)?.contains("\"/new/chronoclip\""));

        disable(&file)?;
        assert!(!is_enabled(&file));
        Ok(())
    }

    #[test]
    fn foreign_entries_are_not_rewritten() -> io::Result<()> {
        let dir = tempfile::tempdir()?;
        let file = dir.path().join("chronoclip.desktop");
        std::fs::write(&file, "[Desktop Entry]\nExec=my-own-wrapper\n")?;
        assert!(!refresh(&file, Path::new("/usr/bin/chronoclip"))?);
        assert_eq!(
            std::fs::read_to_string(&file)?,
            "[Desktop Entry]\nExec=my-own-wrapper\n"
        );
        Ok(())
    }
}
