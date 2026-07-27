//! Tests for the apt repository.
//!
//! Three things here are easy to test wrongly. A hash function can be checked
//! against itself all day and agree with itself all day, so the ones below are
//! the published SHA-256 vectors and, where the machine has `sha256sum`, that
//! program's answer for bytes it has never seen. A date formatter written from
//! memory is wrong about century leap years and right about everything else, so
//! it is checked on both sides of 2000, 2024 and 2100. And a `Packages` file
//! read back by the code that wrote it will always parse, so where
//! `dpkg-scanpackages` or `apt-ftparchive` is installed the catalogue is
//! compared against what Debian's own tool makes of the same package.
//!
//! Where those programs are missing the test says so and steps aside rather
//! than passing on nothing.

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

fn at(seconds: u64) -> SystemTime {
    UNIX_EPOCH + std::time::Duration::from_secs(seconds)
}

/// A real `.deb`, built by the packager this repository is meant to publish.
fn a_package(dir: &Path, version: &str, architecture: &str) -> PathBuf {
    let entries = crate::package::deb_contents(&[
        crate::package::Entry::program(
            "onionskin",
            (0u8..=255).cycle().take(4096).collect::<Vec<u8>>(),
        ),
        crate::package::Entry::file("LICENCE", b"MIT, and here is the notice.\n".to_vec()),
    ]);
    let bytes = crate::package::deb(version, architecture, &entries).unwrap();
    let path = dir.join(format!("built_{version}_{architecture}.deb"));
    std::fs::write(&path, bytes).unwrap();
    path
}

/// A control file split into stanzas, each a list of fields in order.
fn stanzas(text: &str) -> Vec<Vec<(String, String)>> {
    text.split("\n\n")
        .filter(|piece| !piece.trim().is_empty())
        .map(stanza_fields)
        .collect()
}

fn value<'a>(fields: &'a [(String, String)], name: &str) -> Option<&'a str> {
    fields
        .iter()
        .find(|(have, _)| have.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

// ---------------------------------------------------------------------------
// SHA-256
// ---------------------------------------------------------------------------

#[test]
fn the_hashes_are_the_published_sha_256_test_vectors() {
    // The four messages every SHA-256 implementation is checked against. The
    // last two are the interesting ones: fifty-six bytes is exactly the length
    // at which the padding no longer leaves room for the length and a second
    // block is needed, and a hundred and twelve bytes is the same trap one
    // block further along.
    assert_eq!(
        sha256_hex(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );

    let four_hundred_and_forty_eight_bits =
        b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq";
    assert_eq!(four_hundred_and_forty_eight_bits.len(), 56);
    assert_eq!(
        sha256_hex(four_hundred_and_forty_eight_bits),
        "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
    );

    let eight_hundred_and_ninety_six_bits = b"abcdefghbcdefghicdefghijdefghijkefghijklfghijklmghijklmnhijklmnoijklmnopjklmnopqklmnopqrlmnopqrsmnopqrstnopqrstu";
    assert_eq!(eight_hundred_and_ninety_six_bits.len(), 112);
    assert_eq!(
        sha256_hex(eight_hundred_and_ninety_six_bits),
        "cf5b16a778af8380036ce59e7b0492370b249b11e8f07a51afac45037afee9d1"
    );
}

#[test]
fn a_message_of_over_a_megabyte_hashes_correctly_too() {
    // The length is appended in bits, not bytes, as a sixty-four bit number.
    // Every short message hashes correctly whether that is understood or not,
    // because the top bytes of the length are zero either way. A million bytes
    // is where the mistake shows: at eight million bits the length no longer
    // fits in three bytes, and an implementation that wrote the byte count, or
    // wrote the length in the wrong byte order, gives the wrong answer here and
    // the right one everywhere else.
    let million = vec![b'a'; 1_000_000];
    assert_eq!(
        sha256_hex(&million),
        "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
    );

    // And one comfortably over a megabyte, of bytes that are not all the same
    // — so a bug that mixed the block schedule in the wrong order could not
    // hide behind every block being identical. Checked against `sha256sum`.
    let long: Vec<u8> = (0..1_500_000u32).map(|i| (i % 251) as u8).collect();
    assert!(long.len() > 1024 * 1024);
    assert_eq!(
        sha256_hex(&long),
        "5596d05b12f12e268d4d9418b201f81533c985c09d28f2b91c2b013174e1be48"
    );
}

/// What `sha256sum` says, for bytes it is handed on standard input.
///
/// Only ever called with a few hundred bytes. Writing to a child's pipe without
/// reading its output at the same time deadlocks once the payload is bigger
/// than the pipe's buffer, and the deadlock is a test that hangs rather than a
/// test that fails.
fn sha256sum(bytes: &[u8]) -> Option<String> {
    use std::io::Write;
    let mut child = std::process::Command::new("sha256sum")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    child.stdin.take()?.write_all(bytes).ok()?;
    let output = child.wait_with_output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    Some(text.split_whitespace().next()?.to_string())
}

#[test]
fn the_hashes_agree_with_a_program_that_knows_nothing_about_this_code() {
    if !have("sha256sum") {
        eprintln!("no sha256sum on this machine; skipping");
        return;
    }
    // Every length from nothing up to two blocks and a bit. This walks over the
    // fifty-five, fifty-six, sixty-three and sixty-four byte boundaries where
    // the padding either does or does not need a block of its own — which is
    // the one part of the algorithm a fixed handful of vectors can step past.
    for length in 0..=200usize {
        let bytes: Vec<u8> = (0..length).map(|i| (i * 7 + 3) as u8).collect();
        let Some(theirs) = sha256sum(&bytes) else {
            eprintln!("sha256sum would not run; skipping");
            return;
        };
        assert_eq!(sha256_hex(&bytes), theirs, "{length} bytes");
    }
}

// ---------------------------------------------------------------------------
// The date
// ---------------------------------------------------------------------------

#[test]
fn the_date_is_written_the_way_a_release_file_writes_it() {
    // The epoch, which is the one date a broken conversion still gets right,
    // and so the one worth pinning first.
    assert_eq!(rfc1123(at(0)), "Thu, 01 Jan 1970 00:00:00 UTC");
    // The moment before it, which cannot be written on a `deb` line at all and
    // is deliberately not an error.
    assert_eq!(
        rfc1123(UNIX_EPOCH - std::time::Duration::from_secs(1)),
        "Thu, 01 Jan 1970 00:00:00 UTC"
    );
    assert_eq!(rfc1123(at(1_785_190_200)), "Mon, 27 Jul 2026 22:10:00 UTC");
    // The day carries a leading zero and the hours, minutes and seconds do too,
    // because RFC 1123 says so and because a reader splitting on spaces would
    // otherwise find a different number of fields on different days.
    assert_eq!(rfc1123(at(1_677_628_800)), "Wed, 01 Mar 2023 00:00:00 UTC");
    assert_eq!(rfc1123(at(1_483_228_799)), "Sat, 31 Dec 2016 23:59:59 UTC");
}

#[test]
fn the_leap_year_rule_is_the_one_the_calendar_actually_uses() {
    // 2024 is an ordinary leap year: divisible by four, so there is a 29th.
    assert_eq!(rfc1123(at(1_709_210_096)), "Thu, 29 Feb 2024 12:34:56 UTC");
    assert_eq!(rfc1123(at(1_709_164_799)), "Wed, 28 Feb 2024 23:59:59 UTC");

    // 2000 is the exception to the exception. A century is not a leap year
    // unless it divides by four hundred, and 2000 does — so 29 February 2000
    // existed, and 1 January 2000 is a Saturday and not a Sunday.
    assert_eq!(rfc1123(at(946_684_800)), "Sat, 01 Jan 2000 00:00:00 UTC");
    assert_eq!(rfc1123(at(946_684_799)), "Fri, 31 Dec 1999 23:59:59 UTC");
    assert_eq!(civil_from_days_of("2000-02-29"), (2000, 2, 29));

    // 2100 is a century that does not divide by four hundred, so it is not a
    // leap year and February has twenty-eight days. This is the case the
    // "every fourth year" rule gets wrong, and it gets it wrong by one day
    // for the following seventy-eight years.
    assert_eq!(rfc1123(at(4_107_542_400)), "Mon, 01 Mar 2100 00:00:00 UTC");
    assert_eq!(rfc1123(at(4_107_542_399)), "Sun, 28 Feb 2100 23:59:59 UTC");

    // And the next four-hundred-year exception, which is a leap year again.
    assert_eq!(rfc1123(at(13_574_563_200)), "Tue, 29 Feb 2400 00:00:00 UTC");
}

/// The civil date some days after the epoch, given as a date to save counting.
fn civil_from_days_of(date: &str) -> (i64, u32, u32) {
    // Days from 1 January 1970 to the given date, worked out here rather than
    // by the code under test — by counting whole years and the leap days in
    // between, which is the slow, obvious method the fast one has to agree
    // with.
    let mut parts = date.split('-');
    let year: i64 = parts.next().unwrap().parse().unwrap();
    let month: i64 = parts.next().unwrap().parse().unwrap();
    let day: i64 = parts.next().unwrap().parse().unwrap();

    let leap = |y: i64| y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let mut days = 0i64;
    for y in 1970..year {
        days += if leap(y) { 366 } else { 365 };
    }
    let lengths = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    for m in 1..month {
        days += lengths[(m - 1) as usize] + i64::from(m == 2 && leap(year));
    }
    days += day - 1;
    civil_from_days(days)
}

#[test]
fn every_day_for_a_century_converts_back_to_itself() {
    // The conversion is arithmetic with no loop in it, so a mistake would not
    // be in one branch — it would be an off-by-one that appears on one day in
    // every month, or one day in every leap year, and a handful of hand-picked
    // dates can walk straight past it. So walk the lot: every day from 1970 to
    // 2070, checked against a counter that only ever adds one.
    let leap = |y: i64| y % 4 == 0 && (y % 100 != 0 || y % 400 == 0);
    let lengths = [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let (mut year, mut month, mut day) = (1970i64, 1u32, 1u32);
    for days in 0..36_525 {
        assert_eq!(
            civil_from_days(days),
            (year, month, day),
            "{days} days after the epoch"
        );
        let last = lengths[(month - 1) as usize] + u32::from(month == 2 && leap(year));
        day += 1;
        if day > last {
            day = 1;
            month += 1;
        }
        if month > 12 {
            month = 1;
            year += 1;
        }
    }
}

// ---------------------------------------------------------------------------
// Reading a package back
// ---------------------------------------------------------------------------

#[test]
fn the_control_fields_come_back_out_of_the_package_they_went_into() {
    let dir = tempfile::tempdir().unwrap();
    let bytes = std::fs::read(a_package(dir.path(), "0.1.0", "amd64")).unwrap();
    let control = control(&bytes).unwrap();

    assert_eq!(control.field("Package"), Some("onionskin"));
    assert_eq!(control.field("Version"), Some("0.1.0"));
    assert_eq!(control.field("Architecture"), Some("amd64"));
    assert_eq!(control.field("Maintainer"), Some("Onionskin"));
    assert_eq!(control.field("Section"), Some("utils"));
    assert_eq!(control.field("Priority"), Some("optional"));
    assert_eq!(
        control.field("Homepage"),
        Some("https://github.com/driedbrocoli/onionskin")
    );
    assert!(control.field("Installed-Size").unwrap().parse::<u64>().is_ok());

    // Field names are matched without regard to case, which is what the format
    // says. A package built by hand with `installed-size` in it would otherwise
    // be catalogued with the field missing and nothing to say why.
    assert_eq!(control.field("PACKAGE"), control.field("package"));

    // The description runs to several lines, and the continuation lines have to
    // come back with their leading space still on them — writing
    // `Description: {value}` has to reproduce what the package said, or apt
    // reads the second line as a field it does not know.
    let description = control.field("Description").unwrap();
    assert!(description.starts_with("Add words to a page"), "{description}");
    let (first, rest) = description.split_once('\n').unwrap();
    assert!(!first.is_empty());
    for line in rest.lines() {
        assert!(line.starts_with(' '), "continuation line lost its space: {line:?}");
    }
    // The paragraph break in a Debian description is a full stop on its own.
    assert!(rest.lines().any(|line| line.trim() == "."), "{description}");
}

#[test]
fn something_that_is_not_a_package_is_refused_with_a_sentence() {
    // Every one of these means the same thing to whoever is holding the file —
    // this is not a package — and what differs is only which part gave it away.
    let plain = control(b"I meant to build this first").unwrap_err();
    assert!(plain.contains("`ar`"), "{plain}");

    // An `ar` archive with nothing in it that a .deb has.
    let mut empty = b"!<arch>\n".to_vec();
    let mut member = format!("{:<16}{:<12}{:<6}{:<6}{:<8}{:<10}`\n", "notes.txt", 0, 0, 0, "100644", 5);
    member.push_str("hello");
    empty.extend_from_slice(member.as_bytes());
    let missing = control(&empty).unwrap_err();
    assert!(missing.contains("control.tar"), "{missing}");
}

// ---------------------------------------------------------------------------
// The layout
// ---------------------------------------------------------------------------

#[test]
fn a_repository_is_laid_out_where_apt_goes_looking_for_it() {
    let dir = tempfile::tempdir().unwrap();
    let deb = a_package(dir.path(), "0.1.0", "amd64");
    let out = dir.path().join("repo");
    let built = build(&[deb], &out, &RepoOptions::default(), at(1_785_190_200)).unwrap();

    // The four paths apt asks for, in the shapes it asks for them.
    let pool = out.join("pool/main/o/onionskin/onionskin_0.1.0_amd64.deb");
    assert!(pool.is_file(), "no package in the pool");
    assert!(out.join("dists/stable/main/binary-amd64/Packages").is_file());
    assert!(out.join("dists/stable/main/binary-amd64/Packages.gz").is_file());
    assert!(out.join("dists/stable/Release").is_file());

    assert_eq!(built.root, out);
    assert_eq!(built.release, out.join("dists/stable/Release"));
    assert_eq!(
        built.packages,
        vec!["pool/main/o/onionskin/onionskin_0.1.0_amd64.deb".to_string()]
    );
    assert_eq!(built.architectures, vec!["amd64".to_string()]);

    // The path in `packages` is a repository path and not this machine's, so it
    // is separated by forward slashes wherever this is built.
    assert!(!built.packages[0].contains('\\'), "{:?}", built.packages);
}

#[test]
fn the_catalogue_names_the_file_its_size_and_its_hash() {
    let dir = tempfile::tempdir().unwrap();
    let deb = a_package(dir.path(), "0.1.0", "amd64");
    let bytes = std::fs::read(&deb).unwrap();
    let out = dir.path().join("repo");
    build(&[deb], &out, &RepoOptions::default(), at(1_785_190_200)).unwrap();

    let text = std::fs::read_to_string(out.join("dists/stable/main/binary-amd64/Packages")).unwrap();
    let found = stanzas(&text);
    assert_eq!(found.len(), 1, "{text}");
    let fields = &found[0];

    assert_eq!(value(fields, "Package"), Some("onionskin"));
    assert_eq!(value(fields, "Version"), Some("0.1.0"));
    assert_eq!(value(fields, "Architecture"), Some("amd64"));
    assert_eq!(
        value(fields, "Filename"),
        Some("pool/main/o/onionskin/onionskin_0.1.0_amd64.deb")
    );
    // The size and the hash are of the file as it sits in the pool. If either
    // is of something else — the file before it was copied, say — apt
    // downloads the package, finds it does not match, and reports a corrupt
    // mirror to somebody who has no way of knowing what is really wrong.
    assert_eq!(value(fields, "Size"), Some(bytes.len().to_string().as_str()));
    assert_eq!(value(fields, "SHA256"), Some(sha256_hex(&bytes).as_str()));

    let pooled = std::fs::read(out.join("pool/main/o/onionskin/onionskin_0.1.0_amd64.deb")).unwrap();
    assert_eq!(pooled, bytes, "the package changed on its way into the pool");

    // No weaker hash offered beside the strong one.
    assert_eq!(value(fields, "MD5sum"), None, "{text}");
    assert_eq!(value(fields, "SHA1"), None, "{text}");

    // Description is last, because it is the one field that runs to several
    // lines and a reader that loses its place in a stanza loses the rest of it.
    assert_eq!(fields.last().map(|(name, _)| name.as_str()), Some("Description"));
    // And a blank line ends the stanza, which is what separates one package
    // from the next.
    assert!(text.ends_with("\n\n"), "{text:?}");
}

#[test]
fn the_release_file_lists_both_catalogues_with_their_hashes_and_sizes() {
    let dir = tempfile::tempdir().unwrap();
    let deb = a_package(dir.path(), "0.1.0", "amd64");
    let out = dir.path().join("repo");
    let options = RepoOptions {
        suite: "stable".to_string(),
        component: "main".to_string(),
        origin: "Onionskin".to_string(),
        label: "Onionskin".to_string(),
        // Two lines, to prove they are folded into one. A newline here would
        // turn the rest of the sentence into a field name apt does not know.
        description: "Onionskin packages\nfor Debian and Ubuntu".to_string(),
    };
    build(&[deb], &out, &options, at(1_785_190_200)).unwrap();

    let release = std::fs::read_to_string(out.join("dists/stable/Release")).unwrap();
    for (name, wanted) in [
        ("Origin", "Onionskin"),
        ("Label", "Onionskin"),
        ("Suite", "stable"),
        ("Codename", "stable"),
        ("Architectures", "amd64"),
        ("Components", "main"),
        ("Description", "Onionskin packages for Debian and Ubuntu"),
        ("Date", "Mon, 27 Jul 2026 22:10:00 UTC"),
    ] {
        assert!(
            release.contains(&format!("{name}: {wanted}\n")),
            "no `{name}: {wanted}` in:\n{release}"
        );
    }

    // The SHA256 block: hash, length and path for each catalogue, with the
    // paths relative to dists/<suite>/ — which is where the Release file itself
    // sits, and where apt resolves them from.
    let block = release.split_once("SHA256:\n").expect("no SHA256 block").1;
    let listed: Vec<(&str, u64, &str)> = block
        .lines()
        .filter(|line| line.starts_with(' '))
        .map(|line| {
            let mut parts = line.split_whitespace();
            let hash = parts.next().unwrap();
            let size: u64 = parts.next().unwrap().parse().unwrap();
            (hash, size, parts.next().unwrap())
        })
        .collect();
    assert_eq!(listed.len(), 2, "{release}");

    for (hash, size, path) in listed {
        assert!(
            path == "main/binary-amd64/Packages" || path == "main/binary-amd64/Packages.gz",
            "{path} is not a path relative to dists/stable/"
        );
        let on_disk = std::fs::read(out.join("dists/stable").join(path)).unwrap();
        assert_eq!(size, on_disk.len() as u64, "{path}: wrong length");
        assert_eq!(hash, sha256_hex(&on_disk), "{path}: wrong hash");
        assert_eq!(hash.len(), 64, "{path}: not a SHA-256");
    }

    // The Release file is deliberately left unsigned — the signature belongs to
    // whoever owns the key, and the instructions say so.
    assert!(!release.contains("BEGIN PGP"), "{release}");
}

#[test]
fn the_compressed_catalogue_is_the_plain_one() {
    // apt fetches Packages.gz and checks it against the hash in Release, then
    // decompresses it and expects the result to be the file the *other* hash in
    // Release describes. Two files built from different content would each pass
    // their own check and fail together in a way nothing reports.
    use std::io::Read;
    let dir = tempfile::tempdir().unwrap();
    let deb = a_package(dir.path(), "0.1.0", "amd64");
    let out = dir.path().join("repo");
    build(&[deb], &out, &RepoOptions::default(), at(1_785_190_200)).unwrap();

    let plain = std::fs::read(out.join("dists/stable/main/binary-amd64/Packages")).unwrap();
    let squashed = std::fs::read(out.join("dists/stable/main/binary-amd64/Packages.gz")).unwrap();
    assert_eq!(&squashed[0..2], &[0x1f, 0x8b], "not a gzip");

    let mut unpacked = Vec::new();
    flate2::read::GzDecoder::new(&squashed[..])
        .read_to_end(&mut unpacked)
        .unwrap();
    assert_eq!(unpacked, plain);
}

#[test]
fn two_architectures_get_a_directory_each_and_are_both_named_in_the_release_file() {
    let dir = tempfile::tempdir().unwrap();
    let amd64 = a_package(dir.path(), "0.1.0", "amd64");
    let arm64 = a_package(dir.path(), "0.1.0", "arm64");
    let out = dir.path().join("repo");
    let built = build(
        &[amd64, arm64],
        &out,
        &RepoOptions::default(),
        at(1_785_190_200),
    )
    .unwrap();

    assert_eq!(
        built.architectures,
        vec!["amd64".to_string(), "arm64".to_string()]
    );
    for architecture in ["amd64", "arm64"] {
        let directory = out.join(format!("dists/stable/main/binary-{architecture}"));
        assert!(directory.join("Packages").is_file(), "{architecture}");
        assert!(directory.join("Packages.gz").is_file(), "{architecture}");

        // Each catalogue lists that architecture's package and nothing else. A
        // Packages file under binary-arm64 that offers an amd64 package gives
        // somebody on a Raspberry Pi a download that will not run, and apt has
        // no way to notice.
        let text = std::fs::read_to_string(directory.join("Packages")).unwrap();
        let found = stanzas(&text);
        assert_eq!(found.len(), 1, "{architecture}:\n{text}");
        assert_eq!(value(&found[0], "Architecture"), Some(architecture));
        assert!(
            value(&found[0], "Filename").unwrap().ends_with(&format!("_{architecture}.deb")),
            "{text}"
        );
        assert!(out
            .join(format!(
                "pool/main/o/onionskin/onionskin_0.1.0_{architecture}.deb"
            ))
            .is_file());
    }

    let release = std::fs::read_to_string(&built.release).unwrap();
    assert!(release.contains("Architectures: amd64 arm64\n"), "{release}");
    // Both catalogues and both compressed twins, so four lines in the block.
    let block = release.split_once("SHA256:\n").unwrap().1;
    assert_eq!(block.lines().filter(|l| l.starts_with(' ')).count(), 4, "{release}");
}

#[test]
fn an_architecture_independent_package_is_offered_to_every_architecture() {
    // apt fetches binary-amd64/Packages and nothing else unless it is told
    // otherwise. A package marked `all` and listed only under binary-all is a
    // package nobody can install, and no error anywhere says why — `apt install`
    // simply reports that it has no such package.
    let dir = tempfile::tempdir().unwrap();
    let amd64 = a_package(dir.path(), "0.1.0", "amd64");
    let anywhere = a_package(dir.path(), "0.2.0", "all");
    let out = dir.path().join("repo");
    let built = build(
        &[amd64, anywhere],
        &out,
        &RepoOptions::default(),
        at(1_785_190_200),
    )
    .unwrap();
    assert_eq!(built.architectures, vec!["all".to_string(), "amd64".to_string()]);

    let text = std::fs::read_to_string(out.join("dists/stable/main/binary-amd64/Packages")).unwrap();
    let found = stanzas(&text);
    assert_eq!(found.len(), 2, "{text}");
    let architectures: Vec<&str> = found
        .iter()
        .map(|fields| value(fields, "Architecture").unwrap())
        .collect();
    assert!(architectures.contains(&"all"), "{text}");
    assert!(architectures.contains(&"amd64"), "{text}");

    // And binary-all holds only the one that really is architecture-free.
    let only = std::fs::read_to_string(out.join("dists/stable/main/binary-all/Packages")).unwrap();
    assert_eq!(stanzas(&only).len(), 1, "{only}");
}

#[test]
fn an_epoch_comes_off_the_filename_and_stays_in_the_version() {
    // A colon is reserved in a URL and illegal in a filename on Windows, so a
    // pool holding one cannot be mirrored onto half the machines that might
    // mirror it. It belongs in the Version field and nowhere else.
    let dir = tempfile::tempdir().unwrap();
    let deb = a_package(dir.path(), "1:0.1.0", "amd64");
    let out = dir.path().join("repo");
    let built = build(&[deb], &out, &RepoOptions::default(), at(1_785_190_200)).unwrap();

    assert_eq!(
        built.packages,
        vec!["pool/main/o/onionskin/onionskin_0.1.0_amd64.deb".to_string()]
    );
    assert!(out.join(&built.packages[0]).is_file());

    let text = std::fs::read_to_string(out.join("dists/stable/main/binary-amd64/Packages")).unwrap();
    let fields = &stanzas(&text)[0];
    assert_eq!(value(fields, "Version"), Some("1:0.1.0"));
    assert!(!value(fields, "Filename").unwrap().contains(':'), "{text}");
}

#[test]
fn a_library_goes_under_four_letters_the_way_debian_does_it() {
    // Not an invention: it is Debian's rule, and following it means a mirror
    // script written for Debian works on this without being told anything.
    assert_eq!(pool_prefix("onionskin"), "o");
    assert_eq!(pool_prefix("Onionskin"), "o");
    assert_eq!(pool_prefix("libpdfium"), "libp");
    assert_eq!(pool_prefix("libc6"), "libc");
    // "lib" on its own is three letters and has no fourth to take.
    assert_eq!(pool_prefix("lib"), "l");
    assert_eq!(without_epoch("1:0.1.0"), "0.1.0");
    assert_eq!(without_epoch("0.1.0"), "0.1.0");
}

#[test]
fn a_suite_that_is_really_a_path_is_refused() {
    // The suite is pasted straight into a directory name. A suite of `../..`
    // would write outside the output directory entirely, and an empty one
    // produces `dists//Release`, which apt cannot ask for. Neither is a
    // plausible typo, and both are silent.
    let dir = tempfile::tempdir().unwrap();
    let deb = a_package(dir.path(), "0.1.0", "amd64");
    let out = dir.path().join("repo");

    for bad in ["", "   ", "..", "../../etc", "stable/main"] {
        let options = RepoOptions {
            suite: bad.to_string(),
            ..RepoOptions::default()
        };
        let error = build(
            std::slice::from_ref(&deb),
            &out,
            &options,
            at(1_785_190_200),
        )
        .map(|_| ())
        .unwrap_err();
        assert!(error.to_string().contains("suite"), "{bad:?} said: {error}");
    }

    let options = RepoOptions {
        component: "main/extra".to_string(),
        ..RepoOptions::default()
    };
    let error = build(&[deb], &out, &options, at(1_785_190_200))
        .map(|_| ())
        .unwrap_err();
    assert!(error.to_string().contains("component"), "{error}");
}

#[test]
fn a_repository_with_nothing_in_it_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let error = build(
        &[],
        &dir.path().join("repo"),
        &RepoOptions::default(),
        at(1_785_190_200),
    )
    .map(|_| ())
    .unwrap_err();
    assert!(error.to_string().contains("at least one"), "{error}");

    // And a file that is not a package names itself in the complaint, because
    // the whole point of the message is to say which of the files it was.
    let not_a_package = dir.path().join("notes.txt");
    std::fs::write(&not_a_package, "I meant to build this first").unwrap();
    let error = build(
        &[not_a_package],
        &dir.path().join("repo"),
        &RepoOptions::default(),
        at(1_785_190_200),
    )
    .map(|_| ())
    .unwrap_err();
    assert!(error.to_string().contains("notes.txt"), "{error}");
    assert!(error.to_string().contains("not a Debian package"), "{error}");
}

#[test]
fn building_the_same_packages_twice_writes_the_same_bytes() {
    // Somebody rebuilding the repository and copying it to a server should not
    // have every file appear changed. More to the point, a Release file that
    // differs between two builds of the same input has to be signed again for
    // no reason, and a signature over the wrong Release is what apt reports as
    // tampering.
    let dir = tempfile::tempdir().unwrap();
    let deb = a_package(dir.path(), "0.1.0", "amd64");
    let one = dir.path().join("one");
    let two = dir.path().join("two");
    let when = at(1_785_190_200);
    build(std::slice::from_ref(&deb), &one, &RepoOptions::default(), when).unwrap();
    build(&[deb], &two, &RepoOptions::default(), when).unwrap();

    for name in [
        "dists/stable/Release",
        "dists/stable/main/binary-amd64/Packages",
        "dists/stable/main/binary-amd64/Packages.gz",
        "pool/main/o/onionskin/onionskin_0.1.0_amd64.deb",
    ] {
        assert_eq!(
            std::fs::read(one.join(name)).unwrap(),
            std::fs::read(two.join(name)).unwrap(),
            "{name} differs between builds"
        );
    }
}

// ---------------------------------------------------------------------------
// Against Debian's own tools
// ---------------------------------------------------------------------------

/// What Debian's own catalogue generator makes of a repository, if either of
/// the two programs that can make one is installed.
fn their_catalogue(root: &Path) -> Option<(&'static str, String)> {
    for (program, arguments) in [
        ("dpkg-scanpackages", vec!["--multiversion", "pool"]),
        ("apt-ftparchive", vec!["packages", "pool"]),
    ] {
        if !have(program) {
            continue;
        }
        let output = std::process::Command::new(program)
            .args(&arguments)
            .current_dir(root)
            .output()
            .ok()?;
        assert!(
            output.status.success(),
            "{program} failed:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        return Some((program, String::from_utf8_lossy(&output.stdout).into_owned()));
    }
    None
}

#[test]
fn the_catalogue_says_what_debians_own_tool_says_about_the_same_package() {
    // A Packages file read back by the code that wrote it will always parse,
    // format bugs and all. This compares it against a program that knows
    // nothing about this code and everything about the format.
    let dir = tempfile::tempdir().unwrap();
    let deb = a_package(dir.path(), "0.1.0", "amd64");
    let out = dir.path().join("repo");
    build(&[deb], &out, &RepoOptions::default(), at(1_785_190_200)).unwrap();

    let Some((program, theirs)) = their_catalogue(&out) else {
        eprintln!("no dpkg-scanpackages or apt-ftparchive on this machine; skipping");
        return;
    };
    let ours = std::fs::read_to_string(out.join("dists/stable/main/binary-amd64/Packages")).unwrap();

    let theirs = stanzas(&theirs);
    let ours = stanzas(&ours);
    assert_eq!(ours.len(), 1);
    assert_eq!(theirs.len(), 1, "{program} found a different number of packages");
    let (theirs, ours) = (&theirs[0], &ours[0]);

    // Every field written here has to say exactly what their tool says. The
    // hash, the size and the filename are the three that matter most: those are
    // what apt fetches and checks, and a disagreement in any of them is a
    // package that downloads and then fails to install.
    for (name, mine) in ours {
        let Some(yours) = value(theirs, name) else {
            panic!("{program} does not write a {name} field at all, but this does: {mine:?}");
        };
        assert_eq!(mine, yours, "{name} disagrees with {program}");
    }
    for wanted in ["Package", "Version", "Architecture", "Filename", "Size", "SHA256"] {
        assert!(value(ours, wanted).is_some(), "no {wanted} field: {ours:?}");
    }

    // Their tool also writes MD5sum and SHA1. Those are deliberately left out
    // — both are broken as collision-resistant hashes, and apt has not needed
    // either since 2016.
    assert!(value(theirs, "MD5sum").is_some(), "{program} changed its output");
    assert_eq!(value(ours, "MD5sum"), None);

    // And the fields are in the order their tool puts them in. apt does not
    // care; a person comparing the two files by eye very much does.
    if program == "dpkg-scanpackages" {
        let shared: Vec<&str> = theirs
            .iter()
            .map(|(name, _)| name.as_str())
            .filter(|name| value(ours, name).is_some())
            .collect();
        let mine: Vec<&str> = ours.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(mine, shared, "the fields are in a different order");
    }
}

// ---------------------------------------------------------------------------
// What to tell people
// ---------------------------------------------------------------------------

#[test]
fn the_instructions_give_the_two_lines_that_actually_work() {
    let text = instructions(&RepoOptions::default(), "https://example.com/apt/");

    // A trailing slash on the URL would produce `https://example.com/apt//...`
    // in the fetch and a sources line apt reads as a different repository from
    // the one without it.
    assert!(!text.contains("apt//"), "{text}");

    // The keyring line and the sources line, exactly as they have to be typed.
    assert!(
        text.contains(
            "curl -fsSL https://example.com/apt/onionskin-archive-keyring.gpg | \
             sudo tee /usr/share/keyrings/onionskin-archive-keyring.gpg > /dev/null"
        ),
        "{text}"
    );
    assert!(
        text.contains(
            "echo \"deb [signed-by=/usr/share/keyrings/onionskin-archive-keyring.gpg] \
             https://example.com/apt stable main\" | \
             sudo tee /etc/apt/sources.list.d/onionskin.list"
        ),
        "{text}"
    );
    // And the thing it was all for.
    assert!(text.contains("sudo apt update"), "{text}");
    assert!(text.contains("sudo apt install onionskin"), "{text}");

    // The signing, which is left to gpg on purpose.
    assert!(text.contains("--clearsign -o dists/stable/InRelease dists/stable/Release"), "{text}");
    assert!(
        text.contains("--detach-sign --armor -o dists/stable/Release.gpg dists/stable/Release"),
        "{text}"
    );
    assert!(text.contains("gpg --quick-generate-key"), "{text}");
    assert!(text.contains("gpg --export KEYID > onionskin-archive-keyring.gpg"), "{text}");

    // apt-key is named only to say not to use it. A key added that way is
    // trusted for every repository on the machine, including the operating
    // system, and it has been gone from Debian since 12.
    assert!(text.contains("apt-key"), "{text}");
    assert!(
        !text.lines().any(|line| line.trim_start().starts_with("apt-key")
            || line.contains("sudo apt-key")),
        "the instructions tell somebody to run apt-key:\n{text}"
    );

    // The suite and component follow the options, so instructions for a
    // repository called something else are not quietly wrong.
    let elsewhere = instructions(
        &RepoOptions {
            suite: "bookworm".to_string(),
            component: "tools".to_string(),
            ..RepoOptions::default()
        },
        "https://packages.example.org",
    );
    assert!(
        elsewhere.contains("https://packages.example.org bookworm tools\""),
        "{elsewhere}"
    );
    assert!(elsewhere.contains("dists/bookworm/InRelease"), "{elsewhere}");
    assert!(elsewhere.contains("dists/bookworm/Release.gpg"), "{elsewhere}");
}

#[test]
fn the_commands_to_type_are_set_apart_from_the_prose() {
    // Rust's line continuation eats the whitespace at the start of the next
    // source line, so an indented line written the obvious way comes out flush
    // left and the command somebody is meant to type reads as another sentence.
    let text = instructions(&RepoOptions::default(), "https://example.com/apt");
    for wanted in ["curl -fsSL", "echo \"deb ", "sudo apt install onionskin", "gpg --export"] {
        let line = text
            .lines()
            .find(|line| line.trim_start().starts_with(wanted))
            .unwrap_or_else(|| panic!("no line starting {wanted:?} at all:\n{text}"));
        assert!(
            line.starts_with("    "),
            "not indented: {line:?}\nin:\n{text}"
        );
    }
}
