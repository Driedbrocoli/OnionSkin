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
    if checks.iter().any(|check| check.code == "documents_reversed") {
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
        let x0 = crate::geometry::mm_to_px(region.x0_mm, dpi).floor().max(0.0) as usize;
        let y0 = crate::geometry::mm_to_px(region.y0_mm, dpi).floor().max(0.0) as usize;
        let x1 = (crate::geometry::mm_to_px(region.x1_mm, dpi).ceil().max(0.0) as usize).min(width);
        let y1 = (crate::geometry::mm_to_px(region.y1_mm, dpi).ceil().max(0.0) as usize).min(height);
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
        "Run 'onionskin calibrate target' once per printer to bring that under \
         ±0.5 mm."
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

#[cfg(test)]
mod legibility_tests {
    use super::*;
    use crate::diff::{Mask, PageDiff, Region};
    use crate::calibrate::A4;

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
