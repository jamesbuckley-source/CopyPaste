//! Dominant-colour extraction for screenshot clips.
//!
//! Pixels are bucketed into a coarse RGB cube and each bucket averaged, then the
//! surviving colours are clustered in OKLab so that "close" means close to the
//! eye rather than close in raw channel values. Neutrals are rationed: a
//! screenshot is mostly white, grey and black, and on raw frequency those fill
//! every slot before an accent colour is ever reached.
//!
//! No clustering dependency: the bucket grid is the first pass, and the greedy
//! pass over the buckets is the second.

use std::collections::HashMap;

/// Scheme entries are capped so the preview stays readable.
pub const MAX_COLORS: usize = 5;
/// Pixels under this alpha are treated as absent. Screenshots are opaque, but
/// transparent PNGs reach the same code path.
const MIN_ALPHA: u8 = 32;
/// Longest edge of the sampled grid. Larger images are sampled on a stride so a
/// 4K screenshot does not need millions of bucket updates.
const SAMPLE_EDGE: usize = 160;
/// Euclidean distance in OKLab below which two colours read as shades of each
/// other rather than as different colours.
const MIN_DISTANCE: f32 = 0.08;
/// Chroma below which a colour counts as neutral: white, grey, black and the
/// barely-tinted near-neutrals.
const NEUTRAL_CHROMA: f32 = 0.035;
/// How many neutral entries a scheme may hold. Two keeps a screenshot's
/// background from crowding out its accents while still showing the page colour
/// and the text colour.
const NEUTRAL_SLOTS: usize = 2;

/// One bucket of the sampled grid, averaged and converted to OKLab.
#[derive(Clone, Copy)]
struct Candidate {
    count: u64,
    red: u8,
    green: u8,
    blue: u8,
    lab: (f32, f32, f32),
    chroma: f32,
}

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

    let mut candidates: Vec<Candidate> = buckets
        .into_values()
        .filter(|(count, ..)| *count > 0)
        .map(|(count, red, green, blue)| {
            let red = (red / count) as u8;
            let green = (green / count) as u8;
            let blue = (blue / count) as u8;
            let lab = to_oklab(red, green, blue);
            Candidate {
                count,
                red,
                green,
                blue,
                lab,
                chroma: (lab.1 * lab.1 + lab.2 * lab.2).sqrt(),
            }
        })
        .collect();
    // Frequency first; the colour tie-break keeps the output deterministic when
    // two buckets are equally common.
    candidates.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| (a.red, a.green, a.blue).cmp(&(b.red, b.green, b.blue)))
    });

    choose(&candidates)
        .into_iter()
        .map(|color| format!("#{:02x}{:02x}{:02x}", color.red, color.green, color.blue))
        .collect()
}

/// Reads the colour under a point given as a fraction of the image.
///
/// `x` and `y` run 0..=1 across the image, so the preview can hand over a pointer
/// position without knowing the decoded size or the zoom it is drawn at. `None`
/// means the point holds no visible pixel.
pub fn sample(rgba: &[u8], width: u32, height: u32, x: f64, y: f64) -> Option<String> {
    if width == 0 || height == 0 || !(x.is_finite() && y.is_finite()) {
        return None;
    }
    let column = (x.clamp(0.0, 1.0) * f64::from(width - 1)).round() as u32;
    let row = (y.clamp(0.0, 1.0) * f64::from(height - 1)).round() as u32;
    let index = ((row as usize)
        .checked_mul(width as usize)?
        .checked_add(column as usize)?)
    .checked_mul(4)?;
    let pixel = rgba.get(index..index + 4)?;
    let [red, green, blue, alpha] = [pixel[0], pixel[1], pixel[2], pixel[3]];
    // A transparent pixel has no colour to show, and matching the extractor's
    // alpha floor keeps a barely-there pixel from being added either.
    if alpha < MIN_ALPHA {
        return None;
    }
    Some(format!("#{red:02x}{green:02x}{blue:02x}"))
}

/// Picks the most common colours that still read as different from each other,
/// rationing neutrals, then tops up from whatever is left.
fn choose(candidates: &[Candidate]) -> Vec<Candidate> {
    let mut chosen: Vec<Candidate> = Vec::with_capacity(MAX_COLORS);
    let mut taken = vec![false; candidates.len()];
    let mut neutrals = 0usize;

    for (index, candidate) in candidates.iter().enumerate() {
        if chosen.len() == MAX_COLORS {
            break;
        }
        let neutral = candidate.chroma < NEUTRAL_CHROMA;
        if neutral && neutrals >= NEUTRAL_SLOTS {
            continue;
        }
        if !is_distinct(&chosen, candidate) {
            continue;
        }
        if neutral {
            neutrals += 1;
        }
        taken[index] = true;
        chosen.push(*candidate);
    }

    // Only a palette with nothing chromatic in it — a greyscale image, or a flat
    // panel — should be padded with more neutrals. Padding a colourful image here
    // would undo the rationing above, so those are allowed to come back short.
    if chosen.iter().any(|color| color.chroma >= NEUTRAL_CHROMA) {
        return chosen;
    }

    for (index, candidate) in candidates.iter().enumerate() {
        if chosen.len() == MAX_COLORS {
            break;
        }
        if taken[index] || !is_distinct(&chosen, candidate) {
            continue;
        }
        chosen.push(*candidate);
    }

    chosen
}

fn is_distinct(chosen: &[Candidate], candidate: &Candidate) -> bool {
    chosen
        .iter()
        .all(|existing| ok_distance(existing.lab, candidate.lab) >= MIN_DISTANCE)
}

fn ok_distance(a: (f32, f32, f32), b: (f32, f32, f32)) -> f32 {
    let lightness = a.0 - b.0;
    let green_red = a.1 - b.1;
    let blue_yellow = a.2 - b.2;
    (lightness * lightness + green_red * green_red + blue_yellow * blue_yellow).sqrt()
}

/// sRGB to OKLab, in Ottosson's formulation.
fn to_oklab(red: u8, green: u8, blue: u8) -> (f32, f32, f32) {
    let linear = |channel: u8| {
        let value = f32::from(channel) / 255.0;
        if value <= 0.040_45 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    let red = linear(red);
    let green = linear(green);
    let blue = linear(blue);

    let long = (0.412_221_46 * red + 0.536_332_55 * green + 0.051_445_995 * blue).cbrt();
    let medium = (0.211_903_5 * red + 0.680_699_5 * green + 0.107_396_96 * blue).cbrt();
    let short = (0.088_302_46 * red + 0.281_718_85 * green + 0.629_978_7 * blue).cbrt();

    (
        0.210_454_26 * long + 0.793_617_8 * medium - 0.004_072_047 * short,
        1.977_998_5 * long - 2.428_592_2 * medium + 0.450_593_7 * short,
        0.025_904_037 * long + 0.782_771_77 * medium - 0.808_675_77 * short,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(pixels: &[(u8, u8, u8, u8)]) -> Vec<u8> {
        pixels
            .iter()
            .flat_map(|pixel| [pixel.0, pixel.1, pixel.2, pixel.3])
            .collect()
    }

    fn extract_row(pixels: &[(u8, u8, u8, u8)]) -> Vec<String> {
        extract(&image(pixels), pixels.len() as u32, 1)
    }

    fn is_neutral(hex: &str) -> bool {
        let value = u32::from_str_radix(&hex[1..], 16).unwrap();
        let lab = to_oklab((value >> 16) as u8, (value >> 8) as u8, value as u8);
        (lab.1 * lab.1 + lab.2 * lab.2).sqrt() < NEUTRAL_CHROMA
    }

    #[test]
    fn orders_colours_by_frequency() {
        let pixels = [
            (255, 0, 0, 255),
            (255, 0, 0, 255),
            (255, 0, 0, 255),
            (0, 0, 255, 255),
        ];
        assert_eq!(extract_row(&pixels), vec!["#ff0000", "#0000ff"]);
    }

    #[test]
    fn greys_do_not_crowd_out_an_accent() {
        // A screenshot lookalike: mostly white, a dark text block, a lighter
        // panel and a small accent. Frequency alone returns four greys first.
        let mut pixels = vec![(255, 255, 255, 255); 60];
        pixels.extend(vec![(32, 32, 32, 255); 20]);
        pixels.extend(vec![(200, 200, 200, 255); 10]);
        pixels.extend(vec![(220, 40, 40, 255); 3]);

        let colors = extract_row(&pixels);
        assert!(
            colors.contains(&"#dc2828".to_string()),
            "the accent has to survive: {colors:?}"
        );
        assert!(
            colors.iter().filter(|color| is_neutral(color)).count() <= NEUTRAL_SLOTS,
            "neutrals stay rationed: {colors:?}"
        );
    }

    #[test]
    fn a_greyscale_image_still_fills_the_scheme() {
        let mut pixels = vec![(255, 255, 255, 255); 10];
        pixels.extend(vec![(128, 128, 128, 255); 10]);
        let colors = extract_row(&pixels);
        assert_eq!(
            colors.len(),
            2,
            "both greys survive once nothing competes: {colors:?}"
        );
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
            (255, 128, 0, 255),
            (128, 0, 255, 255),
        ];
        let colors = extract_row(&pixels);
        assert_eq!(colors.len(), MAX_COLORS, "{colors:?}");
        assert!(
            colors
                .iter()
                .all(|color| color.starts_with('#') && color.len() == 7)
        );
    }

    #[test]
    fn ignores_transparent_pixels() {
        let pixels = [(0, 0, 0, 0), (12, 200, 40, 255)];
        assert_eq!(extract_row(&pixels), vec!["#0cc828"]);
    }

    #[test]
    fn rejects_a_truncated_buffer() {
        assert!(extract(&[0, 0, 0, 255], 2, 2).is_empty());
        assert!(extract(&[], 0, 0).is_empty());
    }

    #[test]
    fn near_duplicate_shades_are_merged() {
        let pixels = [(255, 0, 0, 255), (250, 0, 0, 255), (0, 0, 255, 255)];
        let colors = extract_row(&pixels);
        assert_eq!(colors.len(), 2, "the two reds collapse into one entry");
    }

    #[test]
    fn extraction_is_deterministic() {
        let pixels = [
            (10, 20, 30, 255),
            (200, 30, 90, 255),
            (10, 20, 30, 255),
            (90, 200, 30, 255),
        ];
        assert_eq!(extract_row(&pixels), extract_row(&pixels));
    }

    #[test]
    fn sampling_reads_the_pixel_under_a_fraction() {
        let pixels = [
            (255, 0, 0, 255),
            (0, 255, 0, 255),
            (0, 0, 255, 255),
            (255, 255, 255, 255),
        ];
        let rgba = image(&pixels);
        // 2x2 image: the four fractions land on the four pixels.
        assert_eq!(sample(&rgba, 2, 2, 0.0, 0.0).as_deref(), Some("#ff0000"));
        assert_eq!(sample(&rgba, 2, 2, 1.0, 0.0).as_deref(), Some("#00ff00"));
        assert_eq!(sample(&rgba, 2, 2, 0.0, 1.0).as_deref(), Some("#0000ff"));
        assert_eq!(sample(&rgba, 2, 2, 1.0, 1.0).as_deref(), Some("#ffffff"));
    }

    #[test]
    fn sampling_rejects_points_without_a_colour() {
        let rgba = image(&[(9, 9, 9, 0)]);
        assert_eq!(sample(&rgba, 1, 1, 0.5, 0.5), None, "a clear pixel");
        assert_eq!(sample(&rgba, 0, 0, 0.5, 0.5), None, "no image");
        assert_eq!(sample(&rgba, 1, 1, f64::NAN, 0.5), None, "no position");
        assert_eq!(sample(&[], 1, 1, 0.5, 0.5), None, "no pixels");
    }
}
