//! Show/hide plumbing for the overlay window.
//!
//! The window is a frameless panel that starts hidden and is not on top unless
//! the user asks for it. Every entry point that reveals it (tray icon, global
//! hotkey, a command from the UI) goes through here so that focus and z-order
//! behave consistently.

use tauri::{AppHandle, Emitter, Manager, Runtime, WebviewWindow};

use crate::models::{ClipDetail, ClipKind};

/// Must match `app.windows[0].label` in tauri.conf.json.
pub const WINDOW_LABEL: &str = "main";

/// Label of the separate settings window; also listed in the capability file.
pub const SETTINGS_LABEL: &str = "settings";

/// Label of the small "New clip" window; also listed in the capability file.
pub const ADD_CLIP_LABEL: &str = "add-clip";

/// Prefix shared by the independent window for each sticky clip.
pub const STICKY_PREFIX: &str = "sticky-";

/// Emitted to tell the frontend which pane should be in front. The main window
/// is reused rather than opening a second one for settings.
pub const EVENT_ROUTE: &str = "ui:route";

pub const ROUTE_HISTORY: &str = "history";

pub fn window<R: Runtime>(app: &AppHandle<R>) -> Option<WebviewWindow<R>> {
    app.get_webview_window(WINDOW_LABEL)
}

/// Raw handle of our own window, used to avoid mistaking it for "the app the user
/// came from" when deciding where an auto-paste should land.
#[cfg(windows)]
pub fn own_window_handle<R: Runtime>(app: &AppHandle<R>) -> Option<isize> {
    let window = window(app)?;
    window.hwnd().ok().map(|handle| handle.0 as isize)
}

#[cfg(not(windows))]
pub fn own_window_handle<R: Runtime>(_app: &AppHandle<R>) -> Option<isize> {
    None
}

pub fn show<R: Runtime>(app: &AppHandle<R>) {
    show_route(app, ROUTE_HISTORY);
}

pub fn show_settings<R: Runtime>(app: &AppHandle<R>) {
    if app.get_webview_window(SETTINGS_LABEL).is_some() {
        if let Err(e) = open_settings(app) {
            log::warn!("could not focus the settings window: {e}");
        }
        return;
    }
    // Webviews can only be created from the event loop thread, and the call
    // blocks until that thread is free, so it must not run on the caller's
    // thread when the caller is the tray menu or a command.
    let handle = app.clone();
    if let Err(e) = std::thread::Builder::new()
        .name("settings-window".into())
        .spawn(move || {
            if let Err(e) = open_settings(&handle) {
                log::warn!("could not open the settings window: {e}");
            }
        })
    {
        log::warn!("could not spawn the settings window thread: {e}");
    }
}

/// Opens the settings page in its own decorated window.
///
/// A separate window keeps the overlay a pure history surface: the settings page
/// is long, has its own scrolling, and benefits from ordinary minimise and
/// maximise behaviour.
pub fn open_settings<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    const LABEL: &str = SETTINGS_LABEL;
    if let Some(existing) = app.get_webview_window(LABEL) {
        let _ = existing.show();
        let _ = existing.set_focus();
        return Ok(());
    }
    let window = tauri::WebviewWindowBuilder::new(
        app,
        LABEL,
        // The query is how the page knows it is the settings window. Asking Tauri
        // for the label works too, but only once its internals are installed, and a
        // webview that renders before that shows the history UI — the "settings
        // window became another main window" bug.
        tauri::WebviewUrl::App("index.html?window=settings".into()),
    )
    .title("CopyPaste Settings")
    .inner_size(780.0, 820.0)
    .min_inner_size(560.0, 480.0)
    .always_on_top(always_on_top(app))
    .center()
    .build()?;
    let _ = window.set_focus();
    Ok(())
}

pub fn show_add_clip<R: Runtime>(app: &AppHandle<R>) {
    if app.get_webview_window(ADD_CLIP_LABEL).is_some() {
        if let Err(e) = open_add_clip(app) {
            log::warn!("could not focus the add-clip window: {e}");
        }
        return;
    }
    // Same constraint as the settings window: webviews have to be created from the
    // event loop thread, so the call cannot run on the caller's thread.
    let handle = app.clone();
    if let Err(e) = std::thread::Builder::new()
        .name("add-clip-window".into())
        .spawn(move || {
            if let Err(e) = open_add_clip(&handle) {
                log::warn!("could not open the add-clip window: {e}");
            }
        })
    {
        log::warn!("could not spawn the add-clip window thread: {e}");
    }
}

/// Opens the "New clip" form in its own decorated window.
///
/// Typing a clip is a short, deliberate act, and it gets a compact window rather
/// than a pane in the overlay so the history stays visible behind it.
pub fn open_add_clip<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    const LABEL: &str = ADD_CLIP_LABEL;
    if let Some(existing) = app.get_webview_window(LABEL) {
        let _ = existing.show();
        let _ = existing.set_focus();
        return Ok(());
    }
    let window = tauri::WebviewWindowBuilder::new(
        app,
        LABEL,
        // Same reason as the settings window: say which window this is in the URL.
        tauri::WebviewUrl::App("index.html?window=add-clip".into()),
    )
    .title("New clip")
    .inner_size(460.0, 420.0)
    .min_inner_size(360.0, 280.0)
    .always_on_top(always_on_top(app))
    .center()
    .build()?;
    let _ = window.set_focus();
    Ok(())
}

pub fn show_sticky_clip<R: Runtime>(app: &AppHandle<R>, id: i64, size: (f64, f64)) {
    let label = format!("{STICKY_PREFIX}{id}");
    if let Some(existing) = app.get_webview_window(&label) {
        let _ = existing.show();
        let _ = existing.set_focus();
        return;
    }

    let handle = app.clone();
    if let Err(e) = std::thread::Builder::new()
        .name(format!("sticky-clip-{id}"))
        .spawn(move || {
            if let Err(e) = open_sticky_clip(&handle, id, size) {
                log::warn!("could not open sticky clip {id}: {e}");
            }
        })
    {
        log::warn!("could not spawn sticky clip {id} window: {e}");
    }
}

fn open_sticky_clip<R: Runtime>(
    app: &AppHandle<R>,
    id: i64,
    size: (f64, f64),
) -> tauri::Result<()> {
    let label = format!("{STICKY_PREFIX}{id}");
    if let Some(existing) = app.get_webview_window(&label) {
        let _ = existing.show();
        let _ = existing.set_focus();
        return Ok(());
    }
    let window = tauri::WebviewWindowBuilder::new(
        app,
        label,
        tauri::WebviewUrl::App(format!("index.html?window=sticky&id={id}").into()),
    )
    .title("Sticky clip")
    .inner_size(size.0, size.1)
    .min_inner_size(220.0, 160.0)
    .resizable(true)
    .decorations(false)
    .always_on_top(true)
    .build()?;
    let _ = window.set_focus();
    Ok(())
}

pub fn sticky_clip_size(detail: &ClipDetail) -> (f64, f64) {
    const MAX_WIDTH: f64 = 1_000.0;
    const MAX_HEIGHT: f64 = 800.0;
    const HEADER_HEIGHT: f64 = 36.0;

    match detail.summary.kind {
        ClipKind::Image => {
            let width = detail.summary.width.unwrap_or(480).max(1) as f64;
            let height = detail.summary.height.unwrap_or(320).max(1) as f64;
            let scale = 1.0_f64
                .min((MAX_WIDTH - 24.0) / width)
                .min((MAX_HEIGHT - HEADER_HEIGHT - 24.0) / height);
            (width * scale + 24.0, height * scale + HEADER_HEIGHT + 24.0)
        }
        ClipKind::Text => {
            let text = detail.text.as_deref().unwrap_or(&detail.summary.preview);
            let lines = text.lines().count().max(1);
            let longest_line = text
                .lines()
                .map(|line| line.chars().count())
                .max()
                .unwrap_or(1);
            (
                (longest_line as f64 * 8.0 + 32.0).clamp(320.0, MAX_WIDTH),
                (lines as f64 * 20.0 + HEADER_HEIGHT + 24.0).clamp(180.0, MAX_HEIGHT),
            )
        }
        ClipKind::Files => {
            let longest_path = detail
                .file_paths
                .iter()
                .map(|path| path.chars().count())
                .max()
                .unwrap_or(32);
            (
                (longest_path as f64 * 8.0 + 32.0).clamp(360.0, MAX_WIDTH),
                (detail.file_paths.len() as f64 * 22.0 + HEADER_HEIGHT + 24.0)
                    .clamp(180.0, MAX_HEIGHT),
            )
        }
    }
}

fn always_on_top<R: Runtime>(app: &AppHandle<R>) -> bool {
    app.state::<crate::AppState>()
        .store
        .settings()
        .always_on_top
}

pub fn show_route<R: Runtime>(app: &AppHandle<R>, route: &'static str) {
    if let Some(window) = window(app) {
        // Closing a minimised window hides it in that state; showing it again has
        // to restore the size it had before it was minimised.
        if window.is_minimized().unwrap_or(false) {
            let _ = window.unminimize();
        }
        if let Err(e) = window.show() {
            log::warn!("could not show the window: {e}");
        }
        // Focus has to follow the show, otherwise the window appears without
        // keyboard focus and the search box cannot be typed into.
        if let Err(e) = window.set_focus() {
            log::warn!("could not focus the window: {e}");
        }
        // Addressed to this window only. `emit` would broadcast to every webview
        // in the app, and the settings window — which loads the same page and
        // listens for the same event — would switch itself to the history UI. That
        // is what made it "turn into another main window" after a screenshot:
        // restoring the main window after a capture is a route change like any
        // other.
        let _ = window.emit_to(window.label(), EVENT_ROUTE, route);
    }
}

pub fn hide<R: Runtime>(app: &AppHandle<R>) {
    if let Some(window) = window(app)
        && let Err(e) = window.hide()
    {
        log::warn!("could not hide the window: {e}");
    }
}

/// Shows or hides the taskbar button. `skipTaskbar` is the inverse of the setting,
/// which is why this takes the user-facing value rather than the flag.
pub fn set_taskbar_visibility<R: Runtime>(app: &AppHandle<R>, show: bool) {
    if let Some(window) = window(app)
        && let Err(e) = window.set_skip_taskbar(!show)
    {
        log::warn!("could not change taskbar visibility: {e}");
    }
}

/// Applies the user's z-order preference to app windows; sticky clips stay on top.
pub fn set_always_on_top<R: Runtime>(app: &AppHandle<R>, enabled: bool) {
    for (label, window) in app.webview_windows() {
        let keep_on_top = enabled || label.starts_with(STICKY_PREFIX);
        if let Err(e) = window.set_always_on_top(keep_on_top) {
            log::warn!("could not change always-on-top for {label}: {e}");
        }
    }
}

pub fn is_visible<R: Runtime>(app: &AppHandle<R>) -> bool {
    window(app)
        .and_then(|window| window.is_visible().ok())
        .unwrap_or(false)
}

/// Shows the window if hidden, hides it if visible.
pub fn toggle<R: Runtime>(app: &AppHandle<R>) {
    // A minimised window reports itself as visible, so treating "visible" as
    // "already on screen" would hide it instead of restoring it from the taskbar.
    let minimized = window(app)
        .and_then(|window| window.is_minimized().ok())
        .unwrap_or(false);
    if is_visible(app) && !minimized {
        hide(app);
    } else {
        show(app);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ClipKind, ClipSummary};

    fn detail(kind: ClipKind, width: Option<i64>, height: Option<i64>) -> ClipDetail {
        ClipDetail {
            summary: ClipSummary {
                id: 1,
                kind,
                preview: String::new(),
                byte_size: 0,
                source_app: None,
                pinned: false,
                created_at: 0,
                updated_at: 0,
                copy_count: 0,
                width,
                height,
                thumb_file: None,
                image_file: None,
                media_locked: false,
                file_count: 0,
                char_count: 0,
                has_ocr: false,
                has_note: false,
                color: None,
                content_kind: None,
                image_path: None,
                animated: false,
                video: false,
                motion_pending: false,
                collection_ids: Vec::new(),
                tags: Vec::new(),
                secret_level: None,
                face_count: 0,
            },
            text: None,
            file_paths: Vec::new(),
            ocr_text: None,
            note: None,
            has_rich: false,
        }
    }

    #[test]
    fn large_sticky_images_fit_within_window_bounds() {
        let size = sticky_clip_size(&detail(ClipKind::Image, Some(1920), Some(1080)));
        assert!(size.0 <= 1_000.0);
        assert!(size.1 <= 800.0);
    }

    #[test]
    fn text_sticky_windows_grow_to_fit_content() {
        let mut clip = detail(ClipKind::Text, None, None);
        clip.text = Some(format!(
            "{}\nsecond\nthird\nfourth\nfifth\nsixth\nseventh\neighth",
            "x".repeat(80)
        ));

        let size = sticky_clip_size(&clip);

        assert!(size.0 > 320.0);
        assert!(size.1 > 180.0);
    }

    #[test]
    fn file_sticky_windows_grow_to_fit_file_lists() {
        let mut clip = detail(ClipKind::Files, None, None);
        clip.file_paths = vec![format!(r"C:\{}", "x".repeat(80)); 8];

        let size = sticky_clip_size(&clip);

        assert!(size.0 > 360.0);
        assert!(size.1 > 180.0);
    }
}
