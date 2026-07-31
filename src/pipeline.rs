//! End to end: two documents in, one printable delta PDF out.

use std::path::{Path, PathBuf};

use crate::calibrate::{self, Profile};
use crate::delta::{
    apply_correction, build_vector_delta, conform_to_source, preview_page, Outline,
    RasterDeltaWriter,
};
use crate::diff::{diff_page, DiffOptions, PageDiff};
use crate::geometry::{PageSize, Similarity};
use crate::render::{self, RenderError, Workspace};
use crate::safety::{self, Check};

pub const DEFAULT_DPI: f64 = 400.0;

/// How the delta is built.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// Exactly the pixels that are new. Never re-prints existing ink.
    Raster,
    /// The edited PDF clipped to the changed regions. Crisper, but a clip box
    /// is a rectangle.
    Vector,
}

impl Mode {
    pub fn parse(text: &str) -> Option<Mode> {
        match text.trim().to_ascii_lowercase().as_str() {
            "raster" => Some(Mode::Raster),
            "vector" => Some(Mode::Vector),
            _ => None,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Mode::Raster => "raster",
            Mode::Vector => "vector",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Options {
    pub dpi: f64,
    pub mode: Mode,
    pub margin_mm: f64,
    pub profile: Option<String>,
    pub diff: DiffOptions,
    pub pad_mm: f64,
    pub preview_dir: Option<PathBuf>,
    /// Draw a box round each change as well as printing it.
    ///
    /// `None` — the default — prints only the new ink, which is the whole
    /// point of a delta. Somebody checking an edit wants the boxes; somebody
    /// producing a finished page does not, and on a delta the difference is
    /// permanent, because it is printed onto the paper.
    pub outline: Option<Outline>,
    /// Where to write the pages that cannot be overprinted, whole.
    ///
    /// Asking for this is asking for the job to be split: pages whose existing
    /// text moved are blanked in the delta and written here instead, so the
    /// pages that *can* be overprinted are not held back by the ones that
    /// cannot. See [`crate::split`].
    pub fresh: Option<PathBuf>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            dpi: DEFAULT_DPI,
            mode: Mode::Raster,
            margin_mm: safety::DEFAULT_MARGIN_MM,
            profile: None,
            diff: DiffOptions::default(),
            pad_mm: 0.3,
            preview_dir: None,
            outline: None,
            fresh: None,
        }
    }
}

impl Options {
    pub fn validate(&self) -> Result<(), PipelineError> {
        if !(50.0..=1200.0).contains(&self.dpi) {
            return Err(PipelineError::Invalid(
                "dpi must be between 50 and 1200".into(),
            ));
        }
        if self.diff.ink_threshold == 0 || self.diff.ink_threshold == 255 {
            return Err(PipelineError::Invalid(
                "ink-threshold must be between 1 and 254".into(),
            ));
        }
        if !self.margin_mm.is_finite() || self.margin_mm < 0.0 {
            return Err(PipelineError::Invalid(
                "the margin must be zero or more millimetres".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PipelineError {
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Render(#[from] RenderError),
    #[error("{0}")]
    Delta(#[from] crate::delta::DeltaError),
    #[error("{0}")]
    Calibrate(#[from] crate::calibrate::CalibrateError),
    #[error("could not write {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{0}")]
    Image(#[from] image::ImageError),
}

/// What a run produced.
#[derive(Debug)]
pub struct Outcome {
    pub output: PathBuf,
    pub pages: Vec<PageDiff>,
    pub checks: Vec<Check>,
    pub previews: Vec<PathBuf>,
    pub mode: Mode,
    pub dpi: f64,
    pub profile: Option<Profile>,
    /// How much ink the pages already carry, across every sheet being printed
    /// onto — that is, what printing them whole again would cost.
    ///
    /// `None` where it was not measured, rather than nought, because nought
    /// is a real answer meaning "these pages are blank".
    pub whole_page_ink_mm2: Option<f64>,
    /// Where the pages that could not be overprinted were written, whole, when
    /// the job was split. `None` when nothing moved, or no split was asked for.
    pub fresh: Option<PathBuf>,
    /// Which pages went into that file, counted from 1 — and so which pages
    /// were blanked in the delta.
    pub reprinted: Vec<usize>,
}

/// What printing only the additions saved, against printing the pages whole.
///
/// This is the entire argument for the program, and until now nothing said
/// it out loud.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Saving {
    /// Ink in the delta.
    pub delta_mm2: f64,
    /// Ink on the pages as they already are.
    pub whole_mm2: f64,
    /// How many sheets go through the printer.
    pub sheets: usize,
}

impl Saving {
    /// The delta's ink as a fraction of a full reprint's, from 0 to 1.
    ///
    /// A page with no ink on it cannot be improved on, so it reports 1: the
    /// delta is the whole of what would be printed either way.
    pub fn ink_fraction(&self) -> f64 {
        if self.whole_mm2 <= 0.0 {
            return 1.0;
        }
        (self.delta_mm2 / self.whole_mm2).min(1.0)
    }

    /// Whether it is worth saying anything at all.
    ///
    /// Below a twentieth of a square millimetre there is nothing on the page
    /// to compare against, and a percentage of nothing reads as a bug.
    pub fn worth_saying(&self) -> bool {
        self.whole_mm2 > 0.05
    }
}

impl Outcome {
    pub fn blocked(&self) -> bool {
        safety::has_blockers(&self.checks)
    }

    pub fn total_regions(&self) -> usize {
        self.pages.iter().map(|p| p.added_regions.len()).sum()
    }

    pub fn total_added_mm2(&self) -> f64 {
        self.pages.iter().map(|p| p.added_ink_mm2()).sum()
    }

    /// What printing only the additions saved, where that was measured.
    pub fn saving(&self) -> Option<Saving> {
        let whole_mm2 = self.whole_page_ink_mm2?;
        let saving = Saving {
            delta_mm2: self.total_added_mm2(),
            whole_mm2,
            sheets: self.pages.len(),
        };
        saving.worth_saying().then_some(saving)
    }

    /// Pages the *edit* added something to, whether or not the delta carries it.
    pub fn pages_with_additions(&self) -> Vec<usize> {
        self.pages
            .iter()
            .filter(|p| p.has_additions())
            .map(|p| p.index + 1)
            .collect()
    }

    /// Pages the delta as written actually carries ink for.
    ///
    /// Different from [`Outcome::pages_with_additions`] only when the job was
    /// split: a page whose text moved has been blanked, so counting it here
    /// would tell somebody to feed a sheet that will come out unchanged.
    pub fn pages_in_the_delta(&self) -> Vec<usize> {
        self.pages_with_additions()
            .into_iter()
            .filter(|page| !self.reprinted.contains(page))
            .collect()
    }

    /// How many additions the delta as written carries.
    pub fn regions_in_the_delta(&self) -> usize {
        let carried = self.pages_in_the_delta();
        self.pages
            .iter()
            .filter(|page| carried.contains(&(page.index + 1)))
            .map(|page| page.added_regions.len())
            .sum()
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "output": self.output.to_string_lossy(),
            "mode": self.mode.name(),
            "dpi": self.dpi,
            "profile": self.profile.as_ref().map(|p| p.name.clone()),
            "correction": self.profile.as_ref().map(|p| p.correction().describe()),
            "blocked": self.blocked(),
            "total_regions": self.total_regions(),
            "total_added_mm2": (self.total_added_mm2() * 100.0).round() / 100.0,
            "pages_with_additions": self.pages_with_additions(),
            "pages": self.pages.iter().map(|p| serde_json::json!({
                "page": p.index + 1,
                "size_mm": [
                    (p.size.width_mm * 10.0).round() / 10.0,
                    (p.size.height_mm * 10.0).round() / 10.0,
                ],
                "added_ink_mm2": (p.added_ink_mm2() * 100.0).round() / 100.0,
                "removed_ink_mm2": (p.removed_ink_mm2() * 100.0).round() / 100.0,
                "added_regions": p.added_regions.iter().map(|r| serde_json::json!({
                    "x_mm": (r.x0_mm * 100.0).round() / 100.0,
                    "y_mm": (r.y0_mm * 100.0).round() / 100.0,
                    "width_mm": (r.width_mm() * 100.0).round() / 100.0,
                    "height_mm": (r.height_mm() * 100.0).round() / 100.0,
                    "ink_mm2": (r.ink_mm2 * 1000.0).round() / 1000.0,
                })).collect::<Vec<_>>(),
                "removed_region_count": p.removed_regions.len(),
            })).collect::<Vec<_>>(),
            "checks": self.checks,
            "previews": self.previews.iter()
                .map(|p| p.to_string_lossy()).collect::<Vec<_>>(),
        })
    }
}

/// Refuse to write the delta over one of the documents it was made from.
///
/// `onionskin delta report.pdf report-v2.pdf -o report.pdf` is an easy thing to
/// type, and without this it destroys the original — the very sheet the delta
/// is meant to be printed onto, and quite possibly the only copy. Paths are
/// resolved first so a symlink or a roundabout relative path cannot slip past.
pub fn guard_output(output: &Path, inputs: &[&Path]) -> Result<(), PipelineError> {
    let Ok(resolved) = output.canonicalize() else {
        // It does not exist yet, so it cannot be one of the inputs.
        return Ok(());
    };
    for source in inputs {
        if let Ok(candidate) = source.canonicalize() {
            if candidate == resolved {
                return Err(PipelineError::Invalid(format!(
                    "refusing to write the delta over '{}' — that is one of the \
                     documents it is made from, and overwriting it would destroy \
                     the original. Choose a different --output.",
                    source.display()
                )));
            }
        }
    }
    Ok(())
}

/// How far along a job is, for whoever is watching it.
///
/// A hundred-page delta takes minutes, and a program that says nothing for
/// minutes looks like a program that has stopped. This is what it says
/// instead — and it is a callback rather than a print so that the window, the
/// command line and a test can each do something different with it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Step {
    /// What is happening, in words.
    pub doing: &'static str,
    /// Which page, counted from 1. Zero when it is not about a page.
    pub page: usize,
    /// How many pages there are, or zero when that is not known yet.
    pub pages: usize,
}

impl Step {
    /// How far through, between nothing and one — or `None` when there is no
    /// honest answer, which is better than a bar that sits at nothing.
    pub fn fraction(&self) -> Option<f32> {
        (self.pages > 0 && self.page > 0)
            .then(|| (self.page as f32 / self.pages as f32).clamp(0.0, 1.0))
    }

    /// The line to show somebody.
    pub fn describe(&self) -> String {
        if self.pages > 0 && self.page > 0 {
            format!("{} — page {} of {}", self.doing, self.page, self.pages)
        } else {
            self.doing.to_string()
        }
    }
}

/// How long a job has left to run, in seconds, or nothing if it is too early
/// to say.
///
/// "Page 40 of 200" answers how far, and leaves the question somebody is
/// actually asking — whether to wait or to go and do something else. Two
/// hundred pages at four seconds each is eleven minutes, and eleven minutes is
/// a cup of tea rather than a stare at the screen.
///
/// # Why it says nothing at the start
///
/// One page in, the estimate is one measurement times a hundred and ninety-nine,
/// and the first page is the slowest one — the fonts are being loaded, the
/// document is being opened, the renderer is warming up. A number arrived at
/// that way is wrong by a factor of several and then visibly collapses, which
/// is worse than no number: somebody who was told twenty minutes and got two
/// stops believing the next one.
///
/// So it waits for [`ENOUGH_PAGES`] pages and [`ENOUGH_SECONDS`] of evidence,
/// and until then the line simply says which page it is on.
pub fn how_much_longer(done: usize, of: usize, elapsed_seconds: f64) -> Option<f64> {
    if done < ENOUGH_PAGES || done >= of || elapsed_seconds < ENOUGH_SECONDS {
        return None;
    }
    if !elapsed_seconds.is_finite() || elapsed_seconds <= 0.0 {
        return None;
    }
    let each = elapsed_seconds / done as f64;
    Some(each * (of - done) as f64)
}

/// How many pages must be done before a guess is worth making.
pub const ENOUGH_PAGES: usize = 3;

/// And how long they must have taken. A job that finishes in under a second
/// does not need to be told it has half a second left.
pub const ENOUGH_SECONDS: f64 = 2.0;

/// How much longer, in words, rounded so the number does not jitter.
///
/// Rounded coarsely on purpose. An estimate that reads "4 minutes 37 seconds"
/// and then "4 minutes 31 seconds" invites somebody to watch it tick, which is
/// exactly what it exists to stop them doing. It is rounded to something the
/// answer survives being wrong about: the difference between "about 5 minutes"
/// and "about 4 minutes" changes nothing anybody does.
pub fn how_long_in_words(seconds: f64) -> String {
    if !seconds.is_finite() || seconds < 0.0 {
        return "a moment".to_string();
    }
    let seconds = seconds.round() as u64;
    match seconds {
        0..=10 => "a few seconds".to_string(),
        11..=45 => format!(
            "about {} seconds",
            (seconds as f64 / 15.0).round() as u64 * 15
        ),
        46..=90 => "about a minute".to_string(),
        // Up to three quarters of an hour in minutes. Past that, "about 60
        // minutes" is a phrase nobody says, and the exact number has stopped
        // being the useful part of the answer anyway.
        91..=2700 => {
            let minutes = ((seconds as f64) / 60.0).round().clamp(2.0, 45.0) as u64;
            format!("about {minutes} minutes")
        }
        2701..=5400 => "about an hour".to_string(),
        // Never "about 1 hours": an hour and a half rounds up to two, and
        // everything under that was caught above.
        _ => format!(
            "about {} hours",
            ((seconds as f64) / 3600.0).round().max(2.0) as u64
        ),
    }
}

/// Nothing is watching. The ordinary case for a library caller.
fn unwatched(_: Step) {}

/// Turn what the document opener had to say into checks.
///
/// The two documents nearly always produce the same sentences — they went
/// through the same opener — so a repeat is dropped rather than printed twice.
fn opening_notes(original: &[String], edited: &[String]) -> Vec<safety::Check> {
    let mut seen: Vec<&str> = Vec::new();
    let mut checks = Vec::new();
    for note in original.iter().chain(edited.iter()) {
        if seen.contains(&note.as_str()) {
            continue;
        }
        seen.push(note);
        // The first sentence is the message and the rest is the detail, which
        // is how every other check in this program reads.
        let (message, detail) = match note.split_once(". ") {
            Some((first, rest)) => (format!("{first}."), rest.to_string()),
            None => (note.clone(), String::new()),
        };
        checks.push(safety::Check::note("opened-by", message, detail));
    }
    checks
}

fn blank_gray(size: PageSize, dpi: f64) -> Vec<u8> {
    let (w, h) = size.px_size(dpi);
    vec![255u8; (w as usize) * (h as usize)]
}

/// Place text at fixed positions on a document's pages.
///
/// Everything downstream of the delta is shared with [`run`] — the same margin
/// and coverage checks, the same proof images, the same calibration. What is
/// absent is the reflow check, and not by omission: absolutely positioned text
/// cannot displace anything, so no ink can move. That is the whole reason this
/// path exists.
pub fn compose_run(
    source: &Path,
    items: &[crate::document::Item],
    output: &Path,
    font: Option<&crate::font::EmbeddedFont>,
    options: &Options,
) -> Result<Outcome, PipelineError> {
    compose_run_drawing(source, items, &[], output, font, options)
}

/// The same, with shapes as well as words.
///
/// Anything Onionskin can open can be drawn on — a Word file, an OpenDocument,
/// a PDF, a scan — and not only its own documents. Somebody ringing a figure on
/// a statement or ruling a line under a total should not first have to convert
/// their file into a format they have never heard of.
///
/// What comes out is a delta, as everywhere else: the shapes on an otherwise
/// blank page, ready to print onto the sheet that already has the document on
/// it. The source is never altered — it is opened, measured, and left alone.
pub fn compose_run_drawing(
    source: &Path,
    items: &[crate::document::Item],
    shapes: &[(usize, crate::pdf::PlacedShape)],
    output: &Path,
    font: Option<&crate::font::EmbeddedFont>,
    options: &Options,
) -> Result<Outcome, PipelineError> {
    compose_onto(
        source,
        Plan::PerPage {
            items,
            shapes,
            images: &[],
        },
        output,
        font,
        options,
    )
}

/// What is to go on the delta, before it is known how many pages the source
/// has.
///
/// The page count can only be had by converting the source, and converting a
/// Word file means running LibreOffice — so it must happen exactly once. An
/// earlier version of this counted the pages first and then converted again
/// inside, which doubled the slowest thing the program does.
pub(crate) enum Plan<'a> {
    /// One sheet per page of the source, each piece of text or drawing placed
    /// by the page number it names.
    PerPage {
        items: &'a [crate::document::Item],
        shapes: &'a [(usize, crate::pdf::PlacedShape)],
        images: &'a [(usize, crate::pdf::PlacedImage)],
    },
    /// Many sheets, all onto the same page of the source: a certificate each
    /// for two hundred people.
    Repeat {
        /// Counted from 1, as a person would say it.
        from_page: usize,
        per_sheet: &'a [Vec<crate::document::Item>],
        /// A picture list per sheet, the same length as `per_sheet` or empty:
        /// everybody's own signature, or their own photograph.
        pictures_per_sheet: &'a [Vec<crate::pdf::PlacedImage>],
    },
}

impl Plan<'_> {
    /// Turn the plan into one sheet per page of the delta, now that the
    /// source has been opened and its page count is known.
    fn sheets(&self, pages: usize) -> Result<Vec<Sheet>, PipelineError> {
        let missing = |page: usize| {
            PipelineError::Invalid(format!(
                "there is no page {page} — the document has {pages}"
            ))
        };
        match self {
            Plan::PerPage {
                items,
                shapes,
                images,
            } => {
                let mut sheets: Vec<Sheet> = (0..pages)
                    .map(|from| Sheet {
                        from,
                        items: Vec::new(),
                        shapes: Vec::new(),
                        images: Vec::new(),
                    })
                    .collect();
                for item in *items {
                    sheets
                        .get_mut(item.page.saturating_sub(1))
                        .ok_or_else(|| missing(item.page))?
                        .items
                        .push(item.clone());
                }
                for (page, shape) in *shapes {
                    sheets
                        .get_mut(page.saturating_sub(1))
                        .ok_or_else(|| missing(*page))?
                        .shapes
                        .push(shape.clone());
                }
                for (page, image) in *images {
                    sheets
                        .get_mut(page.saturating_sub(1))
                        .ok_or_else(|| missing(*page))?
                        .images
                        .push(image.clone());
                }
                Ok(sheets)
            }
            Plan::Repeat {
                from_page,
                per_sheet,
                pictures_per_sheet,
            } => {
                let from = from_page.saturating_sub(1);
                if from >= pages {
                    return Err(missing(*from_page));
                }
                Ok(per_sheet
                    .iter()
                    .enumerate()
                    .map(|(n, items)| Sheet {
                        from,
                        items: items.clone(),
                        shapes: Vec::new(),
                        // Empty means nobody has a picture, which is the
                        // ordinary case and must not be an error.
                        images: pictures_per_sheet.get(n).cloned().unwrap_or_default(),
                    })
                    .collect())
            }
        }
    }
}

/// The same as `compose_run_drawing`, with pictures as well.
///
/// Signatures, stamps and logos: the commonest things anybody adds to a page
/// that is already printed, and the ones that could not be added at all until
/// now.
pub fn compose_run_pictures(
    source: &Path,
    items: &[crate::document::Item],
    shapes: &[(usize, crate::pdf::PlacedShape)],
    images: &[(usize, crate::pdf::PlacedImage)],
    output: &Path,
    font: Option<&crate::font::EmbeddedFont>,
    options: &Options,
) -> Result<Outcome, PipelineError> {
    compose_onto(
        source,
        Plan::PerPage {
            items,
            shapes,
            images,
        },
        output,
        font,
        options,
    )
}

/// Many sheets from one page: a certificate each for two hundred people.
///
/// Every sheet goes onto the *same* page of the source — the blank
/// certificate, the pre-printed form — with different words on each. What
/// comes out is one PDF of two hundred pages, which is a stack of paper
/// through the printer once rather than two hundred separate jobs.
///
/// `from_page` is counted from 1, like every other page number a person
/// types.
pub fn compose_sheets(
    source: &Path,
    from_page: usize,
    per_sheet: &[Vec<crate::document::Item>],
    output: &Path,
    font: Option<&crate::font::EmbeddedFont>,
    options: &Options,
) -> Result<Outcome, PipelineError> {
    compose_sheets_with_pictures(source, from_page, per_sheet, &[], output, font, options)
}

/// The same, with a picture list for each sheet.
///
/// A stack of certificates where everybody signs their own, or a set of passes
/// each carrying its holder's photograph: the words differ per sheet already,
/// and there is no reason a picture should not.
pub fn compose_sheets_with_pictures(
    source: &Path,
    from_page: usize,
    per_sheet: &[Vec<crate::document::Item>],
    pictures_per_sheet: &[Vec<crate::pdf::PlacedImage>],
    output: &Path,
    font: Option<&crate::font::EmbeddedFont>,
    options: &Options,
) -> Result<Outcome, PipelineError> {
    if per_sheet.is_empty() {
        return Err(PipelineError::Invalid(
            "there are no sheets to make".to_string(),
        ));
    }
    compose_onto(
        source,
        Plan::Repeat {
            from_page,
            per_sheet,
            pictures_per_sheet,
        },
        output,
        font,
        options,
    )
}

/// One sheet of the delta: which page of the source it will be printed onto,
/// and what goes on it.
///
/// Two very different jobs meet here. Writing on a document makes one sheet
/// per page of it. Making two hundred certificates makes two hundred sheets
/// that all go onto the *same* page — the blank certificate — with different
/// words each time. Both are a list of sheets, so both get the same laying
/// out, the same checks and the same registration, rather than a second
/// delta pipeline that has to be kept honest separately.
pub(crate) struct Sheet {
    /// Index into the source document's pages.
    pub from: usize,
    pub items: Vec<crate::document::Item>,
    pub shapes: Vec<crate::pdf::PlacedShape>,
    pub images: Vec<crate::pdf::PlacedImage>,
}

impl Sheet {
    /// Whether anything at all is going onto this sheet.
    ///
    /// A sheet with nothing on it needs no drawing, no reading back and no
    /// looking at the page underneath: the delta page was written from this
    /// same empty list a moment ago, so it is blank by construction. Asking
    /// anyway made the work grow with the length of the document instead of
    /// with the size of the edit.
    pub fn has_anything(&self) -> bool {
        !self.items.is_empty() || !self.shapes.is_empty() || !self.images.is_empty()
    }
}

pub(crate) fn compose_onto(
    source: &Path,
    plan: Plan<'_>,
    output: &Path,
    font: Option<&crate::font::EmbeddedFont>,
    options: &Options,
) -> Result<Outcome, PipelineError> {
    options.validate()?;
    guard_output(output, &[source])?;

    let profile = match &options.profile {
        Some(name) => Some(calibrate::load_profile(name)?),
        None => None,
    };
    if let Some(dir) = &options.preview_dir {
        std::fs::create_dir_all(dir).map_err(|source| PipelineError::Io {
            path: dir.clone(),
            source,
        })?;
    }

    let engine = render::engine()?;
    let workspace = Workspace::new(false)?;
    let work = &workspace.path;

    let (source_pdf, _, source_notes) = render::to_pdf_noting(source, work, 180)?;
    let doc = engine.open(&source_pdf)?;

    // Now, and only now, is the page count known — and it cost one conversion
    // rather than two. Every sheet takes the size and the frame of the source
    // page it is going onto, which for a batch is the same page over and over.
    let sheets = plan.sheets(doc.page_sizes.len())?;
    let sheets = &sheets[..];
    let sizes: Vec<crate::geometry::PageSize> =
        sheets.iter().map(|s| doc.page_sizes[s.from]).collect();
    let frames: Vec<render::PageFrame> = sheets.iter().map(|s| doc.frames[s.from]).collect();

    // Lay the text out against the pages it is going onto.
    let mut per_page: Vec<Vec<crate::pdf::PlacedLine>> = vec![Vec::new(); sheets.len()];
    for (index, sheet) in sheets.iter().enumerate() {
        for item in &sheet.items {
            per_page[index].extend(
                item.lines(font)
                    .map_err(|e| PipelineError::Invalid(e.to_string()))?,
            );
        }
    }

    let drawings_per_page: Vec<Vec<crate::pdf::PlacedShape>> =
        sheets.iter().map(|s| s.shapes.clone()).collect();
    let pictures_per_page: Vec<Vec<crate::pdf::PlacedImage>> =
        sheets.iter().map(|s| s.images.clone()).collect();

    let staged = work.join("delta-raw.pdf");
    crate::pdf::write_page_content_with_pictures(
        &staged,
        &sizes,
        &per_page,
        &drawings_per_page,
        &pictures_per_page,
        "Onionskin delta",
        font,
    )
    .map_err(|e| PipelineError::Invalid(e.to_string()))?;

    // Read back what was actually drawn, so the checks and the proof see the
    // ink rather than the intent. A line that runs off the paper, or lands in
    // the border a printer cannot reach, shows up here and nowhere else.
    let composed = engine.open(&staged)?;
    let mut diffs: Vec<PageDiff> = Vec::new();
    let mut previews: Vec<PathBuf> = Vec::new();

    // The page underneath, small, so the legibility check can ask what is
    // already inked where the additions land. Kept by source page rather than
    // by sheet: two hundred certificates all go onto the same blank, and
    // drawing it two hundred times to ask the same question would be absurd.
    let mut beneath: std::collections::BTreeMap<usize, (Vec<u8>, usize)> =
        std::collections::BTreeMap::new();
    // Only the sheets something is going onto. A forty-page document with one
    // change has thirty-nine pages nobody is asking a question about, and
    // drawing them to answer it anyway made the work grow with the length of
    // the document rather than with the size of the edit.
    let want_every_page = options.preview_dir.is_some();
    for sheet in sheets
        .iter()
        .filter(|s| want_every_page || s.has_anything())
    {
        if let std::collections::btree_map::Entry::Vacant(slot) = beneath.entry(sheet.from) {
            let small = doc.render(sheet.from, safety::BENEATH_DPI)?;
            slot.insert((small.gray, small.width));
        }
    }

    // What printing those pages whole would cost in ink, summed over every
    // sheet that carries something — which for a batch means the same page
    // two hundred times, and rightly so: printing them whole would mean
    // printing it two hundred times. Counted off the same small rendering,
    // so it is free.
    let whole_page_ink_mm2 = sheets
        .iter()
        .filter(|s| s.has_anything())
        .filter_map(|sheet| beneath.get(&sheet.from))
        .map(|(gray, width)| {
            ink_area_mm2(
                gray,
                *width,
                safety::BENEATH_DPI,
                options.diff.ink_threshold,
            )
        })
        .sum();

    for (index, size) in sizes.iter().enumerate() {
        // A sheet nothing was put on cannot have ink on it: the delta page was
        // written from this same empty list a moment ago. Drawing it at
        // printing resolution to discover that costs fifteen million pixels
        // to learn nothing, and a forty-page document with one change was
        // paying it thirty-nine times.
        // …unless proofs were asked for, in which case every page gets one,
        // because somebody who asked to see all forty pages asked to see all
        // forty pages.
        if !sheets[index].has_anything() && options.preview_dir.is_none() {
            diffs.push(PageDiff::blank(*size, options.dpi, index));
            continue;
        }

        let drawn = composed.render(index, options.dpi)?;
        let added = crate::diff::ink_mask(
            &drawn.gray,
            drawn.width,
            drawn.height,
            options.diff.ink_threshold,
        );
        let mut diff = PageDiff {
            index,
            size: *size,
            dpi: options.dpi,
            added_px: added.count(),
            removed_px: 0,
            added_regions: crate::diff::label_regions(
                &added,
                options.dpi,
                options.diff.group_mm,
                options.diff.min_region_mm2,
            ),
            removed_regions: Vec::new(),
            removed: crate::diff::Mask::blank(drawn.width, drawn.height),
            added,
        };

        if let Some(dir) = &options.preview_dir {
            let page = doc.render(sheets[index].from, options.dpi)?;
            let image = preview_page(&diff, &page.gray, page.width);
            let path = dir.join(format!("page-{:03}.png", index + 1));
            image.save(&path)?;
            previews.push(path);
        }
        diff.release();
        diffs.push(diff);
    }

    let mut checks = opening_notes(&source_notes, &[]);
    for diff in &diffs {
        checks.extend(safety::check_margins(diff, options.margin_mm));
        checks.extend(safety::check_coverage(diff));
        if let Some((gray, width)) = beneath.get(&sheets[diff.index].from) {
            checks.extend(safety::check_legibility(
                diff,
                gray,
                *width,
                safety::BENEATH_DPI,
                options.diff.ink_threshold,
            ));
            // Whether the printer being out by a couple of millimetres matters
            // for this particular job, which the calibration note below cannot
            // know and this page can answer.
            checks.extend(safety::check_slack(
                diff,
                gray,
                *width,
                safety::BENEATH_DPI,
                options.diff.ink_threshold,
            ));
        }
    }
    checks.extend(safety::check_empty(&diffs));
    checks.extend(safety::check_calibration(
        profile.is_some(),
        profile.as_ref().map(|p| p.name.as_str()),
    ));
    if let (Some(profile), Some(first)) = (&profile, sizes.first()) {
        checks.extend(safety::check_profile_page(
            &profile.name,
            profile.page,
            profile.error,
            *first,
        ));
    }
    safety::drop_the_symptoms(&mut checks);
    safety::sort_checks(&mut checks);

    let correction = profile
        .as_ref()
        .map(|p| p.correction())
        .unwrap_or(Similarity::IDENTITY);
    if let Some(parent) = output.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|source| PipelineError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
    }
    let corrected = apply_correction(
        &staged,
        &work.join("delta-corrected.pdf"),
        correction,
        &sizes,
    )?;
    conform_to_source(&corrected, output, &frames)?;

    Ok(Outcome {
        output: output.to_path_buf(),
        pages: diffs,
        checks,
        previews,
        mode: options.mode,
        dpi: options.dpi,
        profile,
        whole_page_ink_mm2: Some(whole_page_ink_mm2),
        fresh: None,
        reprinted: Vec::new(),
    })
}

/// How much of a greyscale page is inked, in square millimetres.
fn ink_area_mm2(gray: &[u8], width: usize, dpi: f64, threshold: u8) -> f64 {
    if width == 0 || dpi <= 0.0 {
        return 0.0;
    }
    let dark = gray.iter().filter(|value| **value < threshold).count();
    let per_pixel = crate::geometry::px_to_mm(1.0, dpi).powi(2);
    dark as f64 * per_pixel
}

/// Compare two documents and write the delta PDF.
///
/// Pages are handled one at a time — render, diff, emit, release — so memory
/// stays flat regardless of document length.
pub fn run(
    original: &Path,
    edited: &Path,
    output: &Path,
    options: &Options,
) -> Result<Outcome, PipelineError> {
    run_watched(original, edited, output, options, &mut unwatched)
}

/// Compare the two documents and report, writing no delta at all.
///
/// `onionskin compare` used to run the whole of [`run`] into a temporary
/// folder and delete what it produced, which on a long document is most of the
/// work: every changed region is cropped out of the page, given a soft mask,
/// compressed and written, and then thrown away unread. On a twenty-page
/// document that was seventeen seconds of the forty it took.
///
/// Everything reported — the regions, the ink, the checks — is worked out
/// before the delta is built and is not affected by not building it.
pub fn examine(
    original: &Path,
    edited: &Path,
    options: &Options,
) -> Result<Outcome, PipelineError> {
    examine_watched(original, edited, options, &mut unwatched)
}

/// The same, saying how far along it is as it goes.
pub fn examine_watched(
    original: &Path,
    edited: &Path,
    options: &Options,
    watch: &mut dyn FnMut(Step),
) -> Result<Outcome, PipelineError> {
    compare_documents(original, edited, None, options, watch)
}

/// The same, saying how far along it is as it goes.
pub fn run_watched(
    original: &Path,
    edited: &Path,
    output: &Path,
    options: &Options,
    watch: &mut dyn FnMut(Step),
) -> Result<Outcome, PipelineError> {
    compare_documents(original, edited, Some(output), options, watch)
}

/// One page's worth of work for the comparing thread.
struct Compare {
    index: usize,
    old_gray: Vec<u8>,
    new_gray: Vec<u8>,
    width: usize,
    height: usize,
    size: PageSize,
    dpi: f64,
}

/// Compare two documents, and write a delta if one was asked for.
///
/// `output` is `None` for [`examine`], which reports and writes nothing.
fn compare_documents(
    original: &Path,
    edited: &Path,
    output: Option<&Path>,
    options: &Options,
    watch: &mut dyn FnMut(Step),
) -> Result<Outcome, PipelineError> {
    options.validate()?;
    if let Some(output) = output {
        guard_output(output, &[original, edited])?;
    }

    let profile = match &options.profile {
        Some(name) => Some(calibrate::load_profile(name)?),
        None => None,
    };
    if let Some(dir) = &options.preview_dir {
        std::fs::create_dir_all(dir).map_err(|source| PipelineError::Io {
            path: dir.clone(),
            source,
        })?;
    }

    // Held for the whole run: pdfium is not safe to share while a document
    // is open, and both documents are open at once here.
    let engine = render::engine()?;
    let workspace = Workspace::new(false)?;
    let work = &workspace.path;

    watch(Step {
        doing: "Opening both documents",
        page: 0,
        pages: 0,
    });
    // Both at once. Converting a document is mostly waiting for LibreOffice to
    // start, and the two documents have nothing to do with each other — each
    // conversion already gets a private profile precisely so that two can run
    // together, which is a comment in `render` that had never been taken up on.
    // Neither touches pdfium, so the renderer's lock is not involved.
    let (original_done, edited_done) = std::thread::scope(|scope| {
        let second = scope.spawn(|| render::to_pdf_noting(edited, work, 180));
        let first = render::to_pdf_noting(original, work, 180);
        // A panic in the conversion thread is not something to paper over, and
        // there is nothing sensible to do but carry it up.
        (
            first,
            second.join().expect("the converting thread panicked"),
        )
    });
    let (original_pdf, _, original_notes) = original_done?;
    let (edited_pdf, _, edited_notes) = edited_done?;

    let old_doc = engine.open(&original_pdf)?;
    let new_doc = engine.open(&edited_pdf)?;

    let mut checks = safety::check_documents(&old_doc.page_sizes, &new_doc.page_sizes);
    // Both documents go through the same opener in the same run, so anything
    // it had to say applies to both and is worth saying once.
    checks.extend(opening_notes(&original_notes, &edited_notes));

    // The vector delta keeps the edited document's own page objects, so its
    // pages are already in that document's user space — while the regions it
    // clips to are measured in display space, which is the page cropped and
    // turned. On an ordinary page those are the same thing. On a page with a
    // CropBox that does not start at the origin, or a /Rotate, they are not,
    // and the clip band lands somewhere else entirely: whatever existing text
    // falls inside it is what gets printed, and the actual addition is clipped
    // away. `conform_to_source` then makes it worse, by transforming pages
    // that were never in display space a second time.
    //
    // A cropped page is not exotic — a phone scan has one, and so does
    // anything that has been through a PDF editor. So rather than write a
    // delta that puts ink in the wrong place on paper that cannot be
    // reprinted, the raster delta does it, and the reason is said out loud
    // instead of the setting being quietly ignored.
    let mut mode = options.mode;
    if output.is_some() && mode == Mode::Vector {
        let awkward = new_doc.frames.iter().any(|frame| !frame.is_simple());
        if awkward {
            mode = Mode::Raster;
            checks.push(safety::Check::note(
                "vector-needs-a-plain-page",
                "The raster delta was used rather than the vector one.".to_string(),
                "This document's pages are cropped or turned, and the vector delta \
                 cannot place ink on those correctly. The raster delta can, and \
                 never re-prints existing ink."
                    .to_string(),
            ));
        }
    }

    let staged = work.join("delta-raw.pdf");
    // No delta wanted, no delta built. Cropping every changed region out of
    // the page, giving it a soft mask and compressing it is most of the work
    // on a long document, and `examine` throws the result away unread.
    let mut raster = match (output, mode) {
        (Some(_), Mode::Raster) => {
            Some(RasterDeltaWriter::new(&staged, "Onionskin delta")?.marking(options.outline))
        }
        _ => None,
    };

    let mut diffs: Vec<PageDiff> = Vec::new();
    let mut previews: Vec<PathBuf> = Vec::new();
    let mut sizes: Vec<PageSize> = Vec::new();

    // Drawing and comparing run at the same time, one page apart.
    //
    // pdfium cannot be threaded — see `render::engine` — so every page is still
    // drawn one after another on this thread. But comparing a page is pure
    // arithmetic over two buffers of bytes, and nothing says it has to wait for
    // the drawing to stop. So page *n* is compared on a second thread while
    // page *n+1* is being drawn on this one, and since drawing takes about
    // three times as long as comparing, the comparing disappears entirely into
    // the gaps.
    //
    // One page of lookahead and one worker, deliberately. A second worker would
    // have nothing to do, and every page held in flight is another seventy-odd
    // megabytes of pixels waiting about.
    let compare_with = options.diff;
    let (to_worker, jobs) = std::sync::mpsc::sync_channel::<Compare>(1);
    let (from_worker, done) = std::sync::mpsc::sync_channel::<PageDiff>(1);
    let worker = std::thread::spawn(move || {
        for job in jobs {
            let diff = diff_page(
                &job.old_gray,
                (job.width, job.height),
                &job.new_gray,
                (job.width, job.height),
                job.size,
                job.dpi,
                job.index,
                &compare_with,
            );
            // A closed channel means the main thread has given up — stop
            // rather than compare pages nobody is waiting for.
            if from_worker.send(diff).is_err() {
                return;
            }
        }
    });

    // What this thread has to keep hold of until the comparison comes back:
    // the colour, for writing the delta, and the sheet as it was, for the
    // proof image. Both only when they were asked for.
    let mut waiting: std::collections::VecDeque<(Vec<u8>, Vec<u8>, usize)> =
        std::collections::VecDeque::new();

    let collect = |diffs: &mut Vec<PageDiff>,
                   previews: &mut Vec<PathBuf>,
                   raster: &mut Option<RasterDeltaWriter>,
                   waiting: &mut std::collections::VecDeque<(Vec<u8>, Vec<u8>, usize)>|
     -> Result<(), PipelineError> {
        // The worker is gone and no comparison is coming. Returning quietly
        // here left `waiting` un-popped, and the loop below is
        // `while !waiting.is_empty()` — so the program spun on a queue that
        // could never empty, at a hundred per cent of a core, saying nothing.
        // A thread that died is a fault to report, not a page to wait for.
        let Ok(mut diff) = done.recv() else {
            return Err(PipelineError::Invalid(
                "the page comparison stopped before it finished. Nothing was written.".into(),
            ));
        };
        let (rgb, old_gray, width) = waiting.pop_front().expect("a page was compared unasked");
        if let Some(writer) = raster.as_mut() {
            writer.add_page(&diff, Some(&rgb))?;
        }
        if let Some(dir) = &options.preview_dir {
            let image = preview_page(&diff, &old_gray, width);
            let path = dir.join(format!("page-{:03}.png", diff.index + 1));
            image.save(&path)?;
            previews.push(path);
        }
        diff.release();
        diffs.push(diff);
        Ok(())
    };

    for index in 0..new_doc.len() {
        watch(Step {
            doing: "Comparing",
            page: index + 1,
            pages: new_doc.len(),
        });
        // The colour is only ever wanted for the sheet being written, and only
        // when a delta is actually being built. Everything else here compares
        // grey against grey — see `render::GrayPage`.
        let want_colour = raster.is_some();
        let new_page = new_doc.render_either(index, options.dpi, want_colour)?;
        sizes.push(new_page.size);

        let old_gray: Vec<u8> = if index < old_doc.len() {
            let old_page = old_doc.render_gray(index, options.dpi)?;
            if old_page.size.matches(&new_page.size, 0.5) {
                // Onto the new page's raster exactly, cropping or padding at
                // the far edges. Everything downstream — the comparison, the
                // proof image — reads this buffer as if it were the new page's
                // size, and it was simply handed over at its own.
                //
                // The two sizes only have to agree to half a millimetre, which
                // at 400 dpi is nearly eight pixels of slack, and each axis is
                // rounded on its own. So a page 209.90 mm wide against one
                // 210.0 mm wide — a tenth of a millimetre apart, and well
                // inside the tolerance a scanned or re-saved document lands
                // in — gives buffers of different widths.
                //
                // Narrower, and the comparison indexed past the end of it: a
                // panic on the worker thread, which killed the channel, which
                // made the collecting loop spin on an empty queue for ever. No
                // message, no delta, one core at a hundred per cent.
                //
                // Wider, and every row was read a pixel further left than the
                // one above, so within three or four rows the old ink no
                // longer sat under the new. Every glyph on the page reported
                // as both added and removed — either a delta carrying the
                // whole original document, printed back on top of itself, or
                // two hundred sheets condemned as unprintable.
                //
                // Cropping is the same answer `raster` already gives to the
                // same problem: the difference is never more than a pixel or
                // two, and resampling a thirteen-megapixel page to move it one
                // pixel costs a fifth of the run and blurs every glyph edge.
                crate::render::fit(
                    &old_page.gray,
                    (old_page.width, old_page.height),
                    (new_page.width, new_page.height),
                    1,
                    255,
                )
            } else {
                // The size change is already a blocker; diffing two different
                // geometries would only add noise on top of it.
                blank_gray(new_page.size, options.dpi)
            }
        } else {
            // A page the edit added: there is no printed sheet behind it, so
            // everything on it is new.
            blank_gray(new_page.size, options.dpi)
        };
        debug_assert_eq!(
            old_gray.len(),
            new_page.width * new_page.height,
            "the old page was handed on at a size nothing downstream expects"
        );

        // The proof image draws the sheet as it was under the new ink, so it
        // needs a copy — the worker is about to be given the original. Only
        // when a proof was actually asked for.
        let for_preview = if options.preview_dir.is_some() {
            old_gray.clone()
        } else {
            Vec::new()
        };
        waiting.push_back((new_page.rgb, for_preview, new_page.width));

        let job = Compare {
            index,
            old_gray,
            new_gray: new_page.gray,
            width: new_page.width,
            height: new_page.height,
            size: new_page.size,
            dpi: options.dpi,
        };
        // Blocks once one page is already in flight, which is what keeps the
        // memory bounded: this thread cannot run ahead of the comparison by
        // more than a page.
        if to_worker.send(job).is_err() {
            break;
        }
        if waiting.len() > 1 {
            collect(&mut diffs, &mut previews, &mut raster, &mut waiting)?;
        }
    }

    // Nothing more to draw; take what is still in flight.
    drop(to_worker);
    while !waiting.is_empty() {
        collect(&mut diffs, &mut previews, &mut raster, &mut waiting)?;
    }
    let _ = worker.join();
    // The worker answers in the order it was asked, but a comparison that
    // failed to arrive would leave a gap, and everything downstream reads
    // these by position.
    diffs.sort_by_key(|diff| diff.index);

    if output.is_some() {
        watch(Step {
            doing: "Writing the delta",
            page: 0,
            pages: 0,
        });
        match raster {
            Some(writer) => {
                writer.close()?;
            }
            None => {
                build_vector_delta(
                    &diffs,
                    &edited_pdf,
                    &staged,
                    options.pad_mm,
                    "Onionskin delta",
                    options.outline,
                )?;
            }
        }
    }

    watch(Step {
        doing: "Checking it is safe to print",
        page: 0,
        pages: 0,
    });
    for diff in &diffs {
        checks.extend(safety::check_reflow(diff));
        checks.extend(safety::check_margins(diff, options.margin_mm));
        checks.extend(safety::check_coverage(diff));
    }
    checks.extend(safety::check_empty(&diffs));
    checks.extend(safety::check_calibration(
        profile.is_some(),
        profile.as_ref().map(|p| p.name.as_str()),
    ));
    if let (Some(profile), Some(first)) = (&profile, sizes.first()) {
        checks.extend(safety::check_profile_page(
            &profile.name,
            profile.page,
            profile.error,
            *first,
        ));
    }
    safety::drop_the_symptoms(&mut checks);
    safety::sort_checks(&mut checks);

    let correction = profile
        .as_ref()
        .map(|p| p.correction())
        .unwrap_or(Similarity::IDENTITY);

    // Reporting stops here. Everything below writes the delta; everything
    // above it — the regions, the ink, the checks — was worked out without
    // needing to, which is why `examine` can stop at this line.
    let Some(output) = output else {
        return Ok(Outcome {
            output: PathBuf::new(),
            pages: diffs,
            checks,
            previews,
            mode,
            dpi: options.dpi,
            profile,
            whole_page_ink_mm2: None,
            fresh: None,
            reprinted: Vec::new(),
        });
    };
    if let Some(parent) = output.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|source| PipelineError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
    }
    let corrected = apply_correction(
        &staged,
        &work.join("delta-corrected.pdf"),
        correction,
        &sizes,
    )?;

    // Conform to the ORIGINAL: that is the sheet going back in the tray.
    let mut frames: Vec<crate::render::PageFrame> =
        old_doc.frames.iter().take(sizes.len()).copied().collect();
    for index in frames.len()..sizes.len() {
        if let Some(frame) = new_doc.frames.get(index) {
            frames.push(*frame);
        }
    }
    conform_to_source(&corrected, output, &frames)?;

    // Splitting the job, where one was asked for.
    //
    // Last, because it works on the delta as written rather than on any of the
    // stages before it — and because it changes the verdict: the blocker that
    // said "nothing worth printing was produced" stops being true the moment
    // there is something worth printing.
    // The checks are the authority on which pages moved, not the measurement
    // they came from: two documents given the wrong way round have ink missing
    // from every page, `drop_the_symptoms` has already recognised that as one
    // mistake and dropped the reflow findings, and splitting on the raw numbers
    // would blank the whole delta and call the entire document "fresh".
    let split = crate::split::Split::given(&diffs, &safety::pages_that_moved(&checks));
    let mut fresh = None;
    let mut reprinted: Vec<usize> = Vec::new();
    if let Some(wanted) = &options.fresh {
        let reprint = split.reprint();
        // Not where something else blocks the job as well. A split answers one
        // objection — that these pages moved — and leaving the others standing
        // while writing a file called "fresh pages" tells somebody to print
        // paper for a job that is going nowhere.
        if !reprint.is_empty() && safety::only_moved_text_blocks(&checks) {
            // Blanked first. If writing the fresh pages then fails, the delta
            // is still safe to feed; the other way round would leave ink in it
            // that lands on a sheet nobody should be feeding.
            crate::split::blank_pages(output, &reprint)
                .map_err(|e| PipelineError::Invalid(e.to_string()))?;
            crate::split::keep_only(&edited_pdf, &reprint, wanted)
                .map_err(|e| PipelineError::Invalid(e.to_string()))?;
            safety::reflow_is_handled(&mut checks, split.what_to_do(output, wanted));
            safety::sort_checks(&mut checks);
            fresh = Some(wanted.clone());
            reprinted = reprint;
        }
    }

    Ok(Outcome {
        output: output.to_path_buf(),
        pages: diffs,
        checks,
        previews,
        mode,
        dpi: options.dpi,
        profile,
        whole_page_ink_mm2: None,
        fresh,
        reprinted,
    })
}

#[cfg(test)]
mod tests;
