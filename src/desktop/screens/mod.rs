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
pub mod harvest;
pub mod history;
pub mod jobs;
pub mod join;
pub mod labels;
pub mod merge;
pub mod proof;
pub mod read;
pub mod redact;
pub mod scan;
pub mod stack;
pub mod verify;
pub mod watch;
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
    Redact,
    Watermark,
    Barcode,
    Back,
    Harvest,
    Proof,
    Merge,
    Join,
    Batch,
    Labels,
    Jobs,
    Watch,
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
        Screen::Redact,
        Screen::Watermark,
        Screen::Barcode,
        Screen::Back,
        Screen::Harvest,
        Screen::Proof,
        Screen::Merge,
        Screen::Join,
        Screen::Batch,
        Screen::Labels,
        Screen::Jobs,
        Screen::Watch,
        Screen::Fits,
        Screen::Stack,
        Screen::Verify,
        Screen::History,
        Screen::Devices,
        Screen::Calibrate,
        Screen::Doctor,
    ];

    /// The same screens, under headings, in the order the list is read.
    ///
    /// [`Screen::ALL`] is what exists; this is how somebody finds it. Twenty-six
    /// names down the side of a window, in one undifferentiated column, is not
    /// a list anybody reads — it is a wall to be scrolled past, and the screen
    /// somebody actually wants is somewhere in it. Six headings turn "read
    /// twenty-six names" into "read six, then read four".
    ///
    /// Kept apart from `ALL` rather than replacing it, because `ALL` is what
    /// `key`, `from_key` and the counting test are written against — and
    /// because a screen dropped out of a grouping by accident would then simply
    /// vanish from the window. The test below counts one against the other so
    /// it cannot.
    ///
    /// The headings are in the person's words rather than the program's: not
    /// "delta", not "registration", not "vector". Somebody arriving at this
    /// window is holding a sheet of paper and knows what they want to do to it.
    pub const GROUPS: &'static [(&'static str, &'static [Screen])] = &[
        (
            "ADD SOMETHING TO A SHEET",
            &[
                Screen::Scan,
                Screen::Watermark,
                Screen::Correct,
                Screen::Cover,
                Screen::Barcode,
                Screen::Back,
                Screen::Blanks,
                Screen::Draw,
            ],
        ),
        (
            "PRINT ONLY WHAT CHANGED",
            // `Document` belongs here: the whole point of that screen is that
            // each print carries only what was added since the last one.
            &[
                Screen::Compare,
                Screen::Document,
                Screen::Merge,
                Screen::Join,
            ],
        ),
        (
            "THE SAME THING, MANY TIMES",
            &[Screen::Batch, Screen::Labels, Screen::Jobs, Screen::Watch],
        ),
        (
            "CHECK IT",
            &[Screen::Proof, Screen::Fits, Screen::Verify, Screen::History],
        ),
        (
            "GET WORDS BACK OFF PAPER",
            &[Screen::Read, Screen::Harvest, Screen::Stack],
        ),
        ("SEND IT ON", &[Screen::Redact]),
        (
            "THIS MACHINE",
            &[Screen::Devices, Screen::Calibrate, Screen::Doctor],
        ),
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
            Screen::Watch => "watch",
            Screen::Batch => "batch",
            Screen::Correct => "correct",
            Screen::Cover => "cover",
            Screen::Redact => "redact",
            Screen::Watermark => "watermark",
            Screen::Barcode => "barcode",
            Screen::Back => "back",
            Screen::Harvest => "harvest",
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
            Screen::Scan => "Write on a sheet",
            Screen::Document => "Make a document",
            Screen::Draw => "Draw on a page",
            Screen::Read => "Read a scan",
            Screen::Blanks => "Find the gaps on a form",
            Screen::Proof => "Look at it on screen first",
            Screen::Merge => "Two sets of additions, one pass",
            Screen::Join => "Join files",
            Screen::Labels => "Sheet of labels",
            Screen::Jobs => "Saved jobs",
            Screen::Watch => "Watch a folder",
            Screen::Batch => "One each, from a list",
            Screen::Correct => "Fix a mistake",
            Screen::Cover => "Cover something up",
            Screen::Redact => "Take something out for good",
            Screen::Watermark => "Stamp a word across it",
            Screen::Barcode => "A barcode or a QR code",
            Screen::Back => "The back of the sheet",
            Screen::Harvest => "Forms into a spreadsheet",
            Screen::Fits => "Is this the right sheet?",
            Screen::Stack => "Sort a stack",
            Screen::Verify => "Did it come out right?",
            Screen::History => "Have I printed this already?",
            Screen::Devices => "Printers and scanners",
            Screen::Calibrate => "Measure your printer",
            Screen::Doctor => "What works here",
        }
    }

    /// The sentence under the name in the sidebar, so somebody can tell which
    /// one they want without opening all six.
    pub fn lede(&self) -> &'static str {
        match self {
            Screen::Compare => "Print only what changed between two files",
            Screen::Scan => "Put words where you point on the page",
            Screen::Document => "Start from blank paper and keep adding",
            Screen::Draw => "Lines, boxes and circles, in any colour",
            Screen::Read => "Turn a scan into a Word document",
            Screen::Blanks => "Ask the form where you can write, in millimetres",
            Screen::Proof => "The sheet and the new words together, on screen",
            Screen::Merge => "Several onto one, so the sheet goes through once",
            Screen::Join => "Several one after another, into one document",
            Screen::Labels => "Addresses and files, one per label, from a list",
            Screen::Jobs => "The same stamp on today's document, in two clicks",
            Screen::Watch => "Scan to a folder, and the new page is waiting for you",
            Screen::Batch => "Two hundred certificates, two hundred names",
            Screen::Correct => "Cover the wrong words and set the right ones",
            Screen::Cover => "Print solid over what must not be read",
            Screen::Redact => "For a copy you email, not one you print",
            Screen::Watermark => "DRAFT, COPY or VOID, corner to corner",
            Screen::Barcode => "An asset number or a link, worked out here",
            Screen::Back => "Terms, an address, \"continued overleaf\"",
            Screen::Harvest => "Read the sheets back instead of typing them",
            Screen::Fits => "Same size, same page, before the sheet goes in the tray",
            Screen::Stack => "Which document is each sheet from the feeder?",
            Screen::Verify => "Check the printed sheet — one of them, or the whole run",
            Screen::History => "Everything that has been written, and when",
            Screen::Devices => "Print and scan over the network",
            Screen::Calibrate => "Once, so the words land where you asked",
            Screen::Doctor => "What is installed, and what is missing",
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
    /// Every screen has to be in a group as well, or it is invisible.
    ///
    /// `GROUPS` is what the sidebar is actually drawn from now, and it is a
    /// hand-written list of hand-written lists — so a screen left out of it
    /// compiles, passes every other test, and simply is not in the window.
    /// That is the same fault the test above exists for, one level further in.
    #[test]
    fn every_screen_is_under_a_heading_and_only_one() {
        let mut seen: BTreeSet<&'static str> = BTreeSet::new();
        for (heading, screens) in Screen::GROUPS {
            assert!(
                !screens.is_empty(),
                "the heading '{heading}' has nothing under it"
            );
            for screen in *screens {
                assert!(
                    seen.insert(screen.key()),
                    "{} is under two headings, so it appears in the list twice",
                    screen.key()
                );
            }
        }
        for screen in Screen::ALL {
            assert!(
                seen.contains(screen.key()),
                "{} is in ALL and under no heading, so nobody can reach it",
                screen.key()
            );
        }
        assert_eq!(seen.len(), Screen::ALL.len());
    }

    /// The window's own words, not the program's.
    ///
    /// "Delta" is the word this program thinks in and nobody else does. A
    /// person arriving at this window is holding a sheet of paper; every name
    /// and every sentence under one has to make sense to them before they have
    /// read anything else. The rest of the vocabulary here is the same kind:
    /// registration, raster, vector, XObject — all true, none of them a word
    /// anybody would use about a piece of paper.
    ///
    /// The headings are held to it too, because they are read first.
    #[test]
    fn nothing_in_the_list_is_said_in_the_programs_own_words() {
        const NOT_OUR_WORDS: &[&str] = &[
            "delta",
            "raster",
            "vector",
            "registration",
            "xobject",
            "dpi",
            "rasterise",
            "coincident",
            "affine",
        ];
        for screen in Screen::ALL {
            for said in [screen.name(), screen.lede()] {
                let plain = said.to_lowercase();
                for word in NOT_OUR_WORDS {
                    assert!(
                        !plain.contains(word),
                        "{}: '{said}' says '{word}', which is a word for the inside                          of the program and not for the person reading it",
                        screen.key()
                    );
                }
            }
        }
        for (heading, _) in Screen::GROUPS {
            let plain = heading.to_lowercase();
            for word in NOT_OUR_WORDS {
                assert!(
                    !plain.contains(word),
                    "the heading '{heading}' says '{word}'"
                );
            }
        }
    }

    /// Four screens were four spellings of "check", which a stranger cannot
    /// tell apart — so each is now the question it answers, and no two of them
    /// may be the same question.
    #[test]
    fn no_two_screens_are_called_the_same_thing() {
        let mut names: BTreeSet<&'static str> = BTreeSet::new();
        for screen in Screen::ALL {
            assert!(
                names.insert(screen.name()),
                "two screens are both called '{}'",
                screen.name()
            );
        }
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

#[cfg(test)]
mod drawing {
    use super::*;

    /// Every screen, drawn.
    ///
    /// The screens have tests for what they decide — which columns are named,
    /// which grey is the default, where the delta lands. None of them had a test
    /// for *drawing*, because drawing appeared to need a window and a graphics
    /// card, and this program is tested on machines with neither.
    ///
    /// It does not. egui lays a frame out in software and only hands the result
    /// to a graphics card afterwards, so `Context::run` walks every widget on a
    /// screen — every range, every slider, every identifier — with nothing on the
    /// screen at all.
    ///
    /// What it catches is a panic on the way to the screen: an index into a list
    /// the layout code assumed was not empty, an unwrap on a state that is empty
    /// when a screen first opens, arithmetic that overflows on a default value.
    /// Checked against exactly that — reaching past the end of a screen's own
    /// list of rows turns this red, naming the file and the line.
    ///
    /// What it does not catch is a widget that is merely wrong. An inverted
    /// range was tried and egui takes it without complaint, so this is a test
    /// for falling over rather than for looking right. Looking right needs eyes,
    /// and nothing here claims otherwise.
    ///
    /// Every screen is drawn twice. The first pass is the one that lays out; the
    /// second is where egui compares what it laid out against what it laid out
    /// last time, which is where a clashing identifier is noticed.
    fn draw(screen: Screen) {
        let ctx = egui::Context::default();
        let mut app = Screens::default();
        let mut jobs = Jobs::new(&ctx);
        let mut previews = Previews::default();
        let mut picker = crate::picker::Picker::default();
        let mut dropped = Vec::new();

        for _ in 0..2 {
            let _ = ctx.run_ui(egui::RawInput::default(), |ui| {
                let mut room = Room {
                    picker: &mut picker,
                    jobs: &mut jobs,
                    previews: &mut previews,
                    dropped: &mut dropped,
                    ui,
                };
                app.show(screen, &mut room);
            });
        }
    }

    /// One of each screen's state, so a screen can be drawn without the whole
    /// window being built.
    #[derive(Default)]
    struct Screens {
        compare: compare::State,
        scan: scan::State,
        document: document::State,
        draw: draw::State,
        read: read::State,
        verify: verify::State,
        blanks: blanks::State,
        proof: proof::State,
        merge: merge::State,
        join: join::State,
        fits: fits::State,
        stack: stack::State,
        correct: correct::State,
        cover: cover::State,
        redact: redact::State,
        watermark: watermark::State,
        barcode: barcode::State,
        back: back::State,
        harvest: harvest::State,
        batch: batch::State,
        labels: labels::State,
        jobs: jobs::State,
        watch: watch::State,
        history: history::State,
        devices: devices::State,
        calibrate: calibrate::State,
        doctor: doctor::State,
    }

    impl Screens {
        fn show(&mut self, screen: Screen, room: &mut Room) {
            match screen {
                Screen::Compare => compare::show(&mut self.compare, room),
                Screen::Scan => scan::show(&mut self.scan, room),
                Screen::Document => document::show(&mut self.document, room),
                Screen::Draw => draw::show(&mut self.draw, room),
                Screen::Read => read::show(&mut self.read, room),
                Screen::Verify => verify::show(&mut self.verify, room),
                Screen::Blanks => blanks::show(&mut self.blanks, room),
                Screen::Proof => proof::show(&mut self.proof, room),
                Screen::Merge => merge::show(&mut self.merge, room),
                Screen::Join => join::show(&mut self.join, room),
                Screen::Fits => fits::show(&mut self.fits, room),
                Screen::Stack => stack::show(&mut self.stack, room),
                Screen::Correct => correct::show(&mut self.correct, room),
                Screen::Cover => cover::show(&mut self.cover, room),
                Screen::Redact => redact::show(&mut self.redact, room),
                Screen::Watermark => watermark::show(&mut self.watermark, room),
                Screen::Barcode => barcode::show(&mut self.barcode, room),
                Screen::Back => back::show(&mut self.back, room),
                Screen::Harvest => harvest::show(&mut self.harvest, room),
                Screen::Batch => batch::show(&mut self.batch, room),
                Screen::Labels => labels::show(&mut self.labels, room),
                Screen::Jobs => jobs::show(&mut self.jobs, room),
                Screen::Watch => watch::show(&mut self.watch, room),
                Screen::History => history::show(&mut self.history, room),
                Screen::Devices => devices::show(&mut self.devices, room),
                Screen::Calibrate => calibrate::show(&mut self.calibrate, room),
                Screen::Doctor => doctor::show(&mut self.doctor, room),
            }
        }
    }

    #[test]
    fn every_screen_draws_without_falling_over() {
        for screen in Screen::ALL {
            draw(*screen);
        }
    }
}
