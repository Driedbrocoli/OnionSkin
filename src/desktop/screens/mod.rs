//! The things the window can be showing.
//!
//! One module per screen, each keeping its own state and drawing itself. They
//! do not know about one another, and none of them does slow work on the thread
//! that draws — everything heavy goes through [`Jobs`].

pub mod blanks;
pub mod calibrate;
pub mod compare;
pub mod devices;
pub mod doctor;
pub mod document;
pub mod draw;
pub mod history;
pub mod jobs;
pub mod labels;
pub mod merge;
pub mod proof;
pub mod read;
pub mod scan;
pub mod verify;

use eframe::egui;

use super::job::Jobs;
use super::preview::Previews;

/// Everything a screen is given to draw itself with.
///
/// Passed as one struct rather than five arguments so that adding something a
/// screen needs later does not mean editing every screen that does not.
pub struct Room<'a> {
    pub picker: &'a mut crate::picker::Picker,
    pub ui: &'a mut egui::Ui,
    pub jobs: &'a mut Jobs,
    pub previews: &'a mut Previews,
    /// Files dropped onto the window this frame, waiting to be claimed.
    ///
    /// A control takes the first one it can use and leaves the rest, so
    /// dropping two documents on the comparing screen fills both slots in the
    /// order they are drawn — which is the order they are asked for.
    pub dropped: &'a mut Vec<std::path::PathBuf>,
}

/// A name beside another file, which is where somebody looks for what came out.
///
/// Here rather than in each screen because four of them want it and three had
/// already written it out identically. Two spellings of "beside" is how one
/// screen comes to put its answer somewhere the person is not looking.
pub fn beside(source: &std::path::Path, tail: &str) -> std::path::PathBuf {
    let stem = source
        .file_stem()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "onionskin".to_string());
    source
        .parent()
        .unwrap_or(std::path::Path::new(""))
        .join(format!("{stem}{tail}.pdf"))
}

/// The same file, whatever it has been called on the way there.
///
/// Falls back to comparing the paths as written when either cannot be resolved,
/// which is the case that matters: a file about to be created has no canonical
/// form yet, and that is exactly when a screen is asking.
pub fn same_file(a: &std::path::Path, b: &std::path::Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// Which screen the window is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Compare,
    Scan,
    Document,
    Draw,
    Read,
    Blanks,
    Proof,
    Merge,
    Labels,
    Jobs,
    Verify,
    History,
    Devices,
    Calibrate,
    Doctor,
}

impl Screen {
    /// In the order they appear down the side, which is roughly the order
    /// somebody meets them: the thing the program is for comes first.
    pub const ALL: &'static [Screen] = &[
        Screen::Compare,
        Screen::Scan,
        Screen::Document,
        Screen::Draw,
        Screen::Read,
        Screen::Blanks,
        Screen::Proof,
        Screen::Merge,
        Screen::Labels,
        Screen::Jobs,
        Screen::Verify,
        Screen::History,
        Screen::Devices,
        Screen::Calibrate,
        Screen::Doctor,
    ];

    /// A short name to write in the settings file — the visible one is a
    /// sentence, and a sentence in a settings file is a sentence somebody will
    /// eventually reword.
    pub fn key(&self) -> &'static str {
        match self {
            Screen::Compare => "compare",
            Screen::Scan => "scan",
            Screen::Document => "document",
            Screen::Draw => "draw",
            Screen::Read => "read",
            Screen::Blanks => "blanks",
            Screen::Proof => "proof",
            Screen::Merge => "merge",
            Screen::Labels => "labels",
            Screen::Jobs => "jobs",
            Screen::Verify => "verify",
            Screen::History => "history",
            Screen::Devices => "devices",
            Screen::Calibrate => "calibrate",
            Screen::Doctor => "doctor",
        }
    }

    /// The screen a key names, or nothing if it names none.
    pub fn from_key(key: &str) -> Option<Screen> {
        Screen::ALL
            .iter()
            .copied()
            .find(|screen| screen.key() == key)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Screen::Compare => "Compare two documents",
            Screen::Scan => "Write on a scan",
            Screen::Document => "Make a document",
            Screen::Draw => "Draw on a page",
            Screen::Read => "Read a scan",
            Screen::Blanks => "Where there is room",
            Screen::Proof => "See it before you print",
            Screen::Merge => "Merge deltas",
            Screen::Labels => "Sheet of labels",
            Screen::Jobs => "Saved jobs",
            Screen::Verify => "Check a sheet",
            Screen::History => "What was added",
            Screen::Devices => "Printers and scanners",
            Screen::Calibrate => "Calibration",
            Screen::Doctor => "This machine",
        }
    }

    /// The sentence under the name in the sidebar, so somebody can tell which
    /// one they want without opening all six.
    pub fn lede(&self) -> &'static str {
        match self {
            Screen::Compare => "Print only what changed between two files",
            Screen::Scan => "Type onto a page you only have as a scan",
            Screen::Document => "Start from blank paper and keep adding",
            Screen::Draw => "Lines, boxes and circles, in any colour",
            Screen::Read => "Turn a scan into a Word document",
            Screen::Blanks => "Ask the form where you can write, in millimetres",
            Screen::Proof => "The sheet and the delta together, on screen",
            Screen::Merge => "Several onto one, so the sheet goes through once",
            Screen::Labels => "Addresses and files, one per label, from a list",
            Screen::Jobs => "The same stamp on today's document, in two clicks",
            Screen::Verify => "Did it come out of the printer right?",
            Screen::History => "Have I printed this delta already?",
            Screen::Devices => "Print and scan over the network",
            Screen::Calibrate => "Measure a printer, once, for exactness",
            Screen::Doctor => "What works here, and what is missing",
        }
    }
}
