//! Reading a Word document.
//!
//! A `.docx` is a zip. Inside it, `word/document.xml` is the text, `word/
//! styles.xml` says what the named styles look like, and `word/numbering.xml`
//! says what a list's bullets and numbers are. Nothing else is needed to put
//! the words on a page in the right order, at the right size, in the right
//! place.
//!
//! # The two traps
//!
//! Word writes some things twice. A modern shape is written as DrawingML
//! inside `mc:Choice` *and* as the old VML inside `mc:Fallback`, so that older
//! readers still see something — and a reader that takes both gets every text
//! box twice. So the fallback is skipped whole.
//!
//! Word also writes some things that are not text. `w:instrText` is the
//! machinery of a field — `PAGE \* MERGEFORMAT` and the like — and `w:delText`
//! is text somebody deleted with track changes on. Both would be printed as if
//! they were words. Both are left out.

// Every walk below reaches back into the scanner in the middle of itself — to
// skip an element whole, or to read a nested one with a function of its own. A
// `for` loop borrows the scanner for its whole body and makes that impossible,
// so `while let` is the shape these have to be.
#![allow(clippy::while_let_on_iterator)]

use std::collections::BTreeMap;

use super::super::unzip::Archive;
use super::super::xml::{decode, Event, Reader};
use super::{
    letters, roman, Align, Block, Cell, Family, Margins, Para, Piece, ReadError, Row, Sheet, Style,
    Table,
};
use crate::geometry::PageSize;

/// Twentieths of a point, which is what Word measures nearly everything in.
fn twips_mm(twips: f64) -> f64 {
    twips / 1440.0 * 25.4
}

/// Half-points, which is how Word writes a type size.
fn half_points(value: f64) -> f64 {
    (value / 2.0).clamp(1.0, 400.0)
}

/// `RRGGBB`, which is how Word writes a colour. `auto` means "whatever suits",
/// and what suits on paper is black.
fn colour(text: &str) -> Option<(f64, f64, f64)> {
    let text = text.trim().trim_start_matches('#');
    if text.eq_ignore_ascii_case("auto") {
        return Some((0.0, 0.0, 0.0));
    }
    if text.len() != 6 {
        return None;
    }
    let value = u32::from_str_radix(text, 16).ok()?;
    Some((
        ((value >> 16) & 0xFF) as f64 / 255.0,
        ((value >> 8) & 0xFF) as f64 / 255.0,
        (value & 0xFF) as f64 / 255.0,
    ))
}

// ---------------------------------------------------------------------------
// What a style says, before it is known what it is layered over
// ---------------------------------------------------------------------------

/// How text looks, with nothing said about what is not mentioned.
#[derive(Debug, Clone, Default, PartialEq)]
struct Look {
    size_pt: Option<f64>,
    bold: Option<bool>,
    italic: Option<bool>,
    underline: Option<bool>,
    strike: Option<bool>,
    family: Option<Family>,
    colour: Option<(f64, f64, f64)>,
}

impl Look {
    /// This look laid over another, keeping whatever it does not mention.
    fn over(&self, base: Style) -> Style {
        Style {
            size_pt: self.size_pt.unwrap_or(base.size_pt),
            bold: self.bold.unwrap_or(base.bold),
            italic: self.italic.unwrap_or(base.italic),
            underline: self.underline.unwrap_or(base.underline),
            strike: self.strike.unwrap_or(base.strike),
            family: self.family.unwrap_or(base.family),
            colour: self.colour.unwrap_or(base.colour),
        }
    }

    /// Everything this one says, with the other's answers where it is silent.
    fn on_top_of(&self, under: &Look) -> Look {
        Look {
            size_pt: self.size_pt.or(under.size_pt),
            bold: self.bold.or(under.bold),
            italic: self.italic.or(under.italic),
            underline: self.underline.or(under.underline),
            strike: self.strike.or(under.strike),
            family: self.family.or(under.family),
            colour: self.colour.or(under.colour),
        }
    }
}

/// How a paragraph sits, with nothing said about what is not mentioned.
#[derive(Debug, Clone, Default, PartialEq)]
struct Shape {
    align: Option<Align>,
    space_before_mm: Option<f64>,
    space_after_mm: Option<f64>,
    indent_left_mm: Option<f64>,
    indent_right_mm: Option<f64>,
    first_line_mm: Option<f64>,
    line_spacing: Option<f64>,
    break_before: Option<bool>,
    /// Which heading level this is, if it is one.
    outline: Option<usize>,
    /// The list it belongs to, and how deep in it.
    numbering: Option<(String, usize)>,
}

impl Shape {
    fn on_top_of(&self, under: &Shape) -> Shape {
        Shape {
            align: self.align.or(under.align),
            space_before_mm: self.space_before_mm.or(under.space_before_mm),
            space_after_mm: self.space_after_mm.or(under.space_after_mm),
            indent_left_mm: self.indent_left_mm.or(under.indent_left_mm),
            indent_right_mm: self.indent_right_mm.or(under.indent_right_mm),
            first_line_mm: self.first_line_mm.or(under.first_line_mm),
            line_spacing: self.line_spacing.or(under.line_spacing),
            break_before: self.break_before.or(under.break_before),
            outline: self.outline.or(under.outline),
            numbering: self.numbering.clone().or(under.numbering.clone()),
        }
    }

    /// Turn what the file said into a paragraph ready to be set.
    fn apply(&self, para: &mut Para) {
        para.align = self.align.unwrap_or(Align::Left);
        para.space_before_mm = self.space_before_mm.unwrap_or(0.0);
        para.space_after_mm = self.space_after_mm.unwrap_or(0.0);
        para.indent_left_mm = self.indent_left_mm.unwrap_or(0.0);
        para.indent_right_mm = self.indent_right_mm.unwrap_or(0.0);
        para.first_line_mm = self.first_line_mm.unwrap_or(0.0);
        para.line_spacing = self.line_spacing.unwrap_or(1.0);
        para.break_before = self.break_before.unwrap_or(false);
    }
}

/// A named style out of `styles.xml`.
#[derive(Debug, Clone, Default)]
struct Named {
    based_on: Option<String>,
    look: Look,
    shape: Shape,
}

/// Everything the document says about how things should look.
#[derive(Debug, Default)]
struct Styles {
    default_look: Look,
    default_shape: Shape,
    named: BTreeMap<String, Named>,
}

impl Styles {
    /// A style resolved through its chain of parents.
    ///
    /// Bounded, because a file can name a style whose parent is itself — and a
    /// document that will not open is worse than one whose heading is the
    /// wrong size.
    fn resolve(&self, id: &str) -> (Look, Shape) {
        let mut chain = Vec::new();
        let mut at = Some(id.to_string());
        for _ in 0..16 {
            let Some(name) = at.take() else { break };
            let Some(style) = self.named.get(&name) else {
                break;
            };
            if chain
                .iter()
                .any(|(seen, _): &(String, &Named)| seen == &name)
            {
                break;
            }
            chain.push((name, style));
            at = style.based_on.clone();
        }

        // Nearest first, so a child's answer wins over its parent's.
        let mut look = Look::default();
        let mut shape = Shape::default();
        for (_, style) in &chain {
            look = look.on_top_of(&style.look);
            shape = shape.on_top_of(&style.shape);
        }
        (look, shape)
    }
}

// ---------------------------------------------------------------------------
// Reading
// ---------------------------------------------------------------------------

/// Open a Word document.
pub fn read(bytes: &[u8]) -> Result<Sheet, ReadError> {
    let archive = Archive::open(bytes)?;
    let (_, document) = archive
        .read_any(&["word/document.xml", "word/document2.xml"])
        .ok_or_else(|| {
            ReadError::NotThatKind(
                "this file is a zip but not a Word document — there is no \
                 word/document.xml inside it"
                    .into(),
            )
        })?;

    let styles = match archive.read_any(&["word/styles.xml"]) {
        Some((_, bytes)) => read_styles(&decode(&bytes)),
        None => Styles::default(),
    };
    let numbering = match archive.read_any(&["word/numbering.xml"]) {
        Some((_, bytes)) => Numbering::read(&decode(&bytes)),
        None => Numbering::default(),
    };

    let mut walk = Walk {
        styles: &styles,
        counters: numbering.counting(),
        setup: Setup::default(),
        notes: Vec::new(),
    };
    let blocks = walk.body(&mut Reader::new(&decode(&document)));

    let mut sheet = Sheet::new(walk.setup.page, walk.setup.margins);
    sheet.blocks = blocks;
    for note in std::mem::take(&mut walk.notes) {
        sheet.note(note);
    }

    // A part that is present and never read is worth a sentence: the words in
    // it are on the paper the document would print, and not on ours.
    if archive.names().any(|name| name.starts_with("word/header")) {
        sheet.note("Headers and footers were left out; Onionskin sets the body only.");
    }
    // Word writes a footnotes part into nearly every document whether or not
    // anything is in it, so the part existing is not the question — whether it
    // holds a footnote anybody wrote is.
    if let Some((_, bytes)) = archive.read_any(&["word/footnotes.xml"]) {
        if decode(&bytes).matches("<w:footnote ").count() > 2 {
            sheet.note("Footnotes were left out.");
        }
    }
    if archive.names().any(|name| name.starts_with("word/media/")) {
        sheet.note("Pictures were left out; only the text is set.");
    }
    Ok(sheet)
}

/// Everything in `styles.xml`.
fn read_styles(text: &str) -> Styles {
    let mut styles = Styles::default();
    let mut reader = Reader::new(text);

    while let Some(event) = reader.next() {
        let Event::Start(tag) = event else { continue };
        match tag.name.as_str() {
            "rPrDefault" => {
                // The default look is inside an `rPr` within this.
                while let Some(event) = reader.next() {
                    match event {
                        Event::Start(inner) if inner.name == "rPr" => {
                            styles.default_look = read_look(&mut reader);
                        }
                        Event::End(name) if name == "rPrDefault" => break,
                        _ => {}
                    }
                }
            }
            "pPrDefault" => {
                while let Some(event) = reader.next() {
                    match event {
                        Event::Start(inner) if inner.name == "pPr" => {
                            styles.default_shape = read_properties(&mut reader, None).1;
                        }
                        Event::End(name) if name == "pPrDefault" => break,
                        _ => {}
                    }
                }
            }
            "style" => {
                let Some(id) = tag.get("styleId").map(str::to_string) else {
                    continue;
                };
                let mut named = Named::default();
                while let Some(event) = reader.next() {
                    match event {
                        Event::Start(inner) => match inner.name.as_str() {
                            "basedOn" => {
                                named.based_on = inner.get("val").map(str::to_string);
                            }
                            "rPr" => named.look = read_look(&mut reader),
                            "pPr" => named.shape = read_properties(&mut reader, None).1,
                            _ => {}
                        },
                        Event::End(name) if name == "style" => break,
                        _ => {}
                    }
                }
                // Word does not always write a size on a heading style; it
                // leaves it to the theme, which is a whole typography system
                // this program is not going to grow. The familiar sizes are a
                // better guess than the body size, which would make every
                // heading vanish into the text.
                if let Some(level) = heading_level(&id) {
                    named.shape.outline = Some(level);
                    if named.look.size_pt.is_none() {
                        named.look.size_pt = Some(heading_size(level));
                    }
                    if named.look.bold.is_none() {
                        named.look.bold = Some(true);
                    }
                }
                styles.named.insert(id, named);
            }
            _ => {}
        }
    }
    styles
}

/// Which heading a style id names, if it names one.
fn heading_level(id: &str) -> Option<usize> {
    let lower = id.to_ascii_lowercase().replace([' ', '-', '_'], "");
    let digits = lower.strip_prefix("heading")?;
    digits.parse().ok().filter(|level| (1..=9).contains(level))
}

fn heading_size(level: usize) -> f64 {
    match level {
        1 => 20.0,
        2 => 16.0,
        3 => 14.0,
        4 => 12.0,
        5 => 11.0,
        _ => 10.0,
    }
}

/// A run's look, read from inside an `rPr` that has just started.
fn read_look(reader: &mut Reader) -> Look {
    let mut look = Look::default();
    while let Some(event) = reader.next() {
        match event {
            Event::Start(tag) => match tag.name.as_str() {
                "b" => look.bold = Some(tag.on()),
                "i" => look.italic = Some(tag.on()),
                "u" => {
                    look.underline = Some(!matches!(tag.get("val"), Some("none") | Some("0")));
                }
                "strike" | "dstrike" => look.strike = Some(tag.on()),
                "sz" => look.size_pt = tag.number("val").map(half_points),
                "color" => look.colour = tag.get("val").and_then(colour),
                "rFonts" => {
                    look.family = tag
                        .get("ascii")
                        .or_else(|| tag.get("hAnsi"))
                        .or_else(|| tag.get("cs"))
                        .map(Family::of);
                }
                // Small capitals, spacing, kerning and the rest change how a
                // line measures. Ignoring them is a real difference from what
                // Word would print, and a smaller one than dropping the text.
                _ => {}
            },
            Event::End(name) if name == "rPr" => break,
            _ => {}
        }
    }
    look
}

/// What a paragraph says about itself, read from inside a `pPr` that has just
/// started: the style it names, how it sits, and the look of its paragraph
/// mark.
///
/// One function rather than three passes over the same element, because a
/// scanner only goes forwards — and two of these have to be read to know the
/// third anyway.
///
/// `setup` catches a section break, which is where the paper size is written
/// and which Word puts inside the last paragraph of the section.
fn read_properties(
    reader: &mut Reader,
    mut setup: Option<&mut Setup>,
) -> (Option<String>, Shape, Look) {
    let mut id = None;
    let mut shape = Shape::default();
    let mut mark = Look::default();
    let mut list: (Option<String>, usize) = (None, 0);

    while let Some(event) = reader.next() {
        match event {
            Event::Start(tag) => match tag.name.as_str() {
                // Kept as the id: resolving it needs the whole style table,
                // which is the caller's business.
                "pStyle" => id = tag.get("val").map(str::to_string),
                "rPr" => mark = read_look(reader),
                "jc" => shape.align = tag.get("val").map(align_of),
                "outlineLvl" => shape.outline = tag.number("val").map(|n| n as usize + 1),
                "pageBreakBefore" => shape.break_before = Some(tag.on()),
                "spacing" => {
                    shape.space_before_mm = tag.number("before").map(twips_mm);
                    shape.space_after_mm = tag.number("after").map(twips_mm);
                    // `w:line` is a multiple of single spacing in two-hundred-
                    // and-fortieths when the rule is automatic, and a fixed
                    // height in twips when it is not. Only the first can be
                    // honoured without knowing the type size, and the second
                    // is rare enough to let stand at single.
                    if tag.get("lineRule").unwrap_or("auto") == "auto" {
                        shape.line_spacing = tag
                            .number("line")
                            .map(|line| (line / 240.0).clamp(0.5, 5.0));
                    }
                }
                "ind" => {
                    shape.indent_left_mm = tag
                        .number("left")
                        .or_else(|| tag.number("start"))
                        .map(twips_mm);
                    shape.indent_right_mm = tag
                        .number("right")
                        .or_else(|| tag.number("end"))
                        .map(twips_mm);
                    // A hanging indent is a first line that starts further
                    // left, which is a negative first-line indent.
                    shape.first_line_mm = match (tag.number("firstLine"), tag.number("hanging")) {
                        (_, Some(hanging)) => Some(-twips_mm(hanging)),
                        (Some(first), None) => Some(twips_mm(first)),
                        (None, None) => None,
                    };
                }
                "ilvl" => list.1 = tag.number("val").unwrap_or(0.0).max(0.0) as usize,
                "numId" => list.0 = tag.get("val").map(str::to_string),
                "sectPr" => {
                    if let Some(setup) = setup.as_deref_mut() {
                        read_setup(reader, setup);
                    }
                }
                _ => {}
            },
            Event::End(name) if name == "pPr" => break,
            _ => {}
        }
    }
    if let Some(num_id) = list.0 {
        // A list with no id is Word saying "this paragraph is not in a list
        // after all", which is how it turns numbering off for one paragraph.
        if num_id != "0" {
            shape.numbering = Some((num_id, list.1));
        }
    }
    (id, shape, mark)
}

fn align_of(value: &str) -> Align {
    match value {
        "center" | "centre" => Align::Centre,
        "right" | "end" => Align::Right,
        "both" | "justify" | "distribute" => Align::Justify,
        _ => Align::Left,
    }
}

/// The paper, from a section break.
#[derive(Debug, Clone)]
struct Setup {
    page: PageSize,
    margins: Margins,
}

impl Default for Setup {
    /// US Letter with an inch all round, which is what Word gives a new
    /// document on a machine that has never been told otherwise.
    fn default() -> Setup {
        Setup {
            page: PageSize::new(215.9, 279.4),
            margins: Margins {
                top_mm: 25.4,
                right_mm: 25.4,
                bottom_mm: 25.4,
                left_mm: 25.4,
            },
        }
    }
}

fn read_setup(reader: &mut Reader, setup: &mut Setup) {
    while let Some(event) = reader.next() {
        match event {
            Event::Start(tag) => match tag.name.as_str() {
                "pgSz" => {
                    let width = tag.number("w").map(twips_mm).unwrap_or(215.9);
                    let height = tag.number("h").map(twips_mm).unwrap_or(279.4);
                    // A landscape page is written with the two swapped
                    // already; the attribute only says which way it was set.
                    setup.page = PageSize::new(width.max(10.0), height.max(10.0));
                }
                "pgMar" => {
                    let at = |name: &str, fallback: f64| {
                        tag.number(name)
                            // A negative margin puts text off the paper, which
                            // no printer will do.
                            .map(|value| twips_mm(value).max(0.0))
                            .unwrap_or(fallback)
                    };
                    setup.margins = Margins {
                        top_mm: at("top", 25.4),
                        right_mm: at("right", 25.4),
                        bottom_mm: at("bottom", 25.4),
                        left_mm: at("left", 25.4),
                    };
                }
                _ => {}
            },
            Event::End(name) if name == "sectPr" => break,
            _ => {}
        }
    }
}

/// One pass through a document, and everything it needs to make sense of what
/// it finds.
///
/// A struct rather than six arguments passed down four levels: the styles, the
/// list counters, the paper and the notes are all read or written at every
/// depth, and threading them by hand is how one of them ends up not being
/// threaded.
struct Walk<'a> {
    styles: &'a Styles,
    counters: Counters<'a>,
    setup: Setup,
    notes: Vec<String>,
}

impl Walk<'_> {
    fn note(&mut self, text: &str) {
        if !self.notes.iter().any(|seen| seen == text) {
            self.notes.push(text.to_string());
        }
    }

    /// Walk the body and collect what is in it.
    fn body(&mut self, reader: &mut Reader) -> Vec<Block> {
        let mut blocks = Vec::new();
        while let Some(event) = reader.next() {
            let Event::Start(tag) = event else { continue };
            match tag.name.as_str() {
                "p" => blocks.append(&mut self.paragraph(reader)),
                "tbl" => {
                    if let Some(table) = self.table(reader, 0) {
                        blocks.push(Block::Table(table));
                    }
                }
                "sectPr" => read_setup(reader, &mut self.setup),
                "Fallback" => reader.skip_element("Fallback"),
                _ => {}
            }
        }
        blocks
    }

    /// One paragraph, and anything found inside it that is a block of its own —
    /// a text box holds whole paragraphs, and they belong in the document.
    fn paragraph(&mut self, reader: &mut Reader) -> Vec<Block> {
        let mut para = Para::default();
        let mut extra: Vec<Block> = Vec::new();
        let mut style_id: Option<String> = None;
        let mut own_shape = Shape::default();
        let mut mark = Look::default();
        let mut run = Look::default();
        let mut in_run = false;
        // Worked out once the properties have been read, and then kept:
        // resolving a style walks its whole chain of parents, and a paragraph
        // can hold hundreds of runs.
        let mut base: Option<Style> = None;

        while let Some(event) = reader.next() {
            match event {
                Event::Start(tag) => match tag.name.as_str() {
                    "pPr" => {
                        let (id, shape, found) = read_properties(reader, Some(&mut self.setup));
                        style_id = id;
                        own_shape = shape;
                        mark = found;
                    }
                    "rPr" if in_run => run = read_look(reader),
                    "r" => {
                        in_run = true;
                        run = Look::default();
                    }
                    "t" => {
                        let text = read_text(reader, "t");
                        if !text.is_empty() {
                            let base = *base.get_or_insert_with(|| {
                                resolve_style(self.styles, style_id.as_deref(), &mark)
                            });
                            para.pieces.push(Piece::Text(text, run.over(base)));
                        }
                    }
                    "br" => para.pieces.push(match tag.get("type") {
                        Some("page") => Piece::PageBreak,
                        _ => Piece::LineBreak,
                    }),
                    "cr" => para.pieces.push(Piece::LineBreak),
                    "tab" => para.pieces.push(Piece::Tab),
                    "noBreakHyphen" => {
                        let base = *base.get_or_insert_with(|| {
                            resolve_style(self.styles, style_id.as_deref(), &mark)
                        });
                        para.pieces.push(Piece::Text("-".into(), run.over(base)));
                    }
                    // Field machinery and deleted text are not words on the
                    // page even though they are text in the file.
                    "instrText" | "delText" | "delInstrText" => reader.skip_element(&tag.name),
                    "Fallback" => reader.skip_element("Fallback"),
                    "txbxContent" => extra.append(&mut self.text_box(reader)),
                    "drawing" | "object" => {
                        self.note("Pictures were left out; only the text is set.")
                    }
                    "tbl" => {
                        if let Some(table) = self.table(reader, 0) {
                            extra.push(Block::Table(table));
                        }
                    }
                    _ => {}
                },
                Event::End(name) => match name.as_str() {
                    "r" => in_run = false,
                    "p" => break,
                    _ => {}
                },
                _ => {}
            }
        }

        // The paragraph's own settings sit on top of its named style's.
        let (style_look, style_shape) = match &style_id {
            Some(id) => self.styles.resolve(id),
            None => (Look::default(), Shape::default()),
        };
        let shape = own_shape
            .on_top_of(&style_shape)
            .on_top_of(&self.styles.default_shape);
        shape.apply(&mut para);
        para.style = mark
            .on_top_of(&style_look)
            .on_top_of(&self.styles.default_look)
            .over(Style::default());

        if let Some((num_id, level)) = &shape.numbering {
            let (marker, indent) = self.counters.marker(num_id, *level);
            para.marker = marker;
            if para.indent_left_mm <= 0.0 {
                para.indent_left_mm = indent;
            }
            // Without a hanging indent the bullet and the first word are set in
            // the same place, one over the other.
            if para.first_line_mm >= 0.0 {
                para.first_line_mm = -6.0;
            }
        }

        let mut blocks = vec![Block::Para(para)];
        blocks.append(&mut extra);
        blocks
    }

    /// The paragraphs inside a text box.
    ///
    /// Word anchors a text box at a place on the page. Onionskin sets it in the
    /// flow instead, where it will be read in the right order and printed in
    /// the wrong place — the better half of a bad choice, since dropping it
    /// loses the words altogether.
    fn text_box(&mut self, reader: &mut Reader) -> Vec<Block> {
        let mut blocks = Vec::new();
        while let Some(event) = reader.next() {
            match event {
                Event::Start(tag) if tag.name == "p" => blocks.append(&mut self.paragraph(reader)),
                Event::Start(tag) if tag.name == "Fallback" => reader.skip_element("Fallback"),
                Event::End(name) if name == "txbxContent" => break,
                _ => {}
            }
        }
        blocks
    }

    /// A table, from a `w:tbl` that has just started.
    fn table(&mut self, reader: &mut Reader, depth: usize) -> Option<Table> {
        let mut table = Table::default();
        let mut row: Option<Row> = None;
        let mut cell: Option<Cell> = None;

        while let Some(event) = reader.next() {
            match event {
                Event::Start(tag) => match tag.name.as_str() {
                    "gridCol" => {
                        if let Some(width) = tag.number("w") {
                            table.columns_mm.push(twips_mm(width));
                        }
                    }
                    "tblBorders" => table.bordered = any_border(reader),
                    "tblCellMar" => table.padding_mm = Some(read_cell_margins(reader)),
                    "tr" => row = Some(Row::default()),
                    "tc" => {
                        cell = Some(Cell {
                            span: 1,
                            ..Cell::default()
                        })
                    }
                    "gridSpan" => {
                        if let (Some(cell), Some(span)) = (cell.as_mut(), tag.number("val")) {
                            cell.span = span.max(1.0) as usize;
                        }
                    }
                    "p" => {
                        let found = self.paragraph(reader);
                        // A paragraph outside any cell is not legal in a table
                        // and does turn up in files written by other programs;
                        // there is nowhere to put it, so it goes.
                        if let Some(cell) = cell.as_mut() {
                            cell.blocks.extend(found);
                        }
                    }
                    "tbl" if depth < 3 => {
                        if let Some(inner) = self.table(reader, depth + 1) {
                            if let Some(cell) = cell.as_mut() {
                                cell.blocks.push(Block::Table(inner));
                            }
                        }
                    }
                    _ => {}
                },
                Event::End(name) => match name.as_str() {
                    "tc" => {
                        if let (Some(row), Some(done)) = (row.as_mut(), cell.take()) {
                            row.cells.push(done);
                        }
                    }
                    "tr" => {
                        if let Some(done) = row.take() {
                            table.rows.push(done);
                        }
                    }
                    "tbl" => break,
                    _ => {}
                },
                _ => {}
            }
        }

        if table.rows.is_empty() {
            return None;
        }
        Some(table)
    }
}

/// The style a run starts from: the named style, then the paragraph mark's own.
fn resolve_style(styles: &Styles, id: Option<&str>, mark: &Look) -> Style {
    let named = id.map(|id| styles.resolve(id).0).unwrap_or_default();
    mark.on_top_of(&named)
        .on_top_of(&styles.default_look)
        .over(Style::default())
}

/// Whether a `w:tblBorders` asks for any line to be drawn at all.
///
/// A table used to lay a page out — which is most of them on a letterhead —
/// carries the element with every side set to `none`. Taking its presence as a
/// yes rules a grid across somebody's headed paper.
fn any_border(reader: &mut Reader) -> bool {
    let mut any = false;
    while let Some(event) = reader.next() {
        match event {
            Event::Start(tag) => {
                if matches!(
                    tag.name.as_str(),
                    "top" | "bottom" | "left" | "right" | "insideH" | "insideV" | "start" | "end"
                ) && !matches!(tag.get("val"), None | Some("none") | Some("nil"))
                {
                    any = true;
                }
            }
            Event::End(name) if name == "tblBorders" => break,
            _ => {}
        }
    }
    any
}

/// The white space inside a cell, across and down.
fn read_cell_margins(reader: &mut Reader) -> (f64, f64) {
    // Word's own default, for whichever sides the element does not mention.
    let (mut across, mut down) = (1.9, 0.0);
    while let Some(event) = reader.next() {
        match event {
            Event::Start(tag) => {
                let Some(width) = tag.number("w").map(twips_mm) else {
                    continue;
                };
                match tag.name.as_str() {
                    "left" | "start" => across = width.max(0.0),
                    "top" => down = width.max(0.0),
                    _ => {}
                }
            }
            Event::End(name) if name == "tblCellMar" => break,
            _ => {}
        }
    }
    (across, down)
}

/// Every piece of text between here and the matching end tag.
fn read_text(reader: &mut Reader, name: &str) -> String {
    let mut out = String::new();
    let mut depth = 1usize;
    while let Some(event) = reader.next() {
        match event {
            Event::Text(text) => out.push_str(&text),
            Event::Start(tag) if tag.name == name && !tag.empty => depth += 1,
            Event::End(ended) if ended == name => {
                depth -= 1;
                if depth == 0 {
                    break;
                }
            }
            _ => {}
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Lists
// ---------------------------------------------------------------------------

/// What `numbering.xml` says each list looks like.
#[derive(Debug, Default)]
struct Numbering {
    /// Which abstract list each numbered list uses.
    lists: BTreeMap<String, String>,
    /// The format of each level of each abstract list.
    levels: BTreeMap<(String, usize), Level>,
}

#[derive(Debug, Clone)]
struct Level {
    format: String,
    text: String,
    start: usize,
    indent_mm: Option<f64>,
}

impl Default for Level {
    fn default() -> Level {
        Level {
            format: "bullet".into(),
            text: "\u{2022}".into(),
            start: 1,
            indent_mm: None,
        }
    }
}

impl Numbering {
    fn read(text: &str) -> Numbering {
        let mut out = Numbering::default();
        let mut reader = Reader::new(text);
        let mut abstract_id: Option<String> = None;
        let mut num_id: Option<String> = None;
        let mut level: Option<(usize, Level)> = None;

        while let Some(event) = reader.next() {
            match event {
                Event::Start(tag) => match tag.name.as_str() {
                    "abstractNum" => {
                        abstract_id = tag.get("abstractNumId").map(str::to_string);
                        num_id = None;
                    }
                    "num" => {
                        num_id = tag.get("numId").map(str::to_string);
                        abstract_id = None;
                    }
                    "abstractNumId" => {
                        // Inside a `w:num` this is the link between the two.
                        if let (Some(id), Some(points_at)) = (&num_id, tag.get("val")) {
                            out.lists.insert(id.clone(), points_at.to_string());
                        }
                    }
                    "lvl" => {
                        let at = tag.number("ilvl").unwrap_or(0.0).max(0.0) as usize;
                        level = Some((at, Level::default()));
                    }
                    "numFmt" => {
                        if let (Some((_, level)), Some(format)) = (level.as_mut(), tag.get("val")) {
                            level.format = format.to_string();
                        }
                    }
                    "lvlText" => {
                        if let (Some((_, level)), Some(text)) = (level.as_mut(), tag.get("val")) {
                            level.text = text.to_string();
                        }
                    }
                    "start" => {
                        if let (Some((_, level)), Some(start)) = (level.as_mut(), tag.number("val"))
                        {
                            level.start = start.max(0.0) as usize;
                        }
                    }
                    "ind" => {
                        if let Some((_, level)) = level.as_mut() {
                            level.indent_mm = tag
                                .number("left")
                                .or_else(|| tag.number("start"))
                                .map(twips_mm);
                        }
                    }
                    _ => {}
                },
                Event::End(name) => match name.as_str() {
                    "lvl" => {
                        if let (Some(id), Some((at, found))) = (&abstract_id, level.take()) {
                            out.levels.insert((id.clone(), at), found);
                        }
                    }
                    "abstractNum" => abstract_id = None,
                    "num" => num_id = None,
                    _ => {}
                },
                _ => {}
            }
        }
        out
    }

    /// A fresh set of counters over this numbering.
    fn counting(&self) -> Counters<'_> {
        Counters {
            numbering: self,
            at: BTreeMap::new(),
        }
    }
}

/// Where each list has got to.
struct Counters<'a> {
    numbering: &'a Numbering,
    at: BTreeMap<(String, usize), usize>,
}

impl Counters<'_> {
    /// The marker for the next item of a list, and how far it is indented.
    fn marker(&mut self, num_id: &str, level: usize) -> (Option<String>, f64) {
        let abstract_id = self
            .numbering
            .lists
            .get(num_id)
            .cloned()
            .unwrap_or_else(|| num_id.to_string());
        let found = self
            .numbering
            .levels
            .get(&(abstract_id.clone(), level))
            .cloned()
            .unwrap_or_default();

        let indent = found
            .indent_mm
            .unwrap_or_else(|| 12.7 * (level + 1) as f64 * 0.75);

        if found.format == "none" {
            return (None, indent);
        }
        if found.format == "bullet" {
            return (Some("\u{2022}".to_string()), indent);
        }

        let key = (abstract_id.clone(), level);
        let next = match self.at.get(&key) {
            Some(seen) => seen + 1,
            None => found.start.max(1),
        };
        self.at.insert(key, next);
        // Starting a level again restarts everything under it, which is what
        // makes 1.1, 1.2, then 2.1 rather than 2.3.
        let deeper: Vec<(String, usize)> = self
            .at
            .keys()
            .filter(|(id, at)| id == &abstract_id && *at > level)
            .cloned()
            .collect();
        for key in deeper {
            self.at.remove(&key);
        }

        (Some(render_marker(&found, next, level)), indent)
    }
}

/// A list marker, from the pattern the document gives.
///
/// The pattern is something like `%1.` or `%1.%2`, where each `%n` is the
/// counter at that level. Only the level being numbered is known here, so the
/// others are left as they are — which is right for the overwhelmingly common
/// single-level pattern and honest for the rest.
fn render_marker(level: &Level, count: usize, at: usize) -> String {
    let numeral = match level.format.as_str() {
        "lowerLetter" => letters(count, false),
        "upperLetter" => letters(count, true),
        "lowerRoman" => roman(count).to_lowercase(),
        "upperRoman" => roman(count),
        _ => count.to_string(),
    };
    let pattern = if level.text.trim().is_empty() {
        format!("%{}.", at + 1)
    } else {
        level.text.clone()
    };
    let filled = pattern.replace(&format!("%{}", at + 1), &numeral);
    if filled == pattern && !pattern.contains('%') {
        // A pattern with no placeholder at all is a literal, and a bullet
        // written as a character rather than declared as one.
        return pattern;
    }
    filled
}

#[cfg(test)]
mod tests;
