use super::*;

/// A run of dark modules is one rectangle, not several.
///
/// A laser printer laying down two abutting rectangles leaves a hairline of
/// paper between them often enough that a scanner sees an extra bar where the
/// barcode says there is one wide one.
#[test]
fn touching_modules_come_out_as_one_rectangle() {
    let symbol = Symbol {
        dark: vec![true, true, true, false, true],
        width: 5,
        height: 1,
        quiet: 0,
        text: "x".into(),
    };
    let rects = symbol.rectangles(1.0);
    assert_eq!(rects.len(), 2, "{rects:?}");
    assert_eq!(rects[0], (0.0, 0.0, 3.0, 1.0));
    assert_eq!(rects[1], (4.0, 0.0, 1.0, 1.0));
}

/// The quiet zone is left, and it is left on every side.
#[test]
fn the_quiet_zone_is_paper_the_symbol_does_not_touch() {
    let symbol = Symbol {
        dark: vec![true],
        width: 1,
        height: 1,
        quiet: 10,
        text: "x".into(),
    };
    let rects = symbol.rectangles(0.5);
    assert_eq!(rects[0].0, 5.0, "nothing to the left of it");
    assert_eq!(rects[0].1, 5.0, "nothing above it");
    assert_eq!(symbol.width_mm(0.5), 10.5);
    assert_eq!(symbol.height_mm(0.5), 10.5);
}

/// A linear barcode is one row and is not printed one module tall.
#[test]
fn bars_are_given_their_height_rather_than_stacked() {
    let symbol = code128::encode("A").unwrap();
    assert_eq!(symbol.height, 1);
    let bars = symbol.bars(0.33, 15.0);
    assert!(bars.iter().all(|bar| bar.3 == 15.0), "a bar came out short");
    assert!(bars.len() > 3, "only {} bars", bars.len());
}

/// Under about a quarter of a millimetre a laser printer stops putting a module
/// down the same width twice, which is exactly what a scanner cannot survive.
#[test]
fn a_module_too_small_to_print_is_known_to_be() {
    assert!(too_small_to_print(0.1));
    assert!(!too_small_to_print(0.33));
    assert!(!too_small_to_print(SMALLEST_MODULE_MM));
}

// ---------------------------------------------------------------------------
// The round trip: does a real decoder read what we wrote?
// ---------------------------------------------------------------------------

/// Draw a symbol onto a page, print the page, and read the ink back as a PNG.
///
/// The whole way through, deliberately. A symbol that is right in memory and
/// wrong on paper is the failure that matters, and everything between the two —
/// the rectangles, the PDF, the renderer — is where it would happen.
fn printed_and_scanned(
    symbol: &Symbol,
    module_mm: f64,
    height_mm: f64,
) -> (tempfile::TempDir, std::path::PathBuf) {
    let page = crate::geometry::PageSize::new(
        symbol.width_mm(module_mm) + 20.0,
        (if symbol.height == 1 {
            height_mm + symbol.quiet as f64 * module_mm * 2.0
        } else {
            symbol.height_mm(module_mm)
        }) + 20.0,
    );
    let shapes: Vec<Vec<crate::pdf::PlacedShape>> = vec![symbol
        .bars(module_mm, height_mm)
        .into_iter()
        .map(
            |(x_mm, y_mm, width_mm, height_mm)| crate::pdf::PlacedShape {
                drawing: crate::pdf::Drawing::Rect {
                    x_mm: x_mm + 10.0,
                    y_mm: y_mm + 10.0,
                    width_mm,
                    height_mm,
                    radius_mm: 0.0,
                },
                stroke: None,
                fill: Some((0.0, 0.0, 0.0)),
                width_mm: 0.0,
                dash_mm: None,
            },
        )
        .collect()];

    let dir = tempfile::tempdir().expect("somewhere to work");
    let pdf = dir.path().join("symbol.pdf");
    crate::pdf::write_page_content(&pdf, &[page], &[Vec::new()], &shapes, "Onionskin", None)
        .expect("the page should be written");

    // 300 dpi, which is what a desktop printer does and enough that a module is
    // several pixels across.
    let engine = crate::render::engine().expect("a renderer");
    let document = engine.open(&pdf).expect("the page should open");
    let drawn = document.render_gray(0, 300.0).expect("it should draw");

    let png = dir.path().join("symbol.png");
    let image =
        image::GrayImage::from_raw(drawn.width as u32, drawn.height as u32, drawn.gray.clone())
            .expect("the render should be an image");
    image.save(&png).expect("the image should save");
    // The directory is handed back rather than dropped here, because dropping
    // it would delete the PNG before anybody could read it — and rather than
    // leaked, because the sweep below makes a hundred and sixty of these and a
    // test run should not leave a hundred and sixty folders behind.
    (dir, png)
}

/// What an independent decoder makes of a file, or nothing if there is no
/// decoder on this machine.
///
/// zbar is not a dependency and is not going to become one — it is a C library,
/// and the whole point of writing these by hand was to need nothing. But when it
/// happens to be installed it is the only opinion worth having, because it is
/// the only one that did not come from this program.
fn what_a_scanner_reads(png: &std::path::Path) -> Option<String> {
    let output = std::process::Command::new("zbarimg")
        .args(["--nodbus", "--quiet", "--raw"])
        .arg(png)
        .output()
        .ok()?;
    let read = String::from_utf8_lossy(&output.stdout)
        .trim_end()
        .to_string();
    (!read.is_empty()).then_some(read)
}

/// Whether there is a decoder here at all, so a machine without one says so
/// rather than passing quietly.
fn a_decoder_is_installed() -> bool {
    std::process::Command::new("zbarimg")
        .arg("--version")
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

/// A real scanner reads back exactly what went in.
///
/// This is the only test here that is not this program marking its own work.
/// Everything else checks that the bits are what this program thinks they should
/// be; this checks that somebody else agrees.
#[test]
fn a_scanner_reads_back_what_went_into_the_barcode() {
    if !a_decoder_is_installed() {
        eprintln!(
            "skipped: no zbarimg on this machine, so nothing independent could \
             check the barcodes. `apt install zbar-tools` and run it again."
        );
        return;
    }
    for text in [
        "ONIONSKIN",
        "INV-2024-00817",
        "123456789012",
        "Hello, World!",
        "A1",
        "*",
        // Every character Code 128's B set covers, which is where a wrong row
        // in the pattern table would show.
        r##"!"#$%&'()*+,-./0123456789:;<=>?@"##,
        "ABCDEFGHIJKLMNOPQRSTUVWXYZ[\\]^_`",
        "abcdefghijklmnopqrstuvwxyz{|}~",
    ] {
        let symbol = code128::encode(text).expect("it should encode");
        let (_kept, png) = printed_and_scanned(&symbol, 0.4, 15.0);
        assert_eq!(
            what_a_scanner_reads(&png).as_deref(),
            Some(text),
            "a scanner did not read back '{text}'"
        );
    }
}

/// The same for QR codes, across every way of packing and several sizes.
#[test]
fn a_scanner_reads_back_what_went_into_the_qr_code() {
    if !a_decoder_is_installed() {
        eprintln!("skipped: no zbarimg on this machine.");
        return;
    }
    let long = "X".repeat(300);
    let digits = "9".repeat(120);
    for text in [
        // Digits, packed three to ten bits.
        "12345678901234567890",
        digits.as_str(),
        // Capitals and marks, packed two to eleven.
        "ONIONSKIN-2024",
        "HELLO",
        // Anything at all, a byte each — including letters outside ASCII, which
        // is the case a program written in English is most likely to get wrong.
        "https://example.org/forms/2024/renewal",
        "Onionskin — café, naïve, Ærø",
        "日本語のテキスト",
        long.as_str(),
    ] {
        for level in [
            qr::Ecc::Low,
            qr::Ecc::Medium,
            qr::Ecc::Quartile,
            qr::Ecc::High,
        ] {
            let symbol = qr::encode(text, level).expect("it should encode");
            let (_kept, png) = printed_and_scanned(&symbol, 0.6, 0.0);
            assert_eq!(
                what_a_scanner_reads(&png).as_deref(),
                Some(text),
                "a scanner did not read back '{text}' at {level:?}"
            );
        }
    }
}

/// Every version, at every level of correction, read back by a real decoder.
///
/// The two tables in `qr` are three hundred and twenty numbers copied in by
/// hand from the standard. A wrong one does not make a code that looks wrong —
/// it makes one of exactly the right shape that no scanner can read, and the
/// only way to find out is to ask a scanner. So each of the hundred and sixty
/// combinations is filled to the brim and read back.
///
/// Filled to the brim on purpose: the longest text a version holds is the one
/// that lands on that version and no smaller one, so this really does visit
/// every row of both tables rather than the first few.
#[test]
fn every_version_and_every_level_reads_back() {
    if !a_decoder_is_installed() {
        eprintln!("skipped: no zbarimg on this machine.");
        return;
    }
    for version in 1..=40usize {
        for level in [
            qr::Ecc::Low,
            qr::Ecc::Medium,
            qr::Ecc::Quartile,
            qr::Ecc::High,
        ] {
            // A text with some shape to it rather than one character repeated:
            // a wrong block split can still come back right when every byte is
            // the same.
            let text: String = (0..qr::longest_at(version, level))
                .map(|at| (b'!' + (at % 90) as u8) as char)
                .collect();
            let symbol = qr::encode(&text, level).expect("it should encode");
            assert_eq!(
                symbol.width,
                4 * version + 17,
                "the longest text for version {version} at {level:?} landed on a \
                 {}-module square",
                symbol.width
            );
            let (_kept, png) = printed_and_scanned(&symbol, 0.5, 0.0);
            assert_eq!(
                what_a_scanner_reads(&png).as_deref(),
                Some(text.as_str()),
                "a scanner could not read a full version {version} at {level:?}"
            );
        }
    }
}
