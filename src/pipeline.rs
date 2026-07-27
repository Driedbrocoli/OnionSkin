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

    pub fn pages_with_additions(&self) -> Vec<usize> {
        self.pages
            .iter()
            .filter(|p| p.has_additions())
            .map(|p| p.index + 1)
            .collect()
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
    let sizes = doc.page_sizes.clone();

    // Lay the text out against the pages it is going onto.
    let mut per_page: Vec<Vec<crate::pdf::PlacedLine>> = vec![Vec::new(); sizes.len()];
    for item in items {
        let index = item.page.saturating_sub(1);
        if index >= per_page.len() {
            return Err(PipelineError::Invalid(format!(
                "there is no page {} — the document has {}",
                item.page,
                sizes.len()
            )));
        }
        per_page[index].extend(
            item.lines(font)
                .map_err(|e| PipelineError::Invalid(e.to_string()))?,
        );
    }

    // Shapes are placed by page the same way the words are, and a shape aimed
    // at a page that is not there is the same mistake as a word aimed at one.
    let mut drawings_per_page: Vec<Vec<crate::pdf::PlacedShape>> = vec![Vec::new(); sizes.len()];
    for (page, shape) in shapes {
        let index = page.saturating_sub(1);
        if index >= drawings_per_page.len() {
            return Err(PipelineError::Invalid(format!(
                "there is no page {page} — the document has {}",
                sizes.len()
            )));
        }
        drawings_per_page[index].push(shape.clone());
    }

    let staged = work.join("delta-raw.pdf");
    crate::pdf::write_page_content(
        &staged,
        &sizes,
        &per_page,
        &drawings_per_page,
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

    for (index, size) in sizes.iter().enumerate() {
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
            let page = doc.render(index, options.dpi)?;
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
    conform_to_source(&corrected, output, &doc.frames)?;

    Ok(Outcome {
        output: output.to_path_buf(),
        pages: diffs,
        checks,
        previews,
        mode: options.mode,
        dpi: options.dpi,
        profile,
    })
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

/// The same, saying how far along it is as it goes.
pub fn run_watched(
    original: &Path,
    edited: &Path,
    output: &Path,
    options: &Options,
    watch: &mut dyn FnMut(Step),
) -> Result<Outcome, PipelineError> {
    options.validate()?;
    guard_output(output, &[original, edited])?;

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
    let (original_pdf, _, original_notes) = render::to_pdf_noting(original, work, 180)?;
    let (edited_pdf, _, edited_notes) = render::to_pdf_noting(edited, work, 180)?;

    let old_doc = engine.open(&original_pdf)?;
    let new_doc = engine.open(&edited_pdf)?;

    let mut checks = safety::check_documents(&old_doc.page_sizes, &new_doc.page_sizes);
    // Both documents go through the same opener in the same run, so anything
    // it had to say applies to both and is worth saying once.
    checks.extend(opening_notes(&original_notes, &edited_notes));

    let staged = work.join("delta-raw.pdf");
    let mut raster = match options.mode {
        Mode::Raster => {
            Some(RasterDeltaWriter::new(&staged, "Onionskin delta")?.marking(options.outline))
        }
        Mode::Vector => None,
    };

    let mut diffs: Vec<PageDiff> = Vec::new();
    let mut previews: Vec<PathBuf> = Vec::new();
    let mut sizes: Vec<PageSize> = Vec::new();

    for index in 0..new_doc.len() {
        watch(Step {
            doing: "Comparing",
            page: index + 1,
            pages: new_doc.len(),
        });
        let new_page = new_doc.render(index, options.dpi)?;
        sizes.push(new_page.size);

        let old_gray: Vec<u8> = if index < old_doc.len() {
            let old_page = old_doc.render(index, options.dpi)?;
            if old_page.size.matches(&new_page.size, 0.5) {
                old_page.gray
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

        let mut diff = diff_page(
            &old_gray,
            (new_page.width, new_page.height),
            &new_page.gray,
            (new_page.width, new_page.height),
            new_page.size,
            options.dpi,
            index,
            &options.diff,
        );

        if let Some(writer) = raster.as_mut() {
            writer.add_page(&diff, Some(&new_page.rgb))?;
        }

        if let Some(dir) = &options.preview_dir {
            let image = preview_page(&diff, &old_gray, new_page.width);
            let path = dir.join(format!("page-{:03}.png", index + 1));
            image.save(&path)?;
            previews.push(path);
        }

        diff.release();
        diffs.push(diff);
    }

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

    Ok(Outcome {
        output: output.to_path_buf(),
        pages: diffs,
        checks,
        previews,
        mode: options.mode,
        dpi: options.dpi,
        profile,
    })
}

#[cfg(test)]
mod tests;
