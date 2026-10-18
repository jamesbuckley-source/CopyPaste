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
    let should_paste = auto_paste.unwrap_or(settings.auto_paste);
    window::hide(&app);

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

#[tauri::command]
pub fn set_pinned(state: State<'_, AppState>, id: i64, pinned: bool) -> Result<()> {
    state.store.set_pinned(id, pinned)
}

#[tauri::command]
pub fn delete_clip(state: State<'_, AppState>, id: i64) -> Result<usize> {
    state.store.delete_many(&[id])
}

#[tauri::command]
pub fn delete_clips(state: State<'_, AppState>, ids: Vec<i64>) -> Result<usize> {
    state.store.delete_many(&ids)
}

#[tauri::command]
pub fn clear_history(state: State<'_, AppState>, keep_pinned: Option<bool>) -> Result<usize> {
    state.store.clear(keep_pinned.unwrap_or(true))
}

#[tauri::command]
pub fn prune_history(state: State<'_, AppState>) -> Result<usize> {
    state.store.prune()
}

#[tauri::command]
pub fn list_collections(state: State<'_, AppState>) -> Result<Vec<Collection>> {
    state.store.collections()
}

#[tauri::command]
pub fn create_collection(
    state: State<'_, AppState>,
    name: String,
    color: Option<String>,
) -> Result<Collection> {
    state.store.create_collection(&name, color)
}

#[tauri::command]
pub fn rename_collection(state: State<'_, AppState>, id: i64, name: String) -> Result<()> {
    state.store.rename_collection(id, &name)
}

#[tauri::command]
pub fn delete_collection(state: State<'_, AppState>, id: i64) -> Result<()> {
    state.store.delete_collection(id)
}

#[tauri::command]
pub fn set_clip_collections(
    state: State<'_, AppState>,
    clip_id: i64,
    collection_ids: Vec<i64>,
) -> Result<Vec<i64>> {
    state.store.set_clip_collections(clip_id, &collection_ids)
}

#[tauri::command]
pub fn list_tags(state: State<'_, AppState>) -> Result<Vec<Tag>> {
    state.store.tags()
}

#[tauri::command]
pub fn set_clip_tags(
    state: State<'_, AppState>,
    clip_id: i64,
    tags: Vec<String>,
) -> Result<Vec<String>> {
    state.store.set_clip_tags(clip_id, &tags)
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
