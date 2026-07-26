//! The few things worth remembering between one run and the next.
//!
//! Not configuration. Nothing here changes what Onionskin does — it is where
//! you were last time, so that the file browser opens in the folder your
//! documents are in rather than wherever the program happened to be started
//! from, and the window comes back to the screen you were using.
//!
//! # Why it is allowed to fail quietly
//!
//! A settings file that cannot be read or written must never stop the program.
//! Somebody with a read-only home directory, or a file left behind by a newer
//! version, should get a program that opens where it always did — not one that
//! refuses to start. So every operation here returns something usable and
//! nothing here returns an error.
//!
//! It holds no document, no path to anything private beyond a folder name, and
//! no secret. It is still written owner-only, because the folder somebody keeps
//! their work in is nobody else's business either.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// What is remembered.
///
/// Every field has a default and every field is optional in the file, so a
/// settings file written by an older or newer version still loads.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// The folder the file browser last showed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_folder: Option<PathBuf>,
    /// The folder a result was last written to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_output_folder: Option<PathBuf>,
    /// Which of the window's screens was open.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_screen: Option<String>,
    /// Extra folders to look in for fonts.
    ///
    /// Onionskin already looks where the system keeps fonts, but a word
    /// processor does not always put its own there — LibreOffice ships a set
    /// inside its installation, and somebody who bought a typeface usually
    /// keeps it in a folder of their own. Naming that folder once means the
    /// words Onionskin adds can be set in the same face as the page they are
    /// being added to.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub font_folders: Vec<PathBuf>,
}

/// Where the file lives.
fn path() -> PathBuf {
    crate::calibrate::home_dir().join("settings.json")
}

/// Read what was remembered, or nothing if there is nothing to read.
pub fn load() -> Settings {
    let Ok(text) = std::fs::read_to_string(path()) else {
        return Settings::default();
    };
    // A file from a version that wrote something else is not an error worth
    // reporting to somebody who only wanted to open a document.
    serde_json::from_str(&text).unwrap_or_default()
}

/// Write it back. Silent on failure, by design.
pub fn save(settings: &Settings) {
    let path = path();
    if let Some(parent) = path.parent() {
        if std::fs::create_dir_all(parent).is_err() {
            return;
        }
        crate::render::restrict(parent);
    }
    let Ok(text) = serde_json::to_string_pretty(settings) else {
        return;
    };
    // Through a temporary file and renamed into place, so an interrupted write
    // leaves the old settings rather than an empty file.
    let temporary = path.with_extension("json.new");
    if std::fs::write(&temporary, text).is_err() {
        return;
    }
    restrict_file(&temporary);
    if std::fs::rename(&temporary, &path).is_err() {
        let _ = std::fs::remove_file(&temporary);
    }
}

/// Change one thing and write it back.
///
/// The read-modify-write is deliberate: two Onionskins running at once should
/// not have one of them throw away what the other remembered.
pub fn remember(change: impl FnOnce(&mut Settings)) {
    let mut settings = load();
    let before = settings.clone();
    change(&mut settings);
    if settings != before {
        save(&settings);
    }
}

/// Remember the folder something was chosen from.
pub fn remember_folder(chosen: &Path) {
    let folder = folder_of(chosen);
    remember(|settings| settings.last_folder = folder.clone());
}

/// Remember the folder something was written to.
pub fn remember_output_folder(written: &Path) {
    let folder = folder_of(written);
    remember(|settings| settings.last_output_folder = folder.clone());
}

/// Remember a folder to look in for fonts.
///
/// Returns whether it was added, so a caller can say "already there" rather
/// than reporting success twice for the same folder. The path is resolved
/// first, so the same folder named two ways is stored once.
pub fn add_font_folder(folder: &Path) -> bool {
    let resolved = folder.canonicalize().unwrap_or_else(|_| folder.to_path_buf());
    let mut added = false;
    remember(|settings| {
        if !settings.font_folders.contains(&resolved) {
            settings.font_folders.push(resolved.clone());
            added = true;
        }
    });
    added
}

/// Stop looking in a folder for fonts. Returns whether it was there.
pub fn forget_font_folder(folder: &Path) -> bool {
    let resolved = folder.canonicalize().unwrap_or_else(|_| folder.to_path_buf());
    let mut removed = false;
    remember(|settings| {
        let before = settings.font_folders.len();
        settings
            .font_folders
            .retain(|kept| kept != &resolved && kept != folder);
        removed = settings.font_folders.len() != before;
    });
    removed
}

/// The extra font folders, minus any that have since been deleted.
///
/// Filtered on the way out rather than pruned on load: a folder on a drive
/// that is not plugged in today should still be there tomorrow.
pub fn font_folders() -> Vec<PathBuf> {
    load()
        .font_folders
        .into_iter()
        .filter(|folder| folder.is_dir())
        .collect()
}

/// The folder a path is in, if it is a folder that exists.
fn folder_of(path: &Path) -> Option<PathBuf> {
    let folder = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent()?.to_path_buf()
    };
    folder.is_dir().then_some(folder)
}

/// Where a file browser should open, given what it was last pointed at.
///
/// In order of how likely it is to be right: where this control was pointed
/// before, where anything was last chosen from, and then the folder the program
/// was started in.
pub fn start_in(hint: Option<&Path>) -> PathBuf {
    if let Some(folder) = hint.and_then(folder_of) {
        return folder;
    }
    if let Some(folder) = load().last_folder.filter(|path| path.is_dir()) {
        return folder;
    }
    std::env::current_dir()
        .ok()
        .filter(|path| path.is_dir())
        .unwrap_or_else(crate::install::home)
}

fn restrict_file(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
}

#[cfg(test)]
mod tests;
