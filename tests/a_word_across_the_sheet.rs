//! The watermark, measured on the rendered page rather than in the arithmetic.
//!
//! `watermark`'s unit tests check where the placement says the word will go.
//! That is worth checking and it is not the same question as where the ink
//! lands: the arithmetic can agree with itself perfectly and still put the word
//! off the edge of the paper, because the arithmetic is what would be wrong.
//!
//! So these tests run the command somebody types, draw the delta it wrote, and
//! measure the ink. Onionskin carries its own renderer, so this needs nothing
//! installed.
//!
//! The one that matters most is the grey. A watermark on an already-printed
//! sheet is toner going *over* the printing, and the whole design rests on it
//! being light enough to read through. If it ever comes out black the feature
//! is worse than useless — it destroys the page it was asked to mark — so the
//! darkness of every pixel is checked, not the average.

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

/// A printed sheet to mark, of the size asked for.
fn a_printed_sheet(home: &Path, dir: &Path, page: &str, pages: usize) -> PathBuf {
    let document = dir.join(format!("sheet-{page}-{pages}.osk"));
    let printed = document.with_extension("pdf");
    let (ok, said) = run(
        home,
        &[
            "new",
            &at(&document),
            "--page",
            page,
            "--pages",
            &pages.to_string(),
        ],
    );
    assert!(ok, "{said}");
    let (ok, said) = run(
        home,
        &["write", &at(&document), "--at", "20,30:Quarterly report"],
    );
    assert!(ok, "{said}");
    let (ok, said) = run(home, &["print", &at(&document), "-o", &at(&printed)]);
    assert!(ok, "{said}");
    printed
}

/// Every spot of ink on a page of the delta, in millimetres from the top-left,
/// with how dark it is.
struct Ink {
    spots: Vec<(f64, f64, u8)>,
    width_mm: f64,
    height_mm: f64,
}

impl Ink {
    fn drawn(delta: &Path, page: usize) -> Ink {
        const DPI: f64 = 100.0;
        let engine = onionskin::render::engine().expect("a renderer");
        let doc = engine.open(delta).expect("the delta should open");
        let drawn = doc.render_gray(page - 1, DPI).expect("it should draw");
        let mm = |pixels: usize| (pixels as f64 + 0.5) * 25.4 / DPI;
        let mut spots = Vec::new();
        for y in 0..drawn.height {
            for x in 0..drawn.width {
                let level = drawn.gray[y * drawn.width + x];
                if level < 250 {
                    spots.push((mm(x), mm(y), level));
                }
            }
        }
        Ink {
            spots,
            width_mm: drawn.width as f64 * 25.4 / DPI,
            height_mm: drawn.height as f64 * 25.4 / DPI,
        }
    }

    fn anything(&self) -> bool {
        !self.spots.is_empty()
    }

    /// The middle of the box the ink occupies.
    fn middle(&self) -> (f64, f64) {
        let xs: Vec<f64> = self.spots.iter().map(|s| s.0).collect();
        let ys: Vec<f64> = self.spots.iter().map(|s| s.1).collect();
        let span = |v: &[f64]| {
            (
                v.iter().cloned().fold(f64::MAX, f64::min),
                v.iter().cloned().fold(f64::MIN, f64::max),
            )
        };
        let (x0, x1) = span(&xs);
        let (y0, y1) = span(&ys);
        ((x0 + x1) / 2.0, (y0 + y1) / 2.0)
    }

    /// The box the ink occupies: left, top, right, bottom.
    fn box_mm(&self) -> (f64, f64, f64, f64) {
        let span = |v: Vec<f64>| {
            (
                v.iter().cloned().fold(f64::MAX, f64::min),
                v.iter().cloned().fold(f64::MIN, f64::max),
            )
        };
        let (left, right) = span(self.spots.iter().map(|s| s.0).collect());
        let (top, bottom) = span(self.spots.iter().map(|s| s.1).collect());
        (left, top, right, bottom)
    }

    fn darkest(&self) -> u8 {
        self.spots.iter().map(|s| s.2).min().unwrap_or(255)
    }
}

/// Somewhere to work, and a home of its own so a test never touches the
/// profiles or the history of whoever is running it.
///
/// A fresh directory every time rather than one named after the test: two
/// `cargo test` runs at once — which is one keystroke away, and happens — would
/// otherwise write over each other's sheets and fail for a reason that has
/// nothing to do with the code.
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
}

/// The whole of it, on the page: a grey word, corner to corner, in the middle,
/// and nowhere near the edges.
#[test]
fn the_word_is_drawn_grey_across_the_middle_of_the_sheet() {
    let work = Work::new();
    let (home, dir) = (work.home.as_path(), work.dir.path());
    let printed = a_printed_sheet(home, dir, "a4", 1);
    let delta = work.at("mark.pdf");

    let (ok, said) = run(
        home,
        &[
            "watermark",
            &at(&printed),
            "--text",
            "DRAFT",
            "-o",
            &at(&delta),
        ],
    );
    assert!(ok, "{said}");
    assert!(
        said.contains("DRAFT"),
        "it did not say what it wrote: {said}"
    );

    let ink = Ink::drawn(&delta, 1);
    assert!(ink.anything(), "the delta came out blank");

    // Grey, not black. 0.75 of full black is 191; a renderer is allowed to
    // soften an edge, so the darkest pixel is what is checked and it is checked
    // generously — but nothing like black may appear anywhere.
    assert!(
        ink.darkest() > 150,
        "something on the page is nearly black ({}), which would bury the \
         printing underneath",
        ink.darkest()
    );

    // Across the middle, both ways. The bearings a font puts either side of a
    // word are real and are not in the metrics Onionskin carries, so a few
    // millimetres out of nearly three hundred is as close as this can be.
    let (x, y) = ink.middle();
    assert!(
        (x - ink.width_mm / 2.0).abs() < 6.0,
        "not centred across the paper: {x:.0} mm of {:.0}",
        ink.width_mm
    );
    assert!(
        (y - ink.height_mm / 2.0).abs() < 6.0,
        "not centred down the paper: {y:.0} mm of {:.0}",
        ink.height_mm
    );

    // Clear of the edges, where the printer's grip is and where the unprintable
    // border lives.
    for (x, y, _) in &ink.spots {
        assert!(
            *x > 4.0 && *x < ink.width_mm - 4.0 && *y > 4.0 && *y < ink.height_mm - 4.0,
            "ink at {x:.0}, {y:.0} mm is in the margin of a \
             {:.0}x{:.0} mm sheet",
            ink.width_mm,
            ink.height_mm
        );
    }

    // And it really is big — a watermark that fits in a corner is a footnote.
    let widest = ink.spots.iter().map(|s| s.0).fold(f64::MIN, f64::max)
        - ink.spots.iter().map(|s| s.0).fold(f64::MAX, f64::min);
    assert!(
        widest > ink.width_mm * 0.6,
        "the word spans only {widest:.0} mm of a {:.0} mm sheet",
        ink.width_mm
    );
}

/// Somebody asking for a darker mark gets one, and is told what it will cost
/// them. This is the escape hatch, and the warning is the reason it can be one.
#[test]
fn a_darker_mark_is_given_and_the_cost_is_said_out_loud() {
    let work = Work::new();
    let (home, dir) = (work.home.as_path(), work.dir.path());
    let printed = a_printed_sheet(home, dir, "a4", 1);
    let delta = work.at("dark.pdf");

    let (ok, said) = run(
        home,
        &[
            "watermark",
            &at(&printed),
            "--text",
            "VOID",
            "--grey",
            "0.2",
            "-o",
            &at(&delta),
        ],
    );
    assert!(ok, "{said}");
    assert!(
        said.contains("hard to read"),
        "a mark dark enough to bury the page said nothing about it: {said}"
    );

    let ink = Ink::drawn(&delta, 1);
    assert!(ink.anything());
    assert!(
        ink.darkest() < 100,
        "0.2 grey was asked for and {} came out",
        ink.darkest()
    );
}

/// The delta has as many pages as the sheet even when one page is marked,
/// because a two-page sheet fed a one-page delta prints page one's watermark
/// onto page two.
#[test]
fn the_delta_matches_the_sheet_page_for_page() {
    let work = Work::new();
    let (home, dir) = (work.home.as_path(), work.dir.path());
    let printed = a_printed_sheet(home, dir, "a4", 3);
    let delta = work.at("one.pdf");

    let (ok, said) = run(
        home,
        &["watermark", &at(&printed), "--page", "2", "-o", &at(&delta)],
    );
    assert!(ok, "{said}");

    assert!(!Ink::drawn(&delta, 1).anything(), "page one was marked");
    assert!(Ink::drawn(&delta, 2).anything(), "page two was not marked");
    assert!(!Ink::drawn(&delta, 3).anything(), "page three was marked");

    // And --every-page marks all three.
    let all = work.at("all.pdf");
    let (ok, said) = run(
        home,
        &["watermark", &at(&printed), "--every-page", "-o", &at(&all)],
    );
    assert!(ok, "{said}");
    for page in 1..=3 {
        assert!(
            Ink::drawn(&all, page).anything(),
            "page {page} was left unmarked"
        );
    }
}

/// A landscape sheet gets a landscape diagonal. The word follows the paper, so
/// this is the case where a hard-coded forty-five degrees would show.
#[test]
fn the_word_follows_the_shape_of_the_paper() {
    let work = Work::new();
    let (home, dir) = (work.home.as_path(), work.dir.path());
    let printed = a_printed_sheet(home, dir, "297x210", 1);
    let delta = work.at("wide.pdf");

    let (ok, said) = run(
        home,
        &[
            "watermark",
            &at(&printed),
            "--text",
            "COPY",
            "-o",
            &at(&delta),
        ],
    );
    assert!(ok, "{said}");

    let ink = Ink::drawn(&delta, 1);
    assert!(ink.anything());
    assert!(
        ink.width_mm > ink.height_mm,
        "the delta is not landscape: {:.0}x{:.0}",
        ink.width_mm,
        ink.height_mm
    );
    for (x, y, _) in &ink.spots {
        assert!(
            *x > 4.0 && *x < ink.width_mm - 4.0 && *y > 4.0 && *y < ink.height_mm - 4.0,
            "ink at {x:.0}, {y:.0} mm ran off a landscape sheet"
        );
    }

    // It still runs up to the right: the topmost ink is to the right of the
    // bottom-most.
    let top = ink.spots.iter().map(|s| s.1).fold(f64::MAX, f64::min);
    let bottom = ink.spots.iter().map(|s| s.1).fold(f64::MIN, f64::max);
    let near = |want: f64| -> f64 {
        let picked: Vec<f64> = ink
            .spots
            .iter()
            .filter(|s| (s.1 - want).abs() < 2.0)
            .map(|s| s.0)
            .collect();
        picked.iter().sum::<f64>() / picked.len() as f64
    };
    assert!(
        near(top) > near(bottom),
        "the word runs downhill: top at {:.0} mm, bottom at {:.0} mm",
        near(top),
        near(bottom)
    );
}

/// A word too long to be a watermark is still a watermark: it is set smaller
/// rather than run off the paper.
#[test]
fn a_long_phrase_is_set_smaller_rather_than_run_off_the_edge() {
    let work = Work::new();
    let (home, dir) = (work.home.as_path(), work.dir.path());
    let printed = a_printed_sheet(home, dir, "a4", 1);

    for (name, text) in [
        ("short", "I"),
        ("word", "DRAFT"),
        ("phrase", "NOT FOR CIRCULATION"),
    ] {
        let delta = work.at(&format!("{name}.pdf"));
        let (ok, said) = run(
            home,
            &[
                "watermark",
                &at(&printed),
                "--text",
                text,
                "-o",
                &at(&delta),
            ],
        );
        assert!(ok, "{said}");
        let ink = Ink::drawn(&delta, 1);
        assert!(ink.anything(), "'{text}' wrote nothing");
        for (x, y, _) in &ink.spots {
            assert!(
                *x > 4.0 && *x < ink.width_mm - 4.0 && *y > 4.0 && *y < ink.height_mm - 4.0,
                "'{text}' put ink at {x:.0}, {y:.0} mm, off a \
                 {:.0}x{:.0} mm sheet",
                ink.width_mm,
                ink.height_mm
            );
        }
    }
}

/// --dry-run says what it would do and leaves nothing behind, the way it does
/// everywhere else.
#[test]
fn a_rehearsal_writes_nothing() {
    let work = Work::new();
    let (home, dir) = (work.home.as_path(), work.dir.path());
    let printed = a_printed_sheet(home, dir, "a4", 1);
    let delta = work.at("nothing.pdf");

    let (ok, said) = run(
        home,
        &[
            "watermark",
            &at(&printed),
            "--text",
            "DRAFT",
            "-o",
            &at(&delta),
            "--dry-run",
        ],
    );
    assert!(ok, "{said}");
    assert!(said.contains("DRAFT"), "{said}");
    assert!(!delta.exists(), "a rehearsal left a file behind");
}

/// A size somebody insisted on is taken at their word, and checked against the
/// paper.
///
/// The worked-out size always fits — that is what working it out is for. A size
/// given by hand need not: `--size 400` sets NOT FOR CIRCULATION about a metre
/// long, and the delta came out with the first and last letters clipped off by
/// the edge of the paper. A watermark nobody can read, and nothing said so.
#[test]
fn a_size_too_large_for_the_paper_is_said_out_loud() {
    let work = Work::new();
    let printed = a_printed_sheet(&work.home, work.dir.path(), "a4", 1);

    let delta = work.at("huge.pdf");
    let (ok, said) = run(
        &work.home,
        &[
            "watermark",
            &at(&printed),
            "--text",
            "NOT FOR CIRCULATION",
            "--size",
            "400",
            "-o",
            &at(&delta),
        ],
    );
    assert!(ok, "{said}");
    assert!(
        said.contains("edge of the paper"),
        "a word set a metre long on A4 said nothing: {said}"
    );

    // And the ink really does reach the edge, which is what the warning is
    // about — so it is not warning about nothing.
    let ink = Ink::drawn(&delta, 1);
    let (left, top, right, bottom) = ink.box_mm();
    assert!(
        left < 2.0 || top < 2.0 || right > ink.width_mm - 2.0 || bottom > ink.height_mm - 2.0,
        "nothing reached an edge: {left:.0},{top:.0} to {right:.0},{bottom:.0}"
    );

    // The size Onionskin works out for itself fits, and is not warned about.
    let fitted = work.at("fitted.pdf");
    let (ok, said) = run(
        &work.home,
        &[
            "watermark",
            &at(&printed),
            "--text",
            "NOT FOR CIRCULATION",
            "-o",
            &at(&fitted),
        ],
    );
    assert!(ok, "{said}");
    assert!(
        !said.contains("edge of the paper"),
        "a watermark that fits was warned about: {said}"
    );
}
