use std::collections::BTreeSet;
use std::path::Path;

use lopdf::content::{Content, Operation};
use lopdf::Document;
use tempfile::tempdir;

use super::*;
use crate::geometry::pt_to_mm;
use crate::office::read::{Margins, Row};
use crate::pdf::{self, Font};

// ---------------------------------------------------------------------------
// Building blocks shared by the tests below.
// ---------------------------------------------------------------------------

/// A4, with the same margins every reader gets when a document does not say.
fn sheet(blocks: Vec<Block>) -> Sheet {
    let mut sheet = Sheet::new(PageSize::new(210.0, 297.0), Margins::default());
    sheet.blocks = blocks;
    sheet
}

/// One left-aligned paragraph of plain text in the default style.
fn para(text: &str) -> Para {
    Para {
        pieces: vec![Piece::Text(text.to_string(), Style::default())],
        ..Para::default()
    }
}

/// A table cell holding one plain paragraph.
fn cell(text: &str) -> Cell {
    Cell {
        blocks: vec![Block::Para(para(text))],
        span: 1,
    }
}

/// A `Fonts` that needs nothing from the machine — the built-in faces cover
/// everything asked of them. Built directly rather than through `gather`,
/// because `gather` goes looking for a system font, and whether one is
/// installed is not something a test can rely on either way.
fn plain_fonts() -> Fonts {
    Fonts {
        embedded: None,
        dropped: BTreeSet::new(),
        wanted_a_font: false,
    }
}

/// Write a sheet to a temporary PDF with `layout::write` and load it back.
fn write_pdf(sheet: &Sheet) -> (Document, Vec<String>) {
    let dir = tempdir().expect("a temp dir for the test PDF");
    let path = dir.path().join("out.pdf");
    let notes = write(sheet, &path, Path::new("test.docx"))
        .expect("layout::write should turn this sheet into a PDF");
    let doc = Document::load(&path).expect("the PDF layout::write produced should load back");
    (doc, notes)
}

/// The operations in one page's content stream, in the order they were
/// written (pages are numbered from 1, as `lopdf::Document::get_pages` does).
fn ops(doc: &Document, page_number: u32) -> Vec<Operation> {
    let pages = doc.get_pages();
    let page_id = *pages.get(&page_number).unwrap_or_else(|| {
        panic!(
            "no page {page_number}; the document only has {} page(s)",
            pages.len()
        )
    });
    let bytes = doc
        .get_page_content(page_id)
        .expect("a page's content stream should be readable");
    Content::decode(&bytes)
        .expect("the content stream layout::write wrote should parse back")
        .operations
}

/// Every `Tj` string on a page, decoded and joined — enough to ask "does this
/// text appear on this page", without caring about position.
fn text_of(doc: &Document, page_number: u32) -> String {
    ops(doc, page_number)
        .into_iter()
        .filter(|op| op.operator == "Tj")
        .map(|op| {
            let bytes = op.operands[0]
                .as_str()
                .expect("a Tj operand should be a PDF string");
            String::from_utf8_lossy(bytes).into_owned()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// One piece of text as it actually landed on the page: where, and what was
/// handed to `Tj`. `x_mm`/`y_mm` are page space (top-left origin, y down),
/// converted back from the PDF points `crate::pdf` writes.
#[derive(Debug)]
struct Written {
    x_mm: f64,
    y_mm: f64,
    size_pt: f64,
    text: String,
}

fn written_text(doc: &Document, page_number: u32, page: &PageSize) -> Vec<Written> {
    let mut out = Vec::new();
    let mut size_pt = 0.0;
    let mut at: Option<(f64, f64)> = None;
    for op in ops(doc, page_number) {
        match op.operator.as_str() {
            "Tf" => {
                size_pt = op.operands[1]
                    .as_float()
                    .expect("a Tf operand should be a number") as f64;
            }
            "Td" => {
                let x = op.operands[0].as_float().expect("a Td x operand") as f64;
                let y = op.operands[1].as_float().expect("a Td y operand") as f64;
                at = Some((x, y));
            }
            "Tj" => {
                let (x_pt, y_pt) = at.expect("a Tj must always be preceded by a Td here");
                let bytes = op.operands[0]
                    .as_str()
                    .expect("a Tj operand should be a PDF string");
                out.push(Written {
                    x_mm: pt_to_mm(x_pt),
                    y_mm: page.height_mm - pt_to_mm(y_pt),
                    size_pt,
                    text: String::from_utf8_lossy(bytes).into_owned(),
                });
            }
            _ => {}
        }
    }
    out
}

/// Group placed lines by baseline: everything on one wrapped line shares a
/// `y_mm` exactly, and moving to a new line always changes it.
fn group_by_y(lines: &[PlacedLine]) -> Vec<Vec<&PlacedLine>> {
    let mut groups: Vec<Vec<&PlacedLine>> = Vec::new();
    for line in lines {
        match groups.last_mut() {
            Some(last) if (last[0].y_mm - line.y_mm).abs() < 1e-9 => last.push(line),
            _ => groups.push(vec![line]),
        }
    }
    groups
}

// ---------------------------------------------------------------------------
// 1. A short paragraph.
// ---------------------------------------------------------------------------

#[test]
fn a_short_paragraph_gives_a_one_page_pdf_with_the_text_in_it() {
    let (doc, _) = write_pdf(&sheet(vec![Block::Para(para("Hello, Onionskin."))]));
    let pages = doc.get_pages();
    assert_eq!(
        pages.len(),
        1,
        "a short paragraph must not need more than one page"
    );

    let page = PageSize::new(210.0, 297.0);
    let written = written_text(&doc, 1, &page);
    assert_eq!(
        written.len(),
        1,
        "one short, uniformly-styled line must be written as a single run, got {written:?}"
    );
    assert_eq!(written[0].text, "Hello, Onionskin.");
    // A thousandth of a millimetre, not a millionth: `crate::pdf` narrows
    // every coordinate to an f32 on its way into the file (see `real()` in
    // src/pdf.rs), so a value read back from the PDF is not bit-identical to
    // the f64 arithmetic that produced it — only extremely close to it.
    assert!(
        (written[0].x_mm - 20.0).abs() < 1e-3,
        "a left-aligned line with no indent must start at the left margin, got {}",
        written[0].x_mm
    );
}

// ---------------------------------------------------------------------------
// 2. Overflow onto a second page.
// ---------------------------------------------------------------------------

#[test]
fn a_long_paragraph_runs_onto_a_second_page() {
    // Comfortably more than a page of 11pt text in a 170mm column, whatever
    // the exact metrics turn out to be.
    let text = "pineapple ".repeat(3000);
    let (doc, _) = write_pdf(&sheet(vec![Block::Para(para(&text))]));
    let pages = doc.get_pages();
    assert!(
        pages.len() > 1,
        "expected the paragraph to overflow onto a second page, got {} page(s)",
        pages.len()
    );

    let page = PageSize::new(210.0, 297.0);
    let margins = Margins::default();
    let top_of_page_1 = written_text(&doc, 1, &page)
        .first()
        .expect("page 1 should have text on it")
        .y_mm;
    let top_of_page_2 = written_text(&doc, 2, &page)
        .first()
        .expect("page 2 should have text on it")
        .y_mm;

    // A thousandth of a millimetre: PDF coordinates are narrowed to f32 on
    // the way into the file (see `real()` in src/pdf.rs), so a value read
    // back is extremely close to, but not bit-identical to, the f64 that
    // produced it.
    assert!(
        (top_of_page_1 - top_of_page_2).abs() < 1e-3,
        "the first line of page 2 ({top_of_page_2:.3}mm) must start at the same height as \
         the first line of page 1 ({top_of_page_1:.3}mm)"
    );
    let expected = margins.top_mm + Style::default().size_pt * MM_PER_PT * ASCENT;
    assert!(
        (top_of_page_2 - expected).abs() < 1e-3,
        "expected the first line of page 2 at {expected:.3}mm (the top margin plus one \
         ascent), got {top_of_page_2:.3}mm"
    );
}

// ---------------------------------------------------------------------------
// 3. Word wrapping, measured exactly.
// ---------------------------------------------------------------------------

#[test]
fn word_wrapping_breaks_where_the_measured_widths_say_it_should() {
    let style = Style::default();
    let word = "aaaa";
    let word_w = pdf::builtin_width_mm(Font::Helvetica, word, style.size_pt);
    let space_w = pdf::builtin_width_mm(Font::Helvetica, " ", style.size_pt);
    // Comfortably fits three words and a space short of a fourth, whatever
    // the exact widths turn out to be — the slack is a quarter of a word,
    // far less than the width of a whole extra word plus its space.
    let column = 3.0 * word_w + 2.0 * space_w + word_w / 4.0;

    let text = [word; 10].join(" ");
    let ten_words = Para {
        pieces: vec![Piece::Text(text, style)],
        ..Para::default()
    };
    let mut fonts = plain_fonts();
    let pieces = fragments(&ten_words, &mut fonts);
    let lines = wrap(pieces, column, 0.0);

    assert_eq!(
        lines.len(),
        4,
        "10 words at 3 per line should wrap 3+3+3+1, got {} lines",
        lines.len()
    );
    let words_per_line: Vec<usize> = lines
        .iter()
        .map(|l| l.fragments.iter().filter(|f| !f.space).count())
        .collect();
    assert_eq!(words_per_line, vec![3, 3, 3, 1]);
}

// ---------------------------------------------------------------------------
// 4. A word wider than the column.
// ---------------------------------------------------------------------------

#[test]
fn a_word_wider_than_the_column_is_split_rather_than_running_off_the_paper() {
    let style = Style::default();
    let column = 30.0;
    let huge = "X".repeat(80);
    let fonts = plain_fonts();

    // The huge word is not the first thing on the paragraph's own line —
    // "Hi " comes first — which is exactly the case that must still split it.
    let pieces = vec![
        fragment("Hi".to_string(), style, &fonts, false),
        fragment(" ".to_string(), style, &fonts, true),
        fragment(huge.clone(), style, &fonts, false),
    ];
    let lines = wrap(pieces, column, 0.0);

    assert!(
        lines.len() >= 3,
        "an 80-character word on a 30mm column should need several lines, got {}",
        lines.len()
    );
    for (index, line) in lines.iter().enumerate() {
        assert!(
            line.width_mm <= column + 1e-6,
            "line {index} is {:.2}mm wide, wider than the {column:.2}mm column — the over-wide \
             word must have run off the paper instead of being split",
            line.width_mm
        );
    }
    // The space between "Hi" and the huge word falls exactly at the break
    // between line 1 and line 2, so it is dropped — the same as the space
    // that would trail any other wrapped line. Nothing of the word itself
    // may be lost, though.
    let recovered: String = lines
        .iter()
        .flat_map(|line| line.fragments.iter())
        .map(|f| f.text.as_str())
        .collect();
    assert_eq!(
        recovered,
        format!("Hi{huge}"),
        "no character may be lost while splitting"
    );
}

// ---------------------------------------------------------------------------
// 5. Centre and right alignment.
// ---------------------------------------------------------------------------

#[test]
fn centre_and_right_alignment_place_the_line_where_it_belongs() {
    fn placed_x(fonts: &mut Fonts, align: Align, text: &str, left_x: f64, column: f64) -> f64 {
        let mut pen = Pen::measuring(fonts);
        let para = Para {
            pieces: vec![Piece::Text(text.to_string(), Style::default())],
            align,
            ..Para::default()
        };
        pen.flow_para(&para, left_x, column);
        pen.leaves[0].lines[0].x_mm
    }

    let text = "Short line";
    let column = 100.0;
    let left_x = 20.0;

    let mut fonts = plain_fonts();
    let left_pos = placed_x(&mut fonts, Align::Left, text, left_x, column);
    let centre_pos = placed_x(&mut fonts, Align::Centre, text, left_x, column);
    let right_pos = placed_x(&mut fonts, Align::Right, text, left_x, column);

    // Measured the same way `finish_line` sums a line's width — fragment by
    // fragment — so this matches exactly rather than merely approximately.
    let size = Style::default().size_pt;
    let width = pdf::builtin_width_mm(Font::Helvetica, "Short", size)
        + pdf::builtin_width_mm(Font::Helvetica, " ", size)
        + pdf::builtin_width_mm(Font::Helvetica, "line", size);
    let free = column - width;

    assert!(
        (left_pos - left_x).abs() < 1e-6,
        "left align landed at {left_pos}"
    );
    assert!(
        (centre_pos - (left_x + free / 2.0)).abs() < 1e-6,
        "centre align landed at {centre_pos}"
    );
    assert!(
        (right_pos - (left_x + free)).abs() < 1e-6,
        "right align landed at {right_pos}"
    );
    assert!(
        left_pos < centre_pos && centre_pos < right_pos,
        "left ({left_pos}) < centre ({centre_pos}) < right ({right_pos}) must hold"
    );
}

// ---------------------------------------------------------------------------
// 6. Justification.
// ---------------------------------------------------------------------------

#[test]
fn justify_stretches_every_line_but_the_last() {
    let style = Style::default();
    let word = "hello";
    let word_w = pdf::builtin_width_mm(Font::Helvetica, word, style.size_pt);
    let space_w = pdf::builtin_width_mm(Font::Helvetica, " ", style.size_pt);
    let column = 6.0 * word_w + 5.0 * space_w + word_w / 4.0; // exactly 6 words/line
    let left_x = 20.0;
    let text = vec![word; 20].join(" "); // 20 words -> lines of 6, 6, 6, 2

    let mut fonts_j = plain_fonts();
    let mut pen_j = Pen::measuring(&mut fonts_j);
    pen_j.flow_para(
        &Para {
            pieces: vec![Piece::Text(text.clone(), style)],
            align: Align::Justify,
            ..Para::default()
        },
        left_x,
        column,
    );

    let mut fonts_l = plain_fonts();
    let mut pen_l = Pen::measuring(&mut fonts_l);
    pen_l.flow_para(
        &Para {
            pieces: vec![Piece::Text(text, style)],
            align: Align::Left,
            ..Para::default()
        },
        left_x,
        column,
    );

    let justified = group_by_y(&pen_j.leaves[0].lines);
    let ragged = group_by_y(&pen_l.leaves[0].lines);

    assert_eq!(
        justified.len(),
        4,
        "20 words at 6 per line should wrap into 4 lines"
    );
    assert_eq!(justified.len(), ragged.len());

    let right_margin = left_x + column;
    for (index, group) in justified.iter().enumerate() {
        let last_run = group
            .last()
            .expect("a wrapped line always has at least one run");
        let reach =
            last_run.x_mm + pdf::builtin_width_mm(Font::Helvetica, &last_run.text, style.size_pt);
        if index + 1 < justified.len() {
            assert!(
                (reach - right_margin).abs() < 1e-6,
                "justified line {index} reached {reach:.3}mm, expected the right margin at \
                 {right_margin:.3}mm"
            );
        } else {
            let ragged_last = ragged[index]
                .last()
                .expect("the ragged version has this line too");
            assert!(
                (last_run.x_mm - ragged_last.x_mm).abs() < 1e-6,
                "the last justified line must sit exactly where the ragged version does \
                 ({:.3}mm vs {:.3}mm) — it must not be stretched",
                last_run.x_mm,
                ragged_last.x_mm
            );
            assert!(
                reach < right_margin - 1.0,
                "the last line ({reach:.3}mm) must fall well short of the right margin \
                 ({right_margin:.3}mm), not be stretched to fill it"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// 7. Explicit breaks.
// ---------------------------------------------------------------------------

#[test]
fn an_explicit_page_break_starts_a_new_page_but_a_line_break_does_not() {
    let with_page_break = Para {
        pieces: vec![
            Piece::Text("Before".into(), Style::default()),
            Piece::PageBreak,
            Piece::Text("After".into(), Style::default()),
        ],
        ..Para::default()
    };
    let (doc, _) = write_pdf(&sheet(vec![Block::Para(with_page_break)]));
    let pages = doc.get_pages();
    assert_eq!(pages.len(), 2, "a Piece::PageBreak must start a new page");
    assert!(text_of(&doc, 1).contains("Before"));
    assert!(text_of(&doc, 2).contains("After"));

    let with_line_break = Para {
        pieces: vec![
            Piece::Text("Before".into(), Style::default()),
            Piece::LineBreak,
            Piece::Text("After".into(), Style::default()),
        ],
        ..Para::default()
    };
    let (doc2, _) = write_pdf(&sheet(vec![Block::Para(with_line_break)]));
    assert_eq!(
        doc2.get_pages().len(),
        1,
        "a Piece::LineBreak must not start a new page"
    );
    let text = text_of(&doc2, 1);
    assert!(text.contains("Before") && text.contains("After"));
}

// ---------------------------------------------------------------------------
// 8. break_before.
// ---------------------------------------------------------------------------

#[test]
fn break_before_starts_a_new_page_but_not_on_a_page_that_is_already_empty() {
    // Once something real has been drawn, break_before must push to a new page.
    let first = para("First paragraph");
    let mut second = para("Second paragraph");
    second.break_before = true;
    let (doc, _) = write_pdf(&sheet(vec![Block::Para(first), Block::Para(second)]));
    let pages = doc.get_pages();
    assert_eq!(
        pages.len(),
        2,
        "break_before after real content must start a new page"
    );
    assert!(text_of(&doc, 1).contains("First paragraph"));
    assert!(text_of(&doc, 2).contains("Second paragraph"));

    // On a page that is still blank, break_before must not conjure an empty
    // page in front of the very content it marks.
    let mut only = para("Only paragraph");
    only.break_before = true;
    let (doc, _) = write_pdf(&sheet(vec![Block::Para(only)]));
    assert_eq!(
        doc.get_pages().len(),
        1,
        "break_before on the first paragraph of a document must not leave a blank page in front"
    );
    assert!(text_of(&doc, 1).contains("Only paragraph"));
}

// ---------------------------------------------------------------------------
// 9. An empty paragraph still takes a line's height.
// ---------------------------------------------------------------------------

#[test]
fn an_empty_paragraph_still_takes_up_a_lines_height() {
    let mut fonts = plain_fonts();
    let mut pen = Pen::measuring(&mut fonts);
    pen.flow(
        &[Block::Para(para("First")), Block::Para(para("Second"))],
        20.0,
        170.0,
        0,
    );
    assert_eq!(pen.leaves[0].lines.len(), 2);
    let adjacent_gap = pen.leaves[0].lines[1].y_mm - pen.leaves[0].lines[0].y_mm;

    let mut fonts2 = plain_fonts();
    let mut pen2 = Pen::measuring(&mut fonts2);
    pen2.flow(
        &[
            Block::Para(para("First")),
            Block::Para(Para::default()),
            Block::Para(para("Second")),
        ],
        20.0,
        170.0,
        0,
    );
    assert_eq!(
        pen2.leaves[0].lines.len(),
        2,
        "an empty paragraph must draw no text of its own"
    );
    let spaced_gap = pen2.leaves[0].lines[1].y_mm - pen2.leaves[0].lines[0].y_mm;

    assert!(
        spaced_gap > adjacent_gap + 1e-6,
        "expected the empty paragraph to widen the gap beyond the adjacent-paragraph gap of \
         {adjacent_gap:.3}mm, got {spaced_gap:.3}mm"
    );

    let blank_line_height = Style::default().size_pt * MM_PER_PT * LEADING;
    assert!(
        (spaced_gap - adjacent_gap - blank_line_height).abs() < 1e-6,
        "expected exactly one blank line ({blank_line_height:.3}mm) of extra space, got \
         {:.3}mm",
        spaced_gap - adjacent_gap
    );
}

// ---------------------------------------------------------------------------
// 10. A list marker.
// ---------------------------------------------------------------------------

#[test]
fn a_list_marker_is_written_and_the_wrapped_text_clears_it() {
    let word = "wordword";
    let style = Style::default();
    let word_w = pdf::builtin_width_mm(Font::Helvetica, word, style.size_pt);
    let space_w = pdf::builtin_width_mm(Font::Helvetica, " ", style.size_pt);
    let indent_left = 10.0;
    // Exactly two words per line, so four words wrap into two lines of two.
    let column = 2.0 * word_w + space_w + word_w / 4.0;
    let left_x = 20.0;

    let with_marker = Para {
        pieces: vec![Piece::Text([word; 4].join(" "), style)],
        marker: Some("1.".to_string()),
        indent_left_mm: indent_left,
        first_line_mm: -indent_left,
        align: Align::Left,
        ..Para::default()
    };

    let mut fonts = plain_fonts();
    let mut pen = Pen::measuring(&mut fonts);
    // `width_mm` here is the outer column, before the paragraph's own
    // indent is subtracted from it inside `flow_para`.
    pen.flow_para(&with_marker, left_x, column + indent_left);

    let by_line = group_by_y(&pen.leaves[0].lines);
    assert_eq!(
        by_line.len(),
        2,
        "four words at two per line must wrap onto two lines"
    );

    let marker_run = by_line[0]
        .iter()
        .find(|l| l.text.trim() == "1.")
        .unwrap_or_else(|| {
            panic!(
                "the marker must be written on the first line: {:?}",
                by_line[0]
            )
        });
    assert!(
        (marker_run.x_mm - left_x).abs() < 1e-6,
        "the marker must sit in the hanging indent, at the paragraph's outer edge, got {}",
        marker_run.x_mm
    );

    let first_line_body = by_line[0]
        .iter()
        .find(|l| l.text.trim() != "1.")
        .expect("the first line must have text besides the marker");
    let second_line = by_line[1]
        .first()
        .expect("the second line must have a run of its own");

    assert!(
        (first_line_body.x_mm - (left_x + indent_left)).abs() < 1e-6,
        "the first line's text must start at the normal indent, clear of the marker, got {}",
        first_line_body.x_mm
    );
    assert!(
        (second_line.x_mm - first_line_body.x_mm).abs() < 1e-6,
        "the wrapped line ({}) must line up with the first line's text ({}), not the marker",
        second_line.x_mm,
        first_line_body.x_mm
    );
}

// ---------------------------------------------------------------------------
// 11. Tables.
// ---------------------------------------------------------------------------

#[test]
fn a_table_lays_cells_side_by_side_and_rows_stack_down_the_page_with_borders_only_when_asked() {
    fn table_with(bordered: bool) -> Table {
        Table {
            columns_mm: vec![40.0, 60.0],
            rows: vec![
                Row {
                    cells: vec![cell("R1C1"), cell("R1C2")],
                },
                Row {
                    cells: vec![cell("R2C1"), cell("R2C2")],
                },
            ],
            bordered,
            padding_mm: None,
        }
    }

    let page = PageSize::new(210.0, 297.0);
    let (doc, _) = write_pdf(&sheet(vec![Block::Table(table_with(true))]));
    let written = written_text(&doc, 1, &page);
    let find = |needle: &str| -> &Written {
        written
            .iter()
            .find(|w| w.text.contains(needle))
            .unwrap_or_else(|| panic!("{needle:?} not found among {written:?}"))
    };

    let r1c1 = find("R1C1");
    let r1c2 = find("R1C2");
    let r2c1 = find("R2C1");
    find("R2C2"); // sanity check that the whole grid was written

    assert!(
        r1c2.x_mm > r1c1.x_mm + 10.0,
        "the second cell ({:.2}mm) must sit well right of the first ({:.2}mm)",
        r1c2.x_mm,
        r1c1.x_mm
    );
    // A thousandth of a millimetre: coordinates read back from the PDF have
    // been through an f32 narrowing (see `real()` in src/pdf.rs).
    assert!(
        (r1c1.y_mm - r1c2.y_mm).abs() < 1e-3,
        "two cells in one row must share a baseline"
    );
    assert!(
        r2c1.y_mm > r1c1.y_mm + 1.0,
        "the second row ({:.2}mm) must sit below the first ({:.2}mm)",
        r2c1.y_mm,
        r1c1.y_mm
    );
    assert!(
        (r2c1.x_mm - r1c1.x_mm).abs() < 1e-3,
        "a column must line up between rows"
    );

    let bordered_rects = ops(&doc, 1)
        .into_iter()
        .filter(|op| op.operator == "re")
        .count();
    assert_eq!(
        bordered_rects, 4,
        "expected one rectangle per cell (2 rows x 2 columns) when bordered"
    );

    let (plain_doc, _) = write_pdf(&sheet(vec![Block::Table(table_with(false))]));
    let plain_rects = ops(&plain_doc, 1)
        .into_iter()
        .filter(|op| op.operator == "re")
        .count();
    assert_eq!(plain_rects, 0, "bordered: false must draw no rectangles");
}

// ---------------------------------------------------------------------------
// 12. table_columns.
// ---------------------------------------------------------------------------

#[test]
fn table_columns_share_the_width_equally_or_scale_down_to_fit_the_paper() {
    // No widths given: split the paper equally between however many columns
    // the rows actually use.
    let no_widths = Table {
        columns_mm: vec![],
        rows: vec![Row {
            cells: vec![cell("a"), cell("b"), cell("c")],
        }],
        bordered: false,
        padding_mm: None,
    };
    assert_eq!(table_columns(&no_widths, 180.0), vec![60.0, 60.0, 60.0]);

    // Widths given, and they already fit: kept exactly as the document asked.
    let fits = Table {
        columns_mm: vec![30.0, 30.0],
        rows: vec![Row {
            cells: vec![cell("a"), cell("b")],
        }],
        bordered: false,
        padding_mm: None,
    };
    assert_eq!(table_columns(&fits, 180.0), vec![30.0, 30.0]);

    // Widths given but wider than the paper: scaled down, proportions kept.
    let overflowing = Table {
        columns_mm: vec![100.0, 100.0],
        rows: vec![Row {
            cells: vec![cell("a"), cell("b")],
        }],
        bordered: false,
        padding_mm: None,
    };
    assert_eq!(table_columns(&overflowing, 150.0), vec![75.0, 75.0]);
}

// ---------------------------------------------------------------------------
// 13. Underline and strikethrough.
// ---------------------------------------------------------------------------

#[test]
fn underline_and_strikethrough_add_rules_but_plain_text_adds_none() {
    // A single word, with no spaces, so exactly one rule is drawn per
    // decoration — a multi-word phrase draws one rule per fragment,
    // including the spaces between words, which would muddy the count.
    fn styled(underline: bool, strike: bool) -> Para {
        Para {
            pieces: vec![Piece::Text(
                "Word".to_string(),
                Style {
                    underline,
                    strike,
                    ..Style::default()
                },
            )],
            ..Para::default()
        }
    }

    let rule_count = |doc: &Document| {
        ops(doc, 1)
            .into_iter()
            .filter(|op| op.operator == "l")
            .count()
    };

    let (plain_doc, _) = write_pdf(&sheet(vec![Block::Para(styled(false, false))]));
    let (underline_doc, _) = write_pdf(&sheet(vec![Block::Para(styled(true, false))]));
    let (strike_doc, _) = write_pdf(&sheet(vec![Block::Para(styled(false, true))]));
    let (both_doc, _) = write_pdf(&sheet(vec![Block::Para(styled(true, true))]));

    assert_eq!(rule_count(&plain_doc), 0, "plain text must draw no rule");
    assert_eq!(
        rule_count(&underline_doc),
        1,
        "underline must draw exactly one rule"
    );
    assert_eq!(
        rule_count(&strike_doc),
        1,
        "strikethrough must draw exactly one rule"
    );
    assert_eq!(
        rule_count(&both_doc),
        2,
        "underline and strikethrough together must draw two rules"
    );
}

// ---------------------------------------------------------------------------
// 14. Characters outside WinAnsi.
// ---------------------------------------------------------------------------

#[test]
fn characters_with_a_plain_equivalent_are_mapped_to_it() {
    assert_eq!(plainly('\u{00A0}'), Some(" "), "non-breaking space");
    assert_eq!(plainly('\u{FB01}'), Some("fi"), "the fi ligature");
    assert_eq!(
        plainly('\u{00AD}'),
        Some(""),
        "a soft hyphen has nothing to show on paper"
    );
    assert_eq!(plainly('\u{2212}'), Some("-"), "an en-dash-like minus sign");
    assert_eq!(
        plainly('e'),
        None,
        "an ordinary WinAnsi character has no plain mapping of its own"
    );
}

#[test]
fn tame_drops_characters_nothing_can_write_and_records_them_in_dropped() {
    let mut fonts = plain_fonts();
    // A CJK character: not WinAnsi, no plain equivalent, and (with
    // `embedded: None`) no fallback font either.
    let out = fonts.tame("hello \u{4E2D} world");
    assert_eq!(
        out, "hello  world",
        "the untranslatable character must be dropped, not substituted"
    );
    assert!(fonts.dropped.contains(&'\u{4E2D}'));
    assert_eq!(fonts.dropped.len(), 1);
}

#[test]
fn write_notes_when_characters_could_not_be_written() {
    // One CJK character in otherwise-plain text. Whether or not this machine
    // has a system font, none of `Fonts::gather`'s candidates cover CJK, so
    // the character is dropped either way — see `tests::tame_drops_...`.
    let text = "Report \u{4E2D} approved";
    let (_doc, notes) = write_pdf(&sheet(vec![Block::Para(para(text))]));
    assert!(
        notes.iter().any(|n| n.contains("could not be written")),
        "{notes:?}"
    );
    assert!(notes.iter().any(|n| n.contains('\u{4E2D}')), "{notes:?}");
}

// ---------------------------------------------------------------------------
// 15. Paper size and margins.
// ---------------------------------------------------------------------------

#[test]
fn the_paper_size_and_margins_of_the_sheet_are_honoured() {
    let page = PageSize::new(150.0, 200.0);
    let margins = Margins {
        top_mm: 15.0,
        right_mm: 12.0,
        bottom_mm: 18.0,
        left_mm: 10.0,
    };
    let mut custom_sheet = Sheet::new(page, margins);
    custom_sheet.blocks = vec![Block::Para(para(
        &"lorem ipsum dolor sit amet ".repeat(100),
    ))];

    let (doc, _) = write_pdf(&custom_sheet);
    let pages = doc.get_pages();

    for page_id in pages.values() {
        let media = doc
            .get_dictionary(*page_id)
            .expect("a page must be a dictionary")
            .get(b"MediaBox")
            .expect("a page must have a MediaBox")
            .as_array()
            .expect("MediaBox must be an array");
        let width_pt = media[2].as_float().expect("MediaBox width") as f64;
        let height_pt = media[3].as_float().expect("MediaBox height") as f64;
        assert!(
            (width_pt - page.width_pt()).abs() < 0.01,
            "expected {width_pt} to match the sheet's own width in points"
        );
        assert!(
            (height_pt - page.height_pt()).abs() < 0.01,
            "expected {height_pt} to match the sheet's own height in points"
        );
    }

    // A thousandth of a millimetre of slack throughout this loop: coordinates
    // read back from the PDF have been through an f32 narrowing (see
    // `real()` in src/pdf.rs), so a line sitting exactly on a margin reads
    // back extremely close to it but not bit-identical.
    for page_number in 1..=pages.len() as u32 {
        for w in written_text(&doc, page_number, &page) {
            assert!(
                w.x_mm >= margins.left_mm - 1e-3,
                "text at x={:.2}mm on page {page_number} sits left of the {}mm left margin",
                w.x_mm,
                margins.left_mm
            );
            let right_edge = w.x_mm + pdf::builtin_width_mm(Font::Helvetica, &w.text, w.size_pt);
            assert!(
                right_edge <= page.width_mm - margins.right_mm + 1e-3,
                "text on page {page_number} reaches x={right_edge:.2}mm, past the \
                 {}mm right margin",
                margins.right_mm
            );
            assert!(
                w.y_mm >= margins.top_mm - 1e-3,
                "text baseline at y={:.2}mm on page {page_number} sits above the {}mm top margin",
                w.y_mm,
                margins.top_mm
            );
            assert!(
                w.y_mm <= page.height_mm - margins.bottom_mm + 1e-3,
                "text baseline at y={:.2}mm on page {page_number} sits below the {}mm bottom \
                 margin",
                w.y_mm,
                margins.bottom_mm
            );
        }
    }
}

// ---------------------------------------------------------------------------
// A few more internals named as worth testing directly.
// ---------------------------------------------------------------------------

#[test]
fn a_tab_advances_to_the_next_stop_and_moves_on_from_one_already_there() {
    assert!(
        (next_stop(0.0) - TAB_MM).abs() < 1e-9,
        "the first stop after the margin"
    );
    // Landing exactly on a stop must still advance to the next one, or two
    // tabs typed in a row would do nothing.
    assert!(
        (next_stop(TAB_MM) - TAB_MM * 2.0).abs() < 1e-9,
        "a tab already at a stop must move to the next one"
    );
    assert!(
        (next_stop(TAB_MM * 2.5) - TAB_MM * 3.0).abs() < 1e-9,
        "a tab partway to the next stop rounds up to it"
    );
}

#[test]
fn builtin_picks_the_bold_face_when_a_style_asks_for_bold_and_italic_together() {
    // There is no bold-italic among the eight faces Onionskin carries, and a
    // heading that asked to stand out must still stand out, which italic
    // alone would not manage.
    let sans_both = Style {
        bold: true,
        italic: true,
        family: Family::Sans,
        ..Style::default()
    };
    assert_eq!(builtin(&sans_both), Font::HelveticaBold);

    let serif_both = Style {
        bold: true,
        italic: true,
        family: Family::Serif,
        ..Style::default()
    };
    assert_eq!(builtin(&serif_both), Font::TimesBold);

    let mono_bold = Style {
        bold: true,
        family: Family::Mono,
        ..Style::default()
    };
    assert_eq!(builtin(&mono_bold), Font::CourierBold);

    let sans_italic = Style {
        italic: true,
        family: Family::Sans,
        ..Style::default()
    };
    assert_eq!(builtin(&sans_italic), Font::HelveticaOblique);
}

#[test]
fn split_on_breaks_starts_a_fresh_run_at_each_break_and_marks_page_breaks() {
    let para = Para {
        pieces: vec![
            Piece::Text("one".into(), Style::default()),
            Piece::LineBreak,
            Piece::Text("two".into(), Style::default()),
            Piece::PageBreak,
            Piece::Text("three".into(), Style::default()),
        ],
        ..Para::default()
    };
    let runs = split_on_breaks(&para);

    assert_eq!(runs.len(), 3, "two explicit breaks make three runs");
    assert_eq!(
        runs[0].pieces,
        vec![Piece::Text("one".into(), Style::default())]
    );
    assert!(!runs[0].page_break);
    assert_eq!(
        runs[1].pieces,
        vec![Piece::Text("two".into(), Style::default())]
    );
    assert!(
        !runs[1].page_break,
        "a line break must not be marked as a page break"
    );
    assert_eq!(
        runs[2].pieces,
        vec![Piece::Text("three".into(), Style::default())]
    );
    assert!(
        runs[2].page_break,
        "the run after a page break must be marked as one"
    );
}
