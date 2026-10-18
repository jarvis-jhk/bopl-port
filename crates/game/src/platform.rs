//! SDL2 platform helpers shared by the windowed host and `--list-devices`.

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

/// The window's size in pixels (differs from `Window::size` on high-DPI
/// setups).
pub fn window_pixels(window: &sdl2::video::Window) -> (u32, u32) {
    let (mut w, mut h) = (0, 0);
    // SAFETY: valid window pointer for the duration of the call.
    unsafe { sdl2::sys::SDL_GetWindowSizeInPixels(window.raw(), &mut w, &mut h) };
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
