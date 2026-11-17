//! H.264 encoding with NVENC from CUDA frames.

use std::ffi::{CStr, c_void};
use std::ptr;

use ffmpeg_next::ffi;
use ffmpeg_next::packet::Mut as _;
use ffmpeg_next::{Packet, Rational};

use crate::error::{EncodeError, check};

const ENCODER_NAME: &str = "h264_nvenc";

/// Encoder settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EncoderConfig {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub bitrate_bps: u64,
    /// Keyframe interval in frames. Equal to the segment length so every segment starts with an IDR.
    pub gop_frames: u32,
}

/// NVENC H.264 encoder fed with CUDA frames (`RGB0` in GPU memory).
///
/// Uses the CUDA *primary* context, so other code can share it (see [`Self::cuda_context`]).
pub struct NvencEncoder {
    codec_ctx: *mut ffi::AVCodecContext,
    frames_ref: *mut ffi::AVBufferRef,
    device_ref: *mut ffi::AVBufferRef,
    cuda_ctx: *mut c_void,
    config: EncoderConfig,
}

// SAFETY: The FFmpeg objects are owned exclusively by this value and only used through `&mut self`
// (or `&self` for read-only fields); moving them to another thread is fine.
unsafe impl Send for NvencEncoder {}

/// A CUDA frame from the encoder's pool. Fill it (e.g. with `cuMemcpy2D`) and pass it to
/// [`NvencEncoder::encode`].
pub struct HwFrame {
    frame: *mut ffi::AVFrame,
}

// SAFETY: The AVFrame is exclusively owned; its buffers are reference counted by FFmpeg.
unsafe impl Send for HwFrame {}

impl HwFrame {
    /// `CUdeviceptr` of the first (only) plane.
    pub fn device_ptr(&self) -> u64 {
        // SAFETY: `frame` is a valid frame allocated by `av_hwframe_get_buffer`.
        unsafe { (*self.frame).data[0] as u64 }
    }

    /// Row pitch of the plane in bytes.
    pub fn pitch(&self) -> usize {
        // SAFETY: as above; linesize is non-negative for CUDA frames.
        unsafe { (*self.frame).linesize[0] as usize }
    }
}

impl Drop for HwFrame {
    fn drop(&mut self) {
        // SAFETY: `frame` was allocated with `av_frame_alloc` and is freed exactly once here.
        unsafe { ffi::av_frame_free(&mut self.frame) }
    }
}

fn set_opt(dict: &mut *mut ffi::AVDictionary, key: &CStr, value: &str) -> Result<(), EncodeError> {
    let value = std::ffi::CString::new(value).map_err(|_| EncodeError::OutOfMemory("option"))?;
    // SAFETY: both strings are valid NUL-terminated C strings; av_dict_set copies them.
    check("av_dict_set", unsafe {
        ffi::av_dict_set(dict, key.as_ptr(), value.as_ptr(), 0)
    })
    .map(drop)
}

impl NvencEncoder {
    pub fn new(config: EncoderConfig) -> Result<Self, EncodeError> {
        crate::init()?;
        let mut enc = Self {
            codec_ctx: ptr::null_mut(),
            frames_ref: ptr::null_mut(),
            device_ref: ptr::null_mut(),
            cuda_ctx: ptr::null_mut(),
            config,
        };
        // On error, `enc` is dropped and frees whatever was allocated so far.
        // SAFETY: each step checks for failure before the next uses its result.
        unsafe { enc.open()? };
        Ok(enc)
    }

    unsafe fn open(&mut self) -> Result<(), EncodeError> {
        let cfg = self.config;
        let fps = i32::try_from(cfg.fps).unwrap_or(60);

        // SAFETY (whole function): FFI calls follow the documented FFmpeg hwcontext/encoder setup;
        // every allocation is checked for NULL and stored in `self` so `Drop` frees it.
        unsafe {
            // CUDA device using the primary context.
            let mut opts: *mut ffi::AVDictionary = ptr::null_mut();
            set_opt(&mut opts, c"primary_ctx", "1")?;
            let ret = ffi::av_hwdevice_ctx_create(
                &mut self.device_ref,
                ffi::AVHWDeviceType::AV_HWDEVICE_TYPE_CUDA,
                ptr::null(),
                opts,
                0,
            );
            ffi::av_dict_free(&mut opts);
            check("av_hwdevice_ctx_create(cuda)", ret)?;

            // AVCUDADeviceContext starts with `CUcontext cuda_ctx`.
            let device_ctx = (*self.device_ref).data as *mut ffi::AVHWDeviceContext;
            self.cuda_ctx = *((*device_ctx).hwctx as *mut *mut c_void);

            // Frame pool: RGB0 frames in GPU memory.
            self.frames_ref = ffi::av_hwframe_ctx_alloc(self.device_ref);
            if self.frames_ref.is_null() {
                return Err(EncodeError::OutOfMemory("av_hwframe_ctx_alloc"));
            }
            let frames_ctx = (*self.frames_ref).data as *mut ffi::AVHWFramesContext;
            (*frames_ctx).format = ffi::AVPixelFormat::AV_PIX_FMT_CUDA;
            (*frames_ctx).sw_format = ffi::AVPixelFormat::AV_PIX_FMT_RGB0;
            (*frames_ctx).width = cfg.width as i32;
            (*frames_ctx).height = cfg.height as i32;
            check(
                "av_hwframe_ctx_init",
                ffi::av_hwframe_ctx_init(self.frames_ref),
            )?;

            // Encoder.
            let codec = ffi::avcodec_find_encoder_by_name(c"h264_nvenc".as_ptr());
            if codec.is_null() {
                return Err(EncodeError::EncoderUnavailable(ENCODER_NAME));
            }
            self.codec_ctx = ffi::avcodec_alloc_context3(codec);
            if self.codec_ctx.is_null() {
                return Err(EncodeError::OutOfMemory("avcodec_alloc_context3"));
            }
            let ctx = &mut *self.codec_ctx;
            ctx.width = cfg.width as i32;
            ctx.height = cfg.height as i32;
            ctx.time_base = ffi::AVRational { num: 1, den: fps };
            ctx.framerate = ffi::AVRational { num: fps, den: 1 };
            ctx.pix_fmt = ffi::AVPixelFormat::AV_PIX_FMT_CUDA;
            ctx.gop_size = cfg.gop_frames as i32;
            ctx.max_b_frames = 0;
            ctx.bit_rate = cfg.bitrate_bps as i64;
            ctx.rc_max_rate = (cfg.bitrate_bps * 3 / 2) as i64;
            ctx.rc_buffer_size = (cfg.bitrate_bps * 2) as i32;
            ctx.hw_frames_ctx = ffi::av_buffer_ref(self.frames_ref);
            if ctx.hw_frames_ctx.is_null() {
                return Err(EncodeError::OutOfMemory("av_buffer_ref"));
            }

            let mut opts: *mut ffi::AVDictionary = ptr::null_mut();
            for (key, value) in [
                (c"preset", "p4"),
                (c"tune", "hq"),
                (c"rc", "vbr"),
                (c"profile", "high"),
                // Keyframes exactly every `gop_frames`, so segments can be joined without re-encoding.
                (c"forced-idr", "1"),
                (c"no-scenecut", "1"),
                (c"strict_gop", "1"),
            ] {
                set_opt(&mut opts, key, value)?;
            }
            let ret = ffi::avcodec_open2(self.codec_ctx, codec, &mut opts);
            warn_unused_options(opts);
            ffi::av_dict_free(&mut opts);
            check("avcodec_open2(h264_nvenc)", ret)?;
        }
        tracing::info!(
            width = cfg.width,
            height = cfg.height,
            fps = cfg.fps,
            bitrate_mbps = cfg.bitrate_bps / 1_000_000,
            "NVENC encoder ready"
        );
        Ok(())
    }

    pub fn config(&self) -> EncoderConfig {
        self.config
    }

    /// The `CUcontext` the encoder uses (the device's primary context).
    pub fn cuda_context(&self) -> *mut c_void {
        self.cuda_ctx
    }

    /// Encoder time base (1 / fps): packet timestamps are frame numbers.
    pub fn time_base(&self) -> Rational {
        Rational::new(1, self.config.fps as i32)
    }

    /// Codec parameters for muxers (SPS/PPS are repeated in-band at every keyframe).
    pub fn parameters(&self) -> Result<ffmpeg_next::codec::Parameters, EncodeError> {
        let mut params = ffmpeg_next::codec::Parameters::new();
        // SAFETY: both pointers are valid; the function copies fields into `params`.
        check("avcodec_parameters_from_context", unsafe {
            ffi::avcodec_parameters_from_context(params.as_mut_ptr(), self.codec_ctx)
        })?;
        Ok(params)
    }

    /// Get an empty CUDA frame from the pool.
    pub fn alloc_frame(&mut self) -> Result<HwFrame, EncodeError> {
        // SAFETY: av_frame_alloc result is checked; av_hwframe_get_buffer fills it from our pool.
        unsafe {
            let frame = ffi::av_frame_alloc();
            if frame.is_null() {
                return Err(EncodeError::OutOfMemory("av_frame_alloc"));
            }
            let hw = HwFrame { frame };
            check(
                "av_hwframe_get_buffer",
                ffi::av_hwframe_get_buffer(self.frames_ref, hw.frame, 0),
            )?;
            Ok(hw)
        }
    }

    /// Upload a CPU `RGB0` image (`pitch` bytes per row) into a new CUDA frame.
    /// Used by tests and the CPU fallback path.
    pub fn upload_rgb0(&mut self, data: &[u8], pitch: usize) -> Result<HwFrame, EncodeError> {
        let (w, h) = (self.config.width as usize, self.config.height as usize);
        assert!(
            pitch >= w * 4 && data.len() >= pitch * h,
            "image buffer too small"
        );
        let hw = self.alloc_frame()?;
        // SAFETY: `sw` borrows `data` only for the duration of the transfer; sizes were checked above.
        unsafe {
            let mut sw = ffi::av_frame_alloc();
            if sw.is_null() {
                return Err(EncodeError::OutOfMemory("av_frame_alloc"));
            }
            (*sw).format = ffi::AVPixelFormat::AV_PIX_FMT_RGB0 as i32;
            (*sw).width = w as i32;
            (*sw).height = h as i32;
            (*sw).data[0] = data.as_ptr().cast_mut();
            (*sw).linesize[0] = pitch as i32;
            let ret = ffi::av_hwframe_transfer_data(hw.frame, sw, 0);
            (*sw).data[0] = ptr::null_mut();
            ffi::av_frame_free(&mut sw);
            check("av_hwframe_transfer_data", ret)?;
        }
        Ok(hw)
    }

    /// Encode `frame` with presentation time `pts` (frame number), or flush with `None`.
    /// Finished packets are appended to `out`.
    pub fn encode(
        &mut self,
        frame: Option<(HwFrame, i64)>,
        out: &mut Vec<Packet>,
    ) -> Result<(), EncodeError> {
        // SAFETY: codec_ctx is open; frame (if any) is a valid frame from our pool and is only
        // referenced by the encoder after this call (send_frame takes its own reference).
        unsafe {
            let ret = match &frame {
                Some((hw, pts)) => {
                    (*hw.frame).pts = *pts;
                    ffi::avcodec_send_frame(self.codec_ctx, hw.frame)
                }
                None => ffi::avcodec_send_frame(self.codec_ctx, ptr::null()),
            };
            if ret != ffi::AVERROR_EOF {
                check("avcodec_send_frame", ret)?;
            }
            loop {
                let mut packet = Packet::empty();
                let ret = ffi::avcodec_receive_packet(self.codec_ctx, packet.as_mut_ptr());
                if ret == ffi::AVERROR(ffi::EAGAIN) || ret == ffi::AVERROR_EOF {
                    break;
                }
                check("avcodec_receive_packet", ret)?;
                out.push(packet);
            }
        }
        Ok(())
    }
}

fn warn_unused_options(opts: *mut ffi::AVDictionary) {
    let mut entry: *const ffi::AVDictionaryEntry = ptr::null();
    loop {
        // SAFETY: iterating a valid (possibly NULL) dictionary with an empty key and IGNORE_SUFFIX.
        entry = unsafe { ffi::av_dict_get(opts, c"".as_ptr(), entry, ffi::AV_DICT_IGNORE_SUFFIX) };
        if entry.is_null() {
            break;
        }
        // SAFETY: entry and its key are valid while the dictionary lives.
        let key = unsafe { CStr::from_ptr((*entry).key) }.to_string_lossy();
        tracing::warn!("{ENCODER_NAME} ignored option `{key}`");
    }
}

impl Drop for NvencEncoder {
    fn drop(&mut self) {
        // SAFETY: each pointer is either NULL or owned by us; the free functions accept NULL.
        unsafe {
            ffi::avcodec_free_context(&mut self.codec_ctx);
            ffi::av_buffer_unref(&mut self.frames_ref);
            ffi::av_buffer_unref(&mut self.device_ref);
        }
    }
}
