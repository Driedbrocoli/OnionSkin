//! Finding the places on a form where something can be written.
//!
//! The commonest thing anybody does with Onionskin is fill in a printed form,
//! and the first thing they have to do is work out where. That means holding a
//! ruler against the sheet, or opening the scan in an image editor and reading
//! pixel coordinates off it and converting — for every box on the page. It is
//! the dullest possible use of somebody's time, and the page can be asked
//! instead.
//!
//! What comes back is a list of clear places in millimetres, each one ready to
//! be pasted straight into `--at`.
//!
//! # What counts as a blank
//!
//! Two kinds, because forms use two.
//!
//! A **gap on a line of text** is clear paper to the right of, or between,
//! words that are already printed — "Name:" followed by six centimetres of
//! nothing. It is found by looking along the band of rows that line occupies
//! and taking the runs of columns with no ink in them.
//!
//! An **open area** is a band with no printed ink in it at all and enough room
//! to write: the empty half of a page, the space under the last paragraph.
//!
//! Both are reported with a baseline rather than a top edge, because a baseline
//! is what `--at` takes and what a line of type actually sits on.

use std::path::Path;

use crate::geometry::PageSize;

/// A sheet turned into grey pixels on the paper's own grid, however it arrived.
pub struct Sheet {
    /// One byte a pixel.
    pub gray: Vec<u8>,
    pub width: usize,
    /// Pixels to the inch, so millimetres can be worked back out.
    pub dpi: f64,
    pub page: PageSize,
    /// What registering the scan had to say, where it was a scan. Empty for a
    /// PDF, which needs no finding.
    pub note: String,
}

/// Coarse on purpose. Everything here is looking for empty regions several
/// millimetres across, which a thumbnail settles — and a form rendered at 400
/// dpi is sixteen times the pixels for an answer to the nearest millimetre.
pub const LOOKING_DPI: f64 = 100.0;

/// Open a form, whether it is a PDF or a photograph of one.
///
/// Both end up as the same thing: a page of grey at a known number of pixels to
/// the millimetre, straightened onto the paper's own grid, so that a millimetre
/// in the answer is a millimetre on the sheet however crookedly it was scanned.
///
/// In the library rather than in either binary because the window and the
/// command line both ask this question, and two spellings of "which pixels are
/// this sheet" is how they come to disagree about where a box is.
pub fn open_sheet(
    path: &Path,
    page: PageSize,
    cropped: bool,
    square: bool,
) -> Result<Sheet, String> {
    let looks_like_a_picture = matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref(),
        Some("png" | "jpg" | "jpeg" | "tif" | "tiff" | "bmp" | "gif" | "webp")
    );

    if looks_like_a_picture {
        let image =
            image::open(path).map_err(|e| format!("could not read '{}': {e}", path.display()))?;
        let registration = crate::scan::register(
            &image,
            crate::scan::ScanOptions {
                page,
                assume_cropped: cropped,
                assume_square: square,
                ..crate::scan::ScanOptions::new(page)
            },
        )
        .map_err(|e| e.to_string())?;
        let note = registration.describe();
        let flat = registration.flatten(&image.to_luma8(), LOOKING_DPI);
        let width = flat.width() as usize;
        return Ok(Sheet {
            gray: flat.into_raw(),
            width,
            dpi: LOOKING_DPI,
            page,
            note,
        });
    }

    let engine = crate::render::engine().map_err(|e| e.to_string())?;
    let doc = engine.open(path).map_err(|e| e.to_string())?;
    let drawn = doc.render_gray(0, LOOKING_DPI).map_err(|e| e.to_string())?;
    Ok(Sheet {
        gray: drawn.gray,
        width: drawn.width,
        dpi: LOOKING_DPI,
        page: drawn.size,
        note: String::new(),
    })
}

/// A place on the page with nothing printed in it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Blank {
    /// Millimetres from the left edge of the paper.
    pub x_mm: f64,
    /// The baseline: where the bottom of the letters would sit.
    pub y_mm: f64,
    pub width_mm: f64,
    /// How tall the clear band is, top to bottom.
    pub height_mm: f64,
    /// Whether this is a gap beside words already printed, rather than an
    /// empty area of the page.
    pub beside_text: bool,
}

impl Blank {
    /// The type size that suits this gap, in points.
    ///
    /// Beside a printed line it is the size of that line, worked out from how
    /// tall its capitals are — about seven tenths of the type size in every
    /// face there is. Somebody filling in a form wants their answer to look
    /// like the question, and the question is right there to be measured.
    ///
    /// In an open area there is nothing to match, so it is two thirds of the
    /// clear height: a line of type needs room above and below it or it reads
    /// as crammed, and letters with tails hang below the baseline the band was
    /// measured to.
    pub fn fits_pt(&self) -> f64 {
        let mm = if self.beside_text {
            self.height_mm / 0.7
        } else {
            self.height_mm * 0.66
        };
        (mm * 72.0 / 25.4).clamp(6.0, 24.0)
    }

    /// Roughly how many characters fit, at the size that suits the gap.
    ///
    /// A rough average of the widths of the built-in faces rather than a
    /// measurement of any one of them: this is for saying "about forty
    /// characters", which is what somebody deciding what to write wants.
    pub fn fits_characters(&self) -> usize {
        let per_character_mm = self.fits_pt() * 25.4 / 72.0 * 0.5;
        if per_character_mm <= 0.0 {
            return 0;
        }
        (self.width_mm / per_character_mm).floor().max(0.0) as usize
    }

    /// The `--at` this blank would be written with.
    pub fn placement(&self) -> String {
        format!("{:.0},{:.0}", self.x_mm, self.y_mm)
    }

    pub fn describe(&self) -> String {
        format!(
            "{:>6} mm  {:>5.0} mm wide, about {} characters at {:.0} pt{}",
            self.placement(),
            self.width_mm,
            self.fits_characters(),
            self.fits_pt(),
            if self.beside_text {
                ""
            } else {
                "   (open area)"
            }
        )
    }
}

/// How the page is searched.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlankOptions {
    /// How dark a pixel has to be to count as something already printed.
    pub ink_threshold: u8,
    /// The narrowest gap worth reporting. Below this there is nowhere to put a
    /// word, and every form would come back with two hundred entries.
    pub min_width_mm: f64,
    /// The shortest empty band worth reporting as somewhere to write.
    ///
    /// Applied to open areas only. A gap beside a printed line takes its height
    /// from that line, and a line of ordinary eleven-point type is under three
    /// millimetres of ink — measuring it against this would throw away every
    /// gap on every form set in a normal size, which is all of them.
    pub min_height_mm: f64,
    /// How far from the paper's edge to stay. A gap running into the margin is
    /// a gap a printer cannot reach.
    pub margin_mm: f64,
}

impl Default for BlankOptions {
    fn default() -> Self {
        BlankOptions {
            ink_threshold: 128,
            // Twenty millimetres is about four characters at eleven point —
            // below that nothing useful goes in, and the gaps between words
            // would flood the list.
            min_width_mm: 20.0,
            min_height_mm: 3.5,
            margin_mm: crate::safety::DEFAULT_MARGIN_MM,
        }
    }
}

/// Find the places on this page where something could be written.
///
/// `gray` is the page in greyscale at `dpi`, as the renderer or a registered
/// scan hands it over.
pub fn find(
    gray: &[u8],
    width: usize,
    dpi: f64,
    page: PageSize,
    options: &BlankOptions,
) -> Vec<Blank> {
    if width == 0 || gray.is_empty() || dpi <= 0.0 {
        return Vec::new();
    }
    let height = gray.len() / width;
    if height == 0 {
        return Vec::new();
    }
    let px_per_mm = dpi / 25.4;
    let mm = |px: usize| px as f64 / px_per_mm;

    // Which rows have anything printed on them, inside the printable area.
    let left = (options.margin_mm * px_per_mm).round() as usize;
    let right = ((page.width_mm - options.margin_mm) * px_per_mm)
        .round()
        .max(0.0) as usize;
    let right = right.min(width);
    let top = (options.margin_mm * px_per_mm).round() as usize;
    let bottom = ((page.height_mm - options.margin_mm) * px_per_mm)
        .round()
        .max(0.0) as usize;
    let bottom = bottom.min(height);
    if left >= right || top >= bottom {
        return Vec::new();
    }

    let inked_row: Vec<bool> = (0..height)
        .map(|y| {
            if y < top || y >= bottom {
                return false;
            }
            gray[y * width + left..y * width + right]
                .iter()
                .any(|value| *value < options.ink_threshold)
        })
        .collect();

    let mut blanks = Vec::new();
    for band in bands(&inked_row, top, bottom) {
        match band {
            Band::Text { from, to } => {
                blanks.extend(gaps_in_line(
                    gray, width, left, right, from, to, options, px_per_mm,
                ));
            }
            Band::Clear { from, to } => {
                // An empty band is one place to write, sitting on a baseline a
                // little above its bottom so the letters are inside it.
                let tall = mm(to - from);
                if tall < options.min_height_mm {
                    continue;
                }
                let wide = mm(right - left);
                if wide < options.min_width_mm {
                    continue;
                }
                blanks.push(Blank {
                    x_mm: mm(left),
                    y_mm: mm(to) - tall * 0.25,
                    width_mm: wide,
                    height_mm: tall,
                    beside_text: false,
                });
            }
        }
    }

    // Gaps beside printed words first, then open areas, each widest first.
    //
    // A gap beside a label is a place the form is *asking* to be filled in —
    // there is a word next to it saying what goes there. An open area is a
    // guess at where something might go. Sorting purely by width puts the whole
    // empty half of the page above every box on the form, which is the answer
    // to a question nobody asked.
    blanks.sort_by(|a, b| {
        b.beside_text.cmp(&a.beside_text).then(
            b.width_mm
                .partial_cmp(&a.width_mm)
                .unwrap_or(std::cmp::Ordering::Equal),
        )
    });
    blanks
}

/// A run of rows that either has printed ink in it or does not.
enum Band {
    Text { from: usize, to: usize },
    Clear { from: usize, to: usize },
}

/// Split the page into bands of rows, alternating text and clear paper.
fn bands(inked_row: &[bool], top: usize, bottom: usize) -> Vec<Band> {
    let mut bands = Vec::new();
    let mut at = top;
    while at < bottom {
        let inked = inked_row[at];
        let mut to = at;
        while to < bottom && inked_row[to] == inked {
            to += 1;
        }
        bands.push(if inked {
            Band::Text { from: at, to }
        } else {
            Band::Clear { from: at, to }
        });
        at = to;
    }
    bands
}

/// The runs of clear columns on one line of printed text.
#[allow(clippy::too_many_arguments)]
fn gaps_in_line(
    gray: &[u8],
    width: usize,
    left: usize,
    right: usize,
    from: usize,
    to: usize,
    options: &BlankOptions,
    px_per_mm: f64,
) -> Vec<Blank> {
    let mm = |px: usize| px as f64 / px_per_mm;
    let inked_column: Vec<bool> = (left..right)
        .map(|x| (from..to).any(|y| gray[y * width + x] < options.ink_threshold))
        .collect();

    let mut blanks = Vec::new();
    let mut at = 0usize;
    while at < inked_column.len() {
        if inked_column[at] {
            at += 1;
            continue;
        }
        let start = at;
        while at < inked_column.len() && !inked_column[at] {
            at += 1;
        }
        let wide = mm(at - start);
        if wide < options.min_width_mm {
            continue;
        }
        let tall = mm(to - from);
        blanks.push(Blank {
            x_mm: mm(left + start),
            // The baseline of the line this gap is on, so what goes in it sits
            // level with the words beside it — which is the whole reason to
            // prefer a gap on a line over an empty area.
            y_mm: mm(to),
            width_mm: wide,
            height_mm: tall,
            beside_text: true,
        });
    }
    blanks
}

#[cfg(test)]
#[path = "blanks/tests.rs"]
mod tests;
