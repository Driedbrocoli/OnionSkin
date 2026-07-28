//! Tests for building the file people download.
//!
//! An archive writer is easy to test wrongly: ask the writer to read back what
//! it wrote and it will always agree with itself, format bugs and all. So the
//! archives here are unpacked by `tar`, `unzip` and `dpkg-deb` — programs that
//! know nothing about this code — and the bytes that come out are compared with
//! the bytes that went in. Where those programs are missing the test says so
//! and steps aside rather than passing on nothing.

use super::*;

/// Is a program on this machine, and does it run?
fn have(program: &str) -> bool {
    std::process::Command::new(program)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn run(program: &str, args: &[&str]) -> std::process::Output {
    let output = std::process::Command::new(program)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("could not run {program}: {e}"));
    assert!(
        output.status.success(),
        "{program} {args:?} failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

/// A handful of files with awkward shapes: an empty one, one that lands exactly
/// on a block boundary, one that does not, and some bytes that are not text.
fn some_entries() -> Vec<Entry> {
    vec![
        Entry::program("onionskin", (0u8..=255).cycle().take(4096).collect()),
        Entry::file("LICENCE", b"MIT, and here is the notice.\n".to_vec()),
        Entry::file("exactly-one-block", vec![b'x'; 512]),
        Entry::file("empty", Vec::new()),
        Entry::file("THIRD-PARTY-LICENCES", third_party_licences().into_bytes()),
    ]
}

fn licence_file(dir: &Path) -> PathBuf {
    let path = dir.join("LICENCE");
    std::fs::write(&path, "MIT License\n\nCopyright (c) Onionskin\n").unwrap();
    path
}

fn binary_file(dir: &Path) -> PathBuf {
    binary_for(dir, Platform::Linux)
}

/// A pretend binary that starts the way that platform's binaries start.
fn binary_for(dir: &Path, platform: Platform) -> PathBuf {
    let (name, magic): (&str, &[u8]) = match platform {
        Platform::Linux => ("onionskin", b"\x7fELF"),
        Platform::MacOs => ("onionskin", b"\xcf\xfa\xed\xfe"),
        Platform::Windows => ("onionskin.exe", b"MZ"),
    };
    let path = dir.join(name);
    let mut bytes = magic.to_vec();
    bytes.extend_from_slice(b" pretend program");
    std::fs::write(&path, bytes).unwrap();
    path
}

// ---------------------------------------------------------------------------
// tar
// ---------------------------------------------------------------------------

#[test]
fn tar_is_a_whole_number_of_blocks() {
    let bytes = tar(&some_entries());
    assert_eq!(bytes.len() % 512, 0, "{} bytes", bytes.len());
    // It ends with two empty blocks, which is how a reader knows to stop.
    assert!(bytes[bytes.len() - 1024..].iter().all(|b| *b == 0));
}

#[test]
fn the_tar_checksum_is_the_one_readers_check() {
    // Worked out here the way the format says, not the way the writer says:
    // the sum of every byte with the checksum field replaced by spaces.
    for entry in some_entries() {
        let header = tar_header(&entry);
        let mut blanked = header;
        blanked[148..156].fill(b' ');
        let expected: u32 = blanked.iter().map(|b| *b as u32).sum();

        let text = String::from_utf8_lossy(&header[148..156]);
        let digits: String = text.chars().take_while(|c| c.is_ascii_digit()).collect();
        let stored = u32::from_str_radix(&digits, 8).expect("checksum is not octal");
        assert_eq!(stored, expected, "{}", entry.name);
    }
}

#[test]
fn tar_unpacks_with_the_system_tar() {
    if !have("tar") {
        eprintln!("no tar on this machine; skipping");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("test.tar");
    let entries = some_entries();
    std::fs::write(&archive, tar(&entries)).unwrap();

    // `tar t` refuses an archive whose checksums are wrong, so this alone is a
    // real check; extracting is the stronger one.
    let out = dir.path().join("unpacked");
    std::fs::create_dir_all(&out).unwrap();
    run(
        "tar",
        &[
            "-xf",
            archive.to_str().unwrap(),
            "-C",
            out.to_str().unwrap(),
        ],
    );

    for entry in &entries {
        let there = out.join(&entry.name);
        assert!(there.is_file(), "{} did not come out", entry.name);
        assert_eq!(
            std::fs::read(&there).unwrap(),
            entry.bytes,
            "{} came out changed",
            entry.name
        );
    }
}

#[cfg(unix)]
#[test]
fn the_program_is_still_executable_after_a_tar() {
    if !have("tar") {
        eprintln!("no tar on this machine; skipping");
        return;
    }
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("test.tar");
    std::fs::write(&archive, tar(&some_entries())).unwrap();

    let out = dir.path().join("unpacked");
    std::fs::create_dir_all(&out).unwrap();
    run(
        "tar",
        &[
            "-xf",
            archive.to_str().unwrap(),
            "-C",
            out.to_str().unwrap(),
        ],
    );

    // Somebody who unpacks this and cannot run it has to be told to chmod, and
    // will reasonably conclude the download is broken.
    let mode = std::fs::metadata(out.join("onionskin"))
        .unwrap()
        .permissions()
        .mode();
    assert!(mode & 0o111 != 0, "not executable: {mode:o}");

    let plain = std::fs::metadata(out.join("LICENCE"))
        .unwrap()
        .permissions()
        .mode();
    assert!(plain & 0o111 == 0, "the licence is executable: {plain:o}");
}

// ---------------------------------------------------------------------------
// Compression
// ---------------------------------------------------------------------------

#[test]
fn a_tar_gz_unpacks_with_the_system_tar() {
    if !have("tar") {
        eprintln!("no tar on this machine; skipping");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("test.tar.gz");
    let entries = some_entries();
    std::fs::write(&archive, gzip(&tar(&entries))).unwrap();

    let out = dir.path().join("unpacked");
    std::fs::create_dir_all(&out).unwrap();
    run(
        "tar",
        &[
            "-xzf",
            archive.to_str().unwrap(),
            "-C",
            out.to_str().unwrap(),
        ],
    );
    for entry in &entries {
        assert_eq!(
            std::fs::read(out.join(&entry.name)).unwrap(),
            entry.bytes,
            "{} came out changed",
            entry.name
        );
    }
}

#[test]
fn compressing_makes_the_download_smaller() {
    // The whole reason for the dependency. A binary is not already compressed,
    // so if this is not a real saving something is wrong with how it is called.
    let plain = tar(&some_entries());
    let squashed = gzip(&plain);
    assert!(
        squashed.len() < plain.len() / 2,
        "{} bytes became {}, which is barely a saving",
        plain.len(),
        squashed.len()
    );
    // And it is really gzip, which is what the .gz in the name promises.
    assert_eq!(&squashed[0..2], &[0x1f, 0x8b], "not a gzip header");
}

#[test]
fn the_gzip_header_carries_no_clock() {
    // A timestamp from the clock would make two builds of the same input
    // differ, and then nobody can check a download against a published hash.
    let bytes = gzip(b"anything at all");
    assert_eq!(
        &bytes[4..8],
        &[0, 0, 0, 0],
        "there is an mtime in the header"
    );
}

// ---------------------------------------------------------------------------
// zip
// ---------------------------------------------------------------------------

#[test]
fn the_crc_is_the_one_zip_specifies() {
    // The standard check value for CRC-32.
    assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    assert_eq!(crc32(b""), 0);
}

#[test]
fn zip_unpacks_with_the_system_unzip() {
    if !have("unzip") {
        eprintln!("no unzip on this machine; skipping");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("test.zip");
    let entries = some_entries();
    std::fs::write(&archive, zip(&entries)).unwrap();

    // `unzip -t` checks every CRC. A wrong one is exactly the failure that
    // makes Windows say "the compressed folder is invalid".
    run("unzip", &["-t", archive.to_str().unwrap()]);

    let out = dir.path().join("unpacked");
    run(
        "unzip",
        &["-q", archive.to_str().unwrap(), "-d", out.to_str().unwrap()],
    );
    for entry in &entries {
        let there = out.join(&entry.name);
        assert!(there.is_file(), "{} did not come out", entry.name);
        assert_eq!(
            std::fs::read(&there).unwrap(),
            entry.bytes,
            "{} came out changed",
            entry.name
        );
    }
}

#[cfg(unix)]
#[test]
fn the_program_is_still_executable_after_a_zip() {
    if !have("unzip") {
        eprintln!("no unzip on this machine; skipping");
        return;
    }
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let archive = dir.path().join("test.zip");
    std::fs::write(&archive, zip(&some_entries())).unwrap();

    let out = dir.path().join("unpacked");
    run(
        "unzip",
        &["-q", archive.to_str().unwrap(), "-d", out.to_str().unwrap()],
    );
    let mode = std::fs::metadata(out.join("onionskin"))
        .unwrap()
        .permissions()
        .mode();
    assert!(mode & 0o111 != 0, "not executable: {mode:o}");
}

#[test]
fn the_zip_directory_counts_what_is_actually_there() {
    let entries = some_entries();
    let bytes = zip(&entries);
    // The end-of-directory record is the last twenty-two bytes.
    let end = &bytes[bytes.len() - 22..];
    assert_eq!(&end[0..4], &0x0605_4b50u32.to_le_bytes());
    let count = u16::from_le_bytes([end[10], end[11]]) as usize;
    assert_eq!(count, entries.len());

    // And the offset it gives really is where the directory starts.
    let at = u32::from_le_bytes([end[16], end[17], end[18], end[19]]) as usize;
    assert_eq!(&bytes[at..at + 4], &0x0201_4b50u32.to_le_bytes());
}

// ---------------------------------------------------------------------------
// Debian package
// ---------------------------------------------------------------------------

#[test]
fn deb_is_read_by_dpkg() {
    if !have("dpkg-deb") {
        eprintln!("no dpkg-deb on this machine; skipping");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let package = dir.path().join("onionskin_0.1.0_amd64.deb");
    let entries = deb_contents(&some_entries());
    std::fs::write(&package, deb("0.1.0", "amd64", &entries).unwrap()).unwrap();

    // dpkg-deb parses the ar container, both tars and the control file. If any
    // of the three is malformed this is where it shows.
    let info = run("dpkg-deb", &["--info", package.to_str().unwrap()]);
    let text = String::from_utf8_lossy(&info.stdout);
    assert!(text.contains("Package: onionskin"), "{text}");
    assert!(text.contains("Version: 0.1.0"), "{text}");
    assert!(text.contains("Architecture: amd64"), "{text}");

    let listing = run("dpkg-deb", &["--contents", package.to_str().unwrap()]);
    let listing = String::from_utf8_lossy(&listing.stdout);
    assert!(listing.contains("/usr/bin/onionskin"), "{listing}");
    assert!(
        listing.contains("/usr/share/doc/onionskin/LICENCE"),
        "{listing}"
    );

    // And the bytes survive the round trip.
    let out = dir.path().join("unpacked");
    run(
        "dpkg-deb",
        &[
            "--extract",
            package.to_str().unwrap(),
            out.to_str().unwrap(),
        ],
    );
    let binary = out.join("usr/bin/onionskin");
    assert!(binary.is_file(), "the program is not in the package");
    assert_eq!(
        std::fs::read(&binary).unwrap(),
        some_entries()[0].bytes,
        "the program came out changed"
    );
}

#[test]
fn dpkg_can_actually_install_the_package() {
    // Reading the package is not the same as installing it. dpkg does not make
    // missing parent directories, so a package whose data tar lists only files
    // unpacks fine with `tar` and fails at install with "No such file or
    // directory" — which reads like something is wrong with the machine.
    if !have("dpkg-deb") {
        eprintln!("no dpkg on this machine; skipping");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let binary = binary_file(dir.path());
    let licence = licence_file(dir.path());
    let library = dir.path().join("libpdfium.so");
    std::fs::write(&library, b"pretend pdfium").unwrap();

    let entries = contents(Platform::Linux, &binary, Some(&library), &licence).unwrap();
    let package = dir.path().join("onionskin_0.1.0_amd64.deb");
    std::fs::write(
        &package,
        deb("0.1.0", "amd64", &deb_contents(&entries)).unwrap(),
    )
    .unwrap();

    // Unpack into a root of its own, which is what `dpkg --install` does file
    // by file — and which fails the same way when a directory is missing.
    let root = dir.path().join("root");
    std::fs::create_dir_all(&root).unwrap();
    run(
        "dpkg-deb",
        &[
            "--extract",
            package.to_str().unwrap(),
            root.to_str().unwrap(),
        ],
    );

    assert!(root.join("usr/bin/onionskin").is_file());
    assert!(
        root.join("usr/lib/onionskin/libpdfium.so").is_file(),
        "the renderer did not land where the program looks"
    );
    assert!(root.join("usr/share/doc/onionskin/LICENCE").is_file());
}

#[test]
fn every_file_in_the_package_has_its_parent_directory_listed() {
    // The rule dpkg enforces, checked here rather than only where dpkg exists.
    let laid_out = deb_contents(&some_entries());
    let directories: Vec<&str> = laid_out
        .iter()
        .filter(|e| e.directory)
        .map(|e| e.name.trim_end_matches('/'))
        .collect();
    assert!(!directories.is_empty(), "no directories at all");

    for entry in laid_out.iter().filter(|e| !e.directory) {
        let parent = entry.name.rsplit_once('/').unwrap().0;
        assert!(
            directories.contains(&parent),
            "{} has no entry for {parent}: {directories:?}",
            entry.name
        );
    }

    // And each directory's own parent comes before it, because a reader
    // unpacking in order makes them one at a time.
    let mut made: Vec<&str> = vec!["."];
    for name in &directories {
        if let Some((parent, _)) = name.rsplit_once('/') {
            assert!(made.contains(&parent), "{name} comes before {parent}");
        }
        made.push(name);
    }
}

#[test]
fn the_ar_members_are_in_the_order_dpkg_requires() {
    let bytes = deb("0.1.0", "amd64", &some_entries()).unwrap();
    assert_eq!(&bytes[0..8], b"!<arch>\n");
    let text = String::from_utf8_lossy(&bytes);
    let binary_at = text.find("debian-binary").expect("no debian-binary");
    let control_at = text.find("control.tar.gz").expect("no control.tar.gz");
    let data_at = text.find("data.tar.gz").expect("no data.tar.gz");
    assert!(
        binary_at < control_at && control_at < data_at,
        "dpkg reads them in order and stops at the first surprise"
    );
}

#[test]
fn ar_members_start_on_an_even_byte() {
    // An odd-length member is padded, or every member after it is misread.
    let mut out = Vec::new();
    ar_member(&mut out, "odd", b"12345");
    assert_eq!(out.len() % 2, 0, "{} bytes", out.len());
    ar_member(&mut out, "even", b"1234");
    assert_eq!(out.len() % 2, 0);
}

#[test]
fn a_package_without_a_version_is_refused() {
    // dpkg would reject it later with something unhelpful; better here.
    let error = deb("", "amd64", &some_entries()).unwrap_err();
    assert!(error.to_string().contains("version"), "{error}");
}

// ---------------------------------------------------------------------------
// What goes in
// ---------------------------------------------------------------------------

#[test]
fn the_licences_always_travel_with_the_binary() {
    // This is the reason the packager exists. MIT and BSD both say the notice
    // ships with the software, and an archive built by hand is an archive that
    // will one day be built without it.
    let dir = tempfile::tempdir().unwrap();
    let licence = licence_file(dir.path());

    for platform in [Platform::Linux, Platform::MacOs, Platform::Windows] {
        let binary = binary_for(dir.path(), platform);
        let entries = contents(platform, &binary, None, &licence).unwrap();
        let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"LICENCE"), "{platform:?}: {names:?}");
        assert!(
            names.contains(&"THIRD-PARTY-LICENCES"),
            "{platform:?}: {names:?}"
        );
        assert!(
            names.contains(&platform.binary_name()),
            "{platform:?}: {names:?}"
        );
    }
}

#[test]
fn the_third_party_notice_says_what_is_bundled_and_what_is_not() {
    // The notice is hard-wrapped for a terminal, so a phrase can straddle a
    // line break. What matters is that it is said, not where it wraps.
    let text = third_party_licences();
    let flowed = text.split_whitespace().collect::<Vec<_>>().join(" ");

    // pdfium is shipped, so its licence has to be named.
    assert!(flowed.contains("pdfium"), "{text}");
    assert!(flowed.contains("BSD-3-Clause"), "{text}");
    // LibreOffice is not shipped, and the reason belongs in writing: bundling
    // it would put an obligation on whoever passes the archive on.
    assert!(flowed.contains("LibreOffice"), "{text}");
    assert!(flowed.contains("Mozilla Public License 2.0"), "{text}");
    assert!(flowed.contains("not included"), "{text}");
    // And that Onionskin's own terms are stated, not merely implied.
    assert!(flowed.contains("MIT"), "{text}");
}

#[test]
fn the_note_tells_a_stranger_what_to_do_next() {
    for platform in [Platform::Linux, Platform::MacOs, Platform::Windows] {
        let text = readme(platform);
        assert!(text.contains("install"), "{platform:?}");
        assert!(text.contains("doctor"), "{platform:?}");
        assert!(text.contains("uninstall"), "{platform:?}");
        // Somebody has to be able to find the source, or it is not open source
        // in any sense that matters.
        assert!(text.contains("github.com/driedbrocoli/onionskin"), "{text}");
        assert!(text.contains("MIT"), "{text}");
    }
    // The command has to be the one that works on that platform.
    assert!(readme(Platform::Windows).contains("onionskin.exe install"));
    assert!(readme(Platform::Linux).contains("./onionskin install"));
}

#[test]
fn no_two_files_in_an_archive_are_the_same_name_twice() {
    // Windows and macOS do not distinguish Onionskin.exe from onionskin.exe.
    // Two entries that differ only in case unpack to one file there — the
    // second one written wins — and the archive gives no sign of it: the
    // listing shows both, the sizes are right, and the thing simply does not
    // work on the one platform nobody testing on Linux will notice.
    for platform in [Platform::Linux, Platform::MacOs, Platform::Windows] {
        let dir = tempfile::tempdir().unwrap();
        let licence = licence_file(dir.path());
        let cli = binary_for(dir.path(), platform);

        let window = dir.path().join("window");
        let magic: &[u8] = match platform {
            Platform::Linux => b"\x7fELF",
            Platform::MacOs => b"\xcf\xfa\xed\xfe",
            Platform::Windows => b"MZ",
        };
        let mut bytes = magic.to_vec();
        bytes.extend_from_slice(b" pretend window");
        std::fs::write(&window, bytes).unwrap();

        let entries = contents_with_window(platform, &cli, Some(&window), None, &licence).unwrap();
        // What actually goes in the archive, which on macOS is the bundled
        // layout — that is where `Onionskin` stops sitting beside `onionskin`
        // and moves inside a folder, so checking before it would report a
        // collision the download does not have.
        let entries = if platform == Platform::MacOs {
            mac_bundle(&entries, "0.0.0")
        } else {
            entries
        };

        let mut seen: Vec<String> = Vec::new();
        for entry in &entries {
            let folded = entry.name.to_lowercase();
            assert!(
                !seen.contains(&folded),
                "{platform:?}: two entries called {:?} once case is ignored: {:?}",
                entry.name,
                entries.iter().map(|e| &e.name).collect::<Vec<_>>()
            );
            seen.push(folded);
        }
    }
}

#[test]
fn the_window_is_called_what_install_will_go_looking_for() {
    // `onionskin install` copies the window from beside itself, by name. If
    // the archive spells that name differently the window is simply never
    // installed, and the failure is silent — install reports success, because
    // as far as it can tell no window came along.
    // install::desktop_name() answers for the machine this was compiled for,
    // so only that platform's name can be compared against it here. The other
    // two are pinned to what install.rs would say if it were built there,
    // which is the whole point of writing them down twice.
    let here = if cfg!(windows) {
        Platform::Windows
    } else if cfg!(target_os = "macos") {
        Platform::MacOs
    } else {
        Platform::Linux
    };
    if here != Platform::MacOs {
        assert_eq!(
            here.desktop_name(),
            crate::install::desktop_name(),
            "{here:?}"
        );
    }
    assert_eq!(Platform::Windows.desktop_name(), "onionskin-desktop.exe");
    assert_eq!(Platform::Linux.desktop_name(), "onionskin-desktop");
    // macOS is the exception on purpose: what install copies is a plain
    // program, what the archive holds is the same program inside a .app.
    assert_eq!(Platform::MacOs.desktop_name(), "Onionskin");
    assert_eq!(crate::install::desktop_name(), "onionskin-desktop");
}

#[test]
fn the_note_warns_about_the_unsigned_program_dialog() {
    // Windows and macOS both refuse an unsigned program on first run, with
    // wording that sounds like a damaged download. Somebody who believes it
    // deletes the file and never comes back, so the archive has to say so
    // before they meet the dialog.
    let mac = readme(Platform::MacOs);
    assert!(mac.contains("quarantine"), "{mac}");
    assert!(mac.contains("right-click"), "{mac}");

    let windows = readme(Platform::Windows);
    assert!(windows.contains("Windows protected your PC"), "{windows}");
    // The Run button is behind "More info"; naming only one of the two is the
    // same as naming neither.
    assert!(windows.contains("More info"), "{windows}");
    assert!(windows.contains("Run anyway"), "{windows}");

    // Linux has no such gate, and inventing one would only worry people.
    let linux = readme(Platform::Linux);
    assert!(!linux.contains("protected your PC"), "{linux}");
    assert!(!linux.contains("quarantine"), "{linux}");
}

#[test]
fn the_commands_to_type_are_set_apart_from_the_prose() {
    // Rust's line continuation eats the whitespace at the start of the next
    // source line, so an indented line written the obvious way comes out flush
    // left and the command someone is meant to type reads as another sentence.
    for platform in [Platform::Linux, Platform::MacOs, Platform::Windows] {
        let text = readme(platform);
        let install = text
            .lines()
            .find(|line| line.trim_end().ends_with("install") && line.contains("onionskin"))
            .unwrap_or_else(|| panic!("{platform:?}: no install line at all"));
        assert!(
            install.starts_with("    "),
            "{platform:?}: not indented: {install:?}"
        );
        assert!(
            text.lines().any(|l| l.starts_with("    onionskin doctor")),
            "{platform:?}: the things to try next are not indented:\n{text}"
        );
    }
    let notice = third_party_licences();
    assert!(
        notice.lines().any(|l| l.starts_with("    cargo tree")),
        "the command to check the licences is not indented:\n{notice}"
    );
}

#[test]
fn the_licence_list_matches_what_cargo_actually_reports() {
    // The notice makes a claim about somebody else's software. If a new
    // dependency arrives under a licence the notice does not name, the claim
    // becomes false the moment it ships — so check it against cargo itself.
    let Ok(output) = std::process::Command::new("cargo")
        .args(["tree", "--format", "{l}", "--prefix", "none", "--no-dedupe"])
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
    else {
        eprintln!("cargo would not run; skipping");
        return;
    };
    if !output.status.success() {
        eprintln!("cargo tree failed; skipping");
        return;
    }

    let notice = third_party_licences().to_ascii_lowercase();
    let listed = String::from_utf8_lossy(&output.stdout);

    for expression in listed.lines().map(str::trim).filter(|l| !l.is_empty()) {
        // Every licence named has to be somewhere in the notice, whether it is
        // one we take or one we decline — a reader comparing the two should
        // not find a name in `cargo tree` that the notice never mentions.
        for name in names_in(expression) {
            assert!(
                notice.contains(&name.to_ascii_lowercase()),
                "a dependency is offered under {name}, which THIRD-PARTY-LICENCES \
                 does not mention (from {expression:?})"
            );
        }

        // And the claim that no copyleft obligation reaches this program has
        // to stay true. `A OR B` is a choice, not a requirement: a crate
        // offered as "Apache-2.0 OR GPL-2.0-only" is taken under Apache and
        // carries no GPL obligation at all. Reading it as a requirement is
        // what the first version of this test did, and it would have failed
        // the whole tree over a crate that is perfectly fine.
        assert!(
            permissive(expression),
            "{expression:?} leaves no permissive way to use the crate, and the \
             notice says every one of them has one"
        );
    }
}

/// Is there a way to take this licence expression that carries no copyleft?
///
/// `AND` binds every part: all of them must be satisfiable. `OR` offers a
/// choice: one satisfiable part is enough.
fn permissive(expression: &str) -> bool {
    // Split on AND first, since it is the weaker binding of the two here.
    expression
        .split(" AND ")
        .map(|part| part.trim().trim_matches(['(', ')']).trim())
        .all(|part| {
            part.split(" OR ")
                // The old slash form, which a few crates still use.
                .flat_map(|one| one.split('/'))
                .any(|one| !is_copyleft(one.trim()))
        })
}

fn is_copyleft(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    // A font licence is not code copyleft. OFL and the Ubuntu font licence
    // govern the typeface files and ask that a modified font be renamed; they
    // place no condition on the program that draws with them.
    ["gpl", "mpl", "epl", "cddl", "sspl"]
        .iter()
        .any(|bad| lower.contains(bad))
}

/// Every licence named in an expression, however it is joined.
fn names_in(expression: &str) -> Vec<String> {
    expression
        .split([',', '/'])
        .flat_map(|part| part.split(" AND "))
        .flat_map(|part| part.split(" OR "))
        .map(|name| name.trim().trim_matches(['(', ')']).trim().to_string())
        .filter(|name| !name.is_empty())
        .collect()
}

#[test]
fn the_library_comes_along_when_there_is_one() {
    let dir = tempfile::tempdir().unwrap();
    let binary = binary_file(dir.path());
    let licence = licence_file(dir.path());
    let library = dir.path().join("libpdfium.so");
    std::fs::write(&library, b"pretend pdfium").unwrap();

    let without = contents(Platform::Linux, &binary, None, &licence).unwrap();
    let with = contents(Platform::Linux, &binary, Some(&library), &licence).unwrap();
    assert_eq!(with.len(), without.len() + 1);
    let named = with.iter().find(|e| e.name == "libpdfium.so").unwrap();
    assert_eq!(named.bytes, b"pretend pdfium");
}

#[test]
fn a_binary_for_the_wrong_platform_is_refused() {
    // Building on one machine for three platforms is the ordinary case, so
    // renaming a Linux binary to onionskin.exe is not a hypothetical mistake.
    // An archive with the wrong program in it looks completely normal until
    // somebody downloads it and it will not run.
    let dir = tempfile::tempdir().unwrap();
    let licence = licence_file(dir.path());

    for (have, want) in [
        (Platform::Linux, Platform::Windows),
        (Platform::Linux, Platform::MacOs),
        (Platform::Windows, Platform::Linux),
        (Platform::MacOs, Platform::Windows),
    ] {
        let binary = binary_for(dir.path(), have);
        let error = contents(want, &binary, None, &licence)
            .map(|_| ())
            .unwrap_err();
        let said = error.to_string();
        assert!(
            said.contains(describe(have)) && said.contains(want.name()),
            "{have:?} packaged as {want:?} said: {said}"
        );
    }

    // And each platform's own binary goes through.
    for platform in [Platform::Linux, Platform::MacOs, Platform::Windows] {
        let binary = binary_for(dir.path(), platform);
        contents(platform, &binary, None, &licence)
            .unwrap_or_else(|e| panic!("{platform:?} refused its own binary: {e}"));
    }
}

#[test]
fn something_that_is_not_a_program_at_all_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let licence = licence_file(dir.path());
    let not_a_program = dir.path().join("notes.txt");
    std::fs::write(&not_a_program, "I meant to build this first").unwrap();

    let error = contents(Platform::Linux, &not_a_program, None, &licence)
        .map(|_| ())
        .unwrap_err();
    assert!(error.to_string().contains("does not look like a program"));
    assert_eq!(built_for(b"hello"), None);
}

#[test]
fn a_missing_file_is_named_in_the_error() {
    let dir = tempfile::tempdir().unwrap();
    let licence = licence_file(dir.path());
    let error = contents(Platform::Linux, &dir.path().join("nothing"), None, &licence).unwrap_err();
    assert!(error.to_string().contains("nothing"), "{error}");
}

#[test]
fn each_platform_gets_the_names_it_uses() {
    assert_eq!(Platform::Windows.binary_name(), "onionskin.exe");
    assert_eq!(Platform::Linux.binary_name(), "onionskin");
    assert_eq!(Platform::MacOs.library_name(), "libpdfium.dylib");
    assert_eq!(Platform::Windows.library_name(), "pdfium.dll");
    // Windows opens a zip with a double click and nothing else does tar.
    assert_eq!(Platform::Windows.archive_extension(), "zip");
    assert_eq!(Platform::MacOs.archive_extension(), "tar.gz");

    assert_eq!(Platform::parse("Linux"), Some(Platform::Linux));
    assert_eq!(Platform::parse(" darwin "), Some(Platform::MacOs));
    assert_eq!(Platform::parse("win"), Some(Platform::Windows));
    assert_eq!(Platform::parse("plan9"), None);
}

#[test]
fn a_debian_package_puts_things_where_debian_puts_them() {
    let laid_out = deb_contents(&some_entries());
    let names: Vec<&str> = laid_out.iter().map(|e| e.name.as_str()).collect();
    assert!(names.contains(&"./usr/bin/onionskin"), "{names:?}");
    assert!(
        names.contains(&"./usr/share/doc/onionskin/LICENCE"),
        "{names:?}"
    );
    // Every path is relative, which is what a tar inside a deb must be.
    for name in &names {
        assert!(name.starts_with("./"), "{name} is not relative");
    }
}

#[test]
fn the_renderer_is_put_where_the_program_will_look_for_it() {
    // A .deb cannot drop a private library into /usr/bin beside the binary, so
    // it goes in a directory of the package's own — and if that directory is
    // not one the program searches, everyone who installs the .deb loses PDF
    // rendering with nothing to say why. These two lists must agree.
    let dir = tempfile::tempdir().unwrap();
    let binary = binary_file(dir.path());
    let licence = licence_file(dir.path());
    let library = dir.path().join("libpdfium.so");
    std::fs::write(&library, b"pretend pdfium").unwrap();

    let entries = contents(Platform::Linux, &binary, Some(&library), &licence).unwrap();
    let installed = deb_contents(&entries)
        .into_iter()
        .find(|e| e.name.contains("libpdfium"))
        .expect("the renderer is not in the package at all");

    // "./usr/lib/onionskin/libpdfium.so" is installed as "/usr/lib/...".
    let absolute = installed.name.trim_start_matches('.').to_string();
    assert!(
        crate::render::PACKAGED_LIBRARY_PATHS.contains(&absolute.as_str()),
        "the package puts the renderer at {absolute}, which the program never \
         looks in: {:?}",
        crate::render::PACKAGED_LIBRARY_PATHS
    );
}

// ---------------------------------------------------------------------------
// Building the lot
// ---------------------------------------------------------------------------

#[test]
fn building_writes_what_each_platform_expects() {
    let dir = tempfile::tempdir().unwrap();
    let binary = binary_file(dir.path());
    let licence = licence_file(dir.path());
    let out = dir.path().join("dist");

    let linux = build(Platform::Linux, &binary, None, &licence, "0.1.0", &out).unwrap();
    let names: Vec<String> = linux
        .iter()
        .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    assert!(
        names.contains(&"onionskin-0.1.0-linux.tar.gz".to_string()),
        "{names:?}"
    );
    assert!(
        names.contains(&"onionskin_0.1.0_amd64.deb".to_string()),
        "{names:?}"
    );

    let exe = binary_for(dir.path(), Platform::Windows);
    let windows = build(Platform::Windows, &exe, None, &licence, "0.1.0", &out).unwrap();
    assert_eq!(windows.len(), 1, "there is no .deb for Windows");
    assert!(windows[0].to_string_lossy().ends_with("windows.zip"));

    for path in linux.iter().chain(&windows) {
        assert!(path.is_file(), "{} was not written", path.display());
        assert!(std::fs::metadata(path).unwrap().len() > 0);
    }
}

#[test]
fn the_same_input_builds_the_same_bytes_twice() {
    // A package that differs between builds cannot be checked against a hash
    // somebody else published, which is how people who did not build it decide
    // whether to trust it.
    let dir = tempfile::tempdir().unwrap();
    let binary = binary_file(dir.path());
    let licence = licence_file(dir.path());
    let entries = contents(Platform::Linux, &binary, None, &licence).unwrap();

    assert_eq!(tar(&entries), tar(&entries));
    assert_eq!(zip(&entries), zip(&entries));
    assert_eq!(
        deb("0.1.0", "amd64", &entries).unwrap(),
        deb("0.1.0", "amd64", &entries).unwrap()
    );

    // And through the whole of `build`, which writes to disk in between.
    let first = dir.path().join("one");
    let second = dir.path().join("two");
    build(Platform::Linux, &binary, None, &licence, "0.1.0", &first).unwrap();
    build(Platform::Linux, &binary, None, &licence, "0.1.0", &second).unwrap();
    for name in ["onionskin-0.1.0-linux.tar.gz", "onionskin_0.1.0_amd64.deb"] {
        assert_eq!(
            std::fs::read(first.join(name)).unwrap(),
            std::fs::read(second.join(name)).unwrap(),
            "{name} differs between builds"
        );
    }
}

// ---------------------------------------------------------------------------
// The window in the archive
// ---------------------------------------------------------------------------

#[test]
fn the_window_goes_in_the_archive_beside_the_command_line() {
    let dir = tempfile::tempdir().unwrap();
    let licence = licence_file(dir.path());
    let cli = binary_for(dir.path(), Platform::Linux);
    let window = dir.path().join("onionskin-desktop");
    std::fs::write(&window, b"\x7fELF pretend window").unwrap();

    let entries =
        contents_with_window(Platform::Linux, &cli, Some(&window), None, &licence).unwrap();
    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    assert!(names.contains(&"onionskin"), "{names:?}");
    assert!(names.contains(&"onionskin-desktop"), "{names:?}");

    // Both have to be executable, or somebody unpacks the archive and is told
    // permission denied by a program that is sitting right there.
    for wanted in ["onionskin", "onionskin-desktop"] {
        let entry = entries.iter().find(|e| e.name == wanted).unwrap();
        assert!(
            entry.mode & 0o111 != 0,
            "{wanted} is not executable: {:o}",
            entry.mode
        );
    }
}

#[test]
fn a_window_for_the_wrong_platform_is_refused_too() {
    // The same trap as the command line program, and just as invisible: an
    // archive with a Linux window renamed to .exe looks entirely normal.
    let dir = tempfile::tempdir().unwrap();
    let licence = licence_file(dir.path());
    let cli = binary_for(dir.path(), Platform::Windows);
    let window = dir.path().join("wrong-desktop");
    std::fs::write(&window, b"\x7fELF a Linux window").unwrap();

    let error = contents_with_window(Platform::Windows, &cli, Some(&window), None, &licence)
        .map(|_| ())
        .unwrap_err();
    assert!(error.to_string().contains("Linux program"), "{error}");
}

#[test]
fn the_mac_archive_is_an_application_bundle() {
    // A bare executable on macOS is a terminal command: double-clicking it
    // opens Terminal, with no icon and no name in the Dock. An application is
    // a folder of a particular shape, and the Finder treats it as one thing.
    let dir = tempfile::tempdir().unwrap();
    let licence = licence_file(dir.path());
    let cli = binary_for(dir.path(), Platform::MacOs);
    let window = dir.path().join("onionskin-desktop");
    std::fs::write(&window, b"\xcf\xfa\xed\xfe pretend window").unwrap();

    let entries =
        contents_with_window(Platform::MacOs, &cli, Some(&window), None, &licence).unwrap();
    let bundle = mac_bundle(&entries, "0.1.0");
    let names: Vec<&str> = bundle.iter().map(|e| e.name.as_str()).collect();

    for wanted in [
        "Onionskin.app/",
        "Onionskin.app/Contents/",
        "Onionskin.app/Contents/MacOS/",
        "Onionskin.app/Contents/Info.plist",
        "Onionskin.app/Contents/PkgInfo",
        "Onionskin.app/Contents/MacOS/Onionskin",
    ] {
        assert!(
            names
                .iter()
                .any(|n| *n == wanted || *n == wanted.trim_end_matches('/')),
            "{wanted} is not in the bundle: {names:?}"
        );
    }
    // The command line program stays outside the bundle: a path buried inside
    // one is not a path anybody types.
    assert!(names.contains(&"onionskin"), "{names:?}");

    let plist = bundle
        .iter()
        .find(|e| e.name.ends_with("Info.plist"))
        .expect("no Info.plist");
    let text = String::from_utf8_lossy(&plist.bytes);
    // The executable named in the plist has to be the one that is actually
    // there, or macOS reports the application as damaged.
    assert!(
        text.contains("<key>CFBundleExecutable</key><string>Onionskin</string>"),
        "{text}"
    );
    assert!(text.contains("<string>0.1.0</string>"), "{text}");
    // Without this, macOS runs the window at half resolution and scales it up,
    // and every letter is soft.
    assert!(text.contains("NSHighResolutionCapable"), "{text}");
}

#[test]
fn a_debian_package_with_a_window_gets_a_menu_entry() {
    // A package that installs a window and no menu entry has installed
    // something invisible: people look in the applications menu, not in
    // /usr/bin.
    let dir = tempfile::tempdir().unwrap();
    let licence = licence_file(dir.path());
    let cli = binary_for(dir.path(), Platform::Linux);
    let window = dir.path().join("onionskin-desktop");
    std::fs::write(&window, b"\x7fELF pretend window").unwrap();

    let entries =
        contents_with_window(Platform::Linux, &cli, Some(&window), None, &licence).unwrap();
    let laid_out = deb_contents(&entries);
    let names: Vec<&str> = laid_out.iter().map(|e| e.name.as_str()).collect();
    assert!(names.contains(&"./usr/bin/onionskin-desktop"), "{names:?}");
    assert!(
        names.contains(&"./usr/share/applications/onionskin.desktop"),
        "{names:?}"
    );

    let entry = laid_out
        .iter()
        .find(|e| e.name.ends_with("onionskin.desktop"))
        .unwrap();
    let text = String::from_utf8_lossy(&entry.bytes);
    assert!(text.contains("Exec=onionskin-desktop"), "{text}");
    assert!(text.contains("Terminal=false"), "{text}");

    // And a package without a window gets no menu entry, because there would
    // be nothing for it to open.
    let plain = deb_contents(&contents(Platform::Linux, &cli, None, &licence).unwrap());
    assert!(
        !plain.iter().any(|e| e.name.ends_with("onionskin.desktop")),
        "a menu entry with nothing to launch"
    );
}

#[test]
fn dpkg_reads_a_package_that_has_a_window_in_it() {
    if !have("dpkg-deb") {
        eprintln!("no dpkg-deb on this machine; skipping");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let licence = licence_file(dir.path());
    let cli = binary_for(dir.path(), Platform::Linux);
    let window = dir.path().join("onionskin-desktop");
    std::fs::write(&window, b"\x7fELF pretend window").unwrap();

    let entries =
        contents_with_window(Platform::Linux, &cli, Some(&window), None, &licence).unwrap();
    let package = dir.path().join("onionskin_0.1.0_amd64.deb");
    std::fs::write(
        &package,
        deb("0.1.0", "amd64", &deb_contents(&entries)).unwrap(),
    )
    .unwrap();

    let out = dir.path().join("root");
    std::fs::create_dir_all(&out).unwrap();
    run(
        "dpkg-deb",
        &[
            "--extract",
            package.to_str().unwrap(),
            out.to_str().unwrap(),
        ],
    );
    assert!(out.join("usr/bin/onionskin-desktop").is_file());
    assert!(out
        .join("usr/share/applications/onionskin.desktop")
        .is_file());
}
