//! Filled-in forms, back into a spreadsheet.
//!
//! `batch` goes one way: a list of two hundred names becomes two hundred sheets.
//! This is the other way. The sheets come back — signed, filled in, corrected by
//! hand — and somebody has to get what is on them into a spreadsheet. That
//! somebody is currently a person with a keyboard and a stack of paper, which is
//! where a day goes and where the typing mistakes come from.
//!
//! # How a value is found
//!
//! Not by measuring. A form says what its fields are, in print, right next to
//! them: `Name:`, `Date:`, `Amount:`. So the label is what is named, and the
//! value is whatever is written after it.
//!
//! ```text
//!   Name:  J. Bezzina          Date:  27 July 2024
//!          ^^^^^^^^^^                 ^^^^^^^^^^^^
//! ```
//!
//! The part that makes this work on a real form is knowing where a value
//! *stops*. Read naively, `Name` on the line above would come out as
//! "J. Bezzina Date: 27 July 2024" — the whole rest of the line. So every label
//! is found first, and a value runs from its own label to **the next label on
//! that row**. Two fields on one line is the commonest form layout there is, and
//! it is the case that would otherwise be silently wrong rather than obviously
//! wrong.
//!
//! # What it will not pretend
//!
//! A scan is read letter by letter and letters are sometimes read wrongly. So:
//!
//!   * A field whose label is not on the sheet comes back as nothing, and the
//!     sheet is named. It does not come back as an empty string, because an
//!     empty string in a spreadsheet reads as "this was blank" and this was not
//!     blank — it was not found.
//!   * A label found twice on one sheet is reported rather than guessed at.
//!   * Handwriting is not read at all, and this says so rather than returning
//!     something. The reader matches printed letter shapes against fonts on this
//!     machine; a signature has nothing to match.

use crate::anchor::{self, Row};
use crate::letters::{PageText, Rect};

/// A column to fill in, and the label on the form that finds it.
#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    /// The heading in the spreadsheet.
    pub name: String,
    /// What is printed on the form beside the value. Usually the same as the
    /// name, which is why naming a field is usually one word.
    pub label: String,
    /// Whether the value sits after the label on the same line, or under it.
    pub put: Where,
    /// What sort of thing the value is, which decides how hard to try when the
    /// reading is ambiguous.
    pub kind: Kind,
}

/// What a field holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Kind {
    /// Words. Taken as read.
    #[default]
    Text,
    /// A number. Insisted upon: a value that does not come out as one after the
    /// shape confusions are undone is reported rather than written down as
    /// though it were a figure.
    Number,
}

/// Where a value sits relative to the label that names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Where {
    /// On the same line, to the right of the label. The usual arrangement.
    #[default]
    After,
    /// On the line underneath — a box with its caption above it.
    Below,
}

impl Field {
    /// A field whose label is its name, which is the usual case.
    pub fn named(name: &str) -> Field {
        Field {
            name: name.trim().to_string(),
            label: name.trim().to_string(),
            put: Where::After,
            kind: Kind::Text,
        }
    }

    /// `Name`, or `Name=Full name of applicant`, or `Name/below`.
    ///
    /// Parsed here rather than in the command line, because the window and the
    /// web page ask the same question and three spellings of it is how one of
    /// them comes to behave differently.
    pub fn parse(spec: &str) -> Result<Field, String> {
        let spec = spec.trim();
        if spec.is_empty() {
            return Err("a field with no name is not a column".into());
        }
        // The suffixes, in any order and any number: `Amount/number`,
        // `Total/below/number`. Peeled off the end one at a time rather than
        // matched as a set, because somebody writing the second one down should
        // not have to remember which order the first one went in.
        let mut rest = spec;
        let mut put = Where::After;
        let mut kind = Kind::Text;
        loop {
            let peeled = if let Some(head) = rest.strip_suffix("/below") {
                put = Where::Below;
                head
            } else if let Some(head) = rest.strip_suffix("/after") {
                put = Where::After;
                head
            } else if let Some(head) = rest.strip_suffix("/number") {
                kind = Kind::Number;
                head
            } else {
                break;
            };
            rest = peeled.trim_end();
        }
        let rest = rest.trim();
        let (name, label) = match rest.split_once('=') {
            Some((name, label)) => (name.trim(), label.trim()),
            None => (rest, rest),
        };
        if name.is_empty() {
            return Err(format!("'{spec}' has no column name in it"));
        }
        if label.is_empty() {
            return Err(format!("'{spec}' has no label to look for on the form"));
        }
        Ok(Field {
            name: name.to_string(),
            label: label.to_string(),
            put,
            kind,
        })
    }

    pub fn describe(&self) -> String {
        let mut said = match self.name == self.label {
            true => self.name.clone(),
            false => format!("{} (labelled '{}')", self.name, self.label),
        };
        if self.put == Where::Below {
            said.push_str(", on the line under it");
        }
        if self.kind == Kind::Number {
            said.push_str(", a number");
        }
        said
    }
}

/// What one field on one sheet came to.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// Read, and this is what it says.
    Read(String),
    /// The label is on the sheet and there is nothing after it.
    Blank,
    /// The label is not on the sheet at all.
    NoLabel,
    /// The label is on the sheet more than once, so which one is meant is a
    /// guess — and a guess in a spreadsheet is indistinguishable from a fact.
    Twice(usize),
    /// A field said to hold a number, holding something that is not one.
    ///
    /// The text is kept, because it is better to hand somebody `J200.00` to
    /// correct than an empty cell to go and find on the paper. But it is not
    /// counted as read, so it appears in the list of things to check.
    NotANumber(String),
}

impl Value {
    /// What goes in the spreadsheet cell.
    ///
    /// Nothing found and nothing written are both empty, because a spreadsheet
    /// has one way of saying empty. Which is why they are told apart in the
    /// report instead, where there is room to say why.
    pub fn cell(&self) -> String {
        match self {
            Value::Read(text) | Value::NotANumber(text) => text.clone(),
            _ => String::new(),
        }
    }

    pub fn is_read(&self) -> bool {
        matches!(self, Value::Read(_))
    }

    /// Why there is nothing in the cell, for anything that is not a reading.
    pub fn why_not(&self) -> Option<String> {
        match self {
            Value::Read(_) => None,
            Value::Blank => Some("nothing written after the label".into()),
            Value::NoLabel => Some("the label is not on this sheet".into()),
            Value::Twice(n) => Some(format!("the label is on this sheet {n} times")),
            Value::NotANumber(text) => Some(format!("'{text}' is not a number")),
        }
    }
}

/// One sheet's worth.
#[derive(Debug, Clone)]
pub struct Sheet {
    /// Which page of the scan, counted from 1.
    pub page: usize,
    pub values: Vec<Value>,
}

/// The whole stack.
#[derive(Debug, Clone)]
pub struct Harvest {
    pub fields: Vec<Field>,
    pub sheets: Vec<Sheet>,
}

impl Harvest {
    /// The spreadsheet: a heading row, then one row per sheet.
    pub fn rows(&self) -> Vec<Vec<String>> {
        let mut out = vec![std::iter::once("Sheet".to_string())
            .chain(self.fields.iter().map(|field| field.name.clone()))
            .collect::<Vec<_>>()];
        for sheet in &self.sheets {
            out.push(
                std::iter::once(sheet.page.to_string())
                    .chain(sheet.values.iter().map(Value::cell))
                    .collect(),
            );
        }
        out
    }

    /// The same, as the comma-separated text a spreadsheet opens.
    pub fn csv(&self) -> String {
        self.rows()
            .iter()
            .map(|row| {
                row.iter()
                    .map(|cell| quoted(cell))
                    .collect::<Vec<_>>()
                    .join(",")
            })
            .collect::<Vec<_>>()
            .join("\n")
            + "\n"
    }

    /// How many cells were actually read.
    pub fn read(&self) -> usize {
        self.sheets
            .iter()
            .flat_map(|sheet| sheet.values.iter())
            .filter(|value| value.is_read())
            .count()
    }

    /// How many there could have been.
    pub fn cells(&self) -> usize {
        self.sheets.len() * self.fields.len()
    }

    /// Every cell that came back with nothing, and why.
    ///
    /// This is the part somebody reads. A harvest that filled nine hundred of a
    /// thousand cells is a good day's work and a hundred things to check by
    /// hand, and the hundred have to be findable.
    pub fn gaps(&self) -> Vec<String> {
        let mut said = Vec::new();
        for sheet in &self.sheets {
            for (field, value) in self.fields.iter().zip(&sheet.values) {
                if let Some(why) = value.why_not() {
                    said.push(format!("  sheet {}: {} — {why}", sheet.page, field.name));
                }
            }
        }
        said
    }

    /// One line saying how it went.
    pub fn verdict(&self) -> String {
        let cells = self.cells();
        if cells == 0 {
            return "Nothing to harvest: no sheets, or no fields.".to_string();
        }
        let read = self.read();
        format!(
            "{read} of {cells} cell(s) read, off {} sheet(s).",
            self.sheets.len()
        )
    }
}

/// A cell as a spreadsheet wants it: quoted only where it has to be.
///
/// Quoting everything would be simpler and would make the file harder for a
/// person to read, which matters because the commonest thing done with this
/// file is to open it and look at it.
fn quoted(cell: &str) -> String {
    if cell.contains([',', '"', '\n', '\r']) {
        return format!("\"{}\"", cell.replace('"', "\"\""));
    }
    cell.to_string()
}

/// How far below a label the line under it may be, as a share of the label's
/// own height.
///
/// A caption and the box under it sit about a line and a half apart. Twice the
/// label's height reaches the next line and not the one after it.
const BELOW_REACH: f64 = 2.6;

/// How far a value on the line below may sit to the left or right of its label
/// and still belong to it, in millimetres.
///
/// A value written under a caption is rarely aligned to the character; a
/// centimetre either way is somebody filling in a box by hand.
const BELOW_SLACK_MM: f64 = 10.0;

/// Pick every field off one sheet.
pub fn pick_from(page: &PageText, fields: &[Field]) -> Vec<Value> {
    pick_in(&anchor::rows(page), fields)
}

/// The same, from rows rather than from a scanned page.
///
/// The picking is here rather than in [`pick_from`] because rows are what the
/// matching actually works on, and they can be had from a document's own words
/// as well as from a scan — which is what makes this testable without a scanner
/// and usable on a PDF nobody has printed yet.
pub fn pick_in(rows: &[Row], fields: &[Field]) -> Vec<Value> {
    // Every label first, because a value ends where the next label begins and
    // that cannot be known one field at a time.
    let found: Vec<Vec<Rect>> = fields
        .iter()
        .map(|field| one_each(anchor::boxes_in(rows, &field.label)))
        .collect();
    let every_label: Vec<Rect> = found.iter().flatten().copied().collect();

    fields
        .iter()
        .zip(&found)
        .map(|(field, boxes)| match boxes.len() {
            0 => Value::NoLabel,
            1 => settled(
                value_after(rows, boxes[0], &every_label, field.put),
                field.kind,
            ),
            many => Value::Twice(many),
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Undoing what the reader could not tell apart
// ---------------------------------------------------------------------------

/// A reading, with the shapes the reader confuses resolved where the context
/// says which they were.
fn settled(value: Value, kind: Kind) -> Value {
    let Value::Read(text) = value else {
        return value;
    };
    let text = digits_where_they_belong(&text);
    if kind == Kind::Text {
        return Value::Read(text);
    }
    if looks_like_a_number(&text) {
        return Value::Read(text);
    }
    // The caller has said this field is a number, which is more than the words
    // alone can say. So a second, harder try: every ambiguous shape read as the
    // digit it could be, rather than only those in words that already look like
    // figures. `l2,5OO` is a word by its own shape and £12,500 by its column.
    let insisted: String = text.chars().map(|c| as_a_digit(c).unwrap_or(c)).collect();
    match looks_like_a_number(&insisted) {
        true => Value::Read(insisted),
        // And if it is still not a number, it is not one. Nothing here invents
        // a confusion that this program has not already decided a reader makes.
        false => Value::NotANumber(text),
    }
}

/// The letters a reader confuses with digits, and which digit each really is.
///
/// Exactly the set `anchor` folds together and no more, because that is the set
/// this program has already decided a reader cannot tell apart. Adding to it
/// here would be inventing a confusion in order to fix it — and every entry is
/// a way for a correct reading to be changed into a wrong one.
fn as_a_digit(c: char) -> Option<char> {
    Some(match c {
        'O' | 'o' => '0',
        'l' | 'I' | 'i' => '1',
        'S' | 's' => '5',
        'Z' | 'z' => '2',
        'B' | 'b' => '8',
        'G' | 'g' => '6',
        _ => return None,
    })
}

/// The two of those that are not really a confusion at all.
///
/// A capital O, a lower-case o and a nought are the same ring in most faces;
/// so are a lower-case l, a capital I and a one. There is nothing in the ink to
/// tell them apart and there never was — which is different in kind from `S`
/// against `5`, where the shapes differ and the reader is merely sometimes
/// wrong.
///
/// That difference is what lets `5OO` be read as five hundred while `B2B` is
/// left alone. Both are one digit and two letters that could be digits; only one
/// of them is made of shapes that carry no information.
fn is_the_same_shape_as_a_digit(c: char) -> bool {
    matches!(c, 'O' | 'o' | 'l' | 'I' | 'i')
}

/// Resolve the ambiguous shapes inside anything that is plainly a number.
///
/// Word by word, and only where the word is already mostly digits. `2O24` in a
/// date becomes `2024`; `July` keeps its `l`, because `Ju1y` is not a month and
/// nothing about that word says it was meant to be a figure.
///
/// This is why it is done on the whole value rather than only on fields marked
/// as numbers: a date is text with a year in it, and an address has a house
/// number in it, and both come out wrong without it.
fn digits_where_they_belong(text: &str) -> String {
    text.split(' ')
        .map(|word| match mostly_digits(word) {
            false => word.to_string(),
            true => word
                .chars()
                .map(|c| as_a_digit(c).unwrap_or(c))
                .collect::<String>(),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Whether a word is a figure that has been read imperfectly, rather than a
/// word with a digit in it.
///
/// At least one real digit, and nothing else in it but shapes that carry no
/// information about whether they are a letter or a digit.
///
/// So `2O24`, `5OO` and `1,2OO.5O` are figures read imperfectly, and `B2B`,
/// `A4` and `July` are words. The line is drawn at
/// [`is_the_same_shape_as_a_digit`] rather than at a majority of digits,
/// because a majority is a guess about how much of a word was misread and this
/// is a fact about which marks can be told apart at all.
///
/// It is deliberately shy. A field the caller has *said* is a number gets a
/// harder try; this is the rule for everything else, where being wrong means
/// changing a word somebody wrote into a number they did not.
fn mostly_digits(word: &str) -> bool {
    let letters: Vec<char> = word.chars().filter(|c| c.is_alphanumeric()).collect();
    letters.iter().any(char::is_ascii_digit)
        && letters
            .iter()
            .all(|c| c.is_ascii_digit() || is_the_same_shape_as_a_digit(*c))
}

/// Whether a value is a figure, written the way people write figures.
///
/// Thousands separators, a currency sign, a sign, brackets for a negative: all
/// of them are how a number appears on a form, and none of them stops it being
/// one.
fn looks_like_a_number(text: &str) -> bool {
    // Only what a form really puts around a figure is taken off. Skipping every
    // leading non-digit instead would make `J200.00` a number, which is exactly
    // the misreading this is here to catch.
    let bare: String = text
        .chars()
        .filter(|c| {
            !matches!(
                c,
                ',' | ' ' | '(' | ')' | '\u{00A0}' | '£' | '$' | '€' | '¥' | '%'
            )
        })
        .collect();
    !bare.is_empty() && bare.parse::<f64>().is_ok()
}

/// One box per label, where the matcher offered several overlapping ones.
///
/// The anchor matcher is deliberately forgiving, because a scan is never read
/// perfectly — and forgiving means `Full name of applicant` also matches the
/// words `name of applicant: J.` starting one word along. Both boxes are the
/// same label; they overlap. Left alone they would come back as a label found
/// twice and the value would be dropped, which is the matcher's forgiveness
/// costing exactly what it was meant to buy.
///
/// Boxes that do *not* overlap are a different matter and are kept: those are
/// the same label really printed twice on the sheet, which is a thing to report
/// rather than to guess about.
fn one_each(mut boxes: Vec<Rect>) -> Vec<Rect> {
    boxes.sort_by(|a, b| a.y_mm.total_cmp(&b.y_mm).then(a.x_mm.total_cmp(&b.x_mm)));
    let mut kept: Vec<Rect> = Vec::new();
    for box_ in boxes {
        match kept.iter_mut().find(|other| overlaps(**other, box_)) {
            // The wider of two overlapping readings, because the label is at
            // least as long as the longest run that matched it.
            Some(other) => *other = union(*other, box_),
            None => kept.push(box_),
        }
    }
    kept
}

/// Whether two boxes share any paper.
fn overlaps(a: Rect, b: Rect) -> bool {
    a.x_mm < b.right_mm()
        && b.x_mm < a.right_mm()
        && a.y_mm < b.bottom_mm()
        && b.y_mm < a.bottom_mm()
}

/// The box round two boxes.
fn union(a: Rect, b: Rect) -> Rect {
    let x_mm = a.x_mm.min(b.x_mm);
    let y_mm = a.y_mm.min(b.y_mm);
    Rect {
        x_mm,
        y_mm,
        width_mm: a.right_mm().max(b.right_mm()) - x_mm,
        height_mm: a.bottom_mm().max(b.bottom_mm()) - y_mm,
    }
}

/// The words that belong to a label, given every other label on the sheet.
fn value_after(rows: &[Row], label: Rect, every_label: &[Rect], put: Where) -> Value {
    let wanted = match put {
        Where::After => on_the_same_line(rows, label, every_label),
        Where::Below => on_the_line_below(rows, label, every_label),
    };
    match wanted.trim() {
        "" => Value::Blank,
        text => Value::Read(text.to_string()),
    }
}

/// Whether a word is part of a label rather than part of a value.
///
/// Compared by where the ink is rather than by what it says, because the same
/// word can be both: a form with a `Name:` field filled in with a company called
/// `Name & Sons` has to keep them apart, and only the position can.
fn is_a_label(word: Rect, labels: &[Rect]) -> bool {
    labels.iter().any(|label| {
        let across = word.x_mm < label.right_mm() && label.x_mm < word.right_mm();
        let down = word.y_mm < label.y_mm + label.height_mm && label.y_mm < word.bottom_mm();
        across && down
    })
}

/// The value on the same line: everything right of the label, stopping at the
/// next label along.
fn on_the_same_line(rows: &[Row], label: Rect, every_label: &[Rect]) -> String {
    let middle = label.y_mm + label.height_mm / 2.0;
    // Every row the label's line passes through, gathered together before
    // anything is decided. Words sharing a baseline do not always arrive as one
    // row — two captions across a page are two — and a value read row by row
    // would come out in the order the rows happened to be in rather than in the
    // order the words are printed.
    let mut words: Vec<&(String, Rect, f64)> = rows
        .iter()
        .filter(|row| {
            let (top, bottom) = row_extent(row);
            middle >= top && middle <= bottom
        })
        .flat_map(|row| row.words.iter())
        .filter(|(_, rect, _)| rect.x_mm >= label.right_mm() - 0.5)
        .collect();
    words.sort_by(|a, b| a.1.x_mm.total_cmp(&b.1.x_mm));

    // Rightwards from the label, stopping the moment another label starts —
    // which is what keeps `Name` on a line shared with `Date` from swallowing
    // the date.
    let mut said = Vec::new();
    for (text, rect, _) in words {
        if is_a_label(*rect, every_label) {
            break;
        }
        said.push(text.clone());
    }
    said.join(" ")
}

/// The value on the line below, under the label rather than beside it.
fn on_the_line_below(rows: &[Row], label: Rect, every_label: &[Rect]) -> String {
    let reach = label.bottom_mm() + label.height_mm * BELOW_REACH;
    let below: Vec<&Row> = rows
        .iter()
        .filter(|row| row_extent(row).0 > label.bottom_mm() - label.height_mm * 0.3)
        .filter(|row| row_extent(row).0 <= reach)
        .collect();
    let Some(nearest) = below
        .iter()
        .map(|row| row_extent(row).0)
        .fold(None::<f64>, |best, top| {
            Some(best.map_or(top, |b| b.min(top)))
        })
    else {
        return String::new();
    };

    // Every row on that line, not merely the first of them. Words sharing a
    // baseline do not always arrive as one row — a caption on the left and
    // another on the right of the same line are two — and taking only the first
    // would look for the value under the wrong caption.
    let mut words: Vec<&(String, Rect, f64)> = below
        .iter()
        .filter(|row| row_extent(row).0 <= nearest + label.height_mm * 0.5)
        .flat_map(|row| row.words.iter())
        .filter(|(_, rect, _)| {
            rect.right_mm() > label.x_mm - BELOW_SLACK_MM
                && rect.x_mm < label.right_mm() + BELOW_SLACK_MM
        })
        .filter(|(_, rect, _)| !is_a_label(*rect, every_label))
        .collect();
    words.sort_by(|a, b| a.1.x_mm.total_cmp(&b.1.x_mm));
    words
        .iter()
        .map(|(text, _, _)| text.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The top and bottom of a row's ink.
fn row_extent(row: &Row) -> (f64, f64) {
    let top = row
        .words
        .iter()
        .map(|(_, rect, _)| rect.y_mm)
        .fold(f64::MAX, f64::min);
    let bottom = row
        .words
        .iter()
        .map(|(_, rect, _)| rect.bottom_mm())
        .fold(f64::MIN, f64::max);
    (top, bottom)
}

#[cfg(test)]
#[path = "harvest/tests.rs"]
mod tests;
