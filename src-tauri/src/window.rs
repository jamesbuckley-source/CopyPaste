//! Show/hide plumbing for the overlay window.
//!
//! The window is a frameless panel that starts hidden and is not on top unless
//! the user asks for it. Every entry point that reveals it (tray icon, global
//! hotkey, a command from the UI) goes through here so that focus and z-order
//! behave consistently.

use tauri::{AppHandle, Emitter, Manager, Runtime, WebviewWindow};

/// Must match `app.windows[0].label` in tauri.conf.json.
pub const WINDOW_LABEL: &str = "main";

/// Label of the separate settings window; also listed in the capability file.
pub const SETTINGS_LABEL: &str = "settings";

/// Label of the small "New clip" window; also listed in the capability file.
pub const ADD_CLIP_LABEL: &str = "add-clip";

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
    .center()
    .build()?;
    let _ = window.set_focus();
    Ok(())
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

/// Keeps the overlay above other windows. Off by default so it behaves like an
/// ordinary window unless the user asks otherwise.
pub fn set_always_on_top<R: Runtime>(app: &AppHandle<R>, enabled: bool) {
    if let Some(window) = window(app)
        && let Err(e) = window.set_always_on_top(enabled)
    {
        log::warn!("could not change always-on-top: {e}");
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
