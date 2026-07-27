//! Checking a sheet actually came out of the printer right.
//!
//! Overprinting is the one operation where nothing tells you it went wrong.
//! Everything else in this program can be looked at on the screen before it is
//! committed to; a delta cannot, because the thing it is being added to is on
//! paper. It can be written perfectly and still land two millimetres low, or
//! not print at all because the sheet went back into the tray the wrong way up
//! — and the file on disk looks exactly the same either way. Usually the
//! mistake surfaces when somebody opens the envelope.
//!
//! So: put one sheet through, scan it, and be told. It is stricter than an eye,
//! it takes a few seconds, and it is the difference between one wasted sheet
//! and sixty.

use eframe::egui;

use super::Room;
use crate::job::Outcome;
use crate::widgets;
use onionskin::calibrate;

pub struct State {
    /// The delta that was printed onto the sheet.
    delta: Option<std::path::PathBuf>,
    /// A scan of the sheet afterwards.
    sheet: Option<std::path::PathBuf>,
    page: String,
    /// How far out an addition may be before it counts as wrong.
    tolerance_mm: f64,
    /// A profile to teach from the same scan, if one is wanted.
    learn: String,
}

impl Default for State {
    fn default() -> Self {
        State {
            delta: None,
            sheet: None,
            page: "a4".into(),
            // A millimetre. Enough that an uncalibrated printer having a good
            // day passes, tight enough that a sheet nobody would accept fails.
            tolerance_mm: 1.0,
            learn: String::new(),
        }
    }
}

pub fn show(state: &mut State, room: &mut Room) {
    widgets::title(
        room.ui,
        "Check a sheet",
        "Print one, scan it, and find out before you print sixty.",
    );

    widgets::hint(
        room.ui,
        "Give the delta that was printed and a scan of the sheet it went onto. \
         Onionskin looks for each addition where it asked for it and says how \
         close it came — including the ones that are not there at all.",
    );
    room.ui.add_space(10.0);

    widgets::file_row(
        room.ui,
        room.picker,
        "The delta that was printed",
        &mut state.delta,
        &["pdf"],
        room.dropped,
    );
    widgets::file_row(
        room.ui,
        room.picker,
        "A scan of the sheet afterwards",
        &mut state.sheet,
        &["png", "jpg", "jpeg", "tif", "tiff", "bmp"],
        room.dropped,
    );

    room.ui.horizontal(|ui| {
        ui.label("Paper");
        ui.text_edit_singleline(&mut state.page);
    });

    room.ui.horizontal(|ui| {
        ui.label("Close enough");
        ui.add(
            egui::DragValue::new(&mut state.tolerance_mm)
                .speed(0.1)
                .range(0.1..=10.0)
                .suffix(" mm"),
        );
    });
    widgets::hint(
        room.ui,
        "How far out an addition may be. A signature can be two millimetres \
         off and nobody minds; a box on a form cannot.",
    );

    room.ui.add_space(10.0);
    let ready = state.delta.is_some() && state.sheet.is_some();
    if room
        .ui
        .add_enabled(
            ready && !room.jobs.busy(),
            egui::Button::new(egui::RichText::new("Check the sheet").strong()),
        )
        .clicked()
    {
        check(state, room);
    }
    if !ready {
        widgets::hint(
            room.ui,
            "Give the delta and a scan of the sheet it was printed on.",
        );
    }

    room.ui.add_space(10.0);
    room.ui
        .collapsing("Learn the printer from this scan too", |ui| {
            widgets::hint(
                ui,
                "Having scanned the sheet anyway, the same measurement can be \
                 kept as a calibration profile — and every delta after it lands \
                 truer. Name one to save, or leave it empty.",
            );
            ui.horizontal(|ui| {
                ui.label("Call the profile");
                ui.text_edit_singleline(&mut state.learn);
            });
        });

    if let Some(outcome) = &room.jobs.last {
        room.ui.add_space(12.0);
        if widgets::outcome(room.ui, outcome) {
            room.jobs.dismiss();
        }
    }
}

/// Measure the sheet and say what it says.
fn check(state: &mut State, room: &mut Room) {
    let (Some(delta), Some(sheet)) = (state.delta.clone(), state.sheet.clone()) else {
        return;
    };
    let page_text = state.page.clone();
    let tolerance = state.tolerance_mm;
    let learn = state.learn.trim().to_string();

    room.jobs.start("Checking the sheet", move |report| {
        let page = match onionskin::geometry::parse_page(&page_text) {
            Ok(page) => page,
            Err(e) => return Outcome::refused(e.to_string()),
        };

        report.saying("Reading what the delta asked for…");
        let asked = match calibrate::marks_on_delta(&delta) {
            Ok(marks) => marks,
            Err(e) => return Outcome::refused(e.to_string()),
        };
        if asked.is_empty() {
            return Outcome::refused(
                "There is nothing on that delta, so there is nothing to check \
                 for. Give the delta that was actually printed onto this sheet."
                    .to_string(),
            );
        }

        report.saying("Opening the scan…");
        let image = match image::open(&sheet) {
            Ok(image) => image,
            Err(e) => return Outcome::refused(format!("could not read that scan: {e}")),
        };
        report.saying("Finding the sheet on the glass…");
        let registration =
            match onionskin::scan::register(&image, onionskin::scan::ScanOptions::new(page)) {
                Ok(registration) => registration,
                Err(e) => return Outcome::refused(e.to_string()),
            };

        report.saying("Looking for each addition…");
        let gray = image.to_luma8();
        let landings =
            calibrate::measure_landings(&gray, &registration, &asked, calibrate::ink_threshold());
        let checked = calibrate::PrintReport::of(landings, tolerance);

        // Learning is a separate question from whether the sheet is good, so
        // it happens either way and its failure is a note rather than a
        // refusal — a sheet that cannot teach a printer may still be a
        // perfectly good sheet.
        let mut notes = Vec::new();
        if !learn.is_empty() {
            match calibrate::learn_from_landings(&checked.landings, page, &learn) {
                Ok(profile) => match calibrate::save_profile(&profile) {
                    Ok(_) => notes.push(format!(
                        "Profile '{}' saved. {}",
                        profile.name,
                        profile.correction().describe()
                    )),
                    Err(e) => notes.push(format!("The profile could not be saved: {e}")),
                },
                Err(e) => notes.push(format!("Nothing could be learnt from it: {e}")),
            }
        }

        let message = format!(
            "{} addition(s) asked for:\n{}\n\n{}",
            checked.landings.len(),
            checked.lines().join("\n"),
            checked.verdict()
        );
        if checked.good() {
            Outcome::Done {
                message,
                wrote: Vec::new(),
                notes,
            }
        } else {
            // Refused, because that is what this screen is for: saying no to a
            // sheet before fifty-nine more follow it through the printer.
            let mut message = message;
            for note in notes {
                message.push_str("\n\n");
                message.push_str(&note);
            }
            Outcome::refused(message)
        }
    });
}
