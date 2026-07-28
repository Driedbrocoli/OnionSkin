//! Several PDFs, one after another.
//!
//! The other half of the merge screen. That one puts three files onto one
//! sheet; this puts three files onto three sheets, in order.
//!
//! It is here because almost everything upstream of this program produces one
//! page at a time. A flatbed scanner does; so does a phone. Twenty sheets
//! scanned is twenty files, and the stack reader wants one document with twenty
//! pages in it — so without this, the whole stack workflow was out of reach
//! unless you already had some other PDF tool, which is exactly the assumption
//! this program exists to avoid.

use eframe::egui;

use super::{beside, same_file, Room};
use crate::job::Outcome;
use crate::widgets;

#[derive(Default)]
pub struct State {
    /// The files, in the order their pages will come out.
    files: Vec<std::path::PathBuf>,
    output: Option<std::path::PathBuf>,
    /// One slot for adding the next one, emptied as soon as it is taken.
    next: Option<std::path::PathBuf>,
}

pub fn show(state: &mut State, room: &mut Room) {
    widgets::title(
        room.ui,
        "Several files, one after another",
        "Put their pages into one document, in order.",
    );

    widgets::hint(
        room.ui,
        "Scanned one sheet at a time? This makes the single document the rest \
         of Onionskin wants. Mixed paper is fine — each page keeps its own \
         size.",
    );
    room.ui.add_space(10.0);

    // Anything dropped joins the list, however many at once, which is the
    // natural way to hand over a folder of scans.
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
        state.files.extend(taken);
        // Dropped in a heap, they arrive in whatever order the desktop hands
        // them over, which is rarely page order.
        sort_by_number(&mut state.files);
    }

    if widgets::file_row(
        room.ui,
        room.picker,
        "Add a file",
        &mut state.next,
        &["pdf"],
        room.dropped,
    ) {
        if let Some(chosen) = state.next.take() {
            state.files.push(chosen);
        }
    }

    if state.files.is_empty() {
        widgets::hint(room.ui, "Nothing added yet. Two or more are needed.");
    } else {
        room.ui.label(egui::RichText::new("Page order").strong());
        let mut remove = None;
        let mut move_up = None;
        for (index, file) in state.files.iter().enumerate() {
            room.ui.horizontal(|ui| {
                ui.label(egui::RichText::new(format!("{}.", index + 1)).weak());
                ui.label(
                    egui::RichText::new(
                        file.file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default(),
                    )
                    .monospace(),
                )
                .on_hover_text(file.display().to_string());
                if index > 0 && ui.small_button("↑").clicked() {
                    move_up = Some(index);
                }
                if ui.small_button("×").clicked() {
                    remove = Some(index);
                }
            });
        }
        if let Some(index) = move_up {
            state.files.swap(index - 1, index);
        }
        if let Some(index) = remove {
            state.files.remove(index);
        }

        // A stack in the wrong order is far cheaper to notice here than after
        // twenty sheets have gone through the printer.
        if let Some(muddled) = out_of_order(&state.files) {
            widgets::hint(room.ui, &muddled);
            if room.ui.small_button("Put them in number order").clicked() {
                sort_by_number(&mut state.files);
            }
        }
    }

    room.ui.add_space(8.0);
    widgets::save_row(
        room.ui,
        room.picker,
        "Write the joined document to",
        &mut state.output,
        "joined.pdf",
        &["pdf"],
        "beside the first, as NAME-joined.pdf",
    );

    room.ui.add_space(10.0);
    let ready = state.files.len() >= 2;
    if room
        .ui
        .add_enabled(
            ready && !room.jobs.busy(),
            egui::Button::new(egui::RichText::new("Join them").strong()),
        )
        .clicked()
    {
        join(state, room);
    }
    if state.files.len() == 1 {
        widgets::hint(room.ui, "One file is not a join. Add another.");
    }

    if let Some(outcome) = &room.jobs.last {
        room.ui.add_space(12.0);
        if widgets::outcome(room.ui, outcome) {
            room.jobs.dismiss();
        }
    }
}

/// The last run of digits in a file's name.
///
/// The last, not the first: `2024-invoice-7.pdf` is number seven, and a scan
/// folder is full of names like that.
fn number_in(path: &std::path::Path) -> Option<u64> {
    let name = path.file_stem()?.to_string_lossy().into_owned();
    let digits: String = name
        .chars()
        .rev()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(char::is_ascii_digit)
        .collect();
    digits.chars().rev().collect::<String>().parse().ok()
}

/// Whether the list looks like it arrived in somebody's file manager's order
/// rather than its own.
fn out_of_order(files: &[std::path::PathBuf]) -> Option<String> {
    let numbers: Vec<u64> = files.iter().filter_map(|file| number_in(file)).collect();
    // Only worth saying when every one is numbered: "cover.pdf terms.pdf" is
    // not a numbered sequence and there is nothing to compare it against.
    if numbers.len() != files.len() || numbers.len() < 2 {
        return None;
    }
    let broken = numbers.windows(2).position(|pair| pair[0] > pair[1])?;
    Some(format!(
        "{} comes before {} here, but {} is the larger number — which is what \
         happens when a folder is sorted by name. A stack in that order is a \
         stack in the wrong order.",
        files[broken]
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        files[broken + 1]
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default(),
        numbers[broken],
    ))
}

/// Put numbered files in number order, and leave everything else alone.
///
/// Stable, so files with no number in them keep the order somebody put them
/// in — a cover sheet dropped in first stays first.
fn sort_by_number(files: &mut [std::path::PathBuf]) {
    if files.iter().any(|file| number_in(file).is_none()) {
        return;
    }
    files.sort_by_key(|file| number_in(file).unwrap_or(0));
}

fn join(state: &mut State, room: &mut Room) {
    let files = state.files.clone();
    let Some(first) = files.first().cloned() else {
        return;
    };
    let output = state
        .output
        .clone()
        .unwrap_or_else(|| beside(&first, "-joined"));

    room.jobs.start("Joining the files", move |report| {
        // Writing the join over one of its own inputs destroys it, and with
        // every file in the same folder it is one wrong click away.
        for file in &files {
            if same_file(&output, file) {
                return Outcome::refused(format!(
                    "That would write the joined document over {}, which is one \
                     of the files going into it. Choose somewhere else.",
                    file.display()
                ));
            }
        }

        report.saying("Reading them all…");
        match onionskin::join::join(&files, &output, "Onionskin joined document") {
            Ok(joined) => {
                let mut notes = Vec::new();
                if let Some(muddled) = joined.out_of_order() {
                    notes.push(muddled);
                }
                for (path, what) in &joined.left_behind {
                    notes.push(format!(
                        "{} has {what}, which live above the page and cannot \
                         come along. The pages themselves are all here.",
                        path.display()
                    ));
                }
                Outcome::Done {
                    message: joined.describe(),
                    wrote: vec![output],
                    notes,
                }
            }
            Err(e) => Outcome::refused(e.to_string()),
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    /// Writing the join over one of its own inputs destroys the thing being
    /// joined, and with every file in one folder it is a wrong click away.
    #[test]
    fn a_path_is_recognised_as_one_of_the_inputs_however_it_is_spelt() {
        let dir = tempfile::tempdir().unwrap();
        let page = dir.path().join("page-1.pdf");
        std::fs::write(&page, b"%PDF-1.4\n").unwrap();

        assert!(same_file(&page, &page));
        std::fs::create_dir(dir.path().join("sub")).unwrap();
        assert!(same_file(&dir.path().join("sub/../page-1.pdf"), &page));

        let other = dir.path().join("page-2.pdf");
        std::fs::write(&other, b"%PDF-1.4\n").unwrap();
        assert!(!same_file(&other, &page));
    }

    #[test]
    fn the_joined_document_lands_beside_the_first_file() {
        assert_eq!(
            beside(Path::new("/tmp/page-1.pdf"), "-joined"),
            Path::new("/tmp/page-1-joined.pdf")
        );
    }

    /// A file manager sorts page-10 before page-2, and files dropped in a heap
    /// arrive in that order. A stack in that order is a stack in the wrong
    /// order, and it is far cheaper to notice here than at the printer.
    #[test]
    fn files_numbered_out_of_order_are_noticed_and_can_be_put_right() {
        let mut files: Vec<PathBuf> = ["page-1.pdf", "page-10.pdf", "page-2.pdf"]
            .iter()
            .map(PathBuf::from)
            .collect();
        let said = out_of_order(&files).expect("10 before 2 went unnoticed");
        assert!(said.contains("page-10.pdf"), "{said}");
        assert!(said.contains("page-2.pdf"), "{said}");

        sort_by_number(&mut files);
        assert_eq!(
            files,
            vec![
                PathBuf::from("page-1.pdf"),
                PathBuf::from("page-2.pdf"),
                PathBuf::from("page-10.pdf")
            ]
        );
        assert_eq!(out_of_order(&files), None, "sorting did not settle it");
    }

    /// Files that are not a numbered sequence are not second-guessed: "cover,
    /// terms, appendix" is the order somebody chose, and reordering it by a
    /// number that is not there would be worse than useless.
    #[test]
    fn files_that_are_not_numbered_are_left_exactly_as_they_were_given() {
        let given: Vec<PathBuf> = ["terms.pdf", "cover.pdf", "appendix.pdf"]
            .iter()
            .map(PathBuf::from)
            .collect();
        assert_eq!(out_of_order(&given), None);

        let mut mine = given.clone();
        sort_by_number(&mut mine);
        assert_eq!(mine, given, "an unnumbered list was reordered");

        // And one number among many names settles nothing either.
        let mixed: Vec<PathBuf> = ["cover.pdf", "page-2.pdf", "page-10.pdf"]
            .iter()
            .map(PathBuf::from)
            .collect();
        assert_eq!(out_of_order(&mixed), None);
    }

    /// The number is the last run of digits, because a scan folder is full of
    /// names like "2024-invoice-7.pdf" and the year is not the page number.
    #[test]
    fn the_number_is_the_last_one_in_the_name() {
        assert_eq!(number_in(Path::new("2024-invoice-7.pdf")), Some(7));
        assert_eq!(number_in(Path::new("page-10.pdf")), Some(10));
        assert_eq!(number_in(Path::new("scan007.pdf")), Some(7));
        assert_eq!(number_in(Path::new("cover.pdf")), None);
    }

    /// Two files at least, because one file is not a join — and the screen has
    /// to agree with the library, which refuses it.
    #[test]
    fn one_file_is_not_a_join() {
        let dir = tempfile::tempdir().unwrap();
        let only = dir.path().join("only.pdf");
        let out = dir.path().join("out.pdf");
        let sizes = [onionskin::geometry::PageSize::new(210.0, 297.0)];
        onionskin::pdf::write_delta(&only, &sizes, &[Vec::new()], "t", None).unwrap();

        assert!(onionskin::join::join(&[only], &out, "t").is_err());
    }
}
