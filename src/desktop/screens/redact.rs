//! Taking something out of a document, for a copy that is sent rather than
//! printed.
//!
//! The screen next door covers something up with solid toner, on a sheet you
//! are holding. This is the same job on a file, and it is not the same
//! operation at all — which is the reason it is a screen of its own rather
//! than a tick box on that one.
//!
//! Drawing a black rectangle in a PDF hides nothing. The words stay in the
//! file: selectable, copyable, searchable, and recovered by anybody who
//! presses Ctrl-A. It is the obvious thing to do, it looks completely
//! convincing on screen, and every few months an organisation publishes a
//! document redacted that way and reads the covered names in the newspaper the
//! following week.
//!
//! So this does not draw anything over anything. It takes the words out — see
//! [`onionskin::redact`] for how, and for why every page of the document ends
//! up a picture.

use eframe::egui;

use super::{beside, Room};
use crate::job::Outcome;
use crate::widgets;

pub struct State {
    /// The document with something in it that must not be read.
    document: Option<std::path::PathBuf>,
    output: Option<std::path::PathBuf>,
    /// Words to take out, found on the page rather than measured.
    words: Vec<String>,
    /// Rectangles measured by hand, for anything that is not words.
    boxes: Vec<Area>,
    /// How far beyond the words to take out.
    pad_mm: f64,
    page: usize,
    dpi: f64,
}

/// One rectangle of the page, in millimetres from the top-left.
#[derive(Clone, Copy)]
struct Area {
    x_mm: f64,
    y_mm: f64,
    width_mm: f64,
    height_mm: f64,
}

impl Default for State {
    fn default() -> Self {
        State {
            document: None,
            output: None,
            words: vec![String::new()],
            boxes: Vec::new(),
            pad_mm: 1.0,
            page: 1,
            dpi: onionskin::redact::DEFAULT_DPI,
        }
    }
}

pub fn show(state: &mut State, room: &mut Room) {
    let screen = super::Screen::Redact;
    widgets::title(room.ui, screen.name(), screen.lede());

    widgets::hint(
        room.ui,
        "For the copy you email or hand over. Name the words and Onionskin \
         finds them on every page — no measuring — or give a rectangle for \
         anything that is not words. A Word file has to be saved as a PDF \
         first: what you redact should be the file you would have sent.",
    );
    room.ui.add_space(10.0);

    // PDFs and nothing else, which is narrower than every other screen here
    // and is meant to be.
    //
    // Onionskin can read a Word file, and what it produces is its own rendering
    // of one: close, not identical, and honest about that everywhere else
    // because everywhere else the output is ink on a sheet somebody looks at.
    // Here the output is the copy that gets sent, and redacting an approximate
    // re-typesetting of somebody's document is not the job they asked for. The
    // file they redact should be the file they would otherwise have sent.
    widgets::file_row(
        room.ui,
        room.picker,
        "The document",
        &mut state.document,
        &["pdf"],
        room.dropped,
    );
    widgets::save_row(
        room.ui,
        room.picker,
        "Write the copy to hand over to",
        &mut state.output,
        "redacted.pdf",
        &["pdf"],
        "beside the document, as NAME-redacted.pdf",
    );

    room.ui.add_space(8.0);
    room.ui
        .label(egui::RichText::new("Words to take out").strong());
    widgets::hint(
        room.ui,
        "The whole line each one sits on goes, on every page it appears — \
         because somebody who says 'take out the salary' means the figure, not \
         the label beside it.",
    );
    let mut remove = None;
    for (index, words) in state.words.iter_mut().enumerate() {
        room.ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(words)
                    .hint_text("Salary")
                    .desired_width(280.0),
            );
            if ui.small_button("×").clicked() {
                remove = Some(index);
            }
        });
    }
    if let Some(index) = remove {
        state.words.remove(index);
    }
    if state.words.is_empty() || room.ui.small_button("Another").clicked() {
        state.words.push(String::new());
    }

    room.ui.add_space(8.0);
    room.ui.collapsing("Or a rectangle, measured", |ui| {
        widgets::hint(
            ui,
            "Millimetres from the top-left of the page — for a photograph, a \
             signature, or words the reader cannot make out.",
        );
        let mut drop = None;
        for (index, area) in state.boxes.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                millimetres(ui, &mut area.x_mm);
                ui.label(",");
                millimetres(ui, &mut area.y_mm);
                ui.label("size");
                millimetres(ui, &mut area.width_mm);
                ui.label("×");
                millimetres(ui, &mut area.height_mm);
                if ui.small_button("×").clicked() {
                    drop = Some(index);
                }
            });
        }
        if let Some(index) = drop {
            state.boxes.remove(index);
        }
        if ui.small_button("Add a rectangle").clicked() {
            state.boxes.push(Area {
                x_mm: 20.0,
                y_mm: 40.0,
                width_mm: 60.0,
                height_mm: 8.0,
            });
        }
        ui.horizontal(|ui| {
            ui.label("Beyond the words");
            millimetres(ui, &mut state.pad_mm);
            ui.label("Page");
            ui.add(egui::DragValue::new(&mut state.page).range(1..=999));
        });
    });

    room.ui.add_space(8.0);
    room.ui.collapsing("How finely to draw it", |ui| {
        ui.horizontal(|ui| {
            ui.label("Resolution");
            ui.add(
                egui::DragValue::new(&mut state.dpi)
                    .speed(10.0)
                    .range(72.0..=600.0)
                    .suffix(" dpi"),
            );
        });
        widgets::hint(
            ui,
            "300 is what a printer is asked for, so the copy prints the same as \
             the original. Lower makes a smaller file and softer small type.",
        );
    });

    // Above the button, because it is what somebody needs while they are still
    // deciding whether this is the thing they want.
    room.ui.add_space(8.0);
    widgets::hint(
        room.ui,
        "The words are removed from the file, not covered over — so they cannot \
         be selected, copied or searched back out. Every page becomes a \
         picture to make that true, which means the copy cannot be searched \
         either. Keep the original: this is what you send, not what you work \
         from.",
    );

    room.ui.add_space(10.0);
    let ready = state.document.is_some() && !(named(state).is_empty() && state.boxes.is_empty());
    if room
        .ui
        .add_enabled(
            ready && !room.jobs.busy(),
            egui::Button::new(egui::RichText::new("Take them out").strong()),
        )
        .clicked()
    {
        take_them_out(state, room);
    }
    if !ready {
        widgets::hint(
            room.ui,
            "Give the document, and either some words or a rectangle.",
        );
    }

    if let Some(outcome) = &room.jobs.last {
        room.ui.add_space(12.0);
        if widgets::outcome(room.ui, outcome) {
            room.jobs.dismiss();
        }
    }

    room.ui.add_space(16.0);
    widgets::caution(
        room.ui,
        "Read the copy before you send it. Onionskin checks that no text is \
         left in the file, and it cannot check that you named everything that \
         should have gone.",
    );
}

fn millimetres(ui: &mut egui::Ui, value: &mut f64) {
    ui.add(
        egui::DragValue::new(value)
            .speed(0.5)
            .range(0.0..=2000.0)
            .suffix(" mm"),
    );
}

/// The words actually typed in.
///
/// An empty row is somebody who has clicked "Another" and not typed yet. Sent
/// through as it stands it would match every word on the page — which on this
/// screen means handing back a document with nothing left in it.
fn named(state: &State) -> Vec<String> {
    state
        .words
        .iter()
        .filter(|words| !words.trim().is_empty())
        .cloned()
        .collect()
}

fn take_them_out(state: &mut State, room: &mut Room) {
    let Some(document) = state.document.clone() else {
        return;
    };
    let words = named(state);
    let boxes = state.boxes.clone();
    let (pad_mm, page, dpi) = (state.pad_mm, state.page, state.dpi);
    let output = state
        .output
        .clone()
        .unwrap_or_else(|| beside(&document, "-redacted"));

    // Never over the document it came from: the original is the only record of
    // what was taken out, and somebody who redacts in place has destroyed the
    // thing they will need when they are asked what was removed.
    if super::same_file(&document, &output) {
        room.jobs.refuse(
            "That would write the redacted copy over the document it came from, and \
             the original is the only record of what was taken out.\n\nChoose another \
             name under 'Write the copy to hand over to'."
                .to_string(),
        );
        return;
    }

    room.previews.forget(&output);
    room.jobs.start("Taking them out", move |report| {
        let mut areas: Vec<onionskin::redact::Area> = boxes
            .iter()
            .map(|area| onionskin::redact::Area {
                page,
                x_mm: area.x_mm,
                y_mm: area.y_mm,
                width_mm: area.width_mm,
                height_mm: area.height_mm,
            })
            .collect();

        // The words are found in the document's OWN TEXT, on every page.
        //
        // Not by reading a picture of one page, which is what this did at
        // first: it drew the chosen page, ran the letter reader over it, and
        // marked the box round the matched token. Three things were wrong with
        // that and each of them ends the same way. It looked at one page, so
        // the named word stayed in the clear on all the others. It covered the
        // token, so `Salary` blacked out the label and left `84000 per annum`
        // beside it. And it went through an OCR pass that can misread, on a
        // file that knows perfectly well where its own characters are.
        //
        // `cover` next door has no choice — it marks up a sheet that may only
        // exist as a scan. A redaction is of a file, and "the reader did not
        // spot that one" is not a way to leave a salary in a document somebody
        // is about to send.
        let mut covered = Vec::new();
        if !words.is_empty() {
            report.saying("Looking through the document…");
            let found = match onionskin::redact::lines_carrying(&document, &words, pad_mm) {
                Ok(found) => found,
                Err(why) => return Outcome::refused(why.to_string()),
            };
            if found.from_a_scan {
                return Outcome::refused(
                    "This document carries no text — it is a picture of a page, so \
                     there are no words in it to search for.\n\nGive a rectangle \
                     instead, under 'Or a rectangle, measured'."
                        .to_string(),
                );
            }
            if !found.missing.is_empty() {
                return Outcome::refused(format!(
                    "Nothing in this document reads as {}, so nothing would be taken \
                     out — and a file that has had nothing taken out of it must not be \
                     handed over as though it had.\n\nCheck the spelling, or give a \
                     rectangle instead.",
                    found
                        .missing
                        .iter()
                        .map(|word| format!("'{word}'"))
                        .collect::<Vec<_>>()
                        .join(" or ")
                ));
            }
            areas.extend(found.areas.iter().copied());
            covered = found.covered;
        }

        report.saying("Drawing the pages and checking nothing is left…");
        match onionskin::redact::redact(&document, &output, &areas, dpi) {
            Ok(done) => {
                // What actually went, in the words that were on the page. The
                // person handing this over is the only one who can tell whether
                // it was enough, and they cannot tell that from a count.
                let mut notes: Vec<String> = covered
                    .iter()
                    .map(|gone: &onionskin::redact::Covered| {
                        format!("Page {}: {}", gone.page, gone.line)
                    })
                    .collect();
                notes.extend(done.describe());
                Outcome::Done {
                    message: format!(
                        "{} area{} taken out. Read the copy before you send it.",
                        done.areas,
                        if done.areas == 1 { "" } else { "s" }
                    ),
                    wrote: vec![output],
                    notes,
                }
            }
            Err(why) => Outcome::refused(why.to_string()),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An empty row is somebody part-way through typing, and matching the
    /// empty string would match every word on the page — which on this screen
    /// means blacking out the whole document.
    #[test]
    fn a_row_nobody_has_typed_in_yet_is_not_a_word_to_take_out() {
        let state = State {
            words: vec!["Salary".into(), String::new(), "   ".into()],
            ..Default::default()
        };
        assert_eq!(named(&state), vec!["Salary".to_string()]);
    }

    /// The button stays off until there is something to do, because the one
    /// thing worse than refusing is writing a file somebody believes is
    /// redacted and is not.
    #[test]
    fn nothing_can_be_taken_out_until_there_is_a_document_and_something_to_take() {
        let mut state = State::default();
        assert!(state.document.is_none());
        assert!(named(&state).is_empty() && state.boxes.is_empty());

        state.document = Some(std::path::PathBuf::from("/tmp/offer.pdf"));
        assert!(
            named(&state).is_empty() && state.boxes.is_empty(),
            "a document on its own is not enough"
        );

        state.words = vec!["Salary".into()];
        assert!(!named(&state).is_empty());
    }
}
