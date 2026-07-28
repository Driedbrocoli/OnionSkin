//! The things the window can be showing.
//!
//! One module per screen, each keeping its own state and drawing itself. They
//! do not know about one another, and none of them does slow work on the thread
//! that draws — everything heavy goes through [`Jobs`].

pub mod back;
pub mod barcode;
pub mod batch;
pub mod blanks;
pub mod calibrate;
pub mod compare;
pub mod correct;
pub mod cover;
pub mod devices;
pub mod doctor;
pub mod document;
pub mod draw;
pub mod fits;
pub mod history;
pub mod jobs;
pub mod join;
pub mod labels;
pub mod merge;
pub mod proof;
pub mod read;
pub mod scan;
pub mod stack;
pub mod verify;
pub mod watermark;

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

/// A button that fills in the delta written most recently.
///
/// The counterpart of `--delta last` on the command line, and it exists for the
/// same reason: a delta that was not given a name of its own goes into
/// Onionskin's own folder under one nobody chose, and finding it again means
/// going and looking for a file whose name you never saw. The program already
/// knows which one it was.
///
/// Nothing is shown when there is no delta to offer — a button that is always
/// there and usually does nothing is a button people stop reading.
pub fn last_delta_button(ui: &mut egui::Ui, into: &mut Option<std::path::PathBuf>) {
    let Some(last) = onionskin::history::last_delta() else {
        return;
    };
    let named = last
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    ui.horizontal(|ui| {
        if ui
            .small_button("Use the one just written")
            .on_hover_text(last.display().to_string())
            .clicked()
        {
            *into = Some(last.clone());
        }
        ui.label(egui::RichText::new(named).weak().monospace());
    });
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
    Correct,
    Cover,
    Watermark,
    Barcode,
    Back,
    Proof,
    Merge,
    Join,
    Batch,
    Labels,
    Jobs,
    Fits,
    Stack,
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
        Screen::Correct,
        Screen::Cover,
        Screen::Watermark,
        Screen::Barcode,
        Screen::Back,
        Screen::Proof,
        Screen::Merge,
        Screen::Join,
        Screen::Batch,
        Screen::Labels,
        Screen::Jobs,
        Screen::Fits,
        Screen::Stack,
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
            Screen::Join => "join",
            Screen::Labels => "labels",
            Screen::Jobs => "jobs",
            Screen::Batch => "batch",
            Screen::Correct => "correct",
            Screen::Cover => "cover",
            Screen::Watermark => "watermark",
            Screen::Barcode => "barcode",
            Screen::Back => "back",
            Screen::Fits => "fits",
            Screen::Stack => "stack",
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
            Screen::Join => "Join files",
            Screen::Labels => "Sheet of labels",
            Screen::Jobs => "Saved jobs",
            Screen::Batch => "One each, from a list",
            Screen::Correct => "Fix a mistake",
            Screen::Cover => "Cover something up",
            Screen::Watermark => "Stamp a word across it",
            Screen::Barcode => "A barcode or a QR code",
            Screen::Back => "The back of the sheet",
            Screen::Fits => "Check before printing",
            Screen::Stack => "Sort a stack",
            Screen::Verify => "Check a sheet or a run",
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
            Screen::Join => "Several one after another, into one document",
            Screen::Labels => "Addresses and files, one per label, from a list",
            Screen::Jobs => "The same stamp on today's document, in two clicks",
            Screen::Batch => "Two hundred certificates, two hundred names",
            Screen::Correct => "Cover the wrong words and set the right ones",
            Screen::Cover => "Print solid over what must not be read",
            Screen::Watermark => "DRAFT, COPY or VOID, corner to corner",
            Screen::Barcode => "An asset number or a link, worked out here",
            Screen::Back => "Terms, an address, \"continued overleaf\"",
            Screen::Fits => "Is this the sheet the delta was made for?",
            Screen::Stack => "Which document is each sheet from the feeder?",
            Screen::Verify => "Did it come out of the printer right — one sheet, or all of them?",
            Screen::History => "Have I printed this delta already?",
            Screen::Devices => "Print and scan over the network",
            Screen::Calibrate => "Measure a printer, once, for exactness",
            Screen::Doctor => "What works here, and what is missing",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    /// Every screen has to be in `ALL`, because `ALL` is what the sidebar is
    /// made from.
    ///
    /// Nothing else catches this. `key`, `name` and `summary` are exhaustive
    /// matches, so a new variant will not compile until all three know about
    /// it — but `ALL` is a hand-written list, and a screen left out of it is a
    /// screen nobody can reach. It compiles, it passes, and the feature is
    /// simply not there.
    ///
    /// So the arms are counted out of this file's own source. Exotic, but the
    /// alternative is a number in a test that has to be remembered, and a
    /// number that has to be remembered is a number that will not be.
    #[test]
    fn every_screen_is_one_the_sidebar_offers() {
        let source = include_str!("mod.rs");
        // The arms of `key`, which is the first of the three matches and the
        // one that has one line per screen.
        let key_body = source
            .split("pub fn key(&self)")
            .nth(1)
            .expect("key() should be in this file");
        let arms: BTreeSet<&str> = key_body
            .lines()
            .take_while(|line| !line.contains("from_key"))
            .filter_map(|line| line.trim().strip_prefix("Screen::"))
            .filter_map(|rest| rest.split_once(" =>"))
            .map(|(name, _)| name)
            .collect();

        assert!(
            arms.len() > 10,
            "only {} arms were found, so this test is reading the file wrongly",
            arms.len()
        );
        assert_eq!(
            Screen::ALL.len(),
            arms.len(),
            "there are {} screens and {} in the sidebar — one has been added to \
             the enum and left out of ALL, which makes it unreachable",
            arms.len(),
            Screen::ALL.len()
        );
    }

    /// Two screens with the same key would make the remembered screen
    /// ambiguous — somebody would close the window on one and open it on
    /// another.
    #[test]
    fn every_screen_is_told_apart_by_its_key_and_says_something_of_itself() {
        let mut keys = BTreeSet::new();
        let mut names = BTreeSet::new();
        for screen in Screen::ALL {
            assert!(
                keys.insert(screen.key()),
                "'{}' is the key of more than one screen",
                screen.key()
            );
            assert!(
                names.insert(screen.name()),
                "'{}' is the name of more than one screen",
                screen.name()
            );
            assert!(!screen.lede().is_empty(), "{} has no lede", screen.key());
            // A key goes in a settings file and comes back out, so it has to
            // be something a person can read and a file can hold.
            assert!(
                screen
                    .key()
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c == '-'),
                "'{}' is not a key anybody would want in a settings file",
                screen.key()
            );
        }
    }

    /// The screen somebody was last on is remembered by key, so every key has
    /// to find its way home.
    #[test]
    fn the_screen_somebody_was_last_on_is_found_again() {
        for screen in Screen::ALL {
            assert_eq!(
                Screen::from_key(screen.key()),
                Some(*screen),
                "{} does not come back from its own key",
                screen.key()
            );
        }
        assert_eq!(Screen::from_key("no-such-screen"), None);
        assert_eq!(Screen::from_key(""), None);
    }
}

#[cfg(test)]
mod last_delta_tests {
    /// The window has to offer the delta just written wherever the command
    /// line's `--delta last` would.
    ///
    /// The three screens that take a delta are proof, verify and fits. A
    /// fourth added later that asks for one and does not offer this is a
    /// screen where somebody has to go and find a file whose name they never
    /// saw — so the source is checked rather than a list kept by hand.
    #[test]
    fn every_screen_that_asks_for_a_delta_offers_the_one_just_written() {
        let screens = [
            ("proof", include_str!("proof.rs")),
            ("verify", include_str!("verify.rs")),
            ("fits", include_str!("fits.rs")),
        ];
        for (name, source) in screens {
            assert!(
                source.contains("&mut state.delta"),
                "{name} no longer asks for a delta; this test needs revisiting"
            );
            assert!(
                source.contains("last_delta_button"),
                "the {name} screen asks for a delta and does not offer the one \
                 just written"
            );
        }
    }
}
