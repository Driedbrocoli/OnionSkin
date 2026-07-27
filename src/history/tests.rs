//! Tests for the record of what was added.

use super::*;

fn an_entry(fingerprint: &str, at: u64) -> Entry {
    Entry {
        at,
        source: "invoice.pdf".to_string(),
        delta: "invoice-delta.pdf".to_string(),
        pages: 1,
        additions: 3,
        fingerprint: fingerprint.to_string(),
    }
}

/// The whole point: writing the same delta twice says so, and printing it
/// twice is the one mistake this program cannot undo.
#[test]
fn writing_the_same_delta_again_says_when_it_was_written_before() {
    let dir = tempfile::tempdir().unwrap();
    let _held = crate::calibrate::borrow_home(dir.path());

    assert!(remember(an_entry("abc", 1_700_000_000)).is_none());
    let again = remember(an_entry("abc", 1_700_003_600)).expect("the repeat was not noticed");
    assert_eq!(again.at, 1_700_000_000, "it named the wrong one");
}

/// A different delta is a different delta, however similar the job was.
#[test]
fn a_delta_that_differs_at_all_is_not_a_repeat() {
    let dir = tempfile::tempdir().unwrap();
    let _held = crate::calibrate::borrow_home(dir.path());

    remember(an_entry("abc", 1_700_000_000));
    assert!(remember(an_entry("abd", 1_700_000_100)).is_none());
}

/// A delta is never a repeat of itself: the lookup happens before the append,
/// or every first write would report itself.
#[test]
fn the_first_time_is_not_a_repeat_of_itself() {
    let dir = tempfile::tempdir().unwrap();
    let _held = crate::calibrate::borrow_home(dir.path());

    assert!(remember(an_entry("only-once", 1_700_000_000)).is_none());
    assert_eq!(read().len(), 1);
}

#[test]
fn the_most_recent_come_back_first() {
    let dir = tempfile::tempdir().unwrap();
    let _held = crate::calibrate::borrow_home(dir.path());

    for (n, fingerprint) in ["one", "two", "three"].iter().enumerate() {
        remember(an_entry(fingerprint, 1_700_000_000 + n as u64));
    }
    let recent = recent(2);
    assert_eq!(recent.len(), 2);
    assert_eq!(recent[0].fingerprint, "three");
    assert_eq!(recent[1].fingerprint, "two");
}

/// A file in somebody's home directory that grows forever is a bug with a
/// long fuse.
#[test]
fn the_record_does_not_grow_without_end() {
    let dir = tempfile::tempdir().unwrap();
    let _held = crate::calibrate::borrow_home(dir.path());

    for n in 0..(KEEP + 20) {
        remember(an_entry(&format!("f{n}"), 1_700_000_000 + n as u64));
    }
    let kept = read();
    assert_eq!(kept.len(), KEEP);
    // And it is the oldest that went.
    assert_eq!(kept[0].fingerprint, "f20");
    assert_eq!(kept[KEEP - 1].fingerprint, format!("f{}", KEEP + 19));
}

/// A half-written line from a crash, or one from a version that wrote
/// something else, costs that line and not the record.
#[test]
fn a_line_that_cannot_be_read_does_not_take_the_rest_with_it() {
    let dir = tempfile::tempdir().unwrap();
    let _held = crate::calibrate::borrow_home(dir.path());

    remember(an_entry("good", 1_700_000_000));
    let mut text = std::fs::read_to_string(path()).unwrap();
    text.push_str("{\"at\": 1, \"source\": \"cut off half way\n");
    text.push_str("not json at all\n");
    std::fs::write(path(), text).unwrap();

    let kept = read();
    assert_eq!(kept.len(), 1, "{kept:?}");
    assert_eq!(kept[0].fingerprint, "good");
}

#[test]
fn forgetting_says_how_much_it_forgot_and_leaves_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let _held = crate::calibrate::borrow_home(dir.path());

    remember(an_entry("a", 1_700_000_000));
    remember(an_entry("b", 1_700_000_001));
    assert_eq!(forget(), 2);
    assert!(read().is_empty());
    assert!(!path().exists());
    // And forgetting nothing is not an error.
    assert_eq!(forget(), 0);
}

#[test]
fn nothing_remembered_yet_is_an_empty_list_rather_than_a_failure() {
    let dir = tempfile::tempdir().unwrap();
    let _held = crate::calibrate::borrow_home(dir.path());
    assert!(read().is_empty());
    assert!(recent(10).is_empty());
    assert!(seen_before("anything").is_none());
}

/// The fingerprint is of the file, so two identical deltas match and a
/// one-letter difference does not.
#[test]
fn the_fingerprint_is_of_the_delta_itself() {
    let dir = tempfile::tempdir().unwrap();
    let same = dir.path().join("a.pdf");
    let copy = dir.path().join("b.pdf");
    let other = dir.path().join("c.pdf");
    std::fs::write(&same, b"a delta").unwrap();
    std::fs::write(&copy, b"a delta").unwrap();
    std::fs::write(&other, b"a delts").unwrap();

    assert_eq!(fingerprint(&same), fingerprint(&copy));
    assert_ne!(fingerprint(&same), fingerprint(&other));
    assert!(fingerprint(&dir.path().join("not-there.pdf")).is_none());
}

#[test]
fn a_date_is_written_the_way_somebody_reads_one() {
    // 2023-11-14 22:13:20 UTC.
    assert_eq!(when(1_700_000_000), "2023-11-14 22:13");
    assert_eq!(when(0), "1970-01-01 00:00");
}

/// A path too long for its column loses its front, because the file name is
/// the part that says which one it is.
#[test]
fn a_long_path_keeps_the_end_that_identifies_it() {
    let long = "/home/somebody/documents/work/invoices/2026/march/invoice-4471.pdf";
    let cut = shorten(long, 20);
    assert_eq!(cut.chars().count(), 20);
    assert!(cut.ends_with("invoice-4471.pdf"), "{cut}");
    assert!(cut.starts_with('…'), "{cut}");
    // And one that fits is left exactly alone.
    assert_eq!(shorten("short.pdf", 20), "short.pdf");
}

#[test]
fn how_long_ago_is_said_in_words_rather_than_a_timestamp() {
    let now = now();
    let entry = |ago: u64| Entry {
        at: now.saturating_sub(ago),
        ..an_entry("x", 0)
    };
    assert_eq!(entry(10).how_long_ago(), "a moment ago");
    assert!(entry(600).how_long_ago().contains("minutes"));
    assert!(entry(7_200).how_long_ago().contains("hours"));
    assert!(entry(300_000).how_long_ago().contains("days"));
    // A clock that went backwards is not an error worth reporting.
    assert_eq!(
        Entry {
            at: now + 500,
            ..an_entry("x", 0)
        }
        .how_long_ago(),
        "just now"
    );
}
