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
}

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
    );
    widgets::file_row(
        room.ui,
        room.picker,
        "The edited copy",
        &mut state.edited,
        DOCUMENT_KINDS,
    );

    room.ui.add_space(6.0);
    room.ui
        .collapsing("Settings", |ui| {
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
                ui.add(egui::DragValue::new(&mut state.dpi).range(50.0..=1200.0).suffix(" dpi"));
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

    // Where it goes: beside the edited copy unless somebody has said otherwise,
    // because that is the folder they are already working in.
    let output = state.output.clone().unwrap_or_else(|| {
        let mut path = edited.clone();
        path.set_file_name("delta.pdf");
        path
    });

    let options = pipeline::Options {
        dpi: state.dpi,
        mode: state.mode,
        margin_mm: state.margin_mm,
        profile: Some(state.profile.trim().to_string()).filter(|p| !p.is_empty()),
        ..Default::default()
    };

    room.previews.forget(&output);
    let target = output.clone();
    room.jobs.start("Making the delta", move |report| {
        report.saying("Opening both documents…");
        match pipeline::run(&original, &edited, &target, &options) {
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
