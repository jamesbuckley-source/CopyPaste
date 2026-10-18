//! The IPC surface exposed to the webview.
//!
//! Commands are intentionally coarse (one call per user action) so that the
//! frontend never has to orchestrate multi-step state, and so that every write
//! path goes through the same store locking.

use std::sync::Arc;

use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_autostart::ManagerExt;

use crate::AppState;
use crate::clipboard::{self, EVENT_HISTORY_CHANGED};
use crate::error::{AppError, Result};
use crate::media;
use crate::models::{
    ClipContent, ClipDetail, ClipFilter, ClipKind, ClipPage, Collection, Stats, Tag,
};
use crate::paste;
use crate::settings::Settings;
use crate::storage::StorageInfo;
use crate::window;

/// Page size used when the frontend does not ask for a specific window.
const DEFAULT_PAGE: usize = 200;

#[tauri::command]
pub fn list_clips(
    state: State<'_, AppState>,
    filter: Option<ClipFilter>,
    query: Option<String>,
    limit: Option<usize>,
    offset: Option<usize>,
) -> ClipPage {
    state.store.list(
        &filter.unwrap_or_default(),
        &query.unwrap_or_default(),
        limit.unwrap_or(DEFAULT_PAGE).clamp(1, 1_000),
        offset.unwrap_or(0),
    )
}

#[tauri::command]
pub fn get_clip(state: State<'_, AppState>, id: i64) -> Result<ClipDetail> {
    state.store.detail(id)
}

#[tauri::command]
pub fn get_stats(state: State<'_, AppState>) -> Result<Stats> {
    state.store.stats()
}

/// Puts a stored clip back on the clipboard, then optionally pastes it.
#[tauri::command]
pub fn copy_clip(
    app: AppHandle,
    state: State<'_, AppState>,
    id: i64,
    auto_paste: Option<bool>,
    hide_window: Option<bool>,
) -> Result<()> {
    let store = Arc::clone(&state.store);
    let content = content_for(&store, id)?;

    let hash = clipboard::write(&content)?;
    // Arm the latch *before* recording the copy: the watcher polls on a timer and
    // could otherwise observe our own write and reshuffle the list.
    state.watcher.suppress_hash(hash);
    store.record_copy(id)?;
    let _ = app.emit(EVENT_HISTORY_CHANGED, ());

    let settings = store.settings();
    // Copying from a hover button leaves the window up so the user can keep
    // browsing; only an activation (Enter, double click) dismisses it.
    let should_hide = hide_window.unwrap_or(true);
    let should_paste = should_hide && auto_paste.unwrap_or(settings.auto_paste);
    if should_hide {
        window::hide(&app);
    }

    if should_paste {
        match paste::target() {
            Some(target) => {
                let delay = u64::from(settings.paste_delay_ms);
                // Pasted from a worker thread: `SetForegroundWindow` needs our own
                // window to have finished hiding, and blocking the IPC thread here
                // would freeze the UI while it happens.
                std::thread::spawn(move || {
                    if let Err(e) = paste::restore_and_paste(target, delay) {
                        log::warn!("auto-paste failed: {e}");
                    }
                });
            }
            None => log::debug!("no paste target recorded; leaving the clip on the clipboard"),
        }
    }
    Ok(())
}

/// Copies the text recognised in an image instead of the image itself.
#[tauri::command]
pub fn copy_ocr_text(
    app: AppHandle,
    state: State<'_, AppState>,
    id: i64,
    hide_window: Option<bool>,
) -> Result<()> {
    let store = Arc::clone(&state.store);
    let text = store
        .ocr_text(id)?
        .filter(|text| !text.trim().is_empty())
        .ok_or_else(|| AppError::msg("no text has been recognised in this image"))?;

    let hash = clipboard::write(&ClipContent::Text(text))?;
    // Same latch as `copy_clip`: our own write must not be captured again.
    state.watcher.suppress_hash(hash);
    store.record_copy(id)?;
    notify_history_changed(&app);
    if hide_window.unwrap_or(true) {
        window::hide(&app);
    }
    Ok(())
}

/// Reconstructs the payload of a clip for writing back to the system clipboard.
fn content_for(store: &crate::store::Store, id: i64) -> Result<ClipContent> {
    let detail = store.detail(id)?;
    match detail.summary.kind {
        ClipKind::Text => {
            let text = detail
                .text
                .ok_or_else(|| AppError::msg("this text clip has no stored body"))?;
            Ok(ClipContent::Text(text))
        }
        ClipKind::Files => {
            if detail.file_paths.is_empty() {
                return Err(AppError::msg("this file clip has no stored paths"));
            }
            Ok(ClipContent::Files(detail.file_paths))
        }
        ClipKind::Image => {
            let name = detail
                .summary
                .image_file
                .ok_or_else(|| AppError::msg("this image clip has no stored file"))?;
            let (width, height, rgba) = media::load_rgba(store.media_dir(), &name)?;
            Ok(ClipContent::Image {
                width,
                height,
                rgba,
            })
        }
    }
}

/// Tells every window that the stored history changed.
///
/// Every mutating command must call this. Without it the list has no way to learn
/// that rows moved, so it only refreshes when the filter changes — which is
/// exactly the stale-after-delete and reverting-checkbox behaviour.
fn notify_history_changed(app: &AppHandle) {
    if let Err(e) = app.emit(EVENT_HISTORY_CHANGED, ()) {
        log::warn!("could not emit the history change event: {e}");
    }
}

#[tauri::command]
pub fn set_pinned(
    app: AppHandle,
    state: State<'_, AppState>,
    id: i64,
    pinned: bool,
) -> Result<()> {
    state.store.set_pinned(id, pinned)?;
    notify_history_changed(&app);
    Ok(())
}

#[tauri::command]
pub fn delete_clip(app: AppHandle, state: State<'_, AppState>, id: i64) -> Result<usize> {
    let removed = state.store.delete_many(&[id])?;
    notify_history_changed(&app);
    Ok(removed)
}

#[tauri::command]
pub fn delete_clips(app: AppHandle, state: State<'_, AppState>, ids: Vec<i64>) -> Result<usize> {
    let removed = state.store.delete_many(&ids)?;
    notify_history_changed(&app);
    Ok(removed)
}

#[tauri::command]
pub fn clear_history(
    app: AppHandle,
    state: State<'_, AppState>,
    keep_pinned: Option<bool>,
) -> Result<usize> {
    let removed = state.store.clear(keep_pinned.unwrap_or(true))?;
    notify_history_changed(&app);
    Ok(removed)
}

#[tauri::command]
pub fn prune_history(app: AppHandle, state: State<'_, AppState>) -> Result<usize> {
    let removed = state.store.prune()?;
    if removed > 0 {
        notify_history_changed(&app);
    }
    Ok(removed)
}

#[tauri::command]
pub fn list_collections(state: State<'_, AppState>) -> Result<Vec<Collection>> {
    state.store.collections()
}

#[tauri::command]
pub fn create_collection(
    app: AppHandle,
    state: State<'_, AppState>,
    name: String,
    color: Option<String>,
) -> Result<Collection> {
    let created = state.store.create_collection(&name, color)?;
    notify_history_changed(&app);
    Ok(created)
}

#[tauri::command]
pub fn rename_collection(
    app: AppHandle,
    state: State<'_, AppState>,
    id: i64,
    name: String,
) -> Result<()> {
    state.store.rename_collection(id, &name)?;
    notify_history_changed(&app);
    Ok(())
}

#[tauri::command]
pub fn delete_collection(app: AppHandle, state: State<'_, AppState>, id: i64) -> Result<()> {
    state.store.delete_collection(id)?;
    notify_history_changed(&app);
    Ok(())
}

/// Marks a collection as an archive, which hides its clips from the history views
/// and protects them from clear-history and retention.
#[tauri::command]
pub fn set_collection_archived(
    app: AppHandle,
    state: State<'_, AppState>,
    id: i64,
    archived: bool,
) -> Result<()> {
    state.store.set_collection_archived(id, archived)?;
    notify_history_changed(&app);
    Ok(())
}

#[tauri::command]
pub fn set_clip_collections(
    app: AppHandle,
    state: State<'_, AppState>,
    clip_id: i64,
    collection_ids: Vec<i64>,
) -> Result<Vec<i64>> {
    let applied = state.store.set_clip_collections(clip_id, &collection_ids)?;
    // The list carries `collectionIds`, so it has to be re-read for the checkbox
    // to stay ticked; without this it flips back as soon as React re-renders.
    notify_history_changed(&app);
    Ok(applied)
}

#[tauri::command]
pub fn list_tags(state: State<'_, AppState>) -> Result<Vec<Tag>> {
    state.store.tags()
}

#[tauri::command]
pub fn set_clip_tags(
    app: AppHandle,
    state: State<'_, AppState>,
    clip_id: i64,
    tags: Vec<String>,
) -> Result<Vec<String>> {
    let applied = state.store.set_clip_tags(clip_id, &tags)?;
    notify_history_changed(&app);
    Ok(applied)
}

#[tauri::command]
pub fn get_settings(state: State<'_, AppState>) -> Settings {
    state.store.settings()
}

#[tauri::command]
pub fn update_settings(
    app: AppHandle,
    state: State<'_, AppState>,
    patch: serde_json::Value,
) -> Result<Settings> {
    let before = state.store.settings();
    let after = state.store.update_settings(&patch)?;

    if after.hotkey != before.hotkey
        && let Err(e) = crate::hotkey::apply(&app, &after.hotkey)
    {
        // Roll back rather than leaving the user with a saved hotkey that does
        // not work and no way to reach the app from the keyboard.
        let restore = serde_json::json!({ "hotkey": before.hotkey });
        if let Err(revert_error) = state.store.update_settings(&restore) {
            log::error!("could not restore the previous hotkey: {revert_error}");
        }
        let _ = crate::hotkey::apply(&app, &before.hotkey);
        return Err(e);
    }

    if after.launch_at_login != before.launch_at_login {
        apply_autostart(&app, after.launch_at_login);
    }

    if after.show_in_taskbar != before.show_in_taskbar {
        crate::window::set_taskbar_visibility(&app, after.show_in_taskbar);
    }

    Ok(after)
}

fn apply_autostart(app: &AppHandle, enabled: bool) {
    let manager = app.autolaunch();
    let outcome = if enabled {
        manager.enable()
    } else {
        manager.disable()
    };
    if let Err(e) = outcome {
        log::warn!("could not update the launch-at-login setting: {e}");
    }
}

/// Validates a candidate hotkey without persisting it, so the settings UI can
/// report a conflict while the user is still typing.
#[tauri::command]
pub fn validate_hotkey(hotkey: String) -> Result<String> {
    hotkey
        .parse::<tauri_plugin_global_shortcut::Shortcut>()
        .map(|shortcut| shortcut.to_string())
        .map_err(|e| AppError::msg(format!("`{hotkey}` is not a valid shortcut: {e}")))
}

#[tauri::command]
pub fn hide_window(app: AppHandle) {
    window::hide(&app);
}

#[tauri::command]
pub fn toggle_window(app: AppHandle) {
    paste::capture_foreground(window::own_window_handle(&app));
    window::toggle(&app);
}

/// Opens the folder holding the database, key blob and media files.
#[tauri::command]
pub fn reveal_data_dir(state: State<'_, AppState>) -> Result<()> {
    let dir = state.store.data_dir().to_string_lossy().to_string();
    tauri_plugin_opener::open_path(dir, None::<&str>)
        .map_err(|e| AppError::msg(format!("could not open the data folder: {e}")))
}

/// Reports where the clips live and where the default location is.
#[tauri::command]
pub fn storage_info(app: AppHandle, state: State<'_, AppState>) -> Result<StorageInfo> {
    let default_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| AppError::msg(format!("could not resolve the default data folder: {e}")))?;
    Ok(crate::storage::info(
        &default_dir,
        state.store.data_dir(),
    ))
}

/// Copies the history to `path`, points the app at it and restarts.
///
/// Restarting rather than hot-swapping the store guarantees no thread keeps a
/// handle on the old files, and it re-runs the startup retention pass against the
/// new location. The command therefore never returns.
#[tauri::command]
pub fn set_storage_dir(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> Result<()> {
    let target = std::path::PathBuf::from(path.trim());
    if !target.is_absolute() {
        return Err(AppError::msg("choose an absolute folder path"));
    }
    let default_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| AppError::msg(format!("could not resolve the default data folder: {e}")))?;

    // Stop capturing first: anything copied from here on belongs after the
    // snapshot and would be lost by the restart anyway.
    state.watcher.stop();
    crate::storage::relocate(&state.store, &default_dir, &target)?;
    log::info!("moving the data folder to {}; restarting", target.display());
    app.restart();
}

/// Reports whether the store already holds anything, so the app can decide to
/// introduce itself on first launch.
#[tauri::command]
pub fn history_is_empty(state: State<'_, AppState>) -> bool {
    state.store.is_empty()
}

#[allow(dead_code)]
fn state_of(app: &AppHandle) -> Arc<crate::store::Store> {
    Arc::clone(&app.state::<AppState>().store)
}
