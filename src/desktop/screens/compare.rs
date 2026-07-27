//! Two documents in, a delta out.
//!
//! The thing the program is for. Choose the file as it was printed and the
//! edited copy; Onionskin works out which ink is new and writes a page that is
//! blank except for the additions.

use std::path::PathBuf;

use eframe::egui;

use super::Room;
use crate::job::Outcome;
use crate::widgets;
use onionskin::pipeline;

pub struct State {
    original: Option<PathBuf>,
    edited: Option<PathBuf>,
    output: Option<PathBuf>,
    dpi: f64,
    margin_mm: f64,
    mode: pipeline::Mode,
    profile: String,
    show_settings: bool,
    /// Draw a box round every change.
    outline: bool,
    /// Which colour those boxes are, by name.
    outline_colour: usize,
    /// Keep the delta beside the edited document instead of in Onionskin's own
    /// folder. Off, because most deltas are printed once and never wanted
    /// again — see [`onionskin::delta::scratch_path`].
    keep_beside: bool,
    /// How the comparison itself is made. Held apart from the rest because
    /// nothing here needs touching to get a result, and a wrong value here
    /// produces a delta that looks plausible and is not.
    expert: onionskin::diff::DiffOptions,
    /// How far outside the changed ink a vector delta's clip box reaches.
    pad_mm: f64,
}

/// The colours a box can be drawn in, in the order they are offered.
///
/// Named rather than a colour wheel: somebody marking up a proof wants "red",
/// and the three-number form is on the command line for the rare case where
/// a particular red is meant.
const OUTLINE_COLOURS: &[(&str, (f64, f64, f64))] = &[
    ("Red", (0.80, 0.10, 0.10)),
    ("Blue", (0.10, 0.30, 0.85)),
    ("Green", (0.00, 0.55, 0.20)),
    ("Orange", (0.95, 0.45, 0.00)),
    ("Magenta", (0.85, 0.10, 0.60)),
    ("Black", (0.0, 0.0, 0.0)),
];

impl Default for State {
    fn default() -> Self {
        State {
            original: None,
            edited: None,
            output: None,
            dpi: pipeline::DEFAULT_DPI,
            margin_mm: onionskin::safety::DEFAULT_MARGIN_MM,
            mode: pipeline::Mode::Raster,
            profile: String::new(),
            show_settings: false,
            outline: false,
            outline_colour: 0,
            keep_beside: false,
            expert: onionskin::diff::DiffOptions::default(),
            pad_mm: pipeline::Options::default().pad_mm,
        }
    }
}

/// What the file browser will show.
///
/// The first five need nothing installed. The rest go through LibreOffice, and
/// offering them here rather than hiding them is right: somebody with
/// LibreOffice can use them, and somebody without gets a sentence saying so
/// rather than a file browser that pretends their document does not exist.
const DOCUMENT_KINDS: &[&str] = &[
    "pdf", "docx", "odt", "txt", "md", "doc", "rtf", "docm", "dotx", "ott", "fodt", "xlsx", "ods",
    "pptx", "odp", "html",
];

pub fn show(state: &mut State, room: &mut Room) {
    widgets::title(
        room.ui,
        "Compare two documents",
        "Print only what changed, onto the sheet you already have.",
    );

    widgets::file_row(
        room.ui,
        room.picker,
        "The document as it was printed",
        &mut state.original,
        DOCUMENT_KINDS,
        room.dropped,
    );
    widgets::file_row(
        room.ui,
        room.picker,
        "The edited copy",
        &mut state.edited,
        DOCUMENT_KINDS,
        room.dropped,
    );

    room.ui.add_space(6.0);
    room.ui.checkbox(
        &mut state.outline,
        "Draw a box round every change, so it is easy to see",
    );
    if state.outline {
        room.ui.horizontal(|ui| {
            ui.add_space(24.0);
            ui.label("Colour");
            egui::ComboBox::from_id_salt("outline-colour")
                .selected_text(OUTLINE_COLOURS[state.outline_colour].0)
                .show_ui(ui, |ui| {
                    for (index, (name, _)) in OUTLINE_COLOURS.iter().enumerate() {
                        ui.selectable_value(&mut state.outline_colour, index, *name);
                    }
                });
            widgets::hint(ui, "the box is printed onto the paper too");
        });
    }
    room.ui.checkbox(
        &mut state.keep_beside,
        "Save the delta next to the edited document",
    );
    if !state.keep_beside {
        room.ui.horizontal(|ui| {
            ui.add_space(24.0);
            widgets::hint(
                ui,
                "otherwise it is kept in Onionskin's own folder — print it, or \
                 save a copy afterwards",
            );
        });
    }

    room.ui.add_space(6.0);
    room.ui.collapsing("Settings", |ui| {
        ui.horizontal(|ui| {
            ui.label("Delta");
            egui::ComboBox::from_id_salt("delta-mode")
                .selected_text(match state.mode {
                    pipeline::Mode::Raster => "Raster — exactly the new pixels",
                    pipeline::Mode::Vector => "Vector — sharper, clips to boxes",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut state.mode,
                        pipeline::Mode::Raster,
                        "Raster — exactly the new pixels",
                    );
                    ui.selectable_value(
                        &mut state.mode,
                        pipeline::Mode::Vector,
                        "Vector — sharper, clips to boxes",
                    );
                });
        });
        ui.horizontal(|ui| {
            ui.label("Resolution");
            ui.add(
                egui::DragValue::new(&mut state.dpi)
                    .range(50.0..=1200.0)
                    .suffix(" dpi"),
            );
            ui.label("Edge margin");
            ui.add(
                egui::DragValue::new(&mut state.margin_mm)
                    .range(0.0..=40.0)
                    .speed(0.5)
                    .suffix(" mm"),
            );
        });
        ui.horizontal(|ui| {
            ui.label("Printer profile");
            ui.text_edit_singleline(&mut state.profile);
            widgets::hint(ui, "optional — see Calibration");
        });

        // Folded away inside Settings, which is itself folded away. Two
        // doors deep is right for numbers that decide what counts as ink:
        // nobody needs them to get a delta, and somebody who does need
        // them knows to go looking.
        ui.add_space(8.0);
        ui.collapsing("Expert", |ui| {
            widgets::hint(
                ui,
                "How the comparison itself is made. The defaults are right \
                     for paper — these are here so nothing is hidden from you.",
            );
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label("Counts as ink below");
                ui.add(egui::DragValue::new(&mut state.expert.ink_threshold).range(1..=254));
                widgets::hint(ui, "lower catches fainter marks, and more noise");
            });
            ui.horizontal(|ui| {
                ui.label("Group changes within");
                ui.add(
                    egui::DragValue::new(&mut state.expert.group_mm)
                        .range(0.0..=50.0)
                        .speed(0.1)
                        .suffix(" mm"),
                );
            });
            ui.horizontal(|ui| {
                ui.label("Ignore changes under");
                ui.add(
                    egui::DragValue::new(&mut state.expert.min_region_mm2)
                        .range(0.0..=500.0)
                        .speed(0.1)
                        .suffix(" mm²"),
                );
            });
            ui.horizontal(|ui| {
                ui.label("Ink may move by");
                ui.add(
                    egui::DragValue::new(&mut state.expert.tolerance_mm)
                        .range(0.0..=10.0)
                        .speed(0.05)
                        .suffix(" mm"),
                );
                widgets::hint(ui, "and still count as unchanged");
            });
            if state.mode == pipeline::Mode::Vector {
                ui.horizontal(|ui| {
                    ui.label("Clip box reaches");
                    ui.add(
                        egui::DragValue::new(&mut state.pad_mm)
                            .range(0.0..=10.0)
                            .speed(0.05)
                            .suffix(" mm"),
                    );
                    widgets::hint(ui, "outside the changed ink");
                });
            }
            ui.add_space(4.0);
            // Always offered, so nobody has to remember what four numbers
            // were before they started experimenting with them.
            if ui.button("Put these back to their defaults").clicked() {
                state.expert = onionskin::diff::DiffOptions::default();
                state.pad_mm = pipeline::Options::default().pad_mm;
            }
        });
    });
    let _ = state.show_settings;

    room.ui.add_space(12.0);

    let ready = state.original.is_some() && state.edited.is_some();
    let busy = room.jobs.busy();
    if room
        .ui
        .add_enabled(
            ready && !busy,
            egui::Button::new(egui::RichText::new("Make the delta").strong()),
        )
        .clicked()
    {
        start(state, room);
    }
    if !ready {
        widgets::hint(room.ui, "Choose both files first.");
    }

    if let Some(outcome) = &room.jobs.last {
        room.ui.add_space(12.0);
        if widgets::outcome(room.ui, outcome) {
            room.jobs.dismiss();
        }
    }

    room.ui.add_space(16.0);
    widgets::caution(
        room.ui,
        "Print it at 100%. Put the printed sheet back in the tray, and turn \
         \"Fit to page\" off — it scales by a few percent and nothing will line \
         up. Do one sheet first and hold it against the original.",
    );
}

fn start(state: &mut State, room: &mut Room) {
    let (Some(original), Some(edited)) = (state.original.clone(), state.edited.clone()) else {
        return;
    };

    // Beside the edited copy only if that was asked for. Otherwise Onionskin's
    // own folder, so a job that is printed once does not leave a file behind in
    // somebody's documents and does not make them name one to continue.
    let output = match state.output.clone() {
        Some(chosen) => chosen,
        None if state.keep_beside => {
            let mut path = edited.clone();
            path.set_file_name("delta.pdf");
            path
        }
        None => {
            let name = edited
                .file_stem()
                .map(|stem| format!("{}-delta.pdf", stem.to_string_lossy()))
                .unwrap_or_else(|| "delta.pdf".to_string());
            onionskin::delta::scratch_path(&name)
        }
    };
    // Anything left from a previous run goes now, keeping the one about to be
    // written. Tidying when the program closes never happens to a program that
    // is killed.
    onionskin::delta::tidy_scratch(Some(&output));

    let options = pipeline::Options {
        dpi: state.dpi,
        mode: state.mode,
        margin_mm: state.margin_mm,
        profile: Some(state.profile.trim().to_string()).filter(|p| !p.is_empty()),
        outline: state.outline.then(|| onionskin::delta::Outline {
            colour: OUTLINE_COLOURS[state.outline_colour].1,
            ..Default::default()
        }),
        diff: state.expert,
        pad_mm: state.pad_mm,
        preview_dir: None,
        // The window has nowhere yet to put the pages that cannot be
        // overprinted, so it still reports the blocker rather than splitting.
        fresh: None,
    };

    room.previews.forget(&output);
    let target = output.clone();
    room.jobs.start("Making the delta", move |report| {
        // The pipeline says where it has got to, and a hundred-page delta
        // takes minutes — long enough that a still spinner reads as a program
        // that has stopped.
        let mut say = |step: pipeline::Step| match step.fraction() {
            Some(fraction) => report.part_way(step.describe(), fraction),
            None => report.saying(step.describe()),
        };
        match pipeline::run_watched(&original, &edited, &target, &options, &mut say) {
            Ok(outcome) => {
                // A delta that is blocked is not a success with a warning on
                // it. Ink does not come off paper, and printing this one would
                // put the re-flowed remainder of the page on top of what is
                // already there.
                if outcome.blocked() {
                    let why: Vec<String> = outcome
                        .checks
                        .iter()
                        .filter(|c| c.severity == onionskin::safety::Severity::Blocker)
                        .map(describe)
                        .collect();
                    return Outcome::refused(format!(
                        "The edit moved ink that is already on the paper, so this \
                         cannot be printed onto the sheet you have.\n\n{}\n\nPrint \
                         the whole page fresh instead.",
                        why.join("\n\n")
                    ));
                }

                let additions = outcome.total_regions();
                let pages = outcome.pages_with_additions();
                let notes: Vec<String> = outcome.checks.iter().map(describe).collect();
                Outcome::Done {
                    message: format!(
                        "{additions} addition{} to print, on page{} {}.",
                        if additions == 1 { "" } else { "s" },
                        if pages.len() == 1 { "" } else { "s" },
                        pages
                            .iter()
                            .map(|p| p.to_string())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                    wrote: vec![target],
                    notes,
                }
            }
            Err(e) => Outcome::refused(e.to_string()),
        }
    });
}

/// One safety check, as a sentence a person can act on.
fn describe(check: &onionskin::safety::Check) -> String {
    let mut said = match check.page {
        Some(page) => format!("Page {page}: {}", check.message),
        None => check.message.clone(),
    };
    if !check.detail.is_empty() {
        said.push('\n');
        said.push_str(&check.detail);
    }
    said
}
