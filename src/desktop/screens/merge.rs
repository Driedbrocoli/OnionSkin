//! Several deltas, one pass through the printer.
//!
//! A day's work on one document arrives as more than one delta: the stamp, the
//! signature, the reference number out of a spreadsheet. Printing three of them
//! means feeding the same sheet through the printer three times, and every pass
//! is a chance to skew it, jam it, or pick up the one underneath — on a sheet
//! that already has the letterhead on it and cannot be reprinted.
//!
//! Merged first, it goes through once.

use eframe::egui;

use super::Room;
use crate::job::Outcome;
use crate::widgets;

#[derive(Default)]
pub struct State {
    /// The deltas, in the order they will be drawn.
    deltas: Vec<std::path::PathBuf>,
    output: Option<std::path::PathBuf>,
    /// One slot for adding the next one, emptied as soon as it is taken.
    next: Option<std::path::PathBuf>,
}

pub fn show(state: &mut State, room: &mut Room) {
    widgets::title(
        room.ui,
        "Several deltas, one pass",
        "Put them onto one page, so the sheet goes through the printer once.",
    );

    widgets::hint(
        room.ui,
        "Each delta keeps its own typeface, pictures and colours. Print the \
         merged file instead of the ones it was made from — printing both puts \
         the ink down twice.",
    );
    room.ui.add_space(10.0);

    // Anything dropped on this screen joins the list, however many at once,
    // which is the natural way to hand over four files.
    if !room.dropped.is_empty() {
        let taken: Vec<std::path::PathBuf> = room
            .dropped
            .iter()
            .filter(|path| {
                path.extension()
                    .and_then(|e| e.to_str())
                    .map(|e| e.eq_ignore_ascii_case("pdf"))
                    .unwrap_or(false)
            })
            .cloned()
            .collect();
        room.dropped.retain(|path| !taken.contains(path));
        state.deltas.extend(taken);
    }

    if widgets::file_row(
        room.ui,
        room.picker,
        "Add a delta",
        &mut state.next,
        &["pdf"],
        room.dropped,
    ) {
        if let Some(chosen) = state.next.take() {
            state.deltas.push(chosen);
        }
    }

    if state.deltas.is_empty() {
        widgets::hint(room.ui, "Nothing added yet. Two or more are needed.");
    } else {
        room.ui
            .label(egui::RichText::new("Drawn in this order").strong());
        let mut remove = None;
        let mut move_up = None;
        for (index, delta) in state.deltas.iter().enumerate() {
            room.ui.horizontal(|ui| {
                ui.label(egui::RichText::new(format!("{}.", index + 1)).weak());
                ui.label(
                    egui::RichText::new(
                        delta
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default(),
                    )
                    .monospace(),
                )
                .on_hover_text(delta.display().to_string());
                if index > 0 && ui.small_button("↑").clicked() {
                    move_up = Some(index);
                }
                if ui.small_button("×").clicked() {
                    remove = Some(index);
                }
            });
        }
        if let Some(index) = move_up {
            state.deltas.swap(index - 1, index);
        }
        if let Some(index) = remove {
            state.deltas.remove(index);
        }
        widgets::hint(
            room.ui,
            "A later one lands on top of an earlier one where they overlap.",
        );
    }

    room.ui.add_space(8.0);
    widgets::save_row(
        room.ui,
        room.picker,
        "Write the merged delta to",
        &mut state.output,
        "merged.pdf",
        &["pdf"],
        "beside the first, as NAME-merged.pdf",
    );

    room.ui.add_space(10.0);
    let ready = state.deltas.len() >= 2;
    if room
        .ui
        .add_enabled(
            ready && !room.jobs.busy(),
            egui::Button::new(egui::RichText::new("Merge them").strong()),
        )
        .clicked()
    {
        merge(state, room);
    }
    if state.deltas.len() == 1 {
        widgets::hint(room.ui, "One file is not a merge. Add another.");
    }

    if let Some(outcome) = &room.jobs.last {
        room.ui.add_space(12.0);
        if widgets::outcome(room.ui, outcome) {
            room.jobs.dismiss();
        }
    }
}

fn merge(state: &mut State, room: &mut Room) {
    let deltas = state.deltas.clone();
    let Some(first) = deltas.first().cloned() else {
        return;
    };
    let output = state
        .output
        .clone()
        .unwrap_or_else(|| beside(&first, "-merged"));

    room.jobs.start("Merging the deltas", move |report| {
        // Writing the merge over one of its own inputs destroys it, and with
        // every file in the same folder it is one wrong click away.
        for delta in &deltas {
            if same_file(&output, delta) {
                return Outcome::refused(format!(
                    "That would write the merge over {}, which is one of the \
                     deltas going into it. Choose somewhere else.",
                    delta.display()
                ));
            }
        }

        report.saying("Reading them all…");
        match onionskin::merge::merge(&deltas, &output, "Onionskin merged delta") {
            Ok(merged) => {
                let mut notes = vec!["Print this instead of the deltas it was made from — \
                     printing both it and them puts the ink down twice."
                    .to_string()];
                for repeat in merged.repeats() {
                    if let Some(same) = &repeat.same_as {
                        notes.push(format!(
                            "{} is the same file as {}. Everything in it will \
                             be printed twice, in the same place, which comes \
                             out heavier and blurred.",
                            repeat.path.display(),
                            same.display()
                        ));
                    }
                }
                for short in merged.short() {
                    notes.push(format!(
                        "{} runs out after {} page(s); the pages after that \
                         carry nothing from it.",
                        short.path.display(),
                        short.pages
                    ));
                }
                Outcome::Done {
                    message: merged.describe(),
                    wrote: vec![output],
                    notes,
                }
            }
            Err(e) => Outcome::refused(e.to_string()),
        }
    });
}

fn beside(source: &std::path::Path, tail: &str) -> std::path::PathBuf {
    let stem = source
        .file_stem()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "onionskin".to_string());
    source
        .parent()
        .unwrap_or(std::path::Path::new(""))
        .join(format!("{stem}{tail}.pdf"))
}

fn same_file(a: &std::path::Path, b: &std::path::Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Writing the merge over one of its own inputs destroys the thing being
    /// merged, and with every file in one folder it is a wrong click away.
    #[test]
    fn a_path_is_recognised_as_one_of_the_inputs_however_it_is_spelt() {
        let dir = tempfile::tempdir().unwrap();
        let delta = dir.path().join("stamp.pdf");
        std::fs::write(&delta, b"%PDF-1.4\n").unwrap();

        assert!(same_file(&delta, &delta));
        // The same file by a roundabout path.
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        let roundabout = dir.path().join("sub/../stamp.pdf");
        assert!(same_file(&roundabout, &delta));

        let other = dir.path().join("sign.pdf");
        std::fs::write(&other, b"%PDF-1.4\n").unwrap();
        assert!(!same_file(&other, &delta));
    }

    #[test]
    fn the_merge_lands_beside_the_first_delta() {
        assert_eq!(
            beside(std::path::Path::new("/tmp/stamp.pdf"), "-merged"),
            std::path::Path::new("/tmp/stamp-merged.pdf")
        );
    }

    /// Two files at least, because one file is not a merge — and the screen
    /// has to agree with the library, which refuses it.
    #[test]
    fn one_delta_is_not_a_merge() {
        let dir = tempfile::tempdir().unwrap();
        let only = dir.path().join("only.pdf");
        let out = dir.path().join("out.pdf");
        let sizes = [onionskin::geometry::PageSize::new(210.0, 297.0)];
        onionskin::pdf::write_delta(&only, &sizes, &[Vec::new()], "t", None).unwrap();

        assert!(onionskin::merge::merge(&[only], &out, "t").is_err());
    }
}
