//! Adding words to a page you only have as a scan.
//!
//! The other workflows start from a digital document, so Onionskin already
//! knows where everything is. A scan knows nothing. It is a photograph of a
//! sheet, and the sheet is never quite where the scanner thinks: it sits a few
//! millimetres off the glass corner, it is turned by a degree or so, and the
//! image is in pixels rather than millimetres.
//!
//! That matters because the delta is printed onto the *physical* sheet, which
//! is not skewed at all — the skew is an artefact of scanning. So a point the
//! user picks on the scan has to be carried back through the scanner's own
//! error before it means anything on paper. Getting this wrong puts ink in the
//! wrong place while the preview looks perfect, which is the worst thing this
//! app can do.
//!
//! [`register`] works out that mapping: where the sheet sits in the scan, how
//! far it is turned, and how many pixels make a millimetre.

use image::{GenericImageView, GrayImage, ImageError};

use crate::geometry::PageSize;

/// How the sheet sits inside a scan.
///
/// The mapping is deliberately a similarity — offset, uniform scale and
/// rotation. A flatbed introduces exactly those and nothing else; fitting
/// anything more general would mostly fit noise.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScanRegistration {
    /// The physical sheet the scan is of.
    pub page: PageSize,
    /// Scan resolution, in pixels per millimetre.
    pub px_per_mm: f64,
    /// How far the sheet is turned in the scan, degrees clockwise.
    pub skew_deg: f64,
    /// Where the sheet's top-left corner sits, in scan pixels.
    pub origin_px: (f64, f64),
}

/// The registration's arithmetic with its trigonometry already done.
///
/// The same mapping as the methods on [`ScanRegistration`], and the only
/// implementation of it — those methods build one of these and use it. Worth
/// having separately because reading a page converts millions of points, and a
/// sine and a cosine per point is then most of the work.
#[derive(Debug, Clone, Copy)]
pub struct Mapping {
    origin_px: (f64, f64),
    px_per_mm: f64,
    sin_t: f64,
    cos_t: f64,
}

impl Mapping {
    /// Where a point on the scan actually is on the physical sheet.
    pub fn pixel_to_page_mm(&self, px: (f64, f64)) -> (f64, f64) {
        let (dx, dy) = (px.0 - self.origin_px.0, px.1 - self.origin_px.1);
        // Undo the scanner's rotation, then convert pixels to millimetres.
        (
            (self.cos_t * dx + self.sin_t * dy) / self.px_per_mm,
            (-self.sin_t * dx + self.cos_t * dy) / self.px_per_mm,
        )
    }

    /// Where a point on the sheet appears in the scan.
    pub fn page_mm_to_pixel(&self, mm: (f64, f64)) -> (f64, f64) {
        let (x, y) = (mm.0 * self.px_per_mm, mm.1 * self.px_per_mm);
        (
            self.cos_t * x - self.sin_t * y + self.origin_px.0,
            self.sin_t * x + self.cos_t * y + self.origin_px.1,
        )
    }
}

impl ScanRegistration {
    /// This registration's mapping, with the trigonometry done once.
    pub fn mapping(&self) -> Mapping {
        let (sin_t, cos_t) = self.skew_deg.to_radians().sin_cos();
        Mapping {
            origin_px: self.origin_px,
            px_per_mm: self.px_per_mm,
            sin_t,
            cos_t,
        }
    }

    /// Where a point on the scan actually is on the physical sheet.
    pub fn pixel_to_page_mm(&self, px: (f64, f64)) -> (f64, f64) {
        self.mapping().pixel_to_page_mm(px)
    }

    /// Where a point on the sheet appears in the scan.
    pub fn page_mm_to_pixel(&self, mm: (f64, f64)) -> (f64, f64) {
        self.mapping().page_mm_to_pixel(mm)
    }

    /// Effective scan resolution in dots per inch.
    pub fn dpi(&self) -> f64 {
        self.px_per_mm * crate::geometry::MM_PER_INCH
    }

    /// The sheet alone, straightened onto the paper's own grid.
    ///
    /// Everything measured on a page is measured in millimetres from the
    /// corner of the paper, and a crooked scan makes that a different piece of
    /// arithmetic for every pixel. Sampling the scan once into a picture that
    /// *is* the paper turns it back into index-by-millimetre, which anything
    /// downstream can then treat as an ordinary page — the scanner's backing,
    /// the margin round the sheet and the half-degree it sat at are all gone.
    ///
    /// Nearest neighbour, deliberately. The callers of this are looking for
    /// where the ink is and where it is not, at a scale of millimetres;
    /// interpolating would soften every edge to answer a question nobody
    /// asked, and cost four times the arithmetic to do it.
    pub fn flatten(&self, gray: &image::GrayImage, dpi: f64) -> image::GrayImage {
        let px_per_mm = dpi / crate::geometry::MM_PER_INCH;
        let width = (self.page.width_mm * px_per_mm).round().max(1.0) as u32;
        let height = (self.page.height_mm * px_per_mm).round().max(1.0) as u32;
        let (from_width, from_height) = gray.dimensions();

        let mut flat = image::GrayImage::from_pixel(width, height, image::Luma([255]));
        for y in 0..height {
            let y_mm = (y as f64 + 0.5) / px_per_mm;
            for x in 0..width {
                let x_mm = (x as f64 + 0.5) / px_per_mm;
                let (sx, sy) = self.page_mm_to_pixel((x_mm, y_mm));
                // Off the edge of the scan is paper nobody photographed, and
                // white is the honest answer: there is certainly no ink there.
                if sx < 0.0 || sy < 0.0 {
                    continue;
                }
                let (sx, sy) = (sx as u32, sy as u32);
                if sx >= from_width || sy >= from_height {
                    continue;
                }
                flat.put_pixel(x, y, *gray.get_pixel(sx, sy));
            }
        }
        flat
    }

    pub fn describe(&self) -> String {
        format!(
            "{} at {:.0} dpi, sheet turned {:+.2}°, top-left at ({:.0}, {:.0}) px",
            self.page.describe(),
            self.dpi(),
            self.skew_deg,
            self.origin_px.0,
            self.origin_px.1
        )
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ScanError {
    #[error("could not read the scan: {0}")]
    Image(#[from] ImageError),
    #[error("the scan is empty")]
    Empty,
    #[error("{0}")]
    Detection(String),
}

/// Tuning for [`register`]. The defaults suit an ordinary flatbed scan.
#[derive(Debug, Clone, Copy)]
pub struct ScanOptions {
    /// The physical size of the sheet that was scanned.
    pub page: PageSize,
    /// Take the image to be the sheet exactly: no detection, no straightening.
    pub assume_cropped: bool,
    /// Skip skew estimation and take the sheet as square to the scan.
    pub assume_square: bool,
    /// Largest skew worth searching for, in degrees.
    pub max_skew_deg: f64,
}

impl ScanOptions {
    pub fn new(page: PageSize) -> Self {
        Self {
            page,
            assume_cropped: false,
            assume_square: false,
            max_skew_deg: 5.0,
        }
    }
}

/// A rectangle of the scan, in pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    pub x0: u32,
    pub y0: u32,
    pub x1: u32,
    pub y1: u32,
}

impl Bounds {
    pub fn width(&self) -> u32 {
        self.x1.saturating_sub(self.x0)
    }
    pub fn height(&self) -> u32 {
        self.y1.saturating_sub(self.y0)
    }
}

/// Work out how the sheet sits in a scan.
pub fn register(
    image: &image::DynamicImage,
    options: ScanOptions,
) -> Result<ScanRegistration, ScanError> {
    let (width, height) = image.dimensions();
    if width == 0 || height == 0 {
        return Err(ScanError::Empty);
    }
    let gray = image.to_luma8();

    let bounds = if options.assume_cropped {
        Bounds {
            x0: 0,
            y0: 0,
            x1: width,
            y1: height,
        }
    } else {
        find_sheet(&gray).ok_or_else(|| {
            ScanError::Detection(
                "no sheet of paper could be found in this image — it has no bright \
                 region at all.\n    Check it is a scan of a document, or pass \
                 --cropped if the image really is the sheet."
                    .into(),
            )
        })?
    };

    if bounds.width() < 8 || bounds.height() < 8 {
        return Err(ScanError::Detection(
            "could not find the sheet in this scan — it looks blank or nearly so. \
             Pass the scan resolution instead, or crop it to the page."
                .into(),
        ));
    }

    // A turned sheet does not fill the box drawn around it, so its four
    // corners show scanner backing. That, not a gap at the edges of the image,
    // is the evidence that we are seeing the whole sheet and can measure how
    // far it is turned: a modest margin plus a few degrees of turn is enough
    // for the paper to touch every edge, and requiring a visible border there
    // would silently abandon the correction exactly when it is most needed.
    // Measure the lean first and judge afterwards. Deciding up front whether
    // the sheet "looks turned" — by checking whether its corners show backing
    // — misses small angles entirely, because a sheet a third of a degree off
    // barely intrudes on its own corners. A third of a degree is still a
    // millimetre and a half by the foot of an A4 page.
    let skew_deg = if options.assume_cropped || options.assume_square {
        // Declaring the scan cropped is a statement that the image *is* the
        // sheet, so there is nothing to find and nothing to straighten.
        0.0
    } else {
        edge_skew(&gray, bounds)
            .filter(|angle| angle.abs() <= options.max_skew_deg)
            .unwrap_or(0.0)
    };

    // A turned sheet whose box runs to the edge of the image has had its
    // corners cut off, and a cut-off outline cannot say how big the paper is.
    // Guessing from it misplaces every addition by millimetres while looking
    // perfectly convincing, so this has to be refused rather than estimated.
    // A sheet turned far enough to be clipped may leave no measurable edge at
    // all — every row starts already inside the paper — so its lean reads as
    // zero. The corners of the box give it away instead: on a turned sheet
    // they show scanner backing, on a square one they show paper.
    let touches_edge =
        bounds.x0 == 0 || bounds.y0 == 0 || bounds.x1 >= width || bounds.y1 >= height;
    let looks_turned =
        skew_deg.abs() > 0.05 || (!options.assume_cropped && corners_show_backing(&gray, bounds));
    if touches_edge && looks_turned {
        return Err(ScanError::Detection(
            "the sheet is lying at an angle and runs off the edge of this scan, so \
             Onionskin cannot tell how big the paper is.\n    Scan it again with a \
             margin all round, straighten it on the glass, or pass --cropped if the \
             image really is the sheet."
                .into(),
        ));
    }

    let (sheet_w, sheet_h, origin) = sheet_within(bounds, skew_deg);

    // Average the two axes. A scanner's axes can differ very slightly, and a
    // single similarity has to settle on one number for both.
    // The sheet found should be the shape of the paper the user named. When it
    // is not, something is wrong that no amount of arithmetic will fix — the
    // wrong page size, two sheets on the glass, a photo with the desk in it —
    // and every addition would land at a scale quietly derived from it.
    let found_aspect = sheet_w / sheet_h;
    let page_aspect = options.page.width_mm / options.page.height_mm;
    if page_aspect > 0.0 && (found_aspect / page_aspect - 1.0).abs() > 0.08 {
        let implied = PageSize::new(
            options.page.height_mm * found_aspect,
            options.page.height_mm,
        );
        // Named rather than merely measured. "It looks more like 229.5×297.0
        // mm" is true and useless: nobody knows what paper that is, and the
        // thing they have to type next is a name. So the shape is compared
        // against the sizes Onionskin knows and the answer says "letter",
        // which can be pasted straight back into --page.
        let shape = if found_aspect > 0.0 {
            (1.0 / found_aspect).max(found_aspect)
        } else {
            0.0
        };
        let fits = crate::geometry::pages_shaped_like(shape, 0.03);
        let guess = if fits.is_empty() {
            format!(
                "It looks more like {}, which is no paper size Onionskin knows by \n    \
                 name — give it as --page WIDTHxHEIGHT in millimetres.",
                implied.describe()
            )
        } else {
            format!(
                "It looks like {}. Try:  --page {}",
                fits.join(" or "),
                fits[0]
            )
        };
        return Err(ScanError::Detection(format!(
            "the sheet found in this scan is the wrong shape for {}.\n    \
             {guess}\n    Check too that only one sheet is on the glass.",
            options.page.describe(),
        )));
    }

    let px_per_mm = (sheet_w / options.page.width_mm + sheet_h / options.page.height_mm) / 2.0;
    if !(px_per_mm.is_finite() && px_per_mm > 0.0) {
        return Err(ScanError::Detection(
            "the sheet's size in this scan does not make sense; check the page size".into(),
        ));
    }

    Ok(ScanRegistration {
        page: options.page,
        px_per_mm,
        skew_deg,
        origin_px: origin,
    })
}

/// Recover a turned sheet's true size and top-left corner from its bounding box.
///
/// A sheet lying at an angle does not fill the box drawn around it: the box is
/// wider and taller than the paper, and the paper's corners sit part-way along
/// the box's edges. Taking the box for the sheet inflates the scale by a couple
/// of percent, which is several millimetres by the far side of the page — worse
/// than the misregistration the whole app exists to remove.
///
/// With the box `W × H` and the turn `θ`, the paper `w × h` satisfies
/// `W = w·cosθ + h·sinθ` and `H = w·sinθ + h·cosθ`, which solves directly.
fn sheet_within(bounds: Bounds, skew_deg: f64) -> (f64, f64, (f64, f64)) {
    let box_w = bounds.width() as f64;
    let box_h = bounds.height() as f64;
    let corner = (bounds.x0 as f64, bounds.y0 as f64);

    let turn = skew_deg.abs().to_radians();
    let (sin_t, cos_t) = turn.sin_cos();
    let determinant = cos_t * cos_t - sin_t * sin_t; // cos(2θ)
    if turn < 1e-9 || determinant.abs() < 1e-6 {
        return (box_w, box_h, corner);
    }

    let sheet_w = (box_w * cos_t - box_h * sin_t) / determinant;
    let sheet_h = (box_h * cos_t - box_w * sin_t) / determinant;
    if !(sheet_w.is_finite() && sheet_h.is_finite()) || sheet_w <= 0.0 || sheet_h <= 0.0 {
        return (box_w, box_h, corner);
    }

    // Which edge of the box the paper's top-left corner rests against depends
    // on which way the sheet is turned.
    let origin = if skew_deg > 0.0 {
        (corner.0 + sheet_h * sin_t, corner.1)
    } else {
        (corner.0, corner.1 + sheet_w * sin_t)
    };
    (sheet_w, sheet_h, origin)
}

/// Otsu's threshold: the grey level that best splits the image in two.
///
/// Scans vary enormously in exposure, so a fixed cut-off would work on one
/// scanner and fail on the next. Otsu picks the level from the image itself.
pub fn otsu_threshold(gray: &GrayImage) -> u8 {
    let mut histogram = [0u64; 256];
    for pixel in gray.pixels() {
        histogram[pixel.0[0] as usize] += 1;
    }
    otsu_of_histogram(&histogram)
}

/// Otsu's threshold over a tally of grey levels.
///
/// Separated from the image so a caller can choose which pixels count. Reading
/// text needs exactly that: a flatbed's dark backing around the sheet is a
/// third cluster, and left in the tally it drags the split away from the one
/// that matters — ink against paper — and finds no letters at all.
pub fn otsu_of_histogram(histogram: &[u64; 256]) -> u8 {
    let total: u64 = histogram.iter().sum();
    if total == 0 {
        return 128;
    }

    let sum_all: f64 = histogram
        .iter()
        .enumerate()
        .map(|(level, count)| level as f64 * *count as f64)
        .sum();

    let (mut sum_background, mut weight_background) = (0.0f64, 0.0f64);
    let mut best_variance = -1.0f64;
    // On a clean two-level image every level in the gap scores identically.
    // Taking the first would sit the threshold hard against the dark cluster,
    // and a later `pixel < threshold` test would then find no ink at all — so
    // track the whole winning plateau and take its middle.
    let (mut first_best, mut last_best) = (128usize, 128usize);

    for (level, count) in histogram.iter().enumerate() {
        weight_background += *count as f64;
        if weight_background == 0.0 {
            continue;
        }
        let weight_foreground = total as f64 - weight_background;
        if weight_foreground == 0.0 {
            break;
        }
        sum_background += level as f64 * *count as f64;
        let mean_background = sum_background / weight_background;
        let mean_foreground = (sum_all - sum_background) / weight_foreground;
        let between = weight_background
            * weight_foreground
            * (mean_background - mean_foreground)
            * (mean_background - mean_foreground);
        if between > best_variance * (1.0 + 1e-12) {
            best_variance = between;
            first_best = level;
            last_best = level;
        } else if between >= best_variance * (1.0 - 1e-12) {
            last_best = level;
        }
    }
    ((first_best + last_best) / 2) as u8
}

/// Find the sheet: the bright region sitting on the scanner's darker backing.
///
/// Detection is by the longest unbroken run of paper in each row and column,
/// not by how much of the line is bright. A sheet lying at an angle meets its
/// own bounding box at four corners, and near those corners a row is almost
/// entirely backing — a "mostly bright" test throws those rows away and
/// returns a box smaller than the paper, which then reads as a scan at the
/// wrong resolution. A run is also far steadier against dust and speckle than
/// a count of bright pixels scattered anywhere along the line.
///
/// Many scanners already crop to the page, in which case the whole image is
/// the sheet and this returns its full extent — the right answer, not a
/// failure.
/// What shape the sheet in this scan is: its height divided by its width.
///
/// Enough to tell an A-series page from a US one, which is the paper mistake
/// that actually costs somebody a sheet — see
/// [`crate::geometry::pages_shaped_like`]. `None` when no sheet could be made
/// out at all, which is not an error here: a scan with no findable paper in it
/// has bigger problems than its shape, and they are reported elsewhere.
pub fn sheet_shape(image: &image::DynamicImage) -> Option<f64> {
    let gray = image.to_luma8();
    let bounds = find_sheet(&gray)?;
    let (width, height) = (bounds.width(), bounds.height());
    if width == 0 || height == 0 {
        return None;
    }
    // Longest side down, so a sheet scanned sideways is still recognised as
    // the size it is rather than as nothing at all.
    let (short, long) = if width <= height {
        (width, height)
    } else {
        (height, width)
    };
    Some(long as f64 / short as f64)
}

pub fn find_sheet(gray: &GrayImage) -> Option<Bounds> {
    let (width, height) = gray.dimensions();
    let threshold = otsu_threshold(gray);
    // Paper is the bright side of the split. Bias upward so heavy text does
    // not drag the level into the paper itself.
    let global_paper = threshold.saturating_add((255 - threshold) / 4);

    let min_run_x = (width / 100).max(16).min(width);
    let min_run_y = (height / 100).max(16).min(height);

    let row_has_sheet: Vec<bool> = (0..height)
        .map(|y| {
            let level = line_paper_level(gray, global_paper, width, |x| (x, y));
            longest_run(width, |x| gray.get_pixel(x, y).0[0] >= level) >= min_run_x
        })
        .collect();
    let col_has_sheet: Vec<bool> = (0..width)
        .map(|x| {
            let level = line_paper_level(gray, global_paper, height, |y| (x, y));
            longest_run(height, |y| gray.get_pixel(x, y).0[0] >= level) >= min_run_y
        })
        .collect();

    // No paper anywhere. Falling back to "the whole image" here would hand
    // back a confident resolution for a photograph of a dark room.
    let y0 = row_has_sheet.iter().position(|v| *v)? as u32;
    let y1 = row_has_sheet.iter().rposition(|v| *v)? as u32 + 1;
    let x0 = col_has_sheet.iter().position(|v| *v)? as u32;
    let x1 = col_has_sheet.iter().rposition(|v| *v)? as u32 + 1;

    Some(Bounds { x0, y0, x1, y1 })
}

/// The brightness that separates paper from backing along one line.
///
/// Taken from the line's own range wherever the line spans both, so a scan
/// with the lid ajar — or any photograph, where one side of the page can be
/// darker than the backing is on the other — is read correctly. A line that is
/// all one thing has no range to work from and falls back to the level for the
/// whole image; without that, a row of uniform backing would look like an
/// unbroken run of paper.
fn line_paper_level(
    gray: &GrayImage,
    global_paper: u8,
    length: u32,
    at: impl Fn(u32) -> (u32, u32),
) -> u8 {
    // Every pixel, not a sample of them: a line that only clips the corner of
    // a turned sheet holds a sliver of paper a few pixels wide, and a stride
    // that steps over it would flip that line's verdict from one row to the
    // next and make the sheet's measured outline jitter.
    let (mut low, mut high) = (255u8, 0u8);
    for step in 0..length {
        let (x, y) = at(step);
        let value = gray.get_pixel(x, y).0[0];
        low = low.min(value);
        high = high.max(value);
    }

    const MIN_CONTRAST: u8 = 45;
    if high.saturating_sub(low) < MIN_CONTRAST {
        return global_paper;
    }
    low + (high - low) / 2
}

/// Do the corners of the sheet's bounding box show scanner backing?
///
/// A sheet lying at an angle meets its bounding box at four corners, leaving
/// backing in each. A sheet square to the scan fills its box to the corners.
/// This is the only evidence of a turn that survives when the paper runs off
/// the edge of the image and its edges cannot be traced.
fn corners_show_backing(gray: &GrayImage, bounds: Bounds) -> bool {
    let threshold = otsu_threshold(gray);
    let paper_level = threshold.saturating_add((255 - threshold) / 4);

    // A patch rather than a single pixel: one speck of dust should not decide.
    let patch = (bounds.width().min(bounds.height()) / 40).clamp(2, 32);
    let corners = [
        (bounds.x0, bounds.y0),
        (bounds.x1.saturating_sub(patch), bounds.y0),
        (bounds.x0, bounds.y1.saturating_sub(patch)),
        (
            bounds.x1.saturating_sub(patch),
            bounds.y1.saturating_sub(patch),
        ),
    ];

    let dark_corners = corners
        .iter()
        .filter(|(cx, cy)| {
            let (mut dark, mut total) = (0u32, 0u32);
            for y in *cy..(*cy + patch).min(gray.height()) {
                for x in *cx..(*cx + patch).min(gray.width()) {
                    total += 1;
                    if gray.get_pixel(x, y).0[0] < paper_level {
                        dark += 1;
                    }
                }
            }
            total > 0 && dark * 2 > total
        })
        .count();

    dark_corners >= 3
}

/// Longest unbroken stretch for which `is_set` holds./// Longest unbroken stretch for which `is_set` holds.
fn longest_run(length: u32, is_set: impl Fn(u32) -> bool) -> u32 {
    let (mut best, mut current) = (0u32, 0u32);
    for index in 0..length {
        if is_set(index) {
            current += 1;
            best = best.max(current);
        } else {
            current = 0;
        }
    }
    best
}

/// Estimate how far the sheet is turned, from its own edge.
///
/// The left and right edges of the paper are long, straight and high-contrast,
/// which makes them a far better protractor than the text: they give an answer
/// for a sheet turned by a tenth of a degree, and for a blank one. Each row
/// contributes where the paper starts; a straight line through those points
/// leans by exactly the angle the sheet is lying at.
pub fn edge_skew(gray: &GrayImage, bounds: Bounds) -> Option<f64> {
    // Skip the top and bottom eighth: those rows hold the corners, where the
    // edge turns and would drag the fit.
    let skip = bounds.height() / 8;
    let (from, to) = (bounds.y0 + skip, bounds.y1.saturating_sub(skip));
    if to <= from + 8 {
        return None;
    }

    let mut left: Vec<(f64, f64)> = Vec::new();
    let mut right: Vec<(f64, f64)> = Vec::new();
    for y in from..to {
        if let Some(x) = first_paper(gray, bounds, y, true) {
            left.push((y as f64, x as f64));
        }
        if let Some(x) = first_paper(gray, bounds, y, false) {
            right.push((y as f64, x as f64));
        }
    }

    let angles: Vec<f64> = [left, right]
        .into_iter()
        .filter_map(|points| fit_edge_angle(&points))
        .collect();
    if angles.is_empty() {
        return None;
    }
    // Both edges belong to the same sheet, so they should agree; if they do
    // not, the outline is untrustworthy and it is better to say nothing.
    if angles.len() == 2 && (angles[0] - angles[1]).abs() > 1.0 {
        return None;
    }
    Some(angles.iter().sum::<f64>() / angles.len() as f64)
}

/// Where paper starts on this row, coming in from one side.
///
/// Found by the steepest step in brightness rather than by crossing a fixed
/// level. The edge of a sheet is a strong local jump — backing to paper — and
/// stays one however the lighting falls, whereas a single threshold for the
/// whole image quietly loses the far side of a scan taken with the lid ajar,
/// or of any photograph. That failure is silent: the sheet is still found, but
/// its lean is not, and the additions print a degree out.
fn first_paper(gray: &GrayImage, bounds: Bounds, y: u32, from_left: bool) -> Option<u32> {
    // Deliberately narrow. The step at the edge of a sheet is only a pixel or
    // two wide, and a wide window finds it early: the test fires as soon as the
    // far sample reaches the paper, so every row within a window's width of the
    // box boundary reports the same x. That flattens the very lean being
    // measured — a page a third of a degree off came back as perfectly square.
    let reach = 3i64;
    let search = (bounds.width() / 3).max(24);
    let (low_x, high_x) = (bounds.x0 as i64, bounds.x1 as i64 - 1);
    if high_x <= low_x {
        return None;
    }

    // Average a few pixels so a speck of dust on the glass cannot read as the
    // edge of the paper.
    //
    // Clamped to the *image*, not to the sheet's box. Clamping to the box
    // hides the one thing this function exists to find: on a sheet lying
    // square, the box edge is the paper edge exactly, so the backing outside
    // it is never sampled, no step is ever seen there, and the search walks on
    // into the page and stops at the first letter it meets. The lean of the
    // paper is then read off the ragged right-hand margin of the text — which
    // is how a perfectly square sheet came back turned four and a half
    // degrees, and every addition on it landed a centimetre out.
    let edge_x = gray.width() as i64 - 1;
    let sample = |x: i64| -> i32 {
        let mut total = 0i32;
        for offset in -1..=1i64 {
            let px = (x + offset).clamp(0, edge_x) as u32;
            total += gray.get_pixel(px, y).0[0] as i32;
        }
        total / 3
    };

    // The threshold comes from this row's own range, so it means the same on a
    // bright row as on one lying in shadow.
    let (mut low, mut high) = (255i32, 0i32);
    for x in low_x..=high_x {
        let value = sample(x);
        low = low.min(value);
        high = high.max(value);
    }
    // A fifth of the row's range. Generous enough that the far side of a scan
    // taken under a strong light gradient — where the paper may be darker than
    // the backing is on the near side — still shows an edge, and the averaging
    // above keeps that from admitting noise.
    let threshold = (((high - low) * 20) / 100).max(25);

    // Walk inward from the very edge of the box. Starting even a few pixels in
    // would begin the search past the paper's edge on the rows where the sheet
    // reaches furthest, and the first strong step found would then be a letter
    // rather than the edge of the page.
    //
    // Take the *first* step that clears the threshold, not the largest: coming
    // in from outside, the paper's edge is the first thing met, while the
    // largest step on a row of text is the far side of a letter — ink is darker
    // than the backing, so it rises higher coming out of it.
    for step in 0..search as i64 {
        let x = if from_left {
            low_x + step
        } else {
            high_x - step
        };
        let (inner, outer) = if from_left {
            (x + reach, x - reach)
        } else {
            (x - reach, x + reach)
        };
        if sample(inner) - sample(outer) >= threshold {
            return Some(x.clamp(low_x, high_x) as u32);
        }
    }
    None
}

/// Least-squares lean of a set of edge points, with one outlier pass.
///
/// A speck of dust or a torn corner puts a point far off the line; fitting
/// once, discarding what does not fit, and fitting again keeps those from
/// tilting the answer.
fn fit_edge_angle(points: &[(f64, f64)]) -> Option<f64> {
    if points.len() < 16 {
        return None;
    }
    let slope = |set: &[(f64, f64)]| -> Option<(f64, f64)> {
        let n = set.len() as f64;
        let mean_y = set.iter().map(|p| p.0).sum::<f64>() / n;
        let mean_x = set.iter().map(|p| p.1).sum::<f64>() / n;
        let mut num = 0.0;
        let mut den = 0.0;
        for (y, x) in set {
            num += (y - mean_y) * (x - mean_x);
            den += (y - mean_y) * (y - mean_y);
        }
        if den.abs() < 1e-9 {
            return None;
        }
        let a = num / den;
        Some((a, mean_x - a * mean_y))
    };

    let (a, b) = slope(points)?;
    let residuals: Vec<f64> = points
        .iter()
        .map(|(y, x)| (x - (a * y + b)).abs())
        .collect();
    let mean_residual = residuals.iter().sum::<f64>() / residuals.len() as f64;
    let cutoff = (mean_residual * 3.0).max(2.0);
    let kept: Vec<(f64, f64)> = points
        .iter()
        .copied()
        .zip(residuals.iter())
        .filter(|(_, r)| **r <= cutoff)
        .map(|(p, _)| p)
        .collect();

    let (a, _) = if kept.len() >= 16 {
        slope(&kept)?
    } else {
        (a, b)
    };
    // x falls as y rises when the sheet leans clockwise, so the angle is the
    // negated slope.
    Some((-a).atan().to_degrees())
}

/// Estimate how far the sheet is turned, from the text on it.
///
/// Printed text sits in lines, so projecting the ink onto the vertical axis
/// gives sharp peaks and deep troughs — but only when the projection runs
/// along the lines. Tilt away from true and the peaks smear together. So the
/// angle whose projection varies most is the angle the text is at, and this
/// needs no page edges, which a cropped scan does not have.
pub fn estimate_skew(gray: &GrayImage, bounds: Bounds, max_skew_deg: f64) -> f64 {
    let threshold = otsu_threshold(gray);

    // Pull the edges in before looking for ink. A turned sheet does not fill
    // its own bounding box, so the corners of that box are scanner backing —
    // and backing is dark, which reads as ink. Left in, those four triangles
    // are the strongest "lines" on the page and the estimate follows them
    // instead of the text.
    let inset = (bounds.width().max(bounds.height()) as f64 * max_skew_deg.to_radians().sin())
        .ceil() as u32;
    let inset_x = inset.min(bounds.width() * 15 / 100);
    let inset_y = inset.min(bounds.height() * 15 / 100);
    let inner = Bounds {
        x0: bounds.x0 + inset_x,
        y0: bounds.y0 + inset_y,
        x1: bounds.x1.saturating_sub(inset_x),
        y1: bounds.y1.saturating_sub(inset_y),
    };
    if inner.width() < 8 || inner.height() < 8 {
        return 0.0;
    }

    // Sample columns, never rows. Skipping rows would put the ink on a lattice
    // whose comb pattern dominates the histogram at every angle equally,
    // drowning the signal the search is looking for. Columns can be skipped
    // freely: a line of text is wide, so every line is still hit.
    let area = inner.width() as u64 * inner.height() as u64;
    let stride = (area / 400_000).max(1) as u32;

    let mut ink: Vec<(f64, f64)> = Vec::new();
    for y in inner.y0..inner.y1 {
        let mut x = inner.x0;
        while x < inner.x1 {
            if gray.get_pixel(x, y).0[0] <= threshold {
                ink.push((x as f64, y as f64));
            }
            x += stride;
        }
    }

    // Too little ink to be text. Claiming an angle from noise would be worse
    // than admitting there is no evidence.
    if ink.len() < 200 {
        return 0.0;
    }

    let coarse = best_angle(&ink, -max_skew_deg, max_skew_deg, 0.25);
    best_angle(&ink, coarse - 0.25, coarse + 0.25, 0.02)
}

/// Search a range of angles for the one whose ink projection varies most.
fn best_angle(ink: &[(f64, f64)], from_deg: f64, to_deg: f64, step_deg: f64) -> f64 {
    let mut best = (0.0f64, f64::NEG_INFINITY);
    let mut angle = from_deg;
    while angle <= to_deg + 1e-9 {
        let score = projection_variance(ink, angle);
        if score > best.1 {
            best = (angle, score);
        }
        angle += step_deg;
    }
    best.0
}

/// How sharply the ink stacks up when projected at `angle_deg`.
///
/// Printed text sits in lines, so a projection taken along those lines piles
/// the ink into a few tall bins; taken across them it smears evenly. The score
/// is the sum of squared bin counts over the square of the total, which is
/// dimensionless and — unlike a plain variance — stays comparable as the
/// projected range, and so the bin count, grows with the angle.
fn projection_variance(ink: &[(f64, f64)], angle_deg: f64) -> f64 {
    let theta = (-angle_deg).to_radians();
    let (sin_t, cos_t) = theta.sin_cos();

    let projected: Vec<f64> = ink.iter().map(|(x, y)| sin_t * x + cos_t * y).collect();
    let (min, max) = projected
        .iter()
        .fold((f64::MAX, f64::MIN), |(lo, hi), v| (lo.min(*v), hi.max(*v)));
    // Spelled out rather than `!(max > min)` so the NaN case is obvious: a
    // degenerate or non-finite spread has no structure to score.
    if !max.is_finite() || !min.is_finite() || max <= min {
        return 0.0;
    }

    // One bin per pixel of projected distance, so bins mean the same thing at
    // every angle.
    let buckets = ((max - min).ceil() as usize + 1).clamp(1, 40_000);
    let mut histogram = vec![0u32; buckets];
    let scale = (buckets - 1) as f64 / (max - min);
    for value in projected {
        let index = ((value - min) * scale).round() as usize;
        histogram[index.min(buckets - 1)] += 1;
    }

    let total: f64 = ink.len() as f64;
    if total <= 0.0 {
        return 0.0;
    }
    let sum_squares: f64 = histogram.iter().map(|v| (*v as f64) * (*v as f64)).sum();
    sum_squares / (total * total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;
    use image::{DynamicImage, Luma, RgbImage};

    const A4: PageSize = PageSize {
        width_mm: 210.0,
        height_mm: 297.0,
    };

    /// Build a synthetic scan: a bright sheet on a dark backing, carrying
    /// horizontal lines of "text", optionally turned by a known angle.
    fn synthetic_scan(
        page: PageSize,
        dpi: f64,
        margin_px: u32,
        skew_deg: f64,
        lines: usize,
    ) -> DynamicImage {
        let px_per_mm = dpi / crate::geometry::MM_PER_INCH;
        let sheet_w = (page.width_mm * px_per_mm) as u32;
        let sheet_h = (page.height_mm * px_per_mm) as u32;
        let width = sheet_w + margin_px * 2;
        let height = sheet_h + margin_px * 2;

        // Dark scanner backing.
        let mut img = RgbImage::from_pixel(width, height, image::Rgb([40, 40, 40]));

        let centre = (width as f64 / 2.0, height as f64 / 2.0);
        let theta = skew_deg.to_radians();
        let (sin_t, cos_t) = theta.sin_cos();
        // Map every output pixel back to sheet coordinates, so the rotation is
        // exact rather than an approximation of one.
        for y in 0..height {
            for x in 0..width {
                let (dx, dy) = (x as f64 - centre.0, y as f64 - centre.1);
                let sx = cos_t * dx + sin_t * dy + centre.0 - margin_px as f64;
                let sy = -sin_t * dx + cos_t * dy + centre.1 - margin_px as f64;
                if sx < 0.0 || sy < 0.0 || sx >= sheet_w as f64 || sy >= sheet_h as f64 {
                    continue;
                }
                // Paper, with text bands across it.
                let mut value = 245u8;
                if lines > 0 {
                    let band = sheet_h as f64 / (lines as f64 * 3.0);
                    let row = (sy / band) as usize;
                    let in_text =
                        row % 3 == 1 && sx > sheet_w as f64 * 0.1 && sx < sheet_w as f64 * 0.8;
                    if in_text {
                        value = 25;
                    }
                }
                img.put_pixel(x, y, image::Rgb([value, value, value]));
            }
        }
        DynamicImage::ImageRgb8(img)
    }

    /// A sheet lying square, carrying text whose margins are ragged — every
    /// line starting and ending somewhere different, as real text does.
    ///
    /// The shape that matters: the *only* straight vertical line in this image
    /// is the paper's own edge, and it coincides exactly with the sheet's
    /// bounding box. Anything that measures the lean from the text instead
    /// will find one, because the text leans.
    fn square_sheet_with_ragged_text(dpi: f64, margin_px: u32) -> DynamicImage {
        let px_per_mm = dpi / crate::geometry::MM_PER_INCH;
        let sheet_w = (A4.width_mm * px_per_mm) as u32;
        let sheet_h = (A4.height_mm * px_per_mm) as u32;
        let mut img = RgbImage::from_pixel(
            sheet_w + margin_px * 2,
            sheet_h + margin_px * 2,
            image::Rgb([40, 40, 40]),
        );

        for y in 0..sheet_h {
            for x in 0..sheet_w {
                img.put_pixel(x + margin_px, y + margin_px, image::Rgb([245, 245, 245]));
            }
        }

        // Twelve lines, each indented and ended differently, all of them well
        // inside the margins — the raggedness is the point.
        let line_height = (5.0 * px_per_mm) as u32;
        for line in 0..12u32 {
            let top = margin_px + (30.0 * px_per_mm) as u32 + line * line_height;
            let left = margin_px + ((25.0 + (line % 4) as f64 * 4.0) * px_per_mm) as u32;
            let right = margin_px + ((90.0 + (line % 5) as f64 * 11.0) * px_per_mm) as u32;
            for y in top..top + (2.5 * px_per_mm) as u32 {
                for x in left..right.min(sheet_w + margin_px) {
                    img.put_pixel(x, y, image::Rgb([25, 25, 25]));
                }
            }
        }
        DynamicImage::ImageRgb8(img)
    }

    #[test]
    fn a_square_sheet_of_text_is_not_read_as_turned() {
        // The lean must come from the paper's edge. Measured off the text
        // instead, a sheet lying perfectly square reads as several degrees
        // turned, and every addition on it lands a centimetre from where it
        // was asked for — with nothing in the output saying so.
        let image = square_sheet_with_ragged_text(300.0, 30);
        let registration = register(&image, ScanOptions::new(A4)).unwrap();

        assert!(
            registration.skew_deg.abs() < 0.15,
            "a square sheet read as turned {:.2}°",
            registration.skew_deg
        );
        assert_relative_eq!(registration.origin_px.0, 30.0, epsilon = 2.0);
        assert_relative_eq!(registration.origin_px.1, 30.0, epsilon = 2.0);
        assert_relative_eq!(registration.dpi(), 300.0, epsilon = 2.0);
    }

    #[test]
    fn the_edge_is_measured_even_when_it_touches_the_box() {
        // The underlying reason for the test above: on a square sheet the
        // paper's edge and the detected box are the same line, and a search
        // that cannot see outside the box cannot see the edge at all.
        let image = square_sheet_with_ragged_text(300.0, 30).to_luma8();
        let bounds = find_sheet(&image).unwrap();
        let skew = edge_skew(&image, bounds).expect("the paper's edge was not found");

        assert!(skew.abs() < 0.15, "edge read as {skew:.3}°");
    }

    #[test]
    fn text_touching_the_paper_edge_does_not_become_the_edge() {
        // Ink running right up to where the paper ends is the hardest case for
        // an edge finder, and it happens whenever someone scans a page with a
        // full-bleed rule or a stamp near the margin.
        let mut image = square_sheet_with_ragged_text(300.0, 30).to_rgb8();
        let px_per_mm = 300.0 / crate::geometry::MM_PER_INCH;
        let sheet_h = (A4.height_mm * px_per_mm) as u32;
        for y in 30 + sheet_h / 3..30 + sheet_h / 3 + 40 {
            for x in 30..30 + (6.0 * px_per_mm) as u32 {
                image.put_pixel(x, y, image::Rgb([25, 25, 25]));
            }
        }
        let registration = register(&DynamicImage::ImageRgb8(image), ScanOptions::new(A4)).unwrap();

        assert!(
            registration.skew_deg.abs() < 0.2,
            "read as turned {:.2}°",
            registration.skew_deg
        );
    }

    #[test]
    fn otsu_splits_a_two_level_image() {
        let mut gray = GrayImage::from_pixel(100, 100, Luma([20]));
        for y in 0..50 {
            for x in 0..100 {
                gray.put_pixel(x, y, Luma([230]));
            }
        }
        let threshold = otsu_threshold(&gray);
        // It must land strictly between the clusters, so that `<= threshold`
        // selects the dark pixels and nothing else.
        // It must land strictly between the clusters, so `<= threshold`
        // selects the dark pixels and nothing else.
        assert!((20..230).contains(&threshold), "threshold {threshold}");
    }

    #[test]
    fn otsu_survives_a_blank_image() {
        let gray = GrayImage::from_pixel(10, 10, Luma([200]));
        let _ = otsu_threshold(&gray); // must not panic or divide by zero
    }

    #[test]
    fn the_sheet_is_found_inside_its_border() {
        let scan = synthetic_scan(A4, 150.0, 60, 0.0, 20);
        let bounds = find_sheet(&scan.to_luma8()).unwrap();

        assert!((bounds.x0 as i64 - 60).abs() <= 3, "x0 {}", bounds.x0);
        assert!((bounds.y0 as i64 - 60).abs() <= 3, "y0 {}", bounds.y0);
        let expected_w = (A4.width_mm * 150.0 / 25.4) as i64;
        assert!(
            (bounds.width() as i64 - expected_w).abs() <= 4,
            "width {} vs {expected_w}",
            bounds.width()
        );
    }

    #[test]
    fn a_cropped_scan_is_all_sheet() {
        let scan = synthetic_scan(A4, 150.0, 0, 0.0, 20);
        let bounds = find_sheet(&scan.to_luma8()).unwrap();
        assert_eq!(bounds.x0, 0);
        assert_eq!(bounds.y0, 0);
    }

    #[test]
    fn skew_is_recovered_from_the_text() {
        for truth in [-2.0, -0.8, 0.0, 0.7, 1.5, 3.0] {
            let scan = synthetic_scan(A4, 150.0, 40, truth, 24);
            let gray = scan.to_luma8();
            let bounds = find_sheet(&gray).unwrap();
            let found = estimate_skew(&gray, bounds, 5.0);
            assert!(
                (found - truth).abs() < 0.25,
                "skew {truth} recovered as {found}"
            );
        }
    }

    #[test]
    fn skew_estimation_declines_to_guess_from_nothing() {
        // A blank sheet has no text to align to; inventing an angle would be
        // worse than admitting there is no evidence.
        let scan = synthetic_scan(A4, 150.0, 40, 2.0, 0);
        let gray = scan.to_luma8();
        let bounds = find_sheet(&gray).unwrap();
        assert_eq!(estimate_skew(&gray, bounds, 5.0), 0.0);
    }

    #[test]
    fn registration_recovers_resolution() {
        let scan = synthetic_scan(A4, 300.0, 50, 0.0, 20);
        let registration = register(&scan, ScanOptions::new(A4)).unwrap();
        assert_relative_eq!(registration.dpi(), 300.0, epsilon = 4.0);
    }

    #[test]
    fn a_point_maps_to_the_sheet_and_back() {
        let scan = synthetic_scan(A4, 200.0, 30, 1.2, 24);
        let registration = register(&scan, ScanOptions::new(A4)).unwrap();

        for point_mm in [(10.0, 10.0), (105.0, 148.5), (200.0, 280.0)] {
            let pixel = registration.page_mm_to_pixel(point_mm);
            let back = registration.pixel_to_page_mm(pixel);
            assert_relative_eq!(back.0, point_mm.0, epsilon = 1e-6);
            assert_relative_eq!(back.1, point_mm.1, epsilon = 1e-6);
        }
    }

    /// The point of the whole module: a spot picked on the scan must resolve to
    /// the right physical millimetre on the sheet, skew and border included.
    #[test]
    fn a_known_sheet_position_is_recovered_from_the_scan() {
        let dpi = 200.0;
        let scan = synthetic_scan(A4, dpi, 35, 1.5, 24);
        let registration = register(&scan, ScanOptions::new(A4)).unwrap();

        let px_per_mm = dpi / crate::geometry::MM_PER_INCH;
        let margin = 35.0;
        let centre = (
            (A4.width_mm * px_per_mm + margin * 2.0) / 2.0,
            (A4.height_mm * px_per_mm + margin * 2.0) / 2.0,
        );
        let theta = 1.5f64.to_radians();
        let (sin_t, cos_t) = theta.sin_cos();

        for truth_mm in [(40.0, 60.0), (150.0, 200.0), (105.0, 148.5)] {
            // Where that sheet position lands in the synthetic scan.
            let sx = truth_mm.0 * px_per_mm;
            let sy = truth_mm.1 * px_per_mm;
            let (ux, uy) = (sx + margin - centre.0, sy + margin - centre.1);
            let pixel = (
                cos_t * ux - sin_t * uy + centre.0,
                sin_t * ux + cos_t * uy + centre.1,
            );

            let recovered = registration.pixel_to_page_mm(pixel);
            assert!(
                (recovered.0 - truth_mm.0).abs() < 1.5 && (recovered.1 - truth_mm.1).abs() < 1.5,
                "{:?} recovered as {:?}",
                truth_mm,
                recovered
            );
        }
    }

    #[test]
    fn assume_cropped_skips_detection() {
        let scan = synthetic_scan(A4, 150.0, 40, 0.0, 20);
        let mut options = ScanOptions::new(A4);
        options.assume_cropped = true;
        let registration = register(&scan, options).unwrap();
        assert_eq!(registration.origin_px, (0.0, 0.0));
    }

    #[test]
    fn assume_square_skips_skew() {
        let scan = synthetic_scan(A4, 150.0, 40, 2.0, 24);
        let mut options = ScanOptions::new(A4);
        options.assume_square = true;
        assert_eq!(register(&scan, options).unwrap().skew_deg, 0.0);
    }

    #[test]
    fn a_blank_scan_is_refused_rather_than_guessed_at() {
        let img = DynamicImage::ImageRgb8(RgbImage::from_pixel(4, 4, image::Rgb([40, 40, 40])));
        assert!(matches!(
            register(&img, ScanOptions::new(A4)),
            Err(ScanError::Detection(_))
        ));
    }

    #[test]
    fn an_empty_image_is_refused() {
        let img = DynamicImage::ImageRgb8(RgbImage::new(0, 0));
        assert!(matches!(
            register(&img, ScanOptions::new(A4)),
            Err(ScanError::Empty)
        ));
    }

    #[test]
    fn landscape_sheets_register_too() {
        let landscape = PageSize::new(297.0, 210.0);
        let scan = synthetic_scan(landscape, 150.0, 30, 0.0, 18);
        let registration = register(&scan, ScanOptions::new(landscape)).unwrap();
        assert_relative_eq!(registration.dpi(), 150.0, epsilon = 4.0);
    }
}
