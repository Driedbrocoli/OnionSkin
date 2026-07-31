//! Taking something out of a document, rather than drawing over it.
//!
//! # Why this is a separate thing from `cover`
//!
//! [`crate::cover`]'s neighbour on the command line puts black ink on paper.
//! That is a real redaction: the toner is on the sheet and there is nothing
//! underneath it. This is the other half of the same problem, and it is the
//! half people get wrong.
//!
//! A PDF handed to a regulator, an opposing solicitor, a journalist under
//! freedom of information — that is a *file*, and drawing a black rectangle in
//! a file hides nothing. The text is still there: selectable, copyable,
//! searchable, and recoverable by anybody who presses Ctrl-A. Every few months
//! an organisation publishes a document redacted that way and finds the
//! covered names in the newspaper the following week. It is not an exotic
//! mistake; it is the obvious thing to do, and the obvious thing is wrong.
//!
//! # What this does instead
//!
//! It draws every page as a picture, paints the redacted areas solid on the
//! picture, and writes a document made of those pictures. There is then no
//! text object anywhere in the file, because there is no text object anywhere
//! in the file — not "none in the redacted area", none at all.
//!
//! That is a deliberate choice and it costs something real: the document can
//! no longer be searched or have text copied out of it, including on pages
//! nothing was taken from. The alternative — cutting the text operators out of
//! the page's own instructions and leaving the rest — keeps that, and it is
//! how a subtle version of the same mistake gets made. A word can be in the
//! file in more places than the page it is drawn on: in a form field's value,
//! in an annotation, in a bookmark, in the document's own metadata, in a font
//! subset's glyph names, in the leftovers of an earlier save that PDF's
//! incremental-update format keeps. Something that removes only what it
//! recognises leaves whatever it did not.
//!
//! Flattening has one property none of that can offer: after it, the
//! statement "there is no text in this document" is checkable in one line —
//! and [`redact`] checks it, on every page, before it will hand the file over.
//! A redaction that cannot be checked is not a redaction.

use std::path::{Path, PathBuf};

use crate::geometry::{mm_to_pt, PageSize};
use crate::pdf::{PlacedImage, PlacedLine, PlacedShape};
use crate::picture::Picture;

/// How finely the pages are drawn when nobody says.
///
/// Three hundred is what a printer is asked for and what a scanner is set to,
/// so a redacted document prints the same as the one it came from. Lower and
/// small type goes soft; higher and the file grows without anybody being able
/// to see the difference on paper.
pub const DEFAULT_DPI: f64 = 300.0;

/// A rectangle to take out, in millimetres from the top-left of the page.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Area {
    /// Counted from 1, the way a person counts pages.
    pub page: usize,
    pub x_mm: f64,
    pub y_mm: f64,
    pub width_mm: f64,
    pub height_mm: f64,
}

#[derive(Debug, thiserror::Error)]
pub enum RedactError {
    #[error("{0}")]
    Render(#[from] crate::render::RenderError),
    #[error("{0}")]
    Pdf(#[from] crate::pdf::PdfError),
    #[error("could not write {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error(
        "nothing to take out. Say what to remove:\n    --word 'Salary'            \
         take out whatever that word sits on\n    --over '40,100:70x8'       take out \
         a rectangle, in millimetres"
    )]
    NothingAsked,
    #[error("this document has {pages} page(s), and something was asked for page {asked}")]
    NoSuchPage { pages: usize, asked: usize },
    #[error(
        "the redaction did not hold: page {page} of the file just written still has \
         text in it, so it has been deleted rather than handed over. This is a fault \
         in Onionskin — please report it. Nothing has been redacted."
    )]
    TextSurvived { page: usize },
}

/// What a redaction did.
#[derive(Debug, Clone, PartialEq)]
pub struct Redacted {
    pub output: PathBuf,
    /// How many pages the document has, all of which are now pictures.
    pub pages: usize,
    /// How many rectangles were taken out.
    pub areas: usize,
    pub dpi: f64,
    /// Whether the document had any text in it to begin with.
    ///
    /// A scan has none, and "no text in the result" then says nothing about
    /// whether the flattening worked — so it is reported rather than assumed.
    pub had_text: bool,
}

impl Redacted {
    /// What somebody needs to be told, in the order they need it.
    pub fn describe(&self) -> Vec<String> {
        let mut said = vec![format!(
            "{} area{} taken out of {} page{}, at {:.0} dpi.",
            self.areas,
            if self.areas == 1 { "" } else { "s" },
            self.pages,
            if self.pages == 1 { "" } else { "s" },
            self.dpi
        )];
        said.push(
            "The words are gone from the file, not covered over: there is no text left \
             anywhere in it, which was checked page by page before this was written."
                .to_string(),
        );
        if self.had_text {
            said.push(
                "Which also means the document can no longer be searched, and text \
                 cannot be copied out of it. Keep the original somewhere safe — this \
                 is the copy to hand over, not the copy to work from."
                    .to_string(),
            );
        }
        said
    }
}

/// Every line of the document that carries one of these phrases, and where.
///
/// The heart of `--word`, and the place the first version of this got wrong in
/// two ways at once. It read *one* page and marked up that page's coordinates,
/// so on anything longer than a sheet the named words survived in the clear on
/// every other page — while the program said they were gone. And it covered
/// the matched token rather than the line, so `--word Salary` blacked out the
/// label and left `84000 per annum` perfectly legible. Its own worked example
/// in the README leaked the number it was demonstrating the removal of.
///
/// So: every page, and the whole line. Covering the line is deliberate rather
/// than convenient. Somebody who says "take out the salary" is pointing at
/// `Salary: 84000 per annum`, not at six letters of label, and in a redaction
/// the two mistakes are not equal — covering more than was needed is a
/// document with a longer black bar on it, and covering less is the disclosure
/// this whole feature exists to prevent.
///
/// The lines come from the document's own text layer, so this is not a guess:
/// a PDF that carries text knows where every character is. A scan carries
/// none, which is why [`Found::from_a_scan`] exists and says so.
pub fn lines_carrying(
    document: &Path,
    wanted: &[String],
    pad_mm: f64,
) -> Result<Found, RedactError> {
    let engine = crate::render::engine()?;
    let opened = engine.open(document)?;
    let mut found = Found::default();
    let mut any_text = false;

    for index in 0..opened.len() {
        let lines = opened.lines_on(index)?;
        if !lines.is_empty() {
            any_text = true;
        }
        for line in &lines {
            let haystack = squash(&line.text);
            for phrase in wanted {
                let needle = squash(phrase);
                if needle.is_empty() || !haystack.contains(&needle) {
                    continue;
                }
                found.areas.push(Area {
                    page: index + 1,
                    x_mm: line.x_mm - pad_mm,
                    y_mm: line.y_mm - pad_mm,
                    width_mm: line.width_mm + pad_mm * 2.0,
                    height_mm: line.height_mm + pad_mm * 2.0,
                });
                found.covered.push(Covered {
                    page: index + 1,
                    phrase: phrase.clone(),
                    line: line.text.trim().to_string(),
                });
            }
        }
    }

    found.missing = wanted
        .iter()
        .filter(|phrase| !found.covered.iter().any(|c| &&c.phrase == phrase))
        .cloned()
        .collect();
    found.from_a_scan = !any_text;
    Ok(found)
}

/// Compared with the spaces and the case taken out, so `Salary :` on the page
/// still answers to `salary`. Nothing cleverer: a redaction that matches
/// approximately is a redaction that covers the wrong line.
fn squash(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// What a search for phrases turned up.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Found {
    pub areas: Vec<Area>,
    /// One per line covered, so the person can be shown what went.
    pub covered: Vec<Covered>,
    /// Phrases that appear nowhere. Handing back a document that has had
    /// nothing taken out of it, as though it had, is the worst outcome here —
    /// so this is reported and the caller refuses.
    pub missing: Vec<String>,
    /// The document carries no text at all, so nothing could be searched.
    /// A scan is a picture of words, and `--over` is the only honest way to
    /// redact one.
    pub from_a_scan: bool,
}

/// One line that will be taken out, in the words that are on it.
#[derive(Debug, Clone, PartialEq)]
pub struct Covered {
    pub page: usize,
    pub phrase: String,
    pub line: String,
}

/// Take the areas out of the document, and write what is left.
///
/// The file is written only if it comes back clean. See the module comment for
/// why every page becomes a picture, and [`crate::render::Document::text_on`]
/// for the check that makes it worth anything.
pub fn redact(
    document: &Path,
    out: &Path,
    areas: &[Area],
    dpi: f64,
) -> Result<Redacted, RedactError> {
    if areas.is_empty() {
        return Err(RedactError::NothingAsked);
    }
    let engine = crate::render::engine()?;
    let opened = engine.open(document)?;
    let pages = opened.len();
    if let Some(beyond) = areas
        .iter()
        .find(|area| area.page == 0 || area.page > pages)
    {
        return Err(RedactError::NoSuchPage {
            pages,
            asked: beyond.page,
        });
    }

    let mut had_text = false;
    let mut sizes: Vec<PageSize> = Vec::with_capacity(pages);
    let mut images: Vec<Vec<PlacedImage>> = Vec::with_capacity(pages);
    for index in 0..pages {
        if !opened.text_on(index)?.trim().is_empty() {
            had_text = true;
        }
        let mut drawn = opened.render(index, dpi)?;
        for area in areas.iter().filter(|area| area.page == index + 1) {
            paint_out(&mut drawn.rgb, drawn.width, drawn.height, drawn.size, area);
        }
        sizes.push(drawn.size);
        images.push(vec![PlacedImage {
            picture: Picture::Samples {
                width: drawn.width as u32,
                height: drawn.height as u32,
                rgb: drawn.rgb,
                alpha: None,
            },
            x_mm: 0.0,
            y_mm: 0.0,
            width_mm: drawn.size.width_mm,
            height_mm: drawn.size.height_mm,
            rotation_deg: 0.0,
        }]);
    }

    // Written to a name of its own first, so that a document which fails the
    // check below is never at the path somebody is about to send.
    let nearly = out.with_extension("onionskin-redacting");
    let nothing: Vec<Vec<PlacedLine>> = vec![Vec::new(); pages];
    let no_shapes: Vec<Vec<PlacedShape>> = vec![Vec::new(); pages];
    crate::pdf::write_page_content_with_pictures(
        &nearly,
        &sizes,
        &nothing,
        &no_shapes,
        &images,
        "Onionskin redacted",
        None,
    )?;

    // The check. Everything above is an argument that the file has no text in
    // it; this is the file being asked.
    let check = engine.open(&nearly);
    let verdict = match check {
        Ok(written) => (0..written.len()).try_for_each(|index| {
            match written.text_on(index) {
                Ok(text) if text.trim().is_empty() => Ok(()),
                // Unreadable counts as failed. The point of this step is that
                // nothing is handed over unless it has been shown to be clean.
                _ => Err(RedactError::TextSurvived { page: index + 1 }),
            }
        }),
        Err(why) => Err(RedactError::Render(why)),
    };
    if let Err(why) = verdict {
        let _ = std::fs::remove_file(&nearly);
        return Err(why);
    }

    std::fs::rename(&nearly, out).map_err(|source| RedactError::Io {
        path: out.to_path_buf(),
        source,
    })?;
    Ok(Redacted {
        output: out.to_path_buf(),
        pages,
        areas: areas.len(),
        dpi,
        had_text,
    })
}

/// Paint one rectangle solid black on a rendered page.
///
/// Clamped to the page rather than refused: an area that hangs over the edge
/// is somebody measuring generously round the thing they want gone, and
/// refusing it would be refusing the safe mistake.
fn paint_out(rgb: &mut [u8], width: usize, height: usize, page: PageSize, area: &Area) {
    let px_per_pt = width as f64 / page.width_pt();
    let to_px = |mm: f64| mm_to_pt(mm) * px_per_pt;

    let x0 = to_px(area.x_mm).floor().max(0.0) as usize;
    let y0 = to_px(area.y_mm).floor().max(0.0) as usize;
    let x1 = (to_px(area.x_mm + area.width_mm).ceil().max(0.0) as usize).min(width);
    let y1 = (to_px(area.y_mm + area.height_mm).ceil().max(0.0) as usize).min(height);

    for y in y0..y1 {
        for x in x0..x1 {
            let at = (y * width + x) * 3;
            if at + 2 < rgb.len() {
                rgb[at] = 0;
                rgb[at + 1] = 0;
                rgb[at + 2] = 0;
            }
        }
    }
}

#[cfg(test)]
#[path = "redact/tests.rs"]
mod tests;
