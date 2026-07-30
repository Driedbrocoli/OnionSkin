//! `onionskin watch`, driven the way an office would leave it running.
//!
//! The thing worth testing is not that a delta comes out — `job run` already
//! proves that. It is the three ways a folder full of files goes wrong when
//! nobody is watching it:
//!
//!   * the delta gets a delta of its own, and then that one does, until the
//!     disk is full;
//!   * a file is opened while the scanner is still writing it;
//!   * the program is restarted, and every sheet in the folder gets a second
//!     delta printed onto it, which cannot be undone.
//!
//! So the tests here run the real binary against a real folder, put files into
//! it the way a scanner does, and look at what is on disk afterwards.

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
    scans: PathBuf,
    /// A printed sheet, to copy into the folder as though it were scanned.
    sheet: PathBuf,
}

impl Office {
    /// A home, a scans folder, a printed sheet, and a saved job to run on it.
    fn new() -> Office {
        let dir = tempfile::tempdir().expect("a place to work");
        let home = dir.path().join("home");
        let scans = dir.path().join("scans");
        std::fs::create_dir_all(&home).expect("a home of its own");
        std::fs::create_dir_all(&scans).expect("somewhere to scan into");

        let document = dir.path().join("invoice.osk");
        let sheet = dir.path().join("printed.pdf");
        let scratch = dir.path().join("scratch.pdf");
        for args in [
            vec!["new", &at(&document), "--page", "a4"],
            vec![
                "write",
                &at(&document),
                "--at",
                "20,30:Invoice no: 4471",
                "--at",
                "20,45:Total: 92.00",
            ],
            vec!["print", &at(&document), "-o", &at(&sheet)],
            // The job the office runs every morning. Saved off the printed
            // sheet rather than the document, because that is what it will be
            // run on.
            vec![
                "write",
                &at(&sheet),
                "--at",
                "150,80:PAID {today}",
                "--size",
                "9",
                "--save-as",
                "paid",
                "-o",
                &at(&scratch),
            ],
        ] {
            let (ok, said) = run(&home, &args);
            assert!(ok, "setting up: {said}");
        }

        Office {
            dir,
            home,
            scans,
            sheet,
        }
    }

    /// A sheet lands in the folder, whole, the way a finished scan does.
    fn scan_arrives(&self, name: &str) -> PathBuf {
        let landing = self.scans.join(name);
        std::fs::copy(&self.sheet, &landing).expect("the scan should land");
        landing
    }

    /// One sweep of the folder, as a scheduled task would run it.
    fn sweep(&self, extra: &[&str]) -> (bool, String) {
        let mut args = vec![
            "watch".to_string(),
            at(&self.scans),
            "--job".to_string(),
            "paid".to_string(),
            "--once".to_string(),
            // One second rather than two, because `--once` has to look twice
            // to know a file has settled and the test waits for both.
            "--every".to_string(),
            "1".to_string(),
        ];
        args.extend(extra.iter().map(|s| s.to_string()));
        let borrowed: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        run(&self.home, &borrowed)
    }

    /// Everything in the scans folder now, by name, sorted.
    fn folder(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(&self.scans)
            .expect("the folder should be readable")
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    fn deltas(&self) -> Vec<String> {
        self.folder()
            .into_iter()
            .filter(|name| name.contains("-delta"))
            .collect()
    }
}

/// The whole feature in one test: a scan lands, a delta appears beside it.
#[test]
fn a_scan_lands_and_a_delta_appears_beside_it() {
    let office = Office::new();
    office.scan_arrives("Scan_0007.pdf");

    let (ok, said) = office.sweep(&[]);
    assert!(ok, "{said}");
    assert_eq!(
        office.folder(),
        vec!["Scan_0007-delta.pdf", "Scan_0007.pdf"],
        "{said}"
    );

    // And it is a real delta: one addition, on a page of the same size.
    assert!(said.contains("1 addition"), "{said}");
    let delta = office.scans.join("Scan_0007-delta.pdf");
    let engine = onionskin::render::engine().expect("a renderer");
    let opened = engine.open(&delta).expect("the delta should open");
    assert_eq!(opened.len(), 1);
}

/// The one that fills a disk: the delta is a PDF in the same folder, so a
/// program that is not careful gives it a delta, and gives that one a delta,
/// two seconds apart, forever.
#[test]
fn a_delta_never_gets_a_delta_of_its_own() {
    let office = Office::new();
    office.scan_arrives("Scan_0007.pdf");

    for round in 1..=3 {
        let (ok, said) = office.sweep(&[]);
        assert!(ok, "round {round}: {said}");
        assert_eq!(
            office.deltas(),
            vec!["Scan_0007-delta.pdf"],
            "round {round} — the folder grew: {said}"
        );
    }
    // Which is the same as saying the second sweep found nothing to do.
    let (_, said) = office.sweep(&[]);
    assert!(said.contains("0 done now"), "{said}");
    assert!(said.contains("1 done before"), "{said}");
}

/// Restarting the program is not a reason to print a second delta onto every
/// sheet in the building. Toner does not come off paper.
#[test]
fn a_restart_does_not_redo_the_whole_folder() {
    let office = Office::new();
    for name in ["Scan_0007.pdf", "Scan_0008.pdf", "Scan_0009.pdf"] {
        office.scan_arrives(name);
    }

    let (ok, said) = office.sweep(&[]);
    assert!(ok, "{said}");
    assert!(said.contains("3 done now"), "{said}");

    // Each sweep is a separate run of the program — a restart, three times
    // over. The record is on disk, so it survives them.
    for round in 1..=3 {
        let (ok, said) = office.sweep(&[]);
        assert!(ok, "round {round}: {said}");
        assert!(
            said.contains("0 done now") && said.contains("3 done before"),
            "round {round}: {said}"
        );
    }
    assert_eq!(office.deltas().len(), 3);

    // Asked outright, it does them all again — which is the escape hatch for
    // the delta somebody deleted.
    let (ok, said) = office.sweep(&["--again"]);
    assert!(ok, "{said}");
    assert!(said.contains("3 done now"), "{said}");
}

/// A scanner writing a ten-megabyte PDF over the network does it over several
/// seconds, and for most of them there is a file of that name holding half a
/// document.
#[test]
fn a_file_still_arriving_is_left_where_it_is() {
    let office = Office::new();
    let whole = std::fs::read(&office.sheet).expect("the sheet should read");
    let landing = office.scans.join("Scan_0007.pdf");

    // Half of it, and then — while the sweep is between its two looks — the
    // rest. A program that opened it on sight would open half a PDF.
    std::fs::write(&landing, &whole[..whole.len() / 2]).expect("half of it should write");
    let growing = std::thread::spawn({
        let landing = landing.clone();
        move || {
            std::thread::sleep(std::time::Duration::from_millis(400));
            std::fs::write(&landing, &whole).expect("the rest should write");
        }
    });
    let (ok, said) = office.sweep(&[]);
    growing.join().expect("the writer should finish");
    assert!(ok, "{said}");

    // Nothing was done to it: it changed between the two looks.
    assert!(said.contains("still arriving"), "{said}");
    assert!(office.deltas().is_empty(), "{said}");

    // Now it has stopped changing, and the next sweep does it properly.
    let (ok, said) = office.sweep(&[]);
    assert!(ok, "{said}");
    assert_eq!(office.deltas(), vec!["Scan_0007-delta.pdf"], "{said}");
}

/// A folder collects hidden files, half-written downloads, pictures and
/// zips, and the only two worth mentioning are the ones somebody might have
/// expected to work.
#[test]
fn the_rest_of_what_a_folder_collects_is_left_alone() {
    let office = Office::new();
    office.scan_arrives("Scan_0007.pdf");
    for name in [".DS_Store", "half.pdf.part", "~$notes.docx", "archive.zip"] {
        std::fs::write(office.scans.join(name), b"whatever").expect("it should write");
    }
    std::fs::copy(&office.sheet, office.scans.join("photo.jpg")).expect("a picture should land");

    let (ok, said) = office.sweep(&[]);
    assert!(ok, "{said}");
    assert_eq!(office.deltas(), vec!["Scan_0007-delta.pdf"], "{said}");

    // Said: the two somebody might have expected to work.
    assert!(said.contains("archive.zip"), "{said}");
    assert!(said.contains("photo.jpg"), "{said}");
    // And the picture is told where to go instead, since a scans folder is
    // exactly where scanned pictures land.
    assert!(said.contains("onionskin read"), "{said}");
    // Not said: the operating system's own droppings, which every folder has
    // and nobody wants a commentary on.
    assert!(!said.contains(".DS_Store"), "{said}");
    assert!(!said.contains("~$notes.docx"), "{said}");
}

/// `--dry-run` says what it would do and writes nothing, including nothing to
/// the record — so taking the flag off does the work.
#[test]
fn a_dry_run_writes_nothing_and_forgets_nothing() {
    let office = Office::new();
    office.scan_arrives("Scan_0007.pdf");

    let (ok, said) = office.sweep(&["--dry-run"]);
    assert!(ok, "{said}");
    assert!(said.contains("nothing written"), "{said}");
    assert_eq!(office.folder(), vec!["Scan_0007.pdf"], "{said}");
    assert!(said.contains("1 to do"), "{said}");

    // Nothing was written down either, so the real sweep still has work.
    let (ok, said) = office.sweep(&[]);
    assert!(ok, "{said}");
    assert_eq!(office.deltas(), vec!["Scan_0007-delta.pdf"], "{said}");
}

/// The deltas can go somewhere else, so a scans folder stays a scans folder.
#[test]
fn the_deltas_can_go_into_a_folder_of_their_own() {
    let office = Office::new();
    office.scan_arrives("Scan_0007.pdf");
    let out = office.dir.path().join("to-print");
    std::fs::create_dir_all(&out).expect("somewhere for them to go");

    let (ok, said) = office.sweep(&["--into", &at(&out)]);
    assert!(ok, "{said}");
    assert_eq!(office.folder(), vec!["Scan_0007.pdf"], "{said}");
    assert!(out.join("Scan_0007-delta.pdf").is_file(), "{said}");
}

/// Sitting silently for an hour on a misspelled job name or a folder that is
/// not there is the one failure nobody notices until they go looking for the
/// deltas that were never made.
#[test]
fn a_wrong_name_is_said_at_once_rather_than_watched_in_silence() {
    let office = Office::new();
    let missing = office.dir.path().join("nowhere");

    let (ok, said) = run(
        &office.home,
        &["watch", &at(&office.scans), "--job", "pied", "--once"],
    );
    assert!(!ok, "a job that does not exist should stop it: {said}");
    assert!(said.contains("pied"), "{said}");
    // And it says what does exist, since the usual cause is a typo.
    assert!(said.contains("paid"), "{said}");

    let (ok, said) = run(
        &office.home,
        &["watch", &at(&missing), "--job", "paid", "--once"],
    );
    assert!(!ok, "a folder that is not there should stop it: {said}");
    assert!(said.contains("nowhere"), "{said}");

    // A file is not a folder, and watching one would wait forever.
    let (ok, said) = run(
        &office.home,
        &["watch", &at(&office.sheet), "--job", "paid", "--once"],
    );
    assert!(!ok, "watching a file should stop it: {said}");
    assert!(said.contains("folder"), "{said}");

    // And --into has to exist too, or a whole afternoon's deltas go nowhere.
    let (ok, said) = run(
        &office.home,
        &[
            "watch",
            &at(&office.scans),
            "--job",
            "paid",
            "--once",
            "--into",
            &at(&missing),
        ],
    );
    assert!(!ok, "an --into that is not there should stop it: {said}");
    assert!(said.contains("not a folder"), "{said}");
}

/// A job with a blank in it is refused before anything is watched, rather than
/// producing an afternoon of sheets reading `{ref}`.
#[test]
fn a_job_with_an_unfilled_blank_is_refused_before_watching() {
    let office = Office::new();
    let scratch = office.dir.path().join("scratch2.pdf");
    let (ok, said) = run(
        &office.home,
        &[
            "write",
            &at(&office.sheet),
            "--at",
            "150,80:Ref {ref}",
            "--save-as",
            "reffed",
            "-o",
            &at(&scratch),
        ],
    );
    assert!(ok, "{said}");
    office.scan_arrives("Scan_0007.pdf");

    let (ok, said) = run(
        &office.home,
        &["watch", &at(&office.scans), "--job", "reffed", "--once"],
    );
    assert!(!ok, "{said}");
    assert!(said.contains("{ref}"), "{said}");
    assert!(
        office.deltas().is_empty(),
        "nothing should have been written"
    );

    // Given the value, it runs.
    let (ok, said) = run(
        &office.home,
        &[
            "watch",
            &at(&office.scans),
            "--job",
            "reffed",
            "--once",
            "--every",
            "1",
            "--set",
            "ref=4471",
        ],
    );
    assert!(ok, "{said}");
    assert_eq!(office.deltas(), vec!["Scan_0007-delta.pdf"], "{said}");
}

/// A file the job cannot be run on is tried once, said once, and then left
/// alone — rather than the same error every two seconds for the afternoon.
#[test]
fn something_that_will_never_work_is_not_tried_forever() {
    let office = Office::new();
    let torn = office.scans.join("torn.pdf");
    std::fs::write(&torn, b"%PDF-1.4\nthis is not a PDF at all\n").expect("it should write");

    let (ok, said) = office.sweep(&[]);
    assert!(
        !ok,
        "a file that cannot be opened should be reported: {said}"
    );
    assert!(said.contains("torn.pdf"), "{said}");
    assert!(said.contains("did not work"), "{said}");

    // The second sweep leaves it, and says so once.
    let (ok, said) = office.sweep(&[]);
    assert!(ok, "the second sweep has nothing to do: {said}");
    assert!(said.contains("left alone"), "{said}");
    assert!(said.contains("tried before"), "{said}");
    assert!(office.deltas().is_empty(), "{said}");
}

/// A sheet scanned again over the top of the old one is a new sheet of paper,
/// and wants a new delta.
#[test]
fn rescanning_a_page_gets_it_done_again() {
    let office = Office::new();
    let landing = office.scan_arrives("Scan_0007.pdf");

    let (ok, said) = office.sweep(&[]);
    assert!(ok, "{said}");
    assert!(said.contains("1 done now"), "{said}");
    let first = std::fs::metadata(office.scans.join("Scan_0007-delta.pdf"))
        .expect("the delta should be there")
        .len();

    // The same name, different contents — which is what pressing the scan
    // button twice with the same file name does.
    let mut longer = std::fs::read(&office.sheet).expect("the sheet should read");
    longer.extend_from_slice(b"\n% scanned again\n");
    std::fs::write(&landing, &longer).expect("it should write");

    let (ok, said) = office.sweep(&[]);
    assert!(ok, "{said}");
    assert!(
        said.contains("1 done now"),
        "a rescan should be done again: {said}"
    );
    assert!(
        std::fs::metadata(office.scans.join("Scan_0007-delta.pdf"))
            .unwrap()
            .len()
            > 0
    );
    assert!(first > 0);
}

/// The lines about how to print a delta are said once, not after every file.
#[test]
fn the_printing_instructions_are_not_repeated_for_every_sheet() {
    let office = Office::new();
    for name in ["Scan_0007.pdf", "Scan_0008.pdf", "Scan_0009.pdf"] {
        office.scan_arrives(name);
    }
    let (ok, said) = office.sweep(&[]);
    assert!(ok, "{said}");
    assert!(said.contains("3 done now"), "{said}");

    let times = said.matches("Printing the delta").count();
    assert_eq!(
        times, 1,
        "the twelve lines about the tray were said {times} times, burying the \
         one line per file that differs"
    );
}
