use super::*;

fn rect(x: f64, y: f64, w: f64, h: f64) -> Rect {
    Rect {
        x_mm: x,
        y_mm: y,
        width_mm: w,
        height_mm: h,
    }
}

/// A row of words laid out left to right on one baseline.
///
/// Two millimetres a letter and two between words, which is about eleven
/// point — near enough that the numbers in these tests can be worked out on
/// paper, which is the point of a fixture.
fn row(baseline: f64, start: f64, words: &[&str]) -> Row {
    let mut x = start;
    let mut out = Vec::new();
    for word in words {
        let width = word.chars().count() as f64 * 2.0;
        out.push((word.to_string(), rect(x, baseline - 3.0, width, 3.0), 3.0));
        x += width + 2.0;
    }
    Row {
        baseline_mm: baseline,
        words: out,
    }
}

#[test]
fn words_go_after_the_anchor_on_the_same_line() {
    // The whole point of the feature: nobody knows the gap after "Received:"
    // starts 39.5 mm across. They know it is the gap after "Received:".
    let rows = vec![row(40.0, 20.0, &["Received:", "____________"])];
    let placed = place_in(&rows, "Received:", Where::After, 1.5, 5.0).unwrap();
    // Nine characters from 20 mm ends at 38 mm, plus the gap.
    assert!((placed.x_mm - 39.5).abs() < 1e-9, "{placed:?}");
    assert!((placed.y_mm - 40.0).abs() < 1e-9, "{placed:?}");
}

#[test]
fn words_go_below_the_anchor_when_asked() {
    let rows = vec![row(40.0, 20.0, &["Signature"])];
    let placed = place_in(&rows, "Signature", Where::Below, 1.5, 5.0).unwrap();
    assert!((placed.x_mm - 20.0).abs() < 1e-9, "{placed:?}");
    assert!((placed.y_mm - 45.0).abs() < 1e-9, "{placed:?}");

    // And below-end starts where the anchor finished, for the second line of
    // a box whose label sits on the first.
    let placed = place_in(&rows, "Signature", Where::BelowEnd, 1.5, 5.0).unwrap();
    assert!((placed.x_mm - 38.0).abs() < 1e-9, "{placed:?}");
    assert!((placed.y_mm - 45.0).abs() < 1e-9, "{placed:?}");
}

#[test]
fn an_anchor_spanning_several_words_is_found() {
    // "Date of birth" is three words to the reader and one anchor to a person.
    let rows = vec![row(40.0, 20.0, &["Date", "of", "birth", "____"])];
    let placed = place_in(&rows, "Date of birth", Where::After, 1.0, 5.0).unwrap();
    // 20 + 8 + 2 + 4 + 2 + 10 = 46 mm, plus the gap.
    assert!((placed.x_mm - 47.0).abs() < 1e-9, "{placed:?}");
}

#[test]
fn case_spacing_and_punctuation_are_all_forgiven() {
    // A scan that read "Received:" as "RECEIVED;" should still match. The
    // colon is not what anybody meant by the anchor, and refusing over one
    // would be pedantry paid for in sheets of paper.
    let rows = vec![row(40.0, 20.0, &["RECEIVED;"])];
    for asked in ["received", "Received:", "  RECEIVED  ", "re ce ived"] {
        assert!(
            place_in(&rows, asked, Where::After, 1.0, 5.0).is_ok(),
            "{asked:?} did not match"
        );
    }
}

#[test]
fn an_anchor_that_is_not_there_says_what_is() {
    // "Not found" is no help at all to somebody looking at a page they cannot
    // see the program's version of. What is actually on it is.
    let rows = vec![
        row(40.0, 20.0, &["Received:"]),
        row(50.0, 20.0, &["Approved", "by:"]),
    ];
    let said = place_in(&rows, "Telephone number", Where::After, 1.0, 5.0)
        .unwrap_err()
        .to_string();
    assert!(said.contains("Telephone number"), "{said}");
    assert!(said.contains("Received:"), "{said}");
}

#[test]
fn a_few_letters_read_wrong_are_forgiven() {
    // A scan is never read perfectly. "Received:" comes back as "Peoeived:"
    // on a poor one — R and P differ by very little once a fax has had them —
    // and refusing over that sends somebody back to the ruler, which is the
    // thing this exists to avoid.
    let rows = vec![row(40.0, 20.0, &["Peoeived:"])];
    let placed = place_in(&rows, "Received:", Where::After, 1.0, 5.0);
    assert!(placed.is_ok(), "{placed:?}");

    // A letter missing entirely, and a letter too many, both within budget.
    for scanned in ["Receved:", "Receiived:"] {
        let rows = vec![row(40.0, 20.0, &[scanned])];
        assert!(
            place_in(&rows, "Received:", Where::After, 1.0, 5.0).is_ok(),
            "{scanned} was not forgiven"
        );
    }
}

#[test]
fn a_short_anchor_is_not_forgiven_anything() {
    // "Date" and "Rate" are both four letters and both plausible labels on
    // the same form. Forgiving one wrong letter there would be a coin toss
    // dressed up as helpfulness.
    let rows = vec![row(40.0, 20.0, &["Rate"])];
    assert!(place_in(&rows, "Date", Where::After, 1.0, 5.0).is_err());
    assert_eq!(slack("date"), 0);
    assert_eq!(slack("dates"), 1);
    assert_eq!(slack("received"), 2);
    assert_eq!(slack("dateofbirth"), 2);
}

#[test]
fn an_exact_match_beats_a_near_one() {
    // A page with both "Date" and a badly-read "Dste" on it must resolve to
    // the one that is exactly right, not report an ambiguity — and certainly
    // not pick the wrong one.
    let rows = vec![
        row(40.0, 20.0, &["Dispatched"]),
        row(60.0, 20.0, &["Dispatcher"]),
    ];
    let placed = place_in(&rows, "Dispatched", Where::After, 1.0, 5.0).unwrap();
    assert!((placed.y_mm - 40.0).abs() < 1e-9, "{placed:?}");
}

#[test]
fn a_wildly_different_word_is_never_forgiven_into_a_match() {
    let rows = vec![row(40.0, 20.0, &["Received:"])];
    for asked in ["Telephone", "Signature", "xxxxxxxxx"] {
        assert!(
            place_in(&rows, asked, Where::After, 1.0, 5.0).is_err(),
            "{asked} was wrongly matched"
        );
    }
}

#[test]
fn an_anchor_nothing_resembles_still_shows_the_page() {
    // Nothing scores well enough to suggest, so it falls back to saying what
    // is there — which is the question the person is really asking.
    let rows = vec![row(40.0, 20.0, &["Received:"])];
    let said = place_in(&rows, "zzzzzz", Where::After, 1.0, 5.0)
        .unwrap_err()
        .to_string();
    assert!(said.contains("What is on the page"), "{said}");
    assert!(said.contains("Received:"), "{said}");
}

#[test]
fn an_anchor_that_appears_twice_is_refused_rather_than_guessed() {
    // Putting the words next to the first of five Date: fields is a coin
    // toss, and a coin toss that ruins a sheet of paper is worse than a
    // question.
    let rows = vec![
        row(40.0, 20.0, &["Date:", "______"]),
        row(60.0, 20.0, &["Date:", "______"]),
    ];
    let said = place_in(&rows, "Date:", Where::After, 1.0, 5.0)
        .unwrap_err()
        .to_string();
    assert!(said.contains("appears 2 times"), "{said}");
    // And it says where they are, so more words can be given.
    assert!(said.contains("40 mm down"), "{said}");
    assert!(said.contains("60 mm down"), "{said}");

    // Naming more of the line resolves it.
    let rows = vec![
        row(40.0, 20.0, &["Date:", "issued"]),
        row(60.0, 20.0, &["Date:", "expires"]),
    ];
    assert!(place_in(&rows, "Date: expires", Where::After, 1.0, 5.0).is_ok());
}

#[test]
fn an_empty_page_and_an_empty_anchor_are_both_refused() {
    assert!(matches!(
        place_in(&[], "anything", Where::After, 1.0, 5.0),
        Err(AnchorError::NothingRead)
    ));
    let rows = vec![row(40.0, 20.0, &["Received:"])];
    assert!(place_in(&rows, "   ", Where::After, 1.0, 5.0).is_err());
}

#[test]
fn the_anchors_own_letter_height_comes_back() {
    // So a caller with no size of its own can set the new words to match what
    // is already on the page.
    let rows = vec![row(40.0, 20.0, &["Received:"])];
    let placed = place_in(&rows, "Received:", Where::After, 1.0, 5.0).unwrap();
    assert!((placed.letter_height_mm - 3.0).abs() < 1e-9, "{placed:?}");
    assert_eq!(placed.line, "Received:");
}

#[test]
fn a_run_longer_than_the_anchor_is_abandoned_rather_than_walked_to_the_end() {
    // The inner loop stops as soon as the joined run is longer than what is
    // wanted. A line of two hundred words should not cost two hundred
    // comparisons per starting point, and more importantly a run that has
    // already overshot can never come back.
    let many: Vec<&str> = vec!["filler"; 200];
    let rows = vec![row(40.0, 20.0, &many)];
    assert!(place_in(&rows, "notthere", Where::After, 1.0, 5.0).is_err());
}

#[test]
fn where_is_parsed_from_what_somebody_would_type() {
    assert_eq!(Where::parse("after"), Some(Where::After));
    assert_eq!(Where::parse("BELOW"), Some(Where::Below));
    assert_eq!(Where::parse(" below-end "), Some(Where::BelowEnd));
    assert_eq!(Where::parse("beside"), None);
}

/// A line as a document's own layout hands it over.
fn placed(text: &str, x_mm: f64, baseline_mm: f64, size_pt: f64) -> crate::pdf::PlacedLine {
    crate::pdf::PlacedLine {
        text: text.to_string(),
        x_mm,
        y_mm: baseline_mm,
        size_pt,
        font: crate::pdf::LineFont::Builtin(crate::pdf::Font::Helvetica),
        rotation_deg: 0.0,
        colour: (0.0, 0.0, 0.0),
    }
}

#[test]
fn a_documents_own_words_can_be_anchored_to_without_rendering_it() {
    // A document knows where its words are to the millimetre they will print
    // at, so anchoring in one needs no picture of it and no reading.
    let rows = rows_from_lines(&[placed("Received:", 20.0, 40.0, 11.0)]);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].words.len(), 1);
    assert_eq!(rows[0].words[0].0, "Received:");
    assert!((rows[0].baseline_mm - 40.0).abs() < 1e-9);

    let placed_at = place_in(&rows, "Received:", Where::After, 1.0, 5.0).unwrap();
    // Just past the anchor, which is as wide as Helvetica sets it.
    let width = crate::pdf::builtin_width_mm(crate::pdf::Font::Helvetica, "Received:", 11.0);
    assert!(
        (placed_at.x_mm - (20.0 + width + 1.0)).abs() < 1e-9,
        "{placed_at:?} against a width of {width}"
    );
}

#[test]
fn each_word_on_a_line_gets_its_own_place() {
    // "Date of birth" is one line and three words, and the anchor may be any
    // run of them — so each needs measuring where it actually sits.
    let rows = rows_from_lines(&[placed("Date of birth", 20.0, 40.0, 11.0)]);
    assert_eq!(rows[0].words.len(), 3);
    let (first, second) = (&rows[0].words[0], &rows[0].words[1]);
    assert_eq!(first.0, "Date");
    assert_eq!(second.0, "of");
    assert!(second.1.x_mm > first.1.x_mm + first.1.width_mm, "{rows:?}");

    // And the whole run is findable as one anchor.
    assert!(place_in(&rows, "Date of birth", Where::After, 1.0, 5.0).is_ok());
}

#[test]
fn a_line_set_in_an_embedded_font_is_left_out_rather_than_guessed_at() {
    // Its widths are not knowable from here, and a rectangle invented for it
    // would place the new words at a wrong millimetre — worse than not
    // finding the anchor at all, because it prints.
    let mut line = placed("Received:", 20.0, 40.0, 11.0);
    line.font = crate::pdf::LineFont::Embedded;
    assert!(rows_from_lines(&[line]).is_empty());

    // An empty line contributes nothing either, rather than an empty row.
    assert!(rows_from_lines(&[placed("   ", 20.0, 40.0, 11.0)]).is_empty());
}

#[test]
fn rows_come_back_down_the_page_whatever_order_they_were_written_in() {
    // Items are stored in the order somebody added them, which is not the
    // order they sit in. "the line below" has to mean the one below.
    let rows = rows_from_lines(&[
        placed("third", 20.0, 90.0, 11.0),
        placed("first", 20.0, 40.0, 11.0),
        placed("second", 20.0, 60.0, 11.0),
    ]);
    let order: Vec<&str> = rows.iter().map(|r| r.words[0].0.as_str()).collect();
    assert_eq!(order, vec!["first", "second", "third"]);
}
