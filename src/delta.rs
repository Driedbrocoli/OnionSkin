//! Writing the delta PDF — the transparent sheet you print onto the original.
//!
//! Two ways to build it, with a real trade-off between them:
//!
//! **raster** prints exactly the pixels that are new, and nothing else.
//! Anti-aliasing is recovered as an alpha channel so glyph edges stay smooth.
//! This can never re-print ink that is already on the sheet, which makes it the
//! safe default.
//!
//! **vector** clips the edited PDF to the changed regions and keeps the
//! original vector text. Sharper at any print resolution, but a clip rectangle
//! is a rectangle: if a new word sits hard against an existing one, a sliver of
//! the existing word falls inside the box and gets printed a second time, very
//! slightly offset. On close inspection that reads as a bolded or blurred
//! character.

use std::path::{Path, PathBuf};

use lopdf::{dictionary, Object, Stream};

use crate::diff::{PageDiff, Region};
use crate::geometry::{mm_to_pt, PageSize, Similarity};
use crate::render::PageFrame;

pub const PRODUCER: &str = "Onionskin";

#[derive(Debug, thiserror::Error)]
pub enum DeltaError {
    #[error("could not write the delta: {0}")]
    Io(#[from] std::io::Error),
    #[error("could not build the delta: {0}")]
    Lopdf(#[from] lopdf::Error),
    #[error("could not encode the delta's image: {0}")]
    Image(#[from] image::ImageError),
}

/// Ink recovered from a render composited over white paper.
///
/// A renderer gives `C = a·K + (1-a)·255` — ink colour `K` at coverage `a` over
/// white. Coverage comes from the darkest channel, and the ink colour follows.
/// Without this every anti-aliased edge pixel would be printed at full opacity
/// and new text would sit inside a pale halo.
pub struct Unmatted {
    pub width: usize,
    pub height: usize,
    /// Ink colour, three bytes per pixel.
    pub rgb: Vec<u8>,
    /// Coverage, one byte per pixel. Zero everywhere the mask is clear.
    pub alpha: Vec<u8>,
}

pub fn unmatte(
    rgb: &[u8],
    width: usize,
    height: usize,
    mask: &crate::diff::Mask,
    offset: (usize, usize),
    source_width: usize,
) -> Unmatted {
    let mut out_rgb = vec![0u8; width * height * 3];
    let mut out_alpha = vec![0u8; width * height];

    for y in 0..height {
        for x in 0..width {
            let (sx, sy) = (x + offset.0, y + offset.1);
            if !mask.get(sx, sy) {
                continue;
            }
            let at = (sy * source_width + sx) * 3;
            let (r, g, b) = (rgb[at], rgb[at + 1], rgb[at + 2]);
            let darkest = r.min(g).min(b) as f32;
            let coverage = ((255.0 - darkest) / 255.0).clamp(0.0, 1.0);
            // A floor, so a pixel that is barely inked does not divide by zero
            // and come back as a wild colour.
            let safe = coverage.max(1e-3);

            let recover = |channel: u8| -> u8 {
                (((channel as f32 - (1.0 - safe) * 255.0) / safe).clamp(0.0, 255.0)) as u8
            };
            let to = (y * width + x) * 3;
            out_rgb[to] = recover(r);
            out_rgb[to + 1] = recover(g);
            out_rgb[to + 2] = recover(b);
            out_alpha[y * width + x] = (coverage * 255.0).clamp(0.0, 255.0) as u8;
        }
    }
    Unmatted {
        width,
        height,
        rgb: out_rgb,
        alpha: out_alpha,
    }
}

/// Region boxes as PDF-space rectangle operators (y flips to point up).
fn pdf_rects(regions: &[Region], page: PageSize) -> String {
    regions
        .iter()
        .map(|r| {
            format!(
                "{:.4} {:.4} {:.4} {:.4} re",
                mm_to_pt(r.x0_mm),
                page.height_pt() - mm_to_pt(r.y1_mm),
                mm_to_pt(r.width_mm()),
                mm_to_pt(r.height_mm())
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Where a delta goes when nobody has asked to keep it.
///
/// Most deltas are looked at once and printed once. Writing every one of them
/// into the folder somebody keeps their documents in leaves that folder full of
/// `delta.pdf`, `delta (1).pdf`, `delta (2).pdf` — and makes choosing a name a
/// step in a job that did not need one. So the default is a file here, inside
/// Onionskin's own folder, and saving a copy somewhere is something a person
/// does when they want to rather than something they must do to continue.
///
/// Under the Onionskin home rather than the system temporary directory,
/// because a delta is made from somebody's document and a shared `/tmp` is
/// world-readable on most machines.
pub fn scratch_path(name: &str) -> PathBuf {
    let folder = crate::calibrate::home_dir().join("deltas");
    let _ = std::fs::create_dir_all(&folder);
    crate::render::restrict(&folder);
    folder.join(name)
}

/// Delete deltas left behind by earlier runs, keeping one if asked.
///
/// Called when a new one is made rather than when the program closes: a
/// program that is killed never runs its tidying, and the folder then grows
/// forever. Failures are ignored — a file that cannot be deleted is not a
/// reason to refuse to make the next delta.
pub fn tidy_scratch(keep: Option<&Path>) {
    let folder = crate::calibrate::home_dir().join("deltas");
    let Ok(entries) = std::fs::read_dir(&folder) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if keep.map(|kept| kept == path).unwrap_or(false) {
            continue;
        }
        if path.is_file() {
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// A box drawn round each change, so it can be seen.
///
/// Off unless asked for, and deliberately: a delta is printed onto the paper
/// already in the tray, so a box drawn round a change is as permanent as the
/// change. Somebody checking an edit wants it. Somebody producing a finished
/// page very much does not.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Outline {
    /// How thick the line is, in millimetres.
    pub width_mm: f64,
    /// How far outside the ink the box sits. Enough that the line does not
    /// touch the letters it is drawn around.
    pub pad_mm: f64,
    /// The line's colour: red, green, blue, each 0 to 1.
    pub colour: (f64, f64, f64),
}

impl Default for Outline {
    fn default() -> Outline {
        Outline {
            width_mm: 0.25,
            pad_mm: 1.2,
            // Red, because the point of the box is to be noticed, and because
            // a black box on a black-and-white page reads as part of the page.
            colour: (0.80, 0.10, 0.10),
        }
    }
}

impl Outline {
    /// The PDF operators that stroke a box around each region.
    ///
    /// Self-contained: it saves the graphics state, sets its own colour and
    /// line width, strokes, and restores. So it can be appended to any content
    /// stream without disturbing what was drawn before it.
    pub fn ops(&self, regions: &[Region], page: PageSize) -> String {
        let boxes = self.boxes(regions, page);
        if boxes.is_empty() {
            return String::new();
        }
        // Every rectangle, then one `S`: the whole set is stroked in a single
        // path, which is both smaller and how a PDF is meant to say it.
        format!(
            " q {:.4} {:.4} {:.4} RG {:.4} w {} S Q",
            self.colour.0,
            self.colour.1,
            self.colour.2,
            mm_to_pt(self.width_mm),
            pdf_rects(&boxes, page),
        )
    }

    /// Where the boxes go: each region grown by the padding, and any that then
    /// overlap merged into one.
    pub fn boxes(&self, regions: &[Region], page: PageSize) -> Vec<Region> {
        let padded: Vec<Region> = regions.iter().map(|r| r.padded(self.pad_mm, page)).collect();
        merge_touching(padded)
    }
}

/// Merge boxes that overlap, until none do.
///
/// Two words a few millimetres apart become two boxes that cross each other,
/// which looks like a mistake and is harder to read than the thing it marks.
/// One box round the pair is what somebody drawing this by hand would do.
fn merge_touching(mut boxes: Vec<Region>) -> Vec<Region> {
    let mut merged = true;
    while merged {
        merged = false;
        let mut out: Vec<Region> = Vec::with_capacity(boxes.len());
        'next: for box_ in boxes.drain(..) {
            for kept in out.iter_mut() {
                if overlaps(kept, &box_) {
                    *kept = union(kept, &box_);
                    merged = true;
                    continue 'next;
                }
            }
            out.push(box_);
        }
        boxes = out;
    }
    boxes
}

fn overlaps(a: &Region, b: &Region) -> bool {
    a.x0_mm < b.x1_mm && b.x0_mm < a.x1_mm && a.y0_mm < b.y1_mm && b.y0_mm < a.y1_mm
}

fn union(a: &Region, b: &Region) -> Region {
    Region {
        x0_mm: a.x0_mm.min(b.x0_mm),
        y0_mm: a.y0_mm.min(b.y0_mm),
        x1_mm: a.x1_mm.max(b.x1_mm),
        y1_mm: a.y1_mm.max(b.y1_mm),
        ink_mm2: a.ink_mm2 + b.ink_mm2,
        px_bbox: (
            a.px_bbox.0.min(b.px_bbox.0),
            a.px_bbox.1.min(b.px_bbox.1),
            a.px_bbox.2.max(b.px_bbox.2),
            a.px_bbox.3.max(b.px_bbox.3),
        ),
    }
}

/// Builds a raster delta a page at a time.
///
/// Pages are written and released as they arrive, so a long document never
/// holds more than one page of pixels.
pub struct RasterDeltaWriter {
    out_path: PathBuf,
    doc: lopdf::Document,
    pages_id: lopdf::ObjectId,
    page_ids: Vec<Object>,
    title: String,
    outline: Option<Outline>,
}

impl RasterDeltaWriter {
    pub fn new(out_path: &Path, title: &str) -> Result<RasterDeltaWriter, DeltaError> {
        if let Some(parent) = out_path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let mut doc = lopdf::Document::with_version("1.4");
        let pages_id = doc.new_object_id();
        Ok(RasterDeltaWriter {
            out_path: out_path.to_path_buf(),
            doc,
            pages_id,
            page_ids: Vec::new(),
            title: title.to_string(),
            outline: None,
        })
    }

    /// Draw a box round each change as well as the change itself.
    pub fn marking(mut self, outline: Option<Outline>) -> RasterDeltaWriter {
        self.outline = outline;
        self
    }

    /// Add one page, drawing whatever ink is new on it.
    pub fn add_page(&mut self, diff: &PageDiff, rgb: Option<&[u8]>) -> Result<(), DeltaError> {
        let mut page = dictionary! {
            "Type" => "Page",
            "Parent" => self.pages_id,
            "MediaBox" => Object::Array(vec![
                Object::Real(0.0),
                Object::Real(0.0),
                Object::Real(round6(diff.size.width_pt()) as f32),
                Object::Real(round6(diff.size.height_pt()) as f32),
            ]),
            "Resources" => dictionary! {},
        };

        if let (Some(rgb), true) = (rgb, diff.added_px > 0) {
            if let Some((content, resources)) = self.draw(diff, rgb)? {
                page.set("Contents", content);
                page.set("Resources", resources);
            }
        }
        let id = self.doc.add_object(page);
        self.page_ids.push(Object::Reference(id));
        Ok(())
    }

    /// Embed only the part of the page that has new ink on it.
    ///
    /// A delta is usually a few words on an otherwise empty sheet. Encoding the
    /// whole page anyway means compressing thirteen million transparent pixels
    /// to say nothing — which dominates the run time and bloats the file that
    /// has to travel to the printer. Cropping to the ink and placing that
    /// rectangle at the matching spot is pixel-for-pixel identical.
    fn draw(
        &mut self,
        diff: &PageDiff,
        rgb: &[u8],
    ) -> Result<Option<(Object, Object)>, DeltaError> {
        let (width, height) = (diff.added.width, diff.added.height);
        let Some((x0, y0, x1, y1)) = ink_bounds(&diff.added) else {
            return Ok(None);
        };
        // One pixel of margin so an anti-aliased edge cannot be clipped.
        let x0 = x0.saturating_sub(1);
        let y0 = y0.saturating_sub(1);
        let x1 = (x1 + 1).min(width);
        let y1 = (y1 + 1).min(height);
        let (crop_w, crop_h) = (x1 - x0, y1 - y0);

        let ink = unmatte(rgb, crop_w, crop_h, &diff.added, (x0, y0), width);

        // The image itself, and its coverage as a soft mask. Flate rather than
        // a PNG wrapper: the PDF holds raw samples, and a driver that chokes on
        // an embedded PNG is a real thing on older printers.
        let mut image = Stream::new(
            dictionary! {
                "Type" => "XObject",
                "Subtype" => "Image",
                "Width" => crop_w as i64,
                "Height" => crop_h as i64,
                "ColorSpace" => "DeviceRGB",
                "BitsPerComponent" => 8,
            },
            ink.rgb,
        );
        let _ = image.compress();

        let mut soft_mask = Stream::new(
            dictionary! {
                "Type" => "XObject",
                "Subtype" => "Image",
                "Width" => crop_w as i64,
                "Height" => crop_h as i64,
                "ColorSpace" => "DeviceGray",
                "BitsPerComponent" => 8,
            },
            ink.alpha,
        );
        let _ = soft_mask.compress();
        let mask_id = self.doc.add_object(soft_mask);

        image.dict.set("SMask", Object::Reference(mask_id));
        let image_id = self.doc.add_object(image);

        // Pixels map onto the page linearly, so the crop's rectangle is just
        // its pixel bounds scaled by the page size.
        let px_to_pt_x = diff.size.width_pt() / width as f64;
        let px_to_pt_y = diff.size.height_pt() / height as f64;
        let place_x = x0 as f64 * px_to_pt_x;
        let place_y = diff.size.height_pt() - y1 as f64 * px_to_pt_y;
        let draw_w = crop_w as f64 * px_to_pt_x;
        let draw_h = crop_h as f64 * px_to_pt_y;

        let mut content = format!(
            "q {:.6} 0 0 {:.6} {:.6} {:.6} cm /Ink Do Q",
            draw_w, draw_h, place_x, place_y
        );
        // After the ink, so a box is never hidden underneath the thing it marks.
        if let Some(outline) = self.outline {
            content.push_str(&outline.ops(&diff.added_regions, diff.size));
        }
        let mut stream = Stream::new(dictionary! {}, content.into_bytes());
        let _ = stream.compress();
        let content_id = self.doc.add_object(stream);

        let resources = dictionary! {
            "XObject" => dictionary! { "Ink" => Object::Reference(image_id) },
        };
        Ok(Some((
            Object::Reference(content_id),
            Object::Dictionary(resources),
        )))
    }

    pub fn close(mut self) -> Result<PathBuf, DeltaError> {
        let count = self.page_ids.len() as i64;
        self.doc.objects.insert(
            self.pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Count" => count,
                "Kids" => Object::Array(std::mem::take(&mut self.page_ids)),
            }),
        );
        let catalog_id = self.doc.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => self.pages_id,
        });
        let info_id = self.doc.add_object(dictionary! {
            "Title" => Object::string_literal(self.title.clone()),
            "Producer" => Object::string_literal(PRODUCER),
            "Subject" => Object::string_literal(
                "Additions only — print onto the already-printed sheet",
            ),
        });
        self.doc.trailer.set("Root", catalog_id);
        self.doc.trailer.set("Info", info_id);
        self.doc.compress();
        self.doc.save(&self.out_path)?;
        Ok(self.out_path)
    }
}

/// The pixel bounds of everything set in a mask.
fn ink_bounds(mask: &crate::diff::Mask) -> Option<(usize, usize, usize, usize)> {
    let (mut x0, mut y0) = (usize::MAX, usize::MAX);
    let (mut x1, mut y1) = (0usize, 0usize);
    let mut found = false;
    for y in 0..mask.height {
        let row = y * mask.width;
        for x in 0..mask.width {
            if mask.bits[row + x] {
                found = true;
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x + 1);
                y1 = y1.max(y + 1);
            }
        }
    }
    found.then_some((x0, y0, x1, y1))
}

fn round6(value: f64) -> f64 {
    (value * 1e6).round() / 1e6
}

/// Build a whole raster delta.
pub fn build_raster_delta(
    diffs: &[PageDiff],
    page_rgb: &[Option<Vec<u8>>],
    out_path: &Path,
    title: &str,
    outline: Option<Outline>,
) -> Result<PathBuf, DeltaError> {
    let mut writer = RasterDeltaWriter::new(out_path, title)?.marking(outline);
    for (index, diff) in diffs.iter().enumerate() {
        let rgb = page_rgb.get(index).and_then(|o| o.as_deref());
        writer.add_page(diff, rgb)?;
    }
    writer.close()
}

/// Build a vector delta: the edited PDF, clipped to the changed regions.
pub fn build_vector_delta(
    diffs: &[PageDiff],
    edited_pdf: &Path,
    out_path: &Path,
    pad_mm: f64,
    title: &str,
    outline: Option<Outline>,
) -> Result<PathBuf, DeltaError> {
    if let Some(parent) = out_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let mut source = lopdf::Document::load(edited_pdf)?;
    let source_pages: Vec<lopdf::ObjectId> = source.get_pages().values().copied().collect();

    // Keep only the pages that gained something, and wrap each in a clip.
    let mut keep: Vec<lopdf::ObjectId> = Vec::new();
    for (index, diff) in diffs.iter().enumerate() {
        let Some(&page_id) = source_pages.get(index) else {
            break;
        };
        if diff.added_regions.is_empty() {
            // A page with no additions still has to exist, so the delta's page
            // numbers line up with the sheets in the tray — but it must be
            // blank, or the whole original page prints on top of itself.
            blank_page(&mut source, page_id);
            keep.push(page_id);
            continue;
        }
        let regions: Vec<Region> = diff
            .added_regions
            .iter()
            .map(|r| r.padded(pad_mm, diff.size))
            .collect();
        let clip = format!("q {} W n ", pdf_rects(&regions, diff.size));
        // The box goes outside the clip, not inside it: a line drawn round a
        // region is by definition at the edge of that region, and half of it
        // would be clipped away.
        let mut after = String::from(" Q");
        if let Some(outline) = outline {
            after.push_str(&outline.ops(&diff.added_regions, diff.size));
        }
        wrap_content(&mut source, page_id, &clip, &after)?;
        keep.push(page_id);
    }

    // Any page beyond the diffs is not part of this job.
    for page_id in source_pages.iter().skip(keep.len()) {
        blank_page(&mut source, *page_id);
        keep.push(*page_id);
    }

    let info_id = source.add_object(dictionary! {
        "Title" => Object::string_literal(title),
        "Producer" => Object::string_literal(PRODUCER),
    });
    source.trailer.set("Info", info_id);
    source.compress();
    source.save(out_path)?;
    Ok(out_path.to_path_buf())
}

/// Empty a page's content, leaving its geometry alone.
fn blank_page(pdf: &mut lopdf::Document, page_id: lopdf::ObjectId) {
    if let Ok(page) = pdf.get_dictionary_mut(page_id) {
        page.remove(b"Contents");
        page.remove(b"Annots");
    }
}

/// Put `before` at the front of a page's content stream and `after` at the end.
///
/// A page's content may be one stream or an array of them, and the array's
/// pieces are concatenated *as if they were one stream* — a `q` may open in one
/// and close in the next. So a new stream is spliced in at each end rather than
/// the existing ones being edited.
fn wrap_content(
    pdf: &mut lopdf::Document,
    page_id: lopdf::ObjectId,
    before: &str,
    after: &str,
) -> Result<(), DeltaError> {
    let existing = pdf
        .get_dictionary(page_id)
        .ok()
        .and_then(|page| page.get(b"Contents").ok())
        .cloned();

    let head = pdf.add_object(Stream::new(dictionary! {}, before.as_bytes().to_vec()));
    let tail = pdf.add_object(Stream::new(dictionary! {}, after.as_bytes().to_vec()));

    let mut parts = vec![Object::Reference(head)];
    match existing {
        Some(Object::Array(items)) => parts.extend(items),
        Some(other) => parts.push(other),
        None => {}
    }
    parts.push(Object::Reference(tail));

    pdf.get_dictionary_mut(page_id)?
        .set("Contents", Object::Array(parts));
    Ok(())
}

/// Re-place every page's content through a calibration transform.
///
/// The transform is prepended to each page as a single `cm` matrix and balanced
/// with a trailing `Q`, which keeps vector text vector, keeps every resource
/// attached to the page it came from, and leaves the media box — the physical
/// sheet — untouched. Only the ink moves.
pub fn apply_correction(
    pdf_path: &Path,
    out_path: &Path,
    correction: Similarity,
    sizes: &[PageSize],
) -> Result<PathBuf, DeltaError> {
    if correction.is_identity() {
        if pdf_path != out_path {
            std::fs::copy(pdf_path, out_path)?;
        }
        return Ok(out_path.to_path_buf());
    }

    let mut pdf = lopdf::Document::load(pdf_path)?;
    let page_ids: Vec<lopdf::ObjectId> = pdf.get_pages().values().copied().collect();
    for (index, page_id) in page_ids.iter().enumerate() {
        let size = sizes
            .get(index)
            .or_else(|| sizes.last())
            .copied()
            .unwrap_or(PageSize::new(210.0, 297.0));
        let m = correction.to_pdf_matrix(&size);
        let before = format!(
            "q {:.9} {:.9} {:.9} {:.9} {:.6} {:.6} cm ",
            m.a, m.b, m.c, m.d, m.e, m.f
        );
        wrap_content(&mut pdf, *page_id, &before, " Q")?;
    }
    pdf.save(out_path)?;
    Ok(out_path.to_path_buf())
}

/// Map delta (display-space) coordinates into the source page's user space.
///
/// The delta is written as a plain page: origin at (0,0), the right way up, the
/// size you see on screen. The source page may be none of those things. This is
/// the PDF matrix that puts the delta's ink exactly where the same spot appears
/// on the source page, so both land together on the sheet.
fn display_to_user_matrix(frame: &PageFrame) -> (f64, f64, f64, f64, f64, f64) {
    let (x0, y0) = (frame.crop.0, frame.crop.1);
    let (width, height) = frame.crop_size_pt();
    match frame.rotate {
        //     a    b     c    d     e            f
        90 => (0.0, 1.0, -1.0, 0.0, x0 + width, y0),
        180 => (-1.0, 0.0, 0.0, -1.0, x0 + width, y0 + height),
        270 => (0.0, -1.0, 1.0, 0.0, x0, y0 + height),
        _ => (1.0, 0.0, 0.0, 1.0, x0, y0),
    }
}

/// Give the delta the same page geometry as the document it overlays.
///
/// Printers place a page on paper using its boxes and `/Rotate`. If the delta
/// disagrees with the source about any of those, the two impressions cannot
/// line up no matter how good the calibration is — so the delta copies them
/// exactly, and its content is transformed to match.
pub fn conform_to_source(
    pdf_path: &Path,
    out_path: &Path,
    frames: &[PageFrame],
) -> Result<PathBuf, DeltaError> {
    if frames.iter().all(|f| f.is_simple()) {
        if pdf_path != out_path {
            std::fs::copy(pdf_path, out_path)?;
        }
        return Ok(out_path.to_path_buf());
    }

    let mut pdf = lopdf::Document::load(pdf_path)?;
    let page_ids: Vec<lopdf::ObjectId> = pdf.get_pages().values().copied().collect();

    for (index, page_id) in page_ids.iter().enumerate() {
        let Some(frame) = frames.get(index) else {
            break;
        };
        if frame.is_simple() {
            continue;
        }
        let (a, b, c, d, e, f) = display_to_user_matrix(frame);
        let before = format!("q {a:.6} {b:.6} {c:.6} {d:.6} {e:.6} {f:.6} cm ");
        wrap_content(&mut pdf, *page_id, &before, " Q")?;

        let boxes = |r: (f64, f64, f64, f64)| {
            Object::Array(vec![
                Object::Real(r.0 as f32),
                Object::Real(r.1 as f32),
                Object::Real(r.2 as f32),
                Object::Real(r.3 as f32),
            ])
        };
        let page = pdf.get_dictionary_mut(*page_id)?;
        page.set("MediaBox", boxes(frame.media));
        page.set("CropBox", boxes(frame.crop));
        if frame.rotate != 0 {
            page.set("Rotate", Object::Integer(frame.rotate));
        } else {
            page.remove(b"Rotate");
        }
    }
    pdf.save(out_path)?;
    Ok(out_path.to_path_buf())
}

/// A proof image: the existing sheet faded back, new ink in red.
///
/// This is the thing that actually stops wasted paper — you see where the new
/// ink will land relative to what is already printed, before committing a sheet
/// to the tray.
pub fn preview_page(diff: &PageDiff, old_gray: &[u8], source_width: usize) -> image::RgbImage {
    const NEW_INK: [u8; 3] = [214, 51, 51];
    const GONE: [u8; 3] = [120, 160, 255];

    let (width, height) = (diff.added.width, diff.added.height);
    let mut out = image::RgbImage::new(width as u32, height as u32);

    for y in 0..height {
        for x in 0..width {
            let at = y * source_width + x;
            let base = old_gray.get(at).copied().unwrap_or(255) as f32;
            // Keep the original as a ghost, so the new ink reads clearly
            // against it without the page looking blank.
            let faded = (255.0 - (255.0 - base) * 0.28).clamp(0.0, 255.0) as u8;
            let pixel = if diff.added.get(x, y) {
                NEW_INK
            } else if diff.removed.get(x, y) {
                GONE
            } else {
                [faded, faded, faded]
            };
            out.put_pixel(x as u32, y as u32, image::Rgb(pixel));
        }
    }
    out
}

#[cfg(test)]
mod tests;
