//! Minimal CUDA driver API, loaded at runtime from `libcuda.so.1` (part of the NVIDIA driver).

use std::ffi::{CStr, c_char, c_int, c_uint, c_void};
use std::sync::OnceLock;

use libloading::Library;

use crate::error::CaptureError;

pub type CuResult = c_int;
pub type CuContext = *mut c_void;
pub type CuGraphicsResource = *mut c_void;
pub type CuArray = *mut c_void;
pub type CuStream = *mut c_void;
pub type CuDevicePtr = u64;

pub const CU_GRAPHICS_REGISTER_FLAGS_READ_ONLY: c_uint = 1;
pub const CU_MEMORYTYPE_DEVICE: c_uint = 2;
pub const CU_MEMORYTYPE_ARRAY: c_uint = 3;

/// `CUDA_MEMCPY2D` (v2).
#[repr(C)]
#[derive(Default)]
pub struct CudaMemcpy2D {
    pub src_x_in_bytes: usize,
    pub src_y: usize,
    pub src_memory_type: c_uint,
    pub src_host: usize,
    pub src_device: CuDevicePtr,
    pub src_array: usize,
    pub src_pitch: usize,
    pub dst_x_in_bytes: usize,
    pub dst_y: usize,
    pub dst_memory_type: c_uint,
    pub dst_host: usize,
    pub dst_device: CuDevicePtr,
    pub dst_array: usize,
    pub dst_pitch: usize,
    pub width_in_bytes: usize,
    pub height: usize,
}

pub struct Cuda {
    _lib: Library,
    pub ctx_push: unsafe extern "C" fn(CuContext) -> CuResult,
    pub ctx_pop: unsafe extern "C" fn(*mut CuContext) -> CuResult,
    pub graphics_gl_register_image:
        unsafe extern "C" fn(*mut CuGraphicsResource, c_uint, c_uint, c_uint) -> CuResult,
    pub graphics_unregister_resource: unsafe extern "C" fn(CuGraphicsResource) -> CuResult,
    pub graphics_map_resources:
        unsafe extern "C" fn(c_uint, *mut CuGraphicsResource, CuStream) -> CuResult,
    pub graphics_unmap_resources:
        unsafe extern "C" fn(c_uint, *mut CuGraphicsResource, CuStream) -> CuResult,
    pub graphics_sub_resource_get_mapped_array:
        unsafe extern "C" fn(*mut CuArray, CuGraphicsResource, c_uint, c_uint) -> CuResult,
    pub memcpy_2d: unsafe extern "C" fn(*const CudaMemcpy2D) -> CuResult,
    get_error_name: unsafe extern "C" fn(CuResult, *mut *const c_char) -> CuResult,
}

static CUDA: OnceLock<Result<Cuda, String>> = OnceLock::new();

impl Cuda {
    /// The process-wide CUDA API.
    pub fn get() -> Result<&'static Cuda, CaptureError> {
        CUDA.get_or_init(|| Self::load().map_err(|e| e.to_string()))
            .as_ref()
            .map_err(|e| CaptureError::Xlib(format!("CUDA unavailable: {e}")))
    }

    fn load() -> Result<Self, CaptureError> {
        let err = |source| CaptureError::Library {
            library: "libcuda.so.1 (NVIDIA driver)",
            source,
        };
        // SAFETY: loading the NVIDIA driver library; symbols are resolved with their documented
        // C signatures (the `_v2` variants are the current ABI).
        unsafe {
            let lib = Library::new("libcuda.so.1").map_err(err)?;
            macro_rules! sym {
                ($name:literal) => {
                    *lib.get(concat!($name, "\0").as_bytes()).map_err(err)?
                };
            }
            Ok(Self {
                ctx_push: sym!("cuCtxPushCurrent_v2"),
                ctx_pop: sym!("cuCtxPopCurrent_v2"),
                graphics_gl_register_image: sym!("cuGraphicsGLRegisterImage"),
                graphics_unregister_resource: sym!("cuGraphicsUnregisterResource"),
                graphics_map_resources: sym!("cuGraphicsMapResources"),
                graphics_unmap_resources: sym!("cuGraphicsUnmapResources"),
                graphics_sub_resource_get_mapped_array: sym!("cuGraphicsSubResourceGetMappedArray"),
                memcpy_2d: sym!("cuMemcpy2D_v2"),
                get_error_name: sym!("cuGetErrorName"),
                _lib: lib,
            })
        }
    }

    pub fn check(&self, call: &'static str, code: CuResult) -> Result<(), CaptureError> {
        if code == 0 {
            return Ok(());
        }
        let mut name: *const c_char = std::ptr::null();
        // SAFETY: cuGetErrorName writes a pointer to a static string (or leaves it NULL).
        unsafe { (self.get_error_name)(code, &mut name) };
        let name = if name.is_null() {
            "unknown".to_owned()
        } else {
            // SAFETY: non-NULL pointer to a static NUL-terminated string owned by the driver.
            unsafe { CStr::from_ptr(name) }
                .to_string_lossy()
                .into_owned()
        };
        Err(CaptureError::Cuda { call, code, name })
    }

    /// Run `f` with `ctx` current on this thread.
    pub fn with_context<T>(
        &self,
        ctx: CuContext,
        f: impl FnOnce() -> Result<T, CaptureError>,
    ) -> Result<T, CaptureError> {
        // SAFETY: ctx is a valid context owned by the encoder; push/pop are balanced below.
        self.check("cuCtxPushCurrent", unsafe { (self.ctx_push)(ctx) })?;
        let result = f();
        let mut popped: CuContext = std::ptr::null_mut();
        // SAFETY: pops the context pushed above.
        let pop = self.check("cuCtxPopCurrent", unsafe { (self.ctx_pop)(&mut popped) });
        let value = result?;
        pop?;
        Ok(value)
    }
}
