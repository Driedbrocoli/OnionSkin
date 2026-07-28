//! Building the file people download.
//!
//! One archive per platform, holding the binary, the rendering library if it is
//! to hand, the licences, and a short note saying what to do next. Unpack it,
//! run `onionskin install`, and that is the whole ceremony.
//!
//! The archive writers are here rather than pulled in because the formats are
//! small and the alternative is a dependency for something a page of code does:
//! a tar is a header and some padding, a zip is a local header per file and a
//! directory at the end, and a Debian package is an `ar` archive containing two
//! tars. The compression is not hand-written — that is a real algorithm, and
//! `flate2` is already in the tree because PNG needs it, so using it here adds
//! nothing to the dependency list. A binary compresses to roughly half, which
//! is half the download.
//!
//! # Why this exists at all
//!
//! Because the licences have to travel with the binary. Onionskin is MIT, and
//! the rendering library beside it is BSD, and both say the notice must be
//! shipped with the thing. An archive built by hand is an archive that will one
//! day be built without them.

use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum PackageError {
    #[error("could not read {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{0}")]
    Invalid(String),
}

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> PackageError + '_ {
    move |source| PackageError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// A file on its way into an archive.
#[derive(Debug, Clone)]
pub struct Entry {
    /// Where it sits inside the archive.
    pub name: String,
    pub bytes: Vec<u8>,
    /// Unix permissions. 0o755 for the program, 0o644 for everything else.
    pub mode: u32,
    /// A directory rather than a file.
    ///
    /// `tar` makes missing parent directories itself, so an archive people
    /// unpack by hand never needs these. `dpkg` does not: it refuses the whole
    /// package the moment a file's parent directory is not in the archive.
    pub directory: bool,
}

impl Entry {
    pub fn file(name: &str, bytes: Vec<u8>) -> Entry {
        Entry {
            name: name.to_string(),
            bytes,
            mode: 0o644,
            directory: false,
        }
    }

    pub fn program(name: &str, bytes: Vec<u8>) -> Entry {
        Entry {
            name: name.to_string(),
            bytes,
            mode: 0o755,
            directory: false,
        }
    }

    pub fn directory(name: &str) -> Entry {
        Entry {
            // A trailing slash is how a reader tells without looking further.
            name: if name.ends_with('/') {
                name.to_string()
            } else {
                format!("{name}/")
            },
            bytes: Vec::new(),
            mode: 0o755,
            directory: true,
        }
    }
}

// ---------------------------------------------------------------------------
// tar
// ---------------------------------------------------------------------------

/// A tar archive, in the old ustar format every tool reads.
pub fn tar(entries: &[Entry]) -> Vec<u8> {
    let mut out = Vec::new();
    for entry in entries {
        out.extend_from_slice(&tar_header(entry));
        out.extend_from_slice(&entry.bytes);
        // Every file is padded out to a whole number of 512-byte blocks.
        let padding = (512 - entry.bytes.len() % 512) % 512;
        out.extend(std::iter::repeat(0u8).take(padding));
    }
    // Two empty blocks end the archive.
    out.extend(std::iter::repeat(0u8).take(1024));
    out
}

fn tar_header(entry: &Entry) -> [u8; 512] {
    let mut header = [0u8; 512];
    let mut put = |at: usize, text: &str| {
        let bytes = text.as_bytes();
        header[at..at + bytes.len()].copy_from_slice(bytes);
    };

    put(0, &entry.name);
    put(100, &format!("{:07o}\0", entry.mode)); // mode
    put(108, "0000000\0"); // owner: root, so it unpacks the same anywhere
    put(116, "0000000\0"); // group
    put(124, &format!("{:011o}\0", entry.bytes.len())); // size
                                                        // A fixed timestamp, so building the same input twice gives the same
                                                        // archive. A package that differs byte for byte between builds cannot be
                                                        // checked against a hash somebody else published.
    put(136, "00000000000\0");
    put(156, if entry.directory { "5" } else { "0" });
    put(257, "ustar\0"); // magic
    put(263, "00"); // version

    // The checksum is computed with its own field full of spaces.
    header[148..156].fill(b' ');
    let sum: u32 = header.iter().map(|b| *b as u32).sum();
    let checksum = format!("{sum:06o}\0 ");
    header[148..148 + checksum.len()].copy_from_slice(checksum.as_bytes());
    header
}

/// Gzip a tar, which is what `.tar.gz` means.
///
/// The timestamp in the gzip header is set to zero rather than left to the
/// clock: two builds of the same input have to give the same bytes, or nobody
/// can check a download against a hash somebody else published.
pub fn gzip(bytes: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut encoder = flate2::GzBuilder::new()
        .mtime(0)
        .write(Vec::new(), flate2::Compression::best());
    // Writing to a Vec cannot fail for want of space, and there is nowhere
    // else for an error to come from.
    encoder.write_all(bytes).expect("writing to memory");
    encoder.finish().expect("writing to memory")
}

/// Deflate, without the zlib or gzip wrapper — the form a zip entry holds.
fn deflate(bytes: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut encoder = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::best());
    encoder.write_all(bytes).expect("writing to memory");
    encoder.finish().expect("writing to memory")
}

// ---------------------------------------------------------------------------
// zip
// ---------------------------------------------------------------------------

/// A zip archive, deflated.
///
/// Windows opens these with a double click and no extra software, which is the
/// only reason the format is here.
pub fn zip(entries: &[Entry]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut directory = Vec::new();

    for entry in entries {
        let offset = out.len() as u32;
        let crc = crc32(&entry.bytes);
        let size = entry.bytes.len() as u32;
        // A directory has no content, and deflating nothing still costs two
        // bytes — so store those, and anything deflate makes no smaller.
        let squashed = if entry.directory {
            None
        } else {
            let tried = deflate(&entry.bytes);
            (tried.len() < entry.bytes.len()).then_some(tried)
        };
        let (method, stored): (u16, &[u8]) = match &squashed {
            Some(bytes) => (8, bytes),
            None => (0, &entry.bytes),
        };
        let packed = stored.len() as u32;

        // Local file header.
        out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        out.extend_from_slice(&20u16.to_le_bytes()); // version needed
        out.extend_from_slice(&0u16.to_le_bytes()); // flags
        out.extend_from_slice(&method.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // time
        out.extend_from_slice(&0x21u16.to_le_bytes()); // date: 1 Jan 1980
        out.extend_from_slice(&crc.to_le_bytes());
        out.extend_from_slice(&packed.to_le_bytes());
        out.extend_from_slice(&size.to_le_bytes());
        out.extend_from_slice(&(entry.name.len() as u16).to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes()); // extra
        out.extend_from_slice(entry.name.as_bytes());
        out.extend_from_slice(stored);

        // Central directory entry, written after the lot.
        directory.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        directory.extend_from_slice(&0x031Eu16.to_le_bytes()); // made by unix
        directory.extend_from_slice(&20u16.to_le_bytes());
        directory.extend_from_slice(&0u16.to_le_bytes());
        directory.extend_from_slice(&method.to_le_bytes());
        directory.extend_from_slice(&0u16.to_le_bytes());
        directory.extend_from_slice(&0x21u16.to_le_bytes());
        directory.extend_from_slice(&crc.to_le_bytes());
        directory.extend_from_slice(&packed.to_le_bytes());
        directory.extend_from_slice(&size.to_le_bytes());
        directory.extend_from_slice(&(entry.name.len() as u16).to_le_bytes());
        directory.extend_from_slice(&0u16.to_le_bytes());
        directory.extend_from_slice(&0u16.to_le_bytes()); // comment
        directory.extend_from_slice(&0u16.to_le_bytes()); // disk
        directory.extend_from_slice(&0u16.to_le_bytes()); // internal attributes
                                                          // External attributes carry the unix mode in the top sixteen bits, so
                                                          // the program is still executable after a round trip through a zip.
                                                          // The low bits are MS-DOS attributes, where 0x10 marks a directory —
                                                          // Windows reads those and not the unix ones.
        let kind = if entry.directory {
            0o040_000
        } else {
            0o100_000
        };
        let dos = if entry.directory { 0x10u32 } else { 0 };
        directory.extend_from_slice(&(((entry.mode | kind) << 16) | dos).to_le_bytes());
        directory.extend_from_slice(&offset.to_le_bytes());
        directory.extend_from_slice(entry.name.as_bytes());
    }

    let directory_at = out.len() as u32;
    let directory_size = directory.len() as u32;
    out.extend_from_slice(&directory);

    // End of central directory.
    out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&(entries.len() as u16).to_le_bytes());
    out.extend_from_slice(&directory_size.to_le_bytes());
    out.extend_from_slice(&directory_at.to_le_bytes());
    out.extend_from_slice(&0u16.to_le_bytes()); // comment length
    out
}

/// The CRC-32 a zip uses, computed without a table.
///
/// Shared with the zip *reader* in [`crate::office::unzip`], which checks
/// entries against it — two implementations of one checksum would be one too
/// many, and the one that drifted would be found by a document that would not
/// open.
pub(crate) fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for byte in bytes {
        crc ^= *byte as u32;
        for _ in 0..8 {
            // The reversed polynomial, which is the one zip specifies.
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

// ---------------------------------------------------------------------------
// Debian package
// ---------------------------------------------------------------------------

/// A `.deb`, which is an `ar` archive of three members in a fixed order.
///
/// Double-clickable on Ubuntu, Debian, Mint and everything descended from them
/// — which is most of the desktop Linux anybody actually runs.
pub fn deb(version: &str, architecture: &str, entries: &[Entry]) -> Result<Vec<u8>, PackageError> {
    if version.is_empty() {
        return Err(PackageError::Invalid("a package needs a version".into()));
    }
    let installed_kb = entries.iter().map(|e| e.bytes.len()).sum::<usize>() / 1024 + 1;

    let control = format!(
        "Package: onionskin\n\
         Version: {version}\n\
         Section: utils\n\
         Priority: optional\n\
         Architecture: {architecture}\n\
         Installed-Size: {installed_kb}\n\
         Maintainer: Onionskin\n\
         Homepage: https://github.com/driedbrocoli/onionskin\n\
         Description: Add words to a page that is already printed\n\
         \x20Onionskin works out which ink is new between two documents and writes\n\
         \x20a delta PDF: the same page size, blank except for the additions. Put\n\
         \x20the sheet back in the tray, print the delta at 100%, and the new words\n\
         \x20land in the gaps.\n\
         \x20.\n\
         \x20It also types onto a scanned form, reads the letters off a scan, and\n\
         \x20talks to a printer directly over IPP and eSCL.\n"
    );

    // dpkg reads the compression from the member's name, so `.gz` here and
    // gzip below have to stay together.
    let control_tar = gzip(&tar(&[
        Entry::directory("."),
        Entry::file("./control", control.into_bytes()),
    ]));
    let data_tar = gzip(&tar(entries));

    let mut out = Vec::new();
    out.extend_from_slice(b"!<arch>\n");
    ar_member(&mut out, "debian-binary", b"2.0\n");
    ar_member(&mut out, "control.tar.gz", &control_tar);
    ar_member(&mut out, "data.tar.gz", &data_tar);
    Ok(out)
}

/// One member of an `ar` archive: a sixty-byte header, then the bytes.
fn ar_member(out: &mut Vec<u8>, name: &str, bytes: &[u8]) {
    let header = format!(
        "{:<16}{:<12}{:<6}{:<6}{:<8}{:<10}`\n",
        name,
        0, // a fixed timestamp, so the build is reproducible
        0, // owner
        0, // group
        "100644",
        bytes.len()
    );
    out.extend_from_slice(header.as_bytes());
    out.extend_from_slice(bytes);
    // Members start on an even byte.
    if bytes.len() % 2 == 1 {
        out.push(b'\n');
    }
}

// ---------------------------------------------------------------------------
// What goes in
// ---------------------------------------------------------------------------

/// The platform an archive is being built for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Linux,
    MacOs,
    Windows,
}

impl Platform {
    pub fn parse(text: &str) -> Option<Platform> {
        match text.trim().to_ascii_lowercase().as_str() {
            "linux" => Some(Platform::Linux),
            "macos" | "mac" | "darwin" => Some(Platform::MacOs),
            "windows" | "win" => Some(Platform::Windows),
            _ => None,
        }
    }

    pub fn name(&self) -> &'static str {
        match self {
            Platform::Linux => "linux",
            Platform::MacOs => "macos",
            Platform::Windows => "windows",
        }
    }

    pub fn binary_name(&self) -> &'static str {
        match self {
            Platform::Windows => "onionskin.exe",
            _ => "onionskin",
        }
    }

    /// The window's name on this platform.
    pub fn desktop_name(&self) -> &'static str {
        match self {
            // Not "Onionskin.exe", which is the friendlier thing to double
            // click and also a bug: Windows filenames are case-insensitive, so
            // it and onionskin.exe are one name. Unpacking the zip wrote one
            // file, whichever came second, and the archive looked fine right up
            // until somebody ran it. macOS gets the inviting name honestly,
            // through a .app bundle, which is a folder and cannot collide.
            Platform::Windows => "onionskin-desktop.exe",
            // On macOS this is what goes inside the .app bundle, where the
            // name has to match what Info.plist says it is.
            Platform::MacOs => "Onionskin",
            _ => "onionskin-desktop",
        }
    }

    pub fn library_name(&self) -> &'static str {
        match self {
            Platform::Linux => "libpdfium.so",
            Platform::MacOs => "libpdfium.dylib",
            Platform::Windows => "pdfium.dll",
        }
    }

    /// The one people expect to double-click on that platform.
    pub fn archive_extension(&self) -> &'static str {
        match self {
            Platform::Windows => "zip",
            _ => "tar.gz",
        }
    }
}

/// The note that goes in beside the binary.
///
/// Indented lines are written `\x20   ` rather than four spaces, because Rust's
/// line continuation eats the whitespace at the start of the next source line —
/// and the indent is the only thing marking a command out from the prose.
pub fn readme(platform: Platform) -> String {
    let run = match platform {
        Platform::Windows => "onionskin.exe install",
        _ => "./onionskin install",
    };
    // Both desktop systems stop an unsigned program the first time it is run,
    // and both do it with wording that reads like the file is broken rather
    // than merely unsigned. Saying so here is cheaper than the alternative,
    // which is paying Apple and a certificate authority for a signature.
    let mac_note = match platform {
        Platform::MacOs => {
            "\nThe first time you run it, macOS may say it cannot check the developer.\n\
             That is what it says about every program not signed with a paid Apple\n\
             certificate. To let it run:\n\
             \n\
             \x20   xattr -d com.apple.quarantine onionskin\n\
             \n\
             or open it once from Finder with a right-click and choose Open.\n"
        }
        // "Windows protected your PC" hides its Run button behind "More info",
        // so somebody who does not know that sees a dialog with one button on
        // it saying Don't run. That is where most people stop.
        Platform::Windows => {
            "\nThe first time you run it, Windows may show a blue box saying\n\
             \"Windows protected your PC\". That appears for every program without a\n\
             paid signing certificate. Click \"More info\", then \"Run anyway\".\n"
        }
        _ => "",
    };
    format!(
        "Onionskin — add words to a page that is already printed\n\
         =======================================================\n\
         \n\
         To install, open a terminal in this folder and run:\n\
         \n\
         \x20   {run}\n\
         \n\
         That copies Onionskin somewhere your computer looks for programs, and\n\
         tells you if anything else is needed. Nothing here asks for an\n\
         administrator password: it installs into your own account.\n\
         {mac_note}\
         \n\
         Then:\n\
         \n\
         \x20   onionskin doctor      what works on this machine\n\
         \x20   onionskin --help      everything it can do\n\
         \x20   onionskin serve       the browser interface, on this machine only\n\
         \n\
         To remove it again:  onionskin uninstall\n\
         \n\
         Onionskin is free software under the MIT licence — see LICENCE. You may\n\
         use it, change it, and pass it on. The source is at\n\
         https://github.com/driedbrocoli/onionskin\n\
         \n\
         The PDF renderer it comes with is Google's pdfium, under a BSD licence.\n\
         See THIRD-PARTY-LICENCES for that and for everything else built in.\n"
    )
}

/// The notice that has to travel with the binary.
///
/// Both MIT and BSD say the licence must be shipped with the software, and
/// building the archive by hand is how that gets forgotten. It is generated
/// here so it cannot be.
pub fn third_party_licences() -> String {
    "Third-party licences\n\
     ====================\n\
     \n\
     Onionskin itself is MIT — see LICENCE.\n\
     \n\
     Compiled into the binary\n\
     ------------------------\n\
     \n\
     Every crate Onionskin is built from is permissively licensed. Across the\n\
     whole dependency tree the licences are:\n\
     \n\
     \x20   0BSD, Apache-2.0, Apache-2.0 WITH LLVM-exception, BSD-2-Clause,\n\
     \x20   BSD-3-Clause, ISC, MIT, Unicode-3.0, Unlicense, Zlib\n\
     \n\
     One crate — self_cell, which the window toolkit uses — is offered as\n\
     'Apache-2.0 OR GPL-2.0-only'. That is a choice, not a condition: Onionskin\n\
     takes the Apache-2.0 side, and no GPL obligation reaches this program or\n\
     anything built on it.\n\
     \n\
     Nothing else in the tree is copyleft. That is deliberate, and it is what\n\
     lets you pass this binary on, changed or not, under whatever terms you\n\
     like.\n\
     \n\
     The full text of each licence is in the crate it belongs to. The list can\n\
     be checked against the source at any time with:\n\
     \n\
     \x20   cargo tree --format '{p} {l}'\n\
     \n\
     The typefaces in the window\n\
     ---------------------------\n\
     \n\
     The desktop window draws its own text, so it carries its own fonts:\n\
     Ubuntu Light, Hack, Noto Emoji and Emoji Icon Font, by way of the\n\
     epaint_default_fonts crate.\n\
     \n\
     \x20   Ubuntu Light  — Ubuntu Font Licence 1.0  (Ubuntu-font-1.0)\n\
     \x20   the others    — SIL Open Font Licence 1.1  (OFL-1.1)\n\
     \n\
     Both allow the fonts to be bundled and passed on. Both ask that the font\n\
     files keep their own licence and that a modified font be renamed, which\n\
     is nothing to do with Onionskin's own terms and places no condition on\n\
     your use of the program.\n\
     \n\
     Bundled alongside the binary\n\
     ----------------------------\n\
     \n\
     pdfium — the PDF renderer, from the Chromium project.\n\
     \x20 BSD-3-Clause and Apache-2.0.\n\
     \x20 https://pdfium.googlesource.com/pdfium/\n\
     \n\
     Not bundled\n\
     -----------\n\
     \n\
     LibreOffice converts documents Onionskin cannot read by itself, and it\n\
     is used if it is installed. It is deliberately not included here: it is\n\
     under the Mozilla Public License 2.0, which would oblige anyone\n\
     redistributing this archive to offer LibreOffice's source as well.\n\
     Onionskin detects it instead and points at\n\
     https://www.libreoffice.org/download/ when a document needs it.\n\
     PDFs, images, plain text, .docx and .odt need it not at all.\n"
        .to_string()
}

/// What kind of program a file is, judged by the bytes it starts with.
///
/// Renaming a Linux binary to `onionskin.exe` produces something that looks
/// right in the archive and cannot run at all. Building on one machine for
/// three platforms is the ordinary case, so this is not a hypothetical
/// mistake — it is the mistake.
pub fn built_for(bytes: &[u8]) -> Option<Platform> {
    match bytes {
        [0x7f, b'E', b'L', b'F', ..] => Some(Platform::Linux),
        [b'M', b'Z', ..] => Some(Platform::Windows),
        // Mach-O, thin in either byte order, and the fat wrapper.
        [0xfe, 0xed, 0xfa, 0xce, ..]
        | [0xce, 0xfa, 0xed, 0xfe, ..]
        | [0xfe, 0xed, 0xfa, 0xcf, ..]
        | [0xcf, 0xfa, 0xed, 0xfe, ..]
        | [0xca, 0xfe, 0xba, 0xbe, ..]
        | [0xbe, 0xba, 0xfe, 0xca, ..] => Some(Platform::MacOs),
        _ => None,
    }
}

fn describe(platform: Platform) -> &'static str {
    match platform {
        Platform::Linux => "a Linux program",
        Platform::MacOs => "a macOS program",
        Platform::Windows => "a Windows program",
    }
}

/// Everything that goes into one platform's archive.
pub fn contents(
    platform: Platform,
    binary: &Path,
    library: Option<&Path>,
    licence: &Path,
) -> Result<Vec<Entry>, PackageError> {
    contents_with_window(platform, binary, None, library, licence)
}

/// Everything that goes into one platform's archive, window included.
///
/// The window is optional because the command line is the whole program and
/// works without it — but an archive built with one is what somebody means by
/// "an app", and building the archive by hand is how it gets left out.
pub fn contents_with_window(
    platform: Platform,
    binary: &Path,
    desktop: Option<&Path>,
    library: Option<&Path>,
    licence: &Path,
) -> Result<Vec<Entry>, PackageError> {
    let program = std::fs::read(binary).map_err(io(binary))?;

    // Refused rather than warned about: an archive with the wrong binary in it
    // looks completely normal until somebody downloads it and it will not run.
    match built_for(&program) {
        Some(actual) if actual != platform => {
            return Err(PackageError::Invalid(format!(
                "{} is {}, but this is a {} package.\n\
                 Renaming it would give people a download that cannot run.\n\
                 Build it for {} first — with rustup and cross-compilation, or on \
                 a {} machine — then pass it with --binary.",
                binary.display(),
                describe(actual),
                platform.name(),
                platform.name(),
                platform.name(),
            )));
        }
        None => {
            return Err(PackageError::Invalid(format!(
                "{} does not look like a program at all.\n\
                 It starts with none of the marks a Linux, macOS or Windows \
                 binary starts with.",
                binary.display()
            )));
        }
        Some(_) => {}
    }

    let mut entries = vec![Entry::program(platform.binary_name(), program)];

    if let Some(desktop) = desktop {
        let window = std::fs::read(desktop).map_err(io(desktop))?;
        match built_for(&window) {
            Some(actual) if actual != platform => {
                return Err(PackageError::Invalid(format!(
                    "{} is {}, but this is a {} package.",
                    desktop.display(),
                    describe(actual),
                    platform.name()
                )));
            }
            None => {
                return Err(PackageError::Invalid(format!(
                    "{} does not look like a program at all.",
                    desktop.display()
                )));
            }
            Some(_) => {}
        }
        entries.push(Entry::program(platform.desktop_name(), window));
    }

    if let Some(library) = library {
        entries.push(Entry::file(
            platform.library_name(),
            std::fs::read(library).map_err(io(library))?,
        ));
    }

    entries.push(Entry::file(
        "LICENCE",
        std::fs::read(licence).map_err(io(licence))?,
    ));
    entries.push(Entry::file(
        "THIRD-PARTY-LICENCES",
        third_party_licences().into_bytes(),
    ));
    entries.push(Entry::file(
        if platform == Platform::Windows {
            "README.txt"
        } else {
            "README"
        },
        readme(platform).into_bytes(),
    ));
    Ok(entries)
}

/// The menu entry a package manager installs for everybody on the machine.
fn desktop_entry() -> String {
    "[Desktop Entry]\n\
     Type=Application\n\
     Name=Onionskin\n\
     GenericName=Overprinting\n\
     Comment=Add words to a page that is already printed\n\
     Exec=onionskin-desktop\n\
     Icon=onionskin\n\
     Terminal=false\n\
     Categories=Office;Publishing;Scanning;\n\
     Keywords=print;pdf;scan;delta;overprint;\n\
     StartupWMClass=onionskin\n"
        .to_string()
}

/// The archive laid out as a macOS application bundle.
///
/// A bare executable on macOS is a terminal command: double-clicking it opens
/// Terminal and runs it there, with no icon, no name in the Dock and no window
/// worth looking at. An application is a *folder* of a particular shape with a
/// `.app` on the end, and the Finder treats that folder as one thing. So the
/// window goes inside one, and the command line program goes beside it, where
/// somebody who wants it can find it.
pub fn mac_bundle(entries: &[Entry], version: &str) -> Vec<Entry> {
    const APP: &str = "Onionskin.app/Contents";
    let mut out = vec![
        Entry::directory("Onionskin.app"),
        Entry::directory(APP),
        Entry::directory(&format!("{APP}/MacOS")),
        Entry::directory(&format!("{APP}/Resources")),
        Entry::file(
            &format!("{APP}/Info.plist"),
            info_plist(version).into_bytes(),
        ),
        // The Finder reads this to know the folder is an application even
        // before it looks at the plist.
        Entry::file(&format!("{APP}/PkgInfo"), b"APPL????".to_vec()),
    ];

    for entry in entries {
        let name = match entry.name.as_str() {
            // The window, and the renderer it loads, live inside the bundle.
            "Onionskin" => format!("{APP}/MacOS/Onionskin"),
            "libpdfium.dylib" => format!("{APP}/MacOS/libpdfium.dylib"),
            // The command line program goes beside the app rather than inside
            // it, because a path buried in a bundle is not one anybody types.
            "onionskin" => "onionskin".to_string(),
            other => other.to_string(),
        };
        out.push(Entry {
            name,
            ..entry.clone()
        });
    }
    out
}

/// What macOS reads to learn the application's name, version and icon.
fn info_plist(version: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
         \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n\
         <dict>\n\
         \x20 <key>CFBundleName</key><string>Onionskin</string>\n\
         \x20 <key>CFBundleDisplayName</key><string>Onionskin</string>\n\
         \x20 <key>CFBundleIdentifier</key><string>org.onionskin.Onionskin</string>\n\
         \x20 <key>CFBundleVersion</key><string>{version}</string>\n\
         \x20 <key>CFBundleShortVersionString</key><string>{version}</string>\n\
         \x20 <key>CFBundleExecutable</key><string>Onionskin</string>\n\
         \x20 <key>CFBundlePackageType</key><string>APPL</string>\n\
         \x20 <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>\n\
         \x20 <key>LSMinimumSystemVersion</key><string>10.15</string>\n\
         \x20 <!-- The window draws its own interface at whatever resolution the\n\
         \x20      screen has. Without this, macOS runs it at half resolution and\n\
         \x20      scales the result up, and every letter is soft. -->\n\
         \x20 <key>NSHighResolutionCapable</key><true/>\n\
         \x20 <key>NSSupportsAutomaticGraphicsSwitching</key><true/>\n\
         \x20 <key>NSHumanReadableCopyright</key>\n\
         \x20 <string>MIT licensed. See LICENCE.</string>\n\
         </dict>\n\
         </plist>\n"
    )
}

/// The same contents, laid out where a Debian package puts them.
///
/// Every parent directory is listed too. `tar` makes missing directories on
/// the way in, so an archive people unpack by hand never needs them — but
/// `dpkg` refuses the whole package the moment a file's parent is not in the
/// archive, and the message it gives ("No such file or directory") reads like
/// something is wrong with the machine rather than with the package.
pub fn deb_contents(entries: &[Entry]) -> Vec<Entry> {
    let mut placed: Vec<Entry> = entries
        .iter()
        .map(|entry| {
            let name = if entry.mode & 0o111 != 0 {
                format!("./usr/bin/{}", entry.name)
            } else if entry.name.starts_with("libpdfium") {
                format!("./usr/lib/onionskin/{}", entry.name)
            } else {
                format!("./usr/share/doc/onionskin/{}", entry.name)
            };
            Entry {
                name,
                ..entry.clone()
            }
        })
        .collect();

    // A menu entry, if there is a window to launch. Without it the window is
    // installed and invisible: somebody who installs a package expects to find
    // the application in the applications menu, not to learn a command.
    if entries.iter().any(|e| e.name == "onionskin-desktop") {
        placed.push(Entry::file(
            "./usr/share/applications/onionskin.desktop",
            desktop_entry().into_bytes(),
        ));
    }

    // Each directory once, and in order: a reader unpacking in sequence needs
    // ./usr before ./usr/lib.
    let mut directories: Vec<String> = Vec::new();
    for entry in &placed {
        let mut so_far = String::from(".");
        let after_dot = entry.name.trim_start_matches("./");
        let parts: Vec<&str> = after_dot.split('/').collect();
        for part in &parts[..parts.len().saturating_sub(1)] {
            so_far.push('/');
            so_far.push_str(part);
            if !directories.contains(&so_far) {
                directories.push(so_far.clone());
            }
        }
    }

    let mut out: Vec<Entry> = directories.iter().map(|d| Entry::directory(d)).collect();
    out.extend(placed);
    out
}

/// Build every archive for one platform, and say what was written.
pub fn build(
    platform: Platform,
    binary: &Path,
    library: Option<&Path>,
    licence: &Path,
    version: &str,
    out_dir: &Path,
) -> Result<Vec<PathBuf>, PackageError> {
    build_with_window(platform, binary, None, library, licence, version, out_dir)
}

/// The same, with the desktop window in the archive as well.
#[allow(clippy::too_many_arguments)]
pub fn build_with_window(
    platform: Platform,
    binary: &Path,
    desktop: Option<&Path>,
    library: Option<&Path>,
    licence: &Path,
    version: &str,
    out_dir: &Path,
) -> Result<Vec<PathBuf>, PackageError> {
    std::fs::create_dir_all(out_dir).map_err(io(out_dir))?;
    let entries = contents_with_window(platform, binary, desktop, library, licence)?;
    let mut written = Vec::new();

    // On macOS the window has to be inside an application bundle, or a double
    // click opens Terminal instead of a window.
    let laid_out = match platform {
        Platform::MacOs if desktop.is_some() => mac_bundle(&entries, version),
        _ => entries.clone(),
    };

    let stem = format!("onionskin-{version}-{}", platform.name());
    let archive = out_dir.join(format!("{stem}.{}", platform.archive_extension()));
    let bytes = match platform {
        Platform::Windows => zip(&laid_out),
        _ => gzip(&tar(&laid_out)),
    };
    std::fs::write(&archive, &bytes).map_err(io(&archive))?;
    written.push(archive);

    if platform == Platform::Linux {
        let package = out_dir.join(format!("onionskin_{version}_amd64.deb"));
        let bytes = deb(version, "amd64", &deb_contents(&entries))?;
        std::fs::write(&package, &bytes).map_err(io(&package))?;
        written.push(package);
    }
    Ok(written)
}

#[cfg(test)]
mod tests;
