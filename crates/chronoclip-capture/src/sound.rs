//! Playing short sounds (the save chime) through the PulseAudio "simple" API
//! (`libpulse-simple.so.0`, loaded at runtime; served by PipeWire's pipewire-pulse).

use std::ffi::{CStr, c_char, c_int, c_void};
use std::ptr;
use std::sync::OnceLock;

use libloading::Library;

const LIBRARY: &str = "libpulse-simple.so.0";
const PA_STREAM_PLAYBACK: c_int = 1;
const PA_SAMPLE_S16LE: c_int = 3;

/// `pa_sample_spec`
#[repr(C)]
struct SampleSpec {
    format: c_int,
    rate: u32,
    channels: u8,
}

type SimpleNew = unsafe extern "C" fn(
    server: *const c_char,
    name: *const c_char,
    dir: c_int,
    dev: *const c_char,
    stream_name: *const c_char,
    spec: *const SampleSpec,
    map: *const c_void,
    attr: *const c_void,
    error: *mut c_int,
) -> *mut c_void;

struct PulseSimple {
    _lib: Library,
    new: SimpleNew,
    write: unsafe extern "C" fn(*mut c_void, *const c_void, usize, *mut c_int) -> c_int,
    drain: unsafe extern "C" fn(*mut c_void, *mut c_int) -> c_int,
    free: unsafe extern "C" fn(*mut c_void),
}

impl PulseSimple {
    fn load() -> Result<Self, String> {
        // SAFETY: loading the system's libpulse-simple runs no initialization with side effects.
        let lib =
            unsafe { Library::new(LIBRARY) }.map_err(|e| format!("cannot load {LIBRARY}: {e}"))?;
        macro_rules! sym {
            ($name:literal) => {
                // SAFETY: the field type matches the documented C signature of the symbol.
                *unsafe { lib.get(concat!($name, "\0").as_bytes()) }
                    .map_err(|e| format!("{LIBRARY} has no {}: {e}", $name))?
            };
        }
        Ok(Self {
            new: sym!("pa_simple_new"),
            write: sym!("pa_simple_write"),
            drain: sym!("pa_simple_drain"),
            free: sym!("pa_simple_free"),
            _lib: lib,
        })
    }

    fn get() -> Result<&'static Self, String> {
        static API: OnceLock<Result<PulseSimple, String>> = OnceLock::new();
        API.get_or_init(Self::load).as_ref().map_err(Clone::clone)
    }
}

/// Play mono signed 16-bit `samples` at `sample_rate` on the default output and wait until
/// they have been played. Blocks for about the length of the sound; call it off the UI thread.
pub fn play_sound(samples: &[i16], sample_rate: u32) -> Result<(), String> {
    let api = PulseSimple::get()?;
    let spec = SampleSpec {
        format: PA_SAMPLE_S16LE,
        rate: sample_rate,
        channels: 1,
    };
    let name: &CStr = c"Chronoclip";
    let stream_name: &CStr = c"Save sound";
    let mut error: c_int = 0;
    // SAFETY: all strings are valid NUL-terminated C strings, `spec` outlives the call, NULL
    // selects the default server, device, channel map and buffer attributes.
    let stream = unsafe {
        (api.new)(
            ptr::null(),
            name.as_ptr(),
            PA_STREAM_PLAYBACK,
            ptr::null(),
            stream_name.as_ptr(),
            &spec,
            ptr::null(),
            ptr::null(),
            &mut error,
        )
    };
    if stream.is_null() {
        return Err(format!("cannot open a playback stream (error {error})"));
    }
    // SAFETY: `stream` is a valid connection until `free`; the buffer is read during `write`.
    let result = unsafe {
        if (api.write)(
            stream,
            samples.as_ptr().cast(),
            std::mem::size_of_val(samples),
            &mut error,
        ) < 0
        {
            Err(format!("cannot play the sound (error {error})"))
        } else if (api.drain)(stream, &mut error) < 0 {
            Err(format!("cannot finish playing the sound (error {error})"))
        } else {
            Ok(())
        }
    };
    // SAFETY: frees the connection created above exactly once.
    unsafe { (api.free)(stream) };
    result
}
