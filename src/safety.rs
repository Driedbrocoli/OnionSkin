//! Checks that run before a sheet goes back in the tray.
//!
//! Every one of these exists to stop a specific way of wasting paper, and the
//! first is the important one. Adding a word mid-paragraph pushes everything
//! after it down the page. The delta then contains not just the new word but
//! the re-flowed remainder, which cannot be printed onto a sheet whose text is
//! still in the old position. Detecting that is worth more than any amount of
//! sub-millimetre accuracy.

use crate::diff::PageDiff;
use crate::geometry::{PageSize, Similarity};

/// Default non-printable border. Most inkjets cannot place ink within about
/// 5 mm of any edge; many lasers need more at the trailing edge.
pub const DEFAULT_MARGIN_MM: f64 = 5.0;

/// Enough displaced ink to mean a line moved, rather than a stray anti-aliased
/// pixel. Roughly the footprint of two characters.
pub const REFLOW_INK_MM2: f64 = 1.5;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Nothing will be printed until this is resolved.
    Blocker,
    /// Printing will probably work, but here is what to look at.
    Warning,
    /// Worth knowing, and nothing to do about it.
    Note,
}

impl Severity {
    pub fn label(&self) -> &'static str {
        match self {
            Severity::Blocker => "BLOCKER",
            Severity::Warning => "WARNING",
            Severity::Note => "note",
        }
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Check {
    pub severity: Severity,
    pub code: &'static str,
    pub message: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub detail: String,
    /// Which page, counted from 1. `None` when it is about the whole job.
    pub page: Option<usize>,
}

impl Check {
    fn new(severity: Severity, code: &'static str, message: String) -> Check {
        Check {
            severity,
            code,
            message,
            detail: String::new(),
            page: None,
        }
    }

    fn with_detail(mut self, detail: String) -> Check {
        self.detail = detail;
        self
    }

    fn on_page(mut self, page: usize) -> Check {
        self.page = Some(page);
        self
    }

    /// Something worth knowing that no check went looking for.
    ///
    /// Other parts of the program find things a person ought to be told — how
    /// a document was opened, what a reader could not do — and this is how they
    /// join the list. One list means one place for every interface to read
    /// from, rather than the command line printing something the window does
    /// not.
    pub fn note(code: &'static str, message: String, detail: String) -> Check {
        Check::new(Severity::Note, code, message).with_detail(detail)
    }

    pub fn format(&self) -> String {
        let where_ = match self.page {
            Some(page) => format!(" [page {page}]"),
            None => String::new(),
        };
        let line = format!("{}{where_}: {}", self.severity.label(), self.message);
        if self.detail.is_empty() {
            line
        } else {
            format!("{line}\n    {}", self.detail)
        }
    }
}

/// Worst first, then in page order — the order someone wants to read them.
pub fn sort_checks(checks: &mut [Check]) {
    checks.sort_by(|a, b| {
        a.severity
            .cmp(&b.severity)
            .then(a.page.unwrap_or(0).cmp(&b.page.unwrap_or(0)))
            .then(a.code.cmp(b.code))
    });
}

/// Do the two documents even describe the same sheets of paper?
pub fn check_documents(original: &[PageSize], edited: &[PageSize]) -> Vec<Check> {
    let mut checks = Vec::new();

    if edited.len() > original.len() {
        let extra = edited.len() - original.len();
        checks.push(
            Check::new(
                Severity::Warning,
                "pages_added",
                format!("The edit added {extra} page(s)."),
            )
            .with_detail(format!(
                "The original is {} page(s), the edit is {}. The extra page(s) have \
                 no printed sheet to go onto — print those on blank paper.",
                original.len(),
                edited.len()
            )),
        );
    } else if edited.len() < original.len() {
        checks.push(
            Check::new(
                Severity::Blocker,
                "pages_removed",
                "The edit has fewer pages than the original.".into(),
            )
            .with_detail(format!(
                "{} → {} pages. Content was removed or pulled onto earlier pages, so \
                 the printed sheets no longer match the document. Print a fresh copy.",
                original.len(),
                edited.len()
            )),
        );
    }

    for (index, (old, new)) in original.iter().zip(edited.iter()).enumerate() {
        if !old.matches(new, 0.5) {
            checks.push(
                Check::new(
                    Severity::Blocker,
                    "page_size_mismatch",
                    "Page size changed between the two documents.".into(),
                )
                .with_detail(format!(
                    "original {} vs edited {}. Nothing will line up. Check the page \
                     setup in both files.",
                    old.describe(),
                    new.describe()
                ))
                .on_page(index + 1),
            );
        }
    }
    checks
}

/// Drop the checks that a bigger one has already explained.
///
/// When the two documents were handed over the wrong way round, every page
/// reports reflow — all the added text is "missing" from the document that was
/// called the original. That is true and it is not the problem, and the advice
/// attached to it, about Word text boxes and fixed positions, is about a
/// problem this person does not have. One clear cause beats five true symptoms.
pub fn drop_the_symptoms(checks: &mut Vec<Check>) {
    if checks
        .iter()
        .any(|check| check.code == "documents_reversed")
    {
        checks.retain(|check| check.code != "reflow" && check.code != "heavy_coverage");
    }
}

/// The reflow alarm: ink that is gone from where it was.
pub fn check_reflow(diff: &PageDiff) -> Vec<Check> {
    let removed = diff.removed_ink_mm2();
    if removed < REFLOW_INK_MM2 {
        return Vec::new();
    }
    let top = diff
        .removed_regions
        .iter()
        .map(|r| r.y0_mm)
        .fold(f64::INFINITY, f64::min);
    let top = if top.is_finite() { top } else { 0.0 };

    vec![Check::new(
        Severity::Blocker,
        "reflow",
        "Existing content moved or was deleted on this page.".into(),
    )
    .with_detail(format!(
        "{:.0} mm² of ink is gone from where it was, starting {:.0} mm down the \
         page. The printed sheet no longer matches the document, so an overlay \
         cannot fix it — print this page fresh.\n    To add text without disturbing \
         the layout, put it in a Word text box set to 'Fixed position on page' with \
         no text wrapping.",
        removed, top
    ))
    .on_page(diff.index + 1)]
}

/// The pages the reflow check reported, after any dropping.
///
/// The authority on which pages moved, rather than the raw measurement: two
/// documents handed over the wrong way round have ink missing from every page,
/// [`drop_the_symptoms`] recognises that as one mistake and takes the reflow
/// findings away, and anything acting on "which pages moved" has to see that.
pub fn pages_that_moved(checks: &[Check]) -> Vec<usize> {
    checks
        .iter()
        .filter(|check| check.code == "reflow")
        .filter_map(|check| check.page)
        .collect()
}

/// Whether moved text is the only thing standing in the way.
///
/// Splitting the job answers exactly one objection: that some of these pages
/// cannot be overprinted. It answers none of the others. A document that lost a
/// page, or changed paper size, or was handed over the wrong way round, is not
/// printable onto the sheets somebody has whichever pages are taken out of it —
/// and splitting it anyway writes a "print these fresh" file for a job that
/// nobody should print, which reads as a plan and is not one.
pub fn only_moved_text_blocks(checks: &[Check]) -> bool {
    checks
        .iter()
        .filter(|check| check.severity == Severity::Blocker)
        .all(|check| check.code == "reflow")
}

/// The reflow blockers, once those pages have been taken out of the delta and
/// written out to be printed fresh instead.
///
/// The finding was right and stays on the list — a sheet is being thrown away,
/// which is worth knowing. What is no longer true is that nothing can be
/// printed: the pages that did not move are still an overlay, and holding
/// thirty-nine of them back because one moved is the thing this undoes.
pub fn reflow_is_handled(checks: &mut Vec<Check>, detail: String) {
    let moved: Vec<usize> = checks
        .iter()
        .filter(|check| check.code == "reflow")
        .filter_map(|check| check.page)
        .collect();
    if moved.is_empty() {
        return;
    }
    checks.retain(|check| check.code != "reflow");
    // The one spelling of a page list, so this warning and the instructions
    // sitting directly beneath it do not say "2, 4" and "2 and 4" about the
    // same two sheets.
    let pages = crate::split::sheets(&moved);
    checks.push(
        Check::new(
            Severity::Warning,
            "reflow_split",
            format!(
                "Existing content moved on {} {pages}, so {} cannot be \
                 overprinted. The job has been split.",
                if moved.len() == 1 { "page" } else { "pages" },
                if moved.len() == 1 { "it" } else { "they" },
            ),
        )
        .with_detail(detail),
    );
}

/// Additions that stray into the border a printer cannot reach.
pub fn check_margins(diff: &PageDiff, margin_mm: f64) -> Vec<Check> {
    if margin_mm <= 0.0 || diff.added_regions.is_empty() {
        return Vec::new();
    }
    let size = diff.size;
    let offenders: Vec<_> = diff
        .added_regions
        .iter()
        .filter(|region| {
            region.x0_mm < margin_mm
                || region.y0_mm < margin_mm
                || region.x1_mm > size.width_mm - margin_mm
                || region.y1_mm > size.height_mm - margin_mm
        })
        .collect();
    if offenders.is_empty() {
        return Vec::new();
    }

    let worst = offenders
        .iter()
        .map(|r| {
            r.x0_mm
                .min(r.y0_mm)
                .min(size.width_mm - r.x1_mm)
                .min(size.height_mm - r.y1_mm)
        })
        .fold(f64::INFINITY, f64::min);

    vec![Check::new(
        Severity::Warning,
        "margin",
        format!(
            "{} addition(s) sit inside the {} mm non-printable border.",
            offenders.len(),
            trim_number(margin_mm)
        ),
    )
    .with_detail(format!(
        "The closest comes within {:.1} mm of an edge. Most printers will clip or \
         refuse to print it. Move it inward, or lower --margin if you know this \
         printer goes closer.",
        worst.max(0.0)
    ))
    .on_page(diff.index + 1)]
}

/// How much of what is underneath an addition may be dark before the addition
/// stops being readable.
///
/// A word crossing a ruled line covers very little of its own box and prints
/// perfectly well. A word on top of a logo covers most of it, and does not.
/// A third of the way between is where it stops being worth the sheet.
pub const UNREADABLE_FRACTION: f64 = 0.35;

/// Resolution to look at the page underneath.
///
/// Deliberately coarse. The question is "is there a dark blob here", not
/// "which letter is this", and asking it at printing resolution would double
/// the rendering the delta already costs to answer something a thumbnail
/// settles. At this size a whole page is about half a megabyte.
pub const BENEATH_DPI: f64 = 100.0;

/// Will the additions actually be readable where they land?
///
/// A printer adds toner; it cannot take it away. Black words printed onto a
/// black logo are invisible, and there is no way to find that out except by
/// printing the sheet — unless something already knows what is on it.
/// Onionskin does know, because it has the page in order to compare against
/// it.
///
/// `beneath` is the source page in greyscale at `dpi`, which need not be —
/// and for the sake of the time it takes, should not be — the resolution the
/// delta itself was measured at.
pub fn check_legibility(
    diff: &PageDiff,
    beneath: &[u8],
    width: usize,
    dpi: f64,
    ink_threshold: u8,
) -> Vec<Check> {
    if diff.added_regions.is_empty() || width == 0 || beneath.is_empty() || dpi <= 0.0 {
        return Vec::new();
    }
    let height = beneath.len() / width;

    let mut worst: Option<(crate::diff::Region, f64)> = None;
    let mut offenders = 0usize;
    for region in &diff.added_regions {
        // The region is in millimetres on the paper, so it maps onto any
        // rendering of that paper whatever resolution it was drawn at.
        let x0 = crate::geometry::mm_to_px(region.x0_mm, dpi)
            .floor()
            .max(0.0) as usize;
        let y0 = crate::geometry::mm_to_px(region.y0_mm, dpi)
            .floor()
            .max(0.0) as usize;
        let x1 = (crate::geometry::mm_to_px(region.x1_mm, dpi).ceil().max(0.0) as usize).min(width);
        let y1 =
            (crate::geometry::mm_to_px(region.y1_mm, dpi).ceil().max(0.0) as usize).min(height);
        if x0 >= x1 || y0 >= y1 {
            continue;
        }

        let mut dark = 0usize;
        let mut seen = 0usize;
        for y in y0..y1 {
            for value in &beneath[y * width + x0..y * width + x1] {
                if *value < ink_threshold {
                    dark += 1;
                }
                seen += 1;
            }
        }
        if seen == 0 {
            continue;
        }
        let fraction = dark as f64 / seen as f64;
        if fraction >= UNREADABLE_FRACTION {
            offenders += 1;
            // Not `is_none_or`, which needs a newer Rust than this builds on.
            let this_is_worse = match worst {
                Some((_, had)) => fraction > had,
                None => true,
            };
            if this_is_worse {
                worst = Some((*region, fraction));
            }
        }
    }

    let Some((region, fraction)) = worst else {
        return Vec::new();
    };
    vec![Check::new(
        Severity::Warning,
        "legibility",
        format!("{offenders} addition(s) land on ink that is already there."),
    )
    .with_detail(format!(
        "The worst is at {:.0},{:.0} mm, where {:.0}% of the paper underneath \
         is already dark. A printer can only add toner, never take it away, so \
         black on black stays black. Move it onto clear paper, or print the \
         whole sheet again instead of adding to it.",
        region.x0_mm,
        region.y0_mm,
        fraction * 100.0
    ))
    .on_page(diff.index + 1)]
}

/// How far a printer may be out before the additions start touching things.
///
/// Two millimetres is what an uncalibrated sheet-fed printer does on a second
/// pass. An addition with more clear paper than that around it is safe whatever
/// the printer does; one with less is a job where calibration stops being a
/// nicety and starts being the difference between a filled-in form and a
/// ruined sheet.
pub const REGISTRATION_MM: f64 = 2.0;

/// The furthest out it is worth looking for something already on the paper.
///
/// Past this the answer is "nothing near it" and the exact number stops
/// meaning anything — knowing an addition has 9 mm of clear paper rather than
/// 12 changes no decision anybody makes.
const LOOK_MM: f64 = 8.0;

/// How much clear paper each addition has above and below it.
///
/// The calibration warning says the printer may be two millimetres out. What it
/// cannot say is whether that matters *here*, and the answer is usually no: a
/// signature going into a wide empty box does not care, and the same printer
/// filling a ruled column cares very much. That difference is the whole
/// question behind "calibrate before this job or just print it", and the page
/// already holds the answer.
///
/// # Why only above and below
///
/// Measuring in every direction sounds more thorough and is useless. Filling in
/// a form means writing directly after a label — "Name:" and then the name,
/// with a millimetre between them — so the nearest ink sideways is nearly
/// always something the addition is *deliberately* next to, and a check that
/// fires on every filled-in form is a check nobody reads.
///
/// Up and down is where a collision ruins the sheet. An addition that slips two
/// millimetres down lands on the ruled line under it, or on the next row of the
/// table, and comes out unreadable. Two millimetres sideways moves it along its
/// own empty line and usually nobody can tell.
pub fn check_slack(
    diff: &PageDiff,
    beneath: &[u8],
    width: usize,
    dpi: f64,
    ink_threshold: u8,
) -> Vec<Check> {
    let Some((region, slack_mm)) = tightest_fit(diff, beneath, width, dpi, ink_threshold) else {
        return Vec::new();
    };

    if slack_mm >= REGISTRATION_MM {
        let how = if slack_mm >= LOOK_MM {
            format!("more than {LOOK_MM:.0} mm")
        } else {
            format!("{slack_mm:.1} mm")
        };
        return vec![Check::new(
            Severity::Note,
            "room_to_spare",
            format!("The tightest addition has {how} of clear paper above and below it."),
        )
        .with_detail(format!(
            "More than the ±{REGISTRATION_MM:.0} mm an uncalibrated printer is out \
             by, so this sheet will come out right whether or not this printer \
             has ever been measured."
        ))
        .on_page(diff.index + 1)];
    }

    vec![Check::new(
        Severity::Warning,
        "tight_fit",
        format!("One addition has only {slack_mm:.1} mm of clear paper above or below it."),
    )
    .with_detail(format!(
        "It is at {:.0},{:.0} mm. An uncalibrated printer is out by about \
         ±{REGISTRATION_MM:.0} mm on a second pass, which is more than that gap, so \
         this one can land on the line above or below it.\n    Calibrating brings it \
         under half a millimetre: print this delta, scan the sheet, and run\n      \
         onionskin calibrate learn scan.png --delta <this delta>",
        region.x0_mm, region.y0_mm
    ))
    .on_page(diff.index + 1)]
}

/// The addition with the least clear paper above or below it, and how much.
fn tightest_fit(
    diff: &PageDiff,
    beneath: &[u8],
    width: usize,
    dpi: f64,
    ink_threshold: u8,
) -> Option<(crate::diff::Region, f64)> {
    if diff.added_regions.is_empty() || width == 0 || beneath.is_empty() || dpi <= 0.0 {
        return None;
    }
    let height = beneath.len() / width;
    let mut tightest: Option<(crate::diff::Region, f64)> = None;

    for region in &diff.added_regions {
        let up = clear_above(region, beneath, width, dpi, ink_threshold);
        let down = clear_below(region, beneath, width, height, dpi, ink_threshold);
        let slack = up.min(down);
        let this_is_tighter = match tightest {
            Some((_, had)) => slack < had,
            None => true,
        };
        if this_is_tighter {
            tightest = Some((*region, slack));
        }
    }
    tightest
}

/// How far above this region the paper is clear, in millimetres.
fn clear_above(
    region: &crate::diff::Region,
    beneath: &[u8],
    width: usize,
    dpi: f64,
    ink_threshold: u8,
) -> f64 {
    let px = |mm: f64| crate::geometry::mm_to_px(mm, dpi);
    let x0 = px(region.x0_mm).floor().max(0.0) as usize;
    let x1 = (px(region.x1_mm).ceil().max(0.0) as usize).min(width);
    let top = px(region.y0_mm).floor().max(0.0) as usize;
    if x0 >= x1 || top == 0 {
        return LOOK_MM;
    }
    let stop = top.saturating_sub(px(LOOK_MM).ceil() as usize);
    for y in (stop..top).rev() {
        if beneath[y * width + x0..y * width + x1]
            .iter()
            .any(|value| *value < ink_threshold)
        {
            return crate::geometry::px_to_mm((top - y) as f64, dpi);
        }
    }
    LOOK_MM
}

/// How far below this region the paper is clear, in millimetres.
fn clear_below(
    region: &crate::diff::Region,
    beneath: &[u8],
    width: usize,
    height: usize,
    dpi: f64,
    ink_threshold: u8,
) -> f64 {
    let px = |mm: f64| crate::geometry::mm_to_px(mm, dpi);
    let x0 = px(region.x0_mm).floor().max(0.0) as usize;
    let x1 = (px(region.x1_mm).ceil().max(0.0) as usize).min(width);
    let bottom = (px(region.y1_mm).ceil().max(0.0) as usize).min(height);
    if x0 >= x1 || bottom >= height {
        return LOOK_MM;
    }
    let stop = (bottom + px(LOOK_MM).ceil() as usize).min(height);
    for y in bottom..stop {
        if beneath[y * width + x0..y * width + x1]
            .iter()
            .any(|value| *value < ink_threshold)
        {
            return crate::geometry::px_to_mm((y - bottom) as f64, dpi);
        }
    }
    LOOK_MM
}

/// A delta covering a lot of the page usually means something reflowed in a way
/// the ink test did not catch.
pub fn check_coverage(diff: &PageDiff) -> Vec<Check> {
    let page_mm2 = diff.size.width_mm * diff.size.height_mm;
    let added = diff.added_ink_mm2();
    if page_mm2 <= 0.0 || added <= 0.0 {
        return Vec::new();
    }
    let fraction = added / page_mm2;
    if fraction < 0.06 {
        return Vec::new();
    }
    vec![Check::new(
        Severity::Warning,
        "large_delta",
        format!("The delta covers {:.0}% of this page.", fraction * 100.0),
    )
    .with_detail(
        "That is a lot of new ink for an overlay. If this is not what you expected, \
         the layout probably shifted — compare the preview against the sheet before \
         printing."
            .into(),
    )
    .on_page(diff.index + 1)]
}

/// The two documents render identically.
pub fn check_empty(diffs: &[PageDiff]) -> Vec<Check> {
    if diffs.iter().any(|d| d.has_additions()) {
        return Vec::new();
    }

    // Nothing was added and something was taken away. That is not a document
    // somebody edited badly — it is the same edit read backwards, which is
    // what `onionskin delta new.pdf old.pdf` produces. Saying "the two
    // documents render identically" would be plainly untrue, and the reflow
    // advice that follows is about a problem this person does not have.
    let removed: f64 = diffs.iter().map(|d| d.removed_ink_mm2()).sum();
    if removed > 0.0 {
        return vec![Check::new(
            Severity::Blocker,
            "documents_reversed",
            "Nothing was added, and something was taken away — these two \
             documents look like they were given the wrong way round."
                .into(),
        )
        .with_detail(format!(
            "{removed:.0} mm² of ink is in the first document and not in the \
             second, and none is the other way about. The first one is the \
             document as it was printed; the second is the edited copy. Try them \
             in the other order.\n\nIf they really are in the right order, then \
             the edit only removed things, and ink cannot be taken off paper — \
             that page has to be printed fresh."
        ))];
    }

    vec![Check::new(
        Severity::Blocker,
        "empty_delta",
        "No additions found — the delta would print a blank page.".into(),
    )
    .with_detail(
        "The two documents render identically. Check you passed the edited file \
         second, and that the edit was saved."
            .into(),
    )]
}

pub fn check_calibration(calibrated: bool, profile_name: Option<&str>) -> Vec<Check> {
    if calibrated {
        return vec![Check::new(
            Severity::Note,
            "calibrated",
            format!(
                "Calibration profile '{}' applied.",
                profile_name.unwrap_or("?")
            ),
        )];
    }
    vec![Check::new(
        Severity::Note,
        "uncalibrated",
        "No calibration profile — expect roughly ±2 mm of registration error.".into(),
    )
    .with_detail(
        // Two routes, easiest first. Somebody about to feed a sheet back in is
        // exactly the person who can calibrate without doing anything extra:
        // the sheet they are about to print is the measurement.
        "Print this delta, scan the sheet afterwards, and run 'onionskin calibrate \
         learn scan.png --delta <this file>'.\n    That measures the printer off \
         the job itself. Or do it up front, once, with 'onionskin calibrate \
         target'. Either brings it under ±0.5 mm."
            .into(),
    )]
}

/// Warn when a profile is used on a sheet size it was not measured on.
///
/// A pure shift carries over to any paper: the paper path pushes every sheet
/// the same way. Rotation and scale do not — Onionskin applies them about the
/// centre of the page, so measuring on A4 and printing on A5 pivots the
/// correction around a different point and leaves error behind.
pub fn check_profile_page(
    profile_name: &str,
    measured: PageSize,
    error: Similarity,
    page: PageSize,
) -> Vec<Check> {
    if measured.matches(&page, 2.0) {
        return Vec::new();
    }
    if error.rotation_deg.abs() < 5e-3 && (error.scale - 1.0).abs() < 5e-6 {
        // A shift alone transfers cleanly to any sheet size.
        return Vec::new();
    }

    let drift = (page.width_mm - measured.width_mm)
        .abs()
        .max((page.height_mm - measured.height_mm).abs());

    vec![Check::new(
        Severity::Warning,
        "profile_page_mismatch",
        format!(
            "Profile '{profile_name}' was measured on {}, but this page is {}.",
            measured.describe(),
            page.describe()
        ),
    )
    .with_detail(format!(
        "Its rotation and scale are applied about the centre of the page, and the \
         centre has moved by up to {:.0} mm. The shift still applies, but expect \
         some of the rotation and scale correction to be off. Calibrate again on \
         this paper size for the best result.",
        drift / 2.0
    ))]
}

/// Every check, in the order they should be read.
pub fn check_all(
    diffs: &[PageDiff],
    original_sizes: &[PageSize],
    edited_sizes: &[PageSize],
    margin_mm: f64,
    calibrated: bool,
    profile_name: Option<&str>,
) -> Vec<Check> {
    let mut checks = check_documents(original_sizes, edited_sizes);
    for diff in diffs {
        checks.extend(check_reflow(diff));
        checks.extend(check_margins(diff, margin_mm));
        checks.extend(check_coverage(diff));
    }
    checks.extend(check_empty(diffs));
    checks.extend(check_calibration(calibrated, profile_name));
    sort_checks(&mut checks);
    checks
}

pub fn has_blockers(checks: &[Check]) -> bool {
    checks.iter().any(|c| c.severity == Severity::Blocker)
}

/// `5` rather than `5.0`, but `4.5` still `4.5`.
fn trim_number(value: f64) -> String {
    if (value - value.round()).abs() < 1e-9 {
        format!("{}", value.round() as i64)
    } else {
        format!("{value}")
    }
}

#[cfg(test)]
mod tests;

// ---------------------------------------------------------------------------
// Words placed by millimetre, on a delta written without a diff
// ---------------------------------------------------------------------------

/// Whether words placed by hand land on the paper at all.
///
/// Most of this program works out what is new by comparing two renderings, and
/// [`check_margins`] looks at that comparison. A few commands do not: `back`
/// writes its delta straight, because there is nothing on a blank reverse to
/// compare against. Those had no check at all, and it showed —
/// `back --at '300,400:x'` on A4 wrote a perfectly good delta with the words a
/// clear nine centimetres off the side of the sheet, said "3 additions", and
/// left somebody to find out at the printer.
///
/// Off the paper is refused rather than warned about, because there is no
/// version of that which is what somebody meant. Close to an edge is a warning,
/// because there is.
pub fn check_placements(
    page: PageSize,
    lines: &[crate::pdf::PlacedLine],
    margin_mm: f64,
) -> Vec<Check> {
    let mut checks = Vec::new();
    for line in lines {
        let (left, top, right, bottom) = extent_of(page, line);
        if right <= 0.0 || bottom <= 0.0 || left >= page.width_mm || top >= page.height_mm {
            checks.push(
                Check::new(
                    Severity::Blocker,
                    "off-the-paper",
                    format!(
                        "'{}' is placed at {:.0},{:.0} mm, which is off the {} sheet altogether.",
                        line.text,
                        line.x_mm,
                        line.y_mm,
                        page.describe()
                    ),
                )
                .with_detail(
                    "Nothing of it would print. Measure from the top-left corner of the paper."
                        .to_string(),
                ),
            );
            continue;
        }
        if left < margin_mm
            || top < margin_mm
            || right > page.width_mm - margin_mm
            || bottom > page.height_mm - margin_mm
        {
            checks.push(
                Check::new(
                    Severity::Warning,
                    "near-the-edge",
                    format!(
                        "'{}' comes within {margin_mm:.0} mm of the edge of the paper.",
                        line.text
                    ),
                )
                .with_detail(
                    "Most printers cannot put ink right to the edge, so part of it may be cut off."
                        .to_string(),
                ),
            );
        }
    }
    checks
}

/// The box a line of type occupies, turned if it is turned.
///
/// Only the four corners of the upright box, moved: enough to tell whether a
/// line is on the paper, which is the question being asked. A tighter box would
/// need the font's own outlines and would not change any answer here.
fn extent_of(page: PageSize, line: &crate::pdf::PlacedLine) -> (f64, f64, f64, f64) {
    let width = match line.font {
        crate::pdf::LineFont::Builtin(font) => {
            crate::pdf::builtin_width_mm(font, &line.text, line.size_pt)
        }
        // An embedded font's widths are not to hand here. Its own em is a fair
        // stand-in, and erring wide is the safe way to err for a check about
        // running off an edge.
        crate::pdf::LineFont::Embedded => {
            line.text.chars().count() as f64 * line.size_pt * 25.4 / 72.0 * 0.6
        }
    };
    let cap = line.size_pt * 0.7 * 25.4 / 72.0;
    let radians = line.rotation_deg.to_radians();
    let (along, up) = (
        (radians.cos(), radians.sin()),
        (radians.sin(), -radians.cos()),
    );
    let corner = |a: f64, u: f64| {
        (
            line.x_mm + along.0 * a + up.0 * u,
            line.y_mm + along.1 * a + up.1 * u,
        )
    };
    let corners = [
        corner(0.0, 0.0),
        corner(width, 0.0),
        corner(width, cap),
        corner(0.0, cap),
    ];
    let _ = page;
    (
        corners.iter().map(|c| c.0).fold(f64::MAX, f64::min),
        corners.iter().map(|c| c.1).fold(f64::MAX, f64::min),
        corners.iter().map(|c| c.0).fold(f64::MIN, f64::max),
        corners.iter().map(|c| c.1).fold(f64::MIN, f64::max),
    )
}

#[cfg(test)]
mod legibility_tests {
    use super::*;
    use crate::calibrate::A4;
    use crate::diff::{Mask, PageDiff, Region};

    /// A page of the given size, all paper, with one dark rectangle on it.
    fn page_with_a_blot(dpi: f64, blot: (f64, f64, f64, f64)) -> (Vec<u8>, usize) {
        let (w, h) = A4.px_size(dpi);
        let (w, h) = (w as usize, h as usize);
        let mut gray = vec![255u8; w * h];
        let px = |mm: f64| crate::geometry::mm_to_px(mm, dpi).round() as usize;
        for y in px(blot.1)..px(blot.3).min(h) {
            for x in px(blot.0)..px(blot.2).min(w) {
                gray[y * w + x] = 0;
            }
        }
        (gray, w)
    }

    /// A diff whose only addition is the given box, in millimetres.
    fn diff_adding(area: (f64, f64, f64, f64)) -> PageDiff {
        PageDiff {
            index: 0,
            size: A4,
            dpi: 400.0,
            added: Mask::blank(1, 1),
            removed: Mask::blank(1, 1),
            added_px: 1,
            removed_px: 0,
            added_regions: vec![Region {
                x0_mm: area.0,
                y0_mm: area.1,
                x1_mm: area.2,
                y1_mm: area.3,
                ink_mm2: 1.0,
                px_bbox: (0, 0, 1, 1),
            }],
            removed_regions: Vec::new(),
        }
    }

    // -----------------------------------------------------------------
    // How much room the additions have
    // -----------------------------------------------------------------

    /// A signature going into a wide empty box does not care what the printer
    /// does, and saying so is what stops calibration being a chore somebody
    /// does before every job whether it matters or not.
    #[test]
    fn an_addition_with_room_around_it_says_the_printer_does_not_matter() {
        let (gray, width) = page_with_a_blot(BENEATH_DPI, (40.0, 100.0, 90.0, 110.0));
        // Forty millimetres clear of the blot in every direction.
        let diff = diff_adding((40.0, 160.0, 90.0, 170.0));
        let checks = check_slack(&diff, &gray, width, BENEATH_DPI, 128);
        assert_eq!(checks.len(), 1, "{checks:?}");
        assert_eq!(checks[0].severity, Severity::Note);
        assert_eq!(checks[0].code, "room_to_spare");
        assert!(
            checks[0].detail.contains("whether or not"),
            "{:?}",
            checks[0]
        );
        assert!(
            checks[0].message.contains("above and below"),
            "{:?}",
            checks[0]
        );
    }

    /// Writing directly after a label is what filling in a form *is*, and a
    /// check that fires on it is a check nobody reads. The label is beside the
    /// answer on purpose.
    #[test]
    fn words_written_right_after_a_label_are_not_complained_about() {
        // "Name:" ending where the answer begins, with clear paper above and
        // below both.
        let (gray, width) = page_with_a_blot(BENEATH_DPI, (20.0, 112.0, 49.8, 120.0));
        let diff = diff_adding((50.0, 112.0, 120.0, 120.0));
        let checks = check_slack(&diff, &gray, width, BENEATH_DPI, 128);
        assert_eq!(checks.len(), 1, "{checks:?}");
        assert_eq!(
            checks[0].code, "room_to_spare",
            "an ordinary filled-in form was warned about: {checks:?}"
        );
    }

    /// The same printer filling a ruled column cares very much, and that is the
    /// job worth calibrating before rather than after.
    #[test]
    fn an_addition_in_a_tight_gap_says_to_calibrate_first() {
        // A ruled line a millimetre under where the words go.
        let (gray, width) = page_with_a_blot(BENEATH_DPI, (40.0, 121.0, 160.0, 121.6));
        let diff = diff_adding((50.0, 112.0, 120.0, 120.0));
        let checks = check_slack(&diff, &gray, width, BENEATH_DPI, 128);
        assert_eq!(checks.len(), 1, "{checks:?}");
        assert_eq!(checks[0].severity, Severity::Warning);
        assert_eq!(checks[0].code, "tight_fit");
        assert!(
            checks[0].message.contains("above or below"),
            "{:?}",
            checks[0]
        );
        // And says what to do about it rather than only that it is a problem.
        assert!(
            checks[0].detail.contains("calibrate learn"),
            "{:?}",
            checks[0]
        );
    }

    /// The tightest one is the one worth reporting: a job is as safe as its
    /// worst addition, not as its average.
    #[test]
    fn the_tightest_addition_is_the_one_reported() {
        let (gray, width) = page_with_a_blot(BENEATH_DPI, (40.0, 121.0, 160.0, 121.6));
        let mut diff = diff_adding((50.0, 112.0, 120.0, 120.0));
        // A second addition with the whole bottom of the page to itself.
        diff.added_regions.push(Region {
            x0_mm: 40.0,
            y0_mm: 220.0,
            x1_mm: 90.0,
            y1_mm: 230.0,
            ink_mm2: 1.0,
            px_bbox: (0, 0, 1, 1),
        });
        let checks = check_slack(&diff, &gray, width, BENEATH_DPI, 128);
        assert_eq!(checks[0].code, "tight_fit", "the roomy one won: {checks:?}");
    }

    #[test]
    fn nothing_to_measure_says_nothing() {
        let (gray, width) = page_with_a_blot(BENEATH_DPI, (40.0, 100.0, 90.0, 110.0));
        let mut diff = diff_adding((40.0, 160.0, 90.0, 170.0));
        diff.added_regions.clear();
        assert!(check_slack(&diff, &gray, width, BENEATH_DPI, 128).is_empty());

        let diff = diff_adding((40.0, 160.0, 90.0, 170.0));
        assert!(check_slack(&diff, &[], 0, BENEATH_DPI, 128).is_empty());
        assert!(check_slack(&diff, &gray, width, 0.0, 128).is_empty());
    }

    #[test]
    fn words_landing_on_a_dark_logo_are_reported_before_the_sheet_is_wasted() {
        // A printer can only add toner. Black on black stays black, and the
        // only other way to find that out is to print it.
        let (gray, width) = page_with_a_blot(BENEATH_DPI, (40.0, 100.0, 90.0, 130.0));
        let diff = diff_adding((50.0, 110.0, 80.0, 120.0));
        let checks = check_legibility(&diff, &gray, width, BENEATH_DPI, 128);
        assert_eq!(checks.len(), 1, "{checks:?}");
        assert_eq!(checks[0].severity, Severity::Warning);
        assert!(checks[0].detail.contains("already dark"), "{:?}", checks[0]);
        // And says where, so it can be moved.
        assert!(checks[0].detail.contains("50,110"), "{:?}", checks[0]);
    }

    #[test]
    fn words_on_clear_paper_say_nothing_at_all() {
        let (gray, width) = page_with_a_blot(BENEATH_DPI, (40.0, 100.0, 90.0, 130.0));
        // Well away from the blot.
        let diff = diff_adding((120.0, 200.0, 160.0, 210.0));
        assert!(check_legibility(&diff, &gray, width, BENEATH_DPI, 128).is_empty());
    }

    #[test]
    fn a_word_merely_crossing_a_ruled_line_is_not_complained_about() {
        // The ordinary case of filling in a form: the words sit on the line
        // they are meant to sit on, and print perfectly well. Warning about
        // that would make the check noise, and noise gets turned off.
        let (gray, width) = page_with_a_blot(BENEATH_DPI, (40.0, 119.5, 160.0, 120.0));
        let diff = diff_adding((50.0, 112.0, 120.0, 120.0));
        assert!(
            check_legibility(&diff, &gray, width, BENEATH_DPI, 128).is_empty(),
            "a word on a ruled line was complained about"
        );
    }

    #[test]
    fn nothing_to_check_is_not_an_error() {
        let (gray, width) = page_with_a_blot(BENEATH_DPI, (0.0, 0.0, 10.0, 10.0));
        let mut diff = diff_adding((50.0, 110.0, 80.0, 120.0));
        diff.added_regions.clear();
        assert!(check_legibility(&diff, &gray, width, BENEATH_DPI, 128).is_empty());
        // And neither is having no page to look at.
        let diff = diff_adding((50.0, 110.0, 80.0, 120.0));
        assert!(check_legibility(&diff, &[], 0, BENEATH_DPI, 128).is_empty());
        assert!(check_legibility(&diff, &gray, width, 0.0, 128).is_empty());
    }

    #[test]
    fn a_region_running_off_the_page_is_clipped_rather_than_panicking() {
        let (gray, width) = page_with_a_blot(BENEATH_DPI, (0.0, 0.0, 210.0, 297.0));
        let diff = diff_adding((190.0, 280.0, 400.0, 500.0));
        // All dark, so it warns — the point is that it does not panic.
        let checks = check_legibility(&diff, &gray, width, BENEATH_DPI, 128);
        assert_eq!(checks.len(), 1, "{checks:?}");
    }
}
