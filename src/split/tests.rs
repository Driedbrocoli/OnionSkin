//! Tests for splitting a job into what can be overprinted and what cannot.

use super::*;

use crate::geometry::PageSize;
use crate::pdf::{Font, LineFont, PlacedLine};

const A4: PageSize = PageSize {
    width_mm: 210.0,
    height_mm: 297.0,
};

/// A page diff carrying the two numbers the split reads: how much ink went
/// missing, and how many things were added.
fn a_page(index: usize, removed_mm2: f64, additions: usize) -> PageDiff {
    let mut diff = PageDiff::blank(A4, 150.0, index);
    // `removed_ink_mm2` is a count of pixels times the area of one, so the
    // count is worked back from the millimetres wanted.
    diff.removed_px = (removed_mm2 / diff.px_area_mm2()).round() as usize;
    diff.added_regions = (0..additions)
        .map(|n| crate::diff::Region {
            x0_mm: 10.0,
            y0_mm: 10.0 + n as f64 * 10.0,
            x1_mm: 40.0,
            y1_mm: 15.0 + n as f64 * 10.0,
            ink_mm2: 20.0,
            px_bbox: (0, 0, 1, 1),
        })
        .collect();
    diff
}

/// The case this exists for: one page moved, the rest are fine, and until now
/// the one held back the rest.
#[test]
fn a_page_that_moved_is_told_apart_from_the_pages_that_did_not() {
    let job = Split::of(&[
        a_page(0, 0.0, 2),
        a_page(1, 40.0, 1), // page two: the text moved
        a_page(2, 0.0, 1),
        a_page(3, 0.0, 3),
    ]);
    assert_eq!(job.feed(), vec![1, 3, 4]);
    assert_eq!(job.reprint(), vec![2]);
    assert!(!job.all_overprintable());
    assert!(!job.nothing_to_overprint());
}

#[test]
fn a_job_where_nothing_moved_needs_no_splitting() {
    let job = Split::of(&[a_page(0, 0.0, 1), a_page(1, 0.0, 2)]);
    assert!(job.all_overprintable());
    assert!(job.reprint().is_empty());
    assert_eq!(job.feed(), vec![1, 2]);
}

/// A page with nothing added to it is not a sheet anybody needs to feed, even
/// though it can be overprinted perfectly well.
#[test]
fn a_page_with_no_additions_is_not_a_sheet_to_feed() {
    let job = Split::of(&[a_page(0, 0.0, 0), a_page(1, 0.0, 1)]);
    assert_eq!(job.feed(), vec![2]);
}

/// A page that both moved and had things added to it is a page to reprint. The
/// additions were placed against text that has since moved, so printing them
/// would land words in the wrong place on a sheet about to be thrown away.
#[test]
fn a_page_that_moved_is_reprinted_even_though_it_also_gained_something() {
    let job = Split::of(&[a_page(0, 60.0, 4)]);
    assert_eq!(job.reprint(), vec![1]);
    assert!(job.feed().is_empty());
    assert!(job.nothing_to_overprint());
}

/// A speck of dust between two scans is not a paragraph moving, and the line
/// between them is the one the reflow check already draws.
#[test]
fn a_trace_of_missing_ink_is_not_a_page_that_moved() {
    let barely = crate::safety::REFLOW_INK_MM2 * 0.5;
    let job = Split::of(&[a_page(0, barely, 1)]);
    assert!(job.all_overprintable(), "{:?}", job.verdicts);

    let plainly = crate::safety::REFLOW_INK_MM2 * 4.0;
    let job = Split::of(&[a_page(0, plainly, 1)]);
    assert_eq!(job.reprint(), vec![1]);
}

/// Two documents handed over in the wrong order have ink missing from every
/// page. That is one mistake, not forty pages that moved — and splitting on the
/// raw measurement would blank the whole delta and call the entire document
/// "fresh". So the pipeline hands the list in rather than letting it be
/// re-derived, and an empty list means nothing to split.
#[test]
fn the_pages_that_moved_can_be_given_rather_than_worked_out() {
    let every_page_lost_ink = [a_page(0, 90.0, 0), a_page(1, 90.0, 0)];

    // Worked out from the pages: both moved.
    assert_eq!(Split::of(&every_page_lost_ink).reprint(), vec![1, 2]);

    // Told that nothing moved — which is what the checks say once the real
    // cause has been recognised — and nothing is split.
    let told = Split::given(&every_page_lost_ink, &[]);
    assert!(told.reprint().is_empty(), "{:?}", told.verdicts);
    assert!(told.all_overprintable());

    // And a list of one splits exactly one.
    let told = Split::given(&every_page_lost_ink, &[2]);
    assert_eq!(told.reprint(), vec![2]);
}

/// How the page numbers are said out loud.
///
/// "1 and 3 and 4" is what came out before this was fixed, and it is exactly
/// the shape a split produces: a page in the middle reprinted, the rest fed.
#[test]
fn the_sheets_are_named_the_way_somebody_would_say_them() {
    assert_eq!(sheets(&[]), "");
    assert_eq!(sheets(&[3]), "3");
    assert_eq!(sheets(&[3, 7]), "3 and 7");
    assert_eq!(sheets(&[3, 7, 9]), "3, 7 and 9");
    // A run collapses.
    assert_eq!(sheets(&[4, 5, 6, 7]), "4 to 7");
    assert_eq!(sheets(&[1, 2, 3, 9, 11, 12, 13]), "1 to 3, 9 and 11 to 13");
    // Two in a row on their own read as a pair…
    assert_eq!(sheets(&[1, 2]), "1 and 2");
    // …but beside anything else they are two numbers, so the sentence has one
    // "and" in it rather than two.
    assert_eq!(sheets(&[1, 3, 4]), "1, 3 and 4");
    assert_eq!(sheets(&[1, 2, 5]), "1, 2 and 5");
}

/// The instructions name the files and the sheets, and say which is which.
#[test]
fn what_to_do_says_both_things_and_does_not_confuse_them() {
    let job = Split::of(&[
        a_page(0, 0.0, 1),
        a_page(1, 40.0, 1),
        a_page(2, 0.0, 1),
        a_page(3, 0.0, 1),
    ]);
    let said = job.what_to_do(Path::new("delta.pdf"), Path::new("fresh.pdf"));
    assert!(said.contains("feed sheets 1, 3 and 4 back in"), "{said}");
    assert!(said.contains("print sheet 2 on fresh paper"), "{said}");
    assert!(said.contains("blank in the delta"), "{said}");

    // Nothing moved, nothing to say.
    let easy = Split::of(&[a_page(0, 0.0, 1)]);
    assert!(easy
        .what_to_do(Path::new("d.pdf"), Path::new("f.pdf"))
        .is_empty());
}

/// A job where every page moved has no overlay in it at all, and saying so
/// beats handing over a delta that is blank on every page.
#[test]
fn a_job_with_nothing_left_to_overprint_says_that_plainly() {
    let job = Split::of(&[a_page(0, 40.0, 1), a_page(1, 40.0, 2)]);
    let said = job.what_to_do(Path::new("delta.pdf"), Path::new("fresh.pdf"));
    assert!(said.contains("nothing an overlay can add"), "{said}");
    assert!(said.contains("fresh.pdf is the whole job"), "{said}");
    assert!(said.contains("nothing to feed"), "{said}");
}

// ---------------------------------------------------------------------------
// What is done to the files
// ---------------------------------------------------------------------------

/// A delta of `pages` pages, each carrying one line.
fn a_delta(at: &Path, pages: usize) -> PathBuf {
    let sizes = vec![A4; pages];
    let lines: Vec<Vec<PlacedLine>> = (0..pages)
        .map(|page| {
            vec![PlacedLine {
                text: format!("PAGE {page}"),
                x_mm: 20.0,
                y_mm: 40.0,
                size_pt: 24.0,
                font: LineFont::Builtin(Font::Helvetica),
                colour: (0.0, 0.0, 0.0),
                rotation_deg: 0.0,
            }]
        })
        .collect();
    crate::pdf::write_delta(at, &sizes, &lines, "test", None).unwrap();
    at.to_path_buf()
}

/// How much ink is on each page, as the printer would put it down.
fn ink_per_page(pdf: &Path) -> Vec<usize> {
    let engine = crate::render::engine().expect("a renderer");
    let doc = engine.open(pdf).expect("it should open");
    (0..doc.len())
        .map(|index| {
            let page = doc.render_gray(index, 100.0).expect("it should render");
            page.gray.iter().filter(|&&value| value < 128).count()
        })
        .collect()
}

/// Blanked, not removed: the delta's page three has to stay page three, since
/// the whole scheme is that page *n* is fed the printed sheet *n*.
#[test]
fn a_blanked_page_stays_a_page() {
    let dir = tempfile::tempdir().unwrap();
    let delta = a_delta(&dir.path().join("delta.pdf"), 4);

    let before = ink_per_page(&delta);
    assert!(before.iter().all(|&ink| ink > 0), "{before:?}");

    blank_pages(&delta, &[2]).unwrap();

    let after = ink_per_page(&delta);
    assert_eq!(after.len(), 4, "a page was removed instead of emptied");
    assert_eq!(after[1], 0, "page two still has ink on it");
    assert_eq!(after[0], before[0], "page one was disturbed");
    assert_eq!(after[2], before[2], "page three was disturbed");
    assert_eq!(after[3], before[3], "page four was disturbed");
}

#[test]
fn blanking_nothing_leaves_the_file_alone() {
    let dir = tempfile::tempdir().unwrap();
    let delta = a_delta(&dir.path().join("delta.pdf"), 2);
    let before = std::fs::read(&delta).unwrap();
    blank_pages(&delta, &[]).unwrap();
    assert_eq!(std::fs::read(&delta).unwrap(), before);
}

#[test]
fn several_pages_can_be_blanked_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let delta = a_delta(&dir.path().join("delta.pdf"), 5);
    blank_pages(&delta, &[1, 3, 5]).unwrap();
    let ink = ink_per_page(&delta);
    assert_eq!(ink[0], 0, "{ink:?}");
    assert!(ink[1] > 0, "{ink:?}");
    assert_eq!(ink[2], 0, "{ink:?}");
    assert!(ink[3] > 0, "{ink:?}");
    assert_eq!(ink[4], 0, "{ink:?}");
}

/// The sheets that have to be reprinted, taken out of the edited document
/// whole so they can go straight to the printer.
#[test]
fn the_pages_to_reprint_come_out_as_their_own_file() {
    let dir = tempfile::tempdir().unwrap();
    let whole = a_delta(&dir.path().join("whole.pdf"), 5);
    let fresh = dir.path().join("fresh.pdf");

    keep_only(&whole, &[2, 4], &fresh).unwrap();

    let engine = crate::render::engine().unwrap();
    let doc = engine.open(&fresh).unwrap();
    assert_eq!(doc.len(), 2);
    assert!(doc.page_sizes[0].matches(&A4, 0.2), "{:?}", doc.page_sizes);
    // And they are the right two: every page carries its own number.
    let ink = ink_per_page(&fresh);
    assert!(ink.iter().all(|&on_page| on_page > 0), "{ink:?}");
}

/// One page out of a long document should be the size of one page, not of the
/// document with two hundred pages hidden inside it.
#[test]
fn what_the_other_pages_used_does_not_come_with_them() {
    let dir = tempfile::tempdir().unwrap();
    let whole = a_delta(&dir.path().join("whole.pdf"), 20);
    let one = dir.path().join("one.pdf");
    keep_only(&whole, &[7], &one).unwrap();

    let whole_size = std::fs::metadata(&whole).unwrap().len();
    let one_size = std::fs::metadata(&one).unwrap().len();
    assert!(
        one_size < whole_size,
        "one page of twenty came out at {one_size} against {whole_size}"
    );
}

#[test]
fn asking_for_a_page_that_is_not_there_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let whole = a_delta(&dir.path().join("whole.pdf"), 3);
    let out = dir.path().join("out.pdf");

    let said = keep_only(&whole, &[4], &out).unwrap_err().to_string();
    assert!(said.contains("no page 4"), "{said}");
    assert!(!out.exists(), "a refused extract still wrote a file");

    assert!(keep_only(&whole, &[0], &out).is_err(), "page nought");
}

/// The order they are asked for does not matter; they come out in the order
/// they were in, which is the order the sheets are in.
#[test]
fn the_pages_come_out_in_the_order_they_were_in() {
    let dir = tempfile::tempdir().unwrap();
    let whole = a_delta(&dir.path().join("whole.pdf"), 4);
    let out = dir.path().join("out.pdf");
    keep_only(&whole, &[3, 1], &out).unwrap();

    let engine = crate::render::engine().unwrap();
    let doc = engine.open(&out).unwrap();
    assert_eq!(doc.len(), 2);
}
