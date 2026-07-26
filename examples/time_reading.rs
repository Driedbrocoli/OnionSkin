//! How long a page takes, stage by stage.
//!
//! Answers the only question anybody actually asks about reading a scan: I have
//! a page, how long until I can edit it? Times each stage separately, because
//! "it takes four seconds" is not useful when three of them are one step that
//! could be skipped.
//!
//!     cargo run --release --example time_reading -- [words] [dpi] [font.ttf]

use std::time::Instant;

use onionskin::document::{Document, Item};
use onionskin::font::EmbeddedFont;
use onionskin::geometry::PageSize;
use onionskin::letters::{alphabet_of, read_with_font, ReadOptions};
use onionskin::scan::{register, ScanOptions};

const A4: PageSize = PageSize {
    width_mm: 210.0,
    height_mm: 297.0,
};

/// Ordinary prose, so the letter mix is what a real page has.
const PROSE: &str = "The board met on Tuesday to review the quarter and agreed \
    that the northern depot should stay open until the spring at least. Costs \
    there have fallen since the roof was repaired and the two vans were \
    replaced, and the drivers report far fewer delays on the coast road. A \
    decision on the leasehold will wait until the surveyor has been and the \
    figures for March are in. Nobody expects the answer to change but the \
    paperwork must be right. Please initial the second page and return it to \
    the office before the end of the month so that we can file it in time.";

fn main() {
    let mut args = std::env::args().skip(1);
    let words: usize = args.next().and_then(|a| a.parse().ok()).unwrap_or(100);
    let dpi: f64 = args.next().and_then(|a| a.parse().ok()).unwrap_or(300.0);
    let path = args.next().unwrap_or_else(|| {
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf".to_string()
    });

    let font = EmbeddedFont::load(std::path::Path::new(&path)).expect("that is not a font");
    let text: Vec<&str> = PROSE.split_whitespace().take(words).collect();

    // Lay it out as lines that fit the page, at 11 pt.
    let size_pt = 11.0;
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in &text {
        let candidate = if current.is_empty() {
            word.to_string()
        } else {
            format!("{current} {word}")
        };
        if font.width_mm(&candidate, size_pt).unwrap_or(0.0) > 170.0 {
            lines.push(std::mem::take(&mut current));
            current = word.to_string();
        } else {
            current = candidate;
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }

    let letters: usize = text.iter().map(|w| w.chars().count()).sum();
    println!(
        "{} words, {letters} letters, {} lines, {size_pt} pt, {dpi} dpi",
        text.len(),
        lines.len()
    );

    // --- The scan itself -----------------------------------------------
    let start = Instant::now();
    let mut sheet = Sheet::new(A4, dpi);
    for (index, line) in lines.iter().enumerate() {
        sheet.text(&font, line, 20.0, 30.0 + index as f64 * 6.0, size_pt);
    }
    let image = sheet.image();
    let drawn = start.elapsed();
    println!(
        "\n  {:>7.0} ms  drawing the test page ({}×{} px) — not part of the work",
        drawn.as_secs_f64() * 1000.0,
        image.width(),
        image.height()
    );

    // --- Registration ---------------------------------------------------
    let start = Instant::now();
    let registration = register(
        &image::DynamicImage::ImageLuma8(image.clone()),
        ScanOptions::new(A4),
    )
    .expect("the sheet registers");
    let registered = start.elapsed();

    // --- Building the alphabet -------------------------------------------
    let start = Instant::now();
    let alphabet = alphabet_of(&font);
    let alphabet_built = start.elapsed();

    // --- Reading ----------------------------------------------------------
    let start = Instant::now();
    let page = read_with_font(
        &image,
        &registration,
        &ReadOptions::default(),
        &font,
        Some(&alphabet),
    )
    .expect("the page reads");
    let read = start.elapsed();

    // --- Turning it into something editable --------------------------------
    let start = Instant::now();
    let mut document = Document::blank(A4, 1);
    for line in &page.lines {
        let said = line.text_lossy();
        if said.trim().is_empty() {
            continue;
        }
        let left = line.rect.x_mm;
        let _ = document.add(Item {
            id: 0,
            page: 1,
            x_mm: left,
            y_mm: line.baseline_mm,
            text: said,
            size_pt: 11.0,
            font: "Helvetica".into(),
            width_mm: None,
            rotation_deg: 0.0,
            colour: "#000000".into(),
            leading: 1.2,
        });
    }
    let built = start.elapsed();

    let total = registered + alphabet_built + read + built;
    println!("  {:>7.0} ms  finding the sheet and its skew", registered.as_secs_f64() * 1000.0);
    println!(
        "  {:>7.0} ms  building the alphabet ({} characters the font can draw)",
        alphabet_built.as_secs_f64() * 1000.0,
        alphabet.chars().count()
    );
    println!("  {:>7.0} ms  reading every letter", read.as_secs_f64() * 1000.0);
    println!("  {:>7.0} ms  turning it into an editable document", built.as_secs_f64() * 1000.0);
    println!("  {:>7.0} ms  TOTAL", total.as_secs_f64() * 1000.0);

    println!(
        "\n  read {} letters in {} words on {} lines",
        page.letter_count(),
        page.word_count(),
        page.lines.len()
    );
    println!("  {} items in the editable document", document.items.len());
    if let Some(line) = page.lines.first() {
        println!("\n  first line: {}", line.text_lossy());
    }
}

// ---------------------------------------------------------------------------
// A sheet of paper with words on it
// ---------------------------------------------------------------------------

struct Sheet {
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
        out
    }
}
