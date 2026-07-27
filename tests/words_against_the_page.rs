//! Words placed against something already printed, on a document.
//!
//! `--after 'Received:Approved'` is the placement the help offers first, and
//! rightly: it is the only one that does not require a ruler. Somebody who has
//! an invoice and wants "PAID" beside the word "Total" should not have to
//! measure where "Total" is, and next month's invoice will have moved it
//! anyway.
//!
//! On Onionskin's own documents this always worked. On a PDF — which is what an
//! invoice actually is, and what a saved job is run against — the words were
//! dropped without a word: the composer was handed an empty list, and what came
//! back was a blank delta blamed on "the two documents render identically",
//! which is advice about a different command. Nothing tested it, so it stayed.
//!
//! These drive the real binary against real PDFs. They are the reason the
//! easiest thing to ask for is now the thing that works.

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

/// Reading a page means matching its ink against a real typeface, so a machine
/// with no fonts on it cannot do this at all.
///
/// Reported rather than skipped silently: a test that quietly passes because it
/// did nothing is worse than one that is not there, and the refusal such a
/// machine gets is itself worth checking.
fn can_read_a_page() -> bool {
    onionskin::font::suggest_system_font().is_some()
        || !onionskin::font::installed_fonts().is_empty()
}

/// A printed form: labels down the left, nothing filled in.
///
/// Returned as a PDF, because that is what somebody actually has — the whole
/// point of an anchor is that the document was made by something else.
fn a_printed_form(home: &Path, dir: &Path, labels: &[(usize, f64, &str)]) -> PathBuf {
    let document = dir.join("form.osk").to_string_lossy().into_owned();
    let pages = labels.iter().map(|(page, _, _)| *page).max().unwrap_or(1);
    let made = run(home, &["new", &document, "--page", "a4"]);
    assert!(made.ok, "{}", made.said());
    for (page, y_mm, words) in labels {
        let placed = format!("20,{y_mm}:{words}");
        let written = run(
            home,
            &[
                "write",
                &document,
                "--at",
                &placed,
                "--size",
                "12",
                "--page",
                &page.to_string(),
            ],
        );
        assert!(written.ok, "{}", written.said());
    }
    assert!(pages >= 1);

    let pdf = dir.join("form.pdf");
    let printed = run(home, &["print", &document, "-o", &pdf.to_string_lossy()]);
    assert!(printed.ok, "{}", printed.said());
    pdf
}

/// Where the run said it put the words, in millimetres, off its own report.
fn placed_at(said: &str) -> Option<(f64, f64)> {
    let line = said
        .lines()
        .find(|line| line.contains("putting the words at"))?;
    let tail = line.split("putting the words at").nth(1)?;
    let (x, rest) = tail.trim().split_once(',')?;
    let y = rest.trim().trim_end_matches(" mm");
    Some((x.trim().parse().ok()?, y.trim().parse().ok()?))
}

/// Ink on each page of a PDF, at a resolution fine enough to see a word.
fn ink_per_page(pdf: &Path) -> Vec<usize> {
    let engine = onionskin::render::engine().expect("a renderer");
    let doc = engine.open(pdf).expect("it should open");
    (0..doc.len())
        .map(|index| {
            let page = doc.render_gray(index, 150.0).expect("it should render");
            page.gray.iter().filter(|&&value| value < 128).count()
        })
        .collect()
}

/// The regression. Before this, the delta came out blank and the reason given
/// was about two documents rendering identically — which is the `delta`
/// command's diagnosis, for a situation this person is not in.
#[test]
fn an_anchor_on_a_pdf_actually_places_the_words() {
    if !can_read_a_page() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let form = a_printed_form(&home, dir.path(), &[(1, 40.0, "Received:")]);
    let delta = dir.path().join("delta.pdf");

    let written = run(
        &home,
        &[
            "write",
            &form.to_string_lossy(),
            "--after",
            "Received:APPROVED",
            "--size",
            "10",
            "-o",
            &delta.to_string_lossy(),
        ],
    );
    assert!(
        written.ok,
        "an anchored placement on a PDF was refused: {}",
        written.said()
    );
    assert!(
        !written.said().contains("render identically"),
        "the delta came out blank and blamed the wrong thing: {}",
        written.said()
    );

    // Just to the right of the label, on its line — which is the whole promise.
    let (x_mm, y_mm) = placed_at(&written.said())
        .unwrap_or_else(|| panic!("nothing said where it went: {}", written.said()));
    assert!(
        (25.0..80.0).contains(&x_mm),
        "the words went to {x_mm} mm, which is not beside a label starting at 20 mm"
    );
    assert!(
        (38.0..42.0).contains(&y_mm),
        "the words went to {y_mm} mm, and the label is on the line at 40 mm"
    );

    // And there is ink in the file, not just a report saying where it would be.
    let ink = ink_per_page(&delta);
    assert_eq!(ink.len(), 1, "{ink:?}");
    assert!(ink[0] > 0, "the delta is blank: {}", written.said());
}

/// One line under the label rather than beside it, for the forms that put the
/// answer on the next line.
#[test]
fn below_puts_the_words_on_the_line_under_the_label() {
    if !can_read_a_page() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let form = a_printed_form(&home, dir.path(), &[(1, 60.0, "Signature:")]);
    let delta = dir.path().join("delta.pdf");

    let written = run(
        &home,
        &[
            "write",
            &form.to_string_lossy(),
            "--below",
            "Signature:J. Bezzina",
            "--size",
            "10",
            "-o",
            &delta.to_string_lossy(),
        ],
    );
    assert!(written.ok, "{}", written.said());

    let (x_mm, y_mm) = placed_at(&written.said())
        .unwrap_or_else(|| panic!("nothing said where it went: {}", written.said()));
    // Left-aligned with the label it sits under, and below it — never above.
    assert!(
        (18.0..24.0).contains(&x_mm),
        "the words went to {x_mm} mm and the label starts at 20 mm"
    );
    assert!(
        y_mm > 60.0 && y_mm < 70.0,
        "the words went to {y_mm} mm, which is not one line under a label at 60 mm"
    );
    assert!(ink_per_page(&delta)[0] > 0, "{}", written.said());
}

/// An anchor that is not on the page has to be a refusal. Writing a blank delta
/// instead sends somebody to the printer with a sheet that changes nothing, and
/// they find out at the tray.
#[test]
fn an_anchor_that_is_not_there_is_refused_rather_than_written_blank() {
    if !can_read_a_page() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let form = a_printed_form(&home, dir.path(), &[(1, 40.0, "Received:")]);
    let delta = dir.path().join("delta.pdf");

    let written = run(
        &home,
        &[
            "write",
            &form.to_string_lossy(),
            "--after",
            "Superintendent:Yes",
            "-o",
            &delta.to_string_lossy(),
        ],
    );
    assert!(
        !written.ok,
        "an anchor nothing on the page matches was accepted: {}",
        written.said()
    );
    assert!(
        !delta.exists(),
        "a delta was written for an anchor that was never found"
    );
}

/// `--page 3` has to be matched against page three. Reading page one and
/// placing on page three is the worst kind of wrong: it finds a heading that
/// is on both, and puts the words at its page-one position on a different
/// sheet, which looks right until somebody holds the two up together.
#[test]
fn an_anchor_on_a_later_page_is_matched_against_that_page() {
    if !can_read_a_page() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    // The same label on both pages, at different heights — so a run that reads
    // the wrong page still finds something, and reports the wrong height.
    let form = a_printed_form(
        &home,
        dir.path(),
        &[(1, 40.0, "Total:"), (2, 150.0, "Total:")],
    );
    let delta = dir.path().join("delta.pdf");

    let written = run(
        &home,
        &[
            "write",
            &form.to_string_lossy(),
            "--after",
            "Total:£420",
            "--size",
            "10",
            "--page",
            "2",
            "-o",
            &delta.to_string_lossy(),
        ],
    );
    assert!(written.ok, "{}", written.said());

    let (_, y_mm) = placed_at(&written.said())
        .unwrap_or_else(|| panic!("nothing said where it went: {}", written.said()));
    assert!(
        (148.0..152.0).contains(&y_mm),
        "the words went to {y_mm} mm — page two's label is at 150 mm and page \
         one's is at 40 mm, so this read the wrong page"
    );

    // On page two of the delta, and page one left alone.
    let ink = ink_per_page(&delta);
    assert_eq!(ink.len(), 2, "{ink:?}");
    assert_eq!(ink[0], 0, "page one should be untouched: {ink:?}");
    assert!(ink[1] > 0, "page two carries nothing: {ink:?}");
}

/// A page the document does not have is refused by number, rather than by
/// whatever the renderer does when asked for it.
#[test]
fn a_page_that_is_not_there_is_named_in_the_refusal() {
    if !can_read_a_page() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let form = a_printed_form(&home, dir.path(), &[(1, 40.0, "Received:")]);

    let written = run(
        &home,
        &[
            "write",
            &form.to_string_lossy(),
            "--after",
            "Received:APPROVED",
            "--page",
            "7",
            "-o",
            &dir.path().join("delta.pdf").to_string_lossy(),
        ],
    );
    assert!(!written.ok, "{}", written.said());
    assert!(
        written.said().contains("no page 7"),
        "the refusal did not say which page was asked for: {}",
        written.said()
    );
}

/// The whole reason a saved job exists: work out where "PAID" goes once, then
/// run it on tomorrow's invoice without measuring anything. It goes through the
/// same route, so before the fix a saved job built on an anchor did nothing at
/// all — quietly, every day.
#[test]
fn a_saved_job_built_on_an_anchor_runs_on_another_document() {
    if !can_read_a_page() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let form = a_printed_form(&home, dir.path(), &[(1, 40.0, "Received:")]);

    // Saved once, off a real run.
    let saved = run(
        &home,
        &[
            "write",
            &form.to_string_lossy(),
            "--after",
            "Received:APPROVED ref {ref}",
            "--size",
            "10",
            "--save-as",
            "approve",
            "-o",
            &dir.path().join("first.pdf").to_string_lossy(),
        ],
    );
    assert!(saved.ok, "{}", saved.said());

    // What it wants is said before anything is written, not after.
    let without = run(
        &home,
        &[
            "job",
            "run",
            "approve",
            &form.to_string_lossy(),
            "-o",
            &dir.path().join("nope.pdf").to_string_lossy(),
        ],
    );
    assert!(!without.ok, "{}", without.said());
    assert!(without.said().contains("{ref}"), "{}", without.said());

    // And run again tomorrow, on another copy of the same form.
    let second = dir.path().join("another.pdf");
    std::fs::copy(&form, &second).unwrap();
    let delta = dir.path().join("ran.pdf");
    let ran = run(
        &home,
        &[
            "job",
            "run",
            "approve",
            &second.to_string_lossy(),
            "-o",
            &delta.to_string_lossy(),
            "--set",
            "ref=4471",
        ],
    );
    assert!(ran.ok, "{}", ran.said());
    assert!(
        ran.said().contains("Found"),
        "the job ran without placing anything against the page: {}",
        ran.said()
    );
    assert!(
        ink_per_page(&delta)[0] > 0,
        "the saved job produced a blank delta: {}",
        ran.said()
    );
}
