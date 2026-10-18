//! SDL2 platform helpers shared by the windowed host and `--list-devices`.

use std::ffi::{CStr, c_int, c_void};
use std::sync::OnceLock;

use sdl2::hint;

/// SDL hints set before subsystems start.
pub fn set_hints() {
    // Controllers keep working when the window is unfocused (couch play
    // with a second monitor, streaming).
    hint::set("SDL_JOYSTICK_ALLOW_BACKGROUND_EVENTS", "1");
    // Windows: RawInput + HIDAPI lift XInput's 4-controller limit.
    hint::set("SDL_JOYSTICK_RAWINPUT", "1");
    hint::set("SDL_JOYSTICK_HIDAPI", "1");
    // Do not let SDL disable the compositor or the screensaver behaviour
    // differently from other games.
    hint::set("SDL_VIDEO_X11_NET_WM_BYPASS_COMPOSITOR", "0");
    // Exclusive fullscreen: SDL minimises the window when it loses focus.
    // On Linux that hides the game for good on multi-monitor setups and
    // under some window managers (seen on Xvfb + metacity: the window went
    // iconic right after the mode switch). Windows keeps SDL's default
    // (alt-tab out of an exclusive mode should give the desktop back).
    if !cfg!(windows) {
        hint::set("SDL_VIDEO_MINIMIZE_ON_FOCUS_LOSS", "0");
    }
}

/// Looks up an SDL function that is newer than the oldest SDL we run on
/// (2.0.20, Ubuntu 22.04), so the binary does not need it at link time.
/// Null when the loaded SDL is too old.
pub fn sdl_optional_fn(name: &CStr) -> *mut c_void {
    const LIB: &CStr = if cfg!(windows) {
        c"SDL2.dll"
    } else if cfg!(target_os = "macos") {
        c"libSDL2-2.0.0.dylib"
    } else {
        c"libSDL2-2.0.so.0"
    };
    // SAFETY: SDL_LoadObject / SDL_LoadFunction take NUL-terminated names;
    // the library is already loaded (we link it), so this only takes another
    // reference, which is kept for the life of the process.
    unsafe {
        let lib = sdl2::sys::SDL_LoadObject(LIB.as_ptr());
        if lib.is_null() {
            return std::ptr::null_mut();
        }
        sdl2::sys::SDL_LoadFunction(lib, name.as_ptr())
    }
}

type GetWindowSizeInPixels =
    unsafe extern "C" fn(*mut sdl2::sys::SDL_Window, *mut c_int, *mut c_int);

/// The window's size in pixels (differs from `Window::size` on high-DPI
/// setups). SDL < 2.26 has no `SDL_GetWindowSizeInPixels`; there the Vulkan
/// drawable size is used, which is the same thing.
pub fn window_pixels(window: &sdl2::video::Window) -> (u32, u32) {
    static FN: OnceLock<usize> = OnceLock::new();
    let f = *FN.get_or_init(|| sdl_optional_fn(c"SDL_GetWindowSizeInPixels") as usize);
    let (mut w, mut h) = (0, 0);
    if f != 0 {
        // SAFETY: the symbol has this signature since SDL 2.26; valid window
        // pointer for the duration of the call.
        unsafe {
            let f: GetWindowSizeInPixels = std::mem::transmute(f);
            f(window.raw(), &mut w, &mut h);
        }
    } else {
        let (vw, vh) = window.vulkan_drawable_size();
        (w, h) = (vw as c_int, vh as c_int);
    }
    (w.max(1) as u32, h.max(1) as u32)
}

/// Fails (instead of panicking inside `raw-window-handle`) when the SDL
/// video driver cannot expose native window handles (e.g. the `offscreen`
/// or `dummy` drivers).
pub fn check_native_handles(window: &sdl2::video::Window) -> anyhow::Result<()> {
    // SAFETY: zeroed SysWMinfo with the version filled in is what
    // SDL_GetWindowWMInfo expects.
    unsafe {
        let mut info: sdl2::sys::SDL_SysWMinfo = std::mem::zeroed();
        sdl2::sys::SDL_GetVersion(&mut info.version);
        if sdl2::sys::SDL_GetWindowWMInfo(window.raw(), &mut info) == sdl2::sys::SDL_bool::SDL_FALSE
        {
            anyhow::bail!(
                "SDL video driver '{}' has no native window handles ({}); use --headless",
                window.subsystem().current_video_driver(),
                sdl2::get_error()
            );
        }
    }
    Ok(())
}

/// Resident memory of this process in KiB (Linux; `None` elsewhere).
pub fn rss_kib() -> Option<u64> {
    let s = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = s.lines().find(|l| l.starts_with("VmRSS:"))?;
    line.split_whitespace().nth(1)?.parse().ok()
}
