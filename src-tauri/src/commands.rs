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
    ClipContent, ClipDetail, ClipFilter, ClipKind, ClipPage, Collection, NewClip, RichText, Stats,
    Tag,
};
use crate::paste;
use crate::picker;
use crate::preview;
use crate::search;
use crate::settings::{GifFavorite, Settings};
use crate::sound;
use crate::storage::StorageInfo;
use crate::store::Store;
use crate::window;

use crate::clipboard::PauseState;

/// Emitted when the stored settings change, so a second window can follow.
pub const EVENT_SETTINGS_CHANGED: &str = "settings:changed";

/// Page size used when the frontend does not ask for a specific window.
const DEFAULT_PAGE: usize = 200;

/// Emitted when a screen sample finishes. `error` is set when the pick was
/// cancelled or failed, so the UI can explain what happened.
pub const EVENT_COLOR_PICKED: &str = "ui:color-picked";

/// Emitted when a screenshot finishes. `error` is set when the capture failed,
/// and `id` is the stored clip, or null when the user cancelled.
pub const EVENT_SCREENSHOT_CAPTURED: &str = "ui:screenshot-captured";

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
    };
    if let Err(e) = started {
        log::warn!("could not start {} from its shortcut: {e}", action.label());
    }
}

/// Runs a capture off the IPC thread and reports the result through one event.
///
/// The overlay waits for the user, so the command cannot do the work itself; like
/// the colour picker it answers immediately and reports through the event.
fn start_capture(
    app: AppHandle,
    state: State<'_, AppState>,
    capture: fn(&AppHandle) -> Result<Option<ClipContent>>,
) -> Result<()> {
    let store = Arc::clone(&state.store);
    let handle = app;
    std::thread::spawn(move || {
        let result = match capture(&handle) {
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
        watch_file(app, id, path, before, None);
    } else {
        watch_file(app, id, path, before, Some(child));
    }
    Ok(())
}

/// How long to keep checking a file for a save when no process is holding the
/// editor open. Long enough for any single edit, short enough to stop eventually.
const WATCH_FOR_SAVE: std::time::Duration = std::time::Duration::from_secs(30 * 60);
/// Interval between those checks.
const WATCH_INTERVAL: std::time::Duration = std::time::Duration::from_secs(2);
/// How many identical readings in a row mean the editor has finished writing.
const SETTLED_READINGS: u8 = 2;

/// Waits for an editor session to end, then rebuilds the clip's image files.
///
/// Two endings have to be covered. An editor that keeps its own window open —
/// ShotCraft, Paint — ends when its process exits, and that is the reliable signal.
/// An editor that hands the file to a copy already running exits immediately, so
/// the file's own stamp is watched instead, and the refresh waits for it to stop
/// changing so a half-written PNG is never read.
fn watch_file(
    app: AppHandle,
    id: i64,
    path: std::path::PathBuf,
    before: Option<crate::media::FileStamp>,
    mut child: Option<std::process::Child>,
) {
    let spawned = std::thread::Builder::new()
        .name("editor-watch".into())
        .spawn(move || {
            match child.as_mut() {
                Some(child) => {
                    let _ = child.wait();
                }
                None => {
                    let deadline = std::time::Instant::now() + WATCH_FOR_SAVE;
                    let mut settled = 0u8;
                    while std::time::Instant::now() < deadline {
                        std::thread::sleep(WATCH_INTERVAL);
                        let stamp = crate::media::FileStamp::of(&path);
                        if stamp == before {
                            continue;
                        }
                        settled += 1;
                        if settled >= SETTLED_READINGS {
                            break;
                        }
                    }
                }
            }

            let state = app.state::<AppState>();
            match state.store.refresh_image_files(id, before) {
                Ok(true) => {
                    log::info!("clip {id} changed on disk; rebuilt its image files");
                    let _ = app.emit(EVENT_HISTORY_CHANGED, ());
                }
                Ok(false) => {}
                Err(e) => log::warn!("could not rebuild the image files for clip {id}: {e}"),
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

/// Rich flavours of a text clip, fetched on demand so lists stay small.
#[tauri::command]
pub fn get_rich_text(state: State<'_, AppState>, id: i64) -> Result<RichText> {
    state.store.rich_text(id)
}

#[cfg(test)]
mod tests {
    use super::{is_email_address, save_shortcuts, typed_text_clip};

    use crate::error::AppError;
    use crate::store::{InsertOutcome, test_store};

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
}
