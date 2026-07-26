use image::{DynamicImage, RgbImage};
use onionskin::geometry::PageSize;
use onionskin::scan::*;

fn synthetic(page: PageSize, dpi: f64, margin: u32, skew: f64, lines: usize) -> DynamicImage {
    let ppm = dpi / 25.4;
    let (sw, sh) = ((page.width_mm*ppm) as u32, (page.height_mm*ppm) as u32);
    let (w, h) = (sw + margin*2, sh + margin*2);
    let mut img = RgbImage::from_pixel(w, h, image::Rgb([38,40,44]));
    let c = (w as f64/2.0, h as f64/2.0);
    let (s, co) = skew.to_radians().sin_cos();
    for y in 0..h { for x in 0..w {
        let (dx, dy) = (x as f64 - c.0, y as f64 - c.1);
        let sx = co*dx + s*dy + c.0 - margin as f64;
        let sy = -s*dx + co*dy + c.1 - margin as f64;
        if sx < 0.0 || sy < 0.0 || sx >= sw as f64 || sy >= sh as f64 { continue; }
        let mut v = 245u8;
        if lines > 0 {
            let band = sh as f64/(lines as f64*3.0);
            if (sy/band) as usize % 3 == 1 && sx > sw as f64*0.1 && sx < sw as f64*0.8 { v = 25; }
        }
        img.put_pixel(x, y, image::Rgb([v,v,v]));
    }}
    DynamicImage::ImageRgb8(img)
}

fn main() {
    let a4 = PageSize::new(210.0, 297.0);
    for truth in [-4.0f64, -2.0, 0.0, 2.0, 3.7] {
        let scan = synthetic(a4, 200.0, 40, truth, 26);
        let gray = scan.to_luma8();
        let b = find_sheet(&gray).unwrap();
        let got = estimate_skew(&gray, b, 5.0);
        println!("truth {truth:+5.1} bounds {}x{} -> found {got:+.3}", b.width(), b.height());
    }
    // Does raising the search range help? (tests whether we're clipping at the edge)
    let scan = synthetic(a4, 200.0, 40, -4.0, 26);
    let gray = scan.to_luma8();
    let b = find_sheet(&gray).unwrap();
    for max in [3.0f64, 5.0, 8.0, 12.0] {
        println!("  max_skew {max:4.1} -> {:+.3}", estimate_skew(&gray, b, max));
    }
}
