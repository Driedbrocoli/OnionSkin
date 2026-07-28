//! The rule the whole program is built on: it is Rust, and it is this list.
//!
//! Onionskin has one promise that is easy to make and easy to lose — that it
//! is one Rust program with a short, deliberate set of dependencies, and that
//! nothing it does at run time is farmed out to a script, an interpreter or a
//! toolchain the person running it has to install.
//!
//! Losing that promise is never a decision. It happens a crate at a time, each
//! one reasonable on its own, until the program needs Python to read a page or
//! a hundred and forty crates to draw a rectangle. This file makes the loss
//! into a decision: adding a dependency, or shelling out to a computation,
//! fails the build until somebody edits the list below and says why.
//!
//! Nothing here forbids anything. It only insists that the answer is written
//! down.

use std::collections::BTreeSet;

/// What Onionskin depends on, and why each one has to be there.
///
/// Every entry is something that would be months of work to write and easy to
/// get subtly wrong — a PDF parser, a font programme reader, an image decoder.
/// Nothing here is a convenience.
const ALLOWED: &[(&str, &str)] = &[
    (
        "lopdf",
        "PDF structure: page boxes, rotation, content streams",
    ),
    ("pdfium-render", "drawing a PDF page to pixels"),
    ("image", "decoding a scan: PNG, JPEG, TIFF, BMP"),
    ("clap", "the command line"),
    ("serde", "reading and writing Onionskin's own documents"),
    ("serde_json", "the same, as JSON"),
    ("thiserror", "error types that say what went wrong"),
    ("anyhow", "errors at the edges of the program"),
    ("tempfile", "workspaces that clean themselves up"),
    ("uuid", "naming a delta nothing else will name"),
    (
        "ttf-parser",
        "reading a font programme to measure and draw letters",
    ),
    (
        "flate2",
        "compressing what is written, through miniz_oxide, which is Rust",
    ),
    ("eframe", "the window, drawn by egui onto OpenGL"),
    ("egui", "the widgets in it"),
    ("egui_extras", "a few more of them"),
];

/// Programs the shipped code may run, and why each is unavoidable.
///
/// Two kinds only, and neither is a computation Onionskin could do itself:
/// handing a file to the desktop to open, and the optional office suite that
/// converts formats nothing else can read. Everything the program *works out*
/// it works out in Rust.
const MAY_RUN: &[(&str, &str)] = &[
    ("xdg-open", "asking the Linux desktop to open a file"),
    ("open", "the same, on macOS"),
    ("cmd", "the same, on Windows"),
    (
        "soffice",
        "LibreOffice, the optional fallback for formats with no Rust reader",
    ),
    (
        "scanimage",
        "SANE, the only way to drive an attached scanner",
    ),
];

/// The dependency names in `Cargo.toml`, from the sections that ship.
///
/// `[dev-dependencies]` is deliberately excluded: a test may use whatever it
/// likes, because nothing it uses reaches the person running the program.
fn shipped_dependencies() -> BTreeSet<String> {
    let manifest = include_str!("../Cargo.toml");
    let mut found = BTreeSet::new();
    let mut inside = false;
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            // Section headings: `[dependencies]` and the per-platform ones,
            // but never `[dev-dependencies]` or `[build-dependencies]`.
            inside = line == "[dependencies]"
                || (line.starts_with("[target.") && line.ends_with(".dependencies]"));
            continue;
        }
        if !inside || line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((name, _)) = line.split_once('=') {
            let name = name.trim().trim_matches('"');
            if !name.is_empty() {
                found.insert(name.to_string());
            }
        }
    }
    found
}

/// Adding a dependency is a decision, so it has to be made twice: once in
/// `Cargo.toml` and once here, with a reason.
#[test]
fn every_dependency_is_one_that_was_argued_for() {
    let allowed: BTreeSet<String> = ALLOWED.iter().map(|(name, _)| name.to_string()).collect();
    let shipped = shipped_dependencies();
    assert!(
        !shipped.is_empty(),
        "no dependencies were found at all, so this test is reading the manifest wrongly"
    );

    let unexplained: Vec<&String> = shipped.difference(&allowed).collect();
    assert!(
        unexplained.is_empty(),
        "a dependency was added that nothing here argues for: {unexplained:?}\n\
         \n\
         Onionskin is one Rust program with a deliberately short list. If this \
         one has to be there, add it to ALLOWED in tests/all_in_rust.rs with a \
         sentence saying what it does that Rust in this repository cannot.\n\
         \n\
         If it is a convenience, take it out of Cargo.toml instead."
    );
}

/// And taking one out is a decision too — a name left behind here is a reason
/// nobody can check against a dependency that is no longer there.
#[test]
fn nothing_is_argued_for_that_is_no_longer_used() {
    let allowed: BTreeSet<String> = ALLOWED.iter().map(|(name, _)| name.to_string()).collect();
    let shipped = shipped_dependencies();
    let stale: Vec<&String> = allowed.difference(&shipped).collect();
    assert!(
        stale.is_empty(),
        "these are argued for in tests/all_in_rust.rs and are not in Cargo.toml: \
         {stale:?}. Take them off the list."
    );
}

/// Every reason is a real sentence. An entry added in a hurry with an empty
/// reason is the first step back to a list nobody can defend.
#[test]
fn every_reason_actually_says_something() {
    for (name, reason) in ALLOWED.iter().chain(MAY_RUN) {
        assert!(
            reason.len() > 12,
            "'{name}' is on a list with no reason worth reading: {reason:?}"
        );
    }
}

/// Nothing the program ships runs an interpreter or a toolchain.
///
/// The library and the two binaries only — `#[cfg(test)]` code is exempt, and
/// deliberately so: several tests check Onionskin's own Rust against an
/// outside tool, which is a stronger check than testing it against itself.
/// `unzip` confirming the zip writer, and `sha256sum` confirming the digest,
/// are worth more than nothing at all, and both skip where the tool is absent.
#[test]
fn the_shipped_code_runs_no_interpreter_and_no_toolchain() {
    let allowed: BTreeSet<&str> = MAY_RUN.iter().map(|(name, _)| *name).collect();
    let mut wrong: Vec<String> = Vec::new();

    for file in rust_files("src") {
        let text = std::fs::read_to_string(&file).expect("a source file");
        // Test modules are exempt. Anything from the first `#[cfg(test)]` to
        // the end of the file is test code in this repository's layout: tests
        // live either in a `tests.rs` beside the module or at the bottom.
        let shipped = match text.find("#[cfg(test)]") {
            Some(at) => &text[..at],
            None => &text[..],
        };
        if file.to_string_lossy().contains("/tests.rs") {
            continue;
        }
        for (index, line) in shipped.lines().enumerate() {
            let Some(rest) = line.split("Command::new(\"").nth(1) else {
                continue;
            };
            let Some((program, _)) = rest.split_once('"') else {
                continue;
            };
            if !allowed.contains(program) {
                wrong.push(format!(
                    "{}:{}  runs '{program}'",
                    file.display(),
                    index + 1
                ));
            }
        }
    }

    assert!(
        wrong.is_empty(),
        "the shipped program runs something that is not on the list:\n  {}\n\n\
         Onionskin works things out in Rust. If this genuinely cannot be done \
         here — opening a file in somebody's desktop, or an optional converter \
         for a format with no Rust reader — add it to MAY_RUN in \
         tests/all_in_rust.rs with the reason.",
        wrong.join("\n  ")
    );
}

/// Every file the repository ships is Rust, or one of the few things a
/// repository has to have that is not code.
///
/// What the repository *ships* is what git tracks, which is not the same as
/// what happens to be in the folder. A machine that once ran the Python
/// implementation still has a `.pytest_cache` in it, and somebody's editor
/// leaves swap files about; neither is in the repository and neither is
/// anybody else's problem.
#[test]
fn the_repository_is_rust_and_the_paperwork_a_repository_needs() {
    // Not code, and no amount of Rust would make them so.
    const PAPERWORK: &[&str] = &[
        "toml", "lock", "md", "yml", "yaml", "json", "txt", "desktop", "png", "svg", "sh",
    ];
    let Some(tracked) = tracked_files() else {
        eprintln!("not a git checkout, so there is no list of what is shipped; skipping");
        return;
    };
    assert!(
        tracked.len() > 20,
        "only {} tracked files were found, so this test is asking git wrongly",
        tracked.len()
    );

    let odd: Vec<&String> = tracked
        .iter()
        .filter(|path| {
            let extension = std::path::Path::new(path)
                .extension()
                .and_then(|e| e.to_str());
            match extension {
                // LICENSE, .gitignore and the like.
                None => false,
                Some(extension) => extension != "rs" && !PAPERWORK.contains(&extension),
            }
        })
        .collect();
    assert!(
        odd.is_empty(),
        "these are neither Rust nor the paperwork a repository needs: {odd:?}"
    );
}

/// What the repository tracks, or nothing if this is not a git checkout.
fn tracked_files() -> Option<Vec<String>> {
    let listed = std::process::Command::new("git")
        .args(["ls-files"])
        .output()
        .ok()?;
    if !listed.status.success() {
        return None;
    }
    Some(
        String::from_utf8_lossy(&listed.stdout)
            .lines()
            .map(|line| line.to_string())
            .collect(),
    )
}

fn rust_files(root: &str) -> Vec<std::path::PathBuf> {
    walk(root)
        .into_iter()
        .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("rs"))
        .collect()
}

/// Every file under a folder, without pulling in a crate to do it.
fn walk(root: &str) -> Vec<std::path::PathBuf> {
    let mut found = Vec::new();
    let mut waiting = vec![std::path::PathBuf::from(root)];
    while let Some(at) = waiting.pop() {
        let Ok(entries) = std::fs::read_dir(&at) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name == "target" || name == ".git" {
                continue;
            }
            if path.is_dir() {
                waiting.push(path);
            } else {
                found.push(path);
            }
        }
    }
    found
}
