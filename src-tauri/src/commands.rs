//! The IPC surface exposed to the webview.
//!
//! Commands are intentionally coarse (one call per user action) so that the
//! frontend never has to orchestrate multi-step state, and so that every write
//! path goes through the same store locking.

use std::path::Path;
use std::sync::Arc;

use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_autostart::ManagerExt;

use crate::AppState;
use crate::animated;
use crate::clipboard::{self, EVENT_HISTORY_CHANGED};
use crate::error::{AppError, Result};
use crate::giphy::{self, GifResult};
use crate::hashing;
use crate::media;
use crate::models::{
    ClipContent, ClipDetail, ClipFilter, ClipKind, ClipPage, Collection, NewClip, RichText, Stats,
    Tag,
};
use crate::paste;
use crate::picker;
use crate::search;
use crate::settings::{GifFavorite, Settings};
use crate::sound;
use crate::storage::StorageInfo;
use crate::window;

use crate::clipboard::PauseState;

/// Page size used when the frontend does not ask for a specific window.
const DEFAULT_PAGE: usize = 200;

/// Emitted when a screen sample finishes. `error` is set when the pick was
/// cancelled or failed, so the UI can explain what happened.
pub const EVENT_COLOR_PICKED: &str = "ui:color-picked";

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

/// Minimises the window, samples the next clicked pixel and copies its hex value.
#[tauri::command]
pub fn pick_screen_color(app: AppHandle, state: State<'_, AppState>) -> Result<()> {
    let store = Arc::clone(&state.store);
    let watcher = Arc::clone(&state.watcher);
    let handle = app;
    // The pick waits for a click, so it cannot run on the IPC thread.
    std::thread::spawn(move || {
        let result = match picker::pick(&handle) {
            Ok(hex) => {
                let content = ClipContent::Text(hex.clone());
                match clipboard::write(&content) {
                    Ok(hash) => {
                        // Our own write must not be captured a second time; the clip
                        // is inserted explicitly below so it is available immediately.
                        watcher.suppress_hash(hash);
                        let new = NewClip {
                            content,
                            content_hash: crate::hashing::of_str(&hex),
                            source_app: Some("Color picker".into()),
                            search_text: search::normalize(&hex),
                            preview: hex.clone(),
                            html: None,
                            rtf: None,
                        };
                        let id = match store.insert_clip(new) {
                            Ok(crate::store::InsertOutcome::Inserted(id))
                            | Ok(crate::store::InsertOutcome::Duplicate(id)) => Some(id),
                            Err(e) => {
                                log::warn!("could not record the sampled colour: {e}");
                                None
                            }
                        };
                        notify_history_changed(&handle);
                        serde_json::json!({ "hex": hex, "id": id })
                    }
                    Err(e) => {
                        log::warn!("could not copy the sampled colour: {e}");
                        serde_json::json!({ "hex": hex, "id": null, "error": e.to_string() })
                    }
                }
            }
            Err(e) => serde_json::json!({ "hex": null, "id": null, "error": e.to_string() }),
        };
        let _ = handle.emit(EVENT_COLOR_PICKED, result);
    });
    Ok(())
}

/// Replaces the body of a text clip after the user edits it.
#[tauri::command]
pub fn set_clip_text(
    app: AppHandle,
    state: State<'_, AppState>,
    id: i64,
    text: String,
) -> Result<()> {
    state.store.set_text(id, &text)?;
    notify_history_changed(&app);
    Ok(())
}

/// Attaches or clears the user's note on a clip.
#[tauri::command]
pub fn set_clip_note(
    app: AppHandle,
    state: State<'_, AppState>,
    id: i64,
    note: Option<String>,
) -> Result<Option<String>> {
    let applied = state.store.set_note(id, note.as_deref())?;
    notify_history_changed(&app);
    Ok(applied)
}

/// Searches GIPHY for the GIF utility; an empty query returns trending GIFs.
#[tauri::command]
pub async fn search_giphy(
    state: State<'_, AppState>,
    query: Option<String>,
    limit: Option<usize>,
) -> Result<Vec<GifResult>> {
    let key = state.store.settings().giphy_api_key;
    giphy::search(&key, &query.unwrap_or_default(), limit.unwrap_or(24)).await
}

/// Plays the feedback sound that matches a copy outcome.
fn copy_outcome<T>(state: &AppState, result: &Result<T>) {
    let outcome = if result.is_ok() {
        sound::Outcome::Copied
    } else {
        sound::Outcome::Failed
    };
    sound::play(&state.store.settings(), outcome);
}

/// Downloads a GIPHY GIF and puts the file (plus its URL) on the clipboard.
#[tauri::command]
pub async fn copy_gif(
    app: AppHandle,
    state: State<'_, AppState>,
    url: String,
    hide_window: Option<bool>,
) -> Result<()> {
    let result = copy_gif_inner(&app, &state, &url, hide_window).await;
    copy_outcome(&state, &result);
    result
}

async fn copy_gif_inner(
    app: &AppHandle,
    state: &AppState,
    url: &str,
    hide_window: Option<bool>,
) -> Result<()> {
    let bytes = giphy::download(url).await?;
    let path = giphy::cache_file(state.store.media_dir(), url, &bytes)?;
    let hash = clipboard::write_file_and_text(&path, url)?;
    let sequence = clipboard::sequence_number();
    // The watcher would otherwise record the file we just published as a clip.
    state.watcher.suppress_hash(hash);

    let should_hide = hide_window.unwrap_or(false);
    if should_hide {
        window::hide(app);
    }
    schedule_auto_clear(app, sequence, &state.store.settings(), should_hide);
    Ok(())
}

/// Imports a GIF chosen from disk into the utility's shelf.
///
/// The file is copied into the media folder under a content-addressed name so
/// the shelf entry survives the original being moved or deleted.
#[tauri::command]
pub fn import_custom_gif(state: State<'_, AppState>, path: String) -> Result<GifFavorite> {
    let source = Path::new(&path);
    let metadata = std::fs::metadata(source)
        .map_err(|e| AppError::msg(format!("could not read that GIF: {e}")))?;
    if metadata.len() > animated::MAX_GIF_BYTES as u64 {
        return Err(AppError::msg("that GIF is too large"));
    }
    let bytes = std::fs::read(source)
        .map_err(|e| AppError::msg(format!("could not read that GIF: {e}")))?;
    if !animated::is_gif(&bytes) {
        return Err(AppError::msg("that file is not a GIF"));
    }

    let file = format!("custom-{}.gif", hashing::of_gif(&bytes));
    let target = state.store.media_dir().join(&file);
    if !target.is_file() {
        std::fs::write(&target, &bytes)?;
    }
    let title = source
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.trim().is_empty())
        .unwrap_or("Custom GIF")
        .to_string();
    Ok(GifFavorite {
        id: format!("local:{file}"),
        title,
        url: String::new(),
        preview_url: file,
        clip_id: None,
    })
}

/// Puts an imported GIF file on the clipboard, preserving every frame.
#[tauri::command]
pub fn copy_custom_gif(
    app: AppHandle,
    state: State<'_, AppState>,
    file: String,
    hide_window: Option<bool>,
) -> Result<()> {
    let result = copy_custom_gif_inner(&app, &state, &file, hide_window);
    copy_outcome(&state, &result);
    result
}

fn copy_custom_gif_inner(
    app: &AppHandle,
    state: &AppState,
    file: &str,
    hide_window: Option<bool>,
) -> Result<()> {
    // `media::resolve` is the allowlist, so a manipulated shelf entry cannot
    // point at an arbitrary path.
    let path = media::resolve(state.store.media_dir(), file)
        .ok_or_else(|| AppError::NotFound(format!("GIF file `{file}`")))?;
    let hash = clipboard::write_files(&[path.to_string_lossy().into_owned()])?;
    let sequence = clipboard::sequence_number();
    state.watcher.suppress_hash(hash);
    let should_hide = hide_window.unwrap_or(true);
    if should_hide {
        window::hide(app);
    }
    schedule_auto_clear(app, sequence, &state.store.settings(), should_hide);
    Ok(())
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
    let result = copy_clip_inner(&app, &state, id, auto_paste, hide_window);
    copy_outcome(&state, &result);
    result
}

fn copy_clip_inner(
    app: &AppHandle,
    state: &AppState,
    id: i64,
    auto_paste: Option<bool>,
    hide_window: Option<bool>,
) -> Result<()> {
    let store = Arc::clone(&state.store);
    let settings = store.settings();
    let content = content_for(&store, id)?;
    // Styled copies are pasted back as they were copied; the plain text is always
    // on the clipboard too, so a rich-unaware target still gets something.
    let rich = if settings.paste_rich_text && content.kind() == ClipKind::Text {
        store.rich_text(id).unwrap_or_default()
    } else {
        Default::default()
    };

    // An image clip with a recovered animation is published as a file so GIF-aware
    // targets keep every frame; everything else keeps the normal clipboard shape.
    let animated_path = if content.kind() == ClipKind::Image {
        store.animated_gif_path(id)
    } else {
        None
    };
    let hash = match animated_path {
        Some(path) => clipboard::write_files(&[path.to_string_lossy().into_owned()])?,
        None => {
            clipboard::write_flavoured(&content, rich.html.as_deref(), rich.rtf.as_deref())?
        }
    };
    let sequence = clipboard::sequence_number();
    // Arm the latch *before* recording the copy: the watcher polls on a timer and
    // could otherwise observe our own write and reshuffle the list.
    state.watcher.suppress_hash(hash);
    store.record_copy(id)?;
    let _ = app.emit(EVENT_HISTORY_CHANGED, ());

    // Copying from a hover button leaves the window up so the user can keep
    // browsing; only an activation (Enter, double click) dismisses it.
    let should_hide = hide_window.unwrap_or(true);
    let should_paste = should_hide && auto_paste.unwrap_or(settings.auto_paste);
    if should_hide {
        window::hide(app);
    }
    schedule_auto_clear(app, sequence, &settings, should_hide);

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

/// Empties the clipboard some time after a copy, when the user asked for it.
///
/// The sequence number captured at write time guards the timer: if anything else
/// has touched the clipboard since, the payload on it is no longer ours and
/// clearing it would destroy something the user copied.
fn schedule_auto_clear(app: &AppHandle, sequence: u32, settings: &Settings, hiding: bool) {
    if !settings.auto_clear_clipboard {
        return;
    }
    // A delay of zero means "as soon as the overlay goes away". Auto-paste still
    // has to land first, so the floor keeps the shortcut from racing the paste.
    let delay = if settings.auto_clear_delay_secs == 0 {
        if hiding {
            std::time::Duration::from_millis(1_500)
        } else {
            std::time::Duration::from_millis(300)
        }
    } else {
        std::time::Duration::from_secs(u64::from(settings.auto_clear_delay_secs))
    };

    let _ = app.emit(
        crate::clipboard::EVENT_CLIPBOARD_CLEAR_SCHEDULED,
        delay.as_millis() as u64,
    );
    std::thread::spawn(move || {
        std::thread::sleep(delay);
        if clipboard::sequence_number() != sequence {
            log::debug!("clipboard changed since the copy; leaving it alone");
            return;
        }
        match clipboard::clear() {
            Ok(()) => log::info!("cleared the clipboard after {}ms", delay.as_millis()),
            Err(e) => log::warn!("could not clear the clipboard: {e}"),
        }
    });
}

/// Extracts up to five dominant colours from an image clip.
#[tauri::command]
pub fn extract_color_scheme(state: State<'_, AppState>, id: i64) -> Result<Vec<String>> {
    let detail = state.store.detail(id)?;
    if detail.summary.kind != ClipKind::Image {
        return Err(AppError::msg(
            "color schemes are only available for image clips",
        ));
    }
    let name = detail
        .summary
        .image_file
        .ok_or_else(|| AppError::msg("this image has no stored file"))?;
    let (width, height, rgba) = media::load_rgba(state.store.media_dir(), &name)?;
    Ok(crate::palette::extract(&rgba, width, height))
}

/// Creates history entries from files dropped onto the window.
///
/// A single image becomes an image clip, a single small text file becomes a text
/// clip, and anything else becomes one file-list clip of every dropped path — the
/// same shape Explorer's copy produces.
#[tauri::command]
pub fn import_dropped_files(
    app: AppHandle,
    state: State<'_, AppState>,
    paths: Vec<String>,
) -> Result<usize> {
    let paths: Vec<String> = paths
        .into_iter()
        .map(|path| path.trim().to_string())
        .filter(|path| !path.is_empty() && Path::new(path).exists())
        .collect();
    if paths.is_empty() {
        return Err(AppError::msg("nothing usable was dropped"));
    }

    let mut imported = 0usize;
    let single_image = (paths.len() == 1)
        .then(|| read_image_file(Path::new(&paths[0])))
        .flatten();
    let single_text = (paths.len() == 1)
        .then(|| read_text_file(Path::new(&paths[0])))
        .flatten();

    let new = if let Some(content) = single_image {
        build_new_clip(content, Some("Dropped file".into()))
    } else if let Some(text) = single_text {
        build_new_clip(ClipContent::Text(text), Some("Dropped file".into()))
    } else {
        build_new_clip(ClipContent::Files(paths.clone()), Some("Dropped files".into()))
    };

    let settings = state.store.settings();
    if settings.max_items > 0 && state.store.total() >= settings.max_items as usize {
        let _ = state.store.prune();
    }
    match state.store.insert_clip(new)? {
        crate::store::InsertOutcome::Inserted(_) => imported += 1,
        crate::store::InsertOutcome::Duplicate(_) => {}
    }
    notify_history_changed(&app);
    Ok(imported)
}

/// Builds a `NewClip` with the same preview, search mirror and hash rules capture
/// uses, so a dropped file behaves like a copied one.
fn build_new_clip(content: ClipContent, source_app: Option<String>) -> NewClip {
    let hash = crate::hashing::of_content(&content);
    let (preview, search_text) = match &content {
        ClipContent::Text(text) => {
            let collapsed: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
            (
                collapsed.chars().take(320).collect::<String>(),
                search::normalize(text),
            )
        }
        ClipContent::Files(paths) => {
            let names: Vec<String> = paths
                .iter()
                .map(|path| {
                    path.rsplit(['\\', '/'])
                        .next()
                        .unwrap_or(path)
                        .to_string()
                })
                .collect();
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
    };
    NewClip {
        content,
        content_hash: hash,
        source_app,
        search_text,
        preview,
        html: None,
        rtf: None,
    }
}

/// Decodes a dropped image file, or `None` when it is not an image.
fn read_image_file(path: &Path) -> Option<ClipContent> {
    const MAX_IMAGE_BYTES: u64 = 512 * 1024 * 1024;
    let metadata = std::fs::metadata(path).ok()?;
    if metadata.len() > MAX_IMAGE_BYTES {
        return None;
    }
    let decoded = image::open(path).ok()?.to_rgba8();
    let (width, height) = decoded.dimensions();
    Some(ClipContent::Image {
        width,
        height,
        rgba: decoded.into_raw(),
    })
}

/// Reads a small UTF-8 text file, or `None` when it is binary or too large.
fn read_text_file(path: &Path) -> Option<String> {
    const MAX_TEXT_BYTES: u64 = 1024 * 1024;
    let metadata = std::fs::metadata(path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_TEXT_BYTES {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    if bytes.iter().any(|byte| *byte == 0) {
        return None;
    }
    let text = String::from_utf8(bytes).ok()?;
    (!text.trim().is_empty()).then_some(text)
}

/// Opens the settings page in its own decorated window.
#[tauri::command]
pub fn open_settings_window(app: AppHandle) {
    log::info!("opening the settings window");
    window::show_settings(&app);
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
    let sequence = clipboard::sequence_number();
    // Same latch as `copy_clip`: our own write must not be captured again.
    state.watcher.suppress_hash(hash);
    store.record_copy(id)?;
    notify_history_changed(&app);
    let should_hide = hide_window.unwrap_or(true);
    if should_hide {
        window::hide(&app);
    }
    schedule_auto_clear(&app, sequence, &store.settings(), should_hide);
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

/// Files a clip into exactly one archive, or `None` to bring it back.
#[tauri::command]
pub fn set_clip_archive(
    app: AppHandle,
    state: State<'_, AppState>,
    clip_id: i64,
    archive_id: Option<i64>,
) -> Result<Vec<i64>> {
    let applied = state.store.set_clip_archive(clip_id, archive_id)?;
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

    if (after.hotkey != before.hotkey || after.pause_hotkey != before.pause_hotkey)
        && let Err(e) = crate::hotkey::apply(&app, &after.hotkey, &after.pause_hotkey)
    {
        // Roll back rather than leaving the user with a saved hotkey that does
        // not work and no way to reach the app from the keyboard.
        let restore = serde_json::json!({
            "hotkey": before.hotkey,
            "pauseHotkey": before.pause_hotkey,
        });
        if let Err(revert_error) = state.store.update_settings(&restore) {
            log::error!("could not restore the previous hotkey: {revert_error}");
        }
        let _ = crate::hotkey::apply(&app, &before.hotkey, &before.pause_hotkey);
        return Err(e);
    }

    if after.launch_at_login != before.launch_at_login {
        apply_autostart(&app, after.launch_at_login);
    }

    if after.show_in_taskbar != before.show_in_taskbar {
        crate::window::set_taskbar_visibility(&app, after.show_in_taskbar);
    }

    if after.always_on_top != before.always_on_top {
        crate::window::set_always_on_top(&app, after.always_on_top);
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

/// Opens Windows Explorer with the clip's file selected (or its folder).
#[tauri::command]
pub fn reveal_clip(state: State<'_, AppState>, id: i64) -> Result<()> {
    let path = state.store.reveal_path(id)?;
    tauri_plugin_opener::reveal_item_in_dir(&path)
        .map_err(|e| AppError::msg(format!("could not show {}: {e}", path.display())))
}

/// Opens the dedicated folder that belongs to an archive collection.
#[tauri::command]
pub fn reveal_archive(state: State<'_, AppState>, id: i64) -> Result<()> {
    let name = state.store.collection_name(id)?;
    if !state.store.is_archived(id) {
        return Err(AppError::msg("only archive collections have a folder"));
    }
    let dir = crate::storage::ensure_archive_dir(state.store.data_dir(), &name)?;
    tauri_plugin_opener::open_path(dir, None::<&str>)
        .map_err(|e| AppError::msg(format!("could not open the archive folder: {e}")))
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

// ---------------------------------------------------------------------------
// Private mode
// ---------------------------------------------------------------------------

/// Reports whether capture is currently paused, so the UI can show an indicator.
#[tauri::command]
pub fn capture_state(app: AppHandle) -> PauseState {
    pause_state(&app)
}

/// Pauses clipboard capture for `minutes`, or until the app restarts.
#[tauri::command]
pub fn pause_capture(
    app: AppHandle,
    minutes: Option<u32>,
    until_restart: Option<bool>,
) -> PauseState {
    let watcher = Arc::clone(&app.state::<AppState>().watcher);
    if until_restart.unwrap_or(false) {
        watcher.pause_until_restart();
        log::info!("capture paused until restart");
    } else {
        let minutes = u64::from(minutes.unwrap_or(15).clamp(1, 24 * 60));
        watcher.pause_for(std::time::Duration::from_secs(minutes * 60));
        log::info!("capture paused for {minutes} minute(s)");
    }
    let state = watcher.pause_state();
    emit_pause_state(&app, &state);
    state
}

#[tauri::command]
pub fn resume_capture(app: AppHandle) -> PauseState {
    let watcher = Arc::clone(&app.state::<AppState>().watcher);
    watcher.resume();
    log::info!("capture resumed");
    let state = watcher.pause_state();
    emit_pause_state(&app, &state);
    state
}

pub fn pause_state(app: &AppHandle) -> PauseState {
    app.state::<AppState>().watcher.pause_state()
}

fn emit_pause_state(app: &AppHandle, state: &PauseState) {
    if let Err(e) = app.emit(crate::clipboard::EVENT_CAPTURE_PAUSED, state.clone()) {
        log::warn!("could not emit the pause state: {e}");
    }
}

/// Flips the private-mode pause; used by the tray menu and the pause shortcut.
pub fn toggle_pause(app: &AppHandle) -> PauseState {
    if pause_state(app).paused {
        resume_capture(app.clone())
    } else {
        pause_capture(app.clone(), None, None)
    }
}

// ---------------------------------------------------------------------------
// Tags
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn rename_tag(app: AppHandle, state: State<'_, AppState>, id: i64, name: String) -> Result<()> {
    state.store.rename_tag(id, &name)?;
    notify_history_changed(&app);
    Ok(())
}

#[tauri::command]
pub fn delete_tag(app: AppHandle, state: State<'_, AppState>, id: i64) -> Result<()> {
    state.store.delete_tag(id)?;
    notify_history_changed(&app);
    Ok(())
}

#[tauri::command]
pub fn set_tag_color(
    app: AppHandle,
    state: State<'_, AppState>,
    id: i64,
    color: Option<String>,
) -> Result<()> {
    state.store.set_tag_color(id, color.as_deref())?;
    notify_history_changed(&app);
    Ok(())
}

// ---------------------------------------------------------------------------
// Bulk actions
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn set_pinned_many(
    app: AppHandle,
    state: State<'_, AppState>,
    ids: Vec<i64>,
    pinned: bool,
) -> Result<usize> {
    let changed = state.store.set_pinned_many(&ids, pinned)?;
    notify_history_changed(&app);
    Ok(changed)
}

#[tauri::command]
pub fn add_tags_many(
    app: AppHandle,
    state: State<'_, AppState>,
    ids: Vec<i64>,
    tags: Vec<String>,
) -> Result<usize> {
    let changed = state.store.add_tags_many(&ids, &tags)?;
    notify_history_changed(&app);
    Ok(changed)
}

#[tauri::command]
pub fn add_to_collection(
    app: AppHandle,
    state: State<'_, AppState>,
    ids: Vec<i64>,
    collection_id: i64,
) -> Result<usize> {
    let changed = state.store.add_to_collection_many(&ids, collection_id)?;
    notify_history_changed(&app);
    Ok(changed)
}

// ---------------------------------------------------------------------------
// Saving and opening payloads
// ---------------------------------------------------------------------------

/// Copies a clip's payload into `dir`, returning the files that were written.
#[tauri::command]
pub fn save_clip(
    app: AppHandle,
    state: State<'_, AppState>,
    id: i64,
    dir: String,
) -> Result<Vec<String>> {
    let target = std::path::PathBuf::from(dir.trim());
    if !target.is_absolute() {
        return Err(AppError::msg("choose an absolute folder path"));
    }
    let written = state.store.save_clip(id, &target)?;
    log::info!("saved clip {id} to {}", target.display());
    notify_history_changed(&app);
    Ok(written)
}

/// Exports several clips into one folder, used by the bulk bar.
#[tauri::command]
pub fn save_clips(
    app: AppHandle,
    state: State<'_, AppState>,
    ids: Vec<i64>,
    dir: String,
) -> Result<Vec<String>> {
    let target = std::path::PathBuf::from(dir.trim());
    if !target.is_absolute() {
        return Err(AppError::msg("choose an absolute folder path"));
    }
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let written = state.store.save_clips(&ids, &target)?;
    log::info!("exported {} clip(s) to {}", ids.len(), target.display());
    notify_history_changed(&app);
    Ok(written)
}

/// Puts arbitrary text on the clipboard, used by the emoji browser.
#[tauri::command]
pub fn copy_text(app: AppHandle, state: State<'_, AppState>, text: String) -> Result<()> {
    let result = copy_text_inner(&app, &state, &text);
    copy_outcome(&state, &result);
    result
}

/// Copies a colour scheme swatch and records it as a colour clip.
#[tauri::command]
pub fn copy_color(app: AppHandle, state: State<'_, AppState>, hex: String) -> Result<i64> {
    let normalized = crate::color::parse(hex.trim())
        .ok_or_else(|| AppError::msg("that is not a color"))?;
    let content = ClipContent::Text(normalized.clone());
    let hash = clipboard::write(&content)?;
    // The explicit insert below is the only history entry this write should make.
    state.watcher.suppress_hash(hash);
    let new = NewClip {
        content,
        content_hash: hashing::of_str(&normalized),
        source_app: Some("Color scheme".into()),
        search_text: search::normalize(&normalized),
        preview: normalized.clone(),
        html: None,
        rtf: None,
    };
    let id = match state.store.insert_clip(new)? {
        crate::store::InsertOutcome::Inserted(id)
        | crate::store::InsertOutcome::Duplicate(id) => id,
    };
    notify_history_changed(&app);
    Ok(id)
}

fn copy_text_inner(app: &AppHandle, state: &AppState, text: &str) -> Result<()> {
    if text.is_empty() {
        return Err(AppError::msg("there is nothing to copy"));
    }
    let hash = clipboard::write(&ClipContent::Text(text.to_string()))?;
    // The emoji is ours, so the watcher must not record it as a new clip.
    state.watcher.suppress_hash(hash);
    let _ = app.emit(EVENT_HISTORY_CHANGED, ());
    Ok(())
}

/// Opens a link from a clip in the default browser.
#[tauri::command]
pub fn open_url(url: String) -> Result<()> {
    let trimmed = url.trim();
    if !(trimmed.starts_with("http://") || trimmed.starts_with("https://")) {
        return Err(AppError::msg("only http and https links can be opened"));
    }
    tauri_plugin_opener::open_url(trimmed, None::<&str>)
        .map_err(|e| AppError::msg(format!("could not open the link: {e}")))
}

/// Opens the default mail client with an email clip as the recipient.
#[tauri::command]
pub fn open_email(email: String) -> Result<()> {
    let trimmed = email.trim();
    if !is_email_address(trimmed) {
        return Err(AppError::msg("that does not look like an email address"));
    }
    tauri_plugin_opener::open_url(format!("mailto:{trimmed}"), None::<&str>)
        .map_err(|e| AppError::msg(format!("could not open the mail client: {e}")))
}

/// A deliberately small check: one local part, one dotted domain, no spaces.
fn is_email_address(value: &str) -> bool {
    !value.is_empty()
        && !value.chars().any(char::is_whitespace)
        && value.split_once('@').is_some_and(|(local, domain)| {
            !local.is_empty()
                && !domain.contains('@')
                && domain.contains('.')
                && !domain.starts_with('.')
        })
}

/// Reveals any absolute path in Explorer, used by the content-aware actions.
#[tauri::command]
pub fn reveal_path(path: String) -> Result<()> {
    let target = std::path::PathBuf::from(path.trim());
    if !target.is_absolute() || !target.exists() {
        return Err(AppError::msg("that path no longer exists"));
    }
    tauri_plugin_opener::reveal_item_in_dir(&target)
        .map_err(|e| AppError::msg(format!("could not show {}: {e}", target.display())))
}

/// Opens a path with its default application (a folder, or the file itself).
#[tauri::command]
pub fn open_path(path: String) -> Result<()> {
    let target = std::path::PathBuf::from(path.trim());
    if !target.exists() {
        return Err(AppError::msg("that path no longer exists"));
    }
    tauri_plugin_opener::open_path(target, None::<&str>)
        .map_err(|e| AppError::msg(format!("could not open that path: {e}")))
}

/// Rich flavours of a text clip, fetched on demand so lists stay small.
#[tauri::command]
pub fn get_rich_text(state: State<'_, AppState>, id: i64) -> Result<RichText> {
    state.store.rich_text(id)
}

#[cfg(test)]
mod tests {
    use super::is_email_address;

    #[test]
    fn email_addresses_are_checked_loosely() {
        assert!(is_email_address("hello@example.com"));
        assert!(is_email_address("first.last+tag@mail.example.co.uk"));
        assert!(!is_email_address(""));
        assert!(!is_email_address("hello"));
        assert!(!is_email_address("hello@example"));
        assert!(!is_email_address("hello @example.com"));
        assert!(!is_email_address("hello@example@com"));
        assert!(!is_email_address("@example.com"));
        assert!(!is_email_address("hello@.com"));
    }
}
