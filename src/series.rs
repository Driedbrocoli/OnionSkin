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

/// Every series and where it has got to.
///
/// An unreadable file is an empty one rather than an error: this is a
/// convenience, and losing it costs somebody typing `--start-at` once. Refusing
/// to print at all because a small JSON file went bad would be worse.
pub fn load() -> Counters {
    std::fs::read_to_string(path())
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

/// The number this series will use next, or 1 if it has never been used.
pub fn next_for(name: &str) -> usize {
    load().next.get(name).copied().unwrap_or(1).max(1)
}

/// Say that a series has reached this number, so the next run starts there.
///
/// Called after the sheets exist, never before.
pub fn reached(name: &str, next: usize) -> Result<(), SeriesError> {
    check_name(name)?;
    let mut counters = load();
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
