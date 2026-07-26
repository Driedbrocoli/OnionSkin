//! Measure how much of a page is read correctly, at a range of type sizes.
//!
//! Eyeballing a line tells you something is wrong but never how wrong, and it
//! cannot tell you whether a change helped everywhere or helped one line and
//! hurt three. This types known text at known sizes, reads it back, and counts.
//!
//!     cargo run --release --example measure_reading -- [font.ttf]

use onionskin::font::EmbeddedFont;
use onionskin::geometry::PageSize;
use onionskin::letters::{read_with_font, ReadOptions};
use onionskin::scan::{register, ScanOptions};

const A4: PageSize = PageSize {
    width_mm: 210.0,
    height_mm: 297.0,
};

/// What the pages say. Ordinary prose, digits, and the punctuation that turns
/// up in real documents — not a pangram chosen to be easy.
const LINES: &[&str] = &[
    "The quick brown fox jumps over the lazy dog.",
    "Invoice 2026-114: amount due 1,240.50 (net 30 days).",
    "Dear Ms Iverson, please initial page 3 where indicated.",
    "Item 7b - lithium cell, 3.7V, qty 18 @ 4.95 = 89.10",
    "ACCEPTED. Signed for and on behalf of Willowbrook Ltd.",
];

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| {
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf".to_string()
    });
    let font = EmbeddedFont::load(std::path::Path::new(&path)).expect("that is not a font");

    println!("{path}");
    println!("{:>6}  {:>8}  {:>8}  {:>7}", "size", "letters", "correct", "");
    let mut worst = 100.0f64;

    for size_pt in [8.0, 9.0, 10.0, 11.0, 12.0, 14.0, 18.0] {
        let dpi = 300.0;
        let mut sheet = Sheet::new(A4, dpi);
        let mut wanted = String::new();
        for (index, line) in LINES.iter().enumerate() {
            let y = 30.0 + index as f64 * (size_pt * 0.6);
            sheet.text(&font, line, 15.0, y, size_pt);
            wanted.push_str(line);
        }

        let image = sheet.image();
        let registration =
            register(&image::DynamicImage::ImageLuma8(image.clone()), ScanOptions::new(A4))
                .expect("the sheet registers");
        let page = read_with_font(&image, &registration, &ReadOptions::default(), &font, None)
            .expect("the sheet reads");

        let got: String = page
            .lines
            .iter()
            .flat_map(|l| l.words.iter())
            .flat_map(|w| w.letters.iter())
            .filter_map(|l| l.text)
            .collect();
        let expected: String = wanted.chars().filter(|c| !c.is_whitespace()).collect();

        let correct = agreement(&expected, &got);
        let percent = 100.0 * correct as f64 / expected.chars().count().max(1) as f64;
        worst = worst.min(percent);
        println!(
            "{size_pt:>5} pt  {:>8}  {:>8}  {percent:>6.1}%",
            expected.chars().count(),
            correct
        );
        if percent < 100.0 {
            let missed = differences(&expected, &got);
            if !missed.is_empty() {
                println!("          misread: {missed}");
            }
            if std::env::var("SHOW").is_ok() {
                println!("          want: {expected}");
                println!("          got:  {got}");
            }
        }
    }
    println!("\nworst line: {worst:.1}%");
}

/// How many characters line up, in order. A letter dropped or invented shifts
/// everything after it, so compare as sequences rather than position by
/// position — otherwise one missing comma reads as a whole line misread.
fn agreement(expected: &str, got: &str) -> usize {
    let a: Vec<char> = expected.chars().collect();
    let b: Vec<char> = got.chars().collect();
    // Longest common subsequence, which is the count of characters that are
    // right and in the right order.
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

/// Which characters were expected but not delivered, with how often.
fn differences(expected: &str, got: &str) -> String {
    let mut tally: std::collections::BTreeMap<char, i32> = std::collections::BTreeMap::new();
    for c in expected.chars() {
        *tally.entry(c).or_default() += 1;
    }
    for c in got.chars() {
        *tally.entry(c).or_default() -= 1;
    }
    let mut parts: Vec<String> = Vec::new();
    for (ch, count) in tally {
        if count > 0 {
            parts.push(format!("{ch:?}×{count}"));
        }
    }
    parts.join(" ")
}

// ---------------------------------------------------------------------------
// A sheet of paper with words on it
// ---------------------------------------------------------------------------

struct Sheet {
    page: PageSize,
    dpi: f64,
    width: usize,
    height: usize,
    ink: Vec<f32>,
}

impl Sheet {
    fn new(page: PageSize, dpi: f64) -> Sheet {
        let width = (page.width_mm / 25.4 * dpi).round() as usize;
        let height = (page.height_mm / 25.4 * dpi).round() as usize;
        Sheet {
            page,
            dpi,
            width,
            height,
            ink: vec![0.0; width * height],
        }
    }

    fn text(&mut self, font: &EmbeddedFont, text: &str, x_mm: f64, baseline_mm: f64, size_pt: f64) {
        let em_mm = size_pt * 25.4 / 72.0;
        let upem = font.units_per_em();
        let widths: Vec<f64> = font
            .shape(text)
            .map(|glyphs| glyphs.iter().map(|g| g.advance_1000 / 1000.0).collect())
            .unwrap_or_else(|_| text.chars().map(|_| 0.5).collect());

        let mut pen = x_mm;
        for (index, ch) in text.chars().enumerate() {
            let advance = widths.get(index).copied().unwrap_or(0.5) * em_mm;
            if let Some(contours) = font.outline(ch) {
                let placed: Vec<Vec<(f64, f64)>> = contours
                    .iter()
                    .map(|c| {
                        c.iter()
                            .map(|&(gx, gy)| {
                                (pen + gx / upem * em_mm, baseline_mm - gy / upem * em_mm)
                            })
                            .collect()
                    })
                    .collect();
                self.fill(&placed);
            }
            pen += advance;
        }
    }

    /// Scanline fill with three-by-three sampling, which is roughly what a
    /// printer and a scanner between them do to an edge.
    fn fill(&mut self, polygons: &[Vec<(f64, f64)>]) {
        const SUB: usize = 3;
        let per_mm = self.dpi / 25.4;
        let weight = 1.0 / (SUB * SUB) as f32;
        let (mut y0, mut y1) = (f64::INFINITY, f64::NEG_INFINITY);
        for polygon in polygons {
            for &(_, y) in polygon {
                y0 = y0.min(y);
                y1 = y1.max(y);
            }
        }
        if !y0.is_finite() {
            return;
        }
        let first = ((y0 * per_mm).floor().max(0.0)) as usize;
        let last = ((y1 * per_mm).ceil().min(self.height as f64 - 1.0)) as usize;

        for py in first..=last.min(self.height - 1) {
            for sy in 0..SUB {
                let y = (py as f64 + (sy as f64 + 0.5) / SUB as f64) / per_mm;
                let mut crossings: Vec<f64> = Vec::new();
                for polygon in polygons {
                    for i in 0..polygon.len() {
                        let (ax, ay) = polygon[i];
                        let (bx, by) = polygon[(i + 1) % polygon.len()];
                        if (ay > y) == (by > y) {
                            continue;
                        }
                        let t = (y - ay) / (by - ay);
                        crossings.push(ax + t * (bx - ax));
                    }
                }
                crossings.sort_by(|a, b| a.partial_cmp(b).unwrap());
                for span in crossings.chunks_exact(2) {
                    let (left, right) = (span[0] * per_mm, span[1] * per_mm);
                    let from = left.floor().max(0.0) as usize;
                    let to = (right.ceil() as usize).min(self.width - 1);
                    for px in from..=to {
                        for sx in 0..SUB {
                            let x = px as f64 + (sx as f64 + 0.5) / SUB as f64;
                            if x >= left && x < right {
                                self.ink[py * self.width + px] += weight;
                            }
                        }
                    }
                }
            }
        }
    }

    fn image(&self) -> image::GrayImage {
        let mut out = image::GrayImage::new(self.width as u32, self.height as u32);
        for y in 0..self.height {
            for x in 0..self.width {
                let ink = self.ink[y * self.width + x].clamp(0.0, 1.0);
                let value = 245.0 * (1.0 - ink) + 25.0 * ink;
                out.put_pixel(x as u32, y as u32, image::Luma([value.round() as u8]));
            }
        }
        let _ = self.page;
        out
    }
}
