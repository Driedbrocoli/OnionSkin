//! Whether a delta belongs on the sheet in your hand, before any paper moves.
//!
//! Onionskin can already tell you afterwards. `verify` scans a sheet that has
//! been printed and says whether the additions landed where they were asked to
//! — which is exactly the right check, and exactly one sheet too late. The
//! failure it cannot help with is the one where the wrong form went in the
//! tray: the delta prints perfectly, onto a document it was never made for,
//! and the additions land across whatever that sheet happens to say.
//!
//! That mistake is cheap to make and expensive to hold. A stack of headed
//! paper, a run of numbered certificates, somebody's signed contract — the ink
//! does not come off, and there is no undo at a printer.
//!
//! So: hold the delta against a scan of the sheet and look at what is
//! underneath each addition. On the right sheet the additions land on clear
//! paper, because that is what an overlay is for. On the wrong one they land on
//! top of the text that is already there, and it shows immediately.
//!
//! # The sheet that has been through already
//!
//! There are two ways for an addition to land on ink, and they want opposite
//! things done. On the wrong sheet it lands on somebody else's text, and the
//! answer is to swap the sheet. On the *right* sheet, fed a second time, it
//! lands on itself — and the answer is to stop and think, because printing it
//! again lays every letter down twice in the same place, which comes out
//! heavier and a little blurred.
//!
//! They are told apart by how much ink is underneath. The wrong sheet has
//! whatever it happens to say there, in amounts that have nothing to do with
//! the addition, and most additions land on nothing at all. A sheet that has
//! been through has the addition's own ink under every one of them, plus
//! whatever the form had there to begin with — measurably more than the
//! addition alone, and measurably less than a black rectangle.
//!
//! It is said and not refused. A faint first pass is a real reason to want a
//! second, and there is nothing here that cannot be recovered from by looking
//! at the paper. The one thing that is not left to chance is a script feeding
//! two hundred sheets, which stops.
//!
//! # What this can and cannot say
//!
//! It can say the paper is a different size, and it can say an addition would
//! land on top of existing ink. Both are strong evidence and neither is proof:
//! a form filled in by hand may legitimately have an addition land beside
//! dense printing, and two different sheets of the same letterhead look alike
//! where the delta happens to fall. It reports what it found and how close
//! everything came, and leaves the last word to the person holding the paper.

use crate::calibrate::Asked;
use crate::geometry::PageSize;

/// One of the delta's additions, and what is under it on the sheet.
#[derive(Debug, Clone, PartialEq)]
pub struct Landing {
    /// Where the delta puts ink: x0, y0, x1, y1, in millimetres.
    pub box_mm: (f64, f64, f64, f64),
    /// How much of the sheet's own ink is under that box.
    ///
    /// Nought is what an overlay expects: clear paper, waiting for the words.
    pub under_mm2: f64,
    /// The nearest existing ink to this addition, in millimetres.
    ///
    /// Nought where the addition lands on top of something. Large where the
    /// addition sits in the middle of an empty half of the page.
    pub clearance_mm: f64,
    /// How much ink the delta itself puts in this box.
    ///
    /// The measure that tells the two collisions apart. An addition landing on
    /// the wrong sheet lands on whatever that sheet happens to say, and the
    /// amount of ink under it has nothing to do with the amount the addition
    /// would put down. An addition landing on a sheet that already carries this
    /// delta lands on *itself*, and the two amounts agree.
    pub asked_mm2: f64,
}

impl Landing {
    /// Whether this addition would land on something already printed.
    ///
    /// The threshold is the same one the safety checks use for "ink worth
    /// noticing": below it, a stray speck or the edge of a rule is being
    /// counted, and refusing a job over a speck teaches people to ignore the
    /// warning.
    pub fn lands_on_something(&self) -> bool {
        self.under_mm2 >= ON_TOP_MM2
    }

    /// Whether the ink under this addition looks like the addition itself.
    ///
    /// Not proof — a box of the right size full of somebody else's text would
    /// pass — but every addition on the sheet agreeing at once is a coincidence
    /// nobody should expect. On the wrong sheet the additions land where they
    /// land, and most of them land on nothing at all.
    pub fn looks_already_stamped(&self) -> bool {
        if self.asked_mm2 <= 0.0 || !self.lands_on_something() {
            return false;
        }
        let share = self.under_mm2 / self.asked_mm2;
        (ALREADY_THERE..=MUCH_MORE_THAN_ASKED).contains(&share)
    }

    pub fn describe(&self) -> String {
        let (x0, y0, x1, y1) = self.box_mm;
        if self.lands_on_something() {
            format!(
                "{:.0},{:.0} to {:.0},{:.0} mm — {:.0} mm² of the sheet's own ink is under this",
                x0, y0, x1, y1, self.under_mm2
            )
        } else {
            format!(
                "{:.0},{:.0} to {:.0},{:.0} mm — clear, {:.0} mm to the nearest ink",
                x0, y0, x1, y1, self.clearance_mm
            )
        }
    }
}

/// How much existing ink under an addition is worth calling a collision.
///
/// About two characters' worth. Below that it is a speck of scanner noise or
/// the edge of a printed rule, and a check that cries wolf is a check people
/// learn to pass over.
pub const ON_TOP_MM2: f64 = 1.5;

/// How much of an addition's own ink must already be under it before the sheet
/// counts as having been stamped already.
///
/// One-sided on purpose. A sheet that has been through carries the addition's
/// ink *and* whatever the form had there to begin with, so the amount
/// underneath is more than the addition alone, not equal to it.
///
/// Four fifths, and the number is measured rather than chosen. Held against a
/// form whose ruled lines run under the additions: the blank sheet gives 0.42
/// and 0.46 of each addition's ink, the wrong sheet 0.61, and the sheet that
/// has already been printed 1.33, 1.86 and 1.86. Four fifths sits in the gap
/// with room on both sides of it.
pub const ALREADY_THERE: f64 = 0.8;

/// And how much more than its own ink is too much to be that ink.
///
/// The lower bound alone lets a solid black rectangle stand in for a word:
/// there is certainly enough ink under the addition, and it is not the
/// addition. Text covers something under a third of the box it sits in, so a
/// block that fills the box is three or four times what the addition would
/// put down — while a sheet that really has been through gives 1.33 to 1.86,
/// the extra being the form's own rules underneath. Three sits above the one
/// and below the other.
pub const MUCH_MORE_THAN_ASKED: f64 = 3.0;

/// How far out the paper may be and still be the same paper.
///
/// A scan is measured, not declared, so a millimetre either way is the
/// measurement rather than the sheet.
pub const SAME_PAPER_MM: f64 = 3.0;

/// What holding the delta against the sheet showed.
#[derive(Debug, Clone)]
pub struct Fit {
    pub landings: Vec<Landing>,
    /// The paper the delta is drawn for.
    pub delta_page: PageSize,
    /// The paper the scan turned out to be.
    pub sheet_page: PageSize,
}

impl Fit {
    /// Whether the two are the same size of paper.
    pub fn paper_matches(&self) -> bool {
        (self.delta_page.width_mm - self.sheet_page.width_mm).abs() <= SAME_PAPER_MM
            && (self.delta_page.height_mm - self.sheet_page.height_mm).abs() <= SAME_PAPER_MM
    }

    /// The additions that would land on something already printed.
    pub fn collisions(&self) -> Vec<&Landing> {
        self.landings
            .iter()
            .filter(|landing| landing.lands_on_something())
            .collect()
    }

    /// Whether this looks like the sheet the delta was made for.
    pub fn belongs(&self) -> bool {
        self.paper_matches() && self.collisions().is_empty()
    }

    /// Whether the sheet already has this delta on it.
    ///
    /// Every addition landing on ink, and every one of them landing on about as
    /// much ink as it would itself put down. That is not what the wrong sheet
    /// looks like — the wrong sheet has its own text under the additions, in
    /// amounts that have nothing to do with them.
    ///
    /// Worth telling apart, because the two want opposite things done. The
    /// wrong sheet wants swapping. This one is the right sheet; it has simply
    /// been through already, and printing it again lays every letter down twice
    /// in the same place, which comes out heavier and blurred.
    ///
    /// It is said, not refused. Stamping a sheet twice is somebody\'s decision
    /// to make — a faint first pass is a real reason to want it — and there is
    /// nothing here that could not be recovered from by looking at the paper.
    pub fn already_stamped(&self) -> bool {
        !self.landings.is_empty()
            && self
                .landings
                .iter()
                .all(|landing| landing.looks_already_stamped())
    }

    /// The smallest gap between any addition and the ink already on the sheet.
    ///
    /// `None` when the delta has no additions to measure.
    pub fn tightest_mm(&self) -> Option<f64> {
        self.landings
            .iter()
            .filter(|landing| !landing.lands_on_something())
            .map(|landing| landing.clearance_mm)
            .fold(None, |tightest: Option<f64>, gap| {
                Some(match tightest {
                    Some(least) => least.min(gap),
                    None => gap,
                })
            })
    }

    /// What to tell somebody standing at the printer.
    pub fn describe(&self) -> String {
        let mut said = Vec::new();
        if !self.paper_matches() {
            said.push(format!(
                "The paper is not the same. The delta is drawn for {}, and the \
                 sheet measures {}.",
                self.delta_page.describe(),
                self.sheet_page.describe()
            ));
        }
        let collisions = self.collisions();
        // The sheet that has been through already, which looks like a pile of
        // collisions and is nothing of the sort.
        if self.already_stamped() && self.paper_matches() {
            said.push(match self.landings.len() {
                1 => "This sheet already has this delta on it. The addition is \
                      sitting where it was asked to go, in about the amount of \
                      ink it puts down."
                    .to_string(),
                all => format!(
                    "This sheet already has this delta on it. All {all} additions \
                     are sitting where they were asked to go, in about the amount \
                     of ink they put down."
                ),
            });
            said.push(
                "Printing it again lays every letter down twice in the same \
                 place, which comes out\n  heavier and a little blurred. That is \
                 allowed — a faint first pass is a real reason\n  to want it — but \
                 it is rarely what somebody meant. Check the sheet in your hand."
                    .to_string(),
            );
            return said.join("\n");
        }
        if !collisions.is_empty() {
            // Counted the way a person counts. "1 of the 1 addition would
            // land" is what arithmetic produces and nobody says.
            said.push(match (collisions.len(), self.landings.len()) {
                (1, 1) => {
                    "The addition would land on top of something already printed:".to_string()
                }
                (some, all) if some == all => {
                    format!("All {all} additions would land on top of something already printed:")
                }
                (some, all) => format!(
                    "{some} of the {all} additions would land on top of something \
                     already printed:"
                ),
            });
            for landing in &collisions {
                said.push(format!("    {}", landing.describe()));
            }
            said.push(
                "That is what happens when the wrong sheet is in the tray. Check \
                 it is the one this delta was made for."
                    .to_string(),
            );
        }
        if said.is_empty() {
            let clearance = match self.tightest_mm() {
                Some(gap) => format!(" The tightest sits {gap:.0} mm from the nearest ink."),
                None => String::new(),
            };
            said.push(match self.landings.len() {
                0 => "There is nothing on this delta to land anywhere.".to_string(),
                1 => format!("The addition lands on clear paper.{clearance}"),
                many => format!("All {many} additions land on clear paper.{clearance}"),
            });
        }
        said.join("\n")
    }
}

/// Hold a delta's additions against a sheet and see what is underneath them.
///
/// `gray` is the sheet as the scanner or renderer handed it over, already
/// squared onto the paper's own grid, at `dpi` dots to the inch. `asked` is
/// where the delta puts its ink, which [`crate::calibrate::marks_on_delta`]
/// works out from the delta itself.
pub fn against(
    asked: &[Asked],
    gray: &[u8],
    width: usize,
    dpi: f64,
    delta_page: PageSize,
    sheet_page: PageSize,
    ink_threshold: u8,
) -> Fit {
    let height = if width == 0 { 0 } else { gray.len() / width };
    let px_per_mm = dpi / 25.4;

    let landings = asked
        .iter()
        .map(|mark| {
            let (x0, y0, x1, y1) = mark.bounds_mm;
            let under_mm2 = ink_in(
                gray,
                width,
                height,
                px_per_mm,
                ink_threshold,
                mark.bounds_mm,
            );
            Landing {
                box_mm: (x0, y0, x1, y1),
                under_mm2,
                asked_mm2: mark.ink_mm2,
                // Only worth working out when nothing is underneath: the answer
                // for an addition sitting on top of something is nought, and
                // searching outwards for it would be a hundred times the work
                // for a number already known.
                clearance_mm: if under_mm2 >= ON_TOP_MM2 {
                    0.0
                } else {
                    nearest_ink_mm(
                        gray,
                        width,
                        height,
                        px_per_mm,
                        ink_threshold,
                        mark.bounds_mm,
                    )
                },
            }
        })
        .collect();

    Fit {
        landings,
        delta_page,
        sheet_page,
    }
}

/// How much ink the sheet has inside a box, in square millimetres.
fn ink_in(
    gray: &[u8],
    width: usize,
    height: usize,
    px_per_mm: f64,
    threshold: u8,
    box_mm: (f64, f64, f64, f64),
) -> f64 {
    let (x0, y0, x1, y1) = box_mm;
    let px = |mm: f64| (mm * px_per_mm).round().max(0.0) as usize;
    let (left, right) = (px(x0).min(width), px(x1).min(width));
    let (top, bottom) = (px(y0).min(height), px(y1).min(height));
    if left >= right || top >= bottom {
        return 0.0;
    }
    let dark = (top..bottom)
        .map(|y| {
            gray[y * width + left..y * width + right]
                .iter()
                .filter(|value| **value <= threshold)
                .count()
        })
        .sum::<usize>();
    dark as f64 / (px_per_mm * px_per_mm)
}

/// The distance from a box to the nearest ink outside it, in millimetres.
///
/// Searched in rings outwards rather than over the whole page, and stopped at
/// [`FAR_ENOUGH_MM`] — past that the answer is "nowhere near anything", and
/// the exact number is of no use to anybody.
fn nearest_ink_mm(
    gray: &[u8],
    width: usize,
    height: usize,
    px_per_mm: f64,
    threshold: u8,
    box_mm: (f64, f64, f64, f64),
) -> f64 {
    const FAR_ENOUGH_MM: f64 = 25.0;
    let (x0, y0, x1, y1) = box_mm;
    let px = |mm: f64| (mm * px_per_mm).round().max(0.0) as isize;
    let (left, right) = (px(x0), px(x1));
    let (top, bottom) = (px(y0), px(y1));

    // A millimetre at a time: finer than a person can act on, and coarse
    // enough that a page settles in a moment.
    let step = px_per_mm.max(1.0) as isize;
    let mut out = step;
    while (out as f64 / px_per_mm) <= FAR_ENOUGH_MM {
        let ring_left = left - out;
        let ring_right = right + out;
        let ring_top = top - out;
        let ring_bottom = bottom + out;
        let inked = |x: isize, y: isize| -> bool {
            if x < 0 || y < 0 || x >= width as isize || y >= height as isize {
                return false;
            }
            gray[y as usize * width + x as usize] <= threshold
        };
        let mut found = false;
        for x in ring_left..=ring_right {
            if inked(x, ring_top) || inked(x, ring_bottom) {
                found = true;
                break;
            }
        }
        if !found {
            for y in ring_top..=ring_bottom {
                if inked(ring_left, y) || inked(ring_right, y) {
                    found = true;
                    break;
                }
            }
        }
        if found {
            return out as f64 / px_per_mm;
        }
        out += step;
    }
    FAR_ENOUGH_MM
}

#[cfg(test)]
#[path = "fits/tests.rs"]
mod tests;
