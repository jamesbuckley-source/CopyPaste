//! Dominant-colour extraction for screenshot clips.
//!
//! The picker is deliberately simple: bucket pixels into a coarse RGB cube,
//! average each bucket, then take the most populated buckets while rejecting
//! colours that are too close to one already chosen. That produces a small,
//! representative scheme without a heavyweight clustering dependency.

use std::collections::HashMap;

/// Scheme entries are capped so the preview stays readable.
pub const MAX_COLORS: usize = 5;
/// Pixels under this alpha are treated as absent. Screenshots are opaque, but
/// transparent PNGs reach the same code path.
const MIN_ALPHA: u8 = 32;
/// Longest edge of the sampled grid. Larger images are sampled on a stride so a
/// 4K screenshot does not need millions of bucket updates.
const SAMPLE_EDGE: usize = 160;
/// Squared Euclidean RGB distance below which a colour is considered a shade of
/// one already picked.
const MIN_DISTANCE_SQ: i32 = 40 * 40;

/// Returns up to [`MAX_COLORS`] `#rrggbb` values, most frequent first.
pub fn extract(rgba: &[u8], width: u32, height: u32) -> Vec<String> {
    if width == 0 || height == 0 {
        return Vec::new();
    }
    let (width, height) = (width as usize, height as usize);
    if rgba.len() < width.saturating_mul(height).saturating_mul(4) {
        return Vec::new();
    }

    let stride = width.max(height).div_ceil(SAMPLE_EDGE).max(1);
    // key -> (count, red sum, green sum, blue sum)
    let mut buckets: HashMap<u32, (u64, u64, u64, u64)> = HashMap::new();
    let mut y = 0;
    while y < height {
        let mut x = 0;
        while x < width {
            let index = (y * width + x) * 4;
            let alpha = rgba[index + 3];
            if alpha >= MIN_ALPHA {
                let red = u64::from(rgba[index]);
                let green = u64::from(rgba[index + 1]);
                let blue = u64::from(rgba[index + 2]);
                // Five bits per channel keeps the bucket count small while
                // preserving the hue differences a scheme needs.
                let key = (((red >> 3) as u32) << 10)
                    | (((green >> 3) as u32) << 5)
                    | ((blue >> 3) as u32);
                let entry = buckets.entry(key).or_insert((0, 0, 0, 0));
                entry.0 += 1;
                entry.1 += red;
                entry.2 += green;
                entry.3 += blue;
            }
            x += stride;
        }
        y += stride;
    }

    let mut candidates: Vec<(u64, (u8, u8, u8))> = buckets
        .into_values()
        .filter(|(count, ..)| *count > 0)
        .map(|(count, red, green, blue)| {
            (
                count,
                (
                    (red / count) as u8,
                    (green / count) as u8,
                    (blue / count) as u8,
                ),
            )
        })
        .collect();
    // Frequency first; the tuple tie-break keeps the output deterministic when
    // two buckets are equally common.
    candidates.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));

    let mut chosen: Vec<(u8, u8, u8)> = Vec::with_capacity(MAX_COLORS);
    for (_, color) in &candidates {
        if chosen
            .iter()
            .all(|existing| distance_sq(*existing, *color) >= MIN_DISTANCE_SQ)
        {
            chosen.push(*color);
            if chosen.len() == MAX_COLORS {
                break;
            }
        }
    }

    chosen
        .into_iter()
        .map(|(red, green, blue)| format!("#{red:02x}{green:02x}{blue:02x}"))
        .collect()
}

fn distance_sq(a: (u8, u8, u8), b: (u8, u8, u8)) -> i32 {
    let channel = |x: u8, y: u8| {
        let delta = i32::from(x) - i32::from(y);
        delta * delta
    };
    channel(a.0, b.0) + channel(a.1, b.1) + channel(a.2, b.2)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(width: usize, pixels: &[(u8, u8, u8, u8)]) -> Vec<u8> {
        assert_eq!(pixels.len(), width);
        pixels
            .iter()
            .flat_map(|pixel| [pixel.0, pixel.1, pixel.2, pixel.3])
            .collect()
    }

    #[test]
    fn orders_colours_by_frequency() {
        let pixels = [
            (255, 0, 0, 255),
            (255, 0, 0, 255),
            (255, 0, 0, 255),
            (0, 0, 255, 255),
        ];
        assert_eq!(extract(&image(4, &pixels), 4, 1), vec!["#ff0000", "#0000ff"]);
    }

    #[test]
    fn caps_the_scheme_at_five() {
        let pixels = [
            (255, 0, 0, 255),
            (0, 255, 0, 255),
            (0, 0, 255, 255),
            (255, 255, 0, 255),
            (0, 255, 255, 255),
            (255, 0, 255, 255),
            (128, 0, 0, 255),
            (0, 128, 0, 255),
        ];
        let colors = extract(&image(pixels.len(), &pixels), pixels.len() as u32, 1);
        assert_eq!(colors.len(), MAX_COLORS);
        assert!(colors.iter().all(|color| color.starts_with('#') && color.len() == 7));
    }

    #[test]
    fn ignores_transparent_pixels() {
        let pixels = [(0, 0, 0, 0), (12, 200, 40, 255)];
        assert_eq!(extract(&image(2, &pixels), 2, 1), vec!["#0cc828"]);
    }

    #[test]
    fn rejects_a_truncated_buffer() {
        assert!(extract(&[0, 0, 0, 255], 2, 2).is_empty());
        assert!(extract(&[], 0, 0).is_empty());
    }

    #[test]
    fn near_duplicate_shades_are_merged() {
        let pixels = [(255, 0, 0, 255), (250, 0, 0, 255), (0, 0, 255, 255)];
        let colors = extract(&image(3, &pixels), 3, 1);
        assert_eq!(colors.len(), 2, "the two reds collapse into one entry");
    }
}
