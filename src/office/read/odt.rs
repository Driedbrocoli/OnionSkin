//! Reading OpenDocument Text: `.odt`, `.ott`, and the flat `.fodt`.
//!
//! An `.odt` is a zip holding `content.xml` (the body, and whatever styles are
//! automatic to it) and `styles.xml` (the named styles, and the page itself);
//! a `.fodt` is the same information as one XML document with no zip around
//! it. Both are read by the same walker below — [`read`] unzips and hands the
//! two parts to it, [`read_flat`] hands it the one document twice, once to
//! gather styles from and once to walk the body of — so a bug fixed in how a
//! style is resolved is fixed for both at once.
//!
//! # Styles
//!
//! An OpenDocument style names a `style:parent-style-name` and inherits
//! whatever it does not set itself, all the way up to a `style:default-style`
//! that is the implicit parent of anything that names none. A run of text is
//! set in whatever its `text:span` says, layered over whatever the paragraph
//! around it says, layered in turn over that inheritance chain — each level
//! filling in only the gaps the one inside it left. [`StyleTable`] is that
//! chain, flattened to a name lookup, because nothing here needs the tree
//! shape, only what falls out of walking it once per paragraph.
//!
//! # What is not here
//!
//! Images, footnotes, headers, footers and multi-column sections are noted
//! and left out rather than approximated — [`Sheet::note`] is how the caller
//! finds out. Comments and tracked changes are resolved silently to the
//! document's current state: an insertion reads as ordinary text and a
//! deletion is skipped, because showing both at once is not what either the
//! reader or the author would call reading the document.

// Clippy would rather every `while let Some(x) = reader.next()` here were a
// `for x in reader.by_ref()`. That reads just as well right up until the loop
// body itself needs another look at the reader — to recurse into a nested
// element or to skip one — which every loop in this file does sooner or
// later. A `for` loop holds its iterator borrowed for the whole loop, so that
// second borrow will not compile; only a fresh `.next()` call each iteration
// lets the body borrow the reader again in between.
#![allow(clippy::while_let_on_iterator)]

use std::collections::HashMap;

use super::{
    Align, Block, Cell, Family, Margins, Para, Piece, ReadError, Row, Sheet, Style, Table,
};
use crate::geometry::{mm_to_pt, PageSize};
use crate::office::unzip::Archive;
use crate::office::xml;

// ---------------------------------------------------------------------------
// The public door
// ---------------------------------------------------------------------------

/// Read a zipped `.odt` or `.ott`.
pub fn read(bytes: &[u8]) -> Result<Sheet, ReadError> {
    let archive = Archive::open(bytes)?;
    let content = xml::decode(&archive.read("content.xml")?);
    // `styles.xml` carries the named styles and the page, but a document
    // missing it is still worth reading for its text — so its absence is a
    // bare document rather than a reason to refuse the whole file.
    let styles = archive
        .read("styles.xml")
        .map(|bytes| xml::decode(&bytes))
        .unwrap_or_default();

    let mut table = StyleTable::default();
    table.collect(&styles);
    table.collect(&content);
    Ok(finish(&content, table))
}

/// Read a flat `.fodt`: one XML document with no zip around it, already
/// decoded from whatever encoding it was written in.
pub fn read_flat(text: &str) -> Result<Sheet, ReadError> {
    let mut table = StyleTable::default();
    table.collect(text);
    Ok(finish(text, table))
}

/// Turn a gathered [`StyleTable`] and the document's body into a [`Sheet`].
/// Shared by both entry points, which differ only in how they got here: one
/// document already unzipped into two, or one read as itself twice.
fn finish(content_source: &str, table: StyleTable) -> Sheet {
    let layout = table.page.unwrap_or_default();
    let page = PageSize::new(
        layout.width_mm.unwrap_or(210.0),
        layout.height_mm.unwrap_or(297.0),
    );
    let mut sheet = Sheet::new(page, layout.margins);
    if table.has_header_or_footer {
        sheet.note("This document has a header or a footer. Onionskin leaves them out.");
    }
    let blocks = walk_body(content_source, &table, &mut sheet);
    sheet.blocks = blocks;
    sheet
}

// ---------------------------------------------------------------------------
// Styles: gathering them, and resolving one against its ancestors
// ---------------------------------------------------------------------------

/// Every style, list style and the page, gathered from one or two XML
/// documents before any of it is put together into paragraphs.
///
/// Flat by name rather than a tree, because resolving a style only ever means
/// walking `style:parent-style-name` upward from one name, never traversing
/// the whole set — a map answers that directly.
#[derive(Debug, Default)]
struct StyleTable {
    styles: HashMap<String, StyleDef>,
    lists: HashMap<String, ListStyle>,
    /// `<style:default-style style:family="paragraph">`: the implicit parent
    /// of any paragraph style that names none of its own.
    default_paragraph: StyleDef,
    page: Option<PageLayout>,
    /// Whether a master page's header or footer had anything in it. Recorded
    /// here because by the time the body is walked, the styles document that
    /// held it is out of view.
    has_header_or_footer: bool,
}

/// One `<style:style>` or `<style:default-style>`, whatever family it is —
/// the properties that do not apply to a given family are simply never set,
/// so one shape serves paragraph, text, table-column and table-cell styles
/// alike rather than four near-identical ones.
#[derive(Debug, Clone, Default)]
struct StyleDef {
    parent: Option<String>,
    text: TextProps,
    para: ParaProps,
    column_width_mm: Option<f64>,
    bordered: bool,
    /// White space inside a cell. One number, because `fo:padding` is one
    /// number and the per-side form is rare enough that the left one stands
    /// for all four.
    padding_mm: Option<f64>,
    /// Set on a `style:family="section"` style that gives its section more
    /// than one column.
    column_count: Option<u32>,
}

/// Everything `style:text-properties` might set, each field `None` unless
/// this particular style said so — resolving a chain is telling the
/// difference between "set to plain" and "did not say".
#[derive(Debug, Clone, Copy, Default)]
struct TextProps {
    size: Option<SizeSpec>,
    bold: Option<bool>,
    italic: Option<bool>,
    underline: Option<bool>,
    strike: Option<bool>,
    colour: Option<(f64, f64, f64)>,
    font: Option<Family>,
}

impl TextProps {
    fn from_tag(tag: &xml::Tag) -> TextProps {
        TextProps {
            size: tag.get("font-size").and_then(parse_size),
            bold: tag
                .get("font-weight")
                .map(|value| value.eq_ignore_ascii_case("bold")),
            italic: tag
                .get("font-style")
                .map(|value| value.eq_ignore_ascii_case("italic")),
            underline: tag
                .get("text-underline-style")
                .map(|value| value.trim() != "none"),
            strike: tag
                .get("text-line-through-style")
                .map(|value| value.trim() != "none"),
            colour: tag.get("color").and_then(parse_hex_colour),
            // `style:font-name` refers to a `style:font-face` declaration by
            // name; `fo:font-family` gives the family straight out. Either
            // way, only the shape of the font (serif, sans, mono) is wanted,
            // so it is resolved to that here rather than kept as a string.
            font: tag
                .get("font-name")
                .or_else(|| tag.get("font-family"))
                .map(Family::of),
        }
    }
}

/// A length or a percentage, exactly as `fo:font-size` may hold either —
/// resolving the percentage needs to know what it is a percentage *of*, which
/// is only known once the styles above it in the chain have been applied.
#[derive(Debug, Clone, Copy)]
enum SizeSpec {
    Pt(f64),
    Percent(f64),
}

/// Everything `style:paragraph-properties` might set. Line height is kept as
/// written (a length or a percentage) rather than converted here, because
/// converting it needs the paragraph's resolved type size, which is not yet
/// known while a single style is being read.
#[derive(Debug, Clone, Copy, Default)]
struct ParaProps {
    align: Option<Align>,
    space_before_mm: Option<f64>,
    space_after_mm: Option<f64>,
    indent_left_mm: Option<f64>,
    indent_right_mm: Option<f64>,
    first_line_mm: Option<f64>,
    line_height: Option<LineHeight>,
    break_before: Option<bool>,
}

impl ParaProps {
    fn from_tag(tag: &xml::Tag) -> ParaProps {
        ParaProps {
            align: tag.get("text-align").and_then(parse_align),
            space_before_mm: tag.get("margin-top").and_then(length_mm),
            space_after_mm: tag.get("margin-bottom").and_then(length_mm),
            indent_left_mm: tag.get("margin-left").and_then(length_mm),
            indent_right_mm: tag.get("margin-right").and_then(length_mm),
            first_line_mm: tag.get("text-indent").and_then(length_mm),
            line_height: tag.get("line-height").and_then(parse_line_height),
            break_before: tag.get("break-before").map(|value| value.trim() == "page"),
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum LineHeight {
    Percent(f64),
    Absolute(f64),
}

/// Sets `into` to `from`, but only when `from` actually says something —
/// folding a chain of styles is applying this, in order, for every field of
/// every style from the root down, so a leaf that is silent about a property
/// leaves whatever its ancestors decided untouched.
fn overlay<T: Copy>(into: &mut Option<T>, from: Option<T>) {
    if from.is_some() {
        *into = from;
    }
}

/// A [`TextProps`] chain, folded into one: `None` only where nothing from the
/// document's default up to the leaf style ever set a value.
#[derive(Debug, Clone, Copy, Default)]
struct MergedText {
    size_pt: Option<f64>,
    bold: Option<bool>,
    italic: Option<bool>,
    underline: Option<bool>,
    strike: Option<bool>,
    colour: Option<(f64, f64, f64)>,
    family: Option<Family>,
}

impl MergedText {
    /// Folds in one more style, root to leaf. A percentage size composes
    /// against whatever has been resolved so far — its own parent in the
    /// chain — rather than the document's ultimate default, which is what
    /// `fo:font-size="120%"` means on a style two or three levels deep.
    fn absorb(&mut self, props: &TextProps) {
        if let Some(size) = props.size {
            self.size_pt = Some(match size {
                SizeSpec::Pt(pt) => pt,
                SizeSpec::Percent(percent) => {
                    self.size_pt.unwrap_or(Style::default().size_pt) * percent / 100.0
                }
            });
        }
        overlay(&mut self.bold, props.bold);
        overlay(&mut self.italic, props.italic);
        overlay(&mut self.underline, props.underline);
        overlay(&mut self.strike, props.strike);
        overlay(&mut self.colour, props.colour);
        overlay(&mut self.family, props.font);
    }

    /// Lays the merged chain over a starting style, changing only the fields
    /// the chain actually set. `style` is `Style::default()` for an ordinary
    /// paragraph or a heading's outline-level fallback for one — either way,
    /// what the document's own styles say always wins.
    fn apply_to(&self, mut style: Style) -> Style {
        if let Some(size_pt) = self.size_pt {
            style.size_pt = size_pt;
        }
        if let Some(bold) = self.bold {
            style.bold = bold;
        }
        if let Some(italic) = self.italic {
            style.italic = italic;
        }
        if let Some(underline) = self.underline {
            style.underline = underline;
        }
        if let Some(strike) = self.strike {
            style.strike = strike;
        }
        if let Some(colour) = self.colour {
            style.colour = colour;
        }
        if let Some(family) = self.family {
            style.family = family;
        }
        style
    }
}

/// A [`ParaProps`] chain, folded into one, the same way [`MergedText`] is.
#[derive(Debug, Clone, Copy, Default)]
struct MergedPara {
    align: Option<Align>,
    space_before_mm: Option<f64>,
    space_after_mm: Option<f64>,
    indent_left_mm: Option<f64>,
    indent_right_mm: Option<f64>,
    first_line_mm: Option<f64>,
    line_height: Option<LineHeight>,
    break_before: Option<bool>,
}

impl MergedPara {
    fn absorb(&mut self, props: &ParaProps) {
        overlay(&mut self.align, props.align);
        overlay(&mut self.space_before_mm, props.space_before_mm);
        overlay(&mut self.space_after_mm, props.space_after_mm);
        overlay(&mut self.indent_left_mm, props.indent_left_mm);
        overlay(&mut self.indent_right_mm, props.indent_right_mm);
        overlay(&mut self.first_line_mm, props.first_line_mm);
        overlay(&mut self.line_height, props.line_height);
        overlay(&mut self.break_before, props.break_before);
    }
}

/// A list style's levels: which ones are numbered and which are bulleted.
#[derive(Debug, Clone, Default)]
struct ListStyle {
    levels: HashMap<u32, ListKind>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ListKind {
    Bullet,
    Number,
}

impl StyleTable {
    /// Reads every `style:style`, `style:default-style`, `text:list-style`
    /// and `style:page-layout` out of one XML document. Called once for
    /// `styles.xml` and once for `content.xml` when the file is zipped, or
    /// once for the whole thing when it is flat — either way, a style with
    /// the same name seen twice is fine, since the second reading simply
    /// replaces the first with an identical one.
    fn collect(&mut self, source: &str) {
        let mut reader = xml::Reader::new(source);
        while let Some(event) = reader.next() {
            let xml::Event::Start(tag) = event else {
                continue;
            };
            match tag.name.as_str() {
                "page-layout-properties" if self.page.is_none() => {
                    self.page = Some(PageLayout::from_tag(&tag));
                }
                "style" => match tag.get("name") {
                    Some(name) => {
                        let name = name.to_string();
                        let mut def = read_properties(&mut reader, "style");
                        def.parent = tag.get("parent-style-name").map(str::to_string);
                        self.styles.insert(name, def);
                    }
                    None => reader.skip_element("style"),
                },
                "default-style" => {
                    let is_paragraph = tag.get("family") == Some("paragraph");
                    let def = read_properties(&mut reader, "default-style");
                    if is_paragraph {
                        self.default_paragraph = def;
                    }
                }
                "list-style" => match tag.get("name") {
                    Some(name) => {
                        let name = name.to_string();
                        let list = read_list_style(&mut reader);
                        self.lists.insert(name, list);
                    }
                    None => reader.skip_element("list-style"),
                },
                "header" | "footer" if tag.prefix == "style" && !tag.empty => {
                    self.has_header_or_footer = true;
                    reader.skip_element(&tag.name);
                }
                _ => {}
            }
        }
    }

    /// A style's ancestors through `style:parent-style-name`, root first.
    /// Bounded to sixteen levels: real documents nest three or four deep, and
    /// a bound this generous is only ever reached by a cycle, which would
    /// otherwise chase its own tail forever.
    fn ancestry(&self, name: &str) -> Vec<&StyleDef> {
        let mut chain = Vec::new();
        let mut current = Some(name.to_string());
        for _ in 0..16 {
            let Some(current_name) = current else {
                break;
            };
            let Some(def) = self.styles.get(&current_name) else {
                break;
            };
            chain.push(def);
            current = def.parent.clone();
        }
        chain.reverse();
        chain
    }

    /// A paragraph or heading's own look and layout. `seed` is
    /// `Style::default()` for an ordinary paragraph, or a heading's
    /// outline-level fallback — either way, the document's default paragraph
    /// style and then the named chain are applied on top, so anything the
    /// document actually says always wins over the seed.
    fn resolve_paragraph(&self, name: Option<&str>, seed: Style) -> (Style, MergedPara) {
        let mut chain = vec![&self.default_paragraph];
        if let Some(name) = name {
            chain.extend(self.ancestry(name));
        }
        let mut text = MergedText::default();
        let mut para = MergedPara::default();
        for def in chain {
            text.absorb(&def.text);
            para.absorb(&def.para);
        }
        (text.apply_to(seed), para)
    }

    /// A span's style, layered over the paragraph's own — not over the
    /// document default again — so a span that sets nothing at all reads
    /// exactly as the paragraph around it does.
    fn resolve_span(&self, base: Style, name: Option<&str>) -> Style {
        let Some(name) = name else { return base };
        let mut text = MergedText::default();
        for def in self.ancestry(name) {
            text.absorb(&def.text);
        }
        text.apply_to(base)
    }
}

/// Reads whichever of `style:text-properties`, `style:paragraph-properties`,
/// `style:table-column-properties`, `style:table-cell-properties` and a
/// section's `style:columns` appear inside a `style:style` or
/// `style:default-style`, however many of them there are, until the element
/// they belong to closes.
fn read_properties(reader: &mut xml::Reader, closing: &str) -> StyleDef {
    let mut def = StyleDef::default();
    while let Some(event) = reader.next() {
        match event {
            xml::Event::Start(tag) if tag.name == "text-properties" => {
                def.text = TextProps::from_tag(&tag);
            }
            xml::Event::Start(tag) if tag.name == "paragraph-properties" => {
                def.para = ParaProps::from_tag(&tag);
            }
            xml::Event::Start(tag) if tag.name == "table-column-properties" => {
                def.column_width_mm = tag.get("column-width").and_then(length_mm);
            }
            xml::Event::Start(tag) if tag.name == "table-cell-properties" => {
                def.bordered = has_visible_border(&tag);
                def.padding_mm = tag
                    .get("padding")
                    .or_else(|| tag.get("padding-left"))
                    .and_then(length_mm);
            }
            xml::Event::Start(tag) if tag.name == "columns" => {
                def.column_count = tag.number("number-columns").map(|value| value as u32);
            }
            xml::Event::End(name) if name == closing => break,
            _ => {}
        }
    }
    def
}

fn read_list_style(reader: &mut xml::Reader) -> ListStyle {
    let mut list = ListStyle::default();
    while let Some(event) = reader.next() {
        match event {
            xml::Event::Start(tag) if tag.name == "list-level-style-bullet" => {
                if let Some(level) = tag.number("level").map(|value| value as u32) {
                    list.levels.insert(level, ListKind::Bullet);
                }
            }
            xml::Event::Start(tag) if tag.name == "list-level-style-number" => {
                if let Some(level) = tag.number("level").map(|value| value as u32) {
                    list.levels.insert(level, ListKind::Number);
                }
            }
            xml::Event::End(name) if name == "list-style" => break,
            _ => {}
        }
    }
    list
}

// ---------------------------------------------------------------------------
// The page
// ---------------------------------------------------------------------------

/// A `style:page-layout`'s size, margins and orientation. Orientation is read
/// but not acted on: OpenDocument always writes `fo:page-width` and
/// `fo:page-height` as the actual, final dimensions, landscape included, so
/// there is nothing left for the orientation attribute to change.
#[derive(Debug, Clone, Copy, Default)]
struct PageLayout {
    width_mm: Option<f64>,
    height_mm: Option<f64>,
    margins: Margins,
}

impl PageLayout {
    fn from_tag(tag: &xml::Tag) -> PageLayout {
        let mut margins = Margins::default();
        if let Some(mm) = tag.get("margin-top").and_then(length_mm) {
            margins.top_mm = mm;
        }
        if let Some(mm) = tag.get("margin-right").and_then(length_mm) {
            margins.right_mm = mm;
        }
        if let Some(mm) = tag.get("margin-bottom").and_then(length_mm) {
            margins.bottom_mm = mm;
        }
        if let Some(mm) = tag.get("margin-left").and_then(length_mm) {
            margins.left_mm = mm;
        }
        PageLayout {
            width_mm: tag.get("page-width").and_then(length_mm),
            height_mm: tag.get("page-height").and_then(length_mm),
            margins,
        }
    }
}

// ---------------------------------------------------------------------------
// The body: paragraphs, spans, lists and tables
// ---------------------------------------------------------------------------

/// Finds `office:text` and reads what is in it. Everything before it —
/// `office:automatic-styles`, `office:master-styles`, the font declarations —
/// has already been read by [`StyleTable::collect`], so it is skipped here
/// simply by not matching anything until the body is reached.
fn walk_body(source: &str, table: &StyleTable, sheet: &mut Sheet) -> Vec<Block> {
    let mut reader = xml::Reader::new(source);
    while let Some(event) = reader.next() {
        let xml::Event::Start(tag) = event else {
            continue;
        };
        if tag.name == "text" && tag.prefix == "office" {
            return if tag.empty {
                Vec::new()
            } else {
                read_blocks(&mut reader, table, sheet, "text")
            };
        }
    }
    Vec::new()
}

/// Reads the blocks directly inside `office:text`, a table cell, a list item
/// or a section, until `closing` ends. Shared by all four, because a cell's
/// contents are "paragraphs and possibly nested tables" in exactly the same
/// sense the body's are.
fn read_blocks(
    reader: &mut xml::Reader,
    table: &StyleTable,
    sheet: &mut Sheet,
    closing: &str,
) -> Vec<Block> {
    let mut blocks = Vec::new();
    while let Some(event) = reader.next() {
        match event {
            xml::Event::Start(tag) if tag.name == "p" => {
                blocks.push(Block::Para(read_paragraph(reader, table, sheet, &tag)));
            }
            xml::Event::Start(tag) if tag.name == "h" => {
                blocks.push(Block::Para(read_heading(reader, table, sheet, &tag)));
            }
            xml::Event::Start(tag) if tag.name == "list" => {
                if !tag.empty {
                    read_list(reader, table, sheet, &tag, 1, &mut blocks);
                }
            }
            xml::Event::Start(tag) if tag.name == "table" => {
                blocks.push(Block::Table(read_table(reader, table, sheet, &tag)));
            }
            xml::Event::Start(tag) if tag.name == "frame" => {
                // A frame is not necessarily a picture. It is also how a text
                // box is written — including by Onionskin's own `.odt` writer,
                // which puts every line of a scanned page in one — so throwing
                // the frame away throws away the whole document. What is in it
                // decides.
                if !tag.empty {
                    let pieces = read_frame(reader, table, sheet, Style::default());
                    if !pieces.is_empty() {
                        blocks.push(Block::Para(Para {
                            pieces,
                            ..Para::default()
                        }));
                    }
                }
            }
            xml::Event::Start(tag) if tag.name == "image" => {
                sheet.note("This document has images in it. Onionskin leaves them out.");
                if !tag.empty {
                    reader.skip_element(&tag.name);
                }
            }
            xml::Event::Start(tag) if tag.name == "section" => {
                note_if_columned(table, sheet, &tag);
                if !tag.empty {
                    blocks.extend(read_blocks(reader, table, sheet, "section"));
                }
            }
            xml::Event::Start(tag) if tag.name == "tracked-changes" => {
                if !tag.empty {
                    reader.skip_element("tracked-changes");
                }
            }
            xml::Event::Start(tag) if tag.name == "annotation" => {
                if !tag.empty {
                    reader.skip_element("annotation");
                }
            }
            xml::Event::End(name) if name == closing => break,
            _ => {}
        }
    }
    blocks
}

fn note_if_columned(table: &StyleTable, sheet: &mut Sheet, tag: &xml::Tag) {
    let columns = tag
        .get("style-name")
        .and_then(|name| table.styles.get(name))
        .and_then(|def| def.column_count)
        .unwrap_or(1);
    if columns > 1 {
        sheet.note(
            "This document lays part of the text out in newspaper-style columns. \
             Onionskin sets it all in a single column instead.",
        );
    }
}

fn read_paragraph(
    reader: &mut xml::Reader,
    table: &StyleTable,
    sheet: &mut Sheet,
    tag: &xml::Tag,
) -> Para {
    let (style, merged) = table.resolve_paragraph(tag.get("style-name"), Style::default());
    let mut para = build_para_shell(&merged, style);
    if !tag.empty {
        para.pieces = read_pieces(reader, table, sheet, style, "p");
    }
    para
}

fn read_heading(
    reader: &mut xml::Reader,
    table: &StyleTable,
    sheet: &mut Sheet,
    tag: &xml::Tag,
) -> Para {
    let level = tag
        .number("outline-level")
        .map(|value| value as usize)
        .unwrap_or(1)
        .clamp(1, 6);
    let (style, merged) = table.resolve_paragraph(tag.get("style-name"), heading_seed(level));
    let mut para = build_para_shell(&merged, style);
    if !tag.empty {
        para.pieces = read_pieces(reader, table, sheet, style, "h");
    }
    para
}

/// The size and weight a heading gets when nothing between its own style and
/// the document's default says otherwise — which happens more often than one
/// might expect, since a document that never touches the "Heading 1" style at
/// all still marks the paragraph with `text:outline-level` alone.
fn heading_seed(level: usize) -> Style {
    let size_pt = match level {
        1 => 24.0,
        2 => 18.0,
        3 => 14.0,
        4 => 12.0,
        5 => 11.0,
        _ => 10.0,
    };
    Style {
        size_pt,
        bold: true,
        ..Style::default()
    }
}

/// Builds a paragraph's shape — everything but its pieces — from a resolved
/// style chain. Line height needs the paragraph's own resolved type size to
/// turn an absolute length into a multiple of it, which is why this runs
/// after the style chain is folded rather than as part of folding it.
fn build_para_shell(merged: &MergedPara, own_style: Style) -> Para {
    let mut para = Para {
        style: own_style,
        ..Para::default()
    };
    if let Some(align) = merged.align {
        para.align = align;
    }
    if let Some(mm) = merged.space_before_mm {
        para.space_before_mm = mm;
    }
    if let Some(mm) = merged.space_after_mm {
        para.space_after_mm = mm;
    }
    if let Some(mm) = merged.indent_left_mm {
        para.indent_left_mm = mm;
    }
    if let Some(mm) = merged.indent_right_mm {
        para.indent_right_mm = mm;
    }
    if let Some(mm) = merged.first_line_mm {
        para.first_line_mm = mm;
    }
    if let Some(line_height) = merged.line_height {
        para.line_spacing = match line_height {
            LineHeight::Percent(percent) => percent / 100.0,
            LineHeight::Absolute(mm) => mm_to_pt(mm) / own_style.size_pt.max(1.0),
        };
    }
    if let Some(break_before) = merged.break_before {
        para.break_before = break_before;
    }
    para
}

/// Reads the runs of text directly inside a paragraph, heading, span or
/// hyperlink, until `closing` ends.
fn read_pieces(
    reader: &mut xml::Reader,
    table: &StyleTable,
    sheet: &mut Sheet,
    base: Style,
    closing: &str,
) -> Vec<Piece> {
    let mut pieces = Vec::new();
    while let Some(event) = reader.next() {
        match event {
            xml::Event::Text(text) => {
                if !text.is_empty() {
                    pieces.push(Piece::Text(text, base));
                }
            }
            xml::Event::Start(tag) if tag.name == "span" || tag.name == "a" => {
                // A hyperlink can carry its own `text:style-name` too, so it
                // is resolved exactly as a span is; if it has none, `base`
                // passes through unchanged.
                let style = table.resolve_span(base, tag.get("style-name"));
                if !tag.empty {
                    let name = tag.name.clone();
                    pieces.extend(read_pieces(reader, table, sheet, style, &name));
                }
            }
            xml::Event::Start(tag) if tag.name == "line-break" => {
                pieces.push(Piece::LineBreak);
                if !tag.empty {
                    reader.skip_element("line-break");
                }
            }
            xml::Event::Start(tag) if tag.name == "tab" => {
                pieces.push(Piece::Tab);
                if !tag.empty {
                    reader.skip_element("tab");
                }
            }
            xml::Event::Start(tag) if tag.name == "s" => {
                let count = tag.number("c").map(|value| value as usize).unwrap_or(1);
                if count > 0 {
                    pieces.push(Piece::Text(" ".repeat(count), base));
                }
                if !tag.empty {
                    reader.skip_element("s");
                }
            }
            xml::Event::Start(tag) if tag.name == "note" => {
                sheet.note(
                    "This document has footnotes or endnotes in it. Onionskin leaves them out.",
                );
                if !tag.empty {
                    reader.skip_element("note");
                }
            }
            xml::Event::Start(tag) if tag.name == "frame" => {
                // A frame is not necessarily a picture. It is also how a text
                // box is written — including by Onionskin's own `.odt` writer,
                // which puts every line of a scanned page in one — so throwing
                // the frame away throws away the whole document. What is in it
                // decides.
                if !tag.empty {
                    pieces.extend(read_frame(reader, table, sheet, base));
                }
            }
            xml::Event::Start(tag) if tag.name == "image" => {
                sheet.note("This document has images in it. Onionskin leaves them out.");
                if !tag.empty {
                    reader.skip_element(&tag.name);
                }
            }
            xml::Event::Start(tag) if tag.name == "annotation" => {
                if !tag.empty {
                    reader.skip_element("annotation");
                }
            }
            xml::Event::Start(tag) if tag.name == "deletion" => {
                if !tag.empty {
                    reader.skip_element("deletion");
                }
            }
            xml::Event::Start(tag) => {
                // Nothing above matched: a bookmark, an index entry, a
                // tracked insertion, or anything else this reader has no
                // special handling for. Its own text is still text, so it is
                // descended into rather than thrown away — and for a
                // self-closing tag this simply meets the owed `End` straight
                // back and returns nothing.
                let name = tag.name.clone();
                pieces.extend(read_pieces(reader, table, sheet, base, &name));
            }
            xml::Event::End(name) if name == closing => break,
            _ => {}
        }
    }
    pieces
}

/// What is inside a `draw:frame`: the words of a text box, or an image that
/// is noted and left.
///
/// The text of a box is set in the flow rather than where the frame anchors
/// it. That is not where the document puts it, and it is much better than the
/// alternative — a page of text boxes read as an empty page.
fn read_frame(
    reader: &mut xml::Reader,
    table: &StyleTable,
    sheet: &mut Sheet,
    base: Style,
) -> Vec<Piece> {
    let mut pieces = Vec::new();
    while let Some(event) = reader.next() {
        match event {
            xml::Event::Start(tag) if tag.name == "image" || tag.name == "object" => {
                sheet.note("This document has images in it. Onionskin leaves them out.");
                if !tag.empty {
                    reader.skip_element(&tag.name);
                }
            }
            xml::Event::Start(tag) if tag.name == "p" || tag.name == "h" => {
                // Each paragraph of the box is a line of it. They cannot become
                // paragraphs of their own from here, so a break keeps them
                // apart rather than running them together into one sentence.
                if !pieces.is_empty() {
                    pieces.push(Piece::LineBreak);
                }
                if !tag.empty {
                    let style = table.resolve_span(base, tag.get("style-name"));
                    let name = tag.name.clone();
                    pieces.extend(read_pieces(reader, table, sheet, style, &name));
                }
            }
            xml::Event::End(name) if name == "frame" => break,
            _ => {}
        }
    }
    pieces
}

/// Walks a `text:list`'s items, giving each a marker and appending its
/// paragraphs (and any tables or nested lists) straight into `blocks` —
/// OpenDocument's body has no separate "list" block of its own, only
/// paragraphs that happen to carry a marker and an indent.
fn read_list(
    reader: &mut xml::Reader,
    table: &StyleTable,
    sheet: &mut Sheet,
    tag: &xml::Tag,
    level: u32,
    blocks: &mut Vec<Block>,
) {
    // A nested list commonly reuses its parent's `text:list-style`, with a
    // deeper `text:level` describing the deeper look — so the level counted
    // here doubles as the level looked up in that same style. A nested list
    // under a *different* style name would look up the wrong level; the
    // fallback to a bullet below means that mistake is never worse than a
    // list that looks like a plainer one, not lost or wrong content.
    let numbered = tag
        .get("style-name")
        .and_then(|name| table.lists.get(name))
        .and_then(|list| list.levels.get(&level))
        .is_some_and(|kind| *kind == ListKind::Number);

    let mut counter = 0u32;
    while let Some(event) = reader.next() {
        match event {
            xml::Event::Start(item) if item.name == "list-item" => {
                counter += 1;
                let marker = if numbered {
                    format!("{counter}.")
                } else {
                    "•".to_string()
                };
                if !item.empty {
                    read_list_item(reader, table, sheet, level, &marker, blocks);
                }
            }
            xml::Event::End(name) if name == "list" => break,
            _ => {}
        }
    }
}

/// Reads one list item: its first paragraph gets the marker and every
/// paragraph gets the nesting indent, so that a multi-paragraph item still
/// reads as one item and wrapped lines clear the marker rather than running
/// under it.
fn read_list_item(
    reader: &mut xml::Reader,
    table: &StyleTable,
    sheet: &mut Sheet,
    level: u32,
    marker: &str,
    blocks: &mut Vec<Block>,
) {
    let mut marked_yet = false;
    while let Some(event) = reader.next() {
        match event {
            xml::Event::Start(tag) if tag.name == "p" => {
                let mut para = read_paragraph(reader, table, sheet, &tag);
                mark_list_paragraph(&mut para, level, marker, &mut marked_yet);
                blocks.push(Block::Para(para));
            }
            xml::Event::Start(tag) if tag.name == "h" => {
                let mut para = read_heading(reader, table, sheet, &tag);
                mark_list_paragraph(&mut para, level, marker, &mut marked_yet);
                blocks.push(Block::Para(para));
            }
            xml::Event::Start(tag) if tag.name == "list" => {
                if !tag.empty {
                    read_list(reader, table, sheet, &tag, level + 1, blocks);
                }
            }
            xml::Event::Start(tag) if tag.name == "table" => {
                blocks.push(Block::Table(read_table(reader, table, sheet, &tag)));
            }
            xml::Event::End(name) if name == "list-item" => break,
            _ => {}
        }
    }
}

fn mark_list_paragraph(para: &mut Para, level: u32, marker: &str, marked_yet: &mut bool) {
    para.indent_left_mm += 8.0 * f64::from(level);
    if !*marked_yet {
        para.marker = Some(marker.to_string());
        para.first_line_mm = -6.0;
        *marked_yet = true;
    }
}

fn read_table(
    reader: &mut xml::Reader,
    table: &StyleTable,
    sheet: &mut Sheet,
    tag: &xml::Tag,
) -> Table {
    let mut result = Table::default();
    if tag.empty {
        return result;
    }
    // Column widths are collected provisionally and only kept if every column
    // has one — a table half-described in millimetres and half not is not
    // something a caller can lay out sensibly either way.
    let mut widths: Vec<Option<f64>> = Vec::new();
    let mut bordered = false;
    let mut padding: Option<f64> = None;
    while let Some(event) = reader.next() {
        match event {
            xml::Event::Start(column) if column.name == "table-column" => {
                let repeat = column
                    .number("number-columns-repeated")
                    .map(|value| value as usize)
                    .unwrap_or(1)
                    .max(1);
                let width = column
                    .get("style-name")
                    .and_then(|name| table.styles.get(name))
                    .and_then(|def| def.column_width_mm);
                for _ in 0..repeat {
                    widths.push(width);
                }
                if !column.empty {
                    reader.skip_element("table-column");
                }
            }
            xml::Event::Start(row) if row.name == "table-row" => {
                result.rows.push(read_row(
                    reader,
                    table,
                    sheet,
                    &row,
                    &mut bordered,
                    &mut padding,
                ));
            }
            xml::Event::End(name) if name == "table" => break,
            _ => {}
        }
    }
    if !widths.is_empty() && widths.iter().all(Option::is_some) {
        result.columns_mm = widths.into_iter().flatten().collect();
    }
    result.bordered = bordered;
    result.padding_mm = padding.map(|amount| (amount, amount));
    result
}

fn read_row(
    reader: &mut xml::Reader,
    table: &StyleTable,
    sheet: &mut Sheet,
    tag: &xml::Tag,
    bordered: &mut bool,
    padding_mm: &mut Option<f64>,
) -> Row {
    let mut row = Row::default();
    if tag.empty {
        return row;
    }
    while let Some(event) = reader.next() {
        match event {
            xml::Event::Start(cell) if cell.name == "table-cell" => {
                row.cells
                    .push(read_cell(reader, table, sheet, &cell, bordered, padding_mm));
            }
            xml::Event::Start(covered) if covered.name == "covered-table-cell" => {
                // Hidden by an earlier cell's span: it has nothing of its own
                // to show, so it is skipped rather than read as an empty cell
                // that was never there.
                if !covered.empty {
                    reader.skip_element("covered-table-cell");
                }
            }
            xml::Event::End(name) if name == "table-row" => break,
            _ => {}
        }
    }
    row
}

fn read_cell(
    reader: &mut xml::Reader,
    table: &StyleTable,
    sheet: &mut Sheet,
    tag: &xml::Tag,
    bordered: &mut bool,
    padding_mm: &mut Option<f64>,
) -> Cell {
    let span = tag
        .number("number-columns-spanned")
        .map(|value| value as usize)
        .unwrap_or(1)
        .max(1);
    let style = tag
        .get("style-name")
        .and_then(|name| table.styles.get(name));
    if style.is_some_and(|def| def.bordered) {
        *bordered = true;
    }
    // The first cell that says how much room it leaves round its words speaks
    // for the table: a document that sets it per cell is beyond what this can
    // follow, and one number is far nearer than a guess.
    if padding_mm.is_none() {
        *padding_mm = style.and_then(|def| def.padding_mm);
    }
    let blocks = if tag.empty {
        Vec::new()
    } else {
        read_blocks(reader, table, sheet, "table-cell")
    };
    Cell { blocks, span }
}

// ---------------------------------------------------------------------------
// Small conversions
// ---------------------------------------------------------------------------

/// A length as OpenDocument writes one — `21cm`, `210mm`, `8.5in`, `12pt`,
/// `1pc`, `1px` — turned into millimetres. A bare number with no unit is
/// treated as millimetres, which is what a hand-edited file most often means
/// by one.
fn length_mm(text: &str) -> Option<f64> {
    let trimmed = text.trim();
    let split_at = trimmed
        .find(|ch: char| !(ch.is_ascii_digit() || ch == '.' || ch == '-' || ch == '+'))
        .unwrap_or(trimmed.len());
    let (number, unit) = trimmed.split_at(split_at);
    let value: f64 = number.parse().ok()?;
    let mm = match unit.trim().to_ascii_lowercase().as_str() {
        "" | "mm" => value,
        "cm" => value * 10.0,
        "in" => value * 25.4,
        "pt" => value * 25.4 / 72.0,
        "pc" => value * 25.4 / 6.0,
        "px" => value * 25.4 / 96.0,
        _ => return None,
    };
    Some(mm)
}

/// `fo:font-size`, which is either a length or a percentage of whatever the
/// parent style resolves to — the same attribute holds both, distinguished
/// only by a trailing `%`.
fn parse_size(text: &str) -> Option<SizeSpec> {
    let trimmed = text.trim();
    if let Some(number) = trimmed.strip_suffix('%') {
        return number.trim().parse().ok().map(SizeSpec::Percent);
    }
    length_mm(trimmed).map(|mm| SizeSpec::Pt(mm_to_pt(mm)))
}

fn parse_align(text: &str) -> Option<Align> {
    match text.trim() {
        "start" | "left" => Some(Align::Left),
        "center" => Some(Align::Centre),
        "end" | "right" => Some(Align::Right),
        "justify" => Some(Align::Justify),
        _ => None,
    }
}

fn parse_line_height(text: &str) -> Option<LineHeight> {
    let trimmed = text.trim();
    if let Some(number) = trimmed.strip_suffix('%') {
        return number.trim().parse().ok().map(LineHeight::Percent);
    }
    length_mm(trimmed).map(LineHeight::Absolute)
}

fn parse_hex_colour(text: &str) -> Option<(f64, f64, f64)> {
    let hex = text.trim().trim_start_matches('#');
    if hex.len() != 6 || !hex.chars().all(|ch| ch.is_ascii_hexdigit()) {
        return None;
    }
    let component = |at: usize| -> Option<f64> {
        u8::from_str_radix(&hex[at..at + 2], 16)
            .ok()
            .map(|value| f64::from(value) / 255.0)
    };
    Some((component(0)?, component(2)?, component(4)?))
}

/// Whether a `style:table-cell-properties` tag draws a rule on any side —
/// checked as one question because [`Table::bordered`] is one flag for the
/// whole table, set the moment a single cell asks for it.
fn has_visible_border(tag: &xml::Tag) -> bool {
    [
        "border",
        "border-top",
        "border-right",
        "border-bottom",
        "border-left",
    ]
    .into_iter()
    .any(|name| tag.get(name).is_some_and(|value| value.trim() != "none"))
}

#[cfg(test)]
mod tests;
