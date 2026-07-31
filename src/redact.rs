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
    #[error(
        "the file just written could not be read back to check it, so it has been \
         deleted rather than handed over: {why}\n    Nothing has been redacted. A \
         redaction nobody can check is not a redaction."
    )]
    Unchecked { why: String },
    #[error(
        "{dpi} is not a resolution a page can be drawn at. Give a number between \
         {MIN_DPI:.0} and {MAX_DPI:.0} — {DEFAULT_DPI:.0} is what a printer is asked for."
    )]
    BadResolution { dpi: f64 },
    #[error(
        "the rectangle {width_mm}x{height_mm} mm at {x_mm},{y_mm} covers nothing on \
         page {page}, which is {page_width_mm:.0}x{page_height_mm:.0} mm — so nothing \
         would have been taken out of it.\n    Nothing has been written. A document \
         that has had nothing taken out of it must not be handed over as though it \
         had."
    )]
    PaintedNothing {
        page: usize,
        x_mm: f64,
        y_mm: f64,
        width_mm: f64,
        height_mm: f64,
        page_width_mm: f64,
        page_height_mm: f64,
    },
    #[error(
        "drawing {pages} page(s) at {dpi:.0} dpi needs about {gigabytes:.1} GB of \
         memory at once, which is more than this is willing to ask for.\n    Try a \
         lower resolution: --dpi {suggestion:.0} would need about {then:.1} GB and is \
         still readable."
    )]
    TooMuchAtOnce {
        pages: usize,
        dpi: f64,
        gigabytes: f64,
        suggestion: f64,
        then: f64,
    },
    #[error(
        "there is already something at {path}, which is where the half-finished copy \
         has to go.\n    Move it out of the way, or choose another name for the \
         redacted copy."
    )]
    WorkingFileInTheWay { path: PathBuf },
}

/// The coarsest and finest a page may be drawn.
///
/// Not taste, and not a guess. Below about fifty dots to the inch ordinary type
/// stops being letters and becomes grey texture, and a redacted copy nobody can
/// read is not a copy of the document. Above about twelve hundred the file grows
/// without anything appearing on paper that was not there at eight hundred.
///
/// The reason there is a *floor* at all, rather than a shrug, is that
/// [`PageSize::px_size`] clamps to one pixel: `--dpi 0` used to draw every page
/// of the document as a single pixel stretched across the sheet, write it, check
/// it, find no text in it — because there was nothing in it at all — and report
/// a successful redaction. The document was destroyed and the program said the
/// words were gone. They were. So was everything else.
pub const MIN_DPI: f64 = 50.0;
pub const MAX_DPI: f64 = 1200.0;

/// How much memory the drawn pages may take between them, in bytes.
///
/// Every page is held as raw colour samples until the whole document is written,
/// and an A4 page at 300 dpi is 2480 x 3508 x 3 bytes — twenty-six megabytes.
/// A hundred-page report is two and a half gigabytes, which on an ordinary
/// machine is not a slow redaction, it is the program being killed part way
/// through with no explanation anybody could act on.
///
/// So the arithmetic is done first and the answer is a sentence naming a
/// resolution that would work, rather than a process that disappears.
const MOST_MEMORY: f64 = 1.5 * 1024.0 * 1024.0 * 1024.0;

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
        if self.had_text {
            said.push(
                "The words are gone from the file, not covered over: there is no text \
                 left anywhere in it, which was checked page by page before this was \
                 written."
                    .to_string(),
            );
            said.push(
                "Which also means the document can no longer be searched, and text \
                 cannot be copied out of it. Keep the original somewhere safe — this \
                 is the copy to hand over, not the copy to work from."
                    .to_string(),
            );
        } else {
            // The check that makes this feature worth anything is "there is no
            // text left in the file". On a scan there was none to begin with,
            // so the check passes without having proved a thing, and saying
            // "the words are gone from the file" would be leaning on a test
            // that never ran. What is actually true is narrower and worth
            // saying plainly: the ink under those rectangles is not in the
            // copy, and there was never any hidden text to find.
            said.push(
                "This document was already a picture of a page — there was no text in \
                 it to take out, so what has gone is the ink under the rectangles you \
                 gave. Nothing was hidden underneath them to begin with."
                    .to_string(),
            );
            said.push(
                "Which also means nothing here was found for you: a rectangle covers \
                 what you measured and nothing else. Look at the copy before you send \
                 it."
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
    if !dpi.is_finite() || !(MIN_DPI..=MAX_DPI).contains(&dpi) {
        return Err(RedactError::BadResolution { dpi });
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
    weigh(&opened.page_sizes, dpi)?;

    // A rectangle with no size in it, asked before anything is drawn. Rounding
    // outwards to whole pixels means a rectangle 0 mm wide still blackens a
    // single column, so the pixel count after the loop would call that painted
    // — one dot of toner where somebody expected a bar.
    if let Some(empty) = areas.iter().find(|area| !has_size(area)) {
        return Err(nothing_covered(empty, &opened.page_sizes));
    }

    // How much of each rectangle actually landed on paper. Counted rather than
    // assumed, because the answer can be none — see the check after the loop.
    let mut painted = vec![0usize; areas.len()];
    let mut had_text = false;
    let mut sizes: Vec<PageSize> = Vec::with_capacity(pages);
    let mut images: Vec<Vec<PlacedImage>> = Vec::with_capacity(pages);
    for index in 0..pages {
        if !opened.text_on(index)?.trim().is_empty() {
            had_text = true;
        }
        let mut drawn = opened.render(index, dpi)?;
        for (at, area) in areas.iter().enumerate() {
            if area.page != index + 1 {
                continue;
            }
            painted[at] += paint_out(&mut drawn.rgb, drawn.width, drawn.height, drawn.size, area);
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

    // A rectangle that covered nothing. The person asked for something to be
    // taken out, nothing was, and the file about to be written would be the
    // document unchanged with a sentence attached saying the words were gone.
    // Refusing here is the whole difference between this feature working and
    // this feature being the thing that leaks the document.
    if let Some(at) = painted.iter().position(|&pixels| pixels == 0) {
        return Err(nothing_covered(&areas[at], &opened.page_sizes));
    }

    // Written to a name of its own first, so that a document which fails the
    // check below is never at the path somebody is about to send.
    //
    // Refused rather than overwritten if something is already there. The name
    // is predictable — it is `out` with the extension changed — so on a shared
    // directory somebody else can put a symlink at it and have this program
    // write their file for them. Nothing here needs that path badly enough to
    // take it by force.
    let nearly = out.with_extension("onionskin-redacting");
    if std::fs::symlink_metadata(&nearly).is_ok() {
        return Err(RedactError::WorkingFileInTheWay { path: nearly });
    }
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
    //
    // Unreadable counts as failed, and says so in its own words. The point of
    // this step is that nothing is handed over unless it has been *shown* to be
    // clean, and a page that could not be read has not been shown to be
    // anything — but telling somebody "text survived" when the truth is "the
    // check would not run" sends them looking for a leak that is not there.
    let check = engine.open(&nearly);
    let verdict = match check {
        Ok(written) => (0..written.len()).try_for_each(|index| match written.text_on(index) {
            Ok(text) if text.trim().is_empty() => Ok(()),
            Ok(_) => Err(RedactError::TextSurvived { page: index + 1 }),
            Err(why) => Err(RedactError::Unchecked {
                why: format!("page {}: {why}", index + 1),
            }),
        }),
        Err(why) => Err(RedactError::Unchecked {
            why: why.to_string(),
        }),
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

/// Paint one rectangle solid black on a rendered page, and say how much of it
/// landed on the page.
///
/// Clamped rather than refused where it hangs over the edge: an area that runs
/// off the paper is somebody measuring generously round the thing they want
/// gone, and refusing that would be refusing the safe mistake.
///
/// Landing *entirely* off the paper is the other mistake, and the count is
/// here so the caller can tell the two apart. Nothing is painted, everything
/// looks fine, and the file that comes out is the document with the secret
/// still in it — over a sentence saying it has been taken out.
fn paint_out(rgb: &mut [u8], width: usize, height: usize, page: PageSize, area: &Area) -> usize {
    let px_per_pt = width as f64 / page.width_pt();
    let to_px = |mm: f64| mm_to_pt(mm) * px_per_pt;

    // Anything that is not a number cannot be clamped into one: `NaN as usize`
    // is zero, which would put the rectangle at the top-left corner of the page
    // rather than nowhere, and painting the wrong part of somebody's document
    // black is worse than telling them the measurement was not a measurement.
    if [area.x_mm, area.y_mm, area.width_mm, area.height_mm]
        .iter()
        .any(|value| !value.is_finite())
    {
        return 0;
    }

    let x0 = to_px(area.x_mm).floor().max(0.0) as usize;
    let y0 = to_px(area.y_mm).floor().max(0.0) as usize;
    let x1 = (to_px(area.x_mm + area.width_mm).ceil().max(0.0) as usize).min(width);
    let y1 = (to_px(area.y_mm + area.height_mm).ceil().max(0.0) as usize).min(height);

    let mut painted = 0;
    for y in y0..y1 {
        for x in x0..x1 {
            let at = (y * width + x) * 3;
            if at + 2 < rgb.len() {
                rgb[at] = 0;
                rgb[at + 1] = 0;
                rgb[at + 2] = 0;
                painted += 1;
            }
        }
    }
    painted
}

/// Whether a rectangle has any area at all.
///
/// Written the positive way round on purpose: a measurement that came out as
/// `NaN` fails every one of these comparisons, which is the answer wanted.
fn has_size(area: &Area) -> bool {
    area.width_mm.is_finite()
        && area.height_mm.is_finite()
        && area.width_mm > 0.0
        && area.height_mm > 0.0
}

/// The refusal for a rectangle that would take nothing out, told in terms of
/// the page it missed so somebody can see how they missed it.
fn nothing_covered(area: &Area, pages: &[PageSize]) -> RedactError {
    let page = pages
        .get(area.page.saturating_sub(1))
        .copied()
        .unwrap_or(PageSize {
            width_mm: 0.0,
            height_mm: 0.0,
        });
    RedactError::PaintedNothing {
        page: area.page,
        x_mm: area.x_mm,
        y_mm: area.y_mm,
        width_mm: area.width_mm,
        height_mm: area.height_mm,
        page_width_mm: page.width_mm,
        page_height_mm: page.height_mm,
    }
}

/// Whether the drawn pages would fit in memory all at once, and what to do if
/// they would not.
///
/// See [`MOST_MEMORY`] for why this is asked before anything is drawn rather
/// than discovered half way through a hundred-page report.
fn weigh(pages: &[PageSize], dpi: f64) -> Result<(), RedactError> {
    let bytes: f64 = pages
        .iter()
        .map(|size| {
            let (width, height) = size.px_size(dpi);
            width as f64 * height as f64 * 3.0
        })
        .sum();
    if bytes <= MOST_MEMORY {
        return Ok(());
    }
    // Memory goes as the square of the resolution, so the resolution that fits
    // is the current one scaled by the square root of how far over it is.
    // Rounded down to a round number, because "--dpi 137" reads as a machine
    // talking to itself and "--dpi 130" reads as advice.
    let suggestion = ((dpi * (MOST_MEMORY / bytes).sqrt() / 10.0).floor() * 10.0).max(MIN_DPI);
    Err(RedactError::TooMuchAtOnce {
        pages: pages.len(),
        dpi,
        gigabytes: bytes / 1024.0 / 1024.0 / 1024.0,
        suggestion,
        then: bytes * (suggestion / dpi).powi(2) / 1024.0 / 1024.0 / 1024.0,
    })
}

#[cfg(test)]
#[path = "redact/tests.rs"]
mod tests;
