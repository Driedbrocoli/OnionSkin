//! Can this machine reach pdfium, and where?
use pdfium_render::prelude::*;

fn main() {
    let candidates = [
        "/usr/local/lib/python3.11/dist-packages/pypdfium2_raw/libpdfium.so",
        "./libpdfium.so",
    ];
    for path in candidates {
        match Pdfium::bind_to_library(path) {
            Ok(bindings) => {
                let pdfium = Pdfium::new(bindings);
                println!("bound to {path}");
                let doc = pdfium.load_pdf_from_file(std::path::Path::new("/tmp/claude-0/-home-user-OnionSkin/6dd5f051-5f78-5766-9dc0-2899aec2179e/scratchpad/demo/order.pdf"), None).unwrap();
                for (i, page) in doc.pages().iter().enumerate() {
                    println!(
                        "  page {i}: {} x {} pt",
                        page.width().value,
                        page.height().value
                    );
                }
                return;
            }
            Err(e) => println!("no: {path}: {e}"),
        }
    }
    match Pdfium::bind_to_system_library() {
        Ok(_) => println!("bound to system library"),
        Err(e) => println!("no system library: {e}"),
    }
}
