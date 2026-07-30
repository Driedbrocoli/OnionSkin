//! The two things that answer a question from an SSH session.
//!
//! Everything in this program that says "look at it before you print" ends in a
//! file somebody opens. On a server there is nothing to open it with, and the
//! person looking after it has a terminal and nothing else. So a proof can be
//! drawn in the terminal, and the record can be asked about a whole folder at
//! once rather than one file at a time.
//!
//! Both are checked here by running the real binary and reading what came out —
//! which for the drawing means finding the ink in it and working out where on
//! the paper that was.

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

struct Desk {
    dir: tempfile::TempDir,
    home: PathBuf,
    sheet: PathBuf,
}

impl Desk {
    fn new() -> Desk {
        let dir = tempfile::tempdir().expect("a place to work");
        let home = dir.path().join("home");
        std::fs::create_dir_all(&home).expect("a home of its own");

        let document = dir.path().join("invoice.osk");
        let sheet = dir.path().join("invoice.pdf");
        for args in [
            vec!["new", &at(&document), "--page", "a4"],
            vec!["write", &at(&document), "--at", "20,30:Invoice no: 4471"],
            vec!["print", &at(&document), "-o", &at(&sheet)],
        ] {
            let (ok, said) = run(&home, &args);
            assert!(ok, "setting up: {said}");
        }
        Desk { dir, home, sheet }
    }

    fn a_delta(&self, name: &str, place: &str) -> PathBuf {
        let delta = self.dir.path().join(name);
        let (ok, said) = run(
            &self.home,
            &[
                "write",
                &at(&self.sheet),
                "--at",
                place,
                "--size",
                "9",
                "-o",
                &at(&delta),
            ],
        );
        assert!(ok, "{said}");
        delta
    }
}

/// The drawn page out of the command's output: the lines inside the frame.
fn drawing_in(said: &str) -> Vec<String> {
    said.lines()
        .skip_while(|line| !line.starts_with('┌'))
        .skip(1)
        .take_while(|line| !line.starts_with('└'))
        .map(|line| {
            line.trim_start_matches('│')
                .trim_end_matches('│')
                .to_string()
        })
        .collect()
}

/// Where the ink is in a drawing, as a fraction of the way across and down.
fn ink_in(drawing: &[String]) -> Option<(f64, f64, f64, f64)> {
    let across = drawing.iter().map(|line| line.chars().count()).max()?;
    let down = drawing.len();
    let spots: Vec<(usize, usize)> = drawing
        .iter()
        .enumerate()
        .flat_map(|(row, line)| {
            line.chars()
                .enumerate()
                .filter(|(_, c)| !c.is_whitespace())
                .map(move |(column, _)| (row, column))
        })
        .collect();
    if spots.is_empty() {
        return None;
    }
    let x = |column: usize| column as f64 / (across - 1) as f64;
    let y = |row: usize| row as f64 / (down - 1) as f64;
    Some((
        x(spots.iter().map(|(_, c)| *c).min()?),
        y(spots.iter().map(|(r, _)| *r).min()?),
        x(spots.iter().map(|(_, c)| *c).max()?),
        y(spots.iter().map(|(r, _)| *r).max()?),
    ))
}

/// The whole feature: the words show up in the terminal roughly where they are
/// on the paper. That is the question somebody asks from an SSH session, and
/// it was previously unanswerable there at all.
#[test]
fn a_stamp_shows_up_in_the_terminal_where_it_is_on_the_paper() {
    let desk = Desk::new();
    // A stamp at 150,40 mm on A4: 71% across, 13% down.
    let delta = desk.a_delta("delta.pdf", "150,40:PAID 2026-07-30");
    let proof = desk.dir.path().join("proof.pdf");

    let (ok, said) = run(
        &desk.home,
        &[
            "proof",
            &at(&desk.sheet),
            "--delta",
            &at(&delta),
            "-o",
            &at(&proof),
            "--in-the-terminal",
            "--across",
            "60",
        ],
    );
    assert!(ok, "{said}");

    let drawing = drawing_in(&said);
    assert!(!drawing.is_empty(), "nothing was drawn:\n{said}");
    let (left, top, right, bottom) = ink_in(&drawing).expect("there should be ink:\n{said}");

    // The invoice line is at 20,30 mm — 10% across, 10% down — and the stamp at
    // 150,40. So the ink spans from about a tenth across to about four fifths.
    assert!(left < 0.2, "the leftmost ink was at {left}:\n{said}");
    assert!(right > 0.65, "the rightmost ink was at {right}:\n{said}");
    assert!(top < 0.2, "the topmost ink was at {top}:\n{said}");
    assert!(
        bottom < 0.35,
        "something was drawn far down a page with nothing on it: {bottom}\n{said}"
    );
}

/// The frame, the page number and the ruler: everything needed to read a
/// position off the drawing rather than guess at it.
#[test]
fn the_drawing_says_which_page_it_is_and_how_wide_the_paper_is() {
    let desk = Desk::new();
    let delta = desk.a_delta("delta.pdf", "150,40:PAID");
    let proof = desk.dir.path().join("proof.pdf");
    let (ok, said) = run(
        &desk.home,
        &[
            "proof",
            &at(&desk.sheet),
            "--delta",
            &at(&delta),
            "-o",
            &at(&proof),
            "--in-the-terminal",
            "--across",
            "60",
        ],
    );
    assert!(ok, "{said}");

    assert!(said.contains("page 1 of 1"), "{said}");
    assert!(said.contains("210 × 297 mm"), "{said}");
    assert!(said.contains("0mm"), "the ruler is missing:\n{said}");
    assert!(said.contains("150"), "{said}");
    // And it is honest about what it is: nobody should approve a run of two
    // hundred off eighty characters of text.
    assert!(said.contains("Coarse on purpose"), "{said}");

    // Nothing wider than was asked for. `--across 60` on a sixty-column
    // terminal has to come out at sixty *including* the frame — the frame is
    // two of those columns, and a drawing that overruns by two wraps onto a
    // second line and stops being a drawing. The ruler under it too, which is
    // the part a position is actually read off.
    //
    // The drawing itself, then: the ordinary lines of the command's output are
    // prose and wrap wherever the terminal likes.
    const ASKED_FOR: usize = 60;
    let block: Vec<&str> = said
        .lines()
        .skip_while(|line| !line.starts_with('┌'))
        .take_while(|line| !line.contains("Coarse on purpose"))
        .collect();
    assert!(block.len() > 6, "hardly anything was drawn:\n{said}");
    for line in &block {
        assert!(
            line.chars().count() <= ASKED_FOR,
            "{} characters where {ASKED_FOR} were asked for: {line:?}",
            line.chars().count()
        );
        if line.starts_with('│') {
            // And square, or the right-hand edge of the paper is ragged.
            assert_eq!(line.chars().count(), ASKED_FOR, "ragged: {line:?}");
        }
    }
    // The ruler sits under the drawing, not under the terminal's left margin:
    // unindented it is one column out, and every position read off it is a
    // cell too far right.
    let ruler = block
        .iter()
        .rev()
        .find(|line| line.contains("0mm"))
        .unwrap_or_else(|| panic!("no ruler in:\n{said}"));
    assert!(
        ruler.starts_with(' '),
        "the ruler is not under the drawing it measures: {ruler:?}"
    );

    // The proof PDF is still written — the drawing is as well as, not instead
    // of, since a machine with a window wants the real thing.
    assert!(proof.is_file(), "{said}");
}

/// Without the flag nothing is drawn, because the ordinary case is a machine
/// that can open the proof and a terminal that should not fill with hashes.
#[test]
fn nothing_is_drawn_unless_it_is_asked_for() {
    let desk = Desk::new();
    let delta = desk.a_delta("delta.pdf", "150,40:PAID");
    let proof = desk.dir.path().join("proof.pdf");
    let (ok, said) = run(
        &desk.home,
        &[
            "proof",
            &at(&desk.sheet),
            "--delta",
            &at(&delta),
            "-o",
            &at(&proof),
        ],
    );
    assert!(ok, "{said}");
    assert!(!said.contains('┌'), "{said}");
}

/// Holding a folder of deltas and a box of stock, the question is "which of
/// *these*" — and asking it one file at a time of two hundred is not asking it.
#[test]
fn a_whole_folder_can_be_asked_at_once() {
    let desk = Desk::new();
    let waiting = desk.dir.path().join("to-print");
    std::fs::create_dir_all(&waiting).expect("somewhere to put them");

    // Two written by Onionskin, so the record knows them.
    for (name, place) in [("a.pdf", "150,40:PAID"), ("b.pdf", "150,60:FILED")] {
        let delta = desk.a_delta(name, place);
        std::fs::copy(&delta, waiting.join(name)).expect("it should copy");
    }
    // And one it has never seen.
    std::fs::copy(&desk.sheet, waiting.join("stranger.pdf")).expect("it should copy");

    let (ok, said) = run(&desk.home, &["history", "--asking-about", &at(&waiting)]);
    assert!(ok, "{said}");
    assert!(said.contains("3 PDFs"), "{said}");
    assert!(said.contains("Written before"), "{said}");
    assert!(said.contains("a.pdf"), "{said}");
    assert!(said.contains("b.pdf"), "{said}");
    assert!(said.contains("Not in the record"), "{said}");
    assert!(said.contains("stranger.pdf"), "{said}");
    // It does not refuse or scold: a hundred certificates are one delta
    // printed a hundred times, and that is exactly right.
    assert!(said.contains("fresh"), "{said}");
}

/// A delta somebody renamed is still the same delta, because the record knows
/// it by its bytes rather than by what it is called.
#[test]
fn a_renamed_delta_is_still_recognised() {
    let desk = Desk::new();
    let waiting = desk.dir.path().join("to-print");
    std::fs::create_dir_all(&waiting).expect("somewhere to put them");
    let delta = desk.a_delta("delta.pdf", "150,40:PAID");
    std::fs::copy(&delta, waiting.join("something-else-entirely.pdf")).expect("it should copy");

    let (ok, said) = run(&desk.home, &["history", "--asking-about", &at(&waiting)]);
    assert!(ok, "{said}");
    assert!(said.contains("Written before"), "{said}");
    assert!(said.contains("something-else-entirely.pdf"), "{said}");
    assert!(!said.contains("Not in the record"), "{said}");
}

/// A script asking the same question gets the same answer.
#[test]
fn the_folder_question_answers_a_script_too() {
    let desk = Desk::new();
    let waiting = desk.dir.path().join("to-print");
    std::fs::create_dir_all(&waiting).expect("somewhere to put them");
    let delta = desk.a_delta("delta.pdf", "150,40:PAID");
    std::fs::copy(&delta, waiting.join("a.pdf")).expect("it should copy");
    std::fs::copy(&desk.sheet, waiting.join("b.pdf")).expect("it should copy");

    let (ok, said) = run(
        &desk.home,
        &["history", "--asking-about", &at(&waiting), "--json"],
    );
    assert!(ok, "{said}");
    let read: serde_json::Value = serde_json::from_str(&said).expect("it should be JSON");
    assert_eq!(
        read["printed"].as_array().map(|a| a.len()),
        Some(1),
        "{said}"
    );
    assert_eq!(
        read["not_printed"].as_array().map(|a| a.len()),
        Some(1),
        "{said}"
    );
    assert!(read["printed"][0]["file"]
        .as_str()
        .unwrap()
        .ends_with("a.pdf"));
    assert!(!read["printed"][0]["when"].as_str().unwrap().is_empty());
}

/// A folder that is not there, or a file where a folder was meant, is said
/// rather than reported as "nothing has been printed".
#[test]
fn a_folder_that_is_not_a_folder_is_said_so() {
    let desk = Desk::new();
    let nowhere = desk.dir.path().join("nowhere");

    let (ok, said) = run(&desk.home, &["history", "--asking-about", &at(&nowhere)]);
    assert!(!ok, "{said}");
    assert!(said.contains("nowhere"), "{said}");

    let (ok, said) = run(&desk.home, &["history", "--asking-about", &at(&desk.sheet)]);
    assert!(!ok, "{said}");
    assert!(said.contains("folder"), "{said}");

    // An empty folder is not an error, it is an empty folder.
    let empty = desk.dir.path().join("empty");
    std::fs::create_dir_all(&empty).expect("somewhere empty");
    let (ok, said) = run(&desk.home, &["history", "--asking-about", &at(&empty)]);
    assert!(ok, "{said}");
    assert!(said.contains("No PDFs"), "{said}");
}
