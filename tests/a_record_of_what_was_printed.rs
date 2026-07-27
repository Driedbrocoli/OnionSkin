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
