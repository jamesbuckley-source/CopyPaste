//! Local face detection for image clips.
//!
//! The detector is YuNet — the 227 KB model from OpenCV's Zoo — run by tract, a
//! pure-Rust ONNX interpreter. Nothing is uploaded, and no C++ runtime has to be
//! shipped beside the exe.
//!
//! Only *whether* an image holds a face, how many and where is kept. Faces are
//! never compared between clips, so this is not recognition.

use std::sync::OnceLock;

use ndarray::Array4;
use tract::prelude::*;

use crate::error::{AppError, Result};

tract::impl_ndarray_interop!();

/// Vendored model; MIT licensed, see `assets/yunet.LICENSE`.
const MODEL: &[u8] = include_bytes!("../assets/yunet.onnx");

/// The model's declared input is fixed at 640x640.
const SIDE: usize = 640;

/// Below this the box is more likely a pattern than a face. YuNet's own default
/// is 0.9, which misses faces in group shots; a tag is not worth that.
const SCORE_THRESHOLD: f32 = 0.85;

/// Overlap at which two boxes describe the same face.
const NMS_IOU: f32 = 0.3;

/// Detection heads run at three strides, fine to coarse.
const STRIDES: [usize; 3] = [8, 16, 32];

/// One detected face in source-image pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Face {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
    pub score: f32,
}

impl Face {
    fn right(&self) -> f32 {
        self.x + self.width
    }

    fn bottom(&self) -> f32 {
        self.y + self.height
    }

    fn area(&self) -> f32 {
        self.width.max(0.0) * self.height.max(0.0)
    }
}

/// The prepared model, reduced to "pixels in, plain output buffers out" so the
/// concrete tract types never leak past this module.
type Runner = Box<dyn Fn(Vec<f32>) -> std::result::Result<Vec<Vec<f32>>, String> + Send + Sync>;

static RUNNER: OnceLock<Runner> = OnceLock::new();

/// Detects the faces in an RGBA image, best first.
///
/// Sizes and coordinates come back in the caller's pixels, not the model's.
pub fn detect(rgba: &[u8], width: u32, height: u32) -> Result<Vec<Face>> {
    let (pixels, scale) = letterbox(rgba, width, height);
    let outputs = runner()?(pixels).map_err(AppError::msg)?;

    let heads = Heads::parse(&outputs)?;
    Ok(keep_best(decode(&heads, scale)))
}

fn runner() -> Result<&'static Runner> {
    if let Some(runner) = RUNNER.get() {
        return Ok(runner);
    }

    let model = tract::onnx()
        .map_err(|e| AppError::msg(format!("could not start the model parser: {e}")))?
        .load_buffer(MODEL)
        .map_err(|e| AppError::msg(format!("could not read the face model: {e}")))?
        .into_model()
        .map_err(|e| AppError::msg(format!("could not type the face model: {e}")))?;

    let runtime =
        tract::runtime_for_name("default").map_err(|e| AppError::msg(format!("no face runtime: {e}")))?;
    let runnable = runtime
        .prepare(model)
        .map_err(|e| AppError::msg(format!("could not prepare the face model: {e}")))?;

    let runner: Runner = Box::new(move |pixels: Vec<f32>| {
        let input = Array4::from_shape_vec((1, 3, SIDE, SIDE), pixels).map_err(|e| e.to_string())?;
        let outputs = runnable.run([input.tract().map_err(|e| e.to_string())?]).map_err(|e| e.to_string())?;
        Ok(outputs
            .iter()
            .map(|tensor| tensor.as_slice::<f32>().map(<[f32]>::to_vec).unwrap_or_default())
            .collect())
    });

    // Losing the race just drops the second copy; both behave identically.
    let _ = RUNNER.set(runner);
    Ok(RUNNER.get().expect("the runner was just set"))
}

/// The twelve output tensors, grouped by what they predict.
struct Heads<'a> {
    cls: [&'a [f32]; 3],
    obj: [&'a [f32]; 3],
    bbox: [&'a [f32]; 3],
}

impl<'a> Heads<'a> {
    fn parse(outputs: &'a [Vec<f32>]) -> Result<Self> {
        let floats = |index: usize, stride: usize, width: usize| -> Result<&'a [f32]> {
            let values = outputs
                .get(index)
                .ok_or_else(|| AppError::msg("the face model returned too few outputs"))?;
            let cells = (SIDE / stride) * (SIDE / stride);
            if values.len() < cells * width {
                return Err(AppError::msg("the face model returned short outputs"));
            }
            Ok(&values[..cells * width])
        };

        Ok(Self {
            cls: [
                floats(0, STRIDES[0], 1)?,
                floats(1, STRIDES[1], 1)?,
                floats(2, STRIDES[2], 1)?,
            ],
            obj: [
                floats(3, STRIDES[0], 1)?,
                floats(4, STRIDES[1], 1)?,
                floats(5, STRIDES[2], 1)?,
            ],
            bbox: [
                floats(6, STRIDES[0], 4)?,
                floats(7, STRIDES[1], 4)?,
                floats(8, STRIDES[2], 4)?,
            ],
        })
    }
}

/// Turns the raw heads into boxes in source pixels, keeping anything above the
/// score threshold.
fn decode(heads: &Heads<'_>, scale: f32) -> Vec<Face> {
    let mut faces = Vec::new();

    for (index, stride) in STRIDES.iter().enumerate() {
        let cells = SIDE / stride;
        // A head that came back short cannot be decoded; skipping it is safer than
        // indexing past the end of a malformed model output.
        if heads.cls[index].len() < cells
            || heads.obj[index].len() < cells
            || heads.bbox[index].len() < cells * 4
        {
            continue;
        }
        for row in 0..cells {
            for column in 0..cells {
                let cell = row * cells + column;

                let class = heads.cls[index][cell].clamp(0.0, 1.0);
                let object = heads.obj[index][cell].clamp(0.0, 1.0);
                let score = (class * object).sqrt();
                if score < SCORE_THRESHOLD {
                    continue;
                }

                let boxed = &heads.bbox[index][cell * 4..cell * 4 + 4];
                let centre_x = (column as f32 + boxed[0]) * *stride as f32;
                let centre_y = (row as f32 + boxed[1]) * *stride as f32;
                let width = boxed[2].exp() * *stride as f32;
                let height = boxed[3].exp() * *stride as f32;

                faces.push(Face {
                    x: (centre_x - width / 2.0) / scale,
                    y: (centre_y - height / 2.0) / scale,
                    width: width / scale,
                    height: height / scale,
                    score,
                });
            }
        }
    }

    faces
}

/// Drops boxes that overlap a better one, and sorts what remains best first.
fn keep_best(mut faces: Vec<Face>) -> Vec<Face> {
    faces.sort_by(|a, b| b.score.total_cmp(&a.score));

    let mut kept: Vec<Face> = Vec::new();
    for face in faces {
        if kept.iter().all(|other| overlap(&face, other) <= NMS_IOU) {
            kept.push(face);
        }
    }
    kept
}

/// Intersection over union of two boxes.
fn overlap(a: &Face, b: &Face) -> f32 {
    let width = (a.right().min(b.right()) - a.x.max(b.x)).max(0.0);
    let height = (a.bottom().min(b.bottom()) - a.y.max(b.y)).max(0.0);
    let shared = width * height;
    let total = a.area() + b.area() - shared;
    if total <= 0.0 {
        0.0
    } else {
        shared / total
    }
}

/// Scales the image to fit the model's square input, padding the spare room at
/// the right and bottom with black — the same letterbox OpenCV's detector uses.
///
/// Returns the CHW pixel buffer the model expects (BGR, 0-255) and the factor
/// that maps model coordinates back to source pixels.
fn letterbox(rgba: &[u8], width: u32, height: u32) -> (Vec<f32>, f32) {
    let mut pixels = vec![0f32; 3 * SIDE * SIDE];
    if width == 0 || height == 0 {
        return (pixels, 1.0);
    }

    let scale = (SIDE as f32 / width as f32).min(SIDE as f32 / height as f32);
    let scaled_width = ((width as f32 * scale).round() as usize).clamp(1, SIDE);
    let scaled_height = ((height as f32 * scale).round() as usize).clamp(1, SIDE);
    let plane = SIDE * SIDE;

    for y in 0..scaled_height {
        // Nearest neighbour: the source is already being resampled by the network.
        let source_y = ((y as f32 + 0.5) / scale) as u32;
        let source_y = source_y.min(height - 1) as usize;
        for x in 0..scaled_width {
            let source_x = ((x as f32 + 0.5) / scale) as u32;
            let source_x = source_x.min(width - 1) as usize;
            let source = (source_y * width as usize + source_x) * 4;

            let red = rgba[source] as f32;
            let green = rgba[source + 1] as f32;
            let blue = rgba[source + 2] as f32;
            let target = y * SIDE + x;

            pixels[target] = blue;
            pixels[plane + target] = green;
            pixels[2 * plane + target] = red;
        }
    }

    (pixels, scale)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn head(size: usize, fill: f32) -> Vec<f32> {
        vec![fill; size]
    }

    fn boxes_for(class: &[f32], object: &[f32], boxes: &[f32]) -> Vec<Face> {
        let heads = Heads {
            cls: [class, &[], &[]],
            obj: [object, &[], &[]],
            bbox: [boxes, &[], &[]],
        };
        decode(&heads, 1.0)
    }

    #[test]
    fn a_confident_cell_becomes_a_box() {
        // One cell at stride 8, centred at (0.5, 0.5) in cell space with a box
        // exactly one stride wide and tall: centre (4, 4), so 0,0 to 8,8.
        let class = head(6400, 0.0);
        let object = head(6400, 0.0);
        let boxes = head(6400 * 4, 0.0);
        let mut class = class;
        let mut object = object;
        let mut boxes = boxes;
        class[0] = 0.9;
        object[0] = 0.9;
        boxes[2] = 0.0; // exp(0) * 8 = 8 wide
        boxes[3] = 0.0;

        let faces = boxes_for(&class, &object, &boxes);
        assert_eq!(faces.len(), 1);
        let face = faces[0];
        assert!((face.score - 0.9).abs() < 1e-5, "score {}", face.score);
        assert!((face.width - 8.0).abs() < 1e-4);
        assert!((face.height - 8.0).abs() < 1e-4);
        // The cell centre sits at 0,0, so an 8-wide box starts 4 px before it.
        assert!((face.x + 4.0).abs() < 1e-4, "x {}", face.x);
        assert!((face.y + 4.0).abs() < 1e-4, "y {}", face.y);
    }

    #[test]
    fn a_low_score_is_not_a_face() {
        let mut class = head(6400, 0.0);
        let object = head(6400, 0.0);
        let boxes = head(6400 * 4, 0.0);
        class[0] = 0.5;

        assert!(boxes_for(&class, &object, &boxes).is_empty());
    }

    #[test]
    fn the_better_of_two_overlapping_boxes_wins() {
        let small = Face { x: 0.0, y: 0.0, width: 10.0, height: 10.0, score: 0.9 };
        let same = Face { x: 1.0, y: 1.0, width: 10.0, height: 10.0, score: 0.95 };
        let apart = Face { x: 40.0, y: 40.0, width: 10.0, height: 10.0, score: 0.8 };

        let kept = keep_best(vec![small, same, apart]);
        assert_eq!(kept.len(), 2);
        assert!((kept[0].score - 0.95).abs() < 1e-6, "best first");
        assert!((kept[1].score - 0.8).abs() < 1e-6);
    }

    #[test]
    fn coordinates_are_scaled_back_to_the_source_image() {
        let mut class = head(6400, 0.0);
        let mut object = head(6400, 0.0);
        let mut boxes = head(6400 * 4, 0.0);
        class[0] = 1.0;
        object[0] = 1.0;
        boxes[0] = 0.5;
        boxes[1] = 0.5;

        // Half-size scale: a box at 8,8 in model space is 16,16 in the original.
        let heads = Heads { cls: [&class, &[], &[]], obj: [&object, &[], &[]], bbox: [&boxes, &[], &[]] };
        let faces = decode(&heads, 0.5);
        assert_eq!(faces.len(), 1);
        // Centre 4, half an 8-wide box: the box starts at 0 in model space.
        assert!((faces[0].x - 0.0).abs() < 1e-4, "x {}", faces[0].x);
        assert!((faces[0].width - 16.0).abs() < 1e-4, "w {}", faces[0].width);
    }

    #[test]
    fn the_letterbox_keeps_the_aspect_ratio_and_fills_the_rest_with_black() {
        // A 320x160 image scales by 2 to 640x320; the bottom half stays black.
        let rgba = vec![255u8; 320 * 160 * 4];
        let (pixels, scale) = letterbox(&rgba, 320, 160);

        assert!((scale - 2.0).abs() < 1e-6);
        assert_eq!(pixels.len(), 3 * SIDE * SIDE);

        let plane = SIDE * SIDE;
        // Top-left pixel is the image (white in all three planes)…
        assert_eq!(pixels[0], 255.0);
        assert_eq!(pixels[plane], 255.0);
        // …while the padded row under it is black.
        assert_eq!(pixels[400 * SIDE], 0.0);
        assert_eq!(pixels[plane + 400 * SIDE], 0.0);
    }

    #[test]
    fn blue_and_red_are_swapped_for_the_model() {
        let mut rgba = vec![0u8; 4];
        rgba[0] = 10; // red
        rgba[1] = 20; // green
        rgba[2] = 30; // blue

        let (pixels, _) = letterbox(&rgba, 1, 1);
        let plane = SIDE * SIDE;
        assert_eq!(pixels[0], 30.0, "blue plane");
        assert_eq!(pixels[plane], 20.0, "green plane");
        assert_eq!(pixels[2 * plane], 10.0, "red plane");
    }

    #[test]
    fn an_empty_image_is_never_panicked_on() {
        let (pixels, scale) = letterbox(&[], 0, 0);
        assert_eq!(pixels.len(), 3 * SIDE * SIDE);
        assert_eq!(scale, 1.0);
    }

    #[test]
    fn probe_a_real_photo() {
        let Ok(path) = std::env::var("FACES_PROBE") else {
            return;
        };
        let image = image::open(&path).expect("a readable image").to_rgba8();
        let (width, height) = image.dimensions();
        let start = std::time::Instant::now();
        let faces = detect(image.as_raw(), width, height).expect("detection");
        println!("{width}x{height}: {} face(s) in {:?}", faces.len(), start.elapsed());
        for face in &faces {
            println!("{:.3} at {:.0},{:.0} {:.0}x{:.0}", face.score, face.x, face.y, face.width, face.height);
        }
        assert!(!faces.is_empty(), "expected a face in {path}");
    }

    #[test]
    fn a_blank_image_runs_the_model_and_finds_nothing() {
        // Also proves the vendored model parses and shapes itself the way `decode`
        // expects; a corrupt or swapped asset fails here rather than at runtime.
        let rgba = vec![128u8; 64 * 64 * 4];
        let faces = detect(&rgba, 64, 64).expect("detection");
        assert!(faces.is_empty(), "found {} faces in a grey square", faces.len());
    }
}
