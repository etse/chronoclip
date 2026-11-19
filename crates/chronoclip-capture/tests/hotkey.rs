//! Grabs a global hotkey on the real X server and presses it with XTest.
//! Needs an X11 session: `cargo test -p chronoclip-capture -- --ignored`.

#![allow(unsafe_code)] // XTest calls to simulate key presses.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use chronoclip_capture::{HotkeyError, X11Hotkey};
use chronoclip_core::Hotkey;
use x11_dl::{keysym, xlib, xtest};

/// Unusual enough not to collide with desktop shortcuts. Ctrl+Alt+F-keys are avoided: the X
/// server may switch virtual terminals on those.
const HOTKEY: &str = "Ctrl+Shift+Super+F10";

struct Faker {
    xlib: xlib::Xlib,
    xtest: xtest::Xf86vmode,
    display: *mut xlib::Display,
}

impl Faker {
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let xlib = xlib::Xlib::open()?;
        let xtest = xtest::Xf86vmode::open()?;
        // SAFETY: NULL opens $DISPLAY; checked below.
        let display = unsafe { (xlib.XOpenDisplay)(std::ptr::null()) };
        if display.is_null() {
            return Err("no X display".into());
        }
        Ok(Self {
            xlib,
            xtest,
            display,
        })
    }

    fn key(&self, sym: u32, down: bool) {
        // SAFETY: open display; keycode from the server's own mapping.
        unsafe {
            let code = (self.xlib.XKeysymToKeycode)(self.display, sym.into());
            (self.xtest.XTestFakeKeyEvent)(self.display, code.into(), down.into(), 0);
            (self.xlib.XSync)(self.display, xlib::False);
        }
    }

    /// Press and release `HOTKEY`, optionally with Num Lock toggled on.
    fn press_hotkey(&self, num_lock: bool) {
        let mods = [keysym::XK_Control_L, keysym::XK_Shift_L, keysym::XK_Super_L];
        if num_lock {
            self.key(keysym::XK_Num_Lock, true);
            self.key(keysym::XK_Num_Lock, false);
        }
        for m in mods {
            self.key(m, true);
        }
        self.key(keysym::XK_F10, true);
        self.key(keysym::XK_F10, false);
        for m in mods.iter().rev() {
            self.key(*m, false);
        }
        if num_lock {
            self.key(keysym::XK_Num_Lock, true);
            self.key(keysym::XK_Num_Lock, false);
        }
    }
}

impl Drop for Faker {
    fn drop(&mut self) {
        // SAFETY: opened in `new`, closed once.
        unsafe { (self.xlib.XCloseDisplay)(self.display) };
    }
}

fn wait_for(count: &AtomicUsize, n: usize) -> bool {
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if count.load(Ordering::SeqCst) >= n {
            return true;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

#[test]
#[ignore = "needs an X11 session"]
fn grabs_and_receives_global_hotkey() -> Result<(), Box<dyn std::error::Error>> {
    let hotkey: Hotkey = HOTKEY.parse()?;
    let presses = Arc::new(AtomicUsize::new(0));
    let listener = X11Hotkey::start({
        let presses = Arc::clone(&presses);
        move || {
            presses.fetch_add(1, Ordering::SeqCst);
        }
    })?;
    listener.set(Some(hotkey))?;

    // A second client can't take the same key, and says so.
    let other = X11Hotkey::start(|| {})?;
    assert!(matches!(
        other.set(Some(hotkey)),
        Err(HotkeyError::Taken(_))
    ));
    drop(other);

    let faker = Faker::new()?;
    faker.press_hotkey(false);
    assert!(wait_for(&presses, 1), "hotkey press not received");

    // Num Lock on must not matter. Wait out the auto-repeat debounce first.
    std::thread::sleep(Duration::from_millis(800));
    faker.press_hotkey(true);
    assert!(wait_for(&presses, 2), "hotkey with Num Lock not received");

    // A quick second press (auto-repeat) is ignored.
    faker.press_hotkey(false);
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(presses.load(Ordering::SeqCst), 2);

    // After switching to another key the old one is released and free for others.
    listener.set(Some("Ctrl+Shift+Super+F11".parse()?))?;
    let other = X11Hotkey::start(|| {})?;
    other.set(Some(hotkey))?;
    Ok(())
}
