//! Setting a document's paragraphs on paper.
//!
//! This is a page-layout engine, which sounds far grander than it is. It fills
//! one column of text at a time: measure the words, break the line where the
//! next word will not fit, drop down by the line height, and start a new page
//! at the bottom. Tables are the same thing done once per cell, side by side.
//!
//! Everything is measured in millimetres on the paper, like the rest of
//! Onionskin, and handed to the same PDF writer that every other part of the
//! program uses. Nothing here draws — it decides where things go, and
//! [`crate::pdf`] puts them there.
//!
//! # Where the measurements come from
//!
//! Word widths are not estimated. A PDF reader has fourteen fonts built into
//! it, Adobe published their widths to a thousandth of an em, and
//! [`crate::pdf::builtin_width_mm`] uses exactly those — so a line breaks in
//! the same place in this program, in a reader, and on the printer. Text that
//! needs a font from the machine is measured from that font's own outlines,
//! which is exact in the same way.

use std::collections::BTreeSet;
use std::path::Path;

use super::{Align, Block, Cell, Family, Para, Piece, ReadError, Sheet, Style, Table};
use crate::font::EmbeddedFont;
use crate::geometry::PageSize;
use crate::pdf::{self, Drawing, Font, LineFont, PlacedLine, PlacedShape};

/// A point is a seventy-second of an inch, and an inch is 25.4 mm.
const MM_PER_PT: f64 = 25.4 / 72.0;

/// Line height as a multiple of the type size, when nothing says otherwise.
///
/// The same 1.2 the rest of Onionskin uses for a block of text. Word calls it
/// "single" and computes it from the font's own metrics, which lands between
/// 1.15 and 1.2 for the faces people actually use.
const LEADING: f64 = 1.2;

/// Where the baseline sits below the top of a line, as a fraction of the type
/// size. The rest of the line height is the space under the descenders.
const ASCENT: f64 = 0.9;

/// Half an inch, which is the default tab stop in every word processor.
const TAB_MM: f64 = 12.7;

/// The white space inside a table cell, so text does not touch the rule.
const CELL_PAD_MM: f64 = 1.5;

/// How thick a table rule is drawn. Thin enough to look like a rule, thick
/// enough that a laser printer does not drop half of it.
const RULE_MM: f64 = 0.2;

// ---------------------------------------------------------------------------
// Fonts
// ---------------------------------------------------------------------------

/// Which faces this document can be set in, and what had to be given up.
struct Fonts {
    /// A font from the machine, loaded only when the text needs one.
    embedded: Option<EmbeddedFont>,
    /// Characters no available font could write. Collected rather than
    /// refused: one stray symbol should not stop a hundred-page document, and
    /// saying which ones went missing is more use than saying it failed.
    dropped: BTreeSet<char>,
    /// True when a system font was looked for and not found.
    wanted_a_font: bool,
}

impl Fonts {
    /// Work out what the document needs, and load a font only if it does.
    ///
    /// Most documents are Western European text, which the built-in fonts
    /// cover, and loading a twenty-megabyte system font to write them would be
    /// pure waste. So the text is checked first and the font fetched second.
    fn gather(sheet: &Sheet) -> Fonts {
        let text = sheet.text();
        if pdf::encode_winansi(&text).is_ok() {
            return Fonts {
                embedded: None,
                dropped: BTreeSet::new(),
                wanted_a_font: false,
            };
        }
        let embedded = crate::font::suggest_system_font().and_then(|path| {
            // A font that will not load is not an error worth stopping for:
            // the document is still readable in the built-in faces, minus
            // whatever characters they cannot write.
            EmbeddedFont::load(&path).ok()
        });
        Fonts {
            wanted_a_font: embedded.is_none(),
            embedded,
            dropped: BTreeSet::new(),
        }
    }

    /// Turn a piece of text into something that can actually be printed.
    ///
    /// Three passes, in order of how much is lost: characters the built-in
    /// fonts can write are kept; characters with an obvious plain equivalent
    /// are replaced by it; the rest go to whichever font can write them, or are
    /// dropped and recorded.
    fn tame(&mut self, text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        for ch in text.chars() {
            if pdf::encode_winansi(ch.encode_utf8(&mut [0u8; 4])).is_ok() {
                out.push(ch);
                continue;
            }
            if let Some(plain) = plainly(ch) {
                out.push_str(plain);
                continue;
            }
            match &self.embedded {
                Some(font) if font.has(ch) => out.push(ch),
                _ => {
                    self.dropped.insert(ch);
                }
            }
        }
        out
    }

    /// Which face a piece of text has to be set in.
    ///
    /// Per piece rather than per document, so one Greek word in an English
    /// paragraph does not push the whole page into a different font.
    fn face(&self, text: &str, style: &Style) -> LineFont {
        if self.embedded.is_some() && pdf::encode_winansi(text).is_err() {
            return LineFont::Embedded;
        }
        LineFont::Builtin(builtin(style))
    }

    /// How wide a piece of text is, in millimetres.
    fn width_mm(&self, text: &str, style: &Style) -> f64 {
        match self.face(text, style) {
            LineFont::Builtin(font) => pdf::builtin_width_mm(font, text, style.size_pt),
            LineFont::Embedded => self
                .embedded
                .as_ref()
                .and_then(|font| font.width_mm(text, style.size_pt).ok())
                // A width that cannot be measured is one the writer will refuse
                // later; guessing at the em keeps the line from collapsing to
                // nothing in the meantime.
                .unwrap_or(text.chars().count() as f64 * style.size_pt * MM_PER_PT * 0.5),
        }
    }
}

/// The built-in face for a style.
///
/// There is no bold italic among the eight faces Onionskin carries, so bold
/// wins where a style asks for both — a heading that is meant to stand out
/// still stands out, which italic alone would not manage.
fn builtin(style: &Style) -> Font {
    match style.family {
        Family::Sans => {
            if style.bold {
                Font::HelveticaBold
            } else if style.italic {
                Font::HelveticaOblique
            } else {
                Font::Helvetica
            }
        }
        Family::Serif => {
            if style.bold {
                Font::TimesBold
            } else if style.italic {
                Font::TimesItalic
            } else {
                Font::TimesRoman
            }
        }
        Family::Mono => {
            if style.bold {
                Font::CourierBold
            } else {
                Font::Courier
            }
        }
    }
}

/// A plain equivalent for a character the built-in fonts cannot write.
///
/// These are the ones a word processor inserts by itself — from autocorrect,
/// from a pasted web page, from a hyphenation pass — so they turn up in
/// documents whose author would say they had typed nothing unusual.
fn plainly(ch: char) -> Option<&'static str> {
    Some(match ch {
        '\u{00A0}' | '\u{2007}' | '\u{202F}' | '\u{2009}' | '\u{200A}' | '\u{2002}'
        | '\u{2003}' => " ",
        // Zero-width and formatting marks: nothing on paper either way.
        '\u{200B}'..='\u{200F}' | '\u{FEFF}' | '\u{00AD}' | '\u{2060}' => "",
        '\u{2010}' | '\u{2011}' | '\u{2212}' => "-",
        '\u{2028}' | '\u{2029}' => " ",
        '\u{FB00}' => "ff",
        '\u{FB01}' => "fi",
        '\u{FB02}' => "fl",
        '\u{FB03}' => "ffi",
        '\u{FB04}' => "ffl",
        '\u{02BC}' | '\u{2032}' => "'",
        '\u{2033}' => "\"",
        '\u{2043}' | '\u{25CF}' | '\u{25AA}' | '\u{25E6}' => "\u{2022}",
        _ => return None,
    })
}

// ---------------------------------------------------------------------------
// Words on a line
// ---------------------------------------------------------------------------

/// One indivisible thing on a line.
#[derive(Debug, Clone)]
struct Fragment {
    text: String,
    style: Style,
    face: LineFont,
    width_mm: f64,
    /// A space, which is what may be dropped at a line break and stretched
    /// when a line is justified.
    space: bool,
    /// A tab, whose width is not known until its position is.
    tab: bool,
    /// True for a fragment that must never share a `Tj` with its neighbour.
    /// The list marker is the one case: its width carries a gap that is real
    /// only as *distance to the next fragment*. Glue it into the same run as
    /// the word after it and the gap collapses to nothing, because a PDF
    /// reader spaces the characters of one string by the font's own metrics,
    /// not by whatever width Onionskin reserved between them.
    own_run: bool,
}

/// A line, once it has been decided where it breaks.
struct Line {
    fragments: Vec<Fragment>,
    width_mm: f64,
    /// How far in the line starts, for a first-line or hanging indent.
    indent_mm: f64,
    /// The tallest type on it, which sets the height and the baseline.
    tallest_pt: f64,
    /// True for the last line of a paragraph, which is never stretched.
    last: bool,
}

/// Break a paragraph's text into fragments, one per word and one per space.
fn fragments(para: &Para, fonts: &mut Fonts) -> Vec<Fragment> {
    let mut out = Vec::new();
    for piece in &para.pieces {
        match piece {
            Piece::Text(text, style) => {
                let tamed = fonts.tame(text);
                // Spaces become fragments of their own rather than being
                // trimmed away: a space is where a line may break, and it is
                // what a justified line stretches.
                let mut word = String::new();
                for ch in tamed.chars() {
                    if ch == ' ' {
                        if !word.is_empty() {
                            out.push(fragment(std::mem::take(&mut word), *style, fonts, false));
                        }
                        out.push(fragment(" ".to_string(), *style, fonts, true));
                    } else if ch == '\t' {
                        if !word.is_empty() {
                            out.push(fragment(std::mem::take(&mut word), *style, fonts, false));
                        }
                        out.push(tab_fragment(*style));
                    } else {
                        word.push(ch);
                    }
                }
                if !word.is_empty() {
                    out.push(fragment(word, *style, fonts, false));
                }
            }
            Piece::Tab => out.push(tab_fragment(para.style)),
            // Breaks are handled by the caller, which splits the paragraph on
            // them before any of this runs.
            Piece::LineBreak | Piece::PageBreak => {}
        }
    }
    out
}

fn fragment(text: String, style: Style, fonts: &Fonts, space: bool) -> Fragment {
    let face = fonts.face(&text, &style);
    let width_mm = fonts.width_mm(&text, &style);
    Fragment {
        text,
        style,
        face,
        width_mm,
        space,
        tab: false,
        own_run: false,
    }
}

fn tab_fragment(style: Style) -> Fragment {
    Fragment {
        text: String::new(),
        style,
        face: LineFont::Builtin(builtin(&style)),
        width_mm: 0.0,
        space: true,
        tab: true,
        own_run: false,
    }
}

/// The next tab stop after this position.
fn next_stop(x_mm: f64) -> f64 {
    // The tenth of a millimetre keeps a tab that lands exactly on a stop from
    // staying where it is, which would make two tabs in a row do nothing.
    ((x_mm + 0.1) / TAB_MM).floor() * TAB_MM + TAB_MM
}

/// Break fragments into lines that fit.
///
/// `first_line_mm` may be negative, which is a hanging indent: the first line
/// starts further left than the rest, and the space it leaves is where a list's
/// bullet goes.
fn wrap(mut fragments: Vec<Fragment>, width_mm: f64, first_line_mm: f64) -> Vec<Line> {
    let mut lines: Vec<Line> = Vec::new();
    let mut current: Vec<Fragment> = Vec::new();
    let mut used = 0.0f64;
    let mut indent = first_line_mm;
    let mut limit = (width_mm - indent).max(1.0);

    let mut index = 0usize;
    while index < fragments.len() {
        let mut piece = fragments[index].clone();
        if piece.tab {
            let stop = next_stop(indent + used);
            piece.width_mm = (stop - indent - used).max(0.0);
        }

        // A line already carrying something that this piece will not fit
        // beside breaks here, before anything asks whether the piece would
        // fit a line of its own — otherwise that question is asked against
        // the *old* line, which is not the one this piece is about to start.
        if used + piece.width_mm > limit && !current.is_empty() {
            // The space that would have sat at the break goes nowhere.
            while current.last().map(|f| f.space).unwrap_or(false) {
                current.pop();
            }
            lines.push(finish_line(std::mem::take(&mut current), indent, false));
            indent = 0.0;
            limit = width_mm.max(1.0);
            used = 0.0;
            // A space at the start of a fresh line is not written.
            if piece.space {
                index += 1;
                continue;
            }
        }

        // A word too long for an empty line has to be cut, or it runs off the
        // edge of the paper. Rare in prose and ordinary in a document holding
        // a URL or a long identifier. Checked again here, now that a line this
        // piece could not share has just been flushed, so a word too wide for
        // the column is still split even when it is not the first thing on
        // the paragraph's own line — only the first thing on *some* line.
        if !piece.space && piece.width_mm > limit && current.is_empty() {
            let (head, tail) = split_wide(&piece, limit);
            if let Some(tail) = tail {
                fragments[index] = tail;
                lines.push(finish_line(vec![head], indent, false));
                indent = 0.0;
                limit = width_mm.max(1.0);
                used = 0.0;
                continue;
            }
        }

        used += piece.width_mm;
        current.push(piece);
        index += 1;
    }

    while current.last().map(|f| f.space).unwrap_or(false) {
        current.pop();
    }
    if !current.is_empty() || lines.is_empty() {
        lines.push(finish_line(current, indent, false));
    }
    if let Some(last) = lines.last_mut() {
        last.last = true;
    }
    lines
}

/// As much of an over-long word as fits, and what is left of it.
fn split_wide(piece: &Fragment, limit: f64) -> (Fragment, Option<Fragment>) {
    let per_char = piece.width_mm / piece.text.chars().count().max(1) as f64;
    let fits = ((limit / per_char).floor() as usize).max(1);
    let head: String = piece.text.chars().take(fits).collect();
    let tail: String = piece.text.chars().skip(fits).collect();
    if tail.is_empty() {
        return (piece.clone(), None);
    }
    let mut first = piece.clone();
    first.width_mm = per_char * head.chars().count() as f64;
    first.text = head;
    let mut second = piece.clone();
    second.width_mm = per_char * tail.chars().count() as f64;
    second.text = tail;
    (first, Some(second))
}

fn finish_line(fragments: Vec<Fragment>, indent_mm: f64, last: bool) -> Line {
    let width_mm = fragments.iter().map(|f| f.width_mm).sum();
    let tallest_pt = fragments
        .iter()
        .map(|f| f.style.size_pt)
        .fold(0.0f64, f64::max);
    Line {
        fragments,
        width_mm,
        indent_mm,
        tallest_pt,
        last,
    }
}

// ---------------------------------------------------------------------------
// The page
// ---------------------------------------------------------------------------

#[derive(Default, Clone)]
struct Leaf {
    lines: Vec<PlacedLine>,
    shapes: Vec<PlacedShape>,
}

/// Where we are on the paper, and everything put down so far.
struct Pen<'a> {
    fonts: &'a mut Fonts,
    leaves: Vec<Leaf>,
    at: usize,
    /// The top of the next line.
    y_mm: f64,
    top_mm: f64,
    bottom_mm: f64,
    /// False inside a table cell, which is measured before it is placed and so
    /// must not start a page of its own halfway through.
    paginate: bool,
    notes: Vec<String>,
}

impl<'a> Pen<'a> {
    fn new(sheet: &Sheet, fonts: &'a mut Fonts) -> Pen<'a> {
        Pen {
            fonts,
            leaves: vec![Leaf::default()],
            at: 0,
            y_mm: sheet.margins.top_mm,
            top_mm: sheet.margins.top_mm,
            bottom_mm: (sheet.page.height_mm - sheet.margins.bottom_mm)
                .max(sheet.margins.top_mm + 10.0),
            paginate: true,
            notes: Vec::new(),
        }
    }

    /// A pen that measures rather than paginates, for laying out a table cell.
    ///
    /// A lifetime of its own, because it borrows the fonts from a pen that is
    /// itself borrowed — for the length of the measurement and no longer.
    fn measuring<'b>(fonts: &'b mut Fonts) -> Pen<'b> {
        Pen {
            fonts,
            leaves: vec![Leaf::default()],
            at: 0,
            y_mm: 0.0,
            top_mm: 0.0,
            bottom_mm: f64::MAX,
            paginate: false,
            notes: Vec::new(),
        }
    }

    fn leaf(&mut self) -> &mut Leaf {
        while self.leaves.len() <= self.at {
            self.leaves.push(Leaf::default());
        }
        &mut self.leaves[self.at]
    }

    fn new_page(&mut self) {
        if !self.paginate {
            return;
        }
        self.at += 1;
        self.y_mm = self.top_mm;
        self.leaf();
    }

    /// Make room for something this tall, moving to a new page if it will not
    /// fit on this one.
    fn room_for(&mut self, height_mm: f64) {
        if !self.paginate {
            return;
        }
        let at_top = (self.y_mm - self.top_mm).abs() < 1e-9;
        if self.y_mm + height_mm > self.bottom_mm && !at_top {
            self.new_page();
        }
    }

    /// Whether anything has actually been drawn on the page the pen is on.
    ///
    /// `break_before` must still respect a page that is blank because nothing
    /// has been put on it yet, or the very first heading in a document — or
    /// one right after an explicit page break — would leave an empty page
    /// sitting in front of its own content.
    fn page_is_blank(&self) -> bool {
        self.leaves
            .get(self.at)
            .map(|leaf| leaf.lines.is_empty() && leaf.shapes.is_empty())
            .unwrap_or(true)
    }

    fn note(&mut self, text: impl Into<String>) {
        let text = text.into();
        if !self.notes.contains(&text) {
            self.notes.push(text);
        }
    }

    /// Lay out a run of blocks in a column.
    fn flow(&mut self, blocks: &[Block], x_mm: f64, width_mm: f64, depth: usize) {
        for block in blocks {
            match block {
                Block::Para(para) => self.flow_para(para, x_mm, width_mm),
                Block::Table(table) => {
                    // A table inside a table inside a table is a document doing
                    // something Onionskin is not going to follow forever.
                    if depth >= 4 {
                        self.note(
                            "A table was nested more deeply than Onionskin follows, and \
                             the innermost one was left out.",
                        );
                        continue;
                    }
                    self.flow_table(table, x_mm, width_mm, depth + 1);
                }
            }
        }
    }

    fn flow_para(&mut self, para: &Para, x_mm: f64, width_mm: f64) {
        if para.break_before && !self.page_is_blank() {
            self.new_page();
        }
        self.y_mm += para.space_before_mm;

        let left = x_mm + para.indent_left_mm;
        let column = (width_mm - para.indent_left_mm - para.indent_right_mm).max(5.0);

        // An explicit break inside a paragraph splits it into pieces that are
        // wrapped separately, so a break always breaks.
        let mut first_of_paragraph = true;
        for (index, run) in split_on_breaks(para).into_iter().enumerate() {
            if index > 0 && run.page_break {
                self.new_page();
            }
            let mut part = para.clone();
            part.pieces = run.pieces;
            let mut pieces = fragments(&part, self.fonts);
            if first_of_paragraph {
                if let Some(marker) = &para.marker {
                    let tamed = self.fonts.tame(marker);
                    let width = self.fonts.width_mm(&tamed, &para.style);
                    let face = self.fonts.face(&tamed, &para.style);
                    // The marker sits in the hanging indent and the text starts
                    // where it would have anyway, which is what makes a list
                    // line up.
                    let gap = (-para.first_line_mm - width).max(1.5);
                    pieces.insert(
                        0,
                        Fragment {
                            text: tamed,
                            style: para.style,
                            face,
                            width_mm: width + gap,
                            space: false,
                            tab: false,
                            // Otherwise a marker in the same style as the text
                            // — the ordinary case — merges into one `Tj` with
                            // the word after it, and the gap is lost.
                            own_run: true,
                        },
                    );
                }
            }

            let first_line = if first_of_paragraph {
                para.first_line_mm
            } else {
                0.0
            };
            let lines = wrap(pieces, column, first_line);
            for line in lines {
                self.place(&line, left, column, para);
            }
            first_of_paragraph = false;
        }
        self.y_mm += para.space_after_mm;
    }

    /// Put one line on the page.
    fn place(&mut self, line: &Line, left_mm: f64, column_mm: f64, para: &Para) {
        let align = para.align;
        // An empty paragraph has no type on it and is still a blank line the
        // height of whatever it would have been set in — which is how every
        // document in the world puts a gap between two others.
        let size_pt = if line.tallest_pt > 0.0 {
            line.tallest_pt
        } else {
            para.style.size_pt
        };
        let height = size_pt * MM_PER_PT * LEADING * para.line_spacing.max(0.5);
        self.room_for(height);

        let baseline = self.y_mm + size_pt * MM_PER_PT * ASCENT;
        let free = (column_mm - line.indent_mm - line.width_mm).max(0.0);
        let (mut x, stretch) = match align {
            Align::Left => (left_mm + line.indent_mm, 0.0),
            Align::Centre => (left_mm + line.indent_mm + free / 2.0, 0.0),
            Align::Right => (left_mm + line.indent_mm + free, 0.0),
            Align::Justify => {
                let gaps = line.fragments.iter().filter(|f| f.space).count();
                // The last line of a justified paragraph is set flush left,
                // which is what justification means everywhere.
                if line.last || gaps == 0 || free <= 0.0 {
                    (left_mm + line.indent_mm, 0.0)
                } else {
                    (left_mm + line.indent_mm, free / gaps as f64)
                }
            }
        };

        // Neighbouring fragments in the same face and colour are written as one
        // run. It halves the size of the file and keeps words whole for
        // anything that reads the text back out again.
        let mut run: Option<(String, f64, Style, LineFont)> = None;
        for fragment in &line.fragments {
            let width = fragment.width_mm + if fragment.space { stretch } else { 0.0 };
            let joinable = !fragment.tab && !fragment.own_run && stretch == 0.0;
            match &mut run {
                Some((text, _, style, face))
                    if joinable && *style == fragment.style && *face == fragment.face =>
                {
                    text.push_str(&fragment.text);
                }
                _ => {
                    if let Some((text, at, style, face)) = run.take() {
                        self.write(text, at, baseline, &style, face);
                    }
                    if joinable {
                        run = Some((fragment.text.clone(), x, fragment.style, fragment.face));
                    } else if !fragment.text.is_empty() {
                        self.write(
                            fragment.text.clone(),
                            x,
                            baseline,
                            &fragment.style,
                            fragment.face,
                        );
                    }
                }
            }
            if fragment.style.underline || fragment.style.strike {
                self.rule(fragment, x, baseline, width);
            }
            x += width;
        }
        if let Some((text, at, style, face)) = run.take() {
            self.write(text, at, baseline, &style, face);
        }

        self.y_mm += height;
    }

    fn write(&mut self, text: String, x_mm: f64, y_mm: f64, style: &Style, face: LineFont) {
        if text.trim().is_empty() {
            return;
        }
        let line = PlacedLine {
            text,
            x_mm,
            y_mm,
            size_pt: style.size_pt,
            font: face,
            rotation_deg: 0.0,
            colour: style.colour,
        };
        self.leaf().lines.push(line);
    }

    /// The line under or through a piece of text.
    ///
    /// Drawn rather than written: a PDF's built-in fonts have no underline, and
    /// every word processor draws it as a rule anyway.
    fn rule(&mut self, fragment: &Fragment, x_mm: f64, baseline_mm: f64, width_mm: f64) {
        if fragment.text.trim().is_empty() && !fragment.space {
            return;
        }
        let size_mm = fragment.style.size_pt * MM_PER_PT;
        let thickness = (size_mm * 0.055).max(0.15);
        let mut at = Vec::new();
        if fragment.style.underline {
            at.push(baseline_mm + size_mm * 0.13);
        }
        if fragment.style.strike {
            at.push(baseline_mm - size_mm * 0.25);
        }
        for y in at {
            self.leaf().shapes.push(PlacedShape {
                drawing: Drawing::Line {
                    from: (x_mm, y),
                    to: (x_mm + width_mm, y),
                },
                stroke: Some(fragment.style.colour),
                fill: None,
                width_mm: thickness,
                dash_mm: None,
            });
        }
    }

    /// Lay out a table: every cell measured, then the row placed as one.
    fn flow_table(&mut self, table: &Table, x_mm: f64, width_mm: f64, depth: usize) {
        let columns = table_columns(table, width_mm);
        if columns.is_empty() {
            return;
        }
        // What the document says, or the ordinary amount. Worth asking rather
        // than assuming: a table fitted to its contents with hairline margins
        // wraps every cell if a comfortable margin is taken for granted.
        let (pad_x, pad_y) = table.padding_mm.unwrap_or((CELL_PAD_MM, CELL_PAD_MM));

        for row in &table.rows {
            // Each cell is laid out on its own before anything is committed,
            // because the height of the row is the height of its tallest cell
            // and that is not known until they have all been set.
            let mut slabs: Vec<(f64, f64, Leaf, f64)> = Vec::new();
            let mut column = 0usize;
            let mut tallest = 0.0f64;

            for cell in &row.cells {
                if column >= columns.len() {
                    break;
                }
                let span = cell.span.max(1).min(columns.len() - column);
                let cell_x: f64 = x_mm + columns[..column].iter().sum::<f64>();
                let cell_w: f64 = columns[column..column + span].iter().sum();

                let (leaf, height) = self.measure_cell(cell, cell_w, pad_x, depth);
                tallest = tallest.max(height);
                slabs.push((cell_x, cell_w, leaf, height));
                column += span;
            }

            let row_height = tallest + pad_y * 2.0;
            self.room_for(row_height);
            if self.paginate && self.y_mm + row_height > self.bottom_mm {
                self.note(
                    "A table row was taller than the page and has been set from the \
                     top of one; some of it may run off the bottom.",
                );
            }
            let top = self.y_mm;

            for (cell_x, cell_w, leaf, _) in slabs {
                for mut line in leaf.lines {
                    line.x_mm += cell_x + pad_x;
                    line.y_mm += top + pad_y;
                    self.leaf().lines.push(line);
                }
                for shape in leaf.shapes {
                    self.leaf()
                        .shapes
                        .push(shifted(shape, cell_x + pad_x, top + pad_y));
                }
                if table.bordered {
                    self.leaf().shapes.push(PlacedShape {
                        drawing: Drawing::Rect {
                            x_mm: cell_x,
                            y_mm: top,
                            width_mm: cell_w,
                            height_mm: row_height,
                            radius_mm: 0.0,
                        },
                        stroke: Some((0.0, 0.0, 0.0)),
                        fill: None,
                        width_mm: RULE_MM,
                        dash_mm: None,
                    });
                }
            }
            self.y_mm = top + row_height;
        }
    }

    /// Set a cell's contents in a column of its own, and say how tall they are.
    fn measure_cell(
        &mut self,
        cell: &Cell,
        width_mm: f64,
        pad_x: f64,
        depth: usize,
    ) -> (Leaf, f64) {
        let mut inner = Pen::measuring(self.fonts);
        inner.flow(&cell.blocks, 0.0, (width_mm - pad_x * 2.0).max(3.0), depth);
        let height = inner.y_mm;
        let notes = std::mem::take(&mut inner.notes);
        let leaf = inner.leaves.into_iter().next().unwrap_or_default();
        for note in notes {
            self.note(note);
        }
        (leaf, height)
    }
}

/// Move a shape by an offset, which is what placing a measured cell means.
fn shifted(shape: PlacedShape, dx: f64, dy: f64) -> PlacedShape {
    let drawing = match shape.drawing {
        Drawing::Line { from, to } => Drawing::Line {
            from: (from.0 + dx, from.1 + dy),
            to: (to.0 + dx, to.1 + dy),
        },
        Drawing::Rect {
            x_mm,
            y_mm,
            width_mm,
            height_mm,
            radius_mm,
        } => Drawing::Rect {
            x_mm: x_mm + dx,
            y_mm: y_mm + dy,
            width_mm,
            height_mm,
            radius_mm,
        },
        Drawing::Ellipse {
            centre,
            radius_x_mm,
            radius_y_mm,
        } => Drawing::Ellipse {
            centre: (centre.0 + dx, centre.1 + dy),
            radius_x_mm,
            radius_y_mm,
        },
        Drawing::Path { points, closed } => Drawing::Path {
            points: points.into_iter().map(|(x, y)| (x + dx, y + dy)).collect(),
            closed,
        },
    };
    PlacedShape { drawing, ..shape }
}

/// How wide each column of a table is.
///
/// A document usually says, and when it does not the space is shared equally —
/// which is what a word processor does with a table somebody has just inserted.
fn table_columns(table: &Table, width_mm: f64) -> Vec<f64> {
    let widest = table
        .rows
        .iter()
        .map(|row| row.cells.iter().map(|c| c.span.max(1)).sum::<usize>())
        .max()
        .unwrap_or(0);
    let count = table.columns_mm.len().max(widest);
    if count == 0 {
        return Vec::new();
    }
    if table.columns_mm.len() == count {
        let total: f64 = table.columns_mm.iter().sum();
        // A table wider than the paper is set to the paper, keeping the
        // proportions the document asked for.
        if total > width_mm && total > 0.0 {
            let scale = width_mm / total;
            return table.columns_mm.iter().map(|w| w * scale).collect();
        }
        return table.columns_mm.clone();
    }
    vec![width_mm / count as f64; count]
}

/// A paragraph split at its explicit breaks.
struct Run {
    pieces: Vec<Piece>,
    page_break: bool,
}

fn split_on_breaks(para: &Para) -> Vec<Run> {
    let mut runs = vec![Run {
        pieces: Vec::new(),
        page_break: false,
    }];
    for piece in &para.pieces {
        match piece {
            Piece::LineBreak => runs.push(Run {
                pieces: Vec::new(),
                page_break: false,
            }),
            Piece::PageBreak => runs.push(Run {
                pieces: Vec::new(),
                page_break: true,
            }),
            other => runs
                .last_mut()
                .expect("there is always one")
                .pieces
                .push(other.clone()),
        }
    }
    runs
}

// ---------------------------------------------------------------------------
// The public door
// ---------------------------------------------------------------------------

/// Set a document on paper and write the PDF, returning what was approximated.
pub fn write(sheet: &Sheet, into: &Path, source: &Path) -> Result<Vec<String>, ReadError> {
    let mut fonts = Fonts::gather(sheet);
    let mut pen = Pen::new(sheet, &mut fonts);
    pen.flow(
        &sheet.blocks,
        sheet.margins.left_mm,
        (sheet.page.width_mm - sheet.margins.left_mm - sheet.margins.right_mm).max(10.0),
        0,
    );

    let mut notes = std::mem::take(&mut pen.notes);
    let leaves = std::mem::take(&mut pen.leaves);

    let pages: Vec<PageSize> = vec![sheet.page; leaves.len().max(1)];
    let lines: Vec<Vec<PlacedLine>> = leaves.iter().map(|leaf| leaf.lines.clone()).collect();
    let shapes: Vec<Vec<PlacedShape>> = leaves.iter().map(|leaf| leaf.shapes.clone()).collect();

    if fonts.wanted_a_font {
        notes.push(
            "This document uses letters the built-in fonts cannot write, and no font \
             with wider coverage was found on this machine. Install one, or use \
             LibreOffice for this file."
                .into(),
        );
    }
    if !fonts.dropped.is_empty() {
        let shown: String = fonts.dropped.iter().take(12).collect();
        notes.push(format!(
            "{} character(s) could not be written and were left out: {shown}",
            fonts.dropped.len()
        ));
    }

    let title = source
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "document".into());
    pdf::write_page_content(
        into,
        &pages,
        &lines,
        &shapes,
        &title,
        fonts.embedded.as_ref(),
    )?;
    Ok(notes)
}

#[cfg(test)]
mod tests;
