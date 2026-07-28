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
    /// The defaults this person prefers, where they differ from Onionskin's.
    ///
    /// Held apart from the rest because these change what the program *does*,
    /// and the rest only changes where it starts looking. A setting that is
    /// absent means "whatever Onionskin thinks", which is why every one of
    /// them is optional rather than being written out with its default in it:
    /// a default that has been copied into a file stops tracking the default.
    #[serde(default, skip_serializing_if = "Defaults::is_empty")]
    pub defaults: Defaults,
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

/// What somebody wants instead of what Onionskin would have chosen.
///
/// Every field is optional and absent means "use Onionskin's". Anything given
/// on the command line still wins over anything here — the order is always the
/// flag, then this, then the built-in default, and it is never the other way
/// about. Somebody who sets a preference has not given up the ability to
/// override it for one run, which would be a strange kind of preference.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Defaults {
    /// Rendering resolution, in dots per inch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dpi: Option<f64>,
    /// How close to the edge of the paper is worth a warning, in millimetres.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub margin_mm: Option<f64>,
    /// "raster" or "vector".
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    /// Draw a box round every change without being asked each time.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outline: Option<bool>,
    /// What colour those boxes are.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outline_colour: Option<String>,
    /// The calibration profile to use when none is named.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    /// The paper to assume for a scan.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub page: Option<String>,
    /// The printer to send to when none is named.
    ///
    /// An office has one printer and types its address every time. It is the
    /// longest thing anybody types at this program — `ipp://printer.local/ipp/
    /// print` — and it is the same every day.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub printer: Option<String>,
    /// The scanner to fetch from when none is named, for the same reason.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scanner: Option<String>,
}

/// A number as somebody would write it: 300 rather than 300.
fn tidy(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        format!("{value}")
    }
}

impl Defaults {
    /// Nothing set at all, in which case it is left out of the file entirely.
    pub fn is_empty(&self) -> bool {
        *self == Defaults::default()
    }

    /// Every setting there is, as name and current value.
    ///
    /// Listed from one place so that `config show`, `config set` and the help
    /// cannot come to disagree about what exists.
    pub fn each(&self) -> Vec<(&'static str, Option<String>, &'static str)> {
        vec![
            (
                "dpi",
                self.dpi.map(tidy),
                "rendering resolution, 50 to 1200",
            ),
            (
                "margin",
                self.margin_mm.map(tidy),
                "warn about ink closer than this to an edge, in mm",
            ),
            ("mode", self.mode.clone(), "raster or vector"),
            (
                "outline",
                self.outline.map(|v| v.to_string()),
                "draw a box round every change: yes or no",
            ),
            (
                "outline-colour",
                self.outline_colour.clone(),
                "red, blue, green, orange, magenta, black, or R,G,B",
            ),
            (
                "profile",
                self.profile.clone(),
                "the calibration profile to use when none is named",
            ),
            ("page", self.page.clone(), "the paper to assume for a scan"),
            (
                "printer",
                self.printer.clone(),
                "the printer to send to when none is named",
            ),
            (
                "scanner",
                self.scanner.clone(),
                "the scanner to fetch from when none is named",
            ),
        ]
    }
}

/// Change one setting, or take it away with `None`.
///
/// Returns what is wrong with the value, if anything. Checked here rather than
/// where it is used, so that a bad number is refused at the moment somebody
/// types it — not silently stored and then met as an error on some later run
/// they have forgotten this by.
pub fn set_default(name: &str, value: Option<&str>) -> Result<(), String> {
    let name = name.trim().to_ascii_lowercase();
    let number = |what: &str, low: f64, high: f64| -> Result<Option<f64>, String> {
        let Some(text) = value else { return Ok(None) };
        let parsed: f64 = text
            .trim()
            .parse()
            .map_err(|_| format!("'{text}' is not a number"))?;
        if !parsed.is_finite() || parsed < low || parsed > high {
            return Err(format!(
                "{what} must be between {} and {}",
                tidy(low),
                tidy(high)
            ));
        }
        Ok(Some(parsed))
    };

    match name.as_str() {
        "dpi" => {
            let dpi = number("dpi", 50.0, 1200.0)?;
            remember(|s| s.defaults.dpi = dpi);
        }
        "margin" => {
            let margin = number("the margin", 0.0, 40.0)?;
            remember(|s| s.defaults.margin_mm = margin);
        }
        "mode" => {
            let mode = match value {
                None => None,
                Some(text) => match text.trim().to_ascii_lowercase().as_str() {
                    mode @ ("raster" | "vector") => Some(mode.to_string()),
                    other => return Err(format!("mode is 'raster' or 'vector', not '{other}'")),
                },
            };
            remember(|s| s.defaults.mode = mode);
        }
        "outline" => {
            let outline = match value {
                None => None,
                Some(text) => match text.trim().to_ascii_lowercase().as_str() {
                    "yes" | "true" | "on" | "1" => Some(true),
                    "no" | "false" | "off" | "0" => Some(false),
                    other => return Err(format!("outline is 'yes' or 'no', not '{other}'")),
                },
            };
            remember(|s| s.defaults.outline = outline);
        }
        "outline-colour" | "outline-color" => {
            let colour = value.map(|text| text.trim().to_string());
            remember(|s| s.defaults.outline_colour = colour);
        }
        "profile" => {
            let profile = value.map(|text| text.trim().to_string());
            remember(|s| s.defaults.profile = profile);
        }
        "page" => {
            let page = value.map(|text| text.trim().to_string());
            remember(|s| s.defaults.page = page);
        }
        // Addresses are kept as they were typed. `send` and `fetch` already
        // know what a usable one looks like and say so in full when it is
        // wrong; checking here as well would mean two answers to one question,
        // and the one further from the device would be the worse of them.
        "printer" => {
            let printer = value.map(|text| text.trim().to_string());
            remember(|s| s.defaults.printer = printer);
        }
        "scanner" => {
            let scanner = value.map(|text| text.trim().to_string());
            remember(|s| s.defaults.scanner = scanner);
        }
        other => {
            let known: Vec<&str> = Defaults::default()
                .each()
                .into_iter()
                .map(|(name, _, _)| name)
                .collect();
            return Err(format!(
                "there is no setting called '{other}'. There is: {}",
                known.join(", ")
            ));
        }
    }
    Ok(())
}

/// Forget every preference, going back to what Onionskin would have chosen.
pub fn clear_defaults() {
    remember(|s| s.defaults = Defaults::default());
}

/// Where the file lives.
/// Where the settings file lives. Public so `doctor` can say where it is —
/// a program keeping a file in a hidden folder ought to be willing to name it.
pub fn path() -> PathBuf {
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
    let resolved = folder
        .canonicalize()
        .unwrap_or_else(|_| folder.to_path_buf());
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
    let resolved = folder
        .canonicalize()
        .unwrap_or_else(|_| folder.to_path_buf());
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

impl Defaults {
    /// These settings, laid over Onionskin's own answers.
    ///
    /// The one place saved defaults become a run's options, so the window and
    /// the command line cannot apply them differently — a saved job is meant
    /// to behave the same wherever it is run from, and "the window ignored
    /// your calibration profile" is exactly the kind of difference nobody
    /// finds until a sheet comes out two millimetres off.
    ///
    /// A flag still beats these: callers apply theirs afterwards.
    ///
    /// `outline` is deliberately left out. Boxes round the changes are drawn
    /// by the raster delta writer, which only the compare-two-documents path
    /// uses, so honouring it here would produce nothing — and a setting that
    /// quietly does nothing is worse than one that plainly does not apply.
    pub fn over(&self, mut options: crate::pipeline::Options) -> crate::pipeline::Options {
        if let Some(dpi) = self.dpi {
            options.dpi = dpi;
        }
        if let Some(margin) = self.margin_mm {
            options.margin_mm = margin;
        }
        if let Some(mode) = self.mode.as_deref().and_then(crate::pipeline::Mode::parse) {
            options.mode = mode;
        }
        if let Some(profile) = self.profile.clone() {
            options.profile = Some(profile);
        }
        options
    }
}

#[cfg(test)]
mod tests;
