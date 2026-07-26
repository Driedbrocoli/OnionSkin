//! Feed the registrar images no sane scanner would produce, and check it
//! explains itself rather than panicking.
use image::{DynamicImage, GrayImage, Luma, Rgb, RgbImage, Rgba, RgbaImage};
use onionskin::geometry::PageSize;
use onionskin::scan::{register, ScanOptions};

fn main() {
    let a4 = PageSize::new(210.0, 297.0);
    let mut cases: Vec<(&str, DynamicImage)> = Vec::new();

    cases.push(("1x1 white", DynamicImage::ImageRgb8(RgbImage::from_pixel(1,1,Rgb([255,255,255])))));
    cases.push(("1x1 black", DynamicImage::ImageRgb8(RgbImage::from_pixel(1,1,Rgb([0,0,0])))));
    cases.push(("1x4000 sliver", DynamicImage::ImageRgb8(RgbImage::from_pixel(1,4000,Rgb([250,250,250])))));
    cases.push(("4000x1 sliver", DynamicImage::ImageRgb8(RgbImage::from_pixel(4000,1,Rgb([250,250,250])))));
    cases.push(("all white 800x1000", DynamicImage::ImageRgb8(RgbImage::from_pixel(800,1000,Rgb([255,255,255])))));
    cases.push(("all black 800x1000", DynamicImage::ImageRgb8(RgbImage::from_pixel(800,1000,Rgb([0,0,0])))));
    cases.push(("uniform grey", DynamicImage::ImageRgb8(RgbImage::from_pixel(800,1000,Rgb([128,128,128])))));
    cases.push(("luma8", DynamicImage::ImageLuma8(GrayImage::from_pixel(800,1000,Luma([250])))));
    cases.push(("rgba transparent", DynamicImage::ImageRgba8(RgbaImage::from_pixel(800,1000,Rgba([250,250,250,0])))));

    // 16-bit and a noisy scan
    let mut noisy = RgbImage::new(900, 1200);
    let mut seed = 12345u32;
    for p in noisy.pixels_mut() {
        seed = seed.wrapping_mul(1103515245).wrapping_add(12345);
        let v = (seed >> 16) as u8;
        *p = Rgb([v, v, v]);
    }
    cases.push(("pure noise", DynamicImage::ImageRgb8(noisy)));

    // A thin bright cross on dark: bright but not a sheet
    let mut cross = RgbImage::from_pixel(900, 1200, Rgb([20, 20, 20]));
    for x in 0..900 { for t in 0..4 { cross.put_pixel(x, 600 + t, Rgb([255,255,255])); } }
    for y in 0..1200 { for t in 0..4 { cross.put_pixel(450 + t, y, Rgb([255,255,255])); } }
    cases.push(("thin cross", DynamicImage::ImageRgb8(cross)));

    // Two sheets side by side
    let mut two = RgbImage::from_pixel(1600, 1200, Rgb([30, 30, 30]));
    for y in 100..1100 { for x in 100..700 { two.put_pixel(x, y, Rgb([245,245,245])); }
                         for x in 900..1500 { two.put_pixel(x, y, Rgb([245,245,245])); } }
    cases.push(("two sheets", DynamicImage::ImageRgb8(two)));

    let mut panics = 0;
    for (label, img) in cases {
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            register(&img, ScanOptions::new(a4))
        }));
        match outcome {
            Err(_) => { println!("  {label:20} *** PANIC ***"); panics += 1; }
            Ok(Ok(r)) => println!("  {label:20} ok   {:.0} dpi skew {:+.2}", r.dpi(), r.skew_deg),
            Ok(Err(e)) => println!("  {label:20} err  {}", e.to_string().lines().next().unwrap_or("")),
        }
    }
    println!("\npanics: {panics}");
}
