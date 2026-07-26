//! Tests for the comparison.

use super::*;

const A4: PageSize = PageSize {
    width_mm: 210.0,
    height_mm: 297.0,
};

/// A greyscale page from a picture: `#` is ink, `.` is paper.
fn page(rows: &[&str]) -> (Vec<u8>, usize, usize) {
    let width = rows[0].len();
    let mut gray = Vec::with_capacity(width * rows.len());
    for row in rows {
        assert_eq!(row.len(), width, "ragged test picture");
        gray.extend(row.chars().map(|c| if c == '#' { 0u8 } else { 255u8 }));
    }
    (gray, width, rows.len())
}

fn mask_of(rows: &[&str]) -> Mask {
    let (gray, w, h) = page(rows);
    ink_mask(&gray, w, h, DEFAULT_INK_THRESHOLD)
}

/// Draw a mask back out, for comparing against a picture.
fn draw(mask: &Mask) -> Vec<String> {
    (0..mask.height)
        .map(|y| {
            (0..mask.width)
                .map(|x| if mask.get(x, y) { '#' } else { '.' })
                .collect()
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Dilation
// ---------------------------------------------------------------------------

#[test]
fn dilating_by_nothing_changes_nothing() {
    let mask = mask_of(&["..#..", ".....", "#...#"]);
    assert_eq!(dilate(&mask, 0), mask);
}

#[test]
fn a_single_pixel_grows_into_a_square() {
    // Separable dilation must give the same answer as one square window: a
    // cross would leave the corners of a glyph uncovered, and the tolerance
    // this exists to provide would not hold diagonally.
    let mask = mask_of(&[".....", ".....", "..#..", ".....", "....."]);
    assert_eq!(
        draw(&dilate(&mask, 1)),
        vec![".....", ".###.", ".###.", ".###.", "....."]
    );
}

#[test]
fn dilation_stops_at_the_edge_rather_than_wrapping() {
    let mask = mask_of(&["#..", "...", "..#"]);
    assert_eq!(draw(&dilate(&mask, 1)), vec!["##.", "###", ".##"]);
}

#[test]
fn a_radius_larger_than_the_image_fills_it() {
    let mask = mask_of(&["...", ".#.", "..."]);
    assert!(dilate(&mask, 50).bits.iter().all(|b| *b));
}

#[test]
fn dilating_nothing_gives_nothing() {
    let mask = mask_of(&["...", "...", "..."]);
    assert!(!dilate(&mask, 3).any());
}

// ---------------------------------------------------------------------------
// Regions
// ---------------------------------------------------------------------------

/// 25.4 dpi is one pixel per millimetre, which makes the numbers readable.
const ONE_PX_PER_MM: f64 = MM_PER_INCH;

#[test]
fn nothing_makes_no_regions() {
    let mask = mask_of(&["....", "....", "...."]);
    assert!(label_regions(&mask, ONE_PX_PER_MM, 2.0, 0.0).is_empty());
}

#[test]
fn separate_marks_make_separate_regions() {
    let mask = mask_of(&[
        "#........#",
        "#........#",
        "..........",
        "..........",
        "..........",
        "..........",
        "#........#",
    ]);
    // Grouped at 2 mm — two pixels here — so the four corners stay apart.
    let regions = label_regions(&mask, ONE_PX_PER_MM, 2.0, 0.0);
    assert_eq!(regions.len(), 4);
}

#[test]
fn letters_of_a_word_come_back_as_one_box() {
    // The point of grouping: five letters with a pixel between them is a word,
    // not five findings.
    let mask = mask_of(&["#.#.#.#.#", "#.#.#.#.#"]);
    let regions = label_regions(&mask, ONE_PX_PER_MM, 2.0, 0.0);

    assert_eq!(regions.len(), 1);
    assert_eq!(regions[0].px_bbox, (0, 0, 9, 2));
}

#[test]
fn a_box_is_measured_at_full_resolution_not_rounded_to_the_grid() {
    // One pixel of ink, grouped at 10 mm. The region is a tenth of the cell it
    // was found in, and must be reported as such.
    let mut rows = vec![".........."; 10];
    rows[4] = "....#.....";
    let mask = mask_of(&rows);
    let regions = label_regions(&mask, ONE_PX_PER_MM, 10.0, 0.0);

    assert_eq!(regions.len(), 1);
    assert_eq!(regions[0].px_bbox, (4, 4, 5, 5));
    assert!((regions[0].width_mm() - 1.0).abs() < 1e-9);
}

#[test]
fn specks_below_the_minimum_are_dropped() {
    let mask = mask_of(&["#........#", "#........."]);
    // One pixel is 1 mm², so a minimum of 1.5 keeps the pair and drops the one.
    let regions = label_regions(&mask, ONE_PX_PER_MM, 2.0, 1.5);
    assert_eq!(regions.len(), 1);
    assert_eq!(regions[0].px_bbox.0, 0);
}

#[test]
fn regions_come_back_in_reading_order() {
    let mask = mask_of(&[
        "..........",
        "..........",
        "..........",
        "..........",
        "........#.",
        "..........",
        "..........",
        "..........",
        "..........",
        "..........",
        "..#.......",
    ]);
    let regions = label_regions(&mask, ONE_PX_PER_MM, 1.0, 0.0);
    assert_eq!(regions.len(), 2);
    assert!(regions[0].y0_mm < regions[1].y0_mm);
}

#[test]
fn two_marks_on_one_line_read_left_to_right() {
    // Same row, so the only thing left to order them by is where they sit
    // across the page — which is the order someone reads them in.
    let mask = mask_of(&["........#.", "..........", ".#........"]);
    let regions = label_regions(&mask, ONE_PX_PER_MM, 1.0, 0.0);
    assert_eq!(regions.len(), 2);
    assert!(regions[0].y0_mm < regions[1].y0_mm);

    let same_row = mask_of(&[".#......#."]);
    let regions = label_regions(&same_row, ONE_PX_PER_MM, 1.0, 0.0);
    assert_eq!(regions.len(), 2);
    assert!(
        regions[0].x0_mm < regions[1].x0_mm,
        "{:?}",
        regions.iter().map(|r| r.px_bbox).collect::<Vec<_>>()
    );
}

#[test]
fn ink_area_counts_pixels_not_the_box() {
    // A hollow square: nine pixels of box, eight of ink.
    let mask = mask_of(&["###", "#.#", "###"]);
    let regions = label_regions(&mask, ONE_PX_PER_MM, 5.0, 0.0);

    assert_eq!(regions.len(), 1);
    assert!((regions[0].ink_mm2 - 8.0).abs() < 1e-9);
    assert!((regions[0].area_mm2() - 9.0).abs() < 1e-9);
}

#[test]
fn padding_grows_a_region_but_not_off_the_paper() {
    let region = Region {
        x0_mm: 1.0,
        y0_mm: 1.0,
        x1_mm: 209.0,
        y1_mm: 20.0,
        ink_mm2: 5.0,
        px_bbox: (0, 0, 1, 1),
    };
    let padded = region.padded(3.0, A4);

    assert_eq!(padded.x0_mm, 0.0, "clipped at the left edge");
    assert_eq!(padded.y0_mm, 0.0);
    assert_eq!(padded.x1_mm, 210.0, "clipped at the right edge");
    assert_eq!(padded.y1_mm, 23.0);
}

// ---------------------------------------------------------------------------
// Comparing two pages
// ---------------------------------------------------------------------------

fn compare(old: &[&str], new: &[&str], dpi: f64, options: &DiffOptions) -> PageDiff {
    let (old_gray, ow, oh) = page(old);
    let (new_gray, nw, nh) = page(new);
    let size = PageSize::new(px_to_mm(nw as f64, dpi), px_to_mm(nh as f64, dpi));
    diff_page(
        &old_gray,
        (ow, oh),
        &new_gray,
        (nw, nh),
        size,
        dpi,
        0,
        options,
    )
}

fn no_tolerance() -> DiffOptions {
    DiffOptions {
        tolerance_mm: 0.0,
        group_mm: 2.0,
        min_region_mm2: 0.0,
        ..Default::default()
    }
}

#[test]
fn two_identical_pages_differ_in_nothing() {
    let same = ["..#..", ".###.", "..#.."];
    let diff = compare(&same, &same, ONE_PX_PER_MM, &no_tolerance());

    assert_eq!(diff.added_px, 0);
    assert_eq!(diff.removed_px, 0);
    assert!(!diff.has_additions());
    assert!(diff.bounds_mm().is_none());
}

#[test]
fn new_ink_is_found_and_placed() {
    let old = ["....."; 5];
    let new = [".....", ".....", "..##.", "..##.", "....."];
    let diff = compare(&old, &new, ONE_PX_PER_MM, &no_tolerance());

    assert_eq!(diff.added_px, 4);
    assert_eq!(diff.removed_px, 0);
    assert_eq!(diff.added_regions.len(), 1);
    assert_eq!(diff.added_regions[0].px_bbox, (2, 2, 4, 4));
    assert_eq!(diff.bounds_mm(), Some((2.0, 2.0, 4.0, 4.0)));
}

#[test]
fn ink_that_disappeared_is_recorded_but_never_added() {
    // The reflow alarm. It is not printable and it is the most important
    // thing the comparison produces.
    let old = [".....", "..##.", "..##.", ".....", "....."];
    let new = ["....."; 5];
    let diff = compare(&old, &new, ONE_PX_PER_MM, &no_tolerance());

    assert_eq!(diff.added_px, 0);
    assert_eq!(diff.removed_px, 4);
    assert_eq!(diff.removed_regions.len(), 1);
}

#[test]
fn a_mark_that_shifted_a_hair_is_not_reprinted() {
    // Two renders of the same page can disagree by a fraction of a pixel. Left
    // alone that leaves a hairline ghost in the delta, printed on top of text
    // that is already on the sheet.
    let old = [".....", ".##..", ".##..", ".....", "....."];
    let new = [".....", "..##.", "..##.", ".....", "....."];

    let strict = compare(&old, &new, ONE_PX_PER_MM, &no_tolerance());
    assert!(strict.added_px > 0, "with no tolerance the jitter shows");

    // One millimetre of tolerance at one pixel per millimetre.
    let forgiving = compare(
        &old,
        &new,
        ONE_PX_PER_MM,
        &DiffOptions {
            tolerance_mm: 1.0,
            min_region_mm2: 0.0,
            ..Default::default()
        },
    );
    assert_eq!(forgiving.added_px, 0, "the jitter should be absorbed");
    assert_eq!(forgiving.removed_px, 0);
}

#[test]
fn a_real_addition_survives_the_tolerance() {
    // The tolerance must absorb jitter without hiding a whole new word.
    let old = ["..........", "..........", ".........."];
    let new = ["..........", "..####....", ".........."];
    let diff = compare(
        &old,
        &new,
        ONE_PX_PER_MM,
        &DiffOptions {
            tolerance_mm: 1.0,
            min_region_mm2: 0.0,
            ..Default::default()
        },
    );
    assert_eq!(diff.added_px, 4);
}

#[test]
fn anti_aliased_edges_count_as_ink() {
    // A glyph's edge pixels are grey, not black. Counting only black would
    // print the middle of every letter and none of its edges.
    let (mut gray, w, h) = page(&["...", "...", "..."]);
    gray[4] = 180; // pale grey, inside the default threshold
    let mut white = vec![255u8; w * h];
    let size = PageSize::new(px_to_mm(w as f64, 25.4), px_to_mm(h as f64, 25.4));

    let diff = diff_page(
        &white,
        (w, h),
        &gray,
        (w, h),
        size,
        25.4,
        0,
        &no_tolerance(),
    );
    assert_eq!(diff.added_px, 1);

    white[4] = 201; // just outside it
    let diff = diff_page(
        &vec![255u8; w * h],
        (w, h),
        &white,
        (w, h),
        size,
        25.4,
        0,
        &no_tolerance(),
    );
    assert_eq!(diff.added_px, 0);
}

#[test]
fn pages_that_differ_by_a_pixel_in_size_still_compare() {
    // Two renders rounding differently is not worth refusing over.
    let old = ["....", "....", "...."];
    let new = ["...", "...", "...", "..."];
    let diff = compare(&old, &new, ONE_PX_PER_MM, &no_tolerance());
    assert_eq!(diff.added_px, 0);
}

#[test]
fn ink_area_is_reported_in_real_units() {
    let old = ["....."; 5];
    let new = [".....", ".....", "..##.", "..##.", "....."];
    // 50.8 dpi is two pixels per millimetre, so four pixels is one square mm.
    let diff = compare(&old, &new, 50.8, &no_tolerance());

    assert_eq!(diff.added_px, 4);
    assert!((diff.added_ink_mm2() - 1.0).abs() < 1e-9);
}

#[test]
fn releasing_a_diff_keeps_every_measurement() {
    let old = ["....."; 5];
    let new = [".....", ".....", "..##.", "..##.", "....."];
    let mut diff = compare(&old, &new, ONE_PX_PER_MM, &no_tolerance());

    let (added, regions, bounds) = (diff.added_px, diff.added_regions.len(), diff.bounds_mm());
    diff.release();

    assert_eq!(diff.added_px, added);
    assert_eq!(diff.added_regions.len(), regions);
    assert_eq!(diff.bounds_mm(), bounds);
    assert!(diff.added.bits.is_empty(), "the pixels should be gone");
}

#[test]
fn a_blank_diff_is_a_page_with_nothing_on_it() {
    let diff = PageDiff::blank(A4, 300.0, 2);

    assert_eq!(diff.index, 2);
    assert_eq!(diff.added_px, 0);
    assert!(!diff.has_additions());
    assert_eq!(diff.added.width, A4.px_size(300.0).0 as usize);
}
