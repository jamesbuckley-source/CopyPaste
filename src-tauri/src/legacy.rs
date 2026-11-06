//! Carries a CopyPaste installation's data folder over to Kept's, once.
//!
//! The rename moved the app's data folder: Tauri derives it from the bundle
//! identifier, and Kept uses a new one. Everything the previous build left behind
//! lives in the old folder — most importantly `storage.json`, the pointer that says
//! where the history actually is. Without this, an upgrade onto a renamed app would
//! open an empty local database, and anyone who keeps their history on another drive
//! (a synced folder, say) would be looking at an empty history with their clips
//! still sitting where they left them.
//!
//! The old folder is copied rather than moved, so rolling back to the previous
//! release still works.

use std::path::Path;

/// The data folder the previous identity used, beside the current one.
const PREVIOUS_IDENTIFIER: &str = "com.copypaste.app";

/// Written into the new folder once the move has happened, so it happens once.
const MARKER: &str = "migrated-from-copypaste.txt";

/// Files that belong to a data folder and are worth carrying over.
///
/// The database keeps its old file name on purpose: it is what the history is
/// called on disk, and renaming it would be a second migration for no benefit.
/// The `-wal` and `-shm` files are copied with it so a database that was closed
/// mid-write still opens cleanly.
const FILES: [&str; 5] = [
    "storage.json",
    "copypaste.db",
    "copypaste.db-wal",
    "copypaste.db-shm",
    "key.bin",
];

/// Folders that only hold data when the history lived in this default location.
const FOLDERS: [&str; 2] = ["media", "archives"];

/// Copies the previous installation's data folder into `new_dir`.
///
/// Silent on a fresh install — there is nothing to find — and safe to call on every
/// start: the marker makes it a no-op after the first time.
pub fn migrate_from_copypaste(new_dir: &Path) {
    let Some(parent) = new_dir.parent() else {
        return;
    };
    let old_dir = parent.join(PREVIOUS_IDENTIFIER);
    if !old_dir.is_dir() || new_dir.join(MARKER).exists() {
        return;
    }

    if let Err(e) = std::fs::create_dir_all(new_dir) {
        log::warn!(
            "could not create {} to migrate the old CopyPaste data folder: {e}",
            new_dir.display()
        );
        return;
    }

    let mut carried = 0;
    for name in FILES {
        let from = old_dir.join(name);
        let to = new_dir.join(name);
        if !from.is_file() || to.exists() {
            continue;
        }
        match std::fs::copy(&from, &to) {
            Ok(_) => {
                log::info!("migrated {name} from the old CopyPaste data folder");
                carried += 1;
            }
            Err(e) => log::warn!("could not migrate {name}: {e}"),
        }
    }

    // Only when the database came across too: if the history lived in a custom
    // folder, the folders here are leftovers from an older arrangement and copying
    // them would duplicate media for nothing.
    if new_dir.join("copypaste.db").exists() {
        for name in FOLDERS {
            let from = old_dir.join(name);
            if from.is_dir() && !new_dir.join(name).exists() {
                match copy_dir(&from, &new_dir.join(name)) {
                    Ok(count) => {
                        log::info!("migrated the {name} folder ({count} files) from CopyPaste");
                        carried += count;
                    }
                    Err(e) => log::warn!("could not migrate the {name} folder: {e}"),
                }
            }
        }
    }

    // A marker even when nothing was copied: the old folder may only hold an empty
    // shell, and looking again on every start costs a directory listing forever.
    if let Err(e) = std::fs::write(
        new_dir.join(MARKER),
        "Kept carried this installation over from CopyPaste.\n",
    ) {
        log::warn!("could not write the migration marker: {e}");
    }
    log::info!(
        "carried {carried} item(s) over from {} into {}",
        old_dir.display(),
        new_dir.display()
    );
}

/// Copies a directory tree, returning how many files were written.
fn copy_dir(from: &Path, to: &Path) -> std::io::Result<usize> {
    std::fs::create_dir_all(to)?;
    let mut copied = 0;
    for entry in std::fs::read_dir(from)?.flatten() {
        let source = entry.path();
        let target = to.join(entry.file_name());
        if source.is_dir() {
            copied += copy_dir(&source, &target)?;
        } else if !target.exists() {
            std::fs::copy(&source, &target)?;
            copied += 1;
        }
    }
    Ok(copied)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn the_pointer_and_the_database_come_across() {
        let root = scratch();
        let old = root.path().join(PREVIOUS_IDENTIFIER);
        std::fs::create_dir_all(old.join("media")).unwrap();
        std::fs::write(old.join("storage.json"), r#"{"dataDir":"G:\\kept"}"#).unwrap();
        std::fs::write(old.join("copypaste.db"), b"history").unwrap();
        std::fs::write(old.join("media").join("1.png"), b"png").unwrap();

        let new = root.path().join("ikept.it");
        migrate_from_copypaste(&new);

        assert_eq!(
            std::fs::read_to_string(new.join("storage.json")).unwrap(),
            r#"{"dataDir":"G:\\kept"}"#,
            "the pointer survives verbatim, so a custom history folder keeps working"
        );
        assert_eq!(std::fs::read(new.join("copypaste.db")).unwrap(), b"history");
        assert_eq!(std::fs::read(new.join("media").join("1.png")).unwrap(), b"png");
        assert!(new.join(MARKER).exists());
    }

    #[test]
    fn a_custom_history_folder_does_not_drag_stale_media_across() {
        let root = scratch();
        let old = root.path().join(PREVIOUS_IDENTIFIER);
        std::fs::create_dir_all(old.join("media")).unwrap();
        std::fs::write(old.join("storage.json"), r#"{"dataDir":"G:\\kept"}"#).unwrap();
        std::fs::write(old.join("media").join("stale.png"), b"png").unwrap();

        let new = root.path().join("ikept.it");
        migrate_from_copypaste(&new);

        assert!(new.join("storage.json").exists());
        assert!(
            !new.join("media").exists(),
            "media is only copied with the database that belongs to it"
        );
    }

    #[test]
    fn it_runs_once_and_does_nothing_on_a_fresh_install() {
        let root = scratch();
        let new = root.path().join("ikept.it");
        // No old folder at all: a fresh install, untouched.
        migrate_from_copypaste(&new);
        assert!(!new.join(MARKER).exists());

        let old = root.path().join(PREVIOUS_IDENTIFIER);
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("storage.json"), b"first").unwrap();
        migrate_from_copypaste(&new);

        // A later start must not overwrite what the new folder has already written.
        std::fs::write(new.join("storage.json"), b"second").unwrap();
        migrate_from_copypaste(&new);
        assert_eq!(
            std::fs::read(new.join("storage.json")).unwrap(),
            b"second",
            "the migration never overwrites the live data folder"
        );
    }
}
