//! Image persistence and thumbnail generation.
//!
//! Full images and their thumbnails are stored as files next to the database
//! rather than as BLOBs. That keeps the SQLite file small (no multi-megabyte rows
//! bloating WAL checkpoints) while the database still holds the metadata.

use std::path::{Path, PathBuf};

use image::codecs::png::PngEncoder;
use image::{DynamicImage, ExtendedColorType, ImageEncoder, RgbaImage};

use crate::error::{AppError, Result};

/// Longest edge of a generated thumbnail. Sized for the list row preview.
const THUMB_MAX_EDGE: u32 = 400;

/// Files written for one image clip.
///
/// The pixel dimensions are deliberately not repeated here; they already travel
/// with the payload that produced these files.
#[derive(Debug, Clone)]
pub struct ImageFiles {
    pub image: String,
    pub thumb: String,
    pub byte_size: i64,
}

impl ImageFiles {
    /// Removes the files backing this image. Used when a row insert fails after
    /// the bytes were already written.
    pub fn cleanup(&self, dir: &Path) {
        for name in [&self.image, &self.thumb] {
            let _ = std::fs::remove_file(dir.join(name));
        }
    }
}

pub fn write_image_files(
    dir: &Path,
    width: u32,
    height: u32,
    rgba: &[u8],
    max_bytes: u64,
) -> Result<ImageFiles> {
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|px| px.checked_mul(4))
        .ok_or_else(|| AppError::msg("image dimensions overflow"))?;
    if rgba.len() != expected {
        return Err(AppError::msg(format!(
            "image buffer is {} bytes but {}x{} RGBA requires {expected}",
            rgba.len(),
            width,
            height
        )));
    }
    if width == 0 || height == 0 {
        return Err(AppError::msg("refusing to store a zero-sized image"));
    }

    let full = RgbaImage::from_raw(width, height, rgba.to_vec())
        .ok_or_else(|| AppError::msg("could not construct image from clipboard buffer"))?;

    let encoded = encode_rgba_png(&full)?;
    if encoded.len() as u64 > max_bytes {
        return Err(AppError::msg(format!(
            "image encodes to {} bytes, above the configured {} byte limit",
            encoded.len(),
            max_bytes
        )));
    }

    // Built from the original buffer rather than the re-decoded PNG, so a very
    // large image is only decoded once.
    //
    // `DynamicImage::thumbnail` fits the image inside the box and preserves the
    // aspect ratio. The lower-level `imageops::thumbnail` must NOT be used here:
    // it stretches to exactly the requested size, which squashes the preview. The
    // box is also clamped to the source dimensions so a small image is never
    // upscaled into a heavier PNG than the original.
    let thumb_image = DynamicImage::ImageRgba8(full)
        .thumbnail(THUMB_MAX_EDGE.min(width), THUMB_MAX_EDGE.min(height));
    let thumb = encode_rgba_png(&thumb_image.to_rgba8())?;

    let stem = uuid::Uuid::new_v4().simple().to_string();
    let image = format!("{stem}.png");
    let thumb_name = format!("{stem}.thumb.png");

    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join(&image), &encoded)?;
    if let Err(e) = std::fs::write(dir.join(&thumb_name), &thumb) {
        // Do not leave a half-written pair behind.
        let _ = std::fs::remove_file(dir.join(&image));
        return Err(e.into());
    }

    Ok(ImageFiles {
        image,
        thumb: thumb_name,
        byte_size: encoded.len() as i64,
    })
}

/// Identifies a file at one moment, so a save by another program can be noticed.
///
/// Editors do not tell us when they wrote anything, and an editor that was opened
/// and closed without saving leaves the file exactly as it was — this is how those
/// two cases are told apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileStamp {
    pub len: u64,
    pub modified: Option<std::time::SystemTime>,
}

impl FileStamp {
    /// `None` when the file is not there, which is never "unchanged".
    pub fn of(path: &Path) -> Option<Self> {
        let meta = std::fs::metadata(path).ok()?;
        Some(Self {
            len: meta.len(),
            modified: meta.modified().ok(),
        })
    }
}

/// Whether `name` is one of the files this store writes for a clip: `<uuid>.png`
/// and the `.thumb.png` beside it.
///
/// An editor can be pointed at the media folder itself — ShotCraft saves into
/// whatever folder its settings name — and the files CopyPaste puts there must
/// never be mistaken for the editor's output, or a clip would be rebuilt from its
/// own previous rebuild for ever.
pub fn is_managed_name(name: &str) -> bool {
    let stem = name
        .strip_suffix(".thumb.png")
        .or_else(|| name.strip_suffix(".png"));
    let Some(stem) = stem else {
        return false;
    };
    stem.len() == 32 && stem.chars().all(|ch| ch.is_ascii_hexdigit())
}

/// Encodes RGBA pixels as PNG straight from the buffer, so a large clipboard
/// image is never cloned just to be wrapped in a `DynamicImage`.
fn encode_rgba_png(image: &RgbaImage) -> Result<Vec<u8>> {
    let mut buffer = Vec::new();
    PngEncoder::new(&mut buffer).write_image(
        image.as_raw(),
        image.width(),
        image.height(),
        ExtendedColorType::Rgba8,
    )?;
    Ok(buffer)
}

/// Decodes an image fetched for a preview and re-encodes it at thumbnail size.
///
/// Link previews have no full-size original behind them: the page's image is only
/// ever shown small, so keeping the original would cost megabytes for a picture
/// nobody can open. Returning bytes rather than writing them lets the store seal
/// the result when the clip is a secret.
pub fn preview_png(bytes: &[u8]) -> Result<Vec<u8>> {
    if bytes.is_empty() {
        return Err(AppError::msg("the preview image was empty"));
    }
    let decoded = image::load_from_memory(bytes)
        .map_err(|e| AppError::msg(format!("that preview image could not be read: {e}")))?;
    let thumb = decoded.thumbnail(THUMB_MAX_EDGE, THUMB_MAX_EDGE).to_rgba8();
    encode_rgba_png(&thumb)
}

/// Maps a media file name to a path, or `None` if it is not a name this app
/// generated.
///
/// The `media` URI scheme is reachable from the webview, so this allowlist is the
/// boundary that keeps `../../` style names from being served from disk.
pub fn resolve(dir: &Path, name: &str) -> Option<PathBuf> {
    if name.is_empty() || name.len() > 128 {
        return None;
    }
    let safe = name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-' || c == '_');
    if !safe || name.contains("..") {
        return None;
    }
    let path = dir.join(name);
    path.is_file().then_some(path)
}

/// Decodes a stored image back to RGBA, so it can be put on the clipboard again.
pub fn load_rgba(dir: &Path, name: &str) -> Result<(u32, u32, Vec<u8>)> {
    let path =
        resolve(dir, name).ok_or_else(|| AppError::NotFound(format!("media file `{name}`")))?;
    let decoded = image::open(&path)?.to_rgba8();
    let (width, height) = decoded.dimensions();
    Ok((width, height, decoded.into_raw()))
}

/// Decodes a stored image at no more than `max_edge` on its longest side.
///
/// Colour extraction samples a small grid, so decoding a 4K screenshot at full
/// size would spend tens of megabytes on detail it then throws away.
pub fn load_rgba_thumbnail(dir: &Path, name: &str, max_edge: u32) -> Result<(u32, u32, Vec<u8>)> {
    let path =
        resolve(dir, name).ok_or_else(|| AppError::NotFound(format!("media file `{name}`")))?;
    let decoded = image::open(&path)?.thumbnail(max_edge, max_edge).to_rgba8();
    let (width, height) = decoded.dimensions();
    Ok((width, height, decoded.into_raw()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid_rgba(width: u32, height: u32) -> Vec<u8> {
        let mut buf = Vec::with_capacity((width * height * 4) as usize);
        for y in 0..height {
            for x in 0..width {
                buf.extend_from_slice(&[(x % 256) as u8, (y % 256) as u8, 128, 255]);
            }
        }
        buf
    }

    #[test]
    fn writes_full_image_and_a_smaller_thumbnail() {
        let dir = tempfile::tempdir().unwrap();
        let files =
            write_image_files(dir.path(), 800, 400, &solid_rgba(800, 400), u64::MAX).unwrap();

        assert!(dir.path().join(&files.image).is_file());
        assert!(dir.path().join(&files.thumb).is_file());
        assert!(files.byte_size > 0);

        // The stored thumbnail must actually be lighter than the original.
        let full = std::fs::metadata(dir.path().join(&files.image))
            .unwrap()
            .len();
        let thumb = std::fs::metadata(dir.path().join(&files.thumb))
            .unwrap()
            .len();
        assert!(
            thumb < full,
            "thumb {thumb} should be smaller than full {full}"
        );

        // Aspect ratio preserved and fitted inside the 400px box.
        let (width, height) = image::image_dimensions(dir.path().join(&files.thumb)).unwrap();
        assert_eq!((width, height), (400, 200));
    }

    #[test]
    fn small_images_are_not_upscaled() {
        let dir = tempfile::tempdir().unwrap();
        let files = write_image_files(dir.path(), 40, 20, &solid_rgba(40, 20), u64::MAX).unwrap();
        let (width, height) = image::image_dimensions(dir.path().join(&files.thumb)).unwrap();
        assert_eq!(
            (width, height),
            (40, 20),
            "a source smaller than the thumbnail box must keep its own dimensions"
        );
    }

    #[test]
    fn tall_images_are_fitted_by_the_long_edge() {
        let dir = tempfile::tempdir().unwrap();
        let files =
            write_image_files(dir.path(), 100, 4000, &solid_rgba(100, 4000), u64::MAX).unwrap();
        let (width, height) = image::image_dimensions(dir.path().join(&files.thumb)).unwrap();
        assert_eq!(height, 400, "the long edge is the one that gets clamped");
        assert_eq!(width, 10);
    }

    #[test]
    fn oversized_image_is_rejected_without_leaving_files() {
        let dir = tempfile::tempdir().unwrap();
        let err = write_image_files(dir.path(), 64, 32, &solid_rgba(64, 32), 16);
        assert!(err.is_err(), "a 16-byte budget cannot hold any PNG");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn buffer_size_mismatch_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        assert!(write_image_files(dir.path(), 10, 10, &[0u8; 8], u64::MAX).is_err());
    }

    #[test]
    fn zero_sized_image_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        assert!(write_image_files(dir.path(), 0, 0, &[], u64::MAX).is_err());
    }

    #[test]
    fn cleanup_removes_both_files() {
        let dir = tempfile::tempdir().unwrap();
        let files = write_image_files(dir.path(), 32, 32, &solid_rgba(32, 32), u64::MAX).unwrap();
        files.cleanup(dir.path());
        assert!(!dir.path().join(&files.image).exists());
        assert!(!dir.path().join(&files.thumb).exists());
    }

    #[test]
    fn resolve_rejects_path_traversal() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("legit.png"), b"x").unwrap();

        assert!(resolve(dir.path(), "legit.png").is_some());
        assert!(resolve(dir.path(), "../key.bin").is_none());
        assert!(resolve(dir.path(), "..\\key.bin").is_none());
        assert!(resolve(dir.path(), "sub/legit.png").is_none());
        assert!(resolve(dir.path(), "C:\\Windows\\win.ini").is_none());
        assert!(resolve(dir.path(), "missing.png").is_none());
        assert!(resolve(dir.path(), "").is_none());
        assert!(resolve(dir.path(), &"a".repeat(200)).is_none());
        // A traversal that resolves to a real file must still be refused.
        std::fs::write(dir.path().join("outside.png"), b"x").unwrap();
        assert!(resolve(dir.path(), "./outside.png").is_none());
    }

    #[test]
    fn resolve_accepts_generated_names() {
        let dir = tempfile::tempdir().unwrap();
        let files = write_image_files(dir.path(), 16, 16, &solid_rgba(16, 16), u64::MAX).unwrap();
        assert!(resolve(dir.path(), &files.image).is_some());
        assert!(resolve(dir.path(), &files.thumb).is_some());
    }

    #[test]
    fn stored_images_decode_back_to_their_original_pixels() {
        let dir = tempfile::tempdir().unwrap();
        let pixels = solid_rgba(48, 24);
        let files = write_image_files(dir.path(), 48, 24, &pixels, u64::MAX).unwrap();

        let (width, height, decoded) = load_rgba(dir.path(), &files.image).unwrap();
        assert_eq!((width, height), (48, 24));
        assert_eq!(decoded, pixels, "a round trip must not alter the pixels");

        // And the traversal guard still applies on this path.
        assert!(load_rgba(dir.path(), "../key.bin").is_err());
    }
}
