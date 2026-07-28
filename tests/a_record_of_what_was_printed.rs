//! What was added to which sheet, and when.
//!
//! `onionskin history` says of itself that "every delta Onionskin writes goes
//! in here, so a sheet in a filing cabinet can be asked what was added to it
//! and when". That record is what makes it possible to answer a question about
//! a piece of paper that left the building months ago, and it is what the
//! "you have printed this one already" warning is built on.
//!
//! `cover` did not write to it. Of every delta the program makes, that is the
//! one most worth being able to look up: it is the one that blacks out a salary
//! or an address on a sheet somebody is about to hand over. Nothing tested it,
//! because nothing tested `cover` at all.

use std::path::{Path, PathBuf};
use std::process::Command;

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_onionskin"))
}

struct Run {
    ok: bool,
    stdout: String,
    stderr: String,
}

impl Run {
    fn said(&self) -> String {
        format!("{}{}", self.stdout, self.stderr)
    }
}

/// With a home of its own, so a test never touches the history of whoever is
/// running it — and so the record starts empty and every entry is this test's.
fn run(home: &Path, args: &[&str]) -> Run {
    let output = Command::new(binary())
        .args(args)
        .env("ONIONSKIN_HOME", home)
        .output()
        .expect("the binary should run");
    Run {
        ok: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// A sheet with something on it worth covering up.
fn a_sheet_with_a_salary_on_it(home: &Path, dir: &Path) -> PathBuf {
    let document = dir.join("staff.osk").to_string_lossy().into_owned();
    assert!(run(home, &["new", &document, "--page", "a4"]).ok);
    for (at, words) in [
        ("20,40", "Name: A. Smith"),
        ("20,60", "Salary: 48000"),
        ("20,80", "Department: Accounts"),
    ] {
        let placed = format!("{at}:{words}");
        let written = run(home, &["write", &document, "--at", &placed, "--size", "12"]);
        assert!(written.ok, "{}", written.said());
    }
    let pdf = dir.join("staff.pdf");
    let printed = run(home, &["print", &document, "-o", &pdf.to_string_lossy()]);
    assert!(printed.ok, "{}", printed.said());
    pdf
}

/// The record starts empty and says so plainly, rather than showing a heading
/// with nothing under it.
#[test]
fn an_empty_record_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let said = run(&home, &["history"]).said();
    assert!(said.contains("Nothing written yet"), "{said}");
}

/// The regression: a cover is a delta, and it was the one delta that left no
/// trace. A sheet that went out of the building with a salary blacked out is
/// exactly the sheet somebody asks about later.
#[test]
fn a_cover_is_recorded_like_any_other_delta() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let sheet = a_sheet_with_a_salary_on_it(&home, dir.path());
    let covered = dir.path().join("covered.pdf");

    let hidden = run(
        &home,
        &[
            "cover",
            &sheet.to_string_lossy(),
            "--over",
            "18,55:80x10",
            "-o",
            &covered.to_string_lossy(),
        ],
    );
    assert!(hidden.ok, "{}", hidden.said());

    let said = run(&home, &["history"]).said();
    assert!(
        !said.contains("Nothing written yet"),
        "covering something left no record at all: {said}"
    );
    assert!(
        said.contains("staff.pdf"),
        "the record does not name the sheet that was covered: {said}"
    );
    assert!(
        said.contains("covered.pdf"),
        "the record does not name the delta that covers it: {said}"
    );
}

/// The duplicate warning is built on that record, so it did not work for covers
/// either. Printing the same cover onto a sheet that already has it lays the
/// toner down twice, which is worth a word.
#[test]
fn the_same_cover_run_twice_is_recognised_as_the_same() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let sheet = a_sheet_with_a_salary_on_it(&home, dir.path());

    let once = |out: &Path| {
        run(
            &home,
            &[
                "cover",
                &sheet.to_string_lossy(),
                "--over",
                "18,55:80x10",
                "-o",
                &out.to_string_lossy(),
            ],
        )
    };

    let first = once(&dir.path().join("first.pdf"));
    assert!(first.ok, "{}", first.said());
    assert!(
        !first.said().contains("same delta"),
        "the first run thought it had seen itself: {}",
        first.said()
    );

    let again = once(&dir.path().join("second.pdf"));
    assert!(again.ok, "{}", again.said());
    assert!(
        again.said().contains("same delta"),
        "covering the same area twice was not recognised: {}",
        again.said()
    );
}

/// Covering a different area is a different delta, and must not be mistaken for
/// one already printed — a warning that cries wolf is one people learn to skip.
#[test]
fn covering_somewhere_else_is_not_mistaken_for_the_same_delta() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let sheet = a_sheet_with_a_salary_on_it(&home, dir.path());

    let salary = run(
        &home,
        &[
            "cover",
            &sheet.to_string_lossy(),
            "--over",
            "18,55:80x10",
            "-o",
            &dir.path().join("salary.pdf").to_string_lossy(),
        ],
    );
    assert!(salary.ok, "{}", salary.said());

    let department = run(
        &home,
        &[
            "cover",
            &sheet.to_string_lossy(),
            "--over",
            "18,75:80x10",
            "-o",
            &dir.path().join("department.pdf").to_string_lossy(),
        ],
    );
    assert!(department.ok, "{}", department.said());
    assert!(
        !department.said().contains("same delta"),
        "covering a different line was called a repeat: {}",
        department.said()
    );

    // Both are in the record, because both are sheets that went out.
    let said = run(&home, &["history"]).said();
    assert!(said.contains("salary.pdf"), "{said}");
    assert!(said.contains("department.pdf"), "{said}");
}

/// Forgetting is offered and works, because a record of what was on people's
/// paper is somebody's to delete.
#[test]
fn the_record_can_be_forgotten() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let sheet = a_sheet_with_a_salary_on_it(&home, dir.path());
    assert!(
        run(
            &home,
            &[
                "cover",
                &sheet.to_string_lossy(),
                "--over",
                "18,55:80x10",
                "-o",
                &dir.path().join("c.pdf").to_string_lossy(),
            ],
        )
        .ok
    );
    assert!(run(&home, &["history"]).said().contains("staff.pdf"));

    let forgotten = run(&home, &["history", "--forget"]);
    assert!(forgotten.ok, "{}", forgotten.said());
    let said = run(&home, &["history"]).said();
    assert!(
        said.contains("Nothing written yet"),
        "the record survived being forgotten: {said}"
    );
}

/// A batch is a delta too, and the expensive one to print twice: two hundred
/// certificates is two hundred sheets of stock that cannot be un-printed. It
/// was not recorded either, so the warning that exists for exactly this never
/// fired for the case it matters most in.
#[test]
fn a_batch_is_recorded_and_a_repeat_of_it_is_recognised() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let sheet = a_sheet_with_a_salary_on_it(&home, dir.path());
    let list = dir.path().join("people.csv");
    std::fs::write(&list, "name\nAlice\nBob\n").unwrap();

    let once = |out: &Path| {
        run(
            &home,
            &[
                "batch",
                &sheet.to_string_lossy(),
                "--from",
                &list.to_string_lossy(),
                "--at",
                "120,40:{name}",
                "-o",
                &out.to_string_lossy(),
            ],
        )
    };

    let first = once(&dir.path().join("stack.pdf"));
    assert!(first.ok, "{}", first.said());
    let said = run(&home, &["history"]).said();
    assert!(
        said.contains("stack.pdf"),
        "a stack of two hundred certificates left no record: {said}"
    );

    let again = once(&dir.path().join("stack-again.pdf"));
    assert!(again.ok, "{}", again.said());
    assert!(
        again.said().contains("same delta"),
        "printing the same stack twice was not recognised: {}",
        again.said()
    );
}

/// The instructions must not say "blank sheets". The delta carries only the
/// additions, so somebody who reads that literally puts plain paper in the tray
/// and gets two hundred sheets of floating names and no certificate.
#[test]
fn the_stack_instructions_do_not_send_plain_paper_through_the_printer() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let sheet = a_sheet_with_a_salary_on_it(&home, dir.path());
    let list = dir.path().join("people.csv");
    std::fs::write(&list, "name\nAlice\nBob\n").unwrap();

    let made = run(
        &home,
        &[
            "batch",
            &sheet.to_string_lossy(),
            "--from",
            &list.to_string_lossy(),
            "--at",
            "120,40:{name}",
            "-o",
            &dir.path().join("stack.pdf").to_string_lossy(),
        ],
    );
    assert!(made.ok, "{}", made.said());
    let said = made.said();
    assert!(
        said.contains("2 printed copies of"),
        "the stack was not described as printed copies of the sheet: {said}"
    );
    assert!(
        said.contains("staff.pdf"),
        "the instructions do not name the document to put in the tray: {said}"
    );
    assert!(
        said.contains("Not plain paper"),
        "nothing warns against feeding plain paper: {said}"
    );
}

/// `print --delta` writes a delta too — the one for Onionskin's own documents,
/// where the sheet was printed last week and only what was added since goes
/// back through the printer. It was the third place the promise was not kept.
#[test]
fn printing_only_what_was_added_is_recorded_but_printing_the_whole_thing_is_not() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let document = dir.path().join("notes.osk").to_string_lossy().into_owned();
    assert!(run(&home, &["new", &document, "--page", "a4"]).ok);
    assert!(run(&home, &["write", &document, "--at", "20,40:First draft"]).ok);

    // Printed whole and noted as on paper. A whole sheet is not an addition to
    // anything, so it must stay out of a record of what was added to sheets.
    let whole = dir.path().join("whole.pdf");
    let printed = run(
        &home,
        &[
            "print",
            &document,
            "-o",
            &whole.to_string_lossy(),
            "--printed",
        ],
    );
    assert!(printed.ok, "{}", printed.said());
    assert!(
        run(&home, &["history"])
            .said()
            .contains("Nothing written yet"),
        "printing the whole document was filed as an addition to a sheet"
    );

    // Then something is added, and only that goes back through the printer.
    assert!(run(&home, &["write", &document, "--at", "20,60:Added after"]).ok);
    let delta = dir.path().join("since.pdf");
    let only = run(
        &home,
        &[
            "print",
            &document,
            "--delta",
            "-o",
            &delta.to_string_lossy(),
        ],
    );
    assert!(only.ok, "{}", only.said());

    let said = run(&home, &["history"]).said();
    assert!(
        said.contains("since.pdf"),
        "the delta left no record: {said}"
    );
    assert!(
        said.contains("notes.osk"),
        "the record does not name the document it came from: {said}"
    );
}

/// `verify` and `proof` both want a delta written minutes ago, often into a
/// scratch folder under a generated name nobody chose. The record already
/// knows what it was, so `last` means it.
#[test]
fn the_delta_written_last_can_be_named_as_last() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let sheet = a_sheet_with_a_salary_on_it(&home, dir.path());

    // Nothing written yet: `last` is a refusal that says what to do.
    let nothing = run(
        &home,
        &[
            "proof",
            &sheet.to_string_lossy(),
            "--delta",
            "last",
            "-o",
            &dir.path().join("p.pdf").to_string_lossy(),
        ],
    );
    assert!(!nothing.ok, "{}", nothing.said());
    assert!(
        nothing.said().contains("no delta to use"),
        "{}",
        nothing.said()
    );

    // Then one is written, and `last` finds it.
    let covered = dir.path().join("covered.pdf");
    assert!(
        run(
            &home,
            &[
                "cover",
                &sheet.to_string_lossy(),
                "--over",
                "18,55:80x10",
                "-o",
                &covered.to_string_lossy()
            ],
        )
        .ok
    );

    let proof = dir.path().join("proof.pdf");
    let made = run(
        &home,
        &[
            "proof",
            &sheet.to_string_lossy(),
            "--delta",
            "last",
            "-o",
            &proof.to_string_lossy(),
        ],
    );
    assert!(made.ok, "`last` did not find the delta: {}", made.said());
    assert!(proof.is_file(), "{}", made.said());
}

/// A delta that has been tidied away is not named — a path to a file that is
/// no longer there is worse than saying there is nothing.
#[test]
fn a_delta_that_is_gone_is_not_offered_as_last() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let sheet = a_sheet_with_a_salary_on_it(&home, dir.path());
    let covered = dir.path().join("covered.pdf");
    assert!(
        run(
            &home,
            &[
                "cover",
                &sheet.to_string_lossy(),
                "--over",
                "18,55:80x10",
                "-o",
                &covered.to_string_lossy()
            ],
        )
        .ok
    );
    std::fs::remove_file(&covered).unwrap();

    let gone = run(
        &home,
        &[
            "proof",
            &sheet.to_string_lossy(),
            "--delta",
            "last",
            "-o",
            &dir.path().join("p.pdf").to_string_lossy(),
        ],
    );
    assert!(
        !gone.ok,
        "a delta that is no longer on disk was offered: {}",
        gone.said()
    );
}
