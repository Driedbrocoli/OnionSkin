use super::*;

fn look(size: u64, modified: u64) -> Look {
    Look { size, modified }
}

fn seen(name: &str, size: u64, modified: u64) -> Seen {
    Seen {
        path: PathBuf::from(name),
        look: look(size, modified),
    }
}

/// A file has to look the same twice before anything is done to it, because
/// while the scanner is writing it there is a file of that name holding half a
/// document.
#[test]
fn a_file_still_growing_is_left_alone() {
    // First sight of it: nothing to compare against.
    assert!(!settled(None, look(1_000, 100)));
    // Still growing.
    assert!(!settled(Some(look(1_000, 100)), look(4_000, 101)));
    // Same length, but touched again — a rewrite in place.
    assert!(!settled(Some(look(4_000, 101)), look(4_000, 102)));
    // Twice running, identical. Now it can be opened.
    assert!(settled(Some(look(4_000, 102)), look(4_000, 102)));
}

/// The name is created before the bytes arrive, so nought bytes means the
/// interesting part is still on its way however long it sits there.
#[test]
fn an_empty_file_is_never_finished() {
    assert!(!settled(Some(look(0, 100)), look(0, 100)));
    assert!(!settled(None, look(0, 100)));
    // One byte is enough to be a file that has stopped changing. Whether it is
    // a usable PDF is a different question, asked by whatever opens it.
    assert!(settled(Some(look(1, 100)), look(1, 100)));
}

/// The mistake that would fill a disk: a delta treated as a fresh document,
/// given a delta of its own, two seconds apart, forever.
#[test]
fn onionskin_does_not_work_on_its_own_output() {
    assert_eq!(
        worth_opening(Path::new("/scans/invoice-delta.pdf")),
        Err(Leave::OurOwn)
    );
    for tail in OUR_TAILS {
        let name = format!("/scans/march{tail}.pdf");
        assert_eq!(
            worth_opening(Path::new(&name)),
            Err(Leave::OurOwn),
            "{name} was not recognised as something Onionskin wrote"
        );
    }
    // The delta of a delta would be named by the same rule, which is what
    // makes the loop possible and the check necessary.
    let once = where_the_delta_goes(Path::new("/scans/invoice.pdf"), None);
    assert_eq!(once, PathBuf::from("/scans/invoice-delta.pdf"));
    assert_eq!(worth_opening(&once), Err(Leave::OurOwn));
}

/// The document itself is still worked on, tails or no tails.
#[test]
fn an_ordinary_document_is_opened() {
    assert_eq!(worth_opening(Path::new("/scans/invoice.pdf")), Ok(()));
    assert_eq!(worth_opening(Path::new("/scans/Scan_0007.PDF")), Ok(()));
    assert_eq!(worth_opening(Path::new("/scans/letter.docx")), Ok(()));
    assert_eq!(worth_opening(Path::new("/scans/notes.odt")), Ok(()));
    // A name that merely contains a tail in the middle is not our output.
    assert_eq!(worth_opening(Path::new("/scans/delta-force.pdf")), Ok(()));
    assert_eq!(worth_opening(Path::new("/scans/proof-of-post.pdf")), Ok(()));
}

/// Everything a folder collects that is not a document.
#[test]
fn the_things_a_folder_collects_are_passed_over() {
    let cases: &[(&str, Leave)] = &[
        ("/scans/.DS_Store", Leave::Hidden),
        ("/scans/.hidden.pdf", Leave::Hidden),
        ("/scans/~$report.docx", Leave::Hidden),
        ("/scans/scan.pdf.part", Leave::HalfWritten),
        ("/scans/scan.pdf.crdownload", Leave::HalfWritten),
        ("/scans/scan.pdf.tmp", Leave::HalfWritten),
        ("/scans/Scan_0007.jpg", Leave::APicture),
        ("/scans/Scan_0007.tiff", Leave::APicture),
        ("/scans/notes.zip", Leave::NotADocument("zip".to_string())),
        ("/scans/README", Leave::Nameless),
    ];
    for (name, expected) in cases {
        assert_eq!(
            worth_opening(Path::new(name)),
            Err(expected.clone()),
            "{name}"
        );
    }
    // Every reason says something a person could act on.
    for (_, reason) in cases {
        assert!(!reason.why().is_empty());
    }
    // A scanned picture is the one worth pointing somewhere, since the folder
    // it lands in is exactly the folder somebody would watch.
    assert!(Leave::APicture.why().contains("onionskin read"));
}

/// A file dealt with last week is not started again just because the program
/// has been restarted and has never seen it settle.
#[test]
fn a_restart_does_not_do_the_folder_again() {
    let file = seen("/scans/invoice.pdf", 4_000, 900);
    let mut done = Ledger::new();
    done.add(Handled {
        at: 1_000,
        source: "/scans/invoice.pdf".to_string(),
        look: look(4_000, 900),
        delta: "/scans/invoice-delta.pdf".to_string(),
        trouble: String::new(),
    });

    // First sweep after a restart: nothing was seen before, so the settle
    // check alone would say "still arriving" and the sweep after would do it
    // a second time. The record is asked first.
    assert_eq!(what_to_do(&file, None, &done), Verdict::DoneBefore(1_000));
    // And on every sweep after that.
    assert_eq!(
        what_to_do(&file, Some(look(4_000, 900)), &done),
        Verdict::DoneBefore(1_000)
    );
}

/// Scanned again over the top of the old one, and it is a new sheet of paper
/// wanting a new delta.
#[test]
fn the_same_name_with_new_contents_is_a_new_job() {
    let mut done = Ledger::new();
    done.add(Handled {
        at: 1_000,
        source: "/scans/invoice.pdf".to_string(),
        look: look(4_000, 900),
        delta: "/scans/invoice-delta.pdf".to_string(),
        trouble: String::new(),
    });

    let again = seen("/scans/invoice.pdf", 4_100, 2_000);
    // Not known, so it has to settle first.
    assert_eq!(what_to_do(&again, None, &done), Verdict::StillArriving);
    assert_eq!(
        what_to_do(&again, Some(look(4_100, 2_000)), &done),
        Verdict::Do
    );
}

/// A PDF that cannot be opened will not open on the next sweep either, and the
/// folder should not produce the same error every two seconds until somebody
/// notices.
#[test]
fn something_that_failed_is_not_retried_forever() {
    let file = seen("/scans/torn.pdf", 900, 500);
    let mut done = Ledger::new();
    done.add(Handled {
        at: 1_000,
        source: "/scans/torn.pdf".to_string(),
        look: look(900, 500),
        delta: String::new(),
        trouble: "not a PDF: the file starts with 'MZ'".to_string(),
    });

    let verdict = what_to_do(&file, Some(look(900, 500)), &done);
    assert!(matches!(verdict, Verdict::FailedBefore(_)));
    let Verdict::FailedBefore(trouble) = verdict else {
        panic!("expected a remembered failure");
    };
    // The reason is kept, so it can be said once rather than never.
    assert!(trouble.contains("not a PDF"));
    assert!(!done.all()[0].worked());
    assert_eq!(done.tally(), (0, 1));
}

/// A whole sweep, with one of everything in the folder.
#[test]
fn a_sweep_sorts_the_folder_into_what_to_do_and_what_not_to() {
    let now = vec![
        seen("/scans/.DS_Store", 6_148, 1),
        seen("/scans/arriving.pdf", 2_000, 100),
        seen("/scans/invoice-delta.pdf", 900, 90),
        seen("/scans/invoice.pdf", 4_000, 80),
        seen("/scans/ready.pdf", 3_000, 70),
    ];
    let mut before = BTreeMap::new();
    before.insert(PathBuf::from("/scans/arriving.pdf"), look(1_000, 99));
    before.insert(PathBuf::from("/scans/ready.pdf"), look(3_000, 70));
    before.insert(PathBuf::from("/scans/invoice.pdf"), look(4_000, 80));

    let mut done = Ledger::new();
    done.add(Handled {
        at: 500,
        source: "/scans/invoice.pdf".to_string(),
        look: look(4_000, 80),
        delta: "/scans/invoice-delta.pdf".to_string(),
        trouble: String::new(),
    });

    let verdicts = decide(&now, &before, &done);
    let by_name: Vec<(String, Verdict)> = verdicts
        .iter()
        .map(|(seen, verdict)| {
            (
                seen.path
                    .file_name()
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
                verdict.clone(),
            )
        })
        .collect();

    assert_eq!(
        by_name,
        vec![
            (".DS_Store".to_string(), Verdict::Leave(Leave::Hidden)),
            ("arriving.pdf".to_string(), Verdict::StillArriving),
            (
                "invoice-delta.pdf".to_string(),
                Verdict::Leave(Leave::OurOwn)
            ),
            ("invoice.pdf".to_string(), Verdict::DoneBefore(500)),
            ("ready.pdf".to_string(), Verdict::Do),
        ]
    );

    let tally = Tally::of(&verdicts);
    assert_eq!(
        tally,
        Tally {
            done: 1,
            failed: 0,
            arriving: 1,
            already: 1,
            left: 2,
        }
    );
    assert_eq!(tally.line().as_deref(), Some("1 to do, 1 still arriving"));
    // Exactly one thing in that folder is work.
    assert_eq!(verdicts.iter().filter(|(_, v)| v.is_work()).count(), 1);
}

/// Most sweeps see a folder where nothing has changed, and a program that says
/// so every two seconds buries the sweeps where something did.
#[test]
fn a_quiet_sweep_says_nothing() {
    let quiet = Tally {
        done: 0,
        failed: 0,
        arriving: 0,
        already: 12,
        left: 30,
    };
    assert_eq!(quiet.line(), None);
    // Something arriving is worth a line even before it can be worked on: it
    // is the answer to "is it seeing my scanner at all".
    let arriving = Tally {
        arriving: 1,
        ..quiet.clone()
    };
    assert_eq!(arriving.line().as_deref(), Some("1 still arriving"));
}

/// What this sweep saw is what the next sweep compares against.
#[test]
fn each_sweep_hands_the_next_one_what_it_saw() {
    let now = vec![seen("/scans/a.pdf", 10, 1), seen("/scans/b.pdf", 20, 2)];
    let remembered = remember_looks(&now);
    assert_eq!(remembered.len(), 2);
    assert_eq!(remembered[&PathBuf::from("/scans/a.pdf")], look(10, 1));

    // Which is enough for the second sweep to call an unchanged file settled.
    assert_eq!(
        what_to_do(
            &now[0],
            remembered.get(&PathBuf::from("/scans/a.pdf")).copied(),
            &Ledger::new()
        ),
        Verdict::Do
    );
}

/// Beside it, or into a folder of their own.
#[test]
fn the_delta_lands_where_it_can_be_found() {
    assert_eq!(
        where_the_delta_goes(Path::new("/scans/Scan_0007.pdf"), None),
        PathBuf::from("/scans/Scan_0007-delta.pdf")
    );
    // A Word file still gets a PDF, because that is what a printer takes.
    assert_eq!(
        where_the_delta_goes(Path::new("/scans/letter.docx"), None),
        PathBuf::from("/scans/letter-delta.pdf")
    );
    assert_eq!(
        where_the_delta_goes(Path::new("/scans/a.pdf"), Some(Path::new("/out"))),
        PathBuf::from("/out/a-delta.pdf")
    );
    // A bare name keeps a bare name rather than growing a "./".
    assert_eq!(
        where_the_delta_goes(Path::new("a.pdf"), None),
        PathBuf::from("a-delta.pdf")
    );
}

/// Two folders keep two records, so forgetting one is deleting one file.
#[test]
fn each_folder_has_its_own_record() {
    let _home = crate::calibrate::borrow_home(Path::new("/tmp/onionskin-watch-paths"));
    let one = ledger_path(Path::new("/scans/incoming"));
    let other = ledger_path(Path::new("/scans/outgoing"));
    assert_ne!(one, other);
    assert_eq!(one, ledger_path(Path::new("/scans/incoming")));
    assert_eq!(one.extension().and_then(|e| e.to_str()), Some("jsonl"));
}

/// `watch .` and `watch /home/j/Scans` are the same folder, and must not each
/// do the work.
#[test]
fn the_same_folder_under_two_names_is_one_folder() {
    let kept = tempfile::tempdir().unwrap();
    let folder = kept.path().to_path_buf();
    let home = folder.join("home");
    let _home = crate::calibrate::borrow_home(&home);

    let scans = folder.join("scans");
    std::fs::create_dir_all(&scans).unwrap();
    let long = ledger_path(&scans);
    let round_about = ledger_path(&folder.join("scans").join(".").join("..").join("scans"));
    assert_eq!(long, round_about);

    // And the same for the files inside it.
    let file = scans.join("invoice.pdf");
    std::fs::write(&file, b"%PDF-1.4\n").unwrap();
    let mut done = Ledger::new();
    done.add(Handled {
        at: 1,
        source: file.to_string_lossy().into_owned(),
        look: look_at(&file).unwrap(),
        delta: String::new(),
        trouble: String::new(),
    });
    let by_another_name = scans.join(".").join("invoice.pdf");
    assert!(done
        .knows(&by_another_name, look_at(&file).unwrap())
        .is_some());
}

/// The record survives the program being stopped, which is how it is always
/// stopped.
#[test]
fn what_was_done_is_still_known_after_a_restart() {
    let kept = tempfile::tempdir().unwrap();
    let folder = kept.path().to_path_buf();
    let home = folder.join("home");
    let _home = crate::calibrate::borrow_home(&home);

    let scans = folder.join("scans");
    std::fs::create_dir_all(&scans).unwrap();
    assert!(read_ledger(&scans).is_empty());

    let file = scans.join("invoice.pdf");
    std::fs::write(&file, b"%PDF-1.4\n1 0 obj\n").unwrap();
    let seen_now = Seen {
        look: look_at(&file).unwrap(),
        path: file.clone(),
    };

    // Nothing known: it has to settle, then it is work.
    assert_eq!(
        what_to_do(&seen_now, None, &read_ledger(&scans)),
        Verdict::StillArriving
    );
    assert_eq!(
        what_to_do(&seen_now, Some(seen_now.look), &read_ledger(&scans)),
        Verdict::Do
    );

    write_down(
        &scans,
        &Handled {
            at: 1_234,
            source: file.to_string_lossy().into_owned(),
            look: seen_now.look,
            delta: where_the_delta_goes(&file, None)
                .to_string_lossy()
                .into_owned(),
            trouble: String::new(),
        },
    )
    .unwrap();

    // Read back from disk, as a fresh run of the program would.
    let after = read_ledger(&scans);
    assert_eq!(after.len(), 1);
    assert_eq!(after.tally(), (1, 0));
    assert_eq!(
        what_to_do(&seen_now, Some(seen_now.look), &after),
        Verdict::DoneBefore(1_234)
    );

    // And forgetting it puts the work back.
    assert!(forget(&scans));
    assert!(read_ledger(&scans).is_empty());
    assert_eq!(
        what_to_do(&seen_now, Some(seen_now.look), &read_ledger(&scans)),
        Verdict::Do
    );
}

/// A line from a version that kept something else costs that line, not the
/// record.
#[test]
fn a_line_it_cannot_read_does_not_lose_the_rest() {
    let kept = tempfile::tempdir().unwrap();
    let folder = kept.path().to_path_buf();
    let home = folder.join("home");
    let _home = crate::calibrate::borrow_home(&home);
    let scans = folder.join("scans");
    std::fs::create_dir_all(&scans).unwrap();

    for name in ["a.pdf", "b.pdf"] {
        write_down(
            &scans,
            &Handled {
                at: 1,
                source: scans.join(name).to_string_lossy().into_owned(),
                look: look(10, 10),
                delta: String::new(),
                trouble: String::new(),
            },
        )
        .unwrap();
    }
    // Something a later version wrote, in between.
    let path = ledger_path(&scans);
    let text = std::fs::read_to_string(&path).unwrap();
    let mut lines: Vec<&str> = text.lines().collect();
    lines.insert(1, "{\"this\":\"is from a version that kept other things\"}");
    lines.insert(2, "not even json");
    std::fs::write(&path, lines.join("\n")).unwrap();

    let ledger = read_ledger(&scans);
    assert_eq!(
        ledger.len(),
        2,
        "the two readable lines should still be there"
    );
}

/// The listing is what the folder actually holds, in an order that does not
/// change between sweeps.
#[test]
fn the_listing_is_the_files_and_only_the_files() {
    let kept = tempfile::tempdir().unwrap();
    let folder = kept.path().to_path_buf();
    let scans = folder.join("scans");
    std::fs::create_dir_all(scans.join("subfolder")).unwrap();
    for name in ["b.pdf", "a.pdf", "c.docx"] {
        std::fs::write(scans.join(name), b"x").unwrap();
    }

    let listed = listing(&scans).unwrap();
    let names: Vec<String> = listed
        .iter()
        .map(|seen| {
            seen.path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    // Sorted, and the folder is not one of them: a scanner writes into the
    // folder it was pointed at, and walking a tree turns "watch my scans" into
    // "walk my home directory every two seconds".
    assert_eq!(names, vec!["a.pdf", "b.pdf", "c.docx"]);
    assert!(listed.iter().all(|seen| seen.look.size == 1));

    // Twice running gives the same answer, which is what the settle check
    // depends on.
    assert_eq!(listing(&scans).unwrap(), listed);
}

/// Pointed at something that is not a folder, it says so rather than sitting
/// there watching nothing.
#[test]
fn a_folder_that_is_not_there_is_said_at_once() {
    let kept = tempfile::tempdir().unwrap();
    let folder = kept.path().to_path_buf();
    let missing = folder.join("nowhere");
    let Err(WatchError::NoFolder(said)) = listing(&missing) else {
        panic!("watching a folder that is not there should say so");
    };
    assert_eq!(said, missing);

    let file = folder.join("invoice.pdf");
    std::fs::write(&file, b"x").unwrap();
    let Err(WatchError::NotAFolder(said)) = listing(&file) else {
        panic!("watching a file should say it is a file");
    };
    assert_eq!(said, file);
    assert!(WatchError::NotAFolder(file).to_string().contains("folder"));
}

/// The record does not grow without limit in somebody's home directory.
#[test]
fn the_record_is_trimmed_rather_than_growing_forever() {
    let kept = tempfile::tempdir().unwrap();
    let folder = kept.path().to_path_buf();
    let home = folder.join("home");
    let _home = crate::calibrate::borrow_home(&home);
    let scans = folder.join("scans");
    std::fs::create_dir_all(&scans).unwrap();

    let path = ledger_path(&scans);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let text: String = (0..KEEP)
        .map(|n| {
            let handled = Handled {
                at: n as u64,
                source: scans
                    .join(format!("{n}.pdf"))
                    .to_string_lossy()
                    .into_owned(),
                look: look(10, n as u64),
                delta: String::new(),
                trouble: String::new(),
            };
            format!("{}\n", serde_json::to_string(&handled).unwrap())
        })
        .collect();
    std::fs::write(&path, text).unwrap();

    write_down(
        &scans,
        &Handled {
            at: 99_999,
            source: scans.join("newest.pdf").to_string_lossy().into_owned(),
            look: look(10, 10),
            delta: String::new(),
            trouble: String::new(),
        },
    )
    .unwrap();

    let after = read_ledger(&scans);
    assert!(after.len() <= KEEP / 2 + 1, "kept {} lines", after.len());
    // The newest is what survives, because the oldest files are the ones least
    // likely to still be in the folder.
    assert!(after
        .all()
        .iter()
        .any(|had| had.source.ends_with("newest.pdf")));
    assert!(!after.all().iter().any(|had| had.source.ends_with("/0.pdf")));
}
