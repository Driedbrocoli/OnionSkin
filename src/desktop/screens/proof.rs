//! Seeing the finished sheet before any paper is committed to it.
//!
//! A delta on its own is a nearly blank page: whether "Approved" lands inside
//! the box or across the line under it is not visible in it, and it is not
//! visible in the sheet either. Only in the two together — and until there was
//! a command for it, the only way to see the two together was to print them.
//!
//! This is the screen somebody should open before every job, and it is the
//! cheapest thing in the program: nothing goes near a printer, and the answer
//! is a PDF they already know how to look at.

use eframe::egui;

use super::{beside, same_file, Room};
use crate::job::Outcome;
use crate::widgets;

/// Everything here defaults to nothing chosen and the first colour, which is
/// red — so somebody who presses the button without reading anything gets the
/// answer they wanted.
#[derive(Default)]
pub struct State {
    /// The sheet as it is now: the PDF that was printed.
    sheet: Option<std::path::PathBuf>,
    /// The delta that would go onto it.
    delta: Option<std::path::PathBuf>,
    output: Option<std::path::PathBuf>,
    /// Fade the sheet almost away, as though holding it up to the light.
    tracing: bool,
    /// Which of the colours below the additions are drawn in.
    colour: usize,
}

/// What to draw the additions in. Red first because it is the one that reads
/// as "this is new" on a page of black text.
const COLOURS: &[(&str, &str)] = &[
    ("Red", "red"),
    ("Blue", "blue"),
    ("Green", "green"),
    ("Black", "black"),
];

pub fn show(state: &mut State, room: &mut Room) {
    widgets::title(
        room.ui,
        "See it before you print it",
        "The sheet and the delta together, in a PDF. No paper involved.",
    );

    widgets::hint(
        room.ui,
        "The sheet comes out in grey and what would be added on top of it in \
         colour, at the true size of the paper. This is the only honest preview \
         of an overlay, and it costs nothing to look at.",
    );
    room.ui.add_space(10.0);

    widgets::file_row(
        room.ui,
        room.picker,
        "The sheet as it is now",
        &mut state.sheet,
        &["pdf"],
        room.dropped,
    );
    widgets::file_row(
        room.ui,
        room.picker,
        "The delta that would go onto it",
        &mut state.delta,
        &["pdf"],
        room.dropped,
    );
    super::last_delta_button(room.ui, &mut state.delta);
    widgets::save_row(
        room.ui,
        room.picker,
        "Write the proof to",
        &mut state.output,
        "proof.pdf",
        &["pdf"],
        "beside the delta, as NAME-proof.pdf",
    );

    room.ui.horizontal(|ui| {
        ui.label("Draw the additions in");
        egui::ComboBox::from_id_salt("proof-colour")
            .selected_text(COLOURS[state.colour].0)
            .show_ui(ui, |ui| {
                for (index, (name, _)) in COLOURS.iter().enumerate() {
                    ui.selectable_value(&mut state.colour, index, *name);
                }
            });
    });
    room.ui.checkbox(&mut state.tracing, "Tracing paper");
    widgets::hint(
        room.ui,
        "Tracing paper fades the existing page almost away, leaving the \
         additions floating where they will land — the same thing as holding \
         the delta against a window with the original behind it.",
    );

    room.ui.add_space(10.0);
    let ready = state.sheet.is_some() && state.delta.is_some();
    if room
        .ui
        .add_enabled(
            ready && !room.jobs.busy(),
            egui::Button::new(egui::RichText::new("Make the proof").strong()),
        )
        .clicked()
    {
        make(state, room);
    }
    if !ready {
        widgets::hint(room.ui, "Give the sheet and the delta that goes onto it.");
    }

    if let Some(outcome) = &room.jobs.last {
        room.ui.add_space(12.0);
        if widgets::outcome(room.ui, outcome) {
            room.jobs.dismiss();
        }
    }
}

fn make(state: &mut State, room: &mut Room) {
    let (Some(sheet), Some(delta)) = (state.sheet.clone(), state.delta.clone()) else {
        return;
    };
    let output = state
        .output
        .clone()
        .unwrap_or_else(|| beside(&delta, "-proof"));
    let tracing = state.tracing;
    let colour = COLOURS[state.colour].1;

    room.jobs.start("Making the proof", move |report| {
        // Refused rather than allowed: writing the proof over the delta would
        // destroy the thing being previewed, and it is one wrong click away
        // when both files sit in the same folder.
        if same_file(&output, &delta) || same_file(&output, &sheet) {
            return Outcome::refused(
                "That would write the proof over one of the files it is made \
                 from. Choose somewhere else."
                    .to_string(),
            );
        }

        let added = match onionskin::document::parse_colour(colour) {
            Ok(colour) => colour,
            Err(e) => return Outcome::refused(e.to_string()),
        };
        let mut options = onionskin::proof::ProofOptions {
            added: [
                (added.0 * 255.0).round() as u8,
                (added.1 * 255.0).round() as u8,
                (added.2 * 255.0).round() as u8,
            ],
            ..Default::default()
        };
        if tracing {
            options = options.tracing();
        }

        report.saying("Drawing both pages…");
        match onionskin::proof::write_proof(&sheet, &delta, &output, &options) {
            Ok(pages) => Outcome::wrote(
                format!(
                    "{pages} page(s): the sheet in grey, and what would be added \
                     on top of it in {colour}.\n\nLook at it before you print \
                     the delta. Nothing here has gone near a printer."
                ),
                vec![output],
            ),
            Err(e) => Outcome::refused(e.to_string()),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every colour offered has to be one the parser accepts, or picking it
    /// produces a refusal instead of a proof.
    #[test]
    fn every_colour_on_the_list_can_actually_be_parsed() {
        for (shown, name) in COLOURS {
            assert!(
                onionskin::document::parse_colour(name).is_ok(),
                "'{shown}' offers '{name}', which the parser refuses"
            );
        }
    }

    /// Red is the one that reads as "this is new" on a page of black text, and
    /// it is what somebody who changes nothing gets.
    #[test]
    fn the_colour_nobody_chooses_is_red() {
        let state = State::default();
        assert_eq!(COLOURS[state.colour].0, "Red");
        assert!(!state.tracing);
    }

    #[test]
    fn the_proof_lands_beside_the_delta() {
        assert_eq!(
            beside(std::path::Path::new("/tmp/invoice-delta.pdf"), "-proof"),
            std::path::Path::new("/tmp/invoice-delta-proof.pdf")
        );
    }

    /// The proof written over the delta destroys the thing being previewed.
    #[test]
    fn a_proof_written_over_its_own_delta_is_recognised() {
        let dir = tempfile::tempdir().unwrap();
        let delta = dir.path().join("d.pdf");
        std::fs::write(&delta, b"%PDF-1.4\n").unwrap();
        assert!(same_file(&delta, &delta));
        assert!(!same_file(&delta, &dir.path().join("other.pdf")));
    }
}
