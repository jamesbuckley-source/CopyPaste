//! Image persistence and thumbnail generation.
//!
//! Full images and their thumbnails are stored as files next to the database
//! rather than as BLOBs. That keeps the SQLite file small (no multi-megabyte rows
//! bloating WAL checkpoints) while the database still holds the metadata.

use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use image::codecs::png::PngEncoder;
use image::{DynamicImage, ExtendedColorType, ImageEncoder, RgbaImage};

use crate::error::{AppError, Result};

/// How much of an MP4 is read looking for its movie header. `moov` is a few
/// kilobytes for the files this app stores, and it sits at one end or the other.
const MOOV_WINDOW: u64 = 2 * 1024 * 1024;

/// Playing time of an MP4 file, in milliseconds.
///
/// The duration lives in the `mvhd` box inside `moov`, which ffmpeg writes at the
/// front for a faststart file and at the back for one written straight to disk.
/// Both ends are read rather than the whole file, which can be tens of megabytes.
pub fn video_duration_ms(path: &Path) -> Option<u64> {
    let mut file = std::fs::File::open(path).ok()?;
    let length = file.metadata().ok()?.len();
    let window = length.min(MOOV_WINDOW);

    let mut head = vec![0u8; window as usize];
    file.read_exact(&mut head).ok()?;
    if let Some(ms) = mp4_duration_ms(&head) {
        return Some(ms);
    }
    if length <= MOOV_WINDOW {
        return None;
    }

    file.seek(SeekFrom::Start(length - window)).ok()?;
    let mut tail = vec![0u8; window as usize];
    file.read_exact(&mut tail).ok()?;
    // The window starts part-way through a box, so the walk begins at the box that
    // holds the movie header rather than at byte zero.
    let offset = tail
        .windows(4)
        .position(|bytes| bytes == b"moov")
        .and_then(|at| at.checked_sub(4))?;
    mp4_duration_ms(&tail[offset..])
}

/// Reads the duration out of a buffer that starts at a box boundary.
///
/// Only the two boxes that matter are walked: a top-level `moov` holding an `mvhd`.
pub fn mp4_duration_ms(bytes: &[u8]) -> Option<u64> {
    let moov = boxes(bytes).into_iter().find(|(kind, _)| *kind == b"moov")?.1;
    let mvhd = boxes(moov).into_iter().find(|(kind, _)| *kind == b"mvhd")?.1;
    let version = *mvhd.first()?;
    // version, flags, creation and modification times come first; the timescale and
    // duration follow, as 32-bit fields or 64-bit ones in version 1.
    let (timescale, duration) = match version {
        0 => (
            u32::from_be_bytes(mvhd.get(12..16)?.try_into().ok()?),
            u64::from(u32::from_be_bytes(mvhd.get(16..20)?.try_into().ok()?)),
        ),
        1 => (
            u32::from_be_bytes(mvhd.get(20..24)?.try_into().ok()?),
            u64::from_be_bytes(mvhd.get(24..32)?.try_into().ok()?),
        ),
        _ => return None,
    };
    (timescale > 0).then(|| duration.saturating_mul(1_000) / u64::from(timescale))
}

/// Splits a buffer of MP4 boxes into `(type, payload)` pairs.
///
/// A box whose declared size runs past the buffer ends the walk: that only happens
/// when the buffer is a window onto a larger file.
fn boxes(mut data: &[u8]) -> Vec<(&[u8], &[u8])> {
    let mut found: Vec<(&[u8], &[u8])> = Vec::new();
    while data.len() >= 8 {
        let declared = u32::from_be_bytes([data[0], data[1], data[2], data[3]]) as u64;
        let kind = &data[4..8];
        // 1 means "the 64-bit size follows the type"; 0 means "to the end".
        let (size, header) = match declared {
            0 => (data.len() as u64, 8),
            1 => {
                if data.len() < 16 {
                    break;
                }
                let Ok(size) = data[8..16].try_into() else {
                    break;
                };
                (u64::from_be_bytes(size), 16)
            }
            size => (size, 8),
        };
        if size < header || size > data.len() as u64 {
            break;
        }
        found.push((kind, &data[header as usize..size as usize]));
        data = &data[size as usize..];
    }
    found
}

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
/// whatever folder its settings name — and the files Kept puts there must
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

    /// Builds the boxes an MP4 needs for the duration to be readable.
    fn mp4(version: u8, timescale: u32, duration: u64, faststart: bool) -> Vec<u8> {
        let mut mvhd = Vec::new();
        mvhd.push(version);
        mvhd.extend_from_slice(&[0, 0, 0]); // flags
        if version == 0 {
            mvhd.extend_from_slice(&0u32.to_be_bytes()); // created
            mvhd.extend_from_slice(&0u32.to_be_bytes()); // modified
            mvhd.extend_from_slice(&timescale.to_be_bytes());
            mvhd.extend_from_slice(&(duration as u32).to_be_bytes());
        } else {
            mvhd.extend_from_slice(&0u64.to_be_bytes());
            mvhd.extend_from_slice(&0u64.to_be_bytes());
            mvhd.extend_from_slice(&timescale.to_be_bytes());
            mvhd.extend_from_slice(&duration.to_be_bytes());
        }
        mvhd.extend_from_slice(&[0; 80]); // the rest of the header, unused here

        let boxed = |kind: &[u8], payload: &[u8]| {
            let mut out = Vec::new();
            out.extend_from_slice(&((payload.len() + 8) as u32).to_be_bytes());
            out.extend_from_slice(kind);
            out.extend_from_slice(payload);
            out
        };
        let moov = boxed(b"moov", &boxed(b"mvhd", &mvhd));
        let ftyp = boxed(b"ftyp", b"isom\0\0\x02\0isomiso2mp41");
        let mdat = boxed(b"mdat", &[0u8; 64]);
        let mut file = Vec::new();
        if faststart {
            file.extend(ftyp);
            file.extend(moov);
            file.extend(mdat);
        } else {
            file.extend(ftyp);
            file.extend(mdat);
            file.extend(moov);
        }
        file
    }

    #[test]
    fn an_mp4_duration_is_read_from_the_movie_header() {
        // 7250 / 1000 of a second, the shape ffmpeg writes for a normal file.
        let bytes = mp4(0, 1_000, 7_250, true);
        assert_eq!(mp4_duration_ms(&bytes), Some(7_250));

        // A different timescale is converted, not assumed to be milliseconds.
        let bytes = mp4(0, 90_000, 90_000 * 3 / 2, true);
        assert_eq!(mp4_duration_ms(&bytes), Some(1_500));

        // Version 1 carries a 64-bit duration.
        let bytes = mp4(1, 1_000, 62 * 60 * 1_000, true);
        assert_eq!(mp4_duration_ms(&bytes), Some(3_720_000));
    }

    #[test]
    fn a_movie_header_at_the_end_of_the_file_is_still_found() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("plain.mp4");
        std::fs::write(&path, mp4(0, 1_000, 4_250, false)).unwrap();
        assert_eq!(
            video_duration_ms(&path),
            Some(4_250),
            "ffmpeg writes moov last unless faststart was asked for"
        );

        let faststart = dir.path().join("faststart.mp4");
        std::fs::write(&faststart, mp4(0, 1_000, 12_000, true)).unwrap();
        assert_eq!(video_duration_ms(&faststart), Some(12_000));
    }

    #[test]
    fn something_that_is_not_an_mp4_has_no_duration() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("image.png");
        std::fs::write(&path, b"\x89PNG\r\n\x1a\n and then some").unwrap();
        assert_eq!(video_duration_ms(&path), None);

        assert_eq!(mp4_duration_ms(b""), None);
        assert_eq!(mp4_duration_ms(b"GIF89a not a movie"), None);
        // A zero timescale would divide by nothing.
        assert_eq!(mp4_duration_ms(&mp4(0, 0, 1_000, true)), None);
    }
}
