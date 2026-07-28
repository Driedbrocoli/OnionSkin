//! Checking two hundred sheets, through the commands somebody actually types.
//!
//! `verify` answers "did this sheet come out right", which is the right
//! question asked once. A run of certificates is one chance per sheet for the
//! paper to go in crooked, and nobody scans two hundred sheets one at a time to
//! find out — so nobody finds out, and the drifted ones go out in the post.
//!
//! A feeder gives the whole stack back as one PDF, so the run can be checked
//! the way it was printed. These tests build a run with a known fault in it and
//! insist that the fault is the sheet that is named, because a check that says
//! "something is wrong somewhere in the stack" is no better than no check.

use std::path::{Path, PathBuf};
use std::process::Command;

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_onionskin"))
}

struct Run {
    ok: bool,
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

impl Run {
    fn said(&self) -> String {
        format!("{}{}", self.stdout, self.stderr)
    }
}

/// With a home of its own, so a test never touches the profiles or the history
/// of whoever is running it.
fn run(home: &Path, args: &[&str]) -> Run {
    let output = Command::new(binary())
        .args(args)
        .env("ONIONSKIN_HOME", home)
        .output()
        .expect("the binary should run");
    Run {
        ok: output.status.success(),
        code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

fn at(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// A blank certificate, as a PDF to print onto.
fn a_blank(home: &Path, dir: &Path) -> PathBuf {
    let document = dir.join("cert.osk");
    let printed = dir.join("cert.pdf");
    let made = run(home, &["new", &at(&document), "--page", "a4"]);
    assert!(made.ok, "{}", made.said());
    // Something on the sheet, so it is a page rather than nothing: a run of
    // blank paper has no edges for a scan to register against.
    let written = run(
        home,
        &[
            "write",
            &at(&document),
            "--at",
            "25,30:CERTIFICATE OF ATTENDANCE",
            "--at",
            "25,120:Signed ____________________",
        ],
    );
    assert!(written.ok, "{}", written.said());
    let out = run(home, &["print", &at(&document), "-o", &at(&printed)]);
    assert!(out.ok, "{}", out.said());
    printed
}

/// One page of delta: the words that go onto one sheet, at a given height.
fn a_page_of_delta(
    home: &Path,
    dir: &Path,
    blank: &Path,
    name: &str,
    words: &str,
    y_mm: f64,
) -> PathBuf {
    let document = dir.join(format!("{name}.osk"));
    let filled = dir.join(format!("{name}-filled.pdf"));
    let delta = dir.join(format!("{name}.pdf"));
    // The same blank, written on.
    let made = run(home, &["new", &at(&document), "--page", "a4"]);
    assert!(made.ok, "{}", made.said());
    for placement in [
        "25,30:CERTIFICATE OF ATTENDANCE".to_string(),
        "25,120:Signed ____________________".to_string(),
        format!("40,{y_mm}:{words}"),
    ] {
        let written = run(home, &["write", &at(&document), "--at", &placement]);
        assert!(written.ok, "{}", written.said());
    }
    let printed = run(home, &["print", &at(&document), "-o", &at(&filled)]);
    assert!(printed.ok, "{}", printed.said());
    let made = run(
        home,
        &["delta", &at(blank), &at(&filled), "-o", &at(&delta)],
    );
    assert!(made.ok, "{}", made.said());
    delta
}

fn join(home: &Path, out: &Path, parts: &[PathBuf]) -> PathBuf {
    let mut args: Vec<String> = vec!["join".to_string()];
    args.extend(parts.iter().map(|part| at(part)));
    args.push("-o".to_string());
    args.push(at(out));
    let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
    let joined = run(home, &borrowed);
    assert!(joined.ok, "{}", joined.said());
    out.to_path_buf()
}

/// The stack as it comes back from the feeder: each blank sheet with what was
/// really printed onto it.
fn the_stack_that_came_back(home: &Path, dir: &Path, blank: &Path, printed: &Path) -> PathBuf {
    let pages = onionskin::recipe::pages_in(printed).expect("the delta should open");
    let blanks = join(
        home,
        &dir.join("blanks.pdf"),
        &vec![blank.to_path_buf(); pages],
    );
    let stack = dir.join("stack.pdf");
    let laid = run(
        home,
        &[
            "proof",
            &at(&blanks),
            "--delta",
            &at(printed),
            "-o",
            &at(&stack),
        ],
    );
    assert!(laid.ok, "{}", laid.said());
    stack
}

/// A run of four, one of which went through four millimetres low, checked
/// against what was asked for.
///
/// The point of the whole feature: it has to name sheet three. A report that
/// says the stack has a problem somewhere is a report that costs somebody the
/// afternoon it was meant to save.
#[test]
fn the_one_sheet_that_drifted_is_the_one_that_is_named() {
    let home = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let blank = a_blank(home.path(), dir.path());

    // Every page puts its words somewhere different — 55, 65, 75, 85 mm down.
    // That is what makes this a test of the run rather than of one page: a
    // check that held every sheet against page one would find page one's marks
    // missing from sheets two, three and four, and say so.
    let asked: Vec<PathBuf> = (1..=4)
        .map(|n| {
            a_page_of_delta(
                home.path(),
                dir.path(),
                &blank,
                &format!("asked{n}"),
                &format!("Person {n}"),
                45.0 + 10.0 * n as f64,
            )
        })
        .collect();
    // What really came out: sheet three fed in four millimetres out.
    let came_out: Vec<PathBuf> = (1..=4)
        .map(|n| {
            a_page_of_delta(
                home.path(),
                dir.path(),
                &blank,
                &format!("out{n}"),
                &format!("Person {n}"),
                45.0 + 10.0 * n as f64 + if n == 3 { 4.0 } else { 0.0 },
            )
        })
        .collect();

    let wanted = join(home.path(), &dir.path().join("asked.pdf"), &asked);
    let happened = join(home.path(), &dir.path().join("came-out.pdf"), &came_out);
    let stack = the_stack_that_came_back(home.path(), dir.path(), &blank, &happened);

    let checked = run(
        home.path(),
        &["verify", &at(&stack), "--delta", &at(&wanted)],
    );
    let said = checked.said();
    assert!(said.contains("4 sheet(s)"), "{said}");
    assert!(said.contains("1 of 4 sheet(s) drifted: 3."), "{said}");
    // Named, and named alone: the other three have to read as fine, or the
    // report is a wall of noise nobody acts on.
    assert!(said.contains("sheet   1  ✓"), "{said}");
    assert!(said.contains("sheet   2  ✓"), "{said}");
    assert!(said.contains("sheet   3  ✗"), "{said}");
    assert!(said.contains("sheet   4  ✓"), "{said}");
    // And how far out, so somebody can tell four millimetres from a tenth.
    assert!(said.contains(" mm out"), "{said}");
    // Exit 2, so a script feeding a stack stops rather than posting them.
    assert_eq!(checked.code, Some(2), "{said}");
}

/// A run that is entirely right says so plainly, and exits nought.
///
/// Worth its own test: a check that can only report trouble is a check people
/// stop running, and the answer "all two hundred are fine" is the one they are
/// paying the scanner for.
#[test]
fn a_run_that_came_out_right_says_so_and_lets_a_script_carry_on() {
    let home = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let blank = a_blank(home.path(), dir.path());
    let pages: Vec<PathBuf> = (1..=3)
        .map(|n| {
            a_page_of_delta(
                home.path(),
                dir.path(),
                &blank,
                &format!("p{n}"),
                &format!("Person {n}"),
                45.0 + 10.0 * n as f64,
            )
        })
        .collect();
    let delta = join(home.path(), &dir.path().join("delta.pdf"), &pages);
    let stack = the_stack_that_came_back(home.path(), dir.path(), &blank, &delta);

    let checked = run(
        home.path(),
        &["verify", &at(&stack), "--delta", &at(&delta)],
    );
    let said = checked.said();
    assert!(said.contains("All 3 sheet(s) came out right"), "{said}");
    assert!(!said.contains("drifted"), "{said}");
    assert_eq!(checked.code, Some(0), "{said}");
}

/// `--first 2` stops after two, so the shape of a run shows before two hundred
/// sheets have been through the scanner.
#[test]
fn a_run_can_be_looked_at_before_the_whole_stack_is_scanned() {
    let home = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let blank = a_blank(home.path(), dir.path());
    let pages: Vec<PathBuf> = (1..=4)
        .map(|n| {
            a_page_of_delta(
                home.path(),
                dir.path(),
                &blank,
                &format!("p{n}"),
                &format!("Person {n}"),
                45.0 + 10.0 * n as f64,
            )
        })
        .collect();
    let delta = join(home.path(), &dir.path().join("delta.pdf"), &pages);
    let stack = the_stack_that_came_back(home.path(), dir.path(), &blank, &delta);

    let checked = run(
        home.path(),
        &[
            "verify",
            &at(&stack),
            "--delta",
            &at(&delta),
            "--first",
            "2",
        ],
    );
    let said = checked.said();
    assert!(said.contains("2 sheet(s), against"), "{said}");
    assert!(said.contains("sheet   2"), "{said}");
    assert!(
        !said.contains("sheet   3"),
        "it did not stop at two: {said}"
    );
}

/// One page of delta printed onto every sheet of a run — a paid stamp, a
/// signature — is an ordinary thing to check, and is not a mismatch.
#[test]
fn one_page_of_delta_over_a_whole_run_is_checked_against_every_sheet() {
    let home = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let blank = a_blank(home.path(), dir.path());
    let stamp = a_page_of_delta(home.path(), dir.path(), &blank, "stamp", "PAID", 60.0);
    let three = join(
        home.path(),
        &dir.path().join("three.pdf"),
        &vec![stamp.clone(); 3],
    );
    let stack = the_stack_that_came_back(home.path(), dir.path(), &blank, &three);

    let checked = run(
        home.path(),
        &["verify", &at(&stack), "--delta", &at(&stamp)],
    );
    let said = checked.said();
    assert!(said.contains("3 sheet(s), against"), "{said}");
    assert!(said.contains("All 3 sheet(s) came out right"), "{said}");
}

/// Anything between "a page each" and "one page for all of them" is somebody
/// having scanned half the stack. Checked against the wrong pages it would
/// report a run of nonsense, so it is refused with both counts named.
#[test]
fn half_a_stack_is_refused_rather_than_checked_against_the_wrong_pages() {
    let home = tempfile::tempdir().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let blank = a_blank(home.path(), dir.path());
    let pages: Vec<PathBuf> = (1..=4)
        .map(|n| {
            a_page_of_delta(
                home.path(),
                dir.path(),
                &blank,
                &format!("p{n}"),
                &format!("Person {n}"),
                45.0 + 10.0 * n as f64,
            )
        })
        .collect();
    let all_four = join(home.path(), &dir.path().join("four.pdf"), &pages);
    let stack = the_stack_that_came_back(home.path(), dir.path(), &blank, &all_four);
    let only_two = join(home.path(), &dir.path().join("two.pdf"), &pages[..2]);

    let checked = run(
        home.path(),
        &["verify", &at(&stack), "--delta", &at(&only_two)],
    );
    let said = checked.said();
    assert!(!checked.ok, "{said}");
    assert!(said.contains("4 sheet(s)"), "{said}");
    assert!(said.contains("2 page(s)"), "{said}");
    assert!(
        said.contains("single page that went onto every sheet"),
        "{said}"
    );
}
