use super::*;

/// Point the settings at a directory of the test's own.
///
/// The settings live under `ONIONSKIN_HOME`, which is one variable for the
/// whole process — so this borrows it through the lock every other test that
/// redirects it uses, rather than racing them.
struct Elsewhere {
    _keep: tempfile::TempDir,
    _held: std::sync::MutexGuard<'static, ()>,
}

fn elsewhere() -> Elsewhere {
    let keep = tempfile::tempdir().expect("a temporary home");
    let held = crate::calibrate::borrow_home(keep.path());
    Elsewhere {
        _keep: keep,
        _held: held,
    }
}

#[test]
fn nothing_remembered_is_not_an_error() {
    let _home = elsewhere();
    assert_eq!(load(), Settings::default());
}

#[test]
fn what_is_written_comes_back() {
    let _home = elsewhere();
    let folder = std::env::temp_dir();
    save(&Settings {
        last_folder: Some(folder.clone()),
        last_output_folder: None,
        last_screen: Some("Compare".into()),
        font_folders: Vec::new(),
        defaults: Defaults::default(),
    });

    let read = load();
    assert_eq!(read.last_folder.as_deref(), Some(folder.as_path()));
    assert_eq!(read.last_screen.as_deref(), Some("Compare"));
}

#[test]
fn a_settings_file_from_another_version_still_opens_the_program() {
    let _home = elsewhere();
    std::fs::create_dir_all(crate::calibrate::home_dir()).unwrap();
    std::fs::write(
        crate::calibrate::home_dir().join("settings.json"),
        "{\"something\": \"nobody has heard of\", \"last_screen\": 41}",
    )
    .unwrap();

    // Not a panic, not an error: a program that opens where it always did.
    assert_eq!(load(), Settings::default());
}

#[test]
fn a_file_that_is_not_json_at_all_is_ignored() {
    let _home = elsewhere();
    std::fs::create_dir_all(crate::calibrate::home_dir()).unwrap();
    std::fs::write(
        crate::calibrate::home_dir().join("settings.json"),
        "this is not JSON",
    )
    .unwrap();
    assert_eq!(load(), Settings::default());
}

#[test]
fn remembering_one_thing_does_not_forget_another() {
    let _home = elsewhere();
    let folder = std::env::temp_dir();
    remember(|settings| settings.last_screen = Some("Draw".into()));
    remember(|settings| settings.last_folder = Some(folder.clone()));

    let read = load();
    assert_eq!(read.last_screen.as_deref(), Some("Draw"));
    assert_eq!(read.last_folder.as_deref(), Some(folder.as_path()));
}

#[test]
fn the_folder_remembered_is_the_one_the_file_is_in() {
    let _home = elsewhere();
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("report.pdf");
    std::fs::write(&file, b"not really").unwrap();

    remember_folder(&file);
    assert_eq!(load().last_folder.as_deref(), Some(dir.path()));

    // A folder is remembered as itself.
    remember_folder(dir.path());
    assert_eq!(load().last_folder.as_deref(), Some(dir.path()));
}

#[test]
fn a_folder_that_is_not_there_is_not_remembered() {
    let _home = elsewhere();
    remember_folder(std::path::Path::new("/nowhere/at/all/report.pdf"));
    assert_eq!(load().last_folder, None);
}

#[test]
fn a_browser_opens_where_it_was_pointed_first() {
    let _home = elsewhere();
    let asked = tempfile::tempdir().unwrap();
    let remembered = tempfile::tempdir().unwrap();
    remember_folder(remembered.path());

    // What this control was pointed at wins over what was remembered.
    assert_eq!(start_in(Some(asked.path())), asked.path());
    // And with nothing to go on, the remembered folder is better than nothing.
    assert_eq!(start_in(None), remembered.path());
}

#[test]
fn a_browser_falls_back_to_somewhere_that_exists() {
    let _home = elsewhere();
    let gone = std::path::Path::new("/nowhere/at/all");
    let opened = start_in(Some(gone));
    assert!(
        opened.is_dir(),
        "a file browser must open somewhere that exists, got {}",
        opened.display()
    );
}

#[cfg(unix)]
#[test]
fn the_file_is_not_readable_by_other_accounts() {
    use std::os::unix::fs::PermissionsExt;
    let _home = elsewhere();
    save(&Settings {
        last_screen: Some("Read".into()),
        ..Settings::default()
    });

    let mode = std::fs::metadata(crate::calibrate::home_dir().join("settings.json"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o077, 0, "mode {:o}", mode & 0o777);
}

// ---------------------------------------------------------------------------
// Font folders
// ---------------------------------------------------------------------------

#[test]
fn a_fonts_folder_is_remembered_once_however_many_times_it_is_added() {
    let _home = elsewhere();
    let folder = tempfile::tempdir().unwrap();

    assert!(add_font_folder(folder.path()), "the first add should take");
    assert!(
        !add_font_folder(folder.path()),
        "the second add should say it was already there"
    );
    assert_eq!(font_folders().len(), 1, "{:?}", font_folders());
}

#[test]
fn a_fonts_folder_can_be_taken_away_again() {
    let _home = elsewhere();
    let folder = tempfile::tempdir().unwrap();
    add_font_folder(folder.path());

    assert!(forget_font_folder(folder.path()));
    assert!(font_folders().is_empty());
    // And forgetting one that was never there is not an error, it is a no.
    assert!(!forget_font_folder(folder.path()));
}

#[test]
fn a_folder_that_has_gone_away_is_not_offered() {
    // Somebody's fonts on a drive they have unplugged. The folder stays in the
    // settings — it will be back tomorrow — but nothing should try to read it
    // today, and nothing should complain about it either.
    let _home = elsewhere();
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().to_path_buf();
    add_font_folder(&path);
    assert_eq!(font_folders().len(), 1);

    drop(folder);
    assert!(font_folders().is_empty(), "a deleted folder was offered");
    assert_eq!(
        load().font_folders.len(),
        1,
        "the folder should still be remembered, only not offered"
    );
}

#[test]
fn settings_written_by_a_version_that_knew_nothing_of_fonts_still_load() {
    // The whole reason every field has a default: an older Onionskin wrote
    // this file, and a person upgrading should not meet an error.
    let _home = elsewhere();
    std::fs::create_dir_all(crate::calibrate::home_dir()).unwrap();
    std::fs::write(
        crate::calibrate::home_dir().join("settings.json"),
        r#"{"last_screen":"Compare"}"#,
    )
    .unwrap();

    let read = load();
    assert_eq!(read.last_screen.as_deref(), Some("Compare"));
    assert!(read.font_folders.is_empty());
}

// ---------------------------------------------------------------------------
// Defaults somebody chose for themselves
// ---------------------------------------------------------------------------

#[test]
fn nothing_chosen_means_onionskins_own_answer() {
    let _home = elsewhere();
    let mine = load().defaults;
    assert!(mine.is_empty());
    // Absent, not written out with the default in it: a default copied into a
    // file stops tracking the default.
    assert!(mine.dpi.is_none());
    assert!(mine.outline.is_none());
}

#[test]
fn a_setting_is_kept_and_can_be_taken_away_again() {
    let _home = elsewhere();
    set_default("dpi", Some("300")).unwrap();
    assert_eq!(load().defaults.dpi, Some(300.0));

    set_default("dpi", None).unwrap();
    assert!(load().defaults.dpi.is_none());
    assert!(load().defaults.is_empty(), "the file should be back to bare");
}

#[test]
fn a_bad_value_is_refused_when_it_is_typed() {
    // Not silently stored and met as an error on some later run they have
    // forgotten this by.
    let _home = elsewhere();
    for (name, value) in [
        ("dpi", "9999"),
        ("dpi", "ten"),
        ("margin", "-1"),
        ("mode", "sideways"),
        ("outline", "maybe"),
    ] {
        let said = set_default(name, Some(value)).unwrap_err();
        assert!(!said.is_empty(), "{name}={value} was accepted");
    }
    assert!(load().defaults.is_empty(), "a refused value was stored");
}

#[test]
fn a_setting_that_does_not_exist_says_which_ones_do() {
    let _home = elsewhere();
    let said = set_default("colour-scheme", Some("dark")).unwrap_err();
    assert!(said.contains("colour-scheme"), "{said}");
    assert!(said.contains("outline"), "{said}");
    assert!(said.contains("dpi"), "{said}");
}

#[test]
fn yes_and_no_are_accepted_the_ways_people_write_them() {
    let _home = elsewhere();
    for yes in ["yes", "true", "on", "1", "YES"] {
        set_default("outline", Some(yes)).unwrap();
        assert_eq!(load().defaults.outline, Some(true), "{yes}");
    }
    for no in ["no", "false", "off", "0", "No"] {
        set_default("outline", Some(no)).unwrap();
        assert_eq!(load().defaults.outline, Some(false), "{no}");
    }
}

#[test]
fn every_setting_is_listed_with_something_to_read() {
    // `config show`, `config set` and the error message all read this, so a
    // setting missing from it is a setting nobody can discover.
    let _home = elsewhere();
    let listed = Defaults::default().each();
    assert_eq!(listed.len(), 7, "{listed:?}");
    for (name, value, what) in &listed {
        assert!(!name.is_empty());
        assert!(value.is_none(), "{name} has a value before anything was set");
        assert!(!what.is_empty(), "{name} has no description");
        // And every listed name has to be one `set` will actually accept.
        assert!(
            set_default(name, None).is_ok(),
            "{name} is listed but not settable"
        );
    }
}

#[test]
fn clearing_takes_everything_away_and_nothing_else() {
    let _home = elsewhere();
    set_default("dpi", Some("300")).unwrap();
    set_default("outline", Some("yes")).unwrap();
    remember_folder(&std::env::temp_dir());

    clear_defaults();
    assert!(load().defaults.is_empty());
    // The places somebody was working are not preferences and are not touched.
    assert!(load().last_folder.is_some(), "clearing took the folder too");
}

#[test]
fn settings_written_before_defaults_existed_still_load() {
    let _home = elsewhere();
    std::fs::create_dir_all(crate::calibrate::home_dir()).unwrap();
    std::fs::write(
        crate::calibrate::home_dir().join("settings.json"),
        r#"{"last_screen":"Compare","font_folders":[]}"#,
    )
    .unwrap();
    let read = load();
    assert_eq!(read.last_screen.as_deref(), Some("Compare"));
    assert!(read.defaults.is_empty());
}
