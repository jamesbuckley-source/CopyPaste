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

/// Mean per-channel difference at which the chosen offset is still trusted.
const MAX_MEAN_DIFFERENCE: f64 = 6.0;

/// How far below the middle of the distribution the best offset has to sit before it
/// is believed. Text re-renders with sub-pixel differences when it scrolls, so the
/// frames are never identical; a true alignment is still far closer than the field.
const MIN_IMPROVEMENT: f64 = 0.6;

/// Frames this close to each other hold the same pixels, so nothing scrolled.
const STILL_MEAN_DIFFERENCE: f64 = 2.0;

/// At most this many rows and columns are sampled.
const MAX_ROWS: usize = 220;
const MAX_COLUMNS: usize = 64;

/// True when two frames are byte-for-byte the same, which is what "the page has
/// nowhere left to scroll" looks like.
pub fn frames_identical(previous: &[u8], next: &[u8]) -> bool {
    previous == next
}

/// How many rows the content moved up between two frames of the same window, or
/// `None` when they do not line up at all.
///
/// The two frames are compared by mean absolute difference rather than by looking
/// for identical pixels: a scrolled page is re-rendered with sub-pixel glyph
/// differences, so almost nothing matches byte for byte even at the true offset,
/// while the difference still bottoms out sharply there. Parts of the window that
/// do not scroll — a title bar, a sidebar — add the same amount to every candidate
/// and so do not affect which offset wins.
pub fn scroll_offset(previous: &[u8], next: &[u8], width: u32, height: u32) -> Option<u32> {
    if width < 4 || height < 16 {
        return None;
    }
    let row_bytes = width as usize * 4;
    if previous.len() < row_bytes * height as usize || next.len() < row_bytes * height as usize {
        return None;
    }

    let columns: Vec<usize> = (0..width as usize)
        .step_by((width as usize / MAX_COLUMNS).max(1))
        .collect();
    let rows: Vec<u32> = (0..height)
        .step_by(((height as usize) / MAX_ROWS).max(1))
        .collect();

    // Rows and columns that are identical at the same place in both frames are the
    // parts of the window that do not scroll: a title bar, a left-hand sidebar, a
    // status line. They carry no information about the offset and only dilute the
    // score, so they are dropped.
    let same_at = |row: u32, column: usize| -> bool {
        let above = row as usize * row_bytes + column * 4;
        previous[above..above + 4] == next[above..above + 4]
    };
    let moving_rows: Vec<u32> = rows
        .iter()
        .copied()
        .filter(|&row| !columns.iter().all(|&column| same_at(row, column)))
        .collect();
    if moving_rows.len() < 8 {
        return None;
    }
    let moving_columns: Vec<usize> = columns
        .iter()
        .copied()
        .filter(|&column| !moving_rows.iter().all(|&row| same_at(row, column)))
        .collect();
    let (rows, columns) = if moving_columns.len() >= 4 {
        (moving_rows, moving_columns)
    } else {
        (rows, columns)
    };

    let difference = |offset: u32| -> Option<f64> {
        let mut total = 0f64;
        let mut samples = 0u32;
        for &row in &rows {
            if row + offset >= height {
                continue;
            }
            for &column in &columns {
                let above = (row + offset) as usize * row_bytes + column * 4;
                let below = row as usize * row_bytes + column * 4;
                for channel in 0..3 {
                    total += f64::from(
                        (previous[above + channel] as i32 - next[below + channel] as i32).abs(),
                    );
                }
                samples += 1;
            }
        }
        (samples > 0).then(|| total / (f64::from(samples) * 3.0))
    };

    let still = difference(0)?;
    if still <= STILL_MEAN_DIFFERENCE {
        // Nothing moved: the caller decides what that means.
        return None;
    }

    // A wheel notch never moves more than a screenful, so the search stops there.
    let limit = (height / 2 + 32).min(height - 8);
    let mut scores: Vec<(f64, u32)> = Vec::with_capacity(limit as usize);
    for offset in 1..=limit {
        if let Some(score) = difference(offset) {
            scores.push((score, offset));
        }
    }
    if scores.is_empty() {
        return None;
    }

    let (best, offset) = scores
        .iter()
        .copied()
        .fold((f64::MAX, 0u32), |lowest, entry| {
            if entry.0 < lowest.0 { entry } else { lowest }
        });

    // A real alignment stands out against the run of possible offsets; a page that
    // merely happens to fit somewhere does not. Comparing against the middle of the
    // distribution keeps this working on busy pages as well as sparse ones.
    let mut sorted: Vec<f64> = scores.iter().map(|(score, _)| *score).collect();
    sorted.sort_by(f64::total_cmp);
    let median = sorted[sorted.len() / 2];

    let clear = best <= MAX_MEAN_DIFFERENCE
        || (best <= median * MIN_IMPROVEMENT && best < still);
    clear.then_some(offset)
}

/// Columns that two frames of the same window agree on, within a mean per-channel
/// difference of `tolerance`.
///
/// A sidebar or a gutter is drawn at the same place in every frame, so appending
/// rows from later frames would otherwise repeat it all the way down a stitched
/// image. `tolerance` leaves room for an antialiased edge being repainted.
pub fn frozen_columns(a: &[u8], b: &[u8], width: u32, height: u32, tolerance: f32) -> Vec<bool> {
    let columns = width as usize;
    let mut frozen = vec![false; columns];
    if columns == 0 || height == 0 || a.len() != b.len() || a.len() != columns * height as usize * 4
    {
        return frozen;
    }

    for (column, is_frozen) in frozen.iter_mut().enumerate() {
        let mut total = 0u64;
        for row in 0..height as usize {
            let at = (row * columns + column) * 4;
            for channel in 0..3 {
                total += (a[at + channel] as i32 - b[at + channel] as i32).unsigned_abs() as u64;
            }
        }
        let samples = height as u64 * 3;
        *is_frozen = (total as f64 / samples as f64) <= tolerance as f64;
    }

    frozen
}

/// Holds the frozen columns at the value they had at the fold, so a fixed sidebar
/// ends there instead of scrolling with the page it was never part of.
pub fn hold_frozen_columns(stitched: &mut [u8], width: u32, fold: u32, frozen: &[bool]) {
    let columns = width as usize;
    if columns == 0 || fold == 0 || frozen.len() != columns {
        return;
    }
    let row_bytes = columns * 4;
    if stitched.len() < row_bytes * fold as usize || stitched.len() % row_bytes != 0 {
        return;
    }

    let start = (fold as usize - 1) * row_bytes;
    let last = stitched[start..start + row_bytes].to_vec();
    for row in fold as usize..stitched.len() / row_bytes {
        for (column, is_frozen) in frozen.iter().enumerate() {
            if !*is_frozen {
                continue;
            }
            let at = row * row_bytes + column * 4;
            stitched[at..at + 4].copy_from_slice(&last[column * 4..column * 4 + 4]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const WIDTH: u32 = 40;
    const HEIGHT: u32 = 120;
    /// Rows of fixed chrome at the top: a title bar and toolbar.
    const CHROME: u32 = 20;
    /// Columns of fixed pane on the left: a sidebar that does not scroll.
    const SIDEBAR: u32 = 12;
    /// How many rows the virtual page has; past the end it is blank.
    const PAGE_ROWS: u32 = 130;

    fn hash_of(a: u32, b: u32) -> u32 {
        let mut hash = a.wrapping_mul(2_654_435_761) ^ b.wrapping_mul(0x9E37_79B9);
        hash ^= hash >> 15;
        hash = hash.wrapping_mul(2_246_822_519);
        hash ^= hash >> 13;
        hash
    }

    fn colour(seed: u32) -> [u8; 4] {
        [seed as u8, (seed >> 8) as u8, (seed >> 16) as u8, 255]
    }

    /// A frame of the window with the page scrolled to `position`.
    ///
    /// It has both things that defeat whole-row matching: a fixed title bar and a
    /// fixed left sidebar, each identical at the same screen position in every
    /// frame while the pane beside them scrolls.
    fn frame(position: u32) -> Vec<u8> {
        let mut pixels = Vec::with_capacity((WIDTH * HEIGHT * 4) as usize);
        for row in 0..HEIGHT {
            for column in 0..WIDTH {
                let value = if column < SIDEBAR {
                    // Sidebar: busy, and pinned to the screen.
                    colour(hash_of(9_000 + row, column))
                } else if row < CHROME {
                    // Title bar: busy, and pinned to the screen.
                    colour(hash_of(7_000 + row, column))
                } else {
                    let page_row = position + row - CHROME;
                    if page_row < PAGE_ROWS && page_row % 9 >= 2 {
                        colour(hash_of(page_row, column))
                    } else {
                        // Blank page rows: the gaps between blocks of text.
                        [255, 255, 255, 255]
                    }
                };
                pixels.extend_from_slice(&value);
            }
        }
        pixels
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
    fn a_pinned_sidebar_and_title_bar_do_not_hide_the_scroll() {
        // The case that broke the real capture: a window whose left pane and title
        // bar never move while the pane beside them scrolls.
        let offset = scroll_offset(&frame(0), &frame(40), WIDTH, HEIGHT);
        assert_eq!(offset, Some(40));
    }

    #[test]
    fn a_capture_that_did_not_move_is_identical() {
        assert!(frames_identical(&frame(0), &frame(0)));
        assert!(!frames_identical(&frame(0), &frame(1)));
    }

    #[test]
    fn a_window_with_nothing_scrolling_has_no_offset() {
        // Everything pinned: the title bar, the sidebar, and a blank page.
        let still = frame(PAGE_ROWS);
        assert_eq!(scroll_offset(&still, &still, WIDTH, HEIGHT), None);
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

    /// Runs the matcher over two saved PNGs, for diagnosing a real capture:
    /// `SCROLL_PROBE_BEFORE=a.png SCROLL_PROBE_AFTER=b.png cargo test probe_two_frames`.
    #[test]
    fn probe_two_saved_frames() {
        let (Ok(before), Ok(after)) = (
            std::env::var("SCROLL_PROBE_BEFORE"),
            std::env::var("SCROLL_PROBE_AFTER"),
        ) else {
            return;
        };
        let before = image::open(&before).expect("a readable image").to_rgba8();
        let after = image::open(&after).expect("a readable image").to_rgba8();
        assert_eq!(before.dimensions(), after.dimensions(), "the frames must match");
        let (width, height) = before.dimensions();

        let start = std::time::Instant::now();
        let offset = scroll_offset(before.as_raw(), after.as_raw(), width, height);
        println!("{width}x{height}: offset {offset:?} in {:?}", start.elapsed());
        assert!(offset.is_some(), "the frames did not line up");
    }

    /// The sidebar is pinned to the window, so it must appear once: below the fold
    /// it holds the row it already ended on instead of repeating the sidebar again.
    #[test]
    fn a_fixed_sidebar_is_not_repeated_down_the_stitch() {
        let row_bytes = (WIDTH * 4) as usize;
        let fold = HEIGHT;
        let mut stitched = frame(0);
        stitched.extend_from_slice(&frame(25)[((HEIGHT - 25) as usize) * row_bytes..]);

        let frozen = frozen_columns(&frame(0), &frame(25), WIDTH, HEIGHT, 0.0);
        assert!(
            frozen[..SIDEBAR as usize].iter().all(|f| *f),
            "the sidebar columns were not found"
        );
        assert!(
            frozen[SIDEBAR as usize..].iter().all(|f| !*f),
            "the scrolling pane was mistaken for a fixed column"
        );

        let last_sidebar_row = (fold as usize - 1) * row_bytes;
        let expected: Vec<u8> = (0..SIDEBAR as usize)
            .flat_map(|column| {
                let at = last_sidebar_row + column * 4;
                stitched[at..at + 4].to_vec()
            })
            .collect();

        hold_frozen_columns(&mut stitched, WIDTH, fold, &frozen);

        for row in fold as usize..stitched.len() / row_bytes {
            let at = row * row_bytes;
            assert_eq!(
                &stitched[at..at + SIDEBAR as usize * 4],
                &expected[..],
                "row {row} repeated the sidebar"
            );
        }
        // The scrolling pane is untouched.
        let source = frame(25);
        let at = fold as usize * row_bytes + 30 * 4;
        let from = ((HEIGHT - 25) as usize) * row_bytes + 30 * 4;
        assert_eq!(&stitched[at..at + 4], &source[from..from + 4]);
    }

    #[test]
    fn holding_frozen_columns_ignores_a_frame_that_does_not_fit() {
        let mut pixels = frame(0);
        let before = pixels.clone();
        hold_frozen_columns(&mut pixels, WIDTH, HEIGHT + 1, &vec![true; WIDTH as usize]);
        hold_frozen_columns(&mut pixels, WIDTH, HEIGHT, &[true; 3]);
        assert_eq!(pixels, before, "a mismatched size must not rewrite anything");
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

        // Every stitched row below the chrome holds the page row it should, in the
        // scrolling pane. The pinned sidebar columns are excluded: they are copied
        // from whichever frame contributed the row.
        let rows = stitched.len() / row_bytes;
        for row in CHROME..(CHROME + PAGE_ROWS).min(rows as u32) {
            let page_row = row - CHROME;
            // Blank page rows are legitimately white, exactly as the page renders them.
            let expected = if page_row % 9 >= 2 {
                colour(hash_of(page_row, 30))
            } else {
                [255, 255, 255, 255]
            };
            let at = row as usize * row_bytes + 30 * 4;
            assert_eq!(&stitched[at..at + 4], &expected[..], "row {row} of the scrolling pane");
        }
    }
}
