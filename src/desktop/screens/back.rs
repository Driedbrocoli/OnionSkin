//! Writing on the back of a printed sheet.
//!
//! The one thing this screen exists to get right is which way up the back comes
//! out. Nobody knows that about their own printer, there is no way to work it
//! out without printing one, and a guess is not a small error — it is every
//! sheet in the run at the wrong end of the paper, found after the run.
//!
//! So the question is on the screen rather than buried in a flag, the button
//! that answers it is next to it, and the answer is remembered the moment it is
//! given. Somebody who has printed the check sheet once should never see the
//! question again.

use eframe::egui;

use super::{beside, Room};
use crate::job::Outcome;
use crate::widgets;

pub struct State {
    document: Option<std::path::PathBuf>,
    output: Option<std::path::PathBuf>,
    /// One line per row, as it will be seen on the finished back.
    rows: Vec<Row>,
    feed: onionskin::duplex::Feed,
    /// The document is already printed on both sides.
    two_sided: bool,
    /// Just this sheet, rather than all of them.
    one_sheet: bool,
    sheet: usize,
    size_pt: f64,
}

/// Some words, and where they go on the back.
#[derive(Clone)]
struct Row {
    text: String,
    x_mm: f64,
    y_mm: f64,
}

impl Default for State {
    fn default() -> Self {
        State {
            document: None,
            output: None,
            rows: vec![Row {
                text: String::new(),
                x_mm: 20.0,
                y_mm: 40.0,
            }],
            // Whatever this machine was told, or the one most printers do.
            feed: onionskin::settings::load()
                .defaults
                .feed
                .as_deref()
                .and_then(onionskin::duplex::Feed::parse)
                .unwrap_or_default(),
            two_sided: false,
            one_sheet: false,
            sheet: 1,
            size_pt: 12.0,
        }
    }
}

impl State {
    /// The rows somebody has actually typed something into.
    fn written(&self) -> Vec<Row> {
        self.rows
            .iter()
            .filter(|row| !row.text.trim().is_empty())
            .cloned()
            .collect()
    }
}

pub fn show(state: &mut State, room: &mut Room) {
    widgets::title(
        room.ui,
        "The back of the sheet",
        "Terms, an address, \"continued overleaf\" — on the side that is blank.",
    );

    widgets::hint(
        room.ui,
        "Measured on the back as you will look at it, the right way up. \
         Onionskin turns it round if this printer needs it turned.",
    );
    room.ui.add_space(10.0);

    widgets::file_row(
        room.ui,
        room.picker,
        "The printed document",
        &mut state.document,
        &[
            "pdf", "png", "jpg", "jpeg", "tif", "tiff", "bmp", "docx", "odt",
        ],
        room.dropped,
    );
    widgets::save_row(
        room.ui,
        room.picker,
        "Write the delta to",
        &mut state.output,
        "back.pdf",
        &["pdf"],
        "beside the document, as NAME-back.pdf",
    );

    room.ui.add_space(8.0);
    room.ui
        .label(egui::RichText::new("What goes on the back").strong());
    let mut remove = None;
    for (index, row) in state.rows.iter_mut().enumerate() {
        room.ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut row.text)
                    .hint_text("Continued overleaf")
                    .desired_width(230.0),
            );
            ui.label("at");
            millimetres(ui, &mut row.x_mm);
            ui.label(",");
            millimetres(ui, &mut row.y_mm);
            if ui.small_button("×").clicked() {
                remove = Some(index);
            }
        });
    }
    if let Some(index) = remove {
        state.rows.remove(index);
    }
    if state.rows.is_empty() || room.ui.small_button("Another line").clicked() {
        state.rows.push(Row {
            text: String::new(),
            x_mm: 20.0,
            y_mm: 40.0,
        });
    }

    // The question, where it cannot be missed.
    //
    // The two buttons set a flag rather than doing the work here: starting a
    // job needs the whole room, and the room is lent to this group while it is
    // being drawn.
    room.ui.add_space(12.0);
    let mut find_out = false;
    let can_check = state.document.is_some() && !room.jobs.busy();
    room.ui.group(|ui| {
        ui.label(egui::RichText::new("Which way up does the back come out?").strong());
        ui.horizontal(|ui| {
            ui.selectable_value(
                &mut state.feed,
                onionskin::duplex::Feed::SameWayUp,
                "Same way up",
            );
            ui.selectable_value(
                &mut state.feed,
                onionskin::duplex::Feed::TurnedAround,
                "Turned around",
            );
        });
        widgets::hint(ui, state.feed.describe());

        ui.horizontal(|ui| {
            find_out = ui
                .add_enabled(can_check, egui::Button::new("Print a sheet and find out"))
                .clicked();
            if ui.small_button("Remember this answer").clicked() {
                let _ = onionskin::settings::set_default("feed", Some(state.feed.key()));
            }
        });
        widgets::hint(
            ui,
            "Nobody knows this about their own printer, and a guess puts every \
             sheet at the wrong end of the paper. Answer it once and Onionskin \
             will not ask again.",
        );
    });
    if find_out {
        check(state, room);
    }

    room.ui.add_space(8.0);
    room.ui.collapsing("More", |ui| {
        ui.checkbox(
            &mut state.two_sided,
            "The document is already printed on both sides",
        );
        if state.two_sided {
            widgets::hint(
                ui,
                "Then every back is a page of it — sheet three's back is page \
                 six — and the delta has to be printed two-sided the same way \
                 round as the original was. Which way up the paper comes back \
                 stops mattering: the printer does the turning.",
            );
        }

        ui.checkbox(&mut state.one_sheet, "Only one sheet");
        ui.add_enabled_ui(state.one_sheet, |ui| {
            ui.horizontal(|ui| {
                ui.label("Sheet");
                ui.add(egui::DragValue::new(&mut state.sheet).range(1..=9999));
            });
        });

        ui.horizontal(|ui| {
            ui.label("Type size");
            ui.add(
                egui::DragValue::new(&mut state.size_pt)
                    .speed(0.5)
                    .range(4.0..=96.0)
                    .suffix(" pt"),
            );
        });
    });

    room.ui.add_space(10.0);
    let ready = state.document.is_some() && !state.written().is_empty();
    if room
        .ui
        .add_enabled(
            ready && !room.jobs.busy(),
            egui::Button::new(egui::RichText::new("Write the delta").strong()),
        )
        .clicked()
    {
        write_the_backs(state, room);
    }
    if !ready {
        widgets::hint(
            room.ui,
            "Give the document, and something to put on the back of it.",
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

/// The sheet that answers the question.
fn check(state: &mut State, room: &mut Room) {
    let Some(document) = state.document.clone() else {
        return;
    };
    let output = beside(&document, "-which-way-up");
    room.jobs.start("Writing the check sheet", move |report| {
        report.saying("Reading the paper size…");
        let paper = match onionskin::recipe::draw_page(&document, 1) {
            Ok(both) => both.1.page,
            Err(why) => return Outcome::refused(why),
        };
        if let Err(why) = onionskin::pdf::write_delta(
            &output,
            &[paper],
            &[onionskin::duplex::a_test_sheet(paper)],
            "Onionskin: which way up",
            None,
        ) {
            return Outcome::refused(why.to_string());
        }
        Outcome::Done {
            message: format!("One {} sheet, with a word at each end.", paper.describe()),
            wrote: vec![output],
            notes: onionskin::duplex::HOW_TO_USE_THE_TEST_SHEET
                .lines()
                .map(str::trim_end)
                .filter(|line| !line.is_empty())
                .map(str::to_string)
                .collect(),
        }
    });
}

fn write_the_backs(state: &mut State, room: &mut Room) {
    let Some(document) = state.document.clone() else {
        return;
    };
    let rows = state.written();
    let feed = state.feed;
    let two_sided = state.two_sided;
    let only = state.one_sheet.then_some(state.sheet);
    let size_pt = state.size_pt;
    let output = state
        .output
        .clone()
        .unwrap_or_else(|| beside(&document, "-back"));

    room.jobs.start("Writing the backs", move |report| {
        report.saying("Reading the document…");
        let pages = onionskin::recipe::pages_in(&document).unwrap_or(1);
        let sheets = match two_sided {
            true => onionskin::duplex::sheets_for(pages),
            false => pages,
        };
        let wanted: Vec<usize> = match only {
            Some(sheet) => vec![sheet],
            None => (1..=sheets).collect(),
        };
        if let Some(past) = wanted.iter().find(|sheet| **sheet > sheets || **sheet == 0) {
            return Outcome::refused(format!(
                "{} comes to {sheets} sheet(s), so there is no sheet {past}.",
                document.display()
            ));
        }

        let delta_pages = match two_sided {
            true => pages,
            false => sheets,
        };
        let mut sizes = Vec::with_capacity(delta_pages);
        for page in 1..=delta_pages {
            match onionskin::recipe::draw_page(&document, page) {
                Ok(both) => sizes.push(both.1.page),
                Err(why) => return Outcome::refused(why),
            }
        }

        report.saying("Placing the words…");
        let mut lines: Vec<Vec<onionskin::pdf::PlacedLine>> = vec![Vec::new(); delta_pages];
        for sheet in &wanted {
            // A two-sided document has a page for the back and the printer does
            // the turning; a stack going through again is turned by hand.
            let (index, turning) = match two_sided {
                true => (
                    onionskin::duplex::page_of(*sheet, onionskin::duplex::Side::Back),
                    onionskin::duplex::Feed::SameWayUp,
                ),
                false => (*sheet, feed),
            };
            if index > delta_pages {
                continue;
            }
            let paper = sizes[index - 1];
            for row in &rows {
                let (x_mm, y_mm, rotation_deg) =
                    onionskin::duplex::turn_a_placement(row.x_mm, row.y_mm, 0.0, paper, turning);
                lines[index - 1].push(onionskin::pdf::PlacedLine {
                    text: row.text.trim().to_string(),
                    x_mm,
                    y_mm,
                    size_pt,
                    font: onionskin::pdf::LineFont::Builtin(onionskin::pdf::Font::Helvetica),
                    colour: (0.0, 0.0, 0.0),
                    rotation_deg,
                });
            }
        }

        let placed: usize = lines.iter().map(Vec::len).sum();
        if placed == 0 {
            return Outcome::refused(
                "Nothing landed on any back. A document of one page printed on \
                 both sides has no sheet whose back is a page of it."
                    .to_string(),
            );
        }

        report.saying("Writing the delta…");
        if let Err(why) =
            onionskin::pdf::write_delta(&output, &sizes, &lines, "Onionskin back", None)
        {
            return Outcome::refused(why.to_string());
        }

        let notes = match two_sided {
            true => vec![onionskin::duplex::PRINT_IT_THE_SAME_WAY.replace('\n', " ")],
            false => vec![
                "Put the printed stack back in the tray the way you would to \
                 print the other side, and print this delta onto it."
                    .to_string(),
                format!("Placed for a feed of '{}': {}", feed.key(), feed.describe()),
            ],
        };
        Outcome::Done {
            message: format!(
                "{placed} addition(s) on the back of {} sheet(s).",
                wanted.len()
            ),
            wrote: vec![output],
            notes,
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An empty row is somebody who clicked "Another line" and has not typed
    /// yet. Sent through it would print nothing at a place, which is a page of
    /// the delta that exists for no reason.
    #[test]
    fn an_empty_row_is_not_something_to_print() {
        let state = State {
            rows: vec![
                Row {
                    text: "Continued overleaf".into(),
                    x_mm: 20.0,
                    y_mm: 40.0,
                },
                Row {
                    text: "   ".into(),
                    x_mm: 20.0,
                    y_mm: 60.0,
                },
            ],
            ..Default::default()
        };
        assert_eq!(state.written().len(), 1);
    }

    /// The screen opens on every sheet rather than one, because a stack is what
    /// somebody has in their hand.
    #[test]
    fn it_opens_on_the_whole_stack() {
        let state = State::default();
        assert!(!state.one_sheet);
        assert!(!state.two_sided);
    }

    /// The two answers are different, and the one the screen opens on is the
    /// one this machine was told — or the one most printers do.
    #[test]
    fn the_question_opens_on_what_this_machine_was_told() {
        let state = State::default();
        let remembered = onionskin::settings::load()
            .defaults
            .feed
            .as_deref()
            .and_then(onionskin::duplex::Feed::parse);
        assert_eq!(state.feed, remembered.unwrap_or_default());
    }

    /// What the screen writes into the settings is what the settings accept,
    /// which is not true of every word a person might use for these.
    #[test]
    fn the_answer_it_remembers_is_one_the_settings_understand() {
        for feed in [
            onionskin::duplex::Feed::SameWayUp,
            onionskin::duplex::Feed::TurnedAround,
        ] {
            assert_eq!(
                onionskin::duplex::Feed::parse(feed.key()),
                Some(feed),
                "the screen would remember '{}' and not get it back",
                feed.key()
            );
        }
    }

    #[test]
    fn the_delta_lands_beside_the_document() {
        assert_eq!(
            beside(std::path::Path::new("/tmp/invoice.pdf"), "-back"),
            std::path::Path::new("/tmp/invoice-back.pdf")
        );
        assert_eq!(
            beside(std::path::Path::new("/tmp/invoice.pdf"), "-which-way-up"),
            std::path::Path::new("/tmp/invoice-which-way-up.pdf")
        );
    }
}
