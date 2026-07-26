//! The core comparison: what ink is on the edited page that is not on the
//! original.
//!
//! The whole app rests on one asymmetry. Ink that appears in the edited
//! document but not the original is *addable* — a printer can lay it down on
//! the sheet you already have. Ink that appears in the original but not the
//! edited version is not removable; toner does not come off paper. So the
//! removed mask is never printed, but it is the single most valuable signal
//! there is: if anything disappeared from where it used to be, the layout
//! reflowed and the sheet in someone's hand no longer matches the document.

use crate::geometry::{px_to_mm, PageSize, MM_PER_INCH};

/// Pixels at or below this grey level count as ink. Anti-aliased glyph edges
/// run light, so this sits well above pure black.
pub const DEFAULT_INK_THRESHOLD: u8 = 200;

/// How far a mark may move and still count as "the same mark". Absorbs
/// sub-pixel layout jitter between two renders of near-identical content.
pub const DEFAULT_TOLERANCE_MM: f64 = 0.12;

/// Additions closer together than this are reported as one region, so a word
/// comes back as one box rather than five letters.
pub const DEFAULT_GROUP_MM: f64 = 2.0;

/// Specks smaller than this are dropped as rendering noise.
pub const DEFAULT_MIN_REGION_MM2: f64 = 0.05;

/// A boolean image, one bit of meaning per pixel.
#[derive(Debug, Clone, PartialEq)]
pub struct Mask {
    pub width: usize,
    pub height: usize,
    pub bits: Vec<bool>,
}

impl Mask {
    pub fn blank(width: usize, height: usize) -> Mask {
        Mask {
            width,
            height,
            bits: vec![false; width * height],
        }
    }

    #[inline]
    pub fn get(&self, x: usize, y: usize) -> bool {
        self.bits[y * self.width + x]
    }

    #[inline]
    pub fn set(&mut self, x: usize, y: usize, on: bool) {
        self.bits[y * self.width + x] = on;
    }

    pub fn count(&self) -> usize {
        self.bits.iter().filter(|b| **b).count()
    }

    pub fn any(&self) -> bool {
        self.bits.iter().any(|b| *b)
    }

    /// Free the pixels, keeping the shape.
    ///
    /// A page at 400 dpi is a thirteen-megapixel mask, and holding two per page
    /// for a long document is the difference between a few hundred megabytes
    /// and a few kilobytes. Everything downstream needs is measured before this
    /// is called.
    pub fn release(&mut self) {
        self.bits = Vec::new();
    }
}

/// A rectangle of changed ink, in page-space millimetres.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct Region {
    pub x0_mm: f64,
    pub y0_mm: f64,
    pub x1_mm: f64,
    pub y1_mm: f64,
    pub ink_mm2: f64,
    /// The same box in pixels: x0, y0, x1, y1, with x1/y1 exclusive.
    #[serde(skip)]
    pub px_bbox: (usize, usize, usize, usize),
}

impl Region {
    pub fn width_mm(&self) -> f64 {
        self.x1_mm - self.x0_mm
    }
    pub fn height_mm(&self) -> f64 {
        self.y1_mm - self.y0_mm
    }
    pub fn area_mm2(&self) -> f64 {
        self.width_mm() * self.height_mm()
    }

    /// The same region grown by `pad_mm`, clipped to the page.
    pub fn padded(&self, pad_mm: f64, page: PageSize) -> Region {
        Region {
            x0_mm: (self.x0_mm - pad_mm).max(0.0),
            y0_mm: (self.y0_mm - pad_mm).max(0.0),
            x1_mm: (self.x1_mm + pad_mm).min(page.width_mm),
            y1_mm: (self.y1_mm + pad_mm).min(page.height_mm),
            ink_mm2: self.ink_mm2,
            px_bbox: self.px_bbox,
        }
    }
}

/// Everything learned by comparing one page of both documents.
#[derive(Debug, Clone)]
pub struct PageDiff {
    pub index: usize,
    pub size: PageSize,
    pub dpi: f64,
    pub added: Mask,
    pub removed: Mask,
    pub added_px: usize,
    pub removed_px: usize,
    pub added_regions: Vec<Region>,
    pub removed_regions: Vec<Region>,
}

impl PageDiff {
    /// A diff for a page that exists in only one of the two documents.
    pub fn blank(size: PageSize, dpi: f64, index: usize) -> PageDiff {
        let (w, h) = size.px_size(dpi);
        PageDiff {
            index,
            size,
            dpi,
            added: Mask::blank(w as usize, h as usize),
            removed: Mask::blank(w as usize, h as usize),
            added_px: 0,
            removed_px: 0,
            added_regions: Vec::new(),
            removed_regions: Vec::new(),
        }
    }

    /// Drop the pixel masks, keeping every derived measurement.
    pub fn release(&mut self) {
        self.added.release();
        self.removed.release();
    }

    pub fn px_area_mm2(&self) -> f64 {
        let side = MM_PER_INCH / self.dpi;
        side * side
    }

    pub fn added_ink_mm2(&self) -> f64 {
        self.added_px as f64 * self.px_area_mm2()
    }

    pub fn removed_ink_mm2(&self) -> f64 {
        self.removed_px as f64 * self.px_area_mm2()
    }

    pub fn has_additions(&self) -> bool {
        !self.added_regions.is_empty()
    }

    /// Bounding box of every addition on the page.
    pub fn bounds_mm(&self) -> Option<(f64, f64, f64, f64)> {
        let first = self.added_regions.first()?;
        let mut bounds = (first.x0_mm, first.y0_mm, first.x1_mm, first.y1_mm);
        for region in &self.added_regions[1..] {
            bounds.0 = bounds.0.min(region.x0_mm);
            bounds.1 = bounds.1.min(region.y0_mm);
            bounds.2 = bounds.2.max(region.x1_mm);
            bounds.3 = bounds.3.max(region.y1_mm);
        }
        Some(bounds)
    }
}

/// Which pixels of a greyscale page are ink.
pub fn ink_mask(gray: &[u8], width: usize, height: usize, threshold: u8) -> Mask {
    Mask {
        width,
        height,
        bits: gray[..width * height]
            .iter()
            .map(|v| *v <= threshold)
            .collect(),
    }
}

/// Grow a mask by `radius_px` in every direction.
///
/// Dilation by a square is *separable*: growing by r horizontally and then by r
/// vertically gives the same result as one (2r+1)² window, but costs O(r)
/// passes instead of O(r²) comparisons per pixel. On a thirteen-megapixel page
/// that is the difference between the better part of a second and a few
/// milliseconds — this is the hottest operation in the whole app, since it runs
/// twice per page.
pub fn dilate(mask: &Mask, radius_px: usize) -> Mask {
    if radius_px == 0 {
        return mask.clone();
    }
    let (w, h) = (mask.width, mask.height);

    let mut grown = mask.clone();
    for shift in 1..=radius_px {
        if shift >= w {
            break;
        }
        for y in 0..h {
            let row = y * w;
            for x in shift..w {
                // Left neighbour reaches right, and vice versa.
                if mask.bits[row + x - shift] {
                    grown.bits[row + x] = true;
                }
                if mask.bits[row + x] {
                    grown.bits[row + x - shift] = true;
                }
            }
        }
    }

    let mut out = grown.clone();
    for shift in 1..=radius_px {
        if shift >= h {
            break;
        }
        for y in shift..h {
            let (here, above) = (y * w, (y - shift) * w);
            for x in 0..w {
                if grown.bits[above + x] {
                    out.bits[here + x] = true;
                }
                if grown.bits[here + x] {
                    out.bits[above + x] = true;
                }
            }
        }
    }
    out
}

/// Group set pixels into regions with exact bounding boxes.
///
/// Connectivity is resolved on a coarse grid of `group_mm` cells rather than
/// per pixel: at 400 dpi an A4 page is thirteen megapixels but only about
/// sixty thousand cells, so the flood fill stays cheap while still merging the
/// letters of a word into one box. Bounding boxes are then measured back at
/// full resolution, cell by cell, so nothing is rounded up to the grid.
pub fn label_regions(mask: &Mask, dpi: f64, group_mm: f64, min_area_mm2: f64) -> Vec<Region> {
    if mask.bits.is_empty() || !mask.any() {
        return Vec::new();
    }
    let (w, h) = (mask.width, mask.height);
    let cell = ((group_mm * dpi / MM_PER_INCH).round() as usize).max(1);
    let gh = h.div_ceil(cell);
    let gw = w.div_ceil(cell);

    // Which coarse cells hold any ink at all.
    let mut grid = vec![false; gw * gh];
    for y in 0..h {
        let row = y * w;
        let gy = y / cell;
        for x in 0..w {
            if mask.bits[row + x] {
                grid[gy * gw + x / cell] = true;
            }
        }
    }

    let px_mm2 = (MM_PER_INCH / dpi).powi(2);
    let mut seen = vec![false; gw * gh];
    let mut regions: Vec<Region> = Vec::new();
    let mut queue: std::collections::VecDeque<(usize, usize)> = std::collections::VecDeque::new();
    let mut cells: Vec<(usize, usize)> = Vec::new();

    for gy in 0..gh {
        for gx in 0..gw {
            if !grid[gy * gw + gx] || seen[gy * gw + gx] {
                continue;
            }
            seen[gy * gw + gx] = true;
            queue.clear();
            cells.clear();
            queue.push_back((gy, gx));

            while let Some((cy, cx)) = queue.pop_front() {
                cells.push((cy, cx));
                // Eight-connected: a word's letters touch diagonally as often
                // as squarely once the grid is this coarse.
                for dy in -1i64..=1 {
                    for dx in -1i64..=1 {
                        let ny = cy as i64 + dy;
                        let nx = cx as i64 + dx;
                        if ny < 0 || nx < 0 || ny >= gh as i64 || nx >= gw as i64 {
                            continue;
                        }
                        let (ny, nx) = (ny as usize, nx as usize);
                        if grid[ny * gw + nx] && !seen[ny * gw + nx] {
                            seen[ny * gw + nx] = true;
                            queue.push_back((ny, nx));
                        }
                    }
                }
            }

            // Back to full resolution for the box itself.
            let (mut x0, mut y0) = (usize::MAX, usize::MAX);
            let (mut x1, mut y1) = (0usize, 0usize);
            let mut ink_px = 0usize;
            let mut found = false;

            for &(cy, cx) in &cells {
                let r0 = cy * cell;
                let c0 = cx * cell;
                for y in r0..(r0 + cell).min(h) {
                    let row = y * w;
                    for x in c0..(c0 + cell).min(w) {
                        if !mask.bits[row + x] {
                            continue;
                        }
                        found = true;
                        ink_px += 1;
                        x0 = x0.min(x);
                        y0 = y0.min(y);
                        x1 = x1.max(x + 1);
                        y1 = y1.max(y + 1);
                    }
                }
            }
            if !found {
                continue;
            }
            let ink_mm2 = ink_px as f64 * px_mm2;
            if ink_mm2 < min_area_mm2 {
                continue;
            }
            regions.push(Region {
                x0_mm: px_to_mm(x0 as f64, dpi),
                y0_mm: px_to_mm(y0 as f64, dpi),
                x1_mm: px_to_mm(x1 as f64, dpi),
                y1_mm: px_to_mm(y1 as f64, dpi),
                ink_mm2,
                px_bbox: (x0, y0, x1, y1),
            });
        }
    }

    // Reading order, with y rounded so that two boxes on the same line of text
    // sort left to right rather than by a hundredth of a millimetre.
    regions.sort_by(|a, b| {
        let ay = (a.y0_mm * 10.0).round();
        let by = (b.y0_mm * 10.0).round();
        ay.partial_cmp(&by)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(
                a.x0_mm
                    .partial_cmp(&b.x0_mm)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
    });
    regions
}

/// How a page is compared.
#[derive(Debug, Clone, Copy)]
pub struct DiffOptions {
    pub ink_threshold: u8,
    pub tolerance_mm: f64,
    pub group_mm: f64,
    pub min_region_mm2: f64,
}

impl Default for DiffOptions {
    fn default() -> Self {
        Self {
            ink_threshold: DEFAULT_INK_THRESHOLD,
            tolerance_mm: DEFAULT_TOLERANCE_MM,
            group_mm: DEFAULT_GROUP_MM,
            min_region_mm2: DEFAULT_MIN_REGION_MM2,
        }
    }
}

/// Compare one rendered page against another.
///
/// Each mask is taken against the *dilated* opposite: a glyph that shifted by a
/// fraction of a pixel between renders would otherwise leave a hairline outline
/// in the delta, and Onionskin would print a ghost of text already on the sheet.
#[allow(clippy::too_many_arguments)]
pub fn diff_page(
    old_gray: &[u8],
    old_size: (usize, usize),
    new_gray: &[u8],
    new_size: (usize, usize),
    size: PageSize,
    dpi: f64,
    index: usize,
    options: &DiffOptions,
) -> PageDiff {
    // Two renders of the same page should agree on their pixel size, but a
    // rounding difference of one pixel is not worth refusing over.
    let w = old_size.0.min(new_size.0);
    let h = old_size.1.min(new_size.1);

    let crop = |gray: &[u8], from: (usize, usize)| -> Vec<u8> {
        if from.0 == w && from.1 == h {
            return gray[..w * h].to_vec();
        }
        let mut out = Vec::with_capacity(w * h);
        for y in 0..h {
            out.extend_from_slice(&gray[y * from.0..y * from.0 + w]);
        }
        out
    };
    let old_gray = crop(old_gray, old_size);
    let new_gray = crop(new_gray, new_size);

    let old_ink = ink_mask(&old_gray, w, h, options.ink_threshold);
    let new_ink = ink_mask(&new_gray, w, h, options.ink_threshold);

    let radius = (options.tolerance_mm * dpi / MM_PER_INCH).round() as usize;
    let old_grown = dilate(&old_ink, radius);
    let new_grown = dilate(&new_ink, radius);

    let mut added = Mask::blank(w, h);
    let mut removed = Mask::blank(w, h);
    for i in 0..w * h {
        added.bits[i] = new_ink.bits[i] && !old_grown.bits[i];
        removed.bits[i] = old_ink.bits[i] && !new_grown.bits[i];
    }

    PageDiff {
        index,
        size,
        dpi,
        added_px: added.count(),
        removed_px: removed.count(),
        added_regions: label_regions(&added, dpi, options.group_mm, options.min_region_mm2),
        removed_regions: label_regions(&removed, dpi, options.group_mm, options.min_region_mm2),
        added,
        removed,
    }
}

#[cfg(test)]
mod tests;
