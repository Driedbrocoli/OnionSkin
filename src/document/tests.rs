//! Tests for making a document from nothing and editing it.

use super::*;
use std::path::PathBuf;

const A4: PageSize = PageSize {
    width_mm: 210.0,
    height_mm: 297.0,
};

fn item(text: &str, x_mm: f64, y_mm: f64) -> Item {
    Item {
        id: 0,
        page: 1,
        x_mm,
        y_mm,
        text: text.to_string(),
        size_pt: 11.0,
        font: "Helvetica".into(),
        width_mm: None,
        rotation_deg: 0.0,
        colour: "#000000".into(),
        leading: 1.2,
    }
}

fn dejavu() -> Option<EmbeddedFont> {
    let path = PathBuf::from("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf");
    path.is_file().then(|| EmbeddedFont::load(&path).unwrap())
}

/// A drawing with a black outline, no fill, and a 1 mm line — the plainest
/// thing `check` will accept, ready to be customised by each test.
fn shape(kind: ShapeKind) -> Shape {
    Shape {
        id: 0,
        page: 1,
        kind,
        stroke: Some("#000000".into()),
        fill: None,
        width_mm: 1.0,
        dash_mm: None,
    }
}

fn line(x1_mm: f64, y1_mm: f64, x2_mm: f64, y2_mm: f64) -> Shape {
    shape(ShapeKind::Line {
        x1_mm,
        y1_mm,
        x2_mm,
        y2_mm,
    })
}

// ---------------------------------------------------------------------------
// Making one, and writing on it
// ---------------------------------------------------------------------------

#[test]
fn a_new_document_is_one_blank_page() {
    let doc = Document::blank(A4, 1);

    assert_eq!(doc.pages, 1);
    assert!(doc.items.is_empty());
    assert!(!doc.has_been_printed());
    assert_eq!(doc.page_sizes(), vec![A4]);
}

#[test]
fn a_document_cannot_have_no_pages_at_all() {
    assert_eq!(Document::blank(A4, 0).pages, 1);
}

#[test]
fn words_go_on_the_page_and_come_back_numbered() {
    let mut doc = Document::blank(A4, 1);

    let first = doc.add(item("Dear Sir", 25.0, 40.0)).unwrap();
    let second = doc.add(item("Yours faithfully", 25.0, 80.0)).unwrap();

    assert_ne!(first, second, "every item gets its own number");
    assert_eq!(doc.items.len(), 2);
    assert_eq!(doc.get(first).unwrap().text, "Dear Sir");
}

#[test]
fn numbers_are_never_handed_out_twice() {
    // A printed record refers to an item by number, so reusing one would let a
    // new piece of text inherit the standing of a deleted one.
    let mut doc = Document::blank(A4, 1);
    let first = doc.add(item("one", 20.0, 30.0)).unwrap();
    doc.remove(first).unwrap();
    let second = doc.add(item("two", 20.0, 40.0)).unwrap();

    assert_ne!(first, second);
}

#[test]
fn asking_for_a_later_page_makes_the_document_longer() {
    let mut doc = Document::blank(A4, 1);
    let mut later = item("Continued overleaf", 25.0, 40.0);
    later.page = 3;
    doc.add(later).unwrap();

    assert_eq!(doc.pages, 3);
    assert_eq!(doc.page_sizes().len(), 3);
}

#[test]
fn editing_changes_what_is_there() {
    let mut doc = Document::blank(A4, 1);
    let id = doc.add(item("Dear Sir", 25.0, 40.0)).unwrap();

    doc.get_mut(id).unwrap().text = "Dear Madam".into();
    doc.get_mut(id).unwrap().x_mm = 30.0;

    assert_eq!(doc.get(id).unwrap().text, "Dear Madam");
    assert_eq!(doc.get(id).unwrap().x_mm, 30.0);
}

#[test]
fn editing_something_that_is_not_there_says_so() {
    let mut doc = Document::blank(A4, 1);
    let err = doc.get_mut(99).unwrap_err().to_string();
    assert!(err.contains("no item numbered 99"), "{err}");
    assert!(err.contains("onionskin show"), "{err}");
}

#[test]
fn removing_takes_it_off_the_page() {
    let mut doc = Document::blank(A4, 1);
    let id = doc.add(item("a mistake", 25.0, 40.0)).unwrap();
    let gone = doc.remove(id).unwrap();

    assert_eq!(gone.text, "a mistake");
    assert!(doc.items.is_empty());
    assert!(doc.remove(id).is_err());
}

#[test]
fn the_items_on_a_page_can_be_asked_for() {
    let mut doc = Document::blank(A4, 2);
    doc.add(item("page one", 25.0, 40.0)).unwrap();
    let mut second = item("page two", 25.0, 40.0);
    second.page = 2;
    doc.add(second).unwrap();

    assert_eq!(doc.on_page(1).count(), 1);
    assert_eq!(doc.on_page(2).next().unwrap().text, "page two");
}

// ---------------------------------------------------------------------------
// Saving and re-opening
// ---------------------------------------------------------------------------

#[test]
fn a_document_survives_being_saved_and_opened() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("letter.onionskin");

    let mut doc = Document::blank(A4, 2);
    doc.add(item("Dear Sir", 25.0, 40.0)).unwrap();
    let mut wrapped = item("A long paragraph that will be wrapped.", 25.0, 60.0);
    wrapped.width_mm = Some(80.0);
    wrapped.colour = "#202020".into();
    doc.add(wrapped).unwrap();
    doc.save(&path).unwrap();

    let opened = Document::load(&path).unwrap();
    assert_eq!(
        opened, doc,
        "the document changed on the way through a file"
    );
}

#[test]
fn saving_does_not_destroy_the_old_one_if_it_fails() {
    // Writing straight to the destination truncates it first, so a failure
    // halfway leaves an empty file where the work used to be.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("letter.onionskin");
    let mut doc = Document::blank(A4, 1);
    doc.add(item("something worth keeping", 25.0, 40.0))
        .unwrap();
    doc.save(&path).unwrap();

    // A directory in the way is the simplest thing that cannot be renamed over.
    let blocked = dir.path().join("blocked.onionskin");
    std::fs::create_dir(&blocked).unwrap();
    assert!(doc.save(&blocked).is_err());

    // And the good one is still readable.
    assert_eq!(Document::load(&path).unwrap().items.len(), 1);
    // No litter left behind either.
    assert!(!dir.path().join("blocked.onionskin-tmp").exists());
}

#[test]
fn a_missing_document_is_reported_plainly() {
    let err = Document::load(Path::new("/nowhere/letter.onionskin"))
        .unwrap_err()
        .to_string();
    assert!(err.contains("no document at"), "{err}");
}

#[test]
fn a_file_that_is_not_a_document_is_reported_plainly() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notadoc.onionskin");
    std::fs::write(&path, b"this is not JSON at all").unwrap();

    let err = Document::load(&path).unwrap_err().to_string();
    assert!(err.contains("not an Onionskin document"), "{err}");
}

#[test]
fn a_document_with_impossible_numbers_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    for (bad, expected) in [
        (
            r#"{"page":{"width_mm":0,"height_mm":297},"pages":1,"items":[]}"#,
            "size of paper",
        ),
        (
            r#"{"page":{"width_mm":210,"height_mm":297},"pages":1,"items":[
                {"id":1,"page":1,"x_mm":10,"y_mm":10,"text":"x","size_pt":0,"font":"Helvetica"}]}"#,
            "cannot be printed",
        ),
        (
            r#"{"page":{"width_mm":210,"height_mm":297},"pages":1,"items":[
                {"id":1,"page":7,"x_mm":10,"y_mm":10,"text":"x","size_pt":11,"font":"Helvetica"}]}"#,
            "has 1 pages",
        ),
    ] {
        let path = dir.path().join("bad.onionskin");
        std::fs::write(&path, bad).unwrap();
        let err = Document::load(&path).unwrap_err().to_string();
        assert!(err.contains(expected), "expected {expected:?}, got {err}");
    }
}

#[test]
fn a_hand_written_document_does_not_reuse_its_numbers() {
    // Someone editing the JSON by hand will not have set next_id.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("hand.onionskin");
    std::fs::write(
        &path,
        r#"{"page":{"width_mm":210,"height_mm":297},"pages":1,"items":[
            {"id":4,"page":1,"x_mm":10,"y_mm":10,"text":"x","size_pt":11,"font":"Helvetica"}]}"#,
    )
    .unwrap();

    let mut doc = Document::load(&path).unwrap();
    let fresh = doc.add(item("new", 20.0, 20.0)).unwrap();
    assert!(fresh > 4, "handed out {fresh}, which is already in use");
}

// ---------------------------------------------------------------------------
// Laying it out
// ---------------------------------------------------------------------------

#[test]
fn text_lands_where_it_was_put() {
    let mut doc = Document::blank(A4, 1);
    doc.add(item("Dear Sir", 25.0, 40.0)).unwrap();

    let pages = doc.layout(None).unwrap();
    assert_eq!(pages.len(), 1);
    assert_eq!(pages[0].len(), 1);
    assert_eq!(pages[0][0].x_mm, 25.0);
    assert_eq!(pages[0][0].y_mm, 40.0);
    assert_eq!(pages[0][0].text, "Dear Sir");
}

#[test]
fn a_line_break_in_the_text_is_a_line_break_on_the_page() {
    let mut doc = Document::blank(A4, 1);
    doc.add(item("first\nsecond\nthird", 25.0, 40.0)).unwrap();

    let pages = doc.layout(None).unwrap();
    let lines = &pages[0];
    assert_eq!(lines.len(), 3);
    assert_eq!(lines[0].text, "first");
    assert_eq!(lines[2].text, "third");
    // Each one below the last, by the leading.
    let step = 11.0 * 1.2 * 25.4 / 72.0;
    assert!((lines[1].y_mm - lines[0].y_mm - step).abs() < 1e-9);
    assert!((lines[2].y_mm - lines[1].y_mm - step).abs() < 1e-9);
}

#[test]
fn a_blank_line_stays_blank_rather_than_closing_up() {
    let mut doc = Document::blank(A4, 1);
    doc.add(item("above\n\nbelow", 25.0, 40.0)).unwrap();

    let pages = doc.layout(None).unwrap();
    assert_eq!(pages[0].len(), 3);
    assert_eq!(pages[0][1].text, "");
}

#[test]
fn a_paragraph_wraps_at_the_width_it_was_given() {
    let mut doc = Document::blank(A4, 1);
    let mut para = item(
        "The quick brown fox jumps over the lazy dog and keeps on running.",
        25.0,
        40.0,
    );
    para.width_mm = Some(50.0);
    doc.add(para).unwrap();

    let pages = doc.layout(None).unwrap();
    assert!(pages[0].len() > 1, "it did not wrap at all");
    for line in &pages[0] {
        let width = crate::pdf::builtin_width_mm(Font::Helvetica, &line.text, 11.0);
        assert!(width <= 50.0, "{:?} is {width:.1} mm wide", line.text);
    }
    // And nothing was lost on the way.
    let rejoined = pages[0]
        .iter()
        .map(|l| l.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    assert_eq!(
        rejoined,
        "The quick brown fox jumps over the lazy dog and keeps on running."
    );
}

#[test]
fn a_word_too_long_to_fit_is_kept_rather_than_dropped() {
    let mut doc = Document::blank(A4, 1);
    let mut para = item("short Antidisestablishmentarianism short", 25.0, 40.0);
    para.width_mm = Some(15.0);
    doc.add(para).unwrap();

    let pages = doc.layout(None).unwrap();
    let all: String = pages[0]
        .iter()
        .map(|l| l.text.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    assert!(all.contains("Antidisestablishmentarianism"), "{all}");
}

#[test]
fn wrapping_uses_the_supplied_fonts_own_widths() {
    let Some(font) = dejavu() else { return };
    let mut doc = Document::blank(A4, 1);
    let mut para = item("Утверждено двадцать пятого июля две тысячи", 25.0, 40.0);
    para.font = "file".into();
    para.width_mm = Some(45.0);
    doc.add(para).unwrap();

    let pages = doc.layout(Some(&font)).unwrap();
    assert!(pages[0].len() > 1);
    for line in &pages[0] {
        let width = font.width_mm(&line.text, 11.0).unwrap();
        assert!(width <= 45.0, "{:?} is {width:.1} mm", line.text);
    }
}

#[test]
fn wrapping_in_the_supplied_font_without_one_says_so() {
    let mut doc = Document::blank(A4, 1);
    let mut para = item("some words to wrap", 25.0, 40.0);
    para.font = "file".into();
    para.width_mm = Some(20.0);
    doc.add(para).unwrap();

    let err = doc.layout(None).unwrap_err().to_string();
    assert!(err.contains("--font-file"), "{err}");
}

#[test]
fn a_font_that_does_not_exist_is_named() {
    let mut doc = Document::blank(A4, 1);
    let mut odd = item("hello", 25.0, 40.0);
    odd.font = "Comic Sans".into();
    doc.add(odd).unwrap();

    let err = doc.layout(None).unwrap_err().to_string();
    assert!(err.contains("Comic Sans"), "{err}");
    assert!(err.contains("onionskin fonts"), "{err}");
}

#[test]
fn colours_are_read_and_nonsense_is_refused() {
    let mut doc = Document::blank(A4, 1);
    let mut red = item("warning", 25.0, 40.0);
    red.colour = "#ff0000".into();
    doc.add(red).unwrap();
    let pages = doc.layout(None).unwrap();
    assert_eq!(pages[0][0].colour, (1.0, 0.0, 0.0));

    let mut doc = Document::blank(A4, 1);
    let mut wrong = item("warning", 25.0, 40.0);
    wrong.colour = "reddish".into();
    doc.add(wrong).unwrap();
    let err = doc.layout(None).unwrap_err().to_string();
    assert!(err.contains("#rrggbb"), "{err}");
}

#[test]
fn every_page_gets_its_own_list_even_when_empty() {
    let mut doc = Document::blank(A4, 3);
    let mut third = item("only on the last page", 25.0, 40.0);
    third.page = 3;
    doc.add(third).unwrap();

    let pages = doc.layout(None).unwrap();
    assert_eq!(pages.len(), 3);
    assert!(pages[0].is_empty());
    assert!(pages[1].is_empty());
    assert_eq!(pages[2].len(), 1);
}

// ---------------------------------------------------------------------------
// Printing once, editing, and printing only what is new
// ---------------------------------------------------------------------------

#[test]
fn before_it_is_printed_everything_is_new() {
    let mut doc = Document::blank(A4, 1);
    doc.add(item("Dear Sir", 25.0, 40.0)).unwrap();
    doc.add(item("Yours faithfully", 25.0, 80.0)).unwrap();

    assert_eq!(doc.added_since_printing().len(), 2);
    assert!(doc.overlay_problems().is_empty());
}

#[test]
fn after_printing_nothing_is_new_until_something_is_added() {
    let mut doc = Document::blank(A4, 1);
    doc.add(item("Dear Sir", 25.0, 40.0)).unwrap();
    doc.mark_printed();

    assert!(doc.has_been_printed());
    assert!(doc.added_since_printing().is_empty());

    doc.add(item("Approved 25 July", 25.0, 120.0)).unwrap();
    let added = doc.added_since_printing();
    assert_eq!(added.len(), 1);
    assert_eq!(added[0].text, "Approved 25 July");
}

#[test]
fn the_delta_carries_only_the_new_words() {
    let mut doc = Document::blank(A4, 1);
    doc.add(item("Purchase order 4471", 25.0, 40.0)).unwrap();
    doc.add(item("Two hundred widgets", 25.0, 55.0)).unwrap();
    doc.mark_printed();
    doc.add(item("Approved — J. Bezzina", 25.0, 120.0)).unwrap();

    let delta = doc.delta_layout(None).unwrap();
    assert_eq!(delta.len(), 1, "one page in, one page out");
    assert_eq!(delta[0].len(), 1);
    assert_eq!(delta[0][0].text, "Approved — J. Bezzina");
    assert_eq!(delta[0][0].y_mm, 120.0);

    // The whole document still has everything, of course.
    assert_eq!(doc.layout(None).unwrap()[0].len(), 3);
}

#[test]
fn a_delta_across_pages_keeps_each_on_its_own() {
    let mut doc = Document::blank(A4, 2);
    doc.add(item("page one", 25.0, 40.0)).unwrap();
    doc.mark_printed();

    let mut second = item("added to page two", 25.0, 40.0);
    second.page = 2;
    doc.add(second).unwrap();

    let delta = doc.delta_layout(None).unwrap();
    assert!(delta[0].is_empty(), "page one gains nothing");
    assert_eq!(delta[1].len(), 1);
}

#[test]
fn moving_printed_words_is_refused_because_toner_does_not_lift() {
    let mut doc = Document::blank(A4, 1);
    let id = doc.add(item("Purchase order 4471", 25.0, 40.0)).unwrap();
    doc.mark_printed();

    doc.get_mut(id).unwrap().y_mm = 60.0;

    let problems = doc.overlay_problems();
    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].what, Change::Moved);
    let message = problems[0].format();
    assert!(message.contains("BLOCKER"), "{message}");
    assert!(message.contains("has been moved"), "{message}");
    assert!(message.contains("Purchase order 4471"), "{message}");
    assert!(message.contains("Print this page fresh"), "{message}");
}

#[test]
fn rewording_printed_words_is_refused_too() {
    let mut doc = Document::blank(A4, 1);
    let id = doc.add(item("Two hundred widgets", 25.0, 40.0)).unwrap();
    doc.mark_printed();
    doc.get_mut(id).unwrap().text = "Three hundred widgets".into();

    let problems = doc.overlay_problems();
    assert_eq!(problems[0].what, Change::Reworded);
}

#[test]
fn deleting_printed_words_is_refused_too() {
    let mut doc = Document::blank(A4, 1);
    let id = doc.add(item("Cancelled", 25.0, 40.0)).unwrap();
    doc.mark_printed();
    doc.remove(id).unwrap();

    let problems = doc.overlay_problems();
    assert_eq!(problems.len(), 1);
    assert_eq!(problems[0].what, Change::Deleted);
    assert!(problems[0].format().contains("has been deleted"));
}

#[test]
fn restyling_printed_words_is_refused_too() {
    // A different size or colour prints a second copy over the first, which is
    // not the same thing as changing it.
    let mut doc = Document::blank(A4, 1);
    let id = doc.add(item("Total", 25.0, 40.0)).unwrap();
    doc.mark_printed();
    doc.get_mut(id).unwrap().size_pt = 14.0;

    assert_eq!(doc.overlay_problems()[0].what, Change::Restyled);
}

#[test]
fn adding_words_is_never_a_problem() {
    // The case the whole program exists for: the sheet is untouched, and the
    // new words go in a gap.
    let mut doc = Document::blank(A4, 1);
    doc.add(item("Purchase order 4471", 25.0, 40.0)).unwrap();
    doc.mark_printed();
    doc.add(item("Approved", 25.0, 200.0)).unwrap();

    assert!(doc.overlay_problems().is_empty());
    assert_eq!(doc.delta_layout(None).unwrap()[0].len(), 1);
}

#[test]
fn printing_again_resets_what_counts_as_new() {
    let mut doc = Document::blank(A4, 1);
    doc.add(item("first", 25.0, 40.0)).unwrap();
    doc.mark_printed();
    doc.add(item("second", 25.0, 60.0)).unwrap();
    doc.mark_printed();

    assert!(doc.added_since_printing().is_empty());
    doc.add(item("third", 25.0, 80.0)).unwrap();
    assert_eq!(doc.added_since_printing()[0].text, "third");
}

#[test]
fn the_printed_record_survives_a_save() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("letter.onionskin");

    let mut doc = Document::blank(A4, 1);
    doc.add(item("Purchase order 4471", 25.0, 40.0)).unwrap();
    doc.mark_printed();
    doc.add(item("Approved", 25.0, 200.0)).unwrap();
    doc.save(&path).unwrap();

    let opened = Document::load(&path).unwrap();
    assert!(opened.has_been_printed());
    assert_eq!(opened.added_since_printing().len(), 1);
    assert!(opened.overlay_problems().is_empty());
}

#[test]
fn several_edits_are_all_reported_not_just_the_first() {
    let mut doc = Document::blank(A4, 1);
    let a = doc.add(item("one", 25.0, 40.0)).unwrap();
    let b = doc.add(item("two", 25.0, 50.0)).unwrap();
    let c = doc.add(item("three", 25.0, 60.0)).unwrap();
    doc.mark_printed();

    doc.get_mut(a).unwrap().x_mm = 30.0;
    doc.get_mut(b).unwrap().text = "TWO".into();
    doc.remove(c).unwrap();

    let problems = doc.overlay_problems();
    assert_eq!(problems.len(), 3, "{problems:?}");
    assert!(problems.iter().any(|p| p.what == Change::Moved));
    assert!(problems.iter().any(|p| p.what == Change::Reworded));
    assert!(problems.iter().any(|p| p.what == Change::Deleted));
}

// ---------------------------------------------------------------------------
// Writing it out
// ---------------------------------------------------------------------------

#[test]
fn a_document_becomes_a_pdf_at_the_right_size() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("letter.pdf");

    let mut doc = Document::blank(A4, 2);
    doc.add(item("Dear Sir", 25.0, 40.0)).unwrap();

    crate::pdf::write_delta(
        &path,
        &doc.page_sizes(),
        &doc.layout(None).unwrap(),
        "letter",
        None,
    )
    .unwrap();

    let written = lopdf::Document::load(&path).unwrap();
    assert_eq!(written.get_pages().len(), 2);
}

// ---------------------------------------------------------------------------
// Colours
// ---------------------------------------------------------------------------

#[test]
fn a_hex_colour_reads_each_channel_separately() {
    let (r, g, b) = parse_colour("#1a2b3c").unwrap();
    assert_eq!(r, 0x1a as f64 / 255.0, "red channel wrong: got {r}");
    assert_eq!(g, 0x2b as f64 / 255.0, "green channel wrong: got {g}");
    assert_eq!(b, 0x3c as f64 / 255.0, "blue channel wrong: got {b}");
}

#[test]
fn the_rgb_shorthand_repeats_each_digit_rather_than_just_padding_it() {
    // #abc is shorthand for #aabbcc: the digit is repeated (0xa -> 0xaa), not
    // shifted into the top nibble (0xa -> 0xa0). The two give visibly
    // different colours, so a box someone drew as "#abc" must come out
    // identical to one drawn as "#aabbcc".
    let short = parse_colour("#abc").unwrap();
    let long = parse_colour("#aabbcc").unwrap();
    assert_eq!(short, long, "#abc did not expand to #aabbcc: got {short:?}");
}

#[test]
fn fff_shorthand_is_exactly_white_not_fifteen_sixteenths_of_it() {
    // 0xf repeated is 0xff = 255 = full white (multiply by 17). Multiplying by
    // 16 instead (a common off-by-one for this exact trick) gives 0xf0 = 240,
    // i.e. 0.9375 — a white that is a shade of grey by the time it is printed.
    let white = parse_colour("#fff").unwrap();
    assert_eq!(
        white,
        (1.0, 1.0, 1.0),
        "#fff should be pure white, got {white:?}"
    );
}

#[test]
fn every_named_colour_parses_to_its_documented_value() {
    // These names exist so nobody has to look up a hex triple to draw a red
    // box; if one silently mapped to the wrong numbers the box would be the
    // wrong colour with no error to catch it.
    let cases = [
        ("black", (0.0, 0.0, 0.0)),
        ("white", (1.0, 1.0, 1.0)),
        ("grey", (0.5, 0.5, 0.5)),
        ("gray", (0.5, 0.5, 0.5)),
        ("lightgrey", (0.85, 0.85, 0.85)),
        ("lightgray", (0.85, 0.85, 0.85)),
        ("red", (0.8, 0.0, 0.0)),
        ("green", (0.0, 0.5, 0.0)),
        ("blue", (0.0, 0.0, 0.8)),
        ("yellow", (1.0, 0.85, 0.0)),
        ("orange", (0.95, 0.5, 0.0)),
    ];
    for (name, expected) in cases {
        let got = parse_colour(name).unwrap();
        assert_eq!(
            got, expected,
            "{name:?} parsed as {got:?}, expected {expected:?}"
        );
    }
}

#[test]
fn colour_names_and_hex_digits_do_not_care_about_letter_case() {
    // A colour is as likely to be typed with caps lock on or pasted from
    // somewhere that upper-cased it as not; refusing it on case alone would be
    // a needless surprise.
    assert_eq!(
        parse_colour("RED").unwrap(),
        parse_colour("red").unwrap(),
        "RED and red should be the same colour"
    );
    assert_eq!(
        parse_colour("#FF0000").unwrap(),
        parse_colour("#ff0000").unwrap(),
        "#FF0000 and #ff0000 should be the same colour"
    );
    assert_eq!(
        parse_colour("#ABC").unwrap(),
        parse_colour("#abc").unwrap(),
        "#ABC and #abc should be the same colour"
    );
}

#[test]
fn a_stray_space_around_a_colour_is_ignored() {
    // Colours arrive from the command line or a config file someone hand
    // edited, and it is easy to leave a space in either without noticing.
    assert_eq!(
        parse_colour(" red").unwrap(),
        parse_colour("red").unwrap(),
        "a leading space changed the colour"
    );
    assert_eq!(
        parse_colour("red ").unwrap(),
        parse_colour("red").unwrap(),
        "a trailing space changed the colour"
    );
    assert_eq!(
        parse_colour(" #abc123 ").unwrap(),
        parse_colour("#abc123").unwrap(),
        "surrounding spaces changed a hex colour"
    );
}

#[test]
fn a_colour_that_is_neither_a_name_nor_valid_hex_is_refused_and_named() {
    // Whoever typed the rubbish colour needs to see exactly what they typed —
    // not a generic complaint — plus be told the syntax that would work,
    // otherwise the error is a dead end rather than something they can fix.
    for bad in ["reddish", "#12345", "#gggggg", "notacolour", "#12"] {
        let err = parse_colour(bad).unwrap_err().to_string();
        assert!(
            err.contains(&format!("{bad:?}")),
            "error for {bad:?} did not name what was typed: {err}"
        );
        assert!(
            err.contains("#rrggbb"),
            "error for {bad:?} did not say how to write a colour: {err}"
        );
    }
}

// ---------------------------------------------------------------------------
// Drawing: ids are shared with text and never reused
// ---------------------------------------------------------------------------

#[test]
fn text_and_drawings_draw_from_one_sequence_of_ids_with_none_repeated() {
    // A printed record refers to both items and shapes purely by number. If
    // text and drawings each kept their own counter, a piece of text and a
    // shape could end up sharing an id, and looking one up in the printed
    // record would not say which kind it meant.
    let mut doc = Document::blank(A4, 1);
    let t1 = doc.add(item("one", 10.0, 10.0)).unwrap();
    let s1 = doc.draw(line(0.0, 0.0, 10.0, 10.0)).unwrap();
    let t2 = doc.add(item("two", 10.0, 20.0)).unwrap();
    let s2 = doc.draw(line(0.0, 0.0, 20.0, 20.0)).unwrap();

    let ids = [t1, s1, t2, s2];
    for i in 0..ids.len() {
        for j in (i + 1)..ids.len() {
            assert_ne!(ids[i], ids[j], "ids {ids:?} were not all distinct");
        }
    }
}

#[test]
fn erasing_a_drawing_does_not_free_its_id_for_reuse() {
    // The same reasoning as text: an erased shape's number must never come
    // back, whether it is handed to a new drawing or a new piece of text.
    let mut doc = Document::blank(A4, 1);
    let first = doc.draw(line(0.0, 0.0, 10.0, 10.0)).unwrap();
    doc.erase_shape(first).unwrap();

    let second_shape = doc.draw(line(5.0, 5.0, 15.0, 15.0)).unwrap();
    assert_ne!(
        first, second_shape,
        "erasing {first} let it be handed straight back out"
    );

    let text = doc.add(item("after erasing", 10.0, 10.0)).unwrap();
    assert_ne!(text, first, "text reused an erased drawing's id");
}

// ---------------------------------------------------------------------------
// Drawing: what is new since it was printed
// ---------------------------------------------------------------------------

#[test]
fn before_printing_every_drawing_counts_as_new() {
    let mut doc = Document::blank(A4, 1);
    doc.draw(line(0.0, 0.0, 10.0, 10.0)).unwrap();
    doc.draw(line(0.0, 0.0, 20.0, 20.0)).unwrap();

    assert_eq!(
        doc.shapes_added_since_printing().len(),
        2,
        "{:?}",
        doc.shapes_added_since_printing()
    );
}

#[test]
fn after_printing_no_drawing_is_new_until_one_is_added() {
    let mut doc = Document::blank(A4, 1);
    doc.draw(line(0.0, 0.0, 10.0, 10.0)).unwrap();
    doc.mark_printed();

    assert!(
        doc.shapes_added_since_printing().is_empty(),
        "a freshly printed drawing was reported as new: {:?}",
        doc.shapes_added_since_printing()
    );

    let fresh = doc.draw(line(5.0, 5.0, 15.0, 15.0)).unwrap();
    let added = doc.shapes_added_since_printing();
    assert_eq!(added.len(), 1, "{added:?}");
    assert_eq!(
        added[0].id, fresh,
        "the delta contained the wrong drawing: {:?}",
        added[0]
    );
}

// ---------------------------------------------------------------------------
// Drawing: a document older than drawing support
// ---------------------------------------------------------------------------

#[test]
fn a_document_saved_before_drawings_existed_still_loads_without_phantom_shapes() {
    // A file written by an older Onionskin has `printed` (it has been through
    // the press once) but no `printed_shapes` key and no `shapes` key at all —
    // not empty arrays, entirely absent, because those fields did not exist
    // yet. Treating a missing key as "unknown" rather than "empty" would
    // either refuse to load a perfectly good document or invent drawings that
    // were never there.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("old.onionskin");
    std::fs::write(
        &path,
        r#"{"page":{"width_mm":210,"height_mm":297},"pages":1,
            "items":[{"id":1,"page":1,"x_mm":10,"y_mm":10,"text":"x","size_pt":11,"font":"Helvetica"}],
            "printed":[{"id":1,"page":1,"x_mm":10,"y_mm":10,"text":"x","size_pt":11,"font":"Helvetica"}]}"#,
    )
    .unwrap();

    let doc = Document::load(&path).unwrap();
    assert!(
        doc.shapes.is_empty(),
        "an old document grew drawings out of nowhere: {:?}",
        doc.shapes
    );
    assert!(
        doc.shapes_added_since_printing().is_empty(),
        "an old document claimed phantom new drawings: {:?}",
        doc.shapes_added_since_printing()
    );
}

// ---------------------------------------------------------------------------
// Drawing: saving and re-opening
// ---------------------------------------------------------------------------

#[test]
fn a_document_with_one_of_every_drawing_kind_survives_a_save() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("drawings.onionskin");

    let mut doc = Document::blank(A4, 1);

    let mut a_line = line(10.0, 10.0, 50.0, 50.0);
    a_line.fill = Some("#00ff00".into());
    a_line.width_mm = 2.0;
    a_line.dash_mm = Some((4.0, 2.0));
    doc.draw(a_line).unwrap();

    let mut rect = shape(ShapeKind::Rect {
        x_mm: 10.0,
        y_mm: 10.0,
        width_mm: 30.0,
        height_mm: 20.0,
        radius_mm: 3.0,
    });
    rect.fill = Some("#0000ff".into());
    rect.width_mm = 1.5;
    doc.draw(rect).unwrap();

    let mut ellipse = shape(ShapeKind::Ellipse {
        x_mm: 50.0,
        y_mm: 50.0,
        radius_x_mm: 10.0,
        radius_y_mm: 5.0,
    });
    ellipse.fill = Some("#ffff00".into());
    doc.draw(ellipse).unwrap();

    let mut a_path = shape(ShapeKind::Path {
        points: vec![(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)],
        closed: true,
    });
    a_path.fill = Some("#ff00ff".into());
    a_path.dash_mm = Some((1.0, 1.0));
    doc.draw(a_path).unwrap();

    doc.save(&path).unwrap();
    let opened = Document::load(&path).unwrap();

    assert_eq!(opened, doc, "a drawing changed on the way through a file");
}

// ---------------------------------------------------------------------------
// Drawing: bounds
// ---------------------------------------------------------------------------

#[test]
fn a_lines_bounds_extend_half_its_width_past_each_end() {
    // A pen 2 mm wide does not draw an infinitely thin line: the ink actually
    // laid down extends 1 mm beyond each endpoint and to each side. Reporting
    // the bare endpoints as the bounds would understate how much of the page
    // the drawing actually covers.
    let mut l = line(0.0, 0.0, 10.0, 0.0);
    l.width_mm = 2.0;
    let bounds = l.bounds();
    assert_eq!(
        bounds,
        (-1.0, -1.0, 11.0, 1.0),
        "a 2 mm line from (0,0) to (10,0) should have bounds (-1,-1,11,1), got {bounds:?}"
    );
}

#[test]
fn a_boxs_bounds_cover_its_corners_plus_its_line_width() {
    let mut r = shape(ShapeKind::Rect {
        x_mm: 10.0,
        y_mm: 20.0,
        width_mm: 30.0,
        height_mm: 40.0,
        radius_mm: 0.0,
    });
    r.width_mm = 2.0;
    let bounds = r.bounds();
    assert_eq!(
        bounds,
        (9.0, 19.0, 41.0, 61.0),
        "a 30x40 box at (10,20) with a 2 mm outline should have bounds (9,19,41,61), got {bounds:?}"
    );
}

#[test]
fn an_ellipses_bounds_are_its_centre_plus_and_minus_its_radius() {
    let mut e = shape(ShapeKind::Ellipse {
        x_mm: 100.0,
        y_mm: 50.0,
        radius_x_mm: 20.0,
        radius_y_mm: 8.0,
    });
    e.width_mm = 0.0; // isolate the radius from the line-width padding
    let bounds = e.bounds();
    assert_eq!(
        bounds,
        (80.0, 42.0, 120.0, 58.0),
        "an ellipse centred at (100,50) with radii (20,8) should have bounds \
         (80,42,120,58), got {bounds:?}"
    );
}

#[test]
fn a_paths_bounds_cover_every_point_it_visits() {
    let mut p = shape(ShapeKind::Path {
        points: vec![(5.0, 40.0), (30.0, 5.0), (12.0, 20.0)],
        closed: false,
    });
    p.width_mm = 0.0;
    let bounds = p.bounds();
    assert_eq!(
        bounds,
        (5.0, 5.0, 30.0, 40.0),
        "the bounds should span the extremes of all three points, got {bounds:?}"
    );
}

// ---------------------------------------------------------------------------
// Drawing: what check refuses
// ---------------------------------------------------------------------------

#[test]
fn a_drawing_with_no_outline_and_no_fill_is_refused() {
    // With neither stroke nor fill nothing would appear on the printed page —
    // it would be a shape that exists in the document but not on the paper,
    // which is exactly the kind of silent nothing this format is meant to
    // avoid.
    let mut doc = Document::blank(A4, 1);
    let mut invisible = line(0.0, 0.0, 10.0, 10.0);
    invisible.stroke = None;
    invisible.fill = None;
    let err = doc.draw(invisible).unwrap_err().to_string();
    assert!(err.contains("neither an outline nor a fill"), "{err}");
}

#[test]
fn a_negative_line_width_is_refused() {
    // A negative width is not a thickness at all; letting it through would
    // also flip the sign of the padding bounds() applies.
    let mut doc = Document::blank(A4, 1);
    let mut bad = line(0.0, 0.0, 10.0, 10.0);
    bad.width_mm = -1.0;
    let err = doc.draw(bad).unwrap_err().to_string();
    assert!(err.contains("which is not a width"), "{err}");
    assert!(
        err.contains("-1"),
        "the width itself should be named: {err}"
    );
}

#[test]
fn a_non_finite_line_width_is_refused() {
    // NaN and infinity both need an explicit finiteness check: NaN fails
    // every ordinary comparison (so a bare "< 0.0" guard would let it slip
    // through as neither negative nor positive), and infinity is not a real
    // thickness either.
    for bad_width in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        let mut doc = Document::blank(A4, 1);
        let mut bad = line(0.0, 0.0, 10.0, 10.0);
        bad.width_mm = bad_width;
        let err = doc.draw(bad).unwrap_err().to_string();
        assert!(
            err.contains("which is not a width"),
            "width {bad_width}: {err}"
        );
    }
}

#[test]
fn a_drawing_with_an_unparsable_colour_is_refused() {
    let mut doc = Document::blank(A4, 1);
    let mut bad = line(0.0, 0.0, 10.0, 10.0);
    bad.stroke = Some("marmalade".into());
    let err = doc.draw(bad).unwrap_err().to_string();
    assert!(err.contains("marmalade"), "{err}");
    assert!(err.contains("#rrggbb"), "{err}");
}

#[test]
fn a_path_of_fewer_than_two_points_is_refused() {
    // A single point has no line to draw between; allowing it would store a
    // "drawing" that renders as nothing while claiming to be one.
    let mut doc = Document::blank(A4, 1);
    let lone = shape(ShapeKind::Path {
        points: vec![(5.0, 5.0)],
        closed: false,
    });
    let err = doc.draw(lone).unwrap_err().to_string();
    assert!(err.contains("takes two to"), "{err}");
    assert!(err.contains("1 point"), "{err}");
}

#[test]
fn a_drawing_on_a_page_the_document_does_not_have_is_refused() {
    // Unlike `add`/`draw`, which grow the document to fit, a document loaded
    // from disk might have been hand-edited (or come from an older, buggier
    // version) to reference a page count that does not match its content.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bad.onionskin");

    let mut doc = Document::blank(A4, 1);
    let mut stray = line(0.0, 0.0, 10.0, 10.0);
    stray.id = 9;
    stray.page = 4;
    doc.shapes.push(stray);
    std::fs::write(&path, serde_json::to_string(&doc).unwrap()).unwrap();

    let err = Document::load(&path).unwrap_err().to_string();
    assert!(err.contains("drawing 9 is on page 4"), "{err}");
    assert!(err.contains("1 pages"), "{err}");
}

// ---------------------------------------------------------------------------
// Drawing: laying out and erasing
// ---------------------------------------------------------------------------

#[test]
fn shape_layout_places_every_drawing_on_its_own_page_and_drops_nothing() {
    let mut doc = Document::blank(A4, 3);
    let mut first = line(0.0, 0.0, 10.0, 10.0);
    first.page = 1;
    doc.draw(first).unwrap();
    let mut second = shape(ShapeKind::Rect {
        x_mm: 0.0,
        y_mm: 0.0,
        width_mm: 5.0,
        height_mm: 5.0,
        radius_mm: 0.0,
    });
    second.page = 1;
    doc.draw(second).unwrap();
    let mut third = shape(ShapeKind::Ellipse {
        x_mm: 0.0,
        y_mm: 0.0,
        radius_x_mm: 5.0,
        radius_y_mm: 5.0,
    });
    third.page = 3;
    doc.draw(third).unwrap();
    // Page 2 is deliberately left with nothing on it.

    let all: Vec<&Shape> = doc.shapes.iter().collect();
    let layout = doc.shape_layout(&all);

    assert_eq!(
        layout.len(),
        3,
        "there should be one list per page, even an empty one: {layout:?}"
    );
    assert_eq!(
        layout[0].len(),
        2,
        "page one should keep both its drawings: {:?}",
        layout[0]
    );
    assert!(
        layout[1].is_empty(),
        "page two should have nothing drawn on it: {:?}",
        layout[1]
    );
    assert_eq!(
        layout[2].len(),
        1,
        "page three lost its drawing: {:?}",
        layout[2]
    );
}

#[test]
fn erasing_a_drawing_removes_only_that_one() {
    let mut doc = Document::blank(A4, 1);
    let keep = doc.draw(line(0.0, 0.0, 10.0, 10.0)).unwrap();
    let gone = doc.draw(line(20.0, 20.0, 30.0, 30.0)).unwrap();

    let erased = doc.erase_shape(gone).unwrap();
    assert_eq!(erased.id, gone, "erase_shape returned the wrong drawing");
    assert_eq!(doc.shapes.len(), 1, "{:?}", doc.shapes);
    assert_eq!(
        doc.shapes[0].id, keep,
        "erase_shape took the one that should have stayed: {:?}",
        doc.shapes[0]
    );
}

#[test]
fn erasing_a_drawing_that_is_not_there_says_so() {
    let mut doc = Document::blank(A4, 1);
    let err = doc.erase_shape(99).unwrap_err().to_string();
    assert!(err.contains("no item numbered 99"), "{err}");
}

// ---------------------------------------------------------------------------
// Drawing: describing itself
// ---------------------------------------------------------------------------

#[test]
fn a_shape_describes_itself_by_what_it_looks_like_not_just_its_stored_kind() {
    // A box and a rounded box are the same enum variant with a different
    // radius, and a circle is really an ellipse whose two radii happen to
    // match — describe() has to look past the variant name to tell these
    // apart, which is exactly the sort of thing that silently stops working
    // if the radius comparison is ever dropped or its threshold loosened too
    // far.
    let cases: Vec<(Shape, &str)> = vec![
        (line(0.0, 0.0, 1.0, 1.0), "line"),
        (
            shape(ShapeKind::Rect {
                x_mm: 0.0,
                y_mm: 0.0,
                width_mm: 10.0,
                height_mm: 10.0,
                radius_mm: 0.0,
            }),
            "box",
        ),
        (
            shape(ShapeKind::Rect {
                x_mm: 0.0,
                y_mm: 0.0,
                width_mm: 10.0,
                height_mm: 10.0,
                radius_mm: 2.0,
            }),
            "rounded box",
        ),
        (
            shape(ShapeKind::Ellipse {
                x_mm: 0.0,
                y_mm: 0.0,
                radius_x_mm: 5.0,
                radius_y_mm: 5.0,
            }),
            "circle",
        ),
        (
            shape(ShapeKind::Ellipse {
                x_mm: 0.0,
                y_mm: 0.0,
                radius_x_mm: 5.0,
                radius_y_mm: 8.0,
            }),
            "ellipse",
        ),
        (
            shape(ShapeKind::Path {
                points: vec![(0.0, 0.0), (1.0, 1.0)],
                closed: false,
            }),
            "path of 2 points",
        ),
        (
            shape(ShapeKind::Path {
                points: vec![(0.0, 0.0), (1.0, 1.0), (1.0, 0.0)],
                closed: true,
            }),
            "polygon of 3 points",
        ),
    ];
    for (s, expected) in cases {
        let got = s.describe();
        assert_eq!(
            got, expected,
            "{:?} described itself as {got:?}, expected {expected:?}",
            s.kind
        );
    }
}

// ---------------------------------------------------------------------------
// Undo
// ---------------------------------------------------------------------------

fn a_document(dir: &std::path::Path, words: &str) -> PathBuf {
    let path = dir.join("d.onionskin");
    let mut doc = Document::blank(A4, 1);
    doc.add(item(words, 25.0, 40.0)).unwrap();
    doc.save(&path).unwrap();
    path
}

#[test]
fn the_version_before_the_last_change_is_kept() {
    // `erase` takes a piece of text off a page, and there was no way back.
    let dir = tempfile::tempdir().unwrap();
    let path = a_document(dir.path(), "First");
    assert!(!can_undo(&path), "nothing has changed yet");

    let mut doc = Document::load(&path).unwrap();
    doc.remove(1).unwrap();
    doc.save(&path).unwrap();
    assert!(can_undo(&path), "the change left nothing to go back to");
    assert_eq!(Document::load(&path).unwrap().items.len(), 0);

    undo(&path).unwrap();
    let back = Document::load(&path).unwrap();
    assert_eq!(back.items.len(), 1);
    assert_eq!(back.items[0].text, "First");
}

#[test]
fn an_undo_can_itself_be_undone() {
    // Somebody who goes back one step too many should not have to redo the
    // work by hand. It is `redo` that does it now, not a second `undo`:
    // undoing twice used to return you to where you started, which meant
    // three mistakes could not be undone at all.
    let dir = tempfile::tempdir().unwrap();
    let path = a_document(dir.path(), "First");
    let mut doc = Document::load(&path).unwrap();
    doc.remove(1).unwrap();
    doc.save(&path).unwrap();

    undo(&path).unwrap();
    assert_eq!(Document::load(&path).unwrap().items.len(), 1);
    redo(&path).unwrap();
    assert_eq!(Document::load(&path).unwrap().items.len(), 0);
    undo(&path).unwrap();
    assert_eq!(Document::load(&path).unwrap().items.len(), 1);
}

#[test]
fn going_back_three_times_goes_back_three_steps() {
    // The reason the swap had to go. Three mistakes in a row could not be
    // undone at all: the second `undo` put the first one back.
    let dir = tempfile::tempdir().unwrap();
    let path = a_document(dir.path(), "One");
    for words in ["Two", "Three", "Four"] {
        let mut doc = Document::load(&path).unwrap();
        doc.add(item(words, 25.0, 60.0)).unwrap();
        doc.save(&path).unwrap();
    }
    assert_eq!(Document::load(&path).unwrap().items.len(), 4);

    for expected in [3, 2, 1] {
        undo(&path).unwrap();
        assert_eq!(Document::load(&path).unwrap().items.len(), expected);
    }
    // And forward again, as far as it went back.
    for expected in [2, 3, 4] {
        redo(&path).unwrap();
        assert_eq!(Document::load(&path).unwrap().items.len(), expected);
    }
}

#[test]
fn editing_after_an_undo_forgets_what_could_have_been_redone() {
    // Once a new edit is made, the versions that were undone are not anywhere
    // the document can get back to. Offering to redo into a history that has
    // been departed from would hand somebody a document that never existed.
    let dir = tempfile::tempdir().unwrap();
    let path = a_document(dir.path(), "One");
    let mut doc = Document::load(&path).unwrap();
    doc.add(item("Two", 25.0, 60.0)).unwrap();
    doc.save(&path).unwrap();

    undo(&path).unwrap();
    assert_eq!(steps_forward(&path), 1);

    let mut doc = Document::load(&path).unwrap();
    doc.add(item("Something else", 25.0, 80.0)).unwrap();
    doc.save(&path).unwrap();
    assert_eq!(steps_forward(&path), 0);
    assert!(redo(&path).is_err());
}

#[test]
fn a_history_that_is_not_there_is_said_plainly_rather_than_guessed_at() {
    let dir = tempfile::tempdir().unwrap();
    let path = a_document(dir.path(), "One");
    assert_eq!(steps_back(&path), 0);
    assert_eq!(steps_forward(&path), 0);
    assert!(undo(&path)
        .unwrap_err()
        .to_string()
        .contains("nothing to undo"));
    assert!(redo(&path)
        .unwrap_err()
        .to_string()
        .contains("nothing to redo"));
}

#[test]
fn the_history_stops_rather_than_filling_the_folder() {
    // Ten steps, because the mistake somebody wants undone is nearly always
    // the last one or the one before it, and a folder holding fifty copies of
    // a letter is its own kind of mess.
    let dir = tempfile::tempdir().unwrap();
    let path = a_document(dir.path(), "One");
    for n in 0..STEPS_KEPT + 5 {
        let mut doc = Document::load(&path).unwrap();
        doc.add(item(&format!("change {n}"), 25.0, 60.0)).unwrap();
        doc.save(&path).unwrap();
    }

    assert_eq!(steps_back(&path), STEPS_KEPT);
    let kept: Vec<_> = std::fs::read_dir(dir.path())
        .unwrap()
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    // The document itself, and no more history than was promised.
    assert_eq!(kept.len(), STEPS_KEPT + 1, "{kept:?}");

    // And every one of those steps really goes back, rather than merely
    // existing as a file.
    for _ in 0..STEPS_KEPT {
        undo(&path).unwrap();
    }
    assert!(undo(&path).is_err(), "went back further than it kept");
}

#[test]
fn undoing_a_document_that_never_changed_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let path = a_document(dir.path(), "First");
    let said = undo(&path).unwrap_err().to_string();
    assert!(said.contains("nothing to undo"), "{said}");
    // And the document is untouched.
    assert_eq!(Document::load(&path).unwrap().items.len(), 1);
}

#[test]
fn the_kept_copy_sits_beside_the_document_it_belongs_to() {
    // In the same folder and named after it, so it is obvious what it is and
    // obvious that deleting it is safe.
    let dir = tempfile::tempdir().unwrap();
    let path = a_document(dir.path(), "First");
    let mut doc = Document::load(&path).unwrap();
    doc.remove(1).unwrap();
    doc.save(&path).unwrap();

    let beside = dir.path().join("d.onionskin.before");
    assert!(
        beside.is_file(),
        "{:?}",
        std::fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|e| e.file_name())
            .collect::<Vec<_>>()
    );
}

#[test]
fn a_document_is_known_by_what_is_in_it_not_what_it_is_called() {
    // Somebody who calls their document letter.pdf has made a document called
    // letter.pdf. Every command that decided by the extension used to open it
    // expecting a PDF and report it damaged — a file Onionskin wrote itself,
    // one command earlier.
    let dir = tempfile::tempdir().unwrap();
    for name in [
        "letter.pdf",
        "letter.docx",
        "letter.odt",
        "letter",
        "l.onion",
    ] {
        let path = dir.path().join(name);
        Document::blank(A4, 1).save(&path).unwrap();
        assert!(Document::is_one(&path), "{name} was not recognised");
        assert!(Document::load(&path).is_ok(), "{name} would not load");
    }
}

#[test]
fn a_real_pdf_or_word_file_is_never_mistaken_for_a_document() {
    // The other direction matters more: mistaking somebody's Word file for an
    // Onionskin document would edit it in place, and their file is theirs.
    let dir = tempfile::tempdir().unwrap();
    for (name, magic) in [
        ("real.pdf", &b"%PDF-1.7\n"[..]),
        ("real.docx", &b"PK\x03\x04"[..]),
        ("real.png", &b"\x89PNG\r\n\x1a\n"[..]),
        ("empty.onion", &b""[..]),
    ] {
        let path = dir.path().join(name);
        std::fs::write(&path, magic).unwrap();
        assert!(!Document::is_one(&path), "{name} was wrongly claimed");
    }
    assert!(!Document::is_one(&dir.path().join("not-there-at-all")));
}

#[test]
fn a_broken_document_is_still_a_document() {
    // So the complaint is that the document is broken, which is true and
    // fixable, rather than that the PDF is — which is neither.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("half-written.pdf");
    std::fs::write(&path, b"\n  {\"page\": {\"width_mm\": 210.0,").unwrap();
    assert!(Document::is_one(&path));
    assert!(matches!(
        Document::load(&path),
        Err(DocumentError::Malformed { .. })
    ));
}
