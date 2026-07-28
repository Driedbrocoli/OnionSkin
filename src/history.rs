//! What was added to which sheet, and when.
//!
//! Overprinting is the one operation this program does that cannot be undone.
//! Toner does not come off paper, so a delta printed twice onto the same sheet
//! puts every letter down twice — a little heavier, a little blurred, and
//! unfixable. It is an easy mistake to make: the delta is a file like any other,
//! it prints without complaint, and nothing about the second time looks
//! different from the first.
//!
//! So every delta Onionskin writes is remembered, by a fingerprint of the file
//! itself. Write the same one again and it says when you wrote it before. That
//! is not a refusal — printing the same delta onto a *different* sheet is
//! exactly what a hundred certificates are — but it is the question worth being
//! asked.
//!
//! The record is worth having on its own account, too. "What did we add to that
//! invoice, and when" is a question somebody asks months later about a sheet of
//! paper in a filing cabinet, and the answer used to be nowhere.
//!
//! # What is kept, and what is not
//!
//! Where the files were, how many pages and additions, and the fingerprint.
//! **Not the words themselves.** A log of everything anybody ever wrote onto
//! anything would be a far more sensitive file than any document it describes,
//! sitting in a home directory being backed up, and this program's whole claim
//! is that your documents stay yours. The fingerprint identifies a delta
//! without describing it.
//!
//! It is JSON Lines: one object per line, appended. That means a crash halfway
//! through costs the last line rather than the file, and it can be read with
//! anything.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// How many are kept. Beyond this the oldest go, because a file that grows
/// forever in somebody's home directory is a bug with a long fuse.
pub const KEEP: usize = 500;

/// One delta, and what is known about it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    /// Seconds since the epoch.
    pub at: u64,
    /// What it was made from.
    pub source: String,
    /// Where it was written.
    pub delta: String,
    pub pages: usize,
    pub additions: usize,
    /// Of the delta file's own bytes. Onionskin writes the same PDF for the
    /// same additions, so this is the same delta rather than merely a similar
    /// one.
    pub fingerprint: String,
}

impl Entry {
    /// One line about it, for somebody reading a list.
    pub fn describe(&self) -> String {
        format!(
            "{}  {:<28} {} addition(s) on {} page(s) → {}",
            when(self.at),
            shorten(&self.source, 28),
            self.additions,
            self.pages,
            shorten(&self.delta, 40),
        )
    }

    /// The date and time it was written.
    pub fn when(&self) -> String {
        when(self.at)
    }

    /// How long ago, in words somebody reads rather than a timestamp.
    pub fn how_long_ago(&self) -> String {
        let now = now();
        if now <= self.at {
            return "just now".to_string();
        }
        let seconds = now - self.at;
        match seconds {
            0..=90 => "a moment ago".to_string(),
            91..=5400 => format!("{} minutes ago", seconds / 60),
            5401..=172_800 => format!("{} hours ago", seconds / 3600),
            _ => format!("{} days ago", seconds / 86_400),
        }
    }
}

/// Where the record lives.
pub fn path() -> PathBuf {
    crate::calibrate::home_dir().join("history.jsonl")
}

/// The fingerprint of a delta file, or nothing if it cannot be read.
pub fn fingerprint(delta: &Path) -> Option<String> {
    let bytes = std::fs::read(delta).ok()?;
    Some(crate::apt::sha256_hex(&bytes))
}

/// Add one, and say whether the same delta was written before.
///
/// The lookup happens before the append, so a delta is never reported as a
/// repeat of itself.
pub fn remember(entry: Entry) -> Option<Entry> {
    let before = seen_before(&entry.fingerprint);
    append(&entry);
    before
}

/// The last time this exact delta was written, if it was.
pub fn seen_before(fingerprint: &str) -> Option<Entry> {
    read()
        .into_iter()
        .rfind(|entry| entry.fingerprint == fingerprint)
}

/// Everything remembered, oldest first.
pub fn read() -> Vec<Entry> {
    let Ok(text) = std::fs::read_to_string(path()) else {
        return Vec::new();
    };
    text.lines()
        .filter(|line| !line.trim().is_empty())
        // A line from a version that wrote something else is skipped rather
        // than throwing away the rest of the record.
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

/// The most recent first, which is the order somebody wants to read them in.
pub fn recent(limit: usize) -> Vec<Entry> {
    let mut all = read();
    all.reverse();
    all.truncate(limit);
    all
}

/// Forget everything. Returns how many were forgotten.
/// The delta written most recently that is still on disk.
///
/// `verify` and `proof` both want a delta that was written minutes ago, often
/// into a scratch folder under a name nobody chose and nobody remembers. The
/// record already knows what it was.
///
/// Still on disk, because a delta is temporary by default and `tidy` takes
/// them away: naming a file that is no longer there would be worse than saying
/// nothing, and the caller can then ask for one by name.
pub fn last_delta() -> Option<PathBuf> {
    read()
        .into_iter()
        .rev()
        .map(|entry| PathBuf::from(entry.delta))
        .find(|path| path.is_file())
}

pub fn forget() -> usize {
    let had = read().len();
    let _ = std::fs::remove_file(path());
    had
}

/// Append one line, and trim the file if it has grown past [`KEEP`].
///
/// Silent on failure, by design: somebody who only wanted to write a delta
/// should not have it fail because a directory in their home is read-only.
fn append(entry: &Entry) {
    let Ok(line) = serde_json::to_string(entry) else {
        return;
    };
    let path = path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    let mut kept = read();
    kept.push(entry.clone());
    if kept.len() > KEEP {
        // Rewritten rather than appended to, which is the only time the whole
        // file is touched — once every five hundred deltas.
        let from = kept.len() - KEEP;
        let text: String = kept[from..]
            .iter()
            .filter_map(|entry| serde_json::to_string(entry).ok())
            .map(|line| format!("{line}\n"))
            .collect();
        let _ = std::fs::write(&path, text);
        return;
    }

    use std::io::Write;
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = writeln!(file, "{line}");
    }
}

/// Seconds since the epoch, or zero on a machine whose clock is before it.
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| since.as_secs())
        .unwrap_or(0)
}

/// A date somebody can read, from seconds since the epoch.
///
/// Written out rather than pulled in: this is one calendar rule and a division,
/// and `apt` already needed the same arithmetic to write a `Release` file.
fn when(at: u64) -> String {
    let days = (at / 86_400) as i64;
    let (year, month, day) = crate::apt::civil_from_days(days);
    let seconds = at % 86_400;
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}",
        seconds / 3600,
        (seconds % 3600) / 60
    )
}

/// A path short enough to sit in a column, cut at the front so the file name —
/// the part that identifies it — is what survives.
fn shorten(path: &str, to: usize) -> String {
    let count = path.chars().count();
    if count <= to {
        return path.to_string();
    }
    let skip = count.saturating_sub(to.saturating_sub(1));
    format!("…{}", path.chars().skip(skip).collect::<String>())
}

#[cfg(test)]
#[path = "history/tests.rs"]
mod tests;
