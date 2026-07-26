//! End-to-end tests: two documents in, one printable delta out.

use super::*;
use crate::pdf::{write_delta, Font, LineFont, PlacedLine};

const A4: PageSize = PageSize {
    width_mm: 210.0,
    height_mm: 297.0,
};

fn line(text: &str, y_mm: f64) -> PlacedLine {
    PlacedLine {
        text: text.to_string(),
        x_mm: 25.0,
        y_mm,
        size_pt: 14.0,
        font: LineFont::Builtin(Font::Helvetica),
        rotation_deg: 0.0,
        colour: (0.0, 0.0, 0.0),
    }
}

/// A one-page PDF with the given lines on it.
fn a_pdf(dir: &Path, name: &str, lines: &[(&str, f64)]) -> PathBuf {
    let path = dir.join(name);
    let placed: Vec<PlacedLine> = lines.iter().map(|(t, y)| line(t, *y)).collect();
    write_delta(&path, &[A4], &[placed], "test", None).unwrap();
    path
}

fn pages(dir: &Path, name: &str, per_page: &[&[(&str, f64)]]) -> PathBuf {
    let path = dir.join(name);
    let sizes = vec![A4; per_page.len()];
    let lines: Vec<Vec<PlacedLine>> = per_page
        .iter()
        .map(|page| page.iter().map(|(t, y)| line(t, *y)).collect())
        .collect();
    write_delta(&path, &sizes, &lines, "test", None).unwrap();
    path
}

/// Fast enough to run in a test, fine enough to see a word.
fn quick() -> Options {
    Options {
        dpi: 150.0,
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------
// Not destroying the documents
// ---------------------------------------------------------------------------

#[test]
fn the_delta_is_never_written_over_a_document_it_came_from() {
    // Easy to type, and it destroys the sheet you were about to print onto.
    let dir = tempfile::tempdir().unwrap();
    let original = a_pdf(dir.path(), "report.pdf", &[("Report", 40.0)]);
    let edited = a_pdf(dir.path(), "report-v2.pdf", &[("Report", 40.0)]);

    for target in [&original, &edited] {
        let err = guard_output(target, &[&original, &edited])
            .unwrap_err()
            .to_string();
        assert!(err.contains("refusing to write the delta over"), "{err}");
        assert!(err.contains("destroy the original"), "{err}");
    }
}

#[test]
fn a_roundabout_path_to_the_same_file_is_still_the_same_file() {
    let dir = tempfile::tempdir().unwrap();
    let original = a_pdf(dir.path(), "report.pdf", &[("Report", 40.0)]);
    let sneaky = dir.path().join("sub/../report.pdf");
    std::fs::create_dir_all(dir.path().join("sub")).unwrap();

    assert!(guard_output(&sneaky, &[&original]).is_err());
}

#[test]
fn a_destination_that_does_not_exist_yet_is_fine() {
    let dir = tempfile::tempdir().unwrap();
    let original = a_pdf(dir.path(), "report.pdf", &[("Report", 40.0)]);
    assert!(guard_output(&dir.path().join("delta.pdf"), &[&original]).is_ok());
}

// ---------------------------------------------------------------------------
// Options
// ---------------------------------------------------------------------------

#[test]
fn nonsense_settings_are_refused_before_any_work_is_done() {
    for (options, expected) in [
        (
            Options {
                dpi: 10.0,
                ..quick()
            },
            "dpi",
        ),
        (
            Options {
                dpi: 5000.0,
                ..quick()
            },
            "dpi",
        ),
        (
            Options {
                diff: DiffOptions {
                    ink_threshold: 255,
                    ..DiffOptions::default()
                },
                ..quick()
            },
            "ink-threshold",
        ),
        (
            Options {
                margin_mm: -1.0,
                ..quick()
            },
            "margin",
        ),
    ] {
        let err = options.validate().unwrap_err().to_string();
        assert!(err.contains(expected), "expected {expected:?}, got {err}");
    }
}

#[test]
fn a_mode_is_read_from_its_name() {
    assert_eq!(Mode::parse("raster"), Some(Mode::Raster));
    assert_eq!(Mode::parse(" VECTOR "), Some(Mode::Vector));
    assert_eq!(Mode::parse("magic"), None);
}

// ---------------------------------------------------------------------------
// The whole run
// ---------------------------------------------------------------------------

#[test]
fn adding_a_line_gives_a_delta_of_just_that_line() {
    let Ok(_) = render::engine() else { return };
    let dir = tempfile::tempdir().unwrap();
    let original = a_pdf(dir.path(), "before.pdf", &[("PURCHASE ORDER 4471", 40.0)]);
    let edited = a_pdf(
        dir.path(),
        "after.pdf",
        &[("PURCHASE ORDER 4471", 40.0), ("APPROVED", 150.0)],
    );
    let output = dir.path().join("delta.pdf");

    let outcome = run(&original, &edited, &output, &quick()).unwrap();

    assert!(!outcome.blocked(), "{:?}", outcome.checks);
    assert!(output.is_file());
    assert_eq!(outcome.pages.len(), 1);
    assert!(outcome.total_regions() >= 1);
    assert_eq!(outcome.pages_with_additions(), vec![1]);

    // The addition is where the new line was written, not where the heading is.
    let bounds = outcome.pages[0].bounds_mm().unwrap();
    assert!(
        bounds.1 > 140.0,
        "the delta reaches up to {:.1} mm",
        bounds.1
    );
}

#[test]
fn a_reflowed_document_is_blocked_and_says_why() {
    let Ok(_) = render::engine() else { return };
    let dir = tempfile::tempdir().unwrap();
    let original = a_pdf(dir.path(), "before.pdf", &[("Two hundred widgets", 100.0)]);
    // Same line, pushed down — everything after an inserted word does this.
    let edited = a_pdf(
        dir.path(),
        "after.pdf",
        &[("An extra line", 90.0), ("Two hundred widgets", 110.0)],
    );
    let output = dir.path().join("delta.pdf");

    let outcome = run(&original, &edited, &output, &quick()).unwrap();

    assert!(outcome.blocked(), "a reflow must block");
    let reflow = outcome
        .checks
        .iter()
        .find(|c| c.code == "reflow")
        .expect("no reflow check");
    assert!(reflow.format().contains("print this page fresh"));
}

#[test]
fn two_identical_documents_block_rather_than_print_a_blank_sheet() {
    let Ok(_) = render::engine() else { return };
    let dir = tempfile::tempdir().unwrap();
    let original = a_pdf(dir.path(), "before.pdf", &[("Report", 40.0)]);
    let edited = a_pdf(dir.path(), "after.pdf", &[("Report", 40.0)]);
    let output = dir.path().join("delta.pdf");

    let outcome = run(&original, &edited, &output, &quick()).unwrap();
    assert!(outcome.blocked());
    assert!(outcome.checks.iter().any(|c| c.code == "empty_delta"));
}

#[test]
fn a_page_the_edit_added_has_everything_on_it_as_new() {
    let Ok(_) = render::engine() else { return };
    let dir = tempfile::tempdir().unwrap();
    let original = pages(dir.path(), "before.pdf", &[&[("Page one", 40.0)]]);
    let edited = pages(
        dir.path(),
        "after.pdf",
        &[&[("Page one", 40.0)], &[("Page two", 40.0)]],
    );
    let output = dir.path().join("delta.pdf");

    let outcome = run(&original, &edited, &output, &quick()).unwrap();

    assert_eq!(outcome.pages.len(), 2);
    assert_eq!(outcome.pages_with_additions(), vec![2]);
    assert!(outcome.checks.iter().any(|c| c.code == "pages_added"));
    // A warning, not a blocker — the extra sheet just goes on blank paper.
    assert!(!outcome.blocked(), "{:?}", outcome.checks);
}

#[test]
fn a_page_the_edit_removed_blocks() {
    let Ok(_) = render::engine() else { return };
    let dir = tempfile::tempdir().unwrap();
    let original = pages(
        dir.path(),
        "before.pdf",
        &[&[("Page one", 40.0)], &[("Page two", 40.0)]],
    );
    let edited = pages(dir.path(), "after.pdf", &[&[("Page one", 40.0)]]);
    let output = dir.path().join("delta.pdf");

    let outcome = run(&original, &edited, &output, &quick()).unwrap();
    assert!(outcome.blocked());
    assert!(outcome.checks.iter().any(|c| c.code == "pages_removed"));
}

#[test]
fn the_delta_comes_out_at_the_sheets_own_size() {
    let Ok(_) = render::engine() else { return };
    let dir = tempfile::tempdir().unwrap();
    let original = a_pdf(dir.path(), "before.pdf", &[("Report", 40.0)]);
    let edited = a_pdf(
        dir.path(),
        "after.pdf",
        &[("Report", 40.0), ("Approved", 150.0)],
    );
    let output = dir.path().join("delta.pdf");

    run(&original, &edited, &output, &quick()).unwrap();

    let pdf = lopdf::Document::load(&output).unwrap();
    let page_id = *pdf.get_pages().values().next().unwrap();
    let media = pdf
        .get_dictionary(page_id)
        .unwrap()
        .get(b"MediaBox")
        .unwrap()
        .as_array()
        .unwrap();
    assert!((media[2].as_float().unwrap() as f64 - A4.width_pt()).abs() < 0.5);
    assert!((media[3].as_float().unwrap() as f64 - A4.height_pt()).abs() < 0.5);
}

#[test]
fn the_delta_prints_only_the_new_words_and_nothing_else() {
    // The property everything else serves. Render the delta back and check
    // there is no ink where the heading was.
    let Ok(engine) = render::engine() else { return };
    let dir = tempfile::tempdir().unwrap();
    let original = a_pdf(dir.path(), "before.pdf", &[("PURCHASE ORDER 4471", 40.0)]);
    let edited = a_pdf(
        dir.path(),
        "after.pdf",
        &[("PURCHASE ORDER 4471", 40.0), ("APPROVED", 150.0)],
    );
    let output = dir.path().join("delta.pdf");
    run(&original, &edited, &output, &quick()).unwrap();

    let page = engine.open(&output).unwrap().render(0, 150.0).unwrap();
    let px_per_mm = 150.0 / crate::geometry::MM_PER_INCH;

    let band_has_ink = |from_mm: f64, to_mm: f64| -> bool {
        let from = (from_mm * px_per_mm) as usize;
        let to = ((to_mm * px_per_mm) as usize).min(page.height);
        (from..to).any(|y| (0..page.width).any(|x| page.gray[y * page.width + x] < 200))
    };
    assert!(
        !band_has_ink(30.0, 45.0),
        "the delta would re-print the heading that is already on the sheet"
    );
    assert!(band_has_ink(140.0, 155.0), "the new line is missing");
}

#[test]
fn a_proof_image_is_written_when_asked_for() {
    let Ok(_) = render::engine() else { return };
    let dir = tempfile::tempdir().unwrap();
    let original = a_pdf(dir.path(), "before.pdf", &[("Report", 40.0)]);
    let edited = a_pdf(
        dir.path(),
        "after.pdf",
        &[("Report", 40.0), ("Approved", 150.0)],
    );
    let output = dir.path().join("delta.pdf");
    let proofs = dir.path().join("proof");

    let outcome = run(
        &original,
        &edited,
        &output,
        &Options {
            preview_dir: Some(proofs.clone()),
            ..quick()
        },
    )
    .unwrap();

    assert_eq!(outcome.previews.len(), 1);
    assert!(outcome.previews[0].is_file());

    // It shows the new ink in red over a ghost of the old.
    let proof = image::open(&outcome.previews[0]).unwrap().to_rgb8();
    let reds = proof
        .pixels()
        .filter(|p| p.0[0] > 180 && p.0[1] < 100 && p.0[2] < 100)
        .count();
    assert!(reds > 0, "the proof shows no new ink");
}

#[test]
fn a_vector_delta_keeps_the_text_as_text() {
    let Ok(_) = render::engine() else { return };
    let dir = tempfile::tempdir().unwrap();
    let original = a_pdf(dir.path(), "before.pdf", &[("Report", 40.0)]);
    let edited = a_pdf(
        dir.path(),
        "after.pdf",
        &[("Report", 40.0), ("Approved", 150.0)],
    );
    let output = dir.path().join("delta.pdf");

    let outcome = run(
        &original,
        &edited,
        &output,
        &Options {
            mode: Mode::Vector,
            ..quick()
        },
    )
    .unwrap();
    assert_eq!(outcome.mode, Mode::Vector);

    let pdf = lopdf::Document::load(&output).unwrap();
    let page_id = *pdf.get_pages().values().next().unwrap();
    let content = String::from_utf8_lossy(&pdf.get_page_content(page_id).unwrap()).to_string();
    assert!(
        content.contains("Tj") || content.contains("TJ"),
        "no text drawn"
    );
    assert!(content.contains("W n"), "the clip is missing");
}

#[test]
fn a_calibration_profile_moves_the_ink_and_is_reported() {
    let Ok(_) = render::engine() else { return };
    let dir = tempfile::tempdir().unwrap();
    // Keep profiles out of the real home directory.
    let home = dir.path().join("home");
    std::env::set_var("ONIONSKIN_HOME", &home);

    calibrate::save_profile(&Profile {
        name: "testprinter".into(),
        error: Similarity {
            dx_mm: 0.6,
            dy_mm: -0.3,
            rotation_deg: 0.0,
            scale: 1.0,
        },
        page: A4,
        rms_residual_mm: Some(0.01),
        max_residual_mm: Some(0.02),
        n_points: 5,
        created: calibrate::now(),
        notes: String::new(),
    })
    .unwrap();

    let original = a_pdf(dir.path(), "before.pdf", &[("Report", 40.0)]);
    let edited = a_pdf(
        dir.path(),
        "after.pdf",
        &[("Report", 40.0), ("Approved", 150.0)],
    );
    let output = dir.path().join("delta.pdf");

    let outcome = run(
        &original,
        &edited,
        &output,
        &Options {
            profile: Some("testprinter".into()),
            ..quick()
        },
    )
    .unwrap();

    assert_eq!(outcome.profile.as_ref().unwrap().name, "testprinter");
    assert!(outcome.checks.iter().any(|c| c.code == "calibrated"));

    let pdf = lopdf::Document::load(&output).unwrap();
    let page_id = *pdf.get_pages().values().next().unwrap();
    let content = String::from_utf8_lossy(&pdf.get_page_content(page_id).unwrap()).to_string();
    assert!(content.contains("cm"), "the correction was not applied");

    calibrate::delete_profile("testprinter").unwrap();
}

#[test]
fn a_profile_that_does_not_exist_says_so_rather_than_running_uncalibrated() {
    let Ok(_) = render::engine() else { return };
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("ONIONSKIN_HOME", dir.path().join("home"));
    let original = a_pdf(dir.path(), "before.pdf", &[("Report", 40.0)]);
    let edited = a_pdf(dir.path(), "after.pdf", &[("Report", 40.0)]);

    let err = run(
        &original,
        &edited,
        &dir.path().join("delta.pdf"),
        &Options {
            profile: Some("nosuchprinter".into()),
            ..quick()
        },
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("no calibration profile"), "{err}");
}

#[test]
fn an_uncalibrated_run_says_what_accuracy_to_expect() {
    let Ok(_) = render::engine() else { return };
    let dir = tempfile::tempdir().unwrap();
    let original = a_pdf(dir.path(), "before.pdf", &[("Report", 40.0)]);
    let edited = a_pdf(
        dir.path(),
        "after.pdf",
        &[("Report", 40.0), ("Approved", 150.0)],
    );

    let outcome = run(&original, &edited, &dir.path().join("delta.pdf"), &quick()).unwrap();
    assert!(outcome.checks.iter().any(|c| c.code == "uncalibrated"));
}

#[test]
fn the_report_is_json_a_script_can_read() {
    let Ok(_) = render::engine() else { return };
    let dir = tempfile::tempdir().unwrap();
    let original = a_pdf(dir.path(), "before.pdf", &[("Report", 40.0)]);
    let edited = a_pdf(
        dir.path(),
        "after.pdf",
        &[("Report", 40.0), ("Approved", 150.0)],
    );

    let outcome = run(&original, &edited, &dir.path().join("delta.pdf"), &quick()).unwrap();
    let json = outcome.to_json();

    assert_eq!(json["mode"], "raster");
    assert_eq!(json["blocked"], false);
    assert!(json["pages"].as_array().unwrap().len() == 1);
    assert!(!json["pages"][0]["added_regions"]
        .as_array()
        .unwrap()
        .is_empty());
    assert!(json["checks"]
        .as_array()
        .unwrap()
        .iter()
        .any(|c| c["code"] == "uncalibrated"));
}

#[test]
fn a_word_document_goes_through_the_same_path() {
    if render::find_soffice().is_none() || render::engine().is_err() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let before = dir.path().join("note.txt");
    let after = dir.path().join("note-v2.txt");
    std::fs::write(&before, "Purchase order 4471\n").unwrap();
    std::fs::write(&after, "Purchase order 4471\n\n\n\n\n\nApproved\n").unwrap();
    let output = dir.path().join("delta.pdf");

    let outcome = run(&before, &after, &output, &quick()).unwrap();
    assert!(output.is_file());
    assert!(
        outcome.total_regions() >= 1,
        "nothing was found: {:?}",
        outcome.checks
    );
}

#[test]
fn a_file_that_is_not_a_document_is_reported_cleanly() {
    let Ok(_) = render::engine() else { return };
    let dir = tempfile::tempdir().unwrap();
    let bad = dir.path().join("holiday.jpeg");
    std::fs::write(&bad, b"not a document").unwrap();
    let ok = a_pdf(dir.path(), "fine.pdf", &[("Report", 40.0)]);

    let err = run(&bad, &ok, &dir.path().join("delta.pdf"), &quick())
        .unwrap_err()
        .to_string();
    assert!(err.contains("unsupported file type"), "{err}");
}

/// Every `onionskin-` scratch directory in the shared temp directory, by name.
fn workspaces_now() -> std::collections::BTreeSet<String> {
    std::fs::read_dir(std::env::temp_dir())
        .map(|d| {
            d.filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|name| name.starts_with("onionskin-"))
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn the_workspace_does_not_survive_the_run() {
    let Ok(_) = render::engine() else { return };
    // The temp directory is shared with every other test in this binary, and
    // they run at the same time as this one. Counting what is in it therefore
    // says nothing on its own: another test's workspace, alive for the moment
    // the second count is taken, reads exactly like a leak.
    //
    // What distinguishes the two is time. A workspace somebody else is using
    // goes away when they finish with it; one this run leaked never does. So
    // the directories that appeared are watched until they go, and only a
    // directory that stays is a leak.
    let before = workspaces_now();

    let dir = tempfile::tempdir().unwrap();
    let original = a_pdf(dir.path(), "before.pdf", &[("Report", 40.0)]);
    let edited = a_pdf(
        dir.path(),
        "after.pdf",
        &[("Report", 40.0), ("Approved", 150.0)],
    );
    run(&original, &edited, &dir.path().join("delta.pdf"), &quick()).unwrap();

    let mut lingering: Vec<String> = Vec::new();
    for _ in 0..100 {
        lingering = workspaces_now().difference(&before).cloned().collect();
        if lingering.is_empty() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    panic!("a workspace was left behind: {lingering:?}");
}

// ---------------------------------------------------------------------------
// Typing onto an existing document
// ---------------------------------------------------------------------------

fn item(page: usize, x_mm: f64, y_mm: f64, text: &str) -> crate::document::Item {
    crate::document::Item {
        id: 0,
        page,
        x_mm,
        y_mm,
        text: text.into(),
        size_pt: 12.0,
        font: "Helvetica".into(),
        width_mm: None,
        rotation_deg: 0.0,
        colour: "#000000".into(),
        leading: 1.2,
    }
}

#[test]
fn words_typed_onto_a_document_land_where_they_were_asked_for() {
    let Ok(engine) = render::engine() else { return };
    let dir = tempfile::tempdir().unwrap();
    let form = a_pdf(dir.path(), "form.pdf", &[("Name:", 60.0), ("Date:", 80.0)]);
    let output = dir.path().join("delta.pdf");

    let outcome = compose_run(
        &form,
        &[item(1, 60.0, 60.0, "J. Bezzina")],
        &output,
        None,
        &quick(),
    )
    .unwrap();

    assert!(!outcome.blocked(), "{:?}", outcome.checks);
    assert!(output.is_file());

    let page = engine.open(&output).unwrap().render(0, 150.0).unwrap();
    let px_per_mm = 150.0 / crate::geometry::MM_PER_INCH;
    let (mut left, mut bottom) = (usize::MAX, 0usize);
    for y in 0..page.height {
        for x in 0..page.width {
            if page.gray[y * page.width + x] < 128 {
                left = left.min(x);
                bottom = bottom.max(y);
            }
        }
    }
    assert!(bottom > 0, "the delta rendered blank");
    assert!(
        ((left as f64 / px_per_mm) - 60.0).abs() < 1.0,
        "the words start at {:.1} mm, asked for 60",
        left as f64 / px_per_mm
    );
    assert!(
        ((bottom as f64 / px_per_mm) - 60.0).abs() < 1.0,
        "the baseline is at {:.1} mm, asked for 60",
        bottom as f64 / px_per_mm
    );
}

#[test]
fn typing_on_a_page_can_never_reflow_anything() {
    // The reason this path exists at all: the text is placed at a millimetre
    // someone chose, so nothing on the page can move, so the check that blocks
    // the two-document workflow has nothing to fire on.
    let Ok(_) = render::engine() else { return };
    let dir = tempfile::tempdir().unwrap();
    let form = a_pdf(dir.path(), "form.pdf", &[("Name:", 60.0)]);

    let outcome = compose_run(
        &form,
        &[item(1, 60.0, 60.0, "J. Bezzina")],
        &dir.path().join("delta.pdf"),
        None,
        &quick(),
    )
    .unwrap();
    assert!(!outcome.checks.iter().any(|c| c.code == "reflow"));
    assert!(!outcome.blocked());
}

#[test]
fn typing_in_the_dead_border_is_warned_about() {
    let Ok(_) = render::engine() else { return };
    let dir = tempfile::tempdir().unwrap();
    let form = a_pdf(dir.path(), "form.pdf", &[("Name:", 60.0)]);

    let outcome = compose_run(
        &form,
        &[item(1, 1.0, 60.0, "too close to the edge")],
        &dir.path().join("delta.pdf"),
        None,
        &quick(),
    )
    .unwrap();
    assert!(
        outcome.checks.iter().any(|c| c.code == "margin"),
        "{:?}",
        outcome.checks
    );
}

#[test]
fn typing_on_a_page_that_is_not_there_says_so() {
    let Ok(_) = render::engine() else { return };
    let dir = tempfile::tempdir().unwrap();
    let form = a_pdf(dir.path(), "form.pdf", &[("Name:", 60.0)]);

    let err = compose_run(
        &form,
        &[item(4, 60.0, 60.0, "nowhere")],
        &dir.path().join("delta.pdf"),
        None,
        &quick(),
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("there is no page 4"), "{err}");
}

#[test]
fn typing_nothing_blocks_rather_than_printing_a_blank_sheet() {
    let Ok(_) = render::engine() else { return };
    let dir = tempfile::tempdir().unwrap();
    let form = a_pdf(dir.path(), "form.pdf", &[("Name:", 60.0)]);

    let outcome = compose_run(&form, &[], &dir.path().join("delta.pdf"), None, &quick()).unwrap();
    assert!(outcome.blocked());
    assert!(outcome.checks.iter().any(|c| c.code == "empty_delta"));
}

#[test]
fn typing_in_another_alphabet_works_with_a_supplied_font() {
    let Ok(_) = render::engine() else { return };
    let path = std::path::PathBuf::from("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf");
    if !path.is_file() {
        return;
    }
    let font = crate::font::EmbeddedFont::load(&path).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let form = a_pdf(dir.path(), "form.pdf", &[("Name:", 60.0)]);

    let mut cyrillic = item(1, 60.0, 60.0, "Утверждено");
    cyrillic.font = "file".into();

    let outcome = compose_run(
        &form,
        &[cyrillic],
        &dir.path().join("delta.pdf"),
        Some(&font),
        &quick(),
    )
    .unwrap();
    assert!(!outcome.blocked(), "{:?}", outcome.checks);
    assert!(outcome.total_regions() >= 1, "nothing was drawn");
}
