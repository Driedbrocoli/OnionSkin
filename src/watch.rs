//! A folder the scanner drops into.
//!
//! The office multifunction has a button on the front that scans to a folder.
//! Somebody presses it, walks back to their desk, opens the file, runs a
//! command on it, prints the answer, and walks back to the machine. The middle
//! three steps are the ones a computer should be doing.
//!
//! So Onionskin can sit and watch that folder. A file lands; the saved job runs
//! on it; the delta appears beside it. The person presses the button and
//! collects two sheets instead of one.
//!
//! ```text
//! onionskin watch ~/Scans --job paid
//! ```
//!
//! # Why it polls, and why that is the right answer here
//!
//! There is a proper way to be told about a changed folder — inotify on Linux,
//! ReadDirectoryChangesW on Windows, FSEvents on macOS — and three of them,
//! one per platform, plus a crate to hide the differences. None of that is
//! wanted here. The thing being waited for is a person walking to a scanner
//! and back, which takes a minute; noticing it two seconds late costs nothing
//! anybody can perceive. A directory listing every couple of seconds is a few
//! microseconds of work, and it is the same few microseconds on every operating
//! system this runs on, including the network share that the scanner actually
//! writes to and where the change notifications would not have arrived anyway.
//!
//! # Not touching a file that is still arriving
//!
//! A scanner writing a ten-megabyte PDF over SMB does it over several seconds,
//! and for most of those seconds there is a file of that name holding half a
//! document. Opening it then gets a confusing error at best.
//!
//! The rule is that a file has to look the same twice running. Its size and its
//! modified time are taken each sweep, and it is left alone until two
//! consecutive sweeps agree. That costs one extra sweep of latency — two
//! seconds — and it does not need a lock, a rename convention, or the scanner's
//! cooperation, none of which are available.
//!
//! # Remembering what has been done
//!
//! Otherwise stopping and starting the program does every file in the folder
//! again, which for overprinting means a second delta for a sheet that has
//! already had one printed on it.
//!
//! What is remembered is the file's path, size and modified time — not a hash
//! of its contents. A hash would be the more honest identity, and it is what
//! [`crate::history`] uses for deltas, but it means reading every file in the
//! folder on every sweep. Size and modified time are what every backup program
//! in existence uses for the same decision, they are free (the directory
//! listing already has them), and the failure they admit — a file replaced by a
//! different file of exactly the same length in the same second — is not a
//! thing scanners do.
//!
//! Failures are remembered too. A PDF that cannot be opened will not open on
//! the next sweep either, and a folder containing one should not produce the
//! same error message every two seconds for the rest of the afternoon.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// How long between sweeps, in seconds.
///
/// Two: fast enough that the delta is there before somebody has walked back to
/// their desk, slow enough that a folder of a thousand files costs nothing.
pub const BETWEEN_SWEEPS_SECONDS: u64 = 2;

/// What Onionskin adds to a name when it writes the delta beside the source.
pub const TAIL: &str = "-delta";

/// The tails Onionskin puts on the things it writes.
///
/// A watched folder will fill up with them, and the one thing that must not
/// happen is a delta being treated as a fresh document and given a delta of its
/// own — which repeats forever, two seconds apart, until the disk is full.
///
/// This is by name rather than by reading the file, because it has to be
/// decided about every file in the folder on every sweep and reading them all
/// would not be free. It is safe in the direction that matters: a document
/// somebody has genuinely named `march-delta.pdf` is skipped, which is a
/// nuisance they can fix by renaming it, whereas the other mistake fills a
/// disk.
pub const OUR_TAILS: &[&str] = &[
    "-delta",
    "-proof",
    "-merged",
    "-joined",
    "-labels",
    "-batch",
    "-corrected",
    "-covered",
    "-watermark",
    "-barcode",
    "-back",
    "-printed",
    "-which-way-up",
];

/// Endings that mean a file is not finished being written.
///
/// Browsers, sync clients and Office all use one. A file called `scan.pdf.part`
/// will become `scan.pdf` when it is done, and that is the one to work on.
pub const HALF_WRITTEN: &[&str] = &[
    ".part",
    ".crdownload",
    ".tmp",
    ".temp",
    ".download",
    ".partial",
    ".filepart",
    ".opdownload",
    "~",
];

/// How a file looked when it was last listed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Look {
    pub size: u64,
    /// Seconds since the epoch, or zero if the filesystem would not say.
    pub modified: u64,
}

/// One file in the folder, this time round.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seen {
    pub path: PathBuf,
    pub look: Look,
}

/// Why a file in the folder is not something to work on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Leave {
    /// A dotfile, or one of the operating system's own droppings.
    Hidden,
    /// Still being written under a temporary name.
    HalfWritten,
    /// Onionskin wrote it.
    OurOwn,
    /// A picture, which a saved job cannot be run on.
    APicture,
    /// Some other kind of file.
    NotADocument(String),
    /// No extension at all, so there is nothing to go on.
    Nameless,
}

impl Leave {
    /// What to tell somebody who asks why this one was passed over.
    pub fn why(&self) -> String {
        match self {
            Leave::Hidden => "hidden".to_string(),
            Leave::HalfWritten => "still being written".to_string(),
            Leave::OurOwn => "Onionskin wrote it".to_string(),
            Leave::APicture => "a picture — a saved job writes on a document. For a scanned \
                 sheet use: onionskin read"
                .to_string(),
            Leave::NotADocument(suffix) => format!("not a document Onionskin opens (.{suffix})"),
            Leave::Nameless => "no file extension, so its kind is unknown".to_string(),
        }
    }
}

/// What this sweep decides about one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Run the job on it.
    Do,
    /// It changed since the last look, so it is probably still arriving.
    StillArriving,
    /// Already done, at that time.
    DoneBefore(u64),
    /// Already tried, and it did not work. The trouble is repeated so it can
    /// be reported once rather than every two seconds.
    FailedBefore(String),
    /// Not ours to touch.
    Leave(Leave),
}

impl Verdict {
    pub fn is_work(&self) -> bool {
        matches!(self, Verdict::Do)
    }
}

/// One file that has been dealt with, kept so a restart does not do it again.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Handled {
    /// Seconds since the epoch.
    pub at: u64,
    /// The file the job was run on, as an absolute path where one could be had.
    pub source: String,
    pub look: Look,
    /// Where the delta went. Empty when the job did not get that far.
    #[serde(default)]
    pub delta: String,
    /// Empty when it worked; what went wrong when it did not.
    #[serde(default)]
    pub trouble: String,
}

impl Handled {
    pub fn worked(&self) -> bool {
        self.trouble.is_empty()
    }
}

/// How many are kept, per watched folder.
///
/// A folder is swept by name and the record is only consulted for files that
/// are still there, so the old entries are dead weight rather than wrong. Two
/// thousand is a year of a busy scanner and about a fifth of a megabyte.
pub const KEEP: usize = 2000;

/// What has been done, and to which files.
#[derive(Debug, Clone, Default)]
pub struct Ledger {
    by_path: BTreeMap<String, Handled>,
}

impl Ledger {
    pub fn new() -> Ledger {
        Ledger::default()
    }

    /// What was done to this exact file, if it is the same file it was.
    ///
    /// "The same" means the same path, size and modified time. A sheet scanned
    /// again over the top of the old one has a new modified time, and gets a
    /// new delta, which is what somebody rescanning a page wants.
    pub fn knows(&self, path: &Path, look: Look) -> Option<&Handled> {
        self.by_path
            .get(&key_for(path))
            .filter(|had| had.look == look)
    }

    pub fn add(&mut self, handled: Handled) {
        self.by_path
            .insert(key_for(Path::new(&handled.source)), handled);
    }

    pub fn len(&self) -> usize {
        self.by_path.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_path.is_empty()
    }

    /// Everything in it, oldest first.
    pub fn all(&self) -> Vec<&Handled> {
        let mut all: Vec<&Handled> = self.by_path.values().collect();
        all.sort_by_key(|had| had.at);
        all
    }

    /// How many worked and how many did not.
    pub fn tally(&self) -> (usize, usize) {
        let worked = self.by_path.values().filter(|had| had.worked()).count();
        (worked, self.by_path.len() - worked)
    }
}

/// A path in the form the ledger is keyed by.
///
/// Absolute where the filesystem will say so, because `watch .` and
/// `watch /home/j/Scans` are the same folder and must not each do the work.
fn key_for(path: &Path) -> String {
    let absolute = path.canonicalize().unwrap_or_else(|_| {
        match (path.is_absolute(), std::env::current_dir()) {
            (false, Ok(here)) => here.join(path),
            _ => path.to_path_buf(),
        }
    });
    absolute.to_string_lossy().into_owned()
}

#[derive(Debug, thiserror::Error)]
pub enum WatchError {
    #[error("'{0}' is not there. Give a folder that exists — the one the scanner writes into.")]
    NoFolder(PathBuf),
    #[error(
        "'{0}' is a file, not a folder. Watching means watching a folder for files to land in."
    )]
    NotAFolder(PathBuf),
    #[error("could not read the folder {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
}

/// Everything in the folder, as it looks now, in a settled order.
///
/// Sorted by name so two sweeps of an unchanged folder report the same thing in
/// the same order, and so a person reading the output sees a list rather than
/// whatever order the filesystem felt like.
pub fn listing(folder: &Path) -> Result<Vec<Seen>, WatchError> {
    if !folder.exists() {
        return Err(WatchError::NoFolder(folder.to_path_buf()));
    }
    if !folder.is_dir() {
        return Err(WatchError::NotAFolder(folder.to_path_buf()));
    }
    let entries = std::fs::read_dir(folder).map_err(|source| WatchError::Io {
        path: folder.to_path_buf(),
        source,
    })?;

    let mut seen: Vec<Seen> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        // Folders are not descended into. A scanner writes into the folder it
        // was pointed at, and walking a tree turns "watch my scans" into
        // "walk my home directory every two seconds".
        if !path.is_file() {
            continue;
        }
        if let Some(look) = look_at(&path) {
            seen.push(Seen { path, look });
        }
    }
    seen.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(seen)
}

/// How a file looks now, or nothing if it cannot be asked.
pub fn look_at(path: &Path) -> Option<Look> {
    let data = std::fs::metadata(path).ok()?;
    let modified = data
        .modified()
        .ok()
        .and_then(|when| when.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|since| since.as_secs())
        .unwrap_or(0);
    Some(Look {
        size: data.len(),
        modified,
    })
}

/// Whether this file has stopped changing.
///
/// It has to have been seen before, looking exactly as it does now. An empty
/// file is never settled: a scanner creates the name first and fills it after,
/// so nought bytes means the interesting part has not arrived.
pub fn settled(before: Option<Look>, now: Look) -> bool {
    now.size > 0 && before == Some(now)
}

/// Whether this is a file a saved job could be run on at all.
pub fn worth_opening(path: &Path) -> Result<(), Leave> {
    let name = match path.file_name().map(|n| n.to_string_lossy().into_owned()) {
        Some(name) => name,
        None => return Err(Leave::Nameless),
    };
    // Dotfiles, and the droppings every desktop leaves in every folder.
    if name.starts_with('.') || name.starts_with("~$") {
        return Err(Leave::Hidden);
    }
    let lower = name.to_lowercase();
    if HALF_WRITTEN.iter().any(|end| lower.ends_with(end)) {
        return Err(Leave::HalfWritten);
    }

    let suffix = match path.extension() {
        Some(suffix) => suffix.to_string_lossy().to_lowercase(),
        None => return Err(Leave::Nameless),
    };
    let stem = path
        .file_stem()
        .map(|stem| stem.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if OUR_TAILS.iter().any(|tail| stem.ends_with(tail)) {
        return Err(Leave::OurOwn);
    }

    if matches!(
        suffix.as_str(),
        "png" | "jpg" | "jpeg" | "tif" | "tiff" | "bmp" | "gif" | "webp"
    ) {
        return Err(Leave::APicture);
    }
    let known = crate::render::CONVERTIBLE.contains(&suffix.as_str())
        || crate::render::PASSTHROUGH.contains(&suffix.as_str());
    if known {
        Ok(())
    } else {
        Err(Leave::NotADocument(suffix))
    }
}

/// What to do about one file, given what the last sweep saw and what has been
/// done before.
pub fn what_to_do(seen: &Seen, before: Option<Look>, done: &Ledger) -> Verdict {
    if let Err(leave) = worth_opening(&seen.path) {
        return Verdict::Leave(leave);
    }
    // Asked before the settle check on purpose: a file that was finished and
    // dealt with last week is not "still arriving" merely because this is the
    // first sweep since the program started.
    if let Some(had) = done.knows(&seen.path, seen.look) {
        return if had.worked() {
            Verdict::DoneBefore(had.at)
        } else {
            Verdict::FailedBefore(had.trouble.clone())
        };
    }
    if !settled(before, seen.look) {
        return Verdict::StillArriving;
    }
    Verdict::Do
}

/// What to do about everything in the folder.
pub fn decide(
    now: &[Seen],
    before: &BTreeMap<PathBuf, Look>,
    done: &Ledger,
) -> Vec<(Seen, Verdict)> {
    now.iter()
        .map(|seen| {
            let verdict = what_to_do(seen, before.get(&seen.path).copied(), done);
            (seen.clone(), verdict)
        })
        .collect()
}

/// What this sweep saw, to be handed to the next one.
pub fn remember_looks(now: &[Seen]) -> BTreeMap<PathBuf, Look> {
    now.iter()
        .map(|seen| (seen.path.clone(), seen.look))
        .collect()
}

/// Where the delta for this file goes.
///
/// Beside it by default, which is what makes the pair obvious in a file
/// manager; into a folder of their own when one is named, which is what makes
/// the scans folder stay a scans folder.
pub fn where_the_delta_goes(source: &Path, into: Option<&Path>) -> PathBuf {
    let stem = source
        .file_stem()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "onionskin".to_string());
    let name = format!("{stem}{TAIL}.pdf");
    match into {
        Some(folder) => folder.join(name),
        None => source.parent().unwrap_or(Path::new("")).join(name),
    }
}

/// Where the record for a watched folder lives.
///
/// One file per folder, named after the folder, so watching two folders keeps
/// two records and forgetting one is a matter of deleting one file.
pub fn ledger_path(folder: &Path) -> PathBuf {
    let key = key_for(folder);
    let named = crate::apt::sha256_hex(key.as_bytes());
    crate::calibrate::home_dir()
        .join("watched")
        .join(format!("{}.jsonl", &named[..16]))
}

/// Read the record for a folder. An unreadable or absent one is an empty one:
/// watching a folder for the first time is the ordinary case.
pub fn read_ledger(folder: &Path) -> Ledger {
    let mut ledger = Ledger::new();
    let Ok(text) = std::fs::read_to_string(ledger_path(folder)) else {
        return ledger;
    };
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        // A line written by a version that kept something else is skipped
        // rather than throwing away the rest of the record.
        if let Ok(handled) = serde_json::from_str::<Handled>(line) {
            ledger.add(handled);
        }
    }
    ledger
}

/// Add one to the record on disk.
///
/// Appended a line at a time so that stopping the program — which is how it is
/// always stopped — costs at most the file it was in the middle of.
pub fn write_down(folder: &Path, handled: &Handled) -> std::io::Result<()> {
    let path = ledger_path(folder);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let line = serde_json::to_string(handled)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

    // Trimmed only when it has grown past what is kept, which for a busy
    // scanner is once a year.
    let lines = std::fs::read_to_string(&path)
        .map(|text| text.lines().filter(|l| !l.trim().is_empty()).count())
        .unwrap_or(0);
    if lines >= KEEP {
        let kept = read_ledger(folder);
        let mut text: String = kept
            .all()
            .into_iter()
            .rev()
            .take(KEEP / 2)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .filter_map(|had| serde_json::to_string(had).ok())
            .map(|line| format!("{line}\n"))
            .collect();
        text.push_str(&line);
        text.push('\n');
        return std::fs::write(&path, text);
    }

    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    writeln!(file, "{line}")
}

/// Forget what has been done in a folder, so the next sweep does it all again.
pub fn forget(folder: &Path) -> bool {
    std::fs::remove_file(ledger_path(folder)).is_ok()
}

/// What one sweep did, for the line printed after it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tally {
    pub done: usize,
    pub failed: usize,
    pub arriving: usize,
    pub already: usize,
    pub left: usize,
}

impl Tally {
    /// Count up a set of verdicts, before any work is done.
    pub fn of(verdicts: &[(Seen, Verdict)]) -> Tally {
        let mut tally = Tally::default();
        for (_, verdict) in verdicts {
            match verdict {
                Verdict::Do => tally.done += 1,
                Verdict::StillArriving => tally.arriving += 1,
                Verdict::DoneBefore(_) => tally.already += 1,
                Verdict::FailedBefore(_) => tally.failed += 1,
                Verdict::Leave(_) => tally.left += 1,
            }
        }
        tally
    }

    /// One line for somebody watching the terminal, or nothing when there is
    /// nothing to say — which is most sweeps, and printing "nothing happened"
    /// every two seconds would bury the sweeps where something did.
    pub fn line(&self) -> Option<String> {
        if self.done == 0 && self.arriving == 0 {
            return None;
        }
        let mut parts = Vec::new();
        if self.done > 0 {
            parts.push(format!("{} to do", self.done));
        }
        if self.arriving > 0 {
            parts.push(format!("{} still arriving", self.arriving));
        }
        Some(parts.join(", "))
    }
}

/// What is printed when watching starts, so somebody knows what they are
/// looking at and how to stop it.
pub fn how_to_stop() -> &'static str {
    "Watching. Press Ctrl-C to stop."
}

#[cfg(test)]
#[path = "watch/tests.rs"]
mod tests;
