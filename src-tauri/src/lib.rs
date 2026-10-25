//! CopyPaste — a local-only clipboard history for Windows.
//!
//! Architecture in one paragraph: a background thread watches the Windows
//! clipboard sequence number and writes new payloads into SQLite. The database
//! stores clip bodies and previews as plain text (an older encrypted format is
//! migrated on startup). A global
//! hotkey summons a frameless overlay window whose frontend talks to this backend
//! through the commands in [`commands`]. Image bytes live beside the database and
//! are served to the webview through a dedicated `media` URI scheme.

mod animated;
mod capture;
mod clipboard;
mod color;
mod commands;
mod crypto;
mod error;
mod faces;
mod giphy;
mod hashing;
mod hotkey;
mod markdown;
mod media;
mod models;
mod ocr;
mod palette;
mod paste;
mod picker;
mod printscreen;
mod search;
mod secrets;
mod settings;
mod sound;
mod storage;
mod stitch;
mod store;
mod tray;
#[cfg(windows)]
mod uia;
mod vault;
mod window;

use std::sync::Arc;

use tauri::{Emitter, Manager, RunEvent};
use tauri_plugin_autostart::MacosLauncher;
use tauri_plugin_global_shortcut::{Shortcut, ShortcutEvent, ShortcutState};

use crate::clipboard::Watcher;
use crate::store::Store;

/// State shared between the setup hook, the tray, the hotkey handler and the IPC
/// commands.
pub struct AppState {
    pub store: Arc<Store>,
    pub watcher: Arc<Watcher>,
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // Must be registered before anything else: it decides whether this process
        // is the instance that gets to own the tray icon and the global hotkey.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            // A second launch (Start Menu, taskbar, double-clicking the exe) should
            // surface the running instance rather than spawn a rival tray icon and
            // a shortcut that cannot register twice.
            log::info!("another instance was launched; showing the existing window");
            window::show(app);
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(
            tauri_plugin_log::Builder::new()
                .level(log::LevelFilter::Info)
                .build(),
        )
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(on_shortcut)
                .build(),
        )
        // Serves image bytes to the webview. A custom scheme is used instead of the
        // asset protocol so no filesystem scope has to be granted to the frontend.
        .register_asynchronous_uri_scheme_protocol("media", |ctx, request, responder| {
            let app = ctx.app_handle().clone();
            let path = request.uri().path().to_string();
            std::thread::spawn(move || responder.respond(serve_media(&app, &path)));
        })
        // Closing is hiding. This is a tray app whose entire value is being there
        // when the hotkey is pressed, so destroying the window on Alt+F4 would
        // silently kill the clipboard watcher. Quitting is the tray menu's job.
        .on_window_event(|win, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event
                && win.label() == window::WINDOW_LABEL
            {
                api.prevent_close();
                if let Err(e) = win.hide() {
                    log::warn!("could not hide the window on close: {e}");
                }
            }
        })
        .setup(|app| {
            setup(app)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::list_clips,
            commands::get_clip,
            commands::get_stats,
            commands::set_clip_text,
            commands::set_clip_note,
            commands::set_clip_secret,
            commands::vault_status,
            commands::create_vault,
            commands::unlock_vault,
            commands::lock_vault,
            commands::change_vault_passphrase,
            commands::pick_screen_color,
            commands::capture_screen_region,
            commands::capture_screen_window,
            commands::capture_whole_screen,
    commands::capture_scrolling_window,
            commands::open_settings_window,
            commands::open_add_clip_window,
            commands::create_text_clip,
            commands::import_dropped_files,
            commands::search_giphy,
            commands::copy_gif,
            commands::import_custom_gif,
            commands::copy_custom_gif,
            commands::copy_clip,
            commands::set_pinned,
            commands::delete_clip,
            commands::delete_clips,
            commands::clear_history,
            commands::prune_history,
            commands::list_collections,
            commands::create_collection,
            commands::rename_collection,
            commands::delete_collection,
            commands::set_clip_collections,
            commands::set_clip_archive,
            commands::set_clip_saved_gif,
            commands::list_tags,
            commands::set_clip_tags,
            commands::rename_tag,
            commands::delete_tag,
            commands::set_tag_color,
            commands::set_pinned_many,
            commands::add_tags_many,
            commands::add_to_collection,
            commands::save_clip,
            commands::save_clips,
            commands::export_markdown,
            commands::copy_text,
            commands::copy_color,
            commands::save_color_palette,
            commands::open_url,
            commands::open_email,
            commands::reveal_path,
            commands::open_path,
            commands::get_rich_text,
            commands::capture_state,
            commands::pause_capture,
            commands::resume_capture,
            commands::set_collection_archived,
            commands::copy_ocr_text,
            commands::ensure_color_scheme,
            commands::add_scheme_color,
            commands::reveal_clip,
            commands::reveal_archive,
            commands::get_settings,
            commands::update_settings,
            commands::validate_hotkey,
            commands::hide_window,
            commands::toggle_window,
            commands::reveal_data_dir,
            commands::storage_info,
            commands::set_storage_dir,
            commands::history_is_empty,
        ])
        .build(tauri::generate_context!())
        .expect("error while building the tauri application")
        .run(|app, event| {
            if let RunEvent::Exit = event {
                hotkey::release(app);
                if let Some(state) = app.try_state::<AppState>() {
                    state.watcher.stop();
                }
                log::info!("shutting down");
            }
        });
}

fn setup(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    let handle = app.handle().clone();

    let data_dir = app.path().app_data_dir()?;
    let default_dir = data_dir.clone();
    let data_dir = storage::active_dir(&default_dir);
    log::info!("data directory: {}", data_dir.display());

    let store = Arc::new(Store::open(&data_dir)?);
    // A move completed on the previous run: the old copy is no longer in use.
    storage::cleanup_previous(&default_dir, store.data_dir());

    // Images captured before text recognition existed are processed once, in the
    // background, so startup never blocks on them.
    {
        let ocr_store = Arc::clone(&store);
        let ocr_app = handle.clone();
        let spawned = std::thread::Builder::new()
            .name("ocr-backfill".into())
            .spawn(move || {
                let processed = ocr_store.ocr_pending();
                if processed > 0 {
                    log::info!("recognised text in {processed} existing image(s)");
                    let _ = ocr_app.emit(clipboard::EVENT_HISTORY_CHANGED, ());
                }
                if !ocr_store.settings().detect_faces {
                    return;
                }
                let found = ocr_store.faces_pending();
                if found > 0 {
                    log::info!("looked for faces in {found} existing image(s)");
                    let _ = ocr_app.emit(clipboard::EVENT_HISTORY_CHANGED, ());
                }
            });
        if let Err(e) = spawned {
            log::warn!("could not start the image backfill thread: {e}");
        }
    }
    let watcher = Arc::new(Watcher::new());
    let settings = store.settings();

    // Retention is also enforced at capture time, but a database left untouched for
    // weeks should not stay oversized until the next copy happens.
    match store.prune() {
        Ok(0) => {}
        Ok(removed) => log::info!("startup retention pass removed {removed} clip(s)"),
        Err(e) => log::warn!("startup retention pass failed: {e}"),
    }

    app.manage(AppState {
        store: Arc::clone(&store),
        watcher: Arc::clone(&watcher),
    });

    // Applied before the window is ever shown so the taskbar button matches the
    // saved setting from the very first paint.
    window::set_taskbar_visibility(&handle, settings.show_in_taskbar);
    // The config declares the window not-on-top; the preference decides.
    window::set_always_on_top(&handle, settings.always_on_top);

    if let Err(e) = tray::install(&handle) {
        // A missing tray icon degrades discoverability, but the hotkey still works.
        log::warn!("could not create the tray icon: {e}");
    }

    let bindings = hotkey::Bindings::from_settings(&settings);
    if let Err(e) = hotkey::apply(&handle, &hotkey::Bindings::default(), &bindings) {
        log::warn!("{e}");
        // A capture shortcut that another application owns, or a stale value
        // that no longer parses, must not cost the user the summon key.
        let summon_only = bindings.without_capture_shortcuts();
        if let Err(e) = hotkey::apply(&handle, &hotkey::Bindings::default(), &summon_only) {
            log::warn!("{e}");
        }
    }

    sync_autostart(&handle, settings.launch_at_login);
    watcher.start(handle.clone(), Arc::clone(&store));

    // Stay out of the way on every launch except the very first, where the window
    // doubles as an explanation of what just installed itself.
    if !settings.start_hidden || store.is_empty() {
        window::show(&handle);
    }

    Ok(())
}

fn sync_autostart(app: &tauri::AppHandle, enabled: bool) {
    use tauri_plugin_autostart::ManagerExt;
    let manager = app.autolaunch();
    let outcome = if enabled {
        manager.enable()
    } else {
        manager.disable()
    };
    if let Err(e) = outcome {
        log::warn!("could not sync the launch-at-login setting: {e}");
    }
}

fn on_shortcut(app: &tauri::AppHandle, shortcut: &Shortcut, event: ShortcutEvent) {
    if event.state() != ShortcutState::Pressed {
        return;
    }
    let settings = app.state::<AppState>().store.settings();
    // The pause shortcut is a toggle and must never summon the window: it exists
    // for the moments when the user does not want anything recorded or shown.
    if shortcut
        .to_string()
        .eq_ignore_ascii_case(&settings.pause_hotkey)
    {
        commands::toggle_pause(app);
        return;
    }
    // Print Screen takes the default capture type, so the key and the header
    // split button always mean the same thing.
    if hotkey::is_print_screen(shortcut, &settings) {
        commands::run_capture_shortcut(app, hotkey::default_capture(&settings));
        return;
    }
    // A capture shortcut runs the very command its toolbar button calls, so the
    // keyboard and the button cannot drift apart.
    if let Some(action) = hotkey::capture_action(shortcut, &settings) {
        commands::run_capture_shortcut(app, action);
        return;
    }
    // Capture the outgoing foreground window *before* showing ours, so that a later
    // auto-paste knows where the keystroke should land.
    paste::capture_foreground(window::own_window_handle(app));
    window::toggle(app);
}

fn serve_media(app: &tauri::AppHandle, path: &str) -> tauri::http::Response<Vec<u8>> {
    use tauri::http::Response;

    fn not_found() -> Response<Vec<u8>> {
        Response::builder()
            .status(404)
            .body(Vec::new())
            .expect("a static 404 response is always valid")
    }

    // Only the final segment matters: thumbnails and full images live in the same
    // directory under distinct names, so a bare file name is unambiguous. Taking
    // the last segment (rather than a fixed `/kind/name` shape) also lets the
    // frontend hand a plain name to `convertFileSrc`, which percent-encodes its
    // whole argument and would mangle an embedded separator.
    //
    // `media::resolve` is the allowlist that keeps traversal out.
    let name = path.rsplit('/').next().unwrap_or_default();

    let Some(state) = app.try_state::<AppState>() else {
        return not_found();
    };
    if media::resolve(state.store.media_dir(), name).is_none() {
        return not_found();
    }
    // A sealed file is decrypted here, in memory, and never on disk: the whole
    // point is that the pixels are ciphertext everywhere except a live window.
    let Ok(Some(bytes)) = state.store.media_bytes(name) else {
        return not_found();
    };

    Response::builder()
        .status(200)
        .header(
            "Content-Type",
            if name.to_ascii_lowercase().ends_with(".gif") {
                "image/gif"
            } else {
                "image/png"
            },
        )
        // File names are random UUIDs, so the bytes behind a name never change.
        .header("Cache-Control", "public, max-age=31536000, immutable")
        .body(bytes)
        .unwrap_or_else(|_| not_found())
}
