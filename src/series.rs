//! Numbering that carries on where the last run left off.
//!
//! A receipt book is printed two hundred at a time. The first box is numbered 1
//! to 200, and the second box has to be numbered 201 to 400 — because two
//! receipts with the same number on them is the one thing a receipt book must
//! never contain, and it is found out months later by an accountant.
//!
//! `--count 200` numbers from 1 every time, which is right for a run of
//! certificates and wrong for anything that is a *series*. So a series can be
//! given a name, and Onionskin remembers where it got to:
//!
//! ```text
//! onionskin batch receipt.pdf --count 200 --at '150,40:No. {number}' --series receipts
//! onionskin batch receipt.pdf --count 200 --at '150,40:No. {number}' --series receipts
//! ```
//!
//! The first run makes 1 to 200; the second makes 201 to 400, without anybody
//! having to remember which box was last.
//!
//! # Advanced afterwards, by what was really printed
//!
//! Not before, and not by what was asked for. A run that fails writes nothing
//! and must not burn two hundred numbers — the next run would start at 401 with
//! nothing between 201 and 400 in existence, and a receipt book with a gap in it
//! is as hard to explain as one with a repeat. `--dry-run` does not advance it
//! either, for the same reason: a rehearsal that changed the state would not be
//! a rehearsal.
//!
//! `--first 5`, which stops after five to look at them, advances it by five.
//! That is right: those five sheets exist and have numbers on them.
//!
//! # Why a name and not the document
//!
//! Keying this on the file would look automatic and would be wrong twice over.
//! Two receipt books printed from the same blank are two series; one series
//! reprinted from a document somebody edited is still one series. Only a person
//! knows which is which, so a person says.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Where every series has got to.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Counters {
    /// The next number each named series will use.
    #[serde(default)]
    pub next: BTreeMap<String, usize>,
}

#[derive(Debug, thiserror::Error)]
pub enum SeriesError {
    #[error(
        "'{0}' is not a name a series can have. Letters, digits, dashes and underscores, so it can be typed and cannot be mistaken for anything else."
    )]
    BadName(String),
    #[error("could not read {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error(
        "{path} holds where every numbered series has got to, and it cannot be read ({why}).\n    \
         Numbering from it now would start again at 1, on numbers that are already on paper. \
         Move that file aside and use --start-at to say where this run begins."
    )]
    Unreadable { path: PathBuf, why: String },
    #[error(
        "the '{name}' series was at {started_at} when this run began and is at {now} now, so \
         something else moved it while these sheets were being made.\n    They are numbered from \
         {started_at}, and the other run's may be the same numbers. Check both before printing, \
         and put the counter where it belongs with --start-at."
    )]
    MovedUnderneath {
        name: String,
        started_at: usize,
        now: usize,
    },
}

/// A name that can be typed and read back.
pub fn check_name(name: &str) -> Result<(), SeriesError> {
    let good = !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if good {
        Ok(())
    } else {
        Err(SeriesError::BadName(name.to_string()))
    }
}

/// Where the counters live.
pub fn path() -> PathBuf {
    crate::calibrate::home_dir().join("series.json")
}

/// Every series and where it has got to, and whether the file could be read.
///
/// The two cases have to be told apart. A file that is simply not there is the
/// ordinary one — nobody has used a series yet. A file that is there and cannot
/// be understood is a different thing entirely, and treating it as empty does
/// two bad turns at once: this run starts at 1, printing numbers that are
/// already on paper somewhere, and the save at the end writes an object holding
/// only this series, **deleting the counters of every other series on the
/// machine**. One bad file would take the lot.
pub fn read() -> Result<Counters, SeriesError> {
    let path = path();
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        // Never used, which is not a problem.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Counters::default()),
        Err(source) => return Err(SeriesError::Io { path, source }),
    };
    if text.trim().is_empty() {
        return Ok(Counters::default());
    }
    serde_json::from_str(&text).map_err(|source| SeriesError::Unreadable {
        path,
        why: source.to_string(),
    })
}

/// The same, where a file that cannot be read is treated as empty.
///
/// For the places that only want to *show* what is there — `doctor`'s list of
/// what Onionskin keeps. Never for anything that goes on to save, which would
/// take the unreadable file's contents away with it.
pub fn load() -> Counters {
    read().unwrap_or_default()
}

/// The number this series will use next, or 1 if it has never been used.
///
/// Returns an error rather than 1 when the file is there and unreadable, so
/// that a run does not quietly start again at a number already on paper.
pub fn next_for(name: &str) -> Result<usize, SeriesError> {
    Ok(read()?.next.get(name).copied().unwrap_or(1).max(1))
}

/// Say that a series has reached this number, so the next run starts there.
///
/// Called after the sheets exist, never before. Refuses outright if the file
/// cannot be read, because writing over it would take every other series with
/// it — and the sheets are already made, so the caller can say so and move the
/// file aside by hand.
pub fn reached(name: &str, next: usize) -> Result<(), SeriesError> {
    check_name(name)?;
    let mut counters = read()?;
    counters.next.insert(name.to_string(), next.max(1));
    save(&counters)
}

/// Advance a series only if it is still where this run left it.
///
/// Two runs of the same series at once both read the counter, both number their
/// sheets from it, and both write it — and there is no lock to prevent it,
/// because there is no lock this program could take that would also work on the
/// network share the counter might be sitting on. What it can do is notice: the
/// number this run started from is passed back in, and if the counter has moved
/// since, the sheets are already made and somebody has to be told rather than
/// have it written over in silence.
pub fn reached_from(name: &str, started_at: usize, next: usize) -> Result<(), SeriesError> {
    check_name(name)?;
    let mut counters = read()?;
    let now = counters.next.get(name).copied().unwrap_or(1).max(1);
    if now != started_at.max(1) {
        return Err(SeriesError::MovedUnderneath {
            name: name.to_string(),
            started_at: started_at.max(1),
            now,
        });
    }
    counters.next.insert(name.to_string(), next.max(1));
    save(&counters)
}

/// Put a series back to a number, or start one there.
pub fn start_at(name: &str, first: usize) -> Result<(), SeriesError> {
    reached(name, first)
}

/// Forget one. `false` if there was nothing to forget.
pub fn forget(name: &str) -> Result<bool, SeriesError> {
    check_name(name)?;
    let mut counters = load();
    let had = counters.next.remove(name).is_some();
    if had {
        save(&counters)?;
    }
    Ok(had)
}

/// Every series, in the order they would be listed.
pub fn all() -> Vec<(String, usize)> {
    load().next.into_iter().collect()
}

fn save(counters: &Counters) -> Result<(), SeriesError> {
    let path = path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| SeriesError::Io {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    let text = serde_json::to_string_pretty(counters).unwrap_or_else(|_| "{}".to_string());
    // Written through a temporary and renamed, so a failure halfway leaves the
    // counters that were there rather than an empty file where they were —
    // which for a receipt book would mean starting again at 1.
    let temporary = path.with_extension("json-tmp");
    std::fs::write(&temporary, text).map_err(|source| SeriesError::Io {
        path: temporary.clone(),
        source,
    })?;
    std::fs::rename(&temporary, &path).map_err(|source| {
        let _ = std::fs::remove_file(&temporary);
        SeriesError::Io {
            path: path.clone(),
            source,
        }
    })
}

/// What a run of sheets is numbered, given where to start.
///
/// Separated from the counters so it can be used with a plain `--start-at` and
/// no series at all, which is the case for somebody who knows exactly where
/// they are and does not want the program remembering anything.
pub fn numbers(first: usize, how_many: usize) -> std::ops::Range<usize> {
    let first = first.max(1);
    first..first + how_many
}

/// The line said after a run, so somebody can see what was used and what comes
/// next without going and looking at a file.
pub fn where_it_got_to(name: &str, used: std::ops::Range<usize>) -> String {
    if used.is_empty() {
        return format!("Series '{name}' is unchanged — nothing was numbered.");
    }
    format!(
        "Series '{name}': used {} to {}. The next run starts at {}.",
        used.start,
        used.end - 1,
        used.end
    )
}

#[cfg(test)]
#[path = "series/tests.rs"]
mod tests;
