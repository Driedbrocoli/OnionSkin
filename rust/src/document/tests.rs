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
