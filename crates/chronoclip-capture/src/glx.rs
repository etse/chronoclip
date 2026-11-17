//! XComposite + GLX_EXT_texture_from_pixmap capture into CUDA memory.

use std::ffi::{CString, c_int, c_uint, c_ulong};
use std::ptr;

use libloading::Library;
use x11_dl::glx::{self, GLXContext, GLXFBConfig, Glx};
use x11_dl::xlib::{self, Display};

use crate::CudaDestination;
use crate::cuda::{
    CU_GRAPHICS_REGISTER_FLAGS_READ_ONLY, CU_MEMORYTYPE_ARRAY, CU_MEMORYTYPE_DEVICE, CuArray,
    CuGraphicsResource, Cuda, CudaMemcpy2D,
};
use crate::error::CaptureError;
use crate::xlib::XConnection;

// GLX_EXT_texture_from_pixmap
const GLX_BIND_TO_TEXTURE_RGB_EXT: c_int = 0x20D0;
const GLX_BIND_TO_TEXTURE_RGBA_EXT: c_int = 0x20D1;
const GLX_BIND_TO_TEXTURE_TARGETS_EXT: c_int = 0x20D3;
const GLX_Y_INVERTED_EXT: c_int = 0x20D4;
const GLX_TEXTURE_FORMAT_EXT: c_int = 0x20D5;
const GLX_TEXTURE_TARGET_EXT: c_int = 0x20D6;
const GLX_TEXTURE_FORMAT_RGB_EXT: c_int = 0x20D9;
const GLX_TEXTURE_FORMAT_RGBA_EXT: c_int = 0x20DA;
const GLX_TEXTURE_2D_BIT_EXT: c_int = 0x2;
const GLX_TEXTURE_2D_EXT: c_int = 0x20DC;
const GLX_FRONT_EXT: c_int = 0x20DE;

const COMPOSITE_REDIRECT_AUTOMATIC: c_int = 0;

// Copies the window texture into the destination with a full-screen triangle. NVIDIA can't
// attach texture-from-pixmap textures to a framebuffer, so a plain blit isn't possible.
const VERTEX_SHADER: &str = r#"#version 330 core
uniform bool flip_y;
out vec2 uv;
void main() {
    vec2 pos = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));
    uv = vec2(pos.x, flip_y ? 1.0 - pos.y : pos.y);
    gl_Position = vec4(pos * 2.0 - 1.0, 0.0, 1.0);
}
"#;
const FRAGMENT_SHADER: &str = r#"#version 330 core
uniform sampler2D window_texture;
in vec2 uv;
out vec4 color;
void main() {
    color = vec4(texture(window_texture, uv).rgb, 1.0);
}
"#;

type BindTexImage = unsafe extern "C" fn(*mut Display, glx::GLXDrawable, c_int, *const c_int);
type ReleaseTexImage = unsafe extern "C" fn(*mut Display, glx::GLXDrawable, c_int);

struct Composite {
    _lib: Library,
    query_extension: unsafe extern "C" fn(*mut Display, *mut c_int, *mut c_int) -> c_int,
    redirect_window: unsafe extern "C" fn(*mut Display, xlib::Window, c_int),
    unredirect_window: unsafe extern "C" fn(*mut Display, xlib::Window, c_int),
    name_window_pixmap: unsafe extern "C" fn(*mut Display, xlib::Window) -> xlib::Pixmap,
}

impl Composite {
    fn load() -> Result<Self, CaptureError> {
        let err = |source| CaptureError::Library {
            library: "libXcomposite.so.1",
            source,
        };
        // SAFETY: symbols are resolved with their documented Xcomposite signatures.
        unsafe {
            let lib = Library::new("libXcomposite.so.1").map_err(err)?;
            Ok(Self {
                query_extension: *lib.get(b"XCompositeQueryExtension\0").map_err(err)?,
                redirect_window: *lib.get(b"XCompositeRedirectWindow\0").map_err(err)?,
                unredirect_window: *lib.get(b"XCompositeUnredirectWindow\0").map_err(err)?,
                name_window_pixmap: *lib.get(b"XCompositeNameWindowPixmap\0").map_err(err)?,
                _lib: lib,
            })
        }
    }
}

/// Result of [`X11WindowCapture::capture`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureStatus {
    /// The frame was written to the destination.
    Captured,
    /// The window changed size; recreate the capture (and encoder) at the new size.
    Resized { width: u32, height: u32 },
}

/// Captures one X11 window into CUDA memory. Create, use and drop it on the same thread
/// (it owns an OpenGL context).
pub struct X11WindowCapture {
    conn: XConnection,
    glx: Glx,
    composite: Composite,
    cuda: &'static Cuda,
    cuda_ctx: *mut std::ffi::c_void,
    window: xlib::Window,
    width: u32,
    height: u32,
    y_inverted: bool,
    context: GLXContext,
    pbuffer: c_ulong,
    /// The window's backing pixmap (XCompositeNameWindowPixmap).
    pixmap: xlib::Pixmap,
    /// Copy mode: our own pixmap that the window is copied into each frame, for windows whose
    /// backing pixmap NVIDIA refuses to bind directly (Vulkan/DXVK apps such as Wine games).
    copy_pixmap: xlib::Pixmap,
    copy_gc: xlib::GC,
    glx_pixmap: c_ulong,
    bind_tex_image: BindTexImage,
    release_tex_image: ReleaseTexImage,
    src_texture: c_uint,
    dst_texture: c_uint,
    framebuffer: c_uint,
    vertex_array: c_uint,
    program: c_uint,
    resource: CuGraphicsResource,
}

impl X11WindowCapture {
    /// Start capturing `window`. `cuda_ctx` is the CUDA context of the frames passed to
    /// [`Self::capture`] (the encoder's).
    pub fn new(window: u64, cuda_ctx: *mut std::ffi::c_void) -> Result<Self, CaptureError> {
        let conn = XConnection::open()?;
        let glx = Glx::open().map_err(|e| CaptureError::Xlib(e.to_string()))?;
        let composite = Composite::load()?;
        let cuda = Cuda::get()?;

        let (mut event_base, mut error_base) = (0, 0);
        // SAFETY: valid display and out-pointers.
        if unsafe { (composite.query_extension)(conn.display, &mut event_base, &mut error_base) }
            == 0
        {
            return Err(CaptureError::MissingExtension("Composite"));
        }

        let attrs = conn.window_attributes(window)?;
        if attrs.map_state != xlib::IsViewable {
            return Err(CaptureError::WindowNotViewable);
        }
        let (width, height) = (attrs.width.max(1) as u32, attrs.height.max(1) as u32);
        tracing::debug!(
            window = format!("0x{window:x}"),
            width,
            height,
            depth = attrs.depth,
            children = ?conn.children(window),
            "window to capture"
        );

        let mut capture = Self {
            conn,
            glx,
            composite,
            cuda,
            cuda_ctx,
            window,
            width,
            height,
            y_inverted: true,
            context: ptr::null_mut(),
            pbuffer: 0,
            pixmap: 0,
            copy_pixmap: 0,
            copy_gc: ptr::null_mut(),
            glx_pixmap: 0,
            bind_tex_image: noop_bind,
            release_tex_image: noop_release,
            src_texture: 0,
            dst_texture: 0,
            framebuffer: 0,
            vertex_array: 0,
            program: 0,
            resource: ptr::null_mut(),
        };
        // On error, Drop cleans up whatever was created.
        // SAFETY: called once on a freshly constructed value, on this thread.
        unsafe { capture.init(&attrs)? };
        tracing::info!(
            window,
            width,
            height,
            depth = attrs.depth,
            "capturing window"
        );
        Ok(capture)
    }

    pub fn size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    unsafe fn init(&mut self, attrs: &xlib::XWindowAttributes) -> Result<(), CaptureError> {
        let dpy = self.conn.display;
        let screen = self.conn.screen();
        // SAFETY (whole function): standard XComposite/GLX/GL/CUDA setup on a valid display.
        // Each created object is stored in `self` immediately so `Drop` releases it on error.
        unsafe {
            // Keep the window's contents in an off-screen pixmap.
            (self.composite.redirect_window)(dpy, self.window, COMPOSITE_REDIRECT_AUTOMATIC);
            self.conn.check("XCompositeRedirectWindow")?;

            // GL context, made current on a 1x1 pbuffer.
            let ctx_attribs = [
                glx::GLX_DRAWABLE_TYPE,
                glx::GLX_PBUFFER_BIT,
                glx::GLX_RENDER_TYPE,
                glx::GLX_RGBA_BIT,
                glx::GLX_RED_SIZE,
                8,
                glx::GLX_GREEN_SIZE,
                8,
                glx::GLX_BLUE_SIZE,
                8,
                0,
            ];
            let mut count = 0;
            let configs =
                (self.glx.glXChooseFBConfig)(dpy, screen, ctx_attribs.as_ptr(), &mut count);
            if configs.is_null() || count == 0 {
                return Err(CaptureError::NoFbConfig("pbuffer"));
            }
            let ctx_config = *configs;
            (self.conn.xlib.XFree)(configs.cast());

            self.context = (self.glx.glXCreateNewContext)(
                dpy,
                ctx_config,
                glx::GLX_RGBA_TYPE,
                ptr::null_mut(),
                xlib::True,
            );
            if self.context.is_null() {
                return Err(CaptureError::Glx("glXCreateNewContext"));
            }
            let pbuffer_attribs = [glx::GLX_PBUFFER_WIDTH, 1, glx::GLX_PBUFFER_HEIGHT, 1, 0];
            self.pbuffer = (self.glx.glXCreatePbuffer)(dpy, ctx_config, pbuffer_attribs.as_ptr());
            if (self.glx.glXMakeContextCurrent)(dpy, self.pbuffer, self.pbuffer, self.context) == 0
            {
                return Err(CaptureError::Glx("glXMakeContextCurrent"));
            }

            let get_proc = self.glx.glXGetProcAddressARB;
            gl::load_with(|name| {
                let name = CString::new(name).unwrap_or_default();
                get_proc(name.as_ptr().cast()).map_or(ptr::null(), |f| f as *const _)
            });
            let lookup = |name: &[u8]| get_proc(name.as_ptr());
            let (Some(bind), Some(release)) = (
                lookup(b"glXBindTexImageEXT\0"),
                lookup(b"glXReleaseTexImageEXT\0"),
            ) else {
                return Err(CaptureError::MissingExtension(
                    "GLX_EXT_texture_from_pixmap",
                ));
            };
            self.bind_tex_image = std::mem::transmute::<unsafe extern "C" fn(), BindTexImage>(bind);
            self.release_tex_image =
                std::mem::transmute::<unsafe extern "C" fn(), ReleaseTexImage>(release);

            // Pixmap of the window, bound as a texture.
            self.pixmap = (self.composite.name_window_pixmap)(dpy, self.window);
            self.conn.check("XCompositeNameWindowPixmap")?;

            let rgba = attrs.depth == 32;
            let candidates = self.pixmap_configs(attrs)?;
            let pixmap_attribs = [
                GLX_TEXTURE_TARGET_EXT,
                GLX_TEXTURE_2D_EXT,
                GLX_TEXTURE_FORMAT_EXT,
                if rgba {
                    GLX_TEXTURE_FORMAT_RGBA_EXT
                } else {
                    GLX_TEXTURE_FORMAT_RGB_EXT
                },
                0,
            ];
            match self.bind_pixmap(self.pixmap, &candidates, &pixmap_attribs) {
                Ok(()) => tracing::debug!("binding the window pixmap directly"),
                Err(code) => {
                    tracing::info!(
                        x_error = code,
                        "window pixmap can't be bound directly; copying it each frame instead"
                    );
                    let depth = u32::try_from(attrs.depth).unwrap_or(24);
                    self.copy_pixmap = (self.conn.xlib.XCreatePixmap)(
                        dpy,
                        self.conn.root(),
                        self.width,
                        self.height,
                        depth,
                    );
                    self.copy_gc =
                        (self.conn.xlib.XCreateGC)(dpy, self.copy_pixmap, 0, ptr::null_mut());
                    self.conn.check("XCreatePixmap")?;
                    self.bind_pixmap(self.copy_pixmap, &candidates, &pixmap_attribs)
                        .map_err(|code| CaptureError::X(code, "glXCreatePixmap"))?;
                }
            }

            // Source texture (pixmap) and destination texture (shared with CUDA).
            let mut textures = [0; 2];
            gl::GenTextures(2, textures.as_mut_ptr());
            [self.src_texture, self.dst_texture] = textures;
            for texture in textures {
                gl::BindTexture(gl::TEXTURE_2D, texture);
                gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::NEAREST as i32);
                gl::TexParameteri(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::NEAREST as i32);
            }
            gl::BindTexture(gl::TEXTURE_2D, self.dst_texture);
            gl::TexStorage2D(
                gl::TEXTURE_2D,
                1,
                gl::RGBA8,
                self.width as i32,
                self.height as i32,
            );
            gl::BindTexture(gl::TEXTURE_2D, 0);

            gl::GenFramebuffers(1, &mut self.framebuffer);
            gl::BindFramebuffer(gl::FRAMEBUFFER, self.framebuffer);
            gl::FramebufferTexture2D(
                gl::FRAMEBUFFER,
                gl::COLOR_ATTACHMENT0,
                gl::TEXTURE_2D,
                self.dst_texture,
                0,
            );
            let status = gl::CheckFramebufferStatus(gl::FRAMEBUFFER);
            gl::BindFramebuffer(gl::FRAMEBUFFER, 0);
            if status != gl::FRAMEBUFFER_COMPLETE {
                return Err(CaptureError::Gl(format!(
                    "framebuffer incomplete: 0x{status:04x}"
                )));
            }

            self.program = link_program(VERTEX_SHADER, FRAGMENT_SHADER)?;
            gl::UseProgram(self.program);
            gl::Uniform1i(
                gl::GetUniformLocation(self.program, c"window_texture".as_ptr()),
                0,
            );
            gl::Uniform1i(
                gl::GetUniformLocation(self.program, c"flip_y".as_ptr()),
                i32::from(!self.y_inverted),
            );
            gl::UseProgram(0);
            gl::GenVertexArrays(1, &mut self.vertex_array);
            gl_check("texture setup")?;

            let (cuda, dst) = (self.cuda, self.dst_texture);
            let mut resource: CuGraphicsResource = ptr::null_mut();
            cuda.with_context(self.cuda_ctx, || {
                cuda.check(
                    "cuGraphicsGLRegisterImage",
                    (cuda.graphics_gl_register_image)(
                        &mut resource,
                        dst,
                        gl::TEXTURE_2D,
                        CU_GRAPHICS_REGISTER_FLAGS_READ_ONLY,
                    ),
                )
            })?;
            self.resource = resource;
        }
        Ok(())
    }

    /// Create the GLX pixmap for `pixmap`, trying each config. Returns the last X error code.
    unsafe fn bind_pixmap(
        &mut self,
        pixmap: xlib::Pixmap,
        candidates: &[(GLXFBConfig, xlib::VisualID)],
        attribs: &[c_int],
    ) -> Result<(), u8> {
        let dpy = self.conn.display;
        let mut last_error = 0;
        for (i, &(config, visual)) in candidates.iter().enumerate() {
            // SAFETY: valid display, config and pixmap; errors are caught by our handler.
            let glx_pixmap =
                unsafe { (self.glx.glXCreatePixmap)(dpy, config, pixmap, attribs.as_ptr()) };
            match self.conn.sync_error() {
                None if glx_pixmap != 0 => {
                    let mut inverted = 0;
                    // SAFETY: valid display and config.
                    unsafe {
                        (self.glx.glXGetFBConfigAttrib)(
                            dpy,
                            config,
                            GLX_Y_INVERTED_EXT,
                            &mut inverted,
                        )
                    };
                    self.glx_pixmap = glx_pixmap;
                    self.y_inverted = inverted != 0;
                    tracing::debug!(
                        candidate = i,
                        visual = format!("0x{visual:x}"),
                        "glXCreatePixmap succeeded"
                    );
                    return Ok(());
                }
                code => last_error = code.unwrap_or(0),
            }
        }
        Err(last_error)
    }

    /// Framebuffer configs that can bind a pixmap of the window's depth as a 2D texture, with
    /// their visual IDs. Configs matching the window's own visual come first.
    unsafe fn pixmap_configs(
        &self,
        attrs: &xlib::XWindowAttributes,
    ) -> Result<Vec<(GLXFBConfig, xlib::VisualID)>, CaptureError> {
        let dpy = self.conn.display;
        let visual_id = if attrs.visual.is_null() {
            0
        } else {
            // SAFETY: visual comes from XGetWindowAttributes and is owned by Xlib.
            unsafe { (self.conn.xlib.XVisualIDFromVisual)(attrs.visual) }
        };
        let bind_attr = if attrs.depth == 32 {
            GLX_BIND_TO_TEXTURE_RGBA_EXT
        } else {
            GLX_BIND_TO_TEXTURE_RGB_EXT
        };
        let mut count = 0;
        // SAFETY: valid display; the returned array is freed with XFree below.
        let configs = unsafe { (self.glx.glXGetFBConfigs)(dpy, self.conn.screen(), &mut count) };
        if configs.is_null() {
            return Err(CaptureError::NoFbConfig("pixmap"));
        }
        // SAFETY: glXGetFBConfigs returned `count` configs.
        let all = unsafe { std::slice::from_raw_parts(configs, count.max(0) as usize) };
        let attr = |config: GLXFBConfig, name: c_int| {
            let mut value = 0;
            // SAFETY: valid display and config.
            unsafe { (self.glx.glXGetFBConfigAttrib)(dpy, config, name, &mut value) };
            value
        };
        let mut candidates = Vec::new();
        for &config in all {
            if attr(config, glx::GLX_DRAWABLE_TYPE) & glx::GLX_PIXMAP_BIT == 0
                || attr(config, bind_attr) == 0
                || attr(config, GLX_BIND_TO_TEXTURE_TARGETS_EXT) & GLX_TEXTURE_2D_BIT_EXT == 0
            {
                continue;
            }
            // SAFETY: valid display and config; the XVisualInfo is freed right after reading.
            let (depth, vid) = unsafe {
                let info = (self.glx.glXGetVisualFromFBConfig)(dpy, config);
                if info.is_null() {
                    continue;
                }
                let v = ((*info).depth, (*info).visualid);
                (self.conn.xlib.XFree)(info.cast());
                v
            };
            if depth == attrs.depth {
                candidates.push((config, vid));
            }
        }
        // SAFETY: freeing the array returned by glXGetFBConfigs.
        unsafe { (self.conn.xlib.XFree)(configs.cast()) };
        candidates.sort_by_key(|&(_, vid)| vid != visual_id);
        tracing::debug!(
            window_visual = format!("0x{visual_id:x}"),
            depth = attrs.depth,
            candidates = candidates.len(),
            "texture-from-pixmap configs"
        );
        if candidates.is_empty() {
            return Err(CaptureError::NoFbConfig(
                "no texture-from-pixmap config for the window depth",
            ));
        }
        Ok(candidates)
    }

    /// Copy the current window contents into `dst` (an RGB0 image of exactly `size()`).
    pub fn capture(&mut self, dst: CudaDestination) -> Result<CaptureStatus, CaptureError> {
        let attrs = self.conn.window_attributes(self.window)?;
        if attrs.map_state != xlib::IsViewable {
            return Err(CaptureError::WindowNotViewable);
        }
        let (w, h) = (attrs.width.max(1) as u32, attrs.height.max(1) as u32);
        if (w, h) != (self.width, self.height) {
            return Ok(CaptureStatus::Resized {
                width: w,
                height: h,
            });
        }

        let dpy = self.conn.display;
        let (width, height) = (self.width as i32, self.height as i32);
        // SAFETY: our GL context is current on this thread (we never make another current);
        // all GL objects were created in `init`.
        unsafe {
            if self.copy_pixmap != 0 {
                // Server-side GPU copy of the window into our bindable pixmap.
                (self.conn.xlib.XCopyArea)(
                    dpy,
                    self.pixmap,
                    self.copy_pixmap,
                    self.copy_gc,
                    0,
                    0,
                    self.width,
                    self.height,
                    0,
                    0,
                );
                (self.conn.xlib.XSync)(dpy, xlib::False);
            }
            // Re-binding picks up the latest window contents.
            gl::BindTexture(gl::TEXTURE_2D, self.src_texture);
            (self.bind_tex_image)(dpy, self.glx_pixmap, GLX_FRONT_EXT, ptr::null());

            // Draw the window texture into the destination texture. Output row 0 is the top of
            // the window (`flip_y` handles pixmaps whose texture origin is at the bottom).
            gl::BindFramebuffer(gl::FRAMEBUFFER, self.framebuffer);
            gl::Viewport(0, 0, width, height);
            gl::UseProgram(self.program);
            gl::BindVertexArray(self.vertex_array);
            gl::DrawArrays(gl::TRIANGLES, 0, 3);
            gl::BindVertexArray(0);
            gl::UseProgram(0);
            gl::BindFramebuffer(gl::FRAMEBUFFER, 0);

            (self.release_tex_image)(dpy, self.glx_pixmap, GLX_FRONT_EXT);
            gl::BindTexture(gl::TEXTURE_2D, 0);
        }
        gl_check("draw")?;
        if let Some(code) = self.conn.sync_error() {
            // The pixmap becomes invalid when the window is resized, unmapped or destroyed.
            tracing::debug!(code, "X error while capturing");
            return Err(CaptureError::WindowGone);
        }

        let cuda = self.cuda;
        let mut resource = self.resource;
        let row_bytes = self.width as usize * 4;
        let rows = self.height as usize;
        cuda.with_context(dst.context, || {
            // SAFETY: resource is registered in this context; the mapped array is only used
            // between map and unmap; dst points to a frame of at least `rows * pitch` bytes.
            unsafe {
                cuda.check(
                    "cuGraphicsMapResources",
                    (cuda.graphics_map_resources)(1, &mut resource, ptr::null_mut()),
                )?;
                let mut array: CuArray = ptr::null_mut();
                let copied = cuda
                    .check(
                        "cuGraphicsSubResourceGetMappedArray",
                        (cuda.graphics_sub_resource_get_mapped_array)(&mut array, resource, 0, 0),
                    )
                    .and_then(|()| {
                        let copy = CudaMemcpy2D {
                            src_memory_type: CU_MEMORYTYPE_ARRAY,
                            src_array: array as usize,
                            dst_memory_type: CU_MEMORYTYPE_DEVICE,
                            dst_device: dst.device_ptr,
                            dst_pitch: dst.pitch,
                            width_in_bytes: row_bytes,
                            height: rows,
                            ..Default::default()
                        };
                        cuda.check("cuMemcpy2D", (cuda.memcpy_2d)(&copy))
                    });
                let unmapped = cuda.check(
                    "cuGraphicsUnmapResources",
                    (cuda.graphics_unmap_resources)(1, &mut resource, ptr::null_mut()),
                );
                copied.and(unmapped)
            }
        })?;
        Ok(CaptureStatus::Captured)
    }
}

/// Compile and link a shader program. Requires a current GL context.
fn link_program(vertex: &str, fragment: &str) -> Result<c_uint, CaptureError> {
    // SAFETY: a GL context is current; sources are NUL-terminated CStrings that outlive the calls.
    unsafe {
        let compile = |kind, source: &str| -> Result<c_uint, CaptureError> {
            let shader = gl::CreateShader(kind);
            let source = CString::new(source).unwrap_or_default();
            gl::ShaderSource(shader, 1, &source.as_ptr(), ptr::null());
            gl::CompileShader(shader);
            let mut ok = 0;
            gl::GetShaderiv(shader, gl::COMPILE_STATUS, &mut ok);
            if ok == 0 {
                let log = info_log(shader, gl::GetShaderInfoLog);
                gl::DeleteShader(shader);
                return Err(CaptureError::Gl(format!("shader compile failed: {log}")));
            }
            Ok(shader)
        };
        let vs = compile(gl::VERTEX_SHADER, vertex)?;
        let fs = match compile(gl::FRAGMENT_SHADER, fragment) {
            Ok(fs) => fs,
            Err(e) => {
                gl::DeleteShader(vs);
                return Err(e);
            }
        };
        let program = gl::CreateProgram();
        gl::AttachShader(program, vs);
        gl::AttachShader(program, fs);
        gl::LinkProgram(program);
        gl::DeleteShader(vs);
        gl::DeleteShader(fs);
        let mut ok = 0;
        gl::GetProgramiv(program, gl::LINK_STATUS, &mut ok);
        if ok == 0 {
            let log = info_log(program, gl::GetProgramInfoLog);
            gl::DeleteProgram(program);
            return Err(CaptureError::Gl(format!("shader link failed: {log}")));
        }
        Ok(program)
    }
}

unsafe fn info_log(
    object: c_uint,
    get: unsafe fn(c_uint, i32, *mut i32, *mut std::ffi::c_char),
) -> String {
    let mut buf = vec![0u8; 1024];
    let mut len = 0;
    // SAFETY: buffer is large enough for the reported length (GL truncates to buf size).
    unsafe { get(object, buf.len() as i32, &mut len, buf.as_mut_ptr().cast()) };
    buf.truncate(len.max(0) as usize);
    String::from_utf8_lossy(&buf).into_owned()
}

unsafe extern "C" fn noop_bind(_: *mut Display, _: glx::GLXDrawable, _: c_int, _: *const c_int) {}
unsafe extern "C" fn noop_release(_: *mut Display, _: glx::GLXDrawable, _: c_int) {}

fn gl_check(what: &str) -> Result<(), CaptureError> {
    // SAFETY: a GL context is current whenever this is called.
    let error = unsafe { gl::GetError() };
    if error == gl::NO_ERROR {
        Ok(())
    } else {
        Err(CaptureError::Gl(format!("{what}: 0x{error:04x}")))
    }
}

impl Drop for X11WindowCapture {
    fn drop(&mut self) {
        let dpy = self.conn.display;
        // SAFETY: releases only what `init` created (zero/NULL handles are skipped), in reverse
        // order, with our context current. X errors (e.g. window already gone) are swallowed by
        // our error handler.
        unsafe {
            if !self.resource.is_null() {
                let (cuda, resource) = (self.cuda, self.resource);
                let _ = cuda.with_context(self.cuda_ctx, || {
                    cuda.check(
                        "cuGraphicsUnregisterResource",
                        (cuda.graphics_unregister_resource)(resource),
                    )
                });
            }
            if !self.context.is_null() {
                if self.framebuffer != 0 {
                    gl::DeleteFramebuffers(1, &self.framebuffer);
                }
                if self.vertex_array != 0 {
                    gl::DeleteVertexArrays(1, &self.vertex_array);
                }
                if self.program != 0 {
                    gl::DeleteProgram(self.program);
                }
                let textures = [self.src_texture, self.dst_texture];
                if textures.iter().any(|&t| t != 0) {
                    gl::DeleteTextures(2, textures.as_ptr());
                }
            }
            if self.glx_pixmap != 0 {
                (self.glx.glXDestroyPixmap)(dpy, self.glx_pixmap);
            }
            if self.pixmap != 0 {
                (self.conn.xlib.XFreePixmap)(dpy, self.pixmap);
            }
            if !self.copy_gc.is_null() {
                (self.conn.xlib.XFreeGC)(dpy, self.copy_gc);
            }
            if self.copy_pixmap != 0 {
                (self.conn.xlib.XFreePixmap)(dpy, self.copy_pixmap);
            }
            (self.composite.unredirect_window)(dpy, self.window, COMPOSITE_REDIRECT_AUTOMATIC);
            if !self.context.is_null() {
                (self.glx.glXMakeContextCurrent)(dpy, 0, 0, ptr::null_mut());
                (self.glx.glXDestroyContext)(dpy, self.context);
            }
            if self.pbuffer != 0 {
                (self.glx.glXDestroyPbuffer)(dpy, self.pbuffer);
            }
            let _ = self.conn.sync_error();
        }
    }
}
