//! The things the window can be showing.
//!
//! One module per screen, each keeping its own state and drawing itself. They
//! do not know about one another, and none of them does slow work on the thread
//! that draws — everything heavy goes through [`Jobs`].

pub mod calibrate;
pub mod compare;
pub mod devices;
pub mod document;
pub mod doctor;
pub mod draw;
pub mod read;
pub mod scan;

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
}

/// Which screen the window is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    Compare,
    Scan,
    Document,
    Draw,
    Read,
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
        Screen::Devices,
        Screen::Calibrate,
        Screen::Doctor,
    ];

    pub fn name(&self) -> &'static str {
        match self {
            Screen::Compare => "Compare two documents",
            Screen::Scan => "Write on a scan",
            Screen::Document => "Make a document",
            Screen::Draw => "Draw on a page",
            Screen::Read => "Read a scan",
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
            Screen::Devices => "Print and scan over the network",
            Screen::Calibrate => "Measure a printer, once, for exactness",
            Screen::Doctor => "What works here, and what is missing",
        }
    }
}
