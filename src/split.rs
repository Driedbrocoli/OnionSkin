//! What can be overprinted, and what needs a fresh sheet.
//!
//! Ink does not come off paper. So when an edit *moves* what was already
//! printed — a paragraph inserted on page two pushes everything below it down —
//! no overlay can fix that page. The sheet in the tray no longer matches the
//! document, and the only honest answer for it is a new sheet.
//!
//! Onionskin has always said so. What it did next was refuse the whole job:
//!
//! ```text
//! Blocked — see above. Nothing worth printing was produced.
//! ```
//!
//! On a forty-page report where one line moved on page two, that is thirty-nine
//! pages of perfectly good overlay held back by one. The person then either
//! reprints all forty — which is the cost this program exists to avoid — or
//! passes `--force` and prints the delta anyway, which puts the new ink onto
//! page two on top of text that has since moved. Both are worse than the answer
//! nobody was being given: **do both things**.
//!
//! ```text
//! Feed sheets 1, 3 and 4 back in — the delta has their additions.
//! Print fresh.pdf on a new sheet and replace sheet 2.
//! ```
//!
//! # What is done to the delta
//!
//! The pages that cannot be overprinted are blanked in it. Their additions were
//! placed against text that has moved, so printing them would land words in the
//! wrong place on a sheet that is about to be thrown away anyway. Leaving them
//! in the file is an invitation to feed that sheet, and the cost of accepting
//! the invitation is a ruined page.
//!
//! Blanked, not removed: page three of the delta has to stay page three, since
//! the whole scheme is that the delta's page *n* is fed the printed sheet *n*.

use std::path::{Path, PathBuf};

use lopdf::{dictionary, Document, Object, Stream};

use crate::diff::PageDiff;

/// One page, and what has to happen to it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Verdict {
    /// Counted from 1, as the sheet in somebody's hand is.
    pub page: usize,
    /// Ink that is gone from where it was: the sign that the page moved.
    pub removed_mm2: f64,
    /// How many things the edit added to this page.
    pub additions: usize,
    /// Did the text under this page's additions move?
    pub moved: bool,
}

impl Verdict {
    /// Can this page be overprinted, or does it need a new sheet?
    pub fn needs_a_fresh_sheet(&self) -> bool {
        self.moved
    }

    /// Is there anything to print onto the existing sheet?
    pub fn worth_feeding(&self) -> bool {
        !self.moved && self.additions > 0
    }
}

/// A whole job, sorted into the sheets to feed back and the ones to replace.
#[derive(Debug, Clone, PartialEq)]
pub struct Split {
    pub verdicts: Vec<Verdict>,
}

impl Split {
    /// Worked out from the pages themselves, at the same threshold the reflow
    /// check uses.
    pub fn of(diffs: &[PageDiff]) -> Split {
        let moved: Vec<usize> = diffs
            .iter()
            .filter(|diff| diff.removed_ink_mm2() >= crate::safety::REFLOW_INK_MM2)
            .map(|diff| diff.index + 1)
            .collect();
        Split::given(diffs, &moved)
    }

    /// The same, told which pages moved rather than working it out.
    ///
    /// This is what the pipeline uses, and the list it passes comes from the
    /// checks after [`crate::safety::drop_the_symptoms`] has run. That matters:
    /// two documents handed over in the wrong order have ink missing from every
    /// page, which is one mistake and not forty pages that moved. The reflow
    /// findings are dropped in that case, and taking the list from the raw
    /// measurement instead would write the whole document out as "fresh" and
    /// blank the delta entirely.
    pub fn given(diffs: &[PageDiff], moved: &[usize]) -> Split {
        Split {
            verdicts: diffs
                .iter()
                .map(|diff| Verdict {
                    page: diff.index + 1,
                    removed_mm2: diff.removed_ink_mm2(),
                    additions: diff.added_regions.len(),
                    moved: moved.contains(&(diff.index + 1)),
                })
                .collect(),
        }
    }

    /// Sheets to put back through the printer, counted from 1.
    pub fn feed(&self) -> Vec<usize> {
        self.verdicts
            .iter()
            .filter(|verdict| verdict.worth_feeding())
            .map(|verdict| verdict.page)
            .collect()
    }

    /// Sheets to print new and swap in, counted from 1.
    pub fn reprint(&self) -> Vec<usize> {
        self.verdicts
            .iter()
            .filter(|verdict| verdict.needs_a_fresh_sheet())
            .map(|verdict| verdict.page)
            .collect()
    }

    /// Nothing moved: the ordinary case, and no splitting to do.
    pub fn all_overprintable(&self) -> bool {
        self.reprint().is_empty()
    }

    /// Is anything left to do at all?
    ///
    /// A job where every page has to be reprinted is not a job for this
    /// program, and saying so beats handing over a blank delta.
    pub fn nothing_to_overprint(&self) -> bool {
        self.feed().is_empty()
    }

    /// What somebody standing at the printer has to do, in order.
    pub fn what_to_do(&self, delta: &Path, fresh: &Path) -> String {
        let reprint = self.reprint();
        if reprint.is_empty() {
            return String::new();
        }
        let feed = self.feed();
        if feed.is_empty() {
            return format!(
                "Every page's existing text moved, so there is nothing an \
                 overlay can add to any of them.\n    {} is the whole job, to \
                 print on fresh paper.\n    {} was written blank; there is \
                 nothing to feed.",
                fresh.display(),
                delta.display()
            );
        }
        let one_moved = reprint.len() == 1;
        format!(
            "Two things to print, and they are not the same thing.\n    \
             1. {}: feed {} {} back in. That is the overlay.\n    \
             2. {}: print {} {} on fresh paper and throw the old {} away — the \
             text on {} moved, and ink does not come off paper.\n    \
             {} {} blank in the delta, so feeding {} would do nothing.",
            delta.display(),
            if feed.len() == 1 { "sheet" } else { "sheets" },
            sheets(&feed),
            fresh.display(),
            if one_moved { "sheet" } else { "sheets" },
            sheets(&reprint),
            if one_moved { "one" } else { "ones" },
            if one_moved { "it" } else { "them" },
            if one_moved {
                "That page"
            } else {
                "Those pages"
            },
            if one_moved { "is" } else { "are" },
            if one_moved { "it" } else { "them" },
        )
    }
}

/// Page numbers as somebody would say them: "3, 7 and 9", or "4 to 21".
///
/// Runs collapse, because "4 to 21" is a thing a person can act on and
/// eighteen comma-separated numbers is not. A run of exactly two only becomes
/// "1 and 2" when it is the whole answer — inside a longer list it stays two
/// numbers, or "1 and 3 and 4" comes out, which is not how anybody speaks.
pub fn sheets(pages: &[usize]) -> String {
    let mut runs: Vec<(usize, usize)> = Vec::new();
    for page in pages {
        match runs.last_mut() {
            Some(run) if run.1 + 1 == *page => run.1 = *page,
            _ => runs.push((*page, *page)),
        }
    }
    let alone = runs.len() == 1;
    let mut parts: Vec<String> = Vec::new();
    for (first, last) in &runs {
        match last - first {
            0 => parts.push(first.to_string()),
            // Two in a row, with other parts beside them: said separately, so
            // the only "and" in the sentence is the last one.
            1 if !alone => {
                parts.push(first.to_string());
                parts.push(last.to_string());
            }
            1 => parts.push(format!("{first} and {last}")),
            _ => parts.push(format!("{first} to {last}")),
        }
    }
    match parts.len() {
        0 => String::new(),
        1 => parts[0].clone(),
        _ => format!(
            "{} and {}",
            parts[..parts.len() - 1].join(", "),
            parts[parts.len() - 1]
        ),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SplitError {
    #[error("could not read {path}: {source}")]
    Unreadable { path: PathBuf, source: lopdf::Error },
    #[error("could not write {path}: {source}")]
    NotWritten {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{path} has no page {page} to take.")]
    NoSuchPage { path: PathBuf, page: usize },
}

/// Empty the given pages of a written PDF, leaving the pages themselves there.
///
/// Blanked rather than removed, because the delta's page *n* is fed the printed
/// sheet *n* and a page taken out would shift every sheet after it by one.
pub fn blank_pages(pdf: &Path, pages: &[usize]) -> Result<(), SplitError> {
    if pages.is_empty() {
        return Ok(());
    }
    let mut doc = Document::load(pdf).map_err(|source| SplitError::Unreadable {
        path: pdf.to_path_buf(),
        source,
    })?;
    let ids: Vec<lopdf::ObjectId> = doc.get_pages().into_values().collect();
    for page in pages {
        let Some(id) = page.checked_sub(1).and_then(|index| ids.get(index)) else {
            continue;
        };
        // An empty content stream rather than no `Contents` at all: a page with
        // no contents is legal, but an empty stream is what every producer
        // writes for a blank page and is the better-trodden path through a
        // reader.
        let empty = doc.add_object(Stream::new(dictionary! {}, Vec::new()));
        if let Ok(dict) = doc.get_object_mut(*id).and_then(Object::as_dict_mut) {
            dict.set("Contents", Object::Reference(empty));
        }
    }
    // The old content streams are now unreferenced, and so is anything only
    // they used — a picture that was on one of those pages, most of all.
    doc.prune_objects();
    doc.save(pdf).map_err(|source| SplitError::NotWritten {
        path: pdf.to_path_buf(),
        source,
    })?;
    Ok(())
}

/// Write a new PDF holding only the given pages of `source`, in order.
///
/// This is where the sheets that have to be reprinted come from: the edited
/// document's own pages, whole, so they can go straight to the printer at their
/// true size rather than through a print dialogue that will offer to scale them.
pub fn keep_only(source: &Path, pages: &[usize], out: &Path) -> Result<(), SplitError> {
    let mut doc = Document::load(source).map_err(|source| SplitError::Unreadable {
        path: out.to_path_buf(),
        source,
    })?;
    if pages.is_empty() {
        return Err(SplitError::NoSuchPage {
            path: source.to_path_buf(),
            page: 0,
        });
    }
    let count = doc.get_pages().len();
    for page in pages {
        if *page == 0 || *page > count {
            return Err(SplitError::NoSuchPage {
                path: source.to_path_buf(),
                page: *page,
            });
        }
    }
    let unwanted: Vec<u32> = (1..=count as u32)
        .filter(|page| !pages.contains(&(*page as usize)))
        .collect();
    doc.delete_pages(&unwanted);
    // Everything the deleted pages alone were using goes with them, so a
    // one-page extract from a two-hundred-page report is the size of one page.
    doc.prune_objects();
    doc.renumber_objects();
    doc.compress();
    doc.save(out).map_err(|source| SplitError::NotWritten {
        path: out.to_path_buf(),
        source,
    })?;
    Ok(())
}

#[cfg(test)]
#[path = "split/tests.rs"]
mod tests;
