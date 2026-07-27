//! Merging deltas, end to end, through the command somebody actually types.
//!
//! A day's work on one document arrives as more than one delta: the stamp is a
//! saved job, the signature is a picture, the reference came out of a
//! spreadsheet. Printing three of them means feeding the same sheet through the
//! printer three times, and each pass is a chance to skew it, jam it, or lose
//! it — on a sheet that already has the letterhead on it.
//!
//! The unit tests check the merged PDF's structure. This one checks the thing
//! that actually matters: that after `onionskin merge`, every delta's ink is on
//! the paper, in the place its own delta put it. It measures the merged file
//! the same way autocalibration measures a delta — by rendering it and finding
//! the ink — which is as close as a test gets to holding the sheet up.

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
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

/// A delta carrying one line, made the way a person makes one: write it onto a
/// document, print both versions, and take the difference.
fn a_delta(home: &Path, dir: &Path, name: &str, words: &str, at: &str, font: &str) -> PathBuf {
    let document = dir.join(format!("{name}.osk"));
    let before = dir.join(format!("{name}-before.pdf"));
    let after = dir.join(format!("{name}-after.pdf"));
    let delta = dir.join(format!("{name}.pdf"));

    let document = document.to_string_lossy().into_owned();
    let before_s = before.to_string_lossy().into_owned();
    let after_s = after.to_string_lossy().into_owned();

    // A mark in the corner of the "before", so the page is not empty and the
    // two renderings line up the way two printings of one sheet would.
    let made = run(home, &["new", &document, "--page", "a4"]);
    assert!(made.ok, "{}", made.said());
    let marked = run(
        home,
        &["write", &document, "--at", "5,290:.", "--size", "6"],
    );
    assert!(marked.ok, "{}", marked.said());
    let printed = run(home, &["print", &document, "-o", &before_s]);
    assert!(printed.ok, "{}", printed.said());

    let written = run(
        home,
        &[
            "write",
            &document,
            "--at",
            &format!("{at}:{words}"),
            "--size",
            "12",
            "--font",
            font,
        ],
    );
    assert!(written.ok, "{}", written.said());
    let printed = run(home, &["print", &document, "-o", &after_s]);
    assert!(printed.ok, "{}", printed.said());

    let made = run(
        home,
        &["delta", &before_s, &after_s, "-o", &delta.to_string_lossy()],
    );
    assert!(made.ok, "{}", made.said());
    assert!(delta.is_file(), "no delta was written: {}", made.said());
    delta
}

/// Where the ink is on a delta, in millimetres down and across the paper.
///
/// This is the program's own measurement — the one autocalibration takes off
/// a delta to know where it asked for marks — so it answers the question the
/// printer will answer: is the ink on the paper, and is it in the right place.
fn where_the_ink_is(delta: &Path) -> Vec<(f64, f64)> {
    onionskin::calibrate::marks_on_delta(delta)
        .expect("the merged delta should render")
        .into_iter()
        .map(|mark| mark.centre_mm)
        .collect()
}

/// Is there ink within `slack` millimetres of where a line was asked for?
fn ink_near(marks: &[(f64, f64)], want: (f64, f64), slack: f64) -> bool {
    marks
        .iter()
        .any(|(x, y)| (x - want.0).hypot(y - want.1) <= slack)
}

/// The whole point, from the command line: two deltas, one sheet, one pass.
#[test]
fn two_deltas_merged_put_everything_on_one_sheet() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let stamp = a_delta(&home, dir.path(), "stamp", "PAID", "140,40", "helvetica");
    let sign = a_delta(&home, dir.path(), "sign", "SIGNED", "30,200", "helvetica");
    let both = dir.path().join("both.pdf");

    let merged = run(
        &home,
        &[
            "merge",
            &stamp.to_string_lossy(),
            &sign.to_string_lossy(),
            "-o",
            &both.to_string_lossy(),
        ],
    );
    assert!(merged.ok, "{}", merged.said());
    assert!(both.is_file(), "{}", merged.said());
    assert!(merged.stdout.contains("A4"), "{}", merged.said());
    assert!(merged.stdout.contains("one pass"), "{}", merged.said());

    // Both deltas' ink is on the one page, each where its own delta put it.
    // The text is placed from its baseline's left end, so the middle of the
    // word sits up and to the right of the point it was asked for.
    let ink = where_the_ink_is(&both);
    assert!(ink_near(&ink, (140.0, 40.0), 25.0), "no PAID: {ink:?}");
    assert!(ink_near(&ink, (30.0, 200.0), 25.0), "no SIGNED: {ink:?}");

    // And each source delta on its own has only its own.
    let only_stamp = where_the_ink_is(&stamp);
    assert!(
        !ink_near(&only_stamp, (30.0, 200.0), 25.0),
        "{only_stamp:?}"
    );
    let only_sign = where_the_ink_is(&sign);
    assert!(!ink_near(&only_sign, (140.0, 40.0), 25.0), "{only_sign:?}");
}

/// Merging deltas for two different sheets of paper would print one of them off
/// the edge. Asked before the paper goes in, and nothing written.
#[test]
fn deltas_for_different_paper_are_refused_before_anything_is_written() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let a4 = a_delta(&home, dir.path(), "a4", "EUROPE", "40,40", "helvetica");

    // The same thing again, on Letter.
    let document = dir.path().join("us.osk").to_string_lossy().into_owned();
    let before = dir
        .path()
        .join("us-before.pdf")
        .to_string_lossy()
        .into_owned();
    let after = dir
        .path()
        .join("us-after.pdf")
        .to_string_lossy()
        .into_owned();
    let letter = dir.path().join("us.pdf");
    assert!(run(&home, &["new", &document, "--page", "letter"]).ok);
    assert!(
        run(
            &home,
            &["write", &document, "--at", "5,270:.", "--size", "6"]
        )
        .ok
    );
    assert!(run(&home, &["print", &document, "-o", &before]).ok);
    assert!(
        run(
            &home,
            &["write", &document, "--at", "40,40:AMERICA", "--size", "12"]
        )
        .ok
    );
    assert!(run(&home, &["print", &document, "-o", &after]).ok);
    assert!(
        run(
            &home,
            &["delta", &before, &after, "-o", &letter.to_string_lossy()]
        )
        .ok
    );

    let out = dir.path().join("mixed.pdf");
    let refused = run(
        &home,
        &[
            "merge",
            &a4.to_string_lossy(),
            &letter.to_string_lossy(),
            "-o",
            &out.to_string_lossy(),
        ],
    );
    assert!(!refused.ok, "{}", refused.said());
    assert!(refused.stderr.contains("A4"), "{}", refused.said());
    assert!(refused.stderr.contains("Letter"), "{}", refused.said());
    assert!(!out.exists(), "a refused merge still wrote a file");
}

/// The same delta twice puts every letter down twice in the same place. Said
/// out loud, before the paper goes in — not refused, because the file is fine.
#[test]
fn the_same_delta_twice_is_said_out_loud() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let stamp = a_delta(&home, dir.path(), "stamp", "PAID", "140,40", "helvetica");
    let copy = dir.path().join("copy.pdf");
    std::fs::copy(&stamp, &copy).unwrap();
    let out = dir.path().join("twice.pdf");

    let merged = run(
        &home,
        &[
            "merge",
            &stamp.to_string_lossy(),
            &copy.to_string_lossy(),
            "-o",
            &out.to_string_lossy(),
        ],
    );
    assert!(merged.ok, "{}", merged.said());
    assert!(merged.stdout.contains("printed twice"), "{}", merged.said());
}

/// A merge is a delta like any other, so the record knows about it — and knows
/// what it was made of, which is the question somebody asks months later.
#[test]
fn a_merge_goes_into_the_record_saying_what_it_was_made_of() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let stamp = a_delta(&home, dir.path(), "stamp", "PAID", "140,40", "helvetica");
    let sign = a_delta(&home, dir.path(), "sign", "SIGNED", "30,200", "helvetica");
    let out = dir.path().join("both.pdf");
    assert!(
        run(
            &home,
            &[
                "merge",
                &stamp.to_string_lossy(),
                &sign.to_string_lossy(),
                "-o",
                &out.to_string_lossy(),
            ],
        )
        .ok
    );

    let history = run(&home, &["history"]);
    assert!(history.ok, "{}", history.said());
    assert!(
        history.stdout.contains("stamp.pdf + sign.pdf"),
        "{}",
        history.said()
    );
    assert!(history.stdout.contains("both.pdf"), "{}", history.said());
}

/// Merging is deterministic, so the same merge written twice is recognised as
/// the same delta — by the machinery that already catches a delta printed
/// twice onto one sheet.
#[test]
fn the_same_merge_written_twice_is_recognised() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let stamp = a_delta(&home, dir.path(), "stamp", "PAID", "140,40", "helvetica");
    let sign = a_delta(&home, dir.path(), "sign", "SIGNED", "30,200", "helvetica");

    let merge_to = |name: &str| {
        run(
            &home,
            &[
                "merge",
                &stamp.to_string_lossy(),
                &sign.to_string_lossy(),
                "-o",
                &dir.path().join(name).to_string_lossy(),
            ],
        )
    };
    assert!(merge_to("first.pdf").ok);
    let again = merge_to("second.pdf");
    assert!(again.ok, "{}", again.said());
    assert!(
        again.stdout.contains("same delta you wrote"),
        "{}",
        again.said()
    );

    // Deterministic means byte for byte, which is what makes that work.
    let first = std::fs::read(dir.path().join("first.pdf")).unwrap();
    let second = std::fs::read(dir.path().join("second.pdf")).unwrap();
    assert_eq!(first, second, "the same merge came out differently twice");
}

/// One file is not a merge, and saying so beats writing a copy under a new
/// name and calling it done.
#[test]
fn merging_one_file_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let stamp = a_delta(&home, dir.path(), "stamp", "PAID", "140,40", "helvetica");
    let out = dir.path().join("out.pdf");

    let refused = run(
        &home,
        &[
            "merge",
            &stamp.to_string_lossy(),
            "-o",
            &out.to_string_lossy(),
        ],
    );
    assert!(!refused.ok, "{}", refused.said());
    assert!(!out.exists());
}
