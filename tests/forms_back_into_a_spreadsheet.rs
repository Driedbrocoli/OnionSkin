//! `onionskin harvest`, over a stack that was really printed and really read.
//!
//! The unit tests build rows out of words and check the picking. This builds a
//! run of forms with `batch`, renders them, and reads them back through the
//! letter reader — which is the only way to find out what happens when the ink
//! is ambiguous, and ambiguous ink is the whole difficulty.

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

struct Work {
    dir: tempfile::TempDir,
    home: PathBuf,
}

impl Work {
    fn new() -> Work {
        let dir = tempfile::tempdir().expect("a place to work");
        let home = dir.path().join("home");
        std::fs::create_dir_all(&home).expect("a home of its own");
        Work { dir, home }
    }

    fn at(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    /// A run of filled-in forms, made the way Onionskin makes one.
    ///
    /// Printed rather than hand-built, so the words on them have been through a
    /// PDF and come back out of the reader — which is where `0` becomes `O` and
    /// the whole difficulty starts.
    fn a_stack_of_filled_forms(&self, list: &str) -> PathBuf {
        let csv = self.at("people.csv");
        std::fs::write(&csv, list).expect("the list should write");

        let document = self.at("form.osk");
        let blank = self.at("blank.pdf");
        let filled = self.at("filled.pdf");
        for args in [
            vec!["new", &at(&document), "--page", "a4"],
            vec!["write", &at(&document), "--at", "20,30:APPLICATION FORM"],
            vec!["print", &at(&document), "-o", &at(&blank)],
        ] {
            let (ok, said) = run(&self.home, &args);
            assert!(ok, "{said}");
        }
        let (ok, said) = run(
            &self.home,
            &[
                "batch",
                &at(&blank),
                "--from",
                &at(&csv),
                "--at",
                "20,60:Name: {Name}",
                "--at",
                "110,60:Date: {Date}",
                "--at",
                "20,75:Amount: {Amount}",
                "-o",
                &at(&filled),
            ],
        );
        assert!(ok, "{said}");
        filled
    }
}

fn cells(csv: &str) -> Vec<Vec<String>> {
    csv.lines()
        .map(|line| line.split(',').map(str::to_string).collect())
        .collect()
}

/// The spreadsheet out of everything the command said.
///
/// From the heading row to the first blank line: what follows the spreadsheet on
/// the terminal is prose, and prose read as rows is six sheets where there was
/// one.
fn spreadsheet_in(said: &str) -> Vec<Vec<String>> {
    let csv: Vec<&str> = said
        .lines()
        .skip_while(|line| !line.starts_with("Sheet,"))
        .take_while(|line| !line.trim().is_empty())
        .collect();
    cells(&csv.join("\n"))
}

/// The whole of it: a stack in, a spreadsheet out, with the values in the right
/// columns and in the right order.
#[test]
fn a_stack_of_forms_comes_back_as_a_spreadsheet() {
    let work = Work::new();
    let filled = work.a_stack_of_filled_forms(
        "Name,Date,Amount\n\
         J. Bezzina,27 July 2024,240.00\n\
         A. Borg,28 July 2024,95.50\n\
         C. Mifsud,29 July 2024,17.25\n",
    );
    let out = work.at("harvested.csv");

    let (ok, said) = run(
        &work.home,
        &[
            "harvest",
            &at(&filled),
            "--field",
            "Name",
            "--field",
            "Date",
            "--field",
            "Amount/number",
            "-o",
            &at(&out),
        ],
    );
    assert!(ok, "{said}");
    assert!(said.contains("3 sheet(s)"), "{said}");

    let csv = std::fs::read_to_string(&out).expect("the spreadsheet should be there");
    let rows = cells(&csv);
    assert_eq!(rows[0], vec!["Sheet", "Name", "Date", "Amount"]);
    assert_eq!(rows.len(), 4, "{csv}");

    // The names came off the paper. Read through a printer and a reader, so a
    // letter here and there may be wrong — but the initial and the surname are
    // what tell one sheet from another.
    for (row, name) in rows[1..].iter().zip(["Bezzina", "Borg", "Mifsud"]) {
        assert!(
            row[1].contains(name),
            "sheet {} came back as '{}', which is not {name}",
            row[0],
            row[1]
        );
    }

    // The dates and the amounts came back as dates and amounts, which is what
    // resolving the ambiguous shapes is for: a page read straight off the ink
    // gives '27 July 2O24' and '24O.OO'.
    for (row, expected) in rows[1..].iter().zip(["2024", "2024", "2024"]) {
        assert!(
            row[2].contains(expected),
            "sheet {}: the date came back as '{}'",
            row[0],
            row[2]
        );
    }
    //
    // Every amount either comes back as the figure it was, or is named as
    // something that is not a number — never as a wrong figure. That is the
    // contract, and it is the only one worth making: the reader sometimes reads
    // a printed 1 as a J, and no amount of arithmetic here can know whether a
    // J at the front of a reference was a 1 or a J.
    let mut exact = 0;
    for (row, expected) in rows[1..].iter().zip(["240.00", "95.50", "17.25"]) {
        if row[3] == expected {
            exact += 1;
            continue;
        }
        assert!(
            said.contains(&format!("sheet {}: Amount", row[0])),
            "sheet {}: the amount came back as '{}' and nothing said so:\n{said}",
            row[0],
            row[3]
        );
        assert!(
            said.contains("is not a number"),
            "'{}' was neither right nor flagged",
            row[3]
        );
    }
    assert!(
        exact >= 2,
        "only {exact} of 3 amounts came back exactly, so resolving the \
         ambiguous shapes is not doing its work"
    );
}

/// Two fields on one line keep their own values, on real ink.
///
/// This is the case the whole design rests on: without knowing where a value
/// stops, `Name` comes back as the entire rest of the line — which does not look
/// wrong in a spreadsheet, it looks like somebody's name.
#[test]
fn two_fields_on_one_line_do_not_run_into_each_other() {
    let work = Work::new();
    let filled = work.a_stack_of_filled_forms("Name,Date,Amount\nJ. Bezzina,27 July 2024,240.00\n");

    let (ok, said) = run(
        &work.home,
        &[
            "harvest",
            &at(&filled),
            "--field",
            "Name",
            "--field",
            "Date",
            "--stdout",
        ],
    );
    assert!(ok, "{said}");
    let rows = spreadsheet_in(&said);

    assert!(rows[1][1].contains("Bezzina"), "{said}");
    assert!(
        !rows[1][1].contains("July"),
        "the name swallowed the date: '{}'",
        rows[1][1]
    );
    assert!(rows[1][2].contains("2024"), "{said}");
}

/// A field whose label is not on the form comes back as nothing, and the sheet
/// is named — rather than as an empty cell that reads as "this was blank".
#[test]
fn a_label_that_is_not_on_the_form_is_named_rather_than_left_empty() {
    let work = Work::new();
    let filled = work.a_stack_of_filled_forms("Name,Date,Amount\nJ. Bezzina,27 July 2024,240.00\n");

    let (ok, said) = run(
        &work.home,
        &[
            "harvest",
            &at(&filled),
            "--field",
            "Name",
            "--field",
            "Telephone",
            "--stdout",
        ],
    );
    assert!(ok, "{said}");
    assert!(
        said.contains("Telephone") && said.contains("not on this sheet"),
        "the missing field was not named: {said}"
    );
    // And it is counted honestly rather than as a reading.
    assert!(said.contains("1 of 2 cell(s)"), "{said}");
}

/// --first stops early, so the shape of a run can be seen without waiting for
/// two hundred sheets.
#[test]
fn a_run_can_be_looked_at_before_the_whole_stack_is_read() {
    let work = Work::new();
    let filled = work.a_stack_of_filled_forms(
        "Name,Date,Amount\n\
         J. Bezzina,27 July 2024,240.00\n\
         A. Borg,28 July 2024,95.50\n\
         C. Mifsud,29 July 2024,17.25\n",
    );

    let (ok, said) = run(
        &work.home,
        &[
            "harvest",
            &at(&filled),
            "--field",
            "Name",
            "--first",
            "1",
            "--stdout",
        ],
    );
    assert!(ok, "{said}");
    assert!(said.contains("Reading 1 sheet(s)"), "{said}");
    let rows = spreadsheet_in(&said);
    assert_eq!(rows.len(), 2, "more than one sheet was read: {rows:?}");
}

/// A field nobody could mean says so before anything is read.
#[test]
fn a_field_that_is_not_one_is_refused() {
    let work = Work::new();
    let filled = work.a_stack_of_filled_forms("Name,Date,Amount\nJ. Bezzina,27 July 2024,240.00\n");

    let (ok, said) = run(
        &work.home,
        &["harvest", &at(&filled), "--field", "=nothing"],
    );
    assert!(!ok, "{said}");
    assert!(said.contains("no column name"), "{said}");

    // And no fields at all is refused by the command line itself.
    let (ok, said) = run(&work.home, &["harvest", &at(&filled)]);
    assert!(!ok, "{said}");
    assert!(said.contains("--field"), "{said}");
}

/// A rehearsal reads the stack and writes nothing.
#[test]
fn a_rehearsal_writes_no_spreadsheet() {
    let work = Work::new();
    let filled = work.a_stack_of_filled_forms("Name,Date,Amount\nJ. Bezzina,27 July 2024,240.00\n");
    let out = work.at("nothing.csv");

    let (ok, said) = run(
        &work.home,
        &[
            "harvest",
            &at(&filled),
            "--field",
            "Name",
            "-o",
            &at(&out),
            "--dry-run",
        ],
    );
    assert!(ok, "{said}");
    assert!(
        said.contains("Bezzina"),
        "it did not read the sheet: {said}"
    );
    assert!(!out.exists(), "a rehearsal left a file behind");
}
