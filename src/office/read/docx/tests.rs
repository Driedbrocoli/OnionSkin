use super::*;
use crate::office::read::{Align, Block, Family, Piece};
use crate::package::{zip, Entry};

/// The namespaces Word writes at the top of every `document.xml`.
const NAMESPACES: &str =
    "xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\" \
     xmlns:mc=\"http://schemas.openxmlformats.org/markup-compatibility/2006\" \
     xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" \
     xmlns:wps=\"http://schemas.microsoft.com/office/word/2010/wordprocessingShape\"";

/// A `.docx` holding this body and nothing else.
fn docx(body: &str) -> Vec<u8> {
    parts(body, None, None)
}

fn parts(body: &str, styles: Option<&str>, numbering: Option<&str>) -> Vec<u8> {
    let document = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
         <w:document {NAMESPACES}><w:body>{body}</w:body></w:document>"
    );
    let mut entries = vec![
        Entry::file(
            "[Content_Types].xml",
            b"<?xml version=\"1.0\"?><Types/>".to_vec(),
        ),
        Entry::file("word/document.xml", document.into_bytes()),
    ];
    if let Some(styles) = styles {
        entries.push(Entry::file(
            "word/styles.xml",
            format!("<w:styles {NAMESPACES}>{styles}</w:styles>").into_bytes(),
        ));
    }
    if let Some(numbering) = numbering {
        entries.push(Entry::file(
            "word/numbering.xml",
            format!("<w:numbering {NAMESPACES}>{numbering}</w:numbering>").into_bytes(),
        ));
    }
    zip(&entries)
}

/// One paragraph of plain text.
fn para(text: &str) -> String {
    format!("<w:p><w:r><w:t>{text}</w:t></w:r></w:p>")
}

/// The paragraphs of a sheet, in order.
fn paragraphs(sheet: &Sheet) -> Vec<&Para> {
    sheet
        .blocks
        .iter()
        .filter_map(|block| match block {
            Block::Para(para) => Some(para),
            _ => None,
        })
        .collect()
}

#[test]
fn reads_paragraphs_in_order() {
    let sheet = read(&docx(&format!("{}{}", para("First line"), para("Second")))).unwrap();
    let found = paragraphs(&sheet);
    assert_eq!(found.len(), 2);
    assert_eq!(found[0].plain_text(), "First line");
    assert_eq!(found[1].plain_text(), "Second");
}

#[test]
fn keeps_the_spaces_word_asked_to_keep() {
    let sheet = read(&docx(
        "<w:p><w:r><w:t xml:space=\"preserve\">a </w:t></w:r>\
         <w:r><w:t>b</w:t></w:r></w:p>",
    ))
    .unwrap();
    assert_eq!(paragraphs(&sheet)[0].plain_text(), "a b");
}

#[test]
fn reads_bold_italic_size_and_colour() {
    let sheet = read(&docx(
        "<w:p><w:r><w:rPr><w:b/><w:i/><w:sz w:val=\"36\"/>\
         <w:color w:val=\"FF0000\"/><w:u w:val=\"single\"/>\
         <w:rFonts w:ascii=\"Times New Roman\"/></w:rPr>\
         <w:t>Loud</w:t></w:r></w:p>",
    ))
    .unwrap();
    let Piece::Text(text, style) = &paragraphs(&sheet)[0].pieces[0] else {
        panic!("expected text");
    };
    assert_eq!(text, "Loud");
    assert!(style.bold);
    assert!(style.italic);
    assert!(style.underline);
    assert_eq!(style.size_pt, 18.0);
    assert_eq!(style.family, Family::Serif);
    assert!((style.colour.0 - 1.0).abs() < 1e-9);
    assert_eq!(style.colour.1, 0.0);
}

#[test]
fn a_switched_off_property_is_off() {
    let sheet = read(&docx(
        "<w:p><w:r><w:rPr><w:b w:val=\"0\"/></w:rPr><w:t>Quiet</w:t></w:r></w:p>",
    ))
    .unwrap();
    let Piece::Text(_, style) = &paragraphs(&sheet)[0].pieces[0] else {
        panic!("expected text");
    };
    assert!(!style.bold);
}

#[test]
fn a_heading_style_is_bigger_than_the_body() {
    let body = "<w:p><w:pPr><w:pStyle w:val=\"Heading1\"/></w:pPr>\
                <w:r><w:t>Title</w:t></w:r></w:p>";
    // No styles.xml at all: the text still has to come through.
    let sheet = read(&docx(body)).unwrap();
    assert_eq!(paragraphs(&sheet)[0].plain_text(), "Title");

    let styles = "<w:style w:type=\"paragraph\" w:styleId=\"Heading1\">\
                  <w:name w:val=\"heading 1\"/><w:rPr><w:sz w:val=\"48\"/><w:b/></w:rPr>\
                  </w:style>";
    let sheet = read(&parts(body, Some(styles), None)).unwrap();
    let Piece::Text(_, style) = &paragraphs(&sheet)[0].pieces[0] else {
        panic!("expected text");
    };
    assert_eq!(style.size_pt, 24.0);
    assert!(style.bold);
}

#[test]
fn a_heading_with_no_size_of_its_own_is_still_a_heading() {
    // Word leaves a heading's size to the theme often enough that taking the
    // body size would make every heading disappear into the text.
    let styles = "<w:style w:type=\"paragraph\" w:styleId=\"Heading1\">\
                  <w:name w:val=\"heading 1\"/></w:style>";
    let body = "<w:p><w:pPr><w:pStyle w:val=\"Heading1\"/></w:pPr>\
                <w:r><w:t>Title</w:t></w:r></w:p>";
    let sheet = read(&parts(body, Some(styles), None)).unwrap();
    let Piece::Text(_, style) = &paragraphs(&sheet)[0].pieces[0] else {
        panic!("expected text");
    };
    assert!(style.size_pt > Style::default().size_pt, "{style:?}");
    assert!(style.bold);
}

#[test]
fn a_style_inherits_from_the_one_it_is_based_on() {
    let styles = "<w:style w:type=\"paragraph\" w:styleId=\"Base\">\
                  <w:rPr><w:sz w:val=\"30\"/><w:rFonts w:ascii=\"Courier New\"/></w:rPr>\
                  <w:pPr><w:jc w:val=\"center\"/></w:pPr></w:style>\
                  <w:style w:type=\"paragraph\" w:styleId=\"Child\">\
                  <w:basedOn w:val=\"Base\"/><w:rPr><w:b/></w:rPr></w:style>";
    let body = "<w:p><w:pPr><w:pStyle w:val=\"Child\"/></w:pPr>\
                <w:r><w:t>Inherited</w:t></w:r></w:p>";
    let sheet = read(&parts(body, Some(styles), None)).unwrap();
    let found = paragraphs(&sheet);
    let Piece::Text(_, style) = &found[0].pieces[0] else {
        panic!("expected text");
    };
    assert_eq!(style.size_pt, 15.0);
    assert_eq!(style.family, Family::Mono);
    assert!(style.bold);
    assert_eq!(found[0].align, Align::Centre);
}

#[test]
fn a_style_that_is_its_own_parent_does_not_hang() {
    let styles = "<w:style w:type=\"paragraph\" w:styleId=\"Loop\">\
                  <w:basedOn w:val=\"Loop\"/><w:rPr><w:sz w:val=\"28\"/></w:rPr></w:style>";
    let body = "<w:p><w:pPr><w:pStyle w:val=\"Loop\"/></w:pPr>\
                <w:r><w:t>Round</w:t></w:r></w:p>";
    let sheet = read(&parts(body, Some(styles), None)).unwrap();
    let Piece::Text(_, style) = &paragraphs(&sheet)[0].pieces[0] else {
        panic!("expected text");
    };
    assert_eq!(style.size_pt, 14.0);
}

#[test]
fn the_document_default_applies_where_nothing_else_does() {
    let styles = "<w:docDefaults><w:rPrDefault><w:rPr>\
                  <w:sz w:val=\"18\"/><w:rFonts w:ascii=\"Georgia\"/>\
                  </w:rPr></w:rPrDefault></w:docDefaults>";
    let sheet = read(&parts(&para("Small"), Some(styles), None)).unwrap();
    let Piece::Text(_, style) = &paragraphs(&sheet)[0].pieces[0] else {
        panic!("expected text");
    };
    assert_eq!(style.size_pt, 9.0);
    assert_eq!(style.family, Family::Serif);
}

#[test]
fn reads_the_paper_and_its_margins() {
    // A4 in twips, with a two-centimetre border.
    let sheet = read(&docx(&format!(
        "{}<w:sectPr><w:pgSz w:w=\"11906\" w:h=\"16838\"/>\
         <w:pgMar w:top=\"1134\" w:right=\"1134\" w:bottom=\"1134\" w:left=\"1134\"/>\
         </w:sectPr>",
        para("Body")
    )))
    .unwrap();
    assert!(
        (sheet.page.width_mm - 210.0).abs() < 0.2,
        "{:?}",
        sheet.page
    );
    assert!((sheet.page.height_mm - 297.0).abs() < 0.2);
    assert!((sheet.margins.left_mm - 20.0).abs() < 0.2);
}

#[test]
fn without_a_section_the_paper_is_the_one_word_starts_with() {
    let sheet = read(&docx(&para("Body"))).unwrap();
    assert!((sheet.page.width_mm - 215.9).abs() < 0.2);
    assert!((sheet.page.height_mm - 279.4).abs() < 0.2);
}

#[test]
fn reads_alignment_and_indents() {
    let sheet = read(&docx(
        "<w:p><w:pPr><w:jc w:val=\"both\"/>\
         <w:ind w:left=\"720\" w:hanging=\"360\"/>\
         <w:spacing w:before=\"120\" w:after=\"240\" w:line=\"360\" w:lineRule=\"auto\"/>\
         </w:pPr><w:r><w:t>Spread</w:t></w:r></w:p>",
    ))
    .unwrap();
    let para = paragraphs(&sheet)[0];
    assert_eq!(para.align, Align::Justify);
    assert!((para.indent_left_mm - 12.7).abs() < 0.1);
    assert!((para.first_line_mm + 6.35).abs() < 0.1);
    assert!((para.space_after_mm - 4.233).abs() < 0.05);
    assert!((para.line_spacing - 1.5).abs() < 1e-9);
}

#[test]
fn breaks_are_kept_apart() {
    let sheet = read(&docx(
        "<w:p><w:r><w:t>one</w:t><w:br/><w:t>two</w:t>\
         <w:br w:type=\"page\"/><w:t>three</w:t><w:tab/><w:t>four</w:t></w:r></w:p>",
    ))
    .unwrap();
    let pieces = &paragraphs(&sheet)[0].pieces;
    assert!(pieces.contains(&Piece::LineBreak));
    assert!(pieces.contains(&Piece::PageBreak));
    assert!(pieces.contains(&Piece::Tab));
}

#[test]
fn a_page_break_before_is_kept() {
    let sheet = read(&docx(
        "<w:p><w:pPr><w:pageBreakBefore/></w:pPr><w:r><w:t>Next</w:t></w:r></w:p>",
    ))
    .unwrap();
    assert!(paragraphs(&sheet)[0].break_before);
}

#[test]
fn field_codes_and_deleted_text_are_not_words() {
    let sheet = read(&docx(
        "<w:p><w:r><w:t>Page </w:t></w:r>\
         <w:r><w:instrText>PAGE \\* MERGEFORMAT</w:instrText></w:r>\
         <w:r><w:delText>gone</w:delText></w:r>\
         <w:r><w:t>1</w:t></w:r></w:p>",
    ))
    .unwrap();
    assert_eq!(paragraphs(&sheet)[0].plain_text(), "Page 1");
}

#[test]
fn a_shape_is_not_read_twice() {
    // Word writes the modern shape and an old-fashioned copy of it, so that
    // both new and old readers see something. Taking both gives the text twice.
    let sheet = read(&docx(
        "<w:p><w:r><mc:AlternateContent>\
         <mc:Choice Requires=\"wps\"><w:drawing><wps:txbx><w:txbxContent>\
         <w:p><w:r><w:t>In the box</w:t></w:r></w:p>\
         </w:txbxContent></wps:txbx></w:drawing></mc:Choice>\
         <mc:Fallback><w:pict><v:rect><v:textbox><w:txbxContent>\
         <w:p><w:r><w:t>In the box</w:t></w:r></w:p>\
         </w:txbxContent></v:textbox></v:rect></w:pict></mc:Fallback>\
         </mc:AlternateContent></w:r></w:p>",
    ))
    .unwrap();
    assert_eq!(sheet.text().matches("In the box").count(), 1);
}

#[test]
fn a_text_box_keeps_its_words() {
    let sheet = read(&docx(
        "<w:p><w:r><w:pict><v:rect><v:textbox><w:txbxContent>\
         <w:p><w:r><w:t>Boxed words</w:t></w:r></w:p>\
         </w:txbxContent></v:textbox></v:rect></w:pict></w:r></w:p>",
    ))
    .unwrap();
    assert!(sheet.text().contains("Boxed words"));
}

#[test]
fn reads_a_table() {
    let sheet = read(&docx(
        "<w:tbl><w:tblPr><w:tblBorders><w:top w:val=\"single\"/></w:tblBorders></w:tblPr>\
         <w:tblGrid><w:gridCol w:w=\"2880\"/><w:gridCol w:w=\"2880\"/></w:tblGrid>\
         <w:tr><w:tc><w:p><w:r><w:t>Left</w:t></w:r></w:p></w:tc>\
         <w:tc><w:p><w:r><w:t>Right</w:t></w:r></w:p></w:tc></w:tr>\
         <w:tr><w:tc><w:tcPr><w:gridSpan w:val=\"2\"/></w:tcPr>\
         <w:p><w:r><w:t>Across</w:t></w:r></w:p></w:tc></w:tr></w:tbl>",
    ))
    .unwrap();
    let Some(Block::Table(table)) = sheet.blocks.first() else {
        panic!("expected a table, got {:?}", sheet.blocks);
    };
    assert!(table.bordered);
    assert_eq!(table.columns_mm.len(), 2);
    assert!((table.columns_mm[0] - 50.8).abs() < 0.1);
    assert_eq!(table.rows.len(), 2);
    assert_eq!(table.rows[0].cells.len(), 2);
    assert_eq!(table.rows[1].cells[0].span, 2);
    assert!(sheet.text().contains("Across"));
}

#[test]
fn a_table_with_every_border_switched_off_is_not_ruled() {
    // A letterhead lays itself out with an invisible table, and ruling it
    // draws a grid across somebody's headed paper.
    let sheet = read(&docx(
        "<w:tbl><w:tblPr><w:tblBorders><w:top w:val=\"none\"/><w:left w:val=\"nil\"/>\
         </w:tblBorders></w:tblPr>\
         <w:tr><w:tc><w:p><w:r><w:t>Plain</w:t></w:r></w:p></w:tc></w:tr></w:tbl>",
    ))
    .unwrap();
    let Some(Block::Table(table)) = sheet.blocks.first() else {
        panic!("expected a table");
    };
    assert!(!table.bordered);
}

#[test]
fn a_table_says_how_much_room_it_leaves_round_its_words() {
    // Word writes hairline cell margins into a table it has fitted to its
    // contents, and assuming a comfortable margin instead wraps every cell.
    let sheet = read(&docx(
        "<w:tbl><w:tblPr><w:tblCellMar><w:top w:w=\"28\" w:type=\"dxa\"/>\
         <w:left w:w=\"28\" w:type=\"dxa\"/></w:tblCellMar></w:tblPr>\
         <w:tr><w:tc><w:p><w:r><w:t>Tight</w:t></w:r></w:p></w:tc></w:tr></w:tbl>",
    ))
    .unwrap();
    let Some(Block::Table(table)) = sheet.blocks.first() else {
        panic!("expected a table");
    };
    let (across, down) = table.padding_mm.expect("the document said");
    assert!((across - 0.494).abs() < 0.01, "{across}");
    assert!((down - 0.494).abs() < 0.01, "{down}");
}

#[test]
fn a_bulleted_list_gets_bullets() {
    let numbering = "<w:abstractNum w:abstractNumId=\"0\"><w:lvl w:ilvl=\"0\">\
                     <w:numFmt w:val=\"bullet\"/><w:lvlText w:val=\"\u{f0b7}\"/>\
                     </w:lvl></w:abstractNum>\
                     <w:num w:numId=\"1\"><w:abstractNumId w:val=\"0\"/></w:num>";
    let body = "<w:p><w:pPr><w:numPr><w:ilvl w:val=\"0\"/><w:numId w:val=\"1\"/></w:numPr></w:pPr>\
                <w:r><w:t>One</w:t></w:r></w:p>\
                <w:p><w:pPr><w:numPr><w:ilvl w:val=\"0\"/><w:numId w:val=\"1\"/></w:numPr></w:pPr>\
                <w:r><w:t>Two</w:t></w:r></w:p>";
    let sheet = read(&parts(body, None, Some(numbering))).unwrap();
    let found = paragraphs(&sheet);
    assert_eq!(found[0].marker.as_deref(), Some("\u{2022}"));
    assert_eq!(found[1].marker.as_deref(), Some("\u{2022}"));
    // The text has to clear the bullet, or the two land on top of each other.
    assert!(found[0].first_line_mm < 0.0);
    assert!(found[0].indent_left_mm > 0.0);
}

#[test]
fn a_numbered_list_counts_up() {
    let numbering = "<w:abstractNum w:abstractNumId=\"3\"><w:lvl w:ilvl=\"0\">\
                     <w:start w:val=\"1\"/><w:numFmt w:val=\"decimal\"/>\
                     <w:lvlText w:val=\"%1.\"/></w:lvl></w:abstractNum>\
                     <w:num w:numId=\"7\"><w:abstractNumId w:val=\"3\"/></w:num>";
    let item = "<w:p><w:pPr><w:numPr><w:ilvl w:val=\"0\"/><w:numId w:val=\"7\"/></w:numPr></w:pPr>\
                <w:r><w:t>Item</w:t></w:r></w:p>";
    let sheet = read(&parts(&item.repeat(3), None, Some(numbering))).unwrap();
    let markers: Vec<Option<&str>> = paragraphs(&sheet)
        .iter()
        .map(|para| para.marker.as_deref())
        .collect();
    assert_eq!(markers, vec![Some("1."), Some("2."), Some("3.")]);
}

#[test]
fn a_lettered_list_counts_in_letters() {
    let numbering = "<w:abstractNum w:abstractNumId=\"1\"><w:lvl w:ilvl=\"0\">\
                     <w:numFmt w:val=\"lowerLetter\"/><w:lvlText w:val=\"%1)\"/>\
                     </w:lvl></w:abstractNum>\
                     <w:num w:numId=\"2\"><w:abstractNumId w:val=\"1\"/></w:num>";
    let item = "<w:p><w:pPr><w:numPr><w:ilvl w:val=\"0\"/><w:numId w:val=\"2\"/></w:numPr></w:pPr>\
                <w:r><w:t>x</w:t></w:r></w:p>";
    let sheet = read(&parts(&item.repeat(2), None, Some(numbering))).unwrap();
    let markers: Vec<Option<&str>> = paragraphs(&sheet)
        .iter()
        .map(|para| para.marker.as_deref())
        .collect();
    assert_eq!(markers, vec![Some("a)"), Some("b)")]);
}

#[test]
fn numbering_off_for_one_paragraph_means_no_marker() {
    let body = "<w:p><w:pPr><w:numPr><w:ilvl w:val=\"0\"/><w:numId w:val=\"0\"/></w:numPr></w:pPr>\
                <w:r><w:t>Not a list</w:t></w:r></w:p>";
    let sheet = read(&docx(body)).unwrap();
    assert_eq!(paragraphs(&sheet)[0].marker, None);
}

#[test]
fn letters_and_numerals_count_the_way_lists_do() {
    assert_eq!(letters(1, false), "a");
    assert_eq!(letters(26, false), "z");
    assert_eq!(letters(27, false), "aa");
    assert_eq!(letters(2, true), "B");
    assert_eq!(roman(4), "IV");
    assert_eq!(roman(1987), "MCMLXXXVII");
}

#[test]
fn an_ampersand_survives() {
    let sheet = read(&docx(&para("Smith &amp; Sons &lt;Ltd&gt;"))).unwrap();
    assert_eq!(paragraphs(&sheet)[0].plain_text(), "Smith & Sons <Ltd>");
}

#[test]
fn a_zip_that_is_not_a_word_document_says_so() {
    let bytes = zip(&[Entry::file("hello.txt", b"not a document".to_vec())]);
    let error = read(&bytes).unwrap_err().to_string();
    assert!(error.contains("not a Word document"), "{error}");
}

#[test]
fn something_that_is_not_a_zip_says_so() {
    let error = read(b"This is a plain text file, not a zip.")
        .unwrap_err()
        .to_string();
    assert!(error.contains("not a Word or OpenDocument file"), "{error}");
}

#[test]
fn pictures_are_mentioned_rather_than_pretended_away() {
    let sheet = read(&docx(
        "<w:p><w:r><w:drawing><wp:inline/></w:drawing></w:r></w:p>",
    ))
    .unwrap();
    assert!(
        sheet.notes.iter().any(|note| note.contains("Pictures")),
        "{:?}",
        sheet.notes
    );
}

#[test]
fn what_this_program_writes_it_can_read_back() {
    // The other half of the office module writes `.docx`. Reading its own
    // output is the cheapest test there is that the two agree about the format.
    let mut document =
        crate::document::Document::blank(crate::geometry::PageSize::new(210.0, 297.0), 1);
    document
        .add(crate::document::Item {
            id: 0,
            page: 1,
            x_mm: 20.0,
            y_mm: 30.0,
            text: "Written by Onionskin".into(),
            size_pt: 12.0,
            font: "Helvetica".into(),
            width_mm: None,
            rotation_deg: 0.0,
            colour: "#000000".into(),
            leading: 1.2,
        })
        .unwrap();
    let bytes = crate::office::write(
        &document,
        crate::office::Format::Docx,
        crate::office::Layout::Flow,
    )
    .unwrap();

    let sheet = read(&bytes).unwrap();
    assert!(sheet.text().contains("Written by Onionskin"), "{sheet:?}");
}

/// A file that nests for ever used to take the program with it.
///
/// Word's structure is a circle: a table holds paragraphs, a paragraph holds a
/// text box, a text box holds paragraphs. Nothing in the format stops that
/// going round, and following it round ends in an exhausted stack — which Rust
/// answers by killing the process, without unwinding, so nothing anywhere can
/// catch it. The program disappears mid-sentence with no message at all.
///
/// There was a bound on tables, and it did not hold: a paragraph started each
/// table at depth zero, so alternating `tbl` and `p` reset the count every
/// turn. Text boxes had no bound at all. Both are covered here, and so is the
/// alternation, because it is the one that got past the bound that existed.
///
/// Two thousand deep is well past what any real stack survives unbounded and
/// costs nothing to build.
#[test]
fn a_file_nested_beyond_all_reason_is_read_as_far_as_it_goes_and_no_further() {
    const DEEP: usize = 2000;

    let boxes = {
        let mut body = "<w:p><w:r><w:t>outside</w:t></w:r>".to_string();
        for _ in 0..DEEP {
            body.push_str("<w:pict><v:textbox><w:txbxContent><w:p><w:r>");
        }
        body.push_str("<w:t>the very middle</w:t></w:r>");
        for _ in 0..DEEP {
            body.push_str("</w:p></w:txbxContent></v:textbox></w:pict>");
        }
        body.push_str("</w:p>");
        body
    };

    // Tables and paragraphs alternating, which is what defeated the old bound.
    let alternating = {
        let mut body = String::new();
        for _ in 0..DEEP {
            body.push_str("<w:tbl><w:tr><w:tc><w:p><w:r>");
        }
        body.push_str("<w:t>the very middle</w:t>");
        for _ in 0..DEEP {
            body.push_str("</w:r></w:p></w:tc></w:tr></w:tbl>");
        }
        body
    };

    for (what, body) in [("text boxes", boxes), ("tables", alternating)] {
        let sheet = read(&docx(&body)).unwrap_or_else(|why| panic!("{what}: {why}"));
        // It comes back — that is the whole assertion. What it managed to read
        // of the innermost part is not promised.
        let _ = sheet.text();
    }

    // And it says it gave up, rather than quietly dropping the words.
    let sheet = read(&docx(&{
        let mut body = String::new();
        for _ in 0..DEEP {
            body.push_str("<w:tbl><w:tr><w:tc><w:p><w:r>");
        }
        body.push_str("<w:t>x</w:t>");
        for _ in 0..DEEP {
            body.push_str("</w:r></w:p></w:tc></w:tr></w:tbl>");
        }
        body
    }))
    .unwrap();
    assert!(
        sheet.notes.iter().any(|note| note.contains("nested")),
        "it gave up silently: {:?}",
        sheet.notes
    );

    // The depth an ordinary document uses is untouched: a table in a cell in a
    // table, with a text box in it, still reads.
    let ordinary = read(&docx(
        "<w:tbl><w:tr><w:tc><w:tbl><w:tr><w:tc><w:p><w:r>\
         <w:pict><v:textbox><w:txbxContent><w:p><w:r><w:t>still here</w:t></w:r></w:p>\
         </w:txbxContent></v:textbox></w:pict></w:r></w:p></w:tc></w:tr></w:tbl>\
         </w:tc></w:tr></w:tbl>",
    ))
    .unwrap();
    assert!(
        ordinary.text().contains("still here"),
        "{}",
        ordinary.text()
    );
}
