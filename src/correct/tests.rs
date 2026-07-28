use super::*;
use crate::letters::{Letter, PageText, Rect, TextLine, Word};

/// A letter of a given size at a given place, already read.
fn letter(character: char, x_mm: f64, baseline_mm: f64, size_mm: f64) -> Letter {
    Letter::known(
        character,
        Rect {
            x_mm,
            // Letters sit above the baseline by about their cap height.
            y_mm: baseline_mm - size_mm,
            width_mm: size_mm * 0.6,
            height_mm: size_mm,
        },
        size_mm * size_mm * 0.4,
    )
}

/// A word, laid out left to right from a starting point.
fn word(text: &str, x_mm: f64, baseline_mm: f64, size_mm: f64) -> Word {
    let mut letters = Vec::new();
    let mut at = x_mm;
    for character in text.chars() {
        letters.push(letter(character, at, baseline_mm, size_mm));
        at += size_mm * 0.7;
    }
    let right = letters
        .iter()
        .map(|l| l.rect.right_mm())
        .fold(f64::NEG_INFINITY, f64::max);
    Word {
        rect: Rect {
            x_mm,
            y_mm: baseline_mm - size_mm,
            width_mm: right - x_mm,
            height_mm: size_mm,
        },
        letters,
    }
}

/// A line of words on one baseline.
fn line(words: &[(&str, f64)], baseline_mm: f64, size_mm: f64) -> TextLine {
    let built: Vec<Word> = words
        .iter()
        .map(|(text, x_mm)| word(text, *x_mm, baseline_mm, size_mm))
        .collect();
    let left = built
        .iter()
        .flat_map(|w| w.letters.iter())
        .map(|l| l.rect.x_mm)
        .fold(f64::INFINITY, f64::min);
    let right = built
        .iter()
        .flat_map(|w| w.letters.iter())
        .map(|l| l.rect.right_mm())
        .fold(f64::NEG_INFINITY, f64::max);
    TextLine {
        rect: Rect {
            x_mm: left,
            y_mm: baseline_mm - size_mm,
            width_mm: right - left,
            height_mm: size_mm,
        },
        baseline_mm,
        words: built,
    }
}

/// An invoice with a total on it, set at about eleven point.
fn an_invoice() -> PageText {
    PageText {
        lines: vec![
            line(&[("ACME", 20.0), ("LIMITED", 35.0)], 25.0, 3.0),
            line(&[("Total", 20.0), ("120.00", 45.0)], 200.0, 3.0),
        ],
        discarded: 0,
    }
}

fn mistake(was: &str, now: &str) -> Mistake {
    Mistake {
        was: was.to_string(),
        now: now.to_string(),
    }
}

/// The whole point: the old words are found, a box is put over them, and the
/// new ones go at the same place on the same baseline.
#[test]
fn the_old_words_are_covered_and_the_new_ones_take_their_place() {
    let page = an_invoice();
    let planned = plan(&page, None, &[mistake("120.00", "140.00")], None, None).unwrap();

    assert_eq!(planned.len(), 1);
    let fix = &planned[0];

    // The cover sits over where the old words were, with a little either side.
    assert!(
        fix.cover_mm.0 < 45.0 && fix.cover_mm.0 > 42.0,
        "the box does not start just left of the old words: {:?}",
        fix.cover_mm
    );
    assert!(
        fix.cover_mm.2 > 0.0 && fix.cover_mm.3 > 0.0,
        "{:?}",
        fix.cover_mm
    );

    // And the new words start where the old ones did, on the same baseline as
    // the label beside them — which is the whole reason to read the page.
    assert!((fix.x_mm - 45.0).abs() < 0.5, "{}", fix.x_mm);
    assert!((fix.baseline_mm - 200.0).abs() < 0.5, "{}", fix.baseline_mm);
    assert!(
        fix.line.contains("Total"),
        "the wrong line was found: {}",
        fix.line
    );
}

/// A phrase on the page twice is refused. Covering the wrong "Total" is
/// precisely what this is meant to prevent, and there is no undo at a printer.
#[test]
fn a_phrase_that_appears_twice_is_refused_rather_than_guessed_at() {
    let page = PageText {
        lines: vec![
            line(&[("Total", 20.0), ("120.00", 45.0)], 100.0, 3.0),
            line(&[("Total", 20.0), ("240.00", 45.0)], 200.0, 3.0),
        ],
        discarded: 0,
    };
    let refused = plan(&page, None, &[mistake("Total", "Sum")], None, None).unwrap_err();
    let said = refused.to_string();
    assert!(said.contains("2 times"), "{said}");
    assert!(
        said.contains("cannot be undone"),
        "the refusal does not say why it matters: {said}"
    );
}

/// Words that are not on the page are a refusal that points at how to look.
#[test]
fn words_that_are_not_there_are_refused_with_somewhere_to_look() {
    let refused = plan(
        &an_invoice(),
        None,
        &[mistake("Subtotal", "Sum")],
        None,
        None,
    )
    .unwrap_err();
    assert!(refused.to_string().contains("onionskin read"), "{refused}");
}

/// A correction with nothing to put there is a mistake in the asking.
#[test]
fn a_correction_with_no_replacement_is_refused() {
    let refused = plan(&an_invoice(), None, &[mistake("120.00", "  ")], None, None).unwrap_err();
    assert!(refused.to_string().contains("what to put"), "{refused}");
}

/// The size comes off the line the mistake is on, not off the page.
///
/// The reader fits one size through the whole page, which is the right answer
/// to a different question. A heading at fourteen point and a body at twelve
/// average to something that is neither, and a correction set at the average
/// is visibly wrong beside the words it replaces.
#[test]
fn the_size_is_measured_from_the_line_not_from_the_whole_page() {
    // The page's own answer is 30 pt, and it is wrong for both these lines.
    let face = crate::typeface::Typeface {
        font: crate::pdf::Font::TimesRoman,
        size_pt: 30.0,
        confidence: 0.8,
        words_measured: 12,
    };
    let page = PageText {
        lines: vec![
            // A heading: 5 mm of cap height, so about 20 point.
            line(&[("HEADING", 20.0)], 25.0, 5.0),
            // And a body line: 3 mm, so about 12.
            line(&[("Total", 20.0), ("120.00", 45.0)], 200.0, 3.0),
        ],
        discarded: 0,
    };

    let body = plan(
        &page,
        Some(&face),
        &[mistake("120.00", "140.00")],
        None,
        None,
    )
    .unwrap();
    assert!(
        (body[0].size_pt - 12.1).abs() < 1.0,
        "a 3 mm line came out at {} pt",
        body[0].size_pt
    );

    let heading = plan(
        &page,
        Some(&face),
        &[mistake("HEADING", "TITLE")],
        None,
        None,
    )
    .unwrap();
    assert!(
        (heading[0].size_pt - 20.2).abs() < 1.5,
        "a 5 mm line came out at {} pt",
        heading[0].size_pt
    );
    assert!(
        heading[0].size_pt > body[0].size_pt + 4.0,
        "two lines of different sizes were given the same one"
    );

    // The face still comes from the page, which is the one thing a whole page
    // answers better than one line.
    assert_eq!(body[0].font, "Times-Roman");
    assert!(body[0].size_measured, "a measurement was called a guess");
}

/// Three millimetres of cap height is about twelve point, in every face there
/// is — which is what makes measuring off the line work at all.
#[test]
fn cap_height_is_about_seven_tenths_of_the_type_size() {
    let planned = plan(
        &an_invoice(),
        None,
        &[mistake("120.00", "140.00")],
        None,
        None,
    )
    .unwrap();
    assert!(
        (planned[0].size_pt - 12.1).abs() < 1.5,
        "3 mm of cap height came out as {} pt",
        planned[0].size_pt
    );
    assert!(planned[0].size_measured);
}

/// A line with nothing read on it has nothing to measure, so the page's own
/// answer is better than nothing — and it is reported as the guess it is.
#[test]
fn a_line_with_nothing_read_on_it_falls_back_and_says_so() {
    // Words with no letters in them: ink that was found and never read.
    let page = PageText {
        lines: vec![TextLine {
            rect: Rect {
                x_mm: 20.0,
                y_mm: 197.0,
                width_mm: 60.0,
                height_mm: 3.0,
            },
            baseline_mm: 200.0,
            words: vec![Word {
                rect: Rect {
                    x_mm: 20.0,
                    y_mm: 197.0,
                    width_mm: 60.0,
                    height_mm: 3.0,
                },
                letters: Vec::new(),
            }],
        }],
        discarded: 0,
    };
    let face = crate::typeface::Typeface {
        font: crate::pdf::Font::Helvetica,
        size_pt: 13.0,
        confidence: 0.4,
        words_measured: 3,
    };
    // Nothing readable, so nothing to find either — which is its own refusal.
    assert!(plan(&page, Some(&face), &[mistake("Total", "Sum")], None, None).is_err());
}

/// Somebody overruling the reader is taken at their word, because the escape
/// hatch is the whole reason to have one.
#[test]
fn what_somebody_asks_for_beats_what_the_page_says() {
    let face = crate::typeface::Typeface {
        font: crate::pdf::Font::TimesRoman,
        size_pt: 11.0,
        confidence: 0.9,
        words_measured: 20,
    };
    let planned = plan(
        &an_invoice(),
        Some(&face),
        &[mistake("120.00", "140.00")],
        Some(14.0),
        Some("Courier"),
    )
    .unwrap();
    assert_eq!(planned[0].size_pt, 14.0);
    assert_eq!(planned[0].font, "Courier");
}

/// Replacing six characters with nine will run into whatever is to the right.
/// Not a refusal — an invoice going from 120.00 to 1,120.00 is an ordinary
/// correction — but worth knowing before it is printed rather than after.
#[test]
fn words_that_will_not_fit_where_the_old_ones_were_are_noticed() {
    let planned = plan(
        &an_invoice(),
        None,
        &[mistake("120.00", "1,120,000.00")],
        Some(11.0),
        None,
    )
    .unwrap();
    assert!(
        planned[0].wider_than_what_it_replaces().is_some(),
        "a replacement twice as long was not noticed"
    );

    let same = plan(
        &an_invoice(),
        None,
        &[mistake("120.00", "140.00")],
        Some(11.0),
        None,
    )
    .unwrap();
    assert!(
        same[0].wider_than_what_it_replaces().is_none(),
        "a replacement of the same length was called too wide"
    );
}

/// Several corrections on one sheet, because one pass through the printer is
/// the point of the program.
#[test]
fn several_corrections_are_planned_together() {
    let page = PageText {
        lines: vec![
            line(&[("Name", 20.0), ("Smith", 45.0)], 100.0, 3.0),
            line(&[("Total", 20.0), ("120.00", 45.0)], 200.0, 3.0),
        ],
        discarded: 0,
    };
    let planned = plan(
        &page,
        None,
        &[mistake("Smith", "Smyth"), mistake("120.00", "140.00")],
        None,
        None,
    )
    .unwrap();
    assert_eq!(planned.len(), 2);
    assert!((planned[0].baseline_mm - 100.0).abs() < 0.5);
    assert!((planned[1].baseline_mm - 200.0).abs() < 0.5);
}

#[test]
fn a_correction_is_written_as_was_then_now() {
    let parsed = parse_mistake("120.00:140.00").unwrap();
    assert_eq!(parsed.was, "120.00");
    assert_eq!(parsed.now, "140.00");

    // A colon in the replacement is left alone, which matters because
    // "Total:120.00" is exactly the sort of thing being corrected.
    let kept = parse_mistake("Total:Total: 140.00").unwrap();
    assert_eq!(kept.was, "Total");
    assert_eq!(kept.now, "Total: 140.00");
}

#[test]
fn a_correction_missing_either_half_is_refused() {
    for bad in ["120.00", "120.00:", ":140.00", ":"] {
        assert!(
            parse_mistake(bad).is_err(),
            "'{bad}' should not have been accepted"
        );
    }
}
