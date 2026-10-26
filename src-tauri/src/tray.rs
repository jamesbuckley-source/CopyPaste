//! System tray icon and menu.
//!
//! The app has no taskbar button (`skipTaskbar`), so the tray is the discoverable
//! way to reach it when the hotkey is unknown or unbound.
//!
//! The menu is rebuilt whenever its contents depend on state — capture being
//! paused, the vault being open — because a menu built once at startup would keep
//! offering "Resume capture" while nothing is paused and would never show
//! "Lock vault" after the vault was unlocked.

use std::time::{SystemTime, UNIX_EPOCH};

use tauri::menu::{IsMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};

use crate::AppState;
use crate::clipboard::PauseState;
use crate::error::Result;
use crate::hotkey::CaptureAction;
use crate::window;

pub const TRAY_ID: &str = "copypaste-tray";

const ID_NEW_CLIP: &str = "new-clip";
const ID_CAPTURE_REGION: &str = "capture-region";
const ID_CAPTURE_WINDOW: &str = "capture-window";
const ID_CAPTURE_SCREEN: &str = "capture-screen";
const ID_CAPTURE_SCROLLING: &str = "capture-scrolling";
const ID_CAPTURE_COLOR: &str = "capture-color";
const ID_PAUSE_15: &str = "pause-15";
const ID_PAUSE_60: &str = "pause-60";
const ID_PAUSE_RESTART: &str = "pause-restart";
const ID_RESUME: &str = "resume-capture";
const ID_SETTINGS: &str = "settings";
const ID_DATA_FOLDER: &str = "data-folder";
const ID_LOCK_VAULT: &str = "lock-vault";
const ID_QUIT: &str = "quit";

const TOOLTIP: &str = "CopyPaste — clipboard history";

pub fn install(app: &AppHandle) -> Result<()> {
    let menu = build_menu(app)?;

    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip(tooltip(app))
        .menu(&menu)
        // Left click should reveal the window, not open the menu; the menu is on
        // right click, matching the platform convention for tray utilities.
        .show_menu_on_left_click(false)
        .on_menu_event(handle_menu_event)
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

/// Rebuilds the menu and tooltip after something they reflect has changed.
///
/// Safe to call from any thread: Tauri hops to the event loop thread itself when
/// a menu is constructed and when it is attached to the tray.
pub fn refresh(app: &AppHandle) {
    let Some(tray) = app.tray_by_id(TRAY_ID) else {
        return;
    };
    match build_menu(app) {
        Ok(menu) => {
            if let Err(e) = tray.set_menu(Some(menu)) {
                log::warn!("could not rebuild the tray menu: {e}");
            }
        }
        Err(e) => log::warn!("could not rebuild the tray menu: {e}"),
    }
    if let Err(e) = tray.set_tooltip(Some(tooltip(app))) {
        log::warn!("could not update the tray tooltip: {e}");
    }
}

fn build_menu(app: &AppHandle) -> Result<Menu<tauri::Wry>> {
    let new_clip = MenuItem::with_id(app, ID_NEW_CLIP, "New clip…", true, None::<&str>)?;

    let region = MenuItem::with_id(app, ID_CAPTURE_REGION, "Region", true, None::<&str>)?;
    let capture_window = MenuItem::with_id(app, ID_CAPTURE_WINDOW, "Window", true, None::<&str>)?;
    let screen = MenuItem::with_id(app, ID_CAPTURE_SCREEN, "Full screen", true, None::<&str>)?;
    let scrolling = MenuItem::with_id(app, ID_CAPTURE_SCROLLING, "Scrolling", true, None::<&str>)?;
    let color = MenuItem::with_id(app, ID_CAPTURE_COLOR, "Colour picker", true, None::<&str>)?;
    let capture = Submenu::with_items(
        app,
        "Capture",
        true,
        &[&region, &capture_window, &screen, &scrolling, &color],
    )?;

    let pause_state = crate::commands::pause_state(app);
    let pause = if pause_state.paused {
        None
    } else {
        let short = MenuItem::with_id(app, ID_PAUSE_15, "15 minutes", true, None::<&str>)?;
        let hour = MenuItem::with_id(app, ID_PAUSE_60, "1 hour", true, None::<&str>)?;
        let restart = MenuItem::with_id(app, ID_PAUSE_RESTART, "Until restart", true, None::<&str>)?;
        Some(Submenu::with_items(
            app,
            "Pause capture",
            true,
            &[&short, &hour, &restart],
        )?)
    };
    let resume = if pause_state.paused {
        Some(MenuItem::with_id(
            app,
            ID_RESUME,
            format!("Resume capture ({})", paused_for(&pause_state)),
            true,
            None::<&str>,
        )?)
    } else {
        None
    };

    let separator_top = PredefinedMenuItem::separator(app)?;
    let settings = MenuItem::with_id(app, ID_SETTINGS, "Settings…", true, None::<&str>)?;
    let data_folder = MenuItem::with_id(app, ID_DATA_FOLDER, "Data folder", true, None::<&str>)?;
    let lock_vault = if vault_unlocked(app) {
        Some(MenuItem::with_id(
            app,
            ID_LOCK_VAULT,
            "Lock vault",
            true,
            None::<&str>,
        )?)
    } else {
        None
    };
    let separator_bottom = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, ID_QUIT, "Quit", true, None::<&str>)?;

    let mut items: Vec<&dyn IsMenuItem<tauri::Wry>> = vec![&new_clip, &capture];
    if let Some(pause) = &pause {
        items.push(pause);
    }
    if let Some(resume) = &resume {
        items.push(resume);
    }
    items.push(&separator_top);
    items.push(&settings);
    items.push(&data_folder);
    if let Some(lock_vault) = &lock_vault {
        items.push(lock_vault);
    }
    items.push(&separator_bottom);
    items.push(&quit);

    Ok(Menu::with_items(app, &items)?)
}

fn handle_menu_event(app: &AppHandle, event: MenuEvent) {
    match event.id.as_ref() {
        ID_NEW_CLIP => window::show_add_clip(app),
        ID_CAPTURE_REGION => crate::commands::run_capture_shortcut(app, CaptureAction::Region),
        ID_CAPTURE_WINDOW => crate::commands::run_capture_shortcut(app, CaptureAction::Window),
        ID_CAPTURE_SCREEN => crate::commands::run_capture_shortcut(app, CaptureAction::Screen),
        ID_CAPTURE_SCROLLING => {
            crate::commands::run_capture_shortcut(app, CaptureAction::Scrolling)
        }
        ID_CAPTURE_COLOR => crate::commands::run_capture_shortcut(app, CaptureAction::Color),
        ID_PAUSE_15 => {
            crate::commands::pause_capture(app.clone(), Some(15), None);
        }
        ID_PAUSE_60 => {
            crate::commands::pause_capture(app.clone(), Some(60), None);
        }
        ID_PAUSE_RESTART => {
            crate::commands::pause_capture(app.clone(), None, Some(true));
        }
        ID_RESUME => {
            crate::commands::resume_capture(app.clone());
        }
        ID_SETTINGS => window::show_settings(app),
        ID_DATA_FOLDER => {
            if let Err(e) = crate::commands::open_data_dir(app) {
                log::warn!("could not open the data folder: {e}");
            }
        }
        ID_LOCK_VAULT => {
            if let Err(e) = crate::commands::lock_vault_now(app) {
                log::warn!("could not lock the vault: {e}");
            }
        }
        ID_QUIT => app.exit(0),
        other => log::debug!("unhandled tray menu item `{other}`"),
    }
}

fn vault_unlocked(app: &AppHandle) -> bool {
    app.state::<AppState>().store.vault_status() == "unlocked"
}

fn tooltip(app: &AppHandle) -> String {
    let state = crate::commands::pause_state(app);
    if state.paused {
        format!("{TOOLTIP} — {}", paused_for(&state))
    } else {
        TOOLTIP.to_string()
    }
}

/// How long the current pause has left, phrased for a menu label or tooltip.
fn paused_for(state: &PauseState) -> String {
    if state.indefinite {
        return "until restart".into();
    }
    let Some(until) = state.until else {
        return "paused".into();
    };
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or_default();
    match (until - now + 59_999) / 60_000 {
        minutes if minutes > 0 => format!("{minutes} min left"),
        _ => "paused".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn now_ms() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or_default()
    }

    fn timed_in(ms: i64) -> PauseState {
        PauseState {
            paused: true,
            until: Some(now_ms() + ms),
            indefinite: false,
        }
    }

    #[test]
    fn pause_until_restart_says_so() {
        let state = PauseState {
            paused: true,
            until: None,
            indefinite: true,
        };
        assert_eq!(paused_for(&state), "until restart");
    }

    #[test]
    fn a_timed_pause_rounds_up_to_whole_minutes() {
        assert_eq!(paused_for(&timed_in(90_000)), "2 min left");
        assert_eq!(paused_for(&timed_in(15 * 60_000)), "15 min left");
    }

    #[test]
    fn an_expired_pause_reads_as_plain_paused() {
        let mut state = timed_in(0);
        state.until = Some(now_ms() - 5_000);
        assert_eq!(paused_for(&state), "paused");
    }
}
