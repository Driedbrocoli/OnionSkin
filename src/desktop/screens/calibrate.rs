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
}

impl Default for State {
    fn default() -> Self {
        State {
            profiles: None,
            name: String::new(),
            page: "a4".into(),
            target_to: None,
            measurements: String::new(),
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
        "Without this, expect about ±2 mm. With it, better than ±0.5 mm. It is \
         worth doing once for a printer you will use often, and not worth doing \
         at all for one you will not.",
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

    // ------------------------------------------------------------ the target
    room.ui
        .label(egui::RichText::new("Step 1 — print the target").strong());
    widgets::hint(
        room.ui,
        "A sheet with marks at known places. Print it at 100%, put the same \
         sheet back in the tray, and print it a second time.",
    );
    room.ui.horizontal(|ui| {
        ui.label("Paper");
        ui.text_edit_singleline(&mut state.page);
    });
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
                    "Print this at 100%, then put the same sheet back in the tray \
                     and print it again. The second pass will not land exactly on \
                     the first — that gap is what gets measured.",
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
        .label(egui::RichText::new("Step 2 — measure and save").strong());
    widgets::hint(
        room.ui,
        "For each mark, measure how far the second pass landed from the first, \
         in millimetres, right and down. One per line, as P<mark>:<right>,<down> \
         — left and up are negative.",
    );
    room.ui.add(
        egui::TextEdit::multiline(&mut state.measurements)
            .desired_rows(5)
            .hint_text("P1:+0.40,-0.15\nP2:+0.45,-0.10\nP3:+0.38,-0.22\nP4:+0.47,-0.18")
            .font(egui::TextStyle::Monospace),
    );
    room.ui.add_space(6.0);
    room.ui.horizontal(|ui| {
        ui.label("Call it");
        ui.text_edit_singleline(&mut state.name);
    });
    widgets::hint(room.ui, "a name you will recognise, such as 'office' or 'the big one'");

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
