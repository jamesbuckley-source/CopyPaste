//! Show/hide plumbing for the overlay window.
//!
//! The window is a frameless, always-on-top panel that starts hidden. Every entry
//! point that reveals it (tray icon, global hotkey, a command from the UI) goes
//! through here so that focus and z-order behave consistently.

use tauri::{AppHandle, Emitter, Manager, Runtime, WebviewWindow};

/// Must match `app.windows[0].label` in tauri.conf.json.
pub const WINDOW_LABEL: &str = "main";

/// Emitted to tell the frontend which pane should be in front. The main window
/// is reused rather than opening a second one for settings.
pub const EVENT_ROUTE: &str = "ui:route";

pub const ROUTE_HISTORY: &str = "history";
pub const ROUTE_SETTINGS: &str = "settings";

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
    show_route(app, ROUTE_SETTINGS);
}

pub fn show_route<R: Runtime>(app: &AppHandle<R>, route: &'static str) {
    if let Some(window) = window(app) {
        if let Err(e) = window.show() {
            log::warn!("could not show the window: {e}");
        }
        // Focus has to follow the show, otherwise the window appears without
        // keyboard focus and the search box cannot be typed into.
        if let Err(e) = window.set_focus() {
            log::warn!("could not focus the window: {e}");
        }
        let _ = window.emit(EVENT_ROUTE, route);
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

pub fn is_visible<R: Runtime>(app: &AppHandle<R>) -> bool {
    window(app)
        .and_then(|window| window.is_visible().ok())
        .unwrap_or(false)
}

/// Shows the window if hidden, hides it if visible.
pub fn toggle<R: Runtime>(app: &AppHandle<R>) {
    if is_visible(app) {
        hide(app);
    } else {
        show(app);
    }
}
