//! Global shortcut registration.
//!
//! The handler itself lives in the plugin builder (see `lib.rs`); this module only
//! owns reconciling the binding with the user's configured hotkey.

use tauri::{AppHandle, Runtime};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut};

use crate::error::{AppError, Result};

/// Replaces the active bindings with `hotkey` and the private-mode `pause_hotkey`.
///
/// The old bindings are released first: `register` on an already-registered
/// accelerator fails, so rebinding has to clear the previous set or the second
/// attempt would leave the user with no working hotkey.
///
/// The summon shortcut is load-bearing, so failing to register it is an error. A
/// pause shortcut that the OS refuses (someone else owns it) only costs the
/// convenience of a keyboard toggle, so it degrades to a warning.
pub fn apply<R: Runtime>(app: &AppHandle<R>, hotkey: &str, pause_hotkey: &str) -> Result<Shortcut> {
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

    match pause_hotkey.parse::<Shortcut>() {
        Ok(pause) => match manager.register(pause) {
            Ok(()) => log::info!("pause shortcut registered: {pause_hotkey}"),
            Err(e) => log::warn!("`{pause_hotkey}` could not be registered: {e}"),
        },
        Err(e) => log::warn!("`{pause_hotkey}` is not a valid shortcut: {e}"),
    }

    Ok(shortcut)
}

/// Releases every binding, used when the app shuts down.
pub fn release<R: Runtime>(app: &AppHandle<R>) {
    if let Err(e) = app.global_shortcut().unregister_all() {
        log::debug!("could not release global shortcuts: {e}");
    }
}
