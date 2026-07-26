//! Render a PDF page to a PNG, the way a scanner would see it on paper.
//!
//! Used for measuring: the preview images `delta` writes are deliberately
//! recoloured — old ink grey, new ink red — which is the wrong input for
//! judging how well letters are read or how long reading them takes.
//!
//!     cargo run --release --example render_page -- page.pdf out.png 300

use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("usage: render_page <pdf> <png> [dpi]");
        std::process::exit(2);
    }
    let dpi: f64 = args.get(3).and_then(|d| d.parse().ok()).unwrap_or(300.0);

    let guard = onionskin::render::engine().expect("no PDF renderer");
    let document = guard.open(Path::new(&args[1])).expect("could not open");
    let page = document.render(0, dpi).expect("could not render");

    let image = image::RgbImage::from_raw(page.width as u32, page.height as u32, page.rgb)
        .expect("the pixels do not match the size");
    image
        .save(Path::new(&args[2]))
        .expect("could not write the image");
    println!(
        "{} — {}×{} px at {dpi} dpi",
        args[2], page.width, page.height
    );
}
