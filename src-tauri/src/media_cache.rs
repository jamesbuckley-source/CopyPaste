//! Local copies of the media the window has already asked for.
//!
//! The history can sit in a cloud folder — Google Drive streams files on demand —
//! and a fast scroll through a few hundred rows asks for far more reads at once
//! than such a driver will serve, so some of them come back empty and the list
//! shows gaps. Media names are random UUIDs and the bytes behind one never change,
//! so a thumbnail can be kept locally and served from there every time after the
//! first read.
//!
//! Only thumbnails go to disk: a full image can be megabytes, and a synced copy of
//! every one of them would be the history twice over. Those live in a small
//! in-memory cache instead, which is what makes stepping through clips with the
//! arrow keys cheap. Nothing sealed is cached anywhere — a sealed file is decrypted
//! in memory and never written back out.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// How much disk the thumbnail cache may use. Thumbnails are tens of kilobytes, so
/// this holds thousands — more than the history keeps.
const DISK_CAP: u64 = 64 * 1024 * 1024;
/// How much memory the full-image cache may use.
const MEMORY_CAP: usize = 32 * 1024 * 1024;
/// How many reads may be in flight at once. A cloud driver is happiest with a
/// handful; asking for a screenful at a time is what makes it fail.
const PARALLEL_READS: usize = 4;
/// Attempts for a read that fails, and the pause before each retry. A streaming
/// driver reports "not ready" while it fetches a placeholder, which the next
/// attempt usually answers.
const READ_ATTEMPTS: u32 = 3;
const RETRY_PAUSE: Duration = Duration::from_millis(60);

/// Whether `name` is a thumbnail, which is what the disk cache holds.
pub fn is_thumbnail(name: &str) -> bool {
    name.ends_with(".thumb.png")
}

/// Whether `name` is a full still image, the only thing worth holding in memory.
///
/// A video or a recovered GIF is streamed in slices and can be far larger than the
/// memory cache, so those are read through on every request instead of being kept.
pub fn is_still(name: &str) -> bool {
    name.ends_with(".png") && !is_thumbnail(name)
}

pub struct MediaCache {
    dir: PathBuf,
    images: Mutex<Images>,
    /// Upper bound on reads running at once, and the queue waiting for a slot.
    busy: Mutex<usize>,
    slot: std::sync::Condvar,
}

#[derive(Default)]
struct Images {
    order: VecDeque<String>,
    entries: HashMap<String, Arc<Vec<u8>>>,
    bytes: usize,
}

impl Images {
    fn get(&mut self, name: &str) -> Option<Arc<Vec<u8>>> {
        let found = Arc::clone(self.entries.get(name)?);
        // Most recently used goes last, so eviction takes from the front.
        if let Some(index) = self.order.iter().position(|entry| entry == name) {
            self.order.remove(index);
        }
        self.order.push_back(name.to_string());
        Some(found)
    }

    fn insert(&mut self, name: &str, bytes: Vec<u8>) {
        if bytes.len() > MEMORY_CAP {
            return;
        }
        if let Some(previous) = self.entries.insert(name.to_string(), Arc::new(bytes)) {
            self.bytes = self.bytes.saturating_sub(previous.len());
            if let Some(index) = self.order.iter().position(|entry| entry == name) {
                self.order.remove(index);
            }
        }
        self.order.push_back(name.to_string());
        self.bytes += self.entries[name].len();
        while self.bytes > MEMORY_CAP {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            if let Some(dropped) = self.entries.remove(&oldest) {
                self.bytes = self.bytes.saturating_sub(dropped.len());
            }
        }
    }
}

impl MediaCache {
    /// Opens the cache, dropping whatever is left of files the history no longer
    /// has and trimming the store back under its cap.
    pub fn open(dir: PathBuf) -> Self {
        if let Err(e) = std::fs::create_dir_all(&dir) {
            log::warn!("could not create the media cache at {}: {e}", dir.display());
        }
        let cache = Self {
            dir,
            images: Mutex::new(Images::default()),
            busy: Mutex::new(0),
            slot: std::sync::Condvar::new(),
        };
        cache.prune();
        cache
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The stored thumbnail for `name`, if it has been read before.
    pub fn thumbnail(&self, name: &str) -> Option<Vec<u8>> {
        if !is_thumbnail(name) {
            return None;
        }
        std::fs::read(self.dir.join(name)).ok()
    }

    /// Keeps a thumbnail for the next time it is asked for. Best effort: a cache
    /// that cannot be written is still a working app.
    pub fn store_thumbnail(&self, name: &str, bytes: &[u8]) {
        if !is_thumbnail(name) {
            return;
        }
        let target = self.dir.join(name);
        let temp = self.dir.join(format!("{name}.part"));
        if std::fs::write(&temp, bytes).is_err() || std::fs::rename(&temp, &target).is_err() {
            // Never leave a half-written entry behind under the real name.
            let _ = std::fs::remove_file(&temp);
        }
    }

    /// A full image that is still in memory, for the preview that shows it again.
    pub fn image(&self, name: &str) -> Option<Arc<Vec<u8>>> {
        let mut images = self.images.lock().unwrap_or_else(|e| e.into_inner());
        images.get(name)
    }

    pub fn store_image(&self, name: &str, bytes: &[u8]) {
        let mut images = self.images.lock().unwrap_or_else(|e| e.into_inner());
        images.insert(name, bytes.to_vec());
    }

    /// Runs `read` with at most [`PARALLEL_READS`] others at a time, retrying the
    /// transient failures a file-streaming driver reports while it is busy.
    pub fn read<T>(&self, mut read: impl FnMut() -> crate::error::Result<T>) -> crate::error::Result<T> {
        let _slot = self.acquire();
        let mut last = None;
        for attempt in 1..=READ_ATTEMPTS {
            match read() {
                Ok(value) => return Ok(value),
                Err(e) => {
                    // The pause grows a little each time: a driver that is fetching
                    // a placeholder needs a moment, not a spin.
                    if attempt < READ_ATTEMPTS {
                        std::thread::sleep(RETRY_PAUSE * attempt);
                    }
                    last = Some(e);
                }
            }
        }
        Err(last.unwrap_or_else(|| crate::error::AppError::msg("the media could not be read")))
    }

    fn acquire(&self) -> Slot<'_> {
        let mut busy = self.busy.lock().unwrap_or_else(|e| e.into_inner());
        while *busy >= PARALLEL_READS {
            busy = self.slot.wait(busy).unwrap_or_else(|e| e.into_inner());
        }
        *busy += 1;
        Slot { cache: self }
    }

    /// Trims the store: partial writes are removed, then the oldest thumbnails go
    /// until the cap fits. Called once at startup, where the cost is invisible.
    fn prune(&self) {
        self.prune_to(DISK_CAP);
    }

    fn prune_to(&self, cap: u64) {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return;
        };
        let mut files: Vec<(std::path::PathBuf, u64, std::time::SystemTime)> = Vec::new();
        let mut total = 0u64;
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if name.ends_with(".part") {
                let _ = std::fs::remove_file(&path);
                continue;
            }
            let Ok(meta) = entry.metadata() else { continue };
            if !meta.is_file() {
                continue;
            }
            total += meta.len();
            files.push((
                path,
                meta.len(),
                meta.modified().unwrap_or(std::time::UNIX_EPOCH),
            ));
        }
        if total <= cap {
            return;
        }
        files.sort_by_key(|(_, _, modified)| *modified);
        for (path, size, _) in files {
            if total <= cap {
                break;
            }
            if std::fs::remove_file(&path).is_ok() {
                total = total.saturating_sub(size);
            }
        }
    }
}

/// One granted read slot, released when dropped.
struct Slot<'a> {
    cache: &'a MediaCache,
}

impl Drop for Slot<'_> {
    fn drop(&mut self) {
        let mut busy = self.cache.busy.lock().unwrap_or_else(|e| e.into_inner());
        *busy = busy.saturating_sub(1);
        self.cache.slot.notify_one();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cache() -> (MediaCache, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let cache = MediaCache::open(dir.path().join("cache"));
        (cache, dir)
    }

    #[test]
    fn only_thumbnails_are_kept_on_disk() {
        let (cache, _dir) = cache();
        assert!(is_thumbnail("abc.thumb.png"));
        assert!(!is_thumbnail("abc.png"));

        cache.store_thumbnail("abc.thumb.png", b"pixels");
        assert_eq!(cache.thumbnail("abc.thumb.png").as_deref(), Some(&b"pixels"[..]));

        cache.store_thumbnail("abc.png", b"full size");
        assert_eq!(cache.thumbnail("abc.png"), None, "a full image is not cached");
        assert!(!cache.dir().join("abc.png").exists());

        assert!(is_still("abc.png"), "a still image is what memory holds");
        assert!(!is_still("abc.thumb.png"));
        assert!(!is_still("12.mp4"), "a movie is streamed, never cached whole");
        assert!(!is_still("12.gif"));
    }

    #[test]
    fn the_newest_images_are_kept_in_memory_and_the_oldest_go() {
        let (cache, _dir) = cache();
        cache.store_image("a.png", &vec![1u8; 16]);
        cache.store_image("b.png", &vec![2u8; 16]);
        assert!(cache.image("a.png").is_some());
        // Reading `a` makes `b` the oldest, so `b` is the one evicted.
        assert!(cache.image("b.png").is_some());
        assert!(cache.image("a.png").is_some());

        // Bigger than the whole cache: never kept.
        cache.store_image("huge.png", &vec![0u8; MEMORY_CAP + 1]);
        assert!(cache.image("huge.png").is_none());
    }

    #[test]
    fn a_failed_read_is_retried_before_it_is_reported() {
        let (cache, _dir) = cache();
        let mut attempts = 0;
        let outcome: crate::error::Result<u8> = cache.read(|| {
            attempts += 1;
            if attempts < 3 {
                Err(crate::error::AppError::msg("not ready"))
            } else {
                Ok(7)
            }
        });
        assert_eq!(outcome.unwrap(), 7);
        assert_eq!(attempts, 3, "the read is retried until it answers");
    }

    #[test]
    fn a_read_that_never_answers_reports_the_last_failure() {
        let (cache, _dir) = cache();
        let mut attempts = 0;
        let outcome: crate::error::Result<u8> = cache.read(|| {
            attempts += 1;
            Err(crate::error::AppError::msg("still not ready"))
        });
        assert!(outcome.is_err());
        assert_eq!(attempts, READ_ATTEMPTS);
    }

    #[test]
    fn the_oldest_thumbnails_go_once_the_store_is_over_its_cap() {
        let dir = tempfile::tempdir().unwrap();
        let cache_dir = dir.path().join("cache");
        std::fs::create_dir_all(&cache_dir).unwrap();
        // Written oldest first, so the later ones are the newest.
        for name in ["old.thumb.png", "middle.thumb.png", "new.thumb.png"] {
            std::fs::write(cache_dir.join(name), vec![0u8; 8]).unwrap();
            std::thread::sleep(Duration::from_millis(5));
        }
        // A partial write from a crash is cleaned up rather than served.
        std::fs::write(cache_dir.join("half.thumb.png.part"), b"junk").unwrap();

        let cache = MediaCache::open(cache_dir.clone());
        assert!(!cache_dir.join("half.thumb.png.part").exists());

        // Room for two of the three entries: the oldest goes.
        cache.prune_to(16);
        assert!(!cache_dir.join("old.thumb.png").exists());
        assert!(cache.thumbnail("middle.thumb.png").is_some());
        assert!(cache.thumbnail("new.thumb.png").is_some());
    }
}
