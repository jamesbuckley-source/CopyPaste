//! Auto-paste: return focus to where the user was, then synthesise Ctrl+V.
//!
//! Two things have to happen before the keystroke is useful. First, our own
//! window must be out of the way, otherwise the paste lands in the search box the
//! user was just typing in. Second, the previous window has to actually be
//! foreground again — Windows only honours synthetic input for the focused
//! window, so `SetForegroundWindow` is a prerequisite rather than a nicety.
//!
//! This is opt-in and off by default: injecting keystrokes into other
//! applications is exactly the kind of behaviour a user should choose.

use std::sync::atomic::{AtomicIsize, Ordering};
use std::time::Duration;

use crate::error::{AppError, Result};

#[cfg(windows)]
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP,
    MAPVK_VK_TO_VSC, MapVirtualKeyW, SendInput, VIRTUAL_KEY, VK_CONTROL,
};

/// Window handle to paste into, captured when the history window is summoned.
/// `0` means "nothing remembered".
static TARGET: AtomicIsize = AtomicIsize::new(0);

pub fn target() -> Option<isize> {
    match TARGET.load(Ordering::Relaxed) {
        0 => None,
        handle => Some(handle),
    }
}

/// Records the window that is in front right now, ignoring `own_window`.
///
/// Called as the hotkey fires, before our window is shown, so the remembered
/// handle is the application the user was working in.
#[cfg(windows)]
pub fn capture_foreground(own_window: Option<isize>) {
    use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
    // SAFETY: GetForegroundWindow takes no arguments and cannot fail.
    let window = unsafe { GetForegroundWindow() };
    if window.is_invalid() {
        return;
    }
    if Some(window.0 as isize) == own_window {
        return;
    }
    TARGET.store(window.0 as isize, Ordering::Relaxed);
}

#[cfg(not(windows))]
pub fn capture_foreground(_own_window: Option<isize>) {}

/// Brings a remembered window back to the foreground, restoring it if minimised.
#[cfg(windows)]
pub fn activate(handle: isize) -> bool {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        IsIconic, IsWindow, SW_RESTORE, SetForegroundWindow, ShowWindow,
    };

    let window = HWND(handle as *mut core::ffi::c_void);
    // SAFETY: `window` comes from GetForegroundWindow, and every call below only
    // reads or adjusts that window. IsWindow guards against it having been closed
    // while the history window was open.
    unsafe {
        if !IsWindow(Some(window)).as_bool() {
            return false;
        }
        if IsIconic(window).as_bool() {
            let _ = ShowWindow(window, SW_RESTORE);
        }
        SetForegroundWindow(window).as_bool()
    }
}

#[cfg(not(windows))]
pub fn activate(_handle: isize) -> bool {
    false
}

/// Hides our window, focuses the target, and sends Ctrl+V.
pub fn restore_and_paste(handle: isize, delay_ms: u64) -> Result<()> {
    let delay = Duration::from_millis(delay_ms.min(2_000));
    std::thread::sleep(delay);

    if !activate(handle) {
        return Err(AppError::msg(
            "the window you copied from is no longer available",
        ));
    }

    // Give the window manager a moment to finish the activation before the
    // keystroke arrives; without this the input can be discarded or routed to the
    // window that is still in the process of losing focus.
    std::thread::sleep(Duration::from_millis(40));
    send_paste()
}

/// Synthesises a Ctrl+V keystroke into the foreground window.
#[cfg(windows)]
pub fn send_paste() -> Result<()> {
    const VK_V: u16 = 0x56;

    let inputs = [
        key_input(VK_CONTROL, false),
        key_input(VIRTUAL_KEY(VK_V), false),
        key_input(VIRTUAL_KEY(VK_V), true),
        key_input(VK_CONTROL, true),
    ];

    // SAFETY: `inputs` is a valid, correctly sized slice of INPUT structures.
    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    if sent != inputs.len() as u32 {
        return Err(AppError::msg(format!(
            "only {sent} of {} input events were accepted; \
             a window with higher integrity may be in the foreground",
            inputs.len()
        )));
    }
    Ok(())
}

#[cfg(windows)]
fn key_input(key: VIRTUAL_KEY, key_up: bool) -> INPUT {
    // Supplying the scan code as well as the virtual key keeps the synthetic input
    // working in applications that read hardware scan codes directly (games, some
    // terminals and remote-desktop clients), which ignore VK-only events.
    // SAFETY: MapVirtualKeyW is a pure lookup for a valid virtual key.
    let scan = unsafe { MapVirtualKeyW(key.0 as u32, MAPVK_VK_TO_VSC) } as u16;

    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: key,
                wScan: scan,
                dwFlags: if key_up {
                    KEYEVENTF_KEYUP
                } else {
                    KEYBD_EVENT_FLAGS(0)
                },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

#[cfg(not(windows))]
pub fn send_paste() -> Result<()> {
    Err(AppError::msg("auto-paste is only implemented on Windows"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn key_input_marks_key_up_events() {
        // SAFETY: reading back the union fields we just initialised.
        let down = unsafe { key_input(VK_CONTROL, false).Anonymous.ki };
        assert_eq!(down.wVk, VK_CONTROL);
        assert_eq!(down.dwFlags, KEYBD_EVENT_FLAGS(0));
        assert!(
            down.wScan > 0,
            "scan code should be resolved for compatibility"
        );

        // SAFETY: as above.
        let up = unsafe { key_input(VK_CONTROL, true).Anonymous.ki };
        assert_eq!(up.dwFlags, KEYEVENTF_KEYUP);

        let letter = VIRTUAL_KEY(0x56);
        // SAFETY: as above.
        let v = unsafe { key_input(letter, false).Anonymous.ki };
        assert_eq!(v.wVk, letter);
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "sends real keystrokes to the foreground window"]
    fn send_paste_reports_success() {
        assert!(send_paste().is_ok());
    }
}
