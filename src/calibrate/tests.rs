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

#[test]
fn the_target_is_a_pdf_at_the_paper_size_asked_for() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["a4", "a5", "letter", "legal", "100x150"] {
        let page = parse_page(name).unwrap();
        let path = dir.path().join(format!("{name}.pdf"));
        make_target(&path, page, None).unwrap();

        let pdf = lopdf::Document::load(&path).unwrap();
        let pages = pdf.get_pages();
        assert_eq!(pages.len(), 1, "{name}");

        let media = pdf
            .get_dictionary(*pages.values().next().unwrap())
            .unwrap()
            .get(b"MediaBox")
            .unwrap()
            .as_array()
            .unwrap();
        let width = media[2].as_float().unwrap() as f64;
        assert!((width - page.width_pt()).abs() < 0.5, "{name}: {width}");
    }
}

#[test]
fn the_target_carries_the_instructions_for_using_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("target.pdf");
    make_target(&path, A4, None).unwrap();

    let pdf = lopdf::Document::load(&path).unwrap();
    let page_id = *pdf.get_pages().values().next().unwrap();
    let content = String::from_utf8_lossy(&pdf.get_page_content(page_id).unwrap()).to_string();

    assert!(
        content.contains("100%"),
        "it must say to print at full size"
    );
    assert!(content.contains("Fit to page"), "and to turn that off");
    assert!(content.contains("calibrate solve"), "and what to run next");
    // Every fiducial is labelled with where it is, so a reading can be tied
    // back to a point.
    for label in ["P1", "P2", "P3", "P4", "P5"] {
        assert!(content.contains(label), "no {label} on the target");
    }
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
