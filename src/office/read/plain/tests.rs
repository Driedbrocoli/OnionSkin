use super::*;
use crate::office::read::Block;

/// The paragraphs of a sheet, as plain text.
fn lines(sheet: &Sheet) -> Vec<String> {
    sheet
        .blocks
        .iter()
        .map(|block| match block {
            Block::Para(para) => para.plain_text(),
            Block::Table(_) => String::new(),
        })
        .collect()
}

#[test]
fn every_line_is_a_paragraph() {
    let sheet = read("one\ntwo\nthree\n", "txt");
    assert_eq!(lines(&sheet), vec!["one", "two", "three"]);
}

#[test]
fn a_blank_line_stays_blank() {
    // A blank line is how somebody separates two thoughts, and closing it up
    // would rewrite what they wrote.
    let sheet = read("one\n\ntwo\n", "txt");
    assert_eq!(lines(&sheet), vec!["one", "", "two"]);
}

#[test]
fn windows_line_endings_do_not_leave_a_carriage_return_behind() {
    // A stray carriage return has no glyph, and prints as an empty box.
    let sheet = read("one\r\ntwo\r\n", "txt");
    assert_eq!(lines(&sheet), vec!["one", "two"]);
    assert!(!sheet.text().contains('\r'));
}

#[test]
fn a_file_with_no_final_newline_keeps_its_last_line() {
    let sheet = read("no newline at the end", "txt");
    assert_eq!(lines(&sheet), vec!["no newline at the end"]);
}

#[test]
fn markdown_headings_are_set_larger() {
    let sheet = read("# Title\n## Smaller\nbody\n", "md");
    let Block::Para(title) = &sheet.blocks[0] else {
        panic!("expected a paragraph");
    };
    let Block::Para(smaller) = &sheet.blocks[1] else {
        panic!("expected a paragraph");
    };
    let Block::Para(body) = &sheet.blocks[2] else {
        panic!("expected a paragraph");
    };
    assert_eq!(title.plain_text(), "Title");
    assert!(title.style.size_pt > smaller.style.size_pt);
    assert!(smaller.style.size_pt > body.style.size_pt);
    assert!(title.style.bold);
    assert!(!body.style.bold);
}

#[test]
fn a_hash_is_only_a_heading_in_markdown() {
    let sheet = read("# Title\n", "txt");
    let Block::Para(para) = &sheet.blocks[0] else {
        panic!("expected a paragraph");
    };
    assert_eq!(para.plain_text(), "# Title");
    assert!(!para.style.bold);
}

#[test]
fn a_hash_with_no_space_after_it_is_not_a_heading() {
    // `#hashtag` is a word, not a heading, which is the rule every Markdown
    // reader follows.
    let sheet = read("#hashtag\n", "md");
    assert_eq!(lines(&sheet), vec!["#hashtag"]);
}

#[test]
fn markdown_bullets_get_a_marker_and_an_indent() {
    let sheet = read("- first\n* second\n+ third\n", "md");
    for block in &sheet.blocks {
        let Block::Para(para) = block else {
            panic!("expected a paragraph");
        };
        assert_eq!(para.marker.as_deref(), Some("\u{2022}"));
        assert!(para.indent_left_mm > 0.0);
        // The wrapped text has to clear the bullet.
        assert!(para.first_line_mm < 0.0);
    }
    assert_eq!(lines(&sheet), vec!["first", "second", "third"]);
}

#[test]
fn a_markdown_table_is_mentioned_rather_than_drawn() {
    let sheet = read("| a | b |\n|---|---|\n| 1 | 2 |\n", "md");
    assert!(
        sheet.notes.iter().any(|note| note.contains("table")),
        "{:?}",
        sheet.notes
    );
    // Said once, however many rows there are.
    assert_eq!(sheet.notes.len(), 1);
}

#[test]
fn the_paper_is_a4() {
    let sheet = read("anything", "txt");
    assert!((sheet.page.width_mm - 210.0).abs() < 1e-9);
    assert!((sheet.page.height_mm - 297.0).abs() < 1e-9);
}

#[test]
fn an_empty_file_is_an_empty_page_rather_than_an_error() {
    let sheet = read("", "txt");
    assert!(sheet.blocks.is_empty());
    assert!(sheet.text().trim().is_empty());
}
