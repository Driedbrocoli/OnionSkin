use image::{DynamicImage, RgbImage};
use onionskin::geometry::PageSize;
use onionskin::scan::*;

fn synthetic(page: PageSize, dpi: f64, margin: u32, skew: f64, lines: usize) -> (DynamicImage, u32, u32) {
    let ppm = dpi / 25.4;
    let sw = (page.width_mm * ppm) as u32;
    let sh = (page.height_mm * ppm) as u32;
    let (w, h) = (sw + margin * 2, sh + margin * 2);
    let mut img = RgbImage::from_pixel(w, h, image::Rgb([40, 40, 40]));
    let c = (w as f64 / 2.0, h as f64 / 2.0);
    let (s, co) = skew.to_radians().sin_cos();
    for y in 0..h { for x in 0..w {
        let (dx, dy) = (x as f64 - c.0, y as f64 - c.1);
        let sx = co * dx + s * dy + c.0 - margin as f64;
        let sy = -s * dx + co * dy + c.1 - margin as f64;
        if sx < 0.0 || sy < 0.0 || sx >= sw as f64 || sy >= sh as f64 { continue; }
        let mut v = 245u8;
        if lines > 0 {
            let band = sh as f64 / (lines as f64 * 3.0);
            if (sy / band) as usize % 3 == 1 && sx > sw as f64 * 0.1 && sx < sw as f64 * 0.8 { v = 25; }
        }
        img.put_pixel(x, y, image::Rgb([v, v, v]));
    }}
    (DynamicImage::ImageRgb8(img), sw, sh)
}

fn main() {
    let a4 = PageSize::new(210.0, 297.0);
    let (dpi, margin, skew) = (200.0, 35u32, 1.5f64);
    let (scan, sw, sh) = synthetic(a4, dpi, margin, skew, 24);
    let gray = scan.to_luma8();
    let b = find_sheet(&gray).unwrap();
    let reg = register(&scan, ScanOptions::new(a4)).unwrap();

    let ppm = dpi / 25.4;
    let (s, co) = skew.to_radians().sin_cos();
    let exp_w = sw as f64 * co + sh as f64 * s;
    let exp_h = sw as f64 * s + sh as f64 * co;
    println!("truth: sheet {sw}x{sh} px, px_per_mm {ppm:.4}, skew {skew}");
    println!("expected bbox {exp_w:.1}x{exp_h:.1}");
    println!("found bbox ({},{})-({},{}) = {}x{}", b.x0,b.y0,b.x1,b.y1, b.width(), b.height());
    println!("registration: px_per_mm {:.4} skew {:+.3} origin ({:.1},{:.1})",
             reg.px_per_mm, reg.skew_deg, reg.origin_px.0, reg.origin_px.1);

    // Where the sheet's true top-left corner actually is in the image
    let c = ((sw + margin*2) as f64/2.0, (sh + margin*2) as f64/2.0);
    let (ux, uy) = (margin as f64 - c.0, margin as f64 - c.1);
    let true_origin = (co*ux - s*uy + c.0, s*ux + co*uy + c.1);
    println!("true sheet TL corner in image: ({:.1},{:.1})", true_origin.0, true_origin.1);
}
