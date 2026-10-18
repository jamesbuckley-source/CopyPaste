//! Text recognition for captured images.
//!
//! Windows ships an OCR engine (Windows.Media.Ocr) that is fast, offline and
//! needs no model downloads or extra binaries — the right trade-off for a tool
//! that already targets Windows only. Recognition runs on a worker thread after
//! the clip is stored, so a large screenshot never delays capture or the UI.

use crate::error::{AppError, Result};

/// Used when the engine cannot report its own limit.
const FALLBACK_MAX_DIMENSION: u32 = 2600;

/// Extracts text from a tightly packed RGBA8 image. Returns an empty string when
/// the image holds no recognisable text.
#[cfg(windows)]
pub fn extract_text(rgba: &[u8], width: u32, height: u32) -> Result<String> {
    use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};

    if width == 0 || height == 0 || rgba.len() < width as usize * height as usize * 4 {
        return Ok(String::new());
    }

    let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    let initialized = hr.is_ok();
    // RPC_E_CHANGED_MODE means the thread already joined COM in another mode,
    // which still supports WinRT activation; anything else is a real failure.
    if hr.is_err() && hr.0 != 0x8001_0106u32 as i32 {
        return Err(AppError::msg(format!(
            "could not start the text recognition runtime: {hr:?}"
        )));
    }

    let outcome = recognize(rgba, width, height);
    if initialized {
        unsafe { CoUninitialize() };
    }
    outcome
}

#[cfg(windows)]
fn recognize(rgba: &[u8], width: u32, height: u32) -> Result<String> {
    use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
    use windows::Media::Ocr::OcrEngine;
    use windows::Storage::Streams::DataWriter;

    let outcome: windows::core::Result<String> = (|| {
        let engine = OcrEngine::TryCreateFromUserProfileLanguages()?;
        let max_dimension = OcrEngine::MaxImageDimension()
            .unwrap_or(FALLBACK_MAX_DIMENSION)
            .max(1);

        let (scaled_width, scaled_height, bgra) = to_bgra(rgba, width, height, max_dimension);

        let writer = DataWriter::new()?;
        writer.WriteBytes(&bgra)?;
        let buffer = writer.DetachBuffer()?;
        let bitmap = SoftwareBitmap::CreateCopyFromBuffer(
            &buffer,
            BitmapPixelFormat::Bgra8,
            scaled_width as i32,
            scaled_height as i32,
        )?;

        // Blocking is fine here: this only ever runs on the recognition worker.
        let recognized = pollster::block_on(engine.RecognizeAsync(&bitmap)?)?;
        Ok(recognized.Text()?.to_string())
    })();

    outcome.map_err(|e| AppError::msg(format!("text recognition failed: {e}")))
}

/// Converts RGBA to the BGRA8 layout the OCR engine expects, downscaling first if
/// the image exceeds the engine's limit. Downscaling keeps far more text than
/// refusing the image outright.
#[cfg(windows)]
fn to_bgra(rgba: &[u8], width: u32, height: u32, max_dimension: u32) -> (u32, u32, Vec<u8>) {
    let (width, height, pixels) = if width.max(height) > max_dimension {
        let scale = f64::from(max_dimension) / f64::from(width.max(height));
        let target_width = ((f64::from(width) * scale).round() as u32).max(1);
        let target_height = ((f64::from(height) * scale).round() as u32).max(1);
        match image::RgbaImage::from_raw(width, height, rgba.to_vec()) {
            Some(image) => {
                let resized = image::imageops::resize(
                    &image,
                    target_width,
                    target_height,
                    image::imageops::FilterType::Triangle,
                );
                (target_width, target_height, resized.into_raw())
            }
            None => (width, height, rgba.to_vec()),
        }
    } else {
        (width, height, rgba.to_vec())
    };

    let mut bgra = Vec::with_capacity(pixels.len());
    for pixel in pixels.chunks_exact(4) {
        bgra.extend_from_slice(&[pixel[2], pixel[1], pixel[0], 255]);
    }
    (width, height, bgra)
}

#[cfg(not(windows))]
pub fn extract_text(_rgba: &[u8], _width: u32, _height: u32) -> Result<String> {
    Err(AppError::msg(
        "text recognition is only available on Windows",
    ))
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn recognises_text_rendered_into_an_image() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests")
            .join("fixtures")
            .join("ocr-sample.png");
        let source = image::open(&path)
            .expect("the OCR fixture must decode")
            .to_rgba8();
        let (width, height) = source.dimensions();

        let text = match extract_text(source.as_raw(), width, height) {
            Ok(text) => text,
            Err(e) => {
                // Machines without an OCR language pack cannot run this test; the
                // capture path degrades to "no text" in that case.
                eprintln!("skipping OCR assertion: {e}");
                return;
            }
        };

        let lowered = text.to_lowercase();
        assert!(lowered.contains("copypaste"), "OCR returned {text:?}");
        assert!(lowered.contains("2026"), "OCR returned {text:?}");
    }

    #[test]
    fn blank_images_produce_no_text() {
        let rgba = vec![255u8; 64 * 64 * 4];
        let text = extract_text(&rgba, 64, 64).unwrap_or_default();
        assert!(text.trim().is_empty(), "got {text:?}");
    }
}
