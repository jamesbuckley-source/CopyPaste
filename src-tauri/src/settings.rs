//! User-visible configuration, persisted one row per field.
//!
//! Storing fields individually (rather than a single JSON blob) means a partial
//! patch from the settings UI only rewrites the keys it touches, and a truncated
//! write can never lose the whole configuration.

use std::collections::HashMap;

use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use crate::error::{AppError, Result};

pub const DEFAULT_HOTKEY: &str = "Alt+V";

/// Every color the interface paints with. The frontend maps these onto the CSS
/// custom properties Tailwind's `@theme` block declares, so a theme swap is a
/// handful of variable writes rather than a second stylesheet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ThemeColors {
    pub canvas: String,
    pub panel: String,
    pub panel2: String,
    pub panel3: String,
    pub edge: String,
    pub edge_soft: String,
    pub ink: String,
    pub ink_dim: String,
    pub ink_faint: String,
    pub accent: String,
    pub accent_hi: String,
    pub danger: String,
}

impl ThemeColors {
    /// The palette the app shipped with.
    pub fn dark() -> Self {
        ThemeColors {
            canvas: "#0b0e14".into(),
            panel: "#12161f".into(),
            panel2: "#181d29".into(),
            panel3: "#1f2533".into(),
            edge: "#252c3b".into(),
            edge_soft: "#1c2230".into(),
            ink: "#e7eaf0".into(),
            ink_dim: "#99a3b5".into(),
            ink_faint: "#6a7488".into(),
            accent: "#6d8cff".into(),
            accent_hi: "#8aa2ff".into(),
            danger: "#ff6b6b".into(),
        }
    }

    pub fn light() -> Self {
        ThemeColors {
            canvas: "#eef1f6".into(),
            panel: "#ffffff".into(),
            panel2: "#f5f7fb".into(),
            panel3: "#e6eaf2".into(),
            edge: "#d2d8e4".into(),
            edge_soft: "#e6eaf2".into(),
            ink: "#1b2130".into(),
            ink_dim: "#525d75".into(),
            ink_faint: "#8590a6".into(),
            accent: "#3559d6".into(),
            accent_hi: "#2748b8".into(),
            danger: "#cf3b3b".into(),
        }
    }

    fn sanitized(mut self) -> Self {
        let defaults = ThemeColors::dark();
        for (slot, fallback) in [
            (&mut self.canvas, defaults.canvas),
            (&mut self.panel, defaults.panel),
            (&mut self.panel2, defaults.panel2),
            (&mut self.panel3, defaults.panel3),
            (&mut self.edge, defaults.edge),
            (&mut self.edge_soft, defaults.edge_soft),
            (&mut self.ink, defaults.ink),
            (&mut self.ink_dim, defaults.ink_dim),
            (&mut self.ink_faint, defaults.ink_faint),
            (&mut self.accent, defaults.accent),
            (&mut self.accent_hi, defaults.accent_hi),
            (&mut self.danger, defaults.danger),
        ] {
            *slot = normalize_hex(slot).unwrap_or(fallback);
        }
        self
    }
}

impl Default for ThemeColors {
    fn default() -> Self {
        ThemeColors::dark()
    }
}

/// Lower-cases a `#rrggbb` value, rejecting anything a color input could not have
/// produced so that a hand-edited config cannot inject arbitrary CSS.
fn normalize_hex(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    let digits = trimmed.strip_prefix('#')?;
    if digits.len() != 6 || !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    Some(format!("#{}", digits.to_ascii_lowercase()))
}

/// Preferred layout per history category.
///
/// Each sidebar entry keeps its own choice, so images can default to cards while
/// text stays in rows. Collections and tags share the `other` slot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ViewModes {
    pub all: String,
    pub text: String,
    pub images: String,
    pub files: String,
    pub colors: String,
    pub pinned: String,
    pub other: String,
}

impl Default for ViewModes {
    fn default() -> Self {
        ViewModes {
            all: "list".into(),
            text: "list".into(),
            images: "grid".into(),
            files: "list".into(),
            colors: "grid".into(),
            pinned: "list".into(),
            other: "list".into(),
        }
    }
}

impl ViewModes {
    fn sanitized(mut self) -> Self {
        fn clean(value: &mut String) {
            if value != "grid" {
                *value = "list".into();
            }
        }
        clean(&mut self.all);
        clean(&mut self.text);
        clean(&mut self.images);
        clean(&mut self.files);
        clean(&mut self.colors);
        clean(&mut self.pinned);
        clean(&mut self.other);
        self
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Theme {
    /// "dark", "light" or "custom".
    pub mode: String,
    /// Only consulted in custom mode; kept so switching back and forth does not
    /// lose hand-picked colors.
    pub colors: ThemeColors,
}

impl Default for Theme {
    fn default() -> Self {
        Theme {
            mode: "dark".into(),
            colors: ThemeColors::dark(),
        }
    }
}

impl Theme {
    fn sanitized(mut self) -> Self {
        if !matches!(self.mode.as_str(), "dark" | "light" | "custom") {
            self.mode = "dark".into();
        }
        self.colors = self.colors.sanitized();
        self
    }
}

/// Bare media names the GIF utility may store for imported files.
///
/// Mirrors `media::resolve`'s allowlist, so a hand-edited settings row cannot
/// point the shelf at a path outside the media folder.
fn is_custom_gif_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name.to_ascii_lowercase().ends_with(".gif")
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        && !name.contains("..")
}

/// A named query shown in the sidebar. The query is stored verbatim, operators included.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedSearch {
    pub name: String,
    pub query: String,
}

/// One entry in the GIF utility's top shelf.
///
/// Remote GIFs carry their GIPHY addresses; clips saved from the history only
/// carry the clip id, so the browser can resolve a fresh thumbnail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GifFavorite {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub preview_url: String,
    /// Static frame used while the animation is paused (hover-to-play).
    #[serde(default)]
    pub still_url: String,
    #[serde(default)]
    pub clip_id: Option<i64>,
}

/// Per-archive display choices for the history list and cards.
///
/// Some archives exist to keep material out of sight, so their clips can hide
/// the preview text and the note marker independently in each layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ArchiveDisplay {
    pub archive_id: i64,
    /// Show the clip preview in list rows.
    pub list_content: bool,
    /// Show the note marker in list rows.
    pub list_notes: bool,
    /// Show the clip preview on cards.
    pub card_content: bool,
    /// Show the note marker on cards.
    pub card_notes: bool,
}

impl Default for ArchiveDisplay {
    fn default() -> Self {
        ArchiveDisplay {
            archive_id: 0,
            list_content: true,
            list_notes: true,
            card_content: true,
            card_notes: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// Global shortcut that summons the window.
    pub hotkey: String,
    /// Shortcut that toggles the private capture pause.
    pub pause_hotkey: String,
    /// Keep rich text (HTML/RTF) alongside the plain body and paste it back.
    pub paste_rich_text: bool,
    /// Empty the Windows clipboard a while after a clip is reused.
    pub auto_clear_clipboard: bool,
    /// Seconds before the clipboard is emptied; 0 means "as soon as the overlay hides".
    pub auto_clear_delay_secs: u32,
    /// Paste into the previously focused window after selecting a clip.
    pub auto_paste: bool,
    /// Hide the overlay after a clip is activated with a double click.
    pub close_on_double_click: bool,
    /// Grace period between hiding our window and sending Ctrl+V.
    pub paste_delay_ms: u32,
    pub launch_at_login: bool,
    /// Start with the window hidden; the tray icon and hotkey bring it up.
    pub start_hidden: bool,
    /// Hide the window when it loses focus. Off by default: the window is an
    /// inspection surface, and it vanishing when you click away to look something
    /// up is worse than the slight clutter of leaving it open.
    pub hide_on_blur: bool,
    /// Keep the window above other applications.
    pub always_on_top: bool,    /// Keep a taskbar button. Off makes the app tray-only.
    pub show_in_taskbar: bool,
    pub capture_images: bool,
    pub capture_files: bool,
    /// Screenshot mode the header button remembers: "region", "window",
    /// "screen" or "scrolling". Unknown values fall back to "region".
    pub capture_mode: String,
    /// When a copy exposes both text and a bitmap, prefer the bitmap.
    pub prefer_images_over_text: bool,
    /// Honour the opt-out formats password managers publish.
    pub ignore_sensitive: bool,
    /// Lower-cased process names whose copies are never recorded.
    pub ignored_apps: Vec<String>,
    /// Look for faces in screenshots, so images can be filtered as People.
    pub detect_faces: bool,
    /// Oldest items are pruned past this count. `0` means unlimited.
    pub max_items: u32,
    /// `0` means unlimited.
    pub max_age_days: u32,
    /// Sub-day retention for people who want the history to forget within hours.
    /// Both age limits apply; `0` disables the one it belongs to.
    pub max_age_hours: u32,
    pub max_text_bytes: u64,
    pub max_image_bytes: u64,
    pub poll_interval_ms: u64,
    /// Preferred layout per history category.
    pub view_modes: ViewModes,
    /// Card size multiplier for the grid view.
    pub grid_scale: f64,
    /// Card size overrides keyed by history subsection, so each keeps its zoom.
    pub grid_scales: HashMap<String, f64>,
    /// Preview size multiplier for the emoji browser.
    pub emoji_scale: f64,
    /// Whether the right-hand preview pane is expanded.
    pub detail_pane_open: bool,
    /// Width of the preview pane in CSS pixels, until the user drags its edge.
    pub detail_pane_width: u32,
    /// Interface scale in percent; 100 is the designed size.
    pub text_scale: u32,
    /// Ordering used by the history list: "updated-desc", "updated-asc",
    /// "created-desc" or "created-asc".
    pub sort_order: String,
    /// Saved queries that appear as smart views in the sidebar.
    pub saved_searches: Vec<SavedSearch>,
    /// Emoji pinned to the top of the emoji browser.
    pub emoji_favorites: Vec<String>,
    /// Optional personal GIPHY API key; the documented public key is used when empty.
    pub giphy_api_key: String,
    /// GIFs pinned to the top of the GIF utility.
    pub gif_favorites: Vec<GifFavorite>,
    /// GIF preview size multiplier, mirroring the card-size control.
    pub gif_scale: f64,
    /// Keep GIF previews looping; off plays them only while hovered.
    pub gif_loop: bool,
    /// Folder one-click Markdown exports go to. Empty asks for one each time.
    pub markdown_export_dir: String,
    /// Precede each exported note with YAML front matter.
    pub markdown_front_matter: bool,
    /// Prefix note filenames with the capture date.
    pub markdown_date_prefix: bool,
    /// How many archives the sidebar lists before offering "Show all".
    pub sidebar_list_limit: u32,
    /// Whether the left-hand filter sidebar is showing.
    pub sidebar_open: bool,
    /// Whether the sidebar's text payload sub-types are expanded.
    pub text_expanded: bool,
    /// Whether the sidebar's image sub-types are expanded.
    pub images_expanded: bool,
    /// Sidebar sections the user collapsed. History is never collapsible.
    pub sidebar_collapsed: Vec<String>,
    /// Sidebar section ids in the order the user dragged them into; empty keeps
    /// the built-in order.
    pub sidebar_section_order: Vec<String>,
    /// Preview-pane section ids in the user's order, as above.
    pub detail_section_order: Vec<String>,
    /// Per-archive content and note visibility.
    pub archive_display: Vec<ArchiveDisplay>,
    /// Play a short tick when a copy succeeds.
    pub copy_sound: bool,
    /// WAV file for a successful copy; empty plays the bundled tick.
    pub copy_sound_file: String,
    /// Play a lower tone when a copy fails.
    pub copy_fail_sound: bool,
    /// WAV file for a failed copy; empty plays the bundled failure sound.
    pub copy_fail_sound_file: String,
    /// Flag clips that look like they hold a credential.
    pub flag_secrets: bool,
    /// Also flag keyword-and-entropy matches; slightly noisier, so opt-in.
    pub secret_heuristics: bool,
    /// Leave secrets out of history entirely instead of flagging them.
    pub skip_secret_capture: bool,
    /// Palette choice and, for custom mode, the colors themselves.
    pub theme: Theme,
    /// Check GitHub for a newer release; off disables the schedule entirely.
    pub auto_update: bool,
    /// Unix milliseconds of the last check, so the daily schedule survives a
    /// restart instead of hammering the API every launch.
    pub last_update_check: i64,
    /// Set once the first-run wizard has been finished or deliberately skipped.
    /// Existing installations are backfilled to `true` by the store migration.
    pub setup_complete: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            hotkey: DEFAULT_HOTKEY.to_string(),
            pause_hotkey: "Ctrl+Alt+P".to_string(),
            paste_rich_text: true,
            auto_clear_clipboard: false,
            auto_clear_delay_secs: 30,
            auto_paste: false,
            close_on_double_click: true,
            paste_delay_ms: 120,
            launch_at_login: false,
            start_hidden: true,
            hide_on_blur: false,
            always_on_top: false,
            show_in_taskbar: true,
            capture_images: true,
            capture_files: true,
            capture_mode: "region".into(),
            prefer_images_over_text: false,
            ignore_sensitive: true,
            ignored_apps: Vec::new(),
            detect_faces: true,
            max_items: 2000,
            max_age_days: 0,
            max_age_hours: 0,
            max_text_bytes: 5 * 1024 * 1024,
            max_image_bytes: 32 * 1024 * 1024,
            poll_interval_ms: 350,
            view_modes: ViewModes::default(),
            grid_scale: 1.0,
            grid_scales: HashMap::new(),
            emoji_scale: 1.0,
            detail_pane_open: true,
            detail_pane_width: 286,
            text_scale: 100,
            sort_order: "updated-desc".into(),
            saved_searches: Vec::new(),
            emoji_favorites: Vec::new(),
            giphy_api_key: String::new(),
            gif_favorites: Vec::new(),
            gif_scale: 1.0,
            gif_loop: true,
            markdown_export_dir: String::new(),
            markdown_front_matter: true,
            markdown_date_prefix: true,
            sidebar_list_limit: 6,
            sidebar_open: true,
            text_expanded: false,
            images_expanded: false,
            sidebar_collapsed: vec![
                "collections".into(),
                "archives".into(),
                "tags".into(),
                "utilities".into(),
            ],
            sidebar_section_order: Vec::new(),
            detail_section_order: Vec::new(),
            archive_display: Vec::new(),
            copy_sound: true,
            copy_sound_file: String::new(),
            copy_fail_sound: true,
            copy_fail_sound_file: String::new(),
            flag_secrets: true,
            secret_heuristics: false,
            skip_secret_capture: false,
            theme: Theme::default(),
            auto_update: true,
            last_update_check: 0,
            setup_complete: false,
        }
    }
}

impl Settings {
    /// Clamps values into ranges the rest of the app can rely on. Applied after
    /// every load and patch so that hand-edited or stale rows cannot produce an
    /// infinite poll interval or a zero-byte text cap.
    pub fn sanitized(mut self) -> Self {
        self.poll_interval_ms = self.poll_interval_ms.clamp(120, 5_000);
        self.paste_delay_ms = self.paste_delay_ms.min(2_000);
        self.max_items = self.max_items.min(100_000);
        self.max_age_days = self.max_age_days.min(3_650);
        self.max_age_hours = self.max_age_hours.min(87_600);
        self.auto_clear_delay_secs = self.auto_clear_delay_secs.min(3_600);
        // Too narrow to read, too wide to leave the list any room.
        self.detail_pane_width = self.detail_pane_width.clamp(200, 1_600);
        self.last_update_check = self.last_update_check.max(0);
        self.max_text_bytes = self.max_text_bytes.clamp(1024, 64 * 1024 * 1024);
        self.max_image_bytes = self.max_image_bytes.clamp(1024, 512 * 1024 * 1024);
        // A hand-edited or stale row could deserialize as NaN, and `clamp` would
        // propagate NaN into the layout, so screen it out first.
        if !self.grid_scale.is_finite() {
            self.grid_scale = 1.0;
        }
        self.grid_scale = self.grid_scale.clamp(0.6, 2.0);
        for value in self.grid_scales.values_mut() {
            if !value.is_finite() {
                *value = 1.0;
            }
            *value = value.clamp(0.6, 2.0);
        }
        if !self.emoji_scale.is_finite() {
            self.emoji_scale = 1.0;
        }
        self.emoji_scale = self.emoji_scale.clamp(0.6, 2.0);
        if !self.gif_scale.is_finite() {
            self.gif_scale = 1.0;
        }
        self.gif_scale = self.gif_scale.clamp(0.6, 2.0);
        self.sidebar_list_limit = self.sidebar_list_limit.clamp(3, 50);
        self.text_scale = self.text_scale.clamp(70, 200);
        self.theme = self.theme.sanitized();
        if !matches!(
            self.sort_order.as_str(),
            "updated-desc" | "updated-asc" | "created-desc" | "created-asc"
        ) {
            self.sort_order = "updated-desc".into();
        }
        if !matches!(
            self.capture_mode.as_str(),
            "region" | "window" | "screen" | "scrolling"
        ) {
            self.capture_mode = "region".into();
        }
        self.view_modes = self.view_modes.sanitized();

        self.hotkey = self.hotkey.trim().to_string();
        if self.hotkey.is_empty() {
            self.hotkey = DEFAULT_HOTKEY.to_string();
        }
        self.pause_hotkey = self.pause_hotkey.trim().to_string();
        if self.pause_hotkey.is_empty() {
            self.pause_hotkey = "Ctrl+Alt+P".to_string();
        }
        self.saved_searches = {
            let mut seen: Vec<String> = Vec::new();
            let mut cleaned: Vec<SavedSearch> = Vec::new();
            for search in std::mem::take(&mut self.saved_searches) {
                let name = search.name.trim().to_string();
                let query = search.query.trim().to_string();
                if name.is_empty() || query.is_empty() {
                    continue;
                }
                if seen.iter().any(|existing| existing.eq_ignore_ascii_case(&name)) {
                    continue;
                }
                seen.push(name.clone());
                cleaned.push(SavedSearch { name, query });
                if cleaned.len() >= 25 {
                    break;
                }
            }
            cleaned
        };
        self.emoji_favorites = {
            let mut cleaned: Vec<String> = Vec::new();
            for emoji in std::mem::take(&mut self.emoji_favorites) {
                let emoji = emoji.trim().to_string();
                if emoji.is_empty() || cleaned.contains(&emoji) {
                    continue;
                }
                cleaned.push(emoji);
                if cleaned.len() >= 200 {
                    break;
                }
            }
            cleaned
        };
        self.giphy_api_key = self.giphy_api_key.trim().chars().take(200).collect();
        // An empty folder is meaningful: it asks for one at export time, so a
        // cleared value must survive sanitising as empty.
        self.markdown_export_dir = self
            .markdown_export_dir
            .trim()
            .chars()
            .take(1_000)
            .collect();
        self.copy_sound_file = self.copy_sound_file.trim().chars().take(500).collect();
        self.copy_fail_sound_file = self
            .copy_fail_sound_file
            .trim()
            .chars()
            .take(500)
            .collect();
        self.gif_favorites = {
            let mut cleaned: Vec<GifFavorite> = Vec::new();
            for favorite in std::mem::take(&mut self.gif_favorites) {
                let clip_id = favorite.clip_id.filter(|id| *id > 0);
                let title = favorite.title.trim().chars().take(160).collect::<String>();
                let mut id = favorite.id.trim().to_string();
                if id.is_empty() {
                    id = match clip_id {
                        Some(clip_id) => format!("clip:{clip_id}"),
                        None => continue,
                    };
                }
                if cleaned
                    .iter()
                    .any(|existing| existing.id.eq_ignore_ascii_case(&id))
                {
                    continue;
                }
                let (url, preview_url, still_url) = match clip_id {
                    Some(_) => (String::new(), String::new(), String::new()),
                    None if id.starts_with("local:") => {
                        // Imported GIF files live in the media folder, so their
                        // preview is a bare file name rather than a remote URL.
                        let file = favorite.preview_url.trim().to_string();
                        if !is_custom_gif_name(&file) {
                            continue;
                        }
                        (String::new(), file, String::new())
                    }
                    None => {
                        let url = favorite.url.trim().to_string();
                        if crate::giphy::validate_url(&url).is_err() {
                            continue;
                        }
                        let preview = favorite.preview_url.trim();
                        let preview = if crate::giphy::validate_url(preview).is_ok() {
                            preview.to_string()
                        } else {
                            url.clone()
                        };
                        let still = favorite.still_url.trim();
                        let still = if crate::giphy::validate_url(still).is_ok() {
                            still.to_string()
                        } else {
                            preview.clone()
                        };
                        (url, preview, still)
                    }
                };
                cleaned.push(GifFavorite {
                    id,
                    title,
                    url,
                    preview_url,
                    still_url,
                    clip_id,
                });
                if cleaned.len() >= 100 {
                    break;
                }
            }
            cleaned
        };
        self.ignored_apps = self
            .ignored_apps
            .iter()
            .map(|a| a.trim().to_ascii_lowercase())
            .filter(|a| !a.is_empty())
            .collect();
        self.ignored_apps.dedup();

        // Only known sections can be collapsed, and History is always visible.
        let known = ["collections", "archives", "tags", "utilities", "saved", "text"];
        self.sidebar_collapsed = self
            .sidebar_collapsed
            .iter()
            .map(|name| name.trim().to_ascii_lowercase())
            .filter(|name| known.contains(&name.as_str()))
            .fold(Vec::new(), |mut kept: Vec<String>, name| {
                if !kept.contains(&name) {
                    kept.push(name);
                }
                kept
            });

        self.archive_display = {
            let mut cleaned: Vec<ArchiveDisplay> = Vec::new();
            for entry in std::mem::take(&mut self.archive_display) {
                if entry.archive_id <= 0
                    || cleaned.iter().any(|existing| existing.archive_id == entry.archive_id)
                {
                    continue;
                }
                cleaned.push(entry);
                if cleaned.len() >= 200 {
                    break;
                }
            }
            cleaned
        };
        self
    }
}

pub fn load(conn: &Connection) -> Result<Settings> {
    let defaults = serde_json::to_value(Settings::default())?;
    let mut merged = match defaults {
        serde_json::Value::Object(map) => map,
        _ => {
            return Err(AppError::msg(
                "settings defaults must serialize to an object",
            ));
        }
    };

    let mut stmt = conn.prepare("SELECT key, value FROM settings")?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
    })?;
    for row in rows {
        let (key, raw) = row?;
        // Unknown keys are ignored rather than fatal: a config written by a newer
        // build should not brick an older one.
        if !merged.contains_key(&key) {
            continue;
        }
        match serde_json::from_str::<serde_json::Value>(&raw) {
            Ok(value) => {
                merged.insert(key, value);
            }
            Err(e) => log::warn!("ignoring malformed setting `{key}`: {e}"),
        }
    }

    let settings: Settings = serde_json::from_value(serde_json::Value::Object(merged))?;
    Ok(settings.sanitized())
}

pub fn save(conn: &Connection, settings: &Settings) -> Result<()> {
    let value = serde_json::to_value(settings)?;
    let map = match value {
        serde_json::Value::Object(map) => map,
        _ => return Err(AppError::msg("settings must serialize to an object")),
    };
    let mut stmt = conn.prepare(
        "INSERT INTO settings (key, value) VALUES (?1, ?2)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )?;
    for (key, value) in map {
        stmt.execute(params![key, value.to_string()])?;
    }
    Ok(())
}

/// Applies a partial update and returns the full, clamped result.
pub fn apply_patch(conn: &Connection, patch: &serde_json::Value) -> Result<Settings> {
    let mut current = serde_json::to_value(load(conn)?)?;
    let patch = patch
        .as_object()
        .ok_or_else(|| AppError::msg("settings patch must be a JSON object"))?;
    let target = current
        .as_object_mut()
        .ok_or_else(|| AppError::msg("settings must serialize to an object"))?;

    for (key, value) in patch {
        // Silently dropping unknown keys keeps the API forward-compatible.
        if target.contains_key(key) {
            target.insert(key.clone(), value.clone());
        } else {
            log::warn!("ignoring unknown setting `{key}`");
        }
    }

    let updated: Settings = serde_json::from_value(current)?;
    let updated = updated.sanitized();
    save(conn, &updated)?;
    Ok(updated)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store;

    fn memory_store() -> store::Store {
        store::Store::open_in_memory().unwrap()
    }

    #[test]
    fn appearance_defaults_and_clamping() {
        let store = memory_store();
        let defaults = store.settings();
        assert!(defaults.show_in_taskbar, "the taskbar button is on by default");
        assert_eq!(defaults.view_modes.images, "grid", "images default to cards");
        assert_eq!(defaults.view_modes.text, "list");
        assert_eq!(defaults.view_modes.all, "list");
        assert_eq!(defaults.text_scale, 100);
        assert!((defaults.grid_scale - 1.0).abs() < f64::EPSILON);
        assert!(defaults.grid_scales.is_empty());
        assert!((defaults.emoji_scale - 1.0).abs() < f64::EPSILON);
        assert!(defaults.close_on_double_click, "double-click still dismisses by default");

        let clamped = store
            .update_settings(&serde_json::json!({
                "textScale": 900,
                "gridScale": 99.0,
                "gridScales": { "images": 0.1, "text": 9.0 },
                "emojiScale": 0.1,
                "viewModes": { "text": "nonsense", "images": "grid", "other": "grid" }
            }))
            .unwrap();
        assert_eq!(clamped.text_scale, 200, "text scale is capped");
        assert!((clamped.grid_scale - 2.0).abs() < f64::EPSILON, "card scale is capped");
        assert!((clamped.grid_scales["images"] - 0.6).abs() < f64::EPSILON);
        assert!((clamped.grid_scales["text"] - 2.0).abs() < f64::EPSILON);
        assert!((clamped.emoji_scale - 0.6).abs() < f64::EPSILON);
        assert_eq!(
            clamped.view_modes.text, "list",
            "an unknown view falls back to list"
        );
        assert_eq!(clamped.view_modes.images, "grid");
        assert_eq!(clamped.view_modes.other, "grid");
    }

    #[test]
    fn defaults_are_returned_when_nothing_is_persisted() {
        let store = memory_store();
        let settings = store.settings();
        assert_eq!(settings.hotkey, DEFAULT_HOTKEY);
        assert!(!settings.auto_paste, "auto-paste defaults off");
        assert!(settings.start_hidden);
        assert!(
            !settings.always_on_top,
            "the window is not pinned on top by default"
        );
        assert_eq!(settings.max_age_hours, 0, "hourly retention is off by default");
        assert_eq!(settings.theme.mode, "dark");
        assert_eq!(settings.theme.colors, ThemeColors::dark());
        assert!(
            !settings.setup_complete,
            "a fresh install has not been through the wizard"
        );
    }

    #[test]
    fn capture_mode_defaults_to_region_and_rejects_unknown_values() {
        let store = memory_store();
        assert_eq!(
            store.settings().capture_mode,
            "region",
            "a fresh install captures a region"
        );

        let updated = store
            .update_settings(&serde_json::json!({ "captureMode": "window" }))
            .unwrap();
        assert_eq!(updated.capture_mode, "window");
        assert_eq!(store.settings().capture_mode, "window");

        let reset = store
            .update_settings(&serde_json::json!({ "captureMode": "nonsense" }))
            .unwrap();
        assert_eq!(
            reset.capture_mode, "region",
            "an unknown mode falls back to region"
        );
    }

    #[test]
    fn setup_completion_persists() {
        let store = memory_store();
        let updated = store
            .update_settings(&serde_json::json!({ "setupComplete": true }))
            .unwrap();
        assert!(updated.setup_complete);
        assert!(store.settings().setup_complete);
    }

    #[test]
    fn theme_mode_and_colors_are_sanitized() {
        let store = memory_store();
        let updated = store
            .update_settings(&serde_json::json!({
                "theme": {
                    "mode": "neon",
                    "colors": { "canvas": "not-a-color", "accent": "#ABC123" }
                }
            }))
            .unwrap();
        assert_eq!(updated.theme.mode, "dark", "unknown modes fall back to dark");
        assert_eq!(
            updated.theme.colors.canvas,
            ThemeColors::dark().canvas,
            "an unparseable color keeps the shipped value"
        );
        assert_eq!(
            updated.theme.colors.accent, "#abc123",
            "hex values are lower-cased"
        );

        let light = store
            .update_settings(&serde_json::json!({ "theme": { "mode": "light" } }))
            .unwrap();
        assert_eq!(light.theme.mode, "light");
    }

    #[test]
    fn hourly_retention_is_clamped() {
        let store = memory_store();
        let updated = store
            .update_settings(&serde_json::json!({ "maxAgeHours": 999_999 }))
            .unwrap();
        assert_eq!(updated.max_age_hours, 87_600);
    }

    #[test]
    fn patch_round_trips_and_reports_full_settings() {
        let store = memory_store();
        let updated = store
            .update_settings(&serde_json::json!({ "autoPaste": true, "maxItems": 42 }))
            .unwrap();
        assert!(updated.auto_paste);
        assert_eq!(updated.max_items, 42);
        // Untouched fields must keep their defaults.
        assert_eq!(updated.hotkey, DEFAULT_HOTKEY);

        let reloaded = store.settings();
        assert!(reloaded.auto_paste);
        assert_eq!(reloaded.max_items, 42);
    }

    #[test]
    fn out_of_range_values_are_clamped() {
        let store = memory_store();
        let updated = store
            .update_settings(&serde_json::json!({ "pollIntervalMs": 5, "pasteDelayMs": 99_999 }))
            .unwrap();
        assert_eq!(updated.poll_interval_ms, 120);
        assert_eq!(updated.paste_delay_ms, 2_000);
    }

    #[test]
    fn wrong_type_is_rejected_without_corrupting_state() {
        let store = memory_store();
        assert!(
            store
                .update_settings(&serde_json::json!({ "autoPaste": "yes please" }))
                .is_err()
        );
        assert!(!store.settings().auto_paste);
    }

    #[test]
    fn empty_hotkey_falls_back_to_default() {
        let store = memory_store();
        let updated = store
            .update_settings(&serde_json::json!({ "hotkey": "   " }))
            .unwrap();
        assert_eq!(updated.hotkey, DEFAULT_HOTKEY);
    }

    #[test]
    fn ignored_apps_are_normalised_and_deduplicated() {
        let store = memory_store();
        let updated = store
            .update_settings(
                &serde_json::json!({ "ignoredApps": [" KeePassXC.EXE ", "keepassxc.exe", ""] }),
            )
            .unwrap();
        assert_eq!(updated.ignored_apps, vec!["keepassxc.exe"]);
    }

    #[test]
    fn private_mode_and_auto_clear_defaults_are_sane() {
        let store = memory_store();
        let defaults = store.settings();
        assert_eq!(defaults.pause_hotkey, "Ctrl+Alt+P");
        assert!(
            !defaults.auto_clear_clipboard,
            "the clipboard is not emptied unless asked"
        );
        assert_eq!(defaults.auto_clear_delay_secs, 30);
        assert!(defaults.paste_rich_text, "rich text round-trips by default");

        let clamped = store
            .update_settings(&serde_json::json!({
                "pauseHotkey": "  ",
                "autoClearDelaySecs": 99_999
            }))
            .unwrap();
        assert_eq!(clamped.pause_hotkey, "Ctrl+Alt+P", "an empty pause key resets");
        assert_eq!(clamped.auto_clear_delay_secs, 3_600, "the delay is capped");
    }

    #[test]
    fn preview_pane_width_and_emoji_favourites_are_sanitised() {
        let store = memory_store();
        assert_eq!(store.settings().detail_pane_width, 286);
        assert!(store.settings().emoji_favorites.is_empty());

        let clamped = store
            .update_settings(&serde_json::json!({
                "detailPaneWidth": 40,
                "emojiFavorites": [" 🎉 ", "🎉", "", "🚀"]
            }))
            .unwrap();
        assert_eq!(clamped.detail_pane_width, 200, "too narrow to read");
        assert_eq!(clamped.emoji_favorites, vec!["🎉".to_string(), "🚀".to_string()]);

        let wide = store
            .update_settings(&serde_json::json!({ "detailPaneWidth": 9_999 }))
            .unwrap();
        assert_eq!(wide.detail_pane_width, 1_600);
    }

    #[test]
    fn sidebar_and_archive_display_settings_are_sanitised() {
        let store = memory_store();
        assert!(
            store.settings().sidebar_collapsed.contains(&"collections".to_string()),
            "sidebar lists start folded so the history has room"
        );
        assert!(store.settings().archive_display.is_empty());

        let updated = store
            .update_settings(&serde_json::json!({
                "sidebarCollapsed": ["Collections", " tags ", "history", "collections", "", "bogus"],
                "archiveDisplay": [
                    {
                        "archiveId": 4,
                        "listContent": false,
                        "listNotes": true,
                        "cardContent": true,
                        "cardNotes": false
                    },
                    { "archiveId": 4, "listContent": true },
                    { "archiveId": 0, "listContent": false }
                ]
            }))
            .unwrap();

        assert_eq!(
            updated.sidebar_collapsed,
            vec!["collections".to_string(), "tags".to_string()],
            "unknown, History and duplicate names are dropped"
        );
        assert_eq!(updated.archive_display.len(), 1, "duplicate and empty ids are dropped");
        let entry = updated.archive_display[0];
        assert_eq!(entry.archive_id, 4);
        assert!(!entry.list_content);
        assert!(entry.list_notes);
        assert!(entry.card_content);
        assert!(!entry.card_notes);

        // Defaults fill any field the stored entry omits.
        store
            .update_settings(&serde_json::json!({
                "archiveDisplay": [{ "archiveId": 9 }]
            }))
            .unwrap();
        let relaxed = store.settings().archive_display;
        assert!(relaxed[0].list_content && relaxed[0].card_notes);
    }

    #[test]
    fn gif_settings_are_clamped_and_sanitised() {
        let store = memory_store();
        assert_eq!(store.settings().giphy_api_key, "");
        assert!(store.settings().gif_favorites.is_empty());
        assert!((store.settings().gif_scale - 1.0).abs() < f64::EPSILON);
        assert!(store.settings().gif_loop, "GIFs loop by default");

        let updated = store
            .update_settings(&serde_json::json!({
                "giphyApiKey": "  personal-key  ",
                "gifScale": 9.0,
                "gifLoop": false,
                "gifFavorites": [
                    {
                        "id": "giphy:abc",
                        "title": "  Dancing cat  ",
                        "url": "https://media.giphy.com/media/abc/giphy.gif",
                        "previewUrl": "https://media.giphy.com/media/abc/200w.gif",
                        "stillUrl": "https://media.giphy.com/media/abc/200w_s.gif",
                        "clipId": null
                    },
                    {
                        "id": "giphy:bad",
                        "title": "Not GIPHY",
                        "url": "https://example.com/a.gif",
                        "previewUrl": "",
                        "clipId": null
                    },
                    { "id": "clip:7", "title": "Image", "clipId": 7 },
                    { "id": "clip:7", "title": "Duplicate", "clipId": 7 },
                    {
                        "id": "local:custom-abc.gif",
                        "title": "  My GIF  ",
                        "url": "",
                        "previewUrl": "custom-abc.gif",
                        "clipId": null
                    },
                    {
                        "id": "local:evil",
                        "title": "Traversal",
                        "url": "",
                        "previewUrl": "../secret.gif",
                        "clipId": null
                    }
                ]
            }))
            .unwrap();

        assert_eq!(updated.giphy_api_key, "personal-key");
        assert!((updated.gif_scale - 2.0).abs() < f64::EPSILON);
        assert!(!updated.gif_loop, "the hover-to-play choice sticks");
        assert_eq!(
            updated.gif_favorites[0].still_url,
            "https://media.giphy.com/media/abc/200w_s.gif"
        );
        assert_eq!(
            updated.gif_favorites.len(),
            3,
            "unsafe URLs, traversal names and duplicates are dropped"
        );
        assert_eq!(updated.gif_favorites[0].title, "Dancing cat");
        assert_eq!(updated.gif_favorites[1].id, "clip:7");
        assert_eq!(updated.gif_favorites[1].clip_id, Some(7));
        assert!(updated.gif_favorites[1].url.is_empty());
        assert_eq!(updated.gif_favorites[2].id, "local:custom-abc.gif");
        assert_eq!(updated.gif_favorites[2].preview_url, "custom-abc.gif");
    }

    #[test]
    fn markdown_export_settings_are_sanitised() {
        let store = memory_store();
        let defaults = store.settings();
        assert_eq!(
            defaults.markdown_export_dir, "",
            "a fresh install asks for a folder each time"
        );
        assert!(
            defaults.markdown_front_matter,
            "front matter is on by default"
        );
        assert!(
            defaults.markdown_date_prefix,
            "dates prefix names by default"
        );

        let updated = store
            .update_settings(&serde_json::json!({
                "markdownExportDir": "  C:\\Users\\me\\Vault  ",
                "markdownFrontMatter": false
            }))
            .unwrap();
        assert_eq!(updated.markdown_export_dir, "C:\\Users\\me\\Vault");
        assert!(!updated.markdown_front_matter);
        assert!(
            updated.markdown_date_prefix,
            "an untouched field keeps its default"
        );

        let cleared = store
            .update_settings(&serde_json::json!({ "markdownExportDir": "   " }))
            .unwrap();
        assert_eq!(
            cleared.markdown_export_dir, "",
            "clearing the folder restores ask-each-time"
        );
    }

    #[test]
    fn saved_searches_are_trimmed_deduplicated_and_capped() {
        let store = memory_store();
        let updated = store
            .update_settings(&serde_json::json!({
                "savedSearches": [
                    { "name": "  Links  ", "query": "  kind:url  " },
                    { "name": "links", "query": "kind:image" },
                    { "name": "  ", "query": "kind:text" },
                    { "name": "Empty query", "query": "   " }
                ]
            }))
            .unwrap();
        assert_eq!(
            updated.saved_searches,
            vec![SavedSearch {
                name: "Links".into(),
                query: "kind:url".into(),
            }],
            "a duplicate name in different case is dropped, keeping the first"
        );
    }
}
