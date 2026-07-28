//! The back of the sheet.
//!
//! Every printed sheet has two sides and Onionskin has only ever been able to
//! reach one of them. That is a real gap rather than a tidy one: "continued
//! overleaf", the terms on the back of an invoice, an address block on the
//! reverse of a compliment slip, a barcode where the filing system wants it —
//! all of them are on the side the delta could not address.
//!
//! # The two cases, which are not the same problem
//!
//! **The document is already two-sided.** A twenty-page report printed on ten
//! sheets: sheet three's back is page six. Onionskin can already put words on
//! page six; what it could not do was say so, or warn that a delta with words on
//! an even page prints to nothing useful unless it goes through two-sided, the
//! same way round as the original. That is arithmetic and a sentence.
//!
//! **The document is one-sided and the backs are blank.** This is the hard one,
//! and it is the commoner one. There is no page six to write on — there is a
//! stack of paper with nothing on the reverse, and it has to go through the
//! printer a second time.
//!
//! # Which way up the back comes out
//!
//! When a stack goes back into the tray for its second side, the back comes out
//! one of two ways, and which one depends on the printer, not on anything
//! Onionskin can see:
//!
//!   * [`Feed::SameWayUp`] — turn the sheet over like the page of a book and the
//!     back is the right way up. This is what a printer doing its own two-sided
//!     printing on the long edge produces, and what most people expect.
//!   * [`Feed::TurnedAround`] — the back comes out with its top at the other end
//!     of the paper. Turn the sheet over like a book and the words are upside
//!     down.
//!
//! There is no way to know which without trying, so Onionskin does not guess: it
//! prints [`a_test_sheet`], somebody looks at it, and the answer is remembered.
//! A guess here is not a small error. It is every sheet in the run printed upside
//! down, found after the run.
//!
//! Given the answer, everything else is one rotation: a word meant for 20 mm
//! from the left and 40 mm down **as somebody looks at the back the right way
//! up** is written at the diagonally opposite point, turned half a turn. Which is
//! [`turn_a_placement`], and is the whole of the difference.

use crate::geometry::PageSize;
use crate::pdf::{Font, LineFont, PlacedLine};

/// Which side of a sheet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Front,
    Back,
}

impl Side {
    pub fn describe(self) -> &'static str {
        match self {
            Side::Front => "front",
            Side::Back => "back",
        }
    }
}

/// Which way up the back comes out when the stack goes through again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Feed {
    /// Turn the sheet over like the page of a book and the back is upright.
    #[default]
    SameWayUp,
    /// The back's top is at the other end of the paper.
    TurnedAround,
}

impl Feed {
    pub fn parse(name: &str) -> Option<Feed> {
        Some(match name.trim().to_ascii_lowercase().as_str() {
            "same" | "same-way-up" | "book" | "long" | "long-edge" => Feed::SameWayUp,
            "turned" | "turned-around" | "calendar" | "short" | "short-edge" => Feed::TurnedAround,
            _ => return None,
        })
    }

    pub fn key(self) -> &'static str {
        match self {
            Feed::SameWayUp => "same",
            Feed::TurnedAround => "turned",
        }
    }

    pub fn describe(self) -> &'static str {
        match self {
            Feed::SameWayUp => {
                "the back comes out the same way up as the front — turn the \
                 sheet over like the page of a book and it reads upright"
            }
            Feed::TurnedAround => {
                "the back comes out with its top at the other end of the paper \
                 — turn the sheet over like the page of a book and it reads \
                 upside down"
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Which page is which side of which sheet
// ---------------------------------------------------------------------------

/// The page of a two-sided document that is this side of this sheet.
///
/// Sheets and pages both counted from one, the way anybody holding them would.
/// Sheet three's front is page five and its back is page six.
pub fn page_of(sheet: usize, side: Side) -> usize {
    let sheet = sheet.max(1);
    match side {
        Side::Front => sheet * 2 - 1,
        Side::Back => sheet * 2,
    }
}

/// The other way round: which sheet, and which side of it, a page is.
pub fn sheet_and_side(page: usize) -> (usize, Side) {
    let page = page.max(1);
    match page % 2 {
        1 => (page.div_ceil(2), Side::Front),
        _ => (page / 2, Side::Back),
    }
}

/// How many sheets a two-sided document of this many pages comes to.
///
/// An odd page count means the last sheet's back is blank, which is worth
/// counting as a sheet: it is paper that went through the printer.
pub fn sheets_for(pages: usize) -> usize {
    pages.div_ceil(2)
}

// ---------------------------------------------------------------------------
// Turning a placement for a back that comes out the other way up
// ---------------------------------------------------------------------------

/// Where a placement really goes, given how the paper comes back.
///
/// Somebody says "20 mm from the left, 40 mm down" meaning what they will see
/// when they hold the finished back the right way up. If the paper comes back
/// turned around, that point is at the diagonally opposite corner of the sheet
/// as the printer sees it, and everything at it is half a turn round.
///
/// Returned as the same three numbers the rest of the program places things
/// with, so nothing downstream has to know that any of this happened.
pub fn turn_a_placement(
    x_mm: f64,
    y_mm: f64,
    rotation_deg: f64,
    page: PageSize,
    feed: Feed,
) -> (f64, f64, f64) {
    match feed {
        Feed::SameWayUp => (x_mm, y_mm, rotation_deg),
        Feed::TurnedAround => (
            page.width_mm - x_mm,
            page.height_mm - y_mm,
            rotation_deg + 180.0,
        ),
    }
}

/// The same, for a rectangle — which has a size as well as a corner.
///
/// A box's top-left corner becomes its bottom-right when the sheet is turned, so
/// the corner that is given back is the one the far corner used to be.
pub fn turn_a_box(
    x_mm: f64,
    y_mm: f64,
    width_mm: f64,
    height_mm: f64,
    page: PageSize,
    feed: Feed,
) -> (f64, f64, f64, f64) {
    match feed {
        Feed::SameWayUp => (x_mm, y_mm, width_mm, height_mm),
        Feed::TurnedAround => (
            page.width_mm - x_mm - width_mm,
            page.height_mm - y_mm - height_mm,
            width_mm,
            height_mm,
        ),
    }
}

// ---------------------------------------------------------------------------
// Finding out which way the paper comes back
// ---------------------------------------------------------------------------

/// How far in from the top the test sheet's word sits, in millimetres.
///
/// Well clear of the unprintable border, and far enough from the middle that
/// nobody has to think about which half of the sheet it landed in.
const TEST_INSET_MM: f64 = 25.0;

/// A sheet that answers the question, printed on the back of one piece of paper.
///
/// One word near the top edge and one near the bottom, each saying which edge it
/// was *meant* for. Print it on the back of a sheet, turn the sheet over like the
/// page of a book, and read whichever word is now at the top.
///
/// Two words rather than one on purpose. A single word at the top is ambiguous
/// to somebody holding a sheet — top of what, before or after turning it over —
/// and the whole point of the sheet is to remove a question, not move it.
pub fn a_test_sheet(page: PageSize) -> Vec<PlacedLine> {
    let size_pt = 24.0;
    let line = |text: &str, y_mm: f64, rotation_deg: f64| PlacedLine {
        text: text.to_string(),
        x_mm: page.width_mm / 2.0
            - crate::pdf::builtin_width_mm(Font::HelveticaBold, text, size_pt) / 2.0,
        y_mm,
        size_pt,
        font: LineFont::Builtin(Font::HelveticaBold),
        colour: (0.0, 0.0, 0.0),
        rotation_deg,
    };
    let note = |text: &str, y_mm: f64| PlacedLine {
        text: text.to_string(),
        x_mm: page.width_mm / 2.0 - crate::pdf::builtin_width_mm(Font::Helvetica, text, 10.0) / 2.0,
        y_mm,
        size_pt: 10.0,
        font: LineFont::Builtin(Font::Helvetica),
        colour: (0.0, 0.0, 0.0),
        rotation_deg: 0.0,
    };
    vec![
        line("SAME", TEST_INSET_MM, 0.0),
        note(
            "If this word is at the top, the feed is: same",
            TEST_INSET_MM + 10.0,
        ),
        // Upside down at the far end, so that whichever way the sheet comes out
        // exactly one of the two is readable and at the top.
        line("TURNED", page.height_mm - TEST_INSET_MM, 180.0),
        note_turned(
            "If this word is at the top, the feed is: turned",
            page,
            page.height_mm - TEST_INSET_MM - 10.0,
        ),
    ]
}

/// The small print of the second word, upside down with it.
fn note_turned(text: &str, page: PageSize, y_mm: f64) -> PlacedLine {
    PlacedLine {
        text: text.to_string(),
        x_mm: page.width_mm / 2.0 + crate::pdf::builtin_width_mm(Font::Helvetica, text, 10.0) / 2.0,
        y_mm,
        size_pt: 10.0,
        font: LineFont::Builtin(Font::Helvetica),
        colour: (0.0, 0.0, 0.0),
        rotation_deg: 180.0,
    }
}

/// What to do with the test sheet, in the order somebody does it.
pub const HOW_TO_USE_THE_TEST_SHEET: &str =
    "  1. Print one sheet of the document, the ordinary way.
  2. Take it out, put it back in the tray the way you would to print the
     other side, and print this delta onto it.
  3. Hold the sheet with the front the right way up, and turn it over
     sideways — the way you turn the page of a book.
  4. One of the two words is now at the top and readable. That word is the
     answer.

Then say so once and Onionskin will remember it:
  onionskin config set feed same
  onionskin config set feed turned";

/// What the word somebody read means.
///
/// Taking the word itself rather than a yes or no, because the word is what is
/// in front of them and translating it into a yes is where the mistake gets
/// made.
pub fn what_the_word_means(word: &str) -> Option<Feed> {
    match word.trim().to_ascii_uppercase().as_str() {
        "SAME" => Some(Feed::SameWayUp),
        "TURNED" => Some(Feed::TurnedAround),
        _ => None,
    }
}

/// What has to be true at the printer for a two-sided delta to land right.
///
/// Said rather than assumed, because getting it wrong is not a crooked sheet —
/// it is the whole run printed on the wrong sides, and no way to tell until it
/// comes out.
pub const PRINT_IT_THE_SAME_WAY: &str = "\
This delta has words on the back of a sheet. Print it two-sided, and the same \
way round as the\n  original was printed — long edge if that was long edge, \
short edge if it was short. A delta\n  printed one-sided puts the backs onto \
fresh paper, and a delta printed the other way round\n  puts them upside down.";

#[cfg(test)]
#[path = "duplex/tests.rs"]
mod tests;
