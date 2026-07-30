//! `onionskin setup`, driven the way an office would.
//!
//! Somebody measures the printer, works out where the stamp goes, saves it as a
//! job. That is an afternoon. The next person installs Onionskin and none of it
//! is there — so they do the afternoon again, slightly differently, and the two
//! machines put the stamp in two places.
//!
//! The tests here are two real machines: two homes, two sets of everything
//! Onionskin keeps, and the file carried between them.

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

struct Office {
    dir: tempfile::TempDir,
    /// The machine somebody set up.
    first: PathBuf,
    /// The one that has just had Onionskin installed on it.
    second: PathBuf,
    sheet: PathBuf,
}

impl Office {
    fn new() -> Office {
        let dir = tempfile::tempdir().expect("somewhere to work");
        let first = dir.path().join("first");
        let second = dir.path().join("second");
        for home in [&first, &second] {
            std::fs::create_dir_all(home).expect("a machine of its own");
        }

        let document = dir.path().join("invoice.osk");
        let sheet = dir.path().join("invoice.pdf");
        for args in [
            vec!["new", &at(&document), "--page", "a4"],
            vec!["write", &at(&document), "--at", "20,30:Invoice no: 4471"],
            vec!["print", &at(&document), "-o", &at(&sheet)],
        ] {
            let (ok, said) = run(&first, &args);
            assert!(ok, "setting up: {said}");
        }
        Office {
            dir,
            first,
            second,
            sheet,
        }
    }

    /// An afternoon's work on the first machine.
    fn set_the_first_one_up(&self) {
        let scratch = self.dir.path().join("scratch.pdf");
        for args in [
            vec![
                "write",
                &at(&self.sheet),
                "--at",
                "150,40:PAID {today}",
                "--size",
                "9",
                "--save-as",
                "paid",
                "-o",
                &at(&scratch),
            ],
            vec!["config", "set", "dpi", "300"],
        ] {
            let (ok, said) = run(&self.first, &args);
            assert!(ok, "{said}");
        }
    }
}

/// The whole feature: an afternoon's setting up, carried across.
#[test]
fn what_took_an_afternoon_arrives_on_the_second_machine() {
    let office = Office::new();
    office.set_the_first_one_up();
    let carried = office.dir.path().join("our-office.json");

    let (ok, said) = run(
        &office.first,
        &["setup", "save", &at(&carried), "--note", "the big Ricoh"],
    );
    assert!(ok, "{said}");
    assert!(said.contains("paid"), "{said}");
    assert!(said.contains("dpi=300"), "{said}");
    // It says how to use it, on the machine that has not been set up.
    assert!(said.contains("setup use"), "{said}");

    // The second machine has nothing.
    let (_, said) = run(&office.second, &["job", "list"]);
    assert!(said.contains("No jobs saved yet"), "{said}");

    let (ok, said) = run(&office.second, &["setup", "use", &at(&carried)]);
    assert!(ok, "{said}");
    assert!(said.contains("added job"), "{said}");

    // And it is really there.
    let (_, said) = run(&office.second, &["job", "show", "paid"]);
    assert!(said.contains("150,40:PAID"), "{said}");
    let (_, said) = run(&office.second, &["config", "show"]);
    assert!(said.contains("300"), "{said}");
}

/// The one that would do real harm. A series remembers the receipt book has
/// reached 4; carried across, both machines print 5 next, and two receipts with
/// the same number on them is the one thing a receipt book must never contain.
#[test]
fn the_receipt_counter_does_not_travel() {
    let office = Office::new();
    office.set_the_first_one_up();

    // The first machine has printed three receipts.
    let stack = office.dir.path().join("receipts.pdf");
    let (ok, said) = run(
        &office.first,
        &[
            "batch",
            &at(&office.sheet),
            "--count",
            "3",
            "--at",
            "150,60:No. {number}",
            "--series",
            "receipts",
            "-o",
            &at(&stack),
        ],
    );
    assert!(ok, "{said}");
    assert!(said.contains("starts at 4"), "{said}");

    let carried = office.dir.path().join("our-office.json");
    let (ok, said) = run(&office.first, &["setup", "save", &at(&carried)]);
    assert!(ok, "{said}");
    // It says so, so nobody is surprised.
    assert!(said.contains("numbered series"), "{said}");

    // Nothing about it is in the file, however it is looked for.
    let text = std::fs::read_to_string(&carried).expect("it should read");
    assert!(!text.contains("receipts"), "{text}");

    run(&office.second, &["setup", "use", &at(&carried)]);
    let (_, said) = run(&office.second, &["doctor"]);
    assert!(
        said.contains("series     none"),
        "the second machine took the first one's counter: {said}"
    );

    // So its first receipt is number 1, not number 4.
    let theirs = office.dir.path().join("theirs.pdf");
    let (ok, said) = run(
        &office.second,
        &[
            "batch",
            &at(&office.sheet),
            "--count",
            "1",
            "--at",
            "150,60:No. {number}",
            "--series",
            "receipts",
            "-o",
            &at(&theirs),
        ],
    );
    assert!(ok, "{said}");
    assert!(said.contains("used 1 to 1"), "{said}");
}

/// The history is a record of what *that* machine printed, naming the files it
/// was done to. Handing it to a colleague hands over a list of every document
/// somebody worked on.
#[test]
fn the_record_of_what_was_printed_does_not_travel() {
    let office = Office::new();
    office.set_the_first_one_up();

    // Something with a name nobody would want to pass on.
    let private = office.dir.path().join("salary-review-confidential.pdf");
    std::fs::copy(&office.sheet, &private).expect("it should copy");
    let delta = office.dir.path().join("private-delta.pdf");
    let (ok, said) = run(
        &office.first,
        &["write", &at(&private), "--at", "20,20:x", "-o", &at(&delta)],
    );
    assert!(ok, "{said}");
    // Asked of the record itself, not of the listing — which shortens a long
    // path from the front, so the beginning of the name is not in it.
    let record = office.first.join("history.jsonl");
    let kept = std::fs::read_to_string(&record).expect("the record should be there");
    assert!(
        kept.contains("salary-review-confidential"),
        "the record should have it: {kept}"
    );

    let carried = office.dir.path().join("our-office.json");
    run(&office.first, &["setup", "save", &at(&carried)]);
    let text = std::fs::read_to_string(&carried).expect("it should read");
    assert!(!text.contains("salary-review"), "{text}");

    run(&office.second, &["setup", "use", &at(&carried)]);
    let (_, said) = run(&office.second, &["history"]);
    assert!(
        said.contains("Nothing written yet"),
        "somebody else's record arrived: {said}"
    );
}

/// Somebody's own job called `paid`, worked out for their own form, must not be
/// quietly replaced — that is how a person comes to print the wrong thing on a
/// document they have printed correctly a hundred times.
#[test]
fn a_job_of_their_own_with_the_same_name_is_kept() {
    let office = Office::new();
    office.set_the_first_one_up();
    let carried = office.dir.path().join("our-office.json");
    run(&office.first, &["setup", "save", &at(&carried)]);

    // The second person has their own 'paid', in a different place.
    let scratch = office.dir.path().join("theirs.pdf");
    let (ok, said) = run(
        &office.second,
        &[
            "write",
            &at(&office.sheet),
            "--at",
            "5,5:MY OWN PAID",
            "--save-as",
            "paid",
            "-o",
            &at(&scratch),
        ],
    );
    assert!(ok, "{said}");

    let (ok, said) = run(&office.second, &["setup", "use", &at(&carried)]);
    assert!(ok, "{said}");
    assert!(said.contains("kept yours"), "{said}");
    assert!(said.contains("--replace"), "{said}");

    let (_, said) = run(&office.second, &["job", "show", "paid"]);
    assert!(said.contains("MY OWN PAID"), "theirs was replaced: {said}");

    // Asked outright, it does replace.
    let (ok, said) = run(
        &office.second,
        &["setup", "use", &at(&carried), "--replace"],
    );
    assert!(ok, "{said}");
    assert!(said.contains("replaced job"), "{said}");
    let (_, said) = run(&office.second, &["job", "show", "paid"]);
    assert!(said.contains("150,40:PAID"), "{said}");
}

/// Safe in a login script or handed round an office: taking the same file twice
/// changes nothing the second time.
#[test]
fn taking_it_twice_changes_nothing_the_second_time() {
    let office = Office::new();
    office.set_the_first_one_up();
    let carried = office.dir.path().join("our-office.json");
    run(&office.first, &["setup", "save", &at(&carried)]);

    let (ok, said) = run(&office.second, &["setup", "use", &at(&carried)]);
    assert!(ok, "{said}");
    assert!(said.contains("added job"), "{said}");

    let (ok, said) = run(&office.second, &["setup", "use", &at(&carried)]);
    assert!(ok, "{said}");
    assert!(said.contains("already had everything"), "{said}");

    let (_, said) = run(&office.second, &["job", "list"]);
    assert_eq!(
        said.matches("paid").count(),
        1,
        "the job was doubled: {said}"
    );
}

/// A rehearsal says what the real thing would do, and does none of it.
#[test]
fn a_dry_run_says_what_would_happen_and_takes_nothing() {
    let office = Office::new();
    office.set_the_first_one_up();
    let carried = office.dir.path().join("our-office.json");
    run(&office.first, &["setup", "save", &at(&carried)]);

    let (ok, said) = run(
        &office.second,
        &["setup", "use", &at(&carried), "--dry-run"],
    );
    assert!(ok, "{said}");
    assert!(said.contains("added job"), "{said}");
    assert!(said.contains("Nothing taken"), "{said}");

    let (_, said) = run(&office.second, &["job", "list"]);
    assert!(said.contains("No jobs saved yet"), "{said}");

    // `show` is the same question asked of the file rather than the machine.
    let (ok, said) = run(&office.second, &["setup", "show", &at(&carried)]);
    assert!(ok, "{said}");
    assert!(said.contains("paid"), "{said}");
    let (_, still) = run(&office.second, &["job", "list"]);
    assert!(still.contains("No jobs saved yet"), "{still}");
}

/// A machine with nothing on it has nothing to carry, and says so rather than
/// writing an empty file somebody would then hand round.
#[test]
fn a_machine_that_is_not_set_up_says_there_is_nothing_to_carry() {
    let office = Office::new();
    let nothing = office.dir.path().join("empty.json");
    let (ok, said) = run(&office.second, &["setup", "save", &at(&nothing)]);
    assert!(!ok, "{said}");
    assert!(said.contains("nothing set up"), "{said}");
    // And it says what would put something in it.
    assert!(
        said.contains("calibrate") || said.contains("--save-as"),
        "{said}"
    );
    assert!(!nothing.exists(), "an empty file was written anyway");
}

/// A file that is not one of these is refused by name, rather than half-read
/// into a setup nobody asked for.
#[test]
fn something_that_is_not_a_setup_file_is_refused_by_name() {
    let office = Office::new();
    let rubbish = office.dir.path().join("notes.txt");
    std::fs::write(&rubbish, "just some notes").expect("it should write");

    let (ok, said) = run(&office.second, &["setup", "use", &at(&rubbish)]);
    assert!(!ok, "{said}");
    assert!(said.contains("not an Onionskin setup file"), "{said}");
    assert!(said.contains("setup save"), "{said}");

    let missing = office.dir.path().join("nowhere.json");
    let (ok, said) = run(&office.second, &["setup", "use", &at(&missing)]);
    assert!(!ok, "{said}");
    assert!(said.contains("nowhere.json"), "{said}");
}
