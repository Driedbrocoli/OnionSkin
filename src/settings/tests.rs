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
