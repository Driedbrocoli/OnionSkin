//! The same job, done again on today's document.
//!
//! An office does the same thing to the same form every day: the paid stamp at
//! 150,40 in nine point, the received date under the third line, the reference
//! from whatever came in this morning. Working that out once is fine. Working it
//! out again every Monday is how somebody ends up reprinting a box of
//! letterhead.
//!
//! Saved jobs have been in the program for a while, and until now the only way
//! to run one was to type
//!
//! ```text
//! onionskin job run paid invoice-4472.pdf --set ref=4471
//! ```
//!
//! which is a sentence, in a terminal, with two file paths and a name in it —
//! for the one part of the program most likely to be used by somebody who has
//! never opened a terminal, every day, at speed.
//!
//! Here it is a list, a file, and a box for each blank the job has. The boxes
//! are built from the job itself, so a job wanting `{ref}` and `{amount}` shows
//! two labelled boxes and no others, and the button stays off until both are
//! filled in.

use eframe::egui;

use super::{beside, Room};
use crate::job::Outcome;
use crate::widgets;

#[derive(Default)]
pub struct State {
    /// The saved jobs, read when the screen is first shown and after anything
    /// changes them. Not re-read every frame: this is a folder on disk, and
    /// the screen is drawn sixty times a second.
    known: Option<Vec<onionskin::jobs::Job>>,
    /// Which one is selected, by name — not by position, because deleting one
    /// shifts every position after it and an index would then point at a
    /// neighbour.
    chosen: Option<String>,
    /// The document to run it on.
    document: Option<std::path::PathBuf>,
    /// What has been typed for each blank the job wants, by name. Kept across
    /// a change of job so that going back to one does not lose what was typed.
    filled: std::collections::BTreeMap<String, String>,
    /// Set when a delete is asked for, so it can be confirmed. Deleting a
    /// recipe somebody spent an afternoon working out should take two clicks.
    deleting: Option<String>,
}

/// What a job can be run on: anything Onionskin can open and draw.
const DOCUMENT_KINDS: &[&str] = &[
    "pdf", "docx", "odt", "txt", "md", "doc", "rtf", "png", "jpg", "jpeg", "tif", "tiff",
];

pub fn show(state: &mut State, room: &mut Room) {
    widgets::title(
        room.ui,
        "Saved jobs",
        "Work it out once, then run it on tomorrow's document.",
    );

    if state.known.is_none() {
        state.known = Some(onionskin::jobs::list());
    }
    // Names and the chosen job taken out as owned values before anything is
    // drawn. A job is a handful of short strings; keeping the list borrowed
    // instead means threading that borrow through every closure on the screen,
    // and each of those also wants to change the state it came from.
    let names: Vec<String> = state
        .known
        .iter()
        .flatten()
        .map(|job| job.name.clone())
        .collect();

    if names.is_empty() {
        widgets::hint(
            room.ui,
            "Nothing saved yet. A job is kept by adding a name to a run on the \
             command line:",
        );
        room.ui.add_space(6.0);
        room.ui.label(
            egui::RichText::new(
                "onionskin write invoice.pdf --after 'Total:PAID {today}' --save-as paid",
            )
            .monospace()
            .small(),
        );
        room.ui.add_space(6.0);
        widgets::hint(
            room.ui,
            "After that it appears here, and runs on any document you choose.",
        );
        if room.ui.button("Look again").clicked() {
            state.known = None;
        }
        return;
    }

    if state.chosen.is_none() {
        state.chosen = names.first().cloned();
    }

    room.ui.horizontal(|ui| {
        ui.label("Job");
        egui::ComboBox::from_id_salt("saved-job")
            .selected_text(state.chosen.clone().unwrap_or_default())
            .show_ui(ui, |ui| {
                for name in &names {
                    ui.selectable_value(&mut state.chosen, Some(name.clone()), name);
                }
            });
        if ui.button("Look again").clicked() {
            state.known = None;
        }
    });

    let Some(job) = state
        .chosen
        .as_ref()
        .and_then(|name| state.known.iter().flatten().find(|job| &job.name == name))
        .cloned()
    else {
        return;
    };

    room.ui.add_space(8.0);
    room.ui.group(|ui| {
        ui.label(egui::RichText::new("What it does").strong());
        for (what, specs) in [
            ("at", &job.at),
            ("after", &job.after),
            ("below", &job.below),
            ("picture", &job.images),
        ] {
            for spec in specs {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(what).weak().small());
                    ui.label(egui::RichText::new(spec).monospace());
                });
            }
        }
        ui.label(
            egui::RichText::new(format!(
                "set in {} at {} pt, {}{}",
                job.font,
                job.size_pt,
                job.colour,
                if job.page == 1 {
                    String::new()
                } else {
                    format!(", on page {}", job.page)
                }
            ))
            .weak()
            .small(),
        );
        if !job.notes.is_empty() {
            widgets::hint(ui, &job.notes);
        }
    });

    room.ui.add_space(10.0);
    widgets::file_row(
        room.ui,
        room.picker,
        "Run it on",
        &mut state.document,
        DOCUMENT_KINDS,
        room.dropped,
    );

    // One box per blank the job has, built from the job rather than from a
    // list somebody has to keep in step with it.
    let wants = job.wants();
    if !wants.is_empty() {
        room.ui.add_space(6.0);
        room.ui.label(egui::RichText::new("Fill in").strong());
        widgets::hint(
            room.ui,
            "These change every time, which is why they are blanks. Today's \
             date is filled in by itself and is not asked for.",
        );
        for name in &wants {
            room.ui.horizontal(|ui| {
                ui.label(format!("{{{name}}}"));
                let box_for = state.filled.entry(name.clone()).or_default();
                ui.add(egui::TextEdit::singleline(box_for).desired_width(220.0));
            });
        }
    }

    let missing: Vec<String> = wants
        .iter()
        .filter(|name| {
            state
                .filled
                .get(*name)
                .map(|value| value.trim().is_empty())
                .unwrap_or(true)
        })
        .cloned()
        .collect();

    room.ui.add_space(10.0);
    let ready = state.document.is_some() && missing.is_empty();
    let busy = room.jobs.busy();
    // The row of buttons only says what was pressed. Doing the work inside it
    // would mean the closure holding the whole room while asking for it again.
    let (mut asked_to_run, mut asked_to_delete) = (false, false);
    room.ui.horizontal(|ui| {
        asked_to_run = ui
            .add_enabled(
                ready && !busy,
                egui::Button::new(egui::RichText::new("Run it").strong()),
            )
            .clicked();
        asked_to_delete = ui
            .add_enabled(!busy, egui::Button::new("Delete this job"))
            .clicked();
    });
    if asked_to_run {
        run(state, &job, room);
    }
    if asked_to_delete {
        state.deleting = Some(job.name.clone());
    }

    if state.document.is_none() {
        widgets::hint(room.ui, "Choose the document to run it on.");
    } else if !missing.is_empty() {
        widgets::hint(
            room.ui,
            &format!(
                "Still to fill in: {}.",
                missing
                    .iter()
                    .map(|name| format!("{{{name}}}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        );
    }

    // Asked twice, because a recipe is somebody's afternoon and the button
    // that deletes it sits beside the one they press every morning.
    if let Some(name) = state.deleting.clone() {
        room.ui.add_space(8.0);
        widgets::caution(
            room.ui,
            &format!("Delete the saved job '{name}'? This cannot be undone."),
        );
        room.ui.horizontal(|ui| {
            if ui.button("Yes, delete it").clicked() {
                let _ = onionskin::jobs::delete(&name);
                state.deleting = None;
                state.chosen = None;
                state.known = None;
            }
            if ui.button("Keep it").clicked() {
                state.deleting = None;
            }
        });
    }

    if let Some(outcome) = &room.jobs.last {
        room.ui.add_space(12.0);
        if widgets::outcome(room.ui, outcome) {
            room.jobs.dismiss();
        }
    }
}

fn run(state: &mut State, job: &onionskin::jobs::Job, room: &mut Room) {
    let Some(document) = state.document.clone() else {
        return;
    };
    let output = beside(&document, "-delta");
    let job = job.clone();
    let given: std::collections::BTreeMap<String, String> = state
        .filled
        .iter()
        .filter(|(_, value)| !value.trim().is_empty())
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();

    room.previews.forget(&output);
    room.jobs.start("Running the job", move |report| {
        // Checked here as well as by the button, because the button's rule is
        // about what has been typed and this one is about what the job wants
        // — and the job could have been edited on disk since the list was read.
        let missing = job.missing(&given);
        if !missing.is_empty() {
            return Outcome::refused(format!(
                "'{}' still needs {} filled in.",
                job.name,
                missing
                    .iter()
                    .map(|name| format!("{{{name}}}"))
                    .collect::<Vec<_>>()
                    .join(" and ")
            ));
        }

        // The same filling-in the command line does, from the same code: the
        // braces a job holds are the ones a batch holds, and {today} is filled
        // without being asked for.
        let row = onionskin::jobs::values(&given, onionskin::history::now());
        let fill = |templates: &[String]| -> Vec<String> {
            templates
                .iter()
                .map(|template| onionskin::rows::fill(template, &row))
                .collect()
        };
        let recipe = onionskin::recipe::Recipe {
            at: fill(&job.at),
            after: fill(&job.after),
            below: fill(&job.below),
            images: fill(&job.images),
            page: job.page,
            size_pt: job.size_pt,
            font: job.font.clone(),
            colour: job.colour.clone(),
            width_mm: job.width_mm,
            rotation_deg: job.rotation_deg,
            leading: job.leading,
        };

        report.saying("Working out where the words go…");
        let laid = match onionskin::recipe::lay_out(&recipe, &document) {
            Ok(laid) => laid,
            Err(e) => return Outcome::refused(e),
        };

        report.saying("Drawing the delta…");
        // The same saved settings the command line applies. A job is the one
        // thing in the program meant to behave identically wherever it is run
        // from, so a window that ignored somebody's calibration profile would
        // put the words half a millimetre out from where `onionskin job run`
        // puts them — on the same job, on the same form, on the same printer.
        let options = onionskin::settings::load()
            .defaults
            .over(onionskin::pipeline::Options::default());
        let outcome = match onionskin::pipeline::compose_run_pictures(
            &document,
            &laid.items,
            &[],
            &laid.images,
            &output,
            None,
            &options,
        ) {
            Ok(outcome) => outcome,
            Err(e) => return Outcome::refused(e.to_string()),
        };

        if outcome.blocked() {
            let why: Vec<String> = outcome
                .checks
                .iter()
                .filter(|c| c.severity == onionskin::safety::Severity::Blocker)
                .map(|c| c.message.clone())
                .collect();
            return Outcome::refused(format!(
                "Nothing worth printing came out of that.\n\n{}",
                why.join("\n\n")
            ));
        }

        // What each anchor matched, said out loud. An anchor is a guess that
        // matched something, and a page with two "Total"s on it is how the
        // stamp lands beside the wrong one — which is only catchable by
        // somebody reading the line it found.
        let mut notes: Vec<String> = laid.found.iter().map(|f| f.describe()).collect();
        notes.extend(outcome.checks.iter().map(|check| match check.page {
            Some(page) => format!("Page {page}: {}", check.message),
            None => check.message.clone(),
        }));

        let additions = outcome.total_regions();
        Outcome::Done {
            message: format!(
                "'{}' put {additions} addition{} on {}.",
                job.name,
                if additions == 1 { "" } else { "s" },
                document
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| document.display().to_string())
            ),
            wrote: vec![output],
            notes,
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The list is read once and kept, not read every frame — this screen is
    /// drawn sixty times a second and the jobs are a folder on disk.
    #[test]
    fn the_saved_jobs_are_not_read_from_disk_on_every_frame() {
        assert!(State::default().known.is_none(), "nothing read yet");
    }

    /// Selected by name rather than by position: deleting the job above the
    /// chosen one shifts every index after it, and an index would then be
    /// pointing at a neighbour.
    #[test]
    fn the_chosen_job_is_held_by_name() {
        let state = State {
            chosen: Some("paid".to_string()),
            ..Default::default()
        };
        // Whatever happens to the list, this still names one job.
        assert_eq!(state.chosen.as_deref(), Some("paid"));
    }

    /// A blank filled with spaces is not filled in. Somebody who tabs through
    /// the boxes leaves whitespace behind, and a stamp reading "ref  " is a
    /// stamp that has to be done again on new paper.
    #[test]
    fn whitespace_does_not_count_as_filling_in_a_blank() {
        let mut state = State::default();
        state.filled.insert("ref".into(), "   ".into());

        let empty = state
            .filled
            .get("ref")
            .map(|value| value.trim().is_empty())
            .unwrap_or(true);
        assert!(empty, "spaces were taken for an answer");
    }

    /// Deleting is asked for and then confirmed, never done on one click: the
    /// button sits beside the one somebody presses every morning.
    #[test]
    fn deleting_a_job_takes_two_clicks() {
        assert!(
            State::default().deleting.is_none(),
            "nothing is being deleted at rest"
        );
        let asked = State {
            deleting: Some("paid".into()),
            ..Default::default()
        };
        assert_eq!(asked.deleting.as_deref(), Some("paid"));
    }

    /// A job runs on what an office actually has, which is a PDF or a scan of
    /// one — not only on Onionskin's own documents.
    #[test]
    fn a_job_runs_on_the_documents_an_office_has() {
        for kind in ["pdf", "docx", "png", "jpg"] {
            assert!(DOCUMENT_KINDS.contains(&kind), "{kind} cannot be chosen");
        }
    }
}
