//! Which of these documents is this sheet?
//!
//! A stack comes back from the scanner as a pile of files with names like
//! `Scan_0007`, and somewhere on a disk are the documents they were printed
//! from. Matching them up is somebody's afternoon, done by opening each scan,
//! squinting at it, and remembering which invoice looked like that.
//!
//! The page can be asked instead. Two printings of the same document put ink in
//! the same places, and two different documents almost never do — so a coarse
//! map of where the ink is, on the paper's own grid, tells them apart without
//! reading a single word.
//!
//! # Why a coarse map and not the words
//!
//! Reading the page would be the obvious approach and is the wrong one here.
//! It needs a font on the machine, it takes a second a page, and it is beaten
//! by a smudge. Where the ink *is* survives all of that: a scan at 150 dpi and
//! a render at 400 dpi give the same map, because the map is measured in
//! millimetres of paper rather than in pixels.
//!
//! It also means this works on a document nobody can read — a form in a script
//! with no font installed, a page of diagrams, a letterhead in a language the
//! machine has never seen.
//!
//! # What it cannot do
//!
//! It cannot tell two filled-in copies of the same form apart, because they
//! *are* the same document with different words on them, and the map is
//! deliberately too coarse to see the difference. That is a feature for filing
//! ("this is one of the March invoices") and a limit for identifying ("which
//! March invoice"). It says how sure it is, and says so plainly when the answer
//! is a guess.

use std::path::{Path, PathBuf};

use crate::geometry::PageSize;

/// How many cells across and down the page is divided into.
///
/// Twenty-four by thirty-four is about a centimetre a cell on A4 — fine enough
/// that a letterhead, a table and a signature block are in different cells,
/// coarse enough that a word moving by a millimetre does not change the answer.
pub const ACROSS: usize = 24;
pub const DOWN: usize = 34;

/// Where the ink is on a page, coarsely, in a form two pages can be compared in.
#[derive(Debug, Clone, PartialEq)]
pub struct Signature {
    /// How much of each cell is inked, from 0 to 1, row by row from the top.
    pub cells: Vec<f32>,
    /// How much of the whole page is inked, which distinguishes a dense page
    /// from a sparse one with the same shape.
    pub density: f32,
}

impl Signature {
    /// Take the map of a page, however it was drawn or scanned.
    ///
    /// Measured against the paper rather than the image, so the same document
    /// scanned at 150 dpi and rendered at 400 gives the same answer.
    pub fn of(gray: &[u8], width: usize, dpi: f64, page: PageSize, threshold: u8) -> Signature {
        let mut cells = vec![0f32; ACROSS * DOWN];
        if width == 0 || gray.is_empty() || dpi <= 0.0 {
            return Signature {
                cells,
                density: 0.0,
            };
        }
        let height = gray.len() / width;
        if height == 0 || page.width_mm <= 0.0 || page.height_mm <= 0.0 {
            return Signature {
                cells,
                density: 0.0,
            };
        }

        let px_per_mm = dpi / 25.4;
        let cell_w_mm = page.width_mm / ACROSS as f64;
        let cell_h_mm = page.height_mm / DOWN as f64;
        let mut counted = vec![0u32; ACROSS * DOWN];
        let mut inked = vec![0u32; ACROSS * DOWN];

        for y in 0..height {
            let mm_y = y as f64 / px_per_mm;
            let row = (mm_y / cell_h_mm) as usize;
            if row >= DOWN {
                continue;
            }
            for x in 0..width {
                let mm_x = x as f64 / px_per_mm;
                let column = (mm_x / cell_w_mm) as usize;
                if column >= ACROSS {
                    continue;
                }
                let at = row * ACROSS + column;
                counted[at] += 1;
                if gray[y * width + x] <= threshold {
                    inked[at] += 1;
                }
            }
        }

        let mut total_inked = 0u64;
        let mut total_counted = 0u64;
        for at in 0..cells.len() {
            total_inked += inked[at] as u64;
            total_counted += counted[at] as u64;
            if counted[at] > 0 {
                cells[at] = inked[at] as f32 / counted[at] as f32;
            }
        }
        Signature {
            cells,
            density: if total_counted > 0 {
                total_inked as f32 / total_counted as f32
            } else {
                0.0
            },
        }
    }

    /// How unlike another page this one is, from 0 (the same) to 1 (nothing in
    /// common).
    ///
    /// Both maps are turned into a share of the page's ink before they are
    /// compared — what fraction of everything printed falls in each cell —
    /// rather than compared as raw darkness.
    ///
    /// That matters more than it sounds. An ordinary page of eleven-point text
    /// is nearly all paper: a couple of percent ink, most cells empty. Compared
    /// raw, an invoice and a letter come out four thousandths apart, which is
    /// a real difference buried under a much larger sameness — they are both
    /// mostly white, and being mostly white is not evidence of anything. As
    /// shares, the same two pages are most of the way to 1, because their ink
    /// is in different places, which is the only thing being asked.
    ///
    /// It also makes a light scan and a dark one the same page, which they are.
    pub fn distance(&self, other: &Signature) -> f64 {
        if self.cells.len() != other.cells.len() || self.cells.is_empty() {
            return f64::INFINITY;
        }
        let (Some(mine), Some(theirs)) = (self.shares(), other.shares()) else {
            // One of them has no ink at all, so there is no shape to compare.
            return f64::INFINITY;
        };
        // Half the total difference between two distributions: 0 when they put
        // their ink in the same places, 1 when they have no cell in common.
        mine.iter()
            .zip(&theirs)
            .map(|(a, b)| (a - b).abs())
            .sum::<f64>()
            / 2.0
    }

    /// Each cell's share of all the ink on the page, summing to 1.
    ///
    /// `None` for a page with nothing on it, which has no shape to speak of.
    fn shares(&self) -> Option<Vec<f64>> {
        let total: f64 = self.cells.iter().map(|cell| *cell as f64).sum();
        if total <= 0.0 {
            return None;
        }
        Some(self.cells.iter().map(|cell| *cell as f64 / total).collect())
    }

    /// Whether there is enough ink here to say anything at all.
    ///
    /// A blank sheet matches every other blank sheet perfectly, which is true
    /// and useless. Saying so is better than naming a winner at random.
    pub fn worth_comparing(&self) -> bool {
        self.density > 0.0005
    }
}

/// One document that was offered, and how unlike the sheet it is.
#[derive(Debug, Clone)]
pub struct Candidate {
    pub path: PathBuf,
    pub distance: f64,
    /// Where opening it went wrong, when it did. Reported rather than dropped:
    /// a document silently missing from the running is one somebody will go on
    /// believing was considered.
    pub unreadable: Option<String>,
}

/// How far apart two pages can be and still be the same document.
///
/// On the scale [`Signature::distance`] measures — 0 for ink in the same
/// places, 1 for nothing in common — a scan of a printed page against a render
/// of the document it came from lands around a tenth, and two different
/// documents land past a half. The gap between those is wide, which is what
/// makes so coarse a map work at all, and this sits in the middle of it.
pub const THE_SAME_DOCUMENT: f64 = 0.30;

/// How much further away the runner-up must be before the winner is trusted.
///
/// Absolute, not a ratio. A ratio is meaningless close to nought: two
/// candidates at 0.01 and 0.02 are "twice as far apart" and both are the same
/// page as far as anything here can tell, so calling the first one the answer
/// would be picking between two identical things and sounding sure about it.
///
/// A near-tie means they look alike, not that the first is right — two months
/// of the same invoice template, most likely. "It is one of these two" is both
/// the honest answer and the useful one.
pub const CLEARLY_BETTER: f64 = 0.12;

/// What comparing a sheet against a pile of documents came to.
#[derive(Debug, Clone)]
pub struct Ranking {
    /// Every candidate, least unlike first. Unreadable ones come last.
    pub ranked: Vec<Candidate>,
    /// Whether the sheet had enough ink on it to compare at all.
    pub sheet_worth_comparing: bool,
}

impl Ranking {
    /// The closest document, if any could be read.
    pub fn best(&self) -> Option<&Candidate> {
        self.ranked
            .iter()
            .find(|candidate| candidate.unreadable.is_none())
    }

    /// The next closest, which is what decides whether the best means anything.
    pub fn runner_up(&self) -> Option<&Candidate> {
        self.ranked
            .iter()
            .filter(|candidate| candidate.unreadable.is_none())
            .nth(1)
    }

    /// Whether the answer is worth acting on without looking at the sheet.
    ///
    /// Three things have to hold: the sheet has ink on it, the winner is close
    /// enough to be the same document at all, and it is clearly better than
    /// whatever came second.
    pub fn confident(&self) -> bool {
        if !self.sheet_worth_comparing {
            return false;
        }
        let Some(best) = self.best() else {
            return false;
        };
        if best.distance > THE_SAME_DOCUMENT {
            return false;
        }
        match self.runner_up() {
            Some(second) => second.distance - best.distance >= CLEARLY_BETTER,
            // Nothing to be confused with.
            None => true,
        }
    }

    pub fn describe(&self) -> String {
        if !self.sheet_worth_comparing {
            return "There is almost no ink on that sheet, so there is nothing to \
                    recognise it by."
                .to_string();
        }
        let Some(best) = self.best() else {
            // Two different nothings, and telling somebody the documents could
            // not be opened when they offered none would send them looking at
            // files that are perfectly all right.
            return if self.ranked.is_empty() {
                "No documents were offered to compare that sheet against.".to_string()
            } else {
                format!(
                    "None of the {} document(s) offered could be opened:\n{}",
                    self.ranked.len(),
                    self.ranked
                        .iter()
                        .map(|candidate| format!(
                            "  {}  ({})",
                            candidate.path.display(),
                            candidate.unreadable.as_deref().unwrap_or("unreadable")
                        ))
                        .collect::<Vec<_>>()
                        .join("\n")
                )
            };
        };

        let mut said = if self.confident() {
            vec![format!("This is {}.", best.path.display())]
        } else if best.distance > THE_SAME_DOCUMENT {
            vec![format!(
                "None of these looks like that sheet. The closest is {}, and it \
                 is not close.",
                best.path.display()
            )]
        } else {
            vec![format!(
                "This looks like {}, but {} is nearly as close — they are alike \
                 enough that the sheet does not settle it.",
                best.path.display(),
                self.runner_up()
                    .map(|second| second.path.display().to_string())
                    .unwrap_or_default()
            )]
        };

        said.push(String::new());
        said.push("How unlike each one it is, closest first:".to_string());
        for candidate in &self.ranked {
            said.push(match &candidate.unreadable {
                Some(why) => format!("  {:>8}  {}  ({why})", "—", candidate.path.display()),
                None => format!(
                    "  {:>8.4}  {}",
                    candidate.distance,
                    candidate.path.display()
                ),
            });
        }
        said.join("\n")
    }
}

/// Rank a pile of documents by how much they look like this sheet.
///
/// `sheet` is the scan or render to identify, already squared onto the paper's
/// own grid. `draw` opens each candidate and hands back its first page the same
/// way — passed in rather than called directly so this stays testable without a
/// renderer, and so the caller decides what counts as openable.
pub fn among<F>(sheet: &Signature, candidates: &[PathBuf], mut draw: F) -> Ranking
where
    F: FnMut(&Path) -> Result<Signature, String>,
{
    let mut ranked: Vec<Candidate> = candidates
        .iter()
        .map(|path| match draw(path) {
            Ok(theirs) => Candidate {
                path: path.clone(),
                distance: sheet.distance(&theirs),
                unreadable: None,
            },
            Err(why) => Candidate {
                path: path.clone(),
                distance: f64::INFINITY,
                unreadable: Some(why),
            },
        })
        .collect();

    // Readable ones first, closest first among them. An unreadable candidate
    // sorts last rather than anywhere its infinite distance happens to land.
    ranked.sort_by(|a, b| {
        a.unreadable.is_some().cmp(&b.unreadable.is_some()).then(
            a.distance
                .partial_cmp(&b.distance)
                .unwrap_or(std::cmp::Ordering::Equal),
        )
    });

    Ranking {
        ranked,
        sheet_worth_comparing: sheet.worth_comparing(),
    }
}

#[cfg(test)]
#[path = "which/tests.rs"]
mod tests;

// ---------------------------------------------------------------------------
// A whole stack
// ---------------------------------------------------------------------------

/// One sheet of a scanned stack, and which document it turned out to be.
#[derive(Debug, Clone)]
pub struct Placed {
    /// Counted from 1, the way somebody talks about the fourth sheet.
    pub sheet: usize,
    pub ranking: Ranking,
}

impl Placed {
    /// The document this sheet belongs with, where the answer is worth acting
    /// on without somebody looking at the paper.
    pub fn settled(&self) -> Option<&Candidate> {
        self.ranking
            .confident()
            .then(|| self.ranking.best())
            .flatten()
    }

    /// One line, for a list somebody reads down.
    pub fn line(&self) -> String {
        let sheet = self.sheet;
        match self.settled() {
            Some(best) => format!("  sheet {sheet:>3}  {}", best.path.display()),
            None => format!(
                "  sheet {sheet:>3}  ?  {}",
                self.ranking
                    .best()
                    .map(|best| format!(
                        "closest is {} at {:.4}",
                        best.path.display(),
                        best.distance
                    ))
                    .unwrap_or_else(|| "nothing to compare it with".to_string())
            ),
        }
    }
}

/// What sorting a stack came to.
#[derive(Debug, Clone)]
pub struct Sorted {
    pub placed: Vec<Placed>,
    /// The documents the sheets were compared against.
    pub among: Vec<PathBuf>,
}

impl Sorted {
    /// The sheets nobody should file without looking at them.
    ///
    /// A sheet filed under the wrong document is worse than one left in the
    /// pile, which is why these are named rather than guessed at.
    pub fn unplaced(&self) -> Vec<usize> {
        self.placed
            .iter()
            .filter(|placed| placed.settled().is_none())
            .map(|placed| placed.sheet)
            .collect()
    }

    pub fn all_placed(&self) -> bool {
        self.unplaced().is_empty()
    }

    pub fn lines(&self) -> Vec<String> {
        self.placed.iter().map(Placed::line).collect()
    }

    pub fn verdict(&self) -> String {
        let unplaced = self.unplaced();
        if unplaced.is_empty() {
            return "Every sheet was placed.".to_string();
        }
        format!(
            "{} of the {} sheet{} could not be placed — {} {}.\n  \
             Look at those yourself — a sheet filed under the wrong document is\n  \
             worse than one left in the pile.",
            unplaced.len(),
            self.placed.len(),
            if self.placed.len() == 1 { "" } else { "s" },
            if unplaced.len() == 1 {
                "sheet"
            } else {
                "sheets"
            },
            crate::split::sheets(&unplaced)
        )
    }
}

/// Ask every sheet of a scanned stack which document it is.
///
/// The single place that knows how a stack is sorted, so the command line and
/// the window cannot drift apart on it.
///
/// `saying` is called with each sheet as it starts, because forty sheets
/// against ten documents takes long enough that a silent program looks like a
/// stopped one.
pub fn sort_a_stack(
    scan: &Path,
    among: &[PathBuf],
    saying: &mut dyn FnMut(usize, usize),
) -> Result<Sorted, String> {
    let threshold = crate::diff::DiffOptions::default().ink_threshold;
    let sheets = crate::recipe::pages_in(scan)?;
    if sheets == 0 {
        return Err(format!("there are no pages in '{}'.", scan.display()));
    }

    // The candidates are drawn once, not once a sheet. Forty sheets against
    // ten documents would otherwise open and render the same ten files forty
    // times.
    let mut drawn: Vec<(PathBuf, Result<Signature, String>)> = Vec::new();
    for path in among {
        let signature = crate::recipe::draw_page(path, 1).map(|(gray, registration)| {
            let width = gray.width() as usize;
            Signature::of(
                gray.as_raw(),
                width,
                registration.px_per_mm * 25.4,
                registration.page,
                threshold,
            )
        });
        drawn.push((path.clone(), signature));
    }

    let mut placed = Vec::new();
    for sheet in 1..=sheets {
        saying(sheet, sheets);
        let (gray, registration) = crate::recipe::draw_page(scan, sheet)?;
        let width = gray.width() as usize;
        let signature = Signature::of(
            gray.as_raw(),
            width,
            registration.px_per_mm * 25.4,
            registration.page,
            threshold,
        );
        let ranking = among_drawn(&signature, among, &drawn);
        placed.push(Placed { sheet, ranking });
    }

    Ok(Sorted {
        placed,
        among: among.to_vec(),
    })
}

/// The ranking for one sheet, against candidates that have already been drawn.
fn among_drawn(
    sheet: &Signature,
    candidates: &[PathBuf],
    drawn: &[(PathBuf, Result<Signature, String>)],
) -> Ranking {
    among(sheet, candidates, |path| {
        drawn
            .iter()
            .find(|(candidate, _)| candidate == path)
            .map(|(_, signature)| signature.clone())
            .unwrap_or_else(|| Err("it was not drawn".to_string()))
    })
}
