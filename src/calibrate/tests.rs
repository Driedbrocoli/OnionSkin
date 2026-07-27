//! Tests for calibration.

use image::{GrayImage, Luma};

use super::*;
use crate::geometry::parse_page;
use crate::scan::ScanRegistration;

const A5: PageSize = PageSize {
    width_mm: 148.0,
    height_mm: 210.0,
};

/// Point profiles at a scratch directory, so a test never touches the real one.
///
/// Set for the whole process rather than per test, because the tests share it —
/// and each one uses its own profile names, so they do not collide.
fn scratch_home() -> PathBuf {
    static HOME: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    HOME.get_or_init(|| {
        let path = std::env::temp_dir().join(format!("onionskin-test-{}", std::process::id()));
        std::fs::create_dir_all(&path).unwrap();
        path
    })
    .clone()
}

fn a_profile(name: &str) -> Profile {
    Profile {
        name: name.to_string(),
        error: Similarity {
            dx_mm: 0.40,
            dy_mm: -0.15,
            rotation_deg: 0.08,
            scale: 1.0004,
        },
        page: A4,
        rms_residual_mm: Some(0.012),
        max_residual_mm: Some(0.021),
        n_points: 5,
        created: now(),
        notes: String::new(),
    }
}

// ---------------------------------------------------------------------------
// Reading points off the sheet
// ---------------------------------------------------------------------------

#[test]
fn a_reading_is_parsed_the_way_it_is_written_on_the_page() {
    assert_eq!(parse_point("P1:+0.40,-0.15").unwrap(), (1, 0.40, -0.15));
    assert_eq!(parse_point("p3:0.2,0.3").unwrap(), (3, 0.2, 0.3));
    assert_eq!(parse_point("  5 : -1.25 , 2 ").unwrap(), (5, -1.25, 2.0));
    // Someone who leaves the P off has still said what they mean.
    assert_eq!(parse_point("2:0,0").unwrap(), (2, 0.0, 0.0));
}

#[test]
fn a_reading_that_is_not_one_says_what_was_expected() {
    for bad in [
        "P1 0.4 0.2",
        "P1:0.4",
        "Px:0.4,0.2",
        "P1:a,b",
        "P1:0.4,0.2,0.3",
    ] {
        let err = parse_point(bad).unwrap_err().to_string();
        assert!(!err.is_empty(), "{bad}");
        assert!(
            err.contains("P1:dx,dy")
                || err.contains("dx,dy")
                || err.contains("not numbers")
                || err.contains("label"),
            "{bad} → {err}"
        );
    }
}

#[test]
fn an_infinite_offset_is_refused_rather_than_fitted() {
    assert!(parse_point("P1:inf,0").is_err());
    assert!(parse_point("P1:nan,0").is_err());
}

// ---------------------------------------------------------------------------
// Where the crosshairs go
// ---------------------------------------------------------------------------

#[test]
fn the_fiducials_are_spread_as_wide_as_the_sheet_allows() {
    // Rotation and scale are only observable from points far apart. Clustered
    // fiducials leave both unconstrained, and the fit comes back as a shift.
    let points = fiducials(A4, 25.0);
    assert_eq!(points.len(), 5);
    assert_eq!(points[0], (25.0, 25.0));
    assert_eq!(points[3], (185.0, 272.0));
    assert_eq!(points[4], (105.0, 148.5));
}

#[test]
fn a_small_sheet_pulls_its_crosshairs_in() {
    // Otherwise their rulers hang off the edge and cannot be read.
    // Anything from A5 up has room for the full inset.
    assert_eq!(default_inset(A4), 25.0);
    assert_eq!(default_inset(A5), 25.0);

    // Below about 100 mm it has to come in.
    assert!(default_inset(PageSize::new(90.0, 140.0)) < 25.0);
    // But never so far that the spread — and with it the rotation and scale
    // the fit depends on — is lost.
    assert!(default_inset(PageSize::new(40.0, 40.0)) >= 15.0);
}

// ---------------------------------------------------------------------------
// The fit
// ---------------------------------------------------------------------------

#[test]
fn a_pure_shift_is_recovered_exactly() {
    let offsets: Vec<(usize, f64, f64)> = (1..=5).map(|i| (i, 0.40, -0.15)).collect();
    let fit = solve_from_offsets(&offsets, A4, None).unwrap();

    assert!((fit.transform.dx_mm - 0.40).abs() < 1e-9);
    assert!((fit.transform.dy_mm + 0.15).abs() < 1e-9);
    assert!(fit.transform.rotation_deg.abs() < 1e-9);
    assert!((fit.transform.scale - 1.0).abs() < 1e-12);
    assert!(fit.max_residual_mm < 1e-9);
}

#[test]
fn a_rotation_is_recovered_from_the_corners() {
    // Build readings from a known transform and check they come back.
    let truth = Similarity {
        dx_mm: 0.3,
        dy_mm: -0.2,
        rotation_deg: 0.12,
        scale: 1.0003,
    };
    let points = fiducials(A4, default_inset(A4));
    let offsets: Vec<(usize, f64, f64)> = points
        .iter()
        .enumerate()
        .map(|(i, &(x, y))| {
            let (mx, my) = truth.apply((x, y), &A4);
            (i + 1, mx - x, my - y)
        })
        .collect();

    let fit = solve_from_offsets(&offsets, A4, None).unwrap();
    assert!(
        (fit.transform.dx_mm - truth.dx_mm).abs() < 1e-6,
        "{:?}",
        fit.transform
    );
    assert!((fit.transform.dy_mm - truth.dy_mm).abs() < 1e-6);
    assert!((fit.transform.rotation_deg - truth.rotation_deg).abs() < 1e-6);
    assert!((fit.transform.scale - truth.scale).abs() < 1e-9);
    assert!(fit.max_residual_mm < 1e-6);
}

#[test]
fn the_fit_reports_how_well_it_fitted() {
    // One point read wrongly should show up in the residual rather than being
    // absorbed silently.
    let mut offsets: Vec<(usize, f64, f64)> = (1..=5).map(|i| (i, 0.4, -0.15)).collect();
    offsets[2] = (3, 0.4, 1.0);

    let fit = solve_from_offsets(&offsets, A4, None).unwrap();
    assert!(fit.max_residual_mm > 0.2, "rms {}", fit.rms_residual_mm);
}

#[test]
fn a_point_that_is_not_on_the_target_is_refused() {
    let err = solve_from_offsets(&[(9, 0.0, 0.0)], A4, None)
        .unwrap_err()
        .to_string();
    assert!(err.contains("P9 is not on the target"), "{err}");
}

#[test]
fn the_inset_has_to_match_the_target_that_was_printed() {
    // Fitting against fiducials in the wrong place gets the rotation wrong,
    // which is why the default is shared between drawing and solving.
    let truth = Similarity {
        dx_mm: 0.0,
        dy_mm: 0.0,
        rotation_deg: 0.2,
        scale: 1.0,
    };
    let points = fiducials(A4, 25.0);
    let offsets: Vec<(usize, f64, f64)> = points
        .iter()
        .enumerate()
        .map(|(i, &(x, y))| {
            let (mx, my) = truth.apply((x, y), &A4);
            (i + 1, mx - x, my - y)
        })
        .collect();

    let right = solve_from_offsets(&offsets, A4, Some(25.0)).unwrap();
    let wrong = solve_from_offsets(&offsets, A4, Some(15.0)).unwrap();

    assert!((right.transform.rotation_deg - 0.2).abs() < 1e-6);
    assert!(
        (wrong.transform.rotation_deg - 0.2).abs() > 1e-3,
        "the wrong inset should not fit cleanly"
    );
}

// ---------------------------------------------------------------------------
// Profiles on disk
// ---------------------------------------------------------------------------

#[test]
fn a_profile_survives_being_saved_and_loaded() {
    let _home = crate::calibrate::borrow_home(&scratch_home());
    let profile = a_profile("roundtrip");
    save_profile(&profile).unwrap();

    let loaded = load_profile("roundtrip").unwrap();
    assert_eq!(loaded, profile);
    delete_profile("roundtrip").unwrap();
}

#[test]
fn the_correction_is_the_opposite_of_the_error() {
    let profile = a_profile("x");
    let error = profile.error;
    let correction = profile.correction();

    // Applying one and then the other must land back where it started.
    let point = (60.0, 150.0);
    let there = error.apply(point, &A4);
    let back = correction.apply(there, &A4);
    assert!((back.0 - point.0).abs() < 1e-9, "{back:?}");
    assert!((back.1 - point.1).abs() < 1e-9, "{back:?}");
}

#[test]
fn a_profile_that_is_not_there_lists_the_ones_that_are() {
    let _home = crate::calibrate::borrow_home(&scratch_home());
    save_profile(&a_profile("office")).unwrap();

    let err = load_profile("nowhere").unwrap_err().to_string();
    assert!(err.contains("no calibration profile 'nowhere'"), "{err}");
    assert!(
        err.contains("office"),
        "it should say what is available: {err}"
    );
    delete_profile("office").unwrap();
}

#[test]
fn a_profile_name_cannot_escape_its_folder() {
    let _home = crate::calibrate::borrow_home(&scratch_home());
    let path = profile_path("../../etc/passwd").unwrap();
    assert_eq!(path.parent().unwrap(), profiles_dir().unwrap());
    assert!(!path.to_string_lossy().contains(".."), "{path:?}");
}

#[test]
fn a_profile_with_no_usable_name_still_gets_one() {
    let _home = crate::calibrate::borrow_home(&scratch_home());
    let path = profile_path("///").unwrap();
    assert_eq!(path.file_name().unwrap(), "default.json");
}

#[test]
fn a_corrupt_profile_does_not_hide_the_good_ones() {
    let _home = crate::calibrate::borrow_home(&scratch_home());
    save_profile(&a_profile("intact")).unwrap();
    std::fs::write(profiles_dir().unwrap().join("broken.json"), b"{ not json").unwrap();

    let names: Vec<String> = list_profiles()
        .unwrap()
        .into_iter()
        .map(|p| p.name)
        .collect();
    assert!(names.contains(&"intact".to_string()), "{names:?}");

    std::fs::remove_file(profiles_dir().unwrap().join("broken.json")).unwrap();
    delete_profile("intact").unwrap();
}

#[test]
fn deleting_says_whether_there_was_anything_to_delete() {
    let _home = crate::calibrate::borrow_home(&scratch_home());
    save_profile(&a_profile("temporary")).unwrap();
    assert!(delete_profile("temporary").unwrap());
    assert!(!delete_profile("temporary").unwrap());
}

#[cfg(unix)]
#[test]
fn a_profile_is_not_readable_by_other_accounts() {
    use std::os::unix::fs::PermissionsExt;
    let _home = crate::calibrate::borrow_home(&scratch_home());
    let path = save_profile(&a_profile("private")).unwrap();
    let mode = std::fs::metadata(&path).unwrap().permissions().mode();
    assert_eq!(mode & 0o077, 0, "mode {:o}", mode & 0o777);
    delete_profile("private").unwrap();
}

#[test]
fn a_profile_describes_itself_in_terms_someone_can_check() {
    let text = a_profile("office").describe();
    assert!(text.contains("profile 'office'"), "{text}");
    assert!(text.contains("printer error"), "{text}");
    assert!(text.contains("correction"), "{text}");
    assert!(text.contains("5 points"), "{text}");
    assert!(text.contains("A4"), "{text}");
}

// ---------------------------------------------------------------------------
// The target itself
// ---------------------------------------------------------------------------

/// The content stream of one page of a written target.
fn target_page(path: &Path, index: usize) -> String {
    let pdf = lopdf::Document::load(path).unwrap();
    let pages = pdf.get_pages();
    let page_id = *pages.values().nth(index).unwrap();
    String::from_utf8_lossy(&pdf.get_page_content(page_id).unwrap()).to_string()
}

#[test]
fn the_target_is_two_pages_at_the_paper_size_asked_for() {
    // Two, because the second pass has to be printable on its own and has to
    // look different from the first when it lands. Both at the same size, or
    // the two passes would not agree about where the crosshairs are.
    let dir = tempfile::tempdir().unwrap();
    for name in ["a4", "a5", "letter", "legal", "100x150"] {
        let page = parse_page(name).unwrap();
        let path = dir.path().join(format!("{name}.pdf"));
        make_target(&path, page, None).unwrap();

        let pdf = lopdf::Document::load(&path).unwrap();
        let pages = pdf.get_pages();
        assert_eq!(pages.len(), 2, "{name}");

        for page_id in pages.values() {
            let media = pdf
                .get_dictionary(*page_id)
                .unwrap()
                .get(b"MediaBox")
                .unwrap()
                .as_array()
                .unwrap();
            let width = media[2].as_float().unwrap() as f64;
            let height = media[3].as_float().unwrap() as f64;
            assert!((width - page.width_pt()).abs() < 0.5, "{name}: {width}");
            assert!((height - page.height_pt()).abs() < 0.5, "{name}: {height}");
        }
    }
}

#[test]
fn the_target_carries_the_instructions_for_using_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("target.pdf");
    make_target(&path, A4, None).unwrap();
    let content = target_page(&path, 0);

    assert!(
        content.contains("100%"),
        "it must say to print at full size"
    );
    assert!(content.contains("Fit to page"), "and to turn that off");
    assert!(content.contains("calibrate solve"), "and what to run next");
    // And which page is which, now that there are two of them and printing
    // page 1 twice would produce a sheet nothing can read.
    assert!(content.contains("PAGE 1"), "{content}");
    assert!(content.contains("PAGE 2"), "{content}");
    // Every fiducial is labelled with where it is, so a reading can be tied
    // back to a point.
    for label in ["P1", "P2", "P3", "P4", "P5"] {
        assert!(content.contains(label), "no {label} on the target");
    }
}

#[test]
fn the_second_page_says_which_pass_it_is_and_repeats_nothing() {
    // It prints onto a sheet that already carries page 1, so anything it
    // repeats — a ruler, a label, a paragraph — lands on top of what is there
    // and leaves both unreadable.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("target.pdf");
    make_target(&path, A4, None).unwrap();
    let second = target_page(&path, 1);

    assert!(second.contains("PAGE 2"), "{second}");
    assert!(
        !second.contains("P1   25, 25"),
        "the crosshair labels belong to page 1 alone"
    );
    assert!(
        !second.contains("Fit to page"),
        "the instructions belong to page 1 alone"
    );
}

#[test]
fn the_prose_keeps_clear_of_the_middle_crosshair() {
    // It did not always. Six lines of instructions used to be centred 22 mm
    // above the middle of the sheet, which is exactly where the middle
    // crosshair and its label are — so P5 could be read neither by eye nor by
    // scan on any target Onionskin has ever written.
    let inset = default_inset(A4);
    let middle = fiducials(A4, inset)[4];
    let lowest = prose_y(A4) + 4.0 + 7.0 * 3.4 + 2.5;
    let label = middle.1 - ARM_MM - 2.5;

    assert!(
        lowest < label - 5.0,
        "the prose reaches {lowest} mm and P5's label is at {label} mm"
    );
    // And it must not run into the crosshairs along the top of the sheet.
    assert!(prose_y(A4) - 3.0 > inset + RULER_REACH_MM + 8.0);
}

#[test]
fn a_small_target_drops_the_prose_rather_than_printing_it_over_the_crosshairs() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("small.pdf");
    make_target(&path, PageSize::new(100.0, 150.0), None).unwrap();

    let pdf = lopdf::Document::load(&path).unwrap();
    let page_id = *pdf.get_pages().values().next().unwrap();
    let content = String::from_utf8_lossy(&pdf.get_page_content(page_id).unwrap()).to_string();

    assert!(content.contains("P1"), "the crosshairs are still there");
    assert!(
        !content.contains("calibrate solve"),
        "the prose should be gone"
    );
    assert!(content.contains("re-feed"), "but the gist should remain");
}

#[test]
fn the_target_renders_with_ink_on_it() {
    let Ok(engine) = crate::render::engine() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("target.pdf");
    make_target(&path, A4, None).unwrap();

    let document = engine.open(&path).unwrap();
    let page = document.render(0, 150.0).unwrap();

    let inked = page.gray.iter().filter(|v| **v < 128).count();
    assert!(
        inked > 500,
        "only {inked} inked pixels — the target is blank"
    );

    // And the ink is spread across the page, not bunched in one corner.
    let (mut left, mut right, mut top, mut bottom) = (usize::MAX, 0usize, usize::MAX, 0usize);
    for y in 0..page.height {
        for x in 0..page.width {
            if page.gray[y * page.width + x] < 128 {
                left = left.min(x);
                right = right.max(x);
                top = top.min(y);
                bottom = bottom.max(y);
            }
        }
    }
    assert!(
        right - left > page.width / 2 && bottom - top > page.height / 2,
        "the fiducials are not spread out"
    );
}

#[test]
fn the_target_folder_is_made_if_it_is_not_there() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nested/target.pdf");
    make_target(&path, A4, None).unwrap();
    assert!(path.is_file());
}

// ---------------------------------------------------------------------------
// The whole loop
// ---------------------------------------------------------------------------

#[test]
fn a_reading_off_a_printed_target_becomes_a_correction_that_undoes_it() {
    // The property the whole feature rests on: whatever the printer does to
    // the second pass, applying the stored correction first cancels it.
    let _home = crate::calibrate::borrow_home(&scratch_home());
    let printer_does = Similarity {
        dx_mm: 0.45,
        dy_mm: -0.18,
        rotation_deg: 0.09,
        scale: 1.0006,
    };
    let points = fiducials(A4, default_inset(A4));
    let readings: Vec<(usize, f64, f64)> = points
        .iter()
        .enumerate()
        .map(|(i, &(x, y))| {
            let (mx, my) = printer_does.apply((x, y), &A4);
            // Read to the nearest quarter millimetre, as the ruler allows.
            let round = |v: f64| (v * 4.0).round() / 4.0;
            (i + 1, round(mx - x), round(my - y))
        })
        .collect();

    let fit = solve_from_offsets(&readings, A4, None).unwrap();
    let profile = Profile {
        name: "loop".into(),
        error: fit.transform,
        page: A4,
        rms_residual_mm: Some(fit.rms_residual_mm),
        max_residual_mm: Some(fit.max_residual_mm),
        n_points: readings.len(),
        created: now(),
        notes: String::new(),
    };
    save_profile(&profile).unwrap();
    let loaded = load_profile("loop").unwrap();

    // Somewhere on the page, corrected then printed, must land where asked.
    for spot in [(30.0, 40.0), (105.0, 148.5), (180.0, 260.0)] {
        let corrected = loaded.correction().apply(spot, &A4);
        let printed = printer_does.apply(corrected, &A4);
        let error = ((printed.0 - spot.0).powi(2) + (printed.1 - spot.1).powi(2)).sqrt();
        assert!(
            error < 0.5,
            "at {spot:?} the ink lands {error:.3} mm out — worse than uncalibrated"
        );
    }
    delete_profile("loop").unwrap();
}

// ---------------------------------------------------------------------------
// Reading a sheet back off a scan
// ---------------------------------------------------------------------------

/// A sheet with both passes printed on it, drawn straight into an image.
///
/// There is no printer and no scanner in a test run, so the sheet is drawn
/// here, from the same measurements the target itself is drawn from, with the
/// second pass moved by an amount the test chose. Whatever comes back that is
/// not that amount is the measurement's own error — which is a stricter check
/// than a real sheet could ever give, because on paper nobody knows the true
/// answer to compare against.
///
/// It is a stand-in and not a copy. The lines here are laid down a third of a
/// millimetre wide, where the target itself draws a third of a *point*, which
/// is a third as much again — so these sheets have heavier ink than any real
/// one, which is the harder case for telling a ring from a diamond. The one
/// test that reads the genuine article, rendered by the renderer, is
/// `a_target_printed_twice_and_scanned_gives_back_the_offset`.
struct Sheet {
    image: GrayImage,
    registration: ScanRegistration,
}

impl Sheet {
    /// A blank sheet as a scanner sees it: `dpi` pixels to the inch, lying
    /// `skew_deg` off square, with a margin around it.
    fn new(page: PageSize, dpi: f64, skew_deg: f64) -> Sheet {
        let px_per_mm = dpi / crate::geometry::MM_PER_INCH;
        let (sin_t, cos_t) = skew_deg.to_radians().sin_cos();
        let turned = |mm: (f64, f64)| {
            (
                (cos_t * mm.0 - sin_t * mm.1) * px_per_mm,
                (sin_t * mm.0 + cos_t * mm.1) * px_per_mm,
            )
        };
        let corners = [
            turned((0.0, 0.0)),
            turned((page.width_mm, 0.0)),
            turned((0.0, page.height_mm)),
            turned((page.width_mm, page.height_mm)),
        ];
        let low = corners
            .iter()
            .fold((f64::MAX, f64::MAX), |a, c| (a.0.min(c.0), a.1.min(c.1)));
        let high = corners
            .iter()
            .fold((f64::MIN, f64::MIN), |a, c| (a.0.max(c.0), a.1.max(c.1)));
        let margin = 6.0 * px_per_mm;

        Sheet {
            // Paper is never quite white in a scan, and ink is never quite
            // black. Neither should matter, and a test using 255 and 0 would
            // not show it if they did.
            image: GrayImage::from_pixel(
                (high.0 - low.0 + margin * 2.0).ceil() as u32,
                (high.1 - low.1 + margin * 2.0).ceil() as u32,
                Luma([246]),
            ),
            registration: ScanRegistration {
                page,
                px_per_mm,
                skew_deg,
                origin_px: (margin - low.0, margin - low.1),
            },
        }
    }

    /// One round spot of ink, centred on a point of the sheet.
    ///
    /// The edge of the spot is drawn soft, over the one pixel it falls in.
    /// That is what a scanner sees — no printed edge lands neatly on a pixel
    /// boundary, and every scanner blurs the ones that nearly do — and it is
    /// also the difference between a fair test and a flattering one. Ink laid
    /// down as whole pixels only is ink whose shape depends on where the grid
    /// happens to fall, and a diamond drawn that way has its middle a
    /// half-pixel away from where its geometry says, which is an error in the
    /// test rather than in what the test is measuring.
    fn dot(&mut self, at_mm: (f64, f64), radius_mm: f64) {
        let (cx, cy) = self.registration.page_mm_to_pixel(at_mm);
        let radius = radius_mm * self.registration.px_per_mm;
        let (width, height) = self.image.dimensions();
        let x0 = (cx - radius - 1.0).floor().max(0.0) as u32;
        let y0 = (cy - radius - 1.0).floor().max(0.0) as u32;
        let x1 = (cx + radius + 1.0).ceil().clamp(0.0, width as f64 - 1.0) as u32;
        let y1 = (cy + radius + 1.0).ceil().clamp(0.0, height as f64 - 1.0) as u32;
        for y in y0..=y1 {
            for x in x0..=x1 {
                let (dx, dy) = (x as f64 + 0.5 - cx, y as f64 + 0.5 - cy);
                let covered = (radius + 0.5 - dx.hypot(dy)).clamp(0.0, 1.0);
                if covered <= 0.0 {
                    continue;
                }
                let level = (246.0 - covered * (246.0 - 28.0)).round() as u8;
                if level < self.image.get_pixel(x, y).0[0] {
                    self.image.put_pixel(x, y, Luma([level]));
                }
            }
        }
    }

    /// A line, laid down the way a printer lays one: a round nib dragged along
    /// it, so the ink is as wide at the ends as in the middle.
    fn line(&mut self, from: (f64, f64), to: (f64, f64), width_mm: f64) {
        let length = (to.0 - from.0).hypot(to.1 - from.1);
        let steps = ((length * self.registration.px_per_mm * 2.0).ceil() as usize).max(1);
        for step in 0..=steps {
            let along = step as f64 / steps as f64;
            self.dot(
                (
                    from.0 + (to.0 - from.0) * along,
                    from.1 + (to.1 - from.1) * along,
                ),
                width_mm / 2.0,
            );
        }
    }

    fn ring(&mut self, centre: (f64, f64), radius_mm: f64, width_mm: f64) {
        let steps = ((std::f64::consts::TAU * radius_mm * self.registration.px_per_mm * 2.0).ceil()
            as usize)
            .max(16);
        for step in 0..steps {
            let angle = step as f64 / steps as f64 * std::f64::consts::TAU;
            self.dot(
                (
                    centre.0 + radius_mm * angle.cos(),
                    centre.1 + radius_mm * angle.sin(),
                ),
                width_mm / 2.0,
            );
        }
    }

    fn diamond(&mut self, centre: (f64, f64), radius_mm: f64, width_mm: f64) {
        let corners = [
            (centre.0, centre.1 - radius_mm),
            (centre.0 + radius_mm, centre.1),
            (centre.0, centre.1 + radius_mm),
            (centre.0 - radius_mm, centre.1),
        ];
        for corner in 0..4 {
            self.line(corners[corner], corners[(corner + 1) % 4], width_mm);
        }
    }

    /// A block of ink standing in for a piece of printed text.
    fn blot(&mut self, centre: (f64, f64), width_mm: f64, height_mm: f64) {
        let rows = ((height_mm * self.registration.px_per_mm).ceil() as usize).max(1);
        for row in 0..=rows {
            let y = centre.1 - height_mm / 2.0 + height_mm * row as f64 / rows as f64;
            self.line(
                (centre.0 - width_mm / 2.0, y),
                (centre.0 + width_mm / 2.0, y),
                0.12,
            );
        }
    }

    /// Everything page 1 puts around one crosshair: the ring, the four arms,
    /// both scales with their ticks, the printed numbers beside them and the
    /// label above.
    ///
    /// All of it, because most of it is ink the measurement has to ignore, and
    /// a test that drew only the ring would prove nothing about that.
    fn first_pass_at(&mut self, at: (f64, f64)) {
        let (x, y) = at;
        self.ring(at, RING_MM, 0.35);
        self.line((x - ARM_MM, y), (x - ARM_GAP_MM, y), 0.35);
        self.line((x + ARM_GAP_MM, y), (x + ARM_MM, y), 0.35);
        self.line((x, y - ARM_MM), (x, y - ARM_GAP_MM), 0.35);
        self.line((x, y + ARM_GAP_MM), (x, y + ARM_MM), 0.35);

        let scale = SCALE_OFFSET_MM;
        self.line(
            (x - RULER_REACH_MM, y + scale),
            (x + RULER_REACH_MM, y + scale),
            0.25,
        );
        self.line(
            (x - scale, y - RULER_REACH_MM),
            (x - scale, y + RULER_REACH_MM),
            0.25,
        );
        let steps = (RULER_REACH_MM / RULER_STEP_MM) as i64;
        for step in -steps..=steps {
            let offset = step as f64 * RULER_STEP_MM;
            let tick = tick_length(offset);
            self.line((x + offset, y + scale), (x + offset, y + scale + tick), 0.2);
            self.line((x - scale, y + offset), (x - scale - tick, y + offset), 0.2);
        }
        for mark in [-4.0f64, -2.0, 0.0, 2.0, 4.0] {
            self.blot((x + mark, y + 10.9), 1.5, 1.3);
            self.blot((x - 10.1, y + mark), 1.5, 1.3);
        }
        self.blot((x, y - ARM_MM - 3.5), 24.0, 2.1);
    }

    /// What page 2 puts there: the diamond and the two reading arms.
    fn second_pass_at(&mut self, at: (f64, f64)) {
        let (x, y) = at;
        self.line((x, y + SECOND_ARM_GAP_MM), (x, y + ARM_MM), 0.35);
        self.line((x - ARM_MM, y), (x - SECOND_ARM_GAP_MM, y), 0.35);
        self.diamond(
            (x + DIAMOND_OFFSET_MM, y - DIAMOND_OFFSET_MM),
            DIAMOND_MM,
            0.35,
        );
    }

    fn first_pass(&mut self, page: PageSize, inset: f64) {
        for at in fiducials(page, inset) {
            self.first_pass_at(at);
        }
    }

    /// The second pass, landing wherever `printer` puts each crosshair.
    fn second_pass(
        &mut self,
        page: PageSize,
        inset: f64,
        printer: impl Fn((f64, f64)) -> (f64, f64),
    ) {
        for at in fiducials(page, inset) {
            self.second_pass_at(printer(at));
        }
    }
}

/// A sheet printed twice, the second pass shifted by `offset`.
fn printed_twice(page: PageSize, dpi: f64, skew_deg: f64, offset: (f64, f64)) -> Sheet {
    let inset = default_inset(page);
    let mut sheet = Sheet::new(page, dpi, skew_deg);
    sheet.first_pass(page, inset);
    sheet.second_pass(page, inset, |at| (at.0 + offset.0, at.1 + offset.1));
    sheet
}

#[test]
fn the_reading_window_takes_in_the_two_marks_and_nothing_else() {
    // Every number checked here is a measurement off the target, and this test
    // exists because moving any one of them a couple of millimetres would put a
    // printed ruler or a crosshair's label inside the window — where it would
    // arrive as a shape to be argued with, and where a printed "0" is close
    // enough to a small ring to cost the fiducial its reading.
    let window = Window::around((0.0, 0.0));

    assert!(
        window.y1 < SCALE_OFFSET_MM,
        "the x scale below the crosshair is inside the window"
    );
    assert!(
        window.x0 > -SCALE_OFFSET_MM,
        "the y scale to its left is inside the window"
    );
    assert!(
        window.y0 > -(ARM_MM + 2.5),
        "the 'P1  25, 25' label above it is inside the window"
    );

    // And both marks are inside it, with a couple of millimetres to spare for
    // a printer that is out and a scanner that found the paper's edge roughly.
    assert!(window.holds((RING_MM + 2.5, RING_MM + 2.5)));
    assert!(window.holds((-(RING_MM + 2.5), -(RING_MM + 2.5))));
    let reach = DIAMOND_OFFSET_MM + DIAMOND_MM + MAX_OFFSET_MM * 0.8;
    assert!(
        window.holds((reach, -reach)),
        "a diamond {MAX_OFFSET_MM} mm out would run off the edge of the window"
    );
}

#[test]
fn the_two_marks_stand_clear_of_the_crosshair_and_of_each_other() {
    // The whole design rests on this. Two shapes that touch are one shape, and
    // one shape has one middle, so a reading whose marks ran together would be
    // lost — which is why the diamond sits out on the diagonal rather than over
    // the point it is measuring.
    let stroke = 0.35;

    // The ring stands free of its own arms, or there would be nothing on the
    // first pass small enough to measure.
    assert!(
        ARM_GAP_MM - RING_MM - stroke > 1.0,
        "the ring is welded to its arms"
    );
    // The diamond keeps clear of the arms it sits between, and of the ring,
    // for as far out as a reading is accepted at all.
    assert!(
        DIAMOND_OFFSET_MM - DIAMOND_MM - stroke >= 2.0,
        "the diamond touches an arm before the printer is even 2 mm out"
    );
    assert!(
        DIAMOND_OFFSET_MM * 2f64.sqrt() - DIAMOND_MM - RING_MM - stroke >= 2.0,
        "the diamond touches the ring before the printer is even 2 mm out"
    );
}

#[test]
fn a_known_offset_is_recovered_from_a_scan_of_the_sheet() {
    // Including offsets that are negative in one axis and positive in the
    // other, because a sign the wrong way round is the failure the two
    // different marks exist to prevent, and it would pass a test that only
    // ever shifted the second pass down and to the right.
    for offset in [
        (0.40, -0.20),
        (-0.55, 0.35),
        (0.0, 0.0),
        (1.20, 0.95),
        (-1.10, -0.80),
    ] {
        let sheet = printed_twice(A4, 300.0, 0.0, offset);
        let readings = measure_from_scan(&sheet.image, &sheet.registration, A4, None).unwrap();

        assert_eq!(readings.len(), 5, "offset {offset:?}");
        for reading in &readings {
            assert!(
                (reading.dx_mm - offset.0).abs() < 0.05 && (reading.dy_mm - offset.1).abs() < 0.05,
                "P{} read {:+.3}, {:+.3} from a sheet offset by {offset:?}",
                reading.index,
                reading.dx_mm,
                reading.dy_mm
            );
            assert!(
                reading.confidence > 0.75,
                "P{} was only {:.0}% sure of a clean sheet",
                reading.index,
                reading.confidence * 100.0
            );
        }
    }
}

#[test]
fn a_sheet_that_is_turned_on_the_glass_still_reads_true() {
    // The offset is a distance on the paper, and the paper does not know it is
    // lying crooked on a scanner. Measuring in pixels and calling them
    // millimetres would come back short by a cosine and turned by the skew.
    let offset = (0.62, -0.44);
    let sheet = printed_twice(A4, 300.0, 1.7, offset);
    let readings = measure_from_scan(&sheet.image, &sheet.registration, A4, None).unwrap();

    assert_eq!(readings.len(), 5);
    for reading in &readings {
        assert!(
            (reading.dx_mm - offset.0).abs() < 0.06 && (reading.dy_mm - offset.1).abs() < 0.06,
            "P{} read {:+.3}, {:+.3} off a sheet turned 1.7°",
            reading.index,
            reading.dx_mm,
            reading.dy_mm
        );
    }
}

#[test]
fn a_coarser_scan_of_a_smaller_sheet_is_still_read() {
    // 200 dpi on A5: fewer than half as many pixels on each mark, and a page
    // whose fiducials sit closer together.
    let offset = (-0.35, 0.55);
    let sheet = printed_twice(A5, 200.0, 0.0, offset);
    let readings = measure_from_scan(&sheet.image, &sheet.registration, A5, None).unwrap();

    assert_eq!(readings.len(), 5);
    for reading in &readings {
        assert!(
            (reading.dx_mm - offset.0).abs() < 0.08 && (reading.dy_mm - offset.1).abs() < 0.08,
            "P{} read {:+.3}, {:+.3} at 200 dpi",
            reading.index,
            reading.dx_mm,
            reading.dy_mm
        );
    }
}

#[test]
fn a_whole_printer_error_comes_back_off_the_scan() {
    // The property the automatic route rests on: rotation and scale are only
    // visible as the way the offsets differ across the sheet, so a fit through
    // five scanned crosshairs has to recover all four numbers, not just the
    // shift that is obvious at any one of them.
    let truth = Similarity {
        dx_mm: 0.45,
        dy_mm: -0.18,
        rotation_deg: 0.09,
        scale: 1.0006,
    };
    let inset = default_inset(A4);
    let mut sheet = Sheet::new(A4, 400.0, 0.0);
    sheet.first_pass(A4, inset);
    sheet.second_pass(A4, inset, |at| truth.apply(at, &A4));

    let (profile, readings) = calibrate_from_scan(
        &sheet.image,
        &sheet.registration,
        A4,
        None,
        "scanned",
        "from a synthetic sheet",
    )
    .unwrap();

    assert_eq!(readings.len(), 5);
    assert!(
        (profile.error.dx_mm - truth.dx_mm).abs() < 0.05
            && (profile.error.dy_mm - truth.dy_mm).abs() < 0.05,
        "{}",
        profile.error.describe()
    );
    assert!(
        (profile.error.rotation_deg - truth.rotation_deg).abs() < 0.02,
        "rotation came back as {:.4}°",
        profile.error.rotation_deg
    );
    assert!(
        (profile.error.scale - truth.scale).abs() < 2e-4,
        "scale came back as {:.6}",
        profile.error.scale
    );

    // And the whole point of it: corrected first, printed second, lands home.
    for spot in [(30.0, 40.0), (105.0, 148.5), (180.0, 260.0)] {
        let printed = truth.apply(profile.correction().apply(spot, &A4), &A4);
        let error = (printed.0 - spot.0).hypot(printed.1 - spot.1);
        assert!(error < 0.1, "at {spot:?} the ink lands {error:.3} mm out");
    }
    // Measuring is not saving. Nothing should have been written anywhere.
    assert_eq!(profile.name, "scanned");
    assert_eq!(profile.n_points, 5);
}

#[test]
fn a_crosshair_the_second_pass_missed_is_left_out_rather_than_guessed() {
    // A sheet fed slightly askew, a dry nozzle, a fold across one corner: the
    // reading is simply not there. Reporting it as zero would be worse than
    // useless, because zero is a perfectly plausible answer and the fit would
    // quietly average it in with the four that are real.
    let inset = default_inset(A4);
    let mut sheet = Sheet::new(A4, 300.0, 0.0);
    sheet.first_pass(A4, inset);
    for (index, at) in fiducials(A4, inset).into_iter().enumerate() {
        if index != 2 {
            sheet.second_pass_at((at.0 + 0.5, at.1 - 0.3));
        }
    }

    let readings = measure_from_scan(&sheet.image, &sheet.registration, A4, None).unwrap();
    let seen: Vec<usize> = readings.iter().map(|r| r.index).collect();
    assert_eq!(seen, vec![1, 2, 4, 5], "P3 has nothing on it to measure");
}

#[test]
fn a_crosshair_with_three_marks_on_it_is_left_out_rather_than_guessed() {
    // A sheet that went through three times, or two sheets printed on top of
    // one another. Whichever diamond is nearest is not necessarily the second
    // pass, and there is no way to tell from the paper which one is — so this
    // is a fiducial with no answer, not a fiducial with a likely one.
    let inset = default_inset(A4);
    let mut sheet = Sheet::new(A4, 300.0, 0.0);
    sheet.first_pass(A4, inset);
    sheet.second_pass(A4, inset, |at| (at.0 + 0.4, at.1 - 0.25));

    let crowded = fiducials(A4, inset)[1];
    sheet.diamond(
        (
            crowded.0 + DIAMOND_OFFSET_MM - 1.4,
            crowded.1 - DIAMOND_OFFSET_MM + 1.5,
        ),
        DIAMOND_MM,
        0.35,
    );

    let readings = measure_from_scan(&sheet.image, &sheet.registration, A4, None).unwrap();
    let seen: Vec<usize> = readings.iter().map(|r| r.index).collect();
    assert_eq!(seen, vec![1, 3, 4, 5], "P2 has two second passes on it");
}

#[test]
fn speckle_and_dirt_do_not_derail_a_reading() {
    // A scan of real paper is never clean: dust on the glass, fibres in the
    // sheet, a fleck of toner from the last thing through the printer.
    let offset = (0.30, -0.45);
    let mut sheet = printed_twice(A4, 300.0, 0.0, offset);

    // A dependable pseudo-random sprinkle, so a failure here fails every time.
    let mut seed = 0x5eed_1234_9876_4321u64;
    let mut next = move || {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (seed >> 11) as f64 / (1u64 << 53) as f64
    };
    // Two hundred flecks in every window, up to a fifth of a millimetre across
    // — heavier than any scan of a sheet somebody meant to keep. A fleck that
    // lands touching a mark becomes part of it and there is no answer to that,
    // so what is being asked here is that the marks still be found, that the
    // dirt be recognised as dirt, and that what comes back still be better than
    // the quarter-millimetre a person can read off the printed ruler.
    for at in fiducials(A4, default_inset(A4)) {
        for _ in 0..200 {
            let spot = (
                at.0 - WINDOW_NEAR_MM + next() * (WINDOW_NEAR_MM + WINDOW_FAR_MM),
                at.1 - WINDOW_FAR_MM + next() * (WINDOW_NEAR_MM + WINDOW_FAR_MM),
            );
            sheet.dot(spot, 0.03 + next() * 0.07);
        }
    }

    let readings = measure_from_scan(&sheet.image, &sheet.registration, A4, None).unwrap();
    assert_eq!(readings.len(), 5, "speckle lost a whole crosshair");
    for reading in &readings {
        assert!(
            (reading.dx_mm - offset.0).abs() < 0.06 && (reading.dy_mm - offset.1).abs() < 0.06,
            "P{} read {:+.3}, {:+.3} through the speckle",
            reading.index,
            reading.dx_mm,
            reading.dy_mm
        );
    }
}

#[test]
fn a_blank_sheet_is_read_as_nothing_rather_than_as_shapes() {
    // Otsu's threshold always finds a split, even in a photograph of blank
    // paper — and half a sheet of paper called ink is a window full of
    // enormous shapes, any of which might be talked into being a mark.
    let sheet = Sheet::new(A4, 300.0, 0.0);
    let readings = measure_from_scan(&sheet.image, &sheet.registration, A4, None).unwrap();
    assert!(readings.is_empty(), "{readings:?}");
}

#[test]
fn fewer_than_three_readable_crosshairs_are_refused_rather_than_fitted() {
    let inset = default_inset(A4);
    let mut sheet = Sheet::new(A4, 300.0, 0.0);
    sheet.first_pass(A4, inset);
    for at in fiducials(A4, inset).into_iter().take(2) {
        sheet.second_pass_at((at.0 + 0.4, at.1 + 0.2));
    }

    let err = calibrate_from_scan(&sheet.image, &sheet.registration, A4, None, "half", "")
        .unwrap_err()
        .to_string();
    assert!(err.contains("only 2 of the 5"), "{err}");
    assert!(err.contains("three is the fewest"), "{err}");
    assert!(err.contains("300 dpi"), "it should say what to try: {err}");
}

#[test]
fn a_scan_too_coarse_to_measure_says_so_instead_of_trying() {
    // At 100 dpi the marks are a pixel and a half thick. Something would be
    // found, and it would be wrong.
    let sheet = printed_twice(A4, 100.0, 0.0, (0.4, -0.2));
    let err = measure_from_scan(&sheet.image, &sheet.registration, A4, None)
        .unwrap_err()
        .to_string();
    assert!(err.contains("100 dpi"), "{err}");
    assert!(err.contains("300 dpi"), "and what to do about it: {err}");
}

#[test]
fn a_scan_of_the_wrong_paper_size_is_refused() {
    // The registration turns pixels into millimetres against a stated paper
    // size. Measure an A4 scan as though it were Letter and every crosshair is
    // looked for in the wrong place, with nothing in the failure to say why.
    let sheet = printed_twice(A4, 300.0, 0.0, (0.4, -0.2));
    let letter = parse_page("letter").unwrap();
    let err = measure_from_scan(&sheet.image, &sheet.registration, letter, None)
        .unwrap_err()
        .to_string();
    assert!(err.contains("registered as A4"), "{err}");
}

#[test]
fn an_offset_no_printer_could_make_is_refused_rather_than_reported() {
    // Six millimetres out is not a re-feed error, it is two marks paired up
    // wrongly — and a wrong pair fitted with a straight face is exactly what
    // this must never do.
    let sheet = printed_twice(A4, 300.0, 0.0, (6.0, 0.0));
    let readings = measure_from_scan(&sheet.image, &sheet.registration, A4, None).unwrap();
    assert!(readings.is_empty(), "{readings:?}");
}

#[test]
fn a_reading_that_had_to_work_for_it_says_so_in_its_confidence() {
    // Confidence is not decoration. A sheet read cleanly and a sheet read
    // through a mess must not come back looking the same.
    let clean = printed_twice(A4, 300.0, 0.0, (0.4, -0.2));
    let clean = measure_from_scan(&clean.image, &clean.registration, A4, None).unwrap();

    // The same sheet, but the second pass landed two and a half millimetres
    // out — far enough to be doubted, not far enough to be refused.
    let far = printed_twice(A4, 300.0, 0.0, (2.2, 1.2));
    let far = measure_from_scan(&far.image, &far.registration, A4, None).unwrap();

    assert_eq!(clean.len(), 5);
    assert_eq!(far.len(), 5);
    let best = |readings: &[Reading]| readings.iter().map(|r| r.confidence).fold(0.0f64, f64::max);
    assert!(
        best(&far) < best(&clean) - 0.2,
        "{:.2} against {:.2}",
        best(&far),
        best(&clean)
    );
}

#[test]
fn a_reading_describes_itself_the_way_the_sheet_is_labelled() {
    let text = Reading {
        index: 3,
        dx_mm: 0.4,
        dy_mm: -0.15,
        confidence: 0.82,
    }
    .describe();
    assert!(text.contains("P3"), "{text}");
    assert!(text.contains("+0.40"), "{text}");
    assert!(text.contains("-0.15"), "{text}");
    assert!(text.contains("82%"), "{text}");
}

#[test]
fn a_target_printed_twice_and_scanned_gives_back_the_offset() {
    // The one test that reads the target Onionskin really writes, rather than
    // a drawing of it made here. The two pages go through the same renderer
    // that draws every preview, one is laid over the other a known distance
    // away, and the result is measured. Anything that quietly broke the shapes
    // on the page — a diamond stroked as four separate lines and coming apart
    // at the corners, an arm gap narrow enough to weld the ring to its arms —
    // would show up here and nowhere else.
    let Ok(engine) = crate::render::engine() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("target.pdf");
    make_target(&path, A4, None).unwrap();

    let dpi = 300.0;
    let px_per_mm = dpi / crate::geometry::MM_PER_INCH;
    let document = engine.open(&path).unwrap();
    assert_eq!(document.len(), 2);
    let first = document.render(0, dpi).unwrap();
    let second = document.render(1, dpi).unwrap();
    assert_eq!((first.width, first.height), (second.width, second.height));

    // A whole number of pixels, so that what the printer "did" is known
    // exactly rather than to within a rounding.
    let (shift_x, shift_y) = (5i64, -3i64);
    let mut sheet =
        GrayImage::from_raw(first.width as u32, first.height as u32, first.gray.clone()).unwrap();
    for y in 0..second.height {
        for x in 0..second.width {
            let level = second.gray[y * second.width + x];
            if level >= 250 {
                continue;
            }
            let (nx, ny) = (x as i64 + shift_x, y as i64 + shift_y);
            if nx < 0 || ny < 0 || nx >= first.width as i64 || ny >= first.height as i64 {
                continue;
            }
            if level < sheet.get_pixel(nx as u32, ny as u32).0[0] {
                sheet.put_pixel(nx as u32, ny as u32, Luma([level]));
            }
        }
    }

    let registration = ScanRegistration {
        page: A4,
        px_per_mm,
        skew_deg: 0.0,
        origin_px: (0.0, 0.0),
    };
    let readings = measure_from_scan(&sheet, &registration, A4, None).unwrap();
    let truth = (shift_x as f64 / px_per_mm, shift_y as f64 / px_per_mm);

    assert_eq!(readings.len(), 5, "{readings:?}");
    for reading in &readings {
        assert!(
            (reading.dx_mm - truth.0).abs() < 0.05 && (reading.dy_mm - truth.1).abs() < 0.05,
            "P{} read {:+.3}, {:+.3} off a real target shifted by {:+.3}, {:+.3}",
            reading.index,
            reading.dx_mm,
            reading.dy_mm,
            truth.0,
            truth.1
        );
        // A clean sheet read off the real target should be all but certain.
        assert!(reading.confidence > 0.85, "{}", reading.describe());
    }
}

// ---------------------------------------------------------------------------
// Learning from an ordinary job
// ---------------------------------------------------------------------------

/// A scan of a sheet with square blots at the given places, printed by a
/// printer whose error is `error`.
fn sheet_with_blots(page: PageSize, at_mm: &[(f64, f64)], error: Similarity) -> (image::GrayImage, crate::scan::ScanRegistration) {
    let px_per_mm = 300.0 / 25.4;
    let width = (page.width_mm * px_per_mm).round() as u32;
    let height = (page.height_mm * px_per_mm).round() as u32;
    let mut scan = image::GrayImage::from_pixel(width, height, image::Luma([255]));

    for (x_mm, y_mm) in at_mm {
        // Where the printer really put it.
        let (px_mm, py_mm) = error.apply((*x_mm, *y_mm), &page);
        let cx = (px_mm * px_per_mm) as i64;
        let cy = (py_mm * px_per_mm) as i64;
        let arm = (1.5 * px_per_mm) as i64;
        for dy in -arm..=arm {
            for dx in -arm..=arm {
                let (x, y) = (cx + dx, cy + dy);
                if x >= 0 && y >= 0 && (x as u32) < width && (y as u32) < height {
                    scan.put_pixel(x as u32, y as u32, image::Luma([0]));
                }
            }
        }
    }

    let registration = crate::scan::ScanRegistration {
        page,
        px_per_mm,
        skew_deg: 0.0,
        origin_px: (0.0, 0.0),
    };
    (scan, registration)
}

/// The marks a delta asked for, as squares round the given places.
fn asked_for(at_mm: &[(f64, f64)]) -> Vec<Asked> {
    at_mm
        .iter()
        .map(|(x, y)| Asked {
            centre_mm: (*x, *y),
            bounds_mm: (x - 1.5, y - 1.5, x + 1.5, y + 1.5),
            ink_mm2: 9.0,
        })
        .collect()
}

/// A mark read off a rendered delta is centred on its ink, not on its box.
///
/// The two are the same point only for something symmetrical. Here the left
/// half of the box is black and the right half is paper, so the box's middle
/// and the ink's middle are a quarter of the box apart — and it is the ink's
/// middle that must come back, because that is what the scan will be measured
/// against.
#[test]
fn a_mark_is_centred_on_its_ink_rather_than_on_its_box() {
    let dpi = 100.0;
    let px_per_mm = dpi / 25.4;
    let (width, height) = (40usize, 20usize);
    let mut gray = vec![255u8; width * height];
    for y in 0..height {
        for x in 0..width / 2 {
            gray[y * width + x] = 0;
        }
    }
    let region = crate::diff::Region {
        x0_mm: 0.0,
        y0_mm: 0.0,
        x1_mm: width as f64 / px_per_mm,
        y1_mm: height as f64 / px_per_mm,
        ink_mm2: 0.0,
        px_bbox: (0, 0, width, height),
    };

    let marks = marks_asked_for(&gray, width, dpi, &[region], 128);
    assert_eq!(marks.len(), 1);
    let mark = marks[0];
    let box_middle = (region.x1_mm / 2.0, region.y1_mm / 2.0);
    assert!(
        (mark.centre_mm.0 - box_middle.0 / 2.0).abs() < 0.1,
        "centred on the box at {:?} instead of on the ink",
        mark.centre_mm
    );
    assert!((mark.centre_mm.1 - box_middle.1).abs() < 0.1, "{mark:?}");
    // Half the box is solid ink, and that is what it should report.
    let half = region.x1_mm * region.y1_mm / 2.0;
    assert!((mark.ink_mm2 - half).abs() < 0.2, "{mark:?} against {half}");
}

/// A mark with nothing in it is dropped rather than reported at the origin.
#[test]
fn a_mark_with_no_ink_in_it_is_not_a_mark() {
    let region = crate::diff::Region {
        x0_mm: 0.0,
        y0_mm: 0.0,
        x1_mm: 5.0,
        y1_mm: 5.0,
        ink_mm2: 0.0,
        px_bbox: (0, 0, 20, 20),
    };
    let gray = vec![255u8; 20 * 20];
    assert!(marks_asked_for(&gray, 20, 100.0, &[region], 128).is_empty());
}

#[test]
fn a_shift_the_printer_introduced_is_measured_off_an_ordinary_job() {
    // The whole point: every delta ever printed is a set of marks in known
    // places, so calibration can happen by using the program rather than by
    // sitting down to do it.
    let page = A4;
    let places = [(40.0, 40.0), (160.0, 40.0), (40.0, 250.0), (160.0, 250.0)];
    let real_error = Similarity {
        dx_mm: 0.8,
        dy_mm: -0.5,
        rotation_deg: 0.0,
        scale: 1.0,
    };
    let (scan, registration) = sheet_with_blots(page, &places, real_error);

    let landings = measure_landings(&scan, &registration, &asked_for(&places), 128);
    assert_eq!(landings.len(), 4, "{landings:?}");
    for landing in &landings {
        assert!(landing.confidence >= BELIEVABLE, "{landing:?}");
        // Each one missed by about the shift that was applied.
        assert!((landing.miss_mm() - 0.943).abs() < 0.2, "{landing:?}");
    }

    // And the fit recovers the printer's error, to a tenth of a millimetre.
    let learnt = learn_from_landings(&landings, page, "office").unwrap();
    assert!((learnt.error.dx_mm - 0.8).abs() < 0.1, "{:?}", learnt.error);
    assert!((learnt.error.dy_mm - -0.5).abs() < 0.1, "{:?}", learnt.error);
    assert_eq!(learnt.n_points, 4);
}

#[test]
fn a_correction_already_applied_is_not_thrown_away() {
    // A printer that is already being corrected would otherwise appear to
    // have no error at all, and the next profile would undo the correction
    // that was working.
    let page = A4;
    let places = [(40.0, 40.0), (160.0, 40.0), (40.0, 250.0), (160.0, 250.0)];
    let error = Similarity {
        dx_mm: 1.0,
        dy_mm: 0.0,
        rotation_deg: 0.0,
        scale: 1.0,
    };
    // The delta was written with the correction for that error, so its content
    // sits a millimetre back from where it was asked for…
    let correction = error.inverse();
    let rendered: Vec<(f64, f64)> = places
        .iter()
        .map(|p| correction.apply(*p, &page))
        .collect();
    // …and the printer's shift then puts the ink on the mark, which is the
    // whole point of having the profile. A scan of this sheet looks perfect.
    let (scan, registration) = sheet_with_blots(page, &places, Similarity::IDENTITY);

    // What is measured against is the delta as rendered, not as asked for.
    let landings = measure_landings(&scan, &registration, &asked_for(&rendered), 128);
    let learnt = learn_from_landings(&landings, page, "office").unwrap();

    // The printer's own error is still about a millimetre — not nothing.
    assert!(
        (learnt.error.dx_mm - 1.0).abs() < 0.15,
        "the correction was thrown away: {:?}",
        learnt.error
    );
}

#[test]
fn an_addition_that_did_not_print_is_not_measured() {
    // A region where almost no ink turned up did not print, and measuring
    // where its middle is would be measuring the paper.
    let page = A4;
    let places = [(40.0, 40.0), (160.0, 40.0), (40.0, 250.0)];
    // Only two of the three were actually printed.
    let (scan, registration) = sheet_with_blots(page, &places[..2], Similarity::IDENTITY);

    let landings = measure_landings(&scan, &registration, &asked_for(&places), 128);
    let believed = landings.iter().filter(|l| l.believable()).count();
    assert_eq!(believed, 2, "{landings:?}");

    // The one that did not print is still in the list, and says so. Dropping
    // it would lose the most useful thing this sheet has to say, and would
    // make the count below read "2 of 2" when 3 were asked for.
    assert_eq!(landings.len(), 3, "{landings:?}");
    let absent = &landings[2];
    assert!(!absent.printed());
    assert!(absent.describe().contains("did not print"), "{}", absent.describe());

    // And two is not enough to fit from, so it says so rather than guessing.
    let said = learn_from_landings(&landings, page, "office")
        .unwrap_err()
        .to_string();
    assert!(said.contains("three is the fewest"), "{said}");
    assert!(said.contains("2 of the 3"), "{said}");
}

/// An addition written over something already printed is not measured.
///
/// The centroid of a window holding both the addition and the line it was
/// written across is the middle of neither, and it would be fitted to as though
/// it were a real reading — a bad number carried into every later delta,
/// arrived at confidently.
#[test]
fn an_addition_that_landed_on_existing_ink_is_left_out() {
    let page = A4;
    let places = [(40.0, 40.0), (160.0, 40.0), (40.0, 250.0), (160.0, 250.0)];
    let (mut scan, registration) = sheet_with_blots(page, &places, Similarity::IDENTITY);

    // A line of the document running through the last of them.
    let px_per_mm = registration.px_per_mm;
    let y = (250.0 * px_per_mm) as u32;
    for x in ((150.0 * px_per_mm) as u32)..((172.0 * px_per_mm) as u32) {
        for dy in 0..(1.2 * px_per_mm) as u32 {
            scan.put_pixel(x, y + dy, image::Luma([0]));
        }
    }

    let landings = measure_landings(&scan, &registration, &asked_for(&places), 128);
    assert_eq!(landings.len(), 4, "{landings:?}");
    let crowded = &landings[3];
    assert!(
        crowded.confidence > CROWDED,
        "the line through it was not noticed: {crowded:?}"
    );
    assert!(!crowded.believable());
    assert!(
        crowded.doubt().unwrap().contains("already on the sheet"),
        "{:?}",
        crowded.doubt()
    );

    // Three clean readings are still enough to fit from, and the answer is the
    // one the three clean marks say — not one dragged sideways by the fourth.
    let learnt = learn_from_landings(&landings, page, "office").unwrap();
    assert_eq!(learnt.n_points, 3);
    assert!(learnt.error.dx_mm.abs() < 0.15, "{:?}", learnt.error);
    assert!(learnt.error.dy_mm.abs() < 0.15, "{:?}", learnt.error);
}

/// Additions running down one column give a shift and nothing else.
///
/// A form filled in on the left is a perfectly ordinary job to learn from, and
/// its shift is worth having — the paper path pushes every sheet the same way.
/// Its rotation is not: marks in a line say nothing about the direction across
/// them, and a fit asked for one anyway would return a confident number that is
/// wrong everywhere else on the page.
#[test]
fn marks_in_a_line_give_a_shift_and_no_invented_rotation() {
    let page = A4;
    let places = [(40.0, 60.0), (40.0, 140.0), (40.0, 220.0), (44.0, 260.0)];
    let error = Similarity {
        dx_mm: 0.9,
        dy_mm: -0.4,
        rotation_deg: 0.0,
        scale: 1.0,
    };
    let (scan, registration) = sheet_with_blots(page, &places, error);

    let landings = measure_landings(&scan, &registration, &asked_for(&places), 128);
    let learnt = learn_from_landings(&landings, page, "office").unwrap();

    assert_eq!(learnt.error.rotation_deg, 0.0, "{:?}", learnt.error);
    assert_eq!(learnt.error.scale, 1.0, "{:?}", learnt.error);
    assert!((learnt.error.dx_mm - 0.9).abs() < 0.1, "{:?}", learnt.error);
    assert!((learnt.error.dy_mm - -0.4).abs() < 0.1, "{:?}", learnt.error);
    assert!(learnt.notes.contains("shift only"), "{}", learnt.notes);
}

/// Marks spread over the sheet are fitted for everything, as before.
#[test]
fn marks_spread_over_the_sheet_are_fitted_for_rotation_too() {
    let page = A4;
    let places = [(40.0, 40.0), (160.0, 40.0), (40.0, 250.0), (160.0, 250.0)];
    let error = Similarity {
        dx_mm: 0.5,
        dy_mm: 0.3,
        rotation_deg: 0.25,
        scale: 1.0,
    };
    let (scan, registration) = sheet_with_blots(page, &places, error);
    let landings = measure_landings(&scan, &registration, &asked_for(&places), 128);
    let learnt = learn_from_landings(&landings, page, "office").unwrap();

    assert!(!learnt.notes.contains("shift only"), "{}", learnt.notes);
    // Wide enough for the blots landing on whole pixels, which at 300 dpi is
    // a twelfth of a millimetre and about a fiftieth of a degree over this
    // lever arm; narrow enough that a rotation not really there would fail.
    assert!(
        (learnt.error.rotation_deg - 0.25).abs() < 0.07,
        "{:?}",
        learnt.error
    );
}

/// The spread is measured across the marks, not along them.
#[test]
fn a_long_line_of_marks_is_still_a_line() {
    // Nearly the whole height of the sheet, and a millimetre wide.
    let along = [(40.0, 20.0), (40.5, 120.0), (40.2, 200.0), (40.8, 280.0)];
    assert!(spread_across(&along) < 2.0, "{}", spread_across(&along));

    // The same points turned 45°, so neither axis alone would notice.
    let page = A4;
    let turned: Vec<(f64, f64)> = along
        .iter()
        .map(|p| {
            Similarity {
                dx_mm: 0.0,
                dy_mm: 0.0,
                rotation_deg: 45.0,
                scale: 1.0,
            }
            .apply(*p, &page)
        })
        .collect();
    assert!(spread_across(&turned) < 2.0, "{}", spread_across(&turned));

    // And a proper spread reads as one.
    let spread = [(40.0, 40.0), (160.0, 40.0), (40.0, 250.0), (160.0, 250.0)];
    assert!(spread_across(&spread) > 100.0, "{}", spread_across(&spread));
}

#[test]
fn a_blank_sheet_teaches_nothing_rather_than_teaching_nonsense() {
    let page = A4;
    let places = [(40.0, 40.0), (160.0, 40.0), (40.0, 250.0)];
    let (scan, registration) = sheet_with_blots(page, &[], Similarity::IDENTITY);
    let landings = measure_landings(&scan, &registration, &asked_for(&places), 128);
    // One entry per addition asked for, every one of them saying nothing
    // printed — which is what a sheet that went through the printer switched
    // off looks like, and is worth being told.
    assert_eq!(landings.len(), 3, "{landings:?}");
    assert!(landings.iter().all(|l| !l.printed()), "{landings:?}");
    assert!(learn_from_landings(&landings, page, "x").is_err());
}

// ---------------------------------------------------------------------------
// Checking one sheet before committing the rest of the stack
// ---------------------------------------------------------------------------

/// A landing with a chosen miss and a chosen amount of ink found.
fn landed(miss_mm: f64, confidence: f64) -> Landing {
    Landing {
        intended: (100.0, 100.0),
        observed: (100.0 + miss_mm, 100.0),
        confidence,
    }
}

#[test]
fn a_sheet_where_everything_landed_close_enough_is_good() {
    let report = PrintReport::of(vec![landed(0.2, 1.0), landed(0.31, 0.95)], 1.0);
    assert!(report.good());
    assert_eq!((report.absent, report.adrift, report.unmeasurable), (0, 0, 0));
    assert!((report.worst_mm - 0.31).abs() < 1e-9);
    assert!(report.verdict().contains("Everything printed"), "{}", report.verdict());
    assert!(report.lines().iter().all(|l| l.contains('✓')), "{:?}", report.lines());
}

#[test]
fn an_addition_that_is_not_on_the_sheet_fails_it_and_names_the_likely_cause() {
    // The mistake this catches is the sheet going back in the wrong way up,
    // which puts the whole delta somewhere it cannot be seen.
    let report = PrintReport::of(vec![landed(0.0, 0.0), landed(0.0, 0.0)], 1.0);
    assert!(!report.good());
    assert_eq!(report.absent, 2);
    let said = report.verdict();
    assert!(said.contains("did not print"), "{said}");
    assert!(said.contains("wrong way up"), "{said}");
    // And no false comfort about how well it landed.
    assert!(!said.contains("Everything printed"), "{said}");
}

/// An addition written over existing ink is not a misplaced addition.
///
/// Its centroid is the middle of the addition and of whatever it was written
/// across, which is not a distance at all — and treating it as one would fail
/// a sheet that came out perfectly well.
#[test]
fn an_addition_over_existing_ink_is_set_aside_rather_than_failed() {
    let report = PrintReport::of(vec![landed(0.2, 1.0), landed(8.0, 1.9)], 1.0);
    assert!(report.good(), "{}", report.verdict());
    assert_eq!((report.absent, report.adrift, report.unmeasurable), (0, 0, 1));
    // Eight millimetres of nonsense does not become the worst reading.
    assert!((report.worst_mm - 0.2).abs() < 1e-9, "{}", report.worst_mm);
    assert!(report.verdict().contains("already printed"), "{}", report.verdict());
    assert!(report.lines()[1].contains('?'), "{:?}", report.lines());
}

#[test]
fn how_close_is_close_enough_is_the_callers_to_set() {
    // The same sheet: fine for a signature, not fine for a box on a form.
    let landings = vec![landed(1.4, 1.0)];
    assert!(PrintReport::of(landings.clone(), 2.0).good());
    let strict = PrintReport::of(landings, 0.5);
    assert!(!strict.good());
    assert_eq!(strict.adrift, 1);
    assert!(strict.verdict().contains("0.50 mm"), "{}", strict.verdict());
    assert!(strict.verdict().contains("1.40 mm"), "{}", strict.verdict());
}

#[test]
fn a_sheet_can_be_wrong_in_more_than_one_way_at_once() {
    let report = PrintReport::of(
        vec![landed(0.1, 1.0), landed(0.0, 0.0), landed(3.0, 1.0), landed(5.0, 1.8)],
        1.0,
    );
    assert_eq!((report.absent, report.adrift, report.unmeasurable), (1, 1, 1));
    let said = report.verdict();
    assert!(said.contains("did not print"), "{said}");
    assert!(said.contains("landed more than"), "{said}");
    assert!(said.contains("already printed"), "{said}");
}
