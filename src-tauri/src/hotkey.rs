//! Global shortcut registration.
//!
//! The handler itself lives in the plugin builder (see `lib.rs`); this module only
//! owns reconciling the binding with the user's configured hotkey.

use tauri::{AppHandle, Runtime};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};

use crate::error::{AppError, Result};

/// Replaces the active binding with `hotkey`, e.g. `"Alt+V"`.
///
/// The old binding is released first: `register` on an already-registered
/// accelerator fails, so rebinding has to clear the previous set or the second
/// attempt would leave the user with no working hotkey.
pub fn apply<R: Runtime>(app: &AppHandle<R>, hotkey: &str) -> Result<Shortcut> {
    let shortcut: Shortcut = hotkey
        .parse()
        .map_err(|e| AppError::msg(format!("`{hotkey}` is not a valid shortcut: {e}")))?;

    let manager = app.global_shortcut();
    if let Err(e) = manager.unregister_all() {
        // Not fatal: the OS may simply have had nothing registered yet.
        log::debug!("no previous shortcut to release: {e}");
    }
    manager
        .register(shortcut)
        .map_err(|e| AppError::msg(format!("`{hotkey}` could not be registered: {e}")))?;

    log::info!("global shortcut registered: {hotkey}");
    Ok(shortcut)
}

/// Releases every binding, used when the app shuts down.
pub fn release<R: Runtime>(app: &AppHandle<R>) {
    if let Err(e) = app.global_shortcut().unregister_all() {
        log::debug!("could not release global shortcuts: {e}");
    }
}
