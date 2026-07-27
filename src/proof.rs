//! Seeing the sheet before it goes back in the tray.
//!
//! Everything else in Onionskin can be looked at before it is committed to. A
//! delta cannot, because the thing it is being added to is on paper: the file
//! on disk is a nearly blank page, and holding it up tells you nothing about
//! whether "Approved" lands in the box or across the line under it. The only
//! honest preview is the two together, and until now the only way to get that
//! was to print it.
//!
//! So this draws them together: the sheet as it already is, in grey, with the
//! additions on top in colour. One picture per page, at the size the paper
//! really is, in a PDF anybody can open.
//!
//! # And the other proof
//!
//! `delta::preview_page` also draws new ink over a ghost of the old, and is not
//! this. It works from a `PageDiff` — the masks that come out of comparing two
//! documents — so it exists only as a by-product of `delta` and `compare`, and
//! writes a PNG per page. This takes any two PDFs, which means it works on a
//! delta from `write`, `draw`, `batch` or `cover`, on one made last week, or on
//! one somebody else sent, none of which have a diff behind them. And it writes
//! a PDF at the real size of the paper, which is the thing to hold up against
//! the sheet.
//!
//! # Why a picture and not a merge
//!
//! Merging two PDFs properly means resolving both documents' resources — fonts,
//! colour spaces, transparency groups, named objects that collide — and getting
//! it slightly wrong produces a file that looks right in one reader and wrong in
//! another. That is a poor trade for something whose whole job is to be looked
//! at. Both pages are rendered by the same engine that will raster them for the
//! printer, and what comes out is what the printer would put down, which is the
//! question being asked.
//!
//! # Tracing paper
//!
//! Turn `sheet_grey` down and the existing page fades to a hint, leaving the
//! additions floating where they will land — the same thing as holding the
//! delta against a window with the original behind it, which is what people did
//! before they had this. Turn it up and it is a photocopy of the finished
//! sheet.

use std::path::Path;

use crate::geometry::PageSize;
use crate::pdf::{PlacedImage, PlacedShape};
use crate::picture::Picture;

#[derive(Debug, thiserror::Error)]
pub enum ProofError {
    #[error("{0}")]
    Render(String),
    #[error("{0}")]
    Pdf(String),
    #[error(
        "'{sheet}' has {sheet_pages} page(s) and '{delta}' has {delta_pages}. \
         A proof lays one on the other, so they have to be the same document's \
         worth of paper."
    )]
    Mismatched {
        sheet: String,
        delta: String,
        sheet_pages: usize,
        delta_pages: usize,
    },
}

/// How the two are drawn on top of one another.
#[derive(Debug, Clone, Copy)]
pub struct ProofOptions {
    /// How finely to draw. Enough to read the words, not enough to be slow:
    /// a proof is looked at on a screen, not measured with a ruler.
    pub dpi: f64,
    /// How strongly the sheet that is already printed shows through, 0 to 1.
    /// At 1 it is a photocopy; near 0 it is tracing paper.
    pub sheet_grey: f64,
    /// What colour the additions are drawn in.
    pub added: [u8; 3],
}

impl Default for ProofOptions {
    fn default() -> Self {
        ProofOptions {
            dpi: 150.0,
            // Grey enough to read, light enough that the additions are
            // unmistakably the new thing — which is the one question a proof
            // is being asked.
            sheet_grey: 0.55,
            // The red every proofreader's pen has been since before any of
            // this was on computers.
            added: [200, 30, 30],
        }
    }
}

impl ProofOptions {
    /// The sheet reduced to a hint, for holding the additions up against.
    pub fn tracing(self) -> ProofOptions {
        ProofOptions {
            sheet_grey: 0.18,
            ..self
        }
    }
}

/// Draw the delta onto the sheet and write the result as a PDF.
pub fn write_proof(
    sheet: &Path,
    delta: &Path,
    out: &Path,
    options: &ProofOptions,
) -> Result<usize, ProofError> {
    let pages = compose_pages(sheet, delta, options)?;
    let sizes: Vec<PageSize> = pages.iter().map(|(size, _)| *size).collect();
    let images: Vec<Vec<PlacedImage>> = pages
        .iter()
        .map(|(size, picture)| {
            vec![PlacedImage {
                picture: picture.clone(),
                x_mm: 0.0,
                y_mm: 0.0,
                width_mm: size.width_mm,
                height_mm: size.height_mm,
                rotation_deg: 0.0,
            }]
        })
        .collect();
    let nothing_else: Vec<Vec<PlacedShape>> = sizes.iter().map(|_| Vec::new()).collect();
    let no_words = sizes.iter().map(|_| Vec::new()).collect::<Vec<_>>();

    crate::pdf::write_page_content_with_pictures(
        out,
        &sizes,
        &no_words,
        &nothing_else,
        &images,
        "Onionskin proof",
        None,
    )
    .map_err(|e| ProofError::Pdf(e.to_string()))?;
    Ok(sizes.len())
}

/// One composed picture per page, and the paper each is on.
fn compose_pages(
    sheet: &Path,
    delta: &Path,
    options: &ProofOptions,
) -> Result<Vec<(PageSize, Picture)>, ProofError> {
    let engine = crate::render::engine().map_err(|e| ProofError::Render(e.to_string()))?;

    // Rendered one document at a time rather than page-by-page in step,
    // because the renderer is behind one lock and holding two documents open
    // across it buys nothing.
    let beneath = render_all(&engine, sheet, options.dpi)?;
    let on_top = render_all(&engine, delta, options.dpi)?;
    if beneath.len() != on_top.len() {
        return Err(ProofError::Mismatched {
            sheet: sheet.display().to_string(),
            delta: delta.display().to_string(),
            sheet_pages: beneath.len(),
            delta_pages: on_top.len(),
        });
    }

    Ok(beneath
        .into_iter()
        .zip(on_top)
        .map(|(under, over)| {
            let size = under.size;
            (size, lay_over(&under, &over, options))
        })
        .collect())
}

/// One page rendered in grey, at the size its paper really is.
struct Rendered {
    size: PageSize,
    width: usize,
    height: usize,
    gray: Vec<u8>,
}

fn render_all(
    engine: &crate::render::EngineGuard,
    path: &Path,
    dpi: f64,
) -> Result<Vec<Rendered>, ProofError> {
    let doc = engine
        .open(path)
        .map_err(|e| ProofError::Render(e.to_string()))?;
    let mut pages = Vec::new();
    for index in 0..doc.len() {
        let page = doc
            .render_gray(index, dpi)
            .map_err(|e| ProofError::Render(e.to_string()))?;
        pages.push(Rendered {
            size: page.size,
            width: page.width,
            height: page.height,
            gray: page.gray,
        });
    }
    Ok(pages)
}

/// Paint one page over the other.
///
/// The sheet goes down as grey, lightened by `sheet_grey` so it reads as
/// background. The additions go on top in colour wherever their own ink is —
/// darker ink meaning more of the colour, so a thin stroke stays thin instead
/// of turning into a solid block the moment it is dark enough to count.
fn lay_over(under: &Rendered, over: &Rendered, options: &ProofOptions) -> Picture {
    let (width, height) = (under.width, under.height);
    let mut rgb = vec![255u8; width * height * 3];
    let strength = options.sheet_grey.clamp(0.0, 1.0);

    for y in 0..height {
        for x in 0..width {
            let at = y * width + x;
            let paper = under.gray[at] as f64 / 255.0;
            // Lightened towards white by however much of the sheet is wanted.
            let shown = 1.0 - (1.0 - paper) * strength;
            let grey = (shown * 255.0).round().clamp(0.0, 255.0) as u8;
            rgb[at * 3] = grey;
            rgb[at * 3 + 1] = grey;
            rgb[at * 3 + 2] = grey;
        }
    }

    // The delta may have been rendered a pixel wider or narrower than the
    // sheet — the two are separate roundings of the same millimetres — so it
    // is placed by proportion rather than assumed to line up index for index.
    for y in 0..height {
        let over_y = scale(y, height, over.height);
        for x in 0..width {
            let over_x = scale(x, width, over.width);
            let ink = over.gray[over_y * over.width + over_x];
            if ink >= 250 {
                continue;
            }
            let how_much = (255 - ink) as f64 / 255.0;
            let at = (y * width + x) * 3;
            for channel in 0..3 {
                let under = rgb[at + channel] as f64;
                let wanted = options.added[channel] as f64;
                rgb[at + channel] = (under + (wanted - under) * how_much).round() as u8;
            }
        }
    }

    Picture::Samples {
        width: width as u32,
        height: height as u32,
        rgb,
        alpha: None,
    }
}

/// The same place in a picture of a different size.
fn scale(at: usize, from: usize, to: usize) -> usize {
    if from == 0 || to == 0 {
        return 0;
    }
    (at * to / from).min(to - 1)
}

#[cfg(test)]
#[path = "proof/tests.rs"]
mod tests;
