//! Tests for taking something out of a document.
//!
//! The one that matters is the first: a document with a salary in it, redacted,
//! and then asked whether the salary is still in it. Everything else is detail.

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

/// A document with real text in it, one page per group of lines.
fn a_document(dir: &Path, name: &str, per_page: &[&[(&str, f64)]]) -> PathBuf {
    let path = dir.join(name);
    let sizes = vec![A4; per_page.len()];
    let lines: Vec<Vec<PlacedLine>> = per_page
        .iter()
        .map(|page| page.iter().map(|(t, y)| line(t, *y)).collect())
        .collect();
    write_delta(&path, &sizes, &lines, "test", None).unwrap();
    path
}

/// Fast enough for a test, fine enough that the words would still be legible.
const QUICK: f64 = 100.0;

/// The whole point, stated as the question somebody would ask.
///
/// A black rectangle drawn over a salary in a PDF hides nothing: the number is
/// still in the file, and pressing Ctrl-A and Ctrl-C gets it back. This is the
/// test that the number is *gone* — asked of the written file, by the same
/// means anybody else would use to get it out.
#[test]
fn the_words_taken_out_are_not_in_the_file_afterwards() {
    let Ok(engine) = crate::render::engine() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let source = a_document(
        dir.path(),
        "offer.pdf",
        &[&[
            ("Dear Ms Okonkwo", 40.0),
            ("Salary: 84000 per annum", 60.0),
            ("Starting 1 September", 80.0),
        ]],
    );

    // The original really does carry the number, or this test proves nothing.
    let before = engine.open(&source).unwrap().text_on(0).unwrap();
    assert!(before.contains("84000"), "the fixture has no salary in it");

    let out = dir.path().join("to-hand-over.pdf");
    let done = redact(
        &source,
        &out,
        &[Area {
            page: 1,
            x_mm: 20.0,
            y_mm: 52.0,
            width_mm: 120.0,
            height_mm: 12.0,
        }],
        QUICK,
    )
    .expect("it should redact");

    let after = engine.open(&out).unwrap().text_on(0).unwrap();
    assert!(
        !after.contains("84000"),
        "the salary is still in the file: {after:?}"
    );
    // And not because the extractor stopped working — nothing at all is left,
    // which is what flattening means and what the check inside `redact` is.
    assert!(after.trim().is_empty(), "text survived: {after:?}");
    assert!(
        done.had_text,
        "it should have noticed the original had text"
    );
    assert_eq!(done.pages, 1);
    assert_eq!(done.areas, 1);
}

/// The page still looks like the page, with a black rectangle where the words
/// were. Taking the text out is no good if it takes the document with it.
#[test]
fn what_was_not_redacted_is_still_on_the_page() {
    let Ok(engine) = crate::render::engine() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let source = a_document(
        dir.path(),
        "offer.pdf",
        &[&[("Dear Ms Okonkwo", 40.0), ("Salary: 84000", 60.0)]],
    );
    let out = dir.path().join("out.pdf");
    redact(
        &source,
        &out,
        &[Area {
            page: 1,
            x_mm: 20.0,
            y_mm: 52.0,
            width_mm: 120.0,
            height_mm: 12.0,
        }],
        QUICK,
    )
    .unwrap();

    let drawn = engine
        .open(&out)
        .unwrap()
        .render_gray(0, QUICK)
        .expect("it should draw");
    let px_per_mm = QUICK / 25.4;
    let dark_in = |top_mm: f64, bottom_mm: f64| -> usize {
        let from = (top_mm * px_per_mm) as usize;
        let to = ((bottom_mm * px_per_mm) as usize).min(drawn.height);
        (from..to)
            .flat_map(|y| (0..drawn.width).map(move |x| (x, y)))
            .filter(|(x, y)| drawn.gray[y * drawn.width + x] < 128)
            .count()
    };

    // The salutation is untouched: some ink, nothing like a solid band.
    let salutation = dark_in(34.0, 44.0);
    assert!(salutation > 20, "the first line vanished");
    assert!(
        salutation < (drawn.width as f64 * 10.0 * px_per_mm) as usize / 2,
        "the first line was painted over"
    );

    // The redacted band is solid: near enough every pixel of a 120 mm wide
    // rectangle is black.
    let band = dark_in(54.0, 62.0);
    let expected = (120.0 * px_per_mm) * (8.0 * px_per_mm);
    assert!(
        band as f64 > expected * 0.8,
        "the band is {band} dark pixels and a solid one would be about {expected:.0}"
    );
}

/// A page nothing was taken from is flattened too, and that is the point.
///
/// It is the tempting thing to leave alone, and leaving it alone is how the
/// subtle version of this mistake gets made: a word can be in a file in more
/// places than the page it is drawn on, and something that only removes what
/// it recognises leaves whatever it did not. "There is no text in this
/// document" is a sentence that can be checked; "there is no text in the
/// places I looked" is not.
#[test]
fn every_page_is_flattened_not_only_the_redacted_one() {
    let Ok(engine) = crate::render::engine() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let source = a_document(
        dir.path(),
        "two.pdf",
        &[
            &[("Salary: 84000", 60.0)],
            &[("Also 84000, for reference", 60.0)],
        ],
    );
    let out = dir.path().join("out.pdf");
    redact(
        &source,
        &out,
        &[Area {
            page: 1,
            x_mm: 20.0,
            y_mm: 52.0,
            width_mm: 120.0,
            height_mm: 12.0,
        }],
        QUICK,
    )
    .unwrap();

    let written = engine.open(&out).unwrap();
    assert_eq!(written.len(), 2, "a page went missing");
    for index in 0..2 {
        let text = written.text_on(index).unwrap();
        assert!(
            text.trim().is_empty(),
            "page {} still has text: {text:?}",
            index + 1
        );
    }
}

/// Asked for nothing, it does nothing, and says what to ask for.
#[test]
fn nothing_asked_for_is_refused_with_the_two_ways_to_ask() {
    let dir = tempfile::tempdir().unwrap();
    let source = a_document(dir.path(), "one.pdf", &[&[("x", 40.0)]]);
    let out = dir.path().join("out.pdf");
    let why = redact(&source, &out, &[], QUICK).unwrap_err().to_string();
    assert!(why.contains("--word"), "{why}");
    assert!(why.contains("--over"), "{why}");
    assert!(!out.exists(), "it wrote a file anyway");
}

/// A page number nobody has is refused before anything is drawn, rather than
/// quietly redacting nothing — which would hand somebody a file they believe
/// is safe.
#[test]
fn a_page_the_document_does_not_have_is_refused() {
    let Ok(_) = crate::render::engine() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let source = a_document(dir.path(), "one.pdf", &[&[("Salary: 84000", 60.0)]]);
    let out = dir.path().join("out.pdf");
    let why = redact(
        &source,
        &out,
        &[Area {
            page: 4,
            x_mm: 20.0,
            y_mm: 52.0,
            width_mm: 120.0,
            height_mm: 12.0,
        }],
        QUICK,
    )
    .unwrap_err()
    .to_string();
    assert!(why.contains("page 4"), "{why}");
    assert!(why.contains("1 page"), "{why}");
    assert!(!out.exists(), "it wrote a file anyway");
}

/// A rectangle hanging over the edge of the paper is somebody measuring
/// generously round the thing they want gone. That is the safe mistake, and
/// refusing it would be refusing the safe mistake.
#[test]
fn an_area_larger_than_the_page_is_taken_as_the_whole_page() {
    let Ok(engine) = crate::render::engine() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let source = a_document(dir.path(), "one.pdf", &[&[("Salary: 84000", 60.0)]]);
    let out = dir.path().join("out.pdf");
    redact(
        &source,
        &out,
        &[Area {
            page: 1,
            x_mm: -50.0,
            y_mm: -50.0,
            width_mm: 400.0,
            height_mm: 500.0,
        }],
        QUICK,
    )
    .expect("it should cope");

    let drawn = engine.open(&out).unwrap().render_gray(0, QUICK).unwrap();
    let dark = drawn.gray.iter().filter(|value| **value < 128).count();
    assert_eq!(
        dark,
        drawn.gray.len(),
        "the whole page should be black, and {} of {} pixels are",
        dark,
        drawn.gray.len()
    );
}

/// A scan has no text to begin with, so "no text afterwards" says nothing
/// about whether anything worked — and the report says which case it was, so
/// nobody is told their searchable document was made unsearchable when it
/// never was searchable.
#[test]
fn a_document_that_never_had_text_is_reported_as_such() {
    let Ok(_) = crate::render::engine() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    // A page of shapes and no words.
    let source = a_document(dir.path(), "blank.pdf", &[&[]]);
    let out = dir.path().join("out.pdf");
    let done = redact(
        &source,
        &out,
        &[Area {
            page: 1,
            x_mm: 20.0,
            y_mm: 20.0,
            width_mm: 40.0,
            height_mm: 10.0,
        }],
        QUICK,
    )
    .unwrap();
    assert!(!done.had_text);
    let said = done.describe().join("\n");
    assert!(said.contains("no text left"), "{said}");
    assert!(
        !said.contains("no longer be searched"),
        "it warned about losing something that was never there: {said}"
    );
}

/// Nothing half-written is left where somebody might send it.
#[test]
fn the_working_file_does_not_survive_a_finished_run() {
    let Ok(_) = crate::render::engine() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let source = a_document(dir.path(), "one.pdf", &[&[("Salary: 84000", 60.0)]]);
    let out = dir.path().join("out.pdf");
    redact(
        &source,
        &out,
        &[Area {
            page: 1,
            x_mm: 20.0,
            y_mm: 52.0,
            width_mm: 120.0,
            height_mm: 12.0,
        }],
        QUICK,
    )
    .unwrap();
    assert!(out.is_file());
    assert!(
        !out.with_extension("onionskin-redacting").exists(),
        "the working file was left behind"
    );
}

/// The same claim, asked a completely different way.
///
/// Everything else here checks the redaction with pdfium, which is also what
/// `redact` checks itself with — so a fault in that one reader would leave
/// both the program and its test satisfied about a file that is not safe. The
/// test above guards against that by insisting the reader finds the salary in
/// the *original*, and this guards against it again by not using the reader at
/// all: it looks at the file's own structure.
///
/// A PDF cannot show text without a font resource to show it in and a `BT`
/// to begin a text object. A file with neither has nothing to extract however
/// it is read, by any program, now or later.
#[test]
fn the_written_file_has_no_font_and_no_text_object_in_it() {
    let Ok(_) = crate::render::engine() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let source = a_document(
        dir.path(),
        "offer.pdf",
        &[&[("Dear Ms Okonkwo", 40.0), ("Salary: 84000", 60.0)]],
    );

    // The original has both, so their absence below means something.
    let was = lopdf::Document::load(&source).unwrap();
    assert!(
        was.objects.values().any(is_a_font),
        "the fixture has no font in it"
    );

    let out = dir.path().join("out.pdf");
    redact(
        &source,
        &out,
        &[Area {
            page: 1,
            x_mm: 20.0,
            y_mm: 52.0,
            width_mm: 120.0,
            height_mm: 12.0,
        }],
        QUICK,
    )
    .unwrap();

    let now = lopdf::Document::load(&out).unwrap();
    assert!(
        !now.objects.values().any(is_a_font),
        "the redacted file still carries a font, so it can still show text"
    );
    // And the word `/Font` does not appear in the file at all, which is not
    // the same claim and is the one somebody auditing this will make. An empty
    // font dictionary shows no text and is a perfectly true thing to write —
    // and a person searching a redacted document for `/Font` and finding one
    // has no way to know that.
    let bytes = std::fs::read(&out).unwrap();
    assert!(
        !bytes.windows(5).any(|window| window == b"/Font"),
        "the redacted file mentions /Font, which anybody checking it will find"
    );
    // The words themselves are not in the bytes either, compressed or not.
    assert!(
        !bytes.windows(5).any(|window| window == b"84000"),
        "the salary is in the file's bytes"
    );
    for (page, id) in now.get_pages() {
        let content = now.get_page_content(id).unwrap();
        let operators = String::from_utf8_lossy(&content);
        assert!(
            !operators.contains("BT"),
            "page {page} still begins a text object"
        );
        assert!(
            !operators.contains("Tj") && !operators.contains("TJ"),
            "page {page} still shows text"
        );
    }
}

/// Whether an object is a font dictionary, however it is nested.
fn is_a_font(object: &lopdf::Object) -> bool {
    object
        .as_dict()
        .ok()
        .and_then(|dict| dict.get(b"Type").ok())
        .and_then(|kind| kind.as_name().ok())
        .map(|name| name == b"Font")
        .unwrap_or(false)
}
