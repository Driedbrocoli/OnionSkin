//! Tests for working out what a page is set in.
//!
//! None of these is a scan. Every one of them starts from Adobe's own advance
//! widths — the same table a PDF reader uses to lay the page out — sets a
//! handful of words at a known size in a known face, takes a constant off each
//! for the side bearings an ink box loses, and then asks the module what it is
//! looking at.
//!
//! That is the strongest test available here, and stronger than a photograph of
//! a real page would be. A photograph proves that one page worked; this knows
//! the answer exactly, so recovering the size to a hundredth of a point is a
//! result rather than a plausibility, and every face can be tried at every size
//! without anybody printing anything.

use super::*;
use crate::letters::{Rect, TextLine};

/// A page of ordinary words, long and short, upper and lower case.
///
/// Length has to vary or the fit has nothing to work with: the size and the
/// bearing can only be separated by seeing what happens to the width as the
/// words get longer, and twenty words all the same length say that once.
const PROSE: &[&str] = &[
    "the",
    "quick",
    "brown",
    "fox",
    "jumps",
    "over",
    "lazy",
    "dog",
    "Invoice",
    "number",
    "Onionskin",
    "adds",
    "words",
    "to",
    "page",
    "already",
    "printed",
    "Received",
    "with",
    "thanks",
];

/// The widths a page set in this face at this size would show a scanner, with
/// `bearing_mm` lost off each word for the side bearings at its two ends.
fn page_set_in(font: Font, size_pt: f64, bearing_mm: f64) -> Vec<(String, f64)> {
    words_set_in(font, size_pt, bearing_mm, PROSE)
}

fn words_set_in(font: Font, size_pt: f64, bearing_mm: f64, words: &[&str]) -> Vec<(String, f64)> {
    words
        .iter()
        .map(|text| {
            (
                text.to_string(),
                pdf::builtin_width_mm(font, text, size_pt) + bearing_mm,
            )
        })
        .collect()
}

/// Would these two faces have set this page identically?
///
/// Two of the eight would — see `an_oblique_page_cannot_be_told_from_an_upright_one`
/// — so a round trip that comes back with the twin has not failed.
fn same_widths(one: Font, other: Font) -> bool {
    PROSE.iter().all(|text| {
        (pdf::builtin_width_mm(one, text, 10.0) - pdf::builtin_width_mm(other, text, 10.0)).abs()
            < 1e-9
    })
}

/// A deterministic wobble in millimetres, standing in for scanning error.
///
/// Not a random number generator: a test that fails one time in fifty is worse
/// than no test, because it teaches everybody to run it again.
fn wobble(seed: usize, amount_mm: f64) -> f64 {
    let spun = (seed as u64)
        .wrapping_add(1)
        .wrapping_mul(6_364_136_223_846_793_005);
    // The top thirty-two bits, spread over -1 to 1.
    let unit = ((spun >> 32) as f64) / ((1u64 << 31) as f64) - 1.0;
    unit * amount_mm
}

// ---------------------------------------------------------------------------
// The round trip: set a page, then read it back
// ---------------------------------------------------------------------------

#[test]
fn a_page_is_recognised_as_the_face_it_was_set_in() {
    for &font in Font::all() {
        let words = page_set_in(font, 11.0, -0.35);
        let found = detect_measured(&words, None).expect("a page of twenty words is enough");
        assert!(
            same_widths(found.font, font),
            "a page set in {} came back as {}",
            font.base_name(),
            found.font.base_name()
        );
        assert!(
            (found.size_pt - 11.0).abs() < 0.01,
            "{} came back at {} pt, not 11",
            font.base_name(),
            found.size_pt
        );
        assert_eq!(found.words_measured, PROSE.len());
        assert!(found.confidence > 0.9, "confidence {}", found.confidence);
    }
}

#[test]
fn the_size_comes_back_whatever_it_was() {
    for size_pt in [5.0, 8.0, 9.5, 11.0, 12.0, 18.0, 47.5, 144.0] {
        for &font in &[Font::TimesRoman, Font::Helvetica, Font::Courier] {
            let words = page_set_in(font, size_pt, -0.3);
            let found = detect_measured(&words, None).expect("enough words");
            assert!(
                (found.size_pt - size_pt).abs() < 0.01,
                "{} at {} pt came back at {}",
                font.base_name(),
                size_pt,
                found.size_pt
            );
        }
    }
}

#[test]
fn the_side_bearings_are_absorbed_rather_than_taken_off_the_size() {
    // The whole reason two numbers are fitted rather than one. However much
    // the ink box loses at the ends of a word, the size it reports is the size
    // the page was set at.
    for bearing_mm in [0.0, -0.1, -0.35, -0.8, -1.4] {
        let words = page_set_in(Font::TimesRoman, 11.5, bearing_mm);
        let found = detect_measured(&words, None).expect("enough words");
        assert_eq!(found.font, Font::TimesRoman);
        assert!(
            (found.size_pt - 11.5).abs() < 0.01,
            "a bearing of {bearing_mm} mm moved the size to {}",
            found.size_pt
        );
    }
}

#[test]
fn a_scan_that_measures_imperfectly_still_gives_the_face_and_the_size() {
    // A pixel at 300 dpi is 0.085 mm, and an ink box is measured to about one
    // of them at each end. Nothing here should need better than that.
    for &font in &[Font::Helvetica, Font::TimesRoman, Font::Courier] {
        let mut words = page_set_in(font, 11.0, -0.35);
        for (index, word) in words.iter_mut().enumerate() {
            word.1 += wobble(index * 31 + font.base_name().len(), 0.1);
        }
        let found = detect_measured(&words, None).expect("enough words");
        assert!(
            same_widths(found.font, font),
            "{} came back as {} once the widths wobbled",
            font.base_name(),
            found.font.base_name()
        );
        assert!(
            (found.size_pt - 11.0).abs() < 0.15,
            "{} came back at {} pt",
            font.base_name(),
            found.size_pt
        );
    }
}

// ---------------------------------------------------------------------------
// Telling the families apart
// ---------------------------------------------------------------------------

#[test]
fn a_monospaced_page_is_told_from_a_proportional_one() {
    // The clearest evidence there is. In Courier `illinois` and `wombats` are
    // the same width to the thousandth of an em; in Helvetica the first is two
    // thirds the width of the second. Words chosen to make the two disagree as
    // loudly as possible.
    let awkward = &[
        "illinois", "wombats", "mill", "warm", "little", "modem", "if", "ow", "filling", "wowed",
    ];

    let typed = words_set_in(Font::Courier, 10.0, -0.3, awkward);
    let found = detect_measured(&typed, None).expect("enough words");
    assert_eq!(found.font, Font::Courier, "a typewritten page");
    assert!(found.confidence > 0.9);

    let set = words_set_in(Font::Helvetica, 10.0, -0.3, awkward);
    let found = detect_measured(&set, None).expect("enough words");
    assert_eq!(found.font, Font::Helvetica, "a proportionally set page");
    assert!(found.confidence > 0.9);
}

#[test]
fn a_serif_page_is_told_from_a_sans_one() {
    let serif = detect_measured(&page_set_in(Font::TimesRoman, 12.0, -0.3), None).unwrap();
    let sans = detect_measured(&page_set_in(Font::Helvetica, 12.0, -0.3), None).unwrap();
    assert_eq!(serif.font, Font::TimesRoman);
    assert_eq!(sans.font, Font::Helvetica);
}

#[test]
fn an_oblique_page_cannot_be_told_from_an_upright_one() {
    // An honest limit rather than a bug, and it is Adobe's: Helvetica-Oblique
    // is Helvetica width for width, and Courier-Bold is Courier width for
    // width. Nothing measured off the page can separate either pair, so the
    // tie breaks towards the upright and the regular — and the Courier pair is
    // separated by weight instead, below.
    let sloped = detect_measured(&page_set_in(Font::HelveticaOblique, 11.0, -0.3), None).unwrap();
    assert_eq!(sloped.font, Font::Helvetica);

    let heavy = detect_measured(&page_set_in(Font::CourierBold, 11.0, -0.3), None).unwrap();
    assert_eq!(heavy.font, Font::Courier);

    // Times-Italic is genuinely narrower than Times-Roman, so it is found.
    let italic = detect_measured(&page_set_in(Font::TimesItalic, 11.0, -0.3), None).unwrap();
    assert_eq!(italic.font, Font::TimesItalic);
}

#[test]
fn confidence_falls_when_the_widths_stop_agreeing() {
    let clean = detect_measured(&page_set_in(Font::TimesRoman, 11.0, -0.35), None).unwrap();

    let mut muddled = page_set_in(Font::TimesRoman, 11.0, -0.35);
    for (index, word) in muddled.iter_mut().enumerate() {
        word.1 += wobble(index * 7 + 3, 1.2);
    }
    let muddled = detect_measured(&muddled, None).unwrap();

    assert!(clean.confidence > 0.9, "clean {}", clean.confidence);
    assert!(
        muddled.confidence < clean.confidence * 0.6,
        "muddled {} against clean {}",
        muddled.confidence,
        clean.confidence
    );
    assert!((0.0..=1.0).contains(&muddled.confidence));
}

// ---------------------------------------------------------------------------
// Saying nothing rather than saying something wrong
// ---------------------------------------------------------------------------

#[test]
fn too_few_words_is_no_answer() {
    let font = Font::Helvetica;
    for count in 0..FEWEST_WORDS {
        let words = words_set_in(font, 11.0, -0.3, &PROSE[..count]);
        assert!(
            detect_measured(&words, None).is_none(),
            "{count} words should not be enough"
        );
    }
    // And three is, which is what makes the line above a threshold rather than
    // a refusal to work.
    let three = words_set_in(font, 11.0, -0.3, &PROSE[..3]);
    assert!(detect_measured(&three, None).is_some());
}

#[test]
fn a_page_with_nothing_on_it_is_no_answer() {
    let blank = PageText {
        lines: Vec::new(),
        discarded: 0,
    };
    assert!(detect(&blank).is_none());
}

#[test]
fn a_page_of_marks_that_are_not_words_is_no_answer() {
    // Lines and words with no letters in them: found ink that was never read.
    // `Word::text` says `Some("")` for a word of no letters, so this is the
    // case that would slip through a check written as "did it read".
    let page = PageText {
        lines: vec![TextLine {
            rect: somewhere(),
            baseline_mm: 30.0,
            words: vec![
                Word {
                    rect: somewhere(),
                    letters: Vec::new(),
                },
                Word {
                    rect: somewhere(),
                    letters: Vec::new(),
                },
            ],
        }],
        discarded: 12,
    };
    assert!(detect(&page).is_none());
}

fn somewhere() -> Rect {
    Rect {
        x_mm: 20.0,
        y_mm: 30.0,
        width_mm: 8.0,
        height_mm: 3.0,
    }
}

#[test]
fn an_absurd_size_is_no_answer() {
    // Ink far too small to have been printed, and far too large to be a page.
    assert!(detect_measured(&page_set_in(Font::Helvetica, 2.0, 0.0), None).is_none());
    assert!(detect_measured(&page_set_in(Font::Helvetica, 400.0, 0.0), None).is_none());
}

#[test]
fn ink_that_shrinks_as_the_words_grow_is_no_answer() {
    // A negative size, which is what a page of nonsense measurements looks
    // like: the longer the word, the narrower its ink.
    let words: Vec<(String, f64)> = PROSE
        .iter()
        .map(|text| {
            (
                text.to_string(),
                40.0 - pdf::builtin_width_mm(Font::Helvetica, text, 11.0),
            )
        })
        .collect();
    assert!(detect_measured(&words, None).is_none());
}

#[test]
fn a_column_of_typewritten_words_of_one_length_settles_nothing() {
    // A column of a table, typed. Every Courier character is the same width,
    // so words of the same length are the same width to the thousandth of an
    // em, and there is nothing here to fit at all: Courier's own line goes
    // through a single point, and any size fits it once the bearing is chosen
    // to suit. The proportional faces cannot rescue it either — the only way
    // to explain ink that never changes width is to say the page is set at no
    // size at all, and no size is not an answer.
    let words: Vec<(String, f64)> = ["cost", "date", "item", "name", "unit", "each"]
        .iter()
        .map(|text| {
            (
                text.to_string(),
                pdf::builtin_width_mm(Font::Courier, text, 10.0) - 0.3,
            )
        })
        .collect();
    assert!(fit(Font::Courier, &words).is_none());
    assert!(detect_measured(&words, None).is_none());
}

// ---------------------------------------------------------------------------
// A page that is not all one size
// ---------------------------------------------------------------------------

#[test]
fn a_heading_does_not_drag_the_body_text_off_its_size() {
    let mut words = page_set_in(Font::Helvetica, 11.0, -0.35);
    words.extend(words_set_in(
        Font::Helvetica,
        30.0,
        -0.9,
        &["Statement", "Account"],
    ));

    let found = detect_measured(&words, None).expect("enough words");
    assert_eq!(found.font, Font::Helvetica);
    assert!(
        (found.size_pt - 11.0).abs() < 0.05,
        "the heading pulled the answer to {} pt",
        found.size_pt
    );
    // The two heading words were set aside, and said so.
    assert_eq!(found.words_measured, PROSE.len());
}

#[test]
fn a_page_that_is_all_one_size_keeps_every_word_of_it() {
    let found = detect_measured(&page_set_in(Font::TimesRoman, 11.0, -0.35), None).unwrap();
    assert_eq!(found.words_measured, PROSE.len());
}

// ---------------------------------------------------------------------------
// Weight, which the widths cannot see
// ---------------------------------------------------------------------------

#[test]
fn heavy_ink_is_told_from_light_ink() {
    // The measured medians from the metric-compatible clones of the eight
    // faces: a regular tops out around 0.46 and a bold starts around 0.55.
    assert_eq!(weigh(Font::Helvetica, Some(0.46)), Font::Helvetica);
    assert_eq!(weigh(Font::Helvetica, Some(0.62)), Font::HelveticaBold);
    assert_eq!(weigh(Font::TimesRoman, Some(0.40)), Font::TimesRoman);
    assert_eq!(weigh(Font::TimesRoman, Some(0.55)), Font::TimesBold);

    // And it demotes as well as promotes: a page the widths called bold but
    // whose ink is plainly light is set regular.
    assert_eq!(weigh(Font::HelveticaBold, Some(0.40)), Font::Helvetica);
    assert_eq!(weigh(Font::TimesBold, Some(0.38)), Font::TimesRoman);
}

#[test]
fn bold_courier_is_found_by_its_ink_because_its_widths_cannot_be() {
    // The case the whole weight test exists for. Courier and Courier-Bold are
    // identical width for width, so the page below fits Courier exactly — and
    // only the ink can say it was typed in bold.
    let words = page_set_in(Font::CourierBold, 10.0, -0.3);
    assert_eq!(detect_measured(&words, None).unwrap().font, Font::Courier);
    assert_eq!(
        detect_measured(&words, Some(0.59)).unwrap().font,
        Font::CourierBold
    );
}

#[test]
fn ink_that_has_not_made_up_its_mind_leaves_the_widths_alone() {
    // Between the two bands. A scan binarised dark fattens every stroke and a
    // light one thins it, by about as much as the gap between the weights, so
    // the middle is not an intermediate weight — it is silence.
    for coverage in [0.49, 0.50, 0.52] {
        assert_eq!(weigh(Font::Helvetica, Some(coverage)), Font::Helvetica);
        assert_eq!(
            weigh(Font::HelveticaBold, Some(coverage)),
            Font::HelveticaBold
        );
    }
    assert_eq!(weigh(Font::Courier, None), Font::Courier);
}

#[test]
fn the_sloped_faces_are_left_at_the_weight_they_were_found_at() {
    // There is no bold oblique or bold italic among the eight to promote them
    // to, so however heavy the ink is they stay as they are.
    assert_eq!(
        weigh(Font::HelveticaOblique, Some(0.70)),
        Font::HelveticaOblique
    );
    assert_eq!(weigh(Font::TimesItalic, Some(0.70)), Font::TimesItalic);
}

#[test]
fn the_size_is_fitted_again_for_the_face_the_ink_chose() {
    // The path where the face reported is not the face the widths picked.
    let typed = page_set_in(Font::CourierBold, 10.0, -0.3);
    let found = detect_measured(&typed, Some(0.59)).unwrap();
    assert_eq!(found.font, Font::CourierBold);
    assert!((found.size_pt - 10.0).abs() < 0.01, "{} pt", found.size_pt);

    // And it matters, because the two weights of a family are not the same
    // width. Read against Helvetica's table, a page set in Helvetica-Bold comes
    // out several per cent large — so a size fitted before the weight was
    // decided would be quoted beside the wrong name.
    let heavy = page_set_in(Font::HelveticaBold, 12.0, -0.35);
    let regular = fit(Font::Helvetica, &heavy).expect("a fit of some kind");
    assert!(
        (regular.size_pt - 12.0).abs() > 0.1,
        "the regular table happened to agree, at {} pt",
        regular.size_pt
    );
    let found = detect_measured(&heavy, Some(0.62)).unwrap();
    assert_eq!(found.font, Font::HelveticaBold);
    assert!((found.size_pt - 12.0).abs() < 0.01, "{} pt", found.size_pt);
}

// ---------------------------------------------------------------------------
// Measuring the coverage in the first place
// ---------------------------------------------------------------------------

#[test]
fn the_coverage_ignores_letters_of_the_wrong_shape() {
    // An `l` fills half a narrow box in any weight and a `w` is mostly white,
    // so neither says anything about the weight of the page. Only the plain
    // round letters count, and here they are all light.
    let letters = [
        ('l', 0.90),
        ('l', 0.92),
        ('w', 0.20),
        ('W', 0.18),
        ('i', 0.95),
        ('o', 0.41),
        ('e', 0.42),
        ('n', 0.43),
        ('a', 0.40),
        ('s', 0.44),
        ('c', 0.39),
        ('u', 0.42),
        ('m', 0.41),
    ];
    let median = median_coverage(letters.into_iter()).expect("eight plain letters");
    assert!((median - 0.41).abs() < 0.02, "median {median}");
    assert_eq!(weigh(Font::TimesRoman, Some(median)), Font::TimesRoman);
}

#[test]
fn a_page_with_too_few_plain_letters_says_nothing_about_weight() {
    let letters = [('o', 0.60), ('e', 0.62), ('n', 0.61)];
    assert!(median_coverage(letters.into_iter()).is_none());
}

#[test]
fn one_smudged_letter_does_not_decide_the_weight() {
    // A letter merged with the one beside it, or a comma swept up into a `c`,
    // gives a box far too full. A median cannot be moved by one of them; an
    // average would have been dragged over the line into bold.
    let mut letters: Vec<(char, f64)> = PLAIN_LETTERS.chars().map(|ch| (ch, 0.44)).collect();
    letters.push(('o', 3.0));
    let median = median_coverage(letters.into_iter()).expect("nine plain letters");
    assert!((median - 0.44).abs() < 1e-9, "median {median}");
    assert_eq!(weigh(Font::Helvetica, Some(median)), Font::Helvetica);
}

// ---------------------------------------------------------------------------
// Which words count as evidence
// ---------------------------------------------------------------------------

#[test]
fn only_words_whose_width_means_something_are_measured() {
    assert!(worth_measuring("total"));
    assert!(worth_measuring("2024"));
    // WinAnsi is Western European, and the built-in fonts really do set this.
    assert!(worth_measuring("café"));

    // Nothing was read at all, or one mark was — and a single mark on a form
    // is as likely to be a tick, a bullet or a speck as it is to be a word.
    assert!(!worth_measuring(""));
    assert!(!worth_measuring("a"));
    // Punctuation carries far more side bearing than a letter, which is the
    // one thing the fit needs to hold constant across the page.
    assert!(!worth_measuring("total."));
    assert!(!worth_measuring("don't"));
    // And a character with no width in these tables would count as nothing,
    // taking real ink out of the prediction but not out of the measurement.
    assert!(!worth_measuring("ありがとう"));
}

#[test]
fn a_word_that_was_not_all_read_is_not_measured() {
    // `Word::text` is `None` when any letter was left unread, so such a word
    // never reaches `worth_measuring`. A word of no letters is the awkward
    // case: it reads as `Some("")`, which is text, and would slip through a
    // check written as "did it read".
    let empty = Word {
        rect: somewhere(),
        letters: Vec::new(),
    };
    assert_eq!(empty.text().as_deref(), Some(""));
    assert!(measure(&empty).is_none());

    let page = PageText {
        lines: vec![TextLine {
            rect: somewhere(),
            baseline_mm: 40.0,
            words: vec![empty],
        }],
        discarded: 0,
    };
    assert!(measurable(&page).is_empty());
}

// ---------------------------------------------------------------------------
// Saying it out loud
// ---------------------------------------------------------------------------

#[test]
fn describe_says_what_was_found() {
    let found = detect_measured(&page_set_in(Font::TimesRoman, 11.5, -0.35), None).unwrap();
    assert_eq!(
        found.describe(),
        format!("Times-Roman at about 11.5 pt, from {} words", PROSE.len())
    );
}
