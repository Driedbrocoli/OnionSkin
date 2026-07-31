//! Tests for writing the delta.

use super::*;
use crate::diff::{diff_page, DiffOptions, Mask};
use crate::pdf::{write_delta, Font, LineFont, PlacedLine};
use crate::render::engine;

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

fn a_pdf(dir: &Path, name: &str, lines: &[(&str, f64)]) -> PathBuf {
    let path = dir.join(name);
    let placed: Vec<PlacedLine> = lines.iter().map(|(t, y)| line(t, *y)).collect();
    write_delta(&path, &[A4], &[placed], "test", None).unwrap();
    path
}

/// Ink everywhere the mask says, at the given grey.
fn flat_rgb(mask: &Mask, value: u8) -> Vec<u8> {
    let mut rgb = vec![255u8; mask.width * mask.height * 3];
    for y in 0..mask.height {
        for x in 0..mask.width {
            if mask.get(x, y) {
                let at = (y * mask.width + x) * 3;
                rgb[at] = value;
                rgb[at + 1] = value;
                rgb[at + 2] = value;
            }
        }
    }
    rgb
}

fn a_diff(added: &[(usize, usize)], width: usize, height: usize) -> PageDiff {
    let mut mask = Mask::blank(width, height);
    for &(x, y) in added {
        mask.set(x, y, true);
    }
    PageDiff {
        index: 0,
        size: A4,
        dpi: 25.4 * width as f64 / A4.width_mm,
        added_px: mask.count(),
        removed_px: 0,
        added_regions: Vec::new(),
        removed_regions: Vec::new(),
        removed: Mask::blank(width, height),
        added: mask,
    }
}

// ---------------------------------------------------------------------------
// Un-matting
// ---------------------------------------------------------------------------

#[test]
fn solid_black_ink_comes_back_solid_and_opaque() {
    let mut mask = Mask::blank(2, 1);
    mask.set(0, 0, true);
    let rgb = flat_rgb(&mask, 0);

    let ink = unmatte(&rgb, 2, 1, &mask, (0, 0), 2);
    assert_eq!(&ink.rgb[0..3], &[0, 0, 0]);
    assert_eq!(ink.alpha[0], 255);
    // And the pixel outside the mask is untouched and invisible.
    assert_eq!(ink.alpha[1], 0);
}

#[test]
fn a_half_covered_edge_pixel_comes_back_half_transparent() {
    // The point of un-matting. A renderer gives a glyph's edge as mid-grey; if
    // that is printed at full opacity the letter sits inside a pale halo.
    let mut mask = Mask::blank(1, 1);
    mask.set(0, 0, true);
    let rgb = vec![128u8, 128, 128];

    let ink = unmatte(&rgb, 1, 1, &mask, (0, 0), 1);
    assert!(
        (ink.alpha[0] as i32 - 127).abs() <= 2,
        "alpha {}",
        ink.alpha[0]
    );
    // And the recovered ink is near black, not the grey it was composited to.
    assert!(ink.rgb[0] < 12, "ink {}", ink.rgb[0]);
}

#[test]
fn coloured_ink_keeps_its_colour() {
    let mut mask = Mask::blank(1, 1);
    mask.set(0, 0, true);
    // Solid red, fully covered.
    let rgb = vec![255u8, 0, 0];

    let ink = unmatte(&rgb, 1, 1, &mask, (0, 0), 1);
    assert_eq!(ink.alpha[0], 255);
    assert_eq!(&ink.rgb[0..3], &[255, 0, 0]);
}

#[test]
fn a_barely_inked_pixel_does_not_divide_by_nothing() {
    let mut mask = Mask::blank(1, 1);
    mask.set(0, 0, true);
    let rgb = vec![255u8, 255, 255]; // in the mask but the same as paper

    let ink = unmatte(&rgb, 1, 1, &mask, (0, 0), 1);
    // Nothing to see, so nothing is printed. The colour underneath does not
    // matter as long as the division did not run away with it.
    assert_eq!(ink.alpha[0], 0);
    assert_eq!(ink.rgb.len(), 3);
}

#[test]
fn only_the_cropped_window_is_unmatted() {
    let mut mask = Mask::blank(4, 4);
    mask.set(3, 3, true);
    let rgb = flat_rgb(&mask, 0);

    let ink = unmatte(&rgb, 2, 2, &mask, (2, 2), 4);
    assert_eq!(ink.width, 2);
    // The set pixel is the last one of the window.
    assert_eq!(ink.alpha, vec![0, 0, 0, 255]);
}

// ---------------------------------------------------------------------------
// The raster delta
// ---------------------------------------------------------------------------

#[test]
fn a_raster_delta_is_a_pdf_at_the_sheets_size() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("delta.pdf");
    let diff = a_diff(&[(10, 10)], 40, 56);

    let mut writer = RasterDeltaWriter::new(&path, "test").unwrap();
    writer
        .add_page(&diff, Some(&flat_rgb(&diff.added, 0)))
        .unwrap();
    writer.close().unwrap();

    let pdf = lopdf::Document::load(&path).unwrap();
    let pages = pdf.get_pages();
    assert_eq!(pages.len(), 1);

    let media = pdf
        .get_dictionary(*pages.values().next().unwrap())
        .unwrap()
        .get(b"MediaBox")
        .unwrap()
        .as_array()
        .unwrap();
    assert!((media[2].as_float().unwrap() as f64 - A4.width_pt()).abs() < 0.5);
}

#[test]
fn a_page_with_no_additions_is_blank_but_present() {
    // The delta's page numbers have to line up with the sheets in the tray.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("delta.pdf");
    let empty = a_diff(&[], 40, 56);

    let mut writer = RasterDeltaWriter::new(&path, "test").unwrap();
    writer.add_page(&empty, None).unwrap();
    writer
        .add_page(
            &a_diff(&[(5, 5)], 40, 56),
            Some(&flat_rgb(&Mask::blank(40, 56), 0)),
        )
        .unwrap();
    writer.close().unwrap();

    let pdf = lopdf::Document::load(&path).unwrap();
    assert_eq!(pdf.get_pages().len(), 2);
}

#[test]
fn only_the_inked_part_of_the_page_is_embedded() {
    // A delta is a few words on an empty sheet. Encoding the whole page anyway
    // is most of the run time and most of the file that goes to the printer.
    let dir = tempfile::tempdir().unwrap();
    let (w, h) = (400, 560);
    let diff = a_diff(&[(200, 300), (201, 300)], w, h);

    let path = dir.path().join("small.pdf");
    let mut writer = RasterDeltaWriter::new(&path, "test").unwrap();
    writer
        .add_page(&diff, Some(&flat_rgb(&diff.added, 0)))
        .unwrap();
    writer.close().unwrap();

    let pdf = lopdf::Document::load(&path).unwrap();
    let image = pdf
        .objects
        .values()
        .filter_map(|o| o.as_stream().ok())
        .find(|s| {
            s.dict
                .get(b"Subtype")
                .and_then(|o| o.as_name())
                .map(|n| n == b"Image")
                .unwrap_or(false)
                && s.dict.has(b"SMask")
        })
        .expect("no image was embedded");

    let width = image.dict.get(b"Width").unwrap().as_i64().unwrap();
    let height = image.dict.get(b"Height").unwrap().as_i64().unwrap();
    assert!(width <= 6, "embedded {width} px wide for two pixels of ink");
    assert!(height <= 4, "embedded {height} px tall");
}

#[test]
fn the_embedded_ink_carries_a_soft_mask_so_edges_stay_smooth() {
    let dir = tempfile::tempdir().unwrap();
    let diff = a_diff(&[(10, 10)], 40, 56);
    let path = dir.path().join("soft.pdf");

    let mut writer = RasterDeltaWriter::new(&path, "test").unwrap();
    writer
        .add_page(&diff, Some(&flat_rgb(&diff.added, 0)))
        .unwrap();
    writer.close().unwrap();

    let pdf = lopdf::Document::load(&path).unwrap();
    let has_soft_mask = pdf
        .objects
        .values()
        .filter_map(|o| o.as_stream().ok())
        .any(|s| s.dict.has(b"SMask"));
    assert!(has_soft_mask, "no soft mask, so every edge prints opaque");
}

#[test]
fn a_delta_says_what_it_is_for() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("titled.pdf");
    let mut writer = RasterDeltaWriter::new(&path, "Onionskin delta").unwrap();
    writer.add_page(&a_diff(&[], 40, 56), None).unwrap();
    writer.close().unwrap();

    let text = String::from_utf8_lossy(&std::fs::read(&path).unwrap()).to_string();
    assert!(text.contains("Onionskin"), "the producer should be named");
}

#[test]
fn the_delta_folder_is_made_if_it_is_not_there() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nested/deeper/delta.pdf");
    let mut writer = RasterDeltaWriter::new(&path, "test").unwrap();
    writer.add_page(&a_diff(&[], 40, 56), None).unwrap();
    assert_eq!(writer.close().unwrap(), path);
    assert!(path.is_file());
}

// ---------------------------------------------------------------------------
// The vector delta
// ---------------------------------------------------------------------------

#[test]
fn a_vector_delta_keeps_the_page_and_clips_it() {
    let dir = tempfile::tempdir().unwrap();
    let source = a_pdf(dir.path(), "edited.pdf", &[("Approved", 100.0)]);
    let out = dir.path().join("vector.pdf");

    let mut diff = a_diff(&[], 40, 56);
    diff.added_regions = vec![Region {
        x0_mm: 25.0,
        y0_mm: 95.0,
        x1_mm: 60.0,
        y1_mm: 101.0,
        ink_mm2: 20.0,
        px_bbox: (0, 0, 1, 1),
    }];

    build_vector_delta(&[diff], &source, &out, 0.3, "test", None).unwrap();

    let pdf = lopdf::Document::load(&out).unwrap();
    assert_eq!(pdf.get_pages().len(), 1);

    // The clip must be there, and it must be balanced.
    let page_id = *pdf.get_pages().values().next().unwrap();
    let content = pdf.get_page_content(page_id).unwrap();
    let text = String::from_utf8_lossy(&content);
    assert!(text.contains(" re"), "no clip rectangle: {text}");
    assert!(text.contains("W n"), "the rectangle is not used as a clip");
    assert!(text.trim_end().ends_with('Q'), "the clip is never closed");
}

#[test]
fn a_vector_page_with_no_additions_comes_out_blank() {
    // Otherwise the whole original page prints on top of itself.
    let dir = tempfile::tempdir().unwrap();
    let source = a_pdf(dir.path(), "edited.pdf", &[("Approved", 100.0)]);
    let out = dir.path().join("vector.pdf");

    build_vector_delta(&[a_diff(&[], 40, 56)], &source, &out, 0.3, "test", None).unwrap();

    let pdf = lopdf::Document::load(&out).unwrap();
    let page_id = *pdf.get_pages().values().next().unwrap();
    assert!(
        !pdf.get_dictionary(page_id).unwrap().has(b"Contents"),
        "a page with nothing added must print nothing"
    );
}

// ---------------------------------------------------------------------------
// Calibration
// ---------------------------------------------------------------------------

#[test]
fn no_correction_copies_the_file_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let source = a_pdf(dir.path(), "delta.pdf", &[("Approved", 100.0)]);
    let out = dir.path().join("corrected.pdf");

    apply_correction(&source, &out, Similarity::default(), &[A4]).unwrap();
    assert_eq!(
        std::fs::read(&source).unwrap(),
        std::fs::read(&out).unwrap()
    );
}

#[test]
fn a_correction_moves_the_ink_and_leaves_the_sheet_alone() {
    let dir = tempfile::tempdir().unwrap();
    let source = a_pdf(dir.path(), "delta.pdf", &[("Approved", 100.0)]);
    let out = dir.path().join("corrected.pdf");

    let correction = Similarity {
        dx_mm: 0.4,
        dy_mm: -0.15,
        rotation_deg: 0.05,
        scale: 1.0004,
    };
    apply_correction(&source, &out, correction, &[A4]).unwrap();

    let before = lopdf::Document::load(&source).unwrap();
    let after = lopdf::Document::load(&out).unwrap();

    // The media box is the physical sheet and must not move.
    let media = |pdf: &lopdf::Document| -> Vec<f32> {
        let id = *pdf.get_pages().values().next().unwrap();
        pdf.get_dictionary(id)
            .unwrap()
            .get(b"MediaBox")
            .unwrap()
            .as_array()
            .unwrap()
            .iter()
            .map(|o| o.as_float().unwrap())
            .collect()
    };
    assert_eq!(media(&before), media(&after));

    // And the content is wrapped in a transform.
    let page_id = *after.get_pages().values().next().unwrap();
    let content = String::from_utf8_lossy(&after.get_page_content(page_id).unwrap()).to_string();
    assert!(content.contains(" cm"), "no transform was applied");
    assert!(content.starts_with('q'), "the transform is not balanced");
    assert!(content.trim_end().ends_with('Q'));
}

#[test]
fn a_correction_survives_a_page_whose_content_is_an_array() {
    // A page's content may be several streams, concatenated as if they were
    // one — so a `q` may open in one and close in the next. Editing a stream
    // in place would break that; splicing new ones on each end does not.
    let dir = tempfile::tempdir().unwrap();
    let source = a_pdf(dir.path(), "delta.pdf", &[("Approved", 100.0)]);
    let mut pdf = lopdf::Document::load(&source).unwrap();
    let page_id = *pdf.get_pages().values().next().unwrap();
    let existing = pdf
        .get_dictionary(page_id)
        .unwrap()
        .get(b"Contents")
        .unwrap()
        .clone();
    let extra = pdf.add_object(Stream::new(dictionary! {}, b" ".to_vec()));
    pdf.get_dictionary_mut(page_id).unwrap().set(
        "Contents",
        Object::Array(vec![existing, Object::Reference(extra)]),
    );
    let split = dir.path().join("split.pdf");
    pdf.save(&split).unwrap();

    let out = dir.path().join("corrected.pdf");
    let correction = Similarity {
        dx_mm: 0.5,
        dy_mm: 0.0,
        rotation_deg: 0.0,
        scale: 1.0,
    };
    apply_correction(&split, &out, correction, &[A4]).unwrap();

    let after = lopdf::Document::load(&out).unwrap();
    let page_id = *after.get_pages().values().next().unwrap();
    let content = String::from_utf8_lossy(&after.get_page_content(page_id).unwrap()).to_string();
    assert!(content.starts_with('q'), "{content}");
    assert!(content.trim_end().ends_with('Q'), "{content}");
}

// ---------------------------------------------------------------------------
// Conforming to the source page
// ---------------------------------------------------------------------------

fn frame(media: (f64, f64, f64, f64), crop: (f64, f64, f64, f64), rotate: i64) -> PageFrame {
    PageFrame {
        media,
        crop,
        rotate,
    }
}

#[test]
fn a_simple_source_needs_no_conforming() {
    let dir = tempfile::tempdir().unwrap();
    let source = a_pdf(dir.path(), "delta.pdf", &[("Approved", 100.0)]);
    let out = dir.path().join("conformed.pdf");
    let plain = frame((0.0, 0.0, 595.28, 841.89), (0.0, 0.0, 595.28, 841.89), 0);

    conform_to_source(&source, &out, &[plain]).unwrap();
    assert_eq!(
        std::fs::read(&source).unwrap(),
        std::fs::read(&out).unwrap()
    );
}

#[test]
fn a_turned_source_gives_the_delta_the_same_turn() {
    // A printer places a page using its boxes and /Rotate. If the delta
    // disagrees about any of them, no amount of calibration will line the two
    // impressions up.
    let dir = tempfile::tempdir().unwrap();
    let source = a_pdf(dir.path(), "delta.pdf", &[("Approved", 100.0)]);
    let out = dir.path().join("conformed.pdf");
    let turned = frame((0.0, 0.0, 595.28, 841.89), (0.0, 0.0, 595.28, 841.89), 90);

    conform_to_source(&source, &out, &[turned]).unwrap();

    let pdf = lopdf::Document::load(&out).unwrap();
    let page_id = *pdf.get_pages().values().next().unwrap();
    let page = pdf.get_dictionary(page_id).unwrap();

    assert_eq!(page.get(b"Rotate").unwrap().as_i64().unwrap(), 90);
    let content = String::from_utf8_lossy(&pdf.get_page_content(page_id).unwrap()).to_string();
    assert!(content.contains("cm"), "the content was not transformed");
}

#[test]
fn an_offset_source_gives_the_delta_the_same_boxes() {
    let dir = tempfile::tempdir().unwrap();
    let source = a_pdf(dir.path(), "delta.pdf", &[("Approved", 100.0)]);
    let out = dir.path().join("conformed.pdf");
    let offset = frame((10.0, 20.0, 605.0, 861.0), (10.0, 20.0, 605.0, 861.0), 0);

    conform_to_source(&source, &out, &[offset]).unwrap();

    let pdf = lopdf::Document::load(&out).unwrap();
    let page_id = *pdf.get_pages().values().next().unwrap();
    let media: Vec<f32> = pdf
        .get_dictionary(page_id)
        .unwrap()
        .get(b"MediaBox")
        .unwrap()
        .as_array()
        .unwrap()
        .iter()
        .map(|o| o.as_float().unwrap())
        .collect();
    assert!((media[0] - 10.0).abs() < 1e-3, "{media:?}");
    assert!((media[1] - 20.0).abs() < 1e-3, "{media:?}");
}

#[test]
fn every_quarter_turn_has_a_matrix_that_maps_the_corner() {
    // The matrix must send the delta's top-left corner to wherever the same
    // spot appears on the source page.
    for rotate in [0, 90, 180, 270] {
        let f = frame((0.0, 0.0, 400.0, 600.0), (0.0, 0.0, 400.0, 600.0), rotate);
        let (a, b, c, d, e, g) = display_to_user_matrix(&f);

        // The determinant of a rotation is 1: no flip, no scale.
        let determinant = a * d - b * c;
        assert!(
            (determinant - 1.0).abs() < 1e-9,
            "rotate {rotate} determinant {determinant}"
        );
        // And the origin lands inside the page.
        assert!((0.0..=400.0).contains(&e), "rotate {rotate} e {e}");
        assert!((0.0..=600.0).contains(&g), "rotate {rotate} f {g}");
    }
}

// ---------------------------------------------------------------------------
// The proof image
// ---------------------------------------------------------------------------

#[test]
fn the_proof_shows_new_ink_in_red_over_a_ghost_of_the_old() {
    let mut diff = a_diff(&[(3, 3)], 8, 8);
    diff.removed.set(5, 5, true);
    // The old page has ink at (1, 1).
    let mut old = vec![255u8; 64];
    old[8 + 1] = 0;

    let proof = preview_page(&diff, &old, 8);

    assert_eq!(proof.get_pixel(3, 3).0, [214, 51, 51], "new ink is red");
    assert_eq!(proof.get_pixel(5, 5).0, [120, 160, 255], "lost ink is blue");
    // The old ink is a ghost: visible, but well back from black.
    let ghost = proof.get_pixel(1, 1).0[0];
    assert!(ghost > 150 && ghost < 210, "ghost {ghost}");
    assert_eq!(
        proof.get_pixel(7, 7).0,
        [255, 255, 255],
        "paper stays paper"
    );
}

// ---------------------------------------------------------------------------
// End to end, through the real renderer
// ---------------------------------------------------------------------------

#[test]
fn a_delta_built_from_two_real_pages_carries_only_the_new_words() {
    let Ok(engine) = engine() else { return };
    let dir = tempfile::tempdir().unwrap();

    let before = a_pdf(dir.path(), "before.pdf", &[("PURCHASE ORDER 4471", 40.0)]);
    let after = a_pdf(
        dir.path(),
        "after.pdf",
        &[("PURCHASE ORDER 4471", 40.0), ("APPROVED", 150.0)],
    );

    let dpi = 150.0;
    let old = engine.open(&before).unwrap().render(0, dpi).unwrap();
    let new = engine.open(&after).unwrap().render(0, dpi).unwrap();

    let diff = diff_page(
        &old.gray,
        (old.width, old.height),
        &new.gray,
        (new.width, new.height),
        A4,
        dpi,
        0,
        &DiffOptions::default(),
    );

    // The heading is on both pages, so only the approval is new.
    assert!(diff.added_px > 0, "nothing was found to add");
    assert_eq!(diff.removed_px, 0, "nothing should have gone missing");
    let bounds = diff.bounds_mm().expect("no additions");
    assert!(
        bounds.1 > 140.0,
        "the delta reaches up to {:.1} mm, above the new line",
        bounds.1
    );

    // And the delta renders back to just that.
    let out = dir.path().join("delta.pdf");
    build_raster_delta(&[diff], &[Some(new.rgb.clone())], &out, "delta", None).unwrap();

    let rendered = engine.open(&out).unwrap().render(0, dpi).unwrap();
    let mut top = usize::MAX;
    for y in 0..rendered.height {
        for x in 0..rendered.width {
            if rendered.gray[y * rendered.width + x] < 128 {
                top = top.min(y);
            }
        }
    }
    assert!(top != usize::MAX, "the delta rendered blank");
    let top_mm = top as f64 * crate::geometry::MM_PER_INCH / dpi;
    assert!(
        top_mm > 140.0,
        "the delta has ink at {top_mm:.1} mm — it should only carry the new line"
    );
}

#[test]
fn a_delta_of_a_reflowed_page_reports_the_ink_that_went_missing() {
    let Ok(engine) = engine() else { return };
    let dir = tempfile::tempdir().unwrap();

    let before = a_pdf(dir.path(), "before.pdf", &[("Two hundred widgets", 100.0)]);
    // The same line, pushed down the page — a reflow.
    let after = a_pdf(dir.path(), "after.pdf", &[("Two hundred widgets", 110.0)]);

    let dpi = 150.0;
    let old = engine.open(&before).unwrap().render(0, dpi).unwrap();
    let new = engine.open(&after).unwrap().render(0, dpi).unwrap();

    let diff = diff_page(
        &old.gray,
        (old.width, old.height),
        &new.gray,
        (new.width, new.height),
        A4,
        dpi,
        0,
        &DiffOptions::default(),
    );
    assert!(
        diff.removed_px > 0,
        "a line that moved must leave ink behind where it was"
    );
    assert!(crate::safety::has_blockers(&crate::safety::check_reflow(
        &diff
    )));
}

// ---------------------------------------------------------------------------
// Marking what changed
// ---------------------------------------------------------------------------

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

#[test]
fn boxes_that_would_cross_each_other_become_one_box() {
    // Two words a few millimetres apart, padded until their boxes overlap.
    // Two crossing rectangles read as a mistake, and are harder to follow than
    // the single box somebody drawing this by hand would have drawn.
    let page = A4;
    let outline = Outline {
        pad_mm: 2.0,
        ..Default::default()
    };
    let boxes = outline.boxes(
        &[
            region(20.0, 20.0, 40.0, 24.0),
            region(41.0, 20.0, 60.0, 24.0),
        ],
        page,
    );
    assert_eq!(boxes.len(), 1, "{boxes:?}");
    assert!((boxes[0].x0_mm - 18.0).abs() < 1e-9, "{boxes:?}");
    assert!((boxes[0].x1_mm - 62.0).abs() < 1e-9, "{boxes:?}");

    // Far apart, they stay two.
    let boxes = outline.boxes(
        &[
            region(20.0, 20.0, 40.0, 24.0),
            region(90.0, 20.0, 110.0, 24.0),
        ],
        page,
    );
    assert_eq!(boxes.len(), 2, "{boxes:?}");
}

#[test]
fn a_box_never_runs_off_the_paper() {
    // A change hard against the edge of the page would otherwise be given a
    // box drawn partly off the sheet, which prints as three sides.
    let page = A4;
    let outline = Outline {
        pad_mm: 5.0,
        ..Default::default()
    };
    let boxes = outline.boxes(&[region(1.0, 1.0, 30.0, 8.0)], page);
    assert_eq!(boxes.len(), 1);
    assert!(boxes[0].x0_mm >= 0.0, "{boxes:?}");
    assert!(boxes[0].y0_mm >= 0.0, "{boxes:?}");
    assert!(boxes[0].x1_mm <= page.width_mm, "{boxes:?}");
}

#[test]
fn the_operators_stroke_a_box_and_leave_the_state_as_they_found_it() {
    // Appended to somebody else's content stream, so it has to save and
    // restore: a stray colour left set would tint everything drawn after it.
    let page = A4;
    let ops = Outline::default().ops(&[region(20.0, 20.0, 40.0, 24.0)], page);
    assert!(ops.contains(" q "), "{ops}");
    assert!(ops.trim_end().ends_with('Q'), "{ops}");
    assert!(ops.contains("RG"), "{ops}");
    assert!(ops.contains(" re"), "{ops}");
    assert!(ops.contains(" S "), "{ops}");
    // Nothing at all when there is nothing to mark, rather than an empty path.
    assert!(Outline::default().ops(&[], page).is_empty());
}

#[test]
fn the_delta_gains_ink_when_the_changes_are_outlined() {
    // The real check: the same delta, once plain and once outlined, and the
    // outlined one has to actually carry more on the page.
    let dir = tempfile::tempdir().unwrap();
    let (width, height) = (40usize, 56usize);
    let ink: Vec<(usize, usize)> = (10..30).map(|x| (x, 20)).collect();
    let mut diff = a_diff(&ink, width, height);
    diff.added_regions = vec![region(20.0, 30.0, 60.0, 34.0)];
    let rgb = vec![0u8; width * height * 3];

    let plain = dir.path().join("plain.pdf");
    let marked = dir.path().join("marked.pdf");
    build_raster_delta(&[diff.clone()], &[Some(rgb.clone())], &plain, "delta", None).unwrap();
    build_raster_delta(
        &[diff],
        &[Some(rgb)],
        &marked,
        "delta",
        Some(Outline::default()),
    )
    .unwrap();

    let plain_size = std::fs::metadata(&plain).unwrap().len();
    let marked_size = std::fs::metadata(&marked).unwrap().len();
    assert!(
        marked_size > plain_size,
        "outlined delta is not bigger: {marked_size} vs {plain_size}"
    );
}

/// An annotation on a page the edit changed must not travel with the delta.
///
/// An annotation is drawn from its own appearance stream, beside the page's
/// content, so the `W n` clip that holds the page's own ink to the changed
/// regions has no hold on it at all. `blank_page` strips them for exactly this
/// reason — and the pages that *did* gain something kept theirs.
///
/// A filled form field, a highlight, a signature, an approval stamp: all
/// routine on the kind of document somebody overprints. Every one of them was
/// laid down again at full size on a sheet that already had it, offset by the
/// printer's registration error. Toner does not come off paper.
#[test]
fn an_annotation_does_not_travel_with_a_vector_delta() {
    let dir = tempfile::tempdir().unwrap();
    let source = a_pdf(dir.path(), "edited.pdf", &[("Approved", 100.0)]);

    // A stamp of the kind that is already on the sheet.
    let mut doc = lopdf::Document::load(&source).unwrap();
    let page_id = doc.get_pages().into_values().next().unwrap();
    let stamp = doc.add_object(lopdf::dictionary! {
        "Type" => "Annot",
        "Subtype" => "Stamp",
        "Rect" => vec![20.into(), 20.into(), 200.into(), 80.into()],
        "P" => page_id,
    });
    doc.get_dictionary_mut(page_id)
        .unwrap()
        .set("Annots", vec![Object::Reference(stamp)]);
    let stamped = dir.path().join("stamped.pdf");
    doc.save(&stamped).unwrap();
    // The fixture really does carry one, or this test proves nothing.
    let before = lopdf::Document::load(&stamped).unwrap();
    let first = before.get_pages().into_values().next().unwrap();
    assert!(before.get_dictionary(first).unwrap().get(b"Annots").is_ok());

    let mut diff = a_diff(&[], 40, 56);
    diff.added_regions = vec![Region {
        x0_mm: 25.0,
        y0_mm: 95.0,
        x1_mm: 60.0,
        y1_mm: 101.0,
        ink_mm2: 20.0,
        px_bbox: (0, 0, 1, 1),
    }];

    let out = dir.path().join("vector.pdf");
    build_vector_delta(&[diff], &stamped, &out, 0.3, "test", None).unwrap();

    let pdf = lopdf::Document::load(&out).unwrap();
    let page_id = *pdf.get_pages().values().next().unwrap();
    assert!(
        pdf.get_dictionary(page_id).unwrap().get(b"Annots").is_err(),
        "the stamp came with the delta, and no clip can hold it back"
    );
    // The page's own clipped content is still there — this removes an
    // annotation, not the addition.
    let content = pdf.get_page_content(page_id).unwrap();
    let text = String::from_utf8_lossy(&content);
    assert!(text.contains("W n"), "the clip went too: {text}");
}
