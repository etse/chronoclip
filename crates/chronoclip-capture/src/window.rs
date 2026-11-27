//! Finding the window to capture.

use chronoclip_core::games::{Game, WindowFacts, find_game};
use x11_dl::xlib;

use crate::error::CaptureError;
use crate::xlib::XConnection;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowInfo {
    pub id: u64,
    pub title: String,
    pub pid: Option<u32>,
    pub width: u32,
    pub height: u32,
    /// False when minimized or on another workspace; such windows can't be captured.
    pub viewable: bool,
    /// The game the window belongs to.
    pub game: Game,
}

/// Find the first top-level window that belongs to one of `games` (in the window manager's
/// client list order; for each window, the first matching game).
pub fn find_window(games: &[Game]) -> Result<Option<WindowInfo>, CaptureError> {
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
        let facts = WindowFacts {
            title,
            class: conn
                .class_hint(window)
                .map(|(name, class)| vec![name, class])
                .unwrap_or_default(),
            process_names: pid.map(process_names).unwrap_or_default(),
        };
        let Some(game) = find_game(games, &facts) else {
            continue;
        };
        let Ok(attrs) = conn.window_attributes(window) else {
            continue; // vanished meanwhile
        };
        return Ok(Some(WindowInfo {
            id: window,
            title: facts.title,
            pid,
            width: attrs.width.max(1) as u32,
            height: attrs.height.max(1) as u32,
            viewable: attrs.map_state == xlib::IsViewable,
            game: game.clone(),
        }));
    }
    Ok(None)
}

/// The names a process goes by: `/proc/<pid>/comm` (truncated to 15 bytes by the kernel) and
/// its command-line arguments. Wine/Proton set both to the Windows executable. Only kernel
/// metadata about the process is read, never its memory.
pub(crate) fn process_names(pid: u32) -> Vec<String> {
    let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).unwrap_or_default();
    let cmdline = std::fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
    std::iter::once(comm.trim().to_owned())
        .chain(
            cmdline
                .split(|&b| b == 0)
                .map(|arg| String::from_utf8_lossy(arg).into_owned()),
        )
        .filter(|n| !n.is_empty())
        .collect()
}

/// True if process `pid` runs one of `game`'s executables.
pub(crate) fn process_is(pid: u32, game: &Game) -> bool {
    process_names(pid).iter().any(|n| game.has_executable(n))
}
