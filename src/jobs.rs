//! The same job, done again next week.
//!
//! An office does the same thing to the same form every day. The paid stamp
//! goes at 150,40 in nine point; the received date goes under the third line;
//! the signature goes bottom right at forty millimetres wide. Working that out
//! once is fine. Working it out again every Monday, out of a note in somebody's
//! head or a shell history that has scrolled away, is how a person ends up
//! reprinting a box of letterhead.
//!
//! So a job is a thing that can be saved and run:
//!
//! ```text
//! onionskin write invoice.pdf --at '150,40:PAID {today}' --size 9 --save-as paid
//! onionskin job run paid invoice-4472.pdf
//! ```
//!
//! What is saved is the *recipe* — where the words go, how big, in what face —
//! and not the document or the words' values. The document changes every time,
//! which is the point; the recipe does not.
//!
//! # Filling in the blanks
//!
//! A saved job holds templates, the same `{name}` braces the CSV batch uses,
//! and for the same reason: what goes in them is different every time. They are
//! filled from `--set name=value`, and from a few the program knows by itself —
//! `{today}` most of all, because "today's date" is the single commonest thing
//! anybody stamps onto a piece of paper and nobody should be typing it in.
//!
//! A brace naming nothing is left visible rather than quietly blanked, exactly
//! as it is in a batch: a hundred letters reading `{date}` is a bad afternoon,
//! and a hundred reading nothing at all is worse, because they look finished.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// A saved recipe for adding things to a document.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Job {
    pub name: String,
    /// Placements as they were typed: `'150,40:PAID {today}'`.
    #[serde(default)]
    pub at: Vec<String>,
    /// Just after something already printed: `'Invoice no:{number}'`.
    #[serde(default)]
    pub after: Vec<String>,
    /// One line below it.
    #[serde(default)]
    pub below: Vec<String>,
    /// Pictures, as `'FILE:X,Y:SIZE'`.
    #[serde(default)]
    pub images: Vec<String>,
    #[serde(default = "eleven")]
    pub size_pt: f64,
    #[serde(default = "helvetica")]
    pub font: String,
    #[serde(default = "black")]
    pub colour: String,
    #[serde(default)]
    pub width_mm: Option<f64>,
    #[serde(default)]
    pub rotation_deg: f64,
    #[serde(default = "one_point_two")]
    pub leading: f64,
    /// Which page of the document, counted from 1.
    #[serde(default = "one")]
    pub page: usize,
    /// Anything the person wanted to remember about it.
    #[serde(default)]
    pub notes: String,
    /// Seconds since the epoch.
    #[serde(default)]
    pub created: u64,
}

fn eleven() -> f64 {
    11.0
}
fn helvetica() -> String {
    "Helvetica".to_string()
}
fn black() -> String {
    "#000000".to_string()
}
fn one_point_two() -> f64 {
    1.2
}
fn one() -> usize {
    1
}

#[derive(Debug, thiserror::Error)]
pub enum JobError {
    #[error(
        "'{0}' is not a name a job can have. Letters, digits, dashes and \
         underscores, so it can be typed and can be a file."
    )]
    BadName(String),
    #[error("there is no saved job called '{name}'{also}")]
    NoSuchJob { name: String, also: String },
    #[error("could not read {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("the saved job in {path} is unreadable: {source}")]
    Malformed {
        path: PathBuf,
        source: serde_json::Error,
    },
}

impl Job {
    /// Everything with a `{brace}` in it, which is everything that has to be
    /// filled in before this job can run.
    pub fn templates(&self) -> Vec<&String> {
        self.at
            .iter()
            .chain(self.after.iter())
            .chain(self.below.iter())
            .chain(self.images.iter())
            .collect()
    }

    /// The names in braces that this job expects to be given.
    ///
    /// Reported before anything is written, so "you did not say what {ref} is"
    /// arrives while somebody is still at the keyboard rather than as a
    /// hundred sheets of paper saying `{ref}`.
    pub fn wants(&self) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        for template in self.templates() {
            for name in braces_in(template) {
                if !known_without_asking(&name) && !names.contains(&name) {
                    names.push(name);
                }
            }
        }
        names
    }

    /// What is missing from what was given.
    pub fn missing(&self, given: &BTreeMap<String, String>) -> Vec<String> {
        self.wants()
            .into_iter()
            .filter(|name| !given.contains_key(name))
            .collect()
    }

    pub fn describe(&self) -> String {
        let mut lines = vec![format!("job '{}'", self.name)];
        for placement in &self.at {
            lines.push(format!("  at       {placement}"));
        }
        for anchor in &self.after {
            lines.push(format!("  after    {anchor}"));
        }
        for anchor in &self.below {
            lines.push(format!("  below    {anchor}"));
        }
        for image in &self.images {
            lines.push(format!("  picture  {image}"));
        }
        lines.push(format!(
            "  set in   {} at {} pt, {}",
            self.font, self.size_pt, self.colour
        ));
        if self.page != 1 {
            lines.push(format!("  on page  {}", self.page));
        }
        if !self.notes.is_empty() {
            lines.push(format!("  note     {}", self.notes));
        }
        let wants = self.wants();
        if !wants.is_empty() {
            lines.push(format!(
                "  needs    {}",
                wants
                    .iter()
                    .map(|name| format!("--set {name}=…"))
                    .collect::<Vec<_>>()
                    .join(" ")
            ));
        }
        lines.join("\n")
    }
}

/// The names Onionskin fills in without being told.
///
/// Today's date most of all: it is the commonest thing anybody stamps onto a
/// piece of paper, it is different every day, and a person typing it in by hand
/// is a person who will eventually stamp yesterday's.
pub fn known_without_asking(name: &str) -> bool {
    matches!(name, "today" | "date" | "year" | "month" | "day")
}

/// What those names stand for, at the given moment.
pub fn what_the_day_is(at: u64) -> BTreeMap<String, String> {
    let days = (at / 86_400) as i64;
    let (year, month, day) = crate::apt::civil_from_days(days);
    let mut known = BTreeMap::new();
    let date = format!("{year:04}-{month:02}-{day:02}");
    known.insert("today".to_string(), date.clone());
    known.insert("date".to_string(), date);
    known.insert("year".to_string(), format!("{year:04}"));
    known.insert("month".to_string(), format!("{month:02}"));
    known.insert("day".to_string(), format!("{day:02}"));
    known
}

/// Everything a template will be filled from: what was given, over what the
/// program knows by itself.
///
/// Given wins, so somebody stamping yesterday's post with yesterday's date can
/// say `--set today=2026-07-26` and be believed.
pub fn values(given: &BTreeMap<String, String>, at: u64) -> crate::rows::Row {
    let mut values = what_the_day_is(at);
    for (name, value) in given {
        values.insert(name.clone(), value.clone());
    }
    crate::rows::Row { values, number: 1 }
}

/// The names inside `{}` in a template, in the order they appear.
fn braces_in(template: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else {
            break;
        };
        let name = &after[..close];
        if !name.is_empty() && !name.contains('{') {
            found.push(name.to_string());
        }
        rest = &after[close + 1..];
    }
    found
}

/// Where saved jobs live.
pub fn dir() -> PathBuf {
    crate::calibrate::home_dir().join("jobs")
}

pub fn path_of(name: &str) -> PathBuf {
    dir().join(format!("{name}.json"))
}

/// A name that can be typed and can be a file.
///
/// Refused rather than cleaned up: a job silently saved under a different name
/// from the one somebody typed is a job they cannot find again.
pub fn check_name(name: &str) -> Result<(), JobError> {
    let good = !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if good {
        Ok(())
    } else {
        Err(JobError::BadName(name.to_string()))
    }
}

pub fn save(job: &Job) -> Result<PathBuf, JobError> {
    check_name(&job.name)?;
    let dir = dir();
    std::fs::create_dir_all(&dir).map_err(|source| JobError::Io {
        path: dir.clone(),
        source,
    })?;
    let path = path_of(&job.name);
    let text = serde_json::to_string_pretty(job).map_err(|source| JobError::Malformed {
        path: path.clone(),
        source,
    })?;
    // Written through a temporary and renamed, so a failure halfway leaves the
    // job that was there rather than an empty file where it was.
    let temporary = path.with_extension("json-tmp");
    std::fs::write(&temporary, text).map_err(|source| JobError::Io {
        path: temporary.clone(),
        source,
    })?;
    std::fs::rename(&temporary, &path).map_err(|source| {
        let _ = std::fs::remove_file(&temporary);
        JobError::Io {
            path: path.clone(),
            source,
        }
    })?;
    Ok(path)
}

pub fn load(name: &str) -> Result<Job, JobError> {
    check_name(name)?;
    let path = path_of(name);
    let text = std::fs::read_to_string(&path).map_err(|_| JobError::NoSuchJob {
        name: name.to_string(),
        also: also_there(name),
    })?;
    serde_json::from_str(&text).map_err(|source| JobError::Malformed { path, source })
}

/// Every saved job, by name, in the order they would be listed.
pub fn list() -> Vec<Job> {
    let Ok(entries) = std::fs::read_dir(dir()) else {
        return Vec::new();
    };
    let mut jobs: Vec<Job> = entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().extension().and_then(|e| e.to_str()) == Some("json"))
        .filter_map(|entry| std::fs::read_to_string(entry.path()).ok())
        .filter_map(|text| serde_json::from_str::<Job>(&text).ok())
        .collect();
    jobs.sort_by(|a, b| a.name.cmp(&b.name));
    jobs
}

/// Delete one. `false` if there was nothing to delete.
pub fn delete(name: &str) -> Result<bool, JobError> {
    check_name(name)?;
    let path = path_of(name);
    if !path.is_file() {
        return Ok(false);
    }
    std::fs::remove_file(&path)
        .map(|_| true)
        .map_err(|source| JobError::Io { path, source })
}

/// What else is saved, for the message when a name is not found.
///
/// A list of the names that *do* exist is worth more than the fact that this
/// one does not, because the usual cause is a typo or a half-remembered name.
fn also_there(_wanted: &str) -> String {
    let names: Vec<String> = list().into_iter().map(|job| job.name).collect();
    if names.is_empty() {
        ".\n    Nothing has been saved yet. Add --save-as NAME to a write or \
         draw command to keep one."
            .to_string()
    } else {
        format!(".\n    There is: {}", names.join(", "))
    }
}

#[cfg(test)]
#[path = "jobs/tests.rs"]
mod tests;
