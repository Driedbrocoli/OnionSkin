use super::*;
use crate::calibrate::A4;

/// A page at a given resolution with dark boxes on it, in millimetres.
fn page(dpi: f64, ink: &[(f64, f64, f64, f64)]) -> (Vec<u8>, usize) {
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

fn signature(dpi: f64, ink: &[(f64, f64, f64, f64)]) -> Signature {
    let (gray, width) = page(dpi, ink);
    Signature::of(&gray, width, dpi, A4, 128)
}

/// An invoice: a letterhead block, a table, and a total at the bottom.
const INVOICE: &[(f64, f64, f64, f64)] = &[
    (20.0, 20.0, 90.0, 35.0),
    (20.0, 60.0, 190.0, 140.0),
    (140.0, 200.0, 190.0, 210.0),
];

/// A letter: one block of text down the middle, and nothing else.
const LETTER: &[(f64, f64, f64, f64)] = &[(25.0, 50.0, 185.0, 180.0)];

/// The same document is the same document, however it was got at. A scan at
/// 150 dpi and a render at 400 must give the same answer, because the map is
/// measured against the paper and not against the pixels.
#[test]
fn the_same_page_at_two_resolutions_is_the_same_page() {
    let scanned = signature(150.0, INVOICE);
    let rendered = signature(400.0, INVOICE);
    let apart = scanned.distance(&rendered);
    assert!(
        apart < THE_SAME_DOCUMENT,
        "the same page at two resolutions came out {apart:.4} apart"
    );
}

/// And two different documents are far enough apart not to be confused.
///
/// These two are a hard case on purpose: the invoice's table and the letter's
/// text block cover much of the same paper, so a good part of their ink really
/// is in the same cells. Real pages of ordinary text separate far more widely —
/// an invoice against a letter measures better than 0.99 — but the check worth
/// having is that even two pages this alike land clear of "the same document".
#[test]
fn two_different_documents_are_far_enough_apart_not_to_be_confused() {
    let invoice = signature(150.0, INVOICE);
    let letter = signature(150.0, LETTER);
    let apart = invoice.distance(&letter);
    assert!(
        apart > THE_SAME_DOCUMENT,
        "two different pages came out {apart:.4} apart, inside the same-document mark"
    );
}

/// A word moving a millimetre is the same document. Paper shifts in a
/// scanner, and a map so fine that it notices would be no use at all.
#[test]
fn a_millimetre_of_movement_does_not_change_the_answer() {
    let there = signature(150.0, INVOICE);
    let moved: Vec<(f64, f64, f64, f64)> = INVOICE
        .iter()
        .map(|(x0, y0, x1, y1)| (x0 + 1.0, y0 + 1.0, x1 + 1.0, y1 + 1.0))
        .collect();
    let shifted = signature(150.0, &moved);
    let apart = there.distance(&shifted);
    assert!(
        apart < THE_SAME_DOCUMENT,
        "a millimetre of shift came out {apart:.4} apart"
    );
}

/// The sheet is picked out of a pile, and the answer is worth acting on.
#[test]
fn the_right_document_is_picked_out_of_a_pile() {
    let sheet = signature(150.0, INVOICE);
    let candidates: Vec<PathBuf> = ["letter.pdf", "invoice.pdf", "blank.pdf"]
        .iter()
        .map(PathBuf::from)
        .collect();

    let ranking = among(&sheet, &candidates, |path| {
        Ok(match path.to_string_lossy().as_ref() {
            "invoice.pdf" => signature(400.0, INVOICE),
            "letter.pdf" => signature(400.0, LETTER),
            _ => signature(400.0, &[]),
        })
    });

    assert!(ranking.confident(), "{}", ranking.describe());
    assert_eq!(
        ranking.best().unwrap().path,
        PathBuf::from("invoice.pdf"),
        "{}",
        ranking.describe()
    );
    assert!(ranking.describe().contains("This is invoice.pdf"));
}

/// Two candidates that look alike must not be reported as a confident answer.
/// Two months of the same invoice template is the ordinary case, and "it is
/// one of these two" is both the honest answer and the useful one.
#[test]
fn a_near_tie_is_reported_as_a_near_tie() {
    let sheet = signature(150.0, INVOICE);
    let candidates: Vec<PathBuf> = ["march.pdf", "april.pdf"]
        .iter()
        .map(PathBuf::from)
        .collect();

    // The same template, differing only in a small block near the bottom.
    let mut april = INVOICE.to_vec();
    april.push((30.0, 215.0, 45.0, 219.0));

    let ranking = among(&sheet, &candidates, |path| {
        Ok(match path.to_string_lossy().as_ref() {
            "march.pdf" => signature(400.0, INVOICE),
            _ => signature(400.0, &april),
        })
    });

    assert_eq!(ranking.best().unwrap().path, PathBuf::from("march.pdf"));
    assert!(
        !ranking.confident(),
        "two copies of one template were called a confident answer: {}",
        ranking.describe()
    );
    assert!(
        ranking.describe().contains("nearly as close"),
        "{}",
        ranking.describe()
    );
}

/// A pile with nothing like the sheet in it must say so, rather than crowning
/// whichever candidate happened to be least unlike.
#[test]
fn a_pile_with_nothing_like_it_says_so() {
    let sheet = signature(150.0, INVOICE);
    let candidates = vec![PathBuf::from("letter.pdf")];
    let ranking = among(&sheet, &candidates, |_| Ok(signature(400.0, LETTER)));

    assert!(!ranking.confident());
    assert!(
        ranking.describe().contains("None of these looks like"),
        "{}",
        ranking.describe()
    );
}

/// A blank sheet matches every other blank sheet perfectly, which is true and
/// useless. Naming a winner from it would be picking at random.
#[test]
fn a_blank_sheet_is_not_recognised_as_anything() {
    let sheet = signature(150.0, &[]);
    assert!(!sheet.worth_comparing());

    let candidates: Vec<PathBuf> = ["a.pdf", "b.pdf"].iter().map(PathBuf::from).collect();
    let ranking = among(&sheet, &candidates, |_| Ok(signature(400.0, &[])));
    assert!(!ranking.confident(), "a blank sheet was identified");
    assert!(
        ranking.describe().contains("almost no ink"),
        "{}",
        ranking.describe()
    );
}

/// A candidate that cannot be opened is reported, not quietly dropped — a
/// document missing from the running is one somebody goes on believing was
/// considered.
#[test]
fn a_document_that_cannot_be_opened_is_reported_rather_than_dropped() {
    let sheet = signature(150.0, INVOICE);
    let candidates: Vec<PathBuf> = ["broken.pdf", "invoice.pdf"]
        .iter()
        .map(PathBuf::from)
        .collect();

    let ranking = among(&sheet, &candidates, |path| {
        if path.to_string_lossy().contains("broken") {
            Err("it is not a PDF".to_string())
        } else {
            Ok(signature(400.0, INVOICE))
        }
    });

    assert_eq!(ranking.ranked.len(), 2, "a candidate went missing");
    // The readable one wins, and the broken one is last but still named.
    assert_eq!(ranking.best().unwrap().path, PathBuf::from("invoice.pdf"));
    assert_eq!(ranking.ranked[1].path, PathBuf::from("broken.pdf"));
    assert!(
        ranking.describe().contains("it is not a PDF"),
        "{}",
        ranking.describe()
    );
    // One unreadable candidate is not a runner-up, so it must not spoil the
    // confidence of a winner that has nothing real to compete with.
    assert!(ranking.confident(), "{}", ranking.describe());
}

/// Nothing offered at all is not a crash and not a match.
#[test]
fn an_empty_pile_names_nothing() {
    let sheet = signature(150.0, INVOICE);
    let ranking = among(&sheet, &[], |_| Ok(signature(400.0, INVOICE)));
    assert!(ranking.best().is_none());
    assert!(!ranking.confident());
    assert!(
        ranking.describe().contains("No documents were offered"),
        "an empty pile was reported as documents that would not open: {}",
        ranking.describe()
    );
}

/// Two maps of different sizes cannot be compared, and must say so with a
/// distance nothing can win rather than by panicking on a mismatched zip.
#[test]
fn maps_that_cannot_be_compared_are_infinitely_far_apart() {
    let real = signature(150.0, INVOICE);
    let empty = Signature {
        cells: Vec::new(),
        density: 0.0,
    };
    assert!(real.distance(&empty).is_infinite());
    assert!(empty.distance(&real).is_infinite());
}
