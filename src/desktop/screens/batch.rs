//! One sheet each, for everybody on a list.
//!
//! Two hundred certificates. Two hundred names. The rest of this program puts
//! words on one sheet exactly; this does it two hundred times, once per row,
//! which is how certificates, tickets, payslips and membership cards actually
//! get made.
//!
//! The list is a spreadsheet — `.xlsx`, `.ods`, or a CSV. There is no
//! conversion step and nothing to install: [`onionskin::sheets`] opens both
//! formats itself.
//!
//! # Why the columns are shown
//!
//! `{name}` in a placement means that row's own name, and a heading that is not
//! there produces two hundred sheets reading `{name}` — an expensive way to
//! find a typo. So the list is read as soon as it is chosen and its headings
//! are put on the screen to click, and a placement naming a column that does
//! not exist is refused before any paper is committed to.

use eframe::egui;

use super::{beside, Room};
use crate::job::Outcome;
use crate::widgets;
use onionskin::document::Item;

pub struct State {
    /// The sheet everybody's copy is printed onto.
    document: Option<std::path::PathBuf>,
    /// The list: a spreadsheet or a CSV, with a heading row.
    list: Option<std::path::PathBuf>,
    output: Option<std::path::PathBuf>,
    /// Where the words go and what they say.
    placements: Vec<Placement>,
    /// Stop after this many, to try one before committing the stack.
    first: usize,
    /// Which sheet to start at, after a jam. Zero means the first.
    from_sheet: usize,
    /// The name of a series whose numbering carries on between runs, or empty
    /// for a run that starts at one every time — a hundred certificates are
    /// not a series and must not quietly become one.
    series: String,
    size_pt: f64,
    /// The list as last read, so the headings can be shown and checked.
    columns: Vec<String>,
    rows: usize,
    /// What the list says about rows that appear more than once, worked out
    /// when it is read rather than every frame.
    repeats: Option<String>,
    /// Where the named series stands, and which name that was read for. This
    /// screen is drawn sixty times a second and the counter is a file on disk,
    /// so it is read when the name changes and not again.
    counter: Option<Result<usize, String>>,
    counter_for: String,
    /// Why the list would not open, where it would not.
    trouble: Option<String>,
    /// Which list the columns above belong to, so a new one is re-read.
    read_from: Option<std::path::PathBuf>,
}

/// One line of words, and where on the sheet it goes.
#[derive(Default, Clone)]
struct Placement {
    x_mm: f64,
    y_mm: f64,
    text: String,
}

impl Default for State {
    fn default() -> Self {
        State {
            document: None,
            list: None,
            output: None,
            placements: vec![Placement {
                x_mm: 60.0,
                y_mm: 120.0,
                text: "{name}".into(),
            }],
            first: 0,
            from_sheet: 0,
            series: String::new(),
            size_pt: 11.0,
            columns: Vec::new(),
            rows: 0,
            repeats: None,
            counter: None,
            counter_for: String::new(),
            trouble: None,
            read_from: None,
        }
    }
}

pub fn show(state: &mut State, room: &mut Room) {
    widgets::title(
        room.ui,
        "One sheet each, for everybody on a list",
        "Two hundred certificates, two hundred names, one pass through the printer.",
    );

    widgets::hint(
        room.ui,
        "The list is a spreadsheet — .xlsx, .ods or a CSV. Nothing needs \
         converting and nothing needs installing.",
    );
    room.ui.add_space(10.0);

    widgets::file_row(
        room.ui,
        room.picker,
        "The sheet everybody's copy goes onto",
        &mut state.document,
        &[
            "pdf", "png", "jpg", "jpeg", "tif", "tiff", "bmp", "docx", "odt",
        ],
        room.dropped,
    );
    widgets::file_row(
        room.ui,
        room.picker,
        "The list",
        &mut state.list,
        &["xlsx", "ods", "csv"],
        room.dropped,
    );

    // Read as soon as a list is chosen. Two hundred sheets reading "{name}" is
    // an expensive way to find out that the column is called "Name".
    if state.list != state.read_from {
        state.read_from = state.list.clone();
        state.columns.clear();
        state.rows = 0;
        state.repeats = None;
        state.trouble = None;
        if let Some(list) = &state.list {
            match onionskin::rows::List::read(list) {
                Ok(read) => {
                    state.columns = read.columns.clone();
                    state.rows = read.rows.len();
                    state.repeats = read.describe_duplicates();
                }
                Err(why) => state.trouble = Some(why.to_string()),
            }
        }
    }

    if let Some(trouble) = &state.trouble {
        widgets::hint(room.ui, trouble);
    } else if !state.columns.is_empty() {
        room.ui.horizontal_wrapped(|ui| {
            ui.label(egui::RichText::new(format!("{} row(s). Columns:", state.rows)).strong());
            for column in &state.columns {
                if ui.small_button(format!("{{{column}}}")).clicked() {
                    if let Some(last) = state.placements.last_mut() {
                        last.text.push_str(&format!("{{{column}}}"));
                    }
                }
            }
            // Not a column and always available, which is the whole point of
            // it: a certificate saying "no. {number}" needs no column holding
            // the numbers 1 to 200.
            if ui.small_button("{number}").clicked() {
                if let Some(last) = state.placements.last_mut() {
                    last.text.push_str("{number}");
                }
            }
            if ui.small_button("{today}").clicked() {
                if let Some(last) = state.placements.last_mut() {
                    last.text.push_str("{today}");
                }
            }
        });
        widgets::hint(
            room.ui,
            "Click one to add it to the last line below. {number} counts the \
             sheets and {today} is the date, so neither needs a column.",
        );
    }

    room.ui.add_space(8.0);
    widgets::save_row(
        room.ui,
        room.picker,
        "Write the stack of deltas to",
        &mut state.output,
        "batch.pdf",
        &["pdf"],
        "beside the sheet, as NAME-batch.pdf",
    );

    room.ui.add_space(8.0);
    room.ui
        .label(egui::RichText::new("What goes on each sheet").strong());
    let mut remove = None;
    for (index, placement) in state.placements.iter_mut().enumerate() {
        room.ui.horizontal(|ui| {
            millimetres(ui, &mut placement.x_mm);
            ui.label(",");
            millimetres(ui, &mut placement.y_mm);
            ui.add(
                egui::TextEdit::singleline(&mut placement.text)
                    .hint_text("Awarded to {name}")
                    .desired_width(260.0),
            );
            if ui.small_button("×").clicked() {
                remove = Some(index);
            }
        });
    }
    if let Some(index) = remove {
        state.placements.remove(index);
    }
    if state.placements.is_empty() || room.ui.small_button("Another line").clicked() {
        state.placements.push(Placement {
            x_mm: 60.0,
            y_mm: 140.0,
            text: String::new(),
        });
    }
    widgets::hint(
        room.ui,
        "Millimetres from the top-left of the paper. The second number is the \
         baseline — where the letters sit.",
    );

    room.ui.add_space(8.0);
    room.ui.horizontal(|ui| {
        ui.label("Type size");
        ui.add(
            egui::DragValue::new(&mut state.size_pt)
                .speed(0.5)
                .range(4.0..=72.0)
                .suffix(" pt"),
        );
        ui.label("Make only the first");
        ui.add(
            egui::DragValue::new(&mut state.first)
                .speed(1)
                .range(0..=9999),
        );
        ui.label(if state.first == 0 {
            "(all of them)"
        } else {
            ""
        });
    });
    widgets::hint(
        room.ui,
        "Before committing two hundred sheets of stock, make two and hold one \
         against a real sheet.",
    );

    room.ui.add_space(8.0);
    room.ui.horizontal(|ui| {
        ui.label("Carry the numbering on, as");
        ui.add(
            egui::TextEdit::singleline(&mut state.series)
                .desired_width(160.0)
                .hint_text("nothing: start at 1"),
        );
        ui.label("Pick up at sheet");
        ui.add(
            egui::DragValue::new(&mut state.from_sheet)
                .speed(1)
                .range(0..=99_999),
        );
        if state.from_sheet == 0 {
            ui.label("(the first)");
        }
    });
    widgets::hint(
        room.ui,
        "A receipt book printed two hundred at a time needs the second box \
         numbered 201 to 400, and a name here remembers where it got to. Pick \
         up at sheet 81 after a jam at 80: those sheets keep the numbers they \
         already had.",
    );
    let named = state.series.trim().to_string();
    if state.counter_for != named {
        state.counter_for = named.clone();
        state.counter = (!named.is_empty()).then(|| {
            onionskin::series::check_name(&named)
                .map_err(|why| why.to_string())
                .and_then(|()| onionskin::series::next_for(&named).map_err(|why| why.to_string()))
        });
    }
    match &state.counter {
        Some(Ok(next)) => widgets::hint(room.ui, &format!("'{named}' next uses {next}.")),
        Some(Err(why)) => widgets::caution(room.ui, why),
        None => {}
    }
    // The same person on the list twice is nearly always a spreadsheet pasted
    // onto itself: obvious in the list and invisible in the stack of paper.
    if let Some(said) = &state.repeats {
        widgets::caution(room.ui, said);
    }

    // Named before the button, because a column that is not there is two
    // hundred wasted sheets and the check costs nothing.
    let missing = missing_columns(state);
    if !missing.is_empty() {
        widgets::hint(
            room.ui,
            &format!(
                "The list has no column called {}. It has: {}",
                missing
                    .iter()
                    .map(|name| format!("'{name}'"))
                    .collect::<Vec<_>>()
                    .join(" or "),
                state
                    .columns
                    .iter()
                    .map(|name| format!("{{{name}}}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        );
    }

    room.ui.add_space(10.0);
    let ready = state.document.is_some()
        && state.list.is_some()
        && !filled_in(state).is_empty()
        && missing.is_empty();
    if room
        .ui
        .add_enabled(
            ready && !room.jobs.busy(),
            egui::Button::new(egui::RichText::new("Make the sheets").strong()),
        )
        .clicked()
    {
        make(state, room);
    }
    if !ready && state.trouble.is_none() {
        widgets::hint(
            room.ui,
            "Give the sheet, the list, and at least one line of words.",
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

/// The lines that actually say something.
fn filled_in(state: &State) -> Vec<Placement> {
    state
        .placements
        .iter()
        .filter(|placement| !placement.text.trim().is_empty())
        .cloned()
        .collect()
}

/// Columns named in the placements that the list has not got.
///
/// A heading that is not there produces two hundred sheets reading `{name}`,
/// which is an expensive way to find a typo.
fn missing_columns(state: &State) -> Vec<String> {
    if state.columns.is_empty() {
        return Vec::new();
    }
    let templates: Vec<String> = filled_in(state)
        .iter()
        .map(|placement| placement.text.clone())
        .collect();
    let list = onionskin::rows::List {
        columns: state.columns.clone(),
        rows: Vec::new(),
    };
    onionskin::rows::unknown_columns(&templates, &list)
        .into_iter()
        .filter(|name| !onionskin::jobs::known_without_asking(name))
        .collect()
}

fn make(state: &mut State, room: &mut Room) {
    let (Some(document), Some(list)) = (state.document.clone(), state.list.clone()) else {
        return;
    };
    let placements = filled_in(state);
    let output = state
        .output
        .clone()
        .unwrap_or_else(|| beside(&document, "-batch"));
    let first = (state.first > 0).then_some(state.first);
    let from_sheet = state.from_sheet.max(1);
    let series = state.series.trim().to_string();
    let size_pt = state.size_pt;

    room.jobs.start("Making the sheets", move |report| {
        report.saying("Reading the list…");
        let read = match onionskin::rows::List::read(&list) {
            Ok(read) => read,
            Err(why) => return Outcome::refused(why.to_string()),
        };
        if read.rows.is_empty() {
            return Outcome::refused(
                "That list has no rows in it, only headings — so there is \
                 nobody to make a sheet for."
                    .to_string(),
            );
        }

        // Where the numbering starts. Carried on from a named series, or one.
        //
        // Picking up after a jam is refused for a series here as it is on the
        // command line: the counter has already moved past this run's numbers,
        // so reading it would put the wrong ones on the paper, and the
        // wrongness is invisible until an accountant finds it.
        let numbering_from = if series.is_empty() {
            1
        } else {
            if let Err(why) = onionskin::series::check_name(&series) {
                return Outcome::refused(why.to_string());
            }
            if from_sheet > 1 {
                return Outcome::refused(format!(
                    "Picking up at sheet {from_sheet} of a series cannot be done from here: \
                     the '{series}' counter has already moved past this run's numbers, and \
                     numbering from it would put the wrong ones on the paper.\n\nOn the \
                     command line, --start-at says which number the run began at."
                ));
            }
            match onionskin::series::next_for(&series) {
                Ok(next) => next,
                Err(why) => return Outcome::refused(why.to_string()),
            }
        };
        let whole_run = read.rows.len();
        if from_sheet > whole_run {
            return Outcome::refused(format!(
                "Picking up at sheet {from_sheet}, but there are only {whole_run} in all — \
                 so there is nothing left to make."
            ));
        }
        let read = read.starting_at(numbering_from).from_row(from_sheet);
        let wanted = first.unwrap_or(read.rows.len()).min(read.rows.len());

        // {today} and its relatives, beside the columns. A certificate saying
        // "Awarded {today}" should not need a column holding the same date two
        // hundred times.
        let known = onionskin::jobs::what_the_day_is(onionskin::history::now());

        report.saying(format!("Laying out {wanted} sheet(s)…"));
        let mut per_sheet: Vec<Vec<Item>> = Vec::with_capacity(wanted);
        for row in read.rows.iter().take(wanted) {
            // A column of the same name wins: a list carrying its own dates,
            // one per row, means them.
            let mut values = known.clone();
            values.extend(row.values.clone());
            let row = onionskin::rows::Row {
                values,
                number: row.number,
            };
            per_sheet.push(
                placements
                    .iter()
                    .map(|placement| Item {
                        id: 0,
                        page: 1,
                        x_mm: placement.x_mm,
                        y_mm: placement.y_mm,
                        text: onionskin::rows::fill(&placement.text, &row),
                        size_pt,
                        font: "Helvetica".to_string(),
                        width_mm: None,
                        rotation_deg: 0.0,
                        colour: "#000000".to_string(),
                        leading: 1.2,
                    })
                    .collect(),
            );
        }
        let nothing: Vec<Vec<onionskin::pdf::PlacedImage>> =
            per_sheet.iter().map(|_| Vec::new()).collect();

        report.saying("Writing the deltas…");
        let options = onionskin::settings::load()
            .defaults
            .over(onionskin::pipeline::Options::default());
        match onionskin::pipeline::compose_sheets_with_pictures(
            &document, 1, &per_sheet, &nothing, &output, None, &options,
        ) {
            Ok(outcome) if outcome.blocked() => Outcome::refused(
                outcome
                    .checks
                    .iter()
                    .map(|check| check.format())
                    .collect::<Vec<_>>()
                    .join("\n\n"),
            ),
            Ok(outcome) => {
                let mut notes = vec![format!(
                    "Put {wanted} printed copies of that sheet in the tray, the \
                     same way up and the same way round as when you printed \
                     them. Not plain paper: this file carries only the \
                     additions."
                )];
                if wanted < read.rows.len() {
                    notes.push(format!(
                        "Only the first {wanted} of {} — the rest are waiting.",
                        read.rows.len()
                    ));
                }
                if from_sheet > 1 {
                    notes.push(format!(
                        "Picked up at sheet {from_sheet} of {whole_run}; the {} before it were \
                         left alone, and these keep the numbers they already had.",
                        from_sheet - 1
                    ));
                }
                // Moved here and nowhere earlier: by what was really made,
                // after it was made. A run that failed wrote nothing and must
                // not burn the numbers.
                let numbered = onionskin::series::numbers(numbering_from, wanted);
                if !series.is_empty() {
                    match onionskin::series::reached_from(&series, numbering_from, numbered.end) {
                        Ok(()) => notes.push(onionskin::series::where_it_got_to(
                            &series,
                            numbered.clone(),
                        )),
                        Err(why) => notes.push(format!(
                            "The sheets are written, but where the '{series}' series got to \
                             could not be saved ({why}). The next run will start at \
                             {numbering_from} again."
                        )),
                    }
                }
                Outcome::Done {
                    message: format!(
                        "{wanted} sheet(s), {} addition(s) in all.",
                        outcome.total_regions()
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

    /// A heading that is not there produces two hundred sheets reading
    /// "{name}", which is an expensive way to find a typo — so it is caught
    /// while the screen is still on the screen.
    #[test]
    fn a_column_that_is_not_on_the_list_is_named_before_the_button() {
        let state = State {
            columns: vec!["name".into(), "course".into()],
            placements: vec![
                Placement {
                    x_mm: 60.0,
                    y_mm: 120.0,
                    text: "Awarded to {name}".into(),
                },
                Placement {
                    x_mm: 60.0,
                    y_mm: 140.0,
                    text: "{nmae}, {course}".into(),
                },
            ],
            ..Default::default()
        };
        let missing = missing_columns(&state);
        assert_eq!(missing, vec!["nmae".to_string()]);
    }

    /// {number} and {today} are not columns and are always available: a
    /// certificate saying "no. {number}" needs no column holding 1 to 200.
    #[test]
    fn the_ones_that_are_not_columns_are_not_reported_missing() {
        let state = State {
            columns: vec!["name".into()],
            placements: vec![Placement {
                x_mm: 60.0,
                y_mm: 120.0,
                text: "{name} — no. {number}, {today}".into(),
            }],
            ..Default::default()
        };
        assert!(missing_columns(&state).is_empty());
    }

    /// Nothing is checked against a list that has not been read: a screen that
    /// complained about every column before the file was chosen would be
    /// wrong every time, and people would stop reading it.
    #[test]
    fn nothing_is_claimed_missing_until_a_list_has_been_read() {
        let state = State {
            placements: vec![Placement {
                x_mm: 0.0,
                y_mm: 0.0,
                text: "{anything}".into(),
            }],
            ..Default::default()
        };
        assert!(state.columns.is_empty());
        assert!(missing_columns(&state).is_empty());
    }

    /// An empty line is somebody who has clicked "Another" and not typed yet,
    /// not a request to print nothing at that spot.
    #[test]
    fn an_empty_line_is_not_a_placement() {
        let state = State {
            placements: vec![
                Placement {
                    x_mm: 60.0,
                    y_mm: 120.0,
                    text: "{name}".into(),
                },
                Placement {
                    x_mm: 60.0,
                    y_mm: 140.0,
                    text: "   ".into(),
                },
            ],
            ..Default::default()
        };
        assert_eq!(filled_in(&state).len(), 1);
    }

    #[test]
    fn the_stack_lands_beside_the_sheet() {
        assert_eq!(
            beside(std::path::Path::new("/tmp/cert.pdf"), "-batch"),
            std::path::Path::new("/tmp/cert-batch.pdf")
        );
    }

    /// The screen reads a spreadsheet the same way the command line does —
    /// which is the whole point of it taking .xlsx at all.
    #[test]
    fn a_spreadsheet_is_read_for_its_columns_the_same_way_the_command_line_reads_it() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("staff.xlsx");
        std::fs::write(&path, an_xlsx()).unwrap();

        let read = onionskin::rows::List::read(&path).expect("a spreadsheet should open");
        assert_eq!(read.columns, vec!["name", "seat"]);
        assert_eq!(read.rows.len(), 1);
        assert_eq!(read.rows[0].get("seat"), Some("4A"));

        // And a placement naming one of those columns is not reported missing.
        let state = State {
            columns: read.columns.clone(),
            placements: vec![Placement {
                x_mm: 0.0,
                y_mm: 0.0,
                text: "{name} {seat}".into(),
            }],
            ..Default::default()
        };
        assert!(missing_columns(&state).is_empty());
    }

    /// A one-tab workbook of two rows, written the way a real one is.
    fn an_xlsx() -> Vec<u8> {
        let part = |name: &str, xml: &str| onionskin::package::Entry {
            name: name.to_string(),
            bytes: xml.as_bytes().to_vec(),
            mode: 0o644,
            directory: false,
        };
        onionskin::package::zip(&[
            part(
                "xl/workbook.xml",
                "<workbook><sheets><sheet name=\"Staff\" r:id=\"rId1\"/></sheets></workbook>",
            ),
            part(
                "xl/_rels/workbook.xml.rels",
                "<Relationships><Relationship Id=\"rId1\" \
                 Target=\"worksheets/sheet1.xml\"/></Relationships>",
            ),
            part("xl/styles.xml", "<styleSheet/>"),
            part(
                "xl/worksheets/sheet1.xml",
                "<worksheet><sheetData>\
                 <row r=\"1\"><c r=\"A1\" t=\"inlineStr\"><is><t>name</t></is></c>\
                 <c r=\"B1\" t=\"inlineStr\"><is><t>seat</t></is></c></row>\
                 <row r=\"2\"><c r=\"A2\" t=\"inlineStr\"><is><t>J. Bezzina</t></is></c>\
                 <c r=\"B2\" t=\"inlineStr\"><is><t>4A</t></is></c></row>\
                 </sheetData></worksheet>",
            ),
        ])
    }
}
