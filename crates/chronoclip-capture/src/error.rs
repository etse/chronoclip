#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    #[error("could not load {library}: {source}")]
    Library {
        library: &'static str,
        source: libloading::Error,
    },
    #[error("could not load X11 libraries: {0}")]
    Xlib(String),
    #[error("cannot open X display (is this an X11 session?)")]
    NoDisplay,
    #[error("the X server has no {0} extension")]
    MissingExtension(&'static str),
    #[error("no suitable GLX framebuffer config: {0}")]
    NoFbConfig(&'static str),
    #[error("{0} failed")]
    Glx(&'static str),
    #[error("OpenGL error: {0}")]
    Gl(String),
    #[error("{call} failed with CUDA error {code} ({name})")]
    Cuda {
        call: &'static str,
        code: i32,
        name: String,
    },
    #[error("the window is gone")]
    WindowGone,
    #[error("the window is not visible (minimized or on another workspace)")]
    WindowNotViewable,
    #[error("X error {0} during {1}")]
    X(u8, &'static str),
}
