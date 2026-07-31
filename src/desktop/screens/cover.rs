//! Printing solid over something on a sheet you have to hand over.
//!
//! A salary on a payslip, an address on a letter going to somebody else, a bank
//! detail on an invoice being filed. The document is otherwise fine and the one
//! thing that must not be read is four centimetres of it.
//!
//! Every other answer is worse. Editing the original means having it and being
//! allowed to change it. A marker pen goes through the paper and reads straight
//! off the back. A photocopy of a marked-up sheet is a photocopy of a marked-up
//! sheet. This lays solid toner over the words, in one pass, on the sheet
//! already in your hand.
//!
//! # What this is honest about
//!
//! Covering is not erasing. It hides the words from the eye and from a
//! photocopier; it does not take the ink off the paper, and a strong light
//! behind the sheet may still show them through. For anything that must not be
//! recoverable the answer is a fresh page without them, and the screen says so
//! above the button rather than below it.

use eframe::egui;

use super::{beside, Room};
use crate::job::Outcome;
use crate::widgets;

pub struct State {
    /// The sheet with something on it that must not be read.
    document: Option<std::path::PathBuf>,
    output: Option<std::path::PathBuf>,
    /// Words to cover, found on the page rather than measured.
    words: Vec<String>,
    /// Boxes measured by hand, for anything that is not words.
    boxes: Vec<Box>,
    /// How far beyond the words to cover.
    pad_mm: f64,
    page: usize,
}

/// One rectangle of the paper, in millimetres from the top-left.
#[derive(Clone, Copy)]
struct Box {
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
            // A millimetre, the same as the command line. Enough to take the
            // tails of letters with it; not so much that the line above goes
            // too.
            pad_mm: 1.0,
            page: 1,
        }
    }
}

pub fn show(state: &mut State, room: &mut Room) {
    widgets::title(
        room.ui,
        "Cover something up",
        "Print solid over what must not be read, on the sheet you already have.",
    );

    widgets::hint(
        room.ui,
        "Name the words and Onionskin finds them on the page — no measuring. \
         Or give a rectangle, for anything that is not words.",
    );
    room.ui.add_space(10.0);

    widgets::file_row(
        room.ui,
        room.picker,
        "The sheet",
        &mut state.document,
        &[
            "pdf", "png", "jpg", "jpeg", "tif", "tiff", "bmp", "docx", "odt",
        ],
        room.dropped,
    );
    widgets::save_row(
        room.ui,
        room.picker,
        "Write the covering delta to",
        &mut state.output,
        "covered.pdf",
        &["pdf"],
        "beside the sheet, as NAME-covered.pdf",
    );

    room.ui.add_space(8.0);
    room.ui
        .label(egui::RichText::new("Words to cover").strong());
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
    widgets::hint(
        room.ui,
        "Whatever the words sit on is covered, plus a little either side.",
    );

    room.ui.add_space(8.0);
    room.ui.collapsing("Or a rectangle, measured", |ui| {
        widgets::hint(
            ui,
            "Millimetres from the top-left of the paper — for a photograph, a \
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
            state.boxes.push(Box {
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

    // Above the button, not below it: somebody deciding whether to cover a
    // salary or print a fresh page needs this while they are still deciding.
    room.ui.add_space(8.0);
    widgets::hint(
        room.ui,
        "Covering is not erasing. Solid toner hides the words from the eye and \
         from a photocopier; it does not take the ink off the paper, and a \
         strong light behind the sheet may still show them through. For \
         anything that must not be recoverable, print a fresh page without \
         them — and if what you are handing over is a file rather than a \
         sheet, use 'Take something out for good' instead: a black rectangle \
         drawn in a PDF leaves the words in it.",
    );

    room.ui.add_space(10.0);
    let ready = state.document.is_some() && !(named(state).is_empty() && state.boxes.is_empty());
    if room
        .ui
        .add_enabled(
            ready && !room.jobs.busy(),
            egui::Button::new(egui::RichText::new("Cover them").strong()),
        )
        .clicked()
    {
        cover(state, room);
    }
    if !ready {
        widgets::hint(
            room.ui,
            "Give the sheet, and either some words or a rectangle.",
        );
    }

    if let Some(outcome) = &room.jobs.last {
        room.ui.add_space(12.0);
        if widgets::outcome(room.ui, outcome) {
            room.jobs.dismiss();
        }
    }
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
/// through as it stands it would cover the whole page, every word matching the
/// empty string — which is the single worst thing this screen could do.
fn named(state: &State) -> Vec<String> {
    state
        .words
        .iter()
        .filter(|words| !words.trim().is_empty())
        .cloned()
        .collect()
}

fn cover(state: &mut State, room: &mut Room) {
    let Some(document) = state.document.clone() else {
        return;
    };
    let words = named(state);
    let boxes = state.boxes.clone();
    let pad_mm = state.pad_mm;
    let page = state.page;
    let output = state
        .output
        .clone()
        .unwrap_or_else(|| beside(&document, "-covered"));

    room.jobs.start("Covering them", move |report| {
        let mut areas: Vec<(f64, f64, f64, f64)> = boxes
            .iter()
            .map(|area| (area.x_mm, area.y_mm, area.width_mm, area.height_mm))
            .collect();

        if !words.is_empty() {
            report.saying("Reading the page…");
            let (gray, registration) = match onionskin::recipe::draw_page(&document, page) {
                Ok(both) => both,
                Err(why) => return Outcome::refused(why),
            };
            let Some((text, _)) = onionskin::typeface::read_and_match_in(&gray, &registration)
            else {
                return Outcome::refused(
                    "There is no font on this machine to read the page against, \
                     so Onionskin cannot find the words. Install a common face — \
                     DejaVu, Liberation — or give a rectangle instead."
                        .to_string(),
                );
            };
            for wanted in &words {
                let found = onionskin::anchor::boxes_for(&text, wanted);
                if found.is_empty() {
                    return Outcome::refused(format!(
                        "Nothing on the page reads as '{wanted}'. Read the page \
                         to see what is actually there — a scan is read letter \
                         by letter, and a smudged one may not say quite what you \
                         expect."
                    ));
                }
                // Every one of them, unlike a correction: covering two of
                // something is at worst too careful, and leaving one showing is
                // the failure this screen exists to prevent.
                for rect in found {
                    areas.push((
                        rect.x_mm - pad_mm,
                        rect.y_mm - pad_mm,
                        rect.width_mm + pad_mm * 2.0,
                        rect.height_mm + pad_mm * 2.0,
                    ));
                }
            }
        }

        report.saying("Writing the delta…");
        let shapes: Vec<(usize, onionskin::pdf::PlacedShape)> = areas
            .iter()
            .map(|(x_mm, y_mm, width_mm, height_mm)| {
                (
                    page,
                    onionskin::pdf::PlacedShape {
                        drawing: onionskin::pdf::Drawing::Rect {
                            x_mm: *x_mm,
                            y_mm: *y_mm,
                            width_mm: *width_mm,
                            height_mm: *height_mm,
                            radius_mm: 0.0,
                        },
                        stroke: None,
                        fill: Some((0.0, 0.0, 0.0)),
                        width_mm: 0.0,
                        dash_mm: None,
                    },
                )
            })
            .collect();

        let options = onionskin::settings::load()
            .defaults
            .over(onionskin::pipeline::Options::default());
        match onionskin::pipeline::compose_run_drawing(
            &document,
            &[],
            &shapes,
            &output,
            None,
            &options,
        ) {
            Ok(outcome) if outcome.blocked() => Outcome::refused(
                outcome
                    .checks
                    .iter()
                    .map(|check| check.format())
                    .collect::<Vec<_>>()
                    .join("\n\n"),
            ),
            Ok(_) => Outcome::Done {
                message: format!("{} area(s) covered.", shapes.len()),
                wrote: vec![output],
                notes: vec!["Printing this lays solid toner over those areas. It hides \
                     them from the eye and from a photocopier — it does not take \
                     the ink off the paper, and a strong light behind the sheet \
                     may still show them through. For anything that must not be \
                     recoverable, print a fresh page without them."
                    .to_string()],
            },
            Err(why) => Outcome::refused(why.to_string()),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The worst thing this screen could do is cover the whole page.
    ///
    /// An empty row is somebody who has clicked "Another" and not typed yet.
    /// Sent through as it stands, every word on the page matches the empty
    /// string and the sheet comes out solid black.
    #[test]
    fn an_empty_row_is_not_a_request_to_cover_everything() {
        let state = State {
            words: vec!["Salary".into(), String::new(), "   ".into()],
            ..Default::default()
        };
        assert_eq!(named(&state), vec!["Salary".to_string()]);

        // And the button stays off with nothing but empty rows.
        let nothing = State {
            document: Some(std::path::PathBuf::from("/tmp/a.pdf")),
            words: vec![String::new()],
            ..Default::default()
        };
        assert!(named(&nothing).is_empty());
        assert!(nothing.boxes.is_empty());
    }

    /// A rectangle on its own is a perfectly good request — a photograph, a
    /// signature, words the reader cannot make out.
    #[test]
    fn a_rectangle_needs_no_words() {
        let state = State {
            document: Some(std::path::PathBuf::from("/tmp/a.pdf")),
            words: vec![String::new()],
            boxes: vec![Box {
                x_mm: 20.0,
                y_mm: 40.0,
                width_mm: 60.0,
                height_mm: 8.0,
            }],
            ..Default::default()
        };
        assert!(named(&state).is_empty());
        assert!(!state.boxes.is_empty());
    }

    /// The padding starts where the command line's does, so the same sheet
    /// covered either way comes out the same.
    #[test]
    fn the_padding_matches_the_command_line() {
        assert_eq!(State::default().pad_mm, 1.0);
    }

    #[test]
    fn the_delta_lands_beside_the_sheet() {
        assert_eq!(
            beside(std::path::Path::new("/tmp/payslip.pdf"), "-covered"),
            std::path::Path::new("/tmp/payslip-covered.pdf")
        );
    }
}
