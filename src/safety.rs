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
