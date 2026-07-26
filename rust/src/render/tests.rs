//! Tests for turning documents into PDFs and PDFs into pixels.

use super::*;
use crate::geometry::MM_PER_INCH;
use crate::pdf::{write_delta, Font, LineFont, PlacedLine};

const A4: PageSize = PageSize {
    width_mm: 210.0,
    height_mm: 297.0,
};

fn line(text: &str, y_mm: f64) -> PlacedLine {
    PlacedLine {
        text: text.to_string(),
        x_mm: 20.0,
        y_mm,
        size_pt: 14.0,
        font: LineFont::Builtin(Font::Helvetica),
        rotation_deg: 0.0,
        colour: (0.0, 0.0, 0.0),
    }
}

/// A PDF on disk with some words on it.
fn a_pdf(dir: &Path, name: &str, lines: &[(&str, f64)]) -> PathBuf {
    let path = dir.join(name);
    let placed: Vec<PlacedLine> = lines.iter().map(|(t, y)| line(t, *y)).collect();
    write_delta(&path, &[A4], &[placed], "test", None).unwrap();
    path
}

// ---------------------------------------------------------------------------
// Page frames
// ---------------------------------------------------------------------------

fn frame(media: (f64, f64, f64, f64), crop: (f64, f64, f64, f64), rotate: i64) -> PageFrame {
    PageFrame {
        media,
        crop,
        rotate,
    }
}

#[test]
fn a_plain_page_is_the_simple_case() {
    let plain = frame((0.0, 0.0, 595.28, 841.89), (0.0, 0.0, 595.28, 841.89), 0);
    assert!(plain.is_simple());
    assert_eq!(plain.describe(), "standard");

    let size = plain.display_size();
    assert!((size.width_mm - 210.0).abs() < 0.1);
    assert!((size.height_mm - 297.0).abs() < 0.1);
}

#[test]
fn a_quarter_turn_swaps_the_page_over() {
    // The page is stored portrait and displayed landscape. Ink lands on the
    // sheet the way it is *displayed*, so that is the size that matters.
    let turned = frame((0.0, 0.0, 595.28, 841.89), (0.0, 0.0, 595.28, 841.89), 90);
    let size = turned.display_size();

    assert!((size.width_mm - 297.0).abs() < 0.1);
    assert!((size.height_mm - 210.0).abs() < 0.1);
    assert!(!turned.is_simple());
    assert!(turned.describe().contains("rotated 90"));
}

#[test]
fn a_crop_box_shrinks_the_visible_page() {
    let cropped = frame((0.0, 0.0, 595.28, 841.89), (72.0, 72.0, 523.28, 769.89), 0);
    let size = cropped.display_size();

    assert!((size.width_mm - 159.1).abs() < 0.2, "{}", size.width_mm);
    assert!(!cropped.is_simple());
    assert!(cropped.describe().contains("origin at"));
    assert!(cropped.describe().contains("cropped"));
}

#[test]
fn a_media_box_with_an_offset_origin_is_noticed() {
    let offset = frame((10.0, 20.0, 605.0, 861.0), (10.0, 20.0, 605.0, 861.0), 0);
    assert!(
        !offset.is_simple(),
        "an offset origin is not the simple case"
    );
    assert!(offset.describe().contains("origin at"));
}

#[test]
fn a_written_pdf_reads_back_as_the_page_it_was_written_as() {
    let dir = tempfile::tempdir().unwrap();
    let path = a_pdf(dir.path(), "plain.pdf", &[("Approved", 100.0)]);

    let pdf = lopdf::Document::load(&path).unwrap();
    let frames = read_frames(&pdf).unwrap();

    assert_eq!(frames.len(), 1);
    assert!(frames[0].is_simple());
    let size = frames[0].display_size();
    assert!((size.width_mm - 210.0).abs() < 0.1);
}

#[test]
fn boxes_inherited_from_the_page_tree_are_found() {
    // MediaBox is inheritable, and a page that does not carry one takes it from
    // an ancestor. A reader that only looks at the page itself falls back to US
    // Letter, and every measurement after that is wrong.
    let dir = tempfile::tempdir().unwrap();
    let path = a_pdf(dir.path(), "inherit.pdf", &[("x", 50.0)]);
    let mut pdf = lopdf::Document::load(&path).unwrap();

    let page_id = *pdf.get_pages().values().next().unwrap();
    let media = pdf
        .get_dictionary(page_id)
        .unwrap()
        .get(b"MediaBox")
        .unwrap()
        .clone();
    // Move it up to the parent, exactly as a real producer might.
    let parent = pdf
        .get_dictionary(page_id)
        .unwrap()
        .get(b"Parent")
        .unwrap()
        .as_reference()
        .unwrap();
    pdf.get_dictionary_mut(page_id).unwrap().remove(b"MediaBox");
    pdf.get_dictionary_mut(parent)
        .unwrap()
        .set("MediaBox", media);

    let frames = read_frames(&pdf).unwrap();
    let size = frames[0].display_size();
    assert!(
        (size.width_mm - 210.0).abs() < 0.1,
        "read as {} mm wide — the inherited box was missed",
        size.width_mm
    );
}

#[test]
fn a_rotation_that_is_not_a_quarter_turn_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = a_pdf(dir.path(), "askew.pdf", &[("x", 50.0)]);
    let mut pdf = lopdf::Document::load(&path).unwrap();
    let page_id = *pdf.get_pages().values().next().unwrap();
    pdf.get_dictionary_mut(page_id)
        .unwrap()
        .set("Rotate", lopdf::Object::Integer(45));

    let err = read_frames(&pdf).unwrap_err().to_string();
    assert!(err.contains("multiple of 90"), "{err}");
}

#[test]
fn a_negative_rotation_is_read_the_way_a_reader_reads_it() {
    let dir = tempfile::tempdir().unwrap();
    let path = a_pdf(dir.path(), "back.pdf", &[("x", 50.0)]);
    let mut pdf = lopdf::Document::load(&path).unwrap();
    let page_id = *pdf.get_pages().values().next().unwrap();
    pdf.get_dictionary_mut(page_id)
        .unwrap()
        .set("Rotate", lopdf::Object::Integer(-90));

    let frames = read_frames(&pdf).unwrap();
    assert_eq!(frames[0].rotate, 270);
}

#[test]
fn a_crop_box_outside_the_media_box_falls_back_to_it() {
    // Nonsense in, something usable out: a crop box that does not meet the
    // media box would otherwise give a page of zero size.
    let outside = frame((0.0, 0.0, 595.0, 842.0), (700.0, 900.0, 800.0, 1000.0), 0);
    // read_frames does the clamping, so build it the way that does.
    let dir = tempfile::tempdir().unwrap();
    let path = a_pdf(dir.path(), "odd.pdf", &[("x", 50.0)]);
    let mut pdf = lopdf::Document::load(&path).unwrap();
    let page_id = *pdf.get_pages().values().next().unwrap();
    pdf.get_dictionary_mut(page_id).unwrap().set(
        "CropBox",
        lopdf::Object::Array(vec![
            700.0f32.into(),
            900.0f32.into(),
            800.0f32.into(),
            1000.0f32.into(),
        ]),
    );

    let frames = read_frames(&pdf).unwrap();
    assert_eq!(frames[0].crop, frames[0].media, "{outside:?}");
}

// ---------------------------------------------------------------------------
// Fitting
// ---------------------------------------------------------------------------

#[test]
fn an_image_already_the_right_size_is_left_alone() {
    let source = vec![7u8; 4 * 3];
    assert_eq!(fit(&source, (4, 3), (4, 3), 1, 255), source);
}

#[test]
fn an_image_a_pixel_too_big_is_cropped_not_squashed() {
    // 3x2, values 1..6
    let source: Vec<u8> = (1..=6).collect();
    let out = fit(&source, (3, 2), (2, 2), 1, 255);
    assert_eq!(out, vec![1, 2, 4, 5]);
}

#[test]
fn an_image_a_pixel_too_small_is_padded_with_paper() {
    let source: Vec<u8> = vec![1, 2, 3, 4];
    let out = fit(&source, (2, 2), (3, 3), 1, 255);
    assert_eq!(out, vec![1, 2, 255, 3, 4, 255, 255, 255, 255]);
}

#[test]
fn padding_works_for_colour_too() {
    let source: Vec<u8> = vec![1, 2, 3, 4, 5, 6];
    let out = fit(&source, (2, 1), (3, 1), 3, 255);
    assert_eq!(out, vec![1, 2, 3, 4, 5, 6, 255, 255, 255]);
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

#[test]
fn a_page_renders_to_the_size_the_page_actually_is() {
    let Ok(engine) = engine() else { return };
    let dir = tempfile::tempdir().unwrap();
    let path = a_pdf(dir.path(), "one.pdf", &[("Approved", 100.0)]);

    let document = engine.open(&path).unwrap();
    assert_eq!(document.len(), 1);

    let page = document.render(0, 150.0).unwrap();
    let (w, h) = A4.px_size(150.0);
    assert_eq!((page.width, page.height), (w as usize, h as usize));
    assert_eq!(page.gray.len(), page.width * page.height);
    assert_eq!(page.rgb.len(), page.width * page.height * 3);
}

#[test]
fn the_ink_on_a_rendered_page_is_where_it_was_put() {
    let Ok(engine) = engine() else { return };
    let dir = tempfile::tempdir().unwrap();
    // No descenders, so the ink ends exactly on the baseline.
    let path = a_pdf(dir.path(), "placed.pdf", &[("ACCEPTED", 100.0)]);

    let document = engine.open(&path).unwrap();
    let page = document.render(0, 200.0).unwrap();

    // Find the ink and check it sits on the baseline it was written to.
    let mut top = usize::MAX;
    let mut bottom = 0usize;
    let mut left = usize::MAX;
    for y in 0..page.height {
        for x in 0..page.width {
            if page.gray[y * page.width + x] < 128 {
                top = top.min(y);
                bottom = bottom.max(y);
                left = left.min(x);
            }
        }
    }
    assert!(bottom > 0, "the page rendered blank");

    let px_per_mm = 200.0 / MM_PER_INCH;
    assert!(
        ((bottom as f64 / px_per_mm) - 100.0).abs() < 0.6,
        "ink ends at {:.2} mm, written at 100",
        bottom as f64 / px_per_mm
    );
    assert!(
        ((left as f64 / px_per_mm) - 20.0).abs() < 0.6,
        "ink starts at {:.2} mm, written at 20",
        left as f64 / px_per_mm
    );
}

#[test]
fn a_blank_page_renders_blank() {
    let Ok(engine) = engine() else { return };
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("blank.pdf");
    write_delta(&path, &[A4], &[vec![]], "blank", None).unwrap();

    let document = engine.open(&path).unwrap();
    let page = document.render(0, 100.0).unwrap();
    assert!(
        page.gray.iter().all(|v| *v > 250),
        "a blank page has no ink"
    );
}

#[test]
fn two_renders_of_the_same_page_are_identical() {
    // The whole comparison rests on this: if one engine renders the same bytes
    // two different ways, every glyph reads as changed.
    let Ok(engine) = engine() else { return };
    let dir = tempfile::tempdir().unwrap();
    let path = a_pdf(dir.path(), "twice.pdf", &[("Approved 25 July", 100.0)]);

    let document = engine.open(&path).unwrap();
    let first = document.render(0, 150.0).unwrap();
    let second = document.render(0, 150.0).unwrap();
    assert_eq!(first.gray, second.gray);
}

#[test]
fn every_page_of_a_longer_document_renders() {
    let Ok(engine) = engine() else { return };
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("three.pdf");
    write_delta(
        &path,
        &[A4, A4, A4],
        &[
            vec![line("one", 50.0)],
            vec![line("two", 50.0)],
            vec![line("three", 50.0)],
        ],
        "three",
        None,
    )
    .unwrap();

    let document = engine.open(&path).unwrap();
    assert_eq!(document.len(), 3);
    for index in 0..3 {
        let page = document.render(index, 72.0).unwrap();
        assert!(page.gray.iter().any(|v| *v < 200), "page {index} is blank");
    }
}

#[test]
fn a_file_that_is_not_a_pdf_is_explained_rather_than_thrown() {
    let Ok(engine) = engine() else { return };
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notapdf.pdf");
    std::fs::write(&path, b"this is not a PDF at all").unwrap();

    let err = match engine.open(&path) {
        Err(e) => e.to_string(),
        Ok(_) => panic!("a file of prose opened as a PDF"),
    };
    assert!(err.contains("could not be opened as a PDF"), "{err}");
}

#[test]
fn an_empty_file_says_it_is_empty() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("empty.pdf");
    std::fs::write(&path, b"").unwrap();

    let message = unreadable(&path, "some parser complaint");
    assert!(message.contains("empty (0 bytes)"), "{message}");
}

#[test]
fn a_password_protected_file_says_what_to_do_about_it() {
    let message = unreadable(Path::new("/tmp/secret.pdf"), "file is encrypted");
    assert!(message.contains("password-protected"), "{message}");
    assert!(message.contains("unprotected copy"), "{message}");
}

// ---------------------------------------------------------------------------
// Conversion
// ---------------------------------------------------------------------------

#[test]
fn a_pdf_needs_no_conversion() {
    let dir = tempfile::tempdir().unwrap();
    let path = a_pdf(dir.path(), "already.pdf", &[("x", 50.0)]);
    let out = to_pdf(&path, dir.path(), 60).unwrap();
    assert_eq!(out, path, "a PDF should be passed straight through");
}

#[test]
fn a_file_type_nothing_can_convert_is_named() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("holiday.jpeg");
    std::fs::write(&path, b"not a document").unwrap();

    let err = to_pdf(&path, dir.path(), 60).unwrap_err().to_string();
    assert!(err.contains("unsupported file type '.jpeg'"), "{err}");
    assert!(
        err.contains(".docx"),
        "it should list what does work: {err}"
    );
}

#[test]
fn a_missing_file_is_reported_before_anything_is_launched() {
    let dir = tempfile::tempdir().unwrap();
    let err = to_pdf(&dir.path().join("nope.docx"), dir.path(), 60)
        .unwrap_err()
        .to_string();
    assert!(err.contains("no such file"), "{err}");
}

#[test]
fn a_word_document_converts_when_libreoffice_is_there() {
    if find_soffice().is_none() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    // The simplest thing LibreOffice will convert, and a real one.
    let path = dir.path().join("note.txt");
    std::fs::write(&path, "Purchase order 4471\nTwo hundred widgets\n").unwrap();

    let pdf = to_pdf(&path, dir.path(), 180).unwrap();
    assert!(pdf.is_file(), "no PDF came out");
    assert_ne!(pdf, path);

    let Ok(engine) = engine() else { return };
    let document = engine.open(&pdf).unwrap();
    assert!(document.len() >= 1);
    let page = document.render(0, 100.0).unwrap();
    assert!(
        page.gray.iter().any(|v| *v < 200),
        "the text did not render"
    );
}

#[test]
fn a_file_url_is_one_a_program_will_accept() {
    let dir = tempfile::tempdir().unwrap();
    let spaced = dir.path().join("two words");
    std::fs::create_dir_all(&spaced).unwrap();

    let url = file_url(&spaced);
    assert!(url.starts_with("file:///"), "{url}");
    assert!(!url.contains(' '), "a space must be encoded: {url}");
    assert!(url.contains("%20"), "{url}");
    assert!(!url.contains('\\'), "{url}");
}

// ---------------------------------------------------------------------------
// The workspace
// ---------------------------------------------------------------------------

#[test]
fn a_workspace_cleans_itself_up() {
    let path = {
        let workspace = Workspace::new(false).unwrap();
        std::fs::write(workspace.path.join("scratch"), b"working").unwrap();
        assert!(workspace.path.is_dir());
        workspace.path.clone()
    };
    assert!(!path.exists(), "the workspace outlived its scope");
}

#[test]
fn a_kept_workspace_stays() {
    let workspace = Workspace::new(true).unwrap();
    let path = workspace.path.clone();
    drop(workspace);
    assert!(path.is_dir());
    let _ = std::fs::remove_dir_all(&path);
}

#[test]
fn two_workspaces_do_not_collide() {
    let a = Workspace::new(false).unwrap();
    let b = Workspace::new(false).unwrap();
    assert_ne!(a.path, b.path);
}

#[cfg(unix)]
#[test]
fn working_files_are_not_readable_by_other_accounts() {
    use std::os::unix::fs::PermissionsExt;
    let workspace = Workspace::new(false).unwrap();
    let mode = std::fs::metadata(&workspace.path)
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o077, 0, "mode {:o}", mode & 0o777);
}
