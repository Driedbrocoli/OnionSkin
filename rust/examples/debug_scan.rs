use image::{DynamicImage, RgbImage};
use onionskin::geometry::PageSize;
use onionskin::scan::*;

fn synthetic(page: PageSize, dpi: f64, margin_px: u32, skew_deg: f64, lines: usize) -> DynamicImage {
    let px_per_mm = dpi / 25.4;
    let sheet_w = (page.width_mm * px_per_mm) as u32;
    let sheet_h = (page.height_mm * px_per_mm) as u32;
    let width = sheet_w + margin_px * 2;
    let height = sheet_h + margin_px * 2;
    let mut img = RgbImage::from_pixel(width, height, image::Rgb([40, 40, 40]));
    let centre = (width as f64 / 2.0, height as f64 / 2.0);
    let theta = skew_deg.to_radians();
    let (sin_t, cos_t) = theta.sin_cos();
    for y in 0..height {
        for x in 0..width {
            let (dx, dy) = (x as f64 - centre.0, y as f64 - centre.1);
            let sx = cos_t * dx + sin_t * dy + centre.0 - margin_px as f64;
            let sy = -sin_t * dx + cos_t * dy + centre.1 - margin_px as f64;
            if sx < 0.0 || sy < 0.0 || sx >= sheet_w as f64 || sy >= sheet_h as f64 { continue; }
            let mut value = 245u8;
            if lines > 0 {
                let band = sheet_h as f64 / (lines as f64 * 3.0);
                let row = (sy / band) as usize;
                if row % 3 == 1 && sx > sheet_w as f64 * 0.1 && sx < sheet_w as f64 * 0.8 { value = 25; }
            }
            img.put_pixel(x, y, image::Rgb([value, value, value]));
        }
    }
    DynamicImage::ImageRgb8(img)
}

fn main() {
    let a4 = PageSize::new(210.0, 297.0);
    for (label, skew, lines) in [("text -2", -2.0, 24usize), ("text +1.5", 1.5, 24), ("blank", 2.0, 0)] {
        let scan = synthetic(a4, 150.0, 40, skew, lines);
        let gray = scan.to_luma8();
        let t = otsu_threshold(&gray);
        let b = find_sheet(&gray);
        let mut ink = 0u64;
        for y in b.y0..b.y1 { for x in b.x0..b.x1 {
            if gray.get_pixel(x, y).0[0] <= t { ink += 1; } } }
        let found = estimate_skew(&gray, b, 5.0);
        println!("{label:10} truth={skew:+5.1} otsu={t:3} bounds=({},{})-({},{}) ink_px={ink:8} found={found:+.3}",
                 b.x0, b.y0, b.x1, b.y1);
    }
}
