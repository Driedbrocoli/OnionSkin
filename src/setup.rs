//! Carrying a machine's setup to the next machine.
//!
//! Somebody sets Onionskin up once. They measure their printer against a target
//! sheet and save a calibration profile; they work out where the paid stamp
//! goes on the invoice and save it as a job; they set the paper size and the
//! resolution their office uses. That is an afternoon's work, and it is worth
//! an afternoon, because everything after it is two clicks.
//!
//! Then the second person in the office installs Onionskin, and none of it is
//! there. So they do the afternoon again, slightly differently, and now the
//! two machines put the stamp in two places. Then a third. Then the person who
//! worked it out in the first place leaves, and what they knew leaves with
//! them — it was never written down anywhere, because it was never in a form
//! that could be handed over.
//!
//! ```text
//! onionskin setup save our-office.json     # on the machine that is set up
//! onionskin setup use our-office.json      # on every other machine
//! ```
//!
//! # What travels, and what must not
//!
//! Three things travel, and they are the three that took work to arrive at:
//! the **calibration profiles**, the **saved jobs**, and the **defaults**
//! (with the font folders, which are settings about what to use rather than
//! about where somebody last browsed).
//!
//! The rest is deliberately left behind, and the reasons are worth stating
//! because each is a way this feature could have done harm:
//!
//! **Numbered series do not travel.** This is the important one. A series
//! remembers that the receipt book has reached 400. Copy that to a second
//! machine and both will print 401 next — two receipts with the same number
//! on them, which is the one thing a receipt book must never contain, arrived
//! at by a feature whose whole purpose was to be helpful. A counter belongs to
//! one machine, or it is not a counter.
//!
//! **The history does not travel.** It is a record of what *that* machine
//! printed and when, naming the files it was done to. Copying it to a
//! colleague's computer would hand over a list of every document somebody
//! worked on, which is a far more sensitive thing than any setting.
//!
//! **Where the file browser last looked does not travel.** Those are paths on
//! somebody else's disk. Carried across, they send the browser somewhere that
//! does not exist, on every machine but the one they came from.
//!
//! # A name that already exists is kept
//!
//! Somebody may have their own job called `paid`, worked out for their own
//! form. The office setup arriving and quietly replacing it is how a person
//! comes to print the wrong thing on a document they have printed correctly a
//! hundred times before. So a clash keeps what is already there and says so,
//! and `--replace` is how somebody says they mean it.
//!
//! # It is a file a person can read
//!
//! Plain JSON, not an archive. Somebody handed a file that is about to change
//! how their computer behaves should be able to open it and see exactly what
//! is in it, and an office that keeps its setup in a shared folder should be
//! able to see what changed between one version and the next.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// What version of this format the file holds.
///
/// Written so that a file from a later Onionskin can be recognised as such and
/// refused by name, rather than half-read into a setup nobody asked for.
pub const VERSION: u32 = 1;

/// A machine's setup, as it travels.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Setup {
    pub version: u32,
    /// Seconds since the epoch, so somebody can tell one of these from another.
    #[serde(default)]
    pub made_at: u64,
    /// Which Onionskin wrote it. Not who, and not which machine: this file gets
    /// put in shared folders and emailed about, and a person's name or their
    /// computer's is not something to write into it without being asked.
    #[serde(default)]
    pub made_by: String,
    /// Anything the office wants to say about it — which printer, which forms.
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub defaults: crate::settings::Defaults,
    #[serde(default)]
    pub font_folders: Vec<PathBuf>,
    #[serde(default)]
    pub jobs: Vec<crate::jobs::Job>,
    #[serde(default)]
    pub profiles: Vec<crate::calibrate::Profile>,
}

#[derive(Debug, thiserror::Error)]
pub enum SetupError {
    #[error("could not read {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error(
        "{path} is not an Onionskin setup file ({why}).\n    \
         A setup file is the one written by `onionskin setup save`."
    )]
    NotASetup { path: PathBuf, why: String },
    #[error(
        "{path} was written by a later version of Onionskin (it says version {found}, and this \
         one understands {understood}).\n    \
         Update Onionskin on this machine, or write the setup again from a machine running this \
         version."
    )]
    FromTheFuture {
        path: PathBuf,
        found: u32,
        understood: u32,
    },
}

/// This machine's setup, as it stands.
pub fn gather() -> Setup {
    let settings = crate::settings::load();
    Setup {
        version: VERSION,
        made_at: crate::history::now(),
        made_by: format!("Onionskin {}", env!("CARGO_PKG_VERSION")),
        notes: String::new(),
        defaults: settings.defaults,
        font_folders: settings.font_folders,
        jobs: crate::jobs::list(),
        profiles: crate::calibrate::list_profiles().unwrap_or_default(),
    }
}

impl Setup {
    /// Whether there is anything in it worth carrying.
    pub fn is_empty(&self) -> bool {
        self.jobs.is_empty()
            && self.profiles.is_empty()
            && self.font_folders.is_empty()
            && self.defaults == crate::settings::Defaults::default()
    }

    /// What is in it, in the order somebody would want to be told.
    ///
    /// Said before it is written and again before it is applied, because a file
    /// that is about to change how a computer behaves should say what it will
    /// change before it changes it.
    pub fn describe(&self) -> String {
        let mut lines = Vec::new();
        if !self.profiles.is_empty() {
            lines.push(format!(
                "  calibration  {} — {}",
                self.profiles.len(),
                self.profiles
                    .iter()
                    .map(|p| p.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if !self.jobs.is_empty() {
            lines.push(format!(
                "  jobs         {} — {}",
                self.jobs.len(),
                self.jobs
                    .iter()
                    .map(|j| j.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        // The ones actually chosen. `each` lists every setting there is,
        // most of them unset, so its length would report "14 chosen" about a
        // file holding one.
        let chosen = self
            .defaults
            .each()
            .into_iter()
            .filter(|(_, value, _)| value.is_some())
            .count();
        if chosen > 0 {
            lines.push(format!(
                "  settings     {chosen} chosen: {}",
                self.defaults
                    .each()
                    .into_iter()
                    .filter_map(|(name, value, _)| value.map(|v| format!("{name}={v}")))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if !self.font_folders.is_empty() {
            lines.push(format!("  font folders {}", self.font_folders.len()));
        }
        if lines.is_empty() {
            return "Nothing in it: no calibration, no saved jobs, no settings of your own."
                .to_string();
        }
        lines.join("\n")
    }
}

/// Write a setup out.
pub fn write(path: &Path, setup: &Setup) -> Result<(), SetupError> {
    let text = serde_json::to_string_pretty(setup).map_err(|e| SetupError::NotASetup {
        path: path.to_path_buf(),
        why: e.to_string(),
    })?;
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|source| SetupError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
    }
    std::fs::write(path, text).map_err(|source| SetupError::Io {
        path: path.to_path_buf(),
        source,
    })
}

/// Read one back.
pub fn read(path: &Path) -> Result<Setup, SetupError> {
    let text = std::fs::read_to_string(path).map_err(|source| SetupError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let setup: Setup = serde_json::from_str(&text).map_err(|e| SetupError::NotASetup {
        path: path.to_path_buf(),
        why: e.to_string(),
    })?;
    // Checked after parsing rather than before, so a file that is not JSON at
    // all is told apart from one that is a setup from a later version.
    if setup.version > VERSION {
        return Err(SetupError::FromTheFuture {
            path: path.to_path_buf(),
            found: setup.version,
            understood: VERSION,
        });
    }
    Ok(setup)
}

/// What to do about a name that is already in use here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Clashing {
    /// Keep what this machine already has. The default: somebody's own job
    /// called `paid`, worked out for their own form, must not be quietly
    /// replaced by the office one.
    Keep,
    /// Take the arriving one.
    Replace,
}

/// What applying a setup did, or would do.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Applied {
    pub jobs_added: Vec<String>,
    pub jobs_replaced: Vec<String>,
    pub jobs_kept: Vec<String>,
    pub profiles_added: Vec<String>,
    pub profiles_replaced: Vec<String>,
    pub profiles_kept: Vec<String>,
    pub settings_changed: bool,
    pub font_folders_added: usize,
    /// Anything that went wrong for one item without stopping the rest.
    pub trouble: Vec<String>,
}

impl Applied {
    pub fn nothing_happened(&self) -> bool {
        self.jobs_added.is_empty()
            && self.jobs_replaced.is_empty()
            && self.profiles_added.is_empty()
            && self.profiles_replaced.is_empty()
            && !self.settings_changed
            && self.font_folders_added == 0
    }

    /// What happened, in the order somebody would want to be told.
    pub fn describe(&self) -> String {
        let mut lines = Vec::new();
        let say = |what: &str, names: &[String]| -> Option<String> {
            (!names.is_empty()).then(|| format!("  {what} {}", names.join(", ")))
        };
        lines.extend(say("added job    ", &self.jobs_added));
        lines.extend(say("replaced job ", &self.jobs_replaced));
        lines.extend(say("added profile", &self.profiles_added));
        lines.extend(say("replaced prof", &self.profiles_replaced));
        if self.settings_changed {
            lines.push("  settings      taken from the file".to_string());
        }
        if self.font_folders_added > 0 {
            lines.push(format!("  font folders  {} added", self.font_folders_added));
        }
        // The kept ones are said last and said plainly. Somebody who handed a
        // colleague a setup file and finds their job is not there needs to
        // know it was not taken, and why, rather than wondering.
        lines.extend(say("kept yours   ", &self.jobs_kept));
        lines.extend(say("kept yours   ", &self.profiles_kept));
        for said in &self.trouble {
            lines.push(format!("  could not     {said}"));
        }
        if lines.is_empty() {
            return "Nothing to take: this machine already has everything in it.".to_string();
        }
        lines.join("\n")
    }
}

/// Work out what applying a setup would do, without doing any of it.
///
/// Separated from the doing so that `--dry-run` reports exactly what the real
/// run would, from the same code — a rehearsal that is worked out by different
/// arithmetic from the performance is not a rehearsal.
pub fn what_it_would_do(setup: &Setup, clashing: Clashing) -> Applied {
    let mine_jobs: Vec<String> = crate::jobs::list().into_iter().map(|j| j.name).collect();
    let mine_profiles: Vec<String> = crate::calibrate::list_profiles()
        .unwrap_or_default()
        .into_iter()
        .map(|p| p.name)
        .collect();
    let settings = crate::settings::load();

    let mut applied = Applied::default();
    for job in &setup.jobs {
        match (mine_jobs.contains(&job.name), clashing) {
            (false, _) => applied.jobs_added.push(job.name.clone()),
            (true, Clashing::Replace) => applied.jobs_replaced.push(job.name.clone()),
            (true, Clashing::Keep) => applied.jobs_kept.push(job.name.clone()),
        }
    }
    for profile in &setup.profiles {
        match (mine_profiles.contains(&profile.name), clashing) {
            (false, _) => applied.profiles_added.push(profile.name.clone()),
            (true, Clashing::Replace) => applied.profiles_replaced.push(profile.name.clone()),
            (true, Clashing::Keep) => applied.profiles_kept.push(profile.name.clone()),
        }
    }
    applied.settings_changed = setup.defaults != crate::settings::Defaults::default()
        && setup.defaults != settings.defaults;
    applied.font_folders_added = setup
        .font_folders
        .iter()
        .filter(|folder| !settings.font_folders.contains(folder))
        .count();
    applied
}

/// Take a setup onto this machine.
///
/// One item failing does not stop the rest: a profile that will not save
/// because its name is no longer a legal one should not cost somebody the six
/// jobs that would have saved perfectly well. What failed is reported.
pub fn apply(setup: &Setup, clashing: Clashing) -> Applied {
    let mut applied = what_it_would_do(setup, clashing);
    let taking_jobs: Vec<&crate::jobs::Job> = setup
        .jobs
        .iter()
        .filter(|job| {
            applied.jobs_added.contains(&job.name) || applied.jobs_replaced.contains(&job.name)
        })
        .collect();
    for job in taking_jobs {
        if let Err(why) = crate::jobs::save(job) {
            applied
                .trouble
                .push(format!("save the job '{}': {why}", job.name));
            applied.jobs_added.retain(|name| name != &job.name);
            applied.jobs_replaced.retain(|name| name != &job.name);
        }
    }

    let taking_profiles: Vec<&crate::calibrate::Profile> = setup
        .profiles
        .iter()
        .filter(|p| {
            applied.profiles_added.contains(&p.name) || applied.profiles_replaced.contains(&p.name)
        })
        .collect();
    for profile in taking_profiles {
        if let Err(why) = crate::calibrate::save_profile(profile) {
            applied
                .trouble
                .push(format!("save the calibration '{}': {why}", profile.name));
            applied.profiles_added.retain(|name| name != &profile.name);
            applied
                .profiles_replaced
                .retain(|name| name != &profile.name);
        }
    }

    if applied.settings_changed || applied.font_folders_added > 0 {
        let mut settings = crate::settings::load();
        if applied.settings_changed {
            settings.defaults = setup.defaults.clone();
        }
        // Added to, never replaced: a font folder on this machine is a real
        // folder on this machine, and the arriving list names folders on
        // somebody else's. Both may be right.
        //
        // Which ones are new is noted before the write, because afterwards
        // there is no telling them from the ones that were already here — and
        // counting those too would report six folders added to a machine that
        // gained none.
        let new_folders: Vec<PathBuf> = setup
            .font_folders
            .iter()
            .filter(|folder| !settings.font_folders.contains(folder))
            .cloned()
            .collect();
        settings.font_folders.extend(new_folders.iter().cloned());
        // `settings::save` is silent by design — somebody who only wanted to
        // write a delta should not have it fail because a directory in their
        // home is read-only. So it is read back afterwards and the report says
        // what really landed, rather than what was asked for.
        crate::settings::save(&settings);
        let landed = crate::settings::load();
        if applied.settings_changed && landed.defaults != setup.defaults {
            applied
                .trouble
                .push("save the settings — they are unchanged on this machine".to_string());
            applied.settings_changed = false;
        }
        applied.font_folders_added = new_folders
            .iter()
            .filter(|folder| landed.font_folders.contains(folder))
            .count();
    }

    applied
}

/// What is deliberately left behind, and why — for the command to print, so
/// that somebody who expected their numbering to travel finds out here rather
/// than from two receipts carrying the same number.
pub const WHAT_STAYS_BEHIND: &str = "\
Not carried, on purpose:
  numbered series   A counter that reached 400 would make both machines print
                    401 next, and two receipts with the same number on them is
                    the one thing a receipt book must never contain.
  the history       A record of what this machine printed, naming the files. It
                    is nobody else's business.
  remembered places The folders this machine last browsed, which are paths on
                    this disk and nowhere else.";

#[cfg(test)]
#[path = "setup/tests.rs"]
mod tests;
