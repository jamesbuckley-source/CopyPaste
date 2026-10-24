//! Region and window screen capture for the title-bar screenshot buttons.
//!
//! Both captures follow the colour picker's shape rather than a webview overlay:
//! the app window hides, a raw Win32 popup owns the screen (a dimmed drag surface
//! for a region, a highlight frame for a window), Escape cancels, and the pixels
//! come back on the calling thread. The result is a plain [`ClipContent::Image`],
//! so the ordinary store path supplies thumbnails, de-duplication, OCR scheduling
//! and the history event without a capture-specific copy of any of it.
//!
//! The pure geometry and buffer helpers are kept apart from the FFI so the
//! clamping and filtering rules can be tested like the picker's colour helpers.

// The pure helpers below are only called by the Windows overlay and by the tests.
#![cfg_attr(not(windows), allow(dead_code))]

use tauri::AppHandle;

use crate::error::{AppError, Result};
use crate::models::ClipContent;

#[cfg(windows)]
use std::ffi::c_void;
#[cfg(windows)]
use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};
#[cfg(windows)]
use std::sync::{Mutex, MutexGuard, Once};
#[cfg(windows)]
use std::time::{Duration, Instant};

#[cfg(windows)]
use crate::picker::{self, KeyboardBlocker, MouseBlocker, PickerCursor};
#[cfg(windows)]
use crate::window;

/// How long a capture waits for the user to finish before giving up.
#[cfg(windows)]
const CAPTURE_TIMEOUT: Duration = Duration::from_secs(60);
/// The app window needs a moment to leave the screen before pixels are read.
#[cfg(windows)]
const HIDE_DELAY: Duration = Duration::from_millis(180);
/// Fast enough that the highlight tracks the cursor without spinning a core.
#[cfg(windows)]
const POLL_INTERVAL: Duration = Duration::from_millis(15);

/// How long a scrolled window gets to repaint before the next frame is read.
#[cfg(windows)]
const SETTLE_DELAY: Duration = Duration::from_millis(320);
/// Gap between the reads that check whether a scrolled window has moved, and then
/// whether it has stopped moving.
#[cfg(windows)]
const SCROLL_POLL: Duration = Duration::from_millis(60);
/// How long one scroll burst gets to move the window and come to rest.
#[cfg(windows)]
const SCROLL_TIMEOUT: Duration = Duration::from_millis(2_500);
/// Mean per-channel difference at which a column is taken to belong to the window
/// rather than to the page: a sidebar or gutter that does not scroll.
#[cfg(windows)]
const FROZEN_COLUMN_TOLERANCE: f32 = 1.5;
/// Rows dropped from the bottom of every scrolled frame. The captured frame ends on
/// the window's own edge, and repeating that edge at every join draws a line right
/// across the stitched image.
#[cfg(windows)]
const FRAME_BOTTOM_TRIM: u32 = 2;
/// Gap between moving the pointer and wheeling, so the window sees the move first.
#[cfg(windows)]
const POINTER_SETTLE: Duration = Duration::from_millis(40);
/// Wheel notches per scroll step. Deliberately small: the stitching compares bands
/// in the lower half of the frame, so a step that moved most of a screen would
/// leave too little of the old frame to match against.
#[cfg(windows)]
const SCROLL_NOTCHES: i32 = 1;
/// Stop after this many scroll steps, so a runaway page cannot capture forever.
/// At four notches a step this is roughly 15,000 px of page.
#[cfg(windows)]
const MAX_SCROLLS: u32 = 200;
/// A stitched capture taller than this is not useful and eats memory at 4 bytes a
/// pixel, so the run stops there.
#[cfg(windows)]
const MAX_SCROLL_HEIGHT: u32 = 20_000;
/// `WHEEL_DELTA`: one notch of a mouse wheel.
#[cfg(windows)]
const WHEEL_DELTA: i32 = 120;

/// Thickness of the border painted around a selection or highlighted window.
const BORDER: i32 = 2;
/// Size of the "width × height" chip shown while a region is dragged.
const CHIP: (i32, i32) = (96, 20);
const CHIP_MARGIN: i32 = 6;

/// Windows can report a nonsensical rectangle while they are being resized, and a
/// capture allocates four bytes per pixel. Anything past this is refused instead
/// of risking a failed multi-hundred-megabyte allocation.
const MAX_PIXELS: i64 = 50_000_000;

/// One in every this many pixels is checked when deciding whether a window
/// rendered as the all-black bitmap `PrintWindow` produces on hardware surfaces.
const BLANK_SAMPLE_STRIDE: usize = 97;

/// A rectangle in physical screen pixels, with exclusive right and bottom edges
/// to match `RECT` and `BitBlt`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    pub const fn new(left: i32, top: i32, right: i32, bottom: i32) -> Self {
        Self {
            left,
            top,
            right,
            bottom,
        }
    }

    pub const fn width(self) -> i32 {
        self.right - self.left
    }

    pub const fn height(self) -> i32 {
        self.bottom - self.top
    }

    pub const fn is_empty(self) -> bool {
        self.width() <= 0 || self.height() <= 0
    }

    /// The overlapping part of this rectangle and `bounds`, or `None` when they
    /// do not overlap at all.
    pub fn intersect(self, bounds: Rect) -> Option<Rect> {
        let rect = Rect::new(
            self.left.max(bounds.left),
            self.top.max(bounds.top),
            self.right.min(bounds.right),
            self.bottom.min(bounds.bottom),
        );
        (!rect.is_empty()).then_some(rect)
    }

    /// Grows the rectangle by `margin` on every side. Used to leave room for the
    /// border, which must sit outside the pixels that get captured.
    pub const fn inflate(self, margin: i32) -> Rect {
        Rect::new(
            self.left - margin,
            self.top - margin,
            self.right + margin,
            self.bottom + margin,
        )
    }
}

/// Normalises the drag between two points and clips it to the virtual screen.
///
/// A drag in any direction produces the same rectangle, and a click without a
/// drag, or a drag that lies entirely outside the screen, yields `None`.
pub fn selection_rect(start: (i32, i32), end: (i32, i32), bounds: Rect) -> Option<Rect> {
    Rect::new(
        start.0.min(end.0),
        start.1.min(end.1),
        start.0.max(end.0),
        start.1.max(end.1),
    )
    .intersect(bounds)
}

/// Whether a window is worth highlighting: it needs a name the user can recognise
/// and an area to capture. Shell windows and titleless helpers fail the same test.
pub fn is_capture_target(title: &str, rect: Rect) -> bool {
    !title.trim().is_empty() && !rect.is_empty()
}

/// Where the visible part of a window sits inside the rectangle `PrintWindow`
/// renders, so the invisible resize border and the drop shadow can be cropped off.
pub fn visible_inset(frame: Rect, visible: Rect) -> Rect {
    Rect::new(
        visible.left - frame.left,
        visible.top - frame.top,
        visible.right - frame.left,
        visible.bottom - frame.top,
    )
}

/// Validates dimensions before a capture allocates its pixel buffer.
pub fn checked_pixels(width: i32, height: i32) -> Result<usize> {
    if width <= 0 || height <= 0 {
        return Err(AppError::msg("that area has no size"));
    }
    let pixels = i64::from(width) * i64::from(height);
    if pixels > MAX_PIXELS {
        return Err(AppError::msg("that area is too large to capture"));
    }
    Ok(pixels as usize)
}

/// Copies `rect`, in coordinates local to a `width` × `height` RGBA buffer, into
/// its own buffer. A partly visible rectangle is clipped; a rectangle that lies
/// entirely outside, or a buffer too short for the declared size, yields `None`.
pub fn crop(
    rgba: &[u8],
    width: u32,
    height: u32,
    rect: Rect,
) -> Option<(u32, u32, Vec<u8>)> {
    if rgba.len() < width as usize * height as usize * 4 {
        return None;
    }
    let rect = rect.intersect(Rect::new(0, 0, width as i32, height as i32))?;
    let (selected_width, selected_height) = (rect.width() as usize, rect.height() as usize);
    let stride = width as usize * 4;
    let mut out = Vec::with_capacity(selected_width * selected_height * 4);
    for y in rect.top as usize..rect.bottom as usize {
        let start = y * stride + rect.left as usize * 4;
        out.extend_from_slice(&rgba[start..start + selected_width * 4]);
    }
    Some((selected_width as u32, selected_height as u32, out))
}

/// Windows' 32-bit DIBs are BGRA with an alpha byte that `PrintWindow` and
/// `BitBlt` usually leave at zero; the stored image needs opaque RGBA.
pub fn bgra_to_rgba(pixels: &[u8]) -> Vec<u8> {
    let mut rgba = Vec::with_capacity(pixels.len());
    for pixel in pixels.chunks_exact(4) {
        rgba.extend_from_slice(&[pixel[2], pixel[1], pixel[0], 255]);
    }
    rgba
}

/// True when a rendered window is the empty (all-black) bitmap that
/// `PrintWindow` returns for hardware-accelerated or protected windows.
pub fn looks_blank(rgba: &[u8]) -> bool {
    rgba.chunks_exact(4)
        .step_by(BLANK_SAMPLE_STRIDE)
        .all(|pixel| pixel[0] == 0 && pixel[1] == 0 && pixel[2] == 0)
}

/// Hides the app, lets the user drag a rectangle over the virtual screen and
/// returns it as an image.
///
/// `Ok(None)` means the capture was cancelled or timed out, which is not an error.
#[cfg(windows)]
pub fn capture_region(app: &AppHandle) -> Result<Option<ClipContent>> {
    let _busy = Busy::acquire()?;
    window::hide(app);
    let result = region_flow();
    window::show(app);
    result
}

/// Hides the app, highlights the window under the cursor and captures the one
/// that is clicked.
#[cfg(windows)]
pub fn capture_window(app: &AppHandle) -> Result<Option<ClipContent>> {
    let _busy = Busy::acquire()?;
    window::hide(app);
    let result = window_flow();
    window::show(app);
    result
}

/// Hides the app, lets the user pick a window, scrolls it page by page and
/// returns the stitched result.
#[cfg(windows)]
pub fn capture_scrolling(app: &AppHandle) -> Result<Option<ClipContent>> {
    let _busy = Busy::acquire()?;
    window::hide(app);
    let result = scrolling_flow();
    window::show(app);
    result
}

/// Hides the app and captures every monitor as one image, the way the system
/// screenshot tool's fullscreen mode does.
#[cfg(windows)]
pub fn capture_screen(app: &AppHandle) -> Result<Option<ClipContent>> {
    let _busy = Busy::acquire()?;
    window::hide(app);
    let result = screen_flow();
    window::show(app);
    result
}

/// Non-Windows builds have no screen to capture.
#[cfg(not(windows))]
pub fn capture_region(_app: &AppHandle) -> Result<Option<ClipContent>> {
    Err(AppError::msg(
        "capturing a screen region is only supported on Windows",
    ))
}

#[cfg(not(windows))]
pub fn capture_window(_app: &AppHandle) -> Result<Option<ClipContent>> {
    Err(AppError::msg(
        "capturing a window is only supported on Windows",
    ))
}

#[cfg(not(windows))]
pub fn capture_screen(_app: &AppHandle) -> Result<Option<ClipContent>> {
    Err(AppError::msg(
        "capturing the screen is only supported on Windows",
    ))
}

#[cfg(not(windows))]
pub fn capture_scrolling(_app: &AppHandle) -> Result<Option<ClipContent>> {
    Err(AppError::msg(
        "scrolling captures are only supported on Windows",
    ))
}

/// Only one overlay can own the mouse and the shared drag state at a time.
#[cfg(windows)]
static CAPTURING: AtomicBool = AtomicBool::new(false);

#[cfg(windows)]
struct Busy;

#[cfg(windows)]
impl Busy {
    fn acquire() -> Result<Self> {
        if CAPTURING
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return Err(AppError::msg("a screen capture is already in progress"));
        }
        Ok(Self)
    }
}

#[cfg(windows)]
impl Drop for Busy {
    fn drop(&mut self) {
        CAPTURING.store(false, Ordering::SeqCst);
    }
}

#[cfg(windows)]
fn region_flow() -> Result<Option<ClipContent>> {
    std::thread::sleep(HIDE_DELAY);
    // The crosshair goes up before the snapshot: reading the screen can take a
    // moment on a large desktop, and the cursor should already say what is coming.
    let _cursor = PickerCursor::install();
    let bounds = virtual_bounds();
    // Freeze the pixels before the overlay exists, so the stored image is exactly
    // what the user aimed at and can never contain the overlay itself.
    let (snapshot_width, snapshot_height, snapshot) = screen_pixels(bounds)?;
    let overlay = Dimmer::show(bounds)
        .ok_or_else(|| AppError::msg("could not open the capture overlay"))?;

    let deadline = Instant::now() + CAPTURE_TIMEOUT;
    let selected = loop {
        picker::pump();
        if picker::escape_pressed() || Instant::now() > deadline {
            break None;
        }
        if overlay.released() {
            break overlay.selection();
        }
        std::thread::sleep(POLL_INTERVAL);
    };
    drop(overlay);

    let Some(rect) = selected else {
        return Ok(None);
    };
    let (width, height, rgba) = crop(&snapshot, snapshot_width, snapshot_height, rect)
        .ok_or_else(|| AppError::msg("the selection does not fit the screen"))?;
    Ok(Some(ClipContent::Image {
        width,
        height,
        rgba,
    }))
}

#[cfg(windows)]
fn window_flow() -> Result<Option<ClipContent>> {
    let Some(target) = pick_window()? else {
        return Ok(None);
    };
    capture_window_pixels(&target).map(Some)
}

/// Runs the picker overlay until a window is clicked, and hands back what was
/// chosen. Shared by the window capture and the scrolling capture.
#[cfg(windows)]
fn pick_window() -> Result<Option<Target>> {
    std::thread::sleep(HIDE_DELAY);
    // A hand, not the sampling crosshair: the user is choosing a window.
    let _cursor = PickerCursor::install_hand();
    let mut highlight = Highlight::show()
        .ok_or_else(|| AppError::msg("could not open the capture overlay"))?;
    // Swallowing the click keeps the window under the cursor from reacting to the
    // selection, exactly as the colour picker does.
    let blocker = MouseBlocker::install();

    let deadline = Instant::now() + CAPTURE_TIMEOUT;
    let target = loop {
        picker::pump();
        if picker::escape_pressed() || Instant::now() > deadline {
            break None;
        }
        highlight.follow();
        let clicked = MouseBlocker::clicked() || (blocker.is_none() && picker::left_pressed());
        if clicked {
            break match MouseBlocker::click_point() {
                // Use the point the hook saw, not where the cursor drifted to.
                Some((x, y)) => highlight.target_at(x, y),
                None => highlight.target().cloned(),
            };
        }
        std::thread::sleep(POLL_INTERVAL);
    };
    drop(blocker);
    drop(highlight);

    Ok(target)
}

/// Picks a window, scrolls it a page at a time, and stitches the frames into one
/// tall image.
///
/// Anything that stops making sense — the window changing size, a frame that does
/// not line up, the page reaching its end, Escape — ends the run and keeps what has
/// been stitched so far, because a partial screenshot is still a screenshot.
#[cfg(windows)]
fn scrolling_flow() -> Result<Option<ClipContent>> {
    let Some(target) = pick_window()? else {
        return Ok(None);
    };

    // The wheel only reaches the window that has focus, and only under the pointer.
    if !crate::paste::activate(target.hwnd.0 as isize) {
        return Err(AppError::msg(
            "that window could not be focused, so it cannot be scrolled",
        ));
    }
    sleep_pumping(SETTLE_DELAY);
    let centre = (
        (target.rect.left + target.rect.right) / 2,
        (target.rect.top + target.rect.bottom) / 2,
    );
    // SAFETY: moving the pointer is harmless, and the overlay has already closed.
    unsafe {
        win32::SetCursorPos(centre.0, centre.1).ok();
    }
    std::thread::sleep(POLL_INTERVAL);

    // A frame taken before the page has repainted would look like "nothing moved",
    // so the first reading waits for two identical frames in a row.
    let Some((width, height, current)) = settled_screen(&target, &[]) else {
        return Err(AppError::msg(
            "could not read the window from the screen for a scrolling capture",
        ));
    };
    log::info!(
        "scrolling capture of {}: {width}x{height} to start",
        if target.title.is_empty() { "an untitled window".to_string() } else { format!("\"{}\"", target.title) }
    );

    let _mouse = MouseBlocker::install();
    let _keys = KeyboardBlocker::install();
    let toast = Toast::show(target.rect);
    if toast.is_none() {
        log::warn!("could not show the capture banner");
    }
    let _toast = toast;

    // The keyboard hook is the only witness of an Escape press: the key it swallows
    // never reaches the async key state that the region and window pickers poll.
    let escaped = || KeyboardBlocker::escaped();
    let (width, total, stitched) = scroll_pages(
        (width, height, current),
        |step, current| advance_page(&target, centre, current, step, &escaped),
        &escaped,
    );

    Ok(Some(ClipContent::Image {
        width,
        height: total,
        rgba: stitched,
    }))
}

/// How one attempt to scroll the page turned out.
#[cfg(windows)]
enum Advance {
    /// The page moved, and this is the frame once it had come to rest.
    Moved(u32, u32, Vec<u8>),
    /// Nothing moved the window, which is what the end of the page looks like.
    End,
    /// The window could not be focused, so the wheel would have gone elsewhere.
    Unfocused,
}

/// Asks a focused window to scroll and reads the frame it settles on.
///
/// Each reading has to differ from the frame before it: a window that has not moved
/// yet is indistinguishable from the bottom of the page, so the three ways of
/// scrolling are tried before the end is called. Escape is checked between the
/// attempts, so a press ends the step it lands in instead of after the last one.
#[cfg(windows)]
fn advance_page(
    target: &Target,
    centre: (i32, i32),
    current: &[u8],
    step: u32,
    escaped: &dyn Fn() -> bool,
) -> Advance {
    // Anything that steals focus between steps would eat the wheel, so the
    // target is re-asserted before every burst.
    if !crate::paste::activate(target.hwnd.0 as isize) {
        return Advance::Unfocused;
    }

    let sent = scroll_window(centre);
    let mut frame = settled_screen(target, current);
    if frame.is_none() && !escaped() {
        log::info!(
            "scroll {step}: the wheel was accepted {sent}/2 and the window did not move; posting the message"
        );
        post_wheel(target.hwnd, centre);
        frame = settled_screen(target, current);
    }
    if frame.is_none() && !escaped() {
        log::info!("scroll {step}: the message did not move it either; pressing Page Down");
        page_down(target, centre);
        frame = settled_screen(target, current);
    }

    match frame {
        Some((width, height, rgba)) => Advance::Moved(width, height, rgba),
        None => Advance::End,
    }
}

/// Runs the scroll loop, stitching each settled frame onto the last, and hands back
/// the image captured so far.
///
/// `advance` performs one scroll attempt; `escaped` reports a cancellation, and is
/// checked before every attempt and again after each one, so a press stops the run
/// without waiting for the page to end.
#[cfg(windows)]
fn scroll_pages(
    start: (u32, u32, Vec<u8>),
    mut advance: impl FnMut(u32, &[u8]) -> Advance,
    escaped: &dyn Fn() -> bool,
) -> (u32, u32, Vec<u8>) {
    let (width, height, mut current) = start;
    let row_bytes = width as usize * 4;
    let mut stitched = current.clone();
    let mut total = height;
    // Which columns belong to the window and not to the page is decided from the
    // first two frames, then held for the rest of the capture.
    let mut frozen: Option<Vec<bool>> = None;

    for step in 0..MAX_SCROLLS {
        // The hooks are delivered to this thread, so its queue has to be drained.
        picker::pump();
        if escaped() {
            log::info!("scrolling stopped after {step} step(s): Esc was pressed");
            break;
        }
        if total >= MAX_SCROLL_HEIGHT {
            log::info!("scrolling stopped: {total} px is the ceiling");
            break;
        }

        let next = advance(step, &current);
        if escaped() {
            log::info!("scrolling stopped after {step} step(s): Esc was pressed");
            break;
        }
        let (next_width, next_height, next) = match next {
            Advance::Moved(frame_width, frame_height, rgba) => (frame_width, frame_height, rgba),
            Advance::End => {
                log::info!("scrolling stopped after {step} step(s): the page reached its end");
                break;
            }
            Advance::Unfocused => {
                log::info!("scrolling stopped: the window is no longer focusable");
                break;
            }
        };
        if next_width != width || next_height != height {
            log::info!("scrolling stopped: the window changed size to {next_width}x{next_height}");
            break;
        }

        let Some(offset) = measure(&current, (next_width, next_height, next.clone())) else {
            log::info!("scroll {step}: the frame moved but did not line up with the one before");
            // Written next to the log so a failure can be looked at, not guessed at.
            dump_frame("scroll-after", next_width, next_height, &next);
            dump_frame("scroll-before", width, height, &current);
            break;
        };

        if frozen.is_none() {
            frozen = Some(crate::stitch::frozen_columns(
                &current,
                &next,
                width,
                height,
                FROZEN_COLUMN_TOLERANCE,
            ));
        }

        stitched.extend_from_slice(&next[(next_height - offset) as usize * row_bytes..]);
        total += offset;
        current = next;
        log::info!("scroll {step}: the page moved {offset} rows, {total} px so far");
    }

    // A fixed sidebar is in every frame, so without this it would scroll down the
    // stitched image beside content it does not belong to.
    if let Some(frozen) = &frozen {
        crate::stitch::hold_frozen_columns(&mut stitched, width, height, frozen);
        log::info!(
            "held {} fixed column(s) below the fold",
            frozen.iter().filter(|f| **f).count()
        );
    }

    (width, total, stitched)
}

/// Drops the window's bottom edge rows, so a join cannot repeat them.
#[cfg(windows)]
fn trim_bottom(width: u32, height: u32, mut rgba: Vec<u8>) -> (u32, u32, Vec<u8>) {
    if height <= FRAME_BOTTOM_TRIM || width == 0 {
        return (width, height, rgba);
    }
    let keep = (height - FRAME_BOTTOM_TRIM) as usize * width as usize * 4;
    rgba.truncate(keep);
    (width, height - FRAME_BOTTOM_TRIM, rgba)
}

/// Sleeps while keeping the thread's message queue drained.
///
/// The input hooks and the banner are this thread's windows, and Windows delivers
/// hook callbacks through its queue: a plain sleep would stall both, letting keys
/// through and leaving the banner unpainted.
#[cfg(windows)]
fn sleep_pumping(total: Duration) {
    let deadline = Instant::now() + total;
    loop {
        picker::pump();
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        std::thread::sleep(left.min(POLL_INTERVAL));
    }
}

/// Reads the window's rectangle from the desktop.
///
/// The screen is the only source that is never stale: `PrintWindow` renders from
/// the window's backing store, which a compositing browser can leave a frame or
/// two behind the pixels the user is actually looking at.
#[cfg(windows)]
fn screen_frame(target: &Target) -> Option<(u32, u32, Vec<u8>)> {
    match screen_pixels(target.rect) {
        Ok((width, height, rgba)) => Some(trim_bottom(width, height, rgba)),
        Err(e) => {
            log::warn!("could not read the window from the screen: {e}");
            None
        }
    }
}

/// Reads the window from the desktop until it has moved and then stopped.
///
/// Waiting for the frame to change first is what keeps a read taken before the
/// page starts moving from being mistaken for a settled one; two identical reads
/// in a row then mean the scroll animation has finished. `None` means the window
/// never moved, which is what the bottom of the page looks like.
#[cfg(windows)]
fn settled_screen(target: &Target, previous: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let deadline = Instant::now() + SCROLL_TIMEOUT;
    let mut last: Option<(u32, u32, Vec<u8>)> = None;

    while Instant::now() < deadline {
        sleep_pumping(SCROLL_POLL);

        let Some((width, height, rgba)) = screen_frame(target) else {
            continue;
        };
        if crate::stitch::frames_identical(previous, &rgba) {
            continue;
        }
        if let Some((last_width, last_height, last_rgba)) = &last
            && *last_width == width
            && *last_height == height
            && *last_rgba == rgba
        {
            return Some((width, height, rgba));
        }
        last = Some((width, height, rgba));
    }

    last
}

/// Saves a frame beside the log, so a capture that cannot be stitched can be
/// looked at instead of guessed about.
#[cfg(windows)]
fn dump_frame(name: &str, width: u32, height: u32, rgba: &[u8]) {
    let path = std::env::temp_dir().join(format!("copypaste-{name}.png"));
    match image::save_buffer(&path, rgba, width, height, image::ColorType::Rgba8) {
        Ok(()) => log::info!("wrote {} for diagnosis", path.display()),
        Err(e) => log::warn!("could not write {}: {e}", path.display()),
    }
}

/// How far the page moved between two frames of the same window.
#[cfg(windows)]
fn measure(previous: &[u8], frame: (u32, u32, Vec<u8>)) -> Option<u32> {
    let (width, height, rgba) = frame;
    crate::stitch::scroll_offset(previous, &rgba, width, height)
}

/// Scrolls a focused window by a page with the keyboard, for windows that ignore
/// both the wheel and the wheel message.
#[cfg(windows)]
fn page_down(_target: &Target, centre: (i32, i32)) {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput, VK_NEXT,
    };

    // The pointer has to be over the window for the key to land where the wheel
    // would have; focus is already on the target.
    // SAFETY: moving the pointer is harmless.
    unsafe {
        win32::SetCursorPos(centre.0, centre.1).ok();
    }

    let press = |key_up: bool| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VK_NEXT,
                wScan: 0,
                dwFlags: if key_up { KEYEVENTF_KEYUP } else { Default::default() },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };

    // SAFETY: two correctly sized INPUT values, no pointers to keep alive.
    unsafe {
        SendInput(&[press(false), press(true)], std::mem::size_of::<INPUT>() as i32);
    }
    sleep_pumping(POINTER_SETTLE);
}

#[cfg(windows)]
fn scroll_window(centre: (i32, i32)) -> u32 {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_MOVE, MOUSEEVENTF_WHEEL,
        MOUSEINPUT, SendInput,
    };

    // Absolute coordinates are normalised over the whole virtual desktop.
    // SAFETY: the metrics are pure reads.
    let (left, top, width, height) = unsafe {
        (
            win32::GetSystemMetrics(win32::SM_XVIRTUALSCREEN),
            win32::GetSystemMetrics(win32::SM_YVIRTUALSCREEN),
            win32::GetSystemMetrics(win32::SM_CXVIRTUALSCREEN).max(1),
            win32::GetSystemMetrics(win32::SM_CYVIRTUALSCREEN).max(1),
        )
    };
    let normalise = |value: i32, origin: i32, span: i32| -> i32 {
        (((value - origin) as i64 * 65_535) / (span as i64 - 1).max(1)) as i32
    };

    let inputs = [
        INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx: normalise(centre.0, left, width),
                    dy: normalise(centre.1, top, height),
                    mouseData: 0,
                    dwFlags: MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        },
        INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx: 0,
                    dy: 0,
                    mouseData: (-WHEEL_DELTA * SCROLL_NOTCHES) as u32,
                    dwFlags: MOUSEEVENTF_WHEEL,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        },
    ];

    // SAFETY: two correctly sized INPUT values, no pointers to keep alive.
    let sent = unsafe {
        let sent = SendInput(&inputs, std::mem::size_of::<INPUT>() as i32);
        win32::SetCursorPos(centre.0, centre.1).ok();
        sent
    };
    sleep_pumping(POINTER_SETTLE);
    sent
}

/// Posts a wheel message straight to the window, for windows that never see the
/// synthetic device input.
#[cfg(windows)]
fn post_wheel(hwnd: windows::Win32::Foundation::HWND, centre: (i32, i32)) {
    use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_MOUSEWHEEL};

    let delta = -WHEEL_DELTA * SCROLL_NOTCHES;
    let key_state = 0u32; // no modifiers held
    let point = ((centre.1 as u32 & 0xFFFF) << 16) | (centre.0 as u32 & 0xFFFF);
    let wparam = ((delta as u16 as u32) << 16) | key_state;

    // SAFETY: the window handle was validated by the picker, and the message
    // carries no pointers.
    unsafe {
        PostMessageW(
            Some(hwnd),
            WM_MOUSEWHEEL,
            windows::Win32::Foundation::WPARAM(wparam as usize),
            windows::Win32::Foundation::LPARAM(point as isize),
        )
        .ok();
    }
    sleep_pumping(POINTER_SETTLE);
}

/// The whole desktop, with no overlay to aim: the same snapshot the region
/// capture starts from, kept entire.
#[cfg(windows)]
fn screen_flow() -> Result<Option<ClipContent>> {
    std::thread::sleep(HIDE_DELAY);
    let (width, height, rgba) = screen_pixels(virtual_bounds())?;
    Ok(Some(ClipContent::Image {
        width,
        height,
        rgba,
    }))
}

/// Reads the whole virtual screen at once; the region capture crops its selection
/// out of this snapshot.
#[cfg(windows)]
fn screen_pixels(rect: Rect) -> Result<(u32, u32, Vec<u8>)> {
    let (width, height) = (rect.width(), rect.height());
    let rgba = capture_with(width, height, |memory, screen| unsafe {
        win32::BitBlt(
            memory,
            0,
            0,
            width,
            height,
            Some(screen),
            rect.left,
            rect.top,
            win32::SRCCOPY,
        )
        .is_ok()
    })?
    .ok_or_else(|| AppError::msg("could not read that part of the screen"))?;
    Ok((width as u32, height as u32, rgba))
}

/// Renders a window off-screen with `PrintWindow`, falling back to the pixels
/// actually on screen when the window refuses to paint itself.
#[cfg(windows)]
fn capture_window_pixels(target: &Target) -> Result<ClipContent> {
    // `PrintWindow` renders the whole window rectangle, invisible border and all,
    // so the visible part is cropped back out of it afterwards.
    let (width, height) = (target.frame.width(), target.frame.height());
    let inset = visible_inset(target.frame, target.rect);
    if let Some(rgba) = render_window(target.hwnd, width, height)?
        && let Some((width, height, cropped)) = crop(&rgba, width as u32, height as u32, inset)
        && !looks_blank(&cropped)
    {
        return Ok(ClipContent::Image {
            width,
            height,
            rgba: cropped,
        });
    }

    let visible = target
        .rect
        .intersect(virtual_bounds())
        .ok_or_else(|| AppError::msg("that window is not on screen"))?;
    log::debug!("fell back to the screen for window `{}`", target.title);
    let (width, height, rgba) = screen_pixels(visible)?;
    Ok(ClipContent::Image {
        width,
        height,
        rgba,
    })
}

#[cfg(windows)]
fn render_window(
    hwnd: win32::HWND,
    width: i32,
    height: i32,
) -> Result<Option<Vec<u8>>> {
    capture_with(width, height, |memory, _| unsafe {
        win32::PrintWindow(
            hwnd,
            memory,
            win32::PRINT_WINDOW_FLAGS(win32::PW_RENDERFULLCONTENT),
        )
        .as_bool()
    })
}

/// Creates a top-down 32-bit DIB, lets `draw` paint into it and hands back RGBA.
#[cfg(windows)]
fn capture_with(
    width: i32,
    height: i32,
    draw: impl FnOnce(win32::HDC, win32::HDC) -> bool,
) -> Result<Option<Vec<u8>>> {
    let pixels = checked_pixels(width, height)?;
    unsafe {
        let screen = win32::GetDC(None);
        if screen.is_invalid() {
            return Err(AppError::msg("could not read the screen"));
        }
        let memory = win32::CreateCompatibleDC(Some(screen));
        if memory.is_invalid() {
            win32::ReleaseDC(None, screen);
            return Err(AppError::msg("could not prepare a capture surface"));
        }

        let info = win32::BITMAPINFO {
            bmiHeader: win32::BITMAPINFOHEADER {
                biSize: std::mem::size_of::<win32::BITMAPINFOHEADER>() as u32,
                biWidth: width,
                // Negative height keeps the rows in the order the RGBA buffer wants.
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: win32::BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits: *mut c_void = std::ptr::null_mut();
        let bitmap = match win32::CreateDIBSection(
            Some(memory),
            &info,
            win32::DIB_RGB_COLORS,
            &mut bits,
            None,
            0,
        ) {
            Ok(bitmap) => bitmap,
            Err(e) => {
                let _ = win32::DeleteDC(memory);
                win32::ReleaseDC(None, screen);
                return Err(e.into());
            }
        };
        let old = win32::SelectObject(memory, bitmap.into());
        let drawn = draw(memory, screen);
        let rgba = if drawn && !bits.is_null() {
            let bytes = std::slice::from_raw_parts(bits as *const u8, pixels * 4);
            Some(bgra_to_rgba(bytes))
        } else {
            None
        };
        win32::SelectObject(memory, old);
        let _ = win32::DeleteObject(bitmap.into());
        let _ = win32::DeleteDC(memory);
        win32::ReleaseDC(None, screen);
        Ok(rgba)
    }
}

#[cfg(windows)]
fn virtual_bounds() -> Rect {
    unsafe {
        let left = win32::GetSystemMetrics(win32::SM_XVIRTUALSCREEN);
        let top = win32::GetSystemMetrics(win32::SM_YVIRTUALSCREEN);
        let width = win32::GetSystemMetrics(win32::SM_CXVIRTUALSCREEN);
        let height = win32::GetSystemMetrics(win32::SM_CYVIRTUALSCREEN);
        Rect::new(left, top, left + width, top + height)
    }
}

/// The selection being dragged, in coordinates local to the overlay. Only one
/// overlay exists at a time, so module state avoids threading a pointer through
/// the window procedure.
#[cfg(windows)]
#[derive(Clone, Copy)]
struct Drag {
    start: Option<(i32, i32)>,
    released: bool,
}

#[cfg(windows)]
static DRAG: Mutex<Drag> = Mutex::new(Drag {
    start: None,
    released: false,
});

/// The current selection, in coordinates local to the overlay.
#[cfg(windows)]
static SELECTION: Mutex<Option<Rect>> = Mutex::new(None);

/// Handle of the small frame that draws the border and the size chip.
#[cfg(windows)]
static FRAME: AtomicIsize = AtomicIsize::new(0);

#[cfg(windows)]
fn drag() -> MutexGuard<'static, Drag> {
    DRAG.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(windows)]
fn selection() -> MutexGuard<'static, Option<Rect>> {
    SELECTION.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(windows)]
fn frame_window() -> Option<win32::HWND> {
    let handle = FRAME.load(Ordering::Relaxed);
    (handle != 0).then_some(win32::HWND(handle as *mut c_void))
}

/// The full-screen dimmer that owns the drag.
#[cfg(windows)]
struct Dimmer {
    hwnd: win32::HWND,
    frame: win32::HWND,
}

#[cfg(windows)]
impl Dimmer {
    fn show(bounds: Rect) -> Option<Self> {
        unsafe {
            register_classes();
            let module = win32::GetModuleHandleW(None).ok()?;
            let (width, height) = (bounds.width(), bounds.height());
            *drag() = Drag {
                start: None,
                released: false,
            };
            *selection() = None;

            let hwnd = win32::CreateWindowExW(
                win32::WS_EX_TOPMOST
                    | win32::WS_EX_TOOLWINDOW
                    | win32::WS_EX_NOACTIVATE
                    | win32::WS_EX_LAYERED,
                windows::core::w!("CopyPasteCaptureDimmer"),
                windows::core::w!("CopyPasteCaptureDimmer"),
                win32::WS_POPUP,
                bounds.left,
                bounds.top,
                width,
                height,
                None,
                None,
                Some(win32::HINSTANCE(module.0)),
                None,
            )
            .ok()?;
            let _ = win32::SetLayeredWindowAttributes(
                hwnd,
                win32::COLORREF(0),
                110,
                win32::LWA_ALPHA,
            );

            let frame = match win32::CreateWindowExW(
                win32::WS_EX_TOPMOST
                    | win32::WS_EX_TOOLWINDOW
                    | win32::WS_EX_NOACTIVATE
                    | win32::WS_EX_TRANSPARENT,
                windows::core::w!("CopyPasteCaptureFrame"),
                windows::core::w!("CopyPasteCaptureFrame"),
                win32::WS_POPUP,
                0,
                0,
                1,
                1,
                None,
                None,
                Some(win32::HINSTANCE(module.0)),
                None,
            ) {
                Ok(frame) => frame,
                Err(_) => {
                    let _ = win32::DestroyWindow(hwnd);
                    return None;
                }
            };
            FRAME.store(frame.0 as isize, Ordering::Relaxed);

            let _ = win32::ShowWindow(hwnd, win32::SW_SHOW);
            // Showing a topmost window does not guarantee it is above the taskbar,
            // so raise it explicitly while leaving focus where it is.
            let _ = win32::SetWindowPos(
                hwnd,
                Some(win32::HWND_TOPMOST),
                bounds.left,
                bounds.top,
                width,
                height,
                win32::SWP_NOACTIVATE | win32::SWP_SHOWWINDOW,
            );
            Some(Self { hwnd, frame })
        }
    }

    fn released(&self) -> bool {
        drag().released
    }

    fn selection(&self) -> Option<Rect> {
        *selection()
    }
}

#[cfg(windows)]
impl Drop for Dimmer {
    fn drop(&mut self) {
        FRAME.store(0, Ordering::Relaxed);
        unsafe {
            let _ = win32::DestroyWindow(self.frame);
            let _ = win32::DestroyWindow(self.hwnd);
        }
    }
}

/// Shown while a scrolling capture is running: the desktop is frozen, so the
/// banner is the only thing that explains why nothing responds.
#[cfg(windows)]
const TOAST_TEXT: &str = "Capturing scroll — press Esc to end";

/// Banner size in pixels; wide enough for the text at the default font.
#[cfg(windows)]
const TOAST: (i32, i32) = (330, 34);

/// The banner that explains a scrolling capture is in progress.
#[cfg(windows)]
struct Toast {
    hwnd: win32::HWND,
}

#[cfg(windows)]
impl Toast {
    /// Shows the banner just outside `target`, so it can never end up in a frame.
    fn show(target: Rect) -> Option<Self> {
        unsafe {
            register_classes();
            let module = win32::GetModuleHandleW(None).ok()?;
            let hwnd = win32::CreateWindowExW(
                win32::WS_EX_TOPMOST
                    | win32::WS_EX_TOOLWINDOW
                    | win32::WS_EX_NOACTIVATE
                    | win32::WS_EX_TRANSPARENT,
                windows::core::w!("CopyPasteCaptureToast"),
                windows::core::w!("CopyPasteCaptureToast"),
                win32::WS_POPUP,
                0,
                0,
                TOAST.0,
                TOAST.1,
                None,
                None,
                Some(win32::HINSTANCE(module.0)),
                None,
            )
            .ok()?;

            let screen = virtual_bounds();
            let x = ((target.left + target.right) / 2 - TOAST.0 / 2)
                .clamp(screen.left, (screen.right - TOAST.0).max(screen.left));
            // Above the window if there is room, otherwise below it, and only as a
            // last resort on top of it — a banner inside the window would be
            // captured at the top of every frame.
            let above = target.top - TOAST.1 - 8;
            let below = target.bottom + 8;
            let y = if above >= screen.top {
                above
            } else if below + TOAST.1 <= screen.bottom {
                below
            } else {
                (target.top + 8).clamp(screen.top, (screen.bottom - TOAST.1).max(screen.top))
            };

            let placed = win32::SetWindowPos(
                hwnd,
                Some(win32::HWND_TOPMOST),
                x,
                y,
                TOAST.0,
                TOAST.1,
                win32::SWP_NOACTIVATE | win32::SWP_SHOWWINDOW,
            );
            let _ = win32::ShowWindow(hwnd, win32::SW_SHOWNOACTIVATE);
            log::info!(
                "capture banner at {x},{y} (window {}..{}), placed {placed:?}",
                target.top,
                target.bottom
            );
            Some(Self { hwnd })
        }
    }
}

#[cfg(windows)]
impl Drop for Toast {
    fn drop(&mut self) {
        unsafe {
            let _ = win32::DestroyWindow(self.hwnd);
        }
    }
}

/// The border drawn around the window under the cursor.
#[cfg(windows)]
#[derive(Clone)]
struct Target {
    hwnd: win32::HWND,
    /// What `PrintWindow` renders: the window rectangle Windows reports.
    frame: Rect,
    /// What is actually on screen. Windows 10 and 11 wrap every window in an
    /// invisible resize border, and DWM's extended frame bounds are the only
    /// source for the part a user would call "the window".
    rect: Rect,
    title: String,
}

#[cfg(windows)]
struct Highlight {
    hwnd: win32::HWND,
    target: Option<Target>,
}

#[cfg(windows)]
impl Highlight {
    fn show() -> Option<Self> {
        unsafe {
            register_classes();
            let module = win32::GetModuleHandleW(None).ok()?;
            let hwnd = win32::CreateWindowExW(
                win32::WS_EX_TOPMOST
                    | win32::WS_EX_TOOLWINDOW
                    | win32::WS_EX_NOACTIVATE
                    | win32::WS_EX_TRANSPARENT,
                windows::core::w!("CopyPasteCaptureHighlight"),
                windows::core::w!("CopyPasteCaptureHighlight"),
                win32::WS_POPUP,
                0,
                0,
                1,
                1,
                None,
                None,
                Some(win32::HINSTANCE(module.0)),
                None,
            )
            .ok()?;
            Some(Self {
                hwnd,
                target: None,
            })
        }
    }

    /// Moves the border to whatever is under the cursor, hiding it when that is
    /// not a window the user could mean.
    fn follow(&mut self) {
        unsafe {
            let mut point = win32::POINT::default();
            if win32::GetCursorPos(&mut point).is_err() {
                self.clear();
                return;
            }
            match self.target_at(point.x, point.y) {
                Some(target) => self.place(&target),
                None => self.clear(),
            }
        }
    }

    fn target(&self) -> Option<&Target> {
        self.target.as_ref()
    }

    /// Resolves the top-level window at a screen point, skipping our own windows,
    /// the shell and anything with no name or area.
    fn target_at(&self, x: i32, y: i32) -> Option<Target> {
        unsafe {
            let hit = self.hit_test(win32::POINT { x, y })?;
            let root = win32::GetAncestor(hit, win32::GA_ROOT);
            if root.is_invalid() || root == self.hwnd {
                return None;
            }
            if root == win32::GetShellWindow() || root == win32::GetDesktopWindow() {
                return None;
            }
            let mut process = 0u32;
            win32::GetWindowThreadProcessId(root, Some(&mut process));
            if process == win32::GetCurrentProcessId() {
                return None;
            }
            if !win32::IsWindowVisible(root).as_bool() || win32::IsIconic(root).as_bool() {
                return None;
            }
            let mut raw = win32::RECT::default();
            win32::GetWindowRect(root, &mut raw).ok()?;
            let frame = Rect::new(raw.left, raw.top, raw.right, raw.bottom);
            let rect = visible_frame(root, frame);
            let title = window_title(root);
            is_capture_target(&title, rect).then_some(Target {
                hwnd: root,
                frame,
                rect,
                title,
            })
        }
    }

    /// `WindowFromPoint` may return the highlight itself; giving it an empty
    /// region removes it from that one lookup without flickering the border.
    fn hit_test(&self, point: win32::POINT) -> Option<win32::HWND> {
        unsafe {
            let hit = win32::WindowFromPoint(point);
            if hit != self.hwnd {
                return (!hit.is_invalid()).then_some(hit);
            }
            let empty = win32::CreateRectRgn(0, 0, 0, 0);
            let _ = win32::SetWindowRgn(self.hwnd, Some(empty), false);
            let under = win32::WindowFromPoint(point);
            self.restore_region();
            (!under.is_invalid() && under != self.hwnd).then_some(under)
        }
    }

    fn restore_region(&self) {
        unsafe {
            let mut client = win32::RECT::default();
            if win32::GetClientRect(self.hwnd, &mut client).is_err() {
                return;
            }
            let (width, height) = (client.right, client.bottom);
            if width <= 0 || height <= 0 {
                return;
            }
            let _ = win32::SetWindowRgn(self.hwnd, Some(ring_region((width, height))), false);
        }
    }

    fn place(&mut self, target: &Target) {
        // The poll runs many times a second; only touch the window when the
        // highlighted window or its rectangle actually changed.
        if self
            .target
            .as_ref()
            .is_some_and(|current| current.hwnd == target.hwnd && current.rect == target.rect)
        {
            return;
        }
        unsafe {
            let outer = target.rect.inflate(BORDER);
            let (width, height) = (outer.width(), outer.height());
            let _ = win32::SetWindowRgn(self.hwnd, Some(ring_region((width, height))), false);
            let _ = win32::SetWindowPos(
                self.hwnd,
                Some(win32::HWND_TOPMOST),
                outer.left,
                outer.top,
                width,
                height,
                win32::SWP_NOACTIVATE | win32::SWP_SHOWWINDOW,
            );
            let _ = win32::InvalidateRect(Some(self.hwnd), None, true);
            self.target = Some(target.clone());
        }
    }

    fn clear(&mut self) {
        if self.target.take().is_some() {
            unsafe {
                let _ = win32::ShowWindow(self.hwnd, win32::SW_HIDE);
            }
        }
    }
}

#[cfg(windows)]
impl Drop for Highlight {
    fn drop(&mut self) {
        unsafe {
            let _ = win32::DestroyWindow(self.hwnd);
        }
    }
}

/// The part of a window a user would call "the window".
///
/// `GetWindowRect` includes the invisible resize border Windows 10 and 11 put
/// around every window, which is what left a strip of desktop along the left,
/// right and bottom of a window capture. DWM reports the visible frame; when it
/// will not, the reported rectangle is all there is to go on.
#[cfg(windows)]
fn visible_frame(hwnd: win32::HWND, frame: Rect) -> Rect {
    let mut bounds = win32::RECT::default();
    let size = std::mem::size_of::<win32::RECT>() as u32;
    let read = unsafe {
        win32::DwmGetWindowAttribute(
            hwnd,
            win32::DWMWA_EXTENDED_FRAME_BOUNDS,
            std::ptr::from_mut(&mut bounds).cast(),
            size,
        )
        .is_ok()
    };
    if !read {
        return frame;
    }
    let visible = Rect::new(bounds.left, bounds.top, bounds.right, bounds.bottom);
    // Cloaked and just-restored windows can report an empty or larger rectangle;
    // trusting that would move the border off the window.
    if visible.is_empty() || visible.width() > frame.width() || visible.height() > frame.height() {
        return frame;
    }
    visible
}

#[cfg(windows)]
fn window_title(hwnd: win32::HWND) -> String {
    let mut buffer = [0u16; 256];
    let length = unsafe { win32::GetWindowTextW(hwnd, &mut buffer) };
    let length = length.clamp(0, buffer.len() as i32) as usize;
    String::from_utf16_lossy(&buffer[..length])
}

/// A region with `hole` removed: the dimmer's opening and the highlight's ring.
#[cfg(windows)]
fn selection_region(size: (i32, i32), hole: Rect) -> win32::HRGN {
    unsafe {
        let outer = win32::CreateRectRgn(0, 0, size.0, size.1);
        let inner = win32::CreateRectRgn(hole.left, hole.top, hole.right, hole.bottom);
        let _ = win32::CombineRgn(Some(outer), Some(outer), Some(inner), win32::RGN_DIFF);
        let _ = win32::DeleteObject(inner.into());
        outer
    }
}

/// The frame's border, i.e. its client area minus the captured rectangle.
#[cfg(windows)]
fn ring_region(size: (i32, i32)) -> win32::HRGN {
    selection_region(
        size,
        Rect::new(
            BORDER,
            BORDER,
            (size.0 - BORDER).max(BORDER),
            (size.1 - BORDER).max(BORDER),
        ),
    )
}

/// The frame's region keeps the captured area clear but adds the size chip back,
/// which would otherwise be clipped away with the rest of the opening.
#[cfg(windows)]
fn frame_region(size: (i32, i32)) -> win32::HRGN {
    let region = ring_region(size);
    let chip = chip_rect(size);
    if !chip.is_empty() {
        unsafe {
            let chip_region =
                win32::CreateRectRgn(chip.left, chip.top, chip.right, chip.bottom);
            let _ = win32::CombineRgn(Some(region), Some(region), Some(chip_region), win32::RGN_OR);
            let _ = win32::DeleteObject(chip_region.into());
        }
    }
    region
}

/// The chip showing the size, in the frame window's client coordinates.
#[cfg(windows)]
fn chip_rect(size: (i32, i32)) -> Rect {
    let selection = (size.0 - 2 * BORDER, size.1 - 2 * BORDER);
    Rect::new(
        BORDER + CHIP_MARGIN,
        BORDER + CHIP_MARGIN,
        (BORDER + CHIP_MARGIN + CHIP.0).min(BORDER + selection.0),
        (BORDER + CHIP_MARGIN + CHIP.1).min(BORDER + selection.1),
    )
}

#[cfg(windows)]
fn register_classes() {
    static REGISTER: Once = Once::new();
    REGISTER.call_once(|| unsafe {
        let Ok(module) = win32::GetModuleHandleW(None) else {
            return;
        };
        let class = |name: windows::core::PCWSTR, proc: win32::WNDPROC| win32::WNDCLASSW {
            hInstance: win32::HINSTANCE(module.0),
            lpfnWndProc: proc,
            lpszClassName: name,
            ..Default::default()
        };
        // A zero return is fine: a second overlay may find the class registered.
        let _ = win32::RegisterClassW(&class(
            windows::core::w!("CopyPasteCaptureDimmer"),
            Some(win32::dimmer_proc),
        ));
        let _ = win32::RegisterClassW(&class(
            windows::core::w!("CopyPasteCaptureFrame"),
            Some(win32::frame_proc),
        ));
        let _ = win32::RegisterClassW(&class(
            windows::core::w!("CopyPasteCaptureHighlight"),
            Some(win32::highlight_proc),
        ));
        let _ = win32::RegisterClassW(&class(
            windows::core::w!("CopyPasteCaptureToast"),
            Some(win32::toast_proc),
        ));
    });
}

/// Applies a drag to the dimmer's opening and the frame's border.
#[cfg(windows)]
unsafe fn update_selection(
    dimmer: win32::HWND,
    frame: win32::HWND,
    start: (i32, i32),
    end: (i32, i32),
) {
    unsafe {
        let mut client = win32::RECT::default();
        if win32::GetClientRect(dimmer, &mut client).is_err() {
            return;
        }
        let size = (client.right - client.left, client.bottom - client.top);
        let Some(rect) = selection_rect(start, end, Rect::new(0, 0, size.0, size.1)) else {
            // A press without a drag leaves nothing to capture. The guard is held
            // once: locking `selection()` again inside the block deadlocks.
            let mut current = selection();
            let had_selection = current.is_some();
            *current = None;
            drop(current);
            if had_selection {
                let _ = win32::SetWindowRgn(dimmer, None, true);
            }
            let _ = win32::ShowWindow(frame, win32::SW_HIDE);
            return;
        };
        if *selection() == Some(rect) {
            return;
        }
        *selection() = Some(rect);

        let _ = win32::SetWindowRgn(dimmer, Some(selection_region(size, rect)), true);

        let mut origin = win32::RECT::default();
        if win32::GetWindowRect(dimmer, &mut origin).is_err() {
            return;
        }
        let outer = rect.inflate(BORDER);
        let (width, height) = (outer.width(), outer.height());
        // The region is set before the resize so the frame never flashes as a
        // solid block while it is being moved.
        let _ = win32::SetWindowRgn(frame, Some(frame_region((width, height))), false);
        let _ = win32::SetWindowPos(
            frame,
            Some(win32::HWND_TOPMOST),
            origin.left + outer.left,
            origin.top + outer.top,
            width,
            height,
            win32::SWP_NOACTIVATE | win32::SWP_SHOWWINDOW,
        );
        let _ = win32::InvalidateRect(Some(frame), None, true);
    }
}

#[cfg(windows)]
fn point_of(lparam: win32::LPARAM) -> (i32, i32) {
    let value = lparam.0 as u32;
    (
        (value & 0xffff) as u16 as i16 as i32,
        ((value >> 16) & 0xffff) as u16 as i16 as i32,
    )
}

#[cfg(windows)]
mod win32 {
    pub use windows::Win32::Foundation::{
        COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
    };
    pub use windows::Win32::Graphics::Dwm::{DWMWA_EXTENDED_FRAME_BOUNDS, DwmGetWindowAttribute};
    pub use windows::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BeginPaint, BitBlt, CombineRgn,
        CreateCompatibleDC, CreateDIBSection, CreateRectRgn, CreateSolidBrush, DEFAULT_GUI_FONT,
        DIB_RGB_COLORS, DT_CENTER, DT_SINGLELINE, DT_VCENTER, DeleteDC, DeleteObject, DrawTextW,
        EndPaint, FillRect, GetDC, GetStockObject, HDC, HRGN, InvalidateRect, PAINTSTRUCT,
        RGN_DIFF, RGN_OR, ReleaseDC, SRCCOPY, SelectObject, SetBkMode, SetTextColor, SetWindowRgn,
        TRANSPARENT, TextOutW,
    };
    pub use windows::Win32::Storage::Xps::{PRINT_WINDOW_FLAGS, PrintWindow};
    pub use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    pub use windows::Win32::System::Threading::GetCurrentProcessId;
    pub use windows::Win32::UI::Input::KeyboardAndMouse::{ReleaseCapture, SetCapture};
    pub use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, GA_ROOT, GetAncestor, GetClientRect,
        GetCursorPos, GetDesktopWindow, GetShellWindow, GetSystemMetrics, GetWindowRect,
        GetWindowTextW, GetWindowThreadProcessId, HWND_TOPMOST, IDC_CROSS, IsIconic,
        IsWindowVisible, LoadCursorW, LWA_ALPHA, PW_RENDERFULLCONTENT, RegisterClassW,
        SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SW_HIDE,
        SW_SHOW, SW_SHOWNOACTIVATE, SWP_NOACTIVATE, SWP_SHOWWINDOW, SetCursor, SetCursorPos,
        SetLayeredWindowAttributes,
        SetWindowPos, ShowWindow, WNDCLASSW, WNDPROC, WM_ERASEBKGND, WM_LBUTTONDOWN,
        WM_LBUTTONUP, WM_MOUSEMOVE, WM_PAINT, WM_SETCURSOR, WS_EX_LAYERED, WS_EX_NOACTIVATE,
        WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP, WindowFromPoint,
    };

    /// Pins the crosshair to the overlay itself.
    ///
    /// Swapping the system arrow is not enough here: the overlay is a plain Win32
    /// window created on a worker thread, and Windows is free to show its own
    /// "working" cursor over it, which is what the capture used to display. The
    /// window now answers `WM_SETCURSOR` with the crosshair, so the cursor is right
    /// whatever the rest of the process is doing.
    unsafe fn show_crosshair() {
        if let Ok(cross) = unsafe { LoadCursorW(None, IDC_CROSS) } {
            unsafe { SetCursor(Some(cross)) };
        }
    }

    /// The dimmer only paints its background; the drag is tracked by the
    /// procedure so the selection can be reported back to the capture loop.
    pub unsafe extern "system" fn dimmer_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            WM_SETCURSOR => {
                unsafe { show_crosshair() };
                LRESULT(1)
            }
            WM_PAINT => {
                unsafe { fill_client(hwnd, 0x0010_1010) };
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            WM_LBUTTONDOWN => {
                let point = super::point_of(lparam);
                *super::drag() = super::Drag {
                    start: Some(point),
                    released: false,
                };
                unsafe { SetCapture(hwnd) };
                LRESULT(0)
            }
            WM_MOUSEMOVE => {
                unsafe { show_crosshair() };
                let point = super::point_of(lparam);
                if let Some(start) = super::drag().start
                    && let Some(frame) = super::frame_window()
                {
                    unsafe { super::update_selection(hwnd, frame, start, point) };
                }
                LRESULT(0)
            }
            WM_LBUTTONUP => {
                let point = super::point_of(lparam);
                // One guard for the whole handler: taking `drag()` again inside the
                // `if let` would deadlock this thread on the same lock, which is
                // what froze the overlay before.
                let mut drag = super::drag();
                if let Some(start) = drag.start {
                    if let Some(frame) = super::frame_window() {
                        unsafe { super::update_selection(hwnd, frame, start, point) };
                    }
                    drag.released = true;
                }
                drop(drag);
                unsafe { let _ = ReleaseCapture(); }
                LRESULT(0)
            }
            _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
        }
    }

    /// The frame paints its border and the size chip; the captured area itself is
    /// excluded from its region so the desktop shows through.
    pub unsafe extern "system" fn frame_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            WM_PAINT => {
                unsafe { paint_frame(hwnd) };
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
        }
    }

    /// The banner shown while a scrolling capture runs. It paints its own text, so
    /// nothing has to be laid out twice or kept in sync with the window size.
    pub unsafe extern "system" fn toast_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            WM_PAINT => {
                unsafe { paint_toast(hwnd) };
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
        }
    }

    unsafe fn paint_toast(hwnd: HWND) {
        let mut paint = PAINTSTRUCT::default();
        let dc = unsafe { BeginPaint(hwnd, &mut paint) };
        if !dc.is_invalid() {
            let mut client = RECT::default();
            let _ = unsafe { GetClientRect(hwnd, &mut client) };
            let background = unsafe { CreateSolidBrush(COLORREF(0x0028_2018)) };
            unsafe { FillRect(dc, &client, background) };
            let _ = unsafe { DeleteObject(background.into()) };

            let label: Vec<u16> = super::TOAST_TEXT.encode_utf16().collect();
            unsafe {
                SetBkMode(dc, TRANSPARENT);
                SetTextColor(dc, COLORREF(0x00f0_e8e0));
                DrawTextW(
                    dc,
                    &mut label.clone(),
                    &mut client,
                    DT_CENTER | DT_VCENTER | DT_SINGLELINE,
                );
            }
        }
        let _ = unsafe { EndPaint(hwnd, &paint) };
    }

    pub unsafe extern "system" fn highlight_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
    ) -> LRESULT {
        match msg {
            WM_PAINT => {
                unsafe { fill_client(hwnd, 0x00d7_7800) };
                LRESULT(0)
            }
            WM_ERASEBKGND => LRESULT(1),
            _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
        }
    }

    unsafe fn fill_client(hwnd: HWND, color: u32) {
        let mut paint = PAINTSTRUCT::default();
        let dc = unsafe { BeginPaint(hwnd, &mut paint) };
        if !dc.is_invalid() {
            let mut client = RECT::default();
            let _ = unsafe { GetClientRect(hwnd, &mut client) };
            let brush = unsafe { CreateSolidBrush(COLORREF(color)) };
            unsafe { FillRect(dc, &client, brush) };
            let _ = unsafe { DeleteObject(brush.into()) };
        }
        let _ = unsafe { EndPaint(hwnd, &paint) };
    }

    unsafe fn paint_frame(hwnd: HWND) {
        let mut paint = PAINTSTRUCT::default();
        let dc = unsafe { BeginPaint(hwnd, &mut paint) };
        if !dc.is_invalid() {
            let mut client = RECT::default();
            let _ = unsafe { GetClientRect(hwnd, &mut client) };
            let size = (client.right, client.bottom);
            let background = unsafe { CreateSolidBrush(COLORREF(0x00d7_7800)) };
            unsafe { FillRect(dc, &client, background) };
            let _ = unsafe { DeleteObject(background.into()) };

            let selection = (size.0 - 2 * super::BORDER, size.1 - 2 * super::BORDER);
            let chip = super::chip_rect(size);
            if selection.0 > 0 && selection.1 > 0 && !chip.is_empty() {
                let chip_rect = RECT {
                    left: chip.left,
                    top: chip.top,
                    right: chip.right,
                    bottom: chip.bottom,
                };
                let chip_background = unsafe { CreateSolidBrush(COLORREF(0x0020_2020)) };
                unsafe { FillRect(dc, &chip_rect, chip_background) };
                let _ = unsafe { DeleteObject(chip_background.into()) };

                let label = format!("{} × {}", selection.0, selection.1);
                let text: Vec<u16> = label.encode_utf16().collect();
                unsafe {
                    SetBkMode(dc, TRANSPARENT);
                    SetTextColor(dc, COLORREF(0x00ff_ffff));
                    let font = GetStockObject(DEFAULT_GUI_FONT);
                    let old = SelectObject(dc, font);
                    let _ = TextOutW(dc, chip.left + 5, chip.top + 4, &text);
                    SelectObject(dc, old);
                }
            }
        }
        let _ = unsafe { EndPaint(hwnd, &paint) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds() -> Rect {
        Rect::new(0, 0, 1920, 1080)
    }

    #[test]
    fn the_invisible_window_border_is_cropped_off() {
        // What Windows 10 and 11 actually report: a few pixels of invisible frame
        // on the left, right and bottom, with the caption inside the rest.
        let frame = Rect::new(100, 100, 500, 400);
        let visible = Rect::new(108, 100, 492, 392);
        let inset = visible_inset(frame, visible);
        assert_eq!(inset, Rect::new(8, 0, 392, 292));

        let pixels = vec![9u8; 400 * 300 * 4];
        let (width, height, cropped) = crop(&pixels, 400, 300, inset).unwrap();
        assert_eq!((width, height), (384, 292), "the border is gone");
        assert_eq!(cropped.len(), 384 * 292 * 4);
    }

    #[test]
    fn a_window_that_reports_no_extra_frame_is_cropped_to_nothing() {
        let frame = Rect::new(0, 0, 40, 30);
        assert_eq!(visible_inset(frame, frame), Rect::new(0, 0, 40, 30));
    }

    #[test]
    fn a_drag_normalises_to_a_positive_rectangle() {
        let forward = selection_rect((100, 120), (420, 380), bounds()).unwrap();
        let backward = selection_rect((420, 380), (100, 120), bounds()).unwrap();
        assert_eq!(forward, Rect::new(100, 120, 420, 380));
        assert_eq!(backward, forward);
    }

    #[test]
    fn a_selection_is_clamped_to_the_virtual_screen() {
        // A monitor left of the primary has negative virtual coordinates.
        let virtual_screen = Rect::new(-1920, -120, 1920, 1080);
        let rect = selection_rect((-2400, -300), (300, 80), virtual_screen).unwrap();
        assert_eq!(rect, Rect::new(-1920, -120, 300, 80));

        let rect = selection_rect((1800, 1000), (4000, 3000), virtual_screen).unwrap();
        assert_eq!(rect, Rect::new(1800, 1000, 1920, 1080));
    }

    #[test]
    fn a_click_without_a_drag_is_not_a_selection() {
        assert_eq!(selection_rect((50, 50), (50, 50), bounds()), None);
        assert_eq!(selection_rect((50, 50), (50, 300), bounds()), None);
        assert_eq!(selection_rect((50, 50), (300, 50), bounds()), None);
        // One pixel is a real, if tiny, selection.
        assert_eq!(
            selection_rect((50, 50), (51, 51), bounds()),
            Some(Rect::new(50, 50, 51, 51))
        );
    }

    #[test]
    fn a_selection_outside_the_virtual_screen_is_rejected() {
        assert_eq!(selection_rect((2000, 100), (2100, 200), bounds()), None);
        assert_eq!(selection_rect((-300, -300), (-200, -200), bounds()), None);
    }

    #[test]
    fn window_titles_and_rects_filter_capture_targets() {
        assert!(is_capture_target("Untitled - Notepad", Rect::new(0, 0, 800, 600)));
        assert!(!is_capture_target("", Rect::new(0, 0, 800, 600)));
        assert!(!is_capture_target("   ", Rect::new(0, 0, 800, 600)));
        assert!(!is_capture_target("Zero sized", Rect::new(10, 10, 10, 40)));
        assert!(!is_capture_target("Inverted", Rect::new(40, 40, 10, 10)));
    }

    #[test]
    fn cropping_reads_the_requested_pixels() {
        // 3 × 2 RGBA, each pixel tagged with its x and y.
        let mut pixels = Vec::new();
        for y in 0..2u8 {
            for x in 0..3u8 {
                pixels.extend_from_slice(&[x, y, 9, 255]);
            }
        }
        let (width, height, rgba) = crop(&pixels, 3, 2, Rect::new(1, 0, 3, 2)).unwrap();
        assert_eq!((width, height), (2, 2));
        assert_eq!(rgba, vec![1, 0, 9, 255, 2, 0, 9, 255, 1, 1, 9, 255, 2, 1, 9, 255]);
    }

    #[test]
    fn cropping_clamps_to_the_buffer_and_rejects_short_buffers() {
        let pixels = vec![0u8; 3 * 2 * 4];
        // A rectangle reaching past the edge keeps only the visible part.
        let (width, height, _) = crop(&pixels, 3, 2, Rect::new(2, 1, 9, 9)).unwrap();
        assert_eq!((width, height), (1, 1));
        assert_eq!(crop(&pixels, 3, 2, Rect::new(5, 5, 9, 9)), None);
        assert_eq!(crop(&pixels, 3, 2, Rect::new(0, 0, 0, 0)), None);
        // A short buffer cannot satisfy the declared dimensions.
        assert_eq!(crop(&pixels[..8], 3, 2, Rect::new(0, 0, 1, 1)), None);
    }

    #[test]
    fn conversion_swaps_blue_and_red_and_forces_opacity() {
        assert_eq!(bgra_to_rgba(&[1, 2, 3, 0]), vec![3, 2, 1, 255]);
        assert_eq!(
            bgra_to_rgba(&[10, 20, 30, 255, 40, 50, 60, 0]),
            vec![30, 20, 10, 255, 60, 50, 40, 255]
        );
    }

    #[test]
    fn blank_renders_are_detected() {
        assert!(looks_blank(&[0, 0, 0, 255, 0, 0, 0, 0]));
        assert!(!looks_blank(&[1, 0, 0, 255, 0, 0, 0, 0]));
    }

    #[test]
    fn capture_sizes_reject_empty_and_absurd_rectangles() {
        assert_eq!(checked_pixels(100, 100).unwrap(), 10_000);
        assert!(checked_pixels(0, 100).is_err());
        assert!(checked_pixels(100, -1).is_err());
        assert!(checked_pixels(100_000, 100_000).is_err());
    }

    const PAGE_WIDTH: u32 = 40;
    const PAGE_HEIGHT: u32 = 120;
    /// Rows the virtual page advances per scroll step in these tests.
    const PAGE_STEP: u32 = 25;
    /// Rows of fixed chrome: a title bar that never scrolls.
    const PAGE_CHROME: u32 = 20;
    /// Deep enough that a test never reaches the bottom of the page by accident.
    const PAGE_ROWS: u32 = 2_000;

    fn page_hash(a: u32, b: u32) -> u32 {
        let mut hash = a.wrapping_mul(2_654_435_761) ^ b.wrapping_mul(0x9E37_79B9);
        hash ^= hash >> 15;
        hash = hash.wrapping_mul(2_246_822_519);
        hash ^= hash >> 13;
        hash
    }

    fn page_colour(seed: u32) -> [u8; 4] {
        [seed as u8, (seed >> 8) as u8, (seed >> 16) as u8, 255]
    }

    /// A frame of the window with its page scrolled to `position`, with the text
    /// gaps the stitcher has to see past.
    #[cfg(windows)]
    fn page_frame(position: u32) -> Vec<u8> {
        let mut pixels = Vec::with_capacity((PAGE_WIDTH * PAGE_HEIGHT * 4) as usize);
        for row in 0..PAGE_HEIGHT {
            for column in 0..PAGE_WIDTH {
                let value = if row < PAGE_CHROME {
                    page_colour(page_hash(7_000 + row, column))
                } else {
                    let page_row = position + row - PAGE_CHROME;
                    if page_row < PAGE_ROWS && page_row % 9 >= 2 {
                        page_colour(page_hash(page_row, column))
                    } else {
                        [255, 255, 255, 255]
                    }
                };
                pixels.extend_from_slice(&value);
            }
        }
        pixels
    }

    #[cfg(windows)]
    #[test]
    fn escape_stops_the_scroll_loop_and_keeps_the_rows_so_far() {
        let escaped = AtomicBool::new(false);
        let mut position = 0;
        let mut scrolls = 0;

        let (width, total, rgba) = scroll_pages(
            (PAGE_WIDTH, PAGE_HEIGHT, page_frame(0)),
            |step, _current| {
                scrolls += 1;
                // The press lands while the second step is running, so the frame
                // that step turned up must not reach the image.
                if step == 1 {
                    escaped.store(true, Ordering::SeqCst);
                }
                position += PAGE_STEP;
                Advance::Moved(PAGE_WIDTH, PAGE_HEIGHT, page_frame(position))
            },
            &|| escaped.load(Ordering::SeqCst),
        );

        assert_eq!(scrolls, 2, "the press ended the run instead of scrolling on");
        assert_eq!(position, 2 * PAGE_STEP);
        assert_eq!(width, PAGE_WIDTH);
        assert_eq!(
            total,
            PAGE_HEIGHT + PAGE_STEP,
            "only the steps that finished before the press are in the image"
        );
        assert_eq!(
            rgba.len(),
            (total * PAGE_WIDTH * 4) as usize,
            "a partial image is still an image"
        );
    }

    #[cfg(windows)]
    #[test]
    fn an_escape_before_the_first_scroll_returns_the_first_frame() {
        let (width, total, rgba) = scroll_pages(
            (PAGE_WIDTH, PAGE_HEIGHT, page_frame(0)),
            |_, _| panic!("nothing is scrolled once Escape has been pressed"),
            &|| true,
        );

        assert_eq!((width, total), (PAGE_WIDTH, PAGE_HEIGHT));
        assert_eq!(rgba, page_frame(0));
    }

    #[cfg(windows)]
    #[test]
    fn a_page_that_stops_moving_ends_the_run_with_everything_stitched() {
        let mut position = 0;

        let (_, total, _) = scroll_pages(
            (PAGE_WIDTH, PAGE_HEIGHT, page_frame(0)),
            |_, _| {
                if position == 3 * PAGE_STEP {
                    return Advance::End;
                }
                position += PAGE_STEP;
                Advance::Moved(PAGE_WIDTH, PAGE_HEIGHT, page_frame(position))
            },
            &|| false,
        );

        assert_eq!(total, PAGE_HEIGHT + 3 * PAGE_STEP);
    }
}
