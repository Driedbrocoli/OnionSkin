//! Tests for installing and uninstalling.

use super::*;

/// A pretend Onionskin binary, and a pretend rendering library beside it.
fn a_download(dir: &Path) -> PathBuf {
    let binary = dir.join(binary_name());
    std::fs::write(&binary, b"pretend onionskin").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    for name in library_names() {
        std::fs::write(dir.join(name), b"pretend pdfium").unwrap();
    }
    binary
}

// ---------------------------------------------------------------------------
// Where things go
// ---------------------------------------------------------------------------

#[test]
fn the_default_place_needs_no_administrator() {
    // A program that asks for a password to put a file on your own computer
    // teaches people to give passwords to programs.
    let prefix = default_prefix();
    let text = prefix.to_string_lossy();
    assert!(
        !text.starts_with("/usr/") && !text.starts_with("/opt/"),
        "{text} would need a password"
    );
    assert!(
        prefix.starts_with(home()),
        "{text} is not inside the home directory"
    );
}

#[test]
fn the_binary_is_named_for_the_platform() {
    let name = binary_name();
    if cfg!(windows) {
        assert_eq!(name, "onionskin.exe");
    } else {
        assert_eq!(name, "onionskin");
    }
}

#[test]
fn a_directory_already_on_the_path_is_recognised() {
    let dir = tempfile::tempdir().unwrap();
    assert!(!on_path(dir.path()), "a fresh directory is not on the path");

    // Put it on, and it should be found — including through a different but
    // equivalent spelling of the same directory.
    let existing = std::env::var_os("PATH").unwrap_or_default();
    let mut entries: Vec<PathBuf> = std::env::split_paths(&existing).collect();
    entries.push(dir.path().to_path_buf());
    let joined = std::env::join_paths(entries).unwrap();
    std::env::set_var("PATH", &joined);

    assert!(on_path(dir.path()));
    let roundabout = dir.path().join("sub/..");
    std::fs::create_dir_all(dir.path().join("sub")).unwrap();
    assert!(
        on_path(&roundabout),
        "the same directory written another way"
    );

    std::env::set_var("PATH", existing);
}

// ---------------------------------------------------------------------------
// Installing
// ---------------------------------------------------------------------------

#[test]
fn installing_puts_the_program_and_its_library_in_place() {
    let download = tempfile::tempdir().unwrap();
    let prefix = tempfile::tempdir().unwrap();
    a_download(download.path());

    // `install` copies the *running* program, so drive the pieces directly
    // with the download standing in for it.
    let target = prefix.path().join(binary_name());
    place(&download.path().join(binary_name()), &target).unwrap();
    assert!(target.is_file());
    assert_eq!(std::fs::read(&target).unwrap(), b"pretend onionskin");

    for name in library_names() {
        let from = download.path().join(name);
        if from.is_file() {
            let to = prefix.path().join(name);
            place(&from, &to).unwrap();
            assert!(to.is_file(), "{name} was not brought along");
        }
    }
}

#[cfg(unix)]
#[test]
fn what_is_installed_can_actually_be_run() {
    use std::os::unix::fs::PermissionsExt;
    let download = tempfile::tempdir().unwrap();
    let prefix = tempfile::tempdir().unwrap();
    a_download(download.path());

    let target = prefix.path().join(binary_name());
    place(&download.path().join(binary_name()), &target).unwrap();

    let mode = std::fs::metadata(&target).unwrap().permissions().mode();
    assert!(mode & 0o100 != 0, "not executable by its owner: {mode:o}");
}

#[test]
fn installing_over_itself_does_not_empty_the_file() {
    // Running `onionskin install` from the very place it installs to is an
    // easy thing to do by accident, and a plain copy truncates the file first.
    let dir = tempfile::tempdir().unwrap();
    let binary = a_download(dir.path());

    place(&binary, &binary).unwrap();
    assert_eq!(
        std::fs::read(&binary).unwrap(),
        b"pretend onionskin",
        "the program deleted itself"
    );
}

#[test]
fn a_real_install_and_uninstall_leaves_nothing_behind() {
    let prefix = tempfile::tempdir().unwrap();
    let options = Options {
        prefix: Some(prefix.path().to_path_buf()),
        keep_path: true,
        no_menu: true,
    };

    let report = install(&options).unwrap();
    let binary = report.binary.expect("nothing was installed");
    assert!(binary.is_file());
    // It is this very program, so it must still be the real thing.
    assert!(std::fs::metadata(&binary).unwrap().len() > 1000);

    let (where_it_is, installed) = status(&options);
    assert!(installed);
    assert_eq!(where_it_is, binary);

    let removed = uninstall(&options).unwrap();
    assert_eq!(removed.binary.as_deref(), Some(binary.as_path()));
    assert!(!binary.exists(), "the binary is still there");
    assert!(!status(&options).1);
}

#[test]
fn installing_twice_is_the_same_as_installing_once() {
    let prefix = tempfile::tempdir().unwrap();
    let options = Options {
        prefix: Some(prefix.path().to_path_buf()),
        keep_path: true,
        no_menu: true,
    };
    let first = install(&options).unwrap();
    let second = install(&options).unwrap();
    assert_eq!(first.binary, second.binary);
    let _ = uninstall(&options);
}

#[test]
fn a_missing_rendering_library_is_said_rather_than_hidden() {
    let prefix = tempfile::tempdir().unwrap();
    let options = Options {
        prefix: Some(prefix.path().to_path_buf()),
        keep_path: true,
        no_menu: true,
    };
    let report = install(&options).unwrap();

    // This binary is built by cargo, so nothing is beside it — which is
    // exactly the case that has to be reported instead of failing quietly.
    if report.library.is_none() {
        assert!(
            report.notes.iter().any(|n| n.contains("rendering library")),
            "{:?}",
            report.notes
        );
        assert!(
            report.notes.iter().any(|n| n.contains("doctor")),
            "it should say how to check: {:?}",
            report.notes
        );
    }
    let _ = uninstall(&options);
}

// ---------------------------------------------------------------------------
// The path, and the shell profile
// ---------------------------------------------------------------------------

#[test]
fn the_path_line_is_written_in_the_shells_own_syntax() {
    let bash = path_line(
        Path::new("/home/someone/.local/bin"),
        Path::new("/home/x/.profile"),
    );
    assert!(bash.starts_with("export PATH="), "{bash}");
    assert!(bash.contains("/home/someone/.local/bin"));

    let fish = path_line(
        Path::new("/home/someone/.local/bin"),
        Path::new("/home/x/.config/fish/config.fish"),
    );
    assert!(fish.starts_with("fish_add_path"), "{fish}");
}

#[test]
fn every_line_written_is_marked_so_it_can_be_found_again() {
    // A profile is somebody's own file. Anything put in it has to be
    // identifiable, or it can never be taken out safely.
    for profile in ["/home/x/.profile", "/home/x/.config/fish/config.fish"] {
        let line = path_line(Path::new("/somewhere"), Path::new(profile));
        assert!(line.contains(MARKER), "{line}");
    }
}

/// Tests that set `SHELL` or `HOME` take turns.
///
/// There is one environment per process and `cargo test` runs threads, so two
/// tests each setting `SHELL` to what they need will read each other's answer
/// — not every time, which is worse than never: a suite that fails one run in
/// twenty teaches people to run it again rather than to read it.
fn one_at_a_time() -> std::sync::MutexGuard<'static, ()> {
    static ORDERLY: std::sync::Mutex<()> = std::sync::Mutex::new(());
    // A test that panicked while holding it left it poisoned. Every test here
    // sets what it needs on the way in, so there is nothing to recover.
    ORDERLY
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[test]
fn the_profile_chosen_is_the_one_the_shell_reads() {
    let _orderly = one_at_a_time();
    let before = std::env::var("SHELL").ok();

    std::env::set_var("SHELL", "/bin/zsh");
    assert!(shell_profile().ends_with(".zprofile"));

    std::env::set_var("SHELL", "/usr/bin/fish");
    assert!(shell_profile().to_string_lossy().contains("fish"));

    std::env::set_var("SHELL", "/bin/sh");
    let plain = shell_profile();
    assert!(
        plain.ends_with(".profile") || plain.ends_with(".bash_profile"),
        "{plain:?}"
    );

    match before {
        Some(shell) => std::env::set_var("SHELL", shell),
        None => std::env::remove_var("SHELL"),
    }
}

#[cfg(unix)]
#[test]
fn the_path_line_is_added_once_and_taken_out_cleanly() {
    let _orderly = one_at_a_time();
    let fake_home = tempfile::tempdir().unwrap();
    let prefix = tempfile::tempdir().unwrap();
    let before_home = std::env::var("HOME").ok();
    let before_shell = std::env::var("SHELL").ok();
    std::env::set_var("HOME", fake_home.path());
    std::env::set_var("SHELL", "/bin/sh");

    // Something already in the profile, which must survive untouched.
    let profile = fake_home.path().join(".profile");
    std::fs::write(&profile, "export EDITOR=vi\n").unwrap();

    let options = Options {
        prefix: Some(prefix.path().to_path_buf()),
        keep_path: false,
        no_menu: true,
    };
    install(&options).unwrap();
    install(&options).unwrap(); // twice, on purpose

    let text = std::fs::read_to_string(&profile).unwrap();
    assert_eq!(
        text.matches(MARKER).count(),
        1,
        "the line was added twice:\n{text}"
    );
    assert!(
        text.contains("export EDITOR=vi"),
        "it ate the existing line"
    );

    uninstall(&options).unwrap();
    let after = std::fs::read_to_string(&profile).unwrap();
    assert!(
        !after.contains(MARKER),
        "the line was left behind:\n{after}"
    );
    assert!(
        after.contains("export EDITOR=vi"),
        "uninstalling ate somebody else's line:\n{after}"
    );

    match before_home {
        Some(home) => std::env::set_var("HOME", home),
        None => std::env::remove_var("HOME"),
    }
    match before_shell {
        Some(shell) => std::env::set_var("SHELL", shell),
        None => std::env::remove_var("SHELL"),
    }
}

// ---------------------------------------------------------------------------
// The menu entry
// ---------------------------------------------------------------------------

#[test]
fn the_menu_entry_opens_the_window_when_there_is_one() {
    // A menu entry is what somebody clicks expecting an application. If it
    // launches the command line program instead, they get a terminal — or, if
    // it launches one with no visible output, they get nothing at all and
    // conclude the thing is broken.
    let entry = desktop_entry(
        Path::new("/home/someone/.local/bin/onionskin"),
        Some(Path::new("/home/someone/.local/bin/onionskin-desktop")),
    );
    assert!(entry.starts_with("[Desktop Entry]"), "{entry}");
    assert!(entry.contains("Type=Application"));
    assert!(entry.contains("Name=Onionskin"));
    assert!(
        entry.contains("Exec=/home/someone/.local/bin/onionskin-desktop"),
        "{entry}"
    );
    assert!(
        entry.contains("Terminal=false"),
        "a window does not want a terminal behind it: {entry}"
    );
    assert!(
        !entry.contains("serve"),
        "it should not fall back to the browser interface: {entry}"
    );
}

#[test]
fn the_menu_entry_falls_back_to_the_browser_interface() {
    // Somebody who built only the command line program still gets a working
    // menu entry — and that one does need a terminal, to show the address the
    // browser interface is running at.
    let entry = desktop_entry(Path::new("/home/someone/.local/bin/onionskin"), None);
    assert!(
        entry.contains("Exec=/home/someone/.local/bin/onionskin serve"),
        "{entry}"
    );
    assert!(entry.contains("Terminal=true"), "{entry}");
}

#[test]
fn the_window_is_installed_and_removed_with_the_program() {
    // The window is three times the size of the command line program. Leaving
    // it behind after an uninstall is the sort of thing that gets noticed.
    let download = tempfile::tempdir().unwrap();
    let prefix = tempfile::tempdir().unwrap();
    a_download(download.path());
    let window = download.path().join(desktop_name());
    std::fs::write(&window, b"pretend window").unwrap();

    let target = prefix.path().join(desktop_name());
    place(&window, &target).unwrap();
    assert!(target.is_file());

    let options = Options {
        prefix: Some(prefix.path().to_path_buf()),
        keep_path: true,
        no_menu: true,
    };
    let removed = uninstall(&options).unwrap();
    assert_eq!(
        removed.desktop.as_deref(),
        Some(target.as_path()),
        "the window was left behind"
    );
    assert!(!target.exists());
}

#[test]
fn what_the_window_needs_is_asked_for_by_name() {
    // On a machine that has everything this is empty, which is the ordinary
    // case and says nothing useful. What matters is that when something *is*
    // missing, the answer is a command somebody can run rather than a list of
    // filenames they must work out for themselves.
    let missing = desktop_needs();
    for name in &missing {
        assert!(
            name.contains(".so"),
            "{name} is not the name of a library the loader would look for"
        );
    }
    let how = how_to_install_desktop_needs();
    if cfg!(target_os = "linux") {
        assert!(!how.is_empty(), "no advice at all on Linux");
        assert!(
            how.contains("install"),
            "that is not an install command: {how}"
        );
    }
}

// ---------------------------------------------------------------------------
// Uninstalling
// ---------------------------------------------------------------------------

/// The install options a test uses: into its own prefix, touching neither the
/// path nor the applications menu.
fn options_for(prefix: &tempfile::TempDir) -> Options {
    Options {
        prefix: Some(prefix.path().to_path_buf()),
        keep_path: true,
        no_menu: true,
    }
}

#[test]
fn uninstalling_says_what_it_left_alone() {
    let prefix = tempfile::tempdir().unwrap();
    // A home of this test's own, and held for as long as it runs. Without it
    // this reads and writes whichever home some other test happened to set a
    // moment ago — ONIONSKIN_HOME is one variable for the whole process, and
    // tests run beside one another. That is a failure that appears once in
    // however many runs and passes the moment anybody looks at it.
    let home = tempfile::tempdir().unwrap();
    let _held = crate::calibrate::borrow_home(home.path());
    install(&options_for(&prefix)).unwrap();
    let options = options_for(&prefix);

    // Somebody's calibration profiles are their own work and are not the
    // installer's to delete.
    let profiles = crate::calibrate::profiles_dir().unwrap();
    std::fs::create_dir_all(&profiles).unwrap();
    std::fs::write(profiles.join("keepme.json"), b"{}").unwrap();

    let report = uninstall(&options).unwrap();
    assert!(
        report.notes.iter().any(|n| n.contains("calibration")),
        "{:?}",
        report.notes
    );
    assert!(
        profiles.join("keepme.json").is_file(),
        "it deleted the profiles"
    );
    let _ = std::fs::remove_file(profiles.join("keepme.json"));
}

#[test]
fn uninstalling_what_was_never_installed_is_not_an_error() {
    let prefix = tempfile::tempdir().unwrap();
    let options = Options {
        prefix: Some(prefix.path().to_path_buf()),
        keep_path: true,
        no_menu: true,
    };
    let report = uninstall(&options).unwrap();
    assert_eq!(report.binary, None);
}

/// Two copies on the path are both found, in the order the shell would find
/// them.
///
/// The mistake this exists to make visible has no symptom of its own: install
/// a new version while an old one sits earlier on PATH and the old one keeps
/// running, working perfectly, being the wrong program.
#[test]
fn every_copy_on_the_path_is_found_in_the_order_the_shell_would() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first");
    let second = dir.path().join("second");
    std::fs::create_dir_all(&first).unwrap();
    std::fs::create_dir_all(&second).unwrap();
    std::fs::write(first.join("onionskin"), b"one").unwrap();
    std::fs::write(second.join("onionskin"), b"two").unwrap();

    let joined = std::env::join_paths([&first, &second]).unwrap();
    let found = every_binary_on(&joined, "onionskin");
    assert_eq!(found.len(), 2, "{found:?}");
    assert_eq!(found[0], first.join("onionskin"));
    assert_eq!(found[1], second.join("onionskin"));
}

/// The same directory listed twice on PATH is one copy, not two.
///
/// PATH picks up duplicates easily — a shell profile sourced twice is enough —
/// and reporting "2 copies are installed" for one file would send somebody
/// hunting for a second program that does not exist.
#[test]
fn a_directory_on_the_path_twice_is_still_one_copy() {
    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::write(bin.join("onionskin"), b"one").unwrap();

    let joined = std::env::join_paths([&bin, &bin]).unwrap();
    let found = every_binary_on(&joined, "onionskin");
    assert_eq!(found.len(), 1, "{found:?}");
}

#[test]
fn nothing_on_the_path_is_no_copies_rather_than_a_guess() {
    let dir = tempfile::tempdir().unwrap();
    let empty = dir.path().join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    let joined = std::env::join_paths([&empty]).unwrap();
    assert!(every_binary_on(&joined, "onionskin").is_empty());
}
