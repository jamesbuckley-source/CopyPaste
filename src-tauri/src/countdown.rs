//! The countdown shown before a delayed screenshot is taken.
//!
//! It runs *after* the area has been chosen and *before* the pixels are read, so a
//! delay can be used to open a menu, hover a tooltip or move a window and still
//! catch the result. The desktop stays usable throughout: the countdown is one
//! small window in the corner, not an overlay over the screen.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use tauri::AppHandle;

#[cfg(windows)]
use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, RegisterHotKey, UnregisterHotKey, VK_ESCAPE,
};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{MSG, PM_REMOVE, PeekMessageW, WM_HOTKEY};

/// Set by [`cancel`] while a countdown is on screen.
static CANCELLED: AtomicBool = AtomicBool::new(false);

/// Stops a countdown that is already running; the screenshot it was counting down
/// to is dropped. Does nothing when no countdown is showing.
pub fn cancel() {
    CANCELLED.store(true, Ordering::SeqCst);
}

/// Catches Esc for as long as the countdown runs.
///
/// The countdown window deliberately does not take focus, so the key never reaches
/// it: a bare Esc system-wide is a global hotkey, exactly as the ruler's lock uses
/// to give the screen back. It is registered on this worker thread and read from
/// that thread's own queue, so no window is needed.
#[cfg(windows)]
struct EscapeKey {
    registered: bool,
}

#[cfg(windows)]
impl EscapeKey {
    /// Distinct from the ruler's own Esc, and from anything the shortcut settings
    /// register, so the two cannot be unregistered by each other.
    const ID: i32 = 0x0E5D;

    fn install() -> Self {
        // SAFETY: a thread hotkey with no window handle; it is undone in `drop`.
        let registered = unsafe {
            RegisterHotKey(None, Self::ID, HOT_KEY_MODIFIERS(0), u32::from(VK_ESCAPE.0))
        }
        .is_ok();
        if !registered {
            // Something else owns Esc: the ring can still be clicked to cancel.
            log::warn!("could not register Esc to cancel the screenshot countdown");
        }
        Self { registered }
    }

    /// True when Esc was pressed since the last call.
    fn pressed(&self) -> bool {
        if !self.registered {
            return false;
        }
        let mut pressed = false;
        let mut message = MSG::default();
        // SAFETY: the message is a local that outlives the call, and the queue
        // belongs to this thread, which has nothing else in it.
        while unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) }.as_bool() {
            if message.message == WM_HOTKEY && message.wParam.0 as i32 == Self::ID {
                pressed = true;
            }
        }
        pressed
    }
}

#[cfg(windows)]
impl Drop for EscapeKey {
    fn drop(&mut self) {
        if self.registered {
            // SAFETY: undoing this thread's own registration.
            let _ = unsafe { UnregisterHotKey(None, Self::ID) };
        }
    }
}

/// Shows the countdown and waits it out. `false` means the user cancelled, and the
/// caller should take no picture.
pub fn run(app: &AppHandle, seconds: u32) -> bool {
    if seconds == 0 {
        return true;
    }
    CANCELLED.store(false, Ordering::SeqCst);
    #[cfg(windows)]
    let escape = EscapeKey::install();
    crate::window::show_countdown(app, seconds);
    let deadline = Instant::now() + Duration::from_secs(u64::from(seconds));
    // One tick as each second starts, the first included: a countdown that is only
    // read is easy to miss when the eye is on the part of the screen being shot.
    let mut ticking = seconds;
    crate::sound::play_countdown_tick();
    while Instant::now() < deadline && !CANCELLED.load(Ordering::SeqCst) {
        let left = deadline.saturating_duration_since(Instant::now());
        let second = left.as_secs_f32().ceil() as u32;
        if second != ticking {
            ticking = second;
            crate::sound::play_countdown_tick();
        }
        #[cfg(windows)]
        if escape.pressed() {
            CANCELLED.store(true, Ordering::SeqCst);
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    // Esc goes back to the rest of the desktop as soon as the countdown is over.
    #[cfg(windows)]
    drop(escape);
    crate::window::close_countdown(app);
    if CANCELLED.load(Ordering::SeqCst) {
        return false;
    }
    // The window has to be off the screen before anything is captured, exactly as
    // hiding the app window is given a moment in the capture overlays.
    std::thread::sleep(Duration::from_millis(150));
    true
}
