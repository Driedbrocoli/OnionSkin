//! Fixing a mistake on a page that is already printed.
//!
//! The wrong figure went out on two hundred invoices, or a name is misspelt on
//! a certificate somebody is waiting for. The whole document is fine apart from
//! four characters, and the answer everywhere else is to print it again.
//!
//! Onionskin has both halves of something better — solid toner over words that
//! should not be read, and new words put down — and what it did not have was
//! the join. The join is the hard part by hand: measure the box, guess the type
//! size, name the face, and get all three right on a sheet there is only one
//! of. The page can answer all three, so this screen asks for none of them.
//!
//! # What this is honest about
//!
//! Covering is not erasing. Solid toner hides the old text from the eye and
//! from a photocopier; it does not take it off the paper, and a strong light
//! behind the sheet may still show it through. For anything that must not be
//! recoverable the answer is still a fresh page, and the screen says so before
//! the button rather than after.

use eframe::egui;

use super::{beside, Room};
use crate::job::Outcome;
use crate::widgets;
use onionskin::document::Item;

pub struct State {
    /// The sheet with the mistake on it.
    document: Option<std::path::PathBuf>,
    output: Option<std::path::PathBuf>,
    /// What is wrong, and what it should say instead.
    mistakes: Vec<(String, String)>,
    page: usize,
    /// Somebody overruling what the page said of itself, where they want to.
    size_pt: f64,
    match_the_page: bool,
}

impl Default for State {
    fn default() -> Self {
        State {
            document: None,
            output: None,
            // One row to type into, because an empty list is a screen with
            // nothing on it and no hint about what to do.
            mistakes: vec![(String::new(), String::new())],
            page: 1,
            size_pt: 11.0,
            match_the_page: true,
        }
    }
}

pub fn show(state: &mut State, room: &mut Room) {
    widgets::title(
        room.ui,
        "Fix a mistake on a printed page",
        "Cover the wrong words and set the right ones in their place.",
    );

    widgets::hint(
        room.ui,
        "Onionskin reads the page, finds the words, and takes the size and the \
         face off the line they are on — so the new words sit level with what is \
         beside them without anybody measuring anything.",
    );
    room.ui.add_space(10.0);

    widgets::file_row(
        room.ui,
        room.picker,
        "The sheet with the mistake",
        &mut state.document,
        &[
            "pdf", "png", "jpg", "jpeg", "tif", "tiff", "bmp", "docx", "odt",
        ],
        room.dropped,
    );
    widgets::save_row(
        room.ui,
        room.picker,
        "Write the correction delta to",
        &mut state.output,
        "corrected.pdf",
        &["pdf"],
        "beside the sheet, as NAME-corrected.pdf",
    );

    room.ui.add_space(8.0);
    room.ui.label(egui::RichText::new("What is wrong").strong());
    let mut remove = None;
    for (index, (was, now)) in state.mistakes.iter_mut().enumerate() {
        room.ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(was)
                    .hint_text("as printed")
                    .desired_width(170.0),
            );
            ui.label("→");
            ui.add(
                egui::TextEdit::singleline(now)
                    .hint_text("what it should say")
                    .desired_width(170.0),
            );
            if ui.small_button("×").clicked() {
                remove = Some(index);
            }
        });
    }
    if let Some(index) = remove {
        state.mistakes.remove(index);
    }
    if state.mistakes.is_empty() || room.ui.small_button("Another").clicked() {
        state.mistakes.push((String::new(), String::new()));
    }
    widgets::hint(
        room.ui,
        "Give enough of the line to pick it out. A phrase that is on the page \
         twice is refused rather than guessed at — covering the wrong \"Total\" \
         cannot be undone.",
    );

    room.ui.add_space(8.0);
    room.ui.collapsing("The type size", |ui| {
        ui.checkbox(
            &mut state.match_the_page,
            "Take it off the line the mistake is on",
        );
        ui.add_enabled_ui(!state.match_the_page, |ui| {
            ui.horizontal(|ui| {
                ui.label("Set at");
                ui.add(
                    egui::DragValue::new(&mut state.size_pt)
                        .speed(0.5)
                        .range(4.0..=72.0)
                        .suffix(" pt"),
                );
            });
        });
        widgets::hint(
            ui,
            "Measured off the tallest letter on that line, which is a direct \
             measurement of that line — a heading at fourteen point and a body \
             at twelve average to something that is neither.",
        );
        ui.horizontal(|ui| {
            ui.label("Page");
            ui.add(egui::DragValue::new(&mut state.page).range(1..=999));
        });
    });

    // Said before the button, not after: somebody deciding whether to cover a
    // salary or reprint the page needs this while they are still deciding.
    room.ui.add_space(8.0);
    widgets::hint(
        room.ui,
        "Covering is not erasing. Solid toner hides the old words from the eye \
         and from a photocopier; it does not take the ink off the paper, and a \
         strong light behind the sheet may still show it through. For anything \
         that must not be recoverable, print a fresh page instead.",
    );

    room.ui.add_space(10.0);
    let ready = state.document.is_some() && !wanted(state).is_empty();
    if room
        .ui
        .add_enabled(
            ready && !room.jobs.busy(),
            egui::Button::new(egui::RichText::new("Plan the correction").strong()),
        )
        .clicked()
    {
        correct(state, room);
    }
    if !ready {
        widgets::hint(
            room.ui,
            "Give the sheet, and at least one thing to put right.",
        );
    }

    if let Some(outcome) = &room.jobs.last {
        room.ui.add_space(12.0);
        if widgets::outcome(room.ui, outcome) {
            room.jobs.dismiss();
        }
    }
}

/// The corrections that are actually filled in.
///
/// Half a row — words to replace and nothing to put there, or the other way
/// round — is somebody in the middle of typing, not a request.
fn wanted(state: &State) -> Vec<onionskin::correct::Mistake> {
    state
        .mistakes
        .iter()
        .filter(|(was, now)| !was.trim().is_empty() && !now.trim().is_empty())
        .map(|(was, now)| onionskin::correct::Mistake {
            was: was.clone(),
            now: now.clone(),
        })
        .collect()
}

fn correct(state: &mut State, room: &mut Room) {
    let Some(document) = state.document.clone() else {
        return;
    };
    let mistakes = wanted(state);
    let output = state
        .output
        .clone()
        .unwrap_or_else(|| beside(&document, "-corrected"));
    let page = state.page;
    let size = (!state.match_the_page).then_some(state.size_pt);

    room.jobs.start("Planning the correction", move |report| {
        report.saying("Reading the page…");
        let (gray, registration) = match onionskin::recipe::draw_page(&document, page) {
            Ok(both) => both,
            Err(why) => return Outcome::refused(why),
        };
        let Some((text, face)) = onionskin::typeface::read_and_match_in(&gray, &registration)
        else {
            return Outcome::refused(
                "There is no font on this machine to read the page against, so \
                 Onionskin cannot find the words to correct. Install a common \
                 face — DejaVu, Liberation — or cover and write by hand."
                    .to_string(),
            );
        };

        report.saying("Working out what to cover…");
        let planned = match onionskin::correct::plan(&text, face.as_ref(), &mistakes, size, None) {
            Ok(planned) => planned,
            Err(why) => return Outcome::refused(why.to_string()),
        };

        // Anything worth knowing before the paper goes in: a size that was
        // guessed rather than measured, and words that will run past where the
        // old ones ended.
        let mut notes = Vec::new();
        for fix in &planned {
            if !fix.size_measured {
                notes.push(format!(
                    "The size for \"{}\" was guessed from the height of the line \
                     rather than measured. Check it, or set one yourself.",
                    fix.was.trim()
                ));
            }
            if let Some(over) = fix.wider_than_what_it_replaces() {
                notes.push(format!(
                    "\"{}\" is about {over:.0} mm wider than what it replaces, so \
                     it will run to the right of where those words ended.",
                    fix.now.trim()
                ));
            }
        }
        notes.push(
            "Covering is not erasing: a strong light behind the sheet may still \
             show the old words through. For anything that must not be \
             recoverable, print a fresh page."
                .to_string(),
        );

        report.saying("Writing the delta…");
        let shapes: Vec<(usize, onionskin::pdf::PlacedShape)> = planned
            .iter()
            .map(|fix| {
                (
                    page,
                    onionskin::pdf::PlacedShape {
                        drawing: onionskin::pdf::Drawing::Rect {
                            x_mm: fix.cover_mm.0,
                            y_mm: fix.cover_mm.1,
                            width_mm: fix.cover_mm.2,
                            height_mm: fix.cover_mm.3,
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
        // White, because they are printed onto the black that has just covered
        // the old words. Toner over toner is not white on paper — but the
        // reason the box is there at all is that the old words must not be
        // read, and words set in white on it are the only ones that can be.
        let items: Vec<Item> = planned
            .iter()
            .map(|fix| Item {
                id: 0,
                page,
                x_mm: fix.x_mm,
                y_mm: fix.baseline_mm,
                text: fix.now.clone(),
                size_pt: fix.size_pt,
                font: fix.font.clone(),
                width_mm: None,
                rotation_deg: 0.0,
                colour: "#FFFFFF".to_string(),
                leading: 1.2,
            })
            .collect();

        let options = onionskin::settings::load()
            .defaults
            .over(onionskin::pipeline::Options::default());
        match onionskin::pipeline::compose_run_drawing(
            &document, &items, &shapes, &output, None, &options,
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
                message: format!(
                    "{} correction(s):\n{}",
                    planned.len(),
                    planned
                        .iter()
                        .map(|fix| format!("  {}", fix.describe()))
                        .collect::<Vec<_>>()
                        .join("\n\n")
                ),
                wrote: vec![output],
                notes,
            },
            Err(why) => Outcome::refused(why.to_string()),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A row half typed into is somebody in the middle of typing, not a
    /// request. Sent through as it stands it would be refused with "does not
    /// say what to put there", about a row they had not finished.
    #[test]
    fn a_half_filled_row_is_not_a_correction() {
        let state = State {
            mistakes: vec![
                ("120.00".into(), "140.00".into()),
                ("Smith".into(), "  ".into()),
                (String::new(), "something".into()),
                (String::new(), String::new()),
            ],
            ..Default::default()
        };
        let asked = wanted(&state);
        assert_eq!(asked.len(), 1);
        assert_eq!(asked[0].was, "120.00");
        assert_eq!(asked[0].now, "140.00");
    }

    /// The default row is empty, so the button stays off until somebody has
    /// actually said what is wrong.
    #[test]
    fn the_screen_starts_with_nothing_to_do() {
        let state = State::default();
        assert!(wanted(&state).is_empty());
        assert!(state.match_the_page, "the page's own answer is the default");
    }

    #[test]
    fn the_delta_lands_beside_the_sheet() {
        assert_eq!(
            beside(std::path::Path::new("/tmp/invoice.pdf"), "-corrected"),
            std::path::Path::new("/tmp/invoice-corrected.pdf")
        );
    }

    /// The screen and the command line must plan the same correction from the
    /// same page, or one of them is lying about what will be printed.
    #[test]
    fn the_screen_asks_the_library_for_the_same_plan_the_command_line_does() {
        use onionskin::letters::{Letter, PageText, Rect, TextLine, Word};

        let letter = |c: char, x_mm: f64| {
            Letter::known(
                c,
                Rect {
                    x_mm,
                    y_mm: 197.0,
                    width_mm: 1.8,
                    height_mm: 3.0,
                },
                3.6,
            )
        };
        let word = |text: &str, x_mm: f64| Word {
            rect: Rect {
                x_mm,
                y_mm: 197.0,
                width_mm: text.len() as f64 * 2.1,
                height_mm: 3.0,
            },
            letters: text
                .chars()
                .enumerate()
                .map(|(n, c)| letter(c, x_mm + n as f64 * 2.1))
                .collect(),
        };
        let page = PageText {
            lines: vec![TextLine {
                rect: Rect {
                    x_mm: 20.0,
                    y_mm: 197.0,
                    width_mm: 40.0,
                    height_mm: 3.0,
                },
                baseline_mm: 200.0,
                words: vec![word("Total", 20.0), word("120.00", 45.0)],
            }],
            discarded: 0,
        };

        let mistakes = wanted(&State {
            mistakes: vec![("120.00".into(), "140.00".into())],
            ..Default::default()
        });
        let planned = onionskin::correct::plan(&page, None, &mistakes, None, None).unwrap();
        assert_eq!(planned.len(), 1);
        // On the same baseline as the label beside it, which is the whole
        // reason to read the page rather than ask for a measurement.
        assert!((planned[0].baseline_mm - 200.0).abs() < 0.5);
        assert!((planned[0].x_mm - 45.0).abs() < 0.5);
        assert!(planned[0].size_measured);
    }
}
