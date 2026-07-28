//! A document you can make from nothing and keep editing.
//!
//! Onionskin's other two workflows both start with something that already
//! exists: a scan of a printed sheet, or a pair of Word files. This one starts
//! with a blank page. You say how big the paper is, you put words on it, you
//! print it — and then, crucially, you carry on editing and print *only what
//! you added*, onto the same sheet.
//!
//! That last step is the whole point of the program, and here it is exact.
//! There is no rendering, no diffing of pixels and no guessing: the document
//! remembers precisely which words were on the sheet when it was printed, so
//! the delta is the words that were not. Nothing can drift by half a
//! millimetre because nothing is measured.
//!
//! It also cannot reflow. Every piece of text sits at a millimetre you chose,
//! so inserting a paragraph in the middle does not push anything down the page
//! — the failure that makes the compare-two-documents workflow refuse to print
//! simply has no way to happen here. What *can* happen is that you move or
//! delete something already printed, and toner does not come off paper. The
//! document notices that and says so.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::font::EmbeddedFont;
use crate::geometry::PageSize;
use crate::pdf::{Font, LineFont, PlacedLine};

/// One piece of text on the page.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Item {
    /// Stable across edits, so a printed sheet can be matched against the
    /// document it came from even after other items have been added or removed.
    pub id: u32,
    /// Which page, counted from 1 the way a person counts them.
    pub page: usize,
    /// Millimetres from the left edge of the paper.
    pub x_mm: f64,
    /// Millimetres down the paper to the text's baseline — where the letters
    /// sit, not the top of them.
    pub y_mm: f64,
    pub text: String,
    pub size_pt: f64,
    /// A built-in font's name, or `file` for the one supplied alongside.
    pub font: String,
    /// Wrap at this many millimetres. Without it the text stays on one line
    /// however long it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width_mm: Option<f64>,
    #[serde(default)]
    pub rotation_deg: f64,
    /// `#rrggbb`. Black unless you say otherwise, because this is going to a
    /// printer and most of them only have black.
    #[serde(default = "black")]
    pub colour: String,
    /// Space between lines, as a multiple of the type size.
    #[serde(default = "default_leading")]
    pub leading: f64,
}

fn black() -> String {
    "#000000".to_string()
}
fn default_leading() -> f64 {
    1.2
}

impl Item {
    /// The lines this item puts on the page, wrapped if it has a width.
    pub fn lines(&self, font: Option<&EmbeddedFont>) -> Result<Vec<PlacedLine>, DocumentError> {
        let face = self.face()?;
        let colour = parse_colour(&self.colour)?;
        let step = self.size_pt * self.leading * MM_PER_PT;

        let mut placed = Vec::new();
        // Explicit line breaks always break; wrapping fills in between them.
        for (index, run) in self.wrapped(font)?.into_iter().enumerate() {
            placed.push(PlacedLine {
                text: run,
                x_mm: self.x_mm,
                y_mm: self.y_mm + index as f64 * step,
                size_pt: self.size_pt,
                font: face,
                rotation_deg: self.rotation_deg,
                colour,
            });
        }
        Ok(placed)
    }

    fn face(&self) -> Result<LineFont, DocumentError> {
        if self.font.eq_ignore_ascii_case("file") || self.font.eq_ignore_ascii_case("embedded") {
            return Ok(LineFont::Embedded);
        }
        Font::parse(&self.font)
            .map(LineFont::Builtin)
            .ok_or_else(|| {
                DocumentError::Invalid(format!(
                    "no font called {:?}. Run `onionskin fonts` for the list, or use \
                     'file' with --font-file.",
                    self.font
                ))
            })
    }

    /// Split the text into the lines it will actually be set as.
    fn wrapped(&self, font: Option<&EmbeddedFont>) -> Result<Vec<String>, DocumentError> {
        let paragraphs: Vec<&str> = self.text.split('\n').collect();
        let Some(width_mm) = self.width_mm else {
            return Ok(paragraphs.into_iter().map(str::to_string).collect());
        };
        if width_mm <= 0.0 {
            return Err(DocumentError::Invalid(
                "a wrapping width must be more than nothing".into(),
            ));
        }

        let measure = self.measurer(font)?;
        let mut out = Vec::new();
        for paragraph in paragraphs {
            let mut line = String::new();
            for word in paragraph.split(' ') {
                if line.is_empty() {
                    line.push_str(word);
                    continue;
                }
                let candidate = format!("{line} {word}");
                if measure(&candidate)? <= width_mm {
                    line = candidate;
                } else {
                    out.push(std::mem::take(&mut line));
                    line.push_str(word);
                }
            }
            // A paragraph that is entirely empty is still a blank line, and
            // dropping it would silently close up the space someone left.
            out.push(line);
        }
        Ok(out)
    }

    /// How wide a run of text will be, in millimetres.
    ///
    /// Two quite different sources, and both are exact. A supplied font is
    /// measured from its own outlines; a built-in one from the widths Adobe
    /// published, which every reader on every platform uses — so a line breaks
    /// in the same place here as it does on the printer.
    fn measurer<'a>(
        &'a self,
        font: Option<&'a EmbeddedFont>,
    ) -> Result<Measurer<'a>, DocumentError> {
        match self.face()? {
            LineFont::Embedded => {
                let font = font.ok_or(DocumentError::NoFont)?;
                let size = self.size_pt;
                Ok(Box::new(move |text: &str| {
                    font.width_mm(text, size)
                        .map_err(|e| DocumentError::Invalid(e.to_string()))
                }))
            }
            LineFont::Builtin(builtin) => {
                let size = self.size_pt;
                Ok(Box::new(move |text: &str| {
                    Ok(crate::pdf::builtin_width_mm(builtin, text, size))
                }))
            }
        }
    }
}

/// Something that can say how wide a run of text will be, in millimetres.
type Measurer<'a> = Box<dyn Fn(&str) -> Result<f64, DocumentError> + 'a>;

const MM_PER_PT: f64 = 25.4 / 72.0;

/// A drawing on the page: a line, a box, an ellipse, or a run of points.
///
/// Kept apart from [`Item`] rather than folded into it because the two share
/// almost nothing — a shape has no font, no size in points, no text to wrap —
/// and an enum of the two would be a struct of mostly-empty fields wherever it
/// was touched. What they do share is the discipline: a stable id, a page, and
/// a place on that page in millimetres.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Shape {
    pub id: u32,
    pub page: usize,
    pub kind: ShapeKind,
    /// The outline's colour as `#rrggbb`, or none for no outline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<String>,
    /// The inside's colour as `#rrggbb`, or none to leave it hollow.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<String>,
    /// Outline thickness in millimetres.
    pub width_mm: f64,
    /// Dash length and gap in millimetres.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dash_mm: Option<(f64, f64)>,
}

/// What the shape is, and where. All measurements are millimetres from the
/// top-left corner of the paper.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "shape", rename_all = "lowercase")]
pub enum ShapeKind {
    Line {
        x1_mm: f64,
        y1_mm: f64,
        x2_mm: f64,
        y2_mm: f64,
    },
    Rect {
        x_mm: f64,
        y_mm: f64,
        width_mm: f64,
        height_mm: f64,
        #[serde(default)]
        radius_mm: f64,
    },
    Ellipse {
        x_mm: f64,
        y_mm: f64,
        radius_x_mm: f64,
        radius_y_mm: f64,
    },
    Path {
        points: Vec<(f64, f64)>,
        #[serde(default)]
        closed: bool,
    },
}

/// A page of paper, and everything written on it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Document {
    /// The paper this is written for. Everything is placed against it.
    pub page: PageSize,
    /// How many sheets. Kept explicit so a blank page can exist on purpose.
    pub pages: usize,
    pub items: Vec<Item>,
    /// The drawings. Empty in every document written before there were any,
    /// which is why it defaults rather than being required.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shapes: Vec<Shape>,
    /// What was on the sheets the last time this was printed, if it has been.
    /// This is the whole basis of the delta: not a guess, a record.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub printed: Option<Vec<Item>>,
    /// And which drawings were on them. Held separately from `printed` so an
    /// older document, which has one and not the other, still loads.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub printed_shapes: Option<Vec<Shape>>,
    /// The next id to hand out. Ids are never reused, so a printed record
    /// always refers to the same piece of text.
    #[serde(default)]
    next_id: u32,
}

#[derive(Debug, thiserror::Error)]
pub enum DocumentError {
    #[error("no document at {0}")]
    Missing(std::path::PathBuf),
    #[error("could not read {path}: {source}")]
    Io {
        path: std::path::PathBuf,
        source: std::io::Error,
    },
    #[error(
        "{path} is damaged — it is an Onionskin document, but it will not \
             read: {source}"
    )]
    Malformed {
        path: std::path::PathBuf,
        source: serde_json::Error,
    },
    /// A file that was never one of ours, handed to a command that edits ours.
    ///
    /// Kept apart from `Malformed` because the two need opposite answers. A
    /// damaged document wants the parser's complaint; somebody who has handed
    /// over their own PDF wants to be told which command they meant, and
    /// "expected value at line 1 column 1" tells them nothing at all.
    #[error("{path} is {kind}, not an Onionskin document.\n{advice}")]
    NotOurs {
        path: std::path::PathBuf,
        kind: String,
        advice: String,
    },
    #[error("{0}")]
    Invalid(String),
    #[error("no item numbered {0} — run `onionskin show` to see what is there")]
    NoSuchItem(u32),
    #[error("this text is set in the supplied font, but no --font-file was given")]
    NoFont,
}

impl Document {
    /// A blank document, ready to be written on.
    pub fn blank(page: PageSize, pages: usize) -> Document {
        Document {
            page,
            pages: pages.max(1),
            items: Vec::new(),
            shapes: Vec::new(),
            printed: None,
            printed_shapes: None,
            next_id: 1,
        }
    }

    /// Is this file one of Onionskin's own documents?
    ///
    /// By what is in it, not by what it is called. Somebody who names their
    /// document `letter.pdf` has made a document called `letter.pdf`, and
    /// every command that decides by the extension would otherwise open it
    /// expecting a PDF, fail, and report that the file is damaged — about a
    /// file Onionskin wrote itself, one command earlier.
    ///
    /// A document is JSON, so it opens with `{`. A PDF opens with `%PDF`, a
    /// Word or OpenDocument file with `PK`, and every image with a magic
    /// number of its own. None of them can collide with this. The file is not
    /// parsed: one that starts with `{` is *meant* to be a document, and if it
    /// is broken the right complaint is that the document is broken, not that
    /// the PDF is.
    pub fn is_one(path: &Path) -> bool {
        use std::io::Read;
        let Ok(mut file) = std::fs::File::open(path) else {
            return false;
        };
        let mut start = [0u8; 64];
        let Ok(read) = file.read(&mut start) else {
            return false;
        };
        start[..read]
            .iter()
            .find(|byte| !byte.is_ascii_whitespace())
            .is_some_and(|byte| *byte == b'{')
    }

    pub fn load(path: &Path) -> Result<Document, DocumentError> {
        if !path.is_file() {
            return Err(DocumentError::Missing(path.to_path_buf()));
        }
        if !Document::is_one(path) {
            let (kind, advice) = what_it_looks_like(path);
            return Err(DocumentError::NotOurs {
                path: path.to_path_buf(),
                kind: kind.to_string(),
                advice: advice.to_string(),
            });
        }
        let text = std::fs::read_to_string(path).map_err(|source| DocumentError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let mut document: Document =
            serde_json::from_str(&text).map_err(|source| DocumentError::Malformed {
                path: path.to_path_buf(),
                source,
            })?;
        document.check()?;
        // A document written by hand may not have set this, and handing out an
        // id that is already in use would quietly merge two pieces of text.
        let highest = document.items.iter().map(|i| i.id).max().unwrap_or(0);
        document.next_id = document.next_id.max(highest + 1);
        Ok(document)
    }

    /// Write the document out, without disturbing the old one if this fails.
    ///
    /// Straight to the destination would truncate the file first, so a full
    /// disk or a power cut between the two would leave an empty document where
    /// someone's work used to be.
    pub fn save(&self, path: &Path) -> Result<(), DocumentError> {
        keep_the_last_one(path);
        self.save_without_keeping(path)
    }

    /// The same, without setting anything aside to go back to.
    ///
    /// For [`undo`] itself, which would otherwise make the state it is undoing
    /// into the next thing to undo, and leave somebody toggling between two
    /// versions of their document forever.
    pub fn save_without_keeping(&self, path: &Path) -> Result<(), DocumentError> {
        let text = serde_json::to_string_pretty(self)
            .map_err(|e| DocumentError::Invalid(e.to_string()))?;
        let temporary = path.with_extension("onionskin-tmp");
        std::fs::write(&temporary, text.as_bytes()).map_err(|source| DocumentError::Io {
            path: temporary.clone(),
            source,
        })?;
        std::fs::rename(&temporary, path).map_err(|source| {
            let _ = std::fs::remove_file(&temporary);
            DocumentError::Io {
                path: path.to_path_buf(),
                source,
            }
        })
    }

    fn check(&self) -> Result<(), DocumentError> {
        if !(self.page.width_mm.is_finite() && self.page.height_mm.is_finite())
            || self.page.width_mm <= 0.0
            || self.page.height_mm <= 0.0
        {
            return Err(DocumentError::Invalid(
                "the page size is not a size of paper".into(),
            ));
        }
        if self.pages == 0 {
            return Err(DocumentError::Invalid(
                "a document has at least one page".into(),
            ));
        }
        for item in &self.items {
            if !(item.x_mm.is_finite() && item.y_mm.is_finite() && item.size_pt.is_finite()) {
                return Err(DocumentError::Invalid(format!(
                    "item {} is placed at a position that is not a number",
                    item.id
                )));
            }
            if item.size_pt <= 0.0 {
                return Err(DocumentError::Invalid(format!(
                    "item {} is set at {} pt, which cannot be printed",
                    item.id, item.size_pt
                )));
            }
            if item.page == 0 || item.page > self.pages {
                return Err(DocumentError::Invalid(format!(
                    "item {} is on page {}, and the document has {} pages",
                    item.id, item.page, self.pages
                )));
            }
        }
        for shape in &self.shapes {
            if shape.page == 0 || shape.page > self.pages {
                return Err(DocumentError::Invalid(format!(
                    "drawing {} is on page {}, and the document has {} pages",
                    shape.id, shape.page, self.pages
                )));
            }
            if !shape.width_mm.is_finite() || shape.width_mm < 0.0 {
                return Err(DocumentError::Invalid(format!(
                    "drawing {} has a line width of {}, which is not a width",
                    shape.id, shape.width_mm
                )));
            }
            if shape.stroke.is_none() && shape.fill.is_none() {
                return Err(DocumentError::Invalid(format!(
                    "drawing {} has neither an outline nor a fill, so nothing \
                     would appear on the paper",
                    shape.id
                )));
            }
            for colour in [shape.stroke.as_deref(), shape.fill.as_deref()]
                .into_iter()
                .flatten()
            {
                parse_colour(colour)?;
            }
            let (x0, y0, x1, y1) = shape.bounds();
            if ![x0, y0, x1, y1].iter().all(|v| v.is_finite()) {
                return Err(DocumentError::Invalid(format!(
                    "drawing {} is placed at a position that is not a number",
                    shape.id
                )));
            }
            if let ShapeKind::Path { points, .. } = &shape.kind {
                if points.len() < 2 {
                    return Err(DocumentError::Invalid(format!(
                        "drawing {} is a path of {} point(s); it takes two to \
                         draw a line",
                        shape.id,
                        points.len()
                    )));
                }
            }
        }
        Ok(())
    }

    /// Put a new piece of text on the page. Returns its number.
    pub fn add(&mut self, mut item: Item) -> Result<u32, DocumentError> {
        item.id = self.next_id;
        self.next_id += 1;
        if item.page == 0 {
            item.page = 1;
        }
        // Growing the document to fit is friendlier than refusing: someone who
        // asks for page 3 of a one-page document means to have three pages.
        self.pages = self.pages.max(item.page);
        let id = item.id;
        self.items.push(item);
        self.check()?;
        Ok(id)
    }

    pub fn get(&self, id: u32) -> Option<&Item> {
        self.items.iter().find(|item| item.id == id)
    }

    pub fn get_mut(&mut self, id: u32) -> Result<&mut Item, DocumentError> {
        self.items
            .iter_mut()
            .find(|item| item.id == id)
            .ok_or(DocumentError::NoSuchItem(id))
    }

    pub fn remove(&mut self, id: u32) -> Result<Item, DocumentError> {
        let at = self
            .items
            .iter()
            .position(|item| item.id == id)
            .ok_or(DocumentError::NoSuchItem(id))?;
        Ok(self.items.remove(at))
    }

    /// The items on one page, in the order they were added.
    pub fn on_page(&self, page: usize) -> impl Iterator<Item = &Item> {
        self.items.iter().filter(move |item| item.page == page)
    }

    /// Lay the whole document out, page by page, ready to be written.
    pub fn layout(
        &self,
        font: Option<&EmbeddedFont>,
    ) -> Result<Vec<Vec<PlacedLine>>, DocumentError> {
        self.layout_of(&self.items, font)
    }

    fn layout_of(
        &self,
        items: &[Item],
        font: Option<&EmbeddedFont>,
    ) -> Result<Vec<Vec<PlacedLine>>, DocumentError> {
        let mut pages: Vec<Vec<PlacedLine>> = vec![Vec::new(); self.pages];
        for item in items {
            let index = item.page.saturating_sub(1);
            if index >= pages.len() {
                continue;
            }
            pages[index].extend(item.lines(font)?);
        }
        Ok(pages)
    }

    pub fn page_sizes(&self) -> Vec<PageSize> {
        vec![self.page; self.pages]
    }

    /// Note that the document as it stands is now on paper.
    pub fn mark_printed(&mut self) {
        self.printed = Some(self.items.clone());
        self.printed_shapes = Some(self.shapes.clone());
    }

    pub fn has_been_printed(&self) -> bool {
        self.printed.is_some()
    }

    /// What has been added since the sheet was printed.
    pub fn added_since_printing(&self) -> Vec<&Item> {
        let Some(printed) = &self.printed else {
            return self.items.iter().collect();
        };
        self.items
            .iter()
            .filter(|item| !printed.iter().any(|was| was == *item))
            .collect()
    }

    /// Which drawings have been added since the sheet was printed.
    ///
    /// A document written before Onionskin could draw has `printed` recorded
    /// and `printed_shapes` missing. That is not "no drawings were printed" —
    /// it is "nothing was asked". Since such a document has no shapes either,
    /// the two readings agree, and treating a missing record as an empty one is
    /// safe. It stops being safe the moment a shape is added, and by then the
    /// record has been written.
    pub fn shapes_added_since_printing(&self) -> Vec<&Shape> {
        let Some(printed) = &self.printed_shapes else {
            if self.printed.is_some() {
                // Printed before there were drawings: anything present now is
                // new, which is exactly what an empty record would say.
                return self.shapes.iter().collect();
            }
            return self.shapes.iter().collect();
        };
        self.shapes
            .iter()
            .filter(|shape| !printed.iter().any(|was| was == *shape))
            .collect()
    }

    /// Put a drawing on the page, and give it its number.
    pub fn draw(&mut self, mut shape: Shape) -> Result<u32, DocumentError> {
        shape.id = self.next_id;
        self.next_id += 1;
        if shape.page == 0 {
            shape.page = 1;
        }
        self.pages = self.pages.max(shape.page);
        let id = shape.id;
        self.shapes.push(shape);
        self.check()?;
        Ok(id)
    }

    /// Take a drawing off the page.
    pub fn erase_shape(&mut self, id: u32) -> Result<Shape, DocumentError> {
        let at = self
            .shapes
            .iter()
            .position(|shape| shape.id == id)
            .ok_or(DocumentError::NoSuchItem(id))?;
        Ok(self.shapes.remove(at))
    }

    /// The drawings on each page, ready for the PDF writer.
    pub fn shape_layout(&self, shapes: &[&Shape]) -> Vec<Vec<crate::pdf::PlacedShape>> {
        let mut per_page: Vec<Vec<crate::pdf::PlacedShape>> = vec![Vec::new(); self.pages];
        for shape in shapes {
            let index = shape.page.saturating_sub(1);
            if index < per_page.len() {
                per_page[index].push(shape.placed());
            }
        }
        per_page
    }
}

/// Where the version before the last change is kept.
/// What a file that is not one of ours appears to be, and what to do with it.
///
/// Read from the first few bytes rather than the name, for the same reason
/// [`Document::is_one`] is: the name is a label somebody chose and the magic
/// number is what the file actually is.
fn what_it_looks_like(path: &Path) -> (&'static str, &'static str) {
    let mut start = [0u8; 8];
    let read = std::fs::File::open(path)
        .and_then(|mut file| {
            use std::io::Read;
            file.read(&mut start)
        })
        .unwrap_or(0);
    let head = &start[..read];

    const ADVICE_FOR_A_PAGE: &str = concat!(
        "    Put words on the printed sheet:  onionskin write <file> ",
        "--at '25,40:words'\n",
        "    Or compare two versions of it:   onionskin delta <before> <after>",
    );

    if head.starts_with(b"%PDF") {
        ("a PDF", ADVICE_FOR_A_PAGE)
    } else if head.starts_with(b"PK\x03\x04") {
        ("a Word or OpenDocument file", ADVICE_FOR_A_PAGE)
    } else if head.starts_with(b"\x89PNG") || head.starts_with(&[0xff, 0xd8, 0xff]) {
        (
            "an image",
            "    Write on the scanned sheet:  onionskin add <image> --at-mm '45,63:words'",
        )
    } else {
        (
            "not a document Onionskin can open",
            "    Make one:  onionskin new <name>.onionskin",
        )
    }
}

/// How many steps back a document remembers.
///
/// Ten, because the mistake somebody wants undone is nearly always the last
/// one or the one before it, and a folder holding fifty copies of a letter is
/// its own kind of mess. Each one is the whole document, but a document is a
/// few kilobytes of JSON — ten of them is smaller than one page of the PDF it
/// prints to.
pub const STEPS_KEPT: usize = 10;

/// The nth step back, counting from 1. `<name>.before`, `<name>.before2`, …
///
/// Numbered files beside the document rather than a hidden store, so that
/// somebody who wants to know what Onionskin is keeping can see it, and
/// somebody who wants it gone can delete it.
fn previous(path: &Path) -> PathBuf {
    step_file(path, ".before", 1)
}

/// The nth step forward, for redo.
fn next_step(path: &Path, step: usize) -> PathBuf {
    step_file(path, ".after", step)
}

fn step_file(path: &Path, suffix: &str, step: usize) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    if step > 1 {
        name.push(step.to_string());
    }
    path.with_file_name(name)
}

/// Move every kept step one further away, so step 1 is free for a new one.
///
/// The oldest falls off the end. Failures are ignored throughout: a history
/// that cannot be kept is not a reason to refuse to save the document, which
/// is the thing somebody actually asked for.
fn rotate_away(path: &Path, suffix: &str) {
    for step in (1..STEPS_KEPT).rev() {
        let from = step_file(path, suffix, step);
        if from.is_file() {
            let _ = std::fs::rename(&from, step_file(path, suffix, step + 1));
        }
    }
}

/// Move every kept step one nearer, after step 1 has been taken.
fn rotate_towards(path: &Path, suffix: &str) {
    for step in 2..=STEPS_KEPT {
        let from = step_file(path, suffix, step);
        if from.is_file() {
            let _ = std::fs::rename(&from, step_file(path, suffix, step - 1));
        }
    }
}

/// Throw away everything that could be redone.
///
/// Called when the document changes: once a new edit is made, the versions
/// that were undone are no longer anywhere the document could get back to,
/// and offering to "redo" into a history that has been departed from would
/// hand somebody a document that never existed.
fn forget_the_redos(path: &Path) {
    for step in 1..=STEPS_KEPT {
        let _ = std::fs::remove_file(next_step(path, step));
    }
}

/// Set the current version aside before overwriting it.
///
/// `erase` takes a piece of text off a page and there was no way back from it.
/// Nor from an `edit` that replaced the wrong item, nor a `write` at the wrong
/// millimetre. One copy is enough: somebody who has just done the wrong thing
/// wants the last thing undone, and a program that quietly filled a folder with
/// a hundred versions of their document would be solving a different problem
/// badly.
///
/// Failures are ignored on purpose. A read-only folder, a full disk, a file
/// that is not there yet — none is a reason to refuse to save the work somebody
/// just did. Losing the ability to undo is a smaller harm than losing the
/// change.
fn keep_the_last_one(path: &Path) {
    if !path.is_file() {
        return;
    }
    rotate_away(path, ".before");
    let _ = std::fs::copy(path, previous(path));
    forget_the_redos(path);
}

/// Is there a version to go back to?
pub fn can_undo(path: &Path) -> bool {
    previous(path).is_file()
}

/// Put the document back as it was before the last change.
///
/// Can be asked as many times as there are steps kept — see [`STEPS_KEPT`].
/// Coming forward again is [`redo`], which is a different word because it is
/// a different thing.
pub fn undo(path: &Path) -> Result<(), DocumentError> {
    let before = previous(path);
    if !before.is_file() {
        return Err(DocumentError::Invalid(format!(
            "there is nothing to undo for {} — it has not been changed since \
             it was opened.",
            path.display()
        )));
    }
    // Read before either is written, so a failure part-way through leaves the
    // document as one of the two versions rather than as neither.
    let going_back = Document::load(&before)?;
    let current = Document::load(path)?;

    // What is being left becomes the first thing `redo` would return to.
    rotate_away(path, ".after");
    current.save_without_keeping(&next_step(path, 1))?;

    going_back.save_without_keeping(path)?;
    let _ = std::fs::remove_file(&before);
    rotate_towards(path, ".before");
    Ok(())
}

/// Put back a change that `undo` took away.
///
/// The other half of undo, and the reason undo no longer swaps. Swapping made
/// running it twice return you to where you started, so three mistakes could
/// not be undone at all — the second `undo` put the first one back. Going
/// back is going back now, however many times it is asked for, and coming
/// forward is a different word.
pub fn redo(path: &Path) -> Result<(), DocumentError> {
    let after = next_step(path, 1);
    if !after.is_file() {
        return Err(DocumentError::Invalid(format!(
            "there is nothing to redo for {} — nothing has been undone, or the \
             document has been changed since it was.",
            path.display()
        )));
    }
    let going_forward = Document::load(&after)?;
    let current = Document::load(path)?;

    rotate_away(path, ".before");
    current.save_without_keeping(&previous(path))?;

    going_forward.save_without_keeping(path)?;
    let _ = std::fs::remove_file(&after);
    rotate_towards(path, ".after");
    Ok(())
}

/// How many steps back this document can go.
pub fn steps_back(path: &Path) -> usize {
    (1..=STEPS_KEPT)
        .take_while(|step| step_file(path, ".before", *step).is_file())
        .count()
}

/// How many steps forward it can go.
pub fn steps_forward(path: &Path) -> usize {
    (1..=STEPS_KEPT)
        .take_while(|step| step_file(path, ".after", *step).is_file())
        .count()
}

impl Shape {
    /// Turn it into what the PDF writer draws.
    pub fn placed(&self) -> crate::pdf::PlacedShape {
        use crate::pdf::Drawing;
        let drawing = match &self.kind {
            ShapeKind::Line {
                x1_mm,
                y1_mm,
                x2_mm,
                y2_mm,
            } => Drawing::Line {
                from: (*x1_mm, *y1_mm),
                to: (*x2_mm, *y2_mm),
            },
            ShapeKind::Rect {
                x_mm,
                y_mm,
                width_mm,
                height_mm,
                radius_mm,
            } => Drawing::Rect {
                x_mm: *x_mm,
                y_mm: *y_mm,
                width_mm: *width_mm,
                height_mm: *height_mm,
                radius_mm: *radius_mm,
            },
            ShapeKind::Ellipse {
                x_mm,
                y_mm,
                radius_x_mm,
                radius_y_mm,
            } => Drawing::Ellipse {
                centre: (*x_mm, *y_mm),
                radius_x_mm: *radius_x_mm,
                radius_y_mm: *radius_y_mm,
            },
            ShapeKind::Path { points, closed } => Drawing::Path {
                points: points.clone(),
                closed: *closed,
            },
        };
        crate::pdf::PlacedShape {
            drawing,
            // A colour that will not parse is left off rather than guessed at.
            // `check` refuses the shape before it is ever stored, so reaching
            // here with a bad one means somebody built the struct by hand.
            stroke: self.stroke.as_deref().and_then(|c| parse_colour(c).ok()),
            fill: self.fill.as_deref().and_then(|c| parse_colour(c).ok()),
            width_mm: self.width_mm,
            dash_mm: self.dash_mm,
        }
    }

    /// The rectangle the drawing covers, for reporting where it sits.
    pub fn bounds(&self) -> (f64, f64, f64, f64) {
        let pad = self.width_mm / 2.0;
        let (x0, y0, x1, y1) = match &self.kind {
            ShapeKind::Line {
                x1_mm,
                y1_mm,
                x2_mm,
                y2_mm,
            } => (
                x1_mm.min(*x2_mm),
                y1_mm.min(*y2_mm),
                x1_mm.max(*x2_mm),
                y1_mm.max(*y2_mm),
            ),
            ShapeKind::Rect {
                x_mm,
                y_mm,
                width_mm,
                height_mm,
                ..
            } => (
                x_mm.min(x_mm + width_mm),
                y_mm.min(y_mm + height_mm),
                x_mm.max(x_mm + width_mm),
                y_mm.max(y_mm + height_mm),
            ),
            ShapeKind::Ellipse {
                x_mm,
                y_mm,
                radius_x_mm,
                radius_y_mm,
            } => (
                x_mm - radius_x_mm.abs(),
                y_mm - radius_y_mm.abs(),
                x_mm + radius_x_mm.abs(),
                y_mm + radius_y_mm.abs(),
            ),
            ShapeKind::Path { points, .. } => {
                let mut bounds = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
                for &(x, y) in points {
                    bounds.0 = bounds.0.min(x);
                    bounds.1 = bounds.1.min(y);
                    bounds.2 = bounds.2.max(x);
                    bounds.3 = bounds.3.max(y);
                }
                if points.is_empty() {
                    (0.0, 0.0, 0.0, 0.0)
                } else {
                    bounds
                }
            }
        };
        (x0 - pad, y0 - pad, x1 + pad, y1 + pad)
    }

    /// What it is, in a few words.
    pub fn describe(&self) -> String {
        match &self.kind {
            ShapeKind::Line { .. } => "line".into(),
            ShapeKind::Rect { radius_mm, .. } if *radius_mm > 0.0 => "rounded box".into(),
            ShapeKind::Rect { .. } => "box".into(),
            ShapeKind::Ellipse {
                radius_x_mm,
                radius_y_mm,
                ..
            } if (radius_x_mm - radius_y_mm).abs() < 1e-9 => "circle".into(),
            ShapeKind::Ellipse { .. } => "ellipse".into(),
            ShapeKind::Path { points, closed } => {
                let what = if *closed { "polygon" } else { "path" };
                format!("{what} of {} points", points.len())
            }
        }
    }
}

impl Document {
    /// The reasons an overlay cannot be printed onto the existing sheet.
    ///
    /// Ink does not come off paper. If a piece of text that was printed has
    /// since been moved, reworded or deleted, the sheet in your hand no longer
    /// says what the document says, and no amount of adding to it will fix
    /// that — the page has to be printed fresh. This is the same alarm the
    /// two-document workflow raises when it sees ink disappear, except that
    /// here it is certain rather than inferred.
    pub fn overlay_problems(&self) -> Vec<Problem> {
        let Some(printed) = &self.printed else {
            return Vec::new();
        };
        let mut problems = Vec::new();
        for was in printed {
            match self.get(was.id) {
                None => problems.push(Problem {
                    id: was.id,
                    page: was.page,
                    text: was.text.clone(),
                    what: Change::Deleted,
                }),
                Some(now) if now == was => {}
                Some(now) => {
                    let what = if now.text != was.text {
                        Change::Reworded
                    } else if now.page != was.page
                        || (now.x_mm - was.x_mm).abs() > 1e-9
                        || (now.y_mm - was.y_mm).abs() > 1e-9
                    {
                        Change::Moved
                    } else {
                        Change::Restyled
                    };
                    problems.push(Problem {
                        id: was.id,
                        page: was.page,
                        text: was.text.clone(),
                        what,
                    });
                }
            }
        }
        problems
    }

    /// Lay out only what would be added to the sheet already printed.
    pub fn delta_layout(
        &self,
        font: Option<&EmbeddedFont>,
    ) -> Result<Vec<Vec<PlacedLine>>, DocumentError> {
        let added: Vec<Item> = self.added_since_printing().into_iter().cloned().collect();
        self.layout_of(&added, font)
    }
}

/// Something printed that the document no longer agrees with.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Problem {
    pub id: u32,
    pub page: usize,
    pub text: String,
    pub what: Change,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Change {
    Moved,
    Reworded,
    Deleted,
    Restyled,
}

impl Problem {
    pub fn format(&self) -> String {
        let what = match self.what {
            Change::Moved => "has been moved",
            Change::Reworded => "has been reworded",
            Change::Deleted => "has been deleted",
            Change::Restyled => "has been restyled",
        };
        // On one line: this sits inside an indented block, and a line break in
        // someone's text would break the shape of the message it appears in.
        let flat: String = self.text.split_whitespace().collect::<Vec<_>>().join(" ");
        let mut shown: String = flat.chars().take(40).collect();
        if shown.chars().count() < flat.chars().count() {
            shown.push('…');
        }
        format!(
            "BLOCKER [page {}]: item {} {what}, and it is already on the sheet.\n    \
             \"{shown}\"\n    \
             Toner does not come off paper, so an overlay cannot undo it. Print \
             this page fresh.",
            self.page, self.id
        )
    }
}

/// A colour to the three numbers a PDF wants, each from 0 to 1.
///
/// Takes `#rrggbb`, the `#rgb` shorthand people know from the web, and a short
/// list of names. The names are there because somebody drawing a red box round
/// a paragraph should be able to type `red`, and because a person who has to
/// look up a hexadecimal triple to draw a line will not draw the line.
pub fn parse_colour(text: &str) -> Result<(f64, f64, f64), DocumentError> {
    let text = text.trim();
    let ink = |v: f64| (v, v, v);
    match text.to_ascii_lowercase().as_str() {
        "black" => return Ok(ink(0.0)),
        "white" => return Ok(ink(1.0)),
        "grey" | "gray" => return Ok(ink(0.5)),
        // A light grey, for shading a box behind text without hiding the text.
        "lightgrey" | "lightgray" => return Ok(ink(0.85)),
        "red" => return Ok((0.8, 0.0, 0.0)),
        "green" => return Ok((0.0, 0.5, 0.0)),
        "blue" => return Ok((0.0, 0.0, 0.8)),
        "yellow" => return Ok((1.0, 0.85, 0.0)),
        "orange" => return Ok((0.95, 0.5, 0.0)),
        _ => {}
    }

    let hex = text.trim_start_matches('#');
    if !hex.chars().all(|c| c.is_ascii_hexdigit()) || !matches!(hex.len(), 3 | 6) {
        return Err(DocumentError::Invalid(format!(
            "{text:?} is not a colour. Write it as #rrggbb, for example #000000 \
             for black — or use a name: black, white, grey, red, green, blue, \
             yellow, orange."
        )));
    }
    if hex.len() == 3 {
        // #abc means #aabbcc.
        let channel = |at: usize| -> f64 {
            let digit = hex[at..at + 1].chars().next().and_then(|c| c.to_digit(16));
            (digit.unwrap_or(0) * 17) as f64 / 255.0
        };
        return Ok((channel(0), channel(1), channel(2)));
    }
    let channel = |from: usize| -> f64 {
        u8::from_str_radix(&hex[from..from + 2], 16).unwrap_or(0) as f64 / 255.0
    };
    Ok((channel(0), channel(2), channel(4)))
}

#[cfg(test)]
mod tests;
