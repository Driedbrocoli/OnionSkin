//! A word across the whole sheet: DRAFT, COPY, VOID.
//!
//! Somebody has a printed document in their hand and needs it marked before it
//! goes anywhere — a draft that must not be mistaken for the final, a copy that
//! must not be mistaken for the original, a form that has been superseded. The
//! word goes corner to corner, big enough that nobody misses it.
//!
//! A word processor does this by putting grey text *behind* the page's own.
//! Onionskin cannot: a printer adds toner and never takes it away, so a
//! watermark on an already-printed sheet goes **on top** of whatever is there.
//!
//! That is not a defect to be hidden, it is the thing to design around:
//!
//!   * The default is light — about a quarter of full black — because grey
//!     toner over printed text leaves the text readable and dark toner does
//!     not.
//!   * It is one line of outlined-looking type across the diagonal rather than
//!     a tiled pattern, so most of the page is untouched.
//!   * On a *fresh* page there is nothing underneath and it can be as dark as
//!     anybody likes.
//!
//! # Where it goes
//!
//! Corner to corner, centred, and sized to fit. The size is worked out from the
//! word itself: whatever type size makes it span most of the paper's diagonal
//! is the size it is set at, so "VOID" and "NOT FOR CIRCULATION" both come out
//! filling the page rather than one of them lost in the middle of it.

use crate::geometry::PageSize;
use crate::pdf::{builtin_width_mm, Font};

/// How much of the room there is the word takes up.
///
/// Not all of it: a word sized to touch all four edges has its first and last
/// letters in the corners, which is where a printer's grip is and where the
/// unprintable border lives. Four fifths keeps it clear of both.
pub const ACROSS: f64 = 0.8;

/// How dark the word is by default, as a share of full black.
///
/// Light, because this is toner going on top of printing that has to stay
/// readable. Dark enough to be unmistakable from arm's length; light enough
/// that the words underneath still are.
pub const GREY: f64 = 0.75;

/// Cap height as a share of the type size, the same ratio the rest of the
/// program measures type by.
const CAP_SHARE: f64 = 0.7;

/// A word laid across the sheet.
#[derive(Debug, Clone, PartialEq)]
pub struct Watermark {
    pub text: String,
    /// Where the first letter starts, in millimetres from the top-left.
    pub x_mm: f64,
    /// The baseline of that first letter.
    pub y_mm: f64,
    pub size_pt: f64,
    /// Clockwise on the page, the way the rest of the program turns things.
    pub rotation_deg: f64,
    /// Grey, as a fraction of full black: 0 is black, 1 is invisible.
    pub grey: f64,
}

impl Watermark {
    pub fn colour(&self) -> (f64, f64, f64) {
        let level = self.grey.clamp(0.0, 1.0);
        (level, level, level)
    }

    pub fn describe(&self) -> String {
        format!(
            "\"{}\" across the sheet at {:.0} pt, turned {:.0}°, {:.0}% grey",
            self.text,
            self.size_pt,
            -self.rotation_deg,
            self.grey * 100.0
        )
    }
}

/// Lay a word across a sheet, corner to corner and centred.
///
/// `size_pt` and `grey`, when given, are somebody overruling the two things
/// this works out for itself.
pub fn across(
    text: &str,
    page: PageSize,
    font: Font,
    size_pt: Option<f64>,
    grey: Option<f64>,
) -> Option<Watermark> {
    let text = text.trim();
    if text.is_empty() || page.width_mm <= 0.0 || page.height_mm <= 0.0 {
        return None;
    }

    let rotation_deg = angle_of(page);

    let size_pt = match size_pt {
        // A size somebody asked for is taken at their word — but a word set at
        // nought points is no ink at all, and a delta of no ink is a sheet
        // through the printer for nothing. Refused here as well as at the
        // command line, so no caller can produce one.
        Some(given) if given.is_finite() && given > 0.0 => given,
        Some(_) => return None,
        None => biggest_that_fits(text, page, font)?,
    };

    let width_mm = builtin_width_mm(font, text, size_pt);
    let cap_mm = size_pt * CAP_SHARE * 25.4 / 72.0;

    // The middle of the paper, then back along the line by half the word, and
    // down by half a cap height so the letters straddle the centre rather than
    // hang from it.
    let radians = rotation_deg.to_radians();
    let (along_x, along_y) = (radians.cos(), radians.sin());
    // "Down" in the word's own frame, which is where a baseline sits relative
    // to the letters above it.
    let (down_x, down_y) = (-radians.sin(), radians.cos());

    Some(Watermark {
        text: text.to_string(),
        x_mm: page.width_mm / 2.0 - along_x * width_mm / 2.0 + down_x * cap_mm / 2.0,
        y_mm: page.height_mm / 2.0 - along_y * width_mm / 2.0 + down_y * cap_mm / 2.0,
        size_pt,
        rotation_deg,
        grey: grey.unwrap_or(GREY).clamp(0.0, 1.0),
    })
}

/// The largest type size whose word still fits on the paper, turned.
///
/// Sizing to the diagonal alone is the obvious answer and the wrong one: a word
/// is not a line, it is a band as tall as its letters, and a band laid across a
/// diagonal is wider than the diagonal. "DRAFT" at a size that spans A4's
/// 364 mm corner to corner stands 61 mm tall, and the box it really occupies is
/// 218 mm across a sheet 210 mm wide — so the D and the T print off the edges.
///
/// So the turned box is fitted instead. Both the word's width and its cap
/// height grow in proportion to the type size, so the largest size that fits is
/// arithmetic rather than a search:
///
/// ```text
///   across the paper : width·|cos θ| + height·|sin θ| ≤ the paper's width
///   down the paper   : width·|sin θ| + height·|cos θ| ≤ the paper's height
/// ```
fn biggest_that_fits(text: &str, page: PageSize, font: Font) -> Option<f64> {
    const AT: f64 = 10.0;
    let width_at = builtin_width_mm(font, text, AT);
    let cap_at = AT * CAP_SHARE * 25.4 / 72.0;
    if width_at <= 0.0 {
        return None;
    }
    let radians = angle_of(page).to_radians();
    let (down, along) = (radians.sin().abs(), radians.cos().abs());

    let by_width = page.width_mm * AT / (width_at * along + cap_at * down);
    let by_height = page.height_mm * AT / (width_at * down + cap_at * along);
    Some((by_width.min(by_height) * ACROSS).clamp(6.0, 400.0))
}

/// The way the word runs: along the paper's own diagonal.
///
/// Page space has y downwards and turns clockwise, so up-and-to-the-right — the
/// way every watermark anybody has seen runs — is a negative turn.
fn angle_of(page: PageSize) -> f64 {
    -(page.height_mm / page.width_mm).atan().to_degrees()
}

/// Whether a watermark this dark will make the printing under it hard to read.
///
/// Toner goes on top. Below about half black the words underneath survive it;
/// above that the sheet is being defaced rather than marked, which is
/// occasionally what somebody wants and never what they want by accident.
pub fn too_dark_to_read_through(grey: f64) -> bool {
    grey < 0.5
}

#[cfg(test)]
#[path = "watermark/tests.rs"]
mod tests;
