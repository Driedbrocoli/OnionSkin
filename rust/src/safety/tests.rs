//! Tests for the checks that run before paper is committed.

use super::*;
use crate::diff::{Mask, Region};

const A4: PageSize = PageSize {
    width_mm: 210.0,
    height_mm: 297.0,
};
const A5: PageSize = PageSize {
    width_mm: 148.0,
    height_mm: 210.0,
};

fn region(x0: f64, y0: f64, x1: f64, y1: f64) -> Region {
    Region {
        x0_mm: x0,
        y0_mm: y0,
        x1_mm: x1,
        y1_mm: y1,
        ink_mm2: (x1 - x0) * (y1 - y0) * 0.3,
        px_bbox: (0, 0, 1, 1),
    }
}

/// A diff with the given added and removed ink, and nothing else real.
fn diff(index: usize, added: Vec<Region>, removed: Vec<Region>) -> PageDiff {
    let dpi = 300.0;
    let px_mm2 = (crate::geometry::MM_PER_INCH / dpi).powi(2);
    let sum = |regions: &[Region]| -> usize {
        (regions.iter().map(|r| r.ink_mm2).sum::<f64>() / px_mm2).round() as usize
    };
    PageDiff {
        index,
        size: A4,
        dpi,
        added_px: sum(&added),
        removed_px: sum(&removed),
        added_regions: added,
        removed_regions: removed,
        added: Mask::blank(0, 0),
        removed: Mask::blank(0, 0),
    }
}

fn codes(checks: &[Check]) -> Vec<&str> {
    checks.iter().map(|c| c.code).collect()
}

// ---------------------------------------------------------------------------
// Reflow — the one that matters
// ---------------------------------------------------------------------------

#[test]
fn ink_that_vanished_blocks_the_job() {
    // The whole reason the program is careful. Toner does not come off paper,
    // so if anything moved, an overlay cannot put it right.
    let page = diff(0, vec![], vec![region(20.0, 96.0, 80.0, 99.0)]);
    let checks = check_reflow(&page);

    assert_eq!(checks.len(), 1);
    assert_eq!(checks[0].severity, Severity::Blocker);
    assert_eq!(checks[0].page, Some(1));

    let message = checks[0].format();
    assert!(message.contains("BLOCKER [page 1]"), "{message}");
    assert!(message.contains("moved or was deleted"), "{message}");
    // It must say where to look, and what to do instead.
    assert!(message.contains("96 mm down the page"), "{message}");
    assert!(message.contains("print this page fresh"), "{message}");
    assert!(message.contains("Fixed position on page"), "{message}");
}

#[test]
fn a_stray_pixel_is_not_a_reflow() {
    // Two renders of the same page disagree slightly at glyph edges. Calling
    // that a reflow would block every job.
    let speck = Region {
        ink_mm2: 0.4,
        ..region(20.0, 96.0, 20.5, 96.5)
    };
    assert!(check_reflow(&diff(0, vec![], vec![speck])).is_empty());
}

#[test]
fn the_threshold_is_about_two_characters_of_ink() {
    let under = Region {
        ink_mm2: REFLOW_INK_MM2 - 0.01,
        ..region(20.0, 96.0, 21.0, 97.0)
    };
    let over = Region {
        ink_mm2: REFLOW_INK_MM2 + 0.01,
        ..region(20.0, 96.0, 21.0, 97.0)
    };
    assert!(check_reflow(&diff(0, vec![], vec![under])).is_empty());
    assert_eq!(check_reflow(&diff(0, vec![], vec![over])).len(), 1);
}

// ---------------------------------------------------------------------------
// Documents that do not match each other
// ---------------------------------------------------------------------------

#[test]
fn two_matching_documents_raise_nothing() {
    assert!(check_documents(&[A4, A4], &[A4, A4]).is_empty());
}

#[test]
fn a_page_that_vanished_blocks() {
    let checks = check_documents(&[A4, A4, A4], &[A4, A4]);
    assert_eq!(codes(&checks), vec!["pages_removed"]);
    assert_eq!(checks[0].severity, Severity::Blocker);
    assert!(
        checks[0].detail.contains("3 → 2 pages"),
        "{}",
        checks[0].detail
    );
}

#[test]
fn a_page_that_appeared_only_warns() {
    // There is no printed sheet for it, but nothing about the existing sheets
    // has gone wrong — so this is advice, not a refusal.
    let checks = check_documents(&[A4], &[A4, A4]);
    assert_eq!(codes(&checks), vec!["pages_added"]);
    assert_eq!(checks[0].severity, Severity::Warning);
    assert!(checks[0].detail.contains("blank paper"));
}

#[test]
fn a_document_that_changed_paper_size_blocks() {
    let checks = check_documents(&[A4, A4], &[A4, A5]);
    assert_eq!(codes(&checks), vec!["page_size_mismatch"]);
    assert_eq!(checks[0].page, Some(2));
    assert!(checks[0].detail.contains("A4"), "{}", checks[0].detail);
    assert!(checks[0].detail.contains("A5"), "{}", checks[0].detail);
}

#[test]
fn a_fraction_of_a_millimetre_is_not_a_size_change() {
    let nearly = PageSize::new(210.2, 297.1);
    assert!(check_documents(&[A4], &[nearly]).is_empty());
}

// ---------------------------------------------------------------------------
// Margins
// ---------------------------------------------------------------------------

#[test]
fn an_addition_in_the_middle_of_the_page_is_fine() {
    let page = diff(0, vec![region(50.0, 100.0, 90.0, 106.0)], vec![]);
    assert!(check_margins(&page, DEFAULT_MARGIN_MM).is_empty());
}

#[test]
fn an_addition_in_the_dead_border_is_warned_about() {
    let page = diff(0, vec![region(2.0, 100.0, 40.0, 106.0)], vec![]);
    let checks = check_margins(&page, DEFAULT_MARGIN_MM);

    assert_eq!(checks.len(), 1);
    assert_eq!(checks[0].severity, Severity::Warning);
    assert!(checks[0].message.contains("5 mm"), "{}", checks[0].message);
    assert!(
        checks[0].detail.contains("within 2.0 mm"),
        "{}",
        checks[0].detail
    );
}

#[test]
fn each_edge_of_the_page_is_checked() {
    for region in [
        region(1.0, 100.0, 40.0, 106.0),    // left
        region(50.0, 1.0, 90.0, 4.0),       // top
        region(206.0, 100.0, 209.0, 106.0), // right
        region(50.0, 294.0, 90.0, 296.0),   // bottom
    ] {
        let page = diff(0, vec![region], vec![]);
        assert_eq!(
            check_margins(&page, DEFAULT_MARGIN_MM).len(),
            1,
            "{region:?}"
        );
    }
}

#[test]
fn a_margin_of_zero_turns_the_check_off() {
    let page = diff(0, vec![region(0.0, 0.0, 5.0, 5.0)], vec![]);
    assert!(check_margins(&page, 0.0).is_empty());
}

#[test]
fn the_worst_offender_is_the_one_reported() {
    let page = diff(
        0,
        vec![
            region(4.0, 100.0, 40.0, 106.0),
            region(0.5, 120.0, 40.0, 126.0),
        ],
        vec![],
    );
    let checks = check_margins(&page, DEFAULT_MARGIN_MM);
    assert!(checks[0].message.contains("2 addition"));
    assert!(
        checks[0].detail.contains("within 0.5 mm"),
        "{}",
        checks[0].detail
    );
}

// ---------------------------------------------------------------------------
// Coverage, and an empty delta
// ---------------------------------------------------------------------------

#[test]
fn a_small_delta_says_nothing() {
    let page = diff(0, vec![region(50.0, 100.0, 90.0, 106.0)], vec![]);
    assert!(check_coverage(&page).is_empty());
}

#[test]
fn a_delta_covering_much_of_the_page_is_worth_saying() {
    // Usually the sign of a reflow that the ink test did not catch.
    let big = Region {
        ink_mm2: A4.width_mm * A4.height_mm * 0.2,
        ..region(20.0, 20.0, 190.0, 280.0)
    };
    let checks = check_coverage(&diff(0, vec![big], vec![]));

    assert_eq!(codes(&checks), vec!["large_delta"]);
    assert!(checks[0].message.contains("20%"), "{}", checks[0].message);
}

#[test]
fn two_identical_documents_block_rather_than_print_a_blank_sheet() {
    let checks = check_empty(&[diff(0, vec![], vec![])]);
    assert_eq!(codes(&checks), vec!["empty_delta"]);
    assert_eq!(checks[0].severity, Severity::Blocker);
    // The commonest cause is the arguments the wrong way round.
    assert!(checks[0].detail.contains("edited file second"));
}

#[test]
fn one_page_with_additions_is_enough_to_not_be_empty() {
    let pages = [
        diff(0, vec![], vec![]),
        diff(1, vec![region(50.0, 100.0, 90.0, 106.0)], vec![]),
    ];
    assert!(check_empty(&pages).is_empty());
}

// ---------------------------------------------------------------------------
// Calibration
// ---------------------------------------------------------------------------

#[test]
fn an_uncalibrated_run_says_what_to_expect() {
    let checks = check_calibration(false, None);
    assert_eq!(codes(&checks), vec!["uncalibrated"]);
    assert_eq!(checks[0].severity, Severity::Note);
    assert!(checks[0].message.contains("±2 mm"));
    assert!(checks[0].detail.contains("calibrate target"));
}

#[test]
fn a_calibrated_run_names_the_profile() {
    let checks = check_calibration(true, Some("office"));
    assert_eq!(codes(&checks), vec!["calibrated"]);
    assert!(checks[0].message.contains("'office'"));
}

#[test]
fn a_profile_used_on_its_own_paper_says_nothing() {
    let error = Similarity {
        dx_mm: 0.4,
        dy_mm: -0.2,
        rotation_deg: 0.05,
        scale: 1.0002,
    };
    assert!(check_profile_page("office", A4, error, A4).is_empty());
}

#[test]
fn a_profile_used_on_another_paper_size_is_warned_about() {
    let error = Similarity {
        dx_mm: 0.4,
        dy_mm: -0.2,
        rotation_deg: 0.05,
        scale: 1.0002,
    };
    let checks = check_profile_page("office", A4, error, A5);

    assert_eq!(codes(&checks), vec!["profile_page_mismatch"]);
    assert!(checks[0].message.contains("A4"), "{}", checks[0].message);
    assert!(checks[0].message.contains("A5"), "{}", checks[0].message);
    assert!(checks[0].detail.contains("centre of the page"));
}

#[test]
fn a_profile_that_is_only_a_shift_carries_to_any_paper() {
    // The paper path pushes every sheet the same way, so a pure offset
    // transfers cleanly and there is nothing to warn about.
    let shift_only = Similarity {
        dx_mm: 0.4,
        dy_mm: -0.2,
        rotation_deg: 0.0,
        scale: 1.0,
    };
    assert!(check_profile_page("office", A4, shift_only, A5).is_empty());
}

// ---------------------------------------------------------------------------
// Everything together
// ---------------------------------------------------------------------------

#[test]
fn checks_come_back_worst_first() {
    let pages = [
        diff(0, vec![region(1.0, 100.0, 40.0, 106.0)], vec![]),
        diff(1, vec![], vec![region(20.0, 96.0, 80.0, 99.0)]),
    ];
    let checks = check_all(&pages, &[A4, A4], &[A4, A4], DEFAULT_MARGIN_MM, false, None);

    assert_eq!(
        checks[0].severity,
        Severity::Blocker,
        "{:?}",
        codes(&checks)
    );
    assert!(checks
        .windows(2)
        .all(|pair| pair[0].severity <= pair[1].severity));
}

#[test]
fn a_clean_job_has_no_blockers() {
    let pages = [diff(0, vec![region(50.0, 100.0, 90.0, 106.0)], vec![])];
    let checks = check_all(
        &pages,
        &[A4],
        &[A4],
        DEFAULT_MARGIN_MM,
        true,
        Some("office"),
    );

    assert!(!has_blockers(&checks));
    assert_eq!(codes(&checks), vec!["calibrated"]);
}

#[test]
fn a_reflowed_job_has_blockers() {
    let pages = [diff(0, vec![], vec![region(20.0, 96.0, 80.0, 99.0)])];
    let checks = check_all(&pages, &[A4], &[A4], DEFAULT_MARGIN_MM, false, None);
    assert!(has_blockers(&checks));
}

#[test]
fn a_check_with_no_detail_still_formats_cleanly() {
    let check = Check::new(Severity::Note, "x", "Something happened.".into());
    assert_eq!(check.format(), "note: Something happened.");
}

#[test]
fn margins_are_written_the_way_a_person_writes_them() {
    assert_eq!(trim_number(5.0), "5");
    assert_eq!(trim_number(4.5), "4.5");
    assert_eq!(trim_number(0.0), "0");
}
