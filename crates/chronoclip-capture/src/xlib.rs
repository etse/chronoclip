//! A private Xlib connection with error handling that doesn't kill the process.
//!
//! Xlib's default error handler calls `exit()`. We install one process-wide handler that records
//! errors for our own displays and forwards everything else to the previous handler (e.g. winit's).

use std::ffi::{CString, c_int, c_long, c_uchar, c_ulong};
use std::ptr;
use std::sync::{Mutex, OnceLock};

use x11_dl::xlib::{self, Display, XErrorEvent, Xlib};

use crate::error::CaptureError;

type ErrorHandler = Option<unsafe extern "C" fn(*mut Display, *mut XErrorEvent) -> c_int>;

/// (display address, last error code) for each of our connections.
static OUR_DISPLAYS: Mutex<Vec<(usize, u8)>> = Mutex::new(Vec::new());
static PREVIOUS_HANDLER: OnceLock<ErrorHandler> = OnceLock::new();

unsafe extern "C" fn error_handler(display: *mut Display, event: *mut XErrorEvent) -> c_int {
    {
        let mut ours = OUR_DISPLAYS.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(entry) = ours.iter_mut().find(|(d, _)| *d == display as usize) {
            // SAFETY: Xlib passes a valid event pointer to error handlers.
            entry.1 = unsafe { (*event).error_code };
            return 0;
        }
    }
    match PREVIOUS_HANDLER.get().copied().flatten() {
        // SAFETY: forwarding the same arguments to the handler that was installed before ours.
        Some(previous) => unsafe { previous(display, event) },
        None => 0,
    }
}

pub(crate) struct XConnection {
    pub xlib: Xlib,
    pub display: *mut Display,
}

impl XConnection {
    pub fn open() -> Result<Self, CaptureError> {
        let xlib = Xlib::open().map_err(|e| CaptureError::Xlib(e.to_string()))?;
        PREVIOUS_HANDLER.get_or_init(|| {
            // SAFETY: installing a handler with the signature Xlib expects; we keep the old one.
            unsafe { (xlib.XSetErrorHandler)(Some(error_handler)) }
        });
        // SAFETY: NULL means "use $DISPLAY"; the result is checked.
        let display = unsafe { (xlib.XOpenDisplay)(ptr::null()) };
        if display.is_null() {
            return Err(CaptureError::NoDisplay);
        }
        OUR_DISPLAYS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push((display as usize, 0));
        Ok(Self { xlib, display })
    }

    /// Wait for the server to process all requests, then return (and clear) the last X error.
    pub fn sync_error(&self) -> Option<u8> {
        // SAFETY: display is open.
        unsafe { (self.xlib.XSync)(self.display, xlib::False) };
        let mut ours = OUR_DISPLAYS.lock().unwrap_or_else(|e| e.into_inner());
        let entry = ours.iter_mut().find(|(d, _)| *d == self.display as usize)?;
        let code = std::mem::take(&mut entry.1);
        (code != 0).then_some(code)
    }

    pub fn check(&self, what: &'static str) -> Result<(), CaptureError> {
        match self.sync_error() {
            Some(code) => Err(CaptureError::X(code, what)),
            None => Ok(()),
        }
    }

    pub fn root(&self) -> xlib::Window {
        // SAFETY: display is open.
        unsafe { (self.xlib.XDefaultRootWindow)(self.display) }
    }

    pub fn screen(&self) -> c_int {
        // SAFETY: display is open.
        unsafe { (self.xlib.XDefaultScreen)(self.display) }
    }

    pub fn atom(&self, name: &str) -> xlib::Atom {
        let name = CString::new(name).unwrap_or_default();
        // SAFETY: valid display and NUL-terminated name.
        unsafe { (self.xlib.XInternAtom)(self.display, name.as_ptr(), xlib::False) }
    }

    /// Read a window property as raw items. Format-32 items are returned as `c_long`s.
    fn property(
        &self,
        window: xlib::Window,
        property: xlib::Atom,
        req_type: xlib::Atom,
    ) -> Option<(c_int, Vec<u8>, c_ulong)> {
        let mut actual_type = 0;
        let mut format = 0;
        let mut n_items: c_ulong = 0;
        let mut bytes_after: c_ulong = 0;
        let mut data: *mut c_uchar = ptr::null_mut();
        // SAFETY: all out-pointers are valid; the returned buffer is copied and freed with XFree.
        unsafe {
            let status = (self.xlib.XGetWindowProperty)(
                self.display,
                window,
                property,
                0,
                1 << 16,
                xlib::False,
                req_type,
                &mut actual_type,
                &mut format,
                &mut n_items,
                &mut bytes_after,
                &mut data,
            );
            if status != xlib::Success as c_int || data.is_null() {
                return None;
            }
            let item_size = match format {
                8 => 1,
                16 => std::mem::size_of::<std::ffi::c_short>(),
                32 => std::mem::size_of::<c_long>(),
                _ => 0,
            };
            let bytes = std::slice::from_raw_parts(data, n_items as usize * item_size).to_vec();
            (self.xlib.XFree)(data.cast());
            Some((format, bytes, n_items))
        }
    }

    pub fn property_longs(
        &self,
        window: xlib::Window,
        property: &str,
        req_type: xlib::Atom,
    ) -> Vec<c_ulong> {
        match self.property(window, self.atom(property), req_type) {
            Some((32, bytes, _)) => bytes
                .as_chunks::<{ std::mem::size_of::<c_ulong>() }>()
                .0
                .iter()
                .map(|c| c_ulong::from_ne_bytes(*c))
                .collect(),
            _ => Vec::new(),
        }
    }

    pub fn property_string(&self, window: xlib::Window, property: &str) -> Option<String> {
        let (format, bytes, _) = self.property(
            window,
            self.atom(property),
            xlib::AnyPropertyType as xlib::Atom,
        )?;
        (format == 8).then(|| String::from_utf8_lossy(&bytes).into_owned())
    }

    /// `(res_name, res_class)` from WM_CLASS.
    pub fn class_hint(&self, window: xlib::Window) -> Option<(String, String)> {
        let bytes = self.property_string(window, "WM_CLASS")?;
        let mut parts = bytes.split('\0');
        Some((
            parts.next().unwrap_or_default().to_owned(),
            parts.next().unwrap_or_default().to_owned(),
        ))
    }

    /// Child windows as `(id, depth, width, height, viewable)`, for diagnostics.
    pub fn children(&self, window: xlib::Window) -> Vec<(String, i32, i32, i32, bool)> {
        let (mut root, mut parent) = (0, 0);
        let mut children: *mut xlib::Window = ptr::null_mut();
        let mut count = 0;
        // SAFETY: valid display and out-pointers; the list is freed with XFree.
        let list = unsafe {
            if (self.xlib.XQueryTree)(
                self.display,
                window,
                &mut root,
                &mut parent,
                &mut children,
                &mut count,
            ) == 0
                || children.is_null()
            {
                return Vec::new();
            }
            let list = std::slice::from_raw_parts(children, count as usize).to_vec();
            (self.xlib.XFree)(children.cast());
            list
        };
        list.into_iter()
            .filter_map(|child| {
                let a = self.window_attributes(child).ok()?;
                Some((
                    format!("0x{child:x}"),
                    a.depth,
                    a.width,
                    a.height,
                    a.map_state == xlib::IsViewable,
                ))
            })
            .collect()
    }

    pub fn window_attributes(
        &self,
        window: xlib::Window,
    ) -> Result<xlib::XWindowAttributes, CaptureError> {
        // SAFETY: XWindowAttributes is plain data; zeroed is a valid initial state.
        let mut attrs: xlib::XWindowAttributes = unsafe { std::mem::zeroed() };
        // SAFETY: valid display and out-pointer; errors are caught by our handler.
        let ok = unsafe { (self.xlib.XGetWindowAttributes)(self.display, window, &mut attrs) };
        if self.sync_error().is_some() || ok == 0 {
            return Err(CaptureError::WindowGone);
        }
        Ok(attrs)
    }
}

impl Drop for XConnection {
    fn drop(&mut self) {
        // SAFETY: display was opened by us and is closed exactly once.
        unsafe { (self.xlib.XCloseDisplay)(self.display) };
        OUR_DISPLAYS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retain(|(d, _)| *d != self.display as usize);
    }
}
