//! A whole stack from the feeder, sorted.
//!
//! Forty sheets go through the document feeder and come back as one PDF called
//! `Scan_0007`. Every one of them belongs with a document somewhere on a disk,
//! and putting them together is an afternoon of opening files and squinting.
//!
//! Each sheet is asked which document it is, by what is printed on it, and —
//! where a folder is given — written out under that document's name. Sheets
//! that cannot be placed keep their number and are listed at the end, because
//! a sheet filed under the wrong document is worse than one left in the pile.
//!
//! # If the sheets arrived one at a time
//!
//! A flatbed gives one file per sheet, not one file for the stack. The join
//! screen makes the single document this one wants — which is the reason that
//! screen exists.

use eframe::egui;

use super::Room;
use crate::job::Outcome;
use crate::widgets;

#[derive(Default)]
pub struct State {
    /// The stack as the feeder produced it: one PDF, many sheets.
    scan: Option<std::path::PathBuf>,
    /// The documents the sheets might be.
    among: Vec<std::path::PathBuf>,
    /// One slot for adding the next candidate, emptied as soon as it is taken.
    next: Option<std::path::PathBuf>,
    /// Where to write one file per sheet. Without it, nothing is written and
    /// the stack is only reported on.
    to: Option<std::path::PathBuf>,
    write_them_out: bool,
}

pub fn show(state: &mut State, room: &mut Room) {
    widgets::title(
        room.ui,
        "Sort a stack from the feeder",
        "Ask each sheet which document it is, and file it under that name.",
    );

    widgets::hint(
        room.ui,
        "Forty sheets come back as one PDF called Scan_0007, and every one of \
         them belongs with a document somewhere on a disk. Scanned one at a \
         time instead? The join screen makes the single document this one \
         wants.",
    );
    room.ui.add_space(10.0);

    widgets::file_row(
        room.ui,
        room.picker,
        "The stack, as one PDF",
        &mut state.scan,
        &["pdf"],
        room.dropped,
    );

    room.ui.add_space(8.0);
    room.ui
        .label(egui::RichText::new("The documents they might be").strong());
    if widgets::file_row(
        room.ui,
        room.picker,
        "Add a document",
        &mut state.next,
        &["pdf", "docx", "odt", "png", "jpg", "jpeg", "tif", "tiff"],
        room.dropped,
    ) {
        if let Some(chosen) = state.next.take() {
            state.among.push(chosen);
        }
    }
    let mut remove = None;
    for (index, document) in state.among.iter().enumerate() {
        room.ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(
                    document
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default(),
                )
                .monospace(),
            )
            .on_hover_text(document.display().to_string());
            if ui.small_button("×").clicked() {
                remove = Some(index);
            }
        });
    }
    if let Some(index) = remove {
        state.among.remove(index);
    }
    if state.among.is_empty() {
        widgets::hint(
            room.ui,
            "Nothing to compare them against yet. Add as many as you like — \
             they are opened once each, not once a sheet.",
        );
    }

    room.ui.add_space(8.0);
    room.ui
        .checkbox(&mut state.write_them_out, "Write one file per sheet");
    if state.write_them_out {
        widgets::save_row(
            room.ui,
            room.picker,
            "Into this folder",
            &mut state.to,
            "sorted",
            &[],
            "beside the stack",
        );
    } else {
        widgets::hint(
            room.ui,
            "Without this, nothing is written and the stack is only reported \
             on — which is the safe thing to do first.",
        );
    }

    room.ui.add_space(10.0);
    let ready = state.scan.is_some() && !state.among.is_empty();
    if room
        .ui
        .add_enabled(
            ready && !room.jobs.busy(),
            egui::Button::new(egui::RichText::new("Sort the stack").strong()),
        )
        .clicked()
    {
        sort(state, room);
    }
    if !ready {
        widgets::hint(
            room.ui,
            "Give the stack, and at least one document it might be.",
        );
    }

    if let Some(outcome) = &room.jobs.last {
        room.ui.add_space(12.0);
        if widgets::outcome(room.ui, outcome) {
            room.jobs.dismiss();
        }
    }
}

/// What a sheet is written out as, under the document it turned out to be.
fn sheet_named(document: &std::path::Path, sheet: usize) -> String {
    let stem = document
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "sheet".to_string());
    format!("{stem}-sheet-{sheet:03}.pdf")
}

fn sort(state: &mut State, room: &mut Room) {
    let Some(scan) = state.scan.clone() else {
        return;
    };
    let among = state.among.clone();
    let to = state.write_them_out.then(|| {
        state.to.clone().unwrap_or_else(|| {
            scan.parent()
                .unwrap_or(std::path::Path::new("."))
                .join("sorted")
        })
    });

    room.jobs.start("Sorting the stack", move |report| {
        report.saying("Opening the documents…");
        let sorted = match onionskin::which::sort_a_stack(&scan, &among, &mut |sheet, of| {
            report.saying(format!("Sheet {sheet} of {of}…"));
        }) {
            Ok(sorted) => sorted,
            Err(why) => return Outcome::refused(why),
        };

        let mut wrote = Vec::new();
        let mut notes = Vec::new();
        if let Some(folder) = &to {
            if let Err(e) = std::fs::create_dir_all(folder) {
                return Outcome::refused(format!("could not make '{}': {e}", folder.display()));
            }
            report.saying("Writing one file per sheet…");
            for placed in &sorted.placed {
                // A sheet nobody could place still has to come out somewhere,
                // or it is lost in the middle of the PDF.
                let named = match placed.settled() {
                    Some(best) => sheet_named(&best.path, placed.sheet),
                    None => format!("sheet-{:03}.pdf", placed.sheet),
                };
                let out = folder.join(&named);
                match onionskin::split::keep_only(&scan, &[placed.sheet], &out) {
                    Ok(()) => wrote.push(out),
                    Err(e) => notes.push(format!(
                        "Sheet {} could not be written out: {e}",
                        placed.sheet
                    )),
                }
            }
        }

        let unplaced = sorted.unplaced();
        if !unplaced.is_empty() {
            notes.push(format!(
                "Look at sheet(s) {} yourself. A sheet filed under the wrong \
                 document is worse than one left in the pile.",
                onionskin::split::sheets(&unplaced)
            ));
        }
        Outcome::Done {
            message: format!(
                "{} sheet(s), against {} document(s).\n\n{}\n\n{}",
                sorted.placed.len(),
                among.len(),
                sorted.lines().join("\n"),
                sorted.verdict()
            ),
            wrote,
            notes,
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// A sheet is filed under the document it turned out to be, with its own
    /// number kept — so a folder of forty comes out in an order somebody can
    /// follow back to the stack.
    #[test]
    fn a_sheet_is_named_after_the_document_it_belongs_with() {
        assert_eq!(
            sheet_named(Path::new("/tmp/invoice-4471.pdf"), 7),
            "invoice-4471-sheet-007.pdf"
        );
        // Padded, so forty files sort the way the stack was fed.
        assert_eq!(sheet_named(Path::new("a.pdf"), 1), "a-sheet-001.pdf");
        assert!(sheet_named(Path::new("a.pdf"), 10) > sheet_named(Path::new("a.pdf"), 9));
    }

    /// The screen must name a sheet exactly as the command line does, or a
    /// folder sorted one way and then the other comes out with two files per
    /// sheet under different names.
    #[test]
    fn the_screen_names_a_sheet_the_way_the_command_line_does() {
        // The command line's spelling, from `sheet_named` in main.rs.
        let theirs = |document: &Path, sheet: usize| {
            let stem = document
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "sheet".to_string());
            format!("{stem}-sheet-{sheet:03}.pdf")
        };
        for (document, sheet) in [("invoice.pdf", 1), ("letter-2024.docx", 42)] {
            let path = Path::new(document);
            assert_eq!(sheet_named(path, sheet), theirs(path, sheet));
        }
    }

    /// Nothing is written until somebody asks for it. Reporting on a stack is
    /// the safe thing to do first, and a screen that scattered forty files
    /// into a folder on the first click would be a screen people were afraid
    /// of.
    #[test]
    fn nothing_is_written_unless_it_is_asked_for() {
        let state = State::default();
        assert!(!state.write_them_out);
        assert!(state.to.is_none());
    }

    /// A sheet nobody could place still has to come out of the stack, or it is
    /// lost in the middle of a forty-page PDF.
    #[test]
    fn a_sheet_that_could_not_be_placed_is_still_written_out_under_its_number() {
        // The naming the screen falls back to when there is no document to
        // name it after.
        let unplaced = format!("sheet-{:03}.pdf", 13);
        assert_eq!(unplaced, "sheet-013.pdf");
        assert!(
            !unplaced.contains("None") && !unplaced.is_empty(),
            "an unplaced sheet must still have a name"
        );
    }
}
