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

impl ScanRegistration {
    /// Where a point on the scan actually is on the physical sheet.
    pub fn pixel_to_page_mm(&self, px: (f64, f64)) -> (f64, f64) {
        let (dx, dy) = (px.0 - self.origin_px.0, px.1 - self.origin_px.1);
        // Undo the scanner's rotation, then convert pixels to millimetres.
        let theta = (-self.skew_deg).to_radians();
        let (sin_t, cos_t) = theta.sin_cos();
        (
            (cos_t * dx - sin_t * dy) / self.px_per_mm,
            (sin_t * dx + cos_t * dy) / self.px_per_mm,
        )
    }

    /// Where a point on the sheet appears in the scan.
    pub fn page_mm_to_pixel(&self, mm: (f64, f64)) -> (f64, f64) {
        let (x, y) = (mm.0 * self.px_per_mm, mm.1 * self.px_per_mm);
        let theta = self.skew_deg.to_radians();
        let (sin_t, cos_t) = theta.sin_cos();
        (
            cos_t * x - sin_t * y + self.origin_px.0,
            sin_t * x + cos_t * y + self.origin_px.1,
        )
    }

    /// Effective scan resolution in dots per inch.
    pub fn dpi(&self) -> f64 {
        self.px_per_mm * crate::geometry::MM_PER_INCH
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
    /// Skip page detection and treat the whole image as the sheet.
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
        Bounds { x0: 0, y0: 0, x1: width, y1: height }
    } else {
        find_sheet(&gray)
    };

    if bounds.width() < 8 || bounds.height() < 8 {
        return Err(ScanError::Detection(
            "could not find the sheet in this scan — it looks blank or nearly so. \
             Pass the scan resolution instead, or crop it to the page."
                .into(),
        ));
    }

    // Only when background is visible on every side do we know we are looking
    // at the whole sheet and can trust its outline. If the sheet runs to the
    // image edge it may be cropped, and a cropped scan cannot tell us how the
    // paper was turned — the evidence has been cut off.
    let framed = bounds.x0 > 0
        && bounds.y0 > 0
        && bounds.x1 < width
        && bounds.y1 < height;

    let skew_deg = if options.assume_square || !framed {
        0.0
    } else {
        estimate_skew(&gray, bounds, options.max_skew_deg)
    };

    let (sheet_w, sheet_h, origin) = sheet_within(bounds, skew_deg);

    // Average the two axes. A scanner's axes can differ very slightly, and a
    // single similarity has to settle on one number for both.
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

    for level in 0..256usize {
        weight_background += histogram[level] as f64;
        if weight_background == 0.0 {
            continue;
        }
        let weight_foreground = total as f64 - weight_background;
        if weight_foreground == 0.0 {
            break;
        }
        sum_background += level as f64 * histogram[level] as f64;
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
pub fn find_sheet(gray: &GrayImage) -> Bounds {
    let (width, height) = gray.dimensions();
    let threshold = otsu_threshold(gray);
    // Paper is the bright side of the split. Bias upward so heavy text does
    // not drag the level into the paper itself.
    let paper_level = threshold.saturating_add((255 - threshold) / 4);

    let min_run_x = (width / 100).max(16).min(width);
    let min_run_y = (height / 100).max(16).min(height);

    let row_has_sheet: Vec<bool> = (0..height)
        .map(|y| {
            longest_run(width, |x| gray.get_pixel(x, y).0[0] >= paper_level) >= min_run_x
        })
        .collect();
    let col_has_sheet: Vec<bool> = (0..width)
        .map(|x| {
            longest_run(height, |y| gray.get_pixel(x, y).0[0] >= paper_level) >= min_run_y
        })
        .collect();

    let y0 = row_has_sheet.iter().position(|v| *v).unwrap_or(0) as u32;
    let y1 = row_has_sheet
        .iter()
        .rposition(|v| *v)
        .map(|i| i as u32 + 1)
        .unwrap_or(height);
    let x0 = col_has_sheet.iter().position(|v| *v).unwrap_or(0) as u32;
    let x1 = col_has_sheet
        .iter()
        .rposition(|v| *v)
        .map(|i| i as u32 + 1)
        .unwrap_or(width);

    Bounds { x0, y0, x1, y1 }
}

/// Longest unbroken stretch for which `is_set` holds.
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
    let inset = (bounds.width().max(bounds.height()) as f64
        * max_skew_deg.to_radians().sin())
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
    let (min, max) = projected.iter().fold((f64::MAX, f64::MIN), |(lo, hi), v| {
        (lo.min(*v), hi.max(*v))
    });
    if !(max > min) {
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
                    let in_text = row % 3 == 1
                        && sx > sheet_w as f64 * 0.1
                        && sx < sheet_w as f64 * 0.8;
                    if in_text {
                        value = 25;
                    }
                }
                img.put_pixel(x, y, image::Rgb([value, value, value]));
            }
        }
        DynamicImage::ImageRgb8(img)
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
        assert!(threshold >= 20 && threshold < 230, "threshold {threshold}");
        assert!(20 <= threshold, "dark pixels must count as ink");
        assert!(230 > threshold, "bright pixels must not count as ink");
    }

    #[test]
    fn otsu_survives_a_blank_image() {
        let gray = GrayImage::from_pixel(10, 10, Luma([200]));
        let _ = otsu_threshold(&gray); // must not panic or divide by zero
    }

    #[test]
    fn the_sheet_is_found_inside_its_border() {
        let scan = synthetic_scan(A4, 150.0, 60, 0.0, 20);
        let bounds = find_sheet(&scan.to_luma8());

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
        let bounds = find_sheet(&scan.to_luma8());
        assert_eq!(bounds.x0, 0);
        assert_eq!(bounds.y0, 0);
    }

    #[test]
    fn skew_is_recovered_from_the_text() {
        for truth in [-2.0, -0.8, 0.0, 0.7, 1.5, 3.0] {
            let scan = synthetic_scan(A4, 150.0, 40, truth, 24);
            let gray = scan.to_luma8();
            let bounds = find_sheet(&gray);
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
        let bounds = find_sheet(&gray);
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
                (recovered.0 - truth_mm.0).abs() < 1.5
                    && (recovered.1 - truth_mm.1).abs() < 1.5,
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
        assert!(matches!(register(&img, ScanOptions::new(A4)), Err(ScanError::Empty)));
    }

    #[test]
    fn landscape_sheets_register_too() {
        let landscape = PageSize::new(297.0, 210.0);
        let scan = synthetic_scan(landscape, 150.0, 30, 0.0, 18);
        let registration = register(&scan, ScanOptions::new(landscape)).unwrap();
        assert_relative_eq!(registration.dpi(), 150.0, epsilon = 4.0);
    }
}
