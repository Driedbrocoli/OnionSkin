//! Measuring a printer, once, so every delta after it lands truer.
//!
//! A printer does not put the paper down in quite the same place twice. Feed a
//! sheet through again and the second pass is a millimetre or two out, turned
//! by a fraction of a degree, and very slightly the wrong size. That error is
//! the printer's, it is repeatable, and it can be measured — which is what
//! turns "about right" into "right".
//!
//! Done once per printer. The profile is kept and named, and every later delta
//! can ask for it.

use eframe::egui;

use super::Room;
use crate::job::Outcome;
use crate::widgets;
use onionskin::calibrate;

pub struct State {
    profiles: Option<Vec<calibrate::Profile>>,
    name: String,
    page: String,
    target_to: Option<std::path::PathBuf>,
    /// What was measured off the printed target, one line per mark.
    measurements: String,
    /// A scan of the printed target, for the route that needs no ruler.
    sheet: Option<std::path::PathBuf>,
    /// A delta that was printed on an ordinary job.
    delta: Option<std::path::PathBuf>,
    /// A scan of the sheet that job came out on.
    job_sheet: Option<std::path::PathBuf>,
}

impl Default for State {
    fn default() -> Self {
        State {
            profiles: None,
            name: String::new(),
            page: "a4".into(),
            target_to: None,
            measurements: String::new(),
            sheet: None,
            delta: None,
            job_sheet: None,
        }
    }
}

pub fn show(state: &mut State, room: &mut Room) {
    widgets::title(
        room.ui,
        "Calibration",
        "Measure a printer once, and every delta after it lands truer.",
    );

    widgets::caution(
        room.ui,
        "Without this, expect about ±2 mm. With it, better than ±0.5 mm. The \
         quickest way needs nothing you were not already doing: print a delta, \
         scan the sheet afterwards, and hand both of them over.",
    );
    room.ui.add_space(12.0);

    // ------------------------------------------------------- what is stored
    if state.profiles.is_none() {
        state.profiles = calibrate::list_profiles().ok();
    }
    room.ui.label(egui::RichText::new("Profiles here").strong());
    match state.profiles.as_deref() {
        Some([]) | None => widgets::hint(room.ui, "none yet"),
        Some(profiles) => {
            let mut delete: Option<String> = None;
            for profile in profiles {
                room.ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(&profile.name).monospace());
                    ui.label(
                        egui::RichText::new(format!(
                            "{:+.2}, {:+.2} mm · {:+.3}° · ×{:.4}",
                            profile.error.dx_mm,
                            profile.error.dy_mm,
                            profile.error.rotation_deg,
                            profile.error.scale
                        ))
                        .small()
                        .weak(),
                    );
                    if ui.small_button("Delete").clicked() {
                        delete = Some(profile.name.clone());
                    }
                });
            }
            if let Some(name) = delete {
                let _ = calibrate::delete_profile(&name);
                state.profiles = calibrate::list_profiles().ok();
            }
        }
    }

    room.ui.add_space(14.0);
    room.ui.separator();
    room.ui.add_space(10.0);

    // ----------------------------------------------------- what both routes need
    //
    // The name and the paper belong to the profile, not to the way it was
    // arrived at, so they are asked for once rather than twice.
    room.ui.horizontal(|ui| {
        ui.label("Call it");
        ui.text_edit_singleline(&mut state.name);
    });
    widgets::hint(
        room.ui,
        "a name you will recognise, such as 'office' or 'the big one'",
    );
    room.ui.horizontal(|ui| {
        ui.label("Paper");
        ui.text_edit_singleline(&mut state.page);
    });
    widgets::hint(
        room.ui,
        "the paper you actually print on — a shift carries over to any size, \
         but rotation and scale do not",
    );

    room.ui.add_space(14.0);
    room.ui.separator();
    room.ui.add_space(10.0);

    // -------------------------------------------------- from an ordinary job
    //
    // Offered first because it costs nothing extra. Every delta ever printed
    // is a set of marks in known places, so the sheet that came out of the
    // printer is already a calibration target — and somebody who was never
    // going to sit down and calibrate gets it anyway.
    room.ui.label(
        egui::RichText::new("The quick way — learn from a job you printed").strong(),
    );
    widgets::hint(
        room.ui,
        "Print a delta as usual, scan the sheet afterwards, and give both here. \
         No target sheet, no ruler.",
    );
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
        &mut state.job_sheet,
        &["png", "jpg", "jpeg", "tif", "tiff", "bmp"],
        room.dropped,
    );
    let can_learn = state.delta.is_some()
        && state.job_sheet.is_some()
        && !state.name.trim().is_empty();
    if room
        .ui
        .add_enabled(
            can_learn && !room.jobs.busy(),
            egui::Button::new(egui::RichText::new("Learn from this sheet").strong()),
        )
        .clicked()
    {
        learn_from_job(state, room);
    }
    if !can_learn {
        widgets::hint(
            room.ui,
            "Give the delta, a scan of the sheet it was printed on, and a name.",
        );
    }

    room.ui.add_space(14.0);
    room.ui.separator();
    room.ui.add_space(10.0);

    // ------------------------------------------------------------ the target
    room.ui
        .label(egui::RichText::new("Or measure a target sheet — step 1, print it").strong());
    widgets::hint(
        room.ui,
        "A sheet with marks at known places. Print it at 100%, put the same \
         sheet back in the tray, and print it a second time.",
    );
    widgets::save_row(
        room.ui,
        room.picker,
        "Write the target to",
        &mut state.target_to,
        "target.pdf",
        &["pdf"],
        "if you do not choose, it goes in this folder as target.pdf",
    );

    if room
        .ui
        .add_enabled(!room.jobs.busy(), egui::Button::new("Make the target"))
        .clicked()
    {
        let page_text = state.page.clone();
        let target = state
            .target_to
            .clone()
            .unwrap_or_else(|| std::path::PathBuf::from("target.pdf"));
        room.previews.forget(&target);
        room.jobs.start("Making the target", move |_report| {
            let page = match onionskin::geometry::parse_page(&page_text) {
                Ok(page) => page,
                Err(e) => return Outcome::refused(e.to_string()),
            };
            match calibrate::make_target(&target, page, None) {
                Ok(_) => Outcome::wrote(
                    "Two pages. Print PAGE 1 at 100% on blank paper, put that same \
                     sheet back in the tray, and print PAGE 2 onto it. Each mark \
                     then carries a cross from the first pass and a diamond from \
                     the second, and the gap between them is what gets measured.",
                    vec![target],
                ),
                Err(e) => Outcome::refused(e.to_string()),
            }
        });
    }

    room.ui.add_space(14.0);
    room.ui.separator();
    room.ui.add_space(10.0);

    // ------------------------------------------------------------ the answer
    room.ui
        .label(egui::RichText::new("Step 2 — read the target back").strong());

    // The route that needs no ruler, offered first because it is the one
    // worth taking: a scanner reads tenths of a millimetre about ten times
    // better than a person squinting at a printed scale, and the numbers it
    // produces are what every later delta is placed by.
    widgets::hint(
        room.ui,
        "Scan the printed sheet and let Onionskin read it — this is both easier \
         and more accurate than reading the scales by eye.",
    );
    widgets::file_row(
        room.ui,
        room.picker,
        "A scan of the printed sheet",
        &mut state.sheet,
        &["png", "jpg", "jpeg", "tif", "tiff", "bmp"],
        room.dropped,
    );
    let can_measure = state.sheet.is_some() && !state.name.trim().is_empty();
    let go = room
        .ui
        .add_enabled(
            can_measure && !room.jobs.busy(),
            egui::Button::new(egui::RichText::new("Read the sheet and save").strong()),
        )
        .clicked();
    if state.sheet.is_some() && state.name.trim().is_empty() {
        widgets::hint(room.ui, "Give it a name below first.");
    }
    if go {
        measure_from_sheet(state, room);
    }

    room.ui.add_space(10.0);
    room.ui
        .collapsing("Or read the scales by eye instead", |ui| {
            widgets::hint(
                ui,
                "For each mark, measure how far the diamond landed from the cross, \
                 in millimetres, right and down. One per line, as \
                 P<mark>:<right>,<down> — left and up are negative.",
            );
        });
    room.ui.add(
        egui::TextEdit::multiline(&mut state.measurements)
            .desired_rows(5)
            .hint_text("P1:+0.40,-0.15\nP2:+0.45,-0.10\nP3:+0.38,-0.22\nP4:+0.47,-0.18")
            .font(egui::TextStyle::Monospace),
    );
    room.ui.add_space(6.0);

    let ready = !state.name.trim().is_empty() && !state.measurements.trim().is_empty();
    if room
        .ui
        .add_enabled(
            ready && !room.jobs.busy(),
            egui::Button::new(egui::RichText::new("Work it out and save").strong()),
        )
        .clicked()
    {
        solve(state, room);
    }
    if !ready {
        widgets::hint(room.ui, "Measure the marks and give the profile a name first.");
    }

    if let Some(outcome) = &room.jobs.last {
        room.ui.add_space(12.0);
        if widgets::outcome(room.ui, outcome) {
            room.jobs.dismiss();
            state.profiles = calibrate::list_profiles().ok();
        }
    }
}

/// Learn the printer from a job that was going to be printed anyway.
///
/// The target sheet exists because crosshairs are easy to find. But a delta is
/// also a set of marks in known places, so the sheet it was printed on carries
/// the same measurement — and taking it that way turns calibration from an
/// errand somebody has to decide to run into something that happens by using
/// the program.
fn learn_from_job(state: &mut State, room: &mut Room) {
    let (Some(delta), Some(sheet)) = (state.delta.clone(), state.job_sheet.clone()) else {
        return;
    };
    let name = state.name.trim().to_string();
    let page_text = state.page.clone();
    state.profiles = None;

    room.jobs.start("Learning from the sheet", move |report| {
        let page = match onionskin::geometry::parse_page(&page_text) {
            Ok(page) => page,
            Err(e) => return Outcome::refused(e.to_string()),
        };

        report.saying("Reading what the delta asked for…");
        let intended = match calibrate::marks_on_delta(&delta) {
            Ok(marks) => marks,
            Err(e) => return Outcome::refused(e.to_string()),
        };
        if intended.is_empty() {
            return Outcome::refused(
                "There is nothing on that delta, so there is nothing to measure. \
                 Give the delta that was actually printed onto this sheet."
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

        report.saying("Finding where each addition landed…");
        let gray = image.to_luma8();
        let landings = calibrate::measure_landings(
            &gray,
            &registration,
            &intended,
            calibrate::ink_threshold(),
        );
        let profile = match calibrate::learn_from_landings(&landings, page, &name) {
            Ok(profile) => profile,
            Err(e) => return Outcome::refused(e.to_string()),
        };

        // Where each one went, so the numbers are something a person can check
        // against the sheet in their hand rather than something to take on
        // trust.
        let measured: Vec<String> = landings
            .iter()
            .map(|landing| {
                format!(
                    "  {:>6.1},{:<6.1} mm   out by {:.2} mm{}",
                    landing.intended.0,
                    landing.intended.1,
                    landing.miss_mm(),
                    match landing.doubt() {
                        Some(why) => format!("   (not counted: {why})"),
                        None => String::new(),
                    }
                )
            })
            .collect();

        match calibrate::save_profile(&profile) {
            Ok(_) => Outcome::done(format!(
                "Saved as '{}'.\n\nWhere the additions landed:\n{}\n\n{}\n\nAsk \
                 for it by name when you make a delta. Scan another job back \
                 later and it gets better.",
                profile.name,
                measured.join("\n"),
                profile.correction().describe(),
            )),
            Err(e) => Outcome::refused(e.to_string()),
        }
    });
}

/// Read the printed sheet from a scan of it, and save the profile.
///
/// The part of calibration that used to be a chore. Reading eight offsets off
/// paper with a ruler, in tenths of a millimetre, is unpleasant to do and easy
/// to do badly — and those numbers are what every later delta is placed by, so
/// the least reliable step in the program was somebody squinting at a scale.
fn measure_from_sheet(state: &mut State, room: &mut Room) {
    let Some(sheet) = state.sheet.clone() else {
        return;
    };
    let name = state.name.trim().to_string();
    let page_text = state.page.clone();
    state.profiles = None;

    room.jobs.start("Reading the sheet", move |report| {
        let page = match onionskin::geometry::parse_page(&page_text) {
            Ok(page) => page,
            Err(e) => return Outcome::refused(e.to_string()),
        };
        report.saying("Opening the scan…");
        let image = match image::open(&sheet) {
            Ok(image) => image,
            Err(e) => return Outcome::refused(format!("could not read that scan: {e}")),
        };
        report.saying("Finding the sheet on the glass…");
        let registration = match onionskin::scan::register(
            &image,
            onionskin::scan::ScanOptions::new(page),
        ) {
            Ok(registration) => registration,
            Err(e) => return Outcome::refused(e.to_string()),
        };

        report.saying("Measuring each mark…");
        let gray = image.to_luma8();
        let (profile, readings) =
            match calibrate::calibrate_from_scan(&gray, &registration, page, None, &name, "") {
                Ok(found) => found,
                Err(e) => return Outcome::refused(e.to_string()),
            };

        let measured: Vec<String> = readings.iter().map(|r| r.describe()).collect();
        match calibrate::save_profile(&profile) {
            Ok(_) => Outcome::done(format!(
                "Saved as '{}'.\n\n{}\n\n{}\n\nUse it by naming it in the \
                 comparing screen's Settings.",
                profile.name,
                measured.join("\n"),
                profile.correction().describe(),
            )),
            Err(e) => Outcome::refused(e.to_string()),
        }
    });
}

fn solve(state: &mut State, room: &mut Room) {
    let name = state.name.trim().to_string();
    let page_text = state.page.clone();
    let lines = state.measurements.clone();

    room.jobs.start("Working out the profile", move |_report| {
        let page = match onionskin::geometry::parse_page(&page_text) {
            Ok(page) => page,
            Err(e) => return Outcome::refused(e.to_string()),
        };

        // Parsed one line at a time so a mistyped line can be named, rather
        // than the whole lot refused for a reason nobody can locate.
        let mut offsets = Vec::new();
        for (number, line) in lines.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            match calibrate::parse_point(line) {
                Ok(point) => offsets.push(point),
                Err(e) => {
                    return Outcome::refused(format!(
                        "Line {}: {e}\n\nEach line names a mark and says how far \
                         right and how far down the second pass landed, in \
                         millimetres — for example P1:+0.40,-0.15",
                        number + 1
                    ))
                }
            }
        }
        if offsets.len() < 2 {
            return Outcome::refused(format!(
                "Only {} mark{} measured. With one point only a shift can be \
                 seen, and rotation and scale are what calibration is for. Four \
                 is better than the minimum.",
                offsets.len(),
                if offsets.len() == 1 { "" } else { "s" }
            ));
        }

        let fit = match calibrate::solve_from_offsets(&offsets, page, None) {
            Ok(fit) => fit,
            Err(e) => return Outcome::refused(e.to_string()),
        };
        let n_points = offsets.len();
        let profile = calibrate::Profile {
            name,
            error: fit.transform,
            page,
            rms_residual_mm: Some(fit.rms_residual_mm),
            max_residual_mm: Some(fit.max_residual_mm),
            n_points,
            created: calibrate::now(),
            notes: String::new(),
        };

        let mut summary = format!(
            "Measured: {:+.2} mm across, {:+.2} mm down, turned {:+.3}°, scaled \
             ×{:.4}.\n\nAsk for it by name when you make a delta.",
            profile.error.dx_mm,
            profile.error.dy_mm,
            profile.error.rotation_deg,
            profile.error.scale
        );
        // A fit that does not fit usually means a reading was taken off the
        // wrong crosshair, or two were swapped. Saying so is worth more than a
        // profile that is quietly wrong.
        if fit.max_residual_mm > 0.3 {
            summary.push_str(&format!(
                "\n\nOne reading is {:.2} mm away from the rest, which is more \
                 than a ruler's resolution. Check the readings — most often two \
                 have been swapped, or one was taken off the wrong crosshair.",
                fit.max_residual_mm
            ));
        }

        match calibrate::save_profile(&profile) {
            Ok(path) => Outcome::wrote(summary, vec![path]),
            Err(e) => Outcome::refused(e.to_string()),
        }
    });
}
