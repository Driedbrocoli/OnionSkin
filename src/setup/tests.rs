use super::*;

/// A machine of its own, with everything Onionskin keeps pointed into it.
fn a_machine() -> (tempfile::TempDir, std::sync::MutexGuard<'static, ()>) {
    let dir = tempfile::tempdir().expect("a machine to work on");
    let held = crate::calibrate::borrow_home(dir.path());
    (dir, held)
}

fn a_job(name: &str, at: &str) -> crate::jobs::Job {
    crate::jobs::Job {
        name: name.to_string(),
        at: vec![at.to_string()],
        ..Default::default()
    }
}

fn a_profile(name: &str) -> crate::calibrate::Profile {
    crate::calibrate::Profile {
        name: name.to_string(),
        error: crate::geometry::Similarity::IDENTITY,
        page: crate::calibrate::A4,
        rms_residual_mm: Some(0.12),
        max_residual_mm: Some(0.3),
        n_points: 8,
        created: 1_000,
        notes: "the big Ricoh".to_string(),
    }
}

/// The whole feature: an afternoon's setting up, carried to the next machine.
#[test]
fn what_took_an_afternoon_travels_to_the_next_machine() {
    let carried = {
        let (_dir, _home) = a_machine();
        crate::jobs::save(&a_job("paid", "150,40:PAID {today}")).unwrap();
        crate::jobs::save(&a_job("received", "20,60:Received")).unwrap();
        crate::calibrate::save_profile(&a_profile("the-ricoh")).unwrap();
        let mut settings = crate::settings::load();
        settings.defaults.dpi = Some(300.0);
        settings.defaults.page = Some("a4".to_string());
        crate::settings::save(&settings);

        let setup = gather();
        assert!(!setup.is_empty());
        assert_eq!(setup.jobs.len(), 2);
        assert_eq!(setup.profiles.len(), 1);
        setup
    };

    // A second machine, which has never been set up.
    let (_dir, _home) = a_machine();
    assert!(gather().is_empty(), "the new machine should start bare");

    let applied = apply(&carried, Clashing::Keep);
    assert_eq!(applied.jobs_added, vec!["paid", "received"]);
    assert_eq!(applied.profiles_added, vec!["the-ricoh"]);
    assert!(applied.settings_changed);
    assert!(applied.trouble.is_empty(), "{:?}", applied.trouble);

    // And it is really there, read back the way the program reads it.
    let now = gather();
    assert_eq!(now.jobs.len(), 2);
    assert_eq!(now.profiles.len(), 1);
    assert_eq!(now.defaults.dpi, Some(300.0));
    // The calibration is the expensive part: a measurement made with a scanner
    // and a target sheet, which is the whole reason this feature exists.
    assert_eq!(now.profiles[0].n_points, 8);
    assert_eq!(now.profiles[0].rms_residual_mm, Some(0.12));
    assert_eq!(now.profiles[0].notes, "the big Ricoh");
}

/// The one that would do real harm. A series remembers the receipt book has
/// reached 400; carried to a second machine, both print 401 next, and two
/// receipts with the same number on them is the one thing a receipt book must
/// never contain.
#[test]
fn numbering_never_travels() {
    let carried = {
        let (_dir, _home) = a_machine();
        crate::series::reached("receipts", 401).unwrap();
        assert_eq!(crate::series::next_for("receipts").unwrap(), 401);
        gather()
    };

    // Nothing in the file mentions it, however it is looked for.
    let written = serde_json::to_string(&carried).unwrap();
    assert!(!written.contains("receipts"), "{written}");
    assert!(!written.contains("401"), "{written}");

    let (_dir, _home) = a_machine();
    apply(&carried, Clashing::Keep);
    assert_eq!(
        crate::series::next_for("receipts").unwrap(),
        1,
        "the second machine took the first one's counter, and both will print the same numbers"
    );
    assert!(crate::series::all().is_empty());
}

/// The history is a record of what *that* machine printed, naming the files it
/// was done to. Copying it to a colleague's computer would hand over a list of
/// every document somebody worked on.
#[test]
fn the_record_of_what_was_printed_never_travels() {
    let carried = {
        let (_dir, _home) = a_machine();
        crate::history::remember(crate::history::Entry {
            at: 1_000,
            source: "/home/j/salary-review-confidential.pdf".to_string(),
            delta: "/home/j/salary-review-confidential-delta.pdf".to_string(),
            pages: 1,
            additions: 1,
            fingerprint: "abc123".to_string(),
        });
        assert_eq!(crate::history::read().len(), 1);
        gather()
    };

    let written = serde_json::to_string(&carried).unwrap();
    assert!(!written.contains("salary-review"), "{written}");
    assert!(!written.contains("abc123"), "{written}");

    let (_dir, _home) = a_machine();
    apply(&carried, Clashing::Keep);
    assert!(
        crate::history::read().is_empty(),
        "somebody else's record arrived"
    );
}

/// Where the browser last looked are paths on somebody else's disk. Carried
/// across they send it somewhere that does not exist on every machine but one.
#[test]
fn the_folders_this_machine_last_looked_in_do_not_travel() {
    let carried = {
        let (_dir, _home) = a_machine();
        let mut settings = crate::settings::load();
        settings.last_folder = Some(PathBuf::from("/home/whoever/Scans"));
        settings.last_output_folder = Some(PathBuf::from("/home/whoever/Deltas"));
        settings.last_screen = Some("harvest".to_string());
        crate::settings::save(&settings);
        gather()
    };
    let written = serde_json::to_string(&carried).unwrap();
    assert!(!written.contains("whoever"), "{written}");
    assert!(!written.contains("harvest"), "{written}");
}

/// Somebody's own job called `paid`, worked out for their own form, must not be
/// quietly replaced by the office one — that is how a person comes to print the
/// wrong thing on a document they have printed correctly a hundred times.
#[test]
fn a_name_already_in_use_here_is_kept_rather_than_replaced() {
    let office = {
        let (_dir, _home) = a_machine();
        crate::jobs::save(&a_job("paid", "150,40:PAID by the office")).unwrap();
        crate::jobs::save(&a_job("filed", "20,20:FILED")).unwrap();
        gather()
    };

    let (_dir, _home) = a_machine();
    crate::jobs::save(&a_job("paid", "10,10:my own PAID")).unwrap();

    let applied = apply(&office, Clashing::Keep);
    assert_eq!(applied.jobs_kept, vec!["paid"]);
    assert_eq!(applied.jobs_added, vec!["filed"]);
    assert!(applied.jobs_replaced.is_empty());
    // Untouched, exactly as it was.
    assert_eq!(
        crate::jobs::load("paid").unwrap().at,
        vec!["10,10:my own PAID".to_string()]
    );
    // And it says so, because somebody who handed over a setup file and finds
    // their job missing needs to know it was not taken.
    assert!(
        applied.describe().contains("kept yours"),
        "{}",
        applied.describe()
    );

    // Asked outright, it does replace. Both names now, because the first
    // pass added 'filed' to this machine — so on the second there is
    // something of ours at each name, and Replace means both.
    let applied = apply(&office, Clashing::Replace);
    // In the order the file holds them, which is the order jobs are listed
    // in — by name — so the report reads the same way twice running.
    assert_eq!(applied.jobs_replaced, vec!["filed", "paid"]);
    assert!(applied.jobs_kept.is_empty());
    assert_eq!(
        crate::jobs::load("paid").unwrap().at,
        vec!["150,40:PAID by the office".to_string()],
        "the one that was ours should now be the office's"
    );
}

/// A rehearsal has to be worked out by the same arithmetic as the performance,
/// or it is not a rehearsal.
#[test]
fn saying_what_would_happen_matches_what_does() {
    let office = {
        let (_dir, _home) = a_machine();
        crate::jobs::save(&a_job("paid", "150,40:PAID")).unwrap();
        crate::jobs::save(&a_job("filed", "20,20:FILED")).unwrap();
        crate::calibrate::save_profile(&a_profile("the-ricoh")).unwrap();
        gather()
    };

    let (_dir, _home) = a_machine();
    crate::jobs::save(&a_job("paid", "1,1:mine")).unwrap();

    let would = what_it_would_do(&office, Clashing::Keep);
    // Nothing has happened yet.
    assert_eq!(crate::jobs::list().len(), 1);

    let did = apply(&office, Clashing::Keep);
    assert_eq!(would.jobs_added, did.jobs_added);
    assert_eq!(would.jobs_kept, did.jobs_kept);
    assert_eq!(would.profiles_added, did.profiles_added);
    assert_eq!(would.settings_changed, did.settings_changed);
}

/// A file that is not one of these, or one from a later version, is refused by
/// name rather than half-read into a setup nobody asked for.
#[test]
fn a_file_that_is_not_a_setup_is_refused_by_name() {
    let (dir, _home) = a_machine();

    let rubbish = dir.path().join("notes.txt");
    std::fs::write(&rubbish, "this is not a setup").unwrap();
    let said = read(&rubbish).unwrap_err().to_string();
    assert!(said.contains("not an Onionskin setup file"), "{said}");
    assert!(said.contains("setup save"), "{said}");

    let missing = dir.path().join("nowhere.json");
    assert!(matches!(read(&missing), Err(SetupError::Io { .. })));

    // From a later version: recognised as a setup, and refused as too new.
    let ahead = dir.path().join("ahead.json");
    std::fs::write(&ahead, format!(r#"{{"version":{}}}"#, VERSION + 1)).unwrap();
    let said = read(&ahead).unwrap_err().to_string();
    assert!(said.contains("later version"), "{said}");
    assert!(said.contains("Update Onionskin"), "{said}");
}

/// It goes out and comes back exactly as it was, and is a file somebody can
/// open and read.
#[test]
fn it_survives_the_trip_through_a_file() {
    let (dir, _home) = a_machine();
    crate::jobs::save(&a_job("paid", "150,40:PAID {today}")).unwrap();
    crate::calibrate::save_profile(&a_profile("the-ricoh")).unwrap();
    let mut settings = crate::settings::load();
    settings.defaults.dpi = Some(300.0);
    crate::settings::save(&settings);

    let out = dir.path().join("office.json");
    let mine = gather();
    write(&out, &mine).unwrap();
    assert_eq!(read(&out).unwrap(), mine);

    // Readable by a person, and by anything else.
    let text = std::fs::read_to_string(&out).unwrap();
    assert!(text.contains("the-ricoh"), "{text}");
    assert!(text.contains("PAID {today}"), "{text}");
    assert!(
        text.contains('\n'),
        "it should be laid out, not one long line"
    );
    // It says which Onionskin wrote it, and not who or on what machine.
    assert!(mine.made_by.starts_with("Onionskin "), "{}", mine.made_by);
}

/// What it holds, said before it is written and again before it is taken —
/// because a file about to change how a computer behaves should say what it
/// will change first.
#[test]
fn it_says_what_is_in_it() {
    let (_dir, _home) = a_machine();
    assert!(gather().describe().contains("Nothing in it"));

    crate::jobs::save(&a_job("paid", "150,40:PAID")).unwrap();
    crate::calibrate::save_profile(&a_profile("the-ricoh")).unwrap();
    let mut settings = crate::settings::load();
    settings.defaults.dpi = Some(300.0);
    crate::settings::save(&settings);

    let said = gather().describe();
    assert!(said.contains("the-ricoh"), "{said}");
    assert!(said.contains("paid"), "{said}");
    // One setting chosen, not the fourteen that exist.
    assert!(said.contains("1 chosen"), "{said}");
    assert!(said.contains("dpi=300"), "{said}");

    // And what stays behind is stated, so somebody who expected their
    // numbering to travel finds out here rather than from two receipts.
    assert!(WHAT_STAYS_BEHIND.contains("numbered series"));
    assert!(WHAT_STAYS_BEHIND.contains("history"));
}

/// Taking the same setup twice changes nothing the second time, which is what
/// makes it safe to put in a login script or hand round an office.
#[test]
fn taking_it_twice_is_the_same_as_taking_it_once() {
    let office = {
        let (_dir, _home) = a_machine();
        crate::jobs::save(&a_job("paid", "150,40:PAID")).unwrap();
        crate::calibrate::save_profile(&a_profile("the-ricoh")).unwrap();
        let mut settings = crate::settings::load();
        settings.defaults.dpi = Some(300.0);
        crate::settings::save(&settings);
        gather()
    };

    let (_dir, _home) = a_machine();
    let first = apply(&office, Clashing::Keep);
    assert!(!first.nothing_happened());

    let again = apply(&office, Clashing::Keep);
    assert!(
        again.nothing_happened(),
        "taking it twice did something the second time: {}",
        again.describe()
    );
    assert_eq!(again.jobs_kept, vec!["paid"]);
    assert_eq!(crate::jobs::list().len(), 1, "the job was doubled");
    assert_eq!(crate::calibrate::list_profiles().unwrap().len(), 1);
}

/// A font folder on this machine is a real folder on this machine, and the
/// arriving list names folders on somebody else's. Both may be right, so the
/// lists are joined rather than one replacing the other.
#[test]
fn font_folders_are_added_to_rather_than_replaced() {
    let office = {
        let (_dir, _home) = a_machine();
        let mut settings = crate::settings::load();
        settings.font_folders = vec![PathBuf::from("/office/fonts")];
        crate::settings::save(&settings);
        gather()
    };

    let (_dir, _home) = a_machine();
    let mut settings = crate::settings::load();
    settings.font_folders = vec![PathBuf::from("/home/me/my-fonts")];
    crate::settings::save(&settings);

    let applied = apply(&office, Clashing::Keep);
    assert_eq!(applied.font_folders_added, 1);
    let after = crate::settings::load().font_folders;
    assert!(
        after.contains(&PathBuf::from("/home/me/my-fonts")),
        "{after:?}"
    );
    assert!(after.contains(&PathBuf::from("/office/fonts")), "{after:?}");

    // And again adds nothing, rather than a second copy.
    let again = apply(&office, Clashing::Keep);
    assert_eq!(again.font_folders_added, 0);
    assert_eq!(crate::settings::load().font_folders.len(), 2);
}
