use super::*;

/// A form, built out of the words that would be printed and written on it.
///
/// Rows rather than a scan, because what the picking works on is rows and a
/// scan is a slow and lossy way of producing some. The reading itself is
/// checked end to end elsewhere, off a real rendered page.
fn a_form(lines: &[(&str, f64, f64)]) -> Vec<Row> {
    let placed: Vec<crate::pdf::PlacedLine> = lines
        .iter()
        .map(|(text, x_mm, y_mm)| crate::pdf::PlacedLine {
            text: text.to_string(),
            x_mm: *x_mm,
            y_mm: *y_mm,
            size_pt: 12.0,
            font: crate::pdf::LineFont::Builtin(crate::pdf::Font::Helvetica),
            colour: (0.0, 0.0, 0.0),
            rotation_deg: 0.0,
        })
        .collect();
    crate::anchor::rows_from_lines(&placed)
}

fn read(values: &[Value]) -> Vec<String> {
    values.iter().map(Value::cell).collect()
}

/// The plain case: a label, and the words after it.
#[test]
fn the_words_after_a_label_are_the_value() {
    let rows = a_form(&[("Name: J. Bezzina", 20.0, 40.0)]);
    let values = pick_in(&rows, &[Field::named("Name")]);
    assert_eq!(read(&values), vec!["J. Bezzina"]);
}

/// Two fields on one line, which is the commonest form layout there is and the
/// case that decides whether any of this is usable.
///
/// Read without knowing where a value stops, `Name` comes out as
/// "J. Bezzina Date: 27 July 2024" — the whole rest of the line. That is not a
/// reading that looks wrong in a spreadsheet. It is a reading that looks like
/// somebody's name.
#[test]
fn a_value_stops_where_the_next_label_starts() {
    let rows = a_form(&[("Name: J. Bezzina    Date: 27 July 2024", 20.0, 40.0)]);
    let values = pick_in(&rows, &[Field::named("Name"), Field::named("Date")]);
    assert_eq!(read(&values), vec!["J. Bezzina", "27 July 2024"]);
}

/// And three of them, because two could be a coincidence of ordering.
#[test]
fn three_fields_on_one_line_each_keep_their_own() {
    let rows = a_form(&[("Ref: A-17 Date: 27 July Amount: 240.00", 20.0, 40.0)]);
    let values = pick_in(
        &rows,
        &[
            Field::named("Ref"),
            Field::named("Date"),
            Field::named("Amount"),
        ],
    );
    assert_eq!(read(&values), vec!["A-17", "27 July", "240.00"]);
}

/// The fields are picked in the order they were asked for, whatever order they
/// appear on the paper — the columns are the caller's, not the form's.
#[test]
fn the_columns_come_out_in_the_order_they_were_asked_for() {
    let rows = a_form(&[("Ref: A-17 Date: 27 July Amount: 240.00", 20.0, 40.0)]);
    let values = pick_in(
        &rows,
        &[
            Field::named("Amount"),
            Field::named("Ref"),
            Field::named("Date"),
        ],
    );
    assert_eq!(read(&values), vec!["240.00", "A-17", "27 July"]);
}

/// A caption with its value underneath, which is the other half of how forms
/// are laid out.
#[test]
fn a_value_can_sit_on_the_line_under_its_label() {
    let rows = a_form(&[
        ("Address", 20.0, 40.0),
        ("14 Republic Street", 20.0, 46.0),
        ("Valletta", 20.0, 52.0),
    ]);
    let values = pick_in(
        &rows,
        &[Field {
            name: "Address".into(),
            label: "Address".into(),
            put: Where::Below,
            kind: Kind::Text,
        }],
    );
    assert_eq!(read(&values), vec!["14 Republic Street"]);
}

/// A label somewhere else on the sheet does not pull a value across the page.
#[test]
fn a_value_below_comes_from_under_the_label_and_not_from_across_the_page() {
    let rows = a_form(&[
        ("Address", 20.0, 40.0),
        ("Signature", 130.0, 40.0),
        ("14 Republic Street", 20.0, 46.0),
        ("J. Bezzina", 130.0, 46.0),
    ]);
    let values = pick_in(
        &rows,
        &[
            Field {
                name: "Address".into(),
                label: "Address".into(),
                put: Where::Below,
                kind: Kind::Text,
            },
            Field {
                name: "Signature".into(),
                label: "Signature".into(),
                put: Where::Below,
                kind: Kind::Text,
            },
        ],
    );
    assert_eq!(read(&values), vec!["14 Republic Street", "J. Bezzina"]);
}

/// Nothing written after a label is not the same as no label, and neither is
/// the same as a label found twice. All three come out of a spreadsheet as an
/// empty cell, so all three have to be told apart somewhere else.
#[test]
fn nothing_found_is_told_apart_from_nothing_written() {
    let rows = a_form(&[("Name: J. Bezzina", 20.0, 40.0), ("Date:", 20.0, 50.0)]);
    let values = pick_in(
        &rows,
        &[
            Field::named("Name"),
            Field::named("Date"),
            Field::named("Amount"),
        ],
    );
    assert_eq!(values[0], Value::Read("J. Bezzina".into()));
    assert_eq!(values[1], Value::Blank);
    assert_eq!(values[2], Value::NoLabel);

    // Every one of them says why, and every one comes out as an empty cell.
    assert!(values[0].why_not().is_none());
    for value in &values[1..] {
        assert_eq!(value.cell(), "");
        assert!(value.why_not().is_some(), "{value:?} does not say why");
    }
}

/// A label on the sheet twice is a guess, and a guess in a spreadsheet is
/// indistinguishable from a fact.
#[test]
fn a_label_found_twice_is_reported_rather_than_guessed_at() {
    let rows = a_form(&[("Date: 27 July", 20.0, 40.0), ("Date: 28 July", 20.0, 50.0)]);
    let values = pick_in(&rows, &[Field::named("Date")]);
    assert_eq!(values[0], Value::Twice(2));
    assert!(values[0].why_not().unwrap().contains("2 times"));
}

/// A word that reads the same as a label but is somewhere else is a value, not
/// a label — which is why labels are told apart by where the ink is rather than
/// by what it says.
#[test]
fn a_value_that_reads_like_a_label_is_still_a_value() {
    let rows = a_form(&[("Company: Date & Sons Ltd", 20.0, 40.0)]);
    let values = pick_in(&rows, &[Field::named("Company")]);
    assert_eq!(read(&values), vec!["Date & Sons Ltd"]);
}

/// The column heading and the label on the form need not be the same word.
#[test]
fn a_column_can_be_called_something_other_than_the_label() {
    let rows = a_form(&[("Full name of applicant: J. Bezzina", 20.0, 40.0)]);
    let field = Field::parse("Name=Full name of applicant").unwrap();
    assert_eq!(field.name, "Name");
    assert_eq!(field.label, "Full name of applicant");
    assert_eq!(read(&pick_in(&rows, &[field])), vec!["J. Bezzina"]);
}

/// The ways somebody writes a field down, and what they mean.
#[test]
fn a_field_is_written_the_way_somebody_would_type_it() {
    assert_eq!(Field::parse("Name").unwrap(), Field::named("Name"));
    assert_eq!(Field::parse("  Name  ").unwrap(), Field::named("Name"));

    let below = Field::parse("Address/below").unwrap();
    assert_eq!(below.put, Where::Below);
    assert_eq!(below.name, "Address");
    assert_eq!(below.label, "Address");

    let both = Field::parse("Addr=Postal address/below").unwrap();
    assert_eq!(both.name, "Addr");
    assert_eq!(both.label, "Postal address");
    assert_eq!(both.put, Where::Below);

    // And the ones that are not a field at all.
    assert!(Field::parse("").is_err());
    assert!(Field::parse("=nothing").is_err());
    assert!(Field::parse("nothing=").is_err());
}

/// A field says what it will look for, because somebody checks that before
/// running it over two hundred sheets.
#[test]
fn a_field_says_what_it_will_look_for() {
    assert_eq!(Field::named("Name").describe(), "Name");
    assert!(Field::parse("Name=Full name")
        .unwrap()
        .describe()
        .contains("Full name"));
    assert!(Field::parse("Address/below")
        .unwrap()
        .describe()
        .contains("under it"));
}

// ---------------------------------------------------------------------------
// The spreadsheet that comes out
// ---------------------------------------------------------------------------

fn a_harvest() -> Harvest {
    Harvest {
        fields: vec![Field::named("Name"), Field::named("Amount")],
        sheets: vec![
            Sheet {
                page: 1,
                values: vec![
                    Value::Read("J. Bezzina".into()),
                    Value::Read("240.00".into()),
                ],
            },
            Sheet {
                page: 2,
                values: vec![Value::Read("A. Borg".into()), Value::Blank],
            },
        ],
    }
}

/// A heading row, then one row per sheet, with the sheet number first so a
/// cell that has to be checked by hand can be found on the paper.
#[test]
fn the_spreadsheet_has_a_heading_and_a_row_per_sheet() {
    let rows = a_harvest().rows();
    assert_eq!(rows[0], vec!["Sheet", "Name", "Amount"]);
    assert_eq!(rows[1], vec!["1", "J. Bezzina", "240.00"]);
    assert_eq!(rows[2], vec!["2", "A. Borg", ""]);
}

/// A comma in a value does not become a new column, and a quotation mark does
/// not end the cell — which is the whole of what CSV gets wrong when it is
/// written by hand.
#[test]
fn a_comma_in_a_value_does_not_become_a_column() {
    let harvest = Harvest {
        fields: vec![Field::named("Address"), Field::named("Note")],
        sheets: vec![Sheet {
            page: 1,
            values: vec![
                Value::Read("14 Republic Street, Valletta".into()),
                Value::Read("said \"urgent\"".into()),
            ],
        }],
    };
    let csv = harvest.csv();
    let line = csv.lines().nth(1).unwrap();
    assert_eq!(
        line,
        "1,\"14 Republic Street, Valletta\",\"said \"\"urgent\"\"\""
    );

    // And a plain value is left alone, because the file is opened and looked at
    // by people as often as by spreadsheets.
    assert!(a_harvest().csv().contains("1,J. Bezzina,240.00"));
    assert!(a_harvest().csv().ends_with('\n'));
}

/// What came back empty is findable, sheet by sheet and field by field.
#[test]
fn every_gap_is_named_so_it_can_be_checked_by_hand() {
    let harvest = Harvest {
        fields: vec![Field::named("Name"), Field::named("Amount")],
        sheets: vec![
            Sheet {
                page: 1,
                values: vec![Value::Read("J. Bezzina".into()), Value::Blank],
            },
            Sheet {
                page: 2,
                values: vec![Value::NoLabel, Value::Twice(3)],
            },
        ],
    };
    let gaps = harvest.gaps();
    assert_eq!(gaps.len(), 3, "{gaps:?}");
    assert!(gaps[0].contains("sheet 1") && gaps[0].contains("Amount"));
    assert!(gaps[1].contains("sheet 2") && gaps[1].contains("Name"));
    assert!(gaps[2].contains("3 times"), "{}", gaps[2]);

    assert_eq!(harvest.read(), 1);
    assert_eq!(harvest.cells(), 4);
    assert!(harvest.verdict().contains("1 of 4"));
}

/// Nothing to harvest says so rather than dividing by no sheets.
#[test]
fn nothing_to_harvest_says_so() {
    let empty = Harvest {
        fields: Vec::new(),
        sheets: Vec::new(),
    };
    assert!(empty.verdict().contains("Nothing to harvest"));
    assert_eq!(empty.rows(), vec![vec!["Sheet".to_string()]]);
    assert!(empty.gaps().is_empty());
}

/// Words sharing a baseline do not always arrive as one row.
///
/// A form sets its captions and its rules separately, and a scan reads what it
/// finds — so `Name:` on the left and `Date:` on the right of one line can come
/// back as two rows with the same baseline. Read one row at a time, the value
/// for `Name` would be whatever that row happened to hold and the stopping rule
/// would never fire.
#[test]
fn a_line_that_arrives_as_several_rows_is_still_one_line() {
    let rows = a_form(&[
        ("Name: J. Bezzina", 20.0, 40.0),
        ("Date: 27 July 2024", 110.0, 40.0),
    ]);
    assert!(
        rows.len() > 1,
        "this test needs the line split up: {rows:?}"
    );
    let values = pick_in(&rows, &[Field::named("Name"), Field::named("Date")]);
    assert_eq!(read(&values), vec!["J. Bezzina", "27 July 2024"]);
}

/// The matcher is forgiving on purpose, and forgiveness costs something: a long
/// label also matches a run of its own words starting one word along. Both are
/// the same label in the same place, so they are one label — but a label really
/// printed twice, somewhere else on the sheet, is still two.
#[test]
fn a_label_matched_twice_in_the_same_place_is_one_label() {
    let one = a_form(&[("Full name of applicant: J. Bezzina", 20.0, 40.0)]);
    let field = Field::parse("Name=Full name of applicant").unwrap();
    assert_eq!(
        pick_in(&one, std::slice::from_ref(&field)),
        vec![Value::Read("J. Bezzina".into())]
    );

    // Twice on the sheet, in two places, is still reported.
    let two = a_form(&[
        ("Full name of applicant: J. Bezzina", 20.0, 40.0),
        ("Full name of applicant: A. Borg", 20.0, 60.0),
    ]);
    assert_eq!(pick_in(&two, &[field]), vec![Value::Twice(2)]);
}

// ---------------------------------------------------------------------------
// Undoing what the reader could not tell apart
// ---------------------------------------------------------------------------

/// A ring on paper is a capital O, a lower-case o and a nought all at once, and
/// the reader says so. That is the right answer for finding a word and the
/// wrong one for a spreadsheet: `240.O0` is not a sum of money.
#[test]
fn a_figure_read_with_letters_in_it_comes_out_as_a_figure() {
    assert_eq!(digits_where_they_belong("240.O0"), "240.00");
    assert_eq!(digits_where_they_belong("27 July 2O24"), "27 July 2024");
    assert_eq!(digits_where_they_belong("95.5O"), "95.50");
    // Every mark in this one is either a digit or a shape that carries no
    // information about whether it is a letter, so all of it resolves.
    assert_eq!(digits_where_they_belong("l2,5OO"), "12,500");

    // And what it will not do on its own: an S is a real confusion rather than
    // an identical shape, so by itself this stays as it was read. A field told
    // it is a number gets it right — see below.
    assert_eq!(digits_where_they_belong("24O.SO"), "24O.SO");
}

/// And a word that is a word keeps its letters. This is the half that matters:
/// a rule that fixes figures by breaking names is worse than no rule.
#[test]
fn a_word_that_is_a_word_is_left_alone() {
    for word in [
        "July",       // the l must not become a 1
        "J. Bezzina", // nor the B an 8
        "Borg",       // nor the o a 0 or the g a 6
        "Valletta",
        "Gozo",
        "A4", // a part number, not a misread figure
        "B2B",
        "",
    ] {
        assert_eq!(
            digits_where_they_belong(word),
            word,
            "'{word}' was changed and should not have been"
        );
    }
}

/// The mixed case, which is what a real line looks like.
#[test]
fn a_line_with_both_on_it_keeps_the_words_and_fixes_the_figures() {
    assert_eq!(
        digits_where_they_belong("Invoice A4 dated 27 July 2O24 for 1,2OO.5O"),
        "Invoice A4 dated 27 July 2024 for 1,200.50"
    );
}

/// Whether a word is a misread figure or a word with a digit in it.
#[test]
fn a_misread_figure_is_told_from_a_word_with_a_digit_in_it() {
    for figure in ["2O24", "240.O0", "l2", "5OO", "1", "1,2OO.5O"] {
        assert!(
            mostly_digits(figure),
            "'{figure}' was not taken for a figure"
        );
    }
    for word in ["A4", "July", "B2B", "Borg", "", "-"] {
        assert!(!mostly_digits(word), "'{word}' was taken for a figure");
    }
}

/// A field said to hold a number, holding something that is not one, is not
/// written down as though it were.
#[test]
fn a_number_that_is_not_one_is_reported_rather_than_written_down() {
    let rows = a_form(&[
        ("Amount: 240.O0", 20.0, 40.0),
        ("Total: J200.0O", 20.0, 50.0),
    ]);
    let values = pick_in(
        &rows,
        &[
            Field::parse("Amount/number").unwrap(),
            Field::parse("Total/number").unwrap(),
        ],
    );
    // The first resolves to a figure and is read.
    assert_eq!(values[0], Value::Read("240.00".into()));
    // The second does not. The J is a real misreading rather than a shape that
    // carries no information, so nothing here guesses at it — and because the
    // word as a whole is therefore not a figure, the O in it is left alone too.
    // What comes back is exactly what was read.
    assert_eq!(values[1], Value::NotANumber("J200.0O".into()));
    assert!(!values[1].is_read());
    // But the text is kept, because handing somebody 'J200.00' to correct beats
    // handing them an empty cell to go and find on the paper.
    assert_eq!(values[1].cell(), "J200.0O");
    assert!(values[1].why_not().unwrap().contains("not a number"));
}

/// The ways a figure appears on a form and is still a figure.
#[test]
fn a_number_is_still_a_number_written_the_way_people_write_them() {
    for figure in [
        "240", "240.00", "1,200.50", "-17", "+3.5", "(240.00)", "£240.00", "12 500",
    ] {
        assert!(
            looks_like_a_number(figure),
            "'{figure}' was not taken for a number"
        );
    }
    for not in ["", "J200.00", "twenty", "27 July", "-"] {
        assert!(!looks_like_a_number(not), "'{not}' was taken for a number");
    }
}

/// A text field is not held to being a number, and still gets its figures
/// tidied — a date is text with a year in it.
#[test]
fn a_text_field_keeps_its_words_and_still_gets_its_year_right() {
    let rows = a_form(&[("Date: 27 July 2O24", 20.0, 40.0)]);
    let values = pick_in(&rows, &[Field::named("Date")]);
    assert_eq!(values[0], Value::Read("27 July 2024".into()));
}

/// The suffixes can be written in any order, because somebody adding a second
/// one should not have to remember where the first went.
#[test]
fn the_suffixes_can_be_written_in_either_order() {
    for spec in ["Total/below/number", "Total/number/below"] {
        let field = Field::parse(spec).unwrap();
        assert_eq!(field.name, "Total", "{spec}");
        assert_eq!(field.put, Where::Below, "{spec}");
        assert_eq!(field.kind, Kind::Number, "{spec}");
    }
    assert!(Field::parse("Total/number")
        .unwrap()
        .describe()
        .contains("number"));
}

/// A field the caller has said holds a number gets a harder try than one that
/// has not — because "this column is money" is real knowledge that the words
/// on the page do not carry.
#[test]
fn a_field_told_it_is_a_number_tries_harder_than_one_that_is_not() {
    let rows = a_form(&[("Total: 24O.SO", 20.0, 40.0)]);

    // As text it is left as it was read. An S against a 5 is a reader being
    // wrong about two different shapes, not two marks that cannot be told
    // apart — so nothing changes it on the strength of the ink alone.
    let as_text = pick_in(&rows, &[Field::named("Total")]);
    assert_eq!(as_text[0], Value::Read("24O.SO".into()));

    // As a number it is settled, because "this column is money" is real
    // knowledge that the marks on the page do not carry.
    let as_number = pick_in(&rows, &[Field::parse("Total/number").unwrap()]);
    assert_eq!(as_number[0], Value::Read("240.50".into()));
}
