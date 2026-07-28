//! Tests for saved jobs.

use super::*;

fn a_job(name: &str) -> Job {
    Job {
        name: name.to_string(),
        at: vec!["150,40:PAID {today}".to_string()],
        size_pt: 9.0,
        font: "Helvetica".to_string(),
        colour: "#000000".to_string(),
        leading: 1.2,
        page: 1,
        created: 1_700_000_000,
        ..Default::default()
    }
}

/// The whole point: work it out once, run it every Monday.
#[test]
fn a_job_saved_comes_back_exactly_as_it_went_in() {
    let dir = tempfile::tempdir().unwrap();
    let _held = crate::calibrate::borrow_home(dir.path());

    let job = a_job("paid");
    save(&job).unwrap();
    assert_eq!(load("paid").unwrap(), job);
}

#[test]
fn saved_jobs_are_listed_by_name() {
    let dir = tempfile::tempdir().unwrap();
    let _held = crate::calibrate::borrow_home(dir.path());

    save(&a_job("zebra")).unwrap();
    save(&a_job("apple")).unwrap();
    let names: Vec<String> = list().into_iter().map(|job| job.name).collect();
    assert_eq!(names, vec!["apple", "zebra"]);
}

/// A name that is not found says what *is* there, because the usual cause is
/// a typo or a half-remembered name and the list is the answer to both.
#[test]
fn a_name_that_is_not_there_says_what_is() {
    let dir = tempfile::tempdir().unwrap();
    let _held = crate::calibrate::borrow_home(dir.path());

    let said = load("paid").unwrap_err().to_string();
    assert!(said.contains("Nothing has been saved yet"), "{said}");

    save(&a_job("paid-stamp")).unwrap();
    let said = load("paidstamp").unwrap_err().to_string();
    assert!(said.contains("paid-stamp"), "{said}");
}

/// Refused rather than cleaned up: a job silently saved under a name other
/// than the one somebody typed is a job they cannot find again.
#[test]
fn a_name_that_cannot_be_a_file_is_refused_rather_than_mangled() {
    let dir = tempfile::tempdir().unwrap();
    let _held = crate::calibrate::borrow_home(dir.path());

    for bad in ["", "../escape", "with space", "slash/es", "dots.and.dots"] {
        assert!(check_name(bad).is_err(), "'{bad}' was accepted");
        assert!(save(&a_job(bad)).is_err(), "'{bad}' was saved");
        assert!(load(bad).is_err(), "'{bad}' was loaded");
    }
    for good in ["paid", "paid-stamp", "paid_stamp", "stamp2026"] {
        assert!(check_name(good).is_ok(), "'{good}' was refused");
    }
}

/// A name that walks out of the jobs folder must not be able to read or write
/// anything outside it.
#[test]
fn a_name_cannot_reach_outside_the_jobs_folder() {
    let dir = tempfile::tempdir().unwrap();
    let _held = crate::calibrate::borrow_home(dir.path());
    let outside = dir.path().join("settings.json");
    std::fs::write(&outside, "{}").unwrap();

    assert!(load("../settings").is_err());
    assert!(delete("../settings").is_err());
    assert!(outside.is_file(), "a job name deleted something outside");
}

#[test]
fn deleting_says_whether_there_was_anything_to_delete() {
    let dir = tempfile::tempdir().unwrap();
    let _held = crate::calibrate::borrow_home(dir.path());

    save(&a_job("paid")).unwrap();
    assert!(delete("paid").unwrap());
    assert!(!delete("paid").unwrap());
    assert!(list().is_empty());
}

// ---------------------------------------------------------------------------
// Filling in the blanks
// ---------------------------------------------------------------------------

/// Today's date is the commonest thing anybody stamps onto paper, it is
/// different every day, and a person typing it by hand eventually stamps
/// yesterday's.
#[test]
fn todays_date_is_filled_in_without_being_asked_for() {
    let job = Job {
        at: vec!["150,40:PAID {today}".to_string()],
        ..a_job("paid")
    };
    assert!(job.wants().is_empty(), "{:?}", job.wants());

    // 2023-11-14.
    let row = values(&BTreeMap::new(), 1_700_000_000);
    assert_eq!(crate::rows::fill("PAID {today}", &row), "PAID 2023-11-14");
    assert_eq!(
        crate::rows::fill("{year}/{month}/{day}", &row),
        "2023/11/14"
    );
}

/// Given wins over known, so somebody stamping yesterday's post with
/// yesterday's date is believed.
#[test]
fn a_date_given_by_hand_beats_the_one_the_program_knows() {
    let mut given = BTreeMap::new();
    given.insert("today".to_string(), "1999-12-31".to_string());
    let row = values(&given, 1_700_000_000);
    assert_eq!(crate::rows::fill("{today}", &row), "1999-12-31");
}

/// "You did not say what {ref} is" has to arrive while somebody is still at
/// the keyboard, not as a hundred sheets of paper saying `{ref}`.
#[test]
fn what_a_job_still_needs_is_known_before_it_runs() {
    let job = Job {
        at: vec!["150,40:PAID {today}".to_string()],
        after: vec!["Invoice:{ref}".to_string()],
        images: vec!["signatures/{who}.png:10,10:40".to_string()],
        ..a_job("paid")
    };
    let wants = job.wants();
    assert_eq!(wants, vec!["ref", "who"], "{wants:?}");

    let mut given = BTreeMap::new();
    given.insert("ref".to_string(), "4471".to_string());
    assert_eq!(job.missing(&given), vec!["who"]);

    given.insert("who".to_string(), "ann".to_string());
    assert!(job.missing(&given).is_empty());
}

/// A name asked for twice is asked for once.
#[test]
fn a_name_used_in_two_places_is_only_wanted_once() {
    let job = Job {
        at: vec!["10,10:{ref}".to_string(), "10,20:also {ref}".to_string()],
        ..a_job("x")
    };
    assert_eq!(job.wants(), vec!["ref"]);
}

#[test]
fn braces_that_are_not_names_are_left_alone() {
    assert!(braces_in("no braces here").is_empty());
    // An unmatched brace is a brace, as it is everywhere else in the program.
    assert!(braces_in("half {open").is_empty());
    assert!(braces_in("{}").is_empty());
    assert_eq!(braces_in("{a} and {b}"), vec!["a", "b"]);
}

/// What a job is, in the words somebody would use to check it is the right
/// one — including what it will ask for.
#[test]
fn a_job_describes_itself_including_what_it_will_want() {
    let job = Job {
        at: vec!["150,40:PAID {today}".to_string()],
        after: vec!["Invoice:{ref}".to_string()],
        notes: "the stamp for supplier invoices".to_string(),
        ..a_job("paid")
    };
    let said = job.describe();
    assert!(said.contains("job 'paid'"), "{said}");
    assert!(said.contains("150,40:PAID {today}"), "{said}");
    assert!(said.contains("Helvetica at 9 pt"), "{said}");
    assert!(said.contains("supplier invoices"), "{said}");
    assert!(said.contains("--set ref="), "{said}");
    // And it does not ask for the one it already knows.
    assert!(!said.contains("--set today="), "{said}");
}

/// A job written by a later version, with a field this one has never heard of,
/// still loads — and one missing a field takes a sensible default rather than
/// refusing.
#[test]
fn a_job_from_another_version_still_loads() {
    let dir = tempfile::tempdir().unwrap();
    let _held = crate::calibrate::borrow_home(dir.path());
    std::fs::create_dir_all(super::dir()).unwrap();
    std::fs::write(
        path_of("sparse"),
        r#"{"name": "sparse", "at": ["10,10:hello"], "something_new": 7}"#,
    )
    .unwrap();

    let job = load("sparse").unwrap();
    assert_eq!(job.at, vec!["10,10:hello"]);
    assert_eq!(job.size_pt, 11.0, "no default type size");
    assert_eq!(job.font, "Helvetica");
    assert_eq!(job.page, 1);
}
