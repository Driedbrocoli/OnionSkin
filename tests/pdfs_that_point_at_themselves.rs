//! `join` and `merge`, over documents whose insides are awkward.
//!
//! These two are the only places Onionskin copies objects out of one PDF into
//! another, and a PDF is a graph rather than a list: a page points at its
//! resources, which point at fonts, which may point back. Copying that needs
//! every object renumbered, every reference followed, and every object already
//! copied recognised so a cycle does not go round for ever.
//!
//! It is the part of this program with the worst history — the same code has
//! had a cycle that never terminated and a remapping that lost a page's
//! resources — so it is worth more than a single happy case. What is asked here
//! is that the pages all arrive, that they are the pages that went in, and that
//! the result opens.

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

    /// A printed document of so many pages, each with its own number on it.
    fn a_document(&self, name: &str, pages: usize, paper: &str) -> PathBuf {
        let document = self.at(&format!("{name}.osk"));
        let printed = self.at(&format!("{name}.pdf"));
        let pages_text = pages.to_string();
        let (ok, said) = run(
            &self.home,
            &[
                "new",
                &at(&document),
                "--page",
                paper,
                "--pages",
                &pages_text,
            ],
        );
        assert!(ok, "{said}");
        for page in 1..=pages {
            let (ok, said) = run(
                &self.home,
                &[
                    "write",
                    &at(&document),
                    "--page",
                    &page.to_string(),
                    "--at",
                    &format!("20,30:{name} page {page}"),
                ],
            );
            assert!(ok, "{said}");
        }
        let (ok, said) = run(&self.home, &["print", &at(&document), "-o", &at(&printed)]);
        assert!(ok, "{said}");
        printed
    }
}

/// How many pages a PDF has, read by something other than the code that wrote
/// it.
fn pages_in(pdf: &Path) -> usize {
    lopdf::Document::load(pdf)
        .unwrap_or_else(|why| panic!("{} will not open: {why}", pdf.display()))
        .get_pages()
        .len()
}

/// Every page has ink on it, drawn rather than assumed.
///
/// A join that loses a page's resources produces a document of the right length
/// whose pages are blank — which passes a page count and fails on paper.
fn every_page_has_ink(pdf: &Path) {
    let engine = onionskin::render::engine().expect("a renderer");
    let document = engine.open(pdf).expect("the joined document should open");
    for page in 0..document.len() {
        let drawn = document.render_gray(page, 72.0).expect("it should draw");
        assert!(
            drawn.gray.iter().any(|level| *level < 200),
            "page {} of {} came out blank",
            page + 1,
            pdf.display()
        );
    }
}

/// The same file joined to itself, over and over.
///
/// Every object in it is copied afresh each time, and each copy has to be told
/// from the last. A remapping that reuses a number silently makes the second
/// copy of a page the same object as the first, and eight pages come back as
/// one page eight times over — or as four.
#[test]
fn a_file_joined_to_itself_comes_back_as_many_pages_as_went_in() {
    let work = Work::new();
    let one = work.a_document("one", 2, "a4");

    for copies in [2usize, 3, 8] {
        let out = work.at(&format!("joined{copies}.pdf"));
        let mut args = vec!["join".to_string()];
        for _ in 0..copies {
            args.push(at(&one));
        }
        args.push("-o".into());
        args.push(at(&out));
        let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
        let (ok, said) = run(&work.home, &borrowed);
        assert!(ok, "{copies} copies: {said}");

        assert_eq!(
            pages_in(&out),
            copies * 2,
            "{copies} copies of a two-page file should be {} pages",
            copies * 2
        );
        every_page_has_ink(&out);
    }
}

/// A join of a join of a join. Each round copies objects that were themselves
/// copies, which is where a remapping that is nearly right stops being nearly
/// right.
#[test]
fn joining_what_was_already_joined_still_keeps_every_page() {
    let work = Work::new();
    let a = work.a_document("a", 1, "a4");
    let b = work.a_document("b", 1, "a4");

    let mut latest = work.at("round0.pdf");
    let (ok, said) = run(&work.home, &["join", &at(&a), &at(&b), "-o", &at(&latest)]);
    assert!(ok, "{said}");

    for round in 1..=4 {
        let next = work.at(&format!("round{round}.pdf"));
        let (ok, said) = run(
            &work.home,
            &["join", &at(&latest), &at(&a), "-o", &at(&next)],
        );
        assert!(ok, "round {round}: {said}");
        assert_eq!(
            pages_in(&next),
            2 + round,
            "round {round} lost or gained a page"
        );
        every_page_has_ink(&next);
        latest = next;
    }
}

/// Mixed paper, which is the case `join` undertakes to handle and `merge`
/// refuses. Every page has to keep its own size through the copy.
#[test]
fn every_page_keeps_its_own_paper_through_a_join() {
    let work = Work::new();
    let a4 = work.a_document("small", 1, "a4");
    let wide = work.a_document("wide", 1, "297x210");
    let letter = work.a_document("us", 1, "letter");

    let out = work.at("mixed.pdf");
    let (ok, said) = run(
        &work.home,
        &["join", &at(&a4), &at(&wide), &at(&letter), "-o", &at(&out)],
    );
    assert!(ok, "{said}");
    assert_eq!(pages_in(&out), 3);
    every_page_has_ink(&out);

    // The three shapes, measured off the joined file rather than trusted.
    let engine = onionskin::render::engine().expect("a renderer");
    let document = engine.open(&out).expect("it should open");
    let shape = |page: usize| {
        let drawn = document.render_gray(page, 72.0).expect("it should draw");
        (drawn.width > drawn.height, drawn.width, drawn.height)
    };
    assert!(!shape(0).0, "the A4 page came out landscape");
    assert!(shape(1).0, "the landscape page came out upright");
    assert!(!shape(2).0, "the Letter page came out landscape");
    assert_ne!(
        shape(0).1,
        shape(2).1,
        "A4 and Letter came out the same width, so a size was not kept"
    );
}

/// Merging a delta with itself, which is the case `merge` points out rather
/// than refuses — and having pointed it out, it still has to produce a file
/// that opens and carries the ink twice.
#[test]
fn a_delta_merged_with_itself_is_pointed_out_and_still_works() {
    let work = Work::new();
    let sheet = work.a_document("sheet", 1, "a4");

    let delta = work.at("stamp.pdf");
    let (ok, said) = run(
        &work.home,
        &[
            "write",
            &at(&sheet),
            "--at",
            "40,120:Approved",
            "-o",
            &at(&delta),
        ],
    );
    assert!(ok, "{said}");

    let out = work.at("twice.pdf");
    let (ok, said) = run(
        &work.home,
        &["merge", &at(&delta), &at(&delta), "-o", &at(&out)],
    );
    assert!(ok, "{said}");
    assert!(
        said.to_lowercase().contains("twice") || said.to_lowercase().contains("same"),
        "the same delta twice was not pointed out: {said}"
    );
    assert_eq!(pages_in(&out), 1);
    every_page_has_ink(&out);
}
