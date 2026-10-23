//! Vertically stitching the frames of a scrolling capture.
//!
//! Two questions have to be answered, and nothing else: "is this frame the same as
//! the last one" (which means the page cannot scroll any further) and "how far did
//! the content move" (so only the genuinely new rows are appended). Keeping both
//! pure means the awkward part is testable without a screen.
//!
//! Every frame is compared with the one before it, never with the whole stitched
//! image: consecutive frames differ by nothing but a vertical shift, so the answer
//! is exact and errors cannot accumulate.

/// Rows carrying real content that a candidate offset must have compared before it
/// can be believed. Blank stretches of a page are ignored, so a dozen is already a
/// strong signal.
const MIN_COMPARED_ROWS: u32 = 12;

/// Share of those rows that has to line up exactly. A page that lazy-loads an image
/// or animates a banner mid-capture loses some rows, and that must not lose the whole
/// offset; the offset with the most matching rows wins regardless.
const MIN_MATCH_PERCENT: u32 = 85;

/// True when two frames are byte-for-byte the same, which is what "the page has
/// nowhere left to scroll" looks like.
pub fn frames_identical(previous: &[u8], next: &[u8]) -> bool {
    previous == next
}

/// How many rows the content moved up between two frames of the same window, or
/// `None` when they do not line up at all.
///
/// Rows that sit still in both frames are the window's own chrome — a tab strip, a
/// toolbar, a sticky page header — and are left out of the comparison, because they
/// "match" at every offset and would hide the answer.
pub fn scroll_offset(previous: &[u8], next: &[u8], width: u32, height: u32) -> Option<u32> {
    if width == 0 || height < 6 {
        return None;
    }
    let row_bytes = width as usize * 4;
    if previous.len() < row_bytes * height as usize || next.len() < row_bytes * height as usize {
        return None;
    }

    let before = scan(previous, row_bytes, height);
    let after = scan(next, row_bytes, height);

    // Everything identical from the top down is pinned in place by the window, not
    // by the page. Capped at a third so there is always something left to compare.
    let mut fixed = 0;
    while fixed < height / 3 && before.hashes[fixed as usize] == after.hashes[fixed as usize] {
        fixed += 1;
    }

    // Most rows that line up wins; a tie goes to the smaller move, because a page
    // that has scrolled a little is far likelier than one that has scrolled a lot.
    let mut best: Option<(u32, u32)> = None;
    for offset in 1..height.saturating_sub(fixed) {
        let mut compared = 0;
        let mut matched = 0;
        for row in fixed..(height - offset) {
            let (Some(above), Some(below)) = (
                before.signature(row + offset),
                after.signature(row),
            ) else {
                continue;
            };
            compared += 1;
            if above == below {
                matched += 1;
            }
        }

        if compared < MIN_COMPARED_ROWS || matched * 100 < compared * MIN_MATCH_PERCENT {
            continue;
        }
        if best.is_none_or(|(best_matched, _)| matched > best_matched) {
            best = Some((matched, offset));
        }
    }

    best.map(|(_, offset)| offset)
}

/// Per-row hashes, plus whether each row carries anything worth comparing.
struct Scan {
    hashes: Vec<u64>,
    distinctive: Vec<bool>,
}

impl Scan {
    /// The hash of a row, but only if that row is more than one flat colour: a blank
    /// strip matches at every offset and cannot say where the page moved.
    fn signature(&self, row: u32) -> Option<u64> {
        let row = row as usize;
        self.distinctive[row].then(|| self.hashes[row])
    }
}

fn scan(rgba: &[u8], row_bytes: usize, height: u32) -> Scan {
    let mut hashes = Vec::with_capacity(height as usize);
    let mut distinctive = Vec::with_capacity(height as usize);

    for row in 0..height as usize {
        let start = row * row_bytes;
        let slice = &rgba[start..start + row_bytes];
        let mut varies = false;
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;

        for (index, byte) in slice.iter().enumerate() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100_0000_01b3);
            // Compare with the same channel of the pixel to the left: a row of one
            // flat colour repeats its first pixel forever, whatever colour that is.
            if index >= 4 && *byte != slice[index - 4] {
                varies = true;
            }
        }

        hashes.push(hash);
        distinctive.push(varies);
    }

    Scan { hashes, distinctive }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIDTH: u32 = 40;
    const HEIGHT: u32 = 120;
    /// Rows of fixed browser chrome: tab strip, toolbar and the browser's own edge.
    /// They never move, whatever the page does.
    const CHROME: u32 = 30;

    /// A row of the virtual page, written into `pixels`. Some rows are blank, like
    /// the gaps between blocks of text, which is what makes matching naive.
    fn push_page_row(pixels: &mut Vec<u8>, index: u32) {
        if index % 7 < 2 {
            for _ in 0..WIDTH {
                pixels.extend_from_slice(&[255, 255, 255, 255]);
            }
            return;
        }
        let mut hash = index.wrapping_mul(2_654_435_761) ^ 0x9E37_79B9;
        hash ^= hash >> 15;
        hash = hash.wrapping_mul(2_246_822_519);
        hash ^= hash >> 13;
        // Varies across the row as well as down it, like a line of text.
        for x in 0..WIDTH {
            pixels.extend_from_slice(&[
                hash as u8,
                (hash >> 8) as u8 ^ (x as u8),
                x as u8,
                255,
            ]);
        }
    }

    /// How many rows the page has; below this it is blank, so scrolling stops.
    const PAGE_ROWS: u32 = 130;

    /// A frame of the window with the page scrolled to `position`.
    fn frame(position: u32) -> Vec<u8> {
        let mut pixels = Vec::with_capacity((WIDTH * HEIGHT * 4) as usize);
        for row in 0..HEIGHT {
            if row < CHROME {
                // The tab strip and toolbar: busy, and never moving.
                for x in 0..WIDTH {
                    pixels.extend_from_slice(&[32, 34, (x * 3) as u8, 255]);
                }
            } else {
                let page_row = position + row - CHROME;
                if page_row < PAGE_ROWS {
                    push_page_row(&mut pixels, page_row);
                } else {
                    for _ in 0..WIDTH {
                        pixels.extend_from_slice(&[255, 255, 255, 255]);
                    }
                }
            }
        }
        pixels
    }

    /// A single row of the virtual page, for comparing against the stitched result.
    fn page_row(index: u32) -> Vec<u8> {
        let mut row = Vec::with_capacity((WIDTH * 4) as usize);
        push_page_row(&mut row, index);
        row
    }

    #[test]
    fn a_small_scroll_is_measured_exactly() {
        assert_eq!(scroll_offset(&frame(0), &frame(25), WIDTH, HEIGHT), Some(25));
    }

    #[test]
    fn a_scroll_of_most_of_a_frame_is_still_measured() {
        assert_eq!(scroll_offset(&frame(0), &frame(70), WIDTH, HEIGHT), Some(70));
    }

    #[test]
    fn a_capture_that_did_not_move_is_identical() {
        assert!(frames_identical(&frame(0), &frame(0)));
        assert!(!frames_identical(&frame(0), &frame(1)));
    }

    #[test]
    fn a_blank_page_offers_no_evidence_of_movement() {
        let mut blank = Vec::new();
        for row in 0..HEIGHT {
            let colour = if row < CHROME { [32, 34, 38, 255] } else { [255, 255, 255, 255] };
            for _ in 0..WIDTH {
                blank.extend_from_slice(&colour);
            }
        }
        assert!(frames_identical(&blank, &blank.clone()), "a still frame is still");
        assert_eq!(scroll_offset(&blank, &blank, WIDTH, HEIGHT), None);
    }

    #[test]
    fn unrelated_frames_do_not_line_up() {
        assert_eq!(scroll_offset(&frame(0), &frame(5_000), WIDTH, HEIGHT), None);
    }

    #[test]
    fn buffers_that_do_not_match_their_size_are_rejected() {
        let short = vec![0u8; 16];
        assert_eq!(scroll_offset(&short, &short, WIDTH, HEIGHT), None);
        assert_eq!(scroll_offset(&[], &[], 0, 0), None);
    }

    /// End to end: a page taller than the window, scrolled a notch at a time, has to
    /// be rebuilt row for row — no repeats, no gaps, no split lines — and stop
    /// exactly when the page can no longer move.
    #[test]
    fn a_page_is_rebuilt_row_for_row() {
        let row_bytes = (WIDTH * 4) as usize;
        let visible_rows = HEIGHT - CHROME;
        let furthest = PAGE_ROWS - visible_rows;
        let step = 25;

        let mut stitched = frame(0);
        let mut current = frame(0);
        let mut position = 0;
        let mut steps = 0;

        loop {
            let next_position = (position + step).min(furthest);
            let next = frame(next_position);
            if frames_identical(&current, &next) {
                break;
            }
            let offset = scroll_offset(&current, &next, WIDTH, HEIGHT)
                .unwrap_or_else(|| panic!("no offset at position {position}"));

            let fresh_start = (HEIGHT - offset) as usize;
            stitched.extend_from_slice(&next[fresh_start * row_bytes..]);

            current = next;
            position = next_position;
            steps += 1;
            assert!(steps < 20, "the loop never reached the bottom");
        }

        assert_eq!(position, furthest, "stopped before the end of the page");
        assert_eq!(stitched.len() / row_bytes, (CHROME + PAGE_ROWS) as usize, "wrong height");

        for row in CHROME..(CHROME + PAGE_ROWS) {
            let expected = page_row(row - CHROME);
            let at = row as usize * row_bytes;
            assert_eq!(&stitched[at..at + row_bytes], &expected[..], "row {row}");
        }
    }
}
