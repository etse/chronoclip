//! Audio capture through the PulseAudio client API (`libpulse.so.0`, loaded at runtime).
//!
//! On PipeWire systems the PulseAudio API is served by `pipewire-pulse`, which every desktop
//! distribution ships, so this works on PipeWire and on plain PulseAudio. The API can record
//! a single application's stream (`pa_stream_set_monitor_stream`), which is how "game only"
//! audio leaves voice chat and music out.
//!
//! Each [`AudioCapture`] runs its own thread with a non-threaded PulseAudio main loop. The
//! [`AudioSink`] gets the samples on that thread, plus a regular tick (also while nothing is
//! captured) so it can keep its track continuous.

use std::ffi::{CStr, CString, c_char, c_int, c_void};
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use chronoclip_core::audio::{CHANNELS, SAMPLE_RATE};
use libloading::Library;

use crate::window::{is_wow_executable, process_is_wow};

const LIBRARY: &str = "libpulse.so.0";
/// How often the main loop wakes up without data (and calls [`AudioSink::tick`]).
const TICK: Duration = Duration::from_millis(50);
/// How often the capture target is checked again (game stream appeared, default output changed).
const CHECK_INTERVAL: Duration = Duration::from_secs(2);
/// Wait before reconnecting after the sound server went away or refused us.
const RETRY: Duration = Duration::from_secs(3);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// Requested capture latency (fragment size): small, so arrival times are accurate.
const FRAGMENT: Duration = Duration::from_millis(10);

/// What to record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioSourceKind {
    /// The game's own output stream; everything on the default output while it isn't found.
    Game,
    /// Everything played on the default output.
    AllSound,
    /// The default input (microphone).
    Microphone,
}

/// What an [`AudioCapture`] is recording right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AudioSourceState {
    Connecting,
    /// The game's output stream.
    Game,
    /// Everything on the default output; `game_missing` when the game's stream wasn't found.
    AllSound {
        game_missing: bool,
    },
    Microphone,
    /// No audio (no sound server, or the device failed); the track is filled with silence.
    Unavailable(String),
}

/// Receives captured audio on the capture thread.
pub trait AudioSink: Send + 'static {
    /// Interleaved stereo `f32` samples at [`SAMPLE_RATE`]; `end` is when the last one was
    /// captured (as close as we can tell).
    fn samples(&mut self, samples: &[f32], end: Instant);
    /// Called about every 50 ms, also while nothing is captured.
    fn tick(&mut self, now: Instant);
    /// The source changed (connected, switched to the game stream, failed, …).
    fn state(&mut self, state: AudioSourceState);
}

/// The game window whose sound "game only" records.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GameWindow {
    /// Process that owns the window (`_NET_WM_PID`).
    pub pid: Option<u32>,
    /// Window title.
    pub title: String,
}

/// A running audio capture. Dropping it stops the thread.
pub struct AudioCapture {
    stop: Arc<AtomicBool>,
    game: Arc<Mutex<Option<GameWindow>>>,
    thread: Option<JoinHandle<()>>,
}

impl AudioCapture {
    pub fn start(kind: AudioSourceKind, sink: impl AudioSink) -> std::io::Result<Self> {
        let stop = Arc::new(AtomicBool::new(false));
        let game = Arc::new(Mutex::new(None));
        let thread = {
            let stop = Arc::clone(&stop);
            let game = Arc::clone(&game);
            let name = match kind {
                AudioSourceKind::Microphone => "audio-mic",
                _ => "audio",
            };
            thread::Builder::new()
                .name(name.into())
                .spawn(move || run(kind, &stop, &game, Box::new(sink)))?
        };
        Ok(Self {
            stop,
            game,
            thread: Some(thread),
        })
    }

    /// The recorded game window, to find the game's audio stream (`Game` only).
    pub fn set_game(&self, game: Option<GameWindow>) {
        *self.game.lock().unwrap_or_else(|e| e.into_inner()) = game;
    }
}

impl Drop for AudioCapture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            tracing::error!("audio thread panicked");
        }
    }
}

// --- libpulse FFI -------------------------------------------------------------------------

type Opaque = c_void;

const PA_SAMPLE_FLOAT32LE: c_int = 5;
const PA_CONTEXT_NOAUTOSPAWN: c_int = 1;
const PA_CONTEXT_READY: c_int = 4;
const PA_CONTEXT_FAILED: c_int = 5;
const PA_CONTEXT_TERMINATED: c_int = 6;
const PA_STREAM_READY: c_int = 2;
const PA_STREAM_FAILED: c_int = 3;
const PA_STREAM_TERMINATED: c_int = 4;
const PA_OPERATION_RUNNING: c_int = 0;
const PA_STREAM_ADJUST_LATENCY: c_int = 0x2000;
const PA_CHANNELS_MAX: usize = 32;

#[repr(C)]
struct SampleSpec {
    format: c_int,
    rate: u32,
    channels: u8,
}

#[repr(C)]
struct BufferAttr {
    maxlength: u32,
    tlength: u32,
    prebuf: u32,
    minreq: u32,
    fragsize: u32,
}

#[repr(C)]
struct ChannelMap {
    channels: u8,
    map: [c_int; PA_CHANNELS_MAX],
}

#[repr(C)]
struct CVolume {
    channels: u8,
    values: [u32; PA_CHANNELS_MAX],
}

/// `pa_sink_input_info` up to the fields we read (we only ever read it through a pointer).
#[repr(C)]
struct SinkInputInfo {
    index: u32,
    name: *const c_char,
    owner_module: u32,
    client: u32,
    sink: u32,
    sample_spec: SampleSpec,
    channel_map: ChannelMap,
    volume: CVolume,
    buffer_usec: u64,
    sink_usec: u64,
    resample_method: *const c_char,
    driver: *const c_char,
    mute: c_int,
    proplist: *mut Opaque,
    corked: c_int,
}

/// `pa_server_info` up to the fields we read.
#[repr(C)]
struct ServerInfo {
    user_name: *const c_char,
    host_name: *const c_char,
    server_version: *const c_char,
    server_name: *const c_char,
    sample_spec: SampleSpec,
    default_sink_name: *const c_char,
    default_source_name: *const c_char,
}

type SinkInputInfoCb = unsafe extern "C" fn(*mut Opaque, *const SinkInputInfo, c_int, *mut c_void);
type ServerInfoCb = unsafe extern "C" fn(*mut Opaque, *const ServerInfo, *mut c_void);
type ReadCb = unsafe extern "C" fn(*mut Opaque, usize, *mut c_void);

struct Pulse {
    _lib: Library,
    mainloop_new: unsafe extern "C" fn() -> *mut Opaque,
    mainloop_get_api: unsafe extern "C" fn(*mut Opaque) -> *mut Opaque,
    mainloop_prepare: unsafe extern "C" fn(*mut Opaque, c_int) -> c_int,
    mainloop_poll: unsafe extern "C" fn(*mut Opaque) -> c_int,
    mainloop_dispatch: unsafe extern "C" fn(*mut Opaque) -> c_int,
    mainloop_free: unsafe extern "C" fn(*mut Opaque),
    context_new: unsafe extern "C" fn(*mut Opaque, *const c_char) -> *mut Opaque,
    context_connect:
        unsafe extern "C" fn(*mut Opaque, *const c_char, c_int, *const c_void) -> c_int,
    context_get_state: unsafe extern "C" fn(*mut Opaque) -> c_int,
    context_errno: unsafe extern "C" fn(*mut Opaque) -> c_int,
    context_disconnect: unsafe extern "C" fn(*mut Opaque),
    context_unref: unsafe extern "C" fn(*mut Opaque),
    context_get_server_info:
        unsafe extern "C" fn(*mut Opaque, ServerInfoCb, *mut c_void) -> *mut Opaque,
    context_get_sink_input_info_list:
        unsafe extern "C" fn(*mut Opaque, SinkInputInfoCb, *mut c_void) -> *mut Opaque,
    operation_get_state: unsafe extern "C" fn(*mut Opaque) -> c_int,
    operation_unref: unsafe extern "C" fn(*mut Opaque),
    proplist_gets: unsafe extern "C" fn(*mut Opaque, *const c_char) -> *const c_char,
    stream_new: unsafe extern "C" fn(
        *mut Opaque,
        *const c_char,
        *const SampleSpec,
        *const c_void,
    ) -> *mut Opaque,
    stream_set_monitor_stream: unsafe extern "C" fn(*mut Opaque, u32) -> c_int,
    stream_set_read_callback: unsafe extern "C" fn(*mut Opaque, Option<ReadCb>, *mut c_void),
    stream_connect_record:
        unsafe extern "C" fn(*mut Opaque, *const c_char, *const BufferAttr, c_int) -> c_int,
    stream_get_state: unsafe extern "C" fn(*mut Opaque) -> c_int,
    stream_peek: unsafe extern "C" fn(*mut Opaque, *mut *const c_void, *mut usize) -> c_int,
    stream_drop: unsafe extern "C" fn(*mut Opaque) -> c_int,
    stream_disconnect: unsafe extern "C" fn(*mut Opaque) -> c_int,
    stream_unref: unsafe extern "C" fn(*mut Opaque),
    strerror: unsafe extern "C" fn(c_int) -> *const c_char,
}

impl Pulse {
    fn load() -> Result<Self, String> {
        // SAFETY: loading the system's libpulse runs no initialization code with side
        // effects that matter to us.
        let lib = unsafe { Library::new(LIBRARY) }.map_err(|e| {
            format!("cannot load {LIBRARY} (is PipeWire or PulseAudio installed?): {e}")
        })?;
        macro_rules! sym {
            ($name:literal) => {
                // SAFETY: the field type matches the documented C signature of the symbol.
                *unsafe { lib.get(concat!($name, "\0").as_bytes()) }
                    .map_err(|e| format!("{LIBRARY} has no {}: {e}", $name))?
            };
        }
        Ok(Self {
            mainloop_new: sym!("pa_mainloop_new"),
            mainloop_get_api: sym!("pa_mainloop_get_api"),
            mainloop_prepare: sym!("pa_mainloop_prepare"),
            mainloop_poll: sym!("pa_mainloop_poll"),
            mainloop_dispatch: sym!("pa_mainloop_dispatch"),
            mainloop_free: sym!("pa_mainloop_free"),
            context_new: sym!("pa_context_new"),
            context_connect: sym!("pa_context_connect"),
            context_get_state: sym!("pa_context_get_state"),
            context_errno: sym!("pa_context_errno"),
            context_disconnect: sym!("pa_context_disconnect"),
            context_unref: sym!("pa_context_unref"),
            context_get_server_info: sym!("pa_context_get_server_info"),
            context_get_sink_input_info_list: sym!("pa_context_get_sink_input_info_list"),
            operation_get_state: sym!("pa_operation_get_state"),
            operation_unref: sym!("pa_operation_unref"),
            proplist_gets: sym!("pa_proplist_gets"),
            stream_new: sym!("pa_stream_new"),
            stream_set_monitor_stream: sym!("pa_stream_set_monitor_stream"),
            stream_set_read_callback: sym!("pa_stream_set_read_callback"),
            stream_connect_record: sym!("pa_stream_connect_record"),
            stream_get_state: sym!("pa_stream_get_state"),
            stream_peek: sym!("pa_stream_peek"),
            stream_drop: sym!("pa_stream_drop"),
            stream_disconnect: sym!("pa_stream_disconnect"),
            stream_unref: sym!("pa_stream_unref"),
            strerror: sym!("pa_strerror"),
            _lib: lib,
        })
    }

    fn error_text(&self, code: c_int) -> String {
        // SAFETY: pa_strerror returns a static string (or NULL for unknown codes).
        let text = unsafe { (self.strerror)(code) };
        if text.is_null() {
            format!("error {code}")
        } else {
            // SAFETY: non-NULL result is a valid NUL-terminated static string.
            unsafe { CStr::from_ptr(text) }
                .to_string_lossy()
                .into_owned()
        }
    }
}

/// Copy a C string owned by libpulse.
fn owned(text: *const c_char) -> Option<String> {
    // SAFETY: callers pass NULL or a NUL-terminated string that is valid for this call.
    (!text.is_null()).then(|| {
        unsafe { CStr::from_ptr(text) }
            .to_string_lossy()
            .into_owned()
    })
}

/// A connection to the sound server with its own (non-threaded) main loop.
struct Connection<'a> {
    pulse: &'a Pulse,
    mainloop: *mut Opaque,
    context: *mut Opaque,
}

/// A sink input (an application's playback stream).
#[derive(Debug, Clone, PartialEq, Eq)]
struct SinkInput {
    index: u32,
    pid: Option<u32>,
    binary: Option<String>,
    app_name: Option<String>,
    corked: bool,
}

impl SinkInput {
    /// For the log: `#index pid=… binary=… name=…`.
    fn describe(&self) -> String {
        format!(
            "#{} pid={} binary={} name={}{}",
            self.index,
            self.pid.map_or("?".into(), |p| p.to_string()),
            self.binary.as_deref().unwrap_or("?"),
            self.app_name.as_deref().unwrap_or("?"),
            if self.corked { " (paused)" } else { "" }
        )
    }
}

impl<'a> Connection<'a> {
    fn open(pulse: &'a Pulse) -> Result<Self, String> {
        // SAFETY: plain constructor; result checked.
        let mainloop = unsafe { (pulse.mainloop_new)() };
        if mainloop.is_null() {
            return Err("pa_mainloop_new failed".into());
        }
        let mut conn = Self {
            pulse,
            mainloop,
            context: ptr::null_mut(),
        };
        // SAFETY: mainloop is valid; the API vtable lives as long as the main loop.
        let api = unsafe { (pulse.mainloop_get_api)(mainloop) };
        // SAFETY: api is valid, the name a static C string (copied by libpulse).
        conn.context = unsafe { (pulse.context_new)(api, c"Chronoclip".as_ptr()) };
        if conn.context.is_null() {
            return Err("pa_context_new failed".into());
        }
        // SAFETY: valid context; NULL server = default; no spawn API.
        let ret = unsafe {
            (pulse.context_connect)(
                conn.context,
                ptr::null(),
                PA_CONTEXT_NOAUTOSPAWN,
                ptr::null(),
            )
        };
        if ret < 0 {
            return Err(format!(
                "cannot connect to the sound server: {}",
                conn.last_error()
            ));
        }
        let deadline = Instant::now() + CONNECT_TIMEOUT;
        loop {
            // SAFETY: valid context.
            match unsafe { (pulse.context_get_state)(conn.context) } {
                PA_CONTEXT_READY => return Ok(conn),
                PA_CONTEXT_FAILED | PA_CONTEXT_TERMINATED => {
                    return Err(format!(
                        "cannot connect to the sound server: {}",
                        conn.last_error()
                    ));
                }
                _ if Instant::now() > deadline => {
                    return Err("the sound server did not answer".into());
                }
                _ => conn.iterate_once(TICK)?,
            }
        }
    }

    fn last_error(&self) -> String {
        // SAFETY: valid context.
        self.pulse
            .error_text(unsafe { (self.pulse.context_errno)(self.context) })
    }

    /// Run the main loop once: wait up to `timeout` for events and dispatch callbacks.
    fn iterate_once(&self, timeout: Duration) -> Result<(), String> {
        let timeout = c_int::try_from(timeout.as_micros()).unwrap_or(c_int::MAX);
        // SAFETY: the main loop is valid and only used from this thread.
        let ok = unsafe {
            (self.pulse.mainloop_prepare)(self.mainloop, timeout) >= 0
                && (self.pulse.mainloop_poll)(self.mainloop) >= 0
                && (self.pulse.mainloop_dispatch)(self.mainloop) >= 0
        };
        if !ok {
            return Err("the sound server main loop failed".into());
        }
        Ok(())
    }

    /// Run the main loop once and check that the connection is still up.
    fn iterate(&self, timeout: Duration) -> Result<(), String> {
        self.iterate_once(timeout)?;
        // SAFETY: valid context.
        match unsafe { (self.pulse.context_get_state)(self.context) } {
            PA_CONTEXT_FAILED | PA_CONTEXT_TERMINATED => Err(format!(
                "lost the connection to the sound server: {}",
                self.last_error()
            )),
            _ => Ok(()),
        }
    }

    /// Wait for an operation started with a callback that writes into data we own.
    fn wait(&self, operation: *mut Opaque) -> Result<(), String> {
        if operation.is_null() {
            return Err(format!("request failed: {}", self.last_error()));
        }
        let result = loop {
            // SAFETY: valid operation (we hold a reference until unref below).
            if unsafe { (self.pulse.operation_get_state)(operation) } != PA_OPERATION_RUNNING {
                break Ok(());
            }
            if let Err(e) = self.iterate(TICK) {
                break Err(e);
            }
        };
        // SAFETY: drop our reference exactly once.
        unsafe { (self.pulse.operation_unref)(operation) };
        result
    }

    fn default_sink(&self) -> Result<Option<String>, String> {
        unsafe extern "C" fn on_info(_: *mut Opaque, info: *const ServerInfo, data: *mut c_void) {
            // SAFETY: `data` is the `Option<String>` passed below, alive during the operation;
            // `info` is valid for this call.
            unsafe {
                let out = &mut *data.cast::<Option<String>>();
                if let Some(info) = info.as_ref() {
                    *out = owned(info.default_sink_name);
                }
            }
        }
        let mut name: Option<String> = None;
        // SAFETY: the callback only writes `name`, which outlives the operation (we wait).
        let op = unsafe {
            (self.pulse.context_get_server_info)(self.context, on_info, (&raw mut name).cast())
        };
        self.wait(op)?;
        Ok(name)
    }

    fn sink_inputs(&self) -> Result<Vec<SinkInput>, String> {
        struct List<'p> {
            pulse: &'p Pulse,
            inputs: Vec<SinkInput>,
        }
        unsafe extern "C" fn on_input(
            _: *mut Opaque,
            info: *const SinkInputInfo,
            eol: c_int,
            data: *mut c_void,
        ) {
            if eol != 0 {
                return;
            }
            // SAFETY: `data` is the `List` passed below, alive during the operation; `info`
            // and its proplist are valid for this call.
            unsafe {
                let list = &mut *data.cast::<List>();
                let Some(info) = info.as_ref() else { return };
                let get = |key: &CStr| {
                    if info.proplist.is_null() {
                        None
                    } else {
                        owned((list.pulse.proplist_gets)(info.proplist, key.as_ptr()))
                    }
                };
                list.inputs.push(SinkInput {
                    index: info.index,
                    pid: get(c"application.process.id").and_then(|p| p.parse().ok()),
                    binary: get(c"application.process.binary"),
                    app_name: get(c"application.name"),
                    corked: info.corked != 0,
                });
            }
        }
        let mut list = List {
            pulse: self.pulse,
            inputs: Vec::new(),
        };
        // SAFETY: the callback only writes `list`, which outlives the operation (we wait).
        let op = unsafe {
            (self.pulse.context_get_sink_input_info_list)(
                self.context,
                on_input,
                (&raw mut list).cast(),
            )
        };
        self.wait(op)?;
        Ok(list.inputs)
    }
}

impl Drop for Connection<'_> {
    fn drop(&mut self) {
        // SAFETY: each pointer is valid or NULL and released exactly once; streams were
        // dropped before (they borrow the connection).
        unsafe {
            if !self.context.is_null() {
                (self.pulse.context_disconnect)(self.context);
                (self.pulse.context_unref)(self.context);
            }
            (self.pulse.mainloop_free)(self.mainloop);
        }
    }
}

/// State the read callback works on. Lives in a `Box` for the whole capture thread; the
/// callback runs inside `Connection::iterate*` on the same thread, never concurrently with
/// the loop's own use of it.
struct Shared {
    pulse: *const Pulse,
    sink: Box<dyn AudioSink>,
    buffer: Vec<f32>,
}

unsafe extern "C" fn on_read(stream: *mut Opaque, _bytes: usize, data: *mut c_void) {
    // SAFETY: `data` is the `Shared` registered in `Recording::start`; it outlives the
    // stream and is not otherwise borrowed while the main loop dispatches callbacks.
    let shared = unsafe { &mut *data.cast::<Shared>() };
    // SAFETY: `pulse` points to the `Pulse` that outlives every stream.
    let pulse = unsafe { &*shared.pulse };
    loop {
        let mut bytes: *const c_void = ptr::null();
        let mut len = 0usize;
        // SAFETY: valid stream; out pointers are valid.
        if unsafe { (pulse.stream_peek)(stream, &mut bytes, &mut len) } < 0 || len == 0 {
            break;
        }
        if !bytes.is_null() {
            // Whole f32 samples (libpulse hands out whole frames).
            let count = len / size_of::<f32>();
            let start = shared.buffer.len();
            shared.buffer.resize(start + count, 0.0);
            // SAFETY: `bytes` holds `len` readable bytes until pa_stream_drop; the
            // destination has room for `count` f32s. Byte copy, so alignment doesn't matter.
            unsafe {
                ptr::copy_nonoverlapping(
                    bytes.cast::<u8>(),
                    shared.buffer.as_mut_ptr().add(start).cast::<u8>(),
                    count * size_of::<f32>(),
                );
            }
        }
        // A hole (NULL data) is skipped; the timeline fills it with silence.
        // SAFETY: we peeked a fragment above.
        unsafe { (pulse.stream_drop)(stream) };
    }
    if !shared.buffer.is_empty() {
        let end = Instant::now();
        shared.sink.samples(&shared.buffer, end);
        shared.buffer.clear();
    }
}

/// Where to record from.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Target {
    /// Source to record (`None` = default source).
    device: Option<CString>,
    /// Sink input to record on its own (an application's stream).
    monitor_stream: Option<u32>,
    state: AudioSourceState,
    /// Default output when the target was chosen; a change means "pick again".
    default_sink: Option<String>,
}

/// A recording stream on a [`Connection`].
struct Recording<'c, 'a> {
    conn: &'c Connection<'a>,
    stream: *mut Opaque,
}

impl<'c, 'a> Recording<'c, 'a> {
    fn start(
        conn: &'c Connection<'a>,
        target: &Target,
        name: &CStr,
        shared: *mut Shared,
    ) -> Result<Self, String> {
        let pulse = conn.pulse;
        let spec = SampleSpec {
            format: PA_SAMPLE_FLOAT32LE,
            rate: SAMPLE_RATE,
            channels: CHANNELS as u8,
        };
        // SAFETY: valid context; spec and name are valid for the call (copied); NULL map =
        // default channel map for the spec.
        let stream = unsafe { (pulse.stream_new)(conn.context, name.as_ptr(), &spec, ptr::null()) };
        if stream.is_null() {
            return Err(format!("cannot create a stream: {}", conn.last_error()));
        }
        let recording = Self { conn, stream };
        if let Some(index) = target.monitor_stream {
            // SAFETY: valid, unconnected stream.
            if unsafe { (pulse.stream_set_monitor_stream)(stream, index) } < 0 {
                return Err(format!(
                    "cannot record stream {index}: {}",
                    conn.last_error()
                ));
            }
        }
        let fragment = u32::try_from(
            FRAGMENT.as_millis() * u128::from(SAMPLE_RATE) / 1000
                * (CHANNELS * size_of::<f32>()) as u128,
        )
        .unwrap_or(u32::MAX);
        let attr = BufferAttr {
            maxlength: u32::MAX,
            tlength: u32::MAX,
            prebuf: u32::MAX,
            minreq: u32::MAX,
            fragsize: fragment,
        };
        let device = target.device.as_ref().map_or(ptr::null(), |d| d.as_ptr());
        // SAFETY: `shared` outlives the stream (see `Shared`); the callback is removed in
        // Drop before the stream is released. `attr` and `device` are copied by libpulse.
        let ret = unsafe {
            (pulse.stream_set_read_callback)(stream, Some(on_read), shared.cast());
            (pulse.stream_connect_record)(stream, device, &attr, PA_STREAM_ADJUST_LATENCY)
        };
        if ret < 0 {
            return Err(format!("cannot start recording: {}", conn.last_error()));
        }
        let deadline = Instant::now() + CONNECT_TIMEOUT;
        loop {
            match recording.state() {
                PA_STREAM_READY => return Ok(recording),
                PA_STREAM_FAILED | PA_STREAM_TERMINATED => {
                    return Err(format!("recording failed: {}", conn.last_error()));
                }
                _ if Instant::now() > deadline => {
                    return Err("the recording stream did not start".into());
                }
                _ => conn.iterate(TICK)?,
            }
        }
    }

    fn state(&self) -> c_int {
        // SAFETY: valid stream.
        unsafe { (self.conn.pulse.stream_get_state)(self.stream) }
    }

    fn failed(&self) -> bool {
        matches!(self.state(), PA_STREAM_FAILED | PA_STREAM_TERMINATED)
    }
}

impl Drop for Recording<'_, '_> {
    fn drop(&mut self) {
        let pulse = self.conn.pulse;
        // SAFETY: valid stream, released exactly once; the callback is cleared first so it
        // can't run with a dangling pointer.
        unsafe {
            (pulse.stream_set_read_callback)(self.stream, None, ptr::null_mut());
            (pulse.stream_disconnect)(self.stream);
            (pulse.stream_unref)(self.stream);
        }
    }
}

/// True if `input` is the game's playback stream:
/// - same process as the game window (Wine plays sound from the `.exe` process), or a WoW
///   executable as process or application name (PulseAudio clients such as winepulse), or
/// - application name "World of Warcraft…" or equal to the window title: Wine as a native
///   PipeWire client names its stream after the game, and pipewire-pulse then reports no
///   process id for it.
fn is_game_stream(input: &SinkInput, game: Option<&GameWindow>) -> bool {
    let pid = game.and_then(|g| g.pid);
    let title = game.map(|g| g.title.trim()).filter(|t| !t.is_empty());
    let named = |name: &str| {
        let name = name.trim();
        name.to_lowercase().starts_with("world of warcraft")
            || title.is_some_and(|t| name.eq_ignore_ascii_case(t))
    };
    (pid.is_some() && input.pid == pid)
        || input.binary.as_deref().is_some_and(is_wow_executable)
        || input.app_name.as_deref().is_some_and(is_wow_executable)
        || input.app_name.as_deref().is_some_and(named)
        || input.pid.is_some_and(process_is_wow)
}

/// `logged` remembers the playback streams last logged as "game not found", so the log shows
/// each new set of streams once instead of every 2 s.
fn choose_target(
    conn: &Connection,
    kind: AudioSourceKind,
    game: Option<&GameWindow>,
    logged: &mut Option<(Option<GameWindow>, Vec<SinkInput>)>,
) -> Result<Target, String> {
    let all_sound = |game_missing| -> Result<Target, String> {
        Ok(Target {
            device: Some(c"@DEFAULT_MONITOR@".to_owned()),
            monitor_stream: None,
            state: AudioSourceState::AllSound { game_missing },
            default_sink: conn.default_sink()?,
        })
    };
    match kind {
        AudioSourceKind::Microphone => Ok(Target {
            device: None,
            monitor_stream: None,
            state: AudioSourceState::Microphone,
            default_sink: None,
        }),
        AudioSourceKind::AllSound => all_sound(false),
        AudioSourceKind::Game => {
            let inputs = conn.sink_inputs()?;
            tracing::trace!(?inputs, ?game, "playback streams");
            let mut games: Vec<&SinkInput> =
                inputs.iter().filter(|i| is_game_stream(i, game)).collect();
            games.sort_by_key(|i| i.corked); // prefer a stream that is playing
            match games.first() {
                Some(input) => Ok(Target {
                    device: None,
                    monitor_stream: Some(input.index),
                    state: AudioSourceState::Game,
                    default_sink: None,
                }),
                None => {
                    let seen = Some((game.cloned(), inputs));
                    if *logged != seen {
                        let streams: Vec<String> = seen
                            .as_ref()
                            .map(|(_, inputs)| inputs.iter().map(SinkInput::describe).collect())
                            .unwrap_or_default();
                        tracing::info!(
                            ?game,
                            ?streams,
                            "game audio stream not found, recording all sound"
                        );
                        *logged = seen;
                    }
                    all_sound(true)
                }
            }
        }
    }
}

fn run(
    kind: AudioSourceKind,
    stop: &AtomicBool,
    game: &Mutex<Option<GameWindow>>,
    sink: Box<dyn AudioSink>,
) {
    let mut shared = Box::new(Shared {
        pulse: ptr::null(),
        sink,
        buffer: Vec::new(),
    });
    shared.sink.state(AudioSourceState::Connecting);
    let pulse = match Pulse::load() {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("no audio: {e}");
            shared.sink.state(AudioSourceState::Unavailable(e));
            while !idle(&mut shared, stop, Duration::from_secs(3600)) {}
            return;
        }
    };
    shared.pulse = &raw const pulse;
    let shared_ptr: *mut Shared = &raw mut *shared;

    while !stop.load(Ordering::Relaxed) {
        let result =
            Connection::open(&pulse).and_then(|conn| capture(&conn, kind, stop, game, shared_ptr));
        if let Err(e) = result {
            tracing::warn!(?kind, "audio capture: {e}");
            // SAFETY: no stream or callback is alive here, so we have the only access.
            let shared = unsafe { &mut *shared_ptr };
            shared.sink.state(AudioSourceState::Unavailable(e));
            if idle(shared, stop, RETRY) {
                break;
            }
        }
    }
}

/// Record until stopped or the connection fails.
fn capture(
    conn: &Connection,
    kind: AudioSourceKind,
    stop: &AtomicBool,
    game: &Mutex<Option<GameWindow>>,
    shared: *mut Shared,
) -> Result<(), String> {
    let name = match kind {
        AudioSourceKind::Microphone => c"Chronoclip microphone",
        _ => c"Chronoclip game audio",
    };
    let mut current: Option<(Target, Recording)> = None;
    let mut last_game = None;
    let mut last_check: Option<Instant> = None;
    let mut logged = None;
    while !stop.load(Ordering::Relaxed) {
        let current_game = game.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if current.as_ref().is_some_and(|(_, r)| r.failed()) {
            tracing::info!(?kind, "audio stream ended");
            current = None;
        }
        let due = last_check.is_none_or(|t| t.elapsed() >= CHECK_INTERVAL)
            || (kind == AudioSourceKind::Game && current_game != last_game);
        if due {
            last_check = Some(Instant::now());
            let target = choose_target(conn, kind, current_game.as_ref(), &mut logged)?;
            last_game = current_game;
            if current.as_ref().map(|(t, _)| t) != Some(&target) {
                current = None; // stop the old stream first
                let recording = Recording::start(conn, &target, name, shared);
                // SAFETY: called between main loop iterations: no callback is running.
                let sink = unsafe { &mut (*shared).sink };
                match recording {
                    Ok(recording) => {
                        tracing::info!(?kind, state = ?target.state, "recording audio");
                        sink.state(target.state.clone());
                        current = Some((target, recording));
                    }
                    Err(e) => {
                        tracing::warn!(?kind, "audio capture: {e}");
                        sink.state(AudioSourceState::Unavailable(e));
                    }
                }
            }
        }
        conn.iterate(TICK)?;
        // SAFETY: as above, between iterations.
        unsafe { (*shared).sink.tick(Instant::now()) };
    }
    Ok(())
}

/// Sleep up to `duration` while ticking the sink. Returns true if stopped.
fn idle(shared: &mut Shared, stop: &AtomicBool, duration: Duration) -> bool {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        if stop.load(Ordering::Relaxed) {
            return true;
        }
        thread::sleep(TICK);
        shared.sink.tick(Instant::now());
    }
    stop.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(pid: Option<u32>, binary: &str) -> SinkInput {
        SinkInput {
            index: 7,
            pid,
            binary: Some(binary.into()),
            app_name: None,
            corked: false,
        }
    }

    fn window(pid: u32, title: &str) -> GameWindow {
        GameWindow {
            pid: Some(pid),
            title: title.into(),
        }
    }

    #[test]
    fn finds_game_stream_by_pid_or_name() {
        let ffplay = window(42, "cc-avtest");
        assert!(is_game_stream(&input(Some(42), "ffplay"), Some(&ffplay)));
        assert!(!is_game_stream(&input(Some(43), "ffplay"), Some(&ffplay)));
        assert!(!is_game_stream(&input(None, "ffplay"), None));
        assert!(is_game_stream(&input(Some(1), "Wow.exe"), None));
        let mut wine = input(Some(1), "wine64-preloader");
        assert!(!is_game_stream(
            &wine,
            Some(&window(2, "World of Warcraft"))
        ));
        wine.app_name = Some("WowClassic.exe".into());
        assert!(is_game_stream(&wine, Some(&window(2, ""))));
    }

    #[test]
    fn finds_native_pipewire_game_stream_by_name() {
        // Wine/Proton as a native PipeWire client, seen through pipewire-pulse: no process
        // id or binary, only the application name (measured with WoW Retail via Faugus).
        let stream = SinkInput {
            index: 5376,
            pid: None,
            binary: None,
            app_name: Some("World of Warcraft".into()),
            corked: false,
        };
        assert!(is_game_stream(
            &stream,
            Some(&window(532_916, "World of Warcraft"))
        ));
        assert!(is_game_stream(&stream, None));
        // Any game whose stream is named like its window.
        let other = SinkInput {
            app_name: Some("Some Game".into()),
            ..stream.clone()
        };
        assert!(is_game_stream(&other, Some(&window(7, "Some Game"))));
        assert!(!is_game_stream(
            &other,
            Some(&window(7, "World of Warcraft"))
        ));
        assert!(!is_game_stream(&other, None));
    }

    #[test]
    fn struct_layout_matches_libpulse() {
        // Offsets on 64-bit Linux, from <pulse/introspect.h>.
        assert_eq!(std::mem::offset_of!(SinkInputInfo, sample_spec), 28);
        assert_eq!(std::mem::offset_of!(SinkInputInfo, buffer_usec), 304);
        assert_eq!(std::mem::offset_of!(SinkInputInfo, proplist), 344);
        assert_eq!(std::mem::offset_of!(SinkInputInfo, corked), 352);
        assert_eq!(std::mem::offset_of!(ServerInfo, default_sink_name), 48);
    }
}
