//! Tests for reading OpenDocument Text.
//!
//! Most of these hand a small, hand-written document straight to
//! [`super::read_flat`] or [`super::read`], because that is the whole surface
//! this module offers and exercising it the way a caller would is more honest
//! than reaching into the style-resolution machinery directly. The one
//! exception is the length parser, which is simple enough to check on its
//! own.

use super::*;

/// Wraps a fragment of automatic styles and a fragment of body content in a
/// minimal flat ODF document, with every namespace prefix this reader
/// understands declared on the root element.
fn flat_document(styles: &str, body: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document
    xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
    xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0"
    xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"
    xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0"
    xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0"
    xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0"
    xmlns:xlink="http://www.w3.org/1999/xlink"
    office:version="1.3" office:mimetype="application/vnd.oasis.opendocument.text">
<office:automatic-styles>{styles}</office:automatic-styles>
<office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1"/></office:master-styles>
<office:body><office:text>{body}</office:text></office:body>
</office:document>"#
    )
}

fn only_para(sheet: &Sheet) -> &Para {
    match &sheet.blocks[0] {
        Block::Para(para) => para,
        Block::Table(_) => panic!("expected a paragraph, got a table"),
    }
}

// ---------------------------------------------------------------------------
// The length parser
// ---------------------------------------------------------------------------

#[test]
fn the_length_parser_understands_every_unit_it_promises_to() {
    let close = |value: Option<f64>, want: f64| {
        let got = value.unwrap_or(f64::NAN);
        assert!((got - want).abs() < 0.001, "wanted {want}, got {got}");
    };
    close(length_mm("21cm"), 210.0);
    close(length_mm("210mm"), 210.0);
    close(length_mm("8.5in"), 215.9);
    close(length_mm("12pt"), 4.2333);
    close(length_mm("1pc"), 4.2333);
    close(length_mm("96px"), 25.4);
    close(length_mm("10"), 10.0);
    close(length_mm("  10.5 mm  "), 10.5);
    assert!(length_mm("banana").is_none());
    assert!(length_mm("10furlongs").is_none());
}

// ---------------------------------------------------------------------------
// A flat document
// ---------------------------------------------------------------------------

#[test]
fn a_flat_document_gives_headings_spans_alignment_and_page_size() {
    let xml = flat_document(
        r#"<style:page-layout style:name="pm1"><style:page-layout-properties
                fo:page-width="8.5in" fo:page-height="11in"
                fo:margin-top="1in" fo:margin-right="1in"
                fo:margin-bottom="1in" fo:margin-left="1in"
                style:print-orientation="portrait"/></style:page-layout>
           <style:style style:name="P1" style:family="paragraph">
               <style:paragraph-properties fo:text-align="center"/>
           </style:style>
           <style:style style:name="T1" style:family="text">
               <style:text-properties fo:font-weight="bold"/>
           </style:style>
           <style:style style:name="T2" style:family="text">
               <style:text-properties fo:font-style="italic"/>
           </style:style>"#,
        r#"<text:h text:outline-level="1">Title Here</text:h>
           <text:p text:style-name="P1">Some <text:span text:style-name="T1">bold</text:span> and <text:span text:style-name="T2">italic</text:span> words.</text:p>"#,
    );

    let sheet = read_flat(&xml).unwrap();

    assert!(
        (sheet.page.width_mm - 215.9).abs() < 0.1,
        "{:?}",
        sheet.page
    );
    assert!(
        (sheet.page.height_mm - 279.4).abs() < 0.1,
        "{:?}",
        sheet.page
    );
    assert!(
        (sheet.margins.top_mm - 25.4).abs() < 0.1,
        "{:?}",
        sheet.margins
    );

    assert_eq!(sheet.blocks.len(), 2);
    let Block::Para(heading) = &sheet.blocks[0] else {
        panic!("expected the heading to be a paragraph");
    };
    assert_eq!(heading.plain_text(), "Title Here");
    assert!(
        (heading.style.size_pt - 24.0).abs() < 0.01,
        "{:?}",
        heading.style
    );
    assert!(heading.style.bold, "a level-1 heading should be bold");

    let Block::Para(para) = &sheet.blocks[1] else {
        panic!("expected the body text to be a paragraph");
    };
    assert_eq!(para.align, Align::Centre);
    let style_of = |wanted: &str| {
        para.pieces
            .iter()
            .find_map(|piece| match piece {
                Piece::Text(text, style) if text == wanted => Some(*style),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no {wanted:?} piece in {:?}", para.pieces))
    };
    assert!(style_of("bold").bold);
    assert!(style_of("italic").italic);
}

#[test]
fn a_heading_with_a_silent_style_still_gets_the_outline_level_default() {
    // The style exists, and says nothing at all about size or weight — which
    // is exactly the document that would otherwise come back at eleven
    // points and not bold, indistinguishable from ordinary text.
    let xml = flat_document(
        r#"<style:style style:name="P2" style:family="paragraph"/>"#,
        r#"<text:h text:outline-level="2" text:style-name="P2">A Second-Level Heading</text:h>"#,
    );
    let sheet = read_flat(&xml).unwrap();
    let heading = only_para(&sheet);
    assert!(
        (heading.style.size_pt - 18.0).abs() < 0.01,
        "{:?}",
        heading.style
    );
    assert!(heading.style.bold);
}

#[test]
fn a_percentage_font_size_inherits_from_its_parent_style() {
    let xml = flat_document(
        r#"<style:default-style style:family="paragraph">
               <style:text-properties fo:font-size="10pt"/>
           </style:default-style>
           <style:style style:name="Base" style:family="paragraph"/>
           <style:style style:name="Emphasis" style:family="paragraph"
                        style:parent-style-name="Base">
               <style:text-properties fo:font-size="150%"/>
           </style:style>"#,
        r#"<text:p text:style-name="Emphasis">Bigger</text:p>"#,
    );
    let sheet = read_flat(&xml).unwrap();
    let para = only_para(&sheet);
    assert!(
        (para.style.size_pt - 15.0).abs() < 0.01,
        "150% of 10pt should be 15pt, got {:?}",
        para.style
    );
}

#[test]
fn tabs_line_breaks_and_repeated_spaces_come_through() {
    let xml = flat_document(
        "",
        r#"<text:p>A<text:tab/>B<text:line-break/>C<text:s text:c="3"/>D</text:p>"#,
    );
    let sheet = read_flat(&xml).unwrap();
    let para = only_para(&sheet);
    assert_eq!(
        para.pieces,
        vec![
            Piece::Text("A".to_string(), Style::default()),
            Piece::Tab,
            Piece::Text("B".to_string(), Style::default()),
            Piece::LineBreak,
            Piece::Text("C".to_string(), Style::default()),
            Piece::Text("   ".to_string(), Style::default()),
            Piece::Text("D".to_string(), Style::default()),
        ]
    );
}

#[test]
fn an_image_raises_a_note_instead_of_being_shown() {
    let xml = flat_document(
        "",
        r#"<text:p>Before<draw:frame draw:name="Frame1"><draw:image xlink:href="Pictures/a.png"/></draw:frame>After</text:p>"#,
    );
    let sheet = read_flat(&xml).unwrap();
    assert!(
        sheet.notes.iter().any(|note| note.contains("image")),
        "expected a note about the image, got {:?}",
        sheet.notes
    );
    let para = only_para(&sheet);
    assert_eq!(para.plain_text(), "BeforeAfter");
}

#[test]
fn a_footnote_raises_a_note_instead_of_being_shown() {
    let xml = flat_document(
        "",
        r#"<text:p>See note<text:note text:note-class="footnote">
               <text:note-citation>1</text:note-citation>
               <text:note-body><text:p>The footnote text.</text:p></text:note-body>
           </text:note> here.</text:p>"#,
    );
    let sheet = read_flat(&xml).unwrap();
    assert!(
        sheet.notes.iter().any(|note| note.contains("footnote")),
        "{:?}",
        sheet.notes
    );
    let para = only_para(&sheet);
    assert_eq!(para.plain_text(), "See note here.");
}

#[test]
fn a_header_with_content_is_noted() {
    let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document
    xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
    xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0"
    xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"
    office:version="1.3">
<office:automatic-styles>
<style:page-layout style:name="pm1"><style:page-layout-properties fo:page-width="210mm" fo:page-height="297mm"/></style:page-layout>
</office:automatic-styles>
<office:master-styles>
<style:master-page style:name="Standard" style:page-layout-name="pm1">
<style:header><text:p>Page header</text:p></style:header>
</style:master-page>
</office:master-styles>
<office:body><office:text><text:p>Body text.</text:p></office:text></office:body>
</office:document>"#;
    let sheet = read_flat(xml).unwrap();
    assert!(
        sheet.notes.iter().any(|note| note.contains("header")),
        "{:?}",
        sheet.notes
    );
    // The header's own paragraph must not leak into the body.
    assert_eq!(sheet.text().matches("Page header").count(), 0);
    assert!(sheet.text().contains("Body text."));
}

// ---------------------------------------------------------------------------
// Lists
// ---------------------------------------------------------------------------

#[test]
fn a_numbered_list_counts_and_a_bulleted_one_does_not() {
    let xml = flat_document(
        r#"<text:list-style style:name="LNum">
               <text:list-level-style-number text:level="1"/>
           </text:list-style>
           <text:list-style style:name="LBul">
               <text:list-level-style-bullet text:level="1"/>
           </text:list-style>"#,
        r#"<text:list text:style-name="LNum">
               <text:list-item><text:p>First</text:p></text:list-item>
               <text:list-item><text:p>Second</text:p></text:list-item>
               <text:list-item><text:p>Third</text:p></text:list-item>
           </text:list>
           <text:list text:style-name="LBul">
               <text:list-item><text:p>Apple</text:p></text:list-item>
               <text:list-item><text:p>Pear</text:p></text:list-item>
           </text:list>"#,
    );
    let sheet = read_flat(&xml).unwrap();
    assert_eq!(sheet.blocks.len(), 5);

    let markers: Vec<Option<String>> = sheet
        .blocks
        .iter()
        .map(|block| match block {
            Block::Para(para) => para.marker.clone(),
            Block::Table(_) => None,
        })
        .collect();
    assert_eq!(
        markers,
        vec![
            Some("1.".to_string()),
            Some("2.".to_string()),
            Some("3.".to_string()),
            Some("•".to_string()),
            Some("•".to_string()),
        ]
    );

    for block in &sheet.blocks {
        let Block::Para(para) = block else { continue };
        // A hanging first line so wrapped text clears the marker, and one
        // level of nesting worth of indent.
        assert_eq!(para.first_line_mm, -6.0);
        assert!((para.indent_left_mm - 8.0).abs() < 0.001, "{:?}", para);
    }
}

#[test]
fn a_list_with_no_matching_level_falls_back_to_a_bullet() {
    // The style exists but only describes level 1; this list is used at
    // level 1 too, but under a style name that is not registered at all —
    // "cannot tell" should mean a bullet, not a crash.
    let xml = flat_document(
        "",
        r#"<text:list text:style-name="Missing">
               <text:list-item><text:p>Only item</text:p></text:list-item>
           </text:list>"#,
    );
    let sheet = read_flat(&xml).unwrap();
    let para = only_para(&sheet);
    assert_eq!(para.marker, Some("•".to_string()));
}

#[test]
fn nested_lists_nest() {
    let xml = flat_document(
        r#"<text:list-style style:name="LBul">
               <text:list-level-style-bullet text:level="1"/>
               <text:list-level-style-bullet text:level="2"/>
           </text:list-style>"#,
        r#"<text:list text:style-name="LBul">
               <text:list-item>
                   <text:p>Outer</text:p>
                   <text:list text:style-name="LBul">
                       <text:list-item><text:p>Inner</text:p></text:list-item>
                   </text:list>
               </text:list-item>
           </text:list>"#,
    );
    let sheet = read_flat(&xml).unwrap();
    assert_eq!(sheet.blocks.len(), 2);
    let Block::Para(outer) = &sheet.blocks[0] else {
        panic!("expected a paragraph")
    };
    let Block::Para(inner) = &sheet.blocks[1] else {
        panic!("expected a paragraph")
    };
    assert_eq!(outer.plain_text(), "Outer");
    assert_eq!(inner.plain_text(), "Inner");
    assert!((outer.indent_left_mm - 8.0).abs() < 0.001);
    assert!(
        (inner.indent_left_mm - 16.0).abs() < 0.001,
        "a nested item should sit one level deeper: {:?}",
        inner
    );
}

// ---------------------------------------------------------------------------
// Tables
// ---------------------------------------------------------------------------

#[test]
fn a_table_gives_column_widths_and_respects_a_spanned_cell() {
    let xml = flat_document(
        r#"<style:style style:name="Col1" style:family="table-column">
               <style:table-column-properties style:column-width="40mm"/>
           </style:style>
           <style:style style:name="Col2" style:family="table-column">
               <style:table-column-properties style:column-width="60mm"/>
           </style:style>"#,
        r#"<table:table table:name="Table1">
               <table:table-column table:style-name="Col1"/>
               <table:table-column table:style-name="Col2"/>
               <table:table-row>
                   <table:table-cell table:number-columns-spanned="2"><text:p>Spanned</text:p></table:table-cell>
                   <table:covered-table-cell/>
               </table:table-row>
               <table:table-row>
                   <table:table-cell><text:p>A</text:p></table:table-cell>
                   <table:table-cell><text:p>B</text:p></table:table-cell>
               </table:table-row>
           </table:table>"#,
    );
    let sheet = read_flat(&xml).unwrap();
    assert_eq!(sheet.blocks.len(), 1);
    let Block::Table(table) = &sheet.blocks[0] else {
        panic!("expected a table");
    };
    assert_eq!(table.columns_mm, vec![40.0, 60.0]);
    assert!(!table.bordered);

    assert_eq!(table.rows.len(), 2);
    assert_eq!(
        table.rows[0].cells.len(),
        1,
        "the covered cell should not appear as a cell of its own"
    );
    assert_eq!(table.rows[0].cells[0].span, 2);
    let Block::Para(spanned_text) = &table.rows[0].cells[0].blocks[0] else {
        panic!("expected a paragraph in the spanned cell");
    };
    assert_eq!(spanned_text.plain_text(), "Spanned");

    assert_eq!(table.rows[1].cells.len(), 2);
    assert_eq!(table.rows[1].cells[0].span, 1);
    assert_eq!(table.rows[1].cells[1].span, 1);
}

#[test]
fn a_bordered_cell_style_marks_the_whole_table_as_bordered() {
    let xml = flat_document(
        r#"<style:style style:name="Boxed" style:family="table-cell">
               <style:table-cell-properties fo:border="0.5pt solid #000000"/>
           </style:style>"#,
        r#"<table:table>
               <table:table-row>
                   <table:table-cell table:style-name="Boxed"><text:p>Boxed</text:p></table:table-cell>
               </table:table-row>
           </table:table>"#,
    );
    let sheet = read_flat(&xml).unwrap();
    let Block::Table(table) = &sheet.blocks[0] else {
        panic!("expected a table");
    };
    assert!(table.bordered);
    // With no column definitions at all, the widths are simply unknown.
    assert!(table.columns_mm.is_empty());
}

// ---------------------------------------------------------------------------
// A zipped .odt
// ---------------------------------------------------------------------------

#[test]
fn a_zipped_document_reads_content_and_styles_from_the_archive() {
    // Built by hand with the project's own zip writer, the way `office.rs`
    // builds one, rather than by calling this reader's own writer — there is
    // none — so this checks the archive- and two-part-document-reading path
    // specifically.
    let content = r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document-content
    xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
    xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0"
    xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0"
    xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0"
    office:version="1.3">
<office:automatic-styles>
<style:style style:name="P1" style:family="paragraph"><style:paragraph-properties fo:text-align="end"/></style:style>
</office:automatic-styles>
<office:body><office:text><text:p text:style-name="P1">Zipped and read back.</text:p></office:text></office:body>
</office:document-content>"#;

    let styles = r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document-styles
    xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
    xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0"
    xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0"
    office:version="1.3">
<office:automatic-styles>
<style:page-layout style:name="pm1"><style:page-layout-properties
    fo:page-width="210mm" fo:page-height="297mm"
    fo:margin-top="15mm" fo:margin-right="15mm"
    fo:margin-bottom="15mm" fo:margin-left="15mm"/></style:page-layout>
</office:automatic-styles>
<office:master-styles><style:master-page style:name="Standard" style:page-layout-name="pm1"/></office:master-styles>
</office:document-styles>"#;

    let manifest = r#"<?xml version="1.0" encoding="UTF-8"?>
<manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" manifest:version="1.3">
<manifest:file-entry manifest:full-path="/" manifest:media-type="application/vnd.oasis.opendocument.text"/>
<manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/>
<manifest:file-entry manifest:full-path="styles.xml" manifest:media-type="text/xml"/>
</manifest:manifest>"#;

    let bytes = crate::package::zip(&[
        crate::package::Entry::file(
            "mimetype",
            b"application/vnd.oasis.opendocument.text".to_vec(),
        ),
        crate::package::Entry::file("META-INF/manifest.xml", manifest.as_bytes().to_vec()),
        crate::package::Entry::file("content.xml", content.as_bytes().to_vec()),
        crate::package::Entry::file("styles.xml", styles.as_bytes().to_vec()),
    ]);

    let sheet = read(&bytes).unwrap();
    assert!(
        (sheet.margins.top_mm - 15.0).abs() < 0.01,
        "{:?}",
        sheet.margins
    );
    assert_eq!(sheet.blocks.len(), 1);
    let para = only_para(&sheet);
    assert_eq!(para.align, Align::Right);
    assert_eq!(para.plain_text(), "Zipped and read back.");
}

#[test]
fn a_zipped_document_with_no_styles_file_still_reads_its_text() {
    let content = r#"<?xml version="1.0" encoding="UTF-8"?>
<office:document-content xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0"
    xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" office:version="1.3">
<office:body><office:text><text:p>Just text, no styles.xml.</text:p></office:text></office:body>
</office:document-content>"#;
    let bytes = crate::package::zip(&[crate::package::Entry::file(
        "content.xml",
        content.as_bytes().to_vec(),
    )]);
    let sheet = read(&bytes).unwrap();
    assert!(sheet.text().contains("Just text, no styles.xml."));
    // With nothing to say otherwise, the page falls back to A4.
    assert!((sheet.page.width_mm - 210.0).abs() < 0.01);
    assert!((sheet.page.height_mm - 297.0).abs() < 0.01);
}

// ---------------------------------------------------------------------------
// Round-tripping through Onionskin's own writer
// ---------------------------------------------------------------------------

#[test]
fn writing_it_with_our_own_writer_and_reading_it_back_keeps_the_words() {
    let page = PageSize::new(210.0, 297.0);
    let mut document = crate::document::Document::blank(page, 1);
    document
        .add(crate::document::Item {
            id: 0,
            page: 1,
            x_mm: 20.0,
            y_mm: 30.0,
            text: "Round trip words".into(),
            size_pt: 13.0,
            font: "Times New Roman".into(),
            width_mm: None,
            rotation_deg: 0.0,
            colour: "#000000".into(),
            leading: 1.2,
        })
        .unwrap();
    document
        .add(crate::document::Item {
            id: 0,
            page: 1,
            x_mm: 20.0,
            y_mm: 50.0,
            text: "Bold words too".into(),
            size_pt: 13.0,
            font: "Helvetica-Bold".into(),
            width_mm: None,
            rotation_deg: 0.0,
            colour: "#cc0000".into(),
            leading: 1.2,
        })
        .unwrap();

    let bytes = crate::office::write(
        &document,
        crate::office::Format::Odt,
        crate::office::Layout::Flow,
    )
    .unwrap();
    let sheet = read(&bytes).unwrap();

    let text = sheet.text();
    assert!(text.contains("Round trip words"), "{text:?}");
    assert!(text.contains("Bold words too"), "{text:?}");

    // The second item was written in a font whose name says "Bold", which the
    // writer turns into a real `fo:font-weight` — this checks that survives
    // as an actual bold style, not just as the word "Bold" in the text.
    let bold_style = sheet.blocks.iter().find_map(|block| match block {
        Block::Para(para) => para.pieces.iter().find_map(|piece| match piece {
            Piece::Text(text, style) if text.contains("Bold words") => Some(*style),
            _ => None,
        }),
        Block::Table(_) => None,
    });
    assert!(
        bold_style.is_some_and(|style| style.bold),
        "expected a bold piece, got {bold_style:?}"
    );
}

#[test]
fn a_lettered_or_roman_list_counts_the_way_the_document_asks() {
    // OpenDocument says which numerals a level uses, and taking every list for
    // 1, 2, 3 renumbers somebody's appendix.
    for (format, expected) in [
        ("a", ["(a)", "(b)"]),
        ("A", ["(A)", "(B)"]),
        ("i", ["(i)", "(ii)"]),
        ("I", ["(I)", "(II)"]),
        ("1", ["(1)", "(2)"]),
    ] {
        let document = flat_document(
            &format!(
                "<text:list-style style:name=\"L1\">\
                 <text:list-level-style-number text:level=\"1\" \
                 style:num-format=\"{format}\" style:num-prefix=\"(\" \
                 style:num-suffix=\")\"/></text:list-style>"
            ),
            "<text:list text:style-name=\"L1\">\
             <text:list-item><text:p>one</text:p></text:list-item>\
             <text:list-item><text:p>two</text:p></text:list-item></text:list>",
        );
        let sheet = read_flat(&document).unwrap();
        let markers: Vec<String> = sheet
            .blocks
            .iter()
            .filter_map(|block| match block {
                Block::Para(para) => para.marker.clone(),
                Block::Table(_) => None,
            })
            .collect();
        assert_eq!(markers, expected.to_vec(), "num-format {format:?}");
    }
}

#[test]
fn a_bullet_a_printer_cannot_write_becomes_one_it_can() {
    // A word processor writes a plain round bullet as a Wingdings code point in
    // the private-use area, and no font a printer has can draw it — it comes
    // out as an empty box.
    let document = flat_document(
        "<text:list-style style:name=\"L2\">\
         <text:list-level-style-bullet text:level=\"1\" \
         text:bullet-char=\"\u{f0b7}\"/></text:list-style>",
        "<text:list text:style-name=\"L2\">\
         <text:list-item><text:p>one</text:p></text:list-item></text:list>",
    );
    let sheet = read_flat(&document).unwrap();
    assert_eq!(only_para(&sheet).marker.as_deref(), Some("\u{2022}"));
}

#[test]
fn a_bullet_the_document_chose_is_kept() {
    let document = flat_document(
        "<text:list-style style:name=\"L3\">\
         <text:list-level-style-bullet text:level=\"1\" \
         text:bullet-char=\"\u{2013}\"/></text:list-style>",
        "<text:list text:style-name=\"L3\">\
         <text:list-item><text:p>one</text:p></text:list-item></text:list>",
    );
    let sheet = read_flat(&document).unwrap();
    assert_eq!(only_para(&sheet).marker.as_deref(), Some("\u{2013}"));
}
