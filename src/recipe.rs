//! Turning "put these words on that document" into things to draw.
//!
//! Between a person's instruction — `--after 'Received:Approved'`, or a saved
//! job with `{ref}` in it — and the composer that draws a delta, there is a
//! step: work out what the words actually are, find where they go, and load any
//! pictures. That step used to live in the command-line binary, which meant the
//! window could not run a saved job at all: everything it needed was in a
//! program it does not link against.
//!
//! It is here now, so both front ends ask the same question of the same code.
//! Two spellings of "where does this word go" is how a window and a command line
//! come to put the same job in two different places on the same form.
//!
//! # What a recipe is
//!
//! Placements exactly as they were typed, plus how they are to be set. Three
//! ways to say where:
//!
//! - `at`, `'150,40:PAID'` — millimetres measured on the paper.
//! - `after`, `'Received:Approved'` — just past something already printed.
//! - `below`, `'Signature:J. Bezzina'` — one line under it.
//!
//! The last two are the ones worth having, because they survive the document
//! changing. Next month's invoice will have moved whatever you measured; it will
//! still say "Total".

use std::path::{Path, PathBuf};

use crate::document::Item;
use crate::pdf::PlacedImage;

/// Placements as they were typed, and how to set them.
///
/// The strings are taken verbatim rather than pre-parsed, because that is how
/// a person typed them and how a saved job stores them, and because a parse
/// error is worth reporting against the thing somebody actually wrote.
#[derive(Debug, Clone, Default)]
pub struct Recipe {
    /// `'150,40:PAID'` — millimetres on the paper.
    pub at: Vec<String>,
    /// `'Received:Approved'` — just after something already printed.
    pub after: Vec<String>,
    /// `'Signature:J. Bezzina'` — one line under it.
    pub below: Vec<String>,
    /// `'sign.png:120,240:40'` — a picture, and the box it fills.
    pub images: Vec<String>,
    /// Which page, counted from 1.
    pub page: usize,
    pub size_pt: f64,
    pub font: String,
    pub colour: String,
    /// Wrap at this width, if it was asked for.
    pub width_mm: Option<f64>,
    pub rotation_deg: f64,
    pub leading: f64,
}

impl Recipe {
    /// Whether this asks for anything at all.
    pub fn is_empty(&self) -> bool {
        self.at.is_empty()
            && self.after.is_empty()
            && self.below.is_empty()
            && self.images.is_empty()
    }

    /// Whether anything here has to be matched against the page itself, which
    /// is the expensive part — the page has to be drawn and then read.
    pub fn needs_reading(&self) -> bool {
        !self.after.is_empty() || !self.below.is_empty()
    }
}

/// Where an anchor was found, so it can be reported before anything is printed.
///
/// Worth saying out loud: an anchor is a guess that matched, and somebody
/// should be able to see *what* it matched before they put paper in a printer.
#[derive(Debug, Clone)]
pub struct Found {
    /// The anchor as it was asked for.
    pub anchor: String,
    /// The whole line it was found on, as the page was read — which is how
    /// somebody spots that it matched the wrong "Total".
    pub line: String,
    pub x_mm: f64,
    pub y_mm: f64,
}

impl Found {
    pub fn describe(&self) -> String {
        format!(
            "Found \"{}\" on the line: {}\n  putting the words at {:.1}, {:.1} mm",
            self.anchor.trim(),
            self.line,
            self.x_mm,
            self.y_mm
        )
    }
}

/// A recipe worked out against a particular document.
#[derive(Debug, Default)]
pub struct Laid {
    pub items: Vec<Item>,
    pub images: Vec<(usize, PlacedImage)>,
    /// What each anchor matched. Empty when nothing was anchored.
    pub found: Vec<Found>,
}

/// Work out where everything goes on this document.
///
/// Nothing is written and the document is not touched — it is drawn and read,
/// which is what a person does when they look at it.
///
/// Anchors are resolved before anything else is decided, so a run naming
/// something that is not on the page produces a refusal rather than half a
/// delta. Half the words placed and then an error is the worst of both.
pub fn lay_out(recipe: &Recipe, document: &Path) -> Result<Laid, String> {
    let mut laid = Laid::default();

    for placement in &recipe.at {
        let ((x_mm, y_mm), text) = parse_placement(placement)?;
        laid.items.push(Item {
            id: 0,
            page: recipe.page,
            x_mm,
            y_mm,
            text: unescape(&text),
            size_pt: recipe.size_pt,
            font: recipe.font.clone(),
            width_mm: recipe.width_mm,
            rotation_deg: recipe.rotation_deg,
            colour: recipe.colour.clone(),
            leading: recipe.leading,
        });
    }

    if recipe.needs_reading() {
        let page_text = read_page(document, recipe.page)?;
        // Both worked out from the size the new words will be set at, because
        // only the caller knows that: a gap that looks right at nine point is
        // a collision at eighteen.
        let gap_mm = crate::geometry::pt_to_mm(recipe.size_pt * 0.3);
        let step_mm = crate::geometry::pt_to_mm(recipe.size_pt * 1.15);
        for (specs, put) in [
            (&recipe.after, crate::anchor::Where::After),
            (&recipe.below, crate::anchor::Where::Below),
        ] {
            for spec in specs {
                let (anchor, words) = split_anchor(spec)?;
                let placed = crate::anchor::place(&page_text, &anchor, put, gap_mm, step_mm)
                    .map_err(|e| e.to_string())?;
                laid.found.push(Found {
                    anchor: anchor.clone(),
                    line: placed.line.clone(),
                    x_mm: placed.x_mm,
                    y_mm: placed.y_mm,
                });
                laid.items.push(Item {
                    id: 0,
                    page: recipe.page,
                    x_mm: placed.x_mm,
                    y_mm: placed.y_mm,
                    text: unescape(&words),
                    size_pt: recipe.size_pt,
                    font: recipe.font.clone(),
                    width_mm: recipe.width_mm,
                    rotation_deg: recipe.rotation_deg,
                    colour: recipe.colour.clone(),
                    leading: recipe.leading,
                });
            }
        }
    }

    laid.images = placed_images(&recipe.images, recipe.page)?;
    Ok(laid)
}

/// Read one page of a document, so words can be placed against what is on it.
///
/// The page is drawn and then read, letter by letter, against a real typeface.
/// That is slower than asking a PDF for its text, and it is the only thing that
/// works on every document Onionskin can open — including the ones that are
/// pictures of paper, which is most of what an office actually has.
pub fn read_page(path: &Path, page: usize) -> Result<crate::letters::PageText, String> {
    let (image, registration) = draw_page(path, page)?;

    let reference = crate::font::suggest_system_font()
        .or_else(|| {
            crate::font::installed_fonts()
                .first()
                .map(|f| f.path.clone())
        })
        .ok_or(
            "there is no font on this machine to read the document against, so \
             words cannot be placed by what is already in it. Give the position \
             in millimetres instead.",
        )?;
    let reference = crate::font::EmbeddedFont::load(&reference).map_err(|e| e.to_string())?;

    crate::letters::read_with_font(
        &image,
        &registration,
        &crate::letters::ReadOptions::default(),
        &reference,
        Some(crate::letters::COMMON_LATIN),
    )
    .map_err(|e| e.to_string())
}

/// Draw one page of a document, ready to be read.
///
/// Anything Onionskin can open — a PDF, a Word file, a spreadsheet — comes back
/// as grey pixels on the paper's own grid, with a registration that says how
/// many of them go to the millimetre. Nothing has to be found: a document says
/// where its own edges are, so the registration is square and true by
/// construction, where a photograph of paper has to be measured for skew.
///
/// That is what lets the same reader work on both.
pub fn draw_page(
    path: &Path,
    page: usize,
) -> Result<(image::GrayImage, crate::scan::ScanRegistration), String> {
    let engine = crate::render::engine().map_err(|e| e.to_string())?;
    let workspace = crate::render::Workspace::new(false).map_err(|e| e.to_string())?;
    let (pdf, _, _) =
        crate::render::to_pdf_noting(path, &workspace.path, 180).map_err(|e| e.to_string())?;
    let document = engine.open(&pdf).map_err(|e| e.to_string())?;

    let index = page.saturating_sub(1);
    if index >= document.len() {
        return Err(format!(
            "there is no page {page} in '{}' — it has {} page{}.",
            path.display(),
            document.len(),
            if document.len() == 1 { "" } else { "s" }
        ));
    }

    // Enough resolution to read small print, and not so much that a hundred
    // megapixels are matched against a font for the sake of one anchor.
    const DPI: f64 = 300.0;
    let drawn = document.render(index, DPI).map_err(|e| e.to_string())?;
    let image = image::GrayImage::from_raw(drawn.width as u32, drawn.height as u32, drawn.gray)
        .ok_or("the page could not be turned into an image")?;
    Ok((
        image,
        crate::scan::ScanRegistration {
            page: drawn.size,
            px_per_mm: DPI / 25.4,
            skew_deg: 0.0,
            origin_px: (0.0, 0.0),
        },
    ))
}

/// How many pages a document has, for saying so before reading one of them.
pub fn pages_in(path: &Path) -> Result<usize, String> {
    let engine = crate::render::engine().map_err(|e| e.to_string())?;
    let workspace = crate::render::Workspace::new(false).map_err(|e| e.to_string())?;
    let (pdf, _, _) =
        crate::render::to_pdf_noting(path, &workspace.path, 180).map_err(|e| e.to_string())?;
    let document = engine.open(&pdf).map_err(|e| e.to_string())?;
    let pages = document.len();
    Ok(pages)
}

/// Split `X,Y:the words` into a position in millimetres and the words.
pub fn parse_placement(spec: &str) -> Result<((f64, f64), String), String> {
    let (position, text) = spec.split_once(':').ok_or_else(|| {
        format!("bad placement '{spec}'. Expected 'X,Y:the words', e.g. '60,150:Approved'")
    })?;
    if text.trim().is_empty() {
        return Err(format!("the placement '{spec}' has no words in it"));
    }
    let (x, y) = position
        .split_once(',')
        .ok_or_else(|| format!("bad position in '{spec}'. Expected 'X,Y'"))?;
    let x: f64 = x
        .trim()
        .parse()
        .map_err(|_| format!("'{}' is not a number, in '{spec}'", x.trim()))?;
    let y: f64 = y
        .trim()
        .parse()
        .map_err(|_| format!("'{}' is not a number, in '{spec}'", y.trim()))?;
    if !(x.is_finite() && y.is_finite()) {
        return Err(format!("the position in '{spec}' is not a real number"));
    }
    Ok(((x, y), text.to_string()))
}

/// Split `ANCHOR:WORDS` into the two, on the first colon.
///
/// A colon inside the *words* is left alone, which matters for the times and
/// ratios people write. The anchor rarely needs its own — matching forgives
/// punctuation, so "Received" finds "Received:".
pub fn split_anchor(given: &str) -> Result<(String, String), String> {
    let (anchor, text) = given.split_once(':').ok_or_else(|| {
        format!(
            "bad placement '{given}'. Expected 'ANCHOR:the words' — the thing \
             already on the page, a colon, then what to add."
        )
    })?;
    if anchor.trim().is_empty() {
        return Err(format!("'{given}' does not say what to look for"));
    }
    if text.trim().is_empty() {
        return Err(format!("'{given}' does not say what to write"));
    }
    Ok((anchor.to_string(), text.to_string()))
}

/// The two escapes worth having in a shell argument.
pub fn unescape(text: &str) -> String {
    text.replace("\\n", "\n").replace("\\t", "\t")
}

/// A picture as it was typed: which file, where its top-left corner goes, and
/// whichever of the two measurements were given.
#[derive(Debug, PartialEq)]
pub struct ImageSpec {
    pub path: PathBuf,
    pub x_mm: f64,
    pub y_mm: f64,
    /// `None` when only a height was given, and the width is to follow the
    /// picture's own shape.
    pub width_mm: Option<f64>,
    /// `None` when only a width was given, which is the ordinary case.
    pub height_mm: Option<f64>,
}

/// Split `FILE:X,Y:WIDTH` — or `FILE:X,Y:WIDTHxHEIGHT` — into its parts.
pub fn parse_image(spec: &str) -> Result<ImageSpec, String> {
    let bad = || {
        format!(
            "bad picture '{spec}'. Expected 'FILE:X,Y:WIDTH' — the file, where \
             its top-left corner goes in millimetres, and how wide it is:\n    \
             --image 'signature.png:120,240:40'"
        )
    };
    let (rest, size) = spec.rsplit_once(':').ok_or_else(bad)?;
    let (file, position) = rest.rsplit_once(':').ok_or_else(bad)?;
    if file.trim().is_empty() {
        return Err(bad());
    }

    let (x, y) = position.split_once(',').ok_or_else(bad)?;
    let x_mm: f64 = x.trim().parse().map_err(|_| bad())?;
    let y_mm: f64 = y.trim().parse().map_err(|_| bad())?;

    let (width, height) = match size.split_once(['x', 'X']) {
        Some((w, h)) => (w.trim(), Some(h.trim())),
        None => (size.trim(), None),
    };
    let width_mm: Option<f64> = if width.is_empty() {
        None
    } else {
        Some(width.parse().map_err(|_| bad())?)
    };
    let height_mm: Option<f64> = match height {
        Some(h) if !h.is_empty() => Some(h.parse().map_err(|_| bad())?),
        _ => None,
    };
    if width_mm.is_none() && height_mm.is_none() {
        return Err(bad());
    }
    for measure in [width_mm, height_mm].into_iter().flatten() {
        if !(measure.is_finite() && measure > 0.0) {
            return Err(format!(
                "a picture cannot be {measure} mm across. Give a size greater \
                 than nothing."
            ));
        }
    }
    Ok(ImageSpec {
        path: PathBuf::from(file),
        x_mm,
        y_mm,
        width_mm,
        height_mm,
    })
}

/// Load every picture and work out the box each one fills.
pub fn placed_images(specs: &[String], page: usize) -> Result<Vec<(usize, PlacedImage)>, String> {
    let mut out = Vec::new();
    for spec in specs {
        let ImageSpec {
            path,
            x_mm,
            y_mm,
            width_mm,
            height_mm,
        } = parse_image(spec)?;
        let picture = crate::picture::load(&path).map_err(|e| e.to_string())?;
        // Whichever measurement was left out follows the picture's own shape.
        let (width_mm, height_mm) = match (width_mm, height_mm) {
            (Some(w), Some(h)) => (w, h),
            (Some(w), None) => (w, w / picture.aspect()),
            (None, Some(h)) => (h * picture.aspect(), h),
            (None, None) => unreachable!("parse_image refuses both missing"),
        };
        out.push((
            page,
            PlacedImage {
                picture,
                x_mm,
                y_mm,
                width_mm,
                height_mm,
                rotation_deg: 0.0,
            },
        ));
    }
    Ok(out)
}

#[cfg(test)]
#[path = "recipe/tests.rs"]
mod tests;
