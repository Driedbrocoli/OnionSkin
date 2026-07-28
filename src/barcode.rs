//! Barcodes and QR codes, worked out here rather than fetched from anywhere.
//!
//! An asset tag, a file reference, a link to the form somebody has to fill in
//! next: all of them are a barcode on a sheet that is otherwise already printed,
//! and none of them is worth a second program to produce.
//!
//! Everything in here is arithmetic. There is no library behind it, nothing is
//! downloaded, and a machine with no network makes the same symbol as one with
//! it — which matters, because the commonest way to get a barcode today is a
//! website that has been handed the thing being encoded.
//!
//! # What comes out
//!
//! A [`Symbol`]: a grid of dark squares and the size of one square. The caller
//! turns that into rectangles on a page, so a barcode is drawn with exactly the
//! same machinery as everything else Onionskin puts on paper and needs no new
//! kind of thing in the PDF.
//!
//! # What matters when it is printed
//!
//! A barcode is read by a machine, so "it looks right" is not the test. Three
//! things decide whether a scanner sees it:
//!
//!   * **The quiet zone.** White space either side, and no printing in it. A
//!     barcode butted up against a line of text will not read. [`Symbol`] leaves
//!     it, and the placement keeps it.
//!   * **Module size.** One module under about 0.25 mm is smaller than a laser
//!     printer reliably puts down. The defaults here are well above that.
//!   * **Contrast.** Black on white. This is a delta printed over an existing
//!     sheet, so a barcode laid over printing is a barcode that will not read —
//!     which [`Symbol::over_printing`] is for.

pub mod code128;
pub mod qr;

/// A finished symbol: which squares are dark, and how big a square is.
///
/// Both a barcode and a QR code come out of here. A linear barcode is a grid one
/// row deep — the caller stretches it to whatever height it should be printed
/// at, which is what makes the two the same kind of thing.
#[derive(Debug, Clone, PartialEq)]
pub struct Symbol {
    /// Dark squares, row-major, `width` per row.
    pub dark: Vec<bool>,
    pub width: usize,
    pub height: usize,
    /// How many modules of clear paper the symbol needs around it.
    ///
    /// Not decoration. A scanner finds the start of a symbol by the white
    /// before it, and there is no way to recover a symbol that has a table
    /// border running through its quiet zone.
    pub quiet: usize,
    /// What was encoded, kept so a caller can print it underneath.
    pub text: String,
}

impl Symbol {
    pub fn dark_at(&self, x: usize, y: usize) -> bool {
        if x >= self.width || y >= self.height {
            return false;
        }
        self.dark[y * self.width + x]
    }

    /// How wide the symbol is on paper, quiet zone included.
    pub fn width_mm(&self, module_mm: f64) -> f64 {
        (self.width + self.quiet * 2) as f64 * module_mm
    }

    /// How tall it is on paper, for a symbol that is square by nature.
    ///
    /// A linear barcode is one module tall and is not printed that way — the
    /// caller gives it a height. This is the answer for a QR code, where the
    /// height is not a choice.
    pub fn height_mm(&self, module_mm: f64) -> f64 {
        (self.height + self.quiet * 2) as f64 * module_mm
    }

    /// The dark squares as rectangles, in millimetres from the top-left of
    /// where the symbol is placed.
    ///
    /// Runs of dark modules along a row come out as one rectangle rather than
    /// several. Not for tidiness: a laser printer lays down two abutting
    /// rectangles with a hairline of paper showing between them often enough
    /// that scanners see an extra bar, and a QR code drawn as six hundred
    /// separate squares is also six hundred operators in the page.
    pub fn rectangles(&self, module_mm: f64) -> Vec<(f64, f64, f64, f64)> {
        let mut out = Vec::new();
        let offset = self.quiet as f64 * module_mm;
        for y in 0..self.height {
            let mut x = 0;
            while x < self.width {
                if !self.dark_at(x, y) {
                    x += 1;
                    continue;
                }
                let start = x;
                while x < self.width && self.dark_at(x, y) {
                    x += 1;
                }
                out.push((
                    offset + start as f64 * module_mm,
                    offset + y as f64 * module_mm,
                    (x - start) as f64 * module_mm,
                    module_mm,
                ));
            }
        }
        out
    }

    /// The same, stretched to a height — which is what a linear barcode wants.
    ///
    /// A one-row symbol drawn through [`Symbol::rectangles`] would be one module
    /// tall and unreadable. This gives each bar its full height in one
    /// rectangle rather than stacking copies of a row.
    pub fn bars(&self, module_mm: f64, height_mm: f64) -> Vec<(f64, f64, f64, f64)> {
        if self.height != 1 {
            return self.rectangles(module_mm);
        }
        let offset = self.quiet as f64 * module_mm;
        self.rectangles(module_mm)
            .into_iter()
            .map(|(x, _, width, _)| (x, offset, width, height_mm))
            .collect()
    }

    /// Whether printing this over what is already on the sheet will spoil it.
    ///
    /// The honest answer to "can I put a barcode on this printed form": only
    /// where the paper is blank. Toner goes on top, and a bar with a line of
    /// text through it is a bar of a different width — which is not a barcode
    /// that reads wrongly, it is one that does not read at all.
    pub fn over_printing() -> &'static str {
        "A barcode has to go on blank paper. Toner goes on top of what is \
         already printed, and printing showing through the bars changes their \
         widths — a scanner will not read it at all. Use `onionskin blanks` to \
         find somewhere clear on the sheet."
    }
}

/// How small a module may be before a laser printer stops putting it down
/// reliably, in millimetres.
///
/// Below this the bars come out ragged: a 600 dpi printer's dot is 0.042 mm and
/// toner spreads past where it was asked to go, so a 0.2 mm bar prints somewhere
/// between 0.15 and 0.3. Scanners measure bars against their neighbours, so that
/// variation is exactly what they cannot survive.
pub const SMALLEST_MODULE_MM: f64 = 0.25;

/// Whether a module this size will survive being printed.
pub fn too_small_to_print(module_mm: f64) -> bool {
    module_mm < SMALLEST_MODULE_MM
}

#[cfg(test)]
#[path = "barcode/tests.rs"]
mod tests;
