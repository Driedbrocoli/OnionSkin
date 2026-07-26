//! Tests for the parts of the picker that do not need a window.
//!
//! Nothing here drives a real frame: `egui::__run_test_ui` (used elsewhere in
//! the desktop tests) still cannot simulate a key press or a pause for a
//! prefix to expire, so instead every behaviour worth checking — the
//! type-ahead matching, the path box's Enter, the selection arithmetic, the
//! timeout — has been written as a plain function that takes what it needs
//! and hands back an answer. These tests call those functions directly.

use super::*;

/// A row with a made-up path, for tests that only care about the name and
/// whether it is a folder.
fn row(name: &str, is_dir: bool) -> Row {
    Row {
        name: name.to_string(),
        path: PathBuf::from(format!("/pretend/{name}")),
        is_dir,
    }
}

/// A `Browsing` with every field set to something a real one would not have,
/// so a test that checks `navigate_to` really did reset a field cannot pass
/// by accident because that field started out empty anyway.
fn browsing_at(at: &Path) -> Browsing {
    Browsing {
        who: egui::Id::new("test-picker"),
        purpose: Purpose::Open,
        title: "Test".to_string(),
        at: at.to_path_buf(),
        kinds: Vec::new(),
        name: String::new(),
        chosen: Some(2),
        confirm_overwrite: true,
        error: None,
        path_box: "whatever was there before".to_string(),
        path_problem: Some("an old complaint".to_string()),
        type_ahead: "ab".to_string(),
        type_ahead_at: 123.0,
        focus_name: false,
    }
}

// ---------------------------------------------------------------------------
// Moving the selection: Up, Down, Home, End, Page Up, Page Down
// ---------------------------------------------------------------------------

#[test]
fn pressing_down_with_nothing_selected_lands_on_the_first_row() {
    assert_eq!(step_selection(None, 5, Step::Down), Some(0));
}

#[test]
fn pressing_up_with_nothing_selected_also_lands_on_the_first_row() {
    // Not the last row: the first press of any key here means "start
    // looking", not "start at the end".
    assert_eq!(step_selection(None, 5, Step::Up), Some(0));
}

#[test]
fn down_moves_to_the_next_row() {
    assert_eq!(step_selection(Some(2), 5, Step::Down), Some(3));
}

#[test]
fn up_moves_to_the_previous_row() {
    assert_eq!(step_selection(Some(2), 5, Step::Up), Some(1));
}

#[test]
fn down_does_not_move_past_the_last_row() {
    // No wrapping: five hundred files should not be one stray Down away from
    // the selection silently landing back at the top.
    assert_eq!(step_selection(Some(4), 5, Step::Down), Some(4));
}

#[test]
fn up_does_not_move_past_the_first_row() {
    assert_eq!(step_selection(Some(0), 5, Step::Up), Some(0));
}

#[test]
fn home_jumps_to_the_first_row() {
    assert_eq!(step_selection(Some(3), 5, Step::Home), Some(0));
}

#[test]
fn end_jumps_to_the_last_row() {
    assert_eq!(step_selection(Some(0), 5, Step::End), Some(4));
}

#[test]
fn page_down_moves_by_ten() {
    assert_eq!(step_selection(Some(2), 20, Step::PageDown), Some(12));
}

#[test]
fn page_down_stops_at_the_last_row() {
    assert_eq!(step_selection(Some(15), 20, Step::PageDown), Some(19));
}

#[test]
fn page_up_moves_by_ten() {
    assert_eq!(step_selection(Some(15), 20, Step::PageUp), Some(5));
}

#[test]
fn page_up_stops_at_the_first_row() {
    assert_eq!(step_selection(Some(5), 20, Step::PageUp), Some(0));
}

#[test]
fn an_empty_list_has_nothing_to_select() {
    assert_eq!(step_selection(None, 0, Step::Down), None);
    assert_eq!(step_selection(Some(0), 0, Step::Home), None);
}

#[test]
fn the_arithmetic_is_exercised_against_a_real_list_of_rows() {
    let rows = [row("a", false), row("b", false), row("c", false)];
    assert_eq!(step_selection(Some(0), rows.len(), Step::End), Some(2));
}

// ---------------------------------------------------------------------------
// Type to jump
// ---------------------------------------------------------------------------

#[test]
fn typing_jumps_to_the_first_name_that_starts_with_it() {
    let rows = [
        row("apple", false),
        row("banana", false),
        row("cherry", false),
    ];
    assert_eq!(jump_to_prefix(&rows, "ba"), Some(1));
}

#[test]
fn matching_does_not_care_about_case() {
    let rows = [row("Report.pdf", false)];
    assert_eq!(jump_to_prefix(&rows, "rep"), Some(0));
}

#[test]
fn no_match_finds_nothing() {
    let rows = [row("apple", false)];
    assert_eq!(jump_to_prefix(&rows, "zzz"), None);
}

#[test]
fn an_empty_prefix_finds_nothing() {
    let rows = [row("apple", false)];
    assert_eq!(jump_to_prefix(&rows, ""), None);
}

#[test]
fn a_quick_second_letter_does_not_expire_the_prefix() {
    assert!(!prefix_expired(10.0, 10.4));
}

#[test]
fn a_pause_of_over_a_second_expires_the_prefix() {
    assert!(prefix_expired(10.0, 11.2));
}

// ---------------------------------------------------------------------------
// Enter, on the list
// ---------------------------------------------------------------------------

#[test]
fn entering_a_folder_row_goes_into_it() {
    let rows = [row("Photos", true), row("notes.pdf", false)];
    assert_eq!(
        enter_row(&rows, Some(0)),
        Some(Entered::Folder(PathBuf::from("/pretend/Photos")))
    );
}

#[test]
fn entering_a_file_row_hands_it_back() {
    let rows = [row("Photos", true), row("notes.pdf", false)];
    assert_eq!(
        enter_row(&rows, Some(1)),
        Some(Entered::File(PathBuf::from("/pretend/notes.pdf")))
    );
}

#[test]
fn entering_with_nothing_selected_does_nothing() {
    let rows = [row("Photos", true)];
    assert_eq!(enter_row(&rows, None), None);
}

#[test]
fn entering_a_row_past_the_end_of_the_list_does_nothing() {
    let rows = [row("Photos", true)];
    assert_eq!(enter_row(&rows, Some(5)), None);
}

// ---------------------------------------------------------------------------
// The path box: `~`, a folder, a file, somewhere that is not there, and a
// path relative to wherever the dialog currently is
// ---------------------------------------------------------------------------

#[test]
fn a_bare_tilde_means_home() {
    let home = tempfile::tempdir().unwrap();
    let at = tempfile::tempdir().unwrap();
    assert_eq!(
        resolve_path_input("~", at.path(), home.path()),
        Ok(Destination::Folder(home.path().to_path_buf()))
    );
}

#[test]
fn a_tilde_slash_path_is_taken_from_home() {
    let home = tempfile::tempdir().unwrap();
    let at = tempfile::tempdir().unwrap();
    std::fs::create_dir(home.path().join("Documents")).unwrap();
    assert_eq!(
        resolve_path_input("~/Documents", at.path(), home.path()),
        Ok(Destination::Folder(home.path().join("Documents")))
    );
}

#[test]
fn a_name_that_looks_like_someone_elses_home_is_left_alone() {
    // `~fred` is a different person's home directory to a shell. Guessing
    // which `fred` was meant, and sending somebody there, would be worse
    // than just saying the made-up name does not exist.
    let home = tempfile::tempdir().unwrap();
    let at = tempfile::tempdir().unwrap();
    assert!(resolve_path_input("~fred", at.path(), home.path()).is_err());
}

#[test]
fn an_existing_folder_is_recognised() {
    let home = tempfile::tempdir().unwrap();
    let at = tempfile::tempdir().unwrap();
    std::fs::create_dir(at.path().join("reports")).unwrap();
    assert_eq!(
        resolve_path_input("reports", at.path(), home.path()),
        Ok(Destination::Folder(at.path().join("reports")))
    );
}

#[test]
fn an_existing_file_selects_its_folder() {
    let home = tempfile::tempdir().unwrap();
    let at = tempfile::tempdir().unwrap();
    std::fs::write(at.path().join("thing.pdf"), b"pretend pdf").unwrap();
    assert_eq!(
        resolve_path_input("thing.pdf", at.path(), home.path()),
        Ok(Destination::File {
            folder: at.path().to_path_buf(),
            file: at.path().join("thing.pdf"),
        })
    );
}

#[test]
fn a_relative_path_resolves_against_the_current_folder() {
    // Also exercises the ".." folding: without it this would resolve to
    // ".../a/../sibling" rather than the tidy path a person would expect to
    // see afterwards.
    let home = tempfile::tempdir().unwrap();
    let root = tempfile::tempdir().unwrap();
    let a = root.path().join("a");
    let sibling = root.path().join("sibling");
    std::fs::create_dir(&a).unwrap();
    std::fs::create_dir(&sibling).unwrap();
    assert_eq!(
        resolve_path_input("../sibling", &a, home.path()),
        Ok(Destination::Folder(sibling))
    );
}

#[test]
fn an_absolute_path_ignores_the_current_folder() {
    let home = tempfile::tempdir().unwrap();
    let at = tempfile::tempdir().unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    assert_eq!(
        resolve_path_input(&elsewhere.path().to_string_lossy(), at.path(), home.path()),
        Ok(Destination::Folder(elsewhere.path().to_path_buf()))
    );
}

#[test]
fn a_path_that_does_not_exist_is_reported() {
    let home = tempfile::tempdir().unwrap();
    let at = tempfile::tempdir().unwrap();
    let outcome = resolve_path_input("nowhere-near-real", at.path(), home.path());
    assert!(
        outcome.is_err(),
        "a made-up name should not be mistaken for a real folder or file"
    );
}

#[test]
fn empty_or_blank_input_is_reported_rather_than_guessed_at() {
    let home = tempfile::tempdir().unwrap();
    let at = tempfile::tempdir().unwrap();
    assert!(resolve_path_input("", at.path(), home.path()).is_err());
    assert!(resolve_path_input("   ", at.path(), home.path()).is_err());
}

// ---------------------------------------------------------------------------
// Folding "." and ".." out of a path without touching the filesystem
// ---------------------------------------------------------------------------

#[test]
fn dot_dot_cancels_the_segment_before_it() {
    assert_eq!(normalise(Path::new("/a/b/../c")), PathBuf::from("/a/c"));
}

#[test]
fn dot_dot_at_the_root_stays_at_the_root() {
    assert_eq!(normalise(Path::new("/../../etc")), PathBuf::from("/etc"));
}

#[test]
fn a_lone_dot_is_ignored() {
    assert_eq!(normalise(Path::new("/a/./b")), PathBuf::from("/a/b"));
}

#[test]
fn unresolved_parent_segments_are_kept() {
    // Nothing before it in a relative path to cancel against, so it has to
    // stay rather than vanish and quietly change what the path means.
    assert_eq!(normalise(Path::new("../a")), PathBuf::from("../a"));
}

// ---------------------------------------------------------------------------
// Deciding what Save should do
// ---------------------------------------------------------------------------

#[test]
fn a_new_name_is_answered_immediately() {
    let at = tempfile::tempdir().unwrap();
    assert_eq!(
        decide_save(at.path(), "new.onionskin", false),
        SaveOutcome::Answer(at.path().join("new.onionskin"))
    );
}

#[test]
fn an_existing_name_asks_to_be_confirmed_first() {
    let at = tempfile::tempdir().unwrap();
    std::fs::write(at.path().join("existing.onionskin"), b"pretend").unwrap();
    assert_eq!(
        decide_save(at.path(), "existing.onionskin", false),
        SaveOutcome::NeedsConfirmation
    );
}

#[test]
fn confirming_answers_even_though_it_still_exists() {
    let at = tempfile::tempdir().unwrap();
    std::fs::write(at.path().join("existing.onionskin"), b"pretend").unwrap();
    assert_eq!(
        decide_save(at.path(), "existing.onionskin", true),
        SaveOutcome::Answer(at.path().join("existing.onionskin"))
    );
}

#[test]
fn the_name_is_trimmed_before_joining() {
    let at = tempfile::tempdir().unwrap();
    assert_eq!(
        decide_save(at.path(), "  spaced.onionskin  ", false),
        SaveOutcome::Answer(at.path().join("spaced.onionskin"))
    );
}

// ---------------------------------------------------------------------------
// Moving folders
// ---------------------------------------------------------------------------

#[test]
fn moving_to_a_folder_resets_what_belonged_to_the_old_one() {
    let old = tempfile::tempdir().unwrap();
    let new = tempfile::tempdir().unwrap();
    let mut browsing = browsing_at(old.path());

    browsing.navigate_to(new.path().to_path_buf());

    assert_eq!(browsing.at, new.path());
    assert_eq!(browsing.path_box, new.path().to_string_lossy().into_owned());
    assert_eq!(browsing.chosen, None);
    assert!(!browsing.confirm_overwrite);
    assert_eq!(browsing.path_problem, None);
    assert!(browsing.type_ahead.is_empty());
}
