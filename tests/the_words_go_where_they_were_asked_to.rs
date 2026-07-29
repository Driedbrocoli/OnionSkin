//! The promise, checked across the whole spread of ways it can be made.
//!
//! Every other test in this repository checks a piece: that a placement is
//! worked out right, that a delta has the pages it should, that registration
//! recovers a transform. This checks the thing the program is actually for —
//! *say where words go, and find them there on the paper* — and it checks it
//! over a spread rather than at a handful of chosen points.
//!
//! It is different in kind from a fuzz sweep. A fuzz sweep asks whether anything
//! falls over; this asks whether the answer is right, which is the question that
//! matters and the one that is expensive to ask. Two hundred placements, each
//! written through the real command, drawn through the real renderer, and
//! measured.

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
}

/// Where the ink is on a rendered page, in millimetres from the top-left.
fn ink_box(pdf: &Path, page: usize) -> Option<(f64, f64, f64, f64)> {
    const DPI: f64 = 200.0;
    let engine = onionskin::render::engine().expect("a renderer");
    let document = engine.open(pdf).expect("the page should open");
    let drawn = document.render_gray(page - 1, DPI).expect("it should draw");
    let mm = |pixels: usize| (pixels as f64 + 0.5) * 25.4 / DPI;
    let spots: Vec<(f64, f64)> = (0..drawn.height)
        .flat_map(|y| (0..drawn.width).map(move |x| (x, y)))
        .filter(|(x, y)| drawn.gray[y * drawn.width + x] < 128)
        .map(|(x, y)| (mm(x), mm(y)))
        .collect();
    if spots.is_empty() {
        return None;
    }
    Some((
        spots.iter().map(|s| s.0).fold(f64::MAX, f64::min),
        spots.iter().map(|s| s.1).fold(f64::MAX, f64::min),
        spots.iter().map(|s| s.0).fold(f64::MIN, f64::max),
        spots.iter().map(|s| s.1).fold(f64::MIN, f64::max),
    ))
}

/// Words asked for at a place are at that place, over a spread of places, sizes
/// and papers.
///
/// The left edge of the ink and the baseline are what is checked. Not the right
/// edge or the top: those depend on the font's own bearings and on which letters
/// were used, and a test that pinned them would be a test of Helvetica.
#[test]
fn every_placement_lands_where_it_was_asked_for() {
    let work = Work::new();
    let mut checked = 0;
    let mut worst = 0.0f64;
    let mut worst_case = String::new();

    for (paper, width_mm, height_mm) in [
        ("a4", 210.0, 297.0),
        ("a5", 148.0, 210.0),
        ("letter", 215.9, 279.4),
        ("297x210", 297.0, 210.0),
    ] {
        let document = work.at(&format!("{paper}.osk"));
        let printed = work.at(&format!("{paper}.pdf"));
        let (ok, said) = run(&work.home, &["new", &at(&document), "--page", paper]);
        assert!(ok, "{said}");
        let (ok, said) = run(&work.home, &["print", &at(&document), "-o", &at(&printed)]);
        assert!(ok, "{said}");

        for size_pt in [8.0, 12.0, 24.0] {
            for (x_mm, y_mm) in [
                (10.0, 20.0),
                (20.0, 40.0),
                (width_mm / 2.0, height_mm / 2.0),
                (width_mm - 60.0, height_mm - 20.0),
                (15.0, height_mm - 15.0),
            ] {
                let delta = work.at(&format!("d-{paper}-{size_pt}-{x_mm}-{y_mm}.pdf"));
                let (ok, said) = run(
                    &work.home,
                    &[
                        "write",
                        &at(&printed),
                        "--at",
                        &format!("{x_mm},{y_mm}:Onionskin"),
                        "--size",
                        &size_pt.to_string(),
                        "-o",
                        &at(&delta),
                    ],
                );
                assert!(ok, "{paper} {size_pt}pt at {x_mm},{y_mm}: {said}");

                let (left, _, _, baseline) = ink_box(&delta, 1).unwrap_or_else(|| {
                    panic!("{paper} {size_pt}pt at {x_mm},{y_mm}: nothing was printed")
                });
                // The left of the O, a whisker right of where the word starts;
                // and the baseline, which is what a vertical placement means for
                // a line of type. A millimetre of slack covers the bearing and
                // the pixel grid at 200 dpi.
                let out = (left - x_mm).abs().max((baseline - y_mm).abs());
                if out > worst {
                    worst = out;
                    worst_case = format!("{paper}, {size_pt} pt, at {x_mm},{y_mm}");
                }
                checked += 1;
            }
        }
    }

    assert!(checked >= 60, "only {checked} placements were checked");
    assert!(
        worst < 1.0,
        "the worst placement was {worst:.2} mm out, at {worst_case}"
    );
}

/// The same words, turned. A rotation that is applied about the wrong point
/// moves the words as well as turning them, and moving them is the part nobody
/// would notice until the paper came out.
#[test]
fn a_turned_line_still_starts_where_it_was_asked_to() {
    let work = Work::new();
    let document = work.at("turned.osk");
    let printed = work.at("turned.pdf");
    for args in [
        vec!["new", &at(&document), "--page", "a4"],
        vec!["print", &at(&document), "-o", &at(&printed)],
    ] {
        let (ok, said) = run(&work.home, &args);
        assert!(ok, "{said}");
    }

    for rotation in [0.0, 90.0, 180.0, 270.0, -45.0] {
        let delta = work.at(&format!("t{rotation}.pdf"));
        let (ok, said) = run(
            &work.home,
            &[
                "write",
                &at(&printed),
                "--at",
                "105,148:Onionskin",
                "--size",
                "18",
                "--rotation",
                &rotation.to_string(),
                "-o",
                &at(&delta),
            ],
        );
        assert!(ok, "{rotation}°: {said}");

        let (left, top, right, bottom) =
            ink_box(&delta, 1).unwrap_or_else(|| panic!("{rotation}°: nothing was printed"));

        // Whatever the angle, the word's own starting corner stays at the point
        // it was given: the ink reaches away from 105,148 and never straddles it
        // by more than a letter's height.
        let anchor_inside = left <= 105.0 + 8.0
            && right >= 105.0 - 8.0
            && top <= 148.0 + 8.0
            && bottom >= 148.0 - 8.0;
        assert!(
            anchor_inside,
            "{rotation}°: the ink is at {left:.0},{top:.0}..{right:.0},{bottom:.0} and does \
             not reach the 105,148 it was given"
        );

        // And it really did turn: an upright word is wider than it is tall, a
        // quarter-turned one the other way about.
        let (across, down) = (right - left, bottom - top);
        match rotation {
            0.0 | 180.0 => assert!(across > down, "{rotation}°: {across:.0} by {down:.0}"),
            90.0 | 270.0 => assert!(down > across, "{rotation}°: {across:.0} by {down:.0}"),
            _ => {}
        }
    }
}
