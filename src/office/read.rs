//! Opening a Word or OpenDocument file without a word processor.
//!
//! Onionskin used to need LibreOffice to open a `.docx`. That is a fair thing
//! to ask of somebody who already has it and an unreasonable thing to ask of
//! everybody else: a three-hundred-megabyte download, on every machine, so that
//! a program can read a file that is a zip of XML.
//!
//! So it reads them itself. A `.docx` and a `.odt` are both a zip holding an
//! XML description of the text; [`super::unzip`] opens the zip, [`super::xml`]
//! walks the XML, the modules here turn that into paragraphs, and
//! [`layout`] sets those paragraphs on paper and writes a PDF with the same
//! writer that produces every other PDF Onionskin makes.
//!
//! # What this is not
//!
//! It is not LibreOffice. It reads text, headings, lists, tables, alignment,
//! indents, spacing, bold, italic, underline, colour, type size and the paper
//! size. It does not read images, footnotes, columns, headers and footers, or
//! anything that needs a full layout engine, and it says so — every
//! approximation comes back as a note rather than being quietly made.
//!
//! Where LibreOffice is installed, Onionskin still uses it, because it lays a
//! document out the way the word processor that wrote it would. This is the
//! answer for a machine that does not have it.
//!
//! # Why that is safe for a delta
//!
//! A delta compares two renderings of the same document, so what matters is
//! that both are laid out by the *same* engine — and they always are, because
//! both go through this same code in the same run. What changes is the
//! comparison against the sheet already in the tray: if that sheet came out of
//! Word, Word's line breaks are on it, and Onionskin's may differ. That is a
//! note the caller passes on, not a detail to bury.

use std::path::Path;

use super::xml;
use crate::geometry::PageSize;

pub mod docx;
pub mod layout;
pub mod odt;
pub mod plain;

#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    #[error("could not read {path}: {source}")]
    Io {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    #[error("{0}")]
    Zip(#[from] super::unzip::ZipError),
    #[error("{0}")]
    Shape(String),
    #[error("{0}")]
    Pdf(#[from] crate::pdf::PdfError),
    /// The file is one of ours by name and something else inside.
    #[error("{0}")]
    NotThatKind(String),
}

/// What Onionskin can open on its own, without a word processor.
///
/// Deliberately short. Everything here is either a zip of XML or plain text —
/// formats where reading them is a morning's work and getting them wrong is
/// obvious. The binary formats (`.doc`, `.rtf` and the rest) are not, and are
/// left to LibreOffice, which has spent twenty years on them.
pub const READABLE: &[&str] = &[
    // Word, in the format it has written since 2007.
    "docx", "docm", "dotx", "dotm", // OpenDocument, zipped and flat.
    "odt", "ott", "fodt", // Text that needs no reading at all.
    "txt", "text", "md", "markdown",
];

/// Whether the built-in reader can open this kind of file.
pub fn can_read(suffix: &str) -> bool {
    let suffix = suffix.trim_start_matches('.').to_ascii_lowercase();
    READABLE.contains(&suffix.as_str())
}

// ---------------------------------------------------------------------------
// What a document is, once it has been read
// ---------------------------------------------------------------------------

/// A document, as far as Onionskin needs to understand one: some paper, and
/// what goes on it in order.
#[derive(Debug, Clone)]
pub struct Sheet {
    pub page: PageSize,
    pub margins: Margins,
    pub blocks: Vec<Block>,
    /// What was approximated or left out, in words a person can act on.
    pub notes: Vec<String>,
}

impl Sheet {
    pub fn new(page: PageSize, margins: Margins) -> Sheet {
        Sheet {
            page,
            margins,
            blocks: Vec::new(),
            notes: Vec::new(),
        }
    }

    /// Add a note, unless the same one is already there. A document with two
    /// hundred images should say "it has images" once.
    pub fn note(&mut self, text: impl Into<String>) {
        let text = text.into();
        if !self.notes.contains(&text) {
            self.notes.push(text);
        }
    }

    /// Every piece of text in the document, for tests and for deciding which
    /// fonts are needed.
    pub fn text(&self) -> String {
        let mut out = String::new();
        collect_text(&self.blocks, &mut out);
        out
    }
}

fn collect_text(blocks: &[Block], out: &mut String) {
    for block in blocks {
        match block {
            Block::Para(para) => {
                if let Some(marker) = &para.marker {
                    out.push_str(marker);
                    out.push(' ');
                }
                for piece in &para.pieces {
                    match piece {
                        Piece::Text(text, _) => out.push_str(text),
                        Piece::Tab => out.push('\t'),
                        Piece::LineBreak | Piece::PageBreak => out.push('\n'),
                    }
                }
                out.push('\n');
            }
            Block::Table(table) => {
                for row in &table.rows {
                    for cell in &row.cells {
                        collect_text(&cell.blocks, out);
                    }
                }
            }
        }
    }
}

/// The white border round the page, in millimetres.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Margins {
    pub top_mm: f64,
    pub right_mm: f64,
    pub bottom_mm: f64,
    pub left_mm: f64,
}

impl Default for Margins {
    /// Two centimetres, which is what a word processor gives a new document
    /// when nobody says otherwise.
    fn default() -> Margins {
        Margins {
            top_mm: 20.0,
            right_mm: 20.0,
            bottom_mm: 20.0,
            left_mm: 20.0,
        }
    }
}

#[derive(Debug, Clone)]
pub enum Block {
    Para(Para),
    Table(Table),
}

/// A paragraph: some text, and everything about how it sits on the page.
#[derive(Debug, Clone)]
pub struct Para {
    pub pieces: Vec<Piece>,
    pub align: Align,
    /// The style of the paragraph mark, which is what decides the height of an
    /// empty paragraph — and an empty paragraph is how every document in the
    /// world puts a gap between two others.
    pub style: Style,
    pub space_before_mm: f64,
    pub space_after_mm: f64,
    pub indent_left_mm: f64,
    pub indent_right_mm: f64,
    /// Extra indent on the first line. Negative for a hanging indent, which is
    /// how a list keeps its wrapped text clear of the bullet.
    pub first_line_mm: f64,
    /// Line height as a multiple of the type size.
    pub line_spacing: f64,
    pub break_before: bool,
    /// A list bullet or number, set in the hanging indent.
    pub marker: Option<String>,
}

impl Default for Para {
    fn default() -> Para {
        Para {
            pieces: Vec::new(),
            align: Align::Left,
            style: Style::default(),
            space_before_mm: 0.0,
            space_after_mm: 0.0,
            indent_left_mm: 0.0,
            indent_right_mm: 0.0,
            first_line_mm: 0.0,
            line_spacing: 1.0,
            break_before: false,
            marker: None,
        }
    }
}

impl Para {
    /// The words in it, with nothing about how they look.
    pub fn plain_text(&self) -> String {
        let mut out = String::new();
        for piece in &self.pieces {
            match piece {
                Piece::Text(text, _) => out.push_str(text),
                Piece::Tab => out.push('\t'),
                Piece::LineBreak | Piece::PageBreak => out.push('\n'),
            }
        }
        out
    }

    pub fn is_empty(&self) -> bool {
        self.pieces
            .iter()
            .all(|piece| matches!(piece, Piece::Text(text, _) if text.trim().is_empty()))
    }
}

/// One thing inside a paragraph.
#[derive(Debug, Clone, PartialEq)]
pub enum Piece {
    Text(String, Style),
    LineBreak,
    PageBreak,
    Tab,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Align {
    #[default]
    Left,
    Centre,
    Right,
    /// Spread out to both margins, which is what half of all printed documents
    /// are set as and looks obviously wrong if it is quietly turned into Left.
    Justify,
}

/// How a run of text looks.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Style {
    pub size_pt: f64,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub family: Family,
    pub colour: (f64, f64, f64),
}

impl Default for Style {
    /// Eleven point, which is what both Word and Writer start a document at.
    fn default() -> Style {
        Style {
            size_pt: 11.0,
            bold: false,
            italic: false,
            underline: false,
            strike: false,
            family: Family::Sans,
            colour: (0.0, 0.0, 0.0),
        }
    }
}

/// Which of the three shapes of type a font is.
///
/// A PDF reader has fourteen fonts built into it and a document may name any of
/// the thousands that exist. Sorting them into three families is what can
/// honestly be done without shipping a font for every name — and it is enough
/// that a document set in Times does not come back in Helvetica.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Family {
    #[default]
    Sans,
    Serif,
    Mono,
}

impl Family {
    /// Which family a font name belongs to.
    ///
    /// Names, not measurements: there is no way to tell from "Cambria" that it
    /// has serifs except by knowing that it does. So the common faces are
    /// listed, and anything unknown falls to the shape its name suggests, and
    /// then to sans — which is what a word processor's own substitution does.
    pub fn of(name: &str) -> Family {
        let name = name.trim().to_ascii_lowercase();
        const SERIF: &[&str] = &[
            "times",
            "times new roman",
            "georgia",
            "garamond",
            "book antiqua",
            "bookman",
            "cambria",
            "constantia",
            "palatino",
            "century",
            "century schoolbook",
            "minion",
            "liberation serif",
            "dejavu serif",
            "nimbus roman",
            "thorndale",
            "pt serif",
            "noto serif",
            "source serif",
            "charter",
            "utopia",
            "computer modern",
            "cmr",
        ];
        const MONO: &[&str] = &[
            "courier",
            "courier new",
            "consolas",
            "monaco",
            "menlo",
            "inconsolata",
            "liberation mono",
            "dejavu sans mono",
            "nimbus mono",
            "cumberland",
            "andale mono",
            "lucida console",
            "source code",
            "fira code",
            "jetbrains mono",
            "noto sans mono",
            "sf mono",
            "ubuntu mono",
        ];
        // "Monotype Corsiva" is a script face and not a typewriter one, and it
        // is the reason this is not a plain search for "mono".
        let monospaced = MONO.iter().any(|known| name.starts_with(known))
            || (name.contains("mono") && !name.starts_with("monotype"));
        if monospaced {
            return Family::Mono;
        }
        if SERIF.iter().any(|known| name.starts_with(known)) || name.contains("serif") {
            // "sans serif" contains "serif" and is not one.
            if !name.contains("sans") {
                return Family::Serif;
            }
        }
        if name.contains("roman") {
            return Family::Serif;
        }
        Family::Sans
    }
}

// ---------------------------------------------------------------------------
// Counting a list
// ---------------------------------------------------------------------------
//
// Both formats number a list in the same five ways, and two implementations of
// "what comes after (z)" would be one too many — the one that drifted would be
// found by somebody whose appendix came out wrong.

/// 1 → a, 26 → z, 27 → aa, which is how a lettered list counts.
pub(crate) fn letters(count: usize, upper: bool) -> String {
    let mut out = String::new();
    let mut left = count.max(1);
    while left > 0 {
        let index = (left - 1) % 26;
        out.insert(0, (b'a' + index as u8) as char);
        left = (left - 1) / 26;
    }
    if upper {
        out.to_uppercase()
    } else {
        out
    }
}

/// Roman numerals, up to the point where a list has gone wrong anyway.
pub(crate) fn roman(count: usize) -> String {
    const PARTS: [(usize, &str); 13] = [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ];
    let mut left = count.clamp(1, 3999);
    let mut out = String::new();
    for (value, numeral) in PARTS {
        while left >= value {
            out.push_str(numeral);
            left -= value;
        }
    }
    out
}

/// A table: a grid of cells, each holding blocks of its own.
#[derive(Debug, Clone, Default)]
pub struct Table {
    /// Column widths in millimetres. Empty means share the space equally.
    pub columns_mm: Vec<f64>,
    pub rows: Vec<Row>,
    /// Whether to rule the cells. A table used for layout has no lines and one
    /// used for a table of figures has them, and printing the wrong answer is
    /// obvious on the page either way.
    pub bordered: bool,
    /// White space inside each cell — across, and down — where the document
    /// says. `None` for the ordinary amount.
    ///
    /// Worth reading rather than assuming: a narrow table set with hairline
    /// margins fits its words on one line, and the same table with a
    /// comfortable margin assumed for it wraps every cell.
    pub padding_mm: Option<(f64, f64)>,
}

#[derive(Debug, Clone, Default)]
pub struct Row {
    pub cells: Vec<Cell>,
}

#[derive(Debug, Clone, Default)]
pub struct Cell {
    pub blocks: Vec<Block>,
    /// How many columns it covers.
    pub span: usize,
}

// ---------------------------------------------------------------------------
// The public door
// ---------------------------------------------------------------------------

/// Read a document into paragraphs.
pub fn read(path: &Path) -> Result<Sheet, ReadError> {
    let suffix = path
        .extension()
        .map(|s| s.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();

    let bytes = std::fs::read(path).map_err(|source| ReadError::Io {
        path: path.to_path_buf(),
        source,
    })?;

    match suffix.as_str() {
        "docx" | "docm" | "dotx" | "dotm" => docx::read(&bytes),
        "odt" | "ott" => odt::read(&bytes),
        "fodt" => odt::read_flat(&xml::decode(&bytes)),
        "txt" | "text" | "md" | "markdown" => Ok(plain::read(&xml::decode(&bytes), &suffix)),
        other => Err(ReadError::NotThatKind(format!(
            "Onionskin cannot open a '.{other}' by itself"
        ))),
    }
}

/// Read a document and write it out as a PDF, returning what was approximated.
pub fn to_pdf(source: &Path, into: &Path) -> Result<Vec<String>, ReadError> {
    let sheet = read(source)?;
    let mut notes = sheet.notes.clone();
    notes.extend(layout::write(&sheet, into, source)?);
    Ok(notes)
}

#[cfg(test)]
mod tests;
