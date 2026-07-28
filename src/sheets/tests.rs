//! Tests for reading a spreadsheet.
//!
//! The files here are built to the shape two real writers actually produce,
//! checked against them: openpyxl writes inline strings and gives its
//! relationship targets a leading slash; LibreOffice writes a shared-string
//! table and relative targets. Both are covered, because a reader that only
//! handles one of them handles half the spreadsheets in the world.
//!
//! They are built rather than checked in because a `.xlsx` in the repository
//! is a binary blob nobody can review, and `tests/all_in_rust.rs` says so.

use super::*;
use crate::package::Entry;

fn part(name: &str, xml: &str) -> Entry {
    Entry {
        name: name.to_string(),
        bytes: xml.as_bytes().to_vec(),
        mode: 0o644,
        directory: false,
    }
}

/// An `.xlsx` from its parts. `sheets` is (tab name, worksheet XML).
fn xlsx(sheets: &[(&str, &str)], shared: &[&str], styles: &str) -> Vec<u8> {
    let tabs: String = sheets
        .iter()
        .enumerate()
        .map(|(index, (name, _))| {
            format!(
                "<sheet name=\"{name}\" sheetId=\"{}\" r:id=\"rId{}\"/>",
                index + 1,
                index + 1
            )
        })
        .collect();
    let rels: String = sheets
        .iter()
        .enumerate()
        .map(|(index, _)| {
            format!(
                "<Relationship Id=\"rId{}\" Target=\"worksheets/sheet{}.xml\" \
                 Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet\"/>",
                index + 1,
                index + 1
            )
        })
        .collect();

    let mut entries = vec![
        part(
            "xl/workbook.xml",
            &format!("<workbook><sheets>{tabs}</sheets></workbook>"),
        ),
        part(
            "xl/_rels/workbook.xml.rels",
            &format!("<Relationships>{rels}</Relationships>"),
        ),
        part("xl/styles.xml", styles),
    ];
    if !shared.is_empty() {
        let items: String = shared
            .iter()
            .map(|text| format!("<si><t>{text}</t></si>"))
            .collect();
        entries.push(part(
            "xl/sharedStrings.xml",
            &format!("<sst count=\"{}\">{items}</sst>", shared.len()),
        ));
    }
    for (index, (_, sheet)) in sheets.iter().enumerate() {
        entries.push(part(
            &format!("xl/worksheets/sheet{}.xml", index + 1),
            &format!("<worksheet><sheetData>{sheet}</sheetData></worksheet>"),
        ));
    }
    crate::package::zip(&entries)
}

/// The styles part, with `cellXfs` entry N using format code N of `codes`.
///
/// Entry zero is always the plain one, the way every real file has it.
fn styles_using(codes: &[&str]) -> String {
    let declared: String = codes
        .iter()
        .enumerate()
        .map(|(index, code)| {
            format!(
                "<numFmt numFmtId=\"{}\" formatCode=\"{code}\"/>",
                164 + index
            )
        })
        .collect();
    let used: String = std::iter::once("<xf numFmtId=\"0\"/>".to_string())
        .chain((0..codes.len()).map(|index| format!("<xf numFmtId=\"{}\"/>", 164 + index)))
        .collect();
    format!(
        "<styleSheet><numFmts>{declared}</numFmts>\
         <cellStyleXfs><xf numFmtId=\"0\"/></cellStyleXfs>\
         <cellXfs count=\"{}\">{used}</cellXfs></styleSheet>",
        codes.len() + 1
    )
}

/// An `.ods` from its body XML.
fn ods(body: &str) -> Vec<u8> {
    crate::package::zip(&[
        part("mimetype", "application/vnd.oasis.opendocument.spreadsheet"),
        part(
            "content.xml",
            &format!(
                "<office:document-content><office:body><office:spreadsheet>\
                 {body}</office:spreadsheet></office:body></office:document-content>"
            ),
        ),
    ])
}

/// A cell holding text, the OpenDocument way.
fn cell(text: &str) -> String {
    format!(
        "<table:table-cell office:value-type=\"string\"><text:p>{text}</text:p></table:table-cell>"
    )
}

// ---------------------------------------------------------------------------
// The point of the thing
// ---------------------------------------------------------------------------

/// A list read straight out of Excel, without anybody saving it as CSV first.
#[test]
fn a_list_is_read_out_of_an_xlsx() {
    let book = read(&xlsx(
        &[(
            "Staff",
            "<row r=\"1\"><c r=\"A1\" t=\"s\"><v>0</v></c><c r=\"B1\" t=\"s\"><v>1</v></c></row>\
             <row r=\"2\"><c r=\"A2\" t=\"s\"><v>2</v></c><c r=\"B2\" t=\"n\"><v>1200.5</v></c></row>",
        )],
        &["name", "fee", "Ada Lovelace"],
        &styles_using(&[]),
    ))
    .unwrap();

    let sheet = book.first_with_anything().unwrap();
    assert_eq!(sheet.name, "Staff");
    assert_eq!(sheet.rows[0], vec!["name", "fee"]);
    assert_eq!(sheet.rows[1], vec!["Ada Lovelace", "1200.5"]);
}

/// And out of LibreOffice, which stores the text somewhere else entirely.
#[test]
fn a_list_is_read_out_of_an_ods() {
    let book = read(&ods(&format!(
        "<table:table table:name=\"Staff\">\
         <table:table-row>{}{}</table:table-row>\
         <table:table-row>{}{}</table:table-row></table:table>",
        cell("name"),
        cell("fee"),
        cell("Ada Lovelace"),
        cell("1200.50"),
    )))
    .unwrap();

    let sheet = book.first_with_anything().unwrap();
    assert_eq!(sheet.name, "Staff");
    assert_eq!(sheet.rows[0], vec!["name", "fee"]);
    assert_eq!(sheet.rows[1], vec!["Ada Lovelace", "1200.50"]);
}

/// Text kept on the cell rather than in the shared table. openpyxl writes
/// every string this way, so a reader that only knows the shared table reads
/// an openpyxl file as a page of blanks.
#[test]
fn text_written_onto_the_cell_is_read_as_well_as_text_in_the_table() {
    let book = read(&xlsx(
        &[(
            "Staff",
            "<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>name</t></is></c>\
             <c r=\"B1\" t=\"inlineStr\"><is><t>role</t></is></c></row>",
        )],
        &[],
        &styles_using(&[]),
    ))
    .unwrap();
    assert_eq!(book.sheets[0].rows[0], vec!["name", "role"]);
}

// ---------------------------------------------------------------------------
// Dates, which are the whole difficulty
// ---------------------------------------------------------------------------

/// A date in Excel is a number with a format attached. Read without the
/// format, a certificate comes out saying 45487 instead of a date.
#[test]
fn a_date_is_read_back_as_a_date_and_not_as_a_number() {
    let book = read(&xlsx(
        &[(
            "Staff",
            // Style 1 is the first declared format below.
            "<row r=\"1\"><c r=\"A1\" s=\"1\" t=\"n\"><v>45487</v></c>\
             <c r=\"B1\" t=\"n\"><v>45487</v></c></row>",
        )],
        &[],
        &styles_using(&["yyyy-mm-dd"]),
    ))
    .unwrap();
    assert_eq!(book.sheets[0].rows[0][0], "2024-07-14");
    // And the same number with no date format stays the number it is.
    assert_eq!(book.sheets[0].rows[0][1], "45487");
}

/// A format code means the same in either case, and Excel's own dialog writes
/// it in capitals. Matched only in lower case, every British date column in
/// the world comes back as five digits.
#[test]
fn a_format_written_in_capitals_is_still_a_date() {
    for code in ["DD/MM/YYYY", "dd/mm/yyyy", "M/D/YY", "MMM-YY"] {
        let book = read(&xlsx(
            &[(
                "S",
                "<row r=\"1\"><c r=\"A1\" s=\"1\" t=\"n\"><v>45487</v></c></row>",
            )],
            &[],
            &styles_using(&[code]),
        ))
        .unwrap();
        assert_eq!(
            book.sheets[0].rows[0][0], "2024-07-14",
            "{code} was not recognised as a date"
        );
    }
}

/// The numbered formats no file declares, because every spreadsheet has them.
#[test]
fn the_built_in_date_formats_are_known_without_being_declared() {
    // numFmtId 14 is the short date, and nothing in the file says so.
    let styles = "<styleSheet><cellXfs count=\"2\"><xf numFmtId=\"0\"/>\
                  <xf numFmtId=\"14\"/></cellXfs></styleSheet>";
    let book = read(&xlsx(
        &[(
            "S",
            "<row r=\"1\"><c r=\"A1\" s=\"1\" t=\"n\"><v>45487</v></c></row>",
        )],
        &[],
        styles,
    ))
    .unwrap();
    assert_eq!(book.sheets[0].rows[0][0], "2024-07-14");
}

/// Letters inside quotes are text, not a format. `0" days"` is a plain number
/// that happens to contain a `d`, and reading it as a date turns 5 into 1900.
#[test]
fn letters_inside_quotes_do_not_make_a_date() {
    let book = read(&xlsx(
        &[(
            "S",
            "<row r=\"1\"><c r=\"A1\" s=\"1\" t=\"n\"><v>5</v></c></row>",
        )],
        &[],
        &styles_using(&["0&quot; days&quot;"]),
    ))
    .unwrap();
    assert_eq!(book.sheets[0].rows[0][0], "5");
}

/// A time with no date is a fraction of a day, and it is a time that is wanted
/// back — not the first of January 1900.
#[test]
fn a_time_of_day_comes_back_as_a_time() {
    let book = read(&xlsx(
        &[(
            "S",
            "<row r=\"1\"><c r=\"A1\" s=\"1\" t=\"n\"><v>0.3854166666666667</v></c></row>",
        )],
        &[],
        &styles_using(&["HH:MM"]),
    ))
    .unwrap();
    assert_eq!(book.sheets[0].rows[0][0], "09:15:00");
}

/// Excel believes 1900 was a leap year, to stay compatible with a spreadsheet
/// from 1983 that believed it. Counting from the 30th of December 1899 is what
/// makes every real date come out right in spite of that.
#[test]
fn the_epoch_is_the_one_excel_actually_uses() {
    let dates = [
        // The first day anybody's file has, and the day before the phantom.
        (59.0, "1900-02-28"),
        (61.0, "1900-03-01"),
        (25569.0, "1970-01-01"),
        (45487.0, "2024-07-14"),
        (45658.0, "2025-01-01"),
    ];
    for (serial, wanted) in dates {
        assert_eq!(
            date_from_serial(&serial.to_string(), Epoch::From1900).as_deref(),
            Some(wanted),
            "serial {serial}"
        );
    }
    // The Macintosh reckoning, which some files still declare.
    assert_eq!(
        date_from_serial("0", Epoch::From1904).as_deref(),
        Some("1904-01-01")
    );
}

#[test]
fn a_workbook_that_asks_for_the_1904_reckoning_gets_it() {
    assert!(workbook_is_1904(
        "<workbook><workbookPr date1904=\"1\"/></workbook>"
    ));
    assert!(workbook_is_1904(
        "<workbook><workbookPr date1904=\"true\"/></workbook>"
    ));
    assert!(!workbook_is_1904("<workbook><workbookPr/></workbook>"));
    assert!(!workbook_is_1904("<workbook/>"));
}

/// 7.5% is stored as 0.075. Leaving the digits alone is not "what the file
/// says" here — it is a different number, off by a factor of a hundred.
#[test]
fn a_percentage_is_read_as_the_percentage_it_shows() {
    let book = read(&xlsx(
        &[(
            "S",
            "<row r=\"1\"><c r=\"A1\" s=\"1\" t=\"n\"><v>0.075</v></c>\
             <c r=\"B1\" s=\"2\" t=\"n\"><v>0.075</v></c></row>",
        )],
        &[],
        &styles_using(&["0.0%", "0%"]),
    ))
    .unwrap();
    assert_eq!(book.sheets[0].rows[0][0], "7.5%");
    assert_eq!(book.sheets[0].rows[0][1], "8%");
}

/// Digits are handed over exactly as the file has them. Reading "1200.50" as a
/// number and writing it out again is how a total becomes 1200.4999999999998.
#[test]
fn the_digits_a_file_holds_are_not_rounded_on_the_way_through() {
    let book = read(&xlsx(
        &[(
            "S",
            "<row r=\"1\"><c r=\"A1\" t=\"n\"><v>1200.50</v></c>\
             <c r=\"B1\" t=\"n\"><v>12345678901234567</v></c>\
             <c r=\"C1\" t=\"n\"><v>0.1</v></c></row>",
        )],
        &[],
        &styles_using(&[]),
    ))
    .unwrap();
    assert_eq!(
        book.sheets[0].rows[0],
        vec!["1200.50", "12345678901234567", "0.1"]
    );
}

// ---------------------------------------------------------------------------
// Gaps
// ---------------------------------------------------------------------------

/// Neither format writes an empty cell, so a gap has to be worked out. Read
/// wrongly, every value after the gap is in the wrong column — which on a
/// mail merge means the fee printed where the name goes.
#[test]
fn a_gap_in_the_middle_of_a_row_keeps_the_columns_lined_up() {
    let book = read(&xlsx(
        &[(
            "S",
            "<row r=\"1\"><c r=\"A1\" t=\"s\"><v>0</v></c><c r=\"D1\" t=\"s\"><v>1</v></c></row>",
        )],
        &["name", "notes"],
        &styles_using(&[]),
    ))
    .unwrap();
    assert_eq!(book.sheets[0].rows[0], vec!["name", "", "", "notes"]);
}

/// A skipped row number is a blank row, and swallowing it moves every row
/// after it up one.
#[test]
fn a_skipped_row_stays_skipped() {
    let book = read(&xlsx(
        &[(
            "S",
            "<row r=\"1\"><c r=\"A1\" t=\"s\"><v>0</v></c></row>\
             <row r=\"3\"><c r=\"A3\" t=\"s\"><v>1</v></c></row>",
        )],
        &["first", "third"],
        &styles_using(&[]),
    ))
    .unwrap();
    assert_eq!(book.sheets[0].rows.len(), 3);
    assert_eq!(book.sheets[0].rows[0], vec!["first"]);
    assert!(book.sheets[0].rows[1].is_empty());
    assert_eq!(book.sheets[0].rows[2], vec!["third"]);
}

/// A cell address names its column: A is the first, AA is the twenty-seventh.
#[test]
fn a_cell_address_says_which_column_it_is() {
    assert_eq!(column_of("A1"), 0);
    assert_eq!(column_of("B7"), 1);
    assert_eq!(column_of("Z1"), 25);
    assert_eq!(column_of("AA1"), 26);
    assert_eq!(column_of("AB1"), 27);
    assert_eq!(column_of("BA1"), 52);
    // XFD is Excel's last column: the 16,384th, so index 16,383.
    assert_eq!(column_of("XFD1"), MOST_COLUMNS - 1);
    // Nonsense does not panic and does not run away.
    assert_eq!(column_of(""), 0);
    assert_eq!(column_of("1"), 0);
    assert!(column_of("AAAAAAAAAAAAAAAA1") <= MOST_COLUMNS);
}

/// OpenDocument says "and now sixteen thousand empty cells" rather than
/// writing them. Expanded, a hundred thousand rows of that is two billion
/// empty strings; counted, it is nothing at all.
#[test]
fn a_row_claiming_sixteen_thousand_empty_cells_costs_nothing() {
    let book = read(&ods(&format!(
        "<table:table table:name=\"S\"><table:table-row>{}\
         <table:table-cell table:number-columns-repeated=\"16384\"/>\
         </table:table-row></table:table>",
        cell("name"),
    )))
    .unwrap();
    assert_eq!(book.sheets[0].rows[0], vec!["name"]);
}

/// The same trick between two values, where the blanks do have to be kept —
/// otherwise the second value lands in the wrong column.
#[test]
fn repeated_empty_cells_between_two_values_are_kept() {
    let book = read(&ods(&format!(
        "<table:table table:name=\"S\"><table:table-row>{}\
         <table:table-cell table:number-columns-repeated=\"3\"/>{}\
         </table:table-row></table:table>",
        cell("name"),
        cell("notes"),
    )))
    .unwrap();
    assert_eq!(book.sheets[0].rows[0], vec!["name", "", "", "", "notes"]);
}

/// A repeated *value* is a real row of that value, which is how a column of
/// "Yes" is stored.
#[test]
fn a_repeated_value_is_repeated() {
    let book = read(&ods("<table:table table:name=\"S\"><table:table-row>\
         <table:table-cell table:number-columns-repeated=\"3\" \
         office:value-type=\"string\"><text:p>Yes</text:p></table:table-cell>\
         </table:table-row></table:table>"))
    .unwrap();
    assert_eq!(book.sheets[0].rows[0], vec!["Yes", "Yes", "Yes"]);
}

// ---------------------------------------------------------------------------
// Whole books
// ---------------------------------------------------------------------------

/// Every tab is read, in the book's order, so somebody can say which one they
/// meant.
#[test]
fn every_tab_is_read_and_can_be_asked_for_by_name() {
    let book = read(&xlsx(
        &[
            (
                "Notes",
                "<row r=\"1\"><c r=\"A1\" t=\"s\"><v>0</v></c></row>",
            ),
            (
                "Staff",
                "<row r=\"1\"><c r=\"A1\" t=\"s\"><v>1</v></c></row>",
            ),
        ],
        &["not the list", "the list"],
        &styles_using(&[]),
    ))
    .unwrap();

    assert_eq!(book.names(), vec!["Notes", "Staff"]);
    assert_eq!(book.named("staff").unwrap().rows[0], vec!["the list"]);
    assert_eq!(book.named("  STAFF ").unwrap().rows[0], vec!["the list"]);
    assert!(book.named("Payroll").is_none());
}

/// A book whose first tab is an empty "Notes" is ordinary, and reporting "no
/// columns at all" about a file that plainly has a list in it is not.
#[test]
fn the_first_tab_with_anything_on_it_is_the_one_that_counts() {
    let book = read(&xlsx(
        &[
            ("Blank", ""),
            (
                "Staff",
                "<row r=\"1\"><c r=\"A1\" t=\"s\"><v>0</v></c></row>",
            ),
        ],
        &["the list"],
        &styles_using(&[]),
    ))
    .unwrap();
    assert_eq!(book.first_with_anything().unwrap().name, "Staff");
}

/// The tabs are in the workbook's order, and the files inside are not. A
/// reader that takes `sheet1.xml` for the first tab gets it wrong on any book
/// whose tabs have been dragged about.
#[test]
fn the_tab_order_comes_from_the_workbook_and_not_from_the_file_names() {
    let entries = vec![
        part(
            "xl/workbook.xml",
            "<workbook><sheets>\
             <sheet name=\"Second\" sheetId=\"1\" r:id=\"rIdB\"/>\
             <sheet name=\"First\" sheetId=\"2\" r:id=\"rIdA\"/>\
             </sheets></workbook>",
        ),
        part(
            "xl/_rels/workbook.xml.rels",
            "<Relationships>\
             <Relationship Id=\"rIdA\" Target=\"worksheets/sheet1.xml\"/>\
             <Relationship Id=\"rIdB\" Target=\"worksheets/sheet2.xml\"/>\
             </Relationships>",
        ),
        part("xl/styles.xml", "<styleSheet/>"),
        part(
            "xl/worksheets/sheet1.xml",
            "<worksheet><sheetData><row r=\"1\"><c r=\"A1\" t=\"inlineStr\">\
             <is><t>in sheet1.xml</t></is></c></row></sheetData></worksheet>",
        ),
        part(
            "xl/worksheets/sheet2.xml",
            "<worksheet><sheetData><row r=\"1\"><c r=\"A1\" t=\"inlineStr\">\
             <is><t>in sheet2.xml</t></is></c></row></sheetData></worksheet>",
        ),
    ];
    let book = read(&crate::package::zip(&entries)).unwrap();
    assert_eq!(book.sheets[0].name, "Second");
    assert_eq!(book.sheets[0].rows[0], vec!["in sheet2.xml"]);
    assert_eq!(book.sheets[1].rows[0], vec!["in sheet1.xml"]);
}

/// openpyxl writes its relationship targets with a leading slash and
/// LibreOffice writes them without one. Both mean the same file.
#[test]
fn a_relationship_target_is_found_either_way_it_is_written() {
    assert_eq!(
        resolve("xl/", "worksheets/sheet1.xml"),
        "xl/worksheets/sheet1.xml"
    );
    assert_eq!(
        resolve("xl/", "/xl/worksheets/sheet1.xml"),
        "xl/worksheets/sheet1.xml"
    );
}

// ---------------------------------------------------------------------------
// Telling files apart, and refusing the ones that are not this
// ---------------------------------------------------------------------------

/// A Word file is also a zip with XML in it, and reading one as a spreadsheet
/// would give a page of nonsense rather than a refusal.
#[test]
fn a_word_file_is_not_mistaken_for_a_spreadsheet() {
    let docx = crate::package::zip(&[
        part("word/document.xml", "<document><body/></document>"),
        part("[Content_Types].xml", "<Types/>"),
    ]);
    assert!(!is_a_spreadsheet(&docx));
    assert!(read(&docx).is_err());

    // And an OpenDocument *text* file, which has the content.xml a
    // spreadsheet has and nothing else in common with one.
    let odt = crate::package::zip(&[
        part("mimetype", "application/vnd.oasis.opendocument.text"),
        part(
            "content.xml",
            "<office:document-content><office:body><office:text/>\
             </office:body></office:document-content>",
        ),
    ]);
    assert!(!is_a_spreadsheet(&odt), "an .odt was read as a spreadsheet");
}

#[test]
fn a_spreadsheet_is_recognised_by_what_is_in_it() {
    assert!(is_a_spreadsheet(&xlsx(&[("S", "")], &[], "<styleSheet/>")));
    assert!(is_a_spreadsheet(&ods("<table:table table:name=\"S\"/>")));
    assert!(!is_a_spreadsheet(b"name,fee\nAda,12\n"));
    assert!(!is_a_spreadsheet(b""));
}

/// Something that is not a zip at all, and something that is a broken one.
#[test]
fn nonsense_is_refused_rather_than_panicked_on() {
    assert!(read(b"").is_err());
    assert!(read(b"name,fee\nAda,12\n").is_err());
    assert!(read(b"PK\x03\x04 and then rubbish").is_err());
    let mut truncated = xlsx(&[("S", "")], &[], "<styleSheet/>");
    truncated.truncate(truncated.len() / 2);
    assert!(read(&truncated).is_err());
}

/// A file whose row numbers are beyond anything a spreadsheet can hold is
/// damaged, and the answer is to stop rather than to allocate.
#[test]
fn a_row_number_beyond_a_spreadsheet_is_refused() {
    let book = read(&xlsx(
        &[(
            "S",
            "<row r=\"99999999\"><c r=\"A99999999\" t=\"inlineStr\">\
             <is><t>x</t></is></c></row>",
        )],
        &[],
        "<styleSheet/>",
    ));
    assert!(book.is_err(), "a row number in the millions was accepted");
}

// ---------------------------------------------------------------------------
// The odds and ends real files are full of
// ---------------------------------------------------------------------------

/// A cell of several paragraphs is one value with line breaks in it — an
/// address in one cell is the ordinary reason for one.
#[test]
fn a_cell_of_several_lines_keeps_its_lines() {
    let book = read(&ods("<table:table table:name=\"S\"><table:table-row>\
         <table:table-cell office:value-type=\"string\">\
         <text:p>12 High Street</text:p><text:p>Valletta</text:p>\
         </table:table-cell></table:table-row></table:table>"))
    .unwrap();
    assert_eq!(book.sheets[0].rows[0][0], "12 High Street\nValletta");
}

/// A break inside one paragraph is a line break too.
#[test]
fn a_line_break_inside_a_cell_is_a_line_break() {
    let book = read(&ods("<table:table table:name=\"S\"><table:table-row>\
         <table:table-cell office:value-type=\"string\">\
         <text:p>Two<text:line-break/>lines</text:p>\
         </table:table-cell></table:table-row></table:table>"))
    .unwrap();
    assert_eq!(book.sheets[0].rows[0][0], "Two\nlines");
}

/// Excel breaks a coloured or emphasised string into runs. Read one run at a
/// time, "Ada Lovelace" comes back as "Ada" and the surname is lost.
#[test]
fn a_string_broken_into_runs_is_put_back_together() {
    let entries = vec![
        part(
            "xl/workbook.xml",
            "<workbook><sheets><sheet name=\"S\" r:id=\"rId1\"/></sheets></workbook>",
        ),
        part(
            "xl/_rels/workbook.xml.rels",
            "<Relationships><Relationship Id=\"rId1\" Target=\"worksheets/sheet1.xml\"/></Relationships>",
        ),
        part("xl/styles.xml", "<styleSheet/>"),
        part(
            "xl/sharedStrings.xml",
            "<sst><si><r><t>Ada </t></r><r><rPr><b/></rPr><t>Lovelace</t></r></si></sst>",
        ),
        part(
            "xl/worksheets/sheet1.xml",
            "<worksheet><sheetData><row r=\"1\">\
             <c r=\"A1\" t=\"s\"><v>0</v></c></row></sheetData></worksheet>",
        ),
    ];
    let book = read(&crate::package::zip(&entries)).unwrap();
    assert_eq!(book.sheets[0].rows[0][0], "Ada Lovelace");
}

/// A phonetic guide sits inside the string and is not part of it. Left in, a
/// Japanese name comes out written twice.
#[test]
fn a_phonetic_guide_is_not_part_of_the_name() {
    let entries = vec![
        part(
            "xl/workbook.xml",
            "<workbook><sheets><sheet name=\"S\" r:id=\"rId1\"/></sheets></workbook>",
        ),
        part(
            "xl/_rels/workbook.xml.rels",
            "<Relationships><Relationship Id=\"rId1\" Target=\"worksheets/sheet1.xml\"/></Relationships>",
        ),
        part("xl/styles.xml", "<styleSheet/>"),
        part(
            "xl/sharedStrings.xml",
            "<sst><si><t>山田</t><rPh sb=\"0\" eb=\"2\"><t>ヤマダ</t></rPh>\
             <phoneticPr fontId=\"1\"/></si></sst>",
        ),
        part(
            "xl/worksheets/sheet1.xml",
            "<worksheet><sheetData><row r=\"1\">\
             <c r=\"A1\" t=\"s\"><v>0</v></c></row></sheetData></worksheet>",
        ),
    ];
    let book = read(&crate::package::zip(&entries)).unwrap();
    assert_eq!(book.sheets[0].rows[0][0], "山田");
}

/// A formula's answer, a yes-or-no, and an error all have text worth keeping.
#[test]
fn formulas_booleans_and_errors_all_say_something() {
    let book = read(&xlsx(
        &[(
            "S",
            "<row r=\"1\">\
             <c r=\"A1\" t=\"str\"><f>UPPER(B1)</f><v>ADA</v></c>\
             <c r=\"B1\" t=\"b\"><v>1</v></c>\
             <c r=\"C1\" t=\"b\"><v>0</v></c>\
             <c r=\"D1\" t=\"e\"><v>#DIV/0!</v></c></row>",
        )],
        &[],
        "<styleSheet/>",
    ))
    .unwrap();
    assert_eq!(
        book.sheets[0].rows[0],
        vec!["ADA", "TRUE", "FALSE", "#DIV/0!"]
    );
}

/// Trailing empty rows are dropped. A spreadsheet has a thousand of them and
/// none of them is a person to print a certificate for.
#[test]
fn the_empty_rows_at_the_bottom_are_not_rows() {
    let book = read(&xlsx(
        &[(
            "S",
            "<row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>Ada</t></is></c></row>\
             <row r=\"2\"/><row r=\"3\"/><row r=\"4\"><c r=\"A4\"/></row>",
        )],
        &[],
        "<styleSheet/>",
    ))
    .unwrap();
    assert_eq!(book.sheets[0].rows.len(), 1);
}

/// The same for OpenDocument, which writes the blank rows out as a count.
#[test]
fn the_empty_rows_at_the_bottom_of_an_ods_are_not_rows_either() {
    let book = read(&ods(&format!(
        "<table:table table:name=\"S\"><table:table-row>{}</table:table-row>\
         <table:table-row table:number-rows-repeated=\"1048570\">\
         <table:table-cell table:number-columns-repeated=\"16384\"/>\
         </table:table-row></table:table>",
        cell("Ada"),
    )))
    .unwrap();
    assert_eq!(book.sheets[0].rows.len(), 1);
    assert_eq!(book.sheets[0].rows[0], vec!["Ada"]);
}
