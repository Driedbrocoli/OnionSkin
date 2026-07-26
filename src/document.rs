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

use std::path::Path;

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

/// A page of paper, and everything written on it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Document {
    /// The paper this is written for. Everything is placed against it.
    pub page: PageSize,
    /// How many sheets. Kept explicit so a blank page can exist on purpose.
    pub pages: usize,
    pub items: Vec<Item>,
    /// What was on the sheets the last time this was printed, if it has been.
    /// This is the whole basis of the delta: not a guess, a record.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub printed: Option<Vec<Item>>,
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
    #[error("{path} is not an Onionskin document: {source}")]
    Malformed {
        path: std::path::PathBuf,
        source: serde_json::Error,
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
            printed: None,
            next_id: 1,
        }
    }

    pub fn load(path: &Path) -> Result<Document, DocumentError> {
        if !path.is_file() {
            return Err(DocumentError::Missing(path.to_path_buf()));
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

/// `#rrggbb` to the three numbers a PDF wants.
fn parse_colour(text: &str) -> Result<(f64, f64, f64), DocumentError> {
    let hex = text.trim().trim_start_matches('#');
    if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(DocumentError::Invalid(format!(
            "{text:?} is not a colour. Write it as #rrggbb, for example #000000 \
             for black."
        )));
    }
    let channel = |from: usize| -> f64 {
        u8::from_str_radix(&hex[from..from + 2], 16).unwrap_or(0) as f64 / 255.0
    };
    Ok((channel(0), channel(2), channel(4)))
}

#[cfg(test)]
mod tests;
