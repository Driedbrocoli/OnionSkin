//! Tests for reading letters off a scan.
//!
//! The scans are synthetic, and deliberately so: a photograph of a real page
//! proves one page works. These are built from real glyph outlines at a known
//! size at a known millimetre, then laid on scanner backing and turned, so
//! every test knows the truth it is checking against down to the tenth of a
//! millimetre — and the same page can be re-made at four resolutions and five
//! angles without anyone holding a ruler.

use super::*;
use crate::geometry::PageSize;
use crate::scan::{register, ScanOptions};
use image::{GrayImage, Luma};
use std::path::PathBuf;

const A4: PageSize = PageSize {
    width_mm: 210.0,
    height_mm: 297.0,
};

fn dejavu() -> Option<EmbeddedFont> {
    let path = PathBuf::from("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf");
    path.is_file().then(|| EmbeddedFont::load(&path).unwrap())
}

// ---------------------------------------------------------------------------
// Building a page, then a scan of it
// ---------------------------------------------------------------------------

/// A sheet of paper with ink coverage per pixel, before it reaches a scanner.
struct Sheet {
    page: PageSize,
    px_per_mm: f64,
    width: usize,
    height: usize,
    /// How much of each pixel is ink, 0 to 1.
    coverage: Vec<f32>,
}

impl Sheet {
    fn new(page: PageSize, dpi: f64) -> Sheet {
        let px_per_mm = dpi / 25.4;
        let width = (page.width_mm * px_per_mm) as usize;
        let height = (page.height_mm * px_per_mm) as usize;
        Sheet {
            page,
            px_per_mm,
            width,
            height,
            coverage: vec![0.0; width * height],
        }
    }

    /// Set text with its baseline at `(x_mm, y_mm)`. Returns where it ended.
    fn text(&mut self, font: &EmbeddedFont, text: &str, x_mm: f64, y_mm: f64, size_pt: f64) -> f64 {
        // Font units to millimetres, at this type size.
        let mm_per_unit = size_pt * 25.4 / 72.0 / font.units_per_em();
        let mut pen = x_mm;

        for ch in text.chars() {
            let advance = font
                .width_mm(&ch.to_string(), size_pt)
                .expect("the test font has this character");
            if let Some(contours) = font.outline(ch) {
                let placed: Vec<Vec<(f64, f64)>> = contours
                    .iter()
                    .map(|contour| {
                        contour
                            .iter()
                            // Outlines run y-up from the baseline; a page runs
                            // y-down from its top corner.
                            .map(|&(ux, uy)| (pen + ux * mm_per_unit, y_mm - uy * mm_per_unit))
                            .collect()
                    })
                    .collect();
                self.fill(&placed);
            }
            pen += advance;
        }
        pen
    }

    /// A solid rectangle: a rule, a box, a blot.
    fn box_mm(&mut self, x_mm: f64, y_mm: f64, w_mm: f64, h_mm: f64) {
        self.fill(&[vec![
            (x_mm, y_mm),
            (x_mm + w_mm, y_mm),
            (x_mm + w_mm, y_mm + h_mm),
            (x_mm, y_mm + h_mm),
        ]]);
    }

    /// Fill closed polygons given in page millimetres, even-odd, anti-aliased.
    fn fill(&mut self, polygons: &[Vec<(f64, f64)>]) {
        const SUB: usize = 4;
        let weight = 1.0 / (SUB * SUB) as f32;

        let (mut top, mut bottom) = (f64::INFINITY, f64::NEG_INFINITY);
        for polygon in polygons {
            for &(_, y) in polygon {
                top = top.min(y);
                bottom = bottom.max(y);
            }
        }
        if !top.is_finite() {
            return;
        }
        let first = ((top * self.px_per_mm).floor().max(0.0)) as usize;
        let last = ((bottom * self.px_per_mm).ceil() as usize).min(self.height.saturating_sub(1));

        for row in first..=last.max(first) {
            if row >= self.height {
                break;
            }
            for sub in 0..SUB {
                let y = (row as f64 + (sub as f64 + 0.5) / SUB as f64) / self.px_per_mm;
                let mut crossings: Vec<f64> = Vec::new();
                for polygon in polygons {
                    for index in 0..polygon.len() {
                        let (ax, ay) = polygon[index];
                        let (bx, by) = polygon[(index + 1) % polygon.len()];
                        if (ay > y) == (by > y) {
                            continue;
                        }
                        let t = (y - ay) / (by - ay);
                        crossings.push(ax + t * (bx - ax));
                    }
                }
                if crossings.len() < 2 {
                    continue;
                }
                crossings.sort_by(|a, b| a.partial_cmp(b).unwrap());

                for span in crossings.chunks_exact(2) {
                    let left = span[0] * self.px_per_mm;
                    let right = span[1] * self.px_per_mm;
                    let from = left.floor().max(0.0) as usize;
                    let to = (right.ceil() as usize).min(self.width);
                    for column in from..to {
                        // Horizontal coverage of this pixel by this span.
                        let overlap = (right.min(column as f64 + 1.0) - left.max(column as f64))
                            .clamp(0.0, 1.0);
                        let cell = &mut self.coverage[row * self.width + column];
                        *cell = (*cell + overlap as f32 * weight * SUB as f32).min(1.0);
                    }
                }
            }
        }
    }

    /// Lay the sheet on a flatbed and scan it, turned by `skew_deg`.
    fn scan(&self, margin_px: u32, skew_deg: f64) -> GrayImage {
        const BACKING: u8 = 40;
        const PAPER: u8 = 246;
        const INK: u8 = 22;

        let width = self.width as u32 + margin_px * 2;
        let height = self.height as u32 + margin_px * 2;
        let mut image = GrayImage::from_pixel(width, height, Luma([BACKING]));

        let centre = (width as f64 / 2.0, height as f64 / 2.0);
        let (sin_t, cos_t) = skew_deg.to_radians().sin_cos();

        for y in 0..height {
            for x in 0..width {
                let (dx, dy) = (x as f64 - centre.0, y as f64 - centre.1);
                // Turn back into the sheet's own frame.
                let sx = cos_t * dx + sin_t * dy + centre.0 - margin_px as f64;
                let sy = -sin_t * dx + cos_t * dy + centre.1 - margin_px as f64;
                if sx < 0.0 || sy < 0.0 || sx >= self.width as f64 || sy >= self.height as f64 {
                    continue;
                }
                let ink = self.sample(sx, sy);
                let value = PAPER as f64 - ink as f64 * (PAPER - INK) as f64;
                image.put_pixel(x, y, Luma([value.round() as u8]));
            }
        }
        image
    }

    /// How much backing must show for a sheet turned this far to stay inside
    /// the scan. Turning a rectangle grows its bounding box, and registration
    /// needs all four corners — too small a margin and the sheet is cropped,
    /// which is a scan Onionskin rightly refuses rather than a bug.
    fn margin_for(&self, skew_deg: f64) -> u32 {
        let sin = skew_deg.to_radians().abs().sin();
        let grown = (self.height as f64 * sin).max(self.width as f64 * sin) / 2.0;
        // Plus three millimetres of backing all round, as the advice asks for.
        (grown + self.px_per_mm * 3.0).ceil() as u32
    }

    /// Bilinear ink coverage, as a scanner's optics would blur it.
    fn sample(&self, x: f64, y: f64) -> f32 {
        let (x0, y0) = (x.floor() as usize, y.floor() as usize);
        let (fx, fy) = ((x - x0 as f64) as f32, (y - y0 as f64) as f32);
        let at = |cx: usize, cy: usize| -> f32 {
            if cx >= self.width || cy >= self.height {
                0.0
            } else {
                self.coverage[cy * self.width + cx]
            }
        };
        let top = at(x0, y0) * (1.0 - fx) + at(x0 + 1, y0) * fx;
        let bottom = at(x0, y0 + 1) * (1.0 - fx) + at(x0 + 1, y0 + 1) * fx;
        top * (1.0 - fy) + bottom * fy
    }
}

/// Scan a sheet, register it, and read it — the whole path in one call.
fn read_sheet(sheet: &Sheet, margin_px: u32, skew_deg: f64) -> PageText {
    let image = sheet.scan(margin_px, skew_deg);
    let registration = register(
        &image::DynamicImage::ImageLuma8(image.clone()),
        ScanOptions::new(sheet.page),
    )
    .expect("the synthetic scan registers");
    read(&image, &registration, &ReadOptions::default()).expect("the scan reads")
}

fn read_sheet_with(sheet: &Sheet, font: &EmbeddedFont, alphabet: Option<&str>) -> PageText {
    let image = sheet.scan(30, 0.0);
    let registration = register(
        &image::DynamicImage::ImageLuma8(image.clone()),
        ScanOptions::new(sheet.page),
    )
    .expect("the synthetic scan registers");
    read_with_font(
        &image,
        &registration,
        &ReadOptions::default(),
        font,
        alphabet,
    )
    .expect("the scan reads")
}

// ---------------------------------------------------------------------------
// Geometry
// ---------------------------------------------------------------------------

#[test]
fn a_union_holds_both_rectangles() {
    let a = Rect {
        x_mm: 10.0,
        y_mm: 20.0,
        width_mm: 5.0,
        height_mm: 4.0,
    };
    let b = Rect {
        x_mm: 12.0,
        y_mm: 18.0,
        width_mm: 6.0,
        height_mm: 3.0,
    };
    let union = a.union(&b);

    assert_eq!(union.x_mm, 10.0);
    assert_eq!(union.y_mm, 18.0);
    assert_eq!(union.right_mm(), 18.0);
    assert_eq!(union.bottom_mm(), 24.0);
}

#[test]
fn touching_rectangles_are_told_from_separate_ones() {
    let a = Rect {
        x_mm: 10.0,
        y_mm: 10.0,
        width_mm: 10.0,
        height_mm: 10.0,
    };
    let over = Rect {
        x_mm: 15.0,
        y_mm: 15.0,
        width_mm: 10.0,
        height_mm: 10.0,
    };
    let beside = Rect {
        x_mm: 25.0,
        y_mm: 10.0,
        width_mm: 10.0,
        height_mm: 10.0,
    };
    // Sharing an edge exactly is not an overlap: a word may start where the
    // one before it ends.
    let flush = Rect {
        x_mm: 20.0,
        y_mm: 10.0,
        width_mm: 10.0,
        height_mm: 10.0,
    };

    assert!(a.intersects(&over));
    assert!(!a.intersects(&beside));
    assert!(!a.intersects(&flush));
}

// ---------------------------------------------------------------------------
// Finding the marks
// ---------------------------------------------------------------------------

/// Build a tiny ink mask from a picture, `#` for ink.
fn mask(rows: &[&str]) -> (Vec<bool>, usize, usize) {
    let width = rows[0].len();
    let mut ink = Vec::new();
    for row in rows {
        assert_eq!(row.len(), width, "ragged test picture");
        ink.extend(row.chars().map(|c| c == '#'));
    }
    (ink, width, rows.len())
}

#[test]
fn separate_blobs_get_separate_labels() {
    let (ink, w, h) = mask(&["##..##", "##..##", "......", "##...."]);
    let (_, count) = label_components(&ink, w, h);
    assert_eq!(count, 3);
}

#[test]
fn a_diagonal_touch_is_one_blob() {
    // Eight-connected, because a stroke drawn at an angle is one stroke. Under
    // four-connectivity a scanned `\` comes apart into a stack of specks.
    let (ink, w, h) = mask(&["#...", ".#..", "..#.", "...#"]);
    let (_, count) = label_components(&ink, w, h);
    assert_eq!(count, 1);
}

#[test]
fn a_ring_is_one_blob_and_its_hole_is_not_ink() {
    let (ink, w, h) = mask(&["####", "#..#", "#..#", "####"]);
    let (labels, count) = label_components(&ink, w, h);
    assert_eq!(count, 1);
    assert_eq!(labels[w + 1], 0, "the counter of an 'o' is not ink");
}

#[test]
fn a_u_shape_joins_up_around_the_bottom() {
    // The case a single-pass labeller gets wrong: two arms that only meet
    // several rows later, so the labels have to be merged retrospectively.
    let (ink, w, h) = mask(&["#..#", "#..#", "#..#", "####"]);
    let (_, count) = label_components(&ink, w, h);
    assert_eq!(count, 1);
}

#[test]
fn an_empty_picture_has_no_blobs() {
    let (ink, w, h) = mask(&["...."]);
    let (_, count) = label_components(&ink, w, h);
    assert_eq!(count, 0);
}

#[test]
fn a_blank_sheet_has_no_letters() {
    let sheet = Sheet::new(A4, 300.0);
    let page = read_sheet(&sheet, 30, 0.0);

    assert_eq!(page.letter_count(), 0);
    assert!(page.lines.is_empty());
}

#[test]
fn every_letter_of_a_word_is_found() {
    let Some(font) = dejavu() else { return };
    let mut sheet = Sheet::new(A4, 300.0);
    sheet.text(&font, "Approved", 40.0, 100.0, 12.0);

    let page = read_sheet(&sheet, 30, 0.0);

    assert_eq!(page.lines.len(), 1, "{}", page.text_lossy());
    assert_eq!(page.word_count(), 1);
    assert_eq!(page.letter_count(), 8, "Approved has eight letters");
}

#[test]
fn a_letter_is_found_where_it_was_actually_printed() {
    let Some(font) = dejavu() else { return };
    let mut sheet = Sheet::new(A4, 300.0);
    let end = sheet.text(&font, "Approved", 40.0, 100.0, 12.0);

    let page = read_sheet(&sheet, 30, 0.0);
    let line = &page.lines[0];

    // The baseline is where the text was set, to within a scan pixel.
    assert!(
        (line.baseline_mm - 100.0).abs() < 0.3,
        "baseline read as {:.2} mm, set at 100",
        line.baseline_mm
    );
    // The `A` starts where the pen did.
    let first = page.letters().next().unwrap();
    assert!(
        (first.rect.x_mm - 40.0).abs() < 0.4,
        "first letter at {:.2} mm, set at 40",
        first.rect.x_mm
    );
    // And the last letter ends where the pen finished, give or take the
    // sidebearing the `d` leaves after its stem.
    assert!(
        (line.rect.right_mm() - end).abs() < 0.6,
        "line ends at {:.2} mm, pen finished at {end:.2}",
        line.rect.right_mm()
    );
}

#[test]
fn words_are_split_at_the_spaces() {
    let Some(font) = dejavu() else { return };
    let mut sheet = Sheet::new(A4, 300.0);
    sheet.text(&font, "Approved by J Bezzina", 30.0, 90.0, 11.0);

    let page = read_sheet(&sheet, 30, 0.0);

    assert_eq!(page.lines.len(), 1);
    assert_eq!(page.word_count(), 4, "read as {:?}", page.text_lossy());
    assert_eq!(page.letter_count(), 18);
}

#[test]
fn word_splitting_holds_across_type_sizes() {
    // A page carries a heading and a footnote, and a millimetre means
    // something different in each: the gap that is a space at 24 pt is wider
    // than a whole letter at 7 pt.
    let Some(font) = dejavu() else { return };
    for size in [7.0, 9.0, 12.0, 18.0, 24.0] {
        let mut sheet = Sheet::new(A4, 400.0);
        sheet.text(&font, "one two three", 30.0, 100.0, size);

        let page = read_sheet(&sheet, 30, 0.0);
        assert_eq!(
            page.word_count(),
            3,
            "at {size} pt, read as {:?}",
            page.text_lossy()
        );
        assert_eq!(page.letter_count(), 11, "at {size} pt");
    }
}

#[test]
fn two_lines_are_two_lines() {
    let Some(font) = dejavu() else { return };
    let mut sheet = Sheet::new(A4, 300.0);
    sheet.text(&font, "first line", 30.0, 90.0, 11.0);
    sheet.text(&font, "second line", 30.0, 96.0, 11.0);

    let page = read_sheet(&sheet, 30, 0.0);

    assert_eq!(page.lines.len(), 2, "read as {:?}", page.text_lossy());
    assert!(page.lines[0].baseline_mm < page.lines[1].baseline_mm);
    assert!(
        (page.lines[0].baseline_mm - 90.0).abs() < 0.4,
        "first baseline read as {:.3} mm, set at 90",
        page.lines[0].baseline_mm
    );
    assert!(
        (page.lines[1].baseline_mm - 96.0).abs() < 0.4,
        "second baseline read as {:.3} mm, set at 96",
        page.lines[1].baseline_mm
    );
}

#[test]
fn lines_come_back_in_reading_order() {
    let Some(font) = dejavu() else { return };
    let mut sheet = Sheet::new(A4, 300.0);
    // Written out of order on purpose.
    sheet.text(&font, "third", 30.0, 140.0, 11.0);
    sheet.text(&font, "first", 30.0, 60.0, 11.0);
    sheet.text(&font, "second", 30.0, 100.0, 11.0);

    let page = read_sheet(&sheet, 30, 0.0);

    assert_eq!(page.lines.len(), 3);
    let baselines: Vec<f64> = page.lines.iter().map(|l| l.baseline_mm).collect();
    assert!(baselines.windows(2).all(|w| w[0] < w[1]), "{baselines:?}");
}

#[test]
fn the_dot_stays_on_the_i() {
    // A tittle is separate ink. Left alone, "iii" reads as six letters — and
    // three of them floating a millimetre above the line.
    let Some(font) = dejavu() else { return };
    let mut sheet = Sheet::new(A4, 400.0);
    sheet.text(&font, "iii", 40.0, 100.0, 14.0);

    let page = read_sheet(&sheet, 30, 0.0);

    assert_eq!(page.letter_count(), 3, "read as {:?}", page.text_lossy());
    for letter in page.letters() {
        // Each one reaches from the dot down to the baseline.
        assert!(
            letter.rect.height_mm > 2.0,
            "an 'i' only {:.2} mm tall is a dot on its own",
            letter.rect.height_mm
        );
    }
}

#[test]
fn accented_letters_stay_single_letters() {
    let Some(font) = dejavu() else { return };
    let mut sheet = Sheet::new(A4, 400.0);
    sheet.text(&font, "café naïve Ärger", 30.0, 100.0, 14.0);

    let page = read_sheet(&sheet, 30, 0.0);

    assert_eq!(page.word_count(), 3, "read as {:?}", page.text_lossy());
    assert_eq!(page.letter_count(), 14, "read as {:?}", page.text_lossy());
}

#[test]
fn dust_on_the_glass_is_not_a_letter() {
    let Some(font) = dejavu() else { return };
    let mut sheet = Sheet::new(A4, 300.0);
    sheet.text(&font, "Approved", 40.0, 100.0, 12.0);
    // Specks a fifth of a millimetre across, scattered well away from the text.
    for (x, y) in [(80.0, 60.0), (120.0, 150.0), (60.0, 220.0), (150.0, 40.0)] {
        sheet.box_mm(x, y, 0.2, 0.2);
    }

    let page = read_sheet(&sheet, 30, 0.0);
    assert_eq!(page.letter_count(), 8, "read as {:?}", page.text_lossy());
}

#[test]
fn a_rule_is_not_a_letter() {
    let Some(font) = dejavu() else { return };
    let mut sheet = Sheet::new(A4, 300.0);
    sheet.text(&font, "Signed", 30.0, 100.0, 12.0);
    // The line you sign on: long, thin, and not a letter however much it
    // resembles an underscore.
    sheet.box_mm(30.0, 110.0, 120.0, 0.4);

    let page = read_sheet(&sheet, 30, 0.0);
    assert_eq!(page.letter_count(), 6, "read as {:?}", page.text_lossy());
}

#[test]
fn a_photograph_on_the_page_is_not_read_as_letters() {
    let Some(font) = dejavu() else { return };
    let mut sheet = Sheet::new(A4, 300.0);
    sheet.text(&font, "Figure", 30.0, 100.0, 12.0);
    sheet.box_mm(30.0, 120.0, 60.0, 40.0);

    let page = read_sheet(&sheet, 30, 0.0);
    assert_eq!(page.letter_count(), 6, "read as {:?}", page.text_lossy());
    assert!(page.discarded > 0, "the block should have been discarded");
}

#[test]
fn a_turned_sheet_reads_the_same_as_a_straight_one() {
    let Some(font) = dejavu() else { return };
    let mut sheet = Sheet::new(A4, 400.0);
    sheet.text(&font, "Approved 25 July", 40.0, 100.0, 12.0);

    for skew in [-2.0, -1.0, -0.4, 0.0, 0.4, 1.0, 2.0] {
        let page = read_sheet(&sheet, sheet.margin_for(skew), skew);

        assert_eq!(
            page.letter_count(),
            14,
            "at {skew}°, read as {:?}",
            page.text_lossy()
        );
        assert_eq!(page.word_count(), 3, "at {skew}°");
        assert_eq!(page.lines.len(), 1, "at {skew}°");
        // And the words are still where they were printed, not where the scan
        // happened to put them.
        assert!(
            (page.lines[0].baseline_mm - 100.0).abs() < 0.5,
            "at {skew}°, baseline read as {:.2} mm",
            page.lines[0].baseline_mm
        );
        assert!(
            (page.lines[0].rect.x_mm - 40.0).abs() < 0.6,
            "at {skew}°, line starts at {:.2} mm",
            page.lines[0].rect.x_mm
        );
    }
}

#[test]
fn the_same_page_reads_the_same_at_any_resolution() {
    let Some(font) = dejavu() else { return };
    for dpi in [200.0, 300.0, 400.0, 600.0] {
        let mut sheet = Sheet::new(A4, dpi);
        sheet.text(&font, "Approved by hand", 35.0, 120.0, 12.0);

        let page = read_sheet(&sheet, (dpi / 10.0) as u32, 0.0);
        assert_eq!(
            page.letter_count(),
            14,
            "at {dpi} dpi, read as {:?}",
            page.text_lossy()
        );
        assert_eq!(page.word_count(), 3, "at {dpi} dpi");
    }
}

#[test]
fn ink_area_tells_a_hollow_letter_from_a_solid_one() {
    let Some(font) = dejavu() else { return };
    let mut sheet = Sheet::new(A4, 600.0);
    sheet.text(&font, "o", 40.0, 100.0, 24.0);
    sheet.box_mm(60.0, 96.0, 4.0, 4.0);

    let page = read_sheet(&sheet, 40, 0.0);
    let marks: Vec<&Letter> = page.letters().collect();
    assert_eq!(marks.len(), 2);

    let fill = |l: &Letter| l.ink_mm2 / (l.rect.width_mm * l.rect.height_mm);
    // The block is solid; the `o` is a ring around a hole.
    assert!(fill(marks[1]) > 0.9, "block fill {:.2}", fill(marks[1]));
    assert!(fill(marks[0]) < 0.75, "'o' fill {:.2}", fill(marks[0]));
}

// ---------------------------------------------------------------------------
// Keeping clear of what is already there
// ---------------------------------------------------------------------------

#[test]
fn a_gap_is_known_to_be_a_gap() {
    let Some(font) = dejavu() else { return };
    let mut sheet = Sheet::new(A4, 300.0);
    sheet.text(&font, "Name:", 30.0, 100.0, 12.0);

    let page = read_sheet(&sheet, 30, 0.0);

    // Well clear of the label, in the space you would write the name.
    assert!(page.is_clear(&Rect {
        x_mm: 60.0,
        y_mm: 95.0,
        width_mm: 60.0,
        height_mm: 6.0
    }));
    // And straight over it, which is the mistake that ruins the sheet.
    assert!(!page.is_clear(&Rect {
        x_mm: 30.0,
        y_mm: 95.0,
        width_mm: 20.0,
        height_mm: 6.0
    }));
}

#[test]
fn every_letter_is_listed_as_occupied() {
    let Some(font) = dejavu() else { return };
    let mut sheet = Sheet::new(A4, 300.0);
    sheet.text(&font, "Total", 30.0, 100.0, 12.0);

    let page = read_sheet(&sheet, 30, 0.0);
    let occupied = page.occupied();

    assert_eq!(occupied.len(), page.letter_count());
    for rect in &occupied {
        assert!(!page.is_clear(rect), "a letter's own box is not clear");
    }
}

// ---------------------------------------------------------------------------
// Reading the letters against a font
// ---------------------------------------------------------------------------

#[test]
fn a_glyph_becomes_a_stamp_shaped_like_the_letter() {
    let Some(font) = dejavu() else { return };
    let l = Stamp::from_outline(&font.outline('l').unwrap()).unwrap();
    let o = Stamp::from_outline(&font.outline('o').unwrap()).unwrap();

    // An `l` is a solid bar: almost every cell of its own box is ink.
    assert!(l.ink() / (STAMP * STAMP) as f64 > 0.8, "{}", l.ink());
    // An `o` is a ring, so its middle is empty and its edges are not.
    assert!(o.get(STAMP / 2, STAMP / 2) < 0.2);
    assert!(o.get(STAMP / 2, 0) > 0.5);
    assert!(o.get(0, STAMP / 2) > 0.5);
}

#[test]
fn a_stamp_matches_itself_exactly_and_others_less() {
    let Some(font) = dejavu() else { return };
    let o = Stamp::from_outline(&font.outline('o').unwrap()).unwrap();
    let l = Stamp::from_outline(&font.outline('l').unwrap()).unwrap();
    let c = Stamp::from_outline(&font.outline('c').unwrap()).unwrap();

    assert!((o.similarity(&o) - 1.0).abs() < 1e-9);
    // `c` is an `o` with a gap, so it is closer to one than a bar is.
    assert!(o.similarity(&c) > o.similarity(&l));
}

#[test]
fn a_space_has_no_stamp_to_match() {
    let Some(font) = dejavu() else { return };
    // A space has a width and no outline; there is nothing to compare, and
    // pretending otherwise would match it against every gap on the page.
    assert!(font
        .outline(' ')
        .map(|contours| Stamp::from_outline(&contours).is_none())
        .unwrap_or(true));
}

#[test]
fn the_letters_are_read_back_when_the_font_is_given() {
    let Some(font) = dejavu() else { return };
    let mut sheet = Sheet::new(A4, 600.0);
    sheet.text(&font, "Approved", 40.0, 100.0, 14.0);

    let page = read_sheet_with(&sheet, &font, None);

    assert_eq!(page.letter_count(), 8);
    assert_eq!(
        page.text_lossy(),
        "Approved",
        "confidences {:?}",
        page.letters().map(|l| l.confidence).collect::<Vec<_>>()
    );
}

#[test]
fn a_whole_line_of_mixed_text_is_read() {
    let Some(font) = dejavu() else { return };
    let text = "Approved 25 July 2026";
    let mut sheet = Sheet::new(A4, 600.0);
    sheet.text(&font, text, 30.0, 100.0, 14.0);

    let page = read_sheet_with(&sheet, &font, None);

    assert_eq!(page.text_lossy(), text);
    assert_eq!(page.read_count(), page.letter_count());
}

#[test]
fn several_lines_are_read_in_order() {
    let Some(font) = dejavu() else { return };
    let mut sheet = Sheet::new(A4, 600.0);
    sheet.text(&font, "Name", 30.0, 90.0, 13.0);
    sheet.text(&font, "Date", 30.0, 105.0, 13.0);
    sheet.text(&font, "Total", 30.0, 120.0, 13.0);

    let page = read_sheet_with(&sheet, &font, None);
    assert_eq!(page.text_lossy(), "Name\nDate\nTotal");
}

#[test]
fn capitals_are_told_from_their_small_letters() {
    // The pairs that look identical apart from size, which is why recognition
    // is done per line with the line's own type size: `o` and `O`, `s` and
    // `S`, `c` and `C` are the same shape at different heights.
    let Some(font) = dejavu() else { return };
    let mut sheet = Sheet::new(A4, 600.0);
    sheet.text(&font, "Oo Ss Cc Ww Zz", 30.0, 100.0, 16.0);

    let page = read_sheet_with(&sheet, &font, None);
    assert_eq!(page.text_lossy(), "Oo Ss Cc Ww Zz");
}

#[test]
fn descenders_are_told_from_their_capitals() {
    // `p` and `P` differ only in where they sit against the baseline.
    let Some(font) = dejavu() else { return };
    let mut sheet = Sheet::new(A4, 600.0);
    sheet.text(&font, "Pp Yy", 30.0, 100.0, 16.0);

    let page = read_sheet_with(&sheet, &font, None);
    assert_eq!(page.text_lossy(), "Pp Yy");
}

#[test]
fn a_letter_the_alphabet_leaves_out_is_left_unread() {
    // The property that matters most: an answer is never invented. Told to
    // look only for digits, the reader must not decide that an `A` is an `8`.
    let Some(font) = dejavu() else { return };
    let mut sheet = Sheet::new(A4, 600.0);
    sheet.text(&font, "WMX", 40.0, 100.0, 16.0);

    let page = read_sheet_with(&sheet, &font, Some("0123456789"));

    assert_eq!(page.letter_count(), 3, "the marks are still found");
    assert_eq!(page.read_count(), 0, "read as {:?}", page.text_lossy());
    assert_eq!(page.text_lossy(), "???");
}

#[test]
fn a_blot_that_is_not_a_letter_is_not_read_as_one() {
    let Some(font) = dejavu() else { return };
    let mut sheet = Sheet::new(A4, 600.0);
    sheet.box_mm(40.0, 100.0, 3.0, 3.0);

    let page = read_sheet_with(&sheet, &font, None);

    assert_eq!(page.letter_count(), 1);
    let blot = page.letters().next().unwrap();
    // A solid square may honestly read as a block character — a page can carry
    // one. What it must never do is come back as a letter of the alphabet,
    // because then a word has been invented out of a smudge.
    assert!(
        blot.text.map(|c| !c.is_alphabetic()).unwrap_or(true),
        "a solid square was read as the letter {:?} at {:.2}",
        blot.text,
        blot.confidence
    );
}

#[test]
fn confidence_is_reported_for_every_letter_that_was_tried() {
    let Some(font) = dejavu() else { return };
    let mut sheet = Sheet::new(A4, 600.0);
    sheet.text(&font, "Total", 30.0, 100.0, 14.0);

    let page = read_sheet_with(&sheet, &font, None);
    for letter in page.letters() {
        assert!(
            letter.confidence > 0.0 && letter.confidence <= 1.0,
            "confidence {}",
            letter.confidence
        );
    }
}

#[test]
fn a_word_reads_as_text_only_when_all_of_it_was_read() {
    let Some(font) = dejavu() else { return };
    let mut sheet = Sheet::new(A4, 600.0);
    sheet.text(&font, "Paid", 30.0, 100.0, 14.0);

    let page = read_sheet_with(&sheet, &font, None);
    assert_eq!(page.lines[0].words[0].text().as_deref(), Some("Paid"));

    let unread = read_sheet_with(&sheet, &font, Some("xyz"));
    assert_eq!(unread.lines[0].words[0].text(), None);
}

#[test]
fn a_labelled_field_can_be_found_by_its_label() {
    // What the reading is actually for: find the label, and you know where to
    // put the words without anyone measuring the form.
    let Some(font) = dejavu() else { return };
    let mut sheet = Sheet::new(A4, 600.0);
    sheet.text(&font, "Name", 30.0, 90.0, 13.0);
    sheet.text(&font, "Date", 30.0, 110.0, 13.0);

    let page = read_sheet_with(&sheet, &font, None);

    let date = page.find_line("date").expect("the Date line was not found");
    assert!(
        (date.baseline_mm - 110.0).abs() < 0.4,
        "found at {:.2} mm",
        date.baseline_mm
    );
    // And the space beside it is free to write in.
    assert!(page.is_clear(&Rect {
        x_mm: 60.0,
        y_mm: date.baseline_mm - 4.0,
        width_mm: 50.0,
        height_mm: 5.0
    }));
    assert!(page.find_line("total").is_none());
}

#[test]
fn a_turned_sheet_is_still_read_correctly() {
    let Some(font) = dejavu() else { return };
    let mut sheet = Sheet::new(A4, 600.0);
    sheet.text(&font, "Approved", 40.0, 100.0, 15.0);

    for skew in [-1.5, -0.5, 0.5, 1.5] {
        let image = sheet.scan(sheet.margin_for(skew), skew);
        let registration = register(
            &image::DynamicImage::ImageLuma8(image.clone()),
            ScanOptions::new(A4),
        )
        .unwrap();
        let page =
            read_with_font(&image, &registration, &ReadOptions::default(), &font, None).unwrap();

        assert_eq!(page.text_lossy(), "Approved", "at {skew}°");
    }
}

#[test]
fn a_font_with_postscript_outlines_reads_too() {
    // The Word case: Calibri and Cambria are PostScript-flavoured, so a reader
    // that only handled TrueType would be useless on most real documents.
    let Some(path) = crate::font::tests::postscript_font() else {
        return;
    };
    let font = EmbeddedFont::load(&path).unwrap();
    if !"Approved".chars().all(|c| font.has(c)) {
        return; // A font without the Latin alphabet proves nothing here.
    }

    let mut sheet = Sheet::new(A4, 600.0);
    sheet.text(&font, "Approved", 40.0, 100.0, 15.0);

    let page = read_sheet_with(&sheet, &font, None);
    assert_eq!(page.letter_count(), 8);
    assert_eq!(page.text_lossy(), "Approved");
}

// ---------------------------------------------------------------------------
// Things that should not panic
// ---------------------------------------------------------------------------

#[test]
fn an_image_with_no_pixels_is_reported_not_panicked_on() {
    let image = GrayImage::new(0, 0);
    let registration = ScanRegistration {
        page: A4,
        px_per_mm: 11.8,
        skew_deg: 0.0,
        origin_px: (0.0, 0.0),
    };
    assert!(read(&image, &registration, &ReadOptions::default()).is_err());
}

#[test]
fn an_all_black_scan_does_not_hang_or_panic() {
    // Every pixel one component, which is the worst case for the labeller and
    // the case a stack-based flood fill overflows on.
    let image = GrayImage::from_pixel(600, 800, Luma([0]));
    let registration = ScanRegistration {
        page: A4,
        px_per_mm: 600.0 / 210.0,
        skew_deg: 0.0,
        origin_px: (0.0, 0.0),
    };
    let page = read(&image, &registration, &ReadOptions::default()).unwrap();
    assert_eq!(page.letter_count(), 0, "a black page has no letters");
}

#[test]
fn recognition_without_a_usable_alphabet_leaves_the_marks_alone() {
    let Some(font) = dejavu() else { return };
    let mut sheet = Sheet::new(A4, 300.0);
    sheet.text(&font, "Hi", 40.0, 100.0, 12.0);

    let page = read_sheet_with(&sheet, &font, Some(""));
    assert_eq!(page.letter_count(), 2);
    assert_eq!(page.read_count(), 0);
}

// ---------------------------------------------------------------------------
// Small ink: punctuation, accents, and dust
//
// All three are marks too small to be a letter, and for a long time all three
// were thrown away together by a single minimum height. What separates them is
// not their size but what is around them, and every test here is a case where
// judging by size alone gave the wrong answer.
// ---------------------------------------------------------------------------

#[test]
fn a_full_stop_survives_at_the_size_people_actually_print() {
    // At 10 pt a full stop is a third of a millimetre across — well under the
    // half-millimetre floor that keeps dust out. Applying that floor before the
    // lines were grouped silently deleted the punctuation from every page of
    // body text that was ever scanned.
    let Some(font) = dejavu() else { return };
    for size in [8.0, 10.0, 11.0, 12.0] {
        let mut sheet = Sheet::new(A4, 300.0);
        sheet.text(&font, "End of report.", 30.0, 100.0, size);
        let page = read_sheet_with(&sheet, &font, None);
        let said = page.text_lossy();
        assert!(
            said.contains('.'),
            "the full stop went missing at {size} pt: {said:?}"
        );
    }
}

#[test]
fn a_colon_is_one_character_and_not_two_dots() {
    // Neither dot of a colon is a base for the other, so the accent merge can
    // never join them. Left apart, the upper dot is close enough to the letter
    // before it to be taken for an accent on it, and "Date:" reads as "Daté".
    let Some(font) = dejavu() else { return };
    for size in [9.0, 11.0, 14.0] {
        let mut sheet = Sheet::new(A4, 300.0);
        sheet.text(&font, "Date: 4 June", 30.0, 100.0, size);
        let page = read_sheet_with(&sheet, &font, None);
        let said = page.text_lossy();
        assert!(said.contains(':'), "no colon at {size} pt: {said:?}");
        assert!(
            said.starts_with("Date"),
            "the colon's dot was eaten by the word at {size} pt: {said:?}"
        );
    }
}

#[test]
fn an_i_keeps_its_dot_at_body_text_size() {
    // The tittle of an `i` at 11 pt is under half a millimetre. Judged against
    // the minimum letter height before it could be merged, it was thrown away,
    // and the bare stem left behind matches a dotless `ı` better than an `i`.
    let Some(font) = dejavu() else { return };
    for size in [9.0, 11.0, 14.0] {
        let mut sheet = Sheet::new(A4, 300.0);
        sheet.text(&font, "the individual jumps", 30.0, 100.0, size);
        let page = read_sheet_with(&sheet, &font, None);
        let said = page.text_lossy();
        assert!(
            !said.contains('\u{131}') && !said.contains('\u{237}'),
            "a dotless letter came back at {size} pt: {said:?}"
        );
        assert!(said.contains('i'), "no `i` at all at {size} pt: {said:?}");
    }
}

#[test]
fn dust_on_the_glass_is_still_thrown_away() {
    // The other half of the bargain. Small ink is kept because it might be
    // punctuation, so something has to stop a speck on the scanner from being
    // reported as a letter — and what stops it is having no writing around it.
    let Some(font) = dejavu() else { return };
    let mut sheet = Sheet::new(A4, 300.0);
    sheet.text(&font, "Report", 30.0, 100.0, 12.0);
    // Three specks, well away from the writing.
    sheet.box_mm(120.0, 40.0, 0.25, 0.25);
    sheet.box_mm(60.0, 200.0, 0.3, 0.2);
    sheet.box_mm(150.0, 250.0, 0.2, 0.3);

    let page = read_sheet_with(&sheet, &font, None);
    assert_eq!(page.lines.len(), 1, "the dust made lines: {:?}", page.text_lossy());
    assert_eq!(
        page.letter_count(),
        6,
        "Report is six letters, and the dust is not letters: {:?}",
        page.text_lossy()
    );
}

#[test]
fn two_lines_of_writing_are_never_joined_into_one_character() {
    // The rule that assembles a colon looks for two small marks one above the
    // other. Two full stops at the end of consecutive lines are exactly that
    // shape, and joining them would invent a character and destroy two.
    let Some(font) = dejavu() else { return };
    let mut sheet = Sheet::new(A4, 300.0);
    // Single-spaced, which is the hard case: the closer the lines, the more
    // the two full stops look like one colon. 11 pt set on 4.6 mm is about
    // what a word processor calls single spacing.
    sheet.text(&font, "first line.", 30.0, 100.0, 11.0);
    sheet.text(&font, "second line.", 30.0, 104.6, 11.0);

    let page = read_sheet_with(&sheet, &font, None);
    assert_eq!(page.lines.len(), 2, "{:?}", page.text_lossy());
    let said = page.text_lossy();
    assert_eq!(said.matches('.').count(), 2, "{said:?}");
    assert!(!said.contains(':'), "two full stops became a colon: {said:?}");
}

// ---------------------------------------------------------------------------
// Telling apart letters that are drawn the same
// ---------------------------------------------------------------------------

#[test]
fn a_capital_i_and_a_lowercase_l_both_stay_in_the_alphabet() {
    // In most sans-serif faces these are one rectangle, and a pass that drops
    // characters drawn the same dropped one of them. Whichever went could then
    // never be read, on any page, however clear the scan — and the page gave no
    // sign of it. They differ in height, which is something that gets measured,
    // so the matcher can tell them apart and must be given the chance.
    let Some(font) = dejavu() else { return };
    let alphabet = alphabet_of(&font);
    let candidates = build_candidates(&font, &alphabet);
    for want in ['I', 'l', '1', '0', 'O', 'o'] {
        assert!(
            candidates.iter().any(|c| c.ch == want),
            "{want:?} is not in the alphabet at all"
        );
    }
}

#[test]
fn a_hyphen_is_not_a_macron() {
    // Every bar in Unicode is the same drawing once it is squared off to a
    // stamp: hyphen, macron, underscore, overline. Only where it sits against
    // the baseline separates them, and measuring that against the mark's own
    // height — a third of a millimetre — saturates and tells you nothing.
    let Some(font) = dejavu() else { return };
    for size in [10.0, 12.0, 16.0] {
        let mut sheet = Sheet::new(A4, 300.0);
        sheet.text(&font, "part-time work", 30.0, 100.0, size);
        let page = read_sheet_with(&sheet, &font, None);
        let said = page.text_lossy();
        assert!(
            said.contains('-') || said.contains('\u{2010}'),
            "no hyphen at {size} pt: {said:?}"
        );
        assert!(
            !said.contains('\u{00af}') && !said.contains('_') && !said.contains('\u{2550}'),
            "the hyphen came back as an overline or a rule at {size} pt: {said:?}"
        );
    }
}

#[test]
fn the_type_size_is_measured_and_not_assumed() {
    // A line of prose is mostly lowercase, so its tallest quarter are the
    // ascenders of `b d f h k l` — which stand taller than a capital. Reading
    // that height as a cap height makes every letter measure about a sixth
    // shorter than it is, and then an `l` is exactly as tall as a dotless `ı`.
    //
    // The `ı` is what this checks, and not the `I`. A capital `I` and a
    // lowercase `l` are the same rectangle in DejaVu and differ by three
    // hundredths of an em — one and a half pixels at 11 pt on a 300 dpi scan —
    // so which of the two comes back is not something the ink can settle and
    // not something to write a test about. A dotless `ı` is nearly half an em
    // shorter, and getting *that* wrong means the scale is wrong.
    let Some(font) = dejavu() else { return };
    let mut sheet = Sheet::new(A4, 300.0);
    sheet.text(&font, "all the little bells", 30.0, 100.0, 11.0);

    let page = read_sheet_with(&sheet, &font, None);
    let said = page.text_lossy();
    assert!(
        !said.contains('\u{131}'),
        "an `l` came back as a dotless `ı`: {said:?}"
    );
    let bars = said.matches('l').count() + said.matches('I').count();
    assert_eq!(bars, 6, "six tall bars, however they were named: {said:?}");
}

#[test]
fn a_page_of_ordinary_prose_is_read_almost_perfectly() {
    // The whole of it, end to end, at the sizes people actually print. This is
    // the test that would have caught every bug the ones above describe, and it
    // is here so that a change which trades one of them for another shows up.
    let Some(font) = dejavu() else { return };
    let lines = [
        "The quick brown fox jumps over the lazy dog.",
        "Invoice 2026-114: amount due 1,240.50 (net 30 days).",
        "Item 7b - lithium cell, 3.7V, qty 18 @ 4.95 = 89.10",
    ];
    for size in [10.0, 11.0, 12.0] {
        let mut sheet = Sheet::new(A4, 300.0);
        for (index, line) in lines.iter().enumerate() {
            sheet.text(&font, line, 15.0, 40.0 + index as f64 * size * 0.6, size);
        }
        let page = read_sheet_with(&sheet, &font, None);

        let wanted: String = lines.concat().chars().filter(|c| !c.is_whitespace()).collect();
        let got: String = page.text_lossy().chars().filter(|c| !c.is_whitespace()).collect();

        // Counted as a subsequence, so one dropped letter costs one and does
        // not shift everything after it into disagreement.
        let right = common_run(&wanted, &got);
        let share = right as f64 / wanted.chars().count() as f64;
        assert!(
            share >= 0.97,
            "only {:.1}% right at {size} pt\n want: {wanted}\n  got: {got}",
            share * 100.0
        );
    }
}

/// How many characters of `wanted` appear in `got`, in order.
fn common_run(wanted: &str, got: &str) -> usize {
    let a: Vec<char> = wanted.chars().collect();
    let b: Vec<char> = got.chars().collect();
    let mut row = vec![0usize; b.len() + 1];
    for &x in &a {
        let mut diagonal = 0;
        for j in 0..b.len() {
            let above = row[j + 1];
            row[j + 1] = if x == b[j] {
                diagonal + 1
            } else {
                row[j + 1].max(row[j])
            };
            diagonal = above;
        }
    }
    row[b.len()]
}
