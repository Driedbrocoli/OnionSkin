//! The two checks that happen before two hundred sheets of stock go through.
//!
//! A batch is the expensive command. Two hundred certificates is two hundred
//! sheets of pre-printed stock, and a mistake in it is found by somebody
//! holding the stack. Both of these exist to move that discovery earlier:
//! the same person twice is pointed out from the list, and the first sheet can
//! be had on its own to hold against a real one.

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
    home: PathBuf,
    blank: PathBuf,
}

impl Office {
    fn new() -> Office {
        let dir = tempfile::tempdir().expect("a place to work");
        let home = dir.path().join("home");
        std::fs::create_dir_all(&home).expect("a home of its own");
        let document = dir.path().join("certificate.osk");
        let blank = dir.path().join("certificate.pdf");
        for args in [
            vec!["new", &at(&document), "--page", "a4"],
            vec!["write", &at(&document), "--at", "20,30:Awarded to:"],
            vec!["print", &at(&document), "-o", &at(&blank)],
        ] {
            let (ok, said) = run(&home, &args);
            assert!(ok, "setting up: {said}");
        }
        Office { dir, home, blank }
    }

    fn a_list(&self, name: &str, text: &str) -> PathBuf {
        let path = self.dir.path().join(name);
        std::fs::write(&path, text).expect("the list should write");
        path
    }

    fn a_run(&self, out: &str, extra: &[&str]) -> (bool, String, PathBuf) {
        let stack = self.dir.path().join(out);
        let mut args = vec![
            "batch".to_string(),
            at(&self.blank),
            "--after".to_string(),
            "Awarded to:{name}".to_string(),
            "-o".to_string(),
            at(&stack),
        ];
        args.extend(extra.iter().map(|s| s.to_string()));
        let borrowed: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        let (ok, said) = run(&self.home, &borrowed);
        (ok, said, stack)
    }
}

fn pages_in(pdf: &Path) -> usize {
    let engine = onionskin::render::engine().expect("a renderer");
    let pages = engine.open(pdf).expect("it should open").len();
    pages
}

/// A spreadsheet pasted onto itself is obvious in the list and invisible in the
/// stack of paper. It costs two sheets of stock and somebody working out which
/// of the two to hand over.
#[test]
fn the_same_person_twice_is_pointed_out_before_the_paper_goes_in() {
    let office = Office::new();
    let list = office.a_list(
        "people.csv",
        "name,course\nAda,Maths\nGrace,Physics\nAda,Maths\nAlan,Logic\n",
    );

    let (ok, said, stack) = office.a_run("stack.pdf", &["--from", &at(&list)]);
    assert!(ok, "{said}");
    assert!(said.contains("more than once"), "{said}");
    assert!(said.contains("rows 1, 3"), "{said}");
    assert!(said.contains("Ada"), "{said}");
    // What it costs, in the unit that matters.
    assert!(said.contains("1 sheet"), "{said}");

    // Said, not refused — two copies of a ticket is a real thing to want, and
    // a program that stopped would be worse than one that mentioned it.
    assert_eq!(pages_in(&stack), 4, "{said}");
}

/// The same name against a different course is two different sheets. A false
/// alarm on a list that is perfectly correct teaches people to ignore the
/// message, which is worse than not having it.
#[test]
fn a_list_with_no_repeats_says_nothing_about_repeats() {
    let office = Office::new();
    let list = office.a_list(
        "people.csv",
        "name,course\nAda,Maths\nAda,Physics\nGrace,Logic\n",
    );
    let (ok, said, _) = office.a_run("stack.pdf", &["--from", &at(&list)]);
    assert!(ok, "{said}");
    assert!(!said.contains("more than once"), "{said}");
}

/// A counted run has no values at all — every row is {number} and nothing else
/// — so calling two hundred of them duplicates would be nonsense about a run
/// that is exactly what was asked for.
#[test]
fn a_run_of_numbers_is_not_two_hundred_duplicates() {
    let office = Office::new();
    let tickets = office.dir.path().join("tickets.pdf");
    // Not through `a_run`, which anchors to a {name} column a counted run has
    // no business having.
    let (ok, said) = run(
        &office.home,
        &[
            "batch",
            &at(&office.blank),
            "--count",
            "20",
            "--at",
            "150,40:No. {number}",
            "-o",
            &at(&tickets),
        ],
    );
    assert!(ok, "{said}");
    assert!(!said.contains("more than once"), "{said}");
    assert_eq!(pages_in(&tickets), 20, "{said}");
}

/// The first sheet on its own, so it can be printed and held against a real
/// one — and then the rest printed, rather than working out what was typed
/// twenty minutes ago and typing it again with different flags.
#[test]
fn the_first_sheet_comes_out_on_its_own_as_well() {
    let office = Office::new();
    let list = office.a_list("people.csv", "name\nAda\nGrace\nAlan\nBarbara\n");

    let (ok, said, stack) = office.a_run("stack.pdf", &["--from", &at(&list), "--proof-first"]);
    assert!(ok, "{said}");

    // Both files: the whole stack, and page one on its own.
    assert_eq!(pages_in(&stack), 4, "{said}");
    let one = office.dir.path().join("stack-first.pdf");
    assert!(one.is_file(), "the single sheet was not written:\n{said}");
    assert_eq!(pages_in(&one), 1, "{said}");

    // And it says what to do with them, in that order.
    assert!(said.contains("the first sheet on its own"), "{said}");
    assert!(said.contains("hold it against a real"), "{said}");
    assert!(said.contains("other 3"), "{said}");
    assert!(said.contains("pages 2 to 4"), "{said}");
}

/// Page one of the stack and the single sheet have to be the same sheet. A
/// proof that showed something else is worse than no proof: it would be
/// approved and the run would be wrong.
#[test]
fn the_single_sheet_is_the_same_as_page_one_of_the_stack() {
    let office = Office::new();
    let list = office.a_list("people.csv", "name\nAda\nGrace\nAlan\n");
    let (ok, said, stack) = office.a_run("stack.pdf", &["--from", &at(&list), "--proof-first"]);
    assert!(ok, "{said}");
    let one = office.dir.path().join("stack-first.pdf");

    const DPI: f64 = 100.0;
    let engine = onionskin::render::engine().expect("a renderer");
    let whole = engine.open(&stack).expect("the stack should open");
    let single = engine.open(&one).expect("the sheet should open");
    let page_one = whole.render_gray(0, DPI).expect("it should draw");
    let alone = single.render_gray(0, DPI).expect("it should draw");

    assert_eq!(
        (page_one.width, page_one.height),
        (alone.width, alone.height)
    );
    let differing = page_one
        .gray
        .iter()
        .zip(alone.gray.iter())
        .filter(|(a, b)| a != b)
        .count();
    assert_eq!(
        differing, 0,
        "the single sheet is not page one of the stack — {differing} pixels differ"
    );

    // And it is Ada's, not Grace's: page two must not match it.
    let page_two = whole.render_gray(1, DPI).expect("it should draw");
    let same_as_two = page_two
        .gray
        .iter()
        .zip(alone.gray.iter())
        .filter(|(a, b)| a != b)
        .count();
    assert!(same_as_two > 0, "every page of the stack is identical");
}

/// A run of one sheet already is its own proof, so a second copy of it under
/// another name is clutter rather than help.
#[test]
fn a_run_of_one_needs_no_proof_of_itself() {
    let office = Office::new();
    let list = office.a_list("people.csv", "name\nAda\n");
    let (ok, said, _) = office.a_run("stack.pdf", &["--from", &at(&list), "--proof-first"]);
    assert!(ok, "{said}");
    assert!(
        !office.dir.path().join("stack-first.pdf").exists(),
        "{said}"
    );
}

/// A rehearsal writes nothing, and that has to include the proof — otherwise
/// --dry-run leaves a file behind, which is the one thing it promises not to.
#[test]
fn a_rehearsal_leaves_no_proof_behind_either() {
    let office = Office::new();
    let list = office.a_list("people.csv", "name\nAda\nGrace\nAlan\n");
    let (ok, said, stack) = office.a_run(
        "stack.pdf",
        &["--from", &at(&list), "--proof-first", "--dry-run"],
    );
    assert!(ok, "{said}");
    assert!(!stack.exists(), "{said}");
    assert!(
        !office.dir.path().join("stack-first.pdf").exists(),
        "{said}"
    );
}
