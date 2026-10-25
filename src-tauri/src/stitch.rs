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

/// Widest scroll bar that will be looked for along the right edge.
const MAX_SCROLLBAR_PX: usize = 32;
/// Narrowest, so a flat one-pixel line is not mistaken for a bar.
const MIN_SCROLLBAR_PX: usize = 6;
/// A row of the band counts as flat when its channels differ by no more than this.
const FLAT_TOLERANCE: i32 = 2;

/// Width of the scroll bar down the right edge of a frame, in pixels, or 0 for none.
///
/// A scroll bar is window furniture that happens to sit inside the page's pixels, and
/// it is drawn at a different place in every frame, so a stitched image would repeat
/// it down the side. It is recognised by its own shape rather than by a width taken
/// from the system: a bar is a band of columns that is *flat* across (a track has the
/// same colour all the way through) with one run of a different colour inside it (the
/// thumb) covering part of the height. Content at the right edge — a margin, an image —
/// is not flat across the whole band, and an overlay-style bar has the page showing
/// through, so both are left alone.
pub fn scrollbar_width(frame: &[u8], width: u32, height: u32) -> usize {
    let columns = width as usize;
    let rows = height as usize;
    if rows < 40 || columns <= MIN_SCROLLBAR_PX + 1 {
        return 0;
    }
    if frame.len() != columns * rows * 4 {
        return 0;
    }
    // A bar can be most of a narrow frame but never all of it: some content has to
    // be left behind for the capture to be worth cropping.
    let widest = MAX_SCROLLBAR_PX.min(columns - MIN_SCROLLBAR_PX);

    // Widest first: a bar is at least as wide as any flat run inside it.
    for band in (MIN_SCROLLBAR_PX..=widest).rev() {
        let left = columns - band;
        if band_is_scrollbar(frame, columns, rows, left, band) {
            return band;
        }
    }
    0
}

/// One band of columns, judged on how it is painted rather than on where it is.
fn band_is_scrollbar(frame: &[u8], columns: usize, rows: usize, left: usize, band: usize) -> bool {
    let pixel = |row: usize, column: usize| -> [i32; 4] {
        let at = (row * columns + column) * 4;
        [
            frame[at] as i32,
            frame[at + 1] as i32,
            frame[at + 2] as i32,
            frame[at + 3] as i32,
        ]
    };
    let close = |a: [i32; 4], b: [i32; 4]| -> bool {
        (0..4).all(|channel| (a[channel] - b[channel]).abs() <= FLAT_TOLERANCE)
    };

    // A bar is a simple surface: every row across it is one colour, or two where the
    // thumb is inset. Page content almost never is, which is what tells them apart.
    let mut simple_rows = 0usize;
    for row in 0..rows {
        let mut seen: [[i32; 4]; 3] = [[0; 4]; 3];
        let mut distinct = 0usize;
        for column in left..left + band {
            let value = pixel(row, column);
            if !seen[..distinct].iter().any(|known| close(*known, value)) {
                if distinct == seen.len() {
                    distinct += 1;
                    break;
                }
                seen[distinct] = value;
                distinct += 1;
            }
        }
        if distinct <= 2 {
            simple_rows += 1;
        }
    }
    if (simple_rows as f64) < rows as f64 * 0.95 {
        return false;
    }

    // The track runs the full width of the bar, so every column is mostly that one
    // colour. A band that strays into the page cannot say the same.
    let middle = left + band / 2;
    let mut counts: Vec<([i32; 4], usize)> = Vec::new();
    for row in 0..rows {
        let sample = pixel(row, middle);
        match counts.iter_mut().find(|(colour, _)| close(*colour, sample)) {
            Some((_, count)) => *count += 1,
            None => counts.push((sample, 1)),
        }
    }
    let Some((track, _)) = counts.iter().max_by_key(|(_, count)| *count) else {
        return false;
    };
    for column in left..left + band {
        let track_rows = (0..rows)
            .filter(|row| close(*track, pixel(*row, column)))
            .count();
        if (track_rows as f64) < rows as f64 * 0.6 {
            return false;
        }
    }

    // The thumb is the one run of a different colour, long enough to be a thumb and
    // short enough to leave a track showing. Rounded ends and antialiasing split it
    // into a few stretches of a handful of rows, which are not runs of their own.
    let noise = (rows / 100).max(4);
    let mut runs: Vec<usize> = Vec::new();
    let mut run_rows = 0usize;
    for row in 0..rows {
        if !close(*track, pixel(row, middle)) {
            run_rows += 1;
        } else if run_rows > 0 {
            runs.push(run_rows);
            run_rows = 0;
        }
    }
    if run_rows > 0 {
        runs.push(run_rows);
    }
    let real: Vec<usize> = runs.into_iter().filter(|length| *length >= noise).collect();
    if real.len() != 1 {
        return false;
    }
    let share = real[0] as f64 / rows as f64;
    (0.02..=0.9).contains(&share)
}

/// Drops the right-hand `drop` columns from a frame.
pub fn crop_right(frame: &[u8], width: u32, height: u32, drop: usize) -> Vec<u8> {
    let columns = width as usize;
    let rows = height as usize;
    if drop == 0 || drop >= columns || frame.len() != columns * rows * 4 {
        return frame.to_vec();
    }
    let keep = columns - drop;
    let mut cropped = Vec::with_capacity(keep * rows * 4);
    for row in 0..rows {
        let start = row * columns * 4;
        cropped.extend_from_slice(&frame[start..start + keep * 4]);
    }
    cropped
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
    /// Diagnostic: the bar detector against real frames, when a failed capture has
    /// left some beside the log. Nothing to check without them.
    #[test]
    fn probe_saved_frames_for_a_scroll_bar() {
        for name in ["copypaste-scroll-before.png", "copypaste-scroll-after.png"] {
            let path = std::env::temp_dir().join(name);
            let Ok(image) = image::open(&path) else {
                continue;
            };
            let rgba = image.to_rgba8();
            let (width, height) = (rgba.width(), rgba.height());
            let bar = scrollbar_width(rgba.as_raw(), width, height);
            let without = crop_right(rgba.as_raw(), width, height, bar);
            let again = scrollbar_width(&without, width - bar as u32, height);
            eprintln!(
                "{name}: {width}x{height} bar={bar}, and {again} once that is cropped"
            );
        }
    }

    #[test]
    fn probe_two_saved_frames() {        let (Ok(before), Ok(after)) = (
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

    /// Paints a scroll bar down the right-hand `band` columns: a track for the whole
    /// height with a thumb from `thumb_top` to `thumb_top + thumb_rows`.
    fn with_scrollbar(mut pixels: Vec<u8>, band: usize, thumb_top: usize, thumb_rows: usize) -> Vec<u8> {
        let columns = WIDTH as usize;
        for row in 0..HEIGHT as usize {
            let in_thumb = row >= thumb_top && row < thumb_top + thumb_rows;
            let value: [u8; 4] = if in_thumb {
                [150, 150, 150, 255]
            } else {
                [241, 241, 241, 255]
            };
            for column in columns - band..columns {
                let at = (row * columns + column) * 4;
                pixels[at..at + 4].copy_from_slice(&value);
            }
        }
        pixels
    }

    #[test]
    fn a_scroll_bar_on_the_right_is_measured() {
        let flat = with_scrollbar(frame(0), 16, 20, 30);
        assert_eq!(scrollbar_width(&flat, WIDTH, HEIGHT), 16);
        // Where the thumb sits makes no difference, and neither does the theme.
        let near_top = with_scrollbar(frame(0), 16, 0, 8);
        assert_eq!(scrollbar_width(&near_top, WIDTH, HEIGHT), 16);
        let mut dark = with_scrollbar(frame(0), 16, 40, 30);
        for row in 0..HEIGHT as usize {
            for column in WIDTH as usize - 16..WIDTH as usize {
                let at = (row * WIDTH as usize + column) * 4;
                // Dark track, lighter thumb: the same shape, inverted.
                dark[at] = if dark[at] == 150 { 120 } else { 32 };
                dark[at + 1] = dark[at];
                dark[at + 2] = dark[at];
            }
        }
        assert_eq!(scrollbar_width(&dark, WIDTH, HEIGHT), 16);
    }

    #[test]
    fn page_content_is_not_mistaken_for_a_scroll_bar() {
        // The fixture is text and margins right to the edge: not simple across a band.
        assert_eq!(scrollbar_width(&frame(0), WIDTH, HEIGHT), 0);

        // A plain white margin is simple, but has no thumb run in it, so it stays.
        let mut plain = frame(0);
        for row in 0..HEIGHT as usize {
            for column in WIDTH as usize - 16..WIDTH as usize {
                let at = (row * WIDTH as usize + column) * 4;
                plain[at..at + 4].copy_from_slice(&[255, 255, 255, 255]);
            }
        }
        assert_eq!(scrollbar_width(&plain, WIDTH, HEIGHT), 0);

        // An overlay bar has the page showing through it, so the band is not simple.
        let overlay = with_scrollbar(frame(0), 4, 20, 30);
        assert_eq!(scrollbar_width(&overlay, WIDTH, HEIGHT), 0);

        // Once the bar is gone, the frame it left behind is just page.
        let bar = with_scrollbar(frame(0), 16, 20, 30);
        let cropped = crop_right(&bar, WIDTH, HEIGHT, 16);
        assert_eq!(
            scrollbar_width(&cropped, WIDTH - 16, HEIGHT),
            0,
            "a cropped frame has no bar left to find"
        );
    }

    #[test]
    fn cropping_the_bar_leaves_the_rest_of_the_row_alone() {
        let pixels = with_scrollbar(frame(0), 16, 20, 30);
        let cropped = crop_right(&pixels, WIDTH, HEIGHT, 16);
        let keep = WIDTH as usize - 16;
        assert_eq!(cropped.len(), keep * HEIGHT as usize * 4);
        for row in [0usize, 50, HEIGHT as usize - 1] {
            let from = row * WIDTH as usize * 4;
            let to = row * keep * 4;
            assert_eq!(
                &cropped[to..to + keep * 4],
                &pixels[from..from + keep * 4],
                "row {row} lost pixels it should have kept"
            );
        }
        // Nothing to drop is not an error, and neither is a size that does not fit.
        assert_eq!(crop_right(&pixels, WIDTH, HEIGHT, 0), pixels);
        assert_eq!(crop_right(&pixels, WIDTH, HEIGHT, WIDTH as usize), pixels);
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
