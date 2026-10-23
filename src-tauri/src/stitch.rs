//! Vertically stitching the frames of a scrolling capture.
//!
//! The only thing this needs to answer is "how many rows at the top of the new
//! frame were already in the old one" — every other decision belongs to
//! `capture.rs`. Keeping it pure means the awkward part can be tested without a
//! screen.

/// Mean per-channel difference a matched block of rows may have. Rendering a
/// scrolled window is not byte-identical between passes: subpixel text and
/// antialiasing move around, and a sticky header can redraw.
const TOLERANCE: u32 = 14;

/// How many of the newest rows are compared. Big enough to be unambiguous, small
/// enough that a sticky header or a lazily-loaded image cannot dominate it.
const MAX_TEMPLATE_ROWS: u32 = 48;

/// Rows of `next` that also appear at the bottom of `previous`, or `None` when the
/// two frames do not line up at all (the page jumped, or nothing scrolled).
///
/// Both buffers are RGBA and share a width.
pub fn overlap_rows(
    previous: &[u8],
    width: u32,
    previous_height: u32,
    next: &[u8],
    next_height: u32,
) -> Option<u32> {
    if width == 0 || previous_height == 0 || next_height == 0 {
        return None;
    }

    let row_bytes = width as usize * 4;
    if previous.len() < row_bytes * previous_height as usize
        || next.len() < row_bytes * next_height as usize
    {
        return None;
    }

    // Bands are taken from the lower middle of the frame, never the top: a
    // browser's tab strip, its toolbar and any sticky page header stay put when
    // the page scrolls, so a band up there matches the previous frame at every
    // offset and every frame looks entirely already-stitched.
    let rows = (next_height / 50).clamp(6, 14);
    let bands: Vec<(u32, u32)> = [25, 35, 45]
        .iter()
        .map(|percent| (next_height * percent / 100, rows))
        .filter(|(start, rows)| start + rows <= next_height && start + rows <= previous_height)
        .collect();
    if bands.is_empty() {
        return None;
    }

    // A handful of pixels per row is plenty to tell rows apart.
    let column_step = (width as usize / 48).max(1) * 4;
    let row_step = (rows / 4).max(1);

    // The largest overlap that still contains every band is where the search
    // starts: past that the bands would fall off the end of the previous frame.
    let deepest = bands.iter().map(|(start, rows)| start + rows).max()?;

    let mut best: Option<(u32, u32)> = None;
    for overlap in (deepest..=previous_height).rev() {
        let align = previous_height - overlap;
        // Every band has to agree, so the worst one decides the score: a header
        // band that matches trivially cannot carry a wrong offset.
        let mut worst = 0;
        for (start, band_rows) in &bands {
            let score = block_difference(
                previous,
                row_bytes,
                align + start,
                next,
                row_bytes,
                *start,
                *band_rows,
                row_step,
                column_step,
            );
            worst = worst.max(score);
        }
        if best.is_none_or(|(known, _)| worst < known) {
            best = Some((worst, overlap));
        }
    }

    match best {
        Some((score, overlap)) if score <= TOLERANCE * 100 => Some(overlap),
        _ => None,
    }
}

/// Mean absolute per-channel difference between two blocks of rows.
#[allow(clippy::too_many_arguments)]
fn block_difference(
    first: &[u8],
    first_row_bytes: usize,
    first_row: u32,
    second: &[u8],
    second_row_bytes: usize,
    second_row: u32,
    rows: u32,
    row_step: u32,
    column_step: usize,
) -> u32 {
    let mut total: u64 = 0;
    let mut samples: u64 = 0;

    let mut row = 0;
    while row < rows {
        let first_start = (first_row as usize + row as usize) * first_row_bytes;
        let second_start = (second_row as usize + row as usize) * second_row_bytes;
        let mut column = 0;
        while column + 4 <= first_row_bytes {
            for channel in 0..3 {
                let a = first[first_start + column + channel] as i32;
                let b = second[second_start + column + channel] as i32;
                total += (a - b).unsigned_abs() as u64;
                samples += 1;
            }
            column += column_step;
        }
        row += row_step;
    }

    if samples == 0 {
        return u32::MAX;
    }
    // Hundredths of a level, so that a two-level difference between neighbouring
    // rows survives the integer division instead of rounding to a perfect match.
    ((total * 100) / samples) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A frame whose every row is unique, so a wrong overlap scores badly rather
    /// than accidentally matching (a repeating pattern would alias).
    fn frame(width: u32, height: u32, first_row: u32) -> Vec<u8> {
        let mut pixels = Vec::with_capacity((width * height * 4) as usize);
        for row in 0..height {
            let absolute = first_row + row;
            let low = (absolute & 0xFF) as u8;
            let high = ((absolute >> 8) & 0xFF) as u8;
            for _ in 0..width {
                pixels.extend_from_slice(&[low, high, 3, 255]);
            }
        }
        pixels
    }

    #[test]
    fn the_shared_rows_of_a_scroll_are_found() {
        // A 400-row page, scrolled by 150 rows: the second frame's top 250 rows
        // are the bottom of the first.
        let previous = frame(64, 400, 0);
        let next = frame(64, 400, 150);

        let overlap = overlap_rows(&previous, 64, 400, &next, 400);
        assert_eq!(overlap, Some(250));
    }

    #[test]
    fn a_header_that_never_moves_does_not_hide_the_scroll() {
        // Chrome's tab strip and a sticky page header occupy the top of both
        // frames and never change: 100 rows of unchanging chrome over a page that
        // scrolled by 120 rows.
        let mut previous = frame(64, 400, 0);
        let mut next = frame(64, 400, 120);
        for row in 0..100 {
            let fill = [9u8, 9, 9, 255];
            for pixel in 0..64 {
                let at = (row * 64 + pixel) * 4;
                previous[at..at + 4].copy_from_slice(&fill);
                next[at..at + 4].copy_from_slice(&fill);
            }
        }

        assert_eq!(overlap_rows(&previous, 64, 400, &next, 400), Some(280));
    }

    #[test]
    fn a_frame_that_did_not_scroll_is_all_overlap() {
        let previous = frame(64, 400, 0);
        let next = frame(64, 400, 0);

        let overlap = overlap_rows(&previous, 64, 400, &next, 400);
        assert_eq!(overlap, Some(400), "nothing new to append");
    }

    #[test]
    fn unrelated_frames_do_not_stitch() {
        // A page that jumped somewhere else: the rows have nothing in common.
        let previous = frame(64, 400, 0);
        let next = frame(64, 400, 0).into_iter().map(|value| 255 - value).collect::<Vec<_>>();

        assert_eq!(overlap_rows(&previous, 64, 400, &next, 400), None);
    }

    #[test]
    fn a_small_scroll_is_still_matched() {
        let previous = frame(64, 400, 0);
        let next = frame(64, 400, 20);

        assert_eq!(overlap_rows(&previous, 64, 400, &next, 400), Some(380));
    }

    #[test]
    fn buffers_that_do_not_match_their_size_are_rejected() {
        let short = vec![0u8; 16];
        assert_eq!(overlap_rows(&short, 64, 400, &short, 400), None);
        assert_eq!(overlap_rows(&[], 0, 0, &[], 0), None);
    }
}
