//! Global hotkey on X11: a passive key grab (`XGrabKey`) on the root window.
//!
//! The X server delivers the key to us whichever window has focus (including the game), and
//! the game never sees it. Nothing here touches the game process. A dedicated thread owns its
//! own X connection and waits for key events.

use std::ffi::{c_int, c_uint, c_ulong};
use std::sync::mpsc::{self, Receiver, Sender, SyncSender};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use chronoclip_core::hotkey::{Hotkey, Key, Modifiers};
use x11_dl::xlib;

use crate::error::CaptureError;
use crate::xlib::XConnection;

/// How long the thread waits for X events before checking for commands.
const POLL_TIMEOUT_MS: c_int = 100;
/// Presses closer together than this count as one (key auto-repeat while held down).
const DEBOUNCE: Duration = Duration::from_millis(750);
/// X error code for a key that another client already grabbed.
const BAD_ACCESS: u8 = 10;

#[derive(Debug, thiserror::Error)]
pub enum HotkeyError {
    #[error(transparent)]
    Connection(#[from] CaptureError),
    #[error("{0} is not on this keyboard layout")]
    KeyNotOnKeyboard(Hotkey),
    #[error("{0} is already used by another program")]
    Taken(Hotkey),
    #[error("the hotkey thread stopped")]
    Stopped,
}

/// Keysym for `key` (the unshifted symbol, as `XKeysymToKeycode` expects).
fn keysym(key: Key) -> c_ulong {
    match key {
        Key::F(n) => c_ulong::from(x11_dl::keysym::XK_F1) + c_ulong::from(n.saturating_sub(1)),
        Key::Letter(c) | Key::Digit(c) => c_ulong::from(c.to_ascii_lowercase()),
    }
}

/// X modifier mask for `modifiers`.
fn modifier_mask(modifiers: Modifiers) -> c_uint {
    [
        (modifiers.ctrl, xlib::ControlMask),
        (modifiers.shift, xlib::ShiftMask),
        (modifiers.alt, xlib::Mod1Mask),
        (modifiers.super_key, xlib::Mod4Mask),
    ]
    .into_iter()
    .filter(|(held, _)| *held)
    .fold(0, |mask, (_, bit)| mask | bit)
}

/// Lock modifiers that must not stop the hotkey from working: Caps Lock and Num Lock (Mod2 on
/// practically every layout). The key is grabbed once for every combination of them.
const IGNORED_MASKS: [c_uint; 4] = [
    0,
    xlib::LockMask,
    xlib::Mod2Mask,
    xlib::LockMask | xlib::Mod2Mask,
];

/// Modifier bits that count when matching a key press: everything except the lock keys and
/// mouse buttons.
const RELEVANT_MASK: c_uint = xlib::ControlMask | xlib::ShiftMask | xlib::Mod1Mask | xlib::Mod4Mask;

enum Command {
    Set(Option<Hotkey>, SyncSender<Result<(), HotkeyError>>),
}

/// Listens for one global hotkey until dropped.
pub struct X11Hotkey {
    commands: Option<Sender<Command>>,
    thread: Option<JoinHandle<()>>,
}

impl X11Hotkey {
    /// Start the listener thread with no hotkey grabbed. `on_press` runs on that thread.
    pub fn start(on_press: impl Fn() + Send + 'static) -> Result<Self, HotkeyError> {
        let (commands, receiver) = mpsc::channel();
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let thread = thread::Builder::new()
            .name("hotkey".into())
            .spawn(move || match XConnection::open() {
                Ok(conn) => {
                    let _ = ready_tx.send(Ok(()));
                    listen(&conn, &receiver, &on_press);
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                }
            })
            .map_err(|_| HotkeyError::Stopped)?;
        ready_rx.recv().map_err(|_| HotkeyError::Stopped)??;
        Ok(Self {
            commands: Some(commands),
            thread: Some(thread),
        })
    }

    /// Grab `hotkey` (releasing the previous one), or release it with `None`. If the new key
    /// can't be grabbed, no hotkey is active afterwards.
    pub fn set(&self, hotkey: Option<Hotkey>) -> Result<(), HotkeyError> {
        let (reply, result) = mpsc::sync_channel(1);
        self.commands
            .as_ref()
            .ok_or(HotkeyError::Stopped)?
            .send(Command::Set(hotkey, reply))
            .map_err(|_| HotkeyError::Stopped)?;
        result.recv().map_err(|_| HotkeyError::Stopped)?
    }
}

impl Drop for X11Hotkey {
    fn drop(&mut self) {
        drop(self.commands.take()); // the thread exits when the channel closes
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            tracing::error!("hotkey thread panicked");
        }
    }
}

/// A grabbed key: keycode and modifier mask.
#[derive(Clone, Copy)]
struct Grab {
    keycode: c_int,
    mask: c_uint,
}

fn listen(conn: &XConnection, commands: &Receiver<Command>, on_press: &dyn Fn()) {
    let root = conn.root();
    let mut grab: Option<Grab> = None;
    let mut last_press: Option<Instant> = None;
    // SAFETY: the display is open for the lifetime of `conn`.
    let fd = unsafe { (conn.xlib.XConnectionNumber)(conn.display) };

    loop {
        loop {
            match commands.try_recv() {
                Ok(Command::Set(hotkey, reply)) => {
                    if let Some(old) = grab.take() {
                        ungrab(conn, root, old);
                    }
                    let result = hotkey.map_or(Ok(()), |hotkey| {
                        grab_key(conn, root, hotkey).map(|g| {
                            grab = Some(g);
                        })
                    });
                    let _ = reply.send(result);
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    if let Some(old) = grab {
                        ungrab(conn, root, old);
                    }
                    return;
                }
            }
        }

        // SAFETY: the display is open; XPending/XNextEvent are called only from this thread,
        // which owns the connection. XEvent is a C union, zeroed is a valid initial value.
        while unsafe { (conn.xlib.XPending)(conn.display) } > 0 {
            let mut event: xlib::XEvent = unsafe { std::mem::zeroed() };
            unsafe { (conn.xlib.XNextEvent)(conn.display, &mut event) };
            if event.get_type() != xlib::KeyPress {
                continue;
            }
            // SAFETY: the event type is KeyPress, so the `key` member is the active one.
            let key = unsafe { event.key };
            let Some(g) = grab else { continue };
            if key.keycode as c_int != g.keycode || key.state & RELEVANT_MASK != g.mask {
                continue;
            }
            let now = Instant::now();
            if last_press.is_some_and(|t| now.duration_since(t) < DEBOUNCE) {
                continue;
            }
            last_press = Some(now);
            tracing::debug!("hotkey pressed");
            on_press();
        }

        let mut pfd = libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: one valid pollfd for the X connection's socket, which stays open.
        let ret = unsafe { libc::poll(&mut pfd, 1, POLL_TIMEOUT_MS) };
        if ret < 0 && std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted {
            tracing::error!(
                "hotkey: waiting for X events failed: {}",
                std::io::Error::last_os_error()
            );
            return;
        }
        if pfd.revents & (libc::POLLHUP | libc::POLLERR) != 0 {
            tracing::error!("hotkey: lost the connection to the X server");
            return;
        }
    }
}

fn grab_key(conn: &XConnection, root: xlib::Window, hotkey: Hotkey) -> Result<Grab, HotkeyError> {
    // SAFETY: the display is open.
    let keycode = unsafe { (conn.xlib.XKeysymToKeycode)(conn.display, keysym(hotkey.key)) };
    if keycode == 0 {
        return Err(HotkeyError::KeyNotOnKeyboard(hotkey));
    }
    let grab = Grab {
        keycode: c_int::from(keycode),
        mask: modifier_mask(hotkey.modifiers),
    };
    let _ = conn.sync_error(); // clear errors from earlier requests
    for extra in IGNORED_MASKS {
        // SAFETY: valid display, keycode and root window. A key taken by another client
        // raises BadAccess asynchronously, which our error handler records.
        unsafe {
            (conn.xlib.XGrabKey)(
                conn.display,
                grab.keycode,
                grab.mask | extra,
                root,
                xlib::False,
                xlib::GrabModeAsync,
                xlib::GrabModeAsync,
            );
        }
    }
    match conn.sync_error() {
        None => {
            tracing::info!(%hotkey, "global hotkey registered");
            Ok(grab)
        }
        Some(code) => {
            ungrab(conn, root, grab);
            if code == BAD_ACCESS {
                Err(HotkeyError::Taken(hotkey))
            } else {
                Err(CaptureError::X(code, "XGrabKey").into())
            }
        }
    }
}

fn ungrab(conn: &XConnection, root: xlib::Window, grab: Grab) {
    for extra in IGNORED_MASKS {
        // SAFETY: valid display and root window; ungrabbing a key we don't hold is a no-op.
        unsafe { (conn.xlib.XUngrabKey)(conn.display, grab.keycode, grab.mask | extra, root) };
    }
    let _ = conn.sync_error();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keysyms() {
        assert_eq!(keysym(Key::F(1)), 0xffbe);
        assert_eq!(keysym(Key::F(9)), 0xffc6);
        assert_eq!(keysym(Key::F(24)), 0xffd5);
        assert_eq!(keysym(Key::Letter('R')), c_ulong::from(b'r'));
        assert_eq!(keysym(Key::Digit('5')), c_ulong::from(b'5'));
    }

    #[test]
    fn masks() {
        let ctrl_shift: Hotkey = "Ctrl+Shift+F9".parse().unwrap_or_default();
        assert_eq!(
            modifier_mask(ctrl_shift.modifiers),
            xlib::ControlMask | xlib::ShiftMask
        );
        let alt_super: Hotkey = "Alt+Super+R".parse().unwrap_or_default();
        assert_eq!(
            modifier_mask(alt_super.modifiers),
            xlib::Mod1Mask | xlib::Mod4Mask
        );
        assert_eq!(modifier_mask(Modifiers::default()), 0);
        // Lock keys never overlap the modifiers we match on.
        for extra in IGNORED_MASKS {
            assert_eq!(extra & RELEVANT_MASK, 0);
        }
    }
}
