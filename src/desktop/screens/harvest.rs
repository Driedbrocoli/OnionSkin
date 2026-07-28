//! Filled-in forms, back into a spreadsheet.
//!
//! `batch` goes one way and this goes the other. The screen's job is to make the
//! two things somebody has to decide easy to decide: which labels to look for,
//! and which of the columns hold figures.
//!
//! The second one matters more than it sounds. "This column is money" is
//! knowledge that does not exist anywhere in the ink, and giving it changes a
//! column of `24O.OO` into a column of `240.00` — so it is a checkbox next to
//! the field rather than a spelling somebody has to learn.

use eframe::egui;

use super::Room;
use crate::job::Outcome;
use crate::widgets;

pub struct State {
    scan: Option<std::path::PathBuf>,
    output: Option<std::path::PathBuf>,
    fields: Vec<Column>,
    /// Stop after this many sheets, to see the shape of a run.
    only_first: bool,
    first: usize,
}

/// A column, and how to find it on the form.
#[derive(Clone, Default)]
struct Column {
    /// The heading, and the label to look for when nothing else is given.
    name: String,
    /// A different label on the form, where the heading is not what is printed.
    label: String,
    below: bool,
    number: bool,
}

impl Column {
    fn field(&self) -> Option<onionskin::harvest::Field> {
        let name = self.name.trim();
        if name.is_empty() {
            return None;
        }
        let label = match self.label.trim() {
            "" => name,
            other => other,
        };
        Some(onionskin::harvest::Field {
            name: name.to_string(),
            label: label.to_string(),
            put: match self.below {
                true => onionskin::harvest::Where::Below,
                false => onionskin::harvest::Where::After,
            },
            kind: match self.number {
                true => onionskin::harvest::Kind::Number,
                false => onionskin::harvest::Kind::Text,
            },
        })
    }
}

impl Default for State {
    fn default() -> Self {
        State {
            scan: None,
            output: None,
            fields: vec![Column::default()],
            only_first: false,
            first: 5,
        }
    }
}

impl State {
    /// The columns somebody has actually named.
    fn named(&self) -> Vec<onionskin::harvest::Field> {
        self.fields.iter().filter_map(Column::field).collect()
    }
}

pub fn show(state: &mut State, room: &mut Room) {
    widgets::title(
        room.ui,
        "Forms back into a spreadsheet",
        "The sheets came back filled in. Read them, instead of typing them.",
    );

    widgets::hint(
        room.ui,
        "Name a column after the label printed beside it on the form. The value \
         is whatever follows that label, stopping where the next label starts.",
    );
    room.ui.add_space(10.0);

    widgets::file_row(
        room.ui,
        room.picker,
        "The scanned stack",
        &mut state.scan,
        &["pdf", "png", "jpg", "jpeg", "tif", "tiff", "bmp"],
        room.dropped,
    );
    widgets::save_row(
        room.ui,
        room.picker,
        "Write the spreadsheet to",
        &mut state.output,
        "harvested.csv",
        &["csv"],
        "beside the scan, as NAME-harvested.csv",
    );

    room.ui.add_space(8.0);
    room.ui.label(egui::RichText::new("The columns").strong());
    let mut remove = None;
    for (index, column) in state.fields.iter_mut().enumerate() {
        room.ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut column.name)
                    .hint_text("Amount")
                    .desired_width(140.0),
            );
            ui.checkbox(&mut column.number, "a number").on_hover_text(
                "Says this column holds figures, which the marks on the page \
                     cannot say. A ring is a nought and a letter O at once, and \
                     knowing it is money is what settles it.",
            );
            ui.checkbox(&mut column.below, "under the label")
                .on_hover_text("The value is on the line below its caption, not beside it.");
            if ui.small_button("×").clicked() {
                remove = Some(index);
            }
        });
        // The long label, only when it is wanted.
        if !column.name.trim().is_empty() {
            room.ui.horizontal(|ui| {
                ui.add_space(12.0);
                ui.add(
                    egui::TextEdit::singleline(&mut column.label)
                        .hint_text(format!("labelled '{}' on the form", column.name.trim()))
                        .desired_width(300.0),
                );
            });
        }
    }
    if let Some(index) = remove {
        state.fields.remove(index);
    }
    if state.fields.is_empty() || room.ui.small_button("Another column").clicked() {
        state.fields.push(Column::default());
    }

    room.ui.add_space(8.0);
    room.ui.collapsing("More", |ui| {
        ui.checkbox(&mut state.only_first, "Read only the first few sheets");
        ui.add_enabled_ui(state.only_first, |ui| {
            ui.horizontal(|ui| {
                ui.label("The first");
                ui.add(egui::DragValue::new(&mut state.first).range(1..=9999));
                ui.label("sheet(s)");
            });
        });
        widgets::hint(
            ui,
            "So the shape of a run can be seen without waiting for two hundred.",
        );
    });

    room.ui.add_space(8.0);
    widgets::hint(
        room.ui,
        "Handwriting is not read. Onionskin matches printed letter shapes \
         against the fonts on this machine, and a signature has nothing to \
         match — so a hand-filled form comes back mostly empty, and that is the \
         honest answer rather than a guess.",
    );

    room.ui.add_space(10.0);
    let ready = state.scan.is_some() && !state.named().is_empty();
    if room
        .ui
        .add_enabled(
            ready && !room.jobs.busy(),
            egui::Button::new(egui::RichText::new("Read the stack").strong()),
        )
        .clicked()
    {
        harvest(state, room);
    }
    if !ready {
        widgets::hint(
            room.ui,
            "Give the scan, and at least one column to fill in.",
        );
    }

    if let Some(outcome) = &room.jobs.last {
        room.ui.add_space(12.0);
        if widgets::outcome(room.ui, outcome) {
            room.jobs.dismiss();
        }
    }
}

fn harvest(state: &mut State, room: &mut Room) {
    let Some(scan) = state.scan.clone() else {
        return;
    };
    let fields = state.named();
    let first = state.only_first.then_some(state.first);
    let output = state
        .output
        .clone()
        .unwrap_or_else(|| beside_csv(&scan, "-harvested"));

    room.jobs.start("Reading the stack", move |report| {
        let pages = match onionskin::recipe::pages_in(&scan) {
            Ok(pages) => pages,
            Err(why) => return Outcome::refused(why),
        };
        let wanted = first.unwrap_or(pages).min(pages);
        if wanted == 0 {
            return Outcome::refused(format!("{} has no pages on it.", scan.display()));
        }

        let mut sheets = Vec::with_capacity(wanted);
        for page in 1..=wanted {
            report.saying(format!("Reading sheet {page} of {wanted}…"));
            let (gray, registration) = match onionskin::recipe::draw_page(&scan, page) {
                Ok(both) => both,
                Err(why) => return Outcome::refused(why),
            };
            let Some((text, _)) = onionskin::typeface::read_and_match_in(&gray, &registration)
            else {
                return Outcome::refused(
                    "There is no font on this machine to read the pages against, \
                     so Onionskin cannot find the labels. Install a common face \
                     — DejaVu, Liberation — and try again."
                        .to_string(),
                );
            };
            sheets.push(onionskin::harvest::Sheet {
                page,
                values: onionskin::harvest::pick_from(&text, &fields),
            });
        }

        let harvest = onionskin::harvest::Harvest { fields, sheets };
        if let Err(why) = std::fs::write(&output, harvest.csv()) {
            return Outcome::refused(format!("{}: {why}", output.display()));
        }

        let mut notes = vec![harvest.verdict()];
        let gaps = harvest.gaps();
        if !gaps.is_empty() {
            notes.push("Nothing to put in these, so they need checking on the paper:".to_string());
            notes.extend(gaps.iter().take(20).map(|line| line.trim().to_string()));
            if gaps.len() > 20 {
                notes.push(format!("... and {} more.", gaps.len() - 20));
            }
        }
        Outcome::Done {
            message: harvest.verdict(),
            wrote: vec![output],
            notes,
        }
    });
}

/// Beside the scan, but a spreadsheet rather than a PDF.
///
/// The shared `beside` gives everything a `.pdf`, because everything else this
/// program writes is one.
fn beside_csv(source: &std::path::Path, tail: &str) -> std::path::PathBuf {
    let stem = source
        .file_stem()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "onionskin".to_string());
    source
        .parent()
        .unwrap_or(std::path::Path::new(""))
        .join(format!("{stem}{tail}.csv"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An empty row is somebody who clicked "Another column" and has not typed
    /// yet. Sent through it would be a field with no name, which is not a
    /// column and would be refused after the whole stack had been read.
    #[test]
    fn an_empty_row_is_not_a_column() {
        let state = State {
            fields: vec![
                Column {
                    name: "Amount".into(),
                    ..Default::default()
                },
                Column::default(),
                Column {
                    name: "   ".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        assert_eq!(state.named().len(), 1);
        assert_eq!(state.named()[0].name, "Amount");
    }

    /// A column with no long label looks for its own heading, which is the
    /// usual case and the reason naming a column is usually one word.
    #[test]
    fn a_column_looks_for_its_own_name_unless_told_otherwise() {
        let plain = Column {
            name: "Amount".into(),
            ..Default::default()
        };
        let field = plain.field().unwrap();
        assert_eq!(field.label, "Amount");

        let relabelled = Column {
            name: "Name".into(),
            label: "Full name of applicant".into(),
            ..Default::default()
        };
        assert_eq!(relabelled.field().unwrap().label, "Full name of applicant");
    }

    /// The two checkboxes reach the field, because they are the only two things
    /// on this screen that change what comes out.
    #[test]
    fn the_checkboxes_reach_the_field() {
        let column = Column {
            name: "Total".into(),
            below: true,
            number: true,
            ..Default::default()
        };
        let field = column.field().unwrap();
        assert_eq!(field.put, onionskin::harvest::Where::Below);
        assert_eq!(field.kind, onionskin::harvest::Kind::Number);
        // And what it says of itself matches, so the report reads the same way
        // the screen was filled in.
        assert!(field.describe().contains("number"));
        assert!(field.describe().contains("under it"));
    }

    /// The spreadsheet lands beside the scan, and is a spreadsheet.
    #[test]
    fn the_spreadsheet_lands_beside_the_scan() {
        assert_eq!(
            beside_csv(std::path::Path::new("/tmp/scans.pdf"), "-harvested"),
            std::path::Path::new("/tmp/scans-harvested.csv")
        );
    }
}
