//! A sheet of labels from a list.
//!
//! Address labels, file labels, shelf labels. Every office prints them, the
//! stock comes pre-cut in a grid, and the job is always the same: take a column
//! of names and put one in each label.
//!
//! Two things make this worth a screen of its own rather than a note in the
//! manual. The first is that the measurements are four numbers off the side of
//! the box and nobody remembers which is which — so they are asked for by name,
//! with the commonest sheet already filled in. The second is the half-used
//! sheet: there is always one in the drawer with the first five peeled off, and
//! being able to start at the sixth is the difference between using it and
//! throwing it away.

use eframe::egui;

use super::{beside, Room};
use crate::job::Outcome;
use crate::widgets;
use onionskin::labels::Grid;

pub struct State {
    /// The list: a spreadsheet or a CSV, with a heading row.
    list: Option<std::path::PathBuf>,
    output: Option<std::path::PathBuf>,
    /// What goes on each label, with {column} for the values.
    text: String,
    page: String,
    columns: usize,
    rows: usize,
    /// Each label's size, off the box.
    label_width_mm: f64,
    label_height_mm: f64,
    margin_x_mm: f64,
    margin_y_mm: f64,
    gap_x_mm: f64,
    gap_y_mm: f64,
    /// How many labels have already been peeled off the first sheet.
    already_used: usize,
    size_pt: f64,
    /// How far in from the label's own edge the words start.
    pad_mm: f64,
    /// The stock code chosen, where one was — so the screen can say what it
    /// filled in, and go on saying it while somebody checks the box.
    stock: Option<&'static onionskin::stock::Stock>,
}

impl Default for State {
    fn default() -> Self {
        // The commonest address-label sheet in the world, so somebody with that
        // box in their hand can press the button without typing anything.
        State {
            list: None,
            output: None,
            text: "{name}\n{address}".into(),
            page: "a4".into(),
            columns: 3,
            rows: 8,
            label_width_mm: 63.5,
            label_height_mm: 33.9,
            margin_x_mm: 7.0,
            margin_y_mm: 15.0,
            gap_x_mm: 2.5,
            gap_y_mm: 0.0,
            already_used: 0,
            size_pt: 10.0,
            pad_mm: 3.0,
            stock: None,
        }
    }
}

pub fn show(state: &mut State, room: &mut Room) {
    widgets::title(
        room.ui,
        "A sheet of labels",
        "Addresses, files, shelves — one per label, from a list.",
    );

    widgets::hint(
        room.ui,
        "A spreadsheet — .xlsx, .ods or a CSV — with a heading row. Whatever \
         the headings are called, {in braces}, is filled in from each line.",
    );
    room.ui.add_space(10.0);

    widgets::file_row(
        room.ui,
        room.picker,
        "The list",
        &mut state.list,
        &["csv", "xlsx", "ods"],
        room.dropped,
    );
    widgets::save_row(
        room.ui,
        room.picker,
        "Write the labels to",
        &mut state.output,
        "labels.pdf",
        &["pdf"],
        "beside the list, as NAME-labels.pdf",
    );

    room.ui.label(egui::RichText::new("On each label").strong());
    room.ui.add(
        egui::TextEdit::multiline(&mut state.text)
            .desired_rows(3)
            .desired_width(f32::INFINITY),
    );
    widgets::hint(
        room.ui,
        "One line each. {name} and the rest are the headings from the list.",
    );
    room.ui.add_space(8.0);

    // The half-used sheet, first and on its own, because it is the thing
    // somebody has actually come here to do and the one nothing else offers.
    room.ui
        .label(egui::RichText::new("Labels already peeled off the first sheet").strong());
    room.ui.horizontal(|ui| {
        ui.add(
            egui::DragValue::new(&mut state.already_used)
                .speed(1)
                .range(0..=999),
        );
        widgets::hint(ui, "the first name goes on the next one along");
    });
    room.ui.add_space(8.0);

    // The box's own code, which is the thing somebody is holding. It fills the
    // measurements in below rather than hiding them: a code means different
    // sizes in different countries, and the only defence is somebody seeing the
    // numbers it stood for.
    room.ui
        .label(egui::RichText::new("The label sheet").strong());
    room.ui.horizontal(|ui| {
        let chosen = match state.stock {
            Some(stock) => stock.name(),
            None => "Measurements below".to_string(),
        };
        egui::ComboBox::from_id_salt("label-stock")
            .selected_text(chosen)
            .width(260.0)
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(state.stock.is_none(), "Measurements below")
                    .clicked()
                {
                    state.stock = None;
                }
                for stock in onionskin::stock::KNOWN {
                    let picked = state
                        .stock
                        .map(|mine| mine.code == stock.code)
                        .unwrap_or(false);
                    let said = format!("{} — {}", stock.name(), stock.what_for);
                    if ui.selectable_label(picked, said).clicked() {
                        state.stock = Some(stock);
                        // Filled into the fields themselves, not held to one
                        // side: everything below stays true, stays editable,
                        // and a sheet that is a millimetre out is corrected
                        // rather than abandoned.
                        state.page = stock.paper.to_string();
                        state.columns = stock.across;
                        state.rows = stock.down;
                        state.label_width_mm = stock.label_mm.0;
                        state.label_height_mm = stock.label_mm.1;
                        state.margin_x_mm = stock.margin_mm.0;
                        state.margin_y_mm = stock.margin_mm.1;
                        state.gap_x_mm = stock.gap_mm.0;
                        state.gap_y_mm = stock.gap_mm.1;
                    }
                }
            });
    });
    if let Some(stock) = state.stock {
        widgets::hint(
            room.ui,
            &format!(
                "{} — the published measurements, filled in below. The box is \
                 the authority: print one sheet on plain paper and hold it up to \
                 the stock before committing the box.",
                stock.describe().lines().next().unwrap_or_default()
            ),
        );
    }
    room.ui.add_space(8.0);

    room.ui.collapsing("The sheet's measurements", |ui| {
        widgets::hint(
            ui,
            "Off the side of the box. These are for the commonest address sheet \
             — three across, eight down, 63.5 × 33.9 mm — so if that is what is \
             in the printer, nothing here needs touching.",
        );
        ui.horizontal(|ui| {
            ui.label("Paper");
            ui.add(egui::TextEdit::singleline(&mut state.page).desired_width(80.0));
            ui.label("Labels across");
            ui.add(egui::DragValue::new(&mut state.columns).range(1..=40));
            ui.label("down");
            ui.add(egui::DragValue::new(&mut state.rows).range(1..=40));
        });
        ui.horizontal(|ui| {
            ui.label("Each label");
            millimetres(ui, &mut state.label_width_mm);
            ui.label("×");
            millimetres(ui, &mut state.label_height_mm);
        });
        ui.horizontal(|ui| {
            ui.label("From the paper's edge");
            millimetres(ui, &mut state.margin_x_mm);
            ui.label("in,");
            millimetres(ui, &mut state.margin_y_mm);
            ui.label("down");
        });
        ui.horizontal(|ui| {
            ui.label("Between labels");
            millimetres(ui, &mut state.gap_x_mm);
            ui.label("across,");
            millimetres(ui, &mut state.gap_y_mm);
            ui.label("down");
        });
        ui.horizontal(|ui| {
            ui.label("Type size");
            ui.add(
                egui::DragValue::new(&mut state.size_pt)
                    .speed(0.5)
                    .range(4.0..=48.0)
                    .suffix(" pt"),
            );
            ui.label("indented");
            millimetres(ui, &mut state.pad_mm);
        });
    });

    // Said before the button, not after: a grid that runs off the sheet costs
    // a sheet of labels, and the numbers are right there to be checked.
    room.ui.add_space(8.0);
    match grid_from(state) {
        Ok(grid) => {
            widgets::hint(
                room.ui,
                &format!("{}, {} to a sheet.", grid.describe(), grid.per_sheet()),
            );
        }
        Err(said) => widgets::caution(room.ui, &said),
    }

    room.ui.add_space(10.0);
    let ready = state.list.is_some() && grid_from(state).is_ok();
    if room
        .ui
        .add_enabled(
            ready && !room.jobs.busy(),
            egui::Button::new(egui::RichText::new("Make the labels").strong()),
        )
        .clicked()
    {
        make(state, room);
    }
    if state.list.is_none() {
        widgets::hint(room.ui, "Choose the list to make them from.");
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
            .speed(0.1)
            .range(0.0..=500.0)
            .suffix(" mm"),
    );
}

/// The grid these numbers describe, or why it will not fit.
fn grid_from(state: &State) -> Result<Grid, String> {
    let page = onionskin::geometry::parse_page(&state.page).map_err(|e| e.to_string())?;
    let grid = Grid {
        page,
        columns: state.columns,
        rows: state.rows,
        margin_x_mm: state.margin_x_mm,
        margin_y_mm: state.margin_y_mm,
        gap_x_mm: state.gap_x_mm,
        gap_y_mm: state.gap_y_mm,
        label: Some((state.label_width_mm, state.label_height_mm)),
    };
    grid.check()?;
    Ok(grid)
}

fn make(state: &mut State, room: &mut Room) {
    let Some(list) = state.list.clone() else {
        return;
    };
    let Ok(grid) = grid_from(state) else {
        // The button is disabled while the grid does not fit, and the reason is
        // already on screen above it, so there is nothing to say here.
        return;
    };
    let output = state
        .output
        .clone()
        .unwrap_or_else(|| beside(&list, "-labels"));
    let text = state.text.clone();
    let skip = state.already_used;
    let size_pt = state.size_pt;
    let pad_mm = state.pad_mm;

    room.jobs.start("Making the labels", move |report| {
        if skip >= grid.per_sheet() {
            return Outcome::refused(format!(
                "There are only {} labels on a sheet, so {skip} cannot already \
                 have been peeled off it.",
                grid.per_sheet()
            ));
        }

        report.saying("Reading the list…");
        let list = match onionskin::rows::List::read(&list) {
            Ok(list) => list,
            Err(e) => return Outcome::refused(e.to_string()),
        };
        if list.rows.is_empty() {
            return Outcome::refused("That list has no lines in it, only headings.".to_string());
        }

        // A heading that is not there produces a hundred labels reading
        // "{name}", which is a bad afternoon — and a hundred reading nothing
        // at all is worse, because they look finished.
        let unknown: Vec<String> =
            onionskin::rows::unknown_columns(std::slice::from_ref(&text), &list)
                .into_iter()
                .filter(|name| !onionskin::jobs::known_without_asking(name))
                .collect();
        if !unknown.is_empty() {
            return Outcome::refused(format!(
                "That list has no column called {}.\n\nIt has: {}",
                unknown
                    .iter()
                    .map(|name| format!("'{name}'"))
                    .collect::<Vec<_>>()
                    .join(" or "),
                list.describe_columns()
            ));
        }

        report.saying("Laying them out…");
        let known = onionskin::jobs::what_the_day_is(onionskin::history::now());
        let sheets = grid.sheets_needed(list.rows.len(), skip);
        let mut per_page: Vec<Vec<onionskin::pdf::PlacedLine>> = vec![Vec::new(); sheets];
        let mut overfull = 0usize;

        for (n, row) in list.rows.iter().enumerate() {
            let mut values = known.clone();
            values.extend(row.values.clone());
            let row = onionskin::rows::Row {
                values,
                number: row.number,
            };
            let (sheet, at) = grid.place(n, skip);
            let Some(cell) = grid.cell(at) else { continue };
            let filled = onionskin::rows::fill(&text, &row);
            let lines: Vec<&str> = filled.split('\n').collect();
            let fits = cell.lines_that_fit(size_pt, 1.2, pad_mm);
            if lines.len() > fits {
                overfull += 1;
            }
            for (line, words) in lines.iter().enumerate().take(fits) {
                if words.trim().is_empty() {
                    continue;
                }
                let (x_mm, y_mm) = cell.line_at(line, size_pt, 1.2, pad_mm);
                per_page[sheet].push(onionskin::pdf::PlacedLine {
                    text: (*words).to_string(),
                    x_mm,
                    y_mm,
                    size_pt,
                    font: onionskin::pdf::LineFont::Builtin(onionskin::pdf::Font::Helvetica),
                    rotation_deg: 0.0,
                    colour: (0.0, 0.0, 0.0),
                });
            }
        }

        let sizes = vec![grid.page; sheets];
        report.saying("Writing the PDF…");
        match onionskin::pdf::write_delta(&output, &sizes, &per_page, "Onionskin labels", None) {
            Ok(()) => {
                let mut notes = Vec::new();
                if skip > 0 {
                    notes.push(format!(
                        "The first {skip} label(s) of sheet one are left blank, \
                         for the ones already peeled off."
                    ));
                }
                if overfull > 0 {
                    notes.push(format!(
                        "{overfull} of them had more lines than fit on a label \
                         at {size_pt} pt. The extra lines were left off rather \
                         than printed across the next label — smaller type or \
                         fewer lines will hold them."
                    ));
                }
                notes.push(
                    "Print at 100% / 'Actual size'. 'Fit to page' scales by a \
                     few percent and every label will be off."
                        .to_string(),
                );
                Outcome::Done {
                    message: format!(
                        "{} label(s) on {sheets} sheet(s) — {}.",
                        list.rows.len(),
                        grid.describe()
                    ),
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

    /// Somebody who opens this screen with the commonest box of labels in
    /// their hand should be able to press the button without typing a number.
    #[test]
    fn the_defaults_are_a_sheet_that_fits() {
        let state = State::default();
        let grid = grid_from(&state).expect("the defaults must describe a real sheet");
        assert_eq!(grid.per_sheet(), 24);
        assert_eq!(state.already_used, 0, "a fresh sheet, until told otherwise");
    }

    /// Choosing a code has to fill the fields in, not sit beside them.
    ///
    /// The whole reason a code is safe to offer is that the numbers it stands
    /// for stay visible and stay editable — a code means different sizes in
    /// different countries, and a screen that hid them would be the silent
    /// wrongness [`onionskin::stock`] exists to avoid.
    #[test]
    fn a_stock_code_fills_in_the_measurements_and_leaves_them_showing() {
        let mut state = State::default();
        let stock = onionskin::stock::find("l7160").expect("l7160 is known");

        // What the picker does when it is clicked.
        state.stock = Some(stock);
        state.page = stock.paper.to_string();
        state.columns = stock.across;
        state.rows = stock.down;
        state.label_width_mm = stock.label_mm.0;
        state.label_height_mm = stock.label_mm.1;
        state.margin_x_mm = stock.margin_mm.0;
        state.margin_y_mm = stock.margin_mm.1;
        state.gap_x_mm = stock.gap_mm.0;
        state.gap_y_mm = stock.gap_mm.1;

        let grid = grid_from(&state).expect("a real box of labels must describe a real sheet");
        assert_eq!(grid.per_sheet(), 21);
        assert_eq!(state.columns, 3);
        assert_eq!(state.rows, 7);
        assert!((state.label_height_mm - 38.1).abs() < 1e-9);
    }

    /// Every code in the table has to describe a sheet this screen can lay out.
    /// A code that fills the fields with something `grid_from` then refuses
    /// would be a dead entry in the list.
    #[test]
    fn every_code_in_the_list_lays_out_on_this_screen() {
        for stock in onionskin::stock::KNOWN {
            let state = State {
                page: stock.paper.to_string(),
                columns: stock.across,
                rows: stock.down,
                label_width_mm: stock.label_mm.0,
                label_height_mm: stock.label_mm.1,
                margin_x_mm: stock.margin_mm.0,
                margin_y_mm: stock.margin_mm.1,
                gap_x_mm: stock.gap_mm.0,
                gap_y_mm: stock.gap_mm.1,
                stock: Some(stock),
                ..Default::default()
            };
            let grid = grid_from(&state)
                .unwrap_or_else(|why| panic!("{} does not lay out: {why}", stock.code));
            assert_eq!(
                grid.per_sheet(),
                stock.per_sheet(),
                "{} lays out {} labels, not {}",
                stock.code,
                grid.per_sheet(),
                stock.per_sheet()
            );
        }
    }

    /// A grid that runs off the paper costs a sheet of labels rather than
    /// failing, so it is caught while the numbers are still on screen.
    #[test]
    fn a_grid_that_does_not_fit_is_refused_before_the_button() {
        let too_wide = State {
            columns: 4,
            ..Default::default()
        };
        let said = grid_from(&too_wide).unwrap_err();
        assert!(said.contains("right-hand edge"), "{said}");

        let nonsense_paper = State {
            page: "not-a-size".into(),
            ..Default::default()
        };
        assert!(grid_from(&nonsense_paper).is_err());
    }

    /// The window and `onionskin labels` are two views of one answer, so the
    /// same measurements must produce the same grid.
    #[test]
    fn the_window_and_the_command_line_agree_about_the_grid() {
        let state = State::default();
        let from_window = grid_from(&state).unwrap();
        let from_numbers = onionskin::labels::Grid {
            page: onionskin::geometry::parse_page("a4").unwrap(),
            columns: 3,
            rows: 8,
            margin_x_mm: 7.0,
            margin_y_mm: 15.0,
            gap_x_mm: 2.5,
            gap_y_mm: 0.0,
            label: Some((63.5, 33.9)),
        };
        assert_eq!(from_window, from_numbers);
    }

    #[test]
    fn the_labels_land_beside_the_list_they_came_from() {
        let beside = beside(std::path::Path::new("/tmp/addresses.csv"), "-labels");
        assert_eq!(beside, std::path::Path::new("/tmp/addresses-labels.pdf"));
        // A bare name stays a bare name rather than gaining a ./
        assert_eq!(
            beside_str("addresses.csv"),
            std::path::Path::new("addresses-labels.pdf")
        );
    }

    fn beside_str(name: &str) -> std::path::PathBuf {
        beside(std::path::Path::new(name), "-labels")
    }
}
