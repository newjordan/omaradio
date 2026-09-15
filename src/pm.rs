//! projectM backend: the real MilkDrop engine, loaded at runtime.
//!
//! Optional. Needs libprojectM-4 (LGPL, dynamically loaded, never linked),
//! EGL with a GPU that can make a surfaceless OpenGL 3.3 context, and a
//! preset directory. `scripts/setup-milkdrop.sh` provides all three. Without
//! them omaradio simply keeps its built-in presets.
//!
//! Rendering happens into an EGL pbuffer; the frame is read back as RGB and
//! blitted with the Kitty graphics protocol like every other image here.

use libloading::Library;
use std::ffi::{c_char, c_void, CStr, CString};
use std::path::{Path, PathBuf};

pub const W: u32 = 320;
pub const H: u32 = 180;

// EGL
type EGLDisplay = *mut c_void;
type EGLConfig = *mut c_void;
type EGLSurface = *mut c_void;
type EGLContext = *mut c_void;
type EGLBoolean = u32;
const EGL_SURFACE_TYPE: i32 = 0x3033;
const EGL_PBUFFER_BIT: i32 = 0x0001;
const EGL_RENDERABLE_TYPE: i32 = 0x3040;
const EGL_OPENGL_BIT: i32 = 0x0008;
const EGL_RED_SIZE: i32 = 0x3024;
const EGL_GREEN_SIZE: i32 = 0x3023;
const EGL_BLUE_SIZE: i32 = 0x3022;
const EGL_ALPHA_SIZE: i32 = 0x3021;
const EGL_DEPTH_SIZE: i32 = 0x3025;
const EGL_NONE: i32 = 0x3038;
const EGL_WIDTH: i32 = 0x3057;
const EGL_HEIGHT: i32 = 0x3056;
const EGL_OPENGL_API: u32 = 0x30A2;
const EGL_CONTEXT_MAJOR_VERSION: i32 = 0x3098;
const EGL_CONTEXT_MINOR_VERSION: i32 = 0x30FB;
const EGL_CONTEXT_OPENGL_PROFILE_MASK: i32 = 0x30FD;
const EGL_CONTEXT_OPENGL_CORE_PROFILE_BIT: i32 = 0x0001;
const EGL_PLATFORM_DEVICE_EXT: u32 = 0x313F;
const EGL_PLATFORM_SURFACELESS_MESA: u32 = 0x31DD;
// GL
const GL_RGB: u32 = 0x1907;
const GL_UNSIGNED_BYTE: u32 = 0x1401;
const GL_PACK_ALIGNMENT: u32 = 0x0D05;
const GL_RENDERER: u32 = 0x1F01;
// projectM
const PROJECTM_MONO: i32 = 1;

type FnGetProcAddress = unsafe extern "C" fn(*const c_char) -> *const c_void;
type FnGetDisplay = unsafe extern "C" fn(*mut c_void) -> EGLDisplay;
type FnGetPlatformDisplayExt = unsafe extern "C" fn(u32, *mut c_void, *const i32) -> EGLDisplay;
type FnQueryDevicesExt = unsafe extern "C" fn(i32, *mut *mut c_void, *mut i32) -> EGLBoolean;
type FnInitialize = unsafe extern "C" fn(EGLDisplay, *mut i32, *mut i32) -> EGLBoolean;
type FnBindApi = unsafe extern "C" fn(u32) -> EGLBoolean;
type FnChooseConfig = unsafe extern "C" fn(EGLDisplay, *const i32, *mut EGLConfig, i32, *mut i32) -> EGLBoolean;
type FnCreatePbuffer = unsafe extern "C" fn(EGLDisplay, EGLConfig, *const i32) -> EGLSurface;
type FnCreateContext = unsafe extern "C" fn(EGLDisplay, EGLConfig, EGLContext, *const i32) -> EGLContext;
type FnMakeCurrent = unsafe extern "C" fn(EGLDisplay, EGLSurface, EGLSurface, EGLContext) -> EGLBoolean;
type FnDestroyContext = unsafe extern "C" fn(EGLDisplay, EGLContext) -> EGLBoolean;
type FnDestroySurface = unsafe extern "C" fn(EGLDisplay, EGLSurface) -> EGLBoolean;
type FnTerminate = unsafe extern "C" fn(EGLDisplay) -> EGLBoolean;
type FnGetError = unsafe extern "C" fn() -> i32;

type FnGlReadPixels = unsafe extern "C" fn(i32, i32, i32, i32, u32, u32, *mut c_void);
type FnGlPixelStorei = unsafe extern "C" fn(u32, i32);
type FnGlGetString = unsafe extern "C" fn(u32) -> *const u8;
type FnGlFinish = unsafe extern "C" fn();

type Handle = *mut c_void;
type FnPmCreate = unsafe extern "C" fn() -> Handle;
type FnPmDestroy = unsafe extern "C" fn(Handle);
type FnPmSetSize = unsafe extern "C" fn(Handle, usize, usize);
type FnPmSetI32 = unsafe extern "C" fn(Handle, i32);
type FnPmSetBool = unsafe extern "C" fn(Handle, bool);
type FnPmSetF32 = unsafe extern "C" fn(Handle, f32);
type FnPmSetF64 = unsafe extern "C" fn(Handle, f64);
type FnPmTexPaths = unsafe extern "C" fn(Handle, *const *const c_char, usize);
type FnPmLoadFile = unsafe extern "C" fn(Handle, *const c_char, bool);
type FnPmRender = unsafe extern "C" fn(Handle);
type FnPmPcmF32 = unsafe extern "C" fn(Handle, *const f32, u32, i32);
type FnPmVersion = unsafe extern "C" fn(*mut i32, *mut i32, *mut i32);

struct Egl {
    _lib: Library,
    make_current: FnMakeCurrent,
    destroy_context: FnDestroyContext,
    destroy_surface: FnDestroySurface,
    terminate: FnTerminate,
    dpy: EGLDisplay,
    ctx: EGLContext,
    surf: EGLSurface,
}

struct Gl {
    _lib: Option<Library>,
    read_pixels: FnGlReadPixels,
    pixel_storei: FnGlPixelStorei,
    finish: FnGlFinish,
}

struct Pm {
    _lib: Library,
    destroy: FnPmDestroy,
    load_file: FnPmLoadFile,
    render: FnPmRender,
    pcm_f32: FnPmPcmF32,
    handle: Handle,
}

pub struct ProjectM {
    egl: Egl,
    gl: Gl,
    pm: Pm,
    presets: Vec<PathBuf>,
    idx: usize,
    name: String,
    raw: Vec<u8>,
    frame: Vec<u8>,
    pub renderer: String,
    pub version: String,
    pub preset_dir: PathBuf,
}

fn home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

fn data_dir() -> PathBuf {
    if let Some(d) = std::env::var_os("XDG_DATA_HOME") {
        return PathBuf::from(d).join("omaradio");
    }
    home().join(".local/share/omaradio")
}

/// Candidate library paths, first hit wins.
fn lib_candidates() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(p) = std::env::var_os("OMARADIO_PROJECTM_LIB") {
        v.push(PathBuf::from(p));
    }
    for name in ["libprojectM-4.so.4", "libprojectM-4.so"] {
        v.push(home().join(".local/lib").join(name));
        v.push(home().join(".local/lib64").join(name));
        v.push(PathBuf::from("/usr/local/lib").join(name));
        v.push(PathBuf::from(name)); // system search path
    }
    v
}

pub fn preset_dir_candidates() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(p) = std::env::var_os("OMARADIO_MILK_PRESETS") {
        v.push(PathBuf::from(p));
    }
    v.push(data_dir().join("cream-of-the-crop"));
    v.push(data_dir().join("presets"));
    v.push(PathBuf::from("/usr/share/projectM/presets"));
    v.push(PathBuf::from("/usr/local/share/projectM/presets"));
    v
}

fn texture_dirs(preset_dir: &Path) -> Vec<PathBuf> {
    let mut v = vec![data_dir().join("textures"), PathBuf::from("/usr/share/projectM/textures")];
    v.push(preset_dir.to_path_buf());
    v.into_iter().filter(|p| p.is_dir()).collect()
}

/// Every `.milk` under `dir`, sorted, so `M` walks the collection in a
/// stable order and a preset name means the same thing next launch.
pub fn scan_presets(dir: &Path) -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
        if depth > 6 {
            return;
        }
        let Ok(rd) = std::fs::read_dir(dir) else { return };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, out, depth + 1);
            } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("milk")) {
                out.push(p);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, &mut out, 0);
    out.sort();
    out
}

/// What `M` would do right now, without touching the GPU.
pub fn availability() -> Result<(PathBuf, PathBuf, usize), String> {
    let lib = lib_candidates()
        .into_iter()
        .find(|p| p.is_absolute() && p.is_file())
        .ok_or_else(|| "libprojectM-4 not found — run scripts/setup-milkdrop.sh".to_string())?;
    let dir = preset_dir_candidates()
        .into_iter()
        .find(|p| p.is_dir())
        .ok_or_else(|| "no MilkDrop preset directory — run scripts/setup-milkdrop.sh".to_string())?;
    let n = scan_presets(&dir).len();
    if n == 0 {
        return Err(format!("{} has no .milk presets", dir.display()));
    }
    Ok((lib, dir, n))
}

macro_rules! sym {
    ($lib:expr, $ty:ty, $name:literal) => {{
        let s: libloading::Symbol<$ty> = unsafe { $lib.get(concat!($name, "\0").as_bytes()) }
            .map_err(|e| format!("{}: {e}", $name))?;
        *s
    }};
}

impl ProjectM {
    pub fn new(width: u32, height: u32) -> Result<Self, String> {
        let (lib_path, preset_dir, _) = availability()?;
        let presets = scan_presets(&preset_dir);
        let egl = Egl::new(width, height)?;
        let gl = Gl::new(&egl)?;
        let pm = Pm::new(&lib_path, width, height, &preset_dir)?;
        let renderer = gl.renderer();
        let version = pm.version();
        let start = (std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as usize)
            .unwrap_or(0))
            % presets.len().max(1);
        let mut me = Self {
            egl,
            gl,
            pm,
            presets,
            idx: start,
            name: String::new(),
            raw: vec![0; (width * height * 3) as usize],
            frame: vec![0; (width * height * 3) as usize],
            renderer,
            version,
            preset_dir,
        };
        me.load(me.idx, false);
        Ok(me)
    }

    pub fn count(&self) -> usize {
        self.presets.len()
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn index(&self) -> usize {
        self.idx
    }

    fn load(&mut self, idx: usize, smooth: bool) {
        if self.presets.is_empty() {
            return;
        }
        self.idx = idx % self.presets.len();
        let path = &self.presets[self.idx];
        self.name = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        if let Ok(c) = CString::new(path.to_string_lossy().as_bytes()) {
            unsafe { (self.pm.load_file)(self.pm.handle, c.as_ptr(), smooth) };
        }
    }

    pub fn next(&mut self) -> &str {
        self.load(self.idx + 1, true);
        &self.name
    }

    pub fn prev(&mut self) -> &str {
        let n = self.presets.len().max(1);
        self.load((self.idx + n - 1) % n, true);
        &self.name
    }

    /// Jump to the first preset whose file name contains `needle`.
    pub fn find(&mut self, needle: &str) -> Option<&str> {
        let n = needle.to_ascii_lowercase();
        let hit = self
            .presets
            .iter()
            .position(|p| p.file_name().map(|f| f.to_string_lossy().to_ascii_lowercase().contains(&n)).unwrap_or(false))?;
        self.load(hit, true);
        Some(&self.name)
    }

    pub fn feed(&mut self, pcm: &[f32]) {
        if pcm.is_empty() {
            return;
        }
        // projectM keeps a short ring; hand it the newest chunk only.
        let take = pcm.len().min(2048);
        let s = &pcm[pcm.len() - take..];
        unsafe { (self.pm.pcm_f32)(self.pm.handle, s.as_ptr(), take as u32, PROJECTM_MONO) };
    }

    /// Render one frame and return it top-down RGB, W×H.
    pub fn render(&mut self) -> &[u8] {
        let (w, h) = (W as i32, H as i32);
        unsafe {
            (self.egl.make_current)(self.egl.dpy, self.egl.surf, self.egl.surf, self.egl.ctx);
            (self.pm.render)(self.pm.handle);
            (self.gl.finish)();
            (self.gl.pixel_storei)(GL_PACK_ALIGNMENT, 1);
            (self.gl.read_pixels)(0, 0, w, h, GL_RGB, GL_UNSIGNED_BYTE, self.raw.as_mut_ptr() as *mut c_void);
        }
        let row = (W * 3) as usize;
        for y in 0..H as usize {
            let src = &self.raw[(H as usize - 1 - y) * row..(H as usize - y) * row];
            self.frame[y * row..(y + 1) * row].copy_from_slice(src);
        }
        &self.frame
    }
}

impl Drop for ProjectM {
    fn drop(&mut self) {
        unsafe {
            (self.egl.make_current)(self.egl.dpy, self.egl.surf, self.egl.surf, self.egl.ctx);
            (self.pm.destroy)(self.pm.handle);
            (self.egl.make_current)(self.egl.dpy, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut());
            (self.egl.destroy_context)(self.egl.dpy, self.egl.ctx);
            (self.egl.destroy_surface)(self.egl.dpy, self.egl.surf);
            (self.egl.terminate)(self.egl.dpy);
        }
    }
}

impl Egl {
    fn new(width: u32, height: u32) -> Result<Self, String> {
        let lib = unsafe { Library::new("libEGL.so.1") }.map_err(|e| format!("libEGL.so.1: {e}"))?;
        let get_proc: FnGetProcAddress = sym!(lib, FnGetProcAddress, "eglGetProcAddress");
        let get_display: FnGetDisplay = sym!(lib, FnGetDisplay, "eglGetDisplay");
        let initialize: FnInitialize = sym!(lib, FnInitialize, "eglInitialize");
        let bind_api: FnBindApi = sym!(lib, FnBindApi, "eglBindAPI");
        let choose_config: FnChooseConfig = sym!(lib, FnChooseConfig, "eglChooseConfig");
        let create_pbuffer: FnCreatePbuffer = sym!(lib, FnCreatePbuffer, "eglCreatePbufferSurface");
        let create_context: FnCreateContext = sym!(lib, FnCreateContext, "eglCreateContext");
        let make_current: FnMakeCurrent = sym!(lib, FnMakeCurrent, "eglMakeCurrent");
        let destroy_context: FnDestroyContext = sym!(lib, FnDestroyContext, "eglDestroyContext");
        let destroy_surface: FnDestroySurface = sym!(lib, FnDestroySurface, "eglDestroySurface");
        let terminate: FnTerminate = sym!(lib, FnTerminate, "eglTerminate");
        let get_error: FnGetError = sym!(lib, FnGetError, "eglGetError");

        let ext = |name: &str| -> *const c_void {
            let c = CString::new(name).unwrap();
            unsafe { get_proc(c.as_ptr()) }
        };
        let mut tried = Vec::new();
        let mut dpy: EGLDisplay = std::ptr::null_mut();

        // Displays to try: explicit device index, every EGL device, Mesa
        // surfaceless, then the default display.
        let platform_display = ext("eglGetPlatformDisplayEXT");
        let query_devices = ext("eglQueryDevicesEXT");
        let init_ok = |d: EGLDisplay| -> bool {
            if d.is_null() {
                return false;
            }
            let (mut maj, mut min) = (0i32, 0i32);
            unsafe { initialize(d, &mut maj, &mut min) == 1 }
        };
        if !platform_display.is_null() && !query_devices.is_null() {
            let gpd: FnGetPlatformDisplayExt = unsafe { std::mem::transmute(platform_display) };
            let qd: FnQueryDevicesExt = unsafe { std::mem::transmute(query_devices) };
            let mut devs: [*mut c_void; 16] = [std::ptr::null_mut(); 16];
            let mut n = 0i32;
            if unsafe { qd(16, devs.as_mut_ptr(), &mut n) } == 1 && n > 0 {
                let want = std::env::var("OMARADIO_EGL_DEVICE").ok().and_then(|s| s.parse::<usize>().ok());
                let order: Vec<usize> = match want {
                    Some(i) if i < n as usize => vec![i],
                    _ => (0..n as usize).collect(),
                };
                for i in order {
                    let d = unsafe { gpd(EGL_PLATFORM_DEVICE_EXT, devs[i], std::ptr::null()) };
                    if init_ok(d) {
                        dpy = d;
                        break;
                    }
                    tried.push(format!("device {i}"));
                }
            }
            if dpy.is_null() {
                let d = unsafe { gpd(EGL_PLATFORM_SURFACELESS_MESA, std::ptr::null_mut(), std::ptr::null()) };
                if init_ok(d) {
                    dpy = d;
                } else {
                    tried.push("surfaceless".into());
                }
            }
        }
        if dpy.is_null() {
            let d = unsafe { get_display(std::ptr::null_mut()) };
            if init_ok(d) {
                dpy = d;
            } else {
                tried.push("default".into());
            }
        }
        if dpy.is_null() {
            return Err(format!("no usable EGL display (tried {})", tried.join(", ")));
        }
        if unsafe { bind_api(EGL_OPENGL_API) } != 1 {
            return Err("EGL cannot bind desktop OpenGL".into());
        }
        let cfg_attribs = [
            EGL_SURFACE_TYPE, EGL_PBUFFER_BIT,
            EGL_RENDERABLE_TYPE, EGL_OPENGL_BIT,
            EGL_RED_SIZE, 8, EGL_GREEN_SIZE, 8, EGL_BLUE_SIZE, 8, EGL_ALPHA_SIZE, 8,
            EGL_DEPTH_SIZE, 0,
            EGL_NONE,
        ];
        let mut cfg: EGLConfig = std::ptr::null_mut();
        let mut ncfg = 0i32;
        if unsafe { choose_config(dpy, cfg_attribs.as_ptr(), &mut cfg, 1, &mut ncfg) } != 1 || ncfg < 1 {
            return Err(format!("no EGL config with pbuffer + OpenGL (0x{:x})", unsafe { get_error() }));
        }
        let pb_attribs = [EGL_WIDTH, width as i32, EGL_HEIGHT, height as i32, EGL_NONE];
        let surf = unsafe { create_pbuffer(dpy, cfg, pb_attribs.as_ptr()) };
        if surf.is_null() {
            return Err(format!("eglCreatePbufferSurface failed (0x{:x})", unsafe { get_error() }));
        }
        let ctx_attribs = [
            EGL_CONTEXT_MAJOR_VERSION, 3, EGL_CONTEXT_MINOR_VERSION, 3,
            EGL_CONTEXT_OPENGL_PROFILE_MASK, EGL_CONTEXT_OPENGL_CORE_PROFILE_BIT,
            EGL_NONE,
        ];
        let ctx = unsafe { create_context(dpy, cfg, std::ptr::null_mut(), ctx_attribs.as_ptr()) };
        if ctx.is_null() {
            unsafe { destroy_surface(dpy, surf) };
            return Err(format!("no OpenGL 3.3 core context (0x{:x})", unsafe { get_error() }));
        }
        if unsafe { make_current(dpy, surf, surf, ctx) } != 1 {
            return Err(format!("eglMakeCurrent failed (0x{:x})", unsafe { get_error() }));
        }
        Ok(Self { _lib: lib, make_current, destroy_context, destroy_surface, terminate, dpy, ctx, surf })
    }
}

impl Gl {
    fn new(egl: &Egl) -> Result<Self, String> {
        // glvnd's libGL dispatches to whatever context is current, EGL included.
        let lib = unsafe { Library::new("libGL.so.1") }
            .or_else(|_| unsafe { Library::new("libOpenGL.so.0") })
            .map_err(|e| format!("libGL.so.1: {e}"))?;
        let _ = egl;
        let read_pixels: FnGlReadPixels = sym!(lib, FnGlReadPixels, "glReadPixels");
        let pixel_storei: FnGlPixelStorei = sym!(lib, FnGlPixelStorei, "glPixelStorei");
        let finish: FnGlFinish = sym!(lib, FnGlFinish, "glFinish");
        let get_string: FnGlGetString = sym!(lib, FnGlGetString, "glGetString");
        let me = Self { _lib: Some(lib), read_pixels, pixel_storei, finish };
        let p = unsafe { get_string(GL_RENDERER) };
        if p.is_null() {
            return Err("GL context is not current (glGetString returned null)".into());
        }
        let _ = me.renderer_from(p);
        Ok(me)
    }

    fn renderer_from(&self, p: *const u8) -> String {
        if p.is_null() {
            return "unknown".into();
        }
        unsafe { CStr::from_ptr(p as *const c_char) }.to_string_lossy().into_owned()
    }

    fn renderer(&self) -> String {
        let lib = self._lib.as_ref().unwrap();
        let get_string: Result<libloading::Symbol<FnGlGetString>, _> = unsafe { lib.get(b"glGetString\0") };
        match get_string {
            Ok(f) => self.renderer_from(unsafe { f(GL_RENDERER) }),
            Err(_) => "unknown".into(),
        }
    }
}

impl Pm {
    fn new(lib_path: &Path, width: u32, height: u32, preset_dir: &Path) -> Result<Self, String> {
        let lib = unsafe { Library::new(lib_path) }.map_err(|e| format!("{}: {e}", lib_path.display()))?;
        let create: FnPmCreate = sym!(lib, FnPmCreate, "projectm_create");
        let destroy: FnPmDestroy = sym!(lib, FnPmDestroy, "projectm_destroy");
        let set_window_size: FnPmSetSize = sym!(lib, FnPmSetSize, "projectm_set_window_size");
        let set_mesh_size: FnPmSetSize = sym!(lib, FnPmSetSize, "projectm_set_mesh_size");
        let set_fps: FnPmSetI32 = sym!(lib, FnPmSetI32, "projectm_set_fps");
        let set_locked: FnPmSetBool = sym!(lib, FnPmSetBool, "projectm_set_preset_locked");
        let set_aspect: FnPmSetBool = sym!(lib, FnPmSetBool, "projectm_set_aspect_correction");
        let set_beat: FnPmSetF32 = sym!(lib, FnPmSetF32, "projectm_set_beat_sensitivity");
        let set_soft_cut: FnPmSetF64 = sym!(lib, FnPmSetF64, "projectm_set_soft_cut_duration");
        let set_tex_paths: FnPmTexPaths = sym!(lib, FnPmTexPaths, "projectm_set_texture_search_paths");
        let load_file: FnPmLoadFile = sym!(lib, FnPmLoadFile, "projectm_load_preset_file");
        let render: FnPmRender = sym!(lib, FnPmRender, "projectm_opengl_render_frame");
        let pcm_f32: FnPmPcmF32 = sym!(lib, FnPmPcmF32, "projectm_pcm_add_float");
        let handle = unsafe { create() };
        if handle.is_null() {
            return Err("projectm_create returned null (no GL context?)".into());
        }
        unsafe {
            set_window_size(handle, width as usize, height as usize);
            set_mesh_size(handle, 48, 27);
            set_fps(handle, 30);
            set_locked(handle, true);
            set_aspect(handle, true);
            set_beat(handle, 1.0);
            set_soft_cut(handle, 2.0);
        }
        let dirs = texture_dirs(preset_dir);
        let cstrs: Vec<CString> = dirs.iter().filter_map(|d| CString::new(d.to_string_lossy().as_bytes()).ok()).collect();
        let ptrs: Vec<*const c_char> = cstrs.iter().map(|c| c.as_ptr()).collect();
        if !ptrs.is_empty() {
            unsafe { set_tex_paths(handle, ptrs.as_ptr(), ptrs.len()) };
        }
        Ok(Self { _lib: lib, destroy, load_file, render, pcm_f32, handle })
    }

    fn version(&self) -> String {
        let f: Result<libloading::Symbol<FnPmVersion>, _> = unsafe { self._lib.get(b"projectm_get_version_components\0") };
        match f {
            Ok(f) => {
                let (mut a, mut b, mut c) = (0, 0, 0);
                unsafe { f(&mut a, &mut b, &mut c) };
                format!("{a}.{b}.{c}")
            }
            Err(_) => "?".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scan_finds_milk_files_recursively_and_sorted() {
        let dir = std::env::temp_dir().join(format!("omr-pm-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("b/deeper")).unwrap();
        std::fs::write(dir.join("z.milk"), "").unwrap();
        std::fs::write(dir.join("b/deeper/a.MILK"), "").unwrap();
        std::fs::write(dir.join("b/readme.txt"), "").unwrap();
        let found = scan_presets(&dir);
        let names: Vec<String> = found.iter().map(|p| p.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, vec!["a.MILK", "z.milk"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// `OMARADIO_PM_DUMP=/dir cargo test --release -- --ignored pm_dump` renders
    /// a few collection presets headlessly and writes PPMs for eyeballing.
    #[test]
    #[ignore]
    fn pm_dump() {
        let Some(dir) = std::env::var_os("OMARADIO_PM_DUMP") else { return };
        let dir = PathBuf::from(dir);
        let t0 = std::time::Instant::now();
        let mut pm = ProjectM::new(W, H).expect("projectM up");
        eprintln!("projectM {} on {} · {} presets · init {:?}", pm.version, pm.renderer, pm.count(), t0.elapsed());
        let pcm: Vec<f32> = (0..2048).map(|i| ((i as f32) * 0.05).sin() * 0.5 + ((i as f32) * 0.31).sin() * 0.2).collect();
        for shot in 0..4 {
            let t = std::time::Instant::now();
            let mut frame = Vec::new();
            for _ in 0..60 {
                pm.feed(&pcm);
                frame = pm.render().to_vec();
            }
            let per = t.elapsed().as_secs_f32() * 1000.0 / 60.0;
            let nonblack = frame.iter().filter(|&&c| c > 8).count();
            eprintln!("preset {} '{}' · {per:.2} ms/frame · {nonblack} lit bytes", pm.index(), pm.name());
            let mut out = format!("P6 {W} {H} 255\n").into_bytes();
            out.extend_from_slice(&frame);
            std::fs::write(dir.join(format!("pm-{shot}.ppm")), out).unwrap();
            pm.next();
        }
    }

    #[test]
    fn availability_reports_a_reason_when_missing() {
        std::env::set_var("OMARADIO_PROJECTM_LIB", "/nonexistent/libprojectM-4.so");
        std::env::set_var("OMARADIO_MILK_PRESETS", "/nonexistent/presets");
        // Only asserts the shape: on a machine with the lib installed the
        // env override still wins for presets, so either error text is fine.
        match availability() {
            Ok((lib, dir, n)) => assert!(lib.is_file() && dir.is_dir() && n > 0),
            Err(e) => assert!(e.contains("setup-milkdrop") || e.contains("no .milk")),
        }
        std::env::remove_var("OMARADIO_PROJECTM_LIB");
        std::env::remove_var("OMARADIO_MILK_PRESETS");
    }
}
