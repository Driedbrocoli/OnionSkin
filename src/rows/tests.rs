use super::*;

fn list_of(text: &str) -> List {
    List::parse(text, Path::new("test.csv")).expect("should have parsed")
}

#[test]
fn a_plain_spreadsheet_comes_back_as_rows_by_column_name() {
    let list = list_of("name,seat\nJ. Bezzina,4A\nA. Smith,4B\n");
    assert_eq!(list.columns, vec!["name", "seat"]);
    assert_eq!(list.rows.len(), 2);
    assert_eq!(list.rows[0].get("name"), Some("J. Bezzina"));
    assert_eq!(list.rows[1].get("seat"), Some("4B"));
    // Counted the way somebody says "the second one".
    assert_eq!(list.rows[0].number, 1);
    assert_eq!(list.rows[1].number, 2);
}

#[test]
fn a_comma_inside_quotes_is_part_of_the_value() {
    // The reason quoting exists at all: one cell holding "Smith, John".
    let list = list_of("name,address\n\"Smith, John\",\"12 High St, Valletta\"\n");
    assert_eq!(list.rows[0].get("name"), Some("Smith, John"));
    assert_eq!(list.rows[0].get("address"), Some("12 High St, Valletta"));
}

#[test]
fn a_line_break_inside_quotes_does_not_start_a_new_row() {
    // A postal address typed into one cell across two lines. Splitting the
    // file on newlines first would make this two broken rows.
    let list = list_of("name,address\nJ. Bezzina,\"12 High St\nValletta\"\n");
    assert_eq!(list.rows.len(), 1);
    assert_eq!(list.rows[0].get("address"), Some("12 High St\nValletta"));
}

#[test]
fn two_quotes_inside_quotes_are_one_quote() {
    let list = list_of("name\n\"The \"\"Old\"\" Mill\"\n");
    assert_eq!(list.rows[0].get("name"), Some("The \"Old\" Mill"));
}

#[test]
fn windows_and_old_mac_line_endings_are_both_read() {
    for text in [
        "name,seat\r\nJ. Bezzina,4A\r\n",
        "name,seat\rJ. Bezzina,4A\r",
        "name,seat\nJ. Bezzina,4A",
    ] {
        let list = list_of(text);
        assert_eq!(list.rows.len(), 1, "{text:?}");
        assert_eq!(list.rows[0].get("seat"), Some("4A"), "{text:?}");
    }
}

#[test]
fn excels_byte_order_mark_does_not_become_part_of_the_first_column_name() {
    // Without this the first column is named "\u{feff}name", every {name}
    // fails to match, and nothing says why.
    let list = list_of("\u{feff}name,seat\nJ. Bezzina,4A\n");
    assert_eq!(list.columns[0], "name");
    assert_eq!(list.rows[0].get("name"), Some("J. Bezzina"));
}

#[test]
fn a_row_with_the_wrong_number_of_values_says_which_line_and_why() {
    // Almost always an unquoted comma inside a name, so the message says so.
    let said = List::parse("name,seat\nSmith, John,4A\n", Path::new("t.csv"))
        .unwrap_err()
        .to_string();
    assert!(said.contains("line 2"), "{said}");
    assert!(said.contains("3 values"), "{said}");
    assert!(said.contains("inside quotes"), "{said}");
}

/// A spreadsheet is read rather than refused, which is the whole point of
/// [`crate::sheets`] — and it is recognised by what is inside it, so a
/// workbook somebody renamed to `.csv` to make a program accept it is read
/// correctly too.
#[test]
fn a_spreadsheet_is_read_whatever_it_is_called() {
    let dir = tempfile::tempdir().unwrap();
    let book = an_xlsx(&[&["name", "seat"], &["J. Bezzina", "4A"]]);

    for name in ["people.xlsx", "people.csv", "people"] {
        let path = dir.path().join(name);
        std::fs::write(&path, &book).unwrap();
        let list = List::read(&path).unwrap_or_else(|why| panic!("{name}: {why}"));
        assert_eq!(list.columns, vec!["name", "seat"]);
        assert_eq!(list.rows[0].get("seat"), Some("4A"), "{name}");
    }
}

/// A zip that is not a spreadsheet is not a list either. Control characters
/// are valid UTF-8, so left to the CSV reader a Word document comes apart
/// into one column of rubbish and the complaint is that it has no rows.
#[test]
fn a_zip_that_is_not_a_spreadsheet_is_named_as_such_rather_than_read_as_gibberish() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("people.xlsx");
    std::fs::write(&path, b"PK\x03\x04rest-of-a-zip").unwrap();
    let said = List::read(&path).unwrap_err().to_string();
    assert!(said.contains("zip file"), "{said}");
    assert!(said.contains(".xlsx"), "{said}");
}

/// Which tab, for a workbook that has more than one.
#[test]
fn a_tab_can_be_named_and_a_tab_that_is_not_there_is_refused_with_the_list() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("people.xlsx");
    std::fs::write(&path, an_xlsx(&[&["name"], &["J. Bezzina"]])).unwrap();

    assert!(List::read_sheet(&path, Some("Staff")).is_ok());
    let said = List::read_sheet(&path, Some("Payroll"))
        .unwrap_err()
        .to_string();
    assert!(said.contains("no sheet called 'Payroll'"), "{said}");
    assert!(
        said.contains("Staff"),
        "the tabs it does have are not named: {said}"
    );
}

/// Short rows are ordinary in a spreadsheet — the cells to the right were
/// never filled in — so they are padded rather than refused, unlike a ragged
/// CSV where a missing field means a comma in the wrong place.
#[test]
fn a_short_row_in_a_spreadsheet_is_filled_out_rather_than_refused() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("people.xlsx");
    std::fs::write(
        &path,
        an_xlsx(&[&["name", "seat"], &["J. Bezzina"], &["A. Borg", "4A"]]),
    )
    .unwrap();
    let list = List::read(&path).unwrap();
    assert_eq!(list.rows.len(), 2);
    assert_eq!(list.rows[0].get("seat"), Some(""));
    assert_eq!(list.rows[1].get("seat"), Some("4A"));
}

/// A one-tab workbook of rows of text, written the way a real one is.
fn an_xlsx(rows: &[&[&str]]) -> Vec<u8> {
    let body: String = rows
        .iter()
        .enumerate()
        .map(|(row, values)| {
            let cells: String = values
                .iter()
                .enumerate()
                .map(|(column, value)| {
                    format!(
                        "<c r=\"{}{}\" t=\"inlineStr\"><is><t>{value}</t></is></c>",
                        (b'A' + column as u8) as char,
                        row + 1
                    )
                })
                .collect();
            format!("<row r=\"{}\">{cells}</row>", row + 1)
        })
        .collect();
    let part = |name: &str, xml: &str| crate::package::Entry {
        name: name.to_string(),
        bytes: xml.as_bytes().to_vec(),
        mode: 0o644,
        directory: false,
    };
    crate::package::zip(&[
        part(
            "xl/workbook.xml",
            "<workbook><sheets><sheet name=\"Staff\" r:id=\"rId1\"/></sheets></workbook>",
        ),
        part(
            "xl/_rels/workbook.xml.rels",
            "<Relationships><Relationship Id=\"rId1\" \
             Target=\"worksheets/sheet1.xml\"/></Relationships>",
        ),
        part("xl/styles.xml", "<styleSheet/>"),
        part(
            "xl/worksheets/sheet1.xml",
            &format!("<worksheet><sheetData>{body}</sheetData></worksheet>"),
        ),
    ])
}

#[test]
fn a_file_with_only_column_names_says_there_is_nothing_to_make() {
    let said = List::parse("name,seat\n", Path::new("t.csv"))
        .unwrap_err()
        .to_string();
    assert!(said.contains("no sheets to make"), "{said}");
}

#[test]
fn an_empty_file_says_the_columns_are_missing() {
    assert!(matches!(
        List::parse("", Path::new("t.csv")),
        Err(RowsError::NoColumns { .. })
    ));
    assert!(matches!(
        List::parse(",,\n", Path::new("t.csv")),
        Err(RowsError::NoColumns { .. })
    ));
}

#[test]
fn a_trailing_newline_does_not_make_an_extra_blank_sheet() {
    // Every spreadsheet writes one. Two hundred names must not become two
    // hundred and one certificates, the last of them blank.
    let list = list_of("name\nA\nB\n");
    assert_eq!(list.rows.len(), 2);
}

#[test]
fn values_go_into_a_line_where_the_column_is_named() {
    let list = list_of("name,seat\nJ. Bezzina,4A\n");
    let row = &list.rows[0];
    assert_eq!(fill("Awarded to {name}", row), "Awarded to J. Bezzina");
    assert_eq!(fill("{name}, seat {seat}", row), "J. Bezzina, seat 4A");
    assert_eq!(fill("no braces here", row), "no braces here");
}

#[test]
fn a_list_with_its_own_number_column_means_those_numbers() {
    // The bug this pins: `{number}` used to be answered by the row counter
    // before the columns were even looked at, so a list of invoices keyed by a
    // "number" column printed 1, 2, 3 instead of the real numbers — wrong in a
    // way nobody notices until they are in the post.
    let list = list_of("number,name\n4471,Wickham\n4472,Ashby\n");
    assert_eq!(fill("Invoice {number}", &list.rows[0]), "Invoice 4471");
    assert_eq!(fill("Invoice {number}", &list.rows[1]), "Invoice 4472");
}

#[test]
fn the_row_can_number_itself_without_a_column_of_numbers() {
    // Tickets and invoices: "No. 1", "No. 2" with nothing in the file.
    let list = list_of("name\nA\nB\nC\n");
    assert_eq!(fill("Ticket {number}", &list.rows[0]), "Ticket 1");
    assert_eq!(fill("Ticket {number}", &list.rows[2]), "Ticket 3");
}

#[test]
fn a_misspelt_column_is_left_visible_rather_than_printed_as_nothing() {
    // Two hundred certificates reading "{nmae}" is a bad day. Two hundred
    // reading nothing at all is worse: the stack looks right until somebody
    // reads one.
    let list = list_of("name\nJ. Bezzina\n");
    assert_eq!(
        fill("Awarded to {nmae}", &list.rows[0]),
        "Awarded to {nmae}"
    );

    // And it is caught before a single sheet is made.
    let missing = unknown_columns(&["Awarded to {nmae} of {place}".to_string()], &list);
    assert_eq!(missing, vec!["nmae", "place"]);
    assert!(unknown_columns(&["Awarded to {name} #{number}".to_string()], &list).is_empty());
}

#[test]
fn an_unclosed_brace_is_left_alone_rather_than_eating_the_line() {
    let list = list_of("name\nJ. Bezzina\n");
    assert_eq!(fill("100% of {name", &list.rows[0]), "100% of {name");
    assert_eq!(fill("{", &list.rows[0]), "{");
}

#[test]
fn the_columns_are_listed_the_way_they_would_be_typed() {
    let list = list_of("name,seat,table\nA,1,2\n");
    assert_eq!(list.describe_columns(), "{name}, {seat}, {table}");
}
