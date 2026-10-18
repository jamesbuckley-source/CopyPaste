//! Screen colour sampling for the colour-picker utility.
//!
//! The pick waits for a real mouse click rather than running a visible overlay:
//! the window hides, the next left click anywhere on screen is sampled, and the
//! window comes back with the result. Escape cancels.

use std::time::{Duration, Instant};

use tauri::AppHandle;

use crate::error::{AppError, Result};
use crate::window;

/// How long a pick waits for the user to click something.
const PICK_TIMEOUT: Duration = Duration::from_secs(20);

/// Hides the window, waits for the next left click and returns its colour.
#[cfg(windows)]
pub fn pick(app: &AppHandle) -> Result<String> {
    window::hide(app);
    // Give the window a moment to leave the screen before the sample is taken.
    std::thread::sleep(Duration::from_millis(180));

    let deadline = Instant::now() + PICK_TIMEOUT;
    let outcome = loop {
        if Instant::now() > deadline {
            break Err(AppError::msg("no click was recorded; try again"));
        }
        if pressed(win32::VK_ESCAPE) {
            break Err(AppError::msg("the colour pick was cancelled"));
        }
        if pressed(win32::VK_LBUTTON) {
            break sample_under_cursor();
        }
        std::thread::sleep(Duration::from_millis(25));
    };

    window::show(app);
    outcome
}

/// Non-Windows builds have no screen to sample.
#[cfg(not(windows))]
pub fn pick(_app: &AppHandle) -> Result<String> {
    Err(AppError::msg(
        "picking a colour from the screen is only supported on Windows",
    ))
}

#[cfg(windows)]
fn pressed(key: win32::VIRTUAL_KEY) -> bool {
    (unsafe { win32::GetAsyncKeyState(key.0 as i32) } & 0x8000u16 as i16) != 0
}

#[cfg(windows)]
fn sample_under_cursor() -> Result<String> {
    let mut point = win32::POINT { x: 0, y: 0 };
    if unsafe { win32::GetCursorPos(&mut point) }.is_err() {
        return Err(AppError::msg("could not read the cursor position"));
    }
    unsafe {
        let dc = win32::GetDC(None);
        if dc.is_invalid() {
            return Err(AppError::msg("could not read the screen"));
        }
        let pixel = win32::GetPixel(dc, point.x, point.y).0;
        win32::ReleaseDC(None, dc);
        hex_of(pixel)
    }
}

/// `COLORREF` stores blue in the high byte; the picker reports `#rrggbb`.
fn hex_of(pixel: u32) -> Result<String> {
    // CLR_INVALID signals a failed sample (a protected window, for example).
    if pixel == u32::MAX {
        return Err(AppError::msg("that spot could not be read"));
    }
    let red = (pixel & 0xFF) as u8;
    let green = ((pixel >> 8) & 0xFF) as u8;
    let blue = ((pixel >> 16) & 0xFF) as u8;
    Ok(format!("#{red:02x}{green:02x}{blue:02x}"))
}

#[cfg(windows)]
mod win32 {
    pub use windows::Win32::Foundation::POINT;
    pub use windows::Win32::Graphics::Gdi::{GetDC, GetPixel, ReleaseDC};
    pub use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, VIRTUAL_KEY, VK_ESCAPE, VK_LBUTTON,
    };
    pub use windows::Win32::UI::WindowsAndMessaging::GetCursorPos;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_colour_ref_is_reported_as_hex() {
        // COLORREF 0x00BBGGRR: orange (255, 136, 0) is 0x000088ff.
        assert_eq!(hex_of(0x0000_88ff).unwrap(), "#ff8800");
        assert_eq!(hex_of(0).unwrap(), "#000000");
        assert_eq!(hex_of(0x00ff_ffff).unwrap(), "#ffffff");
    }

    #[test]
    fn an_unreadable_spot_is_an_error() {
        assert!(hex_of(u32::MAX).is_err());
    }
}
