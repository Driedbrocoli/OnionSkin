//! `onionskin barcode`, driven the way somebody types it.
//!
//! The encoders have their own tests, and those go as far as handing the
//! finished PDF to an independent decoder. What is left is everything between
//! the command line and the encoder: where the code lands, whether it fits the
//! paper, which page it goes on, and whether the numbers printed back are the
//! numbers somebody would use to decide where to put it.
//!
//! So these run the binary and then read the delta back — decoding it too, when
//! there is a decoder on the machine.

use std::path::{Path, PathBuf};
use std::process::Command;

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_onionskin"))
}

fn at(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn run(home: &Path, args: &[&str]) -> (bool, String) {
    let output = Command::new(binary())
        .args(args)
        .env("ONIONSKIN_HOME", home)
        .output()
        .expect("the binary should run");
    (
        output.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        ),
    )
}

struct Work {
    dir: tempfile::TempDir,
    home: PathBuf,
}

impl Work {
    fn new() -> Work {
        let dir = tempfile::tempdir().expect("a place to work");
        let home = dir.path().join("home");
        std::fs::create_dir_all(&home).expect("a home of its own");
        Work { dir, home }
    }

    fn at(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    /// A printed sheet with something on it, to put a code on.
    fn a_sheet(&self, pages: usize) -> PathBuf {
        let document = self.at("sheet.osk");
        let printed = self.at("sheet.pdf");
        for args in [
            vec![
                "new".to_string(),
                at(&document),
                "--page".into(),
                "a4".into(),
                "--pages".into(),
                pages.to_string(),
            ],
            vec![
                "write".to_string(),
                at(&document),
                "--at".into(),
                "20,25:Asset register".into(),
            ],
            vec![
                "print".to_string(),
                at(&document),
                "-o".into(),
                at(&printed),
            ],
        ] {
            let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
            let (ok, said) = run(&self.home, &borrowed);
            assert!(ok, "{said}");
        }
        printed
    }
}

/// Everything dark on a page of a delta, in millimetres from the top-left.
struct Ink {
    spots: Vec<(f64, f64)>,
    png: PathBuf,
}

impl Ink {
    fn drawn(delta: &Path, page: usize, into: &Path) -> Ink {
        const DPI: f64 = 300.0;
        let engine = onionskin::render::engine().expect("a renderer");
        let document = engine.open(delta).expect("the delta should open");
        let drawn = document.render_gray(page - 1, DPI).expect("it should draw");
        let mm = |pixels: usize| (pixels as f64 + 0.5) * 25.4 / DPI;
        let mut spots = Vec::new();
        for y in 0..drawn.height {
            for x in 0..drawn.width {
                if drawn.gray[y * drawn.width + x] < 128 {
                    spots.push((mm(x), mm(y)));
                }
            }
        }
        let png = into.join(format!("page-{page}.png"));
        image::GrayImage::from_raw(drawn.width as u32, drawn.height as u32, drawn.gray.clone())
            .expect("the render should be an image")
            .save(&png)
            .expect("the image should save");
        Ink { spots, png }
    }

    fn anything(&self) -> bool {
        !self.spots.is_empty()
    }

    /// The box the ink occupies: left, top, right, bottom.
    fn box_mm(&self) -> (f64, f64, f64, f64) {
        let xs: Vec<f64> = self.spots.iter().map(|s| s.0).collect();
        let ys: Vec<f64> = self.spots.iter().map(|s| s.1).collect();
        (
            xs.iter().cloned().fold(f64::MAX, f64::min),
            ys.iter().cloned().fold(f64::MAX, f64::min),
            xs.iter().cloned().fold(f64::MIN, f64::max),
            ys.iter().cloned().fold(f64::MIN, f64::max),
        )
    }

    /// What an independent decoder makes of it, when there is one installed.
    fn what_a_scanner_reads(&self) -> Option<String> {
        let output = Command::new("zbarimg")
            .args(["--nodbus", "--quiet", "--raw"])
            .arg(&self.png)
            .output()
            .ok()?;
        let read = String::from_utf8_lossy(&output.stdout)
            .trim_end()
            .to_string();
        (!read.is_empty()).then_some(read)
    }
}

/// A barcode lands where it was asked to, and reads back.
#[test]
fn a_barcode_lands_where_it_was_put_and_reads_back() {
    let work = Work::new();
    let sheet = work.a_sheet(1);
    let delta = work.at("code.pdf");

    let (ok, said) = run(
        &work.home,
        &[
            "barcode",
            &at(&sheet),
            "--text",
            "INV-2024-00817",
            "--at",
            "20,60",
            "-o",
            &at(&delta),
        ],
    );
    assert!(ok, "{said}");
    // It says how big it came out and where, which is what somebody checks
    // against the sheet in their hand.
    assert!(said.contains("Code 128"), "{said}");
    assert!(
        said.contains("20,60"),
        "it did not say where it went: {said}"
    );
    assert!(
        said.contains("blank paper"),
        "it did not say a barcode needs blank paper: {said}"
    );

    let ink = Ink::drawn(&delta, 1, work.dir.path());
    assert!(ink.anything(), "the delta came out blank");
    let (left, top, _, bottom) = ink.box_mm();

    // The quiet zone is paper, so the first bar is inside where it was asked
    // for rather than exactly on it — but it must not be outside.
    assert!(
        (20.0..30.0).contains(&left),
        "the bars start at {left:.1} mm, not just inside 20"
    );
    assert!(
        (60.0..70.0).contains(&top),
        "the bars start at {top:.1} mm down, not just inside 60"
    );
    // 15 mm of bars, plus the quiet zone above and below.
    assert!(
        bottom - top > 14.0 && bottom - top < 17.0,
        "the bars are {:.1} mm tall, not the 15 asked for",
        bottom - top
    );

    if let Some(read) = ink.what_a_scanner_reads() {
        assert_eq!(read, "INV-2024-00817", "a scanner read it back wrongly");
    }
}

/// A QR code comes out square, and holds a web address.
#[test]
fn a_qr_code_comes_out_square_and_reads_back() {
    let work = Work::new();
    let sheet = work.a_sheet(1);
    let delta = work.at("qr.pdf");

    let (ok, said) = run(
        &work.home,
        &[
            "barcode",
            &at(&sheet),
            "--text",
            "https://example.org/renew/2024",
            "--at",
            "120,60",
            "--qr",
            "-o",
            &at(&delta),
        ],
    );
    assert!(ok, "{said}");
    assert!(said.contains("QR code"), "{said}");
    assert!(said.contains("modules square"), "{said}");

    let ink = Ink::drawn(&delta, 1, work.dir.path());
    assert!(ink.anything());
    let (left, top, right, bottom) = ink.box_mm();
    assert!(
        ((right - left) - (bottom - top)).abs() < 1.0,
        "a QR code came out {:.1} by {:.1} mm",
        right - left,
        bottom - top
    );
    assert!(
        left >= 120.0,
        "it started left of where it was put: {left:.1}"
    );
    assert!(top >= 60.0, "it started above where it was put: {top:.1}");

    if let Some(read) = ink.what_a_scanner_reads() {
        assert_eq!(read, "https://example.org/renew/2024");
    }
}

/// A code too big for the paper is refused with the numbers, before anything is
/// written — rather than written half off the page and found at the printer.
#[test]
fn a_code_that_runs_off_the_paper_is_refused_with_the_numbers() {
    let work = Work::new();
    let sheet = work.a_sheet(1);
    let delta = work.at("nope.pdf");

    let (ok, said) = run(
        &work.home,
        &[
            "barcode",
            &at(&sheet),
            "--text",
            "A LONG ENOUGH REFERENCE THAT IT WILL NOT FIT",
            "--at",
            "20,60",
            "--module",
            "1.5",
            "-o",
            &at(&delta),
        ],
    );
    assert!(!ok, "it should have refused: {said}");
    assert!(said.contains("runs off"), "{said}");
    assert!(
        said.contains("--module"),
        "it did not say what to do: {said}"
    );
    assert!(!delta.exists(), "it wrote a delta anyway");
}

/// A barcode cannot hold an accented letter, and says which one and where
/// rather than dropping it.
#[test]
fn a_letter_a_barcode_cannot_hold_is_named() {
    let work = Work::new();
    let sheet = work.a_sheet(1);

    let (ok, said) = run(
        &work.home,
        &[
            "barcode",
            &at(&sheet),
            "--text",
            "café",
            "--at",
            "20,60",
            "-o",
            &at(&work.at("no.pdf")),
        ],
    );
    assert!(!ok, "{said}");
    assert!(said.contains('é'), "it did not name the letter: {said}");
    assert!(
        said.contains("QR code"),
        "it did not offer the way out: {said}"
    );

    // And a QR code takes it.
    let delta = work.at("yes.pdf");
    let (ok, said) = run(
        &work.home,
        &[
            "barcode",
            &at(&sheet),
            "--text",
            "café",
            "--at",
            "20,60",
            "--qr",
            "-o",
            &at(&delta),
        ],
    );
    assert!(ok, "{said}");
    let ink = Ink::drawn(&delta, 1, work.dir.path());
    if let Some(read) = ink.what_a_scanner_reads() {
        assert_eq!(read, "café");
    }
}

/// The delta has as many pages as the sheet, and the code is on the one asked
/// for — because a three-page sheet fed a one-page delta prints page one's
/// barcode onto page two.
#[test]
fn the_code_goes_on_the_page_it_was_asked_for() {
    let work = Work::new();
    let sheet = work.a_sheet(3);
    let delta = work.at("page2.pdf");

    let (ok, said) = run(
        &work.home,
        &[
            "barcode",
            &at(&sheet),
            "--text",
            "SECOND",
            "--at",
            "20,60",
            "--page",
            "2",
            "-o",
            &at(&delta),
        ],
    );
    assert!(ok, "{said}");
    assert!(said.contains("page 2"), "{said}");

    assert!(
        !Ink::drawn(&delta, 1, work.dir.path()).anything(),
        "page one was marked"
    );
    assert!(
        Ink::drawn(&delta, 2, work.dir.path()).anything(),
        "page two was not"
    );
    assert!(
        !Ink::drawn(&delta, 3, work.dir.path()).anything(),
        "page three was marked"
    );

    // And a page that is not there is refused rather than silently made.
    let (ok, said) = run(
        &work.home,
        &[
            "barcode",
            &at(&sheet),
            "--text",
            "FOURTH",
            "--at",
            "20,60",
            "--page",
            "4",
            "-o",
            &at(&work.at("page4.pdf")),
        ],
    );
    assert!(!ok, "{said}");
    assert!(said.contains("3 page(s)"), "{said}");
}

/// --caption prints what was encoded underneath, so a person can type it in
/// when the scanner will not read it.
#[test]
fn the_caption_goes_under_the_code() {
    let work = Work::new();
    let sheet = work.a_sheet(1);

    let bare = work.at("bare.pdf");
    let (ok, said) = run(
        &work.home,
        &[
            "barcode",
            &at(&sheet),
            "--text",
            "INV-1",
            "--at",
            "20,60",
            "-o",
            &at(&bare),
        ],
    );
    assert!(ok, "{said}");
    let without = Ink::drawn(&bare, 1, work.dir.path()).box_mm();

    let captioned = work.at("captioned.pdf");
    let (ok, said) = run(
        &work.home,
        &[
            "barcode",
            &at(&sheet),
            "--text",
            "INV-1",
            "--at",
            "20,60",
            "--caption",
            "-o",
            &at(&captioned),
        ],
    );
    assert!(ok, "{said}");
    let with = Ink::drawn(&captioned, 1, work.dir.path()).box_mm();

    assert!(
        with.3 > without.3 + 1.0,
        "the caption added nothing below the bars: {:.1} against {:.1}",
        with.3,
        without.3
    );
}

/// A rehearsal says what it would do and leaves nothing behind.
#[test]
fn a_rehearsal_writes_no_code() {
    let work = Work::new();
    let sheet = work.a_sheet(1);
    let delta = work.at("nothing.pdf");

    let (ok, said) = run(
        &work.home,
        &[
            "barcode",
            &at(&sheet),
            "--text",
            "INV-1",
            "--at",
            "20,60",
            "-o",
            &at(&delta),
            "--dry-run",
        ],
    );
    assert!(ok, "{said}");
    assert!(said.contains("Code 128"), "{said}");
    assert!(!delta.exists(), "a rehearsal left a file behind");
}
