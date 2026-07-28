//! Somewhere `apt` will install Onionskin from.
//!
//! A `.deb` on a web page is a file somebody downloads once, installs, and then
//! never hears about again. There is no update, no `apt upgrade`, and no way for
//! a person to find out that the version they are running was replaced eighteen
//! months ago. An apt *repository* is the same `.deb` with three small text
//! files beside it, and those three files are the whole difference between a
//! download and a package their machine looks after for them.
//!
//! So this takes one or more `.deb` files and writes out the directory apt
//! expects, which any static web server can serve as it stands — there is
//! nothing to run on the server, no database, and no software to install there:
//!
//! ```text
//! pool/main/o/onionskin/onionskin_0.1.0_amd64.deb
//! dists/stable/main/binary-amd64/Packages
//! dists/stable/main/binary-amd64/Packages.gz
//! dists/stable/Release
//! ```
//!
//! `Packages` is the catalogue: one stanza per package, holding the control
//! fields read back out of the `.deb` itself, plus where the file is, how big it
//! is, and its SHA-256. `Release` is the index of the catalogue, holding the
//! hash and length of every `Packages` file. Between them, apt can check that
//! what arrived over the network is what the repository meant to send — and once
//! the `Release` file is signed, that it came from the person who owns the key.
//!
//! # Why the fields are read back out of the package
//!
//! It would be easier to be told the package name, version and architecture. It
//! would also be wrong the first time somebody rebuilt the `.deb` and forgot to
//! change one of them. A `Packages` file that disagrees with the package it
//! describes is not an error anybody sees: apt downloads the package the
//! catalogue promised, finds a different one, and reports a hash mismatch or an
//! unmet dependency that has nothing to do with the real cause. So the `.deb` is
//! opened and its own control file is read, and the catalogue cannot drift from
//! the thing it is cataloguing.
//!
//! # Why SHA-256 is written by hand here
//!
//! The rest of this program already writes its own CRC-32, its own tar, its own
//! zip and its own `ar`, for the same reason: these are small, fixed, published
//! formats, and a dependency for each one is a dependency that has to be
//! trusted, audited and updated forever. SHA-256 is about eighty lines and has
//! not changed since 2001. It is here, in full, rather than pulled in.
//!
//! This is emphatically *not* a claim that key handling should be hand-rolled
//! too. Hashing is arithmetic with no secret in it; signing is custody of a
//! private key, and getting that wrong is not a bug that shows up in a test. So
//! the signature is left to `gpg`, which people already have, already trust, and
//! already know how to keep a key safe with. See [`instructions`] for the exact
//! commands.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, thiserror::Error)]
pub enum AptError {
    #[error("could not read {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{path} is not a Debian package: {why}")]
    NotAPackage { path: PathBuf, why: String },
    #[error("{0}")]
    Invalid(String),
}

fn io(path: &Path) -> impl FnOnce(std::io::Error) -> AptError + '_ {
    move |source| AptError::Io {
        path: path.to_path_buf(),
        source,
    }
}

// ---------------------------------------------------------------------------
// What the repository is called
// ---------------------------------------------------------------------------

/// The handful of names that go in the `Release` file.
///
/// None of these change what apt installs. They are what somebody sees when
/// they run `apt policy onionskin` and want to know where this package came
/// from, and what they have to type on the `deb` line to ask for it. The
/// defaults are the ones that make the shortest sources line: suite `stable`
/// and component `main`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepoOptions {
    /// The name on the `deb` line, after the URL — `stable` in
    /// `deb https://example.com/apt stable main`.
    pub suite: String,
    /// The last word on the `deb` line. Debian splits its archive into `main`,
    /// `contrib` and `non-free`; a repository with one program in it has no
    /// reason to be split at all, so this is `main` and stays `main`.
    pub component: String,
    /// Who published this. Shown by `apt policy`.
    pub origin: String,
    /// A shorter name for the same thing, also shown by `apt policy`. Origin
    /// and Label are conventionally both set, and conventionally the same for
    /// a repository belonging to one program.
    pub label: String,
    /// One line saying what is in here, for a person reading the `Release`
    /// file. Newlines are folded out on the way in: `Release` is a control
    /// file, and a second line would be read as the start of a field that does
    /// not exist.
    pub description: String,
}

impl Default for RepoOptions {
    fn default() -> RepoOptions {
        RepoOptions {
            suite: "stable".to_string(),
            component: "main".to_string(),
            origin: "Onionskin".to_string(),
            label: "Onionskin".to_string(),
            description: "Onionskin — add words to a page that is already printed".to_string(),
        }
    }
}

/// What was written, and where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Built {
    /// The directory to serve. Everything below is relative to this.
    pub root: PathBuf,
    /// The file that has to be signed. Everything apt checks hangs off this
    /// one file, which is why it is called out on its own rather than left for
    /// the caller to work out from the suite name.
    pub release: PathBuf,
    /// Every package that went in, as the repository-relative path it went to
    /// — the same string that appears in its `Filename:` field.
    ///
    /// A `String` and not a `PathBuf` because these are apt's paths, not this
    /// machine's: they are separated by forward slashes on every platform, and
    /// a `PathBuf` would quietly start using backslashes on Windows and write a
    /// catalogue no apt can follow.
    pub packages: Vec<String>,
    /// The architectures found among the packages, sorted. The same list that
    /// goes in `Architectures:`.
    pub architectures: Vec<String>,
}

// ---------------------------------------------------------------------------
// Building the repository
// ---------------------------------------------------------------------------

/// Write out a complete apt repository for these packages.
///
/// `now` is passed in rather than read from the clock inside, because the
/// `Release` file carries a date and a function that reads the clock cannot be
/// tested for what it writes. Callers pass `SystemTime::now()`.
///
/// Nothing already in `out` is deleted. Rebuilding into a directory that is
/// being served is therefore safe in the sense that nothing disappears from
/// under a download in progress — but it also means an old `.deb` left in the
/// pool stays on disk. It will not be offered to anybody: apt only ever sees
/// what the `Packages` file lists, and that is rewritten from scratch each
/// time.
pub fn build(
    debs: &[PathBuf],
    out: &Path,
    options: &RepoOptions,
    now: SystemTime,
) -> Result<Built, AptError> {
    if debs.is_empty() {
        return Err(AptError::Invalid(
            "a repository with no packages in it would install nothing. \
             Pass at least one .deb file."
                .to_string(),
        ));
    }
    let suite = checked_name("suite", &options.suite)?;
    let component = checked_name("component", &options.component)?;

    // Read every package first, and place none of them, so that a bad file in
    // the list leaves the directory being served exactly as it was. Half a
    // repository is worse than none: apt would find a Release file promising a
    // Packages file that is not there yet and report the mirror as broken.
    let mut found: Vec<Found> = Vec::new();
    for path in debs {
        let bytes = std::fs::read(path).map_err(io(path))?;
        let control = control(&bytes).map_err(|why| AptError::NotAPackage {
            path: path.to_path_buf(),
            why,
        })?;
        let need = |field: &str| -> Result<String, AptError> {
            control
                .field(field)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
                .ok_or_else(|| AptError::NotAPackage {
                    path: path.to_path_buf(),
                    why: format!("its control file has no {field} field"),
                })
        };
        let package = need("Package")?;
        let version = need("Version")?;
        let architecture = need("Architecture")?;

        // Debian's own layout, and the one every mirror script assumes:
        // pool/<component>/<letter>/<package>/<file>. The file is named from
        // the control fields rather than from whatever the file on disk was
        // called, so a package saved as `latest.deb` still lands under the name
        // apt will ask for.
        let filename = format!(
            "pool/{component}/{}/{package}/{package}_{}_{architecture}.deb",
            pool_prefix(&package),
            without_epoch(&version),
        );
        found.push(Found {
            filename,
            size: bytes.len() as u64,
            sha256: sha256_hex(&bytes),
            architecture,
            package,
            version,
            control,
            bytes,
        });
    }

    // Two packages that would land on the same path is a mistake worth
    // stopping for. Written out in order, the second would silently replace the
    // first, and the catalogue would describe one file and serve the other —
    // which apt reports as a hash mismatch on a package that looks perfectly
    // fine on the server.
    for (at, one) in found.iter().enumerate() {
        if let Some(other) = found[..at]
            .iter()
            .find(|other| other.filename == one.filename)
        {
            if other.sha256 != one.sha256 {
                return Err(AptError::Invalid(format!(
                    "two different packages both want to be {}: {} {} and {} {}. \
                     One of them would overwrite the other.",
                    one.filename, other.package, other.version, one.package, one.version
                )));
            }
        }
    }

    // The pool.
    for one in &found {
        let path = out.join(&one.filename);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(io(parent))?;
        }
        std::fs::write(&path, &one.bytes).map_err(io(&path))?;
    }

    // Every architecture that turned up, in a fixed order so two runs over the
    // same packages produce the same Release file.
    let mut architectures: Vec<String> = found.iter().map(|one| one.architecture.clone()).collect();
    architectures.sort();
    architectures.dedup();

    // What goes in the Release file's SHA256 block: hash, length and path, for
    // each catalogue and its compressed twin.
    let mut indexed: Vec<(String, u64, String)> = Vec::new();

    for architecture in &architectures {
        // A package marked `all` is architecture-independent, and it has to
        // appear in *every* architecture's catalogue as well as its own. apt
        // fetches binary-amd64/Packages and nothing else unless it is told
        // otherwise, so a package listed only under binary-all is a package
        // nobody can install and no error anywhere says why.
        let mut mine: Vec<&Found> = found
            .iter()
            .filter(|one| {
                one.architecture == *architecture
                    || (architecture != "all" && one.architecture == "all")
            })
            .collect();
        mine.sort_by(|a, b| {
            (&a.package, &a.version, &a.filename).cmp(&(&b.package, &b.version, &b.filename))
        });

        let catalogue: String = mine.iter().map(|one| stanza(one)).collect();
        let squashed = crate::package::gzip(catalogue.as_bytes());

        let within = format!("{component}/binary-{architecture}");
        let directory = out.join("dists").join(suite).join(&within);
        std::fs::create_dir_all(&directory).map_err(io(&directory))?;

        let plain = directory.join("Packages");
        std::fs::write(&plain, catalogue.as_bytes()).map_err(io(&plain))?;
        let compressed = directory.join("Packages.gz");
        std::fs::write(&compressed, &squashed).map_err(io(&compressed))?;

        // The paths in Release are relative to dists/<suite>/, which is where
        // the Release file itself sits. Anything else and apt looks for the
        // catalogue in the wrong place and reports the repository as missing a
        // file it can plainly see.
        indexed.push((
            sha256_hex(catalogue.as_bytes()),
            catalogue.len() as u64,
            format!("{within}/Packages"),
        ));
        indexed.push((
            sha256_hex(&squashed),
            squashed.len() as u64,
            format!("{within}/Packages.gz"),
        ));
    }

    let release_at = out.join("dists").join(suite);
    std::fs::create_dir_all(&release_at).map_err(io(&release_at))?;
    let release = release_at.join("Release");
    std::fs::write(
        &release,
        release_file(options, &architectures, &indexed, now),
    )
    .map_err(io(&release))?;

    Ok(Built {
        root: out.to_path_buf(),
        release,
        packages: found.into_iter().map(|one| one.filename).collect(),
        architectures,
    })
}

/// One package, read and weighed, on its way into the catalogue.
struct Found {
    package: String,
    version: String,
    architecture: String,
    filename: String,
    size: u64,
    sha256: String,
    control: Control,
    bytes: Vec<u8>,
}

/// A name that is about to become a directory, checked before it is one.
///
/// The suite and the component come from whoever is running this, and both are
/// pasted straight into a path. A suite of `../../etc` would write outside the
/// output directory entirely, and an empty one would produce `dists//Release`,
/// which apt cannot ask for. Neither is a plausible typo, but both are silent,
/// so they are refused with a sentence saying what was wrong.
fn checked_name<'a>(what: &str, name: &'a str) -> Result<&'a str, AptError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(AptError::Invalid(format!(
            "the {what} cannot be empty — it is a word on the `deb` line and a \
             directory name in the repository."
        )));
    }
    if trimmed.contains(['/', '\\']) || trimmed == "." || trimmed == ".." {
        return Err(AptError::Invalid(format!(
            "{trimmed:?} cannot be used as the {what}: it is a directory name, \
             so it may not contain a path separator."
        )));
    }
    Ok(trimmed)
}

/// The letter a package sits under in the pool.
///
/// One directory per initial, so that a pool with thousands of packages in it
/// does not become one directory with thousands of entries. Anything beginning
/// `lib` uses four letters instead of one, because otherwise `libx` would hold
/// most of Debian on its own — that is Debian's rule, not an invention here,
/// and following it means a mirror script written for Debian works on this.
fn pool_prefix(package: &str) -> String {
    let lower = package.to_ascii_lowercase();
    if lower.starts_with("lib") && lower.chars().count() > 3 {
        lower.chars().take(4).collect()
    } else {
        lower.chars().take(1).collect()
    }
}

/// A version with any epoch taken off the front.
///
/// Debian versions may begin `1:` — an epoch, used to say that a version which
/// sorts lower than the last one is nonetheless newer. The colon stays in the
/// `Version:` field and comes *off* the filename, because a colon in a path is
/// a reserved character in a URL and not a legal filename on Windows, so a pool
/// containing one cannot be mirrored onto half the machines that might mirror
/// it.
fn without_epoch(version: &str) -> &str {
    match version.split_once(':') {
        Some((_, rest)) => rest,
        None => version,
    }
}

// ---------------------------------------------------------------------------
// The catalogue
// ---------------------------------------------------------------------------

/// The control fields, in the order `dpkg` itself writes them.
///
/// The order carries no meaning to apt, which reads a control file into a map
/// and does not care. It matters for a person: `Packages` files are compared by
/// eye and by `diff` against ones produced by `dpkg-scanpackages`, and a file
/// with the same content in a different order looks like a different file.
const BEFORE_FILENAME: &[&str] = &[
    "Package",
    "Source",
    "Version",
    "Architecture",
    "Essential",
    "Origin",
    "Bugs",
    "Maintainer",
    "Installed-Size",
    "Pre-Depends",
    "Depends",
    "Recommends",
    "Suggests",
    "Enhances",
    "Conflicts",
    "Breaks",
    "Replaces",
    "Provides",
    "Built-Using",
];

/// The fields that come after the file's own details. `Description` is last
/// because it is the one field that runs to several lines, and a reader that
/// loses its place in a stanza loses everything after it.
const AFTER_FILENAME: &[&str] = &["Section", "Priority", "Homepage", "Description"];

/// One package's entry in the catalogue, blank line and all.
///
/// The trailing blank line is part of the stanza rather than something put
/// between stanzas, which is how `dpkg-scanpackages` writes it: a `Packages`
/// file that ends without one is still read correctly by apt, but the two files
/// no longer compare equal, and the difference is invisible on screen.
fn stanza(one: &Found) -> String {
    let mut text = String::new();
    let mut put = |name: &str, value: &str| {
        text.push_str(name);
        text.push_str(": ");
        text.push_str(value);
        text.push('\n');
    };

    for name in BEFORE_FILENAME {
        if let Some(value) = one.control.field(name) {
            put(name, value);
        }
    }
    // Where it is, how big it is, and what it should hash to. Everything apt
    // needs in order to fetch the package and know it got the right one.
    put("Filename", &one.filename);
    put("Size", &one.size.to_string());
    // SHA-256 alone, and no MD5sum or SHA1. Both of those are broken as
    // collision-resistant hashes, and apt has not needed either since 2016 —
    // writing them would be publishing a weaker check beside a strong one and
    // hoping nothing chooses the weaker.
    put("SHA256", &one.sha256);
    for name in AFTER_FILENAME {
        if let Some(value) = one.control.field(name) {
            put(name, value);
        }
    }
    text.push('\n');
    text
}

/// The `Release` file: what apt fetches first and checks everything else
/// against.
///
/// Only SHA-256 hashes are listed. Older repositories carry MD5Sum and SHA1
/// blocks as well, and every apt still in use prefers SHA-256 when it is
/// offered — so the older blocks would add nothing but a weaker statement about
/// the same files.
///
/// This file is left unsigned. Signing it is one `gpg` command and it belongs
/// to whoever owns the key, not to this program: see [`instructions`].
fn release_file(
    options: &RepoOptions,
    architectures: &[String],
    indexed: &[(String, u64, String)],
    now: SystemTime,
) -> String {
    let mut text = String::new();
    let mut put = |name: &str, value: &str| {
        text.push_str(name);
        text.push_str(": ");
        text.push_str(&one_line(value));
        text.push('\n');
    };
    put("Origin", &options.origin);
    put("Label", &options.label);
    put("Suite", &options.suite);
    // The same word as the suite. apt will match a `deb` line against either
    // the Suite or the Codename, and it warns about a Release file that has
    // neither; a repository for one program has no separate release names to
    // give, so both say the same thing and the sources line works whichever of
    // the two a given apt happens to look at.
    put("Codename", &options.suite);
    put("Architectures", &architectures.join(" "));
    put("Components", &options.component);
    put("Description", &options.description);
    put("Date", &rfc1123(now));

    text.push_str("SHA256:\n");
    for (hash, size, path) in indexed {
        // The length is padded out so the columns line up for a person reading
        // the file. apt splits the line on whitespace and does not care.
        text.push_str(&format!(" {hash} {size:>16} {path}\n"));
    }
    text
}

/// A value with its line breaks folded out.
///
/// A control file's fields are one line each unless the continuation is
/// deliberate. A newline arriving from a caller's description would turn the
/// rest of the sentence into a field name apt does not know, and apt's
/// complaint names the file rather than the sentence.
fn one_line(value: &str) -> String {
    value
        .split(['\n', '\r'])
        .map(str::trim)
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

// ---------------------------------------------------------------------------
// Reading a .deb back
// ---------------------------------------------------------------------------

/// The control fields of a package, as they were written in it.
///
/// Field names are matched without regard to case, because that is what the
/// format says and because a package built by hand with `installed-size` in it
/// is otherwise cataloged with the field missing and nothing to say why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Control {
    fields: Vec<(String, String)>,
}

impl Control {
    /// One field's value, or nothing if the package has no such field.
    ///
    /// A field that runs to several lines comes back with its continuation
    /// lines still attached, leading space and all, so that writing
    /// `Description: {value}` reproduces what the package said exactly.
    pub fn field(&self, name: &str) -> Option<&str> {
        self.fields
            .iter()
            .find(|(have, _)| have.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    /// Every field, in the order the package listed them.
    pub fn fields(&self) -> impl Iterator<Item = (&str, &str)> {
        self.fields
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str()))
    }
}

/// Read the control fields out of the bytes of a `.deb`.
///
/// A `.deb` is an `ar` archive of three members, and the one wanted here is
/// `control.tar.gz`: a gzipped tar holding a file called `control` with the
/// fields in it. All three layers are unwrapped by hand, which is the same
/// amount of code as calling out to `dpkg-deb` and works on a machine that does
/// not have `dpkg` at all — a repository is quite often built somewhere that is
/// not the kind of machine it is built for.
///
/// The error is a sentence rather than a type, because everything that can go
/// wrong here means the same thing to whoever is holding the file: this is not
/// a package. What differs is only which part gave it away.
pub fn control(deb: &[u8]) -> Result<Control, String> {
    let members = ar_members(deb)?;
    let (name, bytes) = members
        .iter()
        .find(|(name, _)| name.starts_with("control.tar"))
        .ok_or("it has no control.tar member, so it holds no package details at all")?;

    let tarball = match name.as_str() {
        "control.tar.gz" => gunzip(bytes)?,
        "control.tar" => bytes.to_vec(),
        // xz and zstd are both legal here and both are a real compressor's
        // worth of code. Saying so is better than failing to find the control
        // file and reporting a malformed package.
        other => {
            return Err(format!(
                "its control member is {other}, and only control.tar.gz and \
                 control.tar can be read here. Rebuild the package with \
                 `dpkg-deb -Zgzip`."
            ))
        }
    };

    let (_, control) = tar_entries(&tarball)?
        .into_iter()
        .find(|(name, _)| name == "control" || name == "./control")
        .ok_or("its control.tar has no file called control in it")?;

    let text = String::from_utf8_lossy(control);
    let fields = stanza_fields(&text);
    if fields.is_empty() {
        return Err("its control file is empty".to_string());
    }
    Ok(Control { fields })
}

/// Split a control file into fields, continuation lines and all.
fn stanza_fields(text: &str) -> Vec<(String, String)> {
    let mut fields: Vec<(String, String)> = Vec::new();
    for line in text.lines() {
        if line.trim().is_empty() {
            // A blank line ends the stanza. A package's control file holds one,
            // but a stray blank line at the top should not end it before it has
            // begun.
            if fields.is_empty() {
                continue;
            }
            break;
        }
        if line.starts_with(' ') || line.starts_with('\t') {
            if let Some((_, value)) = fields.last_mut() {
                value.push('\n');
                value.push_str(line);
            }
            continue;
        }
        if let Some((name, value)) = line.split_once(':') {
            fields.push((name.trim().to_string(), value.trim().to_string()));
        }
    }
    fields
}

/// The members of an `ar` archive: a name and the bytes, in order.
fn ar_members(bytes: &[u8]) -> Result<Vec<(String, &[u8])>, String> {
    if !bytes.starts_with(b"!<arch>\n") {
        return Err(
            "it does not begin with the `ar` marker that every .deb begins with".to_string(),
        );
    }
    let mut members = Vec::new();
    let mut at = 8;
    while at + 60 <= bytes.len() {
        let header = &bytes[at..at + 60];
        if &header[58..60] != b"`\n" {
            return Err(format!(
                "the member header {at} bytes in does not end the way an `ar` \
                 header ends, so the file is truncated or is not an archive"
            ));
        }
        let raw = String::from_utf8_lossy(&header[0..16]);
        let name = raw.trim_end().trim_end_matches('/').to_string();
        let length = String::from_utf8_lossy(&header[48..58]);
        let size: usize = length
            .trim()
            .parse()
            .map_err(|_| format!("the member {name:?} does not say how long it is"))?;

        let from = at + 60;
        let to = from
            .checked_add(size)
            .filter(|to| *to <= bytes.len())
            .ok_or_else(|| {
                format!("the member {name:?} says it is {size} bytes, which runs off the end")
            })?;
        members.push((name, &bytes[from..to]));
        // Members start on an even byte, so an odd-length one is followed by a
        // padding byte that belongs to nobody.
        at = to + size % 2;
    }
    Ok(members)
}

/// The files in a tar: a name and the bytes, directories left out.
fn tar_entries(bytes: &[u8]) -> Result<Vec<(String, &[u8])>, String> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while at + 512 <= bytes.len() {
        let header = &bytes[at..at + 512];
        // A block of nothing ends the archive.
        if header.iter().all(|byte| *byte == 0) {
            break;
        }
        let mut name = terminated(&header[0..100]);
        // ustar splits a long name across two fields. Without this a file
        // deeper than a hundred characters comes back under the wrong name.
        if &header[257..262] == b"ustar" {
            let prefix = terminated(&header[345..500]);
            if !prefix.is_empty() {
                name = format!("{prefix}/{name}");
            }
        }
        let size = octal(&header[124..136])
            .ok_or_else(|| format!("the tar entry {name:?} does not say how long it is"))?;

        let from = at + 512;
        let to = from
            .checked_add(size)
            .filter(|to| *to <= bytes.len())
            .ok_or_else(|| {
                format!("the tar entry {name:?} says it is {size} bytes, which runs off the end")
            })?;
        // Type '0' is a plain file, and a NUL in that field means the same
        // thing in the older format this may have been written in.
        if header[156] == b'0' || header[156] == 0 {
            out.push((name, &bytes[from..to]));
        }
        at = to + (512 - size % 512) % 512;
    }
    Ok(out)
}

/// A fixed-width field read up to its first NUL.
fn terminated(field: &[u8]) -> String {
    let text = String::from_utf8_lossy(field);
    text.split('\0').next().unwrap_or("").trim().to_string()
}

/// A tar's octal number field, which may be padded with spaces or NULs.
fn octal(field: &[u8]) -> Option<usize> {
    let text = terminated(field);
    if text.is_empty() {
        return Some(0);
    }
    usize::from_str_radix(&text, 8).ok()
}

/// Undo the gzip, using the compressor already in the tree.
fn gunzip(bytes: &[u8]) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(bytes)
        .read_to_end(&mut out)
        .map_err(|why| format!("its control.tar.gz will not decompress: {why}"))?;
    Ok(out)
}

// ---------------------------------------------------------------------------
// SHA-256
// ---------------------------------------------------------------------------

/// The sixty-four constants SHA-256 is defined with: the first thirty-two bits
/// of the fractional part of the cube roots of the first sixty-four primes.
#[rustfmt::skip]
const ROUND_CONSTANTS: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// The SHA-256 of some bytes.
///
/// Written out here rather than pulled in, for the reason given at the top of
/// this file: it is a small, fixed, twenty-five-year-old specification, and the
/// rest of this program already writes its own CRC-32, tar, zip and `ar`.
///
/// Two details are worth naming because they are the ones a hand-written
/// implementation gets wrong and no short test catches. The padding is a single
/// `0x80` byte followed by zeros — and if the last block has fewer than nine
/// bytes to spare, the length will not fit and a *second* block is needed, so
/// the tail is worked in a buffer of two blocks rather than one. And the length
/// appended at the end is in **bits**, not bytes, as a sixty-four bit
/// big-endian number. Get either wrong and short messages still hash correctly,
/// which is exactly why the tests here include a message of over a megabyte.
pub fn sha256(bytes: &[u8]) -> [u8; 32] {
    // The first thirty-two bits of the fractional part of the square roots of
    // the first eight primes.
    let mut state: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];

    let whole = bytes.len() - bytes.len() % 64;
    for block in bytes[..whole].chunks_exact(64) {
        compress(&mut state, block);
    }

    // What is left over, the padding, and the length — in one or two blocks
    // depending on whether the length fits after the padding.
    let rest = &bytes[whole..];
    let mut tail = [0u8; 128];
    tail[..rest.len()].copy_from_slice(rest);
    tail[rest.len()] = 0x80;
    let tail_len = if rest.len() < 56 { 64 } else { 128 };
    let bits = (bytes.len() as u64).wrapping_mul(8);
    tail[tail_len - 8..tail_len].copy_from_slice(&bits.to_be_bytes());
    for block in tail[..tail_len].chunks_exact(64) {
        compress(&mut state, block);
    }

    let mut out = [0u8; 32];
    for (four, word) in out.chunks_exact_mut(4).zip(state.iter()) {
        four.copy_from_slice(&word.to_be_bytes());
    }
    out
}

/// One sixty-four byte block, mixed into the running state.
fn compress(state: &mut [u32; 8], block: &[u8]) {
    // The block as sixteen words, then stretched to sixty-four.
    let mut schedule = [0u32; 64];
    for (word, four) in schedule.iter_mut().zip(block.chunks_exact(4)) {
        *word = u32::from_be_bytes([four[0], four[1], four[2], four[3]]);
    }
    for i in 16..64 {
        let a = schedule[i - 15];
        let b = schedule[i - 2];
        let s0 = a.rotate_right(7) ^ a.rotate_right(18) ^ (a >> 3);
        let s1 = b.rotate_right(17) ^ b.rotate_right(19) ^ (b >> 10);
        schedule[i] = schedule[i - 16]
            .wrapping_add(s0)
            .wrapping_add(schedule[i - 7])
            .wrapping_add(s1);
    }

    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = *state;
    for (constant, word) in ROUND_CONSTANTS.iter().zip(schedule.iter()) {
        let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let choose = (e & f) ^ ((!e) & g);
        let one = h
            .wrapping_add(s1)
            .wrapping_add(choose)
            .wrapping_add(*constant)
            .wrapping_add(*word);
        let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let majority = (a & b) ^ (a & c) ^ (b & c);
        let two = s0.wrapping_add(majority);

        h = g;
        g = f;
        f = e;
        e = d.wrapping_add(one);
        d = c;
        c = b;
        b = a;
        a = one.wrapping_add(two);
    }

    for (had, now) in state.iter_mut().zip([a, b, c, d, e, f, g, h]) {
        *had = had.wrapping_add(now);
    }
}

/// The SHA-256 of some bytes, written the way apt writes it: sixty-four
/// lower-case hexadecimal digits.
pub fn sha256_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(64);
    for byte in sha256(bytes) {
        text.push(DIGITS[(byte >> 4) as usize] as char);
        text.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    text
}

// ---------------------------------------------------------------------------
// The date
// ---------------------------------------------------------------------------

const WEEKDAYS: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// A moment written the way a `Release` file writes it: `Sun, 27 Jul 2026
/// 22:10:00 UTC`.
///
/// Always UTC, never the machine's own time zone. apt compares this date
/// against `Valid-Until` and against the date on the copy it already has, and a
/// repository built at nine in the morning in one country and rebuilt at eight
/// in another would appear to go backwards in time — which apt reports as a
/// downgrade attack, in those words, and refuses.
///
/// A moment before 1970 cannot be represented on the `deb` line at all and is
/// not worth an error path: it comes out as the epoch itself, which is
/// obviously wrong to anybody reading the file, rather than as a failure in the
/// middle of writing a repository.
pub fn rfc1123(at: SystemTime) -> String {
    let seconds = at
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or(0);
    let days = (seconds / 86_400) as i64;
    let within = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    // 1 January 1970 was a Thursday, which is index four counting from Sunday.
    let weekday = WEEKDAYS[((days + 4).rem_euclid(7)) as usize];
    let month = MONTHS[(month - 1) as usize];
    format!(
        "{weekday}, {day:02} {month} {year:04} {:02}:{:02}:{:02} UTC",
        within / 3600,
        (within % 3600) / 60,
        within % 60
    )
}

/// The year, month and day a count of days since 1 January 1970 lands on.
///
/// This is the part of date handling that everybody writes wrong once. The
/// obvious approach — walk forward a year at a time, then a month at a time —
/// works and is slow and has a leap-year test in the middle of it that is
/// wrong about 1900 and 2100 roughly half the time it is written from memory.
/// The rule is not "every four years": a century is not a leap year unless it
/// divides by four hundred, so 2000 was one and 2100 will not be.
///
/// So this uses Howard Hinnant's arithmetic instead, which has no loop and no
/// leap-year test at all. The trick is to move the start of the year to the
/// first of March, so that the leap day becomes the *last* day of the year
/// rather than one wedged into the middle of it. After that a four-hundred-year
/// era is exactly 146,097 days with no exceptions anywhere, and the month and
/// day fall out of two divisions. It is exact for any date it is given, which
/// matters here for one dull reason: a wrong date in a `Release` file makes apt
/// say the repository is either not yet valid or has expired, and neither
/// message mentions the date.
pub fn civil_from_days(days: i64) -> (i64, u32, u32) {
    // Counting from 1 March 0000 rather than 1 January 1970.
    let shifted = days + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = shifted - era * 146_097; // 0 to 146,096
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    // Months numbered from March, which is what lets the leap day be last.
    let shuffled = (5 * day_of_year + 2) / 153; // 0 to 11
    let day = (day_of_year - (153 * shuffled + 2) / 5 + 1) as u32;
    let month = if shuffled < 10 {
        shuffled + 3
    } else {
        shuffled - 9
    };
    // January and February belong to the year after the one that started in
    // March.
    (year + i64::from(month <= 2), month as u32, day)
}

// ---------------------------------------------------------------------------
// What to do with it
// ---------------------------------------------------------------------------

/// Where a keyring has to be put, and why it is this directory and not the
/// other one.
///
/// Debian's own advice for third-party repositories names `/etc/apt/keyrings`,
/// and that is the better home for a key that did not come from a package. It
/// is also a directory that does not exist on anything older than Debian 12 or
/// Ubuntu 22.04, and `tee` into a directory that is not there fails with "No
/// such file or directory" naming the *file* — so somebody following the
/// instructions sees a message about a keyring they were told to create, on a
/// line that appears to have created it. `/usr/share/keyrings` exists on every
/// machine that has apt at all, which is what keeps the setup to two lines that
/// work everywhere.
const KEYRING: &str = "/usr/share/keyrings/onionskin-archive-keyring.gpg";

/// The exact commands to publish this repository, and the exact two lines
/// somebody has to run to install from it.
///
/// `url` is where the directory will be served from — the same address that
/// goes on the `deb` line, with no trailing slash and no `dists` on the end.
///
/// # Why the signing is somebody else's job
///
/// Every command here that touches a key is a `gpg` command. That is
/// deliberate. Hashing is arithmetic and is written out in full a few hundred
/// lines above; signing is custody of a private key, and a mistake in key
/// custody does not announce itself the way a wrong hash does — it produces a
/// repository that works perfectly and that somebody else can also sign. `gpg`
/// is already on the machine, people already know how to back up a key with it,
/// and it is not this program's business to invent a second way to keep one.
///
/// # Why not `apt-key`
///
/// Because a key added with `apt-key` is trusted for *every* repository the
/// machine has, not only this one — so a small program's signing key ends up
/// able to vouch for a replacement to anything at all, including the operating
/// system. It has been deprecated since apt 2.2 and removed since Debian 12.
/// The `signed-by=` form below ties the key to this one repository, which is
/// the only thing it should ever be able to speak for.
pub fn instructions(options: &RepoOptions, url: &str) -> String {
    let url = url.trim().trim_end_matches('/');
    let suite = options.suite.trim();
    let component = options.component.trim();
    let keyring = KEYRING;
    let name = keyring.rsplit('/').next().unwrap_or("keyring.gpg");

    // Indented lines are written `\x20   ` rather than four spaces, because
    // Rust's line continuation eats the whitespace at the start of the next
    // source line — and the indent is the only thing marking a command out
    // from the prose.
    format!(
        "Publishing this as an apt repository\n\
         ====================================\n\
         \n\
         The files are written. What is left is a signature, because apt will\n\
         not install from a repository it cannot check: it stops with \"the\n\
         repository is not signed\" and there is no polite way round it.\n\
         \n\
         1. Make a signing key, once, and keep it somewhere safe. It is the\n\
         \x20  thing that says these packages came from you, and nobody can\n\
         \x20  replace it for you if it is lost.\n\
         \n\
         \x20   gpg --quick-generate-key \"Onionskin archive <you@example.com>\" rsa4096 sign never\n\
         \x20   gpg --list-secret-keys --keyid-format=long\n\
         \n\
         \x20  The second command prints the key id. It is the long hexadecimal\n\
         \x20  number after `sec   rsa4096/`. Use it in place of KEYID below.\n\
         \n\
         2. Sign the Release file. Do this again every single time you rebuild\n\
         \x20  the repository — an old signature over a new Release is worse\n\
         \x20  than none at all, because apt tells everybody the repository has\n\
         \x20  been tampered with and stops updating anything.\n\
         \n\
         \x20   gpg --yes --default-key KEYID --clearsign -o dists/{suite}/InRelease dists/{suite}/Release\n\
         \x20   gpg --yes --default-key KEYID --detach-sign --armor -o dists/{suite}/Release.gpg dists/{suite}/Release\n\
         \n\
         \x20  Both, not one. InRelease is the signed file modern apt asks for\n\
         \x20  first; Release.gpg is the detached signature older apt falls back\n\
         \x20  to, and it costs nothing to write.\n\
         \n\
         3. Export the public half of the key and put it beside the repository,\n\
         \x20  so people have something to fetch.\n\
         \n\
         \x20   gpg --export KEYID > {name}\n\
         \n\
         \x20  Not `--armor`. A file named .gpg has to be a binary keyring, and\n\
         \x20  an armoured one is accepted by the command below and then fails\n\
         \x20  at update time with \"the public key is not available\", which\n\
         \x20  says nothing about the real cause.\n\
         \n\
         4. Serve the whole directory over HTTPS at\n\
         \n\
         \x20   {url}\n\
         \n\
         \x20  Any static web server will do — there is nothing to run and no\n\
         \x20  software to install on it. Copy the directory as it stands and\n\
         \x20  keep the layout, with the keyring at the top of it beside\n\
         \x20  `dists` and `pool`, so that this address fetches it:\n\
         \n\
         \x20   {url}/{name}\n\
         \n\
         Then people install it with two lines\n\
         =====================================\n\
         \n\
         \x20   curl -fsSL {url}/{name} | sudo tee {keyring} > /dev/null\n\
         \x20   echo \"deb [signed-by={keyring}] {url} {suite} {component}\" | sudo tee /etc/apt/sources.list.d/onionskin.list\n\
         \n\
         and from then on it is an ordinary package:\n\
         \n\
         \x20   sudo apt update\n\
         \x20   sudo apt install onionskin\n\
         \n\
         `signed-by=` names the one key allowed to vouch for this one\n\
         repository. The older `apt-key add` trusted a key for everything on\n\
         the machine, including the operating system itself; it has been\n\
         deprecated since apt 2.2 and is gone from Debian 12 onwards.\n\
         \n\
         To remove the repository again:\n\
         \n\
         \x20   sudo rm /etc/apt/sources.list.d/onionskin.list {keyring}\n"
    )
}

#[cfg(test)]
mod tests;
