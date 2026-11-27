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

/// An open top-level window, for the window picker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenWindow {
    pub id: u64,
    pub pid: Option<u32>,
    pub facts: WindowFacts,
}

/// Windows smaller than this (either side) are no games, e.g. hidden helper windows.
const MIN_GAME_SIZE: i32 = 64;

/// Top-level application windows in the window manager's client list, without desktop
/// backgrounds, panels, tiny and untitled windows.
pub fn list_windows() -> Result<Vec<OpenWindow>, CaptureError> {
    let conn = XConnection::open()?;
    let skip_types = [
        conn.atom("_NET_WM_WINDOW_TYPE_DESKTOP"),
        conn.atom("_NET_WM_WINDOW_TYPE_DOCK"),
    ];
    Ok(client_windows(&conn)
        .filter(|w| !w.facts.title.trim().is_empty())
        .filter(|w| {
            !conn
                .property_longs(w.id, "_NET_WM_WINDOW_TYPE", xlib::XA_ATOM)
                .iter()
                .any(|t| skip_types.contains(t))
        })
        .filter(|w| {
            conn.window_attributes(w.id)
                .is_ok_and(|a| a.width >= MIN_GAME_SIZE && a.height >= MIN_GAME_SIZE)
        })
        .collect())
}

/// Find the window to record: `prefer` if it's still open and belongs to one of `games`,
/// else the first top-level window that does (in the window manager's client list order; for
/// each window, the first matching game).
pub fn find_window(
    games: &[Game],
    prefer: Option<u64>,
) -> Result<Option<WindowInfo>, CaptureError> {
    let conn = XConnection::open()?;
    let mut first = None;
    for window in client_windows(&conn) {
        let Some(game) = find_game(games, &window.facts) else {
            continue;
        };
        let preferred = prefer == Some(window.id);
        if first.is_none() || preferred {
            first = Some((window, game.clone()));
        }
        if preferred {
            break;
        }
    }
    let Some((window, game)) = first else {
        return Ok(None);
    };
    let Ok(attrs) = conn.window_attributes(window.id) else {
        return Ok(None); // vanished meanwhile; found again next time
    };
    Ok(Some(WindowInfo {
        id: window.id,
        title: window.facts.title,
        pid: window.pid,
        width: attrs.width.max(1) as u32,
        height: attrs.height.max(1) as u32,
        viewable: attrs.map_state == xlib::IsViewable,
        game,
    }))
}

/// The window manager's top-level windows with title, WM_CLASS and process names.
fn client_windows(conn: &XConnection) -> impl Iterator<Item = OpenWindow> + '_ {
    conn.property_longs(conn.root(), "_NET_CLIENT_LIST", xlib::XA_WINDOW)
        .into_iter()
        .map(|window| {
            let title = conn
                .property_string(window, "_NET_WM_NAME")
                .or_else(|| conn.property_string(window, "WM_NAME"))
                .unwrap_or_default();
            let pid = conn
                .property_longs(window, "_NET_WM_PID", xlib::XA_CARDINAL)
                .first()
                .and_then(|&p| u32::try_from(p).ok());
            OpenWindow {
                id: window,
                pid,
                facts: WindowFacts {
                    title,
                    class: conn
                        .class_hint(window)
                        .map(|(name, class)| vec![name, class])
                        .unwrap_or_default(),
                    process_names: pid.map(process_names).unwrap_or_default(),
                },
            }
        })
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
