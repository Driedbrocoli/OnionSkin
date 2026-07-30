use super::*;

use std::path::Path;

/// A folder of its own, with the counters pointed at it.
fn somewhere() -> (tempfile::TempDir, std::sync::MutexGuard<'static, ()>) {
    let dir = tempfile::tempdir().expect("a place to work");
    let held = crate::calibrate::borrow_home(dir.path());
    (dir, held)
}

/// The whole feature: the second box of receipts is numbered 201 to 400.
#[test]
fn the_second_run_carries_on_where_the_first_stopped() {
    let (_dir, _home) = somewhere();

    let first = numbers(next_for("receipts"), 200);
    assert_eq!(first, 1..201, "a series never used starts at one");
    reached("receipts", first.end).unwrap();

    let second = numbers(next_for("receipts"), 200);
    assert_eq!(second, 201..401);
    reached("receipts", second.end).unwrap();

    assert_eq!(next_for("receipts"), 401);
    // Which is the thing a receipt book must never contain: two of the same.
    let used: Vec<usize> = first.chain(second).collect();
    let mut once = used.clone();
    once.sort_unstable();
    once.dedup();
    assert_eq!(once.len(), used.len(), "a number was printed twice");
}

/// Two receipt books printed from the same blank are two series, and must not
/// share a counter.
#[test]
fn two_series_count_separately() {
    let (_dir, _home) = somewhere();
    reached("receipts", 201).unwrap();
    reached("tickets", 5_001).unwrap();

    assert_eq!(next_for("receipts"), 201);
    assert_eq!(next_for("tickets"), 5_001);
    assert_eq!(next_for("never-used"), 1);
    assert_eq!(
        all(),
        vec![
            ("receipts".to_string(), 201),
            ("tickets".to_string(), 5_001)
        ]
    );
}

/// A run that failed wrote nothing, so it must not burn two hundred numbers:
/// the next box would start at 401 with nothing between 201 and 400 in
/// existence, and a gap is as hard to explain as a repeat.
#[test]
fn a_run_that_wrote_nothing_does_not_use_up_numbers() {
    let (_dir, _home) = somewhere();
    let asked = numbers(next_for("receipts"), 200);
    assert_eq!(asked, 1..201);
    // Nothing is written down, because nothing was printed.
    assert_eq!(next_for("receipts"), 1);
}

/// Stopping after five to look at them uses five numbers, not two hundred:
/// those five sheets exist and have numbers on them.
#[test]
fn a_run_cut_short_uses_only_the_numbers_it_printed() {
    let (_dir, _home) = somewhere();
    let asked = numbers(next_for("receipts"), 200);
    let printed = numbers(asked.start, 5);
    reached("receipts", printed.end).unwrap();

    assert_eq!(printed, 1..6);
    assert_eq!(next_for("receipts"), 6);
}

/// Put back to a number, which is how a mistake is undone and how a book that
/// started life outside Onionskin is joined halfway.
#[test]
fn a_series_can_be_started_anywhere_and_put_back() {
    let (_dir, _home) = somewhere();
    start_at("receipts", 4_471).unwrap();
    assert_eq!(numbers(next_for("receipts"), 3), 4_471..4_474);

    start_at("receipts", 1).unwrap();
    assert_eq!(next_for("receipts"), 1);

    // Zero is not a receipt number. Asking for it gets one rather than a
    // sheet numbered 0.
    start_at("receipts", 0).unwrap();
    assert_eq!(next_for("receipts"), 1);
    assert_eq!(numbers(0, 3), 1..4);

    assert!(forget("receipts").unwrap());
    assert!(
        !forget("receipts").unwrap(),
        "forgetting twice is not an error"
    );
    assert_eq!(next_for("receipts"), 1);
}

/// It survives the program being stopped, which is the whole point — the two
/// boxes are printed a week apart.
#[test]
fn the_count_is_still_there_next_week() {
    let (dir, _home) = somewhere();
    reached("receipts", 201).unwrap();
    assert!(path().is_file());

    // As a fresh run of the program reads it.
    let text = std::fs::read_to_string(path()).unwrap();
    let read: Counters = serde_json::from_str(&text).unwrap();
    assert_eq!(read.next.get("receipts"), Some(&201));
    // And it is a file somebody could read and fix by hand if they had to.
    assert!(text.contains("receipts"), "{text}");
    assert!(dir.path().join("series.json").is_file());
}

/// A file that went bad costs somebody one `--start-at`, not the ability to
/// print.
#[test]
fn a_broken_file_does_not_stop_the_printing() {
    let (_dir, _home) = somewhere();
    reached("receipts", 201).unwrap();
    std::fs::write(path(), "this is not json").unwrap();

    assert_eq!(next_for("receipts"), 1, "it should fall back, not panic");
    assert_eq!(all(), Vec::new());
    // And writing to it again fixes it.
    reached("receipts", 300).unwrap();
    assert_eq!(next_for("receipts"), 300);
}

/// A name that can be typed and read back, refused rather than cleaned up: a
/// series silently saved under a different name is a series somebody starts
/// again from 1 without noticing.
#[test]
fn a_series_name_is_one_a_person_can_type() {
    let (_dir, _home) = somewhere();
    for good in ["receipts", "receipts-2026", "Book_2", "a", "1"] {
        assert!(check_name(good).is_ok(), "{good}");
    }
    for bad in ["", "receipts 2026", "../escape", "book/one", "a\nb"] {
        assert!(check_name(bad).is_err(), "{bad}");
        assert!(reached(bad, 5).is_err(), "{bad}");
    }
    // Long enough for anything, and then refused rather than truncated.
    assert!(check_name(&"a".repeat(64)).is_ok());
    assert!(check_name(&"a".repeat(65)).is_err());

    // A refused name is not written down under some other spelling.
    assert_eq!(all(), Vec::new());
}

/// The line after a run has to say what was used and what comes next, because
/// the alternative is going and reading a JSON file in a hidden folder.
#[test]
fn the_run_says_what_it_used_and_what_is_next() {
    let said = where_it_got_to("receipts", numbers(201, 200));
    assert!(said.contains("201"), "{said}");
    assert!(said.contains("400"), "{said}");
    assert!(said.contains("401"), "{said}");

    // One sheet reads as one sheet, not "1 to 1" reading as a range.
    let one = where_it_got_to("receipts", numbers(7, 1));
    assert!(one.contains("7 to 7"), "{one}");

    // And nothing printed says nothing was used.
    let none = where_it_got_to("receipts", numbers(7, 0));
    assert!(none.contains("unchanged"), "{none}");
}

/// The counters go where everything else Onionskin keeps goes, so "what is on
/// my machine" has one answer.
#[test]
fn the_counters_live_with_everything_else() {
    let (dir, _home) = somewhere();
    assert!(path().starts_with(dir.path()));
    assert_eq!(
        path().file_name().and_then(|n| n.to_str()),
        Some("series.json")
    );
    assert!(!Path::new("series.json").is_absolute());
}
