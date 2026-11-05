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
use crate::capture;
use crate::clipboard::{self, EVENT_HISTORY_CHANGED};
use crate::error::{AppError, Result};
use crate::giphy::{self, GifResult};
use crate::hashing;
use crate::markdown::MarkdownExport;
use crate::media;
use crate::models::{
    ClipContent, ClipDetail, ClipFilter, ClipKind, ClipPage, Collection, NewClip, RichText,
    SplitOutcome, StackEntry, Stats, Tag,
};
use crate::paste;
use crate::picker;
use crate::preview;
use crate::recorder;
use crate::search;
use crate::settings::{GifFavorite, Settings};
use crate::sound;
use crate::storage::StorageInfo;
use crate::store::Store;
use crate::window;

use crate::clipboard::PauseState;

/// Emitted when the stored settings change, so a second window can follow.
pub const EVENT_SETTINGS_CHANGED: &str = "settings:changed";

/// Emitted whenever the recording starts, pauses, resumes or ends, so the toolbar
/// and the floating controls agree about what is happening.
pub const EVENT_RECORD_STATE: &str = "record:state";

/// Emitted once a stopped recording has been saved: `{"id": 12}`, or an `error`.
pub const EVENT_RECORD_FINISHED: &str = "record:finished";

/// Page size used when the frontend does not ask for a specific window.
const DEFAULT_PAGE: usize = 200;

/// Emitted when a screen sample finishes. `error` is set when the pick was
/// cancelled or failed, so the UI can explain what happened.
pub const EVENT_COLOR_PICKED: &str = "ui:color-picked";

/// Emitted when a screenshot finishes. `error` is set when the capture failed,
/// and `id` is the stored clip, or null when the user cancelled.
pub const EVENT_SCREENSHOT_CAPTURED: &str = "ui:screenshot-captured";

/// Emitted whenever the paste stack changes, so the badge, the sidebar row and the
/// panel agree about what is queued.
pub const EVENT_STACK_CHANGED: &str = "stack:changed";

/// Emitted when a paste was scheduled but never landed, with the reason. A failed
/// paste is otherwise invisible: the clipboard write succeeded, so nothing else
/// in the interface can tell.
pub const EVENT_PASTE_FAILED: &str = "paste:failed";

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
            Ok(Some(hex)) => {
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
            Ok(None) => {
                // Cancelled or timed out: the UI stays quiet instead of showing an
                // error for something the user chose to abandon.
                serde_json::json!({ "hex": null, "id": null })
            }
            Err(e) => serde_json::json!({ "hex": null, "id": null, "error": e.to_string() }),
        };
        let _ = handle.emit(EVENT_COLOR_PICKED, result);
    });
    Ok(())
}

/// Opens the onscreen ruler, or unlocks/closes the one already showing.
///
/// Like a capture, the overlay waits for the user and cannot live on the IPC
/// thread, so the command only starts it.
#[tauri::command]
pub fn open_ruler(app: AppHandle, state: State<'_, AppState>) -> Result<()> {
    crate::ruler::open(app, state.store.settings())
}

/// Puts one piece of text on the clipboard and into history, under a source app
/// name of its own.
///
/// Shared by tools that produce a value rather than a clipboard event, such as the
/// ruler's Copy button: the write is suppressed so the watcher does not record it
/// twice, and the inserted clip is available immediately.
pub(crate) fn record_text_clip(app: &AppHandle, text: &str, source: &str) -> Result<Option<i64>> {
    let state = app.state::<AppState>();
    let content = ClipContent::Text(text.to_string());
    let hash = clipboard::write(&content)?;
    state.watcher.suppress_hash(hash);
    let new = NewClip {
        content,
        content_hash: hashing::of_str(text),
        source_app: Some(source.to_string()),
        search_text: search::normalize(text),
        preview: text.to_string(),
        html: None,
        rtf: None,
    };
    let id = match state.store.insert_clip(new)? {
        crate::store::InsertOutcome::Inserted(id) | crate::store::InsertOutcome::Duplicate(id) => {
            Some(id)
        }
    };
    notify_history_changed(app);
    sound::play(&state.store.settings(), sound::Outcome::Copied);
    Ok(id)
}

/// Minimises the window, lets the user drag a rectangle and stores it as an image
/// clip, exactly as if that area had been copied.
#[tauri::command]
pub fn capture_screen_region(app: AppHandle, state: State<'_, AppState>) -> Result<()> {
    start_capture(app, state, capture::capture_region)
}

/// Minimises the window, highlights the window under the cursor and stores the
/// clicked one as an image clip.
#[tauri::command]
pub fn capture_screen_window(app: AppHandle, state: State<'_, AppState>) -> Result<()> {
    start_capture(app, state, capture::capture_window)
}

/// Hides the window and stores every monitor as one image clip.
#[tauri::command]
pub fn capture_whole_screen(app: AppHandle, state: State<'_, AppState>) -> Result<()> {
    start_capture(app, state, capture::capture_screen)
}

/// Picks a window, scrolls it a page at a time and stores the stitched result.
#[tauri::command]
pub fn capture_scrolling_window(app: AppHandle, state: State<'_, AppState>) -> Result<()> {
    start_capture(app, state, capture::capture_scrolling)
}

/// Starts the capture a global shortcut fired.
///
/// It calls the same commands the toolbar buttons do, so the keyboard path and
/// the button path cannot drift apart.
pub fn run_capture_shortcut(app: &AppHandle, action: crate::hotkey::CaptureAction) {
    let state = app.state::<AppState>();
    let started = match action {
        crate::hotkey::CaptureAction::Region => capture_screen_region(app.clone(), state),
        crate::hotkey::CaptureAction::Window => capture_screen_window(app.clone(), state),
        crate::hotkey::CaptureAction::Screen => capture_whole_screen(app.clone(), state),
        crate::hotkey::CaptureAction::Scrolling => capture_scrolling_window(app.clone(), state),
        crate::hotkey::CaptureAction::Color => pick_screen_color(app.clone(), state),
        crate::hotkey::CaptureAction::Ruler => {
            crate::ruler::open(app.clone(), state.store.settings())
        }
    };
    if let Err(e) = started {
        log::warn!("could not start {} from its shortcut: {e}", action.label());
    }
}

/// Starts the recording a global shortcut fired.
///
/// Which mode a keyboard start uses is the one the toolbar last ran, so the key
/// and the button cannot disagree; a `fixed:WxH` mode places a frame of that size.
pub fn run_record_shortcut(app: &AppHandle, action: crate::hotkey::RecordAction) {
    let state = app.state::<AppState>();
    let started = match action {
        crate::hotkey::RecordAction::Start => {
            let mode = state.store.settings().record_mode;
            if let Some((width, height)) = fixed_mode(&mode) {
                record_fixed(app.clone(), state, width, height)
            } else {
                match mode.as_str() {
                    "window" => record_window(app.clone(), state),
                    "screen" => record_screen(app.clone(), state),
                    _ => record_region(app.clone(), state),
                }
            }
        }
        crate::hotkey::RecordAction::Stop => record_stop(app.clone()).map(|_| ()),
        crate::hotkey::RecordAction::Pause => {
            if state.recorder.status().phase == "paused" {
                record_resume(app.clone(), state)
            } else {
                record_pause(app.clone(), state)
            }
        }
    };
    if let Err(e) = started {
        log::warn!("could not run {}: {e}", action.label());
        let _ = app.emit(
            EVENT_RECORD_STATE,
            serde_json::json!({ "phase": "idle", "elapsedMs": 0, "error": e.to_string() }),
        );
    }
}

/// The size a stored `fixed:WxH` mode asks for, if that is what it is.
fn fixed_mode(mode: &str) -> Option<(i32, i32)> {
    let rest = mode.strip_prefix("fixed:")?;
    let (width, height) = rest.split_once('x')?;
    let width: i32 = width.trim().parse().ok()?;
    let height: i32 = height.trim().parse().ok()?;
    (width > 0 && height > 0).then_some((width, height))
}

/// Runs a capture off the IPC thread and reports the result through one event.
///
/// The overlay waits for the user, so the command cannot do the work itself; like
/// the colour picker it answers immediately and reports through the event.
fn start_capture(
    app: AppHandle,
    state: State<'_, AppState>,
    capture: fn(&AppHandle, u32) -> Result<Option<ClipContent>>,
) -> Result<()> {
    let store = Arc::clone(&state.store);
    // The delay is read here rather than inside the capture, so a change made while
    // one is being aimed cannot move the finish line.
    let delay = state.store.settings().capture_delay_secs;
    let handle = app;
    std::thread::spawn(move || {
        let result = match capture(&handle, delay) {
            Ok(Some(content)) => match store_image_clip(&handle, &store, content) {
                Ok((id, copied)) => serde_json::json!({ "id": id, "copied": copied }),
                Err(e) => {
                    log::warn!("could not store the screenshot: {e}");
                    serde_json::json!({ "id": null, "error": e.to_string() })
                }
            },
            // Cancelling is a choice, so the UI stays quiet.
            Ok(None) => serde_json::json!({ "id": null }),
            Err(e) => serde_json::json!({ "id": null, "error": e.to_string() }),
        };
        let _ = handle.emit(EVENT_SCREENSHOT_CAPTURED, result);
    });
    Ok(())
}

/// Stops a countdown that is already running; the screenshot it was counting down
/// to is dropped. Does nothing when no countdown is showing.
#[tauri::command]
pub fn cancel_capture_delay() {
    crate::countdown::cancel();
}

/// Inserts a captured image with the same preview, hash, media files, OCR and
/// history event a clipboard image gets.
fn store_image_clip(
    app: &AppHandle,
    store: &Arc<Store>,
    content: ClipContent,
) -> Result<(i64, bool)> {
    let new = build_new_clip(content, Some("Screenshot".into()));
    // A capture is expected to be pasteable straight away, the way the Snipping
    // Tool leaves its own on the clipboard. Without this the next paste puts back
    // whatever was copied before the screenshot. The hash is armed so the watcher
    // does not record the capture a second time.
    let copied = match clipboard::write(&new.content) {
        Ok(hash) => {
            app.state::<AppState>().watcher.suppress_hash(hash);
            true
        }
        Err(e) => {
            log::warn!("could not put the capture on the clipboard: {e}");
            false
        }
    };
    let (id, inserted) = match store.insert_clip(new)? {
        crate::store::InsertOutcome::Inserted(id) => (id, true),
        crate::store::InsertOutcome::Duplicate(id) => (id, false),
    };
    if inserted {
        clipboard::schedule_ocr(app, Arc::clone(store), id);
        clipboard::schedule_faces(app, Arc::clone(store), id);
    }
    // The same tick a copy makes, on every successful capture: a repeat of pixels
    // already in the history still counts as a capture, it just stored nothing new.
    sound::play(&store.settings(), sound::Outcome::Copied);
    let settings = store.settings();
    if settings.max_items > 0 && store.total() > settings.max_items as usize {
        let _ = store.prune();
    }
    notify_history_changed(app);
    Ok((id, copied))
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

/// Marks or unmarks a clip as a secret from the properties pane.
#[tauri::command]
pub fn set_clip_secret(app: AppHandle, state: State<'_, AppState>, id: i64, secret: bool) -> Result<()> {
    state.store.set_secret(id, secret)?;
    notify_history_changed(&app);
    Ok(())
}

/// The vault's state: "none", "locked" or "unlocked".
///
/// The frontend asks on every load, because a restart always comes back locked and
/// the interface has to say so rather than showing empty secrets with no reason.
#[tauri::command]
pub fn vault_status(state: State<'_, AppState>) -> &'static str {
    state.store.vault_status()
}

/// Creates the vault and leaves it open.
///
/// Answers with how many clips were sealed on the way: anything marked secret
/// before the vault existed was plaintext, and is encrypted now.
#[tauri::command]
pub fn create_vault(
    app: AppHandle,
    state: State<'_, AppState>,
    passphrase: String,
) -> Result<usize> {
    let sealed = state.store.create_vault(&passphrase)?;
    if sealed > 0 {
        notify_history_changed(&app);
    }
    crate::tray::refresh(&app);
    Ok(sealed)
}

/// Opens the vault for this run, answering with how many clips it sealed.
#[tauri::command]
pub fn unlock_vault(
    app: AppHandle,
    state: State<'_, AppState>,
    passphrase: String,
) -> Result<usize> {
    let sealed = state.store.unlock_vault(&passphrase)?;
    if sealed > 0 {
        notify_history_changed(&app);
    }
    crate::tray::refresh(&app);
    Ok(sealed)
}

/// Drops the key from memory; sealed clips stay sealed until it is opened again.
///
/// Shared by the command and the tray menu, which offers "Lock vault" only while
/// the vault is open.
pub fn lock_vault_now(app: &AppHandle) -> Result<()> {
    app.state::<AppState>().store.lock_vault()?;
    // Every sealed clip now reads as empty, so the list has to be rebuilt.
    notify_history_changed(app);
    crate::tray::refresh(app);
    Ok(())
}

#[tauri::command]
pub fn lock_vault(app: AppHandle) -> Result<()> {
    lock_vault_now(&app)
}

/// Rewraps the data key under a new passphrase. The clips are not re-encrypted.
#[tauri::command]
pub fn change_vault_passphrase(
    app: AppHandle,
    state: State<'_, AppState>,
    current: String,
    next: String,
) -> Result<usize> {
    let sealed = state.store.change_vault_passphrase(&current, &next)?;
    if sealed > 0 {
        notify_history_changed(&app);
    }
    Ok(sealed)
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

/// Stores text the user typed in the add-clip window.
///
/// The clipboard is deliberately untouched: this seeds the history from the
/// keyboard rather than copying, so the capture thread has nothing to suppress
/// and no second row appears.
#[tauri::command]
pub fn create_text_clip(app: AppHandle, state: State<'_, AppState>, text: String) -> Result<i64> {
    let new = typed_text_clip(text)?;
    let id = match state.store.insert_clip(new)? {
        crate::store::InsertOutcome::Inserted(id)
        | crate::store::InsertOutcome::Duplicate(id) => id,
    };
    notify_history_changed(&app);
    Ok(id)
}

/// Builds the `NewClip` for typed text, applying the capture path's preview and
/// search rules so a typed clip behaves like a copied one.
fn typed_text_clip(text: String) -> Result<NewClip> {
    if text.trim().is_empty() {
        return Err(AppError::msg("a new clip needs some text"));
    }
    Ok(build_new_clip(
        ClipContent::Text(text),
        Some("New clip".into()),
    ))
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

/// Imports a GIF or video chosen from disk into the utility's shelf.
///
/// The file is copied into the media folder under a content-addressed name so
/// the shelf entry survives the original being moved or deleted.
#[tauri::command]
pub fn import_custom_gif(state: State<'_, AppState>, path: String) -> Result<GifFavorite> {
    let source = Path::new(&path);
    let metadata = std::fs::metadata(source)
        .map_err(|e| AppError::msg(format!("could not read that file: {e}")))?;
    if metadata.len() > animated::MAX_VIDEO_BYTES as u64 {
        return Err(AppError::msg("that file is too large"));
    }
    let bytes = std::fs::read(source)
        .map_err(|e| AppError::msg(format!("could not read that file: {e}")))?;
    // The bytes decide the kind, not the extension: a file renamed to `.gif` is
    // stored under the name that matches its contents.
    let kind = if animated::is_gif(&bytes) {
        animated::MotionKind::Gif
    } else if animated::is_mp4(&bytes) {
        animated::MotionKind::Video
    } else {
        return Err(AppError::msg("that file is neither a GIF nor an MP4 video"));
    };

    let file = format!(
        "custom-{}.{}",
        hashing::of_motion(&bytes, kind),
        kind.extension()
    );
    let target = state.store.media_dir().join(&file);
    if !target.is_file() {
        std::fs::write(&target, &bytes)?;
    }
    let title = source
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.trim().is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            if kind.is_video() {
                "Custom video".to_string()
            } else {
                "Custom GIF".to_string()
            }
        });
    Ok(GifFavorite {
        id: format!("local:{file}"),
        title,
        url: String::new(),
        preview_url: file,
        still_url: String::new(),
        clip_id: None,
    })
}

/// Puts an imported GIF or video file on the clipboard, preserving every frame.
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
        .ok_or_else(|| AppError::NotFound(format!("media file `{file}`")))?;
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

    // A clip with a recovered GIF or video is published as a file so media-aware
    // targets keep every frame; everything else keeps the normal clipboard shape.
    // This covers a pasted movie address too, which is now a movie clip.
    let motion_path = store.motion_path(id);
    let hash = match motion_path {
        // A movie that came from a link publishes that link as well: a file drop is
        // refused by plenty of targets, and without the text they pasted nothing.
        Some(path) if store.is_video_clip(id) => match &content {
            ClipContent::Text(link) if !link.trim().is_empty() => {
                clipboard::write_file_and_text(&path, link.trim())?
            }
            _ => clipboard::write_files(&[path.to_string_lossy().into_owned()])?,
        },
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
                let reporter = app.clone();
                // Pasted from a worker thread: `SetForegroundWindow` needs our own
                // window to have finished hiding, and blocking the IPC thread here
                // would freeze the UI while it happens.
                std::thread::spawn(move || {
                    if let Err(e) = paste::restore_and_paste(target, delay) {
                        log::warn!("auto-paste failed: {e}");
                        report_paste_failure(&reporter, &e);
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

// ---------------------------------------------------------------------------
// Paste stack
// ---------------------------------------------------------------------------

fn notify_stack_changed(app: &AppHandle) {
    if let Err(e) = app.emit(EVENT_STACK_CHANGED, ()) {
        log::warn!("could not emit the stack change event: {e}");
    }
}

#[tauri::command]
pub fn stack_list(state: State<'_, AppState>) -> Result<Vec<StackEntry>> {
    state.store.stack()
}

/// Queues clips; answers how many were added, which is fewer when the cap bites.
#[tauri::command]
pub fn stack_push(app: AppHandle, state: State<'_, AppState>, ids: Vec<i64>) -> Result<usize> {
    let added = state.store.stack_push(&ids)?;
    if added > 0 {
        notify_stack_changed(&app);
    }
    Ok(added)
}

/// Splits a text clip into one queue item per line.
#[tauri::command]
pub fn stack_split(app: AppHandle, state: State<'_, AppState>, id: i64) -> Result<SplitOutcome> {
    let detail = state.store.detail(id)?;
    if detail.summary.kind != ClipKind::Text {
        return Err(AppError::msg("only a text clip can be split into stack items"));
    }
    // The lines would be stored as plain text, which is the opposite of what
    // marking the clip secret asked for.
    if detail.summary.secret_level.is_some() {
        return Err(AppError::msg(
            "this clip is marked secret; splitting it would copy its lines in the open",
        ));
    }
    let text = detail
        .text
        .ok_or_else(|| AppError::msg("unlock the vault to split this clip"))?;
    let lines = crate::store::split_lines(&text);
    if lines.is_empty() {
        return Err(AppError::msg("no list items in that clip"));
    }
    let queued = state.store.stack_push_lines(&lines)?;
    if queued > 0 {
        notify_stack_changed(&app);
    }
    Ok(SplitOutcome {
        total: lines.len(),
        queued,
    })
}

#[tauri::command]
pub fn stack_remove(app: AppHandle, state: State<'_, AppState>, id: i64) -> Result<()> {
    state.store.stack_remove(id)?;
    notify_stack_changed(&app);
    Ok(())
}

#[tauri::command]
pub fn stack_clear(app: AppHandle, state: State<'_, AppState>) -> Result<()> {
    state.store.stack_clear()?;
    notify_stack_changed(&app);
    Ok(())
}

/// Replaces the queue with a collection's clips, in the order the view lists them.
#[tauri::command]
pub fn stack_from_collection(
    app: AppHandle,
    state: State<'_, AppState>,
    collection_id: i64,
) -> Result<usize> {
    let queued = state.store.stack_replace_from_collection(collection_id)?;
    notify_stack_changed(&app);
    Ok(queued)
}

/// Pops the paste stack out into its own always-on-top window, so the queue can
/// sit beside the application it is pasting into.
#[tauri::command]
pub fn open_stack_window(app: AppHandle) -> Result<()> {
    window::show_stack_window(&app);
    Ok(())
}

/// Closes that window and hands the panel back to the main window.
#[tauri::command]
pub fn close_stack_window(app: AppHandle) -> Result<()> {
    window::close_stack_window(&app)
        .map_err(|e| AppError::msg(format!("could not close the stack window: {e}")))
}

/// Pastes the next queued item into whichever window had focus. Answers whether
/// there was anything to paste.
#[tauri::command]
pub fn stack_paste_next(app: AppHandle, state: State<'_, AppState>) -> Result<bool> {
    stack_paste_next_inner(&app, &state)
}

/// The global shortcut's entry point: an empty queue does nothing at all.
pub fn stack_paste_next_from_shortcut(app: &AppHandle) {
    let state = app.state::<AppState>();
    if let Err(e) = stack_paste_next_inner(app, &state) {
        log::warn!("could not paste the next stack item: {e}");
    }
}

fn stack_paste_next_inner(app: &AppHandle, state: &AppState) -> Result<bool> {
    // The shortcut fires while another application has the focus, so the window to
    // paste into is whatever is in front at that instant. Without this the paste
    // either went nowhere or landed in whichever window last summoned CopyPaste —
    // while the item was still consumed.
    paste::capture_foreground_excluding(&own_windows(app));
    if paste::target().is_none() {
        return Err(AppError::msg(
            "there is no window to paste into yet — click the window you want first",
        ));
    }
    let Some((row, clip_id, text)) = state.store.stack_first()? else {
        return Ok(false);
    };
    match (clip_id, text) {
        // A queued clip pastes exactly as activating it does, which is what makes
        // a stack of screenshots and links behave the way the history does.
        (Some(clip_id), _) => copy_clip_inner(app, state, clip_id, Some(true), Some(true))?,
        (_, Some(text)) => paste_literal_line(app, state, &text)?,
        _ => return Ok(false),
    }
    // Only a paste that worked consumes the item: a refusal leaves the queue
    // exactly where it was, so the next press tries the same thing again.
    state.store.stack_remove(row)?;
    notify_stack_changed(app);
    Ok(true)
}

/// Puts a line split out of a text clip on the clipboard and pastes it.
///
/// It is deliberately not added to the history: it came out of a clip that is
/// already there, and a queue of twenty lines would otherwise bury everything.
fn paste_literal_line(app: &AppHandle, state: &AppState, text: &str) -> Result<()> {
    let hash = clipboard::write(&ClipContent::Text(text.to_string()))?;
    let sequence = clipboard::sequence_number();
    state.watcher.suppress_hash(hash);
    let settings = state.store.settings();
    window::hide(app);
    schedule_auto_clear(app, sequence, &settings, true);
    if let Some(target) = paste::target() {
        let delay = u64::from(settings.paste_delay_ms);
        let reporter = app.clone();
        std::thread::spawn(move || {
            if let Err(e) = paste::restore_and_paste(target, delay) {
                log::warn!("auto-paste failed: {e}");
                report_paste_failure(&reporter, &e);
            }
        });
    }
    Ok(())
}

/// Handles of every window this app has open, so a paste never targets one.
fn own_windows(app: &AppHandle) -> Vec<isize> {
    app.webview_windows()
        .values()
        .filter_map(|window| window.hwnd().ok())
        .map(|handle| handle.0 as isize)
        .collect()
}

/// Tells the window that a scheduled paste did not land.
fn report_paste_failure(app: &AppHandle, error: &AppError) {
    if let Err(e) = app.emit(EVENT_PASTE_FAILED, error.to_string()) {
        log::warn!("could not report the failed paste: {e}");
    }
}

/// Longest edge the colour extractor decodes. It samples a 160 px grid, so
/// anything above this would be decoded only to be skipped.
const PALETTE_IMAGE_EDGE: u32 = 320;

/// Longest edge the preview's eyedropper decodes. The preview draws an image at
/// most about this wide, so a pick matches what the user aimed at, and a 4K
/// screenshot is not held in memory to read one pixel from it.
const SAMPLE_IMAGE_EDGE: u32 = 1024;

/// An image clip's colour scheme, extracting and remembering it the first time it
/// is asked for.
///
/// Both the properties pane and the preview's colour row go through this, so the
/// stored copy means a clip is decoded once however often it is shown again.
async fn palette_of(state: &AppState, id: i64) -> Result<Vec<String>> {
    if let Some(stored) = state.store.palette(id)? {
        return Ok(stored);
    }
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
    let dir = state.store.media_dir().to_path_buf();
    // Decoding is the expensive half; keep it off the webview's thread. The grid
    // the extractor samples is small, so a downscaled decode loses nothing.
    let decoded = tauri::async_runtime::spawn_blocking(move || {
        media::load_rgba_thumbnail(&dir, &name, PALETTE_IMAGE_EDGE)
    })
    .await
    .map_err(|e| AppError::msg(format!("colour extraction failed: {e}")))??;
    let (width, height, rgba) = decoded;
    let colors = crate::palette::extract(&rgba, width, height);
    if !colors.is_empty() {
        state.store.set_palette(id, &colors)?;
    }
    Ok(colors)
}

/// The picture a page offers for itself, fetched on request.
///
/// This is the only place a copied link leaves the machine, so it runs when the
/// user asks for it (or when the setting that does it automatically is on).
#[tauri::command]
pub async fn load_link_preview(
    app: AppHandle,
    state: State<'_, AppState>,
    id: i64,
) -> Result<String> {
    let url = link_url(&state, id)?;
    let stored = preview::store_for(&state.store, id, &url).await?;
    notify_history_changed(&app);
    Ok(stored)
}

/// The address of a clip that is nothing but a link.
fn link_url(state: &AppState, id: i64) -> Result<String> {
    let text = state
        .store
        .text_content(id)?
        .ok_or_else(|| AppError::msg("that clip has no text to preview"))?;
    let trimmed = text.trim();
    if crate::color::classify(trimmed) != Some(crate::color::ContentKind::Url)
        || !crate::preview::is_fetchable(trimmed)
    {
        return Err(AppError::msg("only an https link can be previewed"));
    }
    Ok(trimmed.to_string())
}

/// Returns an image clip's colour scheme, reading it from the image the first
/// time the clip is opened.
#[tauri::command]
pub async fn ensure_color_scheme(
    state: State<'_, AppState>,
    id: i64,
) -> Result<Vec<String>> {
    palette_of(&state, id).await
}

/// A colour the preview's eyedropper picked, and the scheme it now belongs to.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PickedColor {
    pub hex: String,
    /// False when the scheme already held the colour and was left alone.
    pub added: bool,
    pub colors: Vec<String>,
}

/// Adds the colour under one point of an image to that clip's scheme.
///
/// The preview samples the image rather than the screen: the user is already
/// pointing at the picture, and the screen picker has to hide the window to show
/// the desktop. `x` and `y` run 0..=1 across the image, so the size the zoomed
/// preview happens to be drawn at never reaches the backend.
#[tauri::command]
pub async fn add_scheme_color(
    state: State<'_, AppState>,
    id: i64,
    x: f64,
    y: f64,
) -> Result<PickedColor> {
    // The extracted scheme is completed first: a stored scheme is never
    // recomputed, so a clip holding nothing but a hand-picked colour would never
    // gain the colours its image already has.
    palette_of(&state, id).await?;
    let name = state
        .store
        .detail(id)?
        .summary
        .image_file
        .ok_or_else(|| AppError::msg("this image has no stored file"))?;
    let dir = state.store.media_dir().to_path_buf();
    let decoded = tauri::async_runtime::spawn_blocking(move || {
        media::load_rgba_thumbnail(&dir, &name, SAMPLE_IMAGE_EDGE)
    })
    .await
    .map_err(|e| AppError::msg(format!("colour sampling failed: {e}")))??;
    let (width, height, rgba) = decoded;
    let hex = crate::palette::sample(&rgba, width, height, x, y)
        .ok_or_else(|| AppError::msg("that spot has no colour to pick"))?;
    let (colors, added) = state.store.add_palette_color(id, &hex)?;
    Ok(PickedColor { hex, added, colors })
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

/// Opens settings on one section, by its slug ("screen-recording").
#[tauri::command]
pub fn open_settings_section(app: AppHandle, section: String) {
    log::info!("opening the settings window on `{section}`");
    window::show_settings_at(&app, Some(&section));
}

/// Opens the small window used to type a new clip.
#[tauri::command]
pub fn open_add_clip_window(app: AppHandle) {
    log::info!("opening the add-clip window");
    window::show_add_clip(&app);
}

/// Opens a clip in its own compact, always-on-top window.
#[tauri::command]
pub fn open_sticky_clip_window(app: AppHandle, state: State<'_, AppState>, id: i64) -> Result<()> {
    if id <= 0 {
        return Err(AppError::msg("Clip id must be positive."));
    }
    let detail = state.store.detail(id)?;
    let size = window::sticky_clip_size(&detail);
    window::show_sticky_clip(&app, id, size);
    Ok(())
}

/// Copies the text recognised in an image instead of the image itself.
///
/// The recognised text also becomes a text clip of its own: copying it is a copy
/// like any other, so the text has to be in the history afterwards rather than
/// only on the clipboard.
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

    let content = ClipContent::Text(text);
    let hash = clipboard::write(&content)?;
    let sequence = clipboard::sequence_number();
    // Same latch as `copy_clip`: our own write must not be captured again.
    state.watcher.suppress_hash(hash);
    store.record_copy(id)?;
    let new = build_new_clip(content, Some("Text in image".into()));
    // The clipboard write already succeeded; a history that refuses the entry must
    // not turn a good copy into a reported failure.
    if let Err(e) = store.insert_clip(new) {
        log::warn!("could not record the text recognised in an image: {e}");
    }
    notify_history_changed(&app);
    let should_hide = hide_window.unwrap_or(true);
    if should_hide {
        window::hide(&app);
    }
    schedule_auto_clear(&app, sequence, &store.settings(), should_hide);
    Ok(())
}

/// Puts only a clip's video file on the clipboard, without the address it came
/// from. `copy_clip` publishes both at once; this is for targets that want just
/// the file.
#[tauri::command]
pub fn copy_video_file(
    app: AppHandle,
    state: State<'_, AppState>,
    id: i64,
    hide_window: Option<bool>,
) -> Result<()> {
    let store = Arc::clone(&state.store);
    let path = store
        .video_path(id)
        .ok_or_else(|| AppError::msg("this clip has no stored video file"))?;
    let name = path
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or_default();
    if store.media_is_sealed(name) {
        return Err(AppError::msg(
            "this video is sealed in the vault, so it cannot be copied as a file",
        ));
    }
    let hash = clipboard::write_files(&[path.to_string_lossy().into_owned()])?;
    finish_media_copy(&app, &state, &store, id, hash, hide_window)
}

/// Puts the address a video was served from on the clipboard as plain text.
#[tauri::command]
pub fn copy_video_url(
    app: AppHandle,
    state: State<'_, AppState>,
    id: i64,
    hide_window: Option<bool>,
) -> Result<()> {
    let store = Arc::clone(&state.store);
    let url = store
        .video_url(id)
        .ok_or_else(|| AppError::msg("the address this video came from was not recorded"))?;
    let hash = clipboard::write(&ClipContent::Text(url))?;
    finish_media_copy(&app, &state, &store, id, hash, hide_window)
}

/// The tail every "copy one part of this clip" command shares: latch the watcher
/// so our own write is not captured again, count the copy, tell the list, and
/// honour the auto-clear setting.
fn finish_media_copy(
    app: &AppHandle,
    state: &AppState,
    store: &Arc<Store>,
    id: i64,
    hash: String,
    hide_window: Option<bool>,
) -> Result<()> {
    let sequence = clipboard::sequence_number();
    state.watcher.suppress_hash(hash);
    store.record_copy(id)?;
    notify_history_changed(app);
    let should_hide = hide_window.unwrap_or(true);
    if should_hide {
        window::hide(app);
    }
    schedule_auto_clear(app, sequence, &store.settings(), should_hide);
    Ok(())
}

/// Playing time of a clip's video, measured from the file the first time it is
/// asked for and remembered afterwards.
#[tauri::command]
pub fn clip_length(state: State<'_, AppState>, id: i64) -> Result<Option<i64>> {
    state.store.video_duration_ms(id)
}

/// Opens the recording toolbar: the modes, the audio source and the pointer switch.
#[tauri::command]
pub fn open_record_bar(app: AppHandle) -> Result<()> {
    window::show_record_bar(&app);
    Ok(())
}

/// What the toolbar needs to know before it can offer to record.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordToolchain {
    /// False when no encoder was found, which is what shows the install button.
    pub ready: bool,
    /// Where the encoder was found, for the settings page to show.
    pub path: Option<String>,
    /// DirectShow audio devices; a loopback one ("Stereo Mix") records the machine.
    pub devices: Vec<String>,
}

/// Finds the encoder and lists the audio devices it can capture.
#[tauri::command]
pub async fn record_toolchain(state: State<'_, AppState>) -> Result<RecordToolchain> {
    let settings = state.store.settings();
    tauri::async_runtime::spawn_blocking(move || match recorder::ffmpeg_path(&settings) {
        Some(path) => {
            let devices = recorder::audio_devices(&path);
            // Logged because "no devices in the dropdown" is otherwise a silent
            // disagreement between the machine and the recorder.
            log::info!(
                "screen recording will encode with {}; {} audio device(s) found",
                path.display(),
                devices.len()
            );
            RecordToolchain {
                ready: true,
                devices,
                path: Some(path.display().to_string()),
            }
        }
        None => {
            log::warn!("screen recording has no FFmpeg to encode with");
            RecordToolchain {
                ready: false,
                path: None,
                devices: Vec::new(),
            }
        }
    })
    .await
    .map_err(|e| AppError::msg(format!("could not inspect the encoder: {e}")))
}

/// Installs FFmpeg with winget, for a machine that has none.
#[tauri::command]
pub async fn install_ffmpeg() -> Result<String> {
    tauri::async_runtime::spawn_blocking(recorder::install_ffmpeg)
        .await
        .map_err(|e| AppError::msg(format!("the installer did not finish: {e}")))?
}

/// What the recording is doing right now; polled by the floating controls.
#[tauri::command]
pub fn record_status(state: State<'_, AppState>) -> recorder::RecordStatus {
    state.recorder.status()
}

/// Starts a recording of a region the user drags.
#[tauri::command]
pub fn record_region(app: AppHandle, state: State<'_, AppState>) -> Result<()> {
    start_recording(app, state, RecordMode::Region)
}

/// Starts a recording of the window the user clicks.
#[tauri::command]
pub fn record_window(app: AppHandle, state: State<'_, AppState>) -> Result<()> {
    start_recording(app, state, RecordMode::Window)
}

/// Starts a recording of every monitor at once.
#[tauri::command]
pub fn record_screen(app: AppHandle, state: State<'_, AppState>) -> Result<()> {
    start_recording(app, state, RecordMode::Screen)
}

/// Opens the placement frame for a recording of a fixed size.
#[tauri::command]
pub fn record_fixed(
    app: AppHandle,
    state: State<'_, AppState>,
    width: i32,
    height: i32,
) -> Result<()> {
    recording_ready(&state)?;
    if width < 64 || height < 64 {
        return Err(AppError::msg("that recording size is too small"));
    }
    // The recorder draws its blinking outline from a cached colour, normally
    // loaded by the picker this mode no longer opens.
    #[cfg(windows)]
    capture::load_outline(&app);
    window::show_record_area(&app, width, height);
    Ok(())
}

/// Starts the fixed-size recording once the frame has been placed.
///
/// The frontend sends the frame's rectangle in CSS pixels; the placement window's
/// physical origin and scale turn that into the screen rectangle the encoder reads.
#[tauri::command]
pub fn record_fixed_area(
    app: AppHandle,
    state: State<'_, AppState>,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<()> {
    recording_ready(&state)?;
    if !(x.is_finite() && y.is_finite() && width.is_finite() && height.is_finite())
        || width < 16.0
        || height < 16.0
    {
        return Err(AppError::msg("the recording frame has no size"));
    }
    let window = app
        .get_webview_window(window::RECORD_AREA_LABEL)
        .ok_or_else(|| AppError::msg("the recording frame is no longer open"))?;
    let origin = window
        .outer_position()
        .map_err(|e| AppError::msg(format!("could not place the recording frame: {e}")))?;
    let scale = window.scale_factor().unwrap_or(1.0);
    let rect = frame_to_screen(origin.x, origin.y, scale, x, y, width, height);
    #[cfg(windows)]
    let rect = clamp_to_desktop(rect);
    let _ = window.close();
    start_recording(app, state, RecordMode::Rect(rect))
}

/// Turns the placement overlay's frame into a screen rectangle.
///
/// The overlay works in CSS pixels relative to its own window; the encoder wants
/// physical pixels of the desktop the window sits on.
fn frame_to_screen(
    origin_x: i32,
    origin_y: i32,
    scale: f64,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> capture::Rect {
    let physical = |value: f64| (value * scale).round() as i32;
    capture::Rect::new(
        origin_x + physical(x),
        origin_y + physical(y),
        origin_x + physical(x + width),
        origin_y + physical(y + height),
    )
}

/// Pulls a recording rectangle back onto the desktop.
///
/// The frame can be dragged to the window's edge, and a frame wider than the
/// screen has nowhere to go, so the encoder is handed a rectangle it can read
/// rather than one whose far corner is off the desktop.
#[cfg(windows)]
fn clamp_to_desktop(mut rect: capture::Rect) -> capture::Rect {
    let bounds = capture::virtual_bounds();
    let width = (rect.right - rect.left).min(bounds.width()).max(1);
    let height = (rect.bottom - rect.top).min(bounds.height()).max(1);
    rect.left = rect.left.clamp(bounds.left, bounds.right - width);
    rect.top = rect.top.clamp(bounds.top, bounds.bottom - height);
    rect.right = rect.left + width;
    rect.bottom = rect.top + height;
    rect
}

/// Pauses the recording in progress; the encoder is closed at a segment boundary.
#[tauri::command]
pub fn record_pause(app: AppHandle, state: State<'_, AppState>) -> Result<()> {
    state.recorder.pause()?;
    emit_record_state(&app, &state);
    Ok(())
}

#[tauri::command]
pub fn record_resume(app: AppHandle, state: State<'_, AppState>) -> Result<()> {
    state.recorder.resume()?;
    emit_record_state(&app, &state);
    Ok(())
}

/// Ends the recording. The encoder takes a moment to close its file and the
/// finished MP4 has to be filed into the media folder, so the work happens on a
/// worker thread and the result arrives as [`EVENT_RECORD_FINISHED`].
#[tauri::command]
pub fn record_stop(app: AppHandle) -> Result<()> {
    if app.state::<AppState>().recorder.status().phase == "idle" {
        return Ok(());
    }
    std::thread::spawn(move || {
        let outcome = finish_recording(&app);
        let payload = match &outcome {
            Ok(Some(id)) => serde_json::json!({ "id": id }),
            Ok(None) => serde_json::json!({}),
            Err(e) => {
                log::warn!("could not finish the recording: {e}");
                serde_json::json!({ "error": e.to_string() })
            }
        };
        let _ = app.emit(EVENT_RECORD_FINISHED, payload);
    });
    Ok(())
}

/// Closes the session, turns its MP4 into a video clip and puts it on the
/// clipboard. Answers with the new clip's id, or `None` when nothing was recorded.
fn finish_recording(app: &AppHandle) -> Result<Option<i64>> {
    let state = app.state::<AppState>();
    let store = Arc::clone(&state.store);
    let settings = store.settings();
    let finished = state.recorder.stop()?;
    emit_record_state(app, &state);
    let Some(finished) = finished else {
        window::close_record_bar(app);
        return Ok(None);
    };
    let ffmpeg = recorder::ffmpeg_path(&settings)
        .ok_or_else(|| AppError::msg("FFmpeg vanished before the recording was saved"))?;

    let folder = finished.video.parent().unwrap_or(Path::new(".")).to_path_buf();
    let poster = folder.join("poster.png");
    recorder::poster(&ffmpeg, &finished.video, &poster)?;
    let (width, height, rgba) = media::load_rgba(&folder, "poster.png")?;
    let content = ClipContent::Image {
        width,
        height,
        rgba,
    };
    let new = build_new_clip(content, Some("Screen recording".into()));
    let id = match store.insert_clip(new)? {
        crate::store::InsertOutcome::Inserted(id) => id,
        crate::store::InsertOutcome::Duplicate(id) => id,
    };
    // The moving asset lives beside the still frame under the clip's id, which is
    // what makes this an ordinary video clip: player, badge, filters and the
    // right-click copy entries all follow from that.
    let target = store.media_dir().join(format!("{id}.mp4"));
    if let Err(e) = recorder::file_into_place(&finished.video, &target) {
        log::warn!("could not file the recording beside its clip: {e}");
        store.delete_many(&[id])?;
        return Err(AppError::msg(format!(
            "the recording could not be filed into the history: {e}"
        )));
    }
    store.refresh(id)?;
    finished.cleanup();

    // Filed first, then published: a failed clipboard write must not cost the user
    // the recording, which is sitting safely in the history by this point.
    if let Err(e) = copy_clip_inner(app, &state, id, Some(false), Some(false)) {
        log::warn!("could not put the recording on the clipboard: {e}");
    }
    sound::play(&settings, sound::Outcome::Copied);
    if settings.max_items > 0 && store.total() > settings.max_items as usize {
        let _ = store.prune();
    }
    notify_history_changed(app);
    window::close_record_bar(app);
    window::toggle(app);
    Ok(Some(id))
}

/// Runs the matching picker for `mode`, then hands the rectangle to the encoder.
fn start_recording(app: AppHandle, state: State<'_, AppState>, mode: RecordMode) -> Result<()> {
    recording_ready(&state)?;
    std::thread::spawn(move || {
        let chosen = match mode {
            RecordMode::Region => capture::choose_region(&app, None),
            RecordMode::Rect(rect) => Ok(Some(rect)),
            RecordMode::Window => capture::choose_window(&app),
            RecordMode::Screen => {
                // Nothing to choose: the whole desktop, with the app out of the way.
                window::hide(&app);
                Ok(Some(capture::desktop_bounds()))
            }
        };
        let rect = match chosen {
            Ok(Some(rect)) => rect,
            Ok(None) => {
                window::show(&app);
                return;
            }
            Err(e) => {
                window::show(&app);
                log::warn!("could not pick a recording area: {e}");
                let _ = app.emit(EVENT_RECORD_STATE, serde_json::json!({
                    "phase": "idle",
                    "elapsedMs": 0,
                    "error": e.to_string(),
                }));
                return;
            }
        };
        let state = app.state::<AppState>();
        match begin_recording(&app, &state, rect) {
            Ok(()) => {
                // The window stays down for the recording: the user asked for the
                // screen, and a clipboard history in the corner of it is noise.
                window::hide(&app);
                window::show_record_bar(&app);
            }
            Err(e) => {
                window::show(&app);
                log::warn!("could not start recording: {e}");
                let _ = app.emit(EVENT_RECORD_STATE, serde_json::json!({
                    "phase": "idle",
                    "elapsedMs": 0,
                    "error": e.to_string(),
                }));
            }
        }
    });
    Ok(())
}

fn begin_recording(app: &AppHandle, state: &AppState, rect: capture::Rect) -> Result<()> {
    let settings = state.store.settings();
    let ffmpeg = recorder::ffmpeg_path(&settings)
        .ok_or_else(|| AppError::msg("screen recording needs FFmpeg, which is not installed"))?;
    state.recorder.start(
        &ffmpeg,
        recorder::RecordRequest {
            rect,
            mouse: settings.record_mouse,
            audio: settings.record_audio_device.clone(),
            fps: settings.record_fps,
        },
    )?;
    // The click the summary asks for, played from the same switch as a copy.
    sound::play(&settings, sound::Outcome::Copied);
    emit_record_state(app, state);
    Ok(())
}

fn emit_record_state(app: &AppHandle, state: &AppState) {
    let _ = app.emit(EVENT_RECORD_STATE, state.recorder.status());
}

/// Confirms a recording can start: FFmpeg is present and nothing is recording yet.
///
/// Checked before any overlay opens: a missing encoder is a message, not a drag
/// the user completes only to be told it could not be recorded.
fn recording_ready(state: &AppState) -> Result<()> {
    let settings = state.store.settings();
    if recorder::ffmpeg_path(&settings).is_none() {
        return Err(AppError::msg(
            "screen recording needs FFmpeg, which was not found. Install it from Settings → Screen Recording.",
        ));
    }
    if state.recorder.status().phase != "idle" {
        return Err(AppError::msg("a recording is already running"));
    }
    Ok(())
}

/// Which picker a recording starts with.
#[derive(Debug, Clone, Copy)]
enum RecordMode {
    Region,
    Window,
    Screen,
    /// A rectangle already chosen by the placement overlay.
    Rect(capture::Rect),
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

/// Files a clip into the hidden archive behind the GIF shelf, or releases it.
#[tauri::command]
pub fn set_clip_saved_gif(
    app: AppHandle,
    state: State<'_, AppState>,
    clip_id: i64,
    saved: bool,
) -> Result<Vec<i64>> {
    let applied = state.store.set_saved_gif(clip_id, saved)?;
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

    // Print Screen belongs to the Snipping Tool until its per-user handler is
    // turned off, so the key has to be taken before the binding is registered —
    // and given back if the registration then fails. Turning the option off does
    // it in the other order: release the binding first, then the key.
    let taking_print_screen = after.print_screen_capture && !before.print_screen_capture;
    let giving_back_print_screen = before.print_screen_capture && !after.print_screen_capture;

    if taking_print_screen {
        match crate::printscreen::claim() {
            Ok(previous) => {
                // Recorded so turning the option off can restore exactly this.
                state
                    .store
                    .update_settings(&serde_json::json!({ "printScreenPrevious": previous }))?;
            }
            Err(e) => {
                if let Err(revert) = state
                    .store
                    .update_settings(&serde_json::json!({ "printScreenCapture": false }))
                {
                    log::error!("could not clear the Print Screen option: {revert}");
                }
                return Err(e);
            }
        }
    }

    if let Err(e) = save_shortcuts(&state.store, &before, &after, |previous, next| {
        crate::hotkey::apply(&app, previous, next)
    }) {
        if taking_print_screen {
            // The binding never took, so the key goes back where it was.
            if let Err(restore) = crate::printscreen::release(before.print_screen_previous) {
                log::error!("could not give Print Screen back: {restore}");
            }
        }
        return Err(e);
    }

    if giving_back_print_screen
        && let Err(e) = crate::printscreen::release(before.print_screen_previous)
    {
        log::warn!("could not give Print Screen back: {e}");
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

    // The settings window is a separate webview; without this the main window
    // would keep its old theme and layout until it was reopened.
    let _ = app.emit(EVENT_SETTINGS_CHANGED, &after);

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

/// Rebinds the global shortcuts after a settings change.
///
/// Windows refuses an accelerator another application owns. That refusal is
/// reported to the caller rather than swallowed, and the combination that used to
/// work stays registered (`hotkey::apply` puts it back) and stays stored, so a
/// rejected candidate can never replace a working shortcut.
///
/// `rebind` is passed in so the rollback can be exercised without a window station.
fn save_shortcuts(
    store: &Store,
    before: &Settings,
    after: &Settings,
    rebind: impl FnOnce(&crate::hotkey::Bindings, &crate::hotkey::Bindings) -> Result<()>,
) -> Result<()> {
    let previous = crate::hotkey::Bindings::from_settings(before);
    let next = crate::hotkey::Bindings::from_settings(after);
    if previous == next {
        return Ok(());
    }

    if let Err(e) = rebind(&previous, &next) {
        if let Err(revert) = store.update_settings(&shortcut_patch(before)) {
            log::error!("could not restore the previous shortcut: {revert}");
        }
        return Err(e);
    }
    Ok(())
}

/// The stored shortcut fields, for putting them back after a rejection.
fn shortcut_patch(settings: &Settings) -> serde_json::Value {
    serde_json::json!({
        "hotkey": settings.hotkey,
        "pauseHotkey": settings.pause_hotkey,
        "captureRegionHotkey": settings.capture_region_hotkey,
        "captureWindowHotkey": settings.capture_window_hotkey,
        "captureScreenHotkey": settings.capture_screen_hotkey,
        "captureScrollingHotkey": settings.capture_scrolling_hotkey,
        "pickColorHotkey": settings.pick_color_hotkey,
        "rulerHotkey": settings.ruler_hotkey,
        "newClipHotkey": settings.new_clip_hotkey,
        "recordHotkey": settings.record_hotkey,
        "recordStopHotkey": settings.record_stop_hotkey,
        "recordPauseHotkey": settings.record_pause_hotkey,
        "stackPasteHotkey": settings.stack_paste_hotkey,
        "printScreenCapture": settings.print_screen_capture,
        "printScreenPrevious": settings.print_screen_previous,
    })
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

/// Opens an image clip in the external editor chosen in Settings.
#[tauri::command]
pub fn edit_clip(app: AppHandle, state: State<'_, AppState>, id: i64) -> Result<()> {
    let editor = state.store.settings().image_editor;
    if editor.trim().is_empty() {
        return Err(AppError::msg(
            "choose an image editor in Settings to use this button",
        ));
    }
    let path = state.store.image_file_to_edit(id)?;
    let mut command = crate::editor::command_for(&editor, &path)?;
    let program = crate::editor::program_of(&editor)
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| editor.clone());
    // What the file looks like before the editor touches it, so a session that
    // changed nothing does not cost a re-encode.
    let before = crate::media::FileStamp::of(&path);
    // Editors that export their own file never touch `path`; their output folder is
    // where the edit has to be found.
    let export = crate::editor::export_dir(&editor);
    // Detached on purpose: the editor outlives this call, and CopyPaste has no
    // reason to wait for it to close.
    let mut child = command
        .spawn()
        .map_err(|e| AppError::msg(format!("could not launch {program}: {e}")))?;

    // Editors are GUI programs, so a rejected command line dies with its complaint
    // on a console nobody sees — exactly the "nothing happened" a wrong flag
    // produces. A quick non-zero exit is the only evidence available, so report it
    // instead of leaving the button looking broken.
    std::thread::sleep(std::time::Duration::from_millis(500));
    if let Ok(Some(status)) = child.try_wait() {
        if !status.success() {
            return Err(AppError::msg(format!(
                "{program} stopped straight away ({status}); check the arguments after the program in Settings"
            )));
        }
        // Exited cleanly already: either it handed the file to a copy that was
        // already running, or it is a launcher of some kind. Neither tells us when
        // the save happens, so the file itself has to.
        watch_file(app, id, path, before, export, None);
    } else {
        watch_file(app, id, path, before, export, Some(child));
    }
    Ok(())
}

/// How long to keep checking a file for a save when no process is holding the
/// editor open. Long enough for any single edit, short enough to stop eventually.
const WATCH_FOR_SAVE: std::time::Duration = std::time::Duration::from_secs(30 * 60);
/// Interval between checks while a running editor is the only thing left to watch
/// for, and while a save that was just seen is settling.
const LIVE_WATCH_INTERVAL: std::time::Duration = std::time::Duration::from_millis(250);
/// Interval between checks when nothing is changing yet: a live editor that has not
/// saved, or one that took the file and exited at once. A `stat` a second is
/// nothing, and it is what keeps a save from waiting on the poll to be noticed.
const WATCH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(1);
/// How many identical readings in a row mean the editor has finished writing.
const SETTLED_READINGS: u8 = 2;

/// Follows the file an editor was handed, so one completed save produces one
/// rebuild.
///
/// A stamp that just changed may still be growing — editors write in chunks — so a
/// change only counts once the same stamp has been read `SETTLED_READINGS` times in
/// a row. `baseline` is the state the clip was last rebuilt from, which is what
/// keeps a settled save from rebuilding twice: the readings that confirm it all
/// compare equal to it afterwards.
#[derive(Debug)]
struct SaveWatch {
    baseline: Option<crate::media::FileStamp>,
    current: Option<crate::media::FileStamp>,
    settled: u8,
}

impl SaveWatch {
    fn new(before: Option<crate::media::FileStamp>) -> Self {
        Self {
            baseline: before,
            current: before,
            settled: 0,
        }
    }

    /// Feeds one reading in, returning the stamp the clip was last rebuilt from
    /// when the file has settled on content the clip does not have yet.
    fn settled_change(
        &mut self,
        stamp: Option<crate::media::FileStamp>,
    ) -> Option<Option<crate::media::FileStamp>> {
        if stamp == self.current {
            self.settled = self.settled.saturating_add(1);
        } else {
            self.current = stamp;
            // This reading is the first of the pair that confirms the change.
            self.settled = 1;
        }
        if self.settled < SETTLED_READINGS || self.current == self.baseline {
            return None;
        }
        let previous = self.baseline;
        self.baseline = self.current;
        // The confirmed reading is spent: the next change starts a fresh pair.
        self.settled = SETTLED_READINGS;
        Some(previous)
    }

    /// Whether a save is still settling, which is when checks are worth making.
    fn pending(&self) -> bool {
        self.current != self.baseline
    }

    /// Whether the newest reading has been confirmed, so nothing more is coming.
    fn quiet(&self) -> bool {
        self.settled >= SETTLED_READINGS
    }
}

/// How long without a new export ends that watch.
///
/// The editor's window can stay open indefinitely, and an editor's save folder can
/// be the media folder itself; watching it for ever would adopt the next screenshot
/// taken into it as an edit of this clip.
const EXPORT_QUIET_AFTER: std::time::Duration = std::time::Duration::from_secs(10 * 60);
/// Image formats the store can read back, so the only ones worth adopting.
const EXPORT_EXTENSIONS: [&str; 3] = ["png", "jpg", "jpeg"];

/// The names in `dir` right now, which are what the editor has not written yet.
fn listing(dir: &Path) -> std::collections::HashSet<std::ffi::OsString> {
    std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name())
        .collect()
}

/// The newest image in `dir` that was not there when the watch began.
fn newest_export(
    dir: &Path,
    known: &std::collections::HashSet<std::ffi::OsString>,
) -> Option<(std::path::PathBuf, crate::media::FileStamp)> {
    let mut newest: Option<(std::path::PathBuf, crate::media::FileStamp)> = None;
    for entry in std::fs::read_dir(dir).ok()? {
        let Ok(entry) = entry else { continue };
        let name = entry.file_name();
        // A name that was already there is not this edit, and one the store wrote
        // itself would rebuild the clip from its own previous output.
        let managed = name
            .to_str()
            .is_some_and(crate::media::is_managed_name);
        if known.contains(&name) || managed {
            continue;
        }
        let path = entry.path();
        let readable = path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| {
                EXPORT_EXTENSIONS
                    .iter()
                    .any(|known| ext.eq_ignore_ascii_case(known))
            });
        if !readable {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        let stamp = crate::media::FileStamp {
            len: meta.len(),
            modified: meta.modified().ok(),
        };
        let Some(modified) = stamp.modified else {
            continue;
        };
        if newest
            .as_ref()
            .and_then(|(_, seen)| seen.modified)
            .is_none_or(|seen| seen <= modified)
        {
            newest = Some((path, stamp));
        }
    }
    newest
}

/// Watches the folder an export-only editor writes its results into.
///
/// The editor names those files itself, so the result is the newest image that
/// appears after the watch begins — the names already in the folder are recorded on
/// the first look and never taken for an edit. A file is only taken once a second look
/// agrees it has stopped changing, and each one is taken once, so saving twice adopts
/// the second save rather than repeating the first. The folder is listed on every
/// check: a directory's own timestamp is not reliably updated when a file lands in
/// it, and a media folder of a few hundred files lists in a few milliseconds.
struct ExportWatch {
    dir: std::path::PathBuf,
    /// Names present when the watch started.
    known: std::collections::HashSet<std::ffi::OsString>,
    started: bool,
    /// When the folder last offered something new, so a stale watch can stop.
    last_seen: std::time::Instant,
    candidate: Option<(std::path::PathBuf, crate::media::FileStamp)>,
    adopted: Option<(std::path::PathBuf, crate::media::FileStamp)>,
}

impl ExportWatch {
    fn new(dir: std::path::PathBuf) -> Self {
        Self {
            dir,
            known: std::collections::HashSet::new(),
            started: false,
            last_seen: std::time::Instant::now(),
            candidate: None,
            adopted: None,
        }
    }

    /// The file the editor has finished writing, if any.
    fn settled(&mut self) -> Option<std::path::PathBuf> {
        if !self.started {
            // The first look only records what is already there.
            self.started = true;
            self.known = listing(&self.dir);
            return None;
        }

        let newest = newest_export(&self.dir, &self.known);
        if newest != self.candidate {
            self.last_seen = std::time::Instant::now();
            self.candidate = newest;
            // The listing that found a file is not yet proof it is complete.
            return None;
        }

        let (path, stamp) = self.candidate.clone()?;
        if crate::media::FileStamp::of(&path) != Some(stamp) {
            return None;
        }
        if self.adopted.as_ref() == Some(&(path.clone(), stamp)) {
            return None;
        }
        self.adopted = Some((path.clone(), stamp));
        Some(path)
    }

    /// How long the folder has been quiet, for ending a watch nobody is saving to.
    fn idle_for(&self) -> std::time::Duration {
        self.last_seen.elapsed()
    }
}

/// Rebuilds the clip's image files whenever the editor saves over them.
///
/// The editor's window usually outlives the save — Paint stays open until the user
/// closes it — so the file is watched beside the process, and a save is picked up
/// as it lands instead of only when the editor quits. An editor that hands the file
/// to a copy already running exits at once, and then the file is all there is to
/// watch, until `WATCH_FOR_SAVE` gives up on it.
///
/// An editor that exports its own file instead of saving over the one it opened
/// never touches `path` at all, so `export` names the folder it writes into and the
/// newest image there is adopted as the edit.
///
/// A change is only acted on once the stamp has read the same twice in a row, so a
/// half-written PNG is never decoded.
fn watch_file(
    app: AppHandle,
    id: i64,
    path: std::path::PathBuf,
    before: Option<crate::media::FileStamp>,
    export: Option<std::path::PathBuf>,
    mut child: Option<std::process::Child>,
) {
    let spawned = std::thread::Builder::new()
        .name("editor-watch".into())
        .spawn(move || {
            let deadline = std::time::Instant::now() + WATCH_FOR_SAVE;
            // With no process to wait on, the deadline is the only ending.
            let handoff = child.is_none();
            let mut watch = SaveWatch::new(before);
            let mut exports = export.map(ExportWatch::new);
            let mut running = !handoff;
            let mut rebuilt = false;

            loop {
                // Brisk while a save is settling, patient at all other times.
                std::thread::sleep(if watch.pending() { LIVE_WATCH_INTERVAL } else { WATCH_INTERVAL });

                if running {
                    running = matches!(
                        child.as_mut().map(std::process::Child::try_wait),
                        Some(Ok(None))
                    );
                }

                let stamp = crate::media::FileStamp::of(&path);
                if let Some(previous) = watch.settled_change(stamp) {
                    // A missing file is the editor between a delete and a rewrite.
                    if stamp.is_some() {
                        let state = app.state::<AppState>();
                        match state.store.refresh_image_files(id, &path, previous) {
                            Ok(true) => {
                                log::info!("clip {id} changed on disk; rebuilt its image files");
                                rebuilt = true;
                                let _ = app.emit(EVENT_HISTORY_CHANGED, ());
                            }
                            Ok(false) => {}
                            Err(e) => {
                                log::warn!("could not rebuild the image files for clip {id}: {e}")
                            }
                        }
                    }
                }

                // A folder that has gone quiet stops being watched: the editor may
                // still be open, but nothing it writes now belongs to this edit.
                if exports
                    .as_ref()
                    .is_some_and(|watch| watch.idle_for() > EXPORT_QUIET_AFTER)
                {
                    exports = None;
                }

                // An export-only editor leaves the opened file alone, so its result
                // is whatever it has just written into its own folder.
                if let Some(export) = exports.as_mut().and_then(ExportWatch::settled) {
                    let state = app.state::<AppState>();
                    match state.store.refresh_image_files(id, &export, None) {
                        Ok(true) => {
                            log::info!("clip {id} adopted {}", export.display());
                            rebuilt = true;
                            let _ = app.emit(EVENT_HISTORY_CHANGED, ());
                        }
                        Ok(false) => {}
                        Err(e) => {
                            log::warn!("could not adopt {} for clip {id}: {e}", export.display())
                        }
                    }
                }

                if !running {
                    // A child that exited leaves nothing more to wait for once the
                    // last save has settled — unless an export folder is still being
                    // watched, which has its own quiet window: the editor may have
                    // handed the file to a window this process no longer owns.
                    let finished = watch.quiet()
                        && exports
                            .as_ref()
                            .is_none_or(|export| export.idle_for() > EXPORT_QUIET_AFTER);
                    if !handoff && finished {
                        break;
                    }
                    if handoff && std::time::Instant::now() >= deadline {
                        break;
                    }
                }
            }

            // Once the row points somewhere else, the file the editor was given is
            // only a working copy, and it would outlive the clip as litter. A
            // hand-off watch leaves it alone: the program that took the file may
            // still have it open, and stopping the watch is not proof it closed.
            if rebuilt && !handoff {
                let state = app.state::<AppState>();
                let replaced = state
                    .store
                    .image_file_to_edit(id)
                    .map(|current| current != path)
                    .unwrap_or(false);
                if replaced {
                    let _ = std::fs::remove_file(&path);
                }
            }
        });
    if let Err(e) = spawned {
        log::warn!("could not watch for the edited image: {e}");
    }
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
///
/// Shared by the command and the tray menu.
pub fn open_data_dir(app: &AppHandle) -> Result<()> {
    let dir = app
        .state::<AppState>()
        .store
        .data_dir()
        .to_string_lossy()
        .to_string();
    tauri_plugin_opener::open_path(dir, None::<&str>)
        .map_err(|e| AppError::msg(format!("could not open the data folder: {e}")))
}

#[tauri::command]
pub fn reveal_data_dir(app: AppHandle) -> Result<()> {
    open_data_dir(&app)
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
        // A timed pause ends on its own, with no event to react to, so tell the
        // tray when to stop offering "Resume capture".
        schedule_tray_refresh(&app, std::time::Duration::from_secs(minutes * 60));
    }
    let state = watcher.pause_state();
    emit_pause_state(&app, &state);
    state
}

fn schedule_tray_refresh(app: &AppHandle, after: std::time::Duration) {
    let handle = app.clone();
    let spawned = std::thread::Builder::new()
        .name("tray-refresh".into())
        .spawn(move || {
            std::thread::sleep(after + std::time::Duration::from_millis(500));
            crate::tray::refresh(&handle);
        });
    if let Err(e) = spawned {
        log::warn!("could not schedule the tray refresh: {e}");
    }
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
    // The tray menu swaps "Pause capture" for "Resume capture" with this state.
    crate::tray::refresh(app);
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

/// Writes a GIF converted from a video clip to the file the user chose.
///
/// The clip stays as it is: an export is a copy in another format, not a new clip.
#[tauri::command]
pub fn export_gif(state: State<'_, AppState>, id: i64, path: String) -> Result<String> {
    let target = std::path::PathBuf::from(path.trim());
    if !target.is_absolute() {
        return Err(AppError::msg("choose an absolute file path"));
    }
    let is_gif = target
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("gif"));
    if !is_gif {
        return Err(AppError::msg("the file has to end in .gif"));
    }
    let video = state.store.media_dir().join(format!("{id}.mp4"));
    if !video.is_file() {
        return Err(AppError::msg("this clip has no video file to convert"));
    }
    let settings = state.store.settings();
    let ffmpeg = recorder::ffmpeg_path(&settings).ok_or_else(|| {
        AppError::msg(
            "exporting a GIF needs FFmpeg, which was not found. Install it from Settings → Screen Recording.",
        )
    })?;
    recorder::gif(
        &ffmpeg,
        &video,
        &target,
        settings.gif_export_fps,
        settings.gif_export_width,
    )?;
    log::info!("exported clip {id} as a GIF to {}", target.display());
    Ok(target.to_string_lossy().into_owned())
}

/// Emitted while a video is being turned into a GIF, as a percentage.
pub const EVENT_GIF_PROGRESS: &str = "gif:progress";

/// Converts a video clip into a GIF and files it as a new image clip.
///
/// The original keeps its video: the GIF is a second, smaller copy that can be
/// pasted into a chat. Its poster is taken from the GIF's own first frame, so what
/// the list shows is what the moving image starts with. Progress goes out as
/// [`EVENT_GIF_PROGRESS`] because the conversion can take a while for a long clip.
#[tauri::command]
pub fn export_gif_clip(
    app: AppHandle,
    state: State<'_, AppState>,
    id: i64,
    quality: String,
) -> Result<i64> {
    let quality = recorder::GifQuality::from_key(&quality)
        .ok_or_else(|| AppError::msg("choose low, medium or high quality"))?;
    let store = Arc::clone(&state.store);
    let settings = store.settings();
    let ffmpeg = recorder::ffmpeg_path(&settings).ok_or_else(|| {
        AppError::msg(
            "exporting a GIF needs FFmpeg, which was not found. Install it from Settings → Screen Recording.",
        )
    })?;
    let video = store
        .video_path(id)
        .ok_or_else(|| AppError::msg("this clip has no video file to convert"))?;

    // A temp folder of its own: the GIF being written must not appear in the media
    // folder until there is a clip to name it after.
    let work = std::env::temp_dir().join(format!("copypaste-gif-{id}-{}", std::process::id()));
    std::fs::create_dir_all(&work)?;
    let out = work.join("clip.gif");
    let (width, fps) = quality.size();
    let report = |percent: u8| {
        let _ = app.emit(
            EVENT_GIF_PROGRESS,
            serde_json::json!({ "id": id, "percent": percent }),
        );
    };

    let converted = recorder::gif_with_progress(&ffmpeg, &video, &out, fps, width, &report);
    let filed = converted.and_then(|()| {
        let poster = work.join("poster.png");
        recorder::poster(&ffmpeg, &out, &poster)?;
        let (width, height, rgba) = media::load_rgba(&work, "poster.png")?;
        let new = build_new_clip(
            ClipContent::Image {
                width,
                height,
                rgba,
            },
            Some("GIF".into()),
        );
        let new_id = match store.insert_clip(new)? {
            crate::store::InsertOutcome::Inserted(id) | crate::store::InsertOutcome::Duplicate(id) => id,
        };
        let bytes = std::fs::read(&out)?;
        store.attach_motion(new_id, crate::animated::MotionKind::Gif, &bytes)?;
        store.refresh(new_id)?;
        Ok(new_id)
    });
    let _ = std::fs::remove_dir_all(&work);
    let new_id = filed?;

    notify_history_changed(&app);
    report(100);
    log::info!(
        "clip {id} became a {} GIF: clip {new_id}",
        quality.label()
    );
    Ok(new_id)
}

/// Writes one Markdown note per text clip, skipping images and file lists.
///
/// The folder is the one from the settings unless the caller resolved a
/// different one by asking the user. It must already exist: a vault that moved
/// should report the problem rather than gain an empty folder.
#[tauri::command]
pub fn export_markdown(
    state: State<'_, AppState>,
    ids: Vec<i64>,
    dir: String,
) -> Result<MarkdownExport> {
    let target = std::path::PathBuf::from(dir.trim());
    if !target.is_absolute() {
        return Err(AppError::msg("choose an absolute folder path"));
    }
    if ids.is_empty() {
        return Ok(MarkdownExport {
            written: Vec::new(),
            skipped: 0,
        });
    }
    let settings = state.store.settings();
    let export = state.store.export_markdown(
        &ids,
        &target,
        settings.markdown_front_matter,
        settings.markdown_date_prefix,
    )?;
    log::info!(
        "exported {} Markdown note(s) to {} ({} skipped)",
        export.written.len(),
        target.display(),
        export.skipped
    );
    Ok(export)
}

/// Puts arbitrary text on the clipboard and records it in the history.
///
/// Used by the emoji browser, the colour-preview copy buttons, the colour-scheme
/// copy and the error banner. A clipboard manager that swallows its own copies
/// looks broken: the tick says something happened, so the text has to be in the
/// history afterwards. `source` is what the list shows in its Source column.
#[tauri::command]
pub fn copy_text(
    app: AppHandle,
    state: State<'_, AppState>,
    text: String,
    source: Option<String>,
) -> Result<()> {
    let result = copy_text_inner(&app, &state, &text, source.as_deref());
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

fn copy_text_inner(
    app: &AppHandle,
    state: &AppState,
    text: &str,
    source: Option<&str>,
) -> Result<()> {
    if text.is_empty() {
        return Err(AppError::msg("there is nothing to copy"));
    }
    let content = ClipContent::Text(text.to_string());
    let hash = clipboard::write(&content)?;
    // Recorded just below, so the watcher must not record the same write again.
    state.watcher.suppress_hash(hash);
    let new = build_new_clip(content, Some(source.unwrap_or("CopyPaste").to_string()));
    // The clipboard write already succeeded; a history that refuses the entry must
    // not turn a good copy into a reported failure.
    match state.store.insert_clip(new) {
        Ok(_) => notify_history_changed(app),
        Err(e) => log::warn!("could not record a value copied inside the app: {e}"),
    }
    Ok(())
}

/// Files a generated colour scheme into a new "Color Palette N" archive.
#[tauri::command]
pub fn save_color_palette(
    app: AppHandle,
    state: State<'_, AppState>,
    colors: Vec<String>,
) -> Result<Collection> {
    let mut cleaned: Vec<String> = Vec::new();
    for raw in colors {
        let hex = crate::color::parse(raw.trim())
            .ok_or_else(|| AppError::msg(format!("\"{}\" is not a color", raw.trim())))?;
        if !cleaned.iter().any(|seen| seen.eq_ignore_ascii_case(&hex)) {
            cleaned.push(hex);
        }
    }
    if cleaned.is_empty() {
        return Err(AppError::msg("there are no colors to save"));
    }
    let archive = state.store.create_palette_archive(&cleaned)?;
    // Filing a palette is a successful action like a copy, so it gets the same tick.
    sound::play(&state.store.settings(), sound::Outcome::Copied);
    notify_history_changed(&app);
    Ok(archive)
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

/// Writes a message the window showed as an error into the log.
///
/// The banner is the only place some failures appear — a clipboard the system
/// refused, a file an editor would not take, a link that would not load — so
/// without this the log would be missing exactly what the user saw.
#[tauri::command]
pub fn report_error(message: String) {
    let text = message.trim();
    if !text.is_empty() {
        log::warn!("shown to the user: {text}");
    }
}

/// Shows the log file in Explorer, creating it first so there is something to find.
///
/// The file name belongs to the logging plugin, so the newest `.log` in the log
/// folder is used rather than a name guessed here.
#[tauri::command]
pub fn reveal_error_log(app: AppHandle) -> Result<()> {
    let dir = app
        .path()
        .app_log_dir()
        .map_err(|e| AppError::msg(format!("could not find the log folder: {e}")))?;
    std::fs::create_dir_all(&dir)?;
    let newest = std::fs::read_dir(&dir).ok().and_then(|entries| {
        entries
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry
                    .path()
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("log"))
            })
            .max_by_key(|entry| entry.metadata().ok().and_then(|meta| meta.modified().ok()))
            .map(|entry| entry.path())
    });
    match newest {
        Some(path) => tauri_plugin_opener::reveal_item_in_dir(&path)
            .map_err(|e| AppError::msg(format!("could not show the log: {e}"))),
        None => tauri_plugin_opener::open_path(dir, None::<&str>)
            .map_err(|e| AppError::msg(format!("could not open the log folder: {e}"))),
    }
}

/// Rich flavours of a text clip, fetched on demand so lists stay small.
#[tauri::command]
pub fn get_rich_text(state: State<'_, AppState>, id: i64) -> Result<RichText> {
    state.store.rich_text(id)
}

#[cfg(test)]
mod tests {
    use super::{
        ExportWatch, SaveWatch, is_email_address, listing, newest_export, save_shortcuts,
        typed_text_clip,
    };

    use crate::error::AppError;
    use crate::store::{InsertOutcome, test_store};

    #[test]
    fn a_placed_frame_becomes_the_screen_rectangle_around_it() {
        // A second monitor at 150%: the overlay's own origin is not zero, and the
        // frame's CSS pixels are not screen pixels.
        let rect = super::frame_to_screen(1_920, -120, 1.5, 40.0, 20.0, 640.0, 360.0);

        assert_eq!((rect.left, rect.top), (1_980, -90));
        assert_eq!((rect.right, rect.bottom), (2_940, 450));
    }

    #[cfg(windows)]
    #[test]
    fn a_frame_dragged_past_the_edge_is_pulled_back_on_screen() {
        use crate::capture::{Rect, virtual_bounds};

        let bounds = virtual_bounds();
        let width = 2_000.min(bounds.width());
        let height = 2_000.min(bounds.height());
        let rect = super::clamp_to_desktop(Rect::new(
            bounds.right - 10,
            bounds.bottom - 10,
            bounds.right + 1_990,
            bounds.bottom + 1_990,
        ));

        // The far corner is pinned to the edge of the desktop and the frame keeps
        // as much of its size as the screen has room for.
        assert_eq!((rect.right, rect.bottom), (bounds.right, bounds.bottom));
        assert_eq!((rect.right - rect.left, rect.bottom - rect.top), (width, height));
    }

    #[test]
    fn a_rejected_shortcut_leaves_the_stored_value_unchanged() {
        let (store, _dir) = test_store();
        let before = store
            .update_settings(&serde_json::json!({ "captureRegionHotkey": "Ctrl+Shift+R" }))
            .unwrap();
        let after = store
            .update_settings(&serde_json::json!({ "captureRegionHotkey": "Ctrl+Shift+K" }))
            .unwrap();

        // The OS owns Ctrl+Shift+K, which is exactly what `rebind` reports.
        let rejected = save_shortcuts(&store, &before, &after, |_, _| {
            Err(AppError::msg(
                "`Ctrl+Shift+K` could not be registered for the region screenshot",
            ))
        });

        assert!(rejected.is_err(), "the conflict reaches the caller");
        assert_eq!(
            store.settings().capture_region_hotkey,
            "Ctrl+Shift+R",
            "the refused combination never replaces the stored one"
        );
    }

    #[test]
    fn an_unchanged_shortcut_is_not_reregistered() {
        let (store, _dir) = test_store();
        let before = store.settings();
        let after = store
            .update_settings(&serde_json::json!({ "autoPaste": true }))
            .unwrap();

        let rebound = save_shortcuts(&store, &before, &after, |_, _| {
            panic!("an unrelated setting must not touch the shortcuts")
        });
        assert!(rebound.is_ok());
    }

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

    #[test]
    fn typed_text_is_refused_when_blank() {
        assert!(typed_text_clip(String::new()).is_err());
        assert!(typed_text_clip("  \n\t ".into()).is_err());
    }

    #[test]
    fn typed_clips_use_the_capture_preview_and_hash() {
        let text = "First line\nsecond line";
        let clip = typed_text_clip(text.into()).unwrap();
        assert_eq!(clip.preview, "First line second line");
        assert_eq!(clip.content_hash, crate::hashing::of_str(text));
        assert_eq!(clip.source_app.as_deref(), Some("New clip"));
        assert_eq!(clip.search_text, crate::search::normalize(text));
    }

    #[test]
    fn typed_markdown_is_stored_with_a_markdown_badge() {
        let (store, _dir) = test_store();
        let clip = typed_text_clip("# Release notes\n\n- one\n- two".into()).unwrap();
        let id = match store.insert_clip(clip).unwrap() {
            InsertOutcome::Inserted(id) => id,
            _ => unreachable!(),
        };
        assert_eq!(
            store.detail(id).unwrap().summary.content_kind.as_deref(),
            Some("markdown")
        );
    }

    #[test]
    fn typed_plain_text_is_not_flagged_as_markdown() {
        let (store, _dir) = test_store();
        let clip =
            typed_text_clip("Remember to call the bank about the invoice".into()).unwrap();
        let id = match store.insert_clip(clip).unwrap() {
            InsertOutcome::Inserted(id) => id,
            _ => unreachable!(),
        };
        assert!(store.detail(id).unwrap().summary.content_kind.is_none());
    }

    /// A file of a given size, last written at a given second.
    fn stamp(len: u64, secs: u64) -> crate::media::FileStamp {
        crate::media::FileStamp {
            len,
            modified: Some(std::time::UNIX_EPOCH + std::time::Duration::from_secs(secs)),
        }
    }

    #[test]
    fn an_unsaved_file_is_never_rebuilt() {
        let mut watch = SaveWatch::new(Some(stamp(10, 1)));
        assert!(watch.settled_change(Some(stamp(10, 1))).is_none());
        assert!(watch.settled_change(Some(stamp(10, 1))).is_none());
        assert!(watch.quiet(), "identical readings are as settled as it gets");
        assert!(!watch.pending(), "so the watch can afford to be patient");
    }

    #[test]
    fn a_save_is_rebuilt_once_it_settles_and_only_once() {
        let mut watch = SaveWatch::new(Some(stamp(10, 1)));
        assert!(
            watch.settled_change(Some(stamp(20, 2))).is_none(),
            "one reading could still be a half-written file"
        );
        assert!(watch.pending(), "a changed stamp keeps the checks brisk");
        assert_eq!(
            watch.settled_change(Some(stamp(20, 2))),
            Some(Some(stamp(10, 1))),
            "the save is rebuilt from what the clip last held"
        );
        assert!(
            watch.settled_change(Some(stamp(20, 2))).is_none(),
            "the readings that confirm one save must not rebuild it twice"
        );
        assert!(!watch.pending());
    }

    #[test]
    fn a_second_save_in_the_same_session_is_rebuilt_too() {
        let mut watch = SaveWatch::new(Some(stamp(10, 1)));
        watch.settled_change(Some(stamp(20, 2)));
        watch.settled_change(Some(stamp(20, 2)));
        assert!(
            watch.settled_change(Some(stamp(30, 3))).is_none(),
            "the editor's later save starts a new settle"
        );
        assert_eq!(
            watch.settled_change(Some(stamp(30, 3))),
            Some(Some(stamp(20, 2))),
            "and is rebuilt in its own right"
        );
    }

    #[test]
    fn a_deleted_file_settles_without_content_to_read() {
        let mut watch = SaveWatch::new(Some(stamp(10, 1)));
        assert!(watch.settled_change(None).is_none());
        assert_eq!(
            watch.settled_change(None),
            Some(Some(stamp(10, 1))),
            "the caller is told the file changed, and reads nothing from it"
        );
        assert!(watch.quiet());
    }

    #[test]
    fn the_newest_export_is_the_edit() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("notes.txt"), b"not an image").unwrap();
        std::fs::write(dir.path().join("Screenshot 1.png"), b"first").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(dir.path().join("Screenshot 2.png"), b"second").unwrap();

        // What the folder held before the watch began.
        let mut known = std::collections::HashSet::new();
        known.insert(std::ffi::OsString::from("Screenshot 1.png"));

        let newest = newest_export(dir.path(), &known);
        assert_eq!(
            newest.map(|(path, _)| path.file_name().unwrap().to_string_lossy().into_owned()),
            Some("Screenshot 2.png".to_string()),
            "the last save wins, and a file that was already there never does"
        );
        assert!(
            newest_export(dir.path(), &listing(dir.path())).is_none(),
            "nothing new in the folder means nothing to adopt"
        );
    }

    #[test]
    fn an_export_is_adopted_once_it_has_settled_and_once_only() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("older.png"), b"older").unwrap();
        let mut watch = ExportWatch::new(dir.path().to_path_buf());
        assert_eq!(
            watch.settled(),
            None,
            "the first look only records what is already there"
        );

        let saved = dir.path().join("Screenshot 2026-10-30 at 09.52.28.png");
        std::fs::write(&saved, b"pixels").unwrap();
        assert_eq!(
            watch.settled(),
            None,
            "a file that has just appeared is not proof it is complete"
        );
        assert_eq!(watch.settled(), Some(saved.clone()));
        assert_eq!(watch.settled(), None, "the same export is not adopted twice");
    }

    #[test]
    fn a_second_export_is_adopted_in_its_own_right() {
        let dir = tempfile::tempdir().unwrap();
        let mut watch = ExportWatch::new(dir.path().to_path_buf());
        assert_eq!(watch.settled(), None, "an empty folder is still a snapshot");

        let first = dir.path().join("first.png");
        std::fs::write(&first, b"first").unwrap();
        assert_eq!(watch.settled(), None, "the new file is listed first");
        assert_eq!(watch.settled(), Some(first));

        std::thread::sleep(std::time::Duration::from_millis(20));
        let second = dir.path().join("second.png");
        std::fs::write(&second, b"second").unwrap();
        assert_eq!(watch.settled(), None, "the newer file starts its own settle");
        assert_eq!(watch.settled(), Some(second));
    }

    #[test]
    fn the_stores_own_files_are_never_taken_for_an_edit() {
        let dir = tempfile::tempdir().unwrap();
        let nothing_known = std::collections::HashSet::new();
        // What the store writes when it rebuilds a clip — and the editor can be made
        // to save into that very folder.
        std::fs::write(
            dir.path().join("0123456789abcdef0123456789abcdef.png"),
            b"image",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("0123456789abcdef0123456789abcdef.thumb.png"),
            b"thumb",
        )
        .unwrap();
        assert!(
            newest_export(dir.path(), &nothing_known).is_none(),
            "a rebuild must never feed the watch that asked for it"
        );

        std::fs::write(
            dir.path().join("Screenshot 2026-10-30 at 09.52.28.png"),
            b"edit",
        )
        .unwrap();
        let newest = newest_export(dir.path(), &nothing_known);
        assert_eq!(
            newest.map(|(path, _)| path.file_name().unwrap().to_string_lossy().into_owned()),
            Some("Screenshot 2026-10-30 at 09.52.28.png".to_string()),
            "the editor's own file is still found beside them"
        );
    }
}
