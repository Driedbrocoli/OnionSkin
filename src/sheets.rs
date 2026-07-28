//! Reading a spreadsheet: `.xlsx` and `.ods`, without a spreadsheet program.
//!
//! [`crate::rows`] reads CSV, and CSV is the right thing to read: every
//! spreadsheet in the world writes it. But nobody *keeps* their list as a CSV.
//! They keep it in Excel, or in LibreOffice Calc, or in whatever their phone
//! calls it, and the answer up to now was a refusal that told them to go and
//! save it again:
//!
//! ```text
//! error: staff.xlsx is not a list Onionskin can read.
//!     (it is a spreadsheet file, not a CSV. Open it and choose
//!      File → Save As → CSV)
//! ```
//!
//! Which is a program telling somebody to do a conversion the program could
//! have done — and doing it wrongly is easy, because Excel's "CSV" menu has
//! four entries and one of them is not UTF-8.
//!
//! Both formats are a zip of XML files, and both halves of that already exist
//! here for reading Word and OpenDocument text: [`crate::office::unzip`] and
//! [`crate::office::xml`]. So this is the same job again, and costs no new
//! dependency.
//!
//! # What is taken from a cell
//!
//! What the spreadsheet *shows*, not what it stores. A list is being read so
//! that its values can be printed on paper for a person to read, and a person
//! reading a certificate wants "14 July 2024", not 45487.
//!
//! For OpenDocument that is easy: the displayed text is in the file, in
//! `<text:p>`, right beside the raw value.
//!
//! Excel does not store it. A date in `.xlsx` is a plain number with a *format*
//! attached, so the formats have to be read as well — `xl/styles.xml`, which
//! cell uses which — and a cell whose format is a date is turned back into one.
//! Anything else keeps the digits exactly as the file has them: reading
//! `1200.50` as a number and printing it again risks `1200.4999999999998`, and
//! the file already has the right characters in it.
//!
//! # Empty cells that are not there
//!
//! Neither format writes a cell that is empty. Excel gives every cell an
//! address — `<c r="D7">` — so a gap is found by the jump. OpenDocument
//! instead says "and now sixteen thousand empty cells", which is why repeats
//! are counted rather than expanded: a hundred thousand rows each claiming
//! 16,384 trailing blanks is a perfectly ordinary file and two billion empty
//! strings is not a perfectly ordinary amount of memory.

use crate::office::unzip::{Archive, ZipError};
use crate::office::xml::{decode, Event, Reader};

/// Excel's own limits, used as this reader's limits.
///
/// A file claiming more than a spreadsheet can hold is damaged or hostile, and
/// either way the answer is to stop rather than to allocate.
const MOST_ROWS: usize = 1_048_576;
const MOST_COLUMNS: usize = 16_384;

/// One sheet of a book: its name and its cells, as they are shown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sheet {
    pub name: String,
    /// Rows of cells. Trailing empty cells and trailing empty rows are
    /// dropped, because a spreadsheet is full of them and none of them mean
    /// anything.
    pub rows: Vec<Vec<String>>,
}

/// A spreadsheet file, read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Book {
    pub sheets: Vec<Sheet>,
}

impl Book {
    /// A sheet by name, ignoring case and surrounding space — because somebody
    /// typing `--sheet staff` should not have to know it is called `Staff `.
    pub fn named(&self, wanted: &str) -> Option<&Sheet> {
        let wanted = wanted.trim().to_lowercase();
        self.sheets
            .iter()
            .find(|sheet| sheet.name.trim().to_lowercase() == wanted)
    }

    /// The first sheet with anything on it.
    ///
    /// Not simply the first: a book whose first tab is a blank "Notes" is
    /// common enough, and reading it would report "no columns at all" about a
    /// file that plainly has a list in it.
    pub fn first_with_anything(&self) -> Option<&Sheet> {
        self.sheets.iter().find(|sheet| !sheet.rows.is_empty())
    }

    pub fn names(&self) -> Vec<&str> {
        self.sheets
            .iter()
            .map(|sheet| sheet.name.as_str())
            .collect()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SheetError {
    #[error("{0}")]
    Zip(#[from] ZipError),
    #[error(
        "this is a zip file but not a spreadsheet Onionskin knows — there is \
         no {0} in it. Excel writes .xlsx and LibreOffice writes .ods; a .xls \
         from before 2007 has to be saved again as one of those."
    )]
    NotASpreadsheet(&'static str),
    #[error("the spreadsheet has no sheets in it at all")]
    NoSheets,
    #[error(
        "the spreadsheet says it has {0} rows, which is more than a \
         spreadsheet can hold. The file is damaged."
    )]
    TooBig(usize),
}

/// Whether these bytes look like a spreadsheet rather than a Word file.
///
/// Both are zips, and both start `PK`. Told apart by what is inside, because
/// the extension is somebody's opinion and the contents are a fact.
pub fn is_a_spreadsheet(bytes: &[u8]) -> bool {
    let Ok(archive) = Archive::open(bytes) else {
        return false;
    };
    archive.has("xl/workbook.xml") || is_an_ods(&archive)
}

fn is_an_ods(archive: &Archive) -> bool {
    if !archive.has("content.xml") {
        return false;
    }
    // An .odt is also a zip with a content.xml, so the mimetype settles it —
    // and where there is none, the presence of a spreadsheet body does.
    match archive.read("mimetype") {
        Ok(kind) => String::from_utf8_lossy(&kind).contains("spreadsheet"),
        Err(_) => archive
            .read("content.xml")
            .map(|bytes| decode(&bytes).contains("office:spreadsheet"))
            .unwrap_or(false),
    }
}

/// Read a spreadsheet from its bytes.
pub fn read(bytes: &[u8]) -> Result<Book, SheetError> {
    let archive = Archive::open(bytes)?;
    if archive.has("xl/workbook.xml") {
        return read_xlsx(&archive);
    }
    if is_an_ods(&archive) {
        return read_ods(&archive);
    }
    Err(SheetError::NotASpreadsheet(
        "xl/workbook.xml or a spreadsheet content.xml",
    ))
}

// ---------------------------------------------------------------------------
// Excel
// ---------------------------------------------------------------------------

fn read_xlsx(archive: &Archive) -> Result<Book, SheetError> {
    let workbook = decode(&archive.read("xl/workbook.xml")?);
    let strings = shared_strings(archive);
    let styles = styles_of(archive);
    let epoch = if workbook_is_1904(&workbook) {
        // The Macintosh reckoning, still written by some files.
        Epoch::From1904
    } else {
        Epoch::From1900
    };
    let targets = relationships(archive, "xl/_rels/workbook.xml.rels");

    let mut sheets = Vec::new();
    let reader = Reader::new(&workbook);
    for event in reader {
        let Event::Start(tag) = event else { continue };
        if tag.name != "sheet" {
            continue;
        }
        let name = tag.get("name").unwrap_or("Sheet").to_string();
        // The sheet's XML is found through a relationship id, because the
        // order of the files inside has nothing to do with the order of the
        // tabs, and `sheet1.xml` is not necessarily the first tab.
        let part = tag
            .get("id")
            .and_then(|id| targets.iter().find(|(key, _)| key == id))
            .map(|(_, target)| resolve("xl/", target))
            .filter(|path| archive.has(path))
            .or_else(|| {
                let guess = format!("xl/worksheets/sheet{}.xml", sheets.len() + 1);
                archive.has(&guess).then_some(guess)
            });
        let rows = match part {
            Some(path) => cells_of_sheet(&decode(&archive.read(&path)?), &strings, &styles, epoch)?,
            // A sheet named in the workbook whose part is missing: an empty
            // tab is a better answer than a refusal, since the other tabs are
            // perfectly readable and one of them is probably the list.
            None => Vec::new(),
        };
        sheets.push(Sheet { name, rows });
    }
    if sheets.is_empty() {
        return Err(SheetError::NoSheets);
    }
    Ok(Book { sheets })
}

/// `r:id` to the file it names, from a `.rels` part.
fn relationships(archive: &Archive, at: &str) -> Vec<(String, String)> {
    let Ok(bytes) = archive.read(at) else {
        return Vec::new();
    };
    let text = decode(&bytes);
    let mut found = Vec::new();
    let reader = Reader::new(&text);
    for event in reader {
        if let Event::Start(tag) = event {
            if tag.name == "Relationship" {
                if let (Some(id), Some(target)) = (tag.get("Id"), tag.get("Target")) {
                    found.push((id.to_string(), target.to_string()));
                }
            }
        }
    }
    found
}

/// A relationship target against the part that named it.
fn resolve(base: &str, target: &str) -> String {
    match target.strip_prefix('/') {
        // Absolute within the package.
        Some(rooted) => rooted.to_string(),
        None => format!("{base}{target}"),
    }
}

/// Every shared string, in the order the file numbers them.
///
/// Excel keeps text in one table and puts indexes in the cells, so a column of
/// two hundred "Yes" is one "Yes" and two hundred noughts.
fn shared_strings(archive: &Archive) -> Vec<String> {
    let Ok(bytes) = archive.read("xl/sharedStrings.xml") else {
        return Vec::new();
    };
    let text = decode(&bytes);
    let mut strings = Vec::new();
    let mut reader = Reader::new(&text);
    let mut building: Option<String> = None;
    let mut in_text = false;
    // Not a `for` loop: the body reaches back into the reader to skip a
    // subtree, which a `for` would have borrowed for the whole body.
    #[allow(clippy::while_let_on_iterator)]
    while let Some(event) = reader.next() {
        match event {
            Event::Start(tag) => match tag.name.as_str() {
                "si" => building = Some(String::new()),
                // Phonetic guides sit inside the string and are not part of
                // it. Left in, a Japanese name comes out written twice.
                "rPh" if !tag.empty => reader.skip_element("rPh"),
                "t" => in_text = true,
                _ => {}
            },
            Event::Text(text) if in_text => {
                if let Some(building) = building.as_mut() {
                    building.push_str(&text);
                }
            }
            Event::End(name) => match name.as_str() {
                "t" => in_text = false,
                "si" => strings.push(building.take().unwrap_or_default()),
                _ => {}
            },
            _ => {}
        }
    }
    strings
}

/// What a cell format turns its number into, when it is not just digits.
///
/// A date in Excel is a number and a format; nothing on the cell says "date".
/// A percentage is a number and a format too, and there the number is not even
/// the one shown — 7.5% is stored as 0.075. Both have to be read out of
/// `xl/styles.xml` or the sheet reads back as arithmetic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Shows {
    /// The digits the file has, unchanged.
    Digits,
    Date,
    /// A hundred times the number, with this many decimal places.
    Percent(usize),
}

/// What each cell style shows, indexed the way a cell's `s` indexes it.
fn styles_of(archive: &Archive) -> Vec<Shows> {
    let Ok(bytes) = archive.read("xl/styles.xml") else {
        return Vec::new();
    };
    let text = decode(&bytes);

    // Formats the file defines for itself, beyond the built-in numbers.
    let mut custom: Vec<(u32, Shows)> = Vec::new();
    let mut styles = Vec::new();
    let mut in_cell_formats = false;
    let reader = Reader::new(&text);
    for event in reader {
        match event {
            Event::Start(tag) => match tag.name.as_str() {
                "numFmt" => {
                    if let (Some(id), Some(code)) = (tag.get("numFmtId"), tag.get("formatCode")) {
                        if let Ok(id) = id.parse::<u32>() {
                            custom.push((id, what_the_code_shows(code)));
                        }
                    }
                }
                // `cellStyleXfs` comes first and has the same `xf` inside it,
                // so only the ones in `cellXfs` are counted — the cells' `s`
                // indexes into that one.
                "cellXfs" => in_cell_formats = true,
                "xf" if in_cell_formats => {
                    let id = tag
                        .get("numFmtId")
                        .and_then(|id| id.parse::<u32>().ok())
                        .unwrap_or(0);
                    styles.push(
                        custom
                            .iter()
                            .find(|(known, _)| *known == id)
                            .map(|(_, shows)| *shows)
                            .unwrap_or_else(|| builtin_shows(id)),
                    );
                }
                _ => {}
            },
            Event::End(name) if name == "cellXfs" => in_cell_formats = false,
            _ => {}
        }
    }
    styles
}

/// The numbered formats every spreadsheet has without declaring them.
///
/// 14 to 22 are the dates and times, 45 to 47 the durations, and 9 and 10 the
/// percentages. Set down in the standard (ECMA-376, §18.8.30) and the same in
/// every file.
fn builtin_shows(id: u32) -> Shows {
    match id {
        14..=22 | 45..=47 => Shows::Date,
        9 => Shows::Percent(0),
        10 => Shows::Percent(2),
        _ => Shows::Digits,
    }
}

/// What a format code makes of the number it is given.
///
/// The letters that matter for a date are `y`, `d`, `h`, `s` and `m` — which
/// means month or minute depending on where it sits, and either way means a
/// date or a time. Quoted text and the `[Red]` sort of section are skipped,
/// because `0" days"` is a plain number that happens to contain a `d`.
fn what_the_code_shows(code: &str) -> Shows {
    // Only the first section is read. A code may hold up to four, separated by
    // semicolons — positive, negative, zero, text — and they agree about what
    // kind of thing this is.
    let first = first_section(code);
    let mut chars = first.chars();
    let mut percent = false;
    let mut decimals = 0usize;
    let mut counting = false;
    while let Some(ch) = chars.next() {
        match ch {
            '"' => {
                for quoted in chars.by_ref() {
                    if quoted == '"' {
                        break;
                    }
                }
            }
            // `[$-409]` is a locale, `[Red]` a colour; `[h]` is an elapsed
            // hour count, which is a time.
            '[' => {
                let mut inside = String::new();
                for bracketed in chars.by_ref() {
                    if bracketed == ']' {
                        break;
                    }
                    inside.push(bracketed);
                }
                if !inside.is_empty()
                    && inside
                        .chars()
                        .all(|c| matches!(c.to_ascii_lowercase(), 'h' | 'm' | 's' | ':'))
                {
                    return Shows::Date;
                }
            }
            // An escaped character is literal.
            '\\' => {
                chars.next();
            }
            '%' => percent = true,
            '.' => counting = true,
            '0' | '#' | '?' if counting => decimals += 1,
            // Lowercased first: a format code means the same in either case,
            // and Excel's own dialog writes `DD/MM/YYYY` in capitals.
            _ => {
                if matches!(ch.to_ascii_lowercase(), 'y' | 'd' | 'h' | 'm' | 's') {
                    return Shows::Date;
                }
                if !matches!(ch, '0' | '#' | '?' | ',') {
                    counting = false;
                }
            }
        }
    }
    if percent {
        Shows::Percent(decimals)
    } else {
        Shows::Digits
    }
}

/// The first of a format code's up-to-four sections.
///
/// Split on semicolons that are not inside quotes or brackets, since both may
/// hold one.
fn first_section(code: &str) -> &str {
    let mut quoted = false;
    let mut bracketed = false;
    for (at, ch) in code.char_indices() {
        match ch {
            '"' => quoted = !quoted,
            '[' if !quoted => bracketed = true,
            ']' if !quoted => bracketed = false,
            ';' if !quoted && !bracketed => return &code[..at],
            _ => {}
        }
    }
    code
}

fn workbook_is_1904(workbook: &str) -> bool {
    let reader = Reader::new(workbook);
    for event in reader {
        if let Event::Start(tag) = event {
            if tag.name == "workbookPr" {
                return matches!(tag.get("date1904").map(str::trim), Some("1") | Some("true"));
            }
        }
    }
    false
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Epoch {
    From1900,
    From1904,
}

/// One worksheet's cells.
fn cells_of_sheet(
    text: &str,
    strings: &[String],
    styles: &[Shows],
    epoch: Epoch,
) -> Result<Vec<Vec<String>>, SheetError> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    let mut row_number = 0usize;

    let mut kind = String::new();
    let mut column = 0usize;
    let mut shows = Shows::Digits;
    let mut value = String::new();
    let mut inline = String::new();
    let mut in_value = false;
    let mut in_inline = false;

    let reader = Reader::new(text);
    for event in reader {
        match event {
            Event::Start(tag) => match tag.name.as_str() {
                "row" => {
                    row = Vec::new();
                    // `r` is the row's real number, so a sheet that starts at
                    // row 5 keeps its four blank rows above rather than
                    // silently sliding up.
                    row_number = tag
                        .get("r")
                        .and_then(|r| r.parse::<usize>().ok())
                        .unwrap_or(rows.len() + 1);
                    if row_number > MOST_ROWS {
                        return Err(SheetError::TooBig(row_number));
                    }
                }
                "c" => {
                    kind = tag.get("t").unwrap_or("n").to_string();
                    column = tag
                        .get("r")
                        .map(column_of)
                        .unwrap_or(row.len())
                        .min(MOST_COLUMNS);
                    shows = tag
                        .get("s")
                        .and_then(|s| s.parse::<usize>().ok())
                        .and_then(|s| styles.get(s).copied())
                        .unwrap_or(Shows::Digits);
                    value.clear();
                    inline.clear();
                }
                "v" => in_value = true,
                // An inline string keeps its text on the cell rather than in
                // the shared table. Word-processed exports do this.
                "is" => in_inline = true,
                _ => {}
            },
            Event::Text(text) => {
                if in_value {
                    value.push_str(&text);
                } else if in_inline {
                    inline.push_str(&text);
                }
            }
            Event::End(name) => match name.as_str() {
                "v" => in_value = false,
                "is" => in_inline = false,
                "c" => {
                    let shown = shown_value(&kind, &value, &inline, shows, strings, epoch);
                    if !shown.is_empty() {
                        // The gap between this cell's address and the last is
                        // the empty cells nobody wrote down.
                        while row.len() < column {
                            row.push(String::new());
                        }
                        row.push(shown);
                    }
                }
                "row" => {
                    if !row.iter().all(String::is_empty) {
                        while rows.len() + 1 < row_number {
                            rows.push(Vec::new());
                        }
                        rows.push(std::mem::take(&mut row));
                    }
                }
                _ => {}
            },
        }
    }
    Ok(rows)
}

/// What a cell shows, from what it stores.
fn shown_value(
    kind: &str,
    value: &str,
    inline: &str,
    shows: Shows,
    strings: &[String],
    epoch: Epoch,
) -> String {
    match kind {
        "s" => value
            .trim()
            .parse::<usize>()
            .ok()
            .and_then(|at| strings.get(at))
            .cloned()
            .unwrap_or_default(),
        "inlineStr" => inline.to_string(),
        // A formula whose answer is text, and an error, both keep their text.
        "str" | "e" => value.to_string(),
        "b" => match value.trim() {
            "1" | "true" | "TRUE" => "TRUE".to_string(),
            "" => String::new(),
            _ => "FALSE".to_string(),
        },
        // A number — which is also how a date is kept.
        _ => {
            let digits = value.trim();
            match shows {
                Shows::Date => {
                    date_from_serial(digits, epoch).unwrap_or_else(|| digits.to_string())
                }
                // 7.5% is stored as 0.075, so this is the one format where
                // leaving the digits alone would be a different number.
                Shows::Percent(places) => match digits.parse::<f64>() {
                    Ok(share) => format!("{:.*}%", places, share * 100.0),
                    Err(_) => digits.to_string(),
                },
                // Left exactly as the file has it. Reading "1200.50" as a
                // number and writing it back risks digits the file never had.
                Shows::Digits => digits.to_string(),
            }
        }
    }
}

/// A date from the number a spreadsheet keeps it as.
fn date_from_serial(value: &str, epoch: Epoch) -> Option<String> {
    let serial: f64 = value.parse().ok()?;
    if !serial.is_finite() || serial < 0.0 {
        return None;
    }
    let whole = serial.trunc() as i64;
    let fraction = serial - serial.trunc();

    // Day zero. Excel believes 1900 was a leap year, to stay compatible with a
    // spreadsheet from 1983 that believed it, so its day 60 is the 29th of
    // February 1900 — a date that never happened. Everything after that
    // phantom is a day out from the real calendar, which is why the base moves
    // by one at day 60: from the 31st of December 1899 below it, and from the
    // 30th above. Day 60 itself has no real date and comes out as the 28th.
    let days = match epoch {
        Epoch::From1900 if whole < 60 => days_from_civil(1899, 12, 31) + whole,
        Epoch::From1900 => days_from_civil(1899, 12, 30) + whole,
        Epoch::From1904 => days_from_civil(1904, 1, 1) + whole,
    };
    let (year, month, day) = civil_from_days(days);

    // A serial below one is a time of day with no date attached.
    let seconds = (fraction * 86_400.0).round() as i64;
    let clock = format!(
        "{:02}:{:02}:{:02}",
        seconds / 3600,
        (seconds / 60) % 60,
        seconds % 60
    );
    // A serial below one is a time of day with no date attached — in the 1900
    // reckoning, where day zero is not a date at all. In the 1904 one it is
    // the first of January 1904, so there the date is the answer.
    if whole == 0 && epoch == Epoch::From1900 {
        return Some(clock);
    }
    if seconds == 0 {
        return Some(format!("{year:04}-{month:02}-{day:02}"));
    }
    Some(format!("{year:04}-{month:02}-{day:02} {clock}"))
}

/// Days from 1970-01-01 to a date. Howard Hinnant's `days_from_civil`, which
/// is exact for every year a spreadsheet can hold.
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month = month as i64;
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day as i64 - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// And back again.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let days = days + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 } / 146_097;
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_prime + 2) / 5 + 1) as u32;
    let month = (month_prime + if month_prime < 10 { 3 } else { -9 }) as u32;
    (year + i64::from(month <= 2), month, day)
}

/// The column a cell address names: `A1` is 0, `AA7` is 26.
fn column_of(reference: &str) -> usize {
    let mut at = 0usize;
    for ch in reference.chars() {
        if !ch.is_ascii_alphabetic() {
            break;
        }
        at = at * 26 + (ch.to_ascii_uppercase() as usize - 'A' as usize) + 1;
        if at > MOST_COLUMNS {
            return MOST_COLUMNS;
        }
    }
    at.saturating_sub(1)
}

// ---------------------------------------------------------------------------
// OpenDocument
// ---------------------------------------------------------------------------

fn read_ods(archive: &Archive) -> Result<Book, SheetError> {
    let text = decode(&archive.read("content.xml")?);
    let mut sheets = Vec::new();
    let reader = Reader::new(&text);

    let mut name = String::new();
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut row: Vec<String> = Vec::new();
    // Repeats are counted rather than expanded: a row saying "and 16,384 more
    // empty cells" is ordinary, and expanding it is not.
    let mut blank_cells = 0usize;
    let mut blank_rows = 0usize;
    let mut repeat_cell = 1usize;
    let mut repeat_row = 1usize;
    let mut cell_text: Vec<String> = Vec::new();
    let mut paragraph = String::new();
    let mut depth_in_cell = 0usize;
    let mut in_paragraph = false;

    for event in reader {
        match event {
            Event::Start(tag) => match tag.name.as_str() {
                "table" => {
                    name = tag.get("name").unwrap_or("Sheet").to_string();
                    rows = Vec::new();
                    blank_rows = 0;
                }
                "table-row" => {
                    row = Vec::new();
                    blank_cells = 0;
                    repeat_row = tag
                        .get("number-rows-repeated")
                        .and_then(|n| n.parse::<usize>().ok())
                        .unwrap_or(1)
                        .clamp(1, MOST_ROWS);
                }
                "table-cell" | "covered-table-cell" => {
                    depth_in_cell += 1;
                    if depth_in_cell == 1 {
                        cell_text = Vec::new();
                        repeat_cell = tag
                            .get("number-columns-repeated")
                            .and_then(|n| n.parse::<usize>().ok())
                            .unwrap_or(1)
                            .clamp(1, MOST_COLUMNS);
                    }
                }
                "p" if depth_in_cell > 0 => {
                    in_paragraph = true;
                    paragraph = String::new();
                }
                // A line break inside a cell — an address on three lines is
                // the ordinary reason for one.
                "line-break" if in_paragraph => paragraph.push('\n'),
                "s" if in_paragraph => {
                    let count = tag
                        .get("c")
                        .and_then(|n| n.parse::<usize>().ok())
                        .unwrap_or(1)
                        .clamp(1, 4096);
                    paragraph.push_str(&" ".repeat(count));
                }
                "tab" if in_paragraph => paragraph.push('\t'),
                _ => {}
            },
            Event::Text(text) => {
                if in_paragraph {
                    paragraph.push_str(&text);
                }
            }
            Event::End(name_ended) => match name_ended.as_str() {
                "p" if in_paragraph => {
                    in_paragraph = false;
                    cell_text.push(std::mem::take(&mut paragraph));
                }
                "table-cell" | "covered-table-cell" => {
                    depth_in_cell = depth_in_cell.saturating_sub(1);
                    if depth_in_cell > 0 {
                        continue;
                    }
                    let shown = cell_text.join("\n");
                    if shown.is_empty() {
                        blank_cells = blank_cells.saturating_add(repeat_cell);
                    } else {
                        for _ in 0..blank_cells.min(MOST_COLUMNS) {
                            row.push(String::new());
                        }
                        blank_cells = 0;
                        for _ in 0..repeat_cell {
                            row.push(shown.clone());
                        }
                    }
                }
                "table-row" => {
                    if row.is_empty() {
                        blank_rows = blank_rows.saturating_add(repeat_row);
                    } else {
                        for _ in 0..blank_rows.min(MOST_ROWS) {
                            rows.push(Vec::new());
                        }
                        blank_rows = 0;
                        for _ in 0..repeat_row {
                            rows.push(row.clone());
                        }
                    }
                    row = Vec::new();
                }
                "table" => sheets.push(Sheet {
                    name: std::mem::take(&mut name),
                    rows: std::mem::take(&mut rows),
                }),
                _ => {}
            },
        }
    }

    if sheets.is_empty() {
        return Err(SheetError::NoSheets);
    }
    Ok(Book { sheets })
}

#[cfg(test)]
#[path = "sheets/tests.rs"]
mod tests;
