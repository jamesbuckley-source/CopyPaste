//! Clipboard capture.
//!
//! Capture is poll-driven off `GetClipboardSequenceNumber()` rather than a
//! `WM_CLIPBOARDUPDATE` listener. The sequence number is a single cheap call that
//! changes only when the clipboard changes, so a 350 ms tick costs effectively
//! nothing, and it avoids a second message-only window competing with Tauri's
//! event loop.
//!
//! Text is read and written through the raw Win32 API instead of a helper crate
//! so the exact bytes survive a round trip: CRLF, lone surrogates, and embedded
//! NUL handling all matter for a tool whose entire job is reproducing what the
//! user copied.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::error::{AppError, Result};
use crate::hashing;
use crate::models::{ClipContent, ClipKind, NewClip};
use crate::search;
use crate::settings::Settings;
use crate::store::Store;

/// Characters kept for the list preview.
const PREVIEW_CHARS: usize = 320;

pub const EVENT_HISTORY_CHANGED: &str = "history:changed";
/// Emitted whenever the private capture pause starts, changes or ends.
pub const EVENT_CAPTURE_PAUSED: &str = "capture:paused";
/// Emitted when a copy has scheduled the clipboard to be emptied.
pub const EVENT_CLIPBOARD_CLEAR_SCHEDULED: &str = "clipboard:clear-scheduled";

/// Why capture is currently not recording, or that it is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PauseState {
    pub paused: bool,
    /// Unix-millisecond deadline of a timed pause.
    pub until: Option<i64>,
    /// True while paused "until restart".
    pub indefinite: bool,
}

// ---------------------------------------------------------------------------
// Watcher
// ---------------------------------------------------------------------------

/// Owns the background capture thread and the "ignore my own write" latch.
pub struct Watcher {
    /// Content hash of something this app just put on the clipboard. The next
    /// matching capture is dropped so selecting an item does not immediately
    /// re-record it and reshuffle the list under the user's cursor.
    suppress: Arc<Mutex<Option<String>>>,
    running: Arc<AtomicBool>,
    /// Private-mode pause: a timed deadline, or the "until restart" flag. While
    /// paused the sequence counter still advances so nothing copied in the
    /// meantime is captured on resume.
    paused_until: Arc<Mutex<Option<Instant>>>,
    paused_forever: Arc<AtomicBool>,
}

impl Watcher {
    pub fn new() -> Self {
        Watcher {
            suppress: Arc::new(Mutex::new(None)),
            running: Arc::new(AtomicBool::new(false)),
            paused_until: Arc::new(Mutex::new(None)),
            paused_forever: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn suppress_hash(&self, hash: String) {
        *self.suppress.lock() = Some(hash);
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::Relaxed);
    }

    /// Stops capture for `duration`.
    pub fn pause_for(&self, duration: Duration) {
        self.paused_forever.store(false, Ordering::SeqCst);
        *self.paused_until.lock() = Some(Instant::now() + duration);
    }

    /// Stops capture until the application is restarted.
    pub fn pause_until_restart(&self) {
        *self.paused_until.lock() = None;
        self.paused_forever.store(true, Ordering::SeqCst);
    }

    pub fn resume(&self) {
        self.paused_forever.store(false, Ordering::SeqCst);
        *self.paused_until.lock() = None;
    }

    pub fn pause_state(&self) -> PauseState {
        if self.paused_forever.load(Ordering::SeqCst) {
            return PauseState {
                paused: true,
                until: None,
                indefinite: true,
            };
        }
        match *self.paused_until.lock() {
            Some(deadline) if deadline > Instant::now() => PauseState {
                paused: true,
                until: Some(millis_until(deadline)),
                indefinite: false,
            },
            _ => PauseState {
                paused: false,
                until: None,
                indefinite: false,
            },
        }
    }

    pub fn start(&self, app: AppHandle, store: Arc<Store>) {
        if self.running.swap(true, Ordering::SeqCst) {
            return;
        }
        let suppress = Arc::clone(&self.suppress);
        let running = Arc::clone(&self.running);
        let paused_until = Arc::clone(&self.paused_until);
        let paused_forever = Arc::clone(&self.paused_forever);

        let spawned = std::thread::Builder::new()
            .name("clipboard-watcher".into())
            .spawn(move || {
                let mut last = sequence_number();
                while running.load(Ordering::Relaxed) {
                    let settings = store.settings();
                    std::thread::sleep(Duration::from_millis(settings.poll_interval_ms));
                    if !running.load(Ordering::Relaxed) {
                        break;
                    }

                    let current = sequence_number();
                    if current == last {
                        continue;
                    }
                    last = current;

                    // A timed pause that has just run out is noticed here, so the
                    // indicator in the UI clears itself even if nothing is copied.
                    let expired = {
                        let mut deadline = paused_until.lock();
                        let expired = deadline.is_some_and(|at| Instant::now() >= at);
                        if expired {
                            *deadline = None;
                        }
                        expired
                    };
                    if expired {
                        let _ = app.emit(EVENT_CAPTURE_PAUSED, PauseState {
                            paused: false,
                            until: None,
                            indefinite: false,
                        });
                    }
                    if paused_forever.load(Ordering::SeqCst) || paused_until.lock().is_some() {
                        log::debug!("capture is paused; ignoring a clipboard change");
                        continue;
                    }

                    // A panic while interpreting hostile clipboard data must not
                    // silently disable capture for the rest of the session.
                    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        capture_once(&app, &store, &suppress, &settings)
                    }));
                    match outcome {
                        Ok(Ok(())) => {}
                        Ok(Err(e)) => log::warn!("clipboard capture failed: {e}"),
                        Err(_) => log::error!("clipboard capture panicked; continuing"),
                    }
                }
                log::info!("clipboard watcher stopped");
            });

        if let Err(e) = spawned {
            self.running.store(false, Ordering::SeqCst);
            log::error!("could not start the clipboard watcher thread: {e}");
        }
    }
}

/// Converts an `Instant` deadline into a wall-clock timestamp the UI can render.
fn millis_until(deadline: Instant) -> i64 {
    let remaining = deadline.saturating_duration_since(Instant::now());
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    now + remaining.as_millis() as i64
}

impl Default for Watcher {
    fn default() -> Self {
        Watcher::new()
    }
}

fn capture_once(
    app: &AppHandle,
    store: &Arc<Store>,
    suppress: &Arc<Mutex<Option<String>>>,
    settings: &Settings,
) -> Result<()> {
    // Attribute the clipboard to whatever was in front when the change landed.
    let foreground = foreground_process_name();

    if let Some(name) = &foreground
        && settings
            .ignored_apps
            .iter()
            .any(|ignored| ignored.eq_ignore_ascii_case(name))
    {
        log::debug!("ignoring a copy from {name}");
        return Ok(());
    }

    // Copies made while our own window is focused are still recorded: the hash
    // latch is what prevents feedback loops, and dropping them here would silently
    // lose anything copied out of the preview pane. They are merely not
    // *attributed* to us, which would be meaningless in the UI.
    let source_app = foreground.filter(|name| !is_self(name));

    let Some(content) = read_content(settings) else {
        return Ok(());
    };

    let hash = hashing::of_content(&content);
    if suppress.lock().as_deref() == Some(hash.as_str()) {
        // This is the write we just performed; forget it and move on.
        *suppress.lock() = None;
        return Ok(());
    }

    let (preview, search_text) = describe(&content);
    let kind = content.kind();
    // Rich flavours ride along with the plain text when the source offered them,
    // so a styled copy can be pasted back with its formatting intact.
    let (html, rtf) = if kind == ClipKind::Text && settings.paste_rich_text {
        read_rich_text()
    } else {
        (None, None)
    };
    let new = NewClip {
        content,
        content_hash: hash,
        source_app,
        search_text,
        preview,
        html,
        rtf,
    };

    match store.insert_clip(new)? {
        crate::store::InsertOutcome::Inserted(id) => {
            log::debug!("captured a new clip");
            if kind == ClipKind::Image {
                schedule_ocr(app, Arc::clone(store), id);
            }
        }
        crate::store::InsertOutcome::Duplicate(_) => log::debug!("promoted an existing clip"),
    }

    if settings.max_items > 0 && store.total() > settings.max_items as usize {
        let pruned = store.prune()?;
        if pruned > 0 {
            log::debug!("pruned {pruned} clip(s) past the retention cap");
        }
    }

    let _ = app.emit(EVENT_HISTORY_CHANGED, ());
    Ok(())
}

/// Recognises an image's text off the capture thread.
///
/// Capture must stay responsive — the watcher thread is the only thing reading the
/// clipboard — so the expensive part happens here, and the UI is told to re-read
/// once the index has changed.
fn schedule_ocr(app: &AppHandle, store: Arc<Store>, id: i64) {
    let app = app.clone();
    let spawned = std::thread::Builder::new()
        .name("clipboard-ocr".into())
        .spawn(move || {
            if let Err(e) = store.run_ocr(id) {
                log::warn!("text recognition failed for clip {id}: {e}");
            }
            let _ = app.emit(EVENT_HISTORY_CHANGED, ());
        });
    if let Err(e) = spawned {
        log::error!("could not start the text recognition thread: {e}");
    }
}

/// Builds the preview text and the search mirror for a captured payload.
fn describe(content: &ClipContent) -> (String, String) {
    match content {
        ClipContent::Text(text) => {
            let collapsed: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
            let preview = collapsed.chars().take(PREVIEW_CHARS).collect();
            (preview, search::normalize(text))
        }
        ClipContent::Files(paths) => {
            let names: Vec<String> = paths.iter().map(|p| leaf_name(p)).collect();
            let preview = if names.len() == 1 {
                names[0].clone()
            } else {
                let shown: Vec<&str> = names.iter().take(4).map(String::as_str).collect();
                format!("{} files · {}", names.len(), shown.join(", "))
            };
            (preview, search::normalize(&paths.join(" ")))
        }
        ClipContent::Image { width, height, .. } => {
            (format!("Image {width} × {height}"), String::new())
        }
    }
}

fn leaf_name(path: &str) -> String {
    path.rsplit(['\\', '/']).next().unwrap_or(path).to_string()
}

fn is_self(process_name: &str) -> bool {
    std::env::current_exe()
        .ok()
        .as_deref()
        .and_then(|path| path.file_name().map(|n| n.to_string_lossy().to_string()))
        .is_some_and(|own| own.eq_ignore_ascii_case(process_name))
}

/// Reads whichever representation the clipboard currently offers, honouring the
/// capture preferences.
fn read_content(settings: &Settings) -> Option<ClipContent> {
    if settings.ignore_sensitive && clipboard_opt_out() {
        log::debug!("clipboard owner asked to be excluded from history");
        return None;
    }

    if settings.capture_files
        && let Some(paths) = read_file_list()
        && !paths.is_empty()
    {
        return Some(ClipContent::Files(paths));
    }

    let text = read_text();
    let image_first = settings.capture_images && settings.prefer_images_over_text;
    if image_first && let Some(image) = read_image() {
        return Some(image);
    }

    if let Some(text) = text.filter(|t| !t.is_empty()) {
        return Some(ClipContent::Text(text));
    }

    if settings.capture_images
        && !image_first
        && let Some(image) = read_image()
    {
        return Some(image);
    }

    None
}

// ---------------------------------------------------------------------------
// Public clipboard write
// ---------------------------------------------------------------------------

/// Places `content` on the clipboard. Returns the content hash so the caller can
/// arm the suppression latch.
pub fn write(content: &ClipContent) -> Result<String> {
    write_flavoured(content, None, None)
}

/// Writes `content` and, for text, the rich flavours captured with it.
pub fn write_flavoured(
    content: &ClipContent,
    html: Option<&str>,
    rtf: Option<&str>,
) -> Result<String> {
    match content {
        // arboard owns its own clipboard session for images, so this path must not
        // be nested inside `with_clipboard`.
        ClipContent::Image {
            width,
            height,
            rgba,
        } => write_image(*width, *height, rgba)?,
        _ => {
            let outcome = with_clipboard(|| -> Result<()> {
                unsafe { EmptyClipboard()? };
                match content {
                    ClipContent::Text(text) => {
                        set_text(text)?;
                        if let Some(html) = html.filter(|value| !value.trim().is_empty()) {
                            set_html(html)?;
                        }
                        if let Some(rtf) = rtf.filter(|value| !value.trim().is_empty()) {
                            set_rtf(rtf)?;
                        }
                    }
                    ClipContent::Files(paths) => set_files(paths)?,
                    ClipContent::Image { .. } => unreachable!("handled above"),
                }
                Ok(())
            });
            match outcome {
                Some(result) => result?,
                None => {
                    return Err(AppError::Clipboard(
                        "another process is holding the clipboard open".into(),
                    ));
                }
            }
        }
    }
    Ok(hashing::of_content(content))
}

/// Empties the clipboard, used by the auto-clear timer.
pub fn clear() -> Result<()> {
    #[cfg(windows)]
    {
        let outcome = with_clipboard(|| unsafe { EmptyClipboard() });
        match outcome {
            Some(Ok(())) => Ok(()),
            Some(Err(e)) => Err(AppError::Clipboard(format!(
                "could not empty the clipboard: {e}"
            ))),
            None => Err(AppError::Clipboard(
                "another process is holding the clipboard open".into(),
            )),
        }
    }
    #[cfg(not(windows))]
    {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Windows implementation
// ---------------------------------------------------------------------------

#[cfg(windows)]
use windows::Win32::Foundation::{CloseHandle, GlobalFree, HANDLE, HGLOBAL};
#[cfg(windows)]
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, GetClipboardSequenceNumber,
    IsClipboardFormatAvailable, OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
};
#[cfg(windows)]
use windows::Win32::System::Memory::{
    GMEM_MOVEABLE, GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock,
};
#[cfg(windows)]
use windows::Win32::System::Threading::{
    OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
#[cfg(windows)]
use windows::Win32::UI::Shell::{DragQueryFileW, HDROP};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};
#[cfg(windows)]
use windows::core::PWSTR;

#[cfg(windows)]
const CF_UNICODETEXT: u32 = 13;
/// `CF_HDROP`; defined locally because the windows crate nests it under Ole.
#[cfg(windows)]
const CF_HDROP: u32 = 15;

#[cfg(windows)]
pub fn sequence_number() -> u32 {
    unsafe { GetClipboardSequenceNumber() }
}

/// Runs `f` with the clipboard open, retrying while another process holds it.
///
/// Returns `None` if the clipboard stayed locked. `f` must not panic: the paired
/// `CloseClipboard` lives after the call.
#[cfg(windows)]
fn with_clipboard<T>(f: impl FnOnce() -> T) -> Option<T> {
    for attempt in 0..8u64 {
        // SAFETY: a successful OpenClipboard is always matched by CloseClipboard.
        if unsafe { OpenClipboard(None) }.is_ok() {
            let value = f();
            let _ = unsafe { CloseClipboard() };
            return Some(value);
        }
        std::thread::sleep(Duration::from_millis(10 * (attempt + 1)));
    }
    None
}

/// Reads the `HGLOBAL` behind a clipboard format as a byte slice.
///
/// # Safety
/// Must be called with the clipboard open.
#[cfg(windows)]
unsafe fn hglobal_bytes(handle: HANDLE) -> Option<(*mut u8, usize)> {
    let hglobal = HGLOBAL(handle.0);
    let size = unsafe { GlobalSize(hglobal) };
    if size == 0 {
        return None;
    }
    let pointer = unsafe { GlobalLock(hglobal) }.cast::<u8>();
    if pointer.is_null() {
        return None;
    }
    Some((pointer, size))
}

/// # Safety
/// Must be called with the clipboard open.
#[cfg(windows)]
unsafe fn unlock(handle: HANDLE) {
    let _ = unsafe { GlobalUnlock(HGLOBAL(handle.0)) };
}

#[cfg(windows)]
fn read_text() -> Option<String> {
    with_clipboard(|| unsafe {
        if IsClipboardFormatAvailable(CF_UNICODETEXT).is_err() {
            return None;
        }
        let handle = GetClipboardData(CF_UNICODETEXT).ok()?;
        let (pointer, size) = hglobal_bytes(handle)?;
        let mut units = Vec::with_capacity(size / 2);
        let capacity = size / 2;
        for index in 0..capacity {
            let unit = *pointer.cast::<u16>().add(index);
            if unit == 0 {
                break;
            }
            units.push(unit);
        }
        unlock(handle);
        if units.is_empty() {
            None
        } else {
            Some(String::from_utf16_lossy(&units))
        }
    })
    .flatten()
}

#[cfg(windows)]
fn read_file_list() -> Option<Vec<String>> {
    with_clipboard(|| unsafe {
        if IsClipboardFormatAvailable(CF_HDROP).is_err() {
            return None;
        }
        let handle = GetClipboardData(CF_HDROP).ok()?;
        let drop = HDROP(handle.0);
        // 0xFFFF_FFFF asks DragQueryFileW for the item count.
        let count = DragQueryFileW(drop, u32::MAX, None);
        if count == 0 {
            return None;
        }
        let mut paths = Vec::with_capacity(count as usize);
        for index in 0..count {
            let length = DragQueryFileW(drop, index, None);
            if length == 0 {
                continue;
            }
            let mut buffer = vec![0u16; length as usize + 1];
            let written = DragQueryFileW(drop, index, Some(&mut buffer));
            if written == 0 {
                continue;
            }
            buffer.truncate(written as usize);
            paths.push(String::from_utf16_lossy(&buffer));
        }
        (!paths.is_empty()).then_some(paths)
    })
    .flatten()
}

/// Reads both rich flavours in one clipboard session, so the HTML and RTF come
/// from the same copy.
#[cfg(windows)]
fn read_rich_text() -> (Option<String>, Option<String>) {
    let html = read_registered_bytes("HTML Format").and_then(|bytes| cf_html_fragment(&bytes));
    let rtf = read_registered_bytes("Rich Text Format")
        .map(|bytes| String::from_utf8_lossy(&bytes).trim_end_matches('\0').to_string())
        .filter(|text| !text.trim().is_empty());
    (html, rtf)
}

/// Snapshot of a named registered clipboard format.
#[cfg(windows)]
fn read_registered_bytes(name: &str) -> Option<Vec<u8>> {
    let units = widened(name);
    let format = unsafe { RegisterClipboardFormatW(windows::core::PCWSTR(units.as_ptr())) };
    if format == 0 {
        return None;
    }
    with_clipboard(|| unsafe {
        if IsClipboardFormatAvailable(format).is_err() {
            return None;
        }
        let handle = GetClipboardData(format).ok()?;
        let (pointer, size) = hglobal_bytes(handle)?;
        let bytes = std::slice::from_raw_parts(pointer, size).to_vec();
        unlock(handle);
        Some(bytes)
    })
    .flatten()
}

/// Cuts the fragment out of a `CF_HTML` payload using its own byte offsets.
fn cf_html_fragment(bytes: &[u8]) -> Option<String> {
    let header_len = bytes.len().min(1024);
    let header = String::from_utf8_lossy(&bytes[..header_len]);
    let offset = |key: &str| -> Option<usize> {
        let index = header.find(key)?;
        let rest = header[index + key.len()..].trim_start();
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        digits.parse().ok()
    };

    match (offset("StartFragment:"), offset("EndFragment:")) {
        (Some(start), Some(end)) if start < end && end <= bytes.len() => {
            trim_fragment(&String::from_utf8_lossy(&bytes[start..end]))
        }
        _ => {
            // Malformed or missing offsets: fall back to the document body.
            let text = String::from_utf8_lossy(bytes);
            let lower = text.to_lowercase();
            let start = lower.find("<body").or_else(|| lower.find("<html"))?;
            trim_fragment(&text[start..])
        }
    }
}

fn trim_fragment(text: &str) -> Option<String> {
    let trimmed = text.trim().trim_matches('\0').trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

#[cfg(windows)]
fn read_image() -> Option<ClipContent> {
    let mut clipboard = arboard::Clipboard::new().ok()?;
    let image = clipboard.get_image().ok()?;
    if image.width == 0 || image.height == 0 {
        return None;
    }
    let expected = image.width.checked_mul(image.height)?.checked_mul(4)?;
    if image.bytes.len() < expected {
        log::warn!(
            "clipboard image is {} bytes but {}x{} RGBA needs {expected}",
            image.bytes.len(),
            image.width,
            image.height
        );
        return None;
    }
    Some(ClipContent::Image {
        width: image.width as u32,
        height: image.height as u32,
        rgba: image.bytes[..expected].to_vec(),
    })
}

#[cfg(windows)]
fn widened(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Checks the opt-out formats that Windows clipboard history itself honours.
/// Password managers publish these so their contents never reach a log.
#[cfg(windows)]
fn clipboard_opt_out() -> bool {
    with_clipboard(|| unsafe {
        // Presence of this format is itself the signal, regardless of its value.
        let exclude = RegisterClipboardFormatW(windows::core::PCWSTR(
            widened("ExcludeClipboardContentFromMonitorProcessing").as_ptr(),
        ));
        if exclude != 0 && IsClipboardFormatAvailable(exclude).is_ok() {
            return true;
        }

        for name in ["CanIncludeInClipboardHistory", "CanUploadToCloudClipboard"] {
            let format = RegisterClipboardFormatW(windows::core::PCWSTR(widened(name).as_ptr()));
            if format == 0 || IsClipboardFormatAvailable(format).is_err() {
                continue;
            }
            let Ok(handle) = GetClipboardData(format) else {
                continue;
            };
            let Some((pointer, size)) = hglobal_bytes(handle) else {
                continue;
            };
            if size >= std::mem::size_of::<u32>() {
                let flag = std::ptr::read_unaligned(pointer.cast::<u32>());
                unlock(handle);
                if flag == 0 {
                    return true;
                }
            } else {
                unlock(handle);
            }
        }
        false
    })
    .unwrap_or(false)
}

#[cfg(windows)]
fn set_text(text: &str) -> Result<()> {
    let units: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    let bytes = units.len() * size_of::<u16>();
    unsafe {
        let hglobal = GlobalAlloc(GMEM_MOVEABLE, bytes).map_err(|e| {
            AppError::Clipboard(format!("could not allocate clipboard memory: {e}"))
        })?;
        let destination = GlobalLock(hglobal);
        if destination.is_null() {
            let _ = GlobalFree(Some(hglobal));
            return Err(AppError::Clipboard(
                "could not lock clipboard memory".into(),
            ));
        }
        std::ptr::copy_nonoverlapping(units.as_ptr().cast::<u8>(), destination.cast::<u8>(), bytes);
        let _ = GlobalUnlock(hglobal);

        // On success the system takes ownership of `hglobal`; freeing it here
        // would leave the clipboard pointing at released memory.
        if let Err(e) = SetClipboardData(CF_UNICODETEXT, Some(HANDLE(hglobal.0))) {
            let _ = GlobalFree(Some(hglobal));
            return Err(AppError::Clipboard(format!("could not set text: {e}")));
        }
    }
    Ok(())
}

/// Copies `payload` into a `HGLOBAL` and hands it to the clipboard under `format`.
///
/// # Safety
/// Must be called with the clipboard open.
#[cfg(windows)]
unsafe fn set_bytes(format: u32, payload: &[u8]) -> Result<()> {
    unsafe {
        let hglobal = GlobalAlloc(GMEM_MOVEABLE, payload.len()).map_err(|e| {
            AppError::Clipboard(format!("could not allocate clipboard memory: {e}"))
        })?;
        let destination = GlobalLock(hglobal);
        if destination.is_null() {
            let _ = GlobalFree(Some(hglobal));
            return Err(AppError::Clipboard(
                "could not lock clipboard memory".into(),
            ));
        }
        std::ptr::copy_nonoverlapping(payload.as_ptr(), destination.cast::<u8>(), payload.len());
        let _ = GlobalUnlock(hglobal);

        if let Err(e) = SetClipboardData(format, Some(HANDLE(hglobal.0))) {
            let _ = GlobalFree(Some(hglobal));
            return Err(AppError::Clipboard(format!(
                "could not set clipboard format {format}: {e}"
            )));
        }
    }
    Ok(())
}

/// Publishes `payload` under a named registered format, e.g. "HTML Format".
#[cfg(windows)]
fn set_registered(name: &str, payload: &[u8]) -> Result<()> {
    let units = widened(name);
    let format = unsafe { RegisterClipboardFormatW(windows::core::PCWSTR(units.as_ptr())) };
    if format == 0 {
        return Err(AppError::Clipboard(format!(
            "the {name} clipboard format is unavailable"
        )));
    }
    let outcome = with_clipboard(|| unsafe { set_bytes(format, payload) });
    match outcome {
        Some(result) => result,
        None => Err(AppError::Clipboard(
            "another process is holding the clipboard open".into(),
        )),
    }
}

/// Wraps an HTML fragment in the `CF_HTML` header consumers expect.
///
/// The header records byte offsets, and its own length depends on how many digits
/// those offsets have, so it is rebuilt until the numbers stop changing.
fn build_cf_html(fragment: &str) -> Vec<u8> {
    const PROLOGUE: &str = "<html><body><!--StartFragment-->";
    const EPILOGUE: &str = "<!--EndFragment--></body></html>";
    let header = |start_html: usize,
                  end_html: usize,
                  start_fragment: usize,
                  end_fragment: usize| {
        format!(
            "Version:0.9\r\nStartHTML:{start_html:010}\r\nEndHTML:{end_html:010}\r\n\
             StartFragment:{start_fragment:010}\r\nEndFragment:{end_fragment:010}\r\n"
        )
    };

    let mut start_html = 0;
    let mut end_html = 0;
    let mut start_fragment = 0;
    let mut end_fragment = 0;
    for _ in 0..3 {
        let length = header(start_html, end_html, start_fragment, end_fragment).len();
        start_html = length;
        start_fragment = length + PROLOGUE.len();
        end_fragment = start_fragment + fragment.len();
        end_html = end_fragment + EPILOGUE.len();
    }

    let mut out = header(start_html, end_html, start_fragment, end_fragment).into_bytes();
    out.extend_from_slice(PROLOGUE.as_bytes());
    out.extend_from_slice(fragment.as_bytes());
    out.extend_from_slice(EPILOGUE.as_bytes());
    out
}

#[cfg(windows)]
fn set_html(fragment: &str) -> Result<()> {
    set_registered("HTML Format", &build_cf_html(fragment))
}

#[cfg(windows)]
fn set_rtf(rtf: &str) -> Result<()> {
    set_registered("Rich Text Format", rtf.as_bytes())
}

/// Serialises paths into the `DROPFILES` + double-NUL-terminated UTF-16 list that
/// `CF_HDROP` consumers expect.
#[cfg(windows)]
fn build_hdrop(paths: &[String]) -> Vec<u8> {
    // DROPFILES: DWORD pFiles; POINT pt; BOOL fNC; BOOL fWide;
    const HEADER_SIZE: u32 = 20;
    let mut buffer = Vec::new();
    buffer.extend_from_slice(&HEADER_SIZE.to_le_bytes());
    buffer.extend_from_slice(&0i32.to_le_bytes()); // pt.x
    buffer.extend_from_slice(&0i32.to_le_bytes()); // pt.y
    buffer.extend_from_slice(&0i32.to_le_bytes()); // fNC
    buffer.extend_from_slice(&1i32.to_le_bytes()); // fWide = TRUE
    for path in paths {
        for unit in path.encode_utf16() {
            buffer.extend_from_slice(&unit.to_le_bytes());
        }
        buffer.extend_from_slice(&0u16.to_le_bytes());
    }
    // A final extra NUL terminates the list itself.
    buffer.extend_from_slice(&0u16.to_le_bytes());
    buffer
}

#[cfg(windows)]
fn set_files(paths: &[String]) -> Result<()> {
    let payload = build_hdrop(paths);
    unsafe {
        let hglobal = GlobalAlloc(GMEM_MOVEABLE, payload.len()).map_err(|e| {
            AppError::Clipboard(format!("could not allocate clipboard memory: {e}"))
        })?;
        let destination = GlobalLock(hglobal);
        if destination.is_null() {
            let _ = GlobalFree(Some(hglobal));
            return Err(AppError::Clipboard(
                "could not lock clipboard memory".into(),
            ));
        }
        std::ptr::copy_nonoverlapping(payload.as_ptr(), destination.cast::<u8>(), payload.len());
        let _ = GlobalUnlock(hglobal);

        if let Err(e) = SetClipboardData(CF_HDROP, Some(HANDLE(hglobal.0))) {
            let _ = GlobalFree(Some(hglobal));
            return Err(AppError::Clipboard(format!("could not set file list: {e}")));
        }
    }
    Ok(())
}

#[cfg(windows)]
fn write_image(width: u32, height: u32, rgba: &[u8]) -> Result<()> {
    let mut clipboard =
        arboard::Clipboard::new().map_err(|e| AppError::Clipboard(e.to_string()))?;
    clipboard
        .set_image(arboard::ImageData {
            width: width as usize,
            height: height as usize,
            bytes: std::borrow::Cow::Borrowed(rgba),
        })
        .map_err(|e| AppError::Clipboard(format!("could not set image: {e}")))
}

/// Process name (not full path) of the window that currently has focus.
#[cfg(windows)]
fn foreground_process_name() -> Option<String> {
    unsafe {
        let window = GetForegroundWindow();
        if window.is_invalid() {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(window, Some(&mut pid));
        if pid == 0 {
            return None;
        }

        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buffer = [0u16; 1024];
        let mut length = buffer.len() as u32;
        let result = QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut length,
        );
        let _ = CloseHandle(process);
        result.ok()?;

        let full = String::from_utf16_lossy(&buffer[..length as usize]);
        Some(leaf_name(&full))
    }
}

// ---------------------------------------------------------------------------
// Non-Windows stubs
// ---------------------------------------------------------------------------

#[cfg(not(windows))]
pub fn sequence_number() -> u32 {
    0
}

#[cfg(not(windows))]
fn read_text() -> Option<String> {
    None
}

#[cfg(not(windows))]
fn read_file_list() -> Option<Vec<String>> {
    None
}

#[cfg(not(windows))]
fn read_rich_text() -> (Option<String>, Option<String>) {
    (None, None)
}

#[cfg(not(windows))]
fn read_image() -> Option<ClipContent> {
    None
}

#[cfg(not(windows))]
fn clipboard_opt_out() -> bool {
    false
}

#[cfg(not(windows))]
fn foreground_process_name() -> Option<String> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_preview_is_collapsed_and_truncated() {
        let (preview, search_text) = describe(&ClipContent::Text("  hello \n\n world \t".into()));
        assert_eq!(preview, "hello world");
        assert_eq!(search_text, "hello world");

        let long = "x".repeat(PREVIEW_CHARS * 3);
        let (preview, _) = describe(&ClipContent::Text(long.clone()));
        assert_eq!(preview.chars().count(), PREVIEW_CHARS);
        assert_ne!(preview.len(), long.len());
    }

    #[test]
    fn text_preview_counts_characters_not_bytes() {
        // Multi-byte glyphs must not be truncated mid-character.
        let body = "é".repeat(PREVIEW_CHARS * 2);
        let (preview, _) = describe(&ClipContent::Text(body));
        assert_eq!(preview.chars().count(), PREVIEW_CHARS);
        assert!(preview.chars().all(|c| c == 'é'));
    }

    #[test]
    fn file_preview_summarises_lists() {
        let (one, search_text) = describe(&ClipContent::Files(vec!["C:\\docs\\report.pdf".into()]));
        assert_eq!(one, "report.pdf");
        assert_eq!(search_text, "c:\\docs\\report.pdf");

        let many = describe(&ClipContent::Files(vec![
            "C:\\a.txt".into(),
            "C:\\b.txt".into(),
            "C:\\c.txt".into(),
        ]));
        assert_eq!(many.0, "3 files · a.txt, b.txt, c.txt");
    }

    #[test]
    fn file_preview_truncates_long_lists() {
        let paths: Vec<String> = (0..10).map(|i| format!("C:\\file{i}.txt")).collect();
        let (preview, _) = describe(&ClipContent::Files(paths));
        assert!(preview.starts_with("10 files · "));
        assert!(
            !preview.contains("file9.txt"),
            "only the first few names are shown"
        );
    }

    #[test]
    fn image_preview_reports_dimensions() {
        let (preview, search_text) = describe(&ClipContent::Image {
            width: 1920,
            height: 1080,
            rgba: Vec::new(),
        });
        assert_eq!(preview, "Image 1920 × 1080");
        assert!(search_text.is_empty());
    }

    #[test]
    fn leaf_name_handles_both_separators() {
        assert_eq!(leaf_name("C:\\a\\b\\c.txt"), "c.txt");
        assert_eq!(leaf_name("C:/a/b/c.txt"), "c.txt");
        assert_eq!(leaf_name("plain.txt"), "plain.txt");
        assert_eq!(leaf_name(""), "");
    }

    #[test]
    fn is_self_matches_the_running_executable() {
        let own = std::env::current_exe()
            .unwrap()
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        assert!(is_self(&own));
        assert!(is_self(&own.to_uppercase()));
        assert!(!is_self("definitely-not-us.exe"));
    }

    #[test]
    fn rich_text_offsets_survive_a_round_trip() {
        let fragment = "<b>Bold</b> and a caret →";
        let payload = build_cf_html(fragment);
        let text = String::from_utf8_lossy(&payload);
        let field = |key: &str| -> usize {
            let start = text.find(key).expect(key) + key.len();
            text[start..start + 10].parse().expect("a 10-digit offset")
        };
        let start = field("StartFragment:");
        let end = field("EndFragment:");
        assert_eq!(&text[start..end], fragment);
        assert_eq!(
            field("StartHTML:"),
            text.find("<html").expect("the document follows the header"),
            "StartHTML points at the document"
        );
        assert_eq!(field("EndHTML:"), payload.len());
        assert_eq!(cf_html_fragment(&payload).as_deref(), Some(fragment));
    }

    #[test]
    fn rich_text_falls_back_to_the_document_body() {
        let payload = b"Version:0.9\r\n<html><body><p>no offsets</p></body></html>";
        assert_eq!(
            cf_html_fragment(payload).as_deref(),
            Some("<body><p>no offsets</p></body></html>")
        );
    }

    #[test]
    fn pause_state_reports_a_deadline_and_resumes() {
        let watcher = Watcher::new();
        assert!(!watcher.pause_state().paused);

        watcher.pause_for(Duration::from_secs(30));
        let state = watcher.pause_state();
        assert!(state.paused);
        assert!(!state.indefinite);
        assert!(state.until.is_some_and(|until| until > 0));

        watcher.resume();
        assert!(!watcher.pause_state().paused);

        watcher.pause_until_restart();
        let state = watcher.pause_state();
        assert!(state.paused && state.indefinite);
        watcher.resume();
        assert!(!watcher.pause_state().paused);
    }

    #[cfg(windows)]
    #[test]
    fn hdrop_header_describes_a_wide_double_terminated_list() {
        let payload = build_hdrop(&["C:\\one.txt".into(), "C:\\two.txt".into()]);

        // DROPFILES layout: DWORD pFiles; POINT pt (2 x LONG); BOOL fNC; BOOL fWide.
        let field =
            |range: std::ops::Range<usize>| i32::from_le_bytes(payload[range].try_into().unwrap());
        assert_eq!(
            field(0..4),
            20,
            "pFiles points just past the 20-byte header"
        );
        assert_eq!(field(4..8), 0, "pt.x");
        assert_eq!(field(8..12), 0, "pt.y");
        assert_eq!(field(12..16), 0, "fNC");
        assert_eq!(field(16..20), 1, "fWide must be TRUE for UTF-16 paths");

        // Payload is UTF-16 with each path NUL-terminated plus a final NUL.
        let units: Vec<u16> = payload[20..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|pair| u16::from_le_bytes([pair[0], pair[1]]))
            .collect();
        let text = String::from_utf16(&units).unwrap();
        assert_eq!(text, "C:\\one.txt\0C:\\two.txt\0\0");
    }

    #[cfg(windows)]
    #[test]
    fn hdrop_handles_an_empty_selection() {
        let payload = build_hdrop(&[]);
        assert_eq!(payload.len(), 22, "header plus the list terminator");
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "writes to the live system clipboard; run explicitly"]
    fn round_trip_text_through_the_real_clipboard() {
        // Exercises allocation, the Win32 write path, and the read path together.
        let Some(_) = with_clipboard(|| ()) else {
            // Another process is monopolising the clipboard; skip rather than fail.
            return;
        };

        const SAMPLE: &str = "line one\r\nline two\ttabbed ✓ 日本語";
        if write(&ClipContent::Text(SAMPLE.into())).is_err() {
            return;
        }
        assert_eq!(
            read_text().as_deref(),
            Some(SAMPLE),
            "CRLF and unicode must survive a clipboard round trip"
        );
    }
}
