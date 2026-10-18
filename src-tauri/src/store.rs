//! SQLite-backed clip storage.
//!
//! Layout notes:
//!
//! * Sensitive columns (`preview`, `body`, `files`) hold AES-256-GCM ciphertext.
//!   Non-sensitive bookkeeping (kind, sizes, timestamps, flags) stays plaintext so
//!   it can be sorted and counted without decrypting anything.
//! * Reads never touch SQLite on the hot path. Startup materialises a compact
//!   in-memory projection of every clip (metadata + normalised search text), and
//!   list/search/filter all run against that. Writes go to SQLite first, then the
//!   projection is refreshed. At clipboard-history scale this trades a few MB of
//!   RAM for search that responds within a keystroke.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use parking_lot::{Mutex, RwLock};
use rusqlite::{Connection, OptionalExtension, params};

use crate::crypto::{Cipher, load_or_create_master_key};
use crate::error::{AppError, Result};
use crate::media::{self, ImageFiles};
use crate::models::{
    ClipContent, ClipDetail, ClipFilter, ClipKind, ClipPage, ClipRow, ClipSummary, Collection,
    NewClip, Stats, Tag,
};
use crate::search::{self, Query};
use crate::settings::{self, Settings};

const SCHEMA_VERSION: &str = "1";

/// Marks the one-time reset of `hideOnBlur` to its new default.
const FOCUS_HIDE_RESET_KEY: &str = "focus_hide_reset_v2";

/// Clips larger than this are indexed by preview only, so that startup does not
/// have to decrypt an unbounded amount of text.
const SEARCH_BODY_LIMIT: i64 = 256 * 1024;

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
  copy_count     INTEGER NOT NULL DEFAULT 1
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_clips_hash  ON clips(content_hash);
CREATE INDEX        IF NOT EXISTS idx_clips_order ON clips(pinned, updated_at DESC);
CREATE INDEX        IF NOT EXISTS idx_clips_kind  ON clips(kind);

CREATE TABLE IF NOT EXISTS collections (
  id         INTEGER PRIMARY KEY AUTOINCREMENT,
  name       TEXT    NOT NULL,
  color      TEXT,
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
const CLIP_COLUMNS: &str = "id, kind, byte_size, char_count, file_count, \
     preview, preview_nonce, body, body_nonce, files, files_nonce, \
     image_path, thumb_path, width, height, source_app, pinned, created_at, updated_at, copy_count";

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
}

pub struct Store {
    conn: Mutex<Connection>,
    cipher: Cipher,
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
        let key = load_or_create_master_key(&data_dir.join("key.bin"))?;

        let conn = Connection::open(&db_path)?;
        // WAL keeps the capture thread from blocking the UI thread on reads.
        let _mode: String = conn.query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))?;
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA synchronous = NORMAL;")?;
        migrate(&conn)?;

        let loaded = settings::load(&conn)?;
        let store = Store {
            conn: Mutex::new(conn),
            cipher: Cipher::new(&key),
            state: RwLock::new(State {
                settings: loaded,
                clips: HashMap::new(),
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

    pub fn data_dir(&self) -> &Path {
        self.db_path.parent().unwrap_or(Path::new("."))
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
        let (rows, collections, tags) = {
            let conn = self.conn.lock();
            let rows = query_clips(&conn, None)?;
            let (collections, tags) = load_all_links(&conn)?;
            (rows, collections, tags)
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
        let preview = match (&row.preview, &row.preview_nonce) {
            (Some(ct), Some(nonce)) => self.cipher.open_str(nonce, ct).unwrap_or_default(),
            _ => String::new(),
        };
        let search = self.compute_search_text(&row, &preview)?;

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
            collection_ids,
            tags,
        };

        Ok(IndexedClip { summary, search })
    }

    fn compute_search_text(&self, row: &ClipRow, preview: &str) -> Result<String> {
        let raw = match row.kind {
            ClipKind::Text => {
                if row.byte_size <= SEARCH_BODY_LIMIT {
                    match (&row.body, &row.body_nonce) {
                        (Some(ct), Some(nonce)) => self
                            .cipher
                            .open_str(nonce, ct)
                            .unwrap_or_else(|_| preview.to_string()),
                        _ => preview.to_string(),
                    }
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
        Ok(search::normalize(&raw))
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
            .filter(|entry| matches_filter(&entry.summary, filter, tag))
            .filter_map(|entry| {
                if parsed.is_empty() {
                    Some((entry, 0))
                } else {
                    search::score(&parsed, &entry.search).map(|score| (entry, score))
                }
            })
            .collect();

        // Pinned first, then relevance, then recency. The final id comparison makes
        // the ordering total, so paging is stable.
        matched.sort_by(|a, b| {
            b.0.summary
                .pinned
                .cmp(&a.0.summary.pinned)
                .then(b.1.cmp(&a.1))
                .then(b.0.summary.updated_at.cmp(&a.0.summary.updated_at))
                .then(b.0.summary.id.cmp(&a.0.summary.id))
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
            ClipKind::Text => match (&row.body, &row.body_nonce) {
                (Some(ct), Some(nonce)) => Some(self.cipher.open_str(nonce, ct)?),
                _ => None,
            },
            _ => None,
        };
        let file_paths = if row.kind == ClipKind::Files {
            self.files_for_row(&row)?
        } else {
            Vec::new()
        };

        Ok(ClipDetail {
            summary,
            text,
            file_paths,
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
        match (&row.body, &row.body_nonce) {
            (Some(ct), Some(nonce)) => Ok(Some(self.cipher.open_str(nonce, ct)?)),
            _ => Ok(None),
        }
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
        match (&row.files, &row.files_nonce) {
            (Some(ct), Some(nonce)) => {
                let json = self.cipher.open_str(nonce, ct)?;
                Ok(serde_json::from_str(&json).unwrap_or_default())
            }
            _ => Ok(Vec::new()),
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

        let inserted = {
            let conn = self.conn.lock();
            let now = next_stamp(&conn)?;
            insert_row(&conn, &self.cipher, &new, now, media.as_ref())
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

    /// Clears history. Pinned clips survive unless `keep_pinned` is false.
    pub fn clear(&self, keep_pinned: bool) -> Result<usize> {
        let ids: Vec<i64> = {
            let conn = self.conn.lock();
            let sql = if keep_pinned {
                "SELECT id FROM clips WHERE pinned = 0"
            } else {
                "SELECT id FROM clips"
            };
            let mut stmt = conn.prepare(sql)?;
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
                let mut stmt =
                    conn.prepare("SELECT id FROM clips WHERE pinned = 0 AND updated_at < ?1")?;
                let rows = stmt.query_map(params![cutoff], |row| row.get::<_, i64>(0))?;
                for row in rows {
                    victims.push(row?);
                }
            }

            if settings.max_items > 0 {
                // Pinned clips are exempt from, and do not count against, the cap.
                let mut stmt = conn.prepare(
                    "SELECT id FROM clips WHERE pinned = 0
                      ORDER BY updated_at DESC, id DESC
                      LIMIT -1 OFFSET ?1",
                )?;
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
            "SELECT c.id, c.name, c.color, c.created_at,
                    (SELECT COUNT(*) FROM clip_collections cc WHERE cc.collection_id = c.id)
               FROM collections c
              ORDER BY c.name COLLATE NOCASE",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(Collection {
                id: row.get(0)?,
                name: row.get(1)?,
                color: row.get(2)?,
                created_at: row.get(3)?,
                clip_count: row.get(4)?,
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
            "INSERT INTO collections (name, color, created_at) VALUES (?1, ?2, ?3)",
            params![name, color, created_at],
        )?;
        Ok(Collection {
            id: conn.last_insert_rowid(),
            name: name.to_string(),
            color,
            clip_count: 0,
            created_at,
        })
    }

    pub fn rename_collection(&self, id: i64, name: &str) -> Result<()> {
        let name = name.trim();
        if name.is_empty() {
            return Err(AppError::msg("collection name cannot be empty"));
        }
        let conn = self.conn.lock();
        let changed = conn.execute(
            "UPDATE collections SET name = ?2 WHERE id = ?1",
            params![id, name],
        )?;
        if changed == 0 {
            return Err(AppError::NotFound(format!("collection {id}")));
        }
        Ok(())
    }

    pub fn delete_collection(&self, id: i64) -> Result<()> {
        {
            let conn = self.conn.lock();
            let changed = conn.execute("DELETE FROM collections WHERE id = ?1", params![id])?;
            if changed == 0 {
                return Err(AppError::NotFound(format!("collection {id}")));
            }
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

    pub fn set_clip_collections(&self, clip_id: i64, ids: &[i64]) -> Result<Vec<i64>> {
        {
            let mut conn = self.conn.lock();
            let tx = conn.transaction()?;
            tx.execute(
                "DELETE FROM clip_collections WHERE clip_id = ?1",
                params![clip_id],
            )?;
            for collection_id in ids {
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
        if let Some(entry) = self.state.write().clips.get_mut(&clip_id) {
            entry.summary.collection_ids = applied.clone();
        }
        Ok(applied)
    }

    // -----------------------------------------------------------------------
    // Tags
    // -----------------------------------------------------------------------

    pub fn tags(&self) -> Result<Vec<Tag>> {
        let conn = self.conn.lock();
        let mut stmt = conn.prepare(
            "SELECT t.id, t.name,
                    (SELECT COUNT(*) FROM clip_tags ct WHERE ct.tag_id = t.id)
               FROM tags t
              ORDER BY t.name COLLATE NOCASE",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(Tag {
                id: row.get(0)?,
                name: row.get(1)?,
                clip_count: row.get(2)?,
            })
        })?;
        Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?)
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
        if let Some(entry) = self.state.write().clips.get_mut(&clip_id) {
            entry.summary.tags = applied.clone();
        }
        Ok(applied)
    }

    // -----------------------------------------------------------------------
    // Stats
    // -----------------------------------------------------------------------

    pub fn stats(&self) -> Result<Stats> {
        let state = self.state.read();
        let mut stats = Stats {
            total: state.clips.len() as i64,
            pinned: 0,
            images: 0,
            files: 0,
            media_bytes: 0,
            database_bytes: 0,
            encryption: "AES-256-GCM (key sealed with Windows DPAPI)".to_string(),
        };
        for entry in state.clips.values() {
            if entry.summary.pinned {
                stats.pinned += 1;
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

    conn.execute(
        "INSERT INTO meta (key, value) VALUES ('schema_version', ?1)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value",
        params![SCHEMA_VERSION],
    )?;
    Ok(())
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
    cipher: &Cipher,
    new: &NewClip,
    now: i64,
    media: Option<&ImageFiles>,
) -> Result<i64> {
    let preview = cipher.seal_str(&new.preview)?;

    let (body, body_nonce) = match &new.content {
        ClipContent::Text(text) => {
            let sealed = cipher.seal_str(text)?;
            (Some(sealed.ciphertext), Some(sealed.nonce.to_vec()))
        }
        _ => (None, None),
    };

    let (files, files_nonce, file_count) = match &new.content {
        ClipContent::Files(paths) => {
            let json = serde_json::to_string(paths)?;
            let sealed = cipher.seal_str(&json)?;
            (
                Some(sealed.ciphertext),
                Some(sealed.nonce.to_vec()),
                paths.len() as i64,
            )
        }
        _ => (None, None, 0),
    };

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
            pinned, created_at, updated_at, copy_count
         ) VALUES (
            ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11,
            ?12, ?13, ?14, ?15, ?16, 0, ?17, ?17, 1
         )",
        params![
            new.content.kind().as_str(),
            new.content_hash,
            byte_size,
            char_count,
            file_count,
            preview.ciphertext,
            preview.nonce.to_vec(),
            body,
            body_nonce,
            files,
            files_nonce,
            media.map(|m| m.image.as_str()),
            media.map(|m| m.thumb.as_str()),
            width,
            height,
            new.source_app,
            now,
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

fn matches_filter(summary: &ClipSummary, filter: &ClipFilter, tag: Option<&str>) -> bool {
    if let Some(kind) = filter.kind
        && summary.kind != kind
    {
        return false;
    }
    if filter.pinned_only && !summary.pinned {
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
    fn text_content_round_trips_through_encryption() {
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
    fn body_is_not_stored_in_plaintext() {
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
        for name in ["copypaste.db", "copypaste.db-wal"] {
            let Ok(raw) = std::fs::read(dir.path().join(name)) else {
                continue;
            };
            assert!(
                !raw.windows(needle.len()).any(|window| window == needle),
                "clip body leaked as plaintext into {name}"
            );
        }
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
        assert!(stats.encryption.contains("AES-256-GCM"));
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
}
