//! One page moved, and the other thirty-nine did not.
//!
//! An edit that pushes existing text down the page cannot be fixed by an
//! overlay: the sheet in the tray no longer matches the document, and ink does
//! not come off paper. Onionskin has always said so, and then refused the whole
//! job — which on a four-page report where one line moved on page two means
//! reprinting four sheets to fix one, or passing `--force` and printing new
//! words onto text that has since moved. Both are worse than doing both things.
//!
//! This drives the real commands end to end: build a document, print it, move
//! one page's text, and check that what comes out is an overlay for the pages
//! that did not move and a whole fresh sheet for the one that did.

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

/// A four-page report, printed, and then edited two ways at once: three pages
/// gain a stamp, and page two's paragraphs are pushed down the page.
///
/// Returns the printed sheet and the edited copy, as PDFs.
fn a_report_with_one_page_moved(home: &Path, dir: &Path) -> (PathBuf, PathBuf) {
    let document = dir.join("report.osk").to_string_lossy().into_owned();
    assert!(run(home, &["new", &document, "--page", "a4"]).ok);
    for page in 1..=4 {
        for (at, words) in [
            ("20,30", format!("Report page {page}")),
            ("20,60", format!("First paragraph on page {page}.")),
            ("20,80", format!("Second paragraph on page {page}.")),
        ] {
            let placed = format!("{at}:{words}");
            let made = run(
                home,
                &[
                    "write",
                    &document,
                    "--at",
                    &placed,
                    "--size",
                    "11",
                    "--page",
                    &page.to_string(),
                ],
            );
            assert!(made.ok, "{}", made.said());
        }
    }
    let before = dir.join("before.pdf");
    assert!(run(home, &["print", &document, "-o", &before.to_string_lossy()]).ok);

    // The edit. Three pages gain a stamp, which an overlay can add.
    let edited = dir.join("edited.osk").to_string_lossy().into_owned();
    std::fs::copy(&document, &edited).unwrap();
    for page in [1, 3, 4] {
        let made = run(
            home,
            &[
                "write",
                &edited,
                "--at",
                "120,40:APPROVED",
                "--size",
                "12",
                "--page",
                &page.to_string(),
            ],
        );
        assert!(made.ok, "{}", made.said());
    }
    // Page two's two paragraphs move down, which an overlay cannot fix. They
    // are items 5 and 6: three to a page, page two is the second three.
    assert!(run(home, &["edit", &edited, "5", "--at", "20,75"]).ok);
    assert!(run(home, &["edit", &edited, "6", "--at", "20,95"]).ok);

    let after = dir.join("after.pdf");
    assert!(run(home, &["print", &edited, "-o", &after.to_string_lossy()]).ok);
    (before, after)
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

/// The whole point: the three good pages are still an overlay, and the fourth
/// comes out whole on its own.
#[test]
fn the_pages_that_did_not_move_are_still_an_overlay() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let (before, after) = a_report_with_one_page_moved(&home, dir.path());
    let delta = dir.path().join("delta.pdf");
    let fresh = dir.path().join("fresh.pdf");

    let split = run(
        &home,
        &[
            "delta",
            &before.to_string_lossy(),
            &after.to_string_lossy(),
            "-o",
            &delta.to_string_lossy(),
            "--fresh",
            &fresh.to_string_lossy(),
        ],
    );
    assert!(split.ok, "the split was still blocked: {}", split.said());
    assert!(delta.is_file(), "{}", split.said());
    assert!(fresh.is_file(), "{}", split.said());

    // The delta keeps all four pages — page n is fed the printed sheet n — but
    // the one that moved is blank, so feeding it would do nothing.
    let delta_ink = ink_per_page(&delta);
    assert_eq!(delta_ink.len(), 4, "a page was dropped: {delta_ink:?}");
    assert!(delta_ink[0] > 0, "{delta_ink:?}");
    assert_eq!(delta_ink[1], 0, "page two was not blanked: {delta_ink:?}");
    assert!(delta_ink[2] > 0, "{delta_ink:?}");
    assert!(delta_ink[3] > 0, "{delta_ink:?}");

    // And the fresh file is one page — the whole of page two, not an overlay.
    let fresh_ink = ink_per_page(&fresh);
    assert_eq!(fresh_ink.len(), 1, "{fresh_ink:?}");
    assert!(
        fresh_ink[0] > delta_ink[0] * 2,
        "the fresh page should carry the whole page, not an addition: \
         {fresh_ink:?} against {delta_ink:?}"
    );
}

/// What somebody at the printer is told: two things to print, which sheets go
/// where, and that the blanked page is not worth feeding.
#[test]
fn the_instructions_name_both_files_and_both_sets_of_sheets() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let (before, after) = a_report_with_one_page_moved(&home, dir.path());
    let delta = dir.path().join("delta.pdf");
    let fresh = dir.path().join("fresh.pdf");

    let split = run(
        &home,
        &[
            "delta",
            &before.to_string_lossy(),
            &after.to_string_lossy(),
            "-o",
            &delta.to_string_lossy(),
            "--fresh",
            &fresh.to_string_lossy(),
        ],
    );
    let said = split.said();
    assert!(said.contains("feed sheets 1, 3 and 4 back in"), "{said}");
    assert!(said.contains("print sheet 2 on fresh paper"), "{said}");
    // The summary counts what the delta carries, not what the edit changed —
    // page two's additions were blanked along with the rest of it.
    assert!(
        said.contains("3 additions on pages 1, 3, 4"),
        "the summary counted a blanked page: {said}"
    );
    // And it is no longer a blocker, because there is something worth printing.
    assert!(!said.contains("BLOCKER"), "{said}");
}

/// Without the flag the job is still refused — but the refusal now says how to
/// get the thirty-nine pages that are fine, instead of leaving somebody to
/// reprint all forty.
#[test]
fn the_refusal_points_at_the_way_out() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let (before, after) = a_report_with_one_page_moved(&home, dir.path());
    let delta = dir.path().join("delta.pdf");

    let refused = run(
        &home,
        &[
            "delta",
            &before.to_string_lossy(),
            &after.to_string_lossy(),
            "-o",
            &delta.to_string_lossy(),
        ],
    );
    assert!(!refused.ok, "{}", refused.said());
    let said = refused.said();
    assert!(said.contains("--fresh"), "{said}");
    assert!(
        said.contains("The text moved on 1 of the 4 page(s)"),
        "{said}"
    );
    assert!(said.contains("the rest can still be overprinted"), "{said}");
}

/// Two documents handed over in the wrong order have ink missing from every
/// page. That is one mistake — the arguments are the wrong way round — and not
/// forty pages that moved, and the split must not act on it: doing so would
/// blank the whole delta and write the entire document out as "fresh", which
/// looks like an answer and is not one.
#[test]
fn documents_given_the_wrong_way_round_are_not_split() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let document = dir.path().join("plain.osk").to_string_lossy().into_owned();
    assert!(run(&home, &["new", &document, "--page", "a4"]).ok);
    assert!(run(&home, &["write", &document, "--at", "20,30:Fixed heading"]).ok);
    let before = dir.path().join("before.pdf");
    assert!(
        run(
            &home,
            &["print", &document, "-o", &before.to_string_lossy()]
        )
        .ok
    );
    assert!(run(&home, &["write", &document, "--at", "20,90:Added later"]).ok);
    let after = dir.path().join("after.pdf");
    assert!(run(&home, &["print", &document, "-o", &after.to_string_lossy()]).ok);

    // The edited copy given as the original, which is the easy mistake.
    let delta = dir.path().join("delta.pdf");
    let fresh = dir.path().join("fresh.pdf");
    let refused = run(
        &home,
        &[
            "delta",
            &after.to_string_lossy(),
            &before.to_string_lossy(),
            "-o",
            &delta.to_string_lossy(),
            "--fresh",
            &fresh.to_string_lossy(),
        ],
    );
    assert!(!refused.ok, "{}", refused.said());
    assert!(
        refused.said().contains("wrong way round"),
        "{}",
        refused.said()
    );
    assert!(
        !fresh.exists(),
        "the whole document was written out as 'fresh' for what is one mistake"
    );
}

/// An edit that moves nothing is not split, and asking for a split does not
/// invent one — no fresh file, and every page still carries its additions.
#[test]
fn a_job_where_nothing_moved_is_left_alone() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let document = dir.path().join("plain.osk").to_string_lossy().into_owned();
    assert!(run(&home, &["new", &document, "--page", "a4"]).ok);
    assert!(run(&home, &["write", &document, "--at", "20,30:Fixed text"]).ok);
    let before = dir.path().join("before.pdf");
    assert!(
        run(
            &home,
            &["print", &document, "-o", &before.to_string_lossy()]
        )
        .ok
    );

    assert!(run(&home, &["write", &document, "--at", "20,80:Added later"]).ok);
    let after = dir.path().join("after.pdf");
    assert!(run(&home, &["print", &document, "-o", &after.to_string_lossy()]).ok);

    let delta = dir.path().join("delta.pdf");
    let fresh = dir.path().join("fresh.pdf");
    let made = run(
        &home,
        &[
            "delta",
            &before.to_string_lossy(),
            &after.to_string_lossy(),
            "-o",
            &delta.to_string_lossy(),
            "--fresh",
            &fresh.to_string_lossy(),
        ],
    );
    assert!(made.ok, "{}", made.said());
    assert!(
        !fresh.exists(),
        "a fresh file was written for a job with nothing to reprint"
    );
    assert!(ink_per_page(&delta)[0] > 0, "the delta came out blank");
}

/// The fresh pages must not be written over one of the documents they came
/// from, which would destroy the thing being printed.
#[test]
fn the_fresh_pages_cannot_be_written_over_an_input() {
    let dir = tempfile::tempdir().unwrap();
    let home = dir.path().join("home");
    let (before, after) = a_report_with_one_page_moved(&home, dir.path());
    let delta = dir.path().join("delta.pdf");

    for (target, name) in [(&before, "original"), (&after, "edited copy")] {
        let refused = run(
            &home,
            &[
                "delta",
                &before.to_string_lossy(),
                &after.to_string_lossy(),
                "-o",
                &delta.to_string_lossy(),
                "--fresh",
                &target.to_string_lossy(),
            ],
        );
        assert!(!refused.ok, "writing over the {name} was allowed");
        assert!(refused.stderr.contains("refusing"), "{}", refused.said());
    }
}
