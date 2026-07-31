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

/// Fine enough that the letter reader has a chance of reading the page back.
/// Below about 150 it starts failing on the original too, which would make a
/// "nothing can be read" result mean nothing at all.
const READ_BACK: f64 = 150.0;

/// A rectangle of a drawn page, in pixels.
struct Patch {
    left: usize,
    right: usize,
    top: usize,
    bottom: usize,
}

impl Patch {
    fn pixels(&self) -> usize {
        (self.right + 1 - self.left) * (self.bottom + 1 - self.top)
    }

    /// How many pixels of this rectangle are dark on some other drawing of the
    /// same page at the same size.
    fn dark_in(&self, drawn: &crate::render::GrayPage) -> usize {
        (self.top..=self.bottom.min(drawn.height.saturating_sub(1)))
            .flat_map(|y| {
                (self.left..=self.right.min(drawn.width.saturating_sub(1))).map(move |x| (x, y))
            })
            .filter(|(x, y)| drawn.gray[y * drawn.width + x] < 128)
            .count()
    }
}

/// The rectangle actually covered by ink, within a band of the page.
///
/// Measured off the drawing rather than read out of the file, so that a test
/// which asks "is this covered" is not asking the same code that decided where
/// to cover. Returns nothing if the band is blank.
fn ink_between(
    drawn: &crate::render::GrayPage,
    top_mm: f64,
    bottom_mm: f64,
    dpi: f64,
) -> Option<Patch> {
    let px_per_mm = dpi / 25.4;
    let from = ((top_mm * px_per_mm) as usize).min(drawn.height);
    let to = ((bottom_mm * px_per_mm) as usize).min(drawn.height);
    let (mut left, mut right) = (usize::MAX, 0usize);
    let (mut top, mut bottom) = (usize::MAX, 0usize);
    for y in from..to {
        for x in 0..drawn.width {
            if drawn.gray[y * drawn.width + x] < 128 {
                left = left.min(x);
                right = right.max(x);
                top = top.min(y);
                bottom = bottom.max(y);
            }
        }
    }
    if left == usize::MAX {
        return None;
    }
    Some(Patch {
        left,
        right,
        top,
        bottom,
    })
}

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
    // The claim that makes this feature worth anything is "there is no text
    // left in the file, and that was checked". On a scan the check passes
    // without having tested anything, because there was nothing to find — so
    // making the claim here would be citing a test that never ran.
    assert!(
        !said.contains("no text left"),
        "it claimed a check that proved nothing on a document with no text in it: {said}"
    );
    assert!(
        said.contains("already a picture"),
        "it did not say which case this was: {said}"
    );
    assert!(
        !said.contains("no longer be searched"),
        "it warned about losing something that was never there: {said}"
    );
}

/// A rectangle that lands nowhere near the page takes nothing out, and the
/// file that would come of it is the document unchanged with a sentence
/// attached saying the words are gone.
///
/// This is the shape of every serious failure in this module: not a crash, not
/// a wrong picture, but a true-looking report over an untouched secret.
#[test]
fn a_rectangle_that_covers_nothing_is_refused_rather_than_reported_as_done() {
    let Ok(_) = crate::render::engine() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let source = a_document(dir.path(), "offer.pdf", &[&[("Salary: 84000", 60.0)]]);
    let out = dir.path().join("out.pdf");

    // Nine hundred millimetres across an A4 page: a slipped decimal point, or
    // inches typed where millimetres were asked for.
    let off = redact(
        &source,
        &out,
        &[Area {
            page: 1,
            x_mm: 900.0,
            y_mm: 900.0,
            width_mm: 20.0,
            height_mm: 10.0,
        }],
        QUICK,
    );
    assert!(
        matches!(off, Err(RedactError::PaintedNothing { .. })),
        "an area entirely off the page was accepted: {off:?}"
    );
    assert!(
        !out.exists(),
        "a file was written for a redaction that did nothing"
    );

    // A rectangle with no size in it does the same nothing, by a different
    // route — `--over '40,100:0x8'`, or a drag that never moved.
    let flat = redact(
        &source,
        &out,
        &[Area {
            page: 1,
            x_mm: 40.0,
            y_mm: 100.0,
            width_mm: 0.0,
            height_mm: 8.0,
        }],
        QUICK,
    );
    assert!(
        matches!(flat, Err(RedactError::PaintedNothing { .. })),
        "a rectangle with no width was accepted: {flat:?}"
    );

    // One good rectangle does not excuse a bad one. Somebody who gave three
    // and had two land has a document with a secret still in it, and a count
    // of three to reassure them.
    let mixed = redact(
        &source,
        &out,
        &[
            Area {
                page: 1,
                x_mm: 20.0,
                y_mm: 52.0,
                width_mm: 120.0,
                height_mm: 12.0,
            },
            Area {
                page: 1,
                x_mm: 900.0,
                y_mm: 900.0,
                width_mm: 20.0,
                height_mm: 10.0,
            },
        ],
        QUICK,
    );
    assert!(
        matches!(mixed, Err(RedactError::PaintedNothing { .. })),
        "one rectangle landing excused another that did not: {mixed:?}"
    );
    assert!(!out.exists());
}

/// `--dpi 0` used to draw every page as a single pixel, stretched across the
/// sheet, and then report a successful redaction — truthfully, in that no text
/// was left in the file, because nothing was left in the file. The document was
/// destroyed and the program said the words were gone.
#[test]
fn a_resolution_no_page_can_be_drawn_at_is_refused() {
    let Ok(_) = crate::render::engine() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let source = a_document(dir.path(), "offer.pdf", &[&[("Salary: 84000", 60.0)]]);
    let out = dir.path().join("out.pdf");
    let area = [Area {
        page: 1,
        x_mm: 20.0,
        y_mm: 52.0,
        width_mm: 120.0,
        height_mm: 12.0,
    }];

    for dpi in [0.0, -300.0, 1e-9, f64::NAN, f64::INFINITY, 100_000.0] {
        let refused = redact(&source, &out, &area, dpi);
        assert!(
            matches!(refused, Err(RedactError::BadResolution { .. })),
            "{dpi} was accepted as a resolution: {refused:?}"
        );
        assert!(!out.exists(), "{dpi} wrote a file");
    }

    // And the ordinary ones still work, or the guard has simply broken the
    // feature instead of fixing it.
    for dpi in [MIN_DPI, 150.0, DEFAULT_DPI] {
        let done = redact(&source, &out, &area, dpi);
        assert!(done.is_ok(), "{dpi} was refused: {done:?}");
        std::fs::remove_file(&out).unwrap();
    }
}

/// The half-finished copy goes to a name that can be worked out from the name
/// somebody typed, so on a shared directory it is a place another person can
/// get to first. Writing through whatever is sitting there is how a program
/// with no privileges of its own does somebody else's overwriting for them.
#[test]
fn something_already_at_the_working_name_is_refused_rather_than_written_through() {
    let Ok(_) = crate::render::engine() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let source = a_document(dir.path(), "offer.pdf", &[&[("Salary: 84000", 60.0)]]);
    let out = dir.path().join("out.pdf");
    let working = out.with_extension("onionskin-redacting");
    let precious = dir.path().join("somebody-elses-notes.txt");
    std::fs::write(&precious, b"do not lose this").unwrap();

    #[cfg(unix)]
    std::os::unix::fs::symlink(&precious, &working).unwrap();
    #[cfg(not(unix))]
    std::fs::write(&working, b"in the way").unwrap();

    let refused = redact(
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
    );
    assert!(
        matches!(refused, Err(RedactError::WorkingFileInTheWay { .. })),
        "it wrote to a path something was already at: {refused:?}"
    );
    assert_eq!(
        std::fs::read(&precious).unwrap(),
        b"do not lose this",
        "the file the working name pointed at was written over"
    );
}

/// A hundred-page report at 300 dpi is two and a half gigabytes of raw pixels
/// held at once. Being killed part way through is not a failure anybody can act
/// on; a sentence naming a resolution that would work is.
#[test]
fn a_document_too_big_to_draw_all_at_once_says_so_before_trying() {
    let a4 = vec![A4; 200];
    assert!(weigh(&a4, 100.0).is_ok(), "200 pages at 100 dpi should fit");

    let Err(RedactError::TooMuchAtOnce {
        gigabytes,
        suggestion,
        then,
        ..
    }) = weigh(&a4, DEFAULT_DPI)
    else {
        panic!("200 A4 pages at 300 dpi was not noticed as too much")
    };
    assert!(
        gigabytes > 4.0,
        "it thinks 200 pages at 300 dpi is {gigabytes} GB"
    );
    // The advice has to be advice: a resolution that is lower, still legible,
    // and actually fits.
    assert!((MIN_DPI..DEFAULT_DPI).contains(&suggestion), "{suggestion}");
    assert!(then < 1.6, "the resolution it suggested needs {then} GB");
    assert!(
        weigh(&a4, suggestion).is_ok(),
        "it suggested {suggestion} dpi, which it would itself refuse"
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

/// The test that should have been written first.
///
/// Everything else here asks whether the file has a *text object* in it. That
/// is necessary and it is nowhere near sufficient, and believing otherwise is
/// how the first version of this shipped: it flattened the document perfectly,
/// proved there was no extractable text three different ways, and left the
/// salary sitting on the page in plain sight because the black bar had been
/// put over the word "Salary" and not over the figure. Every test passed.
///
/// So this one asks the only question that matters — can the secret still be
/// read off the page? — and asks it with the letter reader this program
/// already carries, which is the same thing anybody else's OCR would do.
#[test]
fn the_secret_cannot_be_read_off_the_redacted_page() {
    let Ok(_) = crate::render::engine() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let source = a_document(
        dir.path(),
        "offer.pdf",
        &[
            &[("Dear Ms Okonkwo", 40.0), ("Salary: 84000 per annum", 60.0)],
            &[("Salary: 84000 again on page two", 60.0)],
        ],
    );

    let found = lines_carrying(&source, &["Salary".to_string()], 1.0).expect("it should search");
    assert!(
        found.missing.is_empty(),
        "the phrase was not found at all: {found:?}"
    );
    // Every page, which is the first thing the old version got wrong.
    assert_eq!(
        found.areas.len(),
        2,
        "only {} of 2 pages were marked: {:?}",
        found.areas.len(),
        found.covered
    );
    // And the whole line, which is the second — the figure is what a person
    // means by "the salary", not the label.
    for gone in &found.covered {
        assert!(
            gone.line.contains("84000"),
            "the line covered is {:?}, which does not include the figure",
            gone.line
        );
    }

    // Where that line's ink actually sits on each original page, measured off
    // the drawn page. Deliberately not taken from the text layer: the search
    // above reads the text layer too, and a test whose oracle is the thing
    // under test agrees with it whether it is right or wrong. Ink on paper is
    // the one measurement here that cannot collude.
    let engine = crate::render::engine().unwrap();
    let opened = engine.open(&source).unwrap();
    let before: Vec<Patch> = (0..2)
        .map(|index| {
            let drawn = opened.render_gray(index, READ_BACK).unwrap();
            let patch = ink_between(&drawn, 50.0, 70.0, READ_BACK)
                .expect("the fixture should have a salary line on every page");
            // A safeguard on the safeguard: this must be a line of type, not a
            // speck. Six letters of `Salary` at 14 pt is about 20 mm, and the
            // whole line is nearer 60 — so anything under 40 mm means the ink
            // measurement itself has gone wrong, and every assertion below it
            // would be measuring the wrong thing quietly.
            let width_mm = (patch.right + 1 - patch.left) as f64 * 25.4 / READ_BACK;
            assert!(
                width_mm > 40.0,
                "page {}: the salary line measures {width_mm:.1} mm of ink, which is \
                 not a line of type — the fixture or the reader has changed",
                index + 1
            );
            patch
        })
        .collect();

    let out = dir.path().join("to-send.pdf");
    redact(&source, &out, &found.areas, READ_BACK).expect("it should redact");

    // The bar covers all of that ink, not the six letters of the label. This is
    // the check the reported line text cannot make: `covered.line` says what
    // was *matched*, and the version of this that shipped matched the whole
    // line and then painted a rectangle the width of the word.
    let after = engine.open(&out).unwrap();
    for (index, patch) in before.iter().enumerate() {
        let drawn = after.render_gray(index, READ_BACK).unwrap();
        let dark = patch.dark_in(&drawn);
        let whole = patch.pixels() as f64;
        assert!(
            dark as f64 > whole * 0.95,
            "page {}: the line's ink covers pixels {}..{} across and {}..{} down, and \
             only {dark} of {whole:.0} of them came back black — the rest of the line \
             is still on the page",
            index + 1,
            patch.left,
            patch.right,
            patch.top,
            patch.bottom
        );
    }

    // Now read the result back the way anybody would.
    for page in 1..=2 {
        let Ok((gray, registration)) = crate::recipe::draw_page(&out, page) else {
            panic!("page {page} of the redacted file would not draw")
        };
        let Some((text, _)) = crate::typeface::read_and_match_in(&gray, &registration) else {
            // No font on this machine to read against; the check cannot be
            // made, and passing silently would be the same mistake again.
            return;
        };
        let readable: String = text
            .lines
            .iter()
            .flat_map(|line| line.words.iter().map(|word| word.text_lossy()))
            .collect::<Vec<_>>()
            .join(" ");
        assert!(
            !readable.contains("84000"),
            "page {page} of the redacted document still reads: {readable:?}"
        );
        assert!(
            !readable.to_lowercase().contains("salary"),
            "page {page} of the redacted document still reads: {readable:?}"
        );
    }
}

/// A document with no text layer cannot be searched, and saying "nothing
/// matched" would be a lie that ends in a document handed over unredacted.
#[test]
fn a_scan_says_it_has_no_words_to_search_rather_than_finding_none() {
    let Ok(_) = crate::render::engine() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let source = a_document(dir.path(), "words.pdf", &[&[("Salary: 84000", 60.0)]]);
    // Flattening it is exactly what makes a scan: a picture of a page.
    let scanned = dir.path().join("scanned.pdf");
    redact(
        &source,
        &scanned,
        &[Area {
            page: 1,
            x_mm: 0.0,
            y_mm: 0.0,
            width_mm: 1.0,
            height_mm: 1.0,
        }],
        100.0,
    )
    .unwrap();

    let found = lines_carrying(&scanned, &["Salary".to_string()], 1.0).unwrap();
    assert!(
        found.from_a_scan,
        "a picture of a page was searched as text"
    );
    assert!(found.areas.is_empty());
}

/// A phrase that is nowhere is not "nothing to do" — it is a document about to
/// be handed over with the thing still in it.
#[test]
fn a_phrase_that_appears_nowhere_is_reported_rather_than_ignored() {
    let Ok(_) = crate::render::engine() else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let source = a_document(dir.path(), "one.pdf", &[&[("Salary: 84000", 60.0)]]);
    let found =
        lines_carrying(&source, &["Salary".to_string(), "Pension".to_string()], 1.0).unwrap();
    assert_eq!(found.missing, vec!["Pension".to_string()]);
    // The one that was found is still found — a missing phrase does not
    // silently discard the rest of the search.
    assert_eq!(found.areas.len(), 1);
}
