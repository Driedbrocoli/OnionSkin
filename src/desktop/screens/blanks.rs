//! Asking the form where there is room to write.
//!
//! The commonest thing anybody does with this program is fill in a printed
//! form, and the first thing they have to do is find the coordinates. On the
//! command line that means a ruler against the paper, or reading pixels off a
//! scan in an image editor and converting them, for every box on the page.
//!
//! In a window it should mean neither. The page already knows where its own
//! empty spaces are, so it is asked — and what comes back is a list of places
//! with a "Copy" button beside each one, which pastes straight into the box on
//! the writing screen.
//!
//! This is the screen that makes the program usable by somebody who has never
//! measured anything in millimetres in their life, which is nearly everybody.

use eframe::egui;

use super::Room;
use crate::job::Outcome;
use crate::widgets;

pub struct State {
    /// The form: a PDF, or a photograph of one.
    form: Option<std::path::PathBuf>,
    page: String,
    /// The narrowest gap worth reporting.
    min_width_mm: f64,
    /// The shortest clear band worth reporting, for open areas.
    min_height_mm: f64,
    /// How close to the paper's edge to look.
    margin_mm: f64,
    /// Where the looking thread leaves what it found.
    ///
    /// A shelf rather than a change to [`crate::job::Jobs`], which carries a
    /// message and a list of files and has no room for a typed answer. One
    /// screen wanting one thing back is not a reason to widen what every
    /// screen is given.
    answer: std::sync::Arc<std::sync::Mutex<Option<Found>>>,
    /// Found last time the button was pressed, kept so the list stays up while
    /// somebody copies from it.
    found: Vec<onionskin::blanks::Blank>,
    /// What registering a scan had to say, if it was a scan.
    note: String,
    /// Which one was last copied, so the button can say so.
    copied: Option<usize>,
}

/// What the looking thread hands back.
struct Found {
    blanks: Vec<onionskin::blanks::Blank>,
    note: String,
}

impl Default for State {
    fn default() -> Self {
        State {
            form: None,
            page: "a4".into(),
            min_width_mm: 20.0,
            min_height_mm: 3.5,
            margin_mm: onionskin::safety::DEFAULT_MARGIN_MM,
            answer: Default::default(),
            found: Vec::new(),
            note: String::new(),
            copied: None,
        }
    }
}

/// What a form can arrive as: its own PDF, or a photograph of the paper.
const FORM_KINDS: &[&str] = &["pdf", "png", "jpg", "jpeg", "tif", "tiff", "bmp"];

pub fn show(state: &mut State, room: &mut Room) {
    widgets::title(
        room.ui,
        "Where there is room to write",
        "Ask the form, instead of measuring it with a ruler.",
    );

    widgets::hint(
        room.ui,
        "Onionskin looks for the empty places on the page and says where each \
         one is, how big, and how many characters fit. Copy a place and paste \
         it into the writing screen.",
    );
    room.ui.add_space(10.0);

    if widgets::file_row(
        room.ui,
        room.picker,
        "The form",
        &mut state.form,
        FORM_KINDS,
        room.dropped,
    ) {
        // A different form makes the old answer wrong, and an answer left on
        // screen beside a new file is worse than no answer.
        state.found.clear();
        state.note.clear();
        state.copied = None;
    }

    room.ui.horizontal(|ui| {
        ui.label("Paper");
        ui.add(egui::TextEdit::singleline(&mut state.page).desired_width(90.0));
        widgets::hint(ui, "a4, letter, a5 — only needed for a photograph");
    });

    room.ui.collapsing("What counts as room", |ui| {
        ui.horizontal(|ui| {
            ui.label("At least");
            ui.add(
                egui::DragValue::new(&mut state.min_width_mm)
                    .speed(0.5)
                    .range(2.0..=200.0)
                    .suffix(" mm wide"),
            );
            ui.add(
                egui::DragValue::new(&mut state.min_height_mm)
                    .speed(0.1)
                    .range(1.0..=100.0)
                    .suffix(" mm tall"),
            );
        });
        widgets::hint(
            ui,
            "Lower the width to find narrower boxes. The height only applies to \
             open areas — a gap beside a printed label takes its size from the \
             line it sits on.",
        );
        ui.horizontal(|ui| {
            ui.label("Ignore within");
            ui.add(
                egui::DragValue::new(&mut state.margin_mm)
                    .speed(0.5)
                    .range(0.0..=40.0)
                    .suffix(" mm of the edge"),
            );
        });
        widgets::hint(ui, "most printers cannot put ink there anyway");
    });

    room.ui.add_space(10.0);
    if room
        .ui
        .add_enabled(
            state.form.is_some() && !room.jobs.busy(),
            egui::Button::new(egui::RichText::new("Find the room").strong()),
        )
        .clicked()
    {
        look(state, room);
    }
    if state.form.is_none() {
        widgets::hint(room.ui, "Choose the form to look at.");
    }

    // Taken off the shelf the moment it is there, and kept here so the list
    // stays up while somebody works through it.
    if let Ok(mut shelf) = state.answer.lock() {
        if let Some(found) = shelf.take() {
            state.found = found.blanks;
            state.note = found.note;
            state.copied = None;
        }
    }

    if !state.note.is_empty() {
        room.ui.add_space(8.0);
        widgets::hint(room.ui, &state.note);
    }

    if !state.found.is_empty() {
        room.ui.add_space(10.0);
        room.ui.label(
            egui::RichText::new(format!("{} place(s) to write", state.found.len())).strong(),
        );
        widgets::hint(
            room.ui,
            "Beside a label first, then widest. Copy one and paste it into \
             'Write on a scan'.",
        );
        room.ui.add_space(6.0);

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(room.ui, |ui| {
                for (index, blank) in state.found.iter().enumerate() {
                    ui.horizontal_wrapped(|ui| {
                        let copied = state.copied == Some(index);
                        if ui.button(if copied { "Copied" } else { "Copy" }).clicked() {
                            ui.ctx().copy_text(blank.placement());
                            state.copied = Some(index);
                        }
                        ui.label(egui::RichText::new(blank.placement()).monospace());
                        ui.label(
                            egui::RichText::new(if blank.beside_text {
                                "beside a label"
                            } else {
                                "open"
                            })
                            .weak()
                            .small(),
                        );
                        ui.label(format!(
                            "{:.0} × {:.0} mm — about {} characters at {:.0} pt",
                            blank.width_mm,
                            blank.height_mm,
                            blank.fits_characters(),
                            blank.fits_pt()
                        ));
                    });
                    ui.separator();
                }
            });
    }

    if let Some(outcome) = &room.jobs.last {
        if matches!(outcome, Outcome::Refused { .. }) {
            room.ui.add_space(12.0);
            if widgets::outcome(room.ui, outcome) {
                room.jobs.dismiss();
            }
        }
    }
}

fn look(state: &mut State, room: &mut Room) {
    let Some(form) = state.form.clone() else {
        return;
    };
    let page_text = state.page.clone();
    let options = onionskin::blanks::BlankOptions {
        ink_threshold: 128,
        min_width_mm: state.min_width_mm,
        min_height_mm: state.min_height_mm,
        margin_mm: state.margin_mm,
    };

    let shelf = state.answer.clone();
    state.found.clear();
    state.note.clear();

    room.jobs.start("Looking at the form", move |report| {
        let page = match onionskin::geometry::parse_page(&page_text) {
            Ok(page) => page,
            Err(e) => return Outcome::refused(e.to_string()),
        };
        report.saying("Opening the form…");
        let sheet = match onionskin::blanks::open_sheet(&form, page, false, false) {
            Ok(sheet) => sheet,
            Err(e) => return Outcome::refused(e),
        };
        report.saying("Finding the empty places…");
        let blanks =
            onionskin::blanks::find(&sheet.gray, sheet.width, sheet.dpi, sheet.page, &options);

        if blanks.is_empty() {
            return Outcome::refused(
                "Nothing on that form is clear enough to write in at these \
                 settings.\n\nUnder 'What counts as room', try a smaller width \
                 for narrower boxes, or a smaller edge margin if the form runs \
                 close to the paper's edge."
                    .to_string(),
            );
        }

        let how_many = blanks.len();
        if let Ok(mut shelf) = shelf.lock() {
            *shelf = Some(Found {
                blanks,
                note: sheet.note,
            });
        }
        Outcome::done(format!("{how_many} place(s) to write."))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The defaults have to find the boxes on an ordinary form without anybody
    /// touching a number, or the screen is a settings panel with a button.
    #[test]
    fn the_defaults_are_the_ones_the_command_line_uses() {
        let state = State::default();
        let mine = onionskin::blanks::BlankOptions {
            ink_threshold: 128,
            min_width_mm: state.min_width_mm,
            min_height_mm: state.min_height_mm,
            margin_mm: state.margin_mm,
        };
        assert_eq!(mine, onionskin::blanks::BlankOptions::default());
    }

    /// A form is a PDF or a photograph of one, and the file browser has to
    /// offer both — somebody with the form on paper has only the photograph.
    #[test]
    fn both_a_pdf_and_a_photograph_can_be_chosen() {
        assert!(FORM_KINDS.contains(&"pdf"));
        assert!(FORM_KINDS.contains(&"png"));
        assert!(FORM_KINDS.contains(&"jpg"));
    }
}
