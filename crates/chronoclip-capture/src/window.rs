//! Finding the window to capture.

use x11_dl::xlib;

use crate::error::CaptureError;
use crate::xlib::XConnection;

/// Executable names of World of Warcraft clients (Retail, Classic, PTR, Beta), lower case.
const WOW_EXECUTABLES: &[&str] = &[
    "wow.exe",
    "wowclassic.exe",
    "wowt.exe",
    "wowb.exe",
    "wowclassict.exe",
    "wowclassicb.exe",
];

/// True if `name` (a process name, path, or WM_CLASS) refers to a WoW client executable.
pub fn is_wow_executable(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    let file = lower.rsplit(['/', '\\']).next().unwrap_or(&lower);
    WOW_EXECUTABLES.contains(&file)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WindowMatcher {
    /// The World of Warcraft client window (running under Wine/Proton).
    WorldOfWarcraft,
    /// Any window whose title contains this text (case-insensitive). For testing.
    TitleContains(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowInfo {
    pub id: u64,
    pub title: String,
    pub pid: Option<u32>,
    pub width: u32,
    pub height: u32,
    /// False when minimized or on another workspace; such windows can't be captured.
    pub viewable: bool,
}

/// Find the first top-level window that matches.
pub fn find_window(matcher: &WindowMatcher) -> Result<Option<WindowInfo>, CaptureError> {
    let conn = XConnection::open()?;
    let root = conn.root();
    for window in conn.property_longs(root, "_NET_CLIENT_LIST", xlib::XA_WINDOW) {
        let title = conn
            .property_string(window, "_NET_WM_NAME")
            .or_else(|| conn.property_string(window, "WM_NAME"))
            .unwrap_or_default();
        let pid = conn
            .property_longs(window, "_NET_WM_PID", xlib::XA_CARDINAL)
            .first()
            .and_then(|&p| u32::try_from(p).ok());
        let matches = match matcher {
            WindowMatcher::TitleContains(text) => {
                title.to_lowercase().contains(&text.to_lowercase())
            }
            WindowMatcher::WorldOfWarcraft => {
                conn.class_hint(window).is_some_and(|(name, class)| {
                    is_wow_executable(&name) || is_wow_executable(&class)
                }) || pid.is_some_and(process_is_wow)
            }
        };
        if matches {
            let Ok(attrs) = conn.window_attributes(window) else {
                continue; // vanished meanwhile
            };
            return Ok(Some(WindowInfo {
                id: window,
                title,
                pid,
                width: attrs.width.max(1) as u32,
                height: attrs.height.max(1) as u32,
                viewable: attrs.map_state == xlib::IsViewable,
            }));
        }
    }
    Ok(None)
}

/// Wine/Proton set the process name to the Windows executable (`/proc/<pid>/comm` is
/// truncated to 15 bytes, so also check the command line).
fn process_is_wow(pid: u32) -> bool {
    let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).unwrap_or_default();
    if is_wow_executable(comm.trim()) {
        return true;
    }
    std::fs::read(format!("/proc/{pid}/cmdline"))
        .unwrap_or_default()
        .split(|&b| b == 0)
        .any(|arg| is_wow_executable(&String::from_utf8_lossy(arg)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_wow_executables() {
        assert!(is_wow_executable("Wow.exe"));
        assert!(is_wow_executable("WowClassic.exe"));
        assert!(is_wow_executable("wow.exe"));
        assert!(is_wow_executable(
            r"C:\Program Files (x86)\World of Warcraft\_retail_\Wow.exe"
        ));
        assert!(is_wow_executable(
            "/home/me/Games/wow/drive_c/World of Warcraft/_retail_/Wow.exe"
        ));
        assert!(!is_wow_executable("Battle.net.exe"));
        assert!(!is_wow_executable("WowVoiceProxy.exe"));
        assert!(!is_wow_executable("glxgears"));
        assert!(!is_wow_executable(""));
    }
}
