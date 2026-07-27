use super::*;
use crate::calibrate::{Asked, A4};

/// A sheet at a known resolution, all paper, with dark boxes on it.
///
/// Boxes are given in millimetres — x0, y0, x1, y1 — because that is how
/// everything else in this module speaks, and converting in the test is how a
/// test comes to disagree with the code it checks.
fn sheet(dpi: f64, ink: &[(f64, f64, f64, f64)]) -> (Vec<u8>, usize) {
    let (w, h) = A4.px_size(dpi);
    let (w, h) = (w as usize, h as usize);
    let mut gray = vec![255u8; w * h];
    let px = |mm: f64| (mm * dpi / 25.4).round() as usize;
    for (x0, y0, x1, y1) in ink {
        for y in px(*y0)..px(*y1).min(h) {
            for x in px(*x0)..px(*x1).min(w) {
                gray[y * w + x] = 0;
            }
        }
    }
    (gray, w)
}

/// An addition the delta would put in the given box.
fn asked(box_mm: (f64, f64, f64, f64)) -> Asked {
    let (x0, y0, x1, y1) = box_mm;
    Asked {
        centre_mm: ((x0 + x1) / 2.0, (y0 + y1) / 2.0),
        bounds_mm: box_mm,
        ink_mm2: (x1 - x0) * (y1 - y0),
    }
}

const DPI: f64 = 100.0;

/// The right sheet: the form's own printing is elsewhere, and the addition
/// lands on clear paper. That is what an overlay is, and it has to read as
/// plainly good rather than as an absence of complaints.
#[test]
fn on_the_right_sheet_every_addition_lands_on_clear_paper() {
    // A label at the top left, and the answer going in beside it.
    let (gray, width) = sheet(DPI, &[(20.0, 38.0, 45.0, 43.0)]);
    let fit = against(
        &[asked((60.0, 38.0, 100.0, 43.0))],
        &gray,
        width,
        DPI,
        A4,
        A4,
        128,
    );

    assert!(fit.belongs(), "{}", fit.describe());
    assert!(fit.collisions().is_empty());
    assert_eq!(fit.landings.len(), 1);
    assert!(
        fit.landings[0].under_mm2 < ON_TOP_MM2,
        "found {} mm² under an addition on clear paper",
        fit.landings[0].under_mm2
    );
    assert!(fit.describe().contains("clear paper"), "{}", fit.describe());
}

/// The wrong sheet: whatever that form says is where this delta wants to
/// write, so the addition lands on top of it. This is the whole point.
#[test]
fn on_the_wrong_sheet_an_addition_lands_on_top_of_something() {
    // A block of printing exactly where the delta wants to put its words.
    let (gray, width) = sheet(DPI, &[(60.0, 36.0, 105.0, 45.0)]);
    let fit = against(
        &[asked((60.0, 38.0, 100.0, 43.0))],
        &gray,
        width,
        DPI,
        A4,
        A4,
        128,
    );

    assert!(!fit.belongs(), "the wrong sheet was accepted");
    assert_eq!(fit.collisions().len(), 1);
    assert!(fit.landings[0].lands_on_something());
    assert_eq!(
        fit.landings[0].clearance_mm, 0.0,
        "an addition sitting on top of something has no clearance"
    );

    let said = fit.describe();
    assert!(
        said.contains("on top of something already printed"),
        "{said}"
    );
    assert!(
        said.contains("wrong sheet"),
        "the message does not say what this usually means: {said}"
    );
}

/// Paper of a different size is the other way the wrong sheet shows up — a
/// Letter form fed for an A4 delta, which no amount of clear paper excuses.
#[test]
fn paper_of_a_different_size_is_caught_on_its_own() {
    let (gray, width) = sheet(DPI, &[]);
    let letter = PageSize {
        width_mm: 215.9,
        height_mm: 279.4,
    };
    let fit = against(
        &[asked((60.0, 38.0, 100.0, 43.0))],
        &gray,
        width,
        DPI,
        A4,
        letter,
        128,
    );

    assert!(!fit.paper_matches());
    assert!(!fit.belongs(), "different paper was accepted");
    assert!(
        fit.collisions().is_empty(),
        "nothing is printed on this sheet, so nothing can be landed on"
    );
    assert!(
        fit.describe().contains("not the same"),
        "{}",
        fit.describe()
    );
}

/// A scan is measured rather than declared, so a millimetre either way is the
/// measurement and not a different sheet of paper. Refusing over that would
/// refuse every real scan there is.
#[test]
fn a_millimetre_of_measurement_is_not_a_different_sheet() {
    let (gray, width) = sheet(DPI, &[]);
    let measured = PageSize {
        width_mm: A4.width_mm + 1.2,
        height_mm: A4.height_mm - 0.9,
    };
    let fit = against(&[], &gray, width, DPI, A4, measured, 128);
    assert!(
        fit.paper_matches(),
        "a millimetre was called a different size"
    );
}

/// A speck of scanner noise under an addition is not a collision. A check that
/// refuses a job over one dark pixel is a check people learn to skip, and then
/// it is not there for the sheet that really is wrong.
#[test]
fn a_speck_of_noise_is_not_a_collision() {
    // A tenth of a millimetre square: far below two characters of ink.
    let (gray, width) = sheet(DPI, &[(70.0, 40.0, 70.3, 40.3)]);
    let fit = against(
        &[asked((60.0, 38.0, 100.0, 43.0))],
        &gray,
        width,
        DPI,
        A4,
        A4,
        128,
    );
    assert!(
        fit.collisions().is_empty(),
        "a speck was called a collision: {} mm² under it",
        fit.landings[0].under_mm2
    );
    assert!(fit.belongs());
}

/// How much room an addition has is worth saying, because an addition landing
/// a millimetre from a printed line is one an uncalibrated printer can push
/// onto it.
#[test]
fn the_gap_to_the_nearest_ink_is_measured() {
    // A rule 10 mm below where the words go.
    let (gray, width) = sheet(DPI, &[(20.0, 53.0, 180.0, 54.0)]);
    let fit = against(
        &[asked((60.0, 38.0, 100.0, 43.0))],
        &gray,
        width,
        DPI,
        A4,
        A4,
        128,
    );

    let gap = fit.tightest_mm().expect("something was measured");
    assert!(
        (9.0..=11.5).contains(&gap),
        "the rule is 10 mm below the words and the gap came out {gap} mm"
    );
}

/// Nothing anywhere near is a real answer, and it must not be reported as a
/// gap of nought — which would read as a collision.
#[test]
fn an_addition_in_the_middle_of_an_empty_page_has_room() {
    let (gray, width) = sheet(DPI, &[]);
    let fit = against(
        &[asked((90.0, 140.0, 120.0, 148.0))],
        &gray,
        width,
        DPI,
        A4,
        A4,
        128,
    );
    let gap = fit.tightest_mm().expect("something was measured");
    assert!(gap > 20.0, "an empty page reported {gap} mm of clearance");
    assert!(fit.belongs());
}

/// Several additions, some clear and some not: every one is reported, and the
/// verdict turns on whether any of them is in trouble.
#[test]
fn every_addition_is_looked_at_not_only_the_first() {
    let (gray, width) = sheet(DPI, &[(60.0, 96.0, 105.0, 105.0)]);
    let fit = against(
        &[
            asked((60.0, 38.0, 100.0, 43.0)),
            asked((60.0, 98.0, 100.0, 103.0)),
            asked((60.0, 158.0, 100.0, 163.0)),
        ],
        &gray,
        width,
        DPI,
        A4,
        A4,
        128,
    );

    assert_eq!(fit.landings.len(), 3, "not every addition was looked at");
    assert_eq!(fit.collisions().len(), 1, "{}", fit.describe());
    assert!(!fit.belongs());
    // And the one in trouble is named, so somebody can go and look at it.
    assert!(fit.describe().contains("98"), "{}", fit.describe());
}

/// A delta with nothing on it has nothing to check, and must not claim to have
/// checked anything — nor divide by the nought additions it has.
#[test]
fn a_delta_with_no_additions_says_nothing_was_measured() {
    let (gray, width) = sheet(DPI, &[(20.0, 38.0, 45.0, 43.0)]);
    let fit = against(&[], &gray, width, DPI, A4, A4, 128);
    assert!(fit.landings.is_empty());
    assert!(fit.tightest_mm().is_none(), "nothing was there to measure");
    assert!(fit.belongs(), "nothing to collide, and the paper matches");
}

/// A box running off the paper must be clipped rather than read past the end
/// of the image — a delta made for a longer page than the scan is exactly when
/// this is asked, and a panic here is a panic at the printer.
#[test]
fn an_addition_running_off_the_sheet_does_not_read_past_the_page() {
    let (gray, width) = sheet(DPI, &[]);
    let fit = against(
        &[asked((190.0, 280.0, 260.0, 330.0))],
        &gray,
        width,
        DPI,
        A4,
        A4,
        128,
    );
    assert_eq!(fit.landings.len(), 1);
    assert!(fit.landings[0].under_mm2.is_finite());
}

/// Counted the way a person counts. "1 of the 1 addition would land" is what
/// arithmetic produces and nobody says out loud, and this message is read by
/// somebody standing at a printer holding a sheet.
#[test]
fn one_addition_is_written_about_in_the_singular() {
    let (gray, width) = sheet(DPI, &[(60.0, 36.0, 105.0, 45.0)]);
    let wrong = against(
        &[asked((60.0, 38.0, 100.0, 43.0))],
        &gray,
        width,
        DPI,
        A4,
        A4,
        128,
    );
    let said = wrong.describe();
    assert!(
        said.contains("The addition would land"),
        "one addition was counted at: {said}"
    );
    assert!(!said.contains("1 of the 1"), "{said}");

    let (clear, width) = sheet(DPI, &[]);
    let right = against(
        &[asked((60.0, 38.0, 100.0, 43.0))],
        &clear,
        width,
        DPI,
        A4,
        A4,
        128,
    );
    let said = right.describe();
    assert!(
        said.contains("The addition lands on clear paper"),
        "one addition took a plural verb: {said}"
    );
    assert!(!said.contains("addition land "), "{said}");
}

/// And when every one of several is in trouble, that is said plainly rather
/// than as "3 of the 3".
#[test]
fn all_of_them_in_trouble_is_said_as_all_of_them() {
    let (gray, width) = sheet(DPI, &[(55.0, 30.0, 110.0, 170.0)]);
    let fit = against(
        &[
            asked((60.0, 38.0, 100.0, 43.0)),
            asked((60.0, 98.0, 100.0, 103.0)),
            asked((60.0, 158.0, 100.0, 163.0)),
        ],
        &gray,
        width,
        DPI,
        A4,
        A4,
        128,
    );
    assert_eq!(fit.collisions().len(), 3, "{}", fit.describe());
    let said = fit.describe();
    assert!(said.contains("All 3 additions would land"), "{said}");
    assert!(!said.contains("3 of the 3"), "{said}");
}
