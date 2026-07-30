//! `onionskin batch`, over more than one run.
//!
//! A receipt book is printed two hundred at a time. The first box is numbered 1
//! to 200 and the second has to be numbered 201 to 400, because two receipts
//! with the same number on them is the one thing a receipt book must never
//! contain — and it is found out months later, by an accountant.
//!
//! A run of two hundred also jams at sheet eighty, and what is wanted then is
//! the other hundred and twenty, still numbered 81 onwards.
//!
//! Both are about the numbers on the paper, so the tests here read the numbers
//! off the pages rather than trusting what the program said it did.

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

struct Printers {
    dir: tempfile::TempDir,
    home: PathBuf,
    /// A printed blank, the way a box of receipt stock arrives.
    blank: PathBuf,
}

impl Printers {
    fn new() -> Printers {
        let dir = tempfile::tempdir().expect("a place to work");
        let home = dir.path().join("home");
        std::fs::create_dir_all(&home).expect("a home of its own");

        let document = dir.path().join("receipt.osk");
        let blank = dir.path().join("receipt.pdf");
        for args in [
            vec!["new", &at(&document), "--page", "a4"],
            vec!["write", &at(&document), "--at", "20,30:RECEIPT"],
            vec!["print", &at(&document), "-o", &at(&blank)],
        ] {
            let (ok, said) = run(&home, &args);
            assert!(ok, "setting up: {said}");
        }
        Printers { dir, home, blank }
    }

    /// One page with these words on it, written literally — no {number}, no
    /// counter. The thing every numbered page is checked against.
    fn a_page_saying(&self, words: &str) -> PathBuf {
        let name: String = words
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect();
        let page = self.dir.path().join(format!("says{name}.pdf"));
        if page.is_file() {
            return page;
        }
        let (ok, said) = run(
            &self.home,
            &[
                "batch",
                &at(&self.blank),
                "--count",
                "1",
                "--at",
                &format!("150,40:{words}"),
                "-o",
                &at(&page),
            ],
        );
        assert!(ok, "writing the reference page: {said}");
        page
    }

    /// A box of receipts, numbered by whatever flags are given.
    fn a_box(&self, name: &str, extra: &[&str]) -> (bool, String, PathBuf) {
        let stack = self.dir.path().join(format!("{name}.pdf"));
        let mut args = vec![
            "batch".to_string(),
            at(&self.blank),
            "--at".to_string(),
            "150,40:No. {number}".to_string(),
            "-o".to_string(),
            at(&stack),
        ];
        args.extend(extra.iter().map(|s| s.to_string()));
        let borrowed: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        let (ok, said) = run(&self.home, &borrowed);
        (ok, said, stack)
    }
}

/// Check that the pages of a stack carry exactly these numbers, in order.
///
/// Each page is drawn and compared, pixel for pixel, against a page written
/// with the number spelled out — `--at '150,40:No. 8'`, with no `{number}` and
/// no counter anywhere in it. So the thing being checked against comes from
/// none of the machinery under test.
///
/// Reading the digits back off the page instead would be testing the letter
/// reader rather than the numbering, and it cannot reliably tell a `1` from a
/// `]` at nine point — a limit the reader documents about itself.
fn expect_numbers(shop: &Printers, stack: &Path, numbers: &[usize]) {
    let engine = onionskin::render::engine().expect("a renderer");
    const DPI: f64 = 100.0;

    let drawn = engine.open(stack).expect("the stack should open");
    assert_eq!(
        drawn.len(),
        numbers.len(),
        "the stack has {} pages and {} numbers were expected",
        drawn.len(),
        numbers.len()
    );
    for (index, number) in numbers.iter().enumerate() {
        let want = shop.a_page_saying(&format!("No. {number}"));
        let reference = engine.open(&want).expect("the reference should open");
        let expected = reference.render_gray(0, DPI).expect("it should draw");
        let got = drawn.render_gray(index, DPI).expect("it should draw");

        assert_eq!(
            (got.width, got.height),
            (expected.width, expected.height),
            "page {} is a different size from the reference",
            index + 1
        );
        let differing = got
            .gray
            .iter()
            .zip(expected.gray.iter())
            .filter(|(a, b)| a != b)
            .count();
        assert_eq!(
            differing,
            0,
            "page {} does not say 'No. {number}' — {differing} pixels differ from a page \
             written with those words",
            index + 1
        );
    }
}

/// The whole feature: the second box carries on where the first stopped.
#[test]
fn the_second_box_of_receipts_starts_where_the_first_ended() {
    let shop = Printers::new();

    let (ok, said, first) = shop.a_box("first", &["--count", "5", "--series", "receipts"]);
    assert!(ok, "{said}");
    expect_numbers(&shop, &first, &[1, 2, 3, 4, 5]);

    let (ok, said, second) = shop.a_box("second", &["--count", "5", "--series", "receipts"]);
    assert!(ok, "{said}");
    expect_numbers(&shop, &second, &[6, 7, 8, 9, 10]);
    // And it says so, so nobody has to go and read a file in a hidden folder.
    assert!(said.contains("6 to 10"), "{said}");
    assert!(said.contains("starts at 11"), "{said}");

    // Which is the property that matters, and it is what the two checks above
    // together say: ten sheets carrying ten different numbers, none repeated.
}

/// Two receipt books printed from the same blank are two series and must not
/// share a counter — which is why this is a name and not the file.
#[test]
fn two_series_from_the_same_blank_count_separately() {
    let shop = Printers::new();
    shop.a_box("a", &["--count", "3", "--series", "receipts"]);
    shop.a_box("b", &["--count", "3", "--series", "credit-notes"]);

    let (ok, said, again) = shop.a_box("c", &["--count", "2", "--series", "credit-notes"]);
    assert!(ok, "{said}");
    expect_numbers(&shop, &again, &[4, 5]);

    let (ok, said, receipts) = shop.a_box("d", &["--count", "2", "--series", "receipts"]);
    assert!(ok, "{said}");
    expect_numbers(&shop, &receipts, &[4, 5]);
}

/// Without a series, nothing is remembered and every run starts at one — the
/// right answer for two hundred certificates, which are not a series.
#[test]
fn a_run_with_no_series_starts_at_one_every_time() {
    let shop = Printers::new();
    for name in ["a", "b"] {
        let (ok, said, stack) = shop.a_box(name, &["--count", "3"]);
        assert!(ok, "{said}");
        expect_numbers(&shop, &stack, &[1, 2, 3]);
    }
    let (_, said) = run(&shop.home, &["doctor"]);
    assert!(said.contains("series     none"), "{said}");
}

/// A number given outright, for somebody who knows exactly where they are —
/// and for joining a book that began life outside Onionskin.
#[test]
fn the_numbering_can_be_started_anywhere() {
    let shop = Printers::new();
    let (ok, said, stack) = shop.a_box("joined", &["--count", "3", "--start-at", "4471"]);
    assert!(ok, "{said}");
    expect_numbers(&shop, &stack, &[4471, 4472, 4473]);
    assert!(said.contains("Numbered 4471 to 4473"), "{said}");
}

/// Given both, the number wins and the series is put back to it. That is how a
/// counter that has gone wrong is corrected, and there has to be a way.
#[test]
fn a_series_can_be_put_back_to_a_number() {
    let shop = Printers::new();
    shop.a_box("a", &["--count", "50", "--series", "receipts"]);

    let (ok, said, stack) = shop.a_box(
        "back",
        &["--count", "2", "--series", "receipts", "--start-at", "10"],
    );
    assert!(ok, "{said}");
    expect_numbers(&shop, &stack, &[10, 11]);
    assert!(said.contains("put back to 10"), "{said}");

    // And it carries on from there, not from 51.
    let (ok, said, after) = shop.a_box("after", &["--count", "2", "--series", "receipts"]);
    assert!(ok, "{said}");
    expect_numbers(&shop, &after, &[12, 13]);
}

/// A rehearsal that moved the counter on would not be a rehearsal, and the
/// next real run would start at 201 with nothing between 1 and 200 in
/// existence.
#[test]
fn a_dry_run_does_not_use_up_numbers() {
    let shop = Printers::new();
    let (ok, said, _) = shop.a_box(
        "rehearsal",
        &["--count", "5", "--series", "receipts", "--dry-run"],
    );
    assert!(ok, "{said}");
    assert!(said.contains("unchanged"), "{said}");

    let (ok, said, real) = shop.a_box("real", &["--count", "5", "--series", "receipts"]);
    assert!(ok, "{said}");
    expect_numbers(&shop, &real, &[1, 2, 3, 4, 5]);
}

/// Stopping after two to look at them uses two numbers, not five: those two
/// sheets exist and have numbers on them.
#[test]
fn stopping_early_uses_only_the_numbers_that_were_printed() {
    let shop = Printers::new();
    let (ok, said, few) = shop.a_box(
        "few",
        &["--count", "5", "--first", "2", "--series", "receipts"],
    );
    assert!(ok, "{said}");
    expect_numbers(&shop, &few, &[1, 2]);
    assert!(said.contains("used 1 to 2"), "{said}");

    let (ok, said, rest) = shop.a_box("rest", &["--count", "3", "--series", "receipts"]);
    assert!(ok, "{said}");
    expect_numbers(&shop, &rest, &[3, 4, 5]);
}

/// A run of two hundred jams at sheet eighty. What is wanted is the other
/// hundred and twenty, still numbered 81 onwards — a resumed run that
/// renumbered itself from one would be worse than no resume at all.
#[test]
fn a_jammed_run_is_picked_up_where_it_stopped() {
    let shop = Printers::new();
    let (ok, said, whole) = shop.a_box("whole", &["--count", "6"]);
    assert!(ok, "{said}");
    expect_numbers(&shop, &whole, &[1, 2, 3, 4, 5, 6]);

    let (ok, said, rest) = shop.a_box("rest", &["--count", "6", "--from-sheet", "4"]);
    assert!(ok, "{said}");
    expect_numbers(&shop, &rest, &[4, 5, 6]);
    assert!(said.contains("Picking up at sheet 4 of 6"), "{said}");
    assert!(said.contains("3 to make"), "{said}");
}

/// Resuming a run of a series is the one case where the counter cannot answer:
/// it has already moved past this run's numbers, and reading it would put 10,
/// 11, 12 on sheets the first run numbered 4, 5, 6. Wrong in a way nobody sees
/// until an accountant does, so it is refused — and the number handed over.
#[test]
fn picking_up_a_run_of_a_series_asks_which_numbers_it_had() {
    let shop = Printers::new();
    let (ok, said, _) = shop.a_box("first", &["--count", "6", "--series", "receipts"]);
    assert!(ok, "{said}");

    let (ok, said, _) = shop.a_box(
        "guess",
        &["--count", "6", "--series", "receipts", "--from-sheet", "4"],
    );
    assert!(!ok, "it should not guess: {said}");
    assert!(said.contains("--start-at"), "{said}");
    // And it works the number out rather than leaving somebody to: the run of
    // six used 1 to 6, so that is --start-at 1.
    assert!(said.contains("--start-at 1"), "{said}");
    assert!(said.contains("used 1 to 6"), "{said}");
}

/// Given the number, the resume prints exactly the sheets that jammed, and the
/// counter is left where it is: moving it on again would leave a gap the width
/// of the whole run, and a receipt book with a hole in it is as hard to explain
/// as one with a repeat.
#[test]
fn picking_up_a_run_does_not_use_the_numbers_twice_over() {
    let shop = Printers::new();
    let (ok, said, _) = shop.a_box("first", &["--count", "6", "--series", "receipts"]);
    assert!(ok, "{said}");

    // The printer jammed at sheet 3. The last three sheets again, numbered
    // as they were.
    let (ok, said, rest) = shop.a_box(
        "rest",
        &[
            "--count",
            "6",
            "--series",
            "receipts",
            "--from-sheet",
            "4",
            "--start-at",
            "1",
        ],
    );
    assert!(ok, "{said}");
    expect_numbers(&shop, &rest, &[4, 5, 6]);
    assert!(said.contains("unchanged"), "{said}");
    // Not "put back to 1" — a resume neither carries the series on nor puts it
    // back, and saying either would be a lie about what was written down.
    assert!(!said.contains("put back"), "{said}");

    // And the next real box still starts at 7, not at 10 and not at 4.
    let (ok, said, next) = shop.a_box("next", &["--count", "2", "--series", "receipts"]);
    assert!(ok, "{said}");
    expect_numbers(&shop, &next, &[7, 8]);
}

/// Resuming a run that was itself numbered from 201: sheet 4 of that run is
/// 204, which is the number on the sheet of paper that jammed.
#[test]
fn a_resumed_run_keeps_the_numbers_the_original_had() {
    let shop = Printers::new();
    let (ok, said, rest) = shop.a_box(
        "rest",
        &["--count", "6", "--start-at", "201", "--from-sheet", "4"],
    );
    assert!(ok, "{said}");
    expect_numbers(&shop, &rest, &[204, 205, 206]);
}

/// Past the end is refused, and says where the end is — rather than writing an
/// empty stack that looks like it worked.
#[test]
fn asking_to_resume_past_the_end_says_so() {
    let shop = Printers::new();
    let (ok, said, _) = shop.a_box("nope", &["--count", "5", "--from-sheet", "9"]);
    assert!(!ok, "{said}");
    assert!(said.contains("only 5 sheets"), "{said}");
    assert!(said.contains("--from-sheet 5"), "{said}");
}

/// A name that cannot be typed back is refused before anything is written,
/// rather than saved under some other spelling and never found again.
#[test]
fn a_series_name_has_to_be_one_a_person_can_type() {
    let shop = Printers::new();
    let (ok, said, _) = shop.a_box("bad", &["--count", "2", "--series", "../escape"]);
    assert!(!ok, "{said}");
    assert!(said.contains("not a name"), "{said}");
}

/// What Onionskin keeps has to include this, since it is state in a hidden
/// folder that changes what the next run prints.
#[test]
fn the_counters_are_listed_among_what_onionskin_keeps() {
    let shop = Printers::new();
    shop.a_box("a", &["--count", "3", "--series", "receipts"]);

    let (_, said) = run(&shop.home, &["doctor"]);
    assert!(said.contains("receipts"), "{said}");
    assert!(said.contains("next: 4"), "{said}");
}
