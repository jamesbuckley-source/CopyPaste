//! User-visible configuration, persisted one row per field.
//!
//! Storing fields individually (rather than a single JSON blob) means a partial
//! patch from the settings UI only rewrites the keys it touches, and a truncated
//! write can never lose the whole configuration.

use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

use crate::error::{AppError, Result};

pub const DEFAULT_HOTKEY: &str = "Alt+V";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    /// Global shortcut that summons the window.
    pub hotkey: String,
    /// Paste into the previously focused window after selecting a clip.
    pub auto_paste: bool,
    /// Grace period between hiding our window and sending Ctrl+V.
    pub paste_delay_ms: u32,
    pub launch_at_login: bool,
    /// Start with the window hidden; the tray icon and hotkey bring it up.
    pub start_hidden: bool,
    /// Hide the window when it loses focus. Off by default: the window is an
    /// inspection surface, and it vanishing when you click away to look something
    /// up is worse than the slight clutter of leaving it open.
    pub hide_on_blur: bool,    /// Keep a taskbar button. Off makes the app tray-only.
    pub show_in_taskbar: bool,
    pub capture_images: bool,
    pub capture_files: bool,
    /// When a copy exposes both text and a bitmap, prefer the bitmap.
    pub prefer_images_over_text: bool,
    /// Honour the opt-out formats password managers publish.
    pub ignore_sensitive: bool,
    /// Lower-cased process names whose copies are never recorded.
    pub ignored_apps: Vec<String>,
    /// Oldest items are pruned past this count. `0` means unlimited.
    pub max_items: u32,
    /// `0` means unlimited.
    pub max_age_days: u32,
    pub max_text_bytes: u64,
    pub max_image_bytes: u64,
    pub poll_interval_ms: u64,
    /// "list" or "grid".
    pub view_mode: String,
    /// Card size multiplier for the grid view.
    pub grid_scale: f64,
    /// Whether the right-hand preview pane is expanded.
    pub detail_pane_open: bool,
    /// Interface scale in percent; 100 is the designed size.
    pub text_scale: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            hotkey: DEFAULT_HOTKEY.to_string(),
            auto_paste: false,
            paste_delay_ms: 120,
            launch_at_login: false,
            start_hidden: true,
            hide_on_blur: false,
            show_in_taskbar: true,
            capture_images: true,
            capture_files: true,
            prefer_images_over_text: false,
            ignore_sensitive: true,
            ignored_apps: Vec::new(),
            max_items: 2000,
            max_age_days: 0,
            max_text_bytes: 5 * 1024 * 1024,
            max_image_bytes: 32 * 1024 * 1024,
            poll_interval_ms: 350,
            view_mode: "list".to_string(),
            grid_scale: 1.0,
            detail_pane_open: true,
            text_scale: 100,
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
        self.max_text_bytes = self.max_text_bytes.clamp(1024, 64 * 1024 * 1024);
        self.max_image_bytes = self.max_image_bytes.clamp(1024, 512 * 1024 * 1024);
        // A hand-edited or stale row could deserialize as NaN, and `clamp` would
        // propagate NaN into the layout, so screen it out first.
        if !self.grid_scale.is_finite() {
            self.grid_scale = 1.0;
        }
        self.grid_scale = self.grid_scale.clamp(0.6, 2.0);
        self.text_scale = self.text_scale.clamp(70, 200);
        if self.view_mode != "grid" {
            self.view_mode = "list".to_string();
        }

        self.hotkey = self.hotkey.trim().to_string();
        if self.hotkey.is_empty() {
            self.hotkey = DEFAULT_HOTKEY.to_string();
        }
        self.ignored_apps = self
            .ignored_apps
            .iter()
            .map(|a| a.trim().to_ascii_lowercase())
            .filter(|a| !a.is_empty())
            .collect();
        self.ignored_apps.dedup();
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
        assert_eq!(defaults.view_mode, "list");
        assert_eq!(defaults.text_scale, 100);
        assert!((defaults.grid_scale - 1.0).abs() < f64::EPSILON);

        let clamped = store
            .update_settings(&serde_json::json!({
                "textScale": 900,
                "gridScale": 99.0,
                "viewMode": "nonsense"
            }))
            .unwrap();
        assert_eq!(clamped.text_scale, 200, "text scale is capped");
        assert!((clamped.grid_scale - 2.0).abs() < f64::EPSILON, "card scale is capped");
        assert_eq!(clamped.view_mode, "list", "an unknown view falls back to list");
    }

    #[test]
    fn defaults_are_returned_when_nothing_is_persisted() {
        let store = memory_store();
        let settings = store.settings();
        assert_eq!(settings.hotkey, DEFAULT_HOTKEY);
        assert!(!settings.auto_paste, "auto-paste defaults off");
        assert!(settings.start_hidden);
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
    fn ignored_apps_are_normalised_and_deduplicated() {        let store = memory_store();
        let updated = store
            .update_settings(
                &serde_json::json!({ "ignoredApps": [" KeePassXC.EXE ", "keepassxc.exe", ""] }),
            )
            .unwrap();
        assert_eq!(updated.ignored_apps, vec!["keepassxc.exe"]);
    }
}
