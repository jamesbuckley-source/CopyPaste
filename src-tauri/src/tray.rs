//! System tray icon and menu.
//!
//! The app has no taskbar button (`skipTaskbar`), so the tray is the discoverable
//! way to reach it when the hotkey is unknown or unbound.

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, Runtime};

use crate::AppState;
use crate::error::Result;
use crate::window;

pub const TRAY_ID: &str = "copypaste-tray";

pub fn install<R: Runtime>(app: &AppHandle<R>) -> Result<()> {
    let open = MenuItem::with_id(app, "open", "Open CopyPaste", true, None::<&str>)?;
    let clear = MenuItem::with_id(app, "clear", "Clear unpinned history", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "Settings…", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;

    let menu = Menu::with_items(app, &[&open, &clear, &settings, &separator, &quit])?;

    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("CopyPaste — clipboard history")
        .menu(&menu)
        // Left click should reveal the window, not open the menu; the menu is on
        // right click, matching the platform convention for tray utilities.
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => window::show(app),
            "settings" => window::show_settings(app),
            "clear" => {
                let state = app.state::<AppState>();
                match state.store.clear(true) {
                    Ok(0) => log::info!("nothing to clear"),
                    Ok(removed) => {
                        log::info!("cleared {removed} clip(s)");
                        let _ = app.emit(crate::clipboard::EVENT_HISTORY_CHANGED, ());
                    }
                    Err(e) => log::warn!("could not clear history: {e}"),
                }
            }
            "quit" => app.exit(0),
            other => log::debug!("unhandled tray menu item `{other}`"),
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let app = tray.app_handle();
                // Remember the app the user is coming from so a later auto-paste
                // has somewhere to land.
                crate::paste::capture_foreground(window::own_window_handle(app));
                window::toggle(app);
            }
        });

    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    } else {
        log::warn!("no default window icon available; the tray icon will be blank");
    }

    builder.build(app)?;
    Ok(())
}
