//! SQLite-backed clip storage.
//!
//! Layout notes:
//!
//! * Clip columns (`preview`, `body`, `files`, `ocr_text`, `html`, `rtf`, `note`)
//!   hold plain text. Encryption was removed; rows written by older builds are
//!   migrated to plain text once at startup using the legacy key file.
//! * Reads never touch SQLite on the hot path. Startup materialises a compact
//!   in-memory projection of every clip (metadata + normalised search text), and
//!   list/search/filter all run against that. Writes go to SQLite first, then the
//!   projection is refreshed. At clipboard-history scale this trades a few MB of
//!   RAM for search that responds within a keystroke.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use parking_lot::{Mutex, RwLock};
use rusqlite::{Connection, OptionalExtension, params};

use crate::crypto::{Cipher, load_master_key};
use crate::error::{AppError, Result};
use crate::hashing;
use crate::media::{self, ImageFiles};
use crate::models::{
    ClipContent, ClipDetail, ClipFilter, ClipKind, ClipPage, ClipRow, ClipSummary, Collection,
    NewClip, RichText, Stats, Tag,
};
use crate::search::{self, Query};
use crate::settings::{self, Settings};

const SCHEMA_VERSION: &str = "6";

/// Marks the one-time reset of `hideOnBlur` to its new default.
const FOCUS_HIDE_RESET_KEY: &str = "focus_hide_reset_v2";

/// Marks the one-time backfill of `setupComplete` for installations that predate
/// the first-run wizard.
const SETUP_BACKFILL_KEY: &str = "setup_backfill_v1";

/// Clips larger than this are indexed by preview only, so that startup does not
/// have to decrypt an unbounded amount of text.
const SEARCH_BODY_LIMIT: i64 = 256 * 1024;

/// Upper bound for a clip note. Notes are annotations, not payloads.
pub const MAX_NOTE_CHARS: usize = 4_000;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS meta (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS clips (
  id             INTEGER PRIMARY KEY AUTOINCREMENT,
  kind           TEXT    NOT NULL,
  content_hash   TEXT    NOT NULL,
  byte_size      INTEGER NOT NULL DEFAULT 0,
  char_count     INTEGER NOT NULL DEFAULT 0,
  file_count     INTEGER NOT NULL DEFAULT 0,
  preview        BLOB,
  preview_nonce  BLOB,
  body           BLOB,
  body_nonce     BLOB,
  files          BLOB,
  files_nonce    BLOB,
  image_path     TEXT,
  thumb_path     TEXT,
  width          INTEGER,
  height         INTEGER,
  source_app     TEXT,
  pinned         INTEGER NOT NULL DEFAULT 0,
  created_at     INTEGER NOT NULL,
  updated_at     INTEGER NOT NULL,
  copy_count     INTEGER NOT NULL DEFAULT 1,
  ocr_text       BLOB,
  ocr_nonce      BLOB,
  ocr_state      INTEGER NOT NULL DEFAULT 0,
  html           BLOB,
  html_nonce     BLOB,
  rtf            BLOB,
  rtf_nonce      BLOB,
  note           BLOB,
  note_nonce     BLOB,
  secret_level   TEXT
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_clips_hash  ON clips(content_hash);
CREATE INDEX        IF NOT EXISTS idx_clips_order ON clips(pinned, updated_at DESC);
CREATE INDEX        IF NOT EXISTS idx_clips_kind  ON clips(kind);

CREATE TABLE IF NOT EXISTS collections (
  id         INTEGER PRIMARY KEY AUTOINCREMENT,
  name       TEXT    NOT NULL,
  color      TEXT,
  archived   INTEGER NOT NULL DEFAULT 0,
  created_at INTEGER NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_collections_name ON collections(name COLLATE NOCASE);

CREATE TABLE IF NOT EXISTS clip_collections (
  clip_id       INTEGER NOT NULL REFERENCES clips(id)       ON DELETE CASCADE,
  collection_id INTEGER NOT NULL REFERENCES collections(id) ON DELETE CASCADE,
  PRIMARY KEY (clip_id, collection_id)
);
CREATE INDEX IF NOT EXISTS idx_clip_collections_col ON clip_collections(collection_id, clip_id);

CREATE TABLE IF NOT EXISTS tags (
  id         INTEGER PRIMARY KEY AUTOINCREMENT,
  name       TEXT    NOT NULL,
  color      TEXT,
  created_at INTEGER NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_tags_name ON tags(name COLLATE NOCASE);

CREATE TABLE IF NOT EXISTS clip_tags (
  clip_id INTEGER NOT NULL REFERENCES clips(id) ON DELETE CASCADE,
  tag_id  INTEGER NOT NULL REFERENCES tags(id)  ON DELETE CASCADE,
  PRIMARY KEY (clip_id, tag_id)
);
CREATE INDEX IF NOT EXISTS idx_clip_tags_tag ON clip_tags(tag_id, clip_id);

CREATE TABLE IF NOT EXISTS settings (
  key   TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
"#;

// The hash column is intentionally absent: nothing that builds a `ClipRow` needs
// it, and de-duplication is resolved by its own targeted query.
/// Clips filed in an archive collection are invisible to every bulk operation.
const NOT_ARCHIVED: &str = "id NOT IN (SELECT clip_id FROM clip_collections \
     WHERE collection_id IN (SELECT id FROM collections WHERE archived = 1))";

const CLIP_COLUMNS: &str = "id, kind, byte_size, char_count, file_count, \
     preview, preview_nonce, body, body_nonce, files, files_nonce, \
     image_path, thumb_path, width, height, source_app, pinned, created_at, updated_at, copy_count, \
     ocr_text, ocr_nonce, html, html_nonce, rtf, rtf_nonce, note, note_nonce, secret_level";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InsertOutcome {
    Inserted(i64),
    /// Identical content was already stored; the existing row was promoted.
    Duplicate(i64),
}

/// The in-memory projection of one clip.
#[derive(Debug, Clone)]
struct IndexedClip {
    summary: ClipSummary,
    search: String,
}

#[derive(Debug, Default)]
struct State {
    settings: Settings,
    clips: HashMap<i64, IndexedClip>,
    /// Collections marked as archives. Their clips are hidden from the ordinary
    /// history views, cleared only through the collection itself.
    archived: HashSet<i64>,
}

pub struct Store {
    conn: Mutex<Connection>,
    state: RwLock<State>,
    media_dir: PathBuf,
    db_path: PathBuf,
    /// Holds the throwaway directory alive when the store is a test fixture.
    #[cfg(test)]
    fixture: Option<tempfile::TempDir>,
}

impl Store {
    pub fn open(data_dir: &Path) -> Result<Self> {
        std::fs::create_dir_all(data_dir)?;
        let media_dir = data_dir.join("media");
        std::fs::create_dir_all(&media_dir)?;

        let db_path = data_dir.join("copypaste.db");
        let key_path = data_dir.join("key.bin");
        // Encryption has been removed. A key file only exists on installations
        // created by an older build, where it is used once to rewrite the history
        // as plain text below.
        let legacy_cipher = if key_path.exists() {
            match load_master_key(&key_path) {
                Ok(key) => Some(Cipher::new(&key)),
                Err(e) => {
                    log::warn!("legacy key could not be read: {e}");
                    None
                }
            }
        } else {
            None
        };

        let conn = Connection::open(&db_path)?;
        // WAL keeps the capture thread from blocking the UI thread on reads.
        let _mode: String = conn.query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))?;
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA synchronous = NORMAL;")?;
        migrate(&conn)?;
        migrate_encrypted_rows(&conn, legacy_cipher.as_ref(), &key_path)?;

        let loaded = settings::load(&conn)?;
        let store = Store {
            conn: Mutex::new(conn),
            state: RwLock::new(State {
                settings: loaded,
                clips: HashMap::new(),
                archived: HashSet::new(),
            }),
            media_dir,
            db_path,
            #[cfg(test)]
            fixture: None,
        };
        store.rebuild_index()?;
        Ok(store)
    }

    pub fn media_dir(&self) -> &Path {
        &self.media_dir
    }

    /// Path of the recovered animation for an image clip, when one exists.
    pub fn animated_gif_path(&self, id: i64) -> Option<PathBuf> {
        let path = self.media_dir.join(format!("{id}.gif"));
        path.is_file().then_some(path)
    }

    /// Stores the recovered animation next to the clip's still frame and folds
    /// the new state into the projection so the next copy publishes it as a file.
    pub fn attach_animated_gif(&self, id: i64, bytes: &[u8]) -> Result<()> {
        let path = self.media_dir.join(format!("{id}.gif"));
        std::fs::write(&path, bytes)?;
        if let Err(e) = self.refresh_entry(id) {
            let _ = std::fs::remove_file(&path);
            return Err(e);
        }
        Ok(())
    }

    pub fn data_dir(&self) -> &Path {
        self.db_path.parent().unwrap_or(Path::new("."))
    }

    /// Writes a consistent copy of the database to `target`.
    ///
    /// Used when the user moves the data folder: copying the file directly could
    /// catch a half-written WAL, while `VACUUM INTO` produces a single compact
    /// snapshot without interrupting reads.
    pub fn vacuum_into(&self, target: &Path) -> Result<()> {
        let conn = self.conn.lock();
        conn.execute("VACUUM INTO ?1", params![target.to_string_lossy()])?;
        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        self.state.read().clips.is_empty()
    }

    /// Number of clips in history, irrespective of filters.
    pub fn total(&self) -> usize {
        self.state.read().clips.len()
    }

    /// Rebuilds the whole projection from SQLite. Called once at startup.
    fn rebuild_index(&self) -> Result<()> {
        let (rows, collections, tags, archived) = {
            let conn = self.conn.lock();
            let rows = query_clips(&conn, None)?;
            let (collections, tags) = load_all_links(&conn)?;
            let archived = load_archived_collections(&conn)?;
            (rows, collections, tags, archived)
        };

        let mut index = HashMap::with_capacity(rows.len());
        let mut failures = 0usize;
        for row in rows {
            let collections = collections.get(&row.id).cloned().unwrap_or_default();
            let tags = tags.get(&row.id).cloned().unwrap_or_default();
            match self.build_entry(row.clone(), collections, tags) {
                Ok(entry) => {
                    index.insert(row.id, entry);
                }
                Err(e) => {
                    // A clip sealed with a different key must not take down startup.
                    failures += 1;
                    log::warn!("skipping clip {} while indexing: {e}", row.id);
                }
            }
        }
        if failures > 0 {
            log::warn!("{failures} clip(s) could not be decrypted and were not indexed");
        }

        self.state.write().clips = index;
        self.state.write().archived = archived;
        Ok(())
    }

    /// Re-reads one clip from SQLite into the projection.
    fn refresh_entry(&self, id: i64) -> Result<()> {
        let (row, collections, tags) = {
            let conn = self.conn.lock();
            let row = load_clip_row(&conn, id)?;
            let collections = load_collection_ids(&conn, id)?;
            let tags = load_tag_names(&conn, id)?;
            (row, collections, tags)
        };

        let mut state = self.state.write();
        match row {
            Some(row) => {
                let entry = self.build_entry(row, collections, tags)?;
                state.clips.insert(id, entry);
            }
            None => {
                state.clips.remove(&id);
            }
        }
        Ok(())
    }

    fn build_entry(
        &self,
        row: ClipRow,
        collection_ids: Vec<i64>,
        tags: Vec<String>,
    ) -> Result<IndexedClip> {
        let preview = plain_text(&row.preview, &row.preview_nonce).unwrap_or_default();
        let ocr = plain_text(&row.ocr_text, &row.ocr_nonce);
        let note = plain_text(&row.note, &row.note_nonce);
        let search = self.compute_search_text(&row, &preview, &tags, ocr.as_deref(), note.as_deref())?;
        let color = if row.kind == ClipKind::Text {
            crate::color::parse(&preview)
        } else {
            None
        };
        let content_kind = if row.kind == ClipKind::Text {
            crate::color::classify(&preview).map(|kind| kind.as_str().to_string())
        } else {
            None
        };
        // The frontend needs an absolute path to hand a clip to Explorer or to a
        // drag-and-drop target, but only knows the media-relative file name.
        let image_path = row
            .image_path
            .as_ref()
            .map(|name| self.media_dir.join(name).to_string_lossy().into_owned());
        // The downloaded animation lives beside the still frame under the row id,
        // so presence on disk is the flag; no extra column can drift out of sync.
        let animated = row.kind == ClipKind::Image
            && self.media_dir.join(format!("{}.gif", row.id)).is_file();

        let summary = ClipSummary {
            id: row.id,
            kind: row.kind,
            preview,
            byte_size: row.byte_size,
            source_app: row.source_app,
            pinned: row.pinned,
            created_at: row.created_at,
            updated_at: row.updated_at,
            copy_count: row.copy_count,
            width: row.width,
            height: row.height,
            thumb_file: row.thumb_path,
            image_file: row.image_path,
            file_count: row.file_count,
            char_count: row.char_count,
            has_ocr: ocr.as_deref().is_some_and(|text| !text.trim().is_empty()),
            has_note: note.as_deref().is_some_and(|text| !text.trim().is_empty()),
            color,
            content_kind,
            image_path,
            animated,
            collection_ids,
            tags,
            secret_level: row.secret_level.clone(),
        };

        Ok(IndexedClip { summary, search })
    }

    fn compute_search_text(
        &self,
        row: &ClipRow,
        preview: &str,
        row_tags: &[String],
        ocr: Option<&str>,
        note: Option<&str>,
    ) -> Result<String> {
        let raw = match row.kind {
            ClipKind::Text => {
                if row.byte_size <= SEARCH_BODY_LIMIT {
                    plain_text(&row.body, &row.body_nonce).unwrap_or_else(|| preview.to_string())
                } else {
                    preview.to_string()
                }
            }
            ClipKind::Files => {
                let paths = self.files_for_row(row).unwrap_or_default();
                if paths.is_empty() {
                    preview.to_string()
                } else {
                    paths.join(" ")
                }
            }
            ClipKind::Image => preview.to_string(),
        };
        let mut search = search::normalize(&raw);
        // Recognised text from screenshots is searchable like any other body.
        if let Some(ocr) = ocr {
            search.push(' ');
            search.push_str(&search::normalize(ocr));
        }
        // Notes are part of what the clip means to the user, so they are searchable.
        if let Some(note) = note {
            search.push(' ');
            search.push_str(&search::normalize(note));
        }
        // Tags are searchable too: a clip filed under `#invoice` should turn up for
        // "invoice" even when the body never says the word.
        for tag in row_tags {
            search.push(' ');
            search.push_str(&search::normalize(tag));
        }
        Ok(search)
    }
    // -----------------------------------------------------------------------
    // Reads
    // -----------------------------------------------------------------------

    pub fn settings(&self) -> Settings {
        self.state.read().settings.clone()
    }

    pub fn update_settings(&self, patch: &serde_json::Value) -> Result<Settings> {
        let updated = {
            let conn = self.conn.lock();
            settings::apply_patch(&conn, patch)?
        };
        self.state.write().settings = updated.clone();
        Ok(updated)
    }

    pub fn list(&self, filter: &ClipFilter, query: &str, limit: usize, offset: usize) -> ClipPage {
        let state = self.state.read();
        let parsed = Query::parse(query);
        let tag = filter.tag.as_deref();

        let mut matched: Vec<(&IndexedClip, i32)> = state
            .clips
            .values()
            .filter(|entry| matches_filter(&entry.summary, filter, tag, &state.archived))
            .filter_map(|entry| {
                if parsed.is_empty() {
                    Some((entry, 0))
                } else {
                    search::score(&parsed, &entry.search).map(|score| (entry, score))
                }
            })
            .collect();

        // Pinned first, then relevance while searching, then the order the user
        // picked. The final id comparison makes the ordering total, so paging is
        // stable in both directions.
        let order = state.settings.sort_order.as_str();
        matched.sort_by(|a, b| {
            let pinned = b.0.summary.pinned.cmp(&a.0.summary.pinned);
            if pinned != std::cmp::Ordering::Equal {
                return pinned;
            }
            if !parsed.is_empty() {
                let relevance = b.1.cmp(&a.1);
                if relevance != std::cmp::Ordering::Equal {
                    return relevance;
                }
            }
            let timestamp = |summary: &ClipSummary| match order {
                "created-asc" | "created-desc" => summary.created_at,
                _ => summary.updated_at,
            };
            let ascending = order.ends_with("-asc");
            let by_time = timestamp(&a.0.summary).cmp(&timestamp(&b.0.summary));
            let by_time = if ascending { by_time } else { by_time.reverse() };
            if by_time != std::cmp::Ordering::Equal {
                return by_time;
            }
            if ascending {
                a.0.summary.id.cmp(&b.0.summary.id)
            } else {
                b.0.summary.id.cmp(&a.0.summary.id)
            }
        });

        let total = matched.len();
        let items = matched
            .into_iter()
            .skip(offset)
            .take(limit)
            .map(|(entry, _)| entry.summary.clone())
            .collect();

        ClipPage { items, total }
    }

    pub fn detail(&self, id: i64) -> Result<ClipDetail> {
        let row = {
            let conn = self.conn.lock();
            load_clip_row(&conn, id)?
        }
        .ok_or_else(|| AppError::NotFound(format!("clip {id}")))?;

        let summary = self
            .state
            .read()
            .clips
            .get(&id)
            .map(|entry| entry.summary.clone());

        let summary = match summary {
            Some(summary) => summary,
            None => {
                let (collections, tags) = {
                    let conn = self.conn.lock();
                    (load_collection_ids(&conn, id)?, load_tag_names(&conn, id)?)
                };
                self.build_entry(row.clone(), collections, tags)?.summary
            }
        };

        let text = match row.kind {
            ClipKind::Text => plain_text(&row.body, &row.body_nonce),
            _ => None,
        };
        let file_paths = if row.kind == ClipKind::Files {
            self.files_for_row(&row)?
        } else {
            Vec::new()
        };
        let ocr_text = plain_text(&row.ocr_text, &row.ocr_nonce);
        let note = plain_text(&row.note, &row.note_nonce);

        Ok(ClipDetail {
            summary,
            text,
            file_paths,
            ocr_text,
            note,
            has_rich: row.html.is_some() || row.rtf.is_some(),
        })
    }

    /// Full plaintext of a text clip, used when writing it back to the clipboard.
    pub fn text_content(&self, id: i64) -> Result<Option<String>> {
        let row = {
            let conn = self.conn.lock();
            load_clip_row(&conn, id)?
        }
        .ok_or_else(|| AppError::NotFound(format!("clip {id}")))?;
        if row.kind != ClipKind::Text {
            return Ok(None);
        }
        Ok(plain_text(&row.body, &row.body_nonce))
    }

    /// Rich flavours captured with a text clip, when the source offered any.
    pub fn rich_text(&self, id: i64) -> Result<RichText> {
        let row = {
            let conn = self.conn.lock();
            load_clip_row(&conn, id)?
        }
        .ok_or_else(|| AppError::NotFound(format!("clip {id}")))?;
        let open = |ct: &Option<Vec<u8>>, nonce: &Option<Vec<u8>>| -> Option<String> {
            plain_text(ct, nonce)
        };
        Ok(RichText {
            html: open(&row.html, &row.html_nonce),
            rtf: open(&row.rtf, &row.rtf_nonce),
        })
    }

    pub fn file_paths(&self, id: i64) -> Result<Vec<String>> {
        let row = {
            let conn = self.conn.lock();
            load_clip_row(&conn, id)?
        }
        .ok_or_else(|| AppError::NotFound(format!("clip {id}")))?;
        self.files_for_row(&row)
    }

    fn files_for_row(&self, row: &ClipRow) -> Result<Vec<String>> {
        match plain_text(&row.files, &row.files_nonce) {
            Some(json) => Ok(serde_json::from_str(&json).unwrap_or_default()),
            None => Ok(Vec::new()),
        }
    }

    /// Media file name for the full image of a clip, if it is an image clip.
    pub fn image_file(&self, id: i64) -> Result<Option<String>> {
        let row = {
            let conn = self.conn.lock();
            load_clip_row(&conn, id)?
        };
        Ok(row.and_then(|row| row.image_path))
    }

    /// Content hash of a stored clip, used to suppress capturing our own writes.
    pub fn content_hash(&self, id: i64) -> Result<String> {
        let conn = self.conn.lock();
        conn.query_row(
            "SELECT content_hash FROM clips WHERE id = ?1",
            params![id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .ok_or_else(|| AppError::NotFound(format!("clip {id}")))
    }

    // -----------------------------------------------------------------------
    // Writes
    // -----------------------------------------------------------------------

    pub fn insert_clip(&self, new: NewClip) -> Result<InsertOutcome> {
        // Duplicate content is promoted rather than stored twice. Checked before
        // any media is materialised so a repeat screenshot costs nothing on disk.
        let existing = {
            let conn = self.conn.lock();
            find_by_hash(&conn, &new.content_hash)?
        };
        if let Some(id) = existing {
            {
                let conn = self.conn.lock();
                let now = next_stamp(&conn)?;
                conn.execute(
                    "UPDATE clips
                        SET updated_at = ?2,
                            copy_count = copy_count + 1,
                            source_app = COALESCE(?3, source_app)
                      WHERE id = ?1",
                    params![id, now, new.source_app.as_deref()],
                )?;
            }
            self.refresh_entry(id)?;
            return Ok(InsertOutcome::Duplicate(id));
        }

        let media = match &new.content {
            ClipContent::Image {
                width,
                height,
                rgba,
            } => {
                let max_bytes = self.settings().max_image_bytes;
                let files =
                    media::write_image_files(&self.media_dir, *width, *height, rgba, max_bytes)?;
                Some(files)
            }
            _ => None,
        };

        let secret_level = match &new.content {
            ClipContent::Text(text) => {
                let settings = self.settings();
                if settings.flag_secrets {
                    crate::secrets::detect(text, settings.secret_heuristics)
                        .map(|level| level.as_str().to_string())
                } else {
                    None
                }
            }
            _ => None,
        };

        let inserted = {
            let conn = self.conn.lock();
            let now = next_stamp(&conn)?;
            insert_row(
                &conn,
                &new,
                now,
                media.as_ref(),
                secret_level.as_deref(),
            )
        };

        let id = match inserted {
            Ok(id) => id,
            Err(e) => {
                if let Some(files) = &media {
                    files.cleanup(&self.media_dir);
                }
                return Err(e);
            }
        };

        self.refresh_entry(id)?;
        Ok(InsertOutcome::Inserted(id))
    }

    /// Recognises the text in one stored image and folds it into the search index.
    ///
    /// Runs on a worker thread so a large screenshot never delays capture. The
    /// attempt is recorded either way, so startup does not keep retrying images
    /// that simply contain no text.
    pub fn run_ocr(&self, id: i64) -> Result<()> {
        let row = {
            let conn = self.conn.lock();
            load_clip_row(&conn, id)?
        };
        let Some(row) = row else {
            return Ok(());
        };
        if row.kind != ClipKind::Image {
            return Ok(());
        }

        let text = match row.image_path.as_deref() {
            Some(name) => match media::load_rgba(&self.media_dir, name) {
                Ok((width, height, rgba)) => {
                    crate::ocr::extract_text(&rgba, width, height).unwrap_or_else(|e| {
                        log::warn!("text recognition is unavailable: {e}");
                        String::new()
                    })
                }
                Err(e) => {
                    log::warn!("could not read image {name} for text recognition: {e}");
                    String::new()
                }
            },
            None => String::new(),
        };

        let trimmed = text.trim();
        let (ocr_text, ocr_nonce): (Option<Vec<u8>>, Option<Vec<u8>>) = if trimmed.is_empty() {
            (None, None)
        } else {
            (Some(trimmed.as_bytes().to_vec()), None)
        };
        {
            let conn = self.conn.lock();
            conn.execute(
                "UPDATE clips SET ocr_text = ?2, ocr_nonce = ?3, ocr_state = 1 WHERE id = ?1",
                params![id, ocr_text, ocr_nonce],
            )?;
        }
        self.refresh_entry(id)?;
        Ok(())
    }

    /// Runs recognition over images captured before OCR existed. Returns how many
    /// clips were processed.
    pub fn ocr_pending(&self) -> usize {
        let ids: Vec<i64> = {
            let conn = self.conn.lock();
            let Ok(mut stmt) = conn.prepare(
                "SELECT id FROM clips WHERE kind = 'image' AND ocr_state = 0 ORDER BY updated_at",
            ) else {
                return 0;
            };
            let Ok(rows) = stmt.query_map([], |row| row.get::<_, i64>(0)) else {
                return 0;
            };
            rows.filter_map(std::result::Result::ok).collect()
        };

        let mut processed = 0;
        for id in ids {
            if let Err(e) = self.run_ocr(id) {
                log::warn!("text recognition failed for clip {id}: {e}");
            } else {
                processed += 1;
            }
        }
        processed
    }

    /// Returns the recognised text of an image clip, if there is any.
    pub fn ocr_text(&self, id: i64) -> Result<Option<String>> {
        let row = {
            let conn = self.conn.lock();
            load_clip_row(&conn, id)?
        }
        .ok_or_else(|| AppError::NotFound(format!("clip {id}")))?;
        if row.kind != ClipKind::Image {
            return Ok(None);
        }
        Ok(plain_text(&row.ocr_text, &row.ocr_nonce))
    }

    pub fn set_pinned(&self, id: i64, pinned: bool) -> Result<()> {
        {
            let conn = self.conn.lock();
            let changed = conn.execute(
                "UPDATE clips SET pinned = ?2 WHERE id = ?1",
                params![id, pinned as i64],
            )?;
            if changed == 0 {
                return Err(AppError::NotFound(format!("clip {id}")));
            }
        }
        if let Some(entry) = self.state.write().clips.get_mut(&id) {
            entry.summary.pinned = pinned;
        }
        Ok(())
    }

    /// Replaces the body of a text clip.
    ///
    /// The hash is recomputed so de-duplication keeps working, and the rich
    /// flavours are dropped: they described the old text, and pasting stale
    /// formatting over an edited body is worse than pasting plain text.
    pub fn set_text(&self, id: i64, text: &str) -> Result<()> {
        if text.trim().is_empty() {
            return Err(AppError::msg("a text clip cannot be empty"));
        }
        let new_hash = hashing::of_str(text);
        let collapsed: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
        let preview: String = collapsed.chars().take(320).collect();

        {
            let conn = self.conn.lock();
            let row = load_clip_row(&conn, id)?
                .ok_or_else(|| AppError::NotFound(format!("clip {id}")))?;
            if row.kind != ClipKind::Text {
                return Err(AppError::msg("only text clips can be edited"));
            }
            if let Some(existing) = find_by_hash(&conn, &new_hash)?
                && existing != id
            {
                return Err(AppError::msg(
                    "another clip already holds that exact text; edit or delete it instead",
                ));
            }
            let now = next_stamp(&conn)?;
            conn.execute(
                "UPDATE clips
                    SET content_hash = ?2,
                        byte_size = ?3,
                        char_count = ?4,
                        preview = ?5,
                        preview_nonce = NULL,
                        body = ?6,
                        body_nonce = NULL,
                        html = NULL,
                        html_nonce = NULL,
                        rtf = NULL,
                        rtf_nonce = NULL,
                        updated_at = ?7
                  WHERE id = ?1",
                params![
                    id,
                    new_hash,
                    text.len() as i64,
                    text.chars().count() as i64,
                    preview.as_bytes(),
                    text.as_bytes(),
                    now,
                ],
            )?;
        }
        self.refresh_entry(id)?;
        Ok(())
    }

    /// Replaces the note on a clip; an empty note removes it.
    ///
    /// The search projection is rebuilt so the next query sees the new text.
    pub fn set_note(&self, id: i64, note: Option<&str>) -> Result<Option<String>> {
        let cleaned = note
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(ToOwned::to_owned);
        if let Some(text) = &cleaned
            && text.chars().count() > MAX_NOTE_CHARS
        {
            return Err(AppError::msg(format!(
                "a note can be at most {MAX_NOTE_CHARS} characters"
            )));
        }

        let (note, note_nonce): (Option<Vec<u8>>, Option<Vec<u8>>) = match &cleaned {
            Some(text) => (Some(text.as_bytes().to_vec()), None),
            None => (None, None),
        };
        {
            let conn = self.conn.lock();
            let changed = conn.execute(
                "UPDATE clips SET note = ?2, note_nonce = ?3 WHERE id = ?1",
                params![id, note, note_nonce],
            )?;
            if changed == 0 {
                return Err(AppError::NotFound(format!("clip {id}")));
            }
        }
        self.refresh_entry(id)?;
        Ok(cleaned)
    }

    /// Pins or unpins several clips at once, for the multi-select toolbar.
    pub fn set_pinned_many(&self, ids: &[i64], pinned: bool) -> Result<usize> {
        if ids.is_empty() {
            return Ok(0);
        }
        let placeholders = vec!["?"; ids.len()].join(",");
        let pinned_value: i64 = i64::from(pinned);
        // The named parameter comes first so the anonymous ones continue from 2.
        let mut bindings: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(ids.len() + 1);
        bindings.push(&pinned_value);
        bindings.extend(ids.iter().map(|id| id as &dyn rusqlite::ToSql));
        let sql = format!("UPDATE clips SET pinned = ?1 WHERE id IN ({placeholders})");
        let changed = {
            let conn = self.conn.lock();
            conn.execute(&sql, bindings.as_slice())?
        };
        {
            let mut state = self.state.write();
            for id in ids {
                if let Some(entry) = state.clips.get_mut(id) {
                    entry.summary.pinned = pinned;
                }
            }
        }
        Ok(changed)
    }

    /// Adds tags to several clips without disturbing the tags they already have.
    pub fn add_tags_many(&self, ids: &[i64], names: &[String]) -> Result<usize> {
        let mut cleaned: Vec<&str> = Vec::new();
        for name in names {
            let name = name.trim();
            if !name.is_empty() && !cleaned.iter().any(|seen| seen.eq_ignore_ascii_case(name)) {
                cleaned.push(name);
            }
        }
        if cleaned.is_empty() || ids.is_empty() {
            return Ok(0);
        }
        {
            let mut conn = self.conn.lock();
            let tx = conn.transaction()?;
            let created_at = now_ms();
            let mut tag_ids: Vec<i64> = Vec::new();
            for name in &cleaned {
                tx.execute(
                    "INSERT OR IGNORE INTO tags (name, created_at) VALUES (?1, ?2)",
                    params![name, created_at],
                )?;
                let tag_id: i64 = tx.query_row(
                    "SELECT id FROM tags WHERE name = ?1 COLLATE NOCASE",
                    params![name],
                    |row| row.get(0),
                )?;
                tag_ids.push(tag_id);
            }
            for clip_id in ids {
                for tag_id in &tag_ids {
                    tx.execute(
                        "INSERT OR IGNORE INTO clip_tags (clip_id, tag_id) VALUES (?1, ?2)",
                        params![clip_id, tag_id],
                    )?;
                }
            }
            tx.commit()?;
        }
        self.refresh_many(ids)?;
        Ok(ids.len())
    }

    /// Files several clips into one collection, leaving their other collections alone.
    pub fn add_to_collection_many(&self, ids: &[i64], collection_id: i64) -> Result<usize> {
        if ids.is_empty() {
            return Ok(0);
        }
        let archived = self.state.read().archived.contains(&collection_id);
        {
            let mut conn = self.conn.lock();
            let exists: bool = conn
                .query_row(
                    "SELECT 1 FROM collections WHERE id = ?1",
                    params![collection_id],
                    |_| Ok(true),
                )
                .optional()?
                .unwrap_or(false);
            if !exists {
                return Err(AppError::NotFound(format!("collection {collection_id}")));
            }
            let tx = conn.transaction()?;
            for clip_id in ids {
                // An archive is a single slot, so bulk-archiving replaces whatever
                // archive the clip was in rather than adding a second one.
                if archived {
                    tx.execute(
                        "DELETE FROM clip_collections
                          WHERE clip_id = ?1
                            AND collection_id IN (SELECT id FROM collections WHERE archived = 1)",
                        params![clip_id],
                    )?;
                }
                tx.execute(
                    "INSERT OR IGNORE INTO clip_collections (clip_id, collection_id)
                     VALUES (?1, ?2)",
                    params![clip_id, collection_id],
                )?;
            }
            tx.commit()?;
        }
        // A bulk move into an archive has to drop the clips from the list at once.
        self.refresh_many(ids)?;
        Ok(ids.len())
    }

    /// Exports several clips into one folder, returning everything that was written.
    pub fn save_clips(&self, ids: &[i64], dir: &Path) -> Result<Vec<String>> {
        std::fs::create_dir_all(dir)?;
        let mut written: Vec<String> = Vec::new();
        for id in ids {
            match self.save_clip(*id, dir) {
                Ok(paths) => written.extend(paths),
                // One unreadable clip should not abort the rest of the export.
                Err(e) => log::warn!("could not export clip {id}: {e}"),
            }
        }
        if written.is_empty() && !ids.is_empty() {
            return Err(AppError::msg("nothing could be exported from that selection"));
        }
        Ok(written)
    }

    /// Marks a clip as used: bumps the counter and promotes it to the top.
    pub fn record_copy(&self, id: i64) -> Result<()> {
        let now = {
            let conn = self.conn.lock();
            let now = next_stamp(&conn)?;
            let changed = conn.execute(
                "UPDATE clips SET copy_count = copy_count + 1, updated_at = ?2 WHERE id = ?1",
                params![id, now],
            )?;
            if changed == 0 {
                return Err(AppError::NotFound(format!("clip {id}")));
            }
            now
        };
        if let Some(entry) = self.state.write().clips.get_mut(&id) {
            entry.summary.copy_count += 1;
            entry.summary.updated_at = now;
        }
        Ok(())
    }

    pub fn delete(&self, id: i64) -> Result<()> {
        self.delete_many(&[id])?;
        Ok(())
    }

    pub fn delete_many(&self, ids: &[i64]) -> Result<usize> {
        if ids.is_empty() {
            return Ok(0);
        }
        let removed = {
            let conn = self.conn.lock();
            delete_rows(&conn, &self.media_dir, ids)?
        };
        self.forget(ids);
        Ok(removed)
    }

    /// Clears history. Pinned clips survive unless `keep_pinned` is false, and
    /// clips kept in an archive collection always survive.
    pub fn clear(&self, keep_pinned: bool) -> Result<usize> {
        let ids: Vec<i64> = {
            let conn = self.conn.lock();
            let sql = if keep_pinned {
                format!("SELECT id FROM clips WHERE pinned = 0 AND {NOT_ARCHIVED}")
            } else {
                format!("SELECT id FROM clips WHERE {NOT_ARCHIVED}")
            };
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map([], |row| row.get::<_, i64>(0))?;
            rows.collect::<rusqlite::Result<Vec<i64>>>()?
        };
        self.delete_many(&ids)
    }

    /// Enforces the retention policy. Returns how many clips were dropped.
    pub fn prune(&self) -> Result<usize> {
        let settings = self.settings();
        let now = now_ms();
        let mut victims: Vec<i64> = Vec::new();

        {
            let conn = self.conn.lock();

            if settings.max_age_days > 0 {
                let cutoff = now - (i64::from(settings.max_age_days) * 86_400_000);
                let sql = format!(
                    "SELECT id FROM clips WHERE pinned = 0 AND {NOT_ARCHIVED} AND updated_at < ?1"
                );
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map(params![cutoff], |row| row.get::<_, i64>(0))?;
                for row in rows {
                    victims.push(row?);
                }
            }

            if settings.max_age_hours > 0 {
                let cutoff = now - (i64::from(settings.max_age_hours) * 3_600_000);
                let sql = format!(
                    "SELECT id FROM clips WHERE pinned = 0 AND {NOT_ARCHIVED} AND updated_at < ?1"
                );
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map(params![cutoff], |row| row.get::<_, i64>(0))?;
                for row in rows {
                    victims.push(row?);
                }
            }

            if settings.max_items > 0 {
                // Pinned clips are exempt from, and do not count against, the cap.
                let sql = format!(
                    "SELECT id FROM clips WHERE pinned = 0 AND {NOT_ARCHIVED}
                      ORDER BY updated_at DESC, id DESC
                      LIMIT -1 OFFSET ?1"
                );
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map(params![i64::from(settings.max_items)], |row| {
                    row.get::<_, i64>(0)
                })?;
                for row in rows {
                    victims.push(row?);
                }
            }

            victims.sort_unstable();
            victims.dedup();
            if victims.is_empty() {
                return Ok(0);
            }
            delete_rows(&conn, &self.media_dir, &victims)?;
        }

        self.forget(&victims);
        Ok(victims.len())
    }

    fn forget(&self, ids: &[i64]) {
        let mut state = self.state.write();
        for id in ids {
            state.clips.remove(id);
        }
    }

    // -----------------------------------------------------------------------
    // Collections
    // -----------------------------------------------------------------------

    pub fn collections(&self) -> Result<Vec<Collection>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT c.id, c.name, c.color, c.archived, c.created_at,
                    (SELECT COUNT(*) FROM clip_collections cc WHERE cc.collection_id = c.id)
               FROM collections c
              ORDER BY c.name COLLATE NOCASE",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(Collection {
                id: row.get(0)?,
                name: row.get(1)?,
                color: row.get(2)?,
                archived: row.get::<_, i64>(3)? != 0,
                created_at: row.get(4)?,
                clip_count: row.get(5)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    pub fn create_collection(&self, name: &str, color: Option<String>) -> Result<Collection> {
        let name = name.trim();
        if name.is_empty() {
            return Err(AppError::msg("collection name cannot be empty"));
        }
        let conn = self.conn.lock();
        let duplicate = conn
            .query_row(
                "SELECT id FROM collections WHERE name = ?1 COLLATE NOCASE",
                params![name],
                |row| row.get::<_, i64>(0),
            )
            .optional()?
            .is_some();
        if duplicate {
            return Err(AppError::msg(format!(
                "a collection named \"{name}\" already exists"
            )));
        }
        let created_at = now_ms();
        conn.execute(
            "INSERT INTO collections (name, color, archived, created_at) VALUES (?1, ?2, 0, ?3)",
            params![name, color, created_at],
        )?;
        Ok(Collection {
            id: conn.last_insert_rowid(),
            name: name.to_string(),
            color,
            archived: false,
            clip_count: 0,
            created_at,
        })
    }

    /// Moves a collection in or out of the archive set.
    ///
    /// Archiving is a property of the collection rather than of each clip, so the
    /// clips keep their history entry and become visible again the moment the
    /// collection is unarchived.
    pub fn set_collection_archived(&self, id: i64, archived: bool) -> Result<()> {
        let name = self.collection_name(id)?;
        {
            let conn = self.conn.lock();
            let changed = conn.execute(
                "UPDATE collections SET archived = ?2 WHERE id = ?1",
                params![id, archived as i64],
            )?;
            if changed == 0 {
                return Err(AppError::NotFound(format!("collection {id}")));
            }
        }
        {
            let mut state = self.state.write();
            if archived {
                state.archived.insert(id);
            } else {
                state.archived.remove(&id);
            }
        }
        // Each archive owns a folder so it can be found in Explorer.
        if archived {
            crate::storage::ensure_archive_dir(self.data_dir(), &name)?;
        } else {
            crate::storage::remove_archive_dir(self.data_dir(), &name);
        }
        Ok(())
    }

    /// Returns a collection's name and whether it is an archive.
    pub fn collection_name(&self, id: i64) -> Result<String> {
        let conn = self.conn.lock();
        conn.query_row(
            "SELECT name FROM collections WHERE id = ?1",
            params![id],
            |row| row.get::<_, String>(0),
        )
        .optional()?
        .ok_or_else(|| AppError::NotFound(format!("collection {id}")))
    }

    /// Whether a collection is currently an archive.
    pub fn is_archived(&self, id: i64) -> bool {
        self.state.read().archived.contains(&id)
    }

    /// The path to reveal when the user asks Explorer for a clip.
    ///
    /// File clips point at the first copied path (or its folder when the original
    /// has gone), images at the stored PNG. Text lives only in the encrypted
    /// database, so there is nothing on disk to show.
    pub fn reveal_path(&self, id: i64) -> Result<std::path::PathBuf> {
        let row = {
            let conn = self.conn.lock();
            load_clip_row(&conn, id)?
        }
        .ok_or_else(|| AppError::NotFound(format!("clip {id}")))?;

        match row.kind {
            ClipKind::Image => {
                let name = row
                    .image_path
                    .ok_or_else(|| AppError::msg("this image has no stored file"))?;
                Ok(self.media_dir.join(name))
            }
            ClipKind::Files => {
                let paths = self.file_paths(id)?;
                let first = paths
                    .first()
                    .ok_or_else(|| AppError::msg("this clip has no stored paths"))?;
                let path = std::path::PathBuf::from(first);
                if path.exists() {
                    Ok(path)
                } else {
                    path.parent()
                        .map(std::path::Path::to_path_buf)
                        .ok_or_else(|| AppError::msg("the original location no longer exists"))
                }
            }
            ClipKind::Text => Err(AppError::msg(
                "text clips live in the encrypted database, not in a file",
            )),
        }
    }

    /// Copies a clip's payload into a folder the user picked, so it stops living
    /// only inside the encrypted database. Returns the paths that were written.
    pub fn save_clip(&self, id: i64, dir: &Path) -> Result<Vec<String>> {
        std::fs::create_dir_all(dir)?;
        let row = {
            let conn = self.conn.lock();
            load_clip_row(&conn, id)?
        }
        .ok_or_else(|| AppError::NotFound(format!("clip {id}")))?;

        let mut written: Vec<String> = Vec::new();
        match row.kind {
            ClipKind::Image => {
                let name = row
                    .image_path
                    .ok_or_else(|| AppError::msg("this image has no stored file"))?;
                let source = self.media_dir.join(&name);
                let target = unique_path(dir, &name);
                std::fs::copy(&source, &target)?;
                written.push(target.to_string_lossy().into_owned());
            }
            ClipKind::Files => {
                for path in self.files_for_row(&row)? {
                    let source = PathBuf::from(&path);
                    if !source.is_file() {
                        continue;
                    }
                    let name = source
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| "file".to_string());
                    let target = unique_path(dir, &name);
                    std::fs::copy(&source, &target)?;
                    written.push(target.to_string_lossy().into_owned());
                }
                if written.is_empty() {
                    return Err(AppError::msg("the original files no longer exist"));
                }
            }
            ClipKind::Text => {
                let text = plain_text(&row.body, &row.body_nonce).unwrap_or_default();
                let stem = file_stem_suggestion(&text);
                let target = unique_path(dir, &format!("{stem}.txt"));
                std::fs::write(&target, &text)?;
                written.push(target.to_string_lossy().into_owned());
                // Keeping the styled version next to the plain one costs nothing
                // and is the only way a saved clip keeps its formatting.
                if let Some(html) = plain_text(&row.html, &row.html_nonce)
                    && !html.trim().is_empty()
                {
                    let target = unique_path(dir, &format!("{stem}.html"));
                    std::fs::write(&target, html)?;
                    written.push(target.to_string_lossy().into_owned());
                }
            }
        }
        Ok(written)
    }

    pub fn rename_collection(&self, id: i64, name: &str) -> Result<()> {
        let name = name.trim();
        if name.is_empty() {
            return Err(AppError::msg("collection name cannot be empty"));
        }
        let previous = self.collection_name(id).ok();
        let archived = self.state.read().archived.contains(&id);
        {
            let conn = self.conn.lock();
            let changed = conn.execute(
                "UPDATE collections SET name = ?2 WHERE id = ?1",
                params![id, name],
            )?;
            if changed == 0 {
                return Err(AppError::NotFound(format!("collection {id}")));
            }
        }
        if archived && let Some(previous) = previous {
            self.rename_archive_dir(&previous, name);
        }
        Ok(())
    }

    /// Moves an archive folder to follow a rename, best-effort.
    fn rename_archive_dir(&self, previous: &str, next: &str) {
        let data_dir = self.data_dir();
        let from = crate::storage::archive_dir(data_dir, previous);
        let to = crate::storage::archive_dir(data_dir, next);
        if from != to && from.is_dir() {
            if let Some(parent) = to.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            if let Err(e) = std::fs::rename(&from, &to) {
                log::warn!("could not rename the archive folder: {e}");
            }
        }
    }

    pub fn delete_collection(&self, id: i64) -> Result<()> {
        let name = self.collection_name(id).ok();
        {
            let conn = self.conn.lock();
            let changed = conn.execute("DELETE FROM collections WHERE id = ?1", params![id])?;
            if changed == 0 {
                return Err(AppError::NotFound(format!("collection {id}")));
            }
        }
        self.state.write().archived.remove(&id);
        if let Some(name) = name {
            crate::storage::remove_archive_dir(self.data_dir(), &name);
        }
        // Links cascade, so affected clips need their projections refreshed.
        let affected: Vec<i64> = self
            .state
            .read()
            .clips
            .iter()
            .filter(|(_, entry)| entry.summary.collection_ids.contains(&id))
            .map(|(id, _)| *id)
            .collect();
        for clip_id in affected {
            let ids = {
                let conn = self.conn.lock();
                load_collection_ids(&conn, clip_id)?
            };
            if let Some(entry) = self.state.write().clips.get_mut(&clip_id) {
                entry.summary.collection_ids = ids;
            }
        }
        Ok(())
    }

    /// Replaces the *ordinary* collections a clip is filed under.
    ///
    /// Archive membership is deliberately left alone: archiving is a single slot
    /// (see [`Store::set_clip_archive`]) and unticking a collection in the preview
    /// pane must not silently unarchive the clip.
    pub fn set_clip_collections(&self, clip_id: i64, ids: &[i64]) -> Result<Vec<i64>> {
        let archived = self.state.read().archived.clone();
        {
            let mut conn = self.conn.lock();
            let tx = conn.transaction()?;
            // Only the non-archive links are being replaced.
            tx.execute(
                "DELETE FROM clip_collections
                  WHERE clip_id = ?1
                    AND collection_id NOT IN (SELECT id FROM collections WHERE archived = 1)",
                params![clip_id],
            )?;
            for collection_id in ids {
                if archived.contains(collection_id) {
                    // Archives are chosen through the archive slot instead.
                    continue;
                }
                // Silently skip ids that no longer exist rather than failing the
                // whole update; the UI may be holding a stale list.
                let exists: bool = tx
                    .query_row(
                        "SELECT 1 FROM collections WHERE id = ?1",
                        params![collection_id],
                        |_| Ok(true),
                    )
                    .optional()?
                    .unwrap_or(false);
                if exists {
                    tx.execute(
                        "INSERT OR IGNORE INTO clip_collections (clip_id, collection_id)
                         VALUES (?1, ?2)",
                        params![clip_id, collection_id],
                    )?;
                }
            }
            tx.commit()?;
        }
        let applied = {
            let conn = self.conn.lock();
            load_collection_ids(&conn, clip_id)?
        };
        // Rebuild instead of patching the field: filing a clip has to be reflected
        // in the list and in the counts right away.
        self.refresh_entry(clip_id)?;
        Ok(applied)
    }

    /// Files a clip into exactly one archive collection, or takes it out of every
    /// archive when `archive_id` is `None`.
    pub fn set_clip_archive(&self, clip_id: i64, archive_id: Option<i64>) -> Result<Vec<i64>> {
        {
            let mut conn = self.conn.lock();
            let tx = conn.transaction()?;
            if let Some(archive_id) = archive_id {
                let archived: bool = tx
                    .query_row(
                        "SELECT archived FROM collections WHERE id = ?1",
                        params![archive_id],
                        |row| row.get::<_, i64>(0),
                    )
                    .optional()?
                    .is_some_and(|value| value != 0);
                if !archived {
                    return Err(AppError::msg("that collection is not an archive"));
                }
            }
            // One archive at a time: clearing first is what makes the slot singular.
            tx.execute(
                "DELETE FROM clip_collections
                  WHERE clip_id = ?1
                    AND collection_id IN (SELECT id FROM collections WHERE archived = 1)",
                params![clip_id],
            )?;
            if let Some(archive_id) = archive_id {
                tx.execute(
                    "INSERT OR IGNORE INTO clip_collections (clip_id, collection_id)
                     VALUES (?1, ?2)",
                    params![clip_id, archive_id],
                )?;
            }
            tx.commit()?;
        }
        let applied = {
            let conn = self.conn.lock();
            load_collection_ids(&conn, clip_id)?
        };
        self.refresh_entry(clip_id)?;
        Ok(applied)
    }

    // -----------------------------------------------------------------------
    // Tags
    // -----------------------------------------------------------------------

    pub fn tags(&self) -> Result<Vec<Tag>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT t.id, t.name, t.color,
                    (SELECT COUNT(*) FROM clip_tags ct WHERE ct.tag_id = t.id)
               FROM tags t
              ORDER BY t.name COLLATE NOCASE",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(Tag {
                id: row.get(0)?,
                name: row.get(1)?,
                color: row.get(2)?,
                clip_count: row.get(3)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    fn tag_clip_ids(conn: &Connection, tag_id: i64) -> Result<Vec<i64>> {
        let mut stmt = conn.prepare("SELECT clip_id FROM clip_tags WHERE tag_id = ?1")?;
        let rows = stmt.query_map(params![tag_id], |row| row.get::<_, i64>(0))?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    fn refresh_many(&self, ids: &[i64]) -> Result<()> {
        for id in ids {
            self.refresh_entry(*id)?;
        }
        Ok(())
    }

    /// Renames a tag. Renaming onto a name that already exists merges the two
    /// instead of failing, which is what a user asking for the rename expects.
    pub fn rename_tag(&self, id: i64, name: &str) -> Result<()> {
        let name = name.trim();
        if name.is_empty() {
            return Err(AppError::msg("tag name cannot be empty"));
        }
        let mut affected: Vec<i64> = {
            let mut conn = self.conn.lock();
            let tx = conn.transaction()?;
            let current: Option<String> = tx
                .query_row("SELECT name FROM tags WHERE id = ?1", params![id], |row| {
                    row.get::<_, String>(0)
                })
                .optional()?;
            let Some(current) = current else {
                return Err(AppError::NotFound(format!("tag {id}")));
            };
            if current == name {
                return Ok(());
            }
            let mut affected = Self::tag_clip_ids(&tx, id)?;
            let existing: Option<i64> = tx
                .query_row(
                    "SELECT id FROM tags WHERE name = ?1 COLLATE NOCASE AND id != ?2",
                    params![name, id],
                    |row| row.get::<_, i64>(0),
                )
                .optional()?;
            match existing {
                Some(target) => {
                    tx.execute(
                        "INSERT OR IGNORE INTO clip_tags (clip_id, tag_id)
                         SELECT clip_id, ?2 FROM clip_tags WHERE tag_id = ?1",
                        params![id, target],
                    )?;
                    tx.execute("DELETE FROM tags WHERE id = ?1", params![id])?;
                    affected.extend(Self::tag_clip_ids(&tx, target)?);
                }
                None => {
                    tx.execute(
                        "UPDATE tags SET name = ?2 WHERE id = ?1",
                        params![id, name],
                    )?;
                }
            }
            tx.commit()?;
            affected
        };
        affected.sort_unstable();
        affected.dedup();
        // Tag names are part of the search index, so the projection is stale now.
        self.refresh_many(&affected)?;
        Ok(())
    }

    pub fn delete_tag(&self, id: i64) -> Result<()> {
        let affected = {
            let conn = self.conn.lock();
            let affected = Self::tag_clip_ids(&conn, id)?;
            let removed = conn.execute("DELETE FROM tags WHERE id = ?1", params![id])?;
            if removed == 0 {
                return Err(AppError::NotFound(format!("tag {id}")));
            }
            affected
        };
        self.refresh_many(&affected)?;
        Ok(())
    }

    /// Sets (or clears) the color a tag is painted with in the sidebar.
    pub fn set_tag_color(&self, id: i64, color: Option<&str>) -> Result<()> {
        let color = match color.map(str::trim).filter(|value| !value.is_empty()) {
            Some(raw) => Some(
                crate::color::parse(raw)
                    .ok_or_else(|| AppError::msg(format!("\"{raw}\" is not a color")))?,
            ),
            None => None,
        };
        let conn = self.conn.lock();
        let changed = conn.execute(
            "UPDATE tags SET color = ?2 WHERE id = ?1",
            params![id, color],
        )?;
        if changed == 0 {
            return Err(AppError::NotFound(format!("tag {id}")));
        }
        Ok(())
    }

    pub fn set_clip_tags(&self, clip_id: i64, names: &[String]) -> Result<Vec<String>> {
        let mut cleaned: Vec<String> = Vec::new();
        for name in names {
            let name = name.trim();
            if name.is_empty() {
                continue;
            }
            if !cleaned
                .iter()
                .any(|existing| existing.eq_ignore_ascii_case(name))
            {
                cleaned.push(name.to_string());
            }
        }
        if cleaned.len() > 32 {
            return Err(AppError::msg("at most 32 tags per clip"));
        }

        {
            let mut conn = self.conn.lock();
            let tx = conn.transaction()?;
            tx.execute("DELETE FROM clip_tags WHERE clip_id = ?1", params![clip_id])?;
            for name in &cleaned {
                let created_at = now_ms();
                tx.execute(
                    "INSERT OR IGNORE INTO tags (name, created_at) VALUES (?1, ?2)",
                    params![name, created_at],
                )?;
                let tag_id: i64 = tx.query_row(
                    "SELECT id FROM tags WHERE name = ?1 COLLATE NOCASE",
                    params![name],
                    |row| row.get(0),
                )?;
                tx.execute(
                    "INSERT OR IGNORE INTO clip_tags (clip_id, tag_id) VALUES (?1, ?2)",
                    params![clip_id, tag_id],
                )?;
            }
            // Drop tags that are no longer attached to anything.
            tx.execute(
                "DELETE FROM tags WHERE id NOT IN (SELECT tag_id FROM clip_tags)",
                [],
            )?;
            tx.commit()?;
        }

        let applied = {
            let conn = self.conn.lock();
            load_tag_names(&conn, clip_id)?
        };
        // Tags feed the search index, so the projection has to be rebuilt rather
        // than patched.
        self.refresh_entry(clip_id)?;
        Ok(applied)
    }

    // -----------------------------------------------------------------------
    // Stats
    // -----------------------------------------------------------------------

    pub fn stats(&self) -> Result<Stats> {
        let state = self.state.read();
        let mut stats = Stats {
            total: 0,
            pinned: 0,
            images: 0,
            files: 0,
            archived: 0,
            colors: 0,
            media_bytes: 0,
            database_bytes: 0,
        };
        for entry in state.clips.values() {
            if entry
                .summary
                .collection_ids
                .iter()
                .any(|id| state.archived.contains(id))
            {
                stats.archived += 1;
                continue;
            }
            stats.total += 1;
            if entry.summary.pinned {
                stats.pinned += 1;
            }
            if entry.summary.color.is_some() {
                stats.colors += 1;
            }
            match entry.summary.kind {
                ClipKind::Image => stats.images += 1,
                ClipKind::Files => stats.files += 1,
                ClipKind::Text => {}
            }
        }
        drop(state);

        if let Ok(entries) = std::fs::read_dir(&self.media_dir) {
            stats.media_bytes = entries
                .flatten()
                .filter_map(|entry| entry.metadata().ok())
                .filter(|meta| meta.is_file())
                .map(|meta| meta.len() as i64)
                .sum();
        }
        for suffix in ["", "-wal", "-shm"] {
            let candidate = PathBuf::from(format!("{}{suffix}", self.db_path.display()));
            if let Ok(meta) = std::fs::metadata(&candidate) {
                stats.database_bytes += meta.len() as i64;
            }
        }

        Ok(stats)
    }
}

// ---------------------------------------------------------------------------
// Free helpers
// ---------------------------------------------------------------------------

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// A path in `dir` that does not exist yet, appending " (2)", " (3)" … as needed.
fn unique_path(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let path = Path::new(name);
    let stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_else(|| "clip".to_string());
    let extension = path
        .extension()
        .map(|ext| format!(".{}", ext.to_string_lossy()))
        .unwrap_or_default();
    for index in 2..10_000 {
        let candidate = dir.join(format!("{stem} ({index}){extension}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    dir.join(format!("{stem}-{}{extension}", now_ms()))
}

/// Turns the first meaningful line of a clip into a usable file name.
fn file_stem_suggestion(text: &str) -> String {
    let first = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("clip");
    let mut stem: String = first
        .chars()
        .map(|c| match c {
            '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => ' ',
            c if c.is_control() => ' ',
            c => c,
        })
        .take(60)
        .collect();
    stem = stem.split_whitespace().collect::<Vec<_>>().join(" ");
    let stem = stem.trim().trim_end_matches('.').to_string();
    if stem.is_empty() {
        "clip".to_string()
    } else {
        stem
    }
}

/// Timestamp guaranteed to be strictly newer than every stored clip.
///
/// Wall-clock milliseconds are not fine-grained enough to order two writes that
/// land in the same tick, and the list is sorted by `updated_at`. Without the
/// `max(...)` the promise "the clip you just picked moves to the top" would depend
/// on timer resolution and could quietly stop holding.
///
/// Must be called while holding the connection lock, so that reading
/// `MAX(updated_at)` and the write it accompanies form one atomic step.
fn next_stamp(conn: &Connection) -> Result<i64> {
    let newest: i64 = conn.query_row(
        "SELECT COALESCE(MAX(updated_at), 0) FROM clips",
        [],
        |row| row.get(0),
    )?;
    Ok(now_ms().max(newest.saturating_add(1)))
}

fn migrate(conn: &Connection) -> Result<()> {
    conn.execute_batch(SCHEMA)?;

    // Databases created before archives existed need the column added by hand;
    // `CREATE TABLE IF NOT EXISTS` leaves an existing table alone.
    let has_archived = {
        let mut stmt = conn.prepare("PRAGMA table_info(collections)")?;
        let names = stmt.query_map([], |row| row.get::<_, String>(1))?;
        names
            .collect::<rusqlite::Result<Vec<String>>>()?
            .iter()
            .any(|name| name == "archived")
    };
    if !has_archived {
        conn.execute(
            "ALTER TABLE collections ADD COLUMN archived INTEGER NOT NULL DEFAULT 0",
            [],
        )?;
        log::info!("added the archived flag to collections");
    }

    // OCR arrived later than the first schema; add its columns to older databases.
    let clip_columns: Vec<String> = {
        let mut stmt = conn.prepare("PRAGMA table_info(clips)")?;
        let names = stmt.query_map([], |row| row.get::<_, String>(1))?;
        names.collect::<rusqlite::Result<Vec<String>>>()?
    };
    for (column, ddl) in [
        ("ocr_text", "ALTER TABLE clips ADD COLUMN ocr_text BLOB"),
        ("ocr_nonce", "ALTER TABLE clips ADD COLUMN ocr_nonce BLOB"),
        (
            "ocr_state",
            "ALTER TABLE clips ADD COLUMN ocr_state INTEGER NOT NULL DEFAULT 0",
        ),
        ("html", "ALTER TABLE clips ADD COLUMN html BLOB"),
        ("html_nonce", "ALTER TABLE clips ADD COLUMN html_nonce BLOB"),
        ("rtf", "ALTER TABLE clips ADD COLUMN rtf BLOB"),
        ("rtf_nonce", "ALTER TABLE clips ADD COLUMN rtf_nonce BLOB"),
        ("note", "ALTER TABLE clips ADD COLUMN note BLOB"),
        ("note_nonce", "ALTER TABLE clips ADD COLUMN note_nonce BLOB"),
        ("secret_level", "ALTER TABLE clips ADD COLUMN secret_level TEXT"),
    ] {
        if !clip_columns.iter().any(|name| name == column) {
            conn.execute(ddl, [])?;
            log::info!("added the {column} column to clips");
        }
    }

    let tag_columns: Vec<String> = {
        let mut stmt = conn.prepare("PRAGMA table_info(tags)")?;
        let names = stmt.query_map([], |row| row.get::<_, String>(1))?;
        names.collect::<rusqlite::Result<Vec<String>>>()?
    };
    if !tag_columns.iter().any(|name| name == "color") {
        conn.execute("ALTER TABLE tags ADD COLUMN color TEXT", [])?;
        log::info!("added the color column to tags");
    }

    // One-time correction: `hideOnBlur` used to default to on, and writing any
    // other setting persisted that default into the database. Anyone who changed
    // a setting therefore carries `true` around even though they never chose it,
    // so the new off-by-default would otherwise appear to be ignored.
    let already_reset = conn
        .query_row(
            "SELECT 1 FROM meta WHERE key = ?1",
            params![FOCUS_HIDE_RESET_KEY],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if !already_reset {
        conn.execute(
            "INSERT INTO settings (key, value) VALUES ('hideOnBlur', 'false')
             ON CONFLICT(key) DO UPDATE SET value = 'false'",
            [],
        )?;
        conn.execute(
            "INSERT INTO meta (key, value) VALUES (?1, '1')
             ON CONFLICT(key) DO UPDATE SET value = '1'",
            params![FOCUS_HIDE_RESET_KEY],
        )?;
        log::info!("reset hide-on-blur to the new default (off)");
    }

    // The first-run wizard arrived after the app did. A database that already
    // holds clips, or a setting the user changed, belongs to someone who has
    // used CopyPaste before, so do not interrupt them with setup on upgrade.
    // A fresh database only has the `hideOnBlur` row written just above.
    let already_backfilled = conn
        .query_row(
            "SELECT 1 FROM meta WHERE key = ?1",
            params![SETUP_BACKFILL_KEY],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if !already_backfilled {
        let has_history: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM clips)
                OR EXISTS(SELECT 1 FROM settings WHERE key <> 'hideOnBlur')",
            [],
            |row| row.get(0),
        )?;
        if has_history {
            conn.execute(
                "INSERT INTO settings (key, value) VALUES ('setupComplete', 'true')
                 ON CONFLICT(key) DO UPDATE SET value = 'true'",
                [],
            )?;
            log::info!("marked setup complete for an existing installation");
        }
        conn.execute(
            "INSERT INTO meta (key, value) VALUES (?1, '1')
             ON CONFLICT(key) DO UPDATE SET value = '1'",
            params![SETUP_BACKFILL_KEY],
        )?;
    }

    conn.execute(
        "INSERT INTO meta (key, value) VALUES ('schema_version', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![SCHEMA_VERSION],
    )?;
    Ok(())
}

/// Rewrites any rows written by an encrypting build as plain text.
///
/// Migration is per row and leaves nonces NULL, so a crash midway simply resumes
/// on the next launch. The legacy key is deleted once nothing encrypted remains.
fn migrate_encrypted_rows(
    conn: &Connection,
    cipher: Option<&Cipher>,
    key_path: &Path,
) -> Result<()> {
    let encrypted = has_encrypted_rows(conn)?;
    let Some(cipher) = cipher else {
        if encrypted {
            log::warn!(
                "history contains encrypted clips but key.bin is unavailable; those clips are skipped"
            );
        }
        return Ok(());
    };

    if !encrypted {
        // A leftover key from an old install whose history was already cleared.
        remove_legacy_key(key_path);
        return Ok(());
    }

    let rows = query_clips(conn, None)?;
    let mut migrated = 0usize;
    let mut failures = 0usize;
    for row in rows {
        let still_sealed = row.preview_nonce.is_some()
            || row.body_nonce.is_some()
            || row.files_nonce.is_some()
            || row.ocr_nonce.is_some()
            || row.html_nonce.is_some()
            || row.rtf_nonce.is_some()
            || row.note_nonce.is_some();
        if !still_sealed {
            continue;
        }

        let decrypted = (|| -> Result<_> {
            Ok((
                decrypt_pair(cipher, &row.preview, &row.preview_nonce)?,
                decrypt_pair(cipher, &row.body, &row.body_nonce)?,
                decrypt_pair(cipher, &row.files, &row.files_nonce)?,
                decrypt_pair(cipher, &row.ocr_text, &row.ocr_nonce)?,
                decrypt_pair(cipher, &row.html, &row.html_nonce)?,
                decrypt_pair(cipher, &row.rtf, &row.rtf_nonce)?,
                decrypt_pair(cipher, &row.note, &row.note_nonce)?,
            ))
        })();

        let Ok((preview, body, files, ocr, html, rtf, note)) = decrypted else {
            failures += 1;
            log::warn!("could not decrypt legacy clip {}", row.id);
            continue;
        };

        conn.execute(
            "UPDATE clips
                SET preview = ?2, preview_nonce = NULL,
                    body = ?3, body_nonce = NULL,
                    files = ?4, files_nonce = NULL,
                    ocr_text = ?5, ocr_nonce = NULL,
                    html = ?6, html_nonce = NULL,
                    rtf = ?7, rtf_nonce = NULL,
                    note = ?8, note_nonce = NULL
              WHERE id = ?1",
            params![row.id, preview, body, files, ocr, html, rtf, note],
        )?;
        migrated += 1;
    }

    if failures == 0 {
        remove_legacy_key(key_path);
        log::info!("migrated {migrated} encrypted clip(s) to plain text");
    } else {
        log::warn!(
            "migrated {migrated} encrypted clip(s); {failures} could not be read and key.bin was kept"
        );
    }
    Ok(())
}

fn has_encrypted_rows(conn: &Connection) -> Result<bool> {
    Ok(conn.query_row(
        "SELECT EXISTS(
            SELECT 1 FROM clips
             WHERE preview_nonce IS NOT NULL
                OR body_nonce IS NOT NULL
                OR files_nonce IS NOT NULL
                OR ocr_nonce IS NOT NULL
                OR html_nonce IS NOT NULL
                OR rtf_nonce IS NOT NULL
                OR note_nonce IS NOT NULL
         )",
        [],
        |row| row.get::<_, i64>(0),
    )? != 0)
}

fn decrypt_pair(
    cipher: &Cipher,
    value: &Option<Vec<u8>>,
    nonce: &Option<Vec<u8>>,
) -> Result<Option<Vec<u8>>> {
    match (value, nonce) {
        (Some(ciphertext), Some(nonce)) => Ok(Some(cipher.open(nonce, ciphertext)?)),
        (Some(plain), None) => Ok(Some(plain.clone())),
        _ => Ok(None),
    }
}

fn remove_legacy_key(key_path: &Path) {
    if let Err(e) = std::fs::remove_file(key_path)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        log::warn!("could not remove the legacy key file: {e}");
    }
}

/// Reads a payload column as UTF-8. A legacy row that could not be migrated is
/// left with a nonce and deliberately reads as empty rather than as ciphertext.
fn plain_text(value: &Option<Vec<u8>>, nonce: &Option<Vec<u8>>) -> Option<String> {
    if nonce.is_some() {
        return None;
    }
    value
        .as_ref()
        .and_then(|bytes| String::from_utf8(bytes.clone()).ok())
}

fn row_to_clip(row: &rusqlite::Row<'_>) -> rusqlite::Result<ClipRow> {
    let kind: String = row.get(1)?;
    let kind = ClipKind::parse(&kind).unwrap_or(ClipKind::Text);
    Ok(ClipRow {
        id: row.get(0)?,
        kind,
        byte_size: row.get(2)?,
        char_count: row.get(3)?,
        file_count: row.get(4)?,
        preview: row.get(5)?,
        preview_nonce: row.get(6)?,
        body: row.get(7)?,
        body_nonce: row.get(8)?,
        files: row.get(9)?,
        files_nonce: row.get(10)?,
        image_path: row.get(11)?,
        thumb_path: row.get(12)?,
        width: row.get(13)?,
        height: row.get(14)?,
        source_app: row.get(15)?,
        pinned: row.get::<_, i64>(16)? != 0,
        created_at: row.get(17)?,
        updated_at: row.get(18)?,
        copy_count: row.get(19)?,
        ocr_text: row.get(20)?,
        ocr_nonce: row.get(21)?,
        html: row.get(22)?,
        html_nonce: row.get(23)?,
        rtf: row.get(24)?,
        rtf_nonce: row.get(25)?,
        note: row.get(26)?,
        note_nonce: row.get(27)?,
        secret_level: row.get(28)?,
    })
}

fn load_clip_row(conn: &Connection, id: i64) -> Result<Option<ClipRow>> {
    let sql = format!("SELECT {CLIP_COLUMNS} FROM clips WHERE id = ?1");
    Ok(conn.query_row(&sql, params![id], row_to_clip).optional()?)
}

fn query_clips(conn: &Connection, kind: Option<ClipKind>) -> Result<Vec<ClipRow>> {
    let (sql, kind_param) = match kind {
        Some(kind) => (
            format!("SELECT {CLIP_COLUMNS} FROM clips WHERE kind = ?1"),
            Some(kind.as_str()),
        ),
        None => (format!("SELECT {CLIP_COLUMNS} FROM clips"), None),
    };
    let mut stmt = conn.prepare(&sql)?;
    let rows = match kind_param {
        Some(kind) => stmt.query_map(params![kind], row_to_clip)?,
        None => stmt.query_map([], row_to_clip)?,
    };
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn find_by_hash(conn: &Connection, hash: &str) -> Result<Option<i64>> {
    Ok(conn
        .query_row(
            "SELECT id FROM clips WHERE content_hash = ?1",
            params![hash],
            |row| row.get::<_, i64>(0),
        )
        .optional()?)
}

fn insert_row(
    conn: &Connection,
    new: &NewClip,
    now: i64,
    media: Option<&ImageFiles>,
    secret_level: Option<&str>,
) -> Result<i64> {
    let preview = new.preview.as_bytes().to_vec();

    let body = match &new.content {
        ClipContent::Text(text) => Some(text.as_bytes().to_vec()),
        _ => None,
    };

    let (files, file_count) = match &new.content {
        ClipContent::Files(paths) => (
            Some(serde_json::to_string(paths)?.into_bytes()),
            paths.len() as i64,
        ),
        _ => (None, 0),
    };

    // Rich flavours are optional: most sources offer none, and an empty string is
    // no more useful than a missing one.
    let optional = |value: &Option<String>| -> Option<Vec<u8>> {
        match value {
            Some(text) if !text.trim().is_empty() => Some(text.as_bytes().to_vec()),
            _ => None,
        }
    };
    let html = optional(&new.html);
    let rtf = optional(&new.rtf);

    let byte_size = match &new.content {
        ClipContent::Text(text) => text.len() as i64,
        ClipContent::Image { .. } => media.map(|m| m.byte_size).unwrap_or(0),
        ClipContent::Files(paths) => paths.iter().map(|p| p.len() as i64).sum(),
    };
    let char_count = match &new.content {
        ClipContent::Text(text) => text.chars().count() as i64,
        _ => 0,
    };
    let (width, height) = match &new.content {
        ClipContent::Image { width, height, .. } => (Some(*width as i64), Some(*height as i64)),
        _ => (None, None),
    };

    conn.execute(
        "INSERT INTO clips (
            kind, content_hash, byte_size, char_count, file_count,
            preview, preview_nonce, body, body_nonce, files, files_nonce,
            image_path, thumb_path, width, height, source_app,
            html, html_nonce, rtf, rtf_nonce,
            pinned, created_at, updated_at, copy_count, secret_level
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, NULL, ?7, NULL, ?8, NULL,
            ?9, ?10, ?11, ?12, ?13, ?14, NULL, ?15, NULL, 0, ?16, ?16, 1, ?17
         )",
        params![
            new.content.kind().as_str(),
            new.content_hash,
            byte_size,
            char_count,
            file_count,
            preview,
            body,
            files,
            media.map(|m| m.image.as_str()),
            media.map(|m| m.thumb.as_str()),
            width,
            height,
            new.source_app,
            html,
            rtf,
            now,
            secret_level,
        ],
    )?;
    Ok(conn.last_insert_rowid())
}

fn delete_rows(conn: &Connection, media_dir: &Path, ids: &[i64]) -> Result<usize> {
    if ids.is_empty() {
        return Ok(0);
    }
    let placeholders = vec!["?"; ids.len()].join(",");
    let bindings: Vec<&dyn rusqlite::ToSql> =
        ids.iter().map(|id| id as &dyn rusqlite::ToSql).collect();

    // Collect media paths before the rows disappear, so image files can be unlinked.
    let mut media_files: Vec<String> = Vec::new();
    {
        let sql = format!("SELECT image_path, thumb_path FROM clips WHERE id IN ({placeholders})");
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(bindings.as_slice(), |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, Option<String>>(1)?,
            ))
        })?;
        for row in rows {
            let (image, thumb) = row?;
            media_files.extend(image);
            media_files.extend(thumb);
        }
    }

    let sql = format!("DELETE FROM clips WHERE id IN ({placeholders})");
    let removed = conn.execute(&sql, bindings.as_slice())?;

    for name in media_files {
        let _ = std::fs::remove_file(media_dir.join(name));
    }
    // The recovered animation is stored under the row id rather than in a column.
    for id in ids {
        let _ = std::fs::remove_file(media_dir.join(format!("{id}.gif")));
    }

    Ok(removed)
}

fn load_collection_ids(conn: &Connection, clip_id: i64) -> Result<Vec<i64>> {
    let mut stmt = conn.prepare(
        "SELECT collection_id FROM clip_collections WHERE clip_id = ?1 ORDER BY collection_id",
    )?;
    let rows = stmt.query_map(params![clip_id], |row| row.get::<_, i64>(0))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn load_tag_names(conn: &Connection, clip_id: i64) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT t.name FROM clip_tags ct
           JOIN tags t ON t.id = ct.tag_id
          WHERE ct.clip_id = ?1
          ORDER BY t.name COLLATE NOCASE",
    )?;
    let rows = stmt.query_map(params![clip_id], |row| row.get::<_, String>(0))?;
    Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
}

fn load_archived_collections(conn: &Connection) -> Result<HashSet<i64>> {
    let mut stmt = conn.prepare("SELECT id FROM collections WHERE archived = 1")?;
    let rows = stmt.query_map([], |row| row.get::<_, i64>(0))?;
    Ok(rows.collect::<rusqlite::Result<HashSet<i64>>>()?)
}

type LinkMap<T> = HashMap<i64, Vec<T>>;

fn load_all_links(conn: &Connection) -> Result<(LinkMap<i64>, LinkMap<String>)> {
    let mut collections: LinkMap<i64> = HashMap::new();
    {
        let mut stmt = conn.prepare("SELECT clip_id, collection_id FROM clip_collections")?;
        let rows = stmt.query_map([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))?;
        for row in rows {
            let (clip_id, collection_id) = row?;
            collections.entry(clip_id).or_default().push(collection_id);
        }
    }

    let mut tags: LinkMap<String> = HashMap::new();
    {
        let mut stmt = conn.prepare(
            "SELECT ct.clip_id, t.name FROM clip_tags ct JOIN tags t ON t.id = ct.tag_id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (clip_id, name) = row?;
            tags.entry(clip_id).or_default().push(name);
        }
    }

    Ok((collections, tags))
}

fn matches_filter(
    summary: &ClipSummary,
    filter: &ClipFilter,
    tag: Option<&str>,
    archived: &HashSet<i64>,
) -> bool {
    if let Some(kind) = filter.kind
        && summary.kind != kind
    {
        return false;
    }
    if filter.pinned_only && !summary.pinned {
        return false;
    }
    if filter.colors_only && summary.color.is_none() {
        return false;
    }
    if let Some(collection_id) = filter.collection_id
        && !summary.collection_ids.contains(&collection_id)
    {
        return false;
    }
    if let Some(tag) = tag
        && !summary.tags.iter().any(|t| t.eq_ignore_ascii_case(tag))
    {
        return false;
    }
    if let Some(app) = &filter.app
        && !summary
            .source_app
            .as_deref()
            .is_some_and(|actual| actual.to_lowercase().contains(&app.to_lowercase()))
    {
        return false;
    }
    if let Some(before) = filter.before
        && summary.created_at >= before
    {
        return false;
    }
    if let Some(color) = &filter.color
        && !summary
            .color
            .as_deref()
            .is_some_and(|actual| actual.eq_ignore_ascii_case(color))
    {
        return false;
    }
    if let Some(kind) = &filter.content_kind
        && !summary
            .content_kind
            .as_deref()
            .is_some_and(|actual| actual.eq_ignore_ascii_case(kind))
    {
        return false;
    }
    if filter.secrets_only && summary.secret_level.is_none() {
        return false;
    }

    // Archived clips only appear when their own collection is the one selected;
    // that is the whole point of the archive. The search bar can opt back in.
    let viewing_archive = filter
        .collection_id
        .is_some_and(|id| archived.contains(&id));
    if !filter.include_archived
        && !viewing_archive
        && summary.collection_ids.iter().any(|id| archived.contains(id))
    {
        return false;
    }
    true
}

#[cfg(test)]
pub fn test_store() -> (Store, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = Store::open(dir.path()).expect("store");
    (store, dir)
}

#[cfg(test)]
impl Store {
    /// A store on a throwaway directory that cleans itself up with the store.
    pub fn open_in_memory() -> Result<Self> {
        let dir = tempfile::tempdir()?;
        let mut store = Store::open(dir.path())?;
        store.fixture = Some(dir);
        Ok(store)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unique_text_clip(body: &str) -> NewClip {
        NewClip {
            content: ClipContent::Text(body.to_string()),
            // Mirrors the real capture path: hash the payload, never store it.
            content_hash: crate::hashing::of_str(body),
            source_app: Some("test.exe".into()),
            search_text: search::normalize(body),
            preview: body.chars().take(80).collect(),
            html: None,
            rtf: None,
        }
    }

    fn files_clip(paths: &[&str]) -> NewClip {
        let owned: Vec<String> = paths.iter().map(|p| (*p).to_string()).collect();
        let content = ClipContent::Files(owned.clone());
        NewClip {
            content_hash: crate::hashing::of_content(&content),
            content,
            source_app: None,
            search_text: search::normalize(&owned.join(" ")),
            preview: owned.join(", "),
            html: None,
            rtf: None,
        }
    }

    fn image_clip(width: u32, height: u32, seed: u8) -> NewClip {
        let mut rgba = Vec::with_capacity((width * height * 4) as usize);
        for i in 0..(width * height) {
            rgba.extend_from_slice(&[seed, (i % 256) as u8, 200, 255]);
        }
        let content = ClipContent::Image {
            width,
            height,
            rgba,
        };
        NewClip {
            content_hash: crate::hashing::of_content(&content),
            content,
            source_app: None,
            search_text: String::new(),
            preview: format!("Image {width}×{height}"),
            html: None,
            rtf: None,
        }
    }

    fn all(store: &Store) -> ClipPage {
        store.list(&ClipFilter::default(), "", 100, 0)
    }

    #[test]
    fn insert_then_list_returns_the_clip() {
        let (store, _dir) = test_store();
        let outcome = store
            .insert_clip(unique_text_clip("hello clipboard"))
            .unwrap();
        assert!(matches!(outcome, InsertOutcome::Inserted(_)));

        let page = all(&store);
        assert_eq!(page.total, 1);
        assert_eq!(page.items[0].preview, "hello clipboard");
        assert_eq!(page.items[0].kind, ClipKind::Text);
        assert_eq!(page.items[0].char_count, 15);
        assert_eq!(page.items[0].copy_count, 1);
    }

    #[test]
    fn text_content_round_trips() {
        let (store, _dir) = test_store();
        let id = match store.insert_clip(unique_text_clip("secret body")).unwrap() {
            InsertOutcome::Inserted(id) => id,
            _ => panic!("expected insert"),
        };
        assert_eq!(
            store.text_content(id).unwrap().as_deref(),
            Some("secret body")
        );
        assert_eq!(
            store.detail(id).unwrap().text.as_deref(),
            Some("secret body")
        );
    }

    #[test]
    fn body_is_stored_in_plaintext() {
        let (store, dir) = test_store();
        const CANARY: &str = "PLAINTEXT-CANARY-VALUE";
        store.insert_clip(unique_text_clip(CANARY)).unwrap();

        // Fold the WAL back into the main file so the assertion covers every page
        // that has actually been written, not just the checkpointed prefix.
        {
            let conn = store.conn.lock();
            conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
                .unwrap();
        }

        let needle = CANARY.as_bytes();
        let raw = std::fs::read(dir.path().join("copypaste.db")).unwrap();
        assert!(
            raw.windows(needle.len()).any(|window| window == needle),
            "clip bodies are stored as plain text now"
        );
    }

    #[test]
    fn legacy_encrypted_rows_are_migrated_to_plaintext() {
        let dir = tempfile::tempdir().unwrap();
        let key_path = dir.path().join("key.bin");
        let key = crate::crypto::test_key();
        crate::crypto::write_test_key(&key, &key_path).unwrap();

        // First open creates the schema and removes the unused key.
        let store = Store::open(dir.path()).unwrap();
        let id = match store.insert_clip(unique_text_clip("placeholder")).unwrap() {
            InsertOutcome::Inserted(id) => id,
            _ => unreachable!(),
        };
        let db_path = dir.path().join("copypaste.db");
        drop(store);

        // Rewrite the row the way an encrypting build would have stored it.
        let cipher = Cipher::new(&key);
        let body = cipher.seal_for_test("legacy body — ünïcödé").unwrap();
        let preview = cipher.seal_for_test("legacy body").unwrap();
        {
            let conn = Connection::open(&db_path).unwrap();
            conn.execute(
                "UPDATE clips
                    SET preview = ?2, preview_nonce = ?3, body = ?4, body_nonce = ?5
                  WHERE id = ?1",
                params![
                    id,
                    preview.ciphertext,
                    preview.nonce.to_vec(),
                    body.ciphertext,
                    body.nonce.to_vec(),
                ],
            )
            .unwrap();
            conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);").unwrap();
        }
        crate::crypto::write_test_key(&key, &key_path).unwrap();

        let store = Store::open(dir.path()).unwrap();
        assert_eq!(
            store.text_content(id).unwrap().as_deref(),
            Some("legacy body — ünïcödé")
        );
        assert_eq!(store.detail(id).unwrap().summary.preview, "legacy body");
        assert!(!key_path.exists(), "the legacy key is deleted after migration");

        let (nonce, body): (Option<Vec<u8>>, Vec<u8>) = {
            let conn = store.conn.lock();
            conn.query_row(
                "SELECT body_nonce, body FROM clips WHERE id = ?1",
                params![id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap()
        };
        assert!(nonce.is_none(), "no nonce remains on a migrated row");
        assert_eq!(body, "legacy body — ünïcödé".as_bytes());
    }

    #[test]
    fn notes_round_trip_and_are_searchable() {
        let (store, _dir) = test_store();
        let id = match store.insert_clip(unique_text_clip("a link")).unwrap() {
            InsertOutcome::Inserted(id) => id,
            _ => unreachable!(),
        };
        assert!(store.detail(id).unwrap().note.is_none());
        assert!(!all(&store).items[0].has_note);

        let applied = store.set_note(id, Some("  renewal code for the vault  ")).unwrap();
        assert_eq!(applied.as_deref(), Some("renewal code for the vault"));
        let detail = store.detail(id).unwrap();
        assert_eq!(detail.note.as_deref(), Some("renewal code for the vault"));
        assert!(detail.summary.has_note);
        assert_eq!(
            store.list(&ClipFilter::default(), "vault", 10, 0).total,
            1,
            "note text is part of the search index"
        );

        store.set_note(id, Some("   ")).unwrap();
        assert!(store.detail(id).unwrap().note.is_none());
        assert!(!all(&store).items[0].has_note);
        assert_eq!(store.list(&ClipFilter::default(), "vault", 10, 0).total, 0);
    }

    #[test]
    fn editing_text_replaces_the_body_and_index() {
        let (store, _dir) = test_store();
        let id = match store.insert_clip(unique_text_clip("original wording")).unwrap() {
            InsertOutcome::Inserted(id) => id,
            _ => unreachable!(),
        };

        store.set_text(id, "edited wording").unwrap();
        let detail = store.detail(id).unwrap();
        assert_eq!(detail.text.as_deref(), Some("edited wording"));
        assert_eq!(detail.summary.preview, "edited wording");
        assert_eq!(detail.summary.char_count, 14);
        assert!(!detail.has_rich, "stale rich flavours are dropped");
        assert_eq!(store.list(&ClipFilter::default(), "edited", 10, 0).total, 1);
        assert_eq!(store.list(&ClipFilter::default(), "original", 10, 0).total, 0);
    }

    #[test]
    fn editing_text_refuses_a_duplicate_payload() {
        let (store, _dir) = test_store();
        let first = match store.insert_clip(unique_text_clip("first body")).unwrap() {
            InsertOutcome::Inserted(id) => id,
            _ => unreachable!(),
        };
        store.insert_clip(unique_text_clip("second body")).unwrap();

        let error = store.set_text(first, "second body").unwrap_err().to_string();
        assert!(error.contains("another clip"), "got: {error}");
        assert_eq!(store.detail(first).unwrap().text.as_deref(), Some("first body"));
    }

    #[test]
    fn notes_are_stored_in_plaintext() {
        let (store, dir) = test_store();
        let id = match store.insert_clip(unique_text_clip("clip")).unwrap() {
            InsertOutcome::Inserted(id) => id,
            _ => unreachable!(),
        };
        const CANARY: &str = "NOTE-PLAINTEXT-CANARY";
        store.set_note(id, Some(CANARY)).unwrap();
        {
            let conn = store.conn.lock();
            conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);").unwrap();
        }
        let raw = std::fs::read(dir.path().join("copypaste.db")).unwrap();
        assert!(
            raw.windows(CANARY.len()).any(|window| window == CANARY.as_bytes()),
            "notes are stored as plain text now"
        );
    }

    #[test]
    fn duplicate_content_is_promoted_not_duplicated() {
        let (store, _dir) = test_store();
        let first = store.insert_clip(unique_text_clip("repeat me")).unwrap();
        let second = store.insert_clip(unique_text_clip("repeat me")).unwrap();

        assert!(matches!(first, InsertOutcome::Inserted(_)));
        let id = match first {
            InsertOutcome::Inserted(id) => id,
            _ => unreachable!(),
        };
        assert_eq!(second, InsertOutcome::Duplicate(id));

        let page = all(&store);
        assert_eq!(page.total, 1, "the same content must not create two rows");
        assert_eq!(page.items[0].copy_count, 2);
    }

    #[test]
    fn recency_ordering_puts_newest_first() {
        let (store, _dir) = test_store();
        store.insert_clip(unique_text_clip("first")).unwrap();
        store.insert_clip(unique_text_clip("second")).unwrap();

        let page = all(&store);
        assert_eq!(page.items[0].preview, "second");
        assert_eq!(page.items[1].preview, "first");
    }

    #[test]
    fn pinned_clips_float_above_newer_unpinned_ones() {
        let (store, _dir) = test_store();
        let old = match store
            .insert_clip(unique_text_clip("old but pinned"))
            .unwrap()
        {
            InsertOutcome::Inserted(id) => id,
            _ => unreachable!(),
        };
        store
            .insert_clip(unique_text_clip("new and unpinned"))
            .unwrap();

        store.set_pinned(old, true).unwrap();

        let page = all(&store);
        assert_eq!(page.items[0].preview, "old but pinned");
        assert!(page.items[0].pinned);

        let pinned_only = store.list(
            &ClipFilter {
                pinned_only: true,
                ..Default::default()
            },
            "",
            10,
            0,
        );
        assert_eq!(pinned_only.total, 1);
    }

    #[test]
    fn search_matches_body_text_and_respects_paging() {
        let (store, _dir) = test_store();
        for body in ["alpha one", "beta two", "alpha three", "gamma four"] {
            store.insert_clip(unique_text_clip(body)).unwrap();
        }

        let page = store.list(&ClipFilter::default(), "alpha", 10, 0);
        assert_eq!(page.total, 2);

        let first = store.list(&ClipFilter::default(), "alpha", 1, 0);
        let second = store.list(&ClipFilter::default(), "alpha", 1, 1);
        assert_eq!(first.items.len(), 1);
        assert_eq!(second.items.len(), 1);
        assert_ne!(
            first.items[0].id, second.items[0].id,
            "pages must not overlap"
        );
    }

    #[test]
    fn search_is_scoped_by_kind_filter() {
        let (store, _dir) = test_store();
        store
            .insert_clip(unique_text_clip("budget report"))
            .unwrap();
        store
            .insert_clip(files_clip(&["C:\\docs\\budget.xlsx"]))
            .unwrap();

        let text_only = store.list(
            &ClipFilter {
                kind: Some(ClipKind::Text),
                ..Default::default()
            },
            "budget",
            10,
            0,
        );
        assert_eq!(text_only.total, 1);
        assert_eq!(text_only.items[0].kind, ClipKind::Text);

        let files_only = store.list(
            &ClipFilter {
                kind: Some(ClipKind::Files),
                ..Default::default()
            },
            "budget",
            10,
            0,
        );
        assert_eq!(files_only.total, 1);
        assert_eq!(files_only.items[0].kind, ClipKind::Files);
    }

    #[test]
    fn file_clips_expose_their_paths() {
        let (store, _dir) = test_store();
        let id = match store
            .insert_clip(files_clip(&["C:\\a.txt", "D:\\b.png"]))
            .unwrap()
        {
            InsertOutcome::Inserted(id) => id,
            _ => unreachable!(),
        };
        assert_eq!(
            store.file_paths(id).unwrap(),
            vec!["C:\\a.txt", "D:\\b.png"]
        );
        assert_eq!(store.detail(id).unwrap().file_paths.len(), 2);
        assert_eq!(all(&store).items[0].file_count, 2);
    }

    #[test]
    fn attaching_an_animation_marks_the_image_clip() {
        let (store, _dir) = test_store();
        let id = match store.insert_clip(image_clip(8, 8, 3)).unwrap() {
            InsertOutcome::Inserted(id) => id,
            _ => unreachable!(),
        };
        assert!(!all(&store).items[0].animated);
        assert!(store.animated_gif_path(id).is_none());

        store.attach_animated_gif(id, b"GIF89a demo").unwrap();

        assert!(all(&store).items[0].animated);
        assert!(store.animated_gif_path(id).is_some());

        store.delete_many(&[id]).unwrap();
        assert!(store.animated_gif_path(id).is_none(), "deleting the clip removes the animation");
    }

    #[test]
    fn image_clips_write_media_and_survive_reload() {
        let (store, dir) = test_store();
        let image = image_clip(24, 12, 7);
        let id = match store.insert_clip(image).unwrap() {
            InsertOutcome::Inserted(id) => id,
            _ => unreachable!(),
        };

        let page = all(&store);
        let summary = &page.items[0];
        assert_eq!(summary.kind, ClipKind::Image);
        assert_eq!(summary.width, Some(24));
        assert_eq!(summary.height, Some(12));
        let thumb = summary.thumb_file.clone().expect("thumbnail recorded");
        assert!(dir.path().join("media").join(&thumb).is_file());

        // A fresh Store over the same directory must recover the same projection.
        drop(store);
        let reopened = Store::open(dir.path()).unwrap();
        let page = all(&reopened);
        assert_eq!(page.total, 1);
        assert_eq!(page.items[0].thumb_file.as_deref(), Some(thumb.as_str()));

        // And the same pixels must still dedupe, even though the projection was
        // rebuilt from disk on reopen.
        assert!(matches!(
            reopened.insert_clip(image_clip(24, 12, 7)).unwrap(),
            InsertOutcome::Duplicate(duplicate) if duplicate == id
        ));
    }

    #[test]
    fn deleting_an_image_removes_its_files() {
        let (store, dir) = test_store();
        let id = match store.insert_clip(image_clip(16, 16, 3)).unwrap() {
            InsertOutcome::Inserted(id) => id,
            _ => unreachable!(),
        };
        let media_files = std::fs::read_dir(dir.path().join("media")).unwrap().count();
        assert_eq!(media_files, 2, "full image plus thumbnail");

        store.delete(id).unwrap();
        assert_eq!(store.list(&ClipFilter::default(), "", 10, 0).total, 0);
        assert_eq!(
            std::fs::read_dir(dir.path().join("media")).unwrap().count(),
            0,
            "media files must be unlinked with the row"
        );
        assert!(store.detail(id).is_err());
    }

    #[test]
    fn pinned_items_survive_clear_but_can_be_removed_explicitly() {
        let (store, _dir) = test_store();
        let keep = match store.insert_clip(unique_text_clip("keeper")).unwrap() {
            InsertOutcome::Inserted(id) => id,
            _ => unreachable!(),
        };
        store.insert_clip(unique_text_clip("throwaway")).unwrap();
        store.set_pinned(keep, true).unwrap();

        assert_eq!(store.clear(true).unwrap(), 1);
        assert_eq!(all(&store).total, 1);
        assert_eq!(store.clear(false).unwrap(), 1);
        assert_eq!(all(&store).total, 0);
    }

    #[test]
    fn prune_enforces_max_item_count_ignoring_pinned() {
        let (store, _dir) = test_store();
        let keeper = match store
            .insert_clip(unique_text_clip("pinned oldest"))
            .unwrap()
        {
            InsertOutcome::Inserted(id) => id,
            _ => unreachable!(),
        };
        store.set_pinned(keeper, true).unwrap();
        for i in 0..9 {
            store
                .insert_clip(unique_text_clip(&format!("clip {i}")))
                .unwrap();
        }
        assert_eq!(all(&store).total, 10);

        store
            .update_settings(&serde_json::json!({ "maxItems": 3 }))
            .unwrap();
        assert_eq!(store.prune().unwrap(), 6, "9 unpinned minus 3 kept");

        let page = all(&store);
        assert_eq!(page.total, 4, "3 newest unpinned plus the pinned one");
        assert!(page.items.iter().any(|item| item.id == keeper));
        assert_eq!(
            store.list(&ClipFilter::default(), "", 10, 0).items[0].id,
            keeper
        );
    }

    #[test]
    fn prune_by_age_removes_only_stale_unpinned_clips() {
        let (store, _dir) = test_store();
        let pinned = match store.insert_clip(unique_text_clip("old pinned")).unwrap() {
            InsertOutcome::Inserted(id) => id,
            _ => unreachable!(),
        };
        let stale = match store.insert_clip(unique_text_clip("old unpinned")).unwrap() {
            InsertOutcome::Inserted(id) => id,
            _ => unreachable!(),
        };
        store.set_pinned(pinned, true).unwrap();

        // Backdate both rows well beyond the retention window.
        {
            let conn = store.conn.lock();
            let ancient = now_ms() - 400 * 86_400_000;
            conn.execute(
                "UPDATE clips SET updated_at = ?1 WHERE id IN (?2, ?3)",
                params![ancient, pinned, stale],
            )
            .unwrap();
        }
        store.rebuild_index().unwrap();

        store
            .update_settings(&serde_json::json!({ "maxAgeDays": 30 }))
            .unwrap();
        assert_eq!(store.prune().unwrap(), 1);

        let page = all(&store);
        assert_eq!(page.total, 1);
        assert_eq!(page.items[0].id, pinned);
    }

    #[test]
    fn collections_can_be_assigned_and_filtered() {
        let (store, _dir) = test_store();
        let clip = match store
            .insert_clip(unique_text_clip("in a collection"))
            .unwrap()
        {
            InsertOutcome::Inserted(id) => id,
            _ => unreachable!(),
        };
        store
            .insert_clip(unique_text_clip("not in a collection"))
            .unwrap();

        let work = store
            .create_collection("Work", Some("#ff0000".into()))
            .unwrap();
        assert!(
            store.create_collection("work", None).is_err(),
            "names are case-insensitively unique"
        );

        let applied = store.set_clip_collections(clip, &[work.id]).unwrap();
        assert_eq!(applied, vec![work.id]);

        let filtered = store.list(
            &ClipFilter {
                collection_id: Some(work.id),
                ..Default::default()
            },
            "",
            10,
            0,
        );
        assert_eq!(filtered.total, 1);
        assert_eq!(filtered.items[0].id, clip);

        let listed = store.collections().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].clip_count, 1);

        store.rename_collection(work.id, "Office").unwrap();
        assert_eq!(store.collections().unwrap()[0].name, "Office");

        store.delete_collection(work.id).unwrap();
        assert!(store.collections().unwrap().is_empty());
        assert!(
            store
                .list(&ClipFilter::default(), "", 10, 0)
                .items
                .iter()
                .find(|item| item.id == clip)
                .unwrap()
                .collection_ids
                .is_empty(),
            "the projection must drop links to a deleted collection"
        );
    }

    #[test]
    fn tags_are_attached_deduplicated_and_filtered() {
        let (store, _dir) = test_store();
        let clip = match store.insert_clip(unique_text_clip("tagged clip")).unwrap() {
            InsertOutcome::Inserted(id) => id,
            _ => unreachable!(),
        };
        store
            .insert_clip(unique_text_clip("untagged clip"))
            .unwrap();

        let applied = store
            .set_clip_tags(
                clip,
                &["  ops ".into(), "Ops".into(), "".into(), "urgent".into()],
            )
            .unwrap();
        assert_eq!(applied, vec!["ops", "urgent"]);

        let filtered = store.list(
            &ClipFilter {
                tag: Some("URGENT".into()),
                ..Default::default()
            },
            "",
            10,
            0,
        );
        assert_eq!(filtered.total, 1);
        assert_eq!(filtered.items[0].id, clip);

        // Replacing the tag set must drop the orphaned tag row.
        store.set_clip_tags(clip, &["later".into()]).unwrap();
        let names: Vec<String> = store.tags().unwrap().into_iter().map(|t| t.name).collect();
        assert_eq!(names, vec!["later".to_string()]);
    }

    #[test]
    fn archive_collections_hide_their_clips_until_selected() {
        let (store, _dir) = test_store();
        let archived_clip = match store.insert_clip(unique_text_clip("kept forever")).unwrap() {
            InsertOutcome::Inserted(id) => id,
            _ => unreachable!(),
        };
        let normal_clip = match store.insert_clip(unique_text_clip("ordinary clip")).unwrap() {
            InsertOutcome::Inserted(id) => id,
            _ => unreachable!(),
        };

        let vault = store.create_collection("Vault", None).unwrap();
        assert!(!vault.archived, "collections start as ordinary ones");
        store.set_clip_collections(archived_clip, &[vault.id]).unwrap();

        // Before archiving, everything is part of history.
        assert_eq!(all(&store).total, 2);

        store.set_collection_archived(vault.id, true).unwrap();
        let history = all(&store);
        assert_eq!(history.total, 1, "archived clips leave the history view");
        assert_eq!(history.items[0].id, normal_clip);

        let inside = store.list(
            &ClipFilter {
                collection_id: Some(vault.id),
                ..Default::default()
            },
            "",
            10,
            0,
        );
        assert_eq!(inside.total, 1, "selecting the archive reveals its clips");
        assert_eq!(inside.items[0].id, archived_clip);

        let stats = store.stats().unwrap();
        assert_eq!(stats.total, 1);
        assert_eq!(stats.archived, 1);

        store.set_collection_archived(vault.id, false).unwrap();
        assert_eq!(all(&store).total, 2, "unarchiving restores the clips");
        assert_eq!(store.collections().unwrap()[0].archived, false);
    }

    #[test]
    fn clear_history_keeps_archived_clips() {
        let (store, _dir) = test_store();
        let archived_clip = match store.insert_clip(unique_text_clip("archive keeper")).unwrap() {
            InsertOutcome::Inserted(id) => id,
            _ => unreachable!(),
        };
        store.insert_clip(unique_text_clip("ordinary clip")).unwrap();

        let vault = store.create_collection("Vault", None).unwrap();
        store.set_clip_collections(archived_clip, &[vault.id]).unwrap();
        store.set_collection_archived(vault.id, true).unwrap();

        // Even the "delete everything" variant leaves archives alone; the archive
        // collection is how the user manages that content.
        assert_eq!(store.clear(false).unwrap(), 1);
        let remaining = store.list(
            &ClipFilter {
                collection_id: Some(vault.id),
                ..Default::default()
            },
            "",
            10,
            0,
        );
        assert_eq!(remaining.total, 1);
        assert_eq!(remaining.items[0].id, archived_clip);
    }

    #[test]
    fn retention_honours_hours_and_skips_archives() {
        let (store, _dir) = test_store();
        let archived_clip = match store.insert_clip(unique_text_clip("archived stale")).unwrap() {
            InsertOutcome::Inserted(id) => id,
            _ => unreachable!(),
        };
        let stale = match store.insert_clip(unique_text_clip("stale clip")).unwrap() {
            InsertOutcome::Inserted(id) => id,
            _ => unreachable!(),
        };
        let fresh = match store.insert_clip(unique_text_clip("fresh clip")).unwrap() {
            InsertOutcome::Inserted(id) => id,
            _ => unreachable!(),
        };

        let vault = store.create_collection("Vault", None).unwrap();
        store.set_clip_collections(archived_clip, &[vault.id]).unwrap();
        store.set_collection_archived(vault.id, true).unwrap();

        {
            let conn = store.conn.lock();
            let two_hours_ago = now_ms() - 2 * 3_600_000;
            conn.execute(
                "UPDATE clips SET updated_at = ?1 WHERE id IN (?2, ?3)",
                params![two_hours_ago, archived_clip, stale],
            )
            .unwrap();
        }
        store.rebuild_index().unwrap();

        store
            .update_settings(&serde_json::json!({ "maxAgeHours": 1 }))
            .unwrap();

        assert_eq!(store.prune().unwrap(), 1, "only the stale history clip goes");
        let ids: Vec<i64> = store
            .list(&ClipFilter::default(), "", 10, 0)
            .items
            .iter()
            .map(|item| item.id)
            .collect();
        assert_eq!(ids, vec![fresh]);
        let archived = store.list(
            &ClipFilter {
                collection_id: Some(vault.id),
                ..Default::default()
            },
            "",
            10,
            0,
        );
        assert_eq!(archived.total, 1);
    }

    #[test]
    fn search_finds_clips_by_tag_name() {
        let (store, _dir) = test_store();
        let tagged = match store.insert_clip(unique_text_clip("quarterly numbers")).unwrap() {
            InsertOutcome::Inserted(id) => id,
            _ => unreachable!(),
        };
        store.insert_clip(unique_text_clip("unrelated notes")).unwrap();
        store.set_clip_tags(tagged, &["Invoice".into()]).unwrap();

        let page = store.list(&ClipFilter::default(), "invoice", 10, 0);
        assert_eq!(page.total, 1);
        assert_eq!(page.items[0].id, tagged);

        // Removing the tag removes it from the index again.
        store.set_clip_tags(tagged, &[]).unwrap();
        assert_eq!(store.list(&ClipFilter::default(), "invoice", 10, 0).total, 0);
    }

    #[test]
    fn order_by_capture_date_or_last_used() {
        let (store, _dir) = test_store();
        let first = match store.insert_clip(unique_text_clip("older capture")).unwrap() {
            InsertOutcome::Inserted(id) => id,
            _ => unreachable!(),
        };
        let second = match store.insert_clip(unique_text_clip("newer capture")).unwrap() {
            InsertOutcome::Inserted(id) => id,
            _ => unreachable!(),
        };
        // Reusing the older clip moves it ahead of the newer one in "last used".
        store.record_copy(first).unwrap();

        let ids = |store: &Store| -> Vec<i64> {
            store
                .list(&ClipFilter::default(), "", 10, 0)
                .items
                .iter()
                .map(|item| item.id)
                .collect()
        };

        assert_eq!(ids(&store), vec![first, second], "last used, newest first");
        store
            .update_settings(&serde_json::json!({ "sortOrder": "created-desc" }))
            .unwrap();
        assert_eq!(ids(&store), vec![second, first], "captured, newest first");
        store
            .update_settings(&serde_json::json!({ "sortOrder": "created-asc" }))
            .unwrap();
        assert_eq!(ids(&store), vec![first, second], "captured, oldest first");
        store
            .update_settings(&serde_json::json!({ "sortOrder": "updated-asc" }))
            .unwrap();
        assert_eq!(ids(&store), vec![second, first], "last used, oldest first");
    }

    #[test]
    fn image_text_is_recognised_and_searchable() {
        let (store, _dir) = test_store();
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("ocr-sample.png");
        let source = image::open(&fixture)
            .expect("the OCR fixture must decode")
            .to_rgba8();
        let (width, height) = source.dimensions();
        let content = ClipContent::Image {
            width,
            height,
            rgba: source.into_raw(),
        };
        let id = match store
            .insert_clip(NewClip {
                content_hash: crate::hashing::of_content(&content),
                content,
                source_app: None,
                search_text: String::new(),
                preview: format!("Image {width}×{height}"),
                html: None,
                rtf: None,
            })
            .unwrap()
        {
            InsertOutcome::Inserted(id) => id,
            _ => unreachable!(),
        };

        store.run_ocr(id).unwrap();
        let text = store.ocr_text(id).unwrap().unwrap_or_default();
        if text.trim().is_empty() {
            eprintln!("skipping: no OCR engine is installed");
            return;
        }
        assert!(text.to_lowercase().contains("copypaste"), "got {text:?}");

        let page = store.list(&ClipFilter::default(), "copypaste", 10, 0);
        assert_eq!(page.total, 1, "recognised text feeds the search index");
        assert!(page.items[0].has_ocr);
        assert_eq!(store.detail(id).unwrap().ocr_text.as_deref(), Some(text.trim()));
    }

    #[test]
    fn archiving_creates_a_folder_and_unarchiving_removes_it() {
        let (store, _dir) = test_store();
        let archive = store.create_collection("Q3 / invoices", None).unwrap();

        store.set_collection_archived(archive.id, true).unwrap();
        let folder = crate::storage::archive_dir(store.data_dir(), "Q3 / invoices");
        assert!(folder.is_dir(), "an archive gets its own folder");
        assert!(
            folder.ends_with("Q3 - invoices"),
            "path separators are replaced: {}",
            folder.display()
        );

        store.rename_collection(archive.id, "Q3 funds").unwrap();
        assert!(!folder.exists(), "the folder follows a rename");
        let renamed = crate::storage::archive_dir(store.data_dir(), "Q3 funds");
        assert!(renamed.is_dir());

        store.set_collection_archived(archive.id, false).unwrap();
        assert!(!renamed.exists(), "unarchiving removes the empty folder");
        assert!(store.collection_name(archive.id).is_ok());
    }

    #[test]
    fn reveal_path_points_at_stored_files() {
        let (store, _dir) = test_store();
        let image = match store.insert_clip(image_clip(4, 4, 3)).unwrap() {
            InsertOutcome::Inserted(id) => id,
            _ => unreachable!(),
        };
        let image_path = store.reveal_path(image).unwrap();
        assert!(image_path.is_file());
        assert!(image_path.starts_with(store.media_dir()));

        let files = match store
            .insert_clip(files_clip(&["C:\\definitely\\missing\\a.txt"]))
            .unwrap()
        {
            InsertOutcome::Inserted(id) => id,
            _ => unreachable!(),
        };
        // A path that no longer exists falls back to its folder.
        assert_eq!(
            store.reveal_path(files).unwrap(),
            std::path::PathBuf::from("C:\\definitely\\missing")
        );

        let text = match store.insert_clip(unique_text_clip("no file")).unwrap() {
            InsertOutcome::Inserted(id) => id,
            _ => unreachable!(),
        };
        assert!(store.reveal_path(text).is_err());
    }

    #[test]
    fn color_clips_are_recognised_and_filtered() {
        let (store, _dir) = test_store();
        let hex = match store.insert_clip(unique_text_clip("#FF8800")).unwrap() {
            InsertOutcome::Inserted(id) => id,
            _ => unreachable!(),
        };
        let rgb = match store
            .insert_clip(unique_text_clip("rgb(0, 128, 255)"))
            .unwrap()
        {
            InsertOutcome::Inserted(id) => id,
            _ => unreachable!(),
        };
        let prose = match store
            .insert_clip(unique_text_clip("use #FF8800 on the button"))
            .unwrap()
        {
            InsertOutcome::Inserted(id) => id,
            _ => unreachable!(),
        };

        let page = store.list(&ClipFilter::default(), "", 10, 0);
        let color_of = |id: i64| {
            page.items
                .iter()
                .find(|item| item.id == id)
                .and_then(|item| item.color.clone())
        };
        assert_eq!(color_of(hex).as_deref(), Some("#ff8800"));
        assert_eq!(color_of(rgb).as_deref(), Some("#0080ff"));
        assert_eq!(color_of(prose), None, "prose mentioning a color is text");

        let filtered = store.list(
            &ClipFilter {
                colors_only: true,
                ..Default::default()
            },
            "",
            10,
            0,
        );
        assert_eq!(filtered.total, 2);
        assert_eq!(store.stats().unwrap().colors, 2);

        // The detail payload carries the derived value too.
        assert_eq!(
            store.detail(hex).unwrap().summary.color.as_deref(),
            Some("#ff8800")
        );
    }

    #[test]
    fn record_copy_promotes_the_clip() {
        let (store, _dir) = test_store();
        let first = match store
            .insert_clip(unique_text_clip("will be reused"))
            .unwrap()
        {
            InsertOutcome::Inserted(id) => id,
            _ => unreachable!(),
        };
        store.insert_clip(unique_text_clip("newer")).unwrap();
        assert_eq!(all(&store).items[0].preview, "newer");

        store.record_copy(first).unwrap();
        let page = all(&store);
        assert_eq!(page.items[0].id, first);
        assert_eq!(page.items[0].copy_count, 2);
    }

    /// Guards the ordering contract: the list is sorted by `updated_at`, so the
    /// store must never hand two clips the same timestamp. Deliberately does not
    /// sleep, so the writes all land inside one millisecond.
    #[test]
    fn rapid_writes_get_strictly_increasing_timestamps() {
        let (store, _dir) = test_store();
        for index in 0..25 {
            store
                .insert_clip(unique_text_clip(&format!("rapid {index}")))
                .unwrap();
        }

        let page = all(&store);
        assert_eq!(page.total, 25);
        let stamps: Vec<i64> = page.items.iter().map(|item| item.updated_at).collect();
        assert!(
            stamps.windows(2).all(|pair| pair[0] > pair[1]),
            "timestamps must be strictly decreasing down the list, got {stamps:?}"
        );
    }

    /// The user-visible promise behind `next_stamp`: picking a clip promotes it
    /// even when no clock tick separates it from the previous write.
    #[test]
    fn promotion_wins_without_a_clock_tick() {
        let (store, _dir) = test_store();
        let older = match store.insert_clip(unique_text_clip("older")).unwrap() {
            InsertOutcome::Inserted(id) => id,
            _ => unreachable!(),
        };
        store.insert_clip(unique_text_clip("newer")).unwrap();
        assert_eq!(all(&store).items[0].preview, "newer");

        store.record_copy(older).unwrap();
        let page = all(&store);
        assert_eq!(page.items[0].id, older, "a re-copied clip always floats up");
        assert_eq!(page.items[0].copy_count, 2);
    }

    #[test]
    fn stats_reflect_content_mix() {
        let (store, _dir) = test_store();
        store.insert_clip(unique_text_clip("text")).unwrap();
        store.insert_clip(files_clip(&["C:\\x.txt"])).unwrap();
        store.insert_clip(image_clip(16, 16, 1)).unwrap();

        let stats = store.stats().unwrap();
        assert_eq!(stats.total, 3);
        assert_eq!(stats.images, 1);
        assert_eq!(stats.files, 1);
        assert_eq!(stats.pinned, 0);
        assert!(stats.media_bytes > 0);
        assert!(stats.database_bytes > 0);
    }

    #[test]
    fn errors_are_returned_for_missing_ids() {
        let (store, _dir) = test_store();
        assert!(store.detail(999).is_err());
        assert!(store.text_content(999).is_err());
        assert!(store.set_pinned(999, true).is_err());
        assert!(store.record_copy(999).is_err());
        assert!(store.rename_collection(999, "x").is_err());
        assert!(store.delete_collection(999).is_err());
        // Deleting something already gone is not an error, it is a no-op.
        assert_eq!(store.delete_many(&[]).unwrap(), 0);
    }

    #[test]
    fn empty_preview_and_unicode_bodies_are_handled() {
        let (store, _dir) = test_store();
        store.insert_clip(unique_text_clip("")).unwrap();
        store
            .insert_clip(unique_text_clip("日本語 ✓ emoji 🎉"))
            .unwrap();

        let page = all(&store);
        assert_eq!(page.total, 2);
        let unicode = page
            .items
            .iter()
            .find(|item| item.preview.contains("日本語"))
            .expect("unicode preview preserved");
        assert_eq!(
            unicode.char_count,
            "日本語 ✓ emoji 🎉".chars().count() as i64
        );
    }

    #[test]
    fn rich_text_round_trips_alongside_plain_text() {
        let (store, _dir) = test_store();
        let mut clip = unique_text_clip("<b>bold</b>");
        clip.html = Some("<b>bold</b>".into());
        clip.rtf = Some(r"{\rtf1\b bold}".into());
        let id = match store.insert_clip(clip).unwrap() {
            InsertOutcome::Inserted(id) => id,
            _ => unreachable!(),
        };

        let rich = store.rich_text(id).unwrap();
        assert_eq!(rich.html.as_deref(), Some("<b>bold</b>"));
        assert_eq!(rich.rtf.as_deref(), Some(r"{\rtf1\b bold}"));
        assert!(store.detail(id).unwrap().has_rich);

        // The plain body is still there, so a rich-unaware target still works.
        assert_eq!(store.text_content(id).unwrap().as_deref(), Some("<b>bold</b>"));

        let plain = match store.insert_clip(unique_text_clip("plain")).unwrap() {
            InsertOutcome::Inserted(id) => id,
            _ => unreachable!(),
        };
        assert!(store.rich_text(plain).unwrap().html.is_none());
        assert!(!store.detail(plain).unwrap().has_rich);
    }

    #[test]
    fn rich_text_is_stored_in_plaintext() {
        let (store, _dir) = test_store();
        let mut clip = unique_text_clip("secret body");
        clip.html = Some("<i>secret html</i>".into());
        let id = match store.insert_clip(clip).unwrap() {
            InsertOutcome::Inserted(id) => id,
            _ => unreachable!(),
        };
        let conn = store.conn.lock();
        let (html, nonce): (Vec<u8>, Option<Vec<u8>>) = conn
            .query_row(
                "SELECT html, html_nonce FROM clips WHERE id = ?1",
                params![id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        drop(conn);
        assert_eq!(html, b"<i>secret html</i>");
        assert!(nonce.is_none(), "no nonce is written any more");
    }

    #[test]
    fn content_kind_is_derived_for_actionable_text() {
        let (store, _dir) = test_store();
        let id = match store
            .insert_clip(unique_text_clip("https://example.com/a"))
            .unwrap()
        {
            InsertOutcome::Inserted(id) => id,
            _ => unreachable!(),
        };
        assert_eq!(
            all(&store)
                .items
                .iter()
                .find(|item| item.id == id)
                .and_then(|item| item.content_kind.clone())
                .as_deref(),
            Some("url")
        );
    }

    #[test]
    fn image_summaries_carry_an_absolute_path() {
        let (store, _dir) = test_store();
        store.insert_clip(image_clip(4, 4, 9)).unwrap();
        let summary = all(&store).items.into_iter().next().unwrap();
        let path = summary.image_path.expect("images expose their stored file");
        assert!(std::path::Path::new(&path).is_absolute());
        assert!(std::path::Path::new(&path).is_file());
    }

    #[test]
    fn saved_clips_become_files_with_unique_names() {
        let (store, _dir) = test_store();
        let out = tempfile::tempdir().unwrap();

        let text = match store
            .insert_clip(unique_text_clip("Quarterly report\nsecond line"))
            .unwrap()
        {
            InsertOutcome::Inserted(id) => id,
            _ => unreachable!(),
        };
        let written = store.save_clip(text, out.path()).unwrap();
        assert_eq!(written.len(), 1);
        assert!(written[0].ends_with("Quarterly report.txt"));
        assert_eq!(
            std::fs::read_to_string(&written[0]).unwrap(),
            "Quarterly report\nsecond line"
        );

        // Saving the same clip again must not overwrite the first file.
        let again = store.save_clip(text, out.path()).unwrap();
        assert_ne!(again[0], written[0]);
        assert!(again[0].ends_with("Quarterly report (2).txt"));

        let image = match store.insert_clip(image_clip(3, 3, 5)).unwrap() {
            InsertOutcome::Inserted(id) => id,
            _ => unreachable!(),
        };
        let written = store.save_clip(image, out.path()).unwrap();
        assert!(written[0].ends_with(".png"));
        assert!(std::path::Path::new(&written[0]).is_file());
    }

    #[test]
    fn archives_are_a_single_slot_while_collections_stack() {
        let (store, _dir) = test_store();
        let first = store.create_collection("Photos", None).unwrap();
        let second = store.create_collection("Screenshots", None).unwrap();
        store.set_collection_archived(first.id, true).unwrap();
        store.set_collection_archived(second.id, true).unwrap();
        let normal = store.create_collection("Reference", None).unwrap();

        let id = match store.insert_clip(unique_text_clip("both")).unwrap() {
            InsertOutcome::Inserted(id) => id,
            _ => unreachable!(),
        };

        // Ordinary collections stack.
        store.set_clip_collections(id, &[normal.id]).unwrap();
        let detail = store.detail(id).unwrap();
        assert_eq!(detail.summary.collection_ids, vec![normal.id]);

        // Archiving replaces the previous archive instead of adding one.
        store.set_clip_archive(id, Some(first.id)).unwrap();
        assert_eq!(store.detail(id).unwrap().summary.collection_ids.len(), 2);
        store.set_clip_archive(id, Some(second.id)).unwrap();
        let ids = store.detail(id).unwrap().summary.collection_ids;
        assert_eq!(ids.iter().filter(|id| **id != normal.id).count(), 1);
        assert!(ids.contains(&second.id));
        assert!(!ids.contains(&first.id));

        // Ticking a collection later must not disturb the archive slot.
        store.set_clip_collections(id, &[]).unwrap();
        let ids = store.detail(id).unwrap().summary.collection_ids;
        assert_eq!(ids, vec![second.id]);

        // And clearing it brings the clip back into the ordinary history.
        store.set_clip_archive(id, None).unwrap();
        assert!(store.detail(id).unwrap().summary.collection_ids.is_empty());
        assert!(store.set_clip_archive(id, Some(normal.id)).is_err());
    }

    #[test]
    fn bulk_filing_into_an_archive_keeps_one_archive_per_clip() {
        let (store, _dir) = test_store();
        let photos = store.create_collection("Photos", None).unwrap();
        let shots = store.create_collection("Shots", None).unwrap();
        store.set_collection_archived(photos.id, true).unwrap();
        store.set_collection_archived(shots.id, true).unwrap();

        let ids: Vec<i64> = ["a", "b"]
            .iter()
            .map(|body| match store.insert_clip(unique_text_clip(body)).unwrap() {
                InsertOutcome::Inserted(id) => id,
                _ => unreachable!(),
            })
            .collect();
        store.add_to_collection_many(&ids, photos.id).unwrap();
        store.add_to_collection_many(&ids, shots.id).unwrap();
        for id in ids {
            let collections = store.detail(id).unwrap().summary.collection_ids;
            assert_eq!(collections, vec![shots.id], "one archive, not two");
        }
    }

    #[test]
    fn exporting_several_clips_writes_each_of_them() {
        let (store, _dir) = test_store();
        let out = tempfile::tempdir().unwrap();
        let ids: Vec<i64> = ["first export", "second export", "third export"]
            .iter()
            .map(|body| match store.insert_clip(unique_text_clip(body)).unwrap() {
                InsertOutcome::Inserted(id) => id,
                _ => unreachable!(),
            })
            .collect();

        let written = store.save_clips(&ids, out.path()).unwrap();
        assert_eq!(written.len(), 3);
        assert!(written.iter().all(|path| std::path::Path::new(path).is_file()));
        assert_eq!(store.save_clips(&[], out.path()).unwrap().len(), 0);
        assert!(store.save_clips(&[9_999], out.path()).is_err());
    }

    #[test]
    fn tags_can_be_renamed_merged_recolored_and_deleted() {
        let (store, _dir) = test_store();
        let first = match store.insert_clip(unique_text_clip("first")).unwrap() {
            InsertOutcome::Inserted(id) => id,
            _ => unreachable!(),
        };
        let second = match store.insert_clip(unique_text_clip("second")).unwrap() {
            InsertOutcome::Inserted(id) => id,
            _ => unreachable!(),
        };
        store
            .set_clip_tags(first, &["invoice".into(), "q3".into()])
            .unwrap();
        store.set_clip_tags(second, &["Invoices".into()]).unwrap();

        let tags = store.tags().unwrap();
        let invoice = tags.iter().find(|tag| tag.name == "invoice").unwrap();
        store.set_tag_color(invoice.id, Some("#00FF00")).unwrap();
        let recolored = store.tags().unwrap();
        assert_eq!(
            recolored
                .iter()
                .find(|tag| tag.name == "invoice")
                .and_then(|tag| tag.color.clone())
                .as_deref(),
            Some("#00ff00")
        );
        assert!(store.set_tag_color(invoice.id, Some("not a color")).is_err());

        // Renaming onto an existing name merges the two tags.
        store.rename_tag(invoice.id, "Invoices").unwrap();
        let merged = store.tags().unwrap();
        assert_eq!(
            merged.iter().filter(|tag| tag.name.eq_ignore_ascii_case("invoices")).count(),
            1
        );
        let detail = store.detail(first).unwrap();
        assert_eq!(detail.summary.tags, vec!["Invoices".to_string(), "q3".to_string()]);

        // Renaming re-indexes: the old name no longer matches, the new one does.
        assert_eq!(store.list(&ClipFilter::default(), "invoice", 10, 0).total, 2);

        let q3 = store.tags().unwrap().into_iter().find(|tag| tag.name == "q3").unwrap();
        store.delete_tag(q3.id).unwrap();
        assert_eq!(store.tags().unwrap().len(), 1);
        assert!(store.detail(first).unwrap().summary.tags == vec!["Invoices".to_string()]);
    }

    #[test]
    fn bulk_actions_apply_to_several_clips() {
        let (store, _dir) = test_store();
        let ids: Vec<i64> = ["one", "two", "three"]
            .iter()
            .map(|body| match store.insert_clip(unique_text_clip(body)).unwrap() {
                InsertOutcome::Inserted(id) => id,
                _ => unreachable!(),
            })
            .collect();
        let collection = store.create_collection("Triaged", None).unwrap();

        assert_eq!(store.set_pinned_many(&ids[..2], true).unwrap(), 2);
        assert_eq!(
            all(&store).items.iter().filter(|item| item.pinned).count(),
            2
        );

        assert_eq!(
            store
                .add_tags_many(&ids, &["bulk".into(), "bulk".into()])
                .unwrap(),
            3
        );
        assert!(all(&store)
            .items
            .iter()
            .all(|item| item.tags == vec!["bulk".to_string()]));

        assert_eq!(
            store.add_to_collection_many(&ids[1..], collection.id).unwrap(),
            2
        );
        let in_collection = store
            .list(
                &ClipFilter {
                    collection_id: Some(collection.id),
                    ..Default::default()
                },
                "",
                10,
                0,
            )
            .total;
        assert_eq!(in_collection, 2);
        assert!(store.add_to_collection_many(&ids, 9_999).is_err());
    }

    #[test]
    fn archives_can_be_included_or_excluded_from_listing() {
        let (store, _dir) = test_store();
        let archive = store.create_collection("Old", None).unwrap();
        store.set_collection_archived(archive.id, true).unwrap();
        match store.insert_clip(unique_text_clip("filed away")).unwrap() {
            InsertOutcome::Inserted(id) => {
                store.set_clip_archive(id, Some(archive.id)).unwrap();
            }
            _ => unreachable!(),
        }
        store.insert_clip(unique_text_clip("in the open")).unwrap();

        assert_eq!(all(&store).total, 1, "archived clips stay hidden by default");
        let everything = store.list(
            &ClipFilter {
                include_archived: true,
                ..Default::default()
            },
            "",
            10,
            0,
        );
        assert_eq!(everything.total, 2);
    }

    #[test]
    fn filters_narrow_by_app_before_and_color() {
        let (store, _dir) = test_store();
        let older = match store.insert_clip(unique_text_clip("#112233")).unwrap() {
            InsertOutcome::Inserted(id) => id,
            _ => unreachable!(),
        };
        store.insert_clip(unique_text_clip("later text")).unwrap();

        let by_app = store.list(
            &ClipFilter {
                app: Some("TEST.EXE".into()),
                ..Default::default()
            },
            "",
            10,
            0,
        );
        assert_eq!(by_app.total, 2, "the app match is case-insensitive");

        let cutoff = store.detail(older).unwrap().summary.created_at + 1;
        let by_before = store.list(
            &ClipFilter {
                before: Some(cutoff),
                ..Default::default()
            },
            "",
            10,
            0,
        );
        assert_eq!(by_before.total, 1);

        let by_color = store.list(
            &ClipFilter {
                color: Some("#112233".into()),
                ..Default::default()
            },
            "",
            10,
            0,
        );
        assert_eq!(by_color.total, 1);
        assert_eq!(by_color.items[0].id, older);
    }

    #[test]
    fn fresh_databases_leave_the_setup_wizard_pending() {
        let store = Store::open_in_memory().unwrap();
        assert!(!store.settings().setup_complete);
    }

    #[test]
    fn existing_settings_mark_the_setup_wizard_complete() {
        let dir = tempfile::tempdir().unwrap();
        {
            let conn = Connection::open(dir.path().join("copypaste.db")).unwrap();
            conn.execute_batch(SCHEMA).unwrap();
            conn.execute(
                "INSERT INTO settings (key, value) VALUES ('autoPaste', 'true')",
                [],
            )
            .unwrap();
        }

        let store = Store::open(dir.path()).unwrap();
        assert!(
            store.settings().setup_complete,
            "an upgraded installation must not be shown the wizard"
        );
    }
}
