//! A list of things to put on sheets, read from a spreadsheet.
//!
//! The point of this module is one sheet each for two hundred people. Onionskin
//! can already put words on a printed page exactly; what it could not do was
//! that two hundred times, once per name, which is how certificates, tickets,
//! payslips and membership cards actually get made.
//!
//! What it reads is CSV, because every spreadsheet in the world writes CSV and
//! no other format is as safe to assume. Written out here rather than pulled in
//! from a crate: the format is small, the rules that matter are the quoting
//! ones, and they are written down below where they can be read.
//!
//! It also reads `.xlsx` and `.ods` directly — see [`crate::sheets`] — because
//! nobody *keeps* their list as a CSV, and telling somebody to go and save it
//! again is asking them to do a conversion this program could do. The columns
//! come out the same either way, so everything below this point neither knows
//! nor cares which it was.
//!
//! The rules, from RFC 4180 and from what spreadsheets really emit:
//!
//!   * Fields are separated by commas and records by line breaks.
//!   * A field may be wrapped in double quotes, and then it may contain
//!     commas and line breaks of its own — a postal address in one cell is
//!     the ordinary reason.
//!   * Inside quotes, two double quotes mean one literal double quote.
//!   * `\r\n` and `\n` are both line breaks. A lone `\r` is treated as one
//!     too, because old Mac exports still exist.
//!   * A byte-order mark at the start is Excel's doing and is dropped, since
//!     otherwise the first column is named "\u{feff}name" and never matches.
//!
//! Deliberately *not* implemented: semicolon separators, tab separators, and
//! character sets other than UTF-8. A file that is none of these is refused by
//! name rather than half-read into nonsense.

use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum RowsError {
    #[error("no list at {0}")]
    Missing(std::path::PathBuf),
    #[error("could not read {path}: {source}")]
    Io {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    #[error(
        "{path} is not a list Onionskin can read. It has to be a CSV file — \
         a spreadsheet saved as 'CSV' or 'Comma Separated Values'.\n    \
         ({why})"
    )]
    NotCsv {
        path: std::path::PathBuf,
        why: String,
    },
    #[error("could not read the spreadsheet {path}: {why}")]
    BadSpreadsheet {
        path: std::path::PathBuf,
        why: String,
    },
    #[error("{path} has no sheet called '{wanted}'. It has: {names}")]
    NoSuchSheet {
        path: std::path::PathBuf,
        wanted: String,
        names: String,
    },
    #[error("{path} has no columns at all — the first line names them, and it is empty")]
    NoColumns { path: std::path::PathBuf },
    #[error(
        "{path} has nothing in it but the column names, so there are no \
         sheets to make"
    )]
    NoRows { path: std::path::PathBuf },
    #[error(
        "line {line} has {found} value{} where the columns say there should be \
         {wanted}. A comma inside a value has to be inside quotes: \
         \"Smith, John\"",
        if *.found == 1 { "" } else { "s" }
    )]
    Ragged {
        line: usize,
        found: usize,
        wanted: usize,
    },
}

/// One row of the list: the columns by name, and which row it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// Column name to value. Ordered so that listing them is stable, which
    /// matters for the message that says what a list actually contains.
    pub values: BTreeMap<String, String>,
    /// Counted from 1, the way somebody talks about "the fourth one".
    pub number: usize,
}

impl Row {
    /// The value of a column, or `None` if this list has no such column.
    pub fn get(&self, column: &str) -> Option<&str> {
        self.values.get(column).map(|value| value.as_str())
    }
}

/// A list read off disk: its column names in the order the file had them, and
/// its rows.
#[derive(Debug, Clone)]
pub struct List {
    pub columns: Vec<String>,
    pub rows: Vec<Row>,
}

impl List {
    /// Read a list: a CSV file, or a spreadsheet.
    pub fn read(path: &Path) -> Result<List, RowsError> {
        List::read_sheet(path, None)
    }

    /// The same, naming which tab of a spreadsheet to read.
    ///
    /// `None` takes the first tab with anything on it — not simply the first,
    /// because a book whose first tab is an empty "Notes" is common and
    /// reporting "no columns at all" about it would be a lie about the file.
    pub fn read_sheet(path: &Path, sheet: Option<&str>) -> Result<List, RowsError> {
        if !path.is_file() {
            return Err(RowsError::Missing(path.to_path_buf()));
        }
        let bytes = std::fs::read(path).map_err(|source| RowsError::Io {
            path: path.to_path_buf(),
            source,
        })?;

        // A spreadsheet is a zip. Told apart by what is inside it rather than
        // by its name, because a `.csv` that is really a workbook happens
        // every time somebody renames a file to make a program accept it.
        if crate::sheets::is_a_spreadsheet(&bytes) {
            return List::from_spreadsheet(&bytes, path, sheet);
        }
        // A zip that is not a spreadsheet is not a list either, and control
        // characters are valid UTF-8 — so without this the CSV reader would
        // take a Word document apart into one column of rubbish and complain
        // that it has no rows.
        if bytes.starts_with(b"PK\x03\x04") {
            return Err(RowsError::NotCsv {
                path: path.to_path_buf(),
                why: "it is a zip file — a Word document, a damaged \
                      spreadsheet, or something else packed up — and not a \
                      list of values. Onionskin reads .xlsx, .ods and CSV; \
                      open it and save it again as one of those"
                    .into(),
            });
        }
        let text = String::from_utf8(bytes).map_err(|_| RowsError::NotCsv {
            path: path.to_path_buf(),
            why: "it is not UTF-8 text. Save it again choosing UTF-8 as the \
                  character set"
                .into(),
        })?;
        List::parse(&text, path)
    }

    /// A list out of a spreadsheet's cells.
    fn from_spreadsheet(
        bytes: &[u8],
        path: &Path,
        wanted: Option<&str>,
    ) -> Result<List, RowsError> {
        let book = crate::sheets::read(bytes).map_err(|why| RowsError::BadSpreadsheet {
            path: path.to_path_buf(),
            why: why.to_string(),
        })?;
        let sheet = match wanted {
            Some(name) => book.named(name).ok_or_else(|| RowsError::NoSuchSheet {
                path: path.to_path_buf(),
                wanted: name.to_string(),
                names: book.names().join(", "),
            })?,
            None => book
                .first_with_anything()
                .ok_or_else(|| RowsError::NoColumns {
                    path: path.to_path_buf(),
                })?,
        };

        let mut cells = sheet.rows.iter();
        let Some(header) = cells.next() else {
            return Err(RowsError::NoColumns {
                path: path.to_path_buf(),
            });
        };
        let columns: Vec<String> = header.iter().map(|name| name.trim().to_string()).collect();
        if columns.iter().all(|name| name.is_empty()) {
            return Err(RowsError::NoColumns {
                path: path.to_path_buf(),
            });
        }

        let mut rows = Vec::new();
        for record in cells {
            // A row of nothing is a gap in the sheet, not a person to print a
            // certificate for.
            if record.iter().all(|value| value.trim().is_empty()) {
                continue;
            }
            // Short rows are ordinary in a spreadsheet — the cells to the
            // right were simply never filled in — so they are padded rather
            // than refused, unlike a ragged CSV where a missing field means a
            // comma in the wrong place.
            let values = columns
                .iter()
                .cloned()
                .zip(
                    record
                        .iter()
                        .map(|value| value.trim().to_string())
                        .chain(std::iter::repeat(String::new())),
                )
                .collect();
            rows.push(Row {
                values,
                number: rows.len() + 1,
            });
        }

        if rows.is_empty() {
            return Err(RowsError::NoRows {
                path: path.to_path_buf(),
            });
        }
        Ok(List { columns, rows })
    }

    /// The same, from text already in hand.
    pub fn parse(text: &str, path: &Path) -> Result<List, RowsError> {
        let records = split_records(text.strip_prefix('\u{feff}').unwrap_or(text));
        let mut records = records.into_iter();

        let Some(header) = records.next() else {
            return Err(RowsError::NoColumns {
                path: path.to_path_buf(),
            });
        };
        let columns: Vec<String> = header.iter().map(|name| name.trim().to_string()).collect();
        if columns.iter().all(|name| name.is_empty()) {
            return Err(RowsError::NoColumns {
                path: path.to_path_buf(),
            });
        }

        let mut rows = Vec::new();
        for (index, record) in records.enumerate() {
            // A trailing newline leaves one empty record, which is not a row
            // with no values in it — it is the end of the file.
            if record.len() == 1 && record[0].trim().is_empty() {
                continue;
            }
            if record.len() != columns.len() {
                return Err(RowsError::Ragged {
                    // +2: one for the header line, one because people count
                    // lines from 1.
                    line: index + 2,
                    found: record.len(),
                    wanted: columns.len(),
                });
            }
            let values = columns
                .iter()
                .cloned()
                .zip(record.into_iter().map(|value| value.trim().to_string()))
                .collect();
            rows.push(Row {
                values,
                number: rows.len() + 1,
            });
        }

        if rows.is_empty() {
            return Err(RowsError::NoRows {
                path: path.to_path_buf(),
            });
        }
        Ok(List { columns, rows })
    }

    /// The column names, for a message that has to say what is on offer.
    pub fn describe_columns(&self) -> String {
        self.columns
            .iter()
            .filter(|name| !name.is_empty())
            .map(|name| format!("{{{name}}}"))
            .collect::<Vec<_>>()
            .join(", ")
    }
}

/// Split the whole file into records of fields, honouring quotes.
///
/// One pass, character by character, because the quoting rules mean a line
/// break is only a record break when it is not inside quotes — so the file
/// cannot simply be split on newlines first.
fn split_records(text: &str) -> Vec<Vec<String>> {
    let mut records: Vec<Vec<String>> = Vec::new();
    let mut record: Vec<String> = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();

    while let Some(ch) = chars.next() {
        if quoted {
            match ch {
                '"' => {
                    // Two quotes inside quotes are one quote; a single one
                    // ends the quoting.
                    if chars.peek() == Some(&'"') {
                        chars.next();
                        field.push('"');
                    } else {
                        quoted = false;
                    }
                }
                _ => field.push(ch),
            }
            continue;
        }
        match ch {
            '"' => quoted = true,
            ',' => record.push(std::mem::take(&mut field)),
            '\r' | '\n' => {
                // \r\n is one break, not two.
                if ch == '\r' && chars.peek() == Some(&'\n') {
                    chars.next();
                }
                record.push(std::mem::take(&mut field));
                records.push(std::mem::take(&mut record));
            }
            _ => field.push(ch),
        }
    }

    // Whatever is still in hand at the end of the file is the last record,
    // unless the file ended on a line break and there is nothing left.
    if !field.is_empty() || !record.is_empty() {
        record.push(field);
        records.push(record);
    }
    records
}

/// Put a row's values into a line of text where `{column}` appears.
///
/// `{number}` is the row's own place in the list, so tickets and invoices can
/// be numbered without a column of numbers in the spreadsheet.
///
/// A name in braces that is not a column is left exactly as it was rather than
/// blanked. Printing `{nmae}` on two hundred certificates is a bad day, but it
/// is a visible one — quietly printing nothing there is worse, because the
/// stack looks right until somebody reads it.
pub fn fill(template: &str, row: &Row) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;

    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else {
            // An unmatched brace is a brace.
            out.push_str(&rest[open..]);
            return out;
        };
        let name = &after[..close];
        // A column wins over the counter. `{number}` counts the sheets when
        // the list has no column of that name — but a list that *does* have
        // one means those numbers, and a run of invoices printed 1, 2, 3
        // instead of their real numbers is wrong in a way nobody notices
        // until they are in the post.
        match row.get(name) {
            Some(value) => out.push_str(value),
            None if name == "number" => out.push_str(&row.number.to_string()),
            None => {
                out.push('{');
                out.push_str(name);
                out.push('}');
            }
        }
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    out
}

/// Every `{name}` in a template that this list has no column for.
///
/// Checked before a single sheet is made, because two hundred certificates
/// reading `{nmae}` is a discovery to make now rather than at the printer.
pub fn unknown_columns(templates: &[String], list: &List) -> Vec<String> {
    let mut missing = Vec::new();
    for template in templates {
        let mut rest = template.as_str();
        while let Some(open) = rest.find('{') {
            let after = &rest[open + 1..];
            let Some(close) = after.find('}') else { break };
            let name = &after[..close];
            if name != "number"
                && !list.columns.iter().any(|column| column == name)
                && !missing.iter().any(|already| already == name)
            {
                missing.push(name.to_string());
            }
            rest = &after[close + 1..];
        }
    }
    missing
}

#[cfg(test)]
mod tests;
