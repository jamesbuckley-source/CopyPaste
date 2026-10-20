//! Screen colour sampling for the colour-picker utility.
//!
//! The pick waits for a real mouse click rather than running a visible overlay:
//! the window hides, the next left click anywhere on screen is sampled, and the
//! window comes back with the result. Escape cancels.

use std::sync::Once;
#[cfg(windows)]
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use tauri::AppHandle;

use crate::error::{AppError, Result};
use crate::window;

/// How long a pick waits for the user to click something.
const PICK_TIMEOUT: Duration = Duration::from_secs(20);
/// The magnifier samples an odd square so the cursor sits on its centre pixel.
const ZOOM_PIXELS: i32 = 15;
const ZOOM_FACTOR: i32 = 12;
const ZOOM_SIZE: i32 = ZOOM_PIXELS * ZOOM_FACTOR;
const FOOTER_HEIGHT: i32 = 26;
const MAGNIFIER_OFFSET: i32 = 20;

/// Set by the low-level mouse hook when the pick should sample.
#[cfg(windows)]
static CLICKED: AtomicBool = AtomicBool::new(false);

/// Hides the window, waits for the next left click and returns its colour.
///
/// `Ok(None)` means the pick was cancelled or timed out, which is not an error.
#[cfg(windows)]
pub fn pick(app: &AppHandle) -> Result<Option<String>> {
    window::hide(app);
    let _cursor = PickerCursor::install();
    // Give the window a moment to leave the screen before the sample is taken.
    std::thread::sleep(Duration::from_millis(180));
    let magnifier = Magnifier::show();
    // Swallow clicks while the pick is pending: sampling a colour must not also
    // press a button, open a dropdown or pause a video underneath.
    let blocker = MouseBlocker::install();

    let deadline = Instant::now() + PICK_TIMEOUT;
    let outcome: Result<Option<String>> = loop {
        MouseBlocker::pump();
        if let Some(magnifier) = &magnifier {
            magnifier.follow();
        }
        if Instant::now() > deadline {
            break Ok(None);
        }
        if pressed(win32::VK_ESCAPE) {
            break Ok(None);
        }
        // The hook reports the press itself: returning 1 from a low-level hook
        // stops the system from updating the async key state, so polling
        // `GetAsyncKeyState` would never see a click the blocker swallowed.
        if MouseBlocker::clicked() || (blocker.is_none() && pressed(win32::VK_LBUTTON)) {
            break sample_under_cursor().map(Some);
        }
        std::thread::sleep(Duration::from_millis(25));
    };

    drop(blocker);
    drop(magnifier);
    window::show(app);
    outcome
}

/// Non-Windows builds have no screen to sample.
#[cfg(not(windows))]
pub fn pick(_app: &AppHandle) -> Result<Option<String>> {
    Err(AppError::msg(
        "picking a colour from the screen is only supported on Windows",
    ))
}

#[cfg(windows)]
fn pressed(key: win32::VIRTUAL_KEY) -> bool {
    (unsafe { win32::GetAsyncKeyState(key.0 as i32) } & 0x8000u16 as i16) != 0
}

/// Swaps the system arrow for the picker crosshair until the guard is dropped,
/// so every window shows the sampling cursor while a pick is pending.
#[cfg(windows)]
struct PickerCursor(bool);

#[cfg(windows)]
impl PickerCursor {
    fn install() -> Self {
        unsafe {
            let installed = win32::LoadCursorW(None, win32::IDC_CROSS)
                .ok()
                // A cursor copy is what `SetSystemCursor` takes ownership of; the
                // handle types differ, so cast through the shared pointer.
                .and_then(|cross| win32::CopyIcon(win32::HICON(cross.0)).ok())
                .is_some_and(|copy| {
                    win32::SetSystemCursor(win32::HCURSOR(copy.0), win32::OCR_NORMAL).is_ok()
                });
            Self(installed)
        }
    }
}

#[cfg(windows)]
impl Drop for PickerCursor {
    fn drop(&mut self) {
        if self.0 {
            // Reloading from the registry restores the cursor size and scheme the
            // user actually has, which a saved handle would not survive.
            unsafe {
                _ = win32::SystemParametersInfoW(
                    win32::SPI_SETCURSORS,
                    0,
                    None,
                    win32::SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
                );
            }
        }
    }
}

/// Installs a low-level mouse hook that swallows clicks while a pick is pending.
///
/// A low-level hook is delivered to the thread that installed it, so the picking
/// loop pumps messages itself; without a pump the hook is never called and the
/// click would reach the window underneath.
#[cfg(windows)]
struct MouseBlocker(win32::HHOOK);

#[cfg(windows)]
impl MouseBlocker {
    fn install() -> Option<Self> {
        CLICKED.store(false, Ordering::SeqCst);
        unsafe {
            win32::SetWindowsHookExW(win32::WH_MOUSE_LL, Some(win32::block_mouse), None, 0)
                .ok()
                .map(Self)
        }
    }

    /// True once the hook has seen a left-button press since `install`.
    fn clicked() -> bool {
        CLICKED.load(Ordering::SeqCst)
    }

    /// Drains the thread's message queue so the hook callback runs.
    fn pump() {
        unsafe {
            let mut message = win32::MSG::default();
            while win32::PeekMessageW(&mut message, None, 0, 0, win32::PM_REMOVE).as_bool() {
                let _ = win32::TranslateMessage(&message);
                let _ = win32::DispatchMessageW(&message);
            }
        }
    }
}

#[cfg(windows)]
impl Drop for MouseBlocker {
    fn drop(&mut self) {
        unsafe {
            let _ = win32::UnhookWindowsHookEx(self.0);
        }
    }
}

/// A small topmost popup that magnifies the pixels around the cursor and names
/// the colour under it, so a pick can be aimed instead of guessed.
#[cfg(windows)]
struct Magnifier(win32::HWND);

#[cfg(windows)]
impl Magnifier {
    fn show() -> Option<Self> {
        unsafe {
            let module = win32::GetModuleHandleW(None).ok()?;
            static REGISTER: Once = Once::new();
            REGISTER.call_once(|| {
                let class = win32::WNDCLASSW {
                    hInstance: win32::HINSTANCE(module.0),
                    lpfnWndProc: Some(win32::window_proc),
                    lpszClassName: windows::core::w!("CopyPasteMagnifier"),
                    ..Default::default()
                };
                // A zero return is fine: the class may already be registered.
                win32::RegisterClassW(&class);
            });

            let height = ZOOM_SIZE + FOOTER_HEIGHT;
            let hwnd = win32::CreateWindowExW(
                win32::WS_EX_TOPMOST
                    | win32::WS_EX_TOOLWINDOW
                    | win32::WS_EX_NOACTIVATE
                    | win32::WS_EX_TRANSPARENT,
                windows::core::w!("CopyPasteMagnifier"),
                windows::core::w!("CopyPasteMagnifier"),
                win32::WS_POPUP,
                0,
                0,
                ZOOM_SIZE,
                height,
                None,
                None,
                Some(win32::HINSTANCE(module.0)),
                None,
            )
            .ok()?;
            let _ = win32::ShowWindow(hwnd, win32::SW_SHOWNOACTIVATE);
            Some(Self(hwnd))
        }
    }

    /// Moves the popup next to the cursor and repaints the zoomed sample.
    fn follow(&self) {
        unsafe {
            let mut point = win32::POINT::default();
            if win32::GetCursorPos(&mut point).is_err() {
                return;
            }
            let virtual_left = win32::GetSystemMetrics(win32::SM_XVIRTUALSCREEN);
            let virtual_top = win32::GetSystemMetrics(win32::SM_YVIRTUALSCREEN);
            let virtual_width = win32::GetSystemMetrics(win32::SM_CXVIRTUALSCREEN);
            let virtual_height = win32::GetSystemMetrics(win32::SM_CYVIRTUALSCREEN);
            let height = ZOOM_SIZE + FOOTER_HEIGHT;

            // Prefer below-right of the crosshair; flip at the desktop edges so
            // the popup never runs off any monitor.
            let mut left = point.x + MAGNIFIER_OFFSET;
            let mut top = point.y + MAGNIFIER_OFFSET;
            if left + ZOOM_SIZE > virtual_left + virtual_width {
                left = point.x - MAGNIFIER_OFFSET - ZOOM_SIZE;
            }
            if top + height > virtual_top + virtual_height {
                top = point.y - MAGNIFIER_OFFSET - height;
            }
            left = left.max(virtual_left);
            top = top.max(virtual_top);

            let _ = win32::SetWindowPos(
                self.0,
                Some(win32::HWND_TOPMOST),
                left,
                top,
                ZOOM_SIZE,
                height,
                win32::SWP_NOACTIVATE | win32::SWP_NOREDRAW,
            );
            self.paint(
                point,
                (virtual_left, virtual_top),
                (virtual_width, virtual_height),
            );
        }
    }

    unsafe fn paint(
        &self,
        cursor: win32::POINT,
        virtual_origin: (i32, i32),
        virtual_size: (i32, i32),
    ) {
        use windows::Win32::Foundation::RECT;

        // The desktop DC starts at the virtual-screen origin, so negative
        // coordinates from a monitor left of the primary become valid locals.
        let screen = unsafe { win32::GetDC(Some(win32::GetDesktopWindow())) };
        let window_dc = unsafe { win32::GetDC(Some(self.0)) };
        if screen.is_invalid() || window_dc.is_invalid() {
            if !screen.is_invalid() {
                unsafe { win32::ReleaseDC(Some(win32::GetDesktopWindow()), screen) };
            }
            if !window_dc.is_invalid() {
                unsafe { win32::ReleaseDC(Some(self.0), window_dc) };
            }
            return;
        }

        let (virtual_left, virtual_top) = virtual_origin;
        let (virtual_width, virtual_height) = virtual_size;
        let local_x = cursor.x - virtual_left;
        let local_y = cursor.y - virtual_top;
        let total_height = ZOOM_SIZE + FOOTER_HEIGHT;

        // Compose into an off-screen bitmap and blit once, so the popup never
        // shows a half-drawn frame while it follows the cursor.
        let memory_dc = unsafe { win32::CreateCompatibleDC(Some(window_dc)) };
        let bitmap =
            unsafe { win32::CreateCompatibleBitmap(window_dc, ZOOM_SIZE, total_height) };
        if memory_dc.is_invalid() || bitmap.is_invalid() {
            if !memory_dc.is_invalid() {
                unsafe { let _ = win32::DeleteDC(memory_dc); }
            }
            unsafe {
                win32::ReleaseDC(Some(self.0), window_dc);
                win32::ReleaseDC(Some(win32::GetDesktopWindow()), screen);
            }
            return;
        }
        let old_bitmap = unsafe { win32::SelectObject(memory_dc, bitmap.into()) };

        let footer_top = ZOOM_SIZE;
        let background = unsafe { win32::CreateSolidBrush(win32::COLORREF(0x0020_2020)) };
        let all = RECT {
            left: 0,
            top: 0,
            right: ZOOM_SIZE,
            bottom: total_height,
        };
        unsafe {
            win32::FillRect(memory_dc, &all, background);
            let _ = win32::DeleteObject(background.into());
        }

        // Clamping the source rectangle keeps `StretchBlt` valid at the edges.
        let half = ZOOM_PIXELS / 2;
        let source_x = (local_x - half).clamp(0, (virtual_width - ZOOM_PIXELS).max(0));
        let source_y = (local_y - half).clamp(0, (virtual_height - ZOOM_PIXELS).max(0));
        unsafe {
            win32::SetStretchBltMode(memory_dc, win32::COLORONCOLOR);
            let _ = win32::StretchBlt(
                memory_dc,
                0,
                0,
                ZOOM_SIZE,
                ZOOM_SIZE,
                Some(screen),
                source_x,
                source_y,
                ZOOM_PIXELS,
                ZOOM_PIXELS,
                win32::SRCCOPY,
            );
        }

        // Mark the pixel the click will read.
        let cell_x = (local_x - source_x) * ZOOM_FACTOR;
        let cell_y = (local_y - source_y) * ZOOM_FACTOR;
        unsafe {
            let pen = win32::CreatePen(win32::PS_SOLID, 1, win32::COLORREF(0x00ff_ffff));
            let old_pen = win32::SelectObject(memory_dc, pen.into());
            let null_brush = win32::GetStockObject(win32::NULL_BRUSH);
            let old_brush = win32::SelectObject(memory_dc, null_brush);
            let _ = win32::Rectangle(
                memory_dc,
                cell_x,
                cell_y,
                cell_x + ZOOM_FACTOR,
                cell_y + ZOOM_FACTOR,
            );
            win32::SelectObject(memory_dc, old_brush);
            win32::SelectObject(memory_dc, old_pen);
            let _ = win32::DeleteObject(pen.into());
        }

        let pixel = unsafe {
            win32::GetPixel(
                screen,
                local_x.clamp(0, virtual_width.saturating_sub(1)),
                local_y.clamp(0, virtual_height.saturating_sub(1)),
            )
        };
        let color = if pixel.0 == u32::MAX {
            win32::COLORREF(0)
        } else {
            pixel
        };
        let (red, green, blue) = rgb_of(color.0);
        let label = format!("#{red:02x}{green:02x}{blue:02x}  rgb({red}, {green}, {blue})");
        let ink = if 0.299 * f32::from(red) + 0.587 * f32::from(green) + 0.114 * f32::from(blue)
            > 150.0
        {
            win32::COLORREF(0x0000_0000)
        } else {
            win32::COLORREF(0x00ff_ffff)
        };

        let chip = unsafe { win32::CreateSolidBrush(color) };
        let footer = RECT {
            left: 0,
            top: footer_top,
            right: ZOOM_SIZE,
            bottom: total_height,
        };
        unsafe {
            win32::FillRect(memory_dc, &footer, chip);
            let _ = win32::DeleteObject(chip.into());
            win32::SetBkMode(memory_dc, win32::TRANSPARENT);
            win32::SetTextColor(memory_dc, ink);
            let font = win32::GetStockObject(win32::DEFAULT_GUI_FONT);
            let old_font = win32::SelectObject(memory_dc, font);
            let text: Vec<u16> = label.encode_utf16().collect();
            let _ = win32::TextOutW(memory_dc, 5, footer_top + 6, &text);
            win32::SelectObject(memory_dc, old_font);

            let _ = win32::BitBlt(
                window_dc,
                0,
                0,
                ZOOM_SIZE,
                total_height,
                Some(memory_dc),
                0,
                0,
                win32::SRCCOPY,
            );
            win32::SelectObject(memory_dc, old_bitmap);
            let _ = win32::DeleteObject(bitmap.into());
            let _ = win32::DeleteDC(memory_dc);

            win32::ReleaseDC(Some(self.0), window_dc);
            win32::ReleaseDC(Some(win32::GetDesktopWindow()), screen);
        }
    }
}

#[cfg(windows)]
impl Drop for Magnifier {
    fn drop(&mut self) {
        unsafe {
            let _ = win32::DestroyWindow(self.0);
        }
    }
}

/// `COLORREF` stores blue in the high byte.
fn rgb_of(pixel: u32) -> (u8, u8, u8) {
    (
        (pixel & 0xFF) as u8,
        ((pixel >> 8) & 0xFF) as u8,
        ((pixel >> 16) & 0xFF) as u8,
    )
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
    use std::sync::atomic::Ordering;

    use super::CLICKED;
    pub use windows::Win32::Foundation::{
        COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, WPARAM,
    };
    pub use windows::Win32::Graphics::Gdi::{
        BitBlt, COLORONCOLOR, CreateCompatibleBitmap, CreateCompatibleDC, CreatePen,
        CreateSolidBrush, DEFAULT_GUI_FONT, DeleteDC, DeleteObject, FillRect, GetDC, GetPixel,
        GetStockObject, NULL_BRUSH, PS_SOLID, Rectangle, ReleaseDC, SRCCOPY, SelectObject, SetBkMode,
        SetStretchBltMode, SetTextColor, StretchBlt, TRANSPARENT, TextOutW,
    };
    pub use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    pub use windows::Win32::UI::Input::KeyboardAndMouse::{
        GetAsyncKeyState, VIRTUAL_KEY, VK_ESCAPE, VK_LBUTTON,
    };
    pub use windows::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, CopyIcon, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
        GetCursorPos, GetDesktopWindow, GetSystemMetrics, HCURSOR, HHOOK, HICON, HWND_TOPMOST,
        IDC_CROSS, LoadCursorW, MSG, OCR_NORMAL, PM_REMOVE, PeekMessageW, RegisterClassW,
        SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
        SPI_SETCURSORS, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_NOREDRAW,
        SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SetSystemCursor, SetWindowPos, SetWindowsHookExW,
        ShowWindow, SystemParametersInfoW, TranslateMessage, UnhookWindowsHookEx, WH_MOUSE_LL,
        WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_RBUTTONDOWN, WM_RBUTTONUP,
        WM_XBUTTONDOWN, WM_XBUTTONUP, WNDCLASSW, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
        WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
    };

    /// `WNDCLASSW` requires an `extern "system"` function pointer, which a plain
    /// re-export of `DefWindowProcW` is not.
    pub unsafe extern "system" fn window_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
    }

    /// Swallows button presses so sampling a colour cannot activate anything
    /// underneath. Everything else is passed straight through.
    pub unsafe extern "system" fn block_mouse(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        const SWALLOWED: [u32; 8] = [
            WM_LBUTTONDOWN,
            WM_LBUTTONUP,
            WM_RBUTTONDOWN,
            WM_RBUTTONUP,
            WM_MBUTTONDOWN,
            WM_MBUTTONUP,
            WM_XBUTTONDOWN,
            WM_XBUTTONUP,
        ];
        if code >= 0 && SWALLOWED.contains(&(wparam.0 as u32)) {
            if (wparam.0 as u32) == WM_LBUTTONDOWN {
                CLICKED.store(true, Ordering::SeqCst);
            }
            return LRESULT(1);
        }
        unsafe { CallNextHookEx(None, code, wparam, lparam) }
    }
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
    fn a_colour_ref_splits_into_rgb_channels() {
        assert_eq!(rgb_of(0x0000_88ff), (255, 136, 0));
        assert_eq!(rgb_of(0), (0, 0, 0));
    }

    #[test]
    fn an_unreadable_spot_is_an_error() {
        assert!(hex_of(u32::MAX).is_err());
    }
}
