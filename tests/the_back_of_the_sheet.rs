//! `onionskin back`, driven the way somebody types it.
//!
//! The thing worth testing here is not that a delta comes out — it is that the
//! words land where the person meant, given that the paper may come back through
//! the printer either way up. Getting that wrong is not a crooked sheet, it is
//! the whole run printed at the wrong end of the paper and found afterwards.
//!
//! So every test here draws the delta, turns the picture of it the way a hand
//! turns paper, and measures where the ink really is.

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

    /// A printed document of so many pages, with something on the front of each.
    fn a_document(&self, pages: usize) -> PathBuf {
        let document = self.at("letter.osk");
        let printed = self.at("letter.pdf");
        let pages = pages.to_string();
        for args in [
            vec!["new", &at(&document), "--page", "a4", "--pages", &pages],
            vec!["write", &at(&document), "--at", "20,30:Invoice 2024-8817"],
            vec!["print", &at(&document), "-o", &at(&printed)],
        ] {
            let (ok, said) = run(&self.home, &args);
            assert!(ok, "{said}");
        }
        printed
    }
}

const A4_WIDTH_MM: f64 = 210.0;
const A4_HEIGHT_MM: f64 = 297.0;

/// Where the ink is on a page of a delta, as somebody holding the finished sheet
/// the right way up would see it.
///
/// `turned` is the whole point: it does to the picture what the printer's feed
/// does to the paper. Measuring the delta without it measures what the printer
/// sees, which is not what anybody is asking about.
fn ink_seen(delta: &Path, page: usize, turned: bool) -> Option<(f64, f64, f64, f64)> {
    const DPI: f64 = 100.0;
    let engine = onionskin::render::engine().expect("a renderer");
    let document = engine.open(delta).expect("the delta should open");
    let drawn = document.render_gray(page - 1, DPI).expect("it should draw");

    let seen: Vec<u8> = match turned {
        false => drawn.gray.clone(),
        true => drawn.gray.iter().rev().copied().collect(),
    };
    let mm = |pixels: usize| (pixels as f64 + 0.5) * 25.4 / DPI;
    let spots: Vec<(f64, f64)> = (0..drawn.height)
        .flat_map(|y| (0..drawn.width).map(move |x| (x, y)))
        .filter(|(x, y)| seen[y * drawn.width + x] < 128)
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

/// Whichever way the paper comes back, the words end up where they were asked
/// for. This is the whole feature.
#[test]
fn the_words_land_where_they_were_asked_for_either_way_the_paper_comes_back() {
    let work = Work::new();
    let printed = work.a_document(1);

    for (feed, turned) in [("same", false), ("turned", true)] {
        let delta = work.at(&format!("{feed}.pdf"));
        let (ok, said) = run(
            &work.home,
            &[
                "back",
                &at(&printed),
                "--at",
                "20,40:Continued overleaf",
                "--feed",
                feed,
                "-o",
                &at(&delta),
            ],
        );
        assert!(ok, "{said}");
        assert!(said.contains(feed), "it did not say which feed: {said}");

        let (left, _, _, baseline) =
            ink_seen(&delta, 1, turned).unwrap_or_else(|| panic!("{feed}: nothing was printed"));
        assert!(
            (left - 20.0).abs() < 2.0,
            "{feed}: the words start {left:.1} mm in, not 20"
        );
        assert!(
            (baseline - 40.0).abs() < 2.0,
            "{feed}: the words sit {baseline:.1} mm down, not 40"
        );
    }
}

/// And the wrong answer really does put them at the other end of the paper —
/// which is why the question is asked at all, and why there is a test sheet.
#[test]
fn the_wrong_feed_puts_the_words_at_the_other_end_of_the_paper() {
    let work = Work::new();
    let printed = work.a_document(1);
    let delta = work.at("turned.pdf");
    let (ok, said) = run(
        &work.home,
        &[
            "back",
            &at(&printed),
            "--at",
            "20,40:Continued overleaf",
            "--feed",
            "turned",
            "-o",
            &at(&delta),
        ],
    );
    assert!(ok, "{said}");

    // Read without turning the paper — which is what happens if the printer
    // does not do what was assumed.
    let (left, _, _, baseline) = ink_seen(&delta, 1, false).expect("nothing was printed");
    assert!(
        left > A4_WIDTH_MM / 2.0,
        "a turned placement stayed on the near half of the paper: {left:.0} mm"
    );
    assert!(
        baseline > A4_HEIGHT_MM / 2.0,
        "a turned placement stayed on the top half of the paper: {baseline:.0} mm"
    );
}

/// Every sheet in the stack gets its back written on, and the delta is as long
/// as the stack — because a stack of five fed a delta of one prints one back.
#[test]
fn every_sheet_in_the_stack_gets_a_back() {
    let work = Work::new();
    let printed = work.a_document(4);
    let delta = work.at("all.pdf");

    let (ok, said) = run(
        &work.home,
        &[
            "back",
            &at(&printed),
            "--at",
            "20,40:Terms overleaf",
            "-o",
            &at(&delta),
        ],
    );
    assert!(ok, "{said}");
    assert!(said.contains("4 sheet(s)"), "{said}");

    for page in 1..=4 {
        assert!(
            ink_seen(&delta, page, false).is_some(),
            "sheet {page} got no back"
        );
    }

    // And one sheet on its own leaves the others alone.
    let just_one = work.at("one.pdf");
    let (ok, said) = run(
        &work.home,
        &[
            "back",
            &at(&printed),
            "--at",
            "20,40:Only this one",
            "--sheet",
            "3",
            "-o",
            &at(&just_one),
        ],
    );
    assert!(ok, "{said}");
    for page in 1..=4 {
        assert_eq!(
            ink_seen(&just_one, page, false).is_some(),
            page == 3,
            "sheet {page} came out wrong when only sheet 3 was asked for"
        );
    }
}

/// A document already printed on both sides has a page for every back, and the
/// delta writes on those pages and no others — page six is sheet three's back.
#[test]
fn a_two_sided_document_is_written_on_its_even_pages() {
    let work = Work::new();
    let printed = work.a_document(6);
    let delta = work.at("both.pdf");

    let (ok, said) = run(
        &work.home,
        &[
            "back",
            &at(&printed),
            "--two-sided",
            "--at",
            "20,250:Page footer",
            "-o",
            &at(&delta),
        ],
    );
    assert!(ok, "{said}");
    assert!(
        said.contains("3 sheet(s)"),
        "six pages is three sheets: {said}"
    );
    // The thing that ruins the run if it is not said.
    assert!(
        said.contains("two-sided"),
        "it did not say to print it two-sided: {said}"
    );

    for page in 1..=6 {
        assert_eq!(
            ink_seen(&delta, page, false).is_some(),
            page % 2 == 0,
            "page {page} came out wrong: the backs are the even pages"
        );
    }
}

/// The sheet that answers the question has both answers on it, one at each end,
/// so exactly one of them is readable whichever way the paper comes out.
#[test]
fn the_check_sheet_carries_both_answers_one_at_each_end() {
    let work = Work::new();
    let printed = work.a_document(1);
    let delta = work.at("which.pdf");

    let (ok, said) = run(
        &work.home,
        &["back", &at(&printed), "--check", "-o", &at(&delta)],
    );
    assert!(ok, "{said}");
    assert!(said.contains("config set feed same"), "{said}");
    assert!(said.contains("config set feed turned"), "{said}");

    // Ink at both ends of the paper and nothing in the middle third, so there
    // is no doubt about which end a word is at.
    let (_, top, _, bottom) = ink_seen(&delta, 1, false).expect("the sheet came out blank");
    assert!(
        top < A4_HEIGHT_MM / 3.0,
        "nothing near the top: {top:.0} mm"
    );
    assert!(
        bottom > A4_HEIGHT_MM * 2.0 / 3.0,
        "nothing near the bottom: {bottom:.0} mm"
    );

    // And it reads the same either way up, because one answer is upside down.
    let upright = ink_seen(&delta, 1, false).unwrap();
    let turned = ink_seen(&delta, 1, true).unwrap();
    assert!(
        (upright.1 - turned.1).abs() < 2.0 && (upright.3 - turned.3).abs() < 2.0,
        "the sheet is not the same both ways up: {upright:?} against {turned:?}"
    );
}

/// The answer is remembered, so nobody is asked twice.
#[test]
fn the_answer_is_remembered_and_then_used() {
    let work = Work::new();
    let printed = work.a_document(1);

    let (ok, said) = run(&work.home, &["config", "set", "feed", "turned"]);
    assert!(ok, "{said}");

    let delta = work.at("remembered.pdf");
    let (ok, said) = run(
        &work.home,
        &[
            "back",
            &at(&printed),
            "--at",
            "20,40:Continued overleaf",
            "-o",
            &at(&delta),
        ],
    );
    assert!(ok, "{said}");
    assert!(
        said.contains("turned"),
        "the remembered answer was not used: {said}"
    );

    // The words are where they were asked for, on a sheet turned round.
    let (left, _, _, baseline) = ink_seen(&delta, 1, true).expect("nothing was printed");
    assert!((left - 20.0).abs() < 2.0, "{left:.1}");
    assert!((baseline - 40.0).abs() < 2.0, "{baseline:.1}");

    // And a flag still wins over what was remembered, for one run.
    let overruled = work.at("overruled.pdf");
    let (ok, said) = run(
        &work.home,
        &[
            "back",
            &at(&printed),
            "--at",
            "20,40:Continued overleaf",
            "--feed",
            "same",
            "-o",
            &at(&overruled),
        ],
    );
    assert!(ok, "{said}");
    let (left, _, _, baseline) = ink_seen(&overruled, 1, false).expect("nothing was printed");
    assert!((left - 20.0).abs() < 2.0, "{left:.1}");
    assert!((baseline - 40.0).abs() < 2.0, "{baseline:.1}");
}

/// Nothing to put on the back is a question with the answer in it, not a crash.
#[test]
fn nothing_to_put_on_the_back_says_what_to_type() {
    let work = Work::new();
    let printed = work.a_document(1);

    let (ok, said) = run(&work.home, &["back", &at(&printed)]);
    assert!(!ok, "{said}");
    assert!(said.contains("--at"), "{said}");
    assert!(said.contains("--check"), "{said}");

    // A feed nobody could mean says so, and says how to find out.
    let (ok, said) = run(
        &work.home,
        &[
            "back",
            &at(&printed),
            "--at",
            "20,40:x",
            "--feed",
            "sideways",
        ],
    );
    assert!(!ok, "{said}");
    assert!(said.contains("'same' or 'turned'"), "{said}");
    assert!(said.contains("--check"), "{said}");

    // And a sheet that is not in the stack.
    let (ok, said) = run(
        &work.home,
        &["back", &at(&printed), "--at", "20,40:x", "--sheet", "9"],
    );
    assert!(!ok, "{said}");
    assert!(said.contains("1 sheet(s)"), "{said}");
}

/// A rehearsal says what it would do and leaves nothing behind.
#[test]
fn a_rehearsal_writes_no_back() {
    let work = Work::new();
    let printed = work.a_document(2);
    let delta = work.at("nothing.pdf");

    let (ok, said) = run(
        &work.home,
        &[
            "back",
            &at(&printed),
            "--at",
            "20,40:Continued overleaf",
            "-o",
            &at(&delta),
            "--dry-run",
        ],
    );
    assert!(ok, "{said}");
    assert!(said.contains("2 sheet(s)"), "{said}");
    assert!(!delta.exists(), "a rehearsal left a file behind");
}
