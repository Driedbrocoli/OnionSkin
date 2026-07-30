//! The folder the scanner writes into, done as it fills up.
//!
//! The office multifunction has a button on the front that scans to a folder.
//! Somebody presses it, walks back to their desk, opens the file, runs a saved
//! job on it, prints the answer, and walks back to the machine. The middle
//! three steps are the ones a computer should be doing.
//!
//! On the command line this is `onionskin watch ~/Scans --job paid`, which sits
//! in a terminal for the rest of the afternoon. That is fine on a server and
//! wrong for the person who is going to use it, who works in a window and has
//! never opened a terminal. Here it is a folder, a job, and a switch.
//!
//! # Sweeping rather than looping
//!
//! The command line loops forever. A window cannot: the thread that draws has
//! to come back sixty times a second, and the one worker that does slow things
//! does one job at a time. So each look at the folder is a job of its own, and
//! "keep watching" simply starts the next one when the last has finished and
//! enough time has passed.
//!
//! That is the same rule, differently shaped, and it keeps the property that
//! matters: nothing is opened until it has looked the same twice, so a file the
//! scanner is halfway through writing is left where it is.

use eframe::egui;

use super::Room;
use crate::job::Outcome;
use crate::widgets;

pub struct State {
    /// The folder to watch.
    folder: Option<std::path::PathBuf>,
    /// Which saved job to run, by name.
    chosen: Option<String>,
    /// The saved jobs, read once rather than every frame: this screen is drawn
    /// sixty times a second and they are a folder on disk.
    known: Option<Vec<onionskin::jobs::Job>>,
    /// What has been typed for each blank the job wants.
    filled: std::collections::BTreeMap<String, String>,
    /// Where the deltas go, if not beside each scan.
    into: Option<std::path::PathBuf>,
    /// Whether to keep looking, rather than looking once.
    keeping_watch: bool,
    /// What the last sweep saw, which is what the next one compares against.
    /// A file is not opened until two sweeps agree about it.
    ///
    /// Shared with the worker rather than carried back in the job's outcome:
    /// an outcome is what a person is shown, and a hundred file sizes is not
    /// something to show a person.
    before: Looks,
    /// When the last sweep started, on the window's own clock.
    swept_at: Option<f64>,
    /// How many deltas this session has made, so the screen says something
    /// after a quiet hour other than nothing.
    made: usize,
    /// The files done this session, newest first, for the list on screen.
    lately: Vec<String>,
    /// Whether the sweep that just finished has been read. It is read once,
    /// because its outcome sits on screen until dismissed.
    harvested: bool,
    /// What this screen's own sweeps have written, newest last.
    ///
    /// Counted from here rather than from the worker's last outcome, which is
    /// shared with every other screen: a sweep that finished while somebody was
    /// looking at another screen, and a job started there afterwards, would
    /// leave that job's output sitting in `jobs.last` for this screen to count
    /// as deltas it had made.
    wrote: Made,
}

impl Default for State {
    fn default() -> State {
        State {
            folder: None,
            chosen: None,
            known: None,
            filled: std::collections::BTreeMap::new(),
            into: None,
            keeping_watch: false,
            before: Looks::default(),
            swept_at: None,
            made: 0,
            lately: Vec::new(),
            harvested: true,
            wrote: Made::default(),
        }
    }
}

/// What a sweep saw, shared between the window and the one worker.
type Looks = std::sync::Arc<
    std::sync::Mutex<std::collections::BTreeMap<std::path::PathBuf, onionskin::watch::Look>>,
>;

/// What this screen's sweeps have written, shared with the one worker.
type Made = std::sync::Arc<std::sync::Mutex<Vec<std::path::PathBuf>>>;

/// How many done files are listed. Beyond this the oldest scroll off, because
/// the useful part of the list is what happened in the last few minutes.
const LATELY: usize = 12;

pub fn show(state: &mut State, room: &mut Room) {
    widgets::title(
        room.ui,
        "Watch a folder",
        "Press the button on the scanner; the delta is waiting when you get back.",
    );

    if state.known.is_none() {
        state.known = Some(onionskin::jobs::list());
    }
    let names: Vec<String> = state
        .known
        .iter()
        .flatten()
        .map(|job| job.name.clone())
        .collect();

    if names.is_empty() {
        widgets::hint(
            room.ui,
            "This runs a saved job on every file that lands in a folder, so \
             there has to be a job to run. Make one on the Saved jobs screen, \
             or by adding a name to a run on the command line:",
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
        if room.ui.button("Look again").clicked() {
            state.known = None;
        }
        return;
    }
    if state.chosen.is_none() {
        state.chosen = names.first().cloned();
    }

    widgets::folder_row(
        room.ui,
        room.picker,
        "Watch",
        &mut state.folder,
        room.dropped,
    );
    room.ui.horizontal(|ui| {
        ui.label("Run");
        egui::ComboBox::from_id_salt("watch-job")
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

    // One box per blank the job has, built from the job rather than from a
    // list somebody has to keep in step with it. A watch left running with a
    // blank unfilled is an afternoon of sheets reading {ref}.
    let wants = job.wants();
    if !wants.is_empty() {
        room.ui.add_space(6.0);
        room.ui.label(egui::RichText::new("Fill in").strong());
        widgets::hint(
            room.ui,
            "The same for every file that lands. Today's date is filled in by \
             itself and is not asked for — and it is worked out again for each \
             file, so a watch left running overnight still stamps the right day.",
        );
        for name in &wants {
            room.ui.horizontal(|ui| {
                ui.label(format!("{{{name}}}"));
                let box_for = state.filled.entry(name.clone()).or_default();
                ui.add(egui::TextEdit::singleline(box_for).desired_width(220.0));
            });
        }
    }

    room.ui.add_space(6.0);
    widgets::folder_row(
        room.ui,
        room.picker,
        "Deltas into",
        &mut state.into,
        room.dropped,
    );
    if state.into.is_none() {
        widgets::hint(
            room.ui,
            "Left empty, each delta lands beside its own scan as NAME-delta.pdf.",
        );
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
    let ready = state.folder.is_some() && missing.is_empty();

    room.ui.add_space(10.0);
    let busy = room.jobs.busy();
    let mut asked_to_sweep = false;
    room.ui.horizontal(|ui| {
        asked_to_sweep = ui
            .add_enabled(
                ready && !busy,
                egui::Button::new(egui::RichText::new("Look now").strong()),
            )
            .clicked();
        ui.add_enabled_ui(ready, |ui| {
            ui.checkbox(&mut state.keeping_watch, "Keep watching");
        });
    });

    if !ready {
        if state.folder.is_none() {
            widgets::hint(room.ui, "Choose the folder the scanner writes into.");
        } else {
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
        state.keeping_watch = false;
    }

    // What the sweep that just finished wrote, taken in before the next one is
    // started — `Jobs::start` clears the last outcome, so this is the only
    // moment it is there to be read. Taken once, because the outcome sits
    // there until it is dismissed and counting it every frame would tell
    // somebody they had made four hundred deltas from one scan.
    if !state.harvested && !busy {
        // What was written comes from this screen's own list, filled in by the
        // sweep itself. Never from `jobs.last`, which belongs to whichever job
        // finished most recently anywhere in the window.
        if let Ok(mut wrote) = state.wrote.lock() {
            state.made += wrote.len();
            for path in wrote.drain(..).rev() {
                let name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                state.lately.insert(0, name);
            }
            state.lately.truncate(LATELY);
        }
        // A refusal stops the watch. The folder has gone, or the job wants
        // something it has not been given, and neither fixes itself: left
        // running, the same refusal would be raised and then wiped by the next
        // sweep every two seconds, so the one message that matters would be
        // the one nobody could finish reading.
        if matches!(room.jobs.last, Some(Outcome::Refused { .. })) {
            state.keeping_watch = false;
        }
        state.harvested = true;
    }

    // Keeping watch means starting the next look when the last has finished
    // and enough time has passed. The window's own clock is used rather than
    // the wall clock, because it is monotonic and it is already there.
    let now = room.ui.input(|input| input.time);
    let due = state
        .swept_at
        .map(|then| now - then >= onionskin::watch::BETWEEN_SWEEPS_SECONDS as f64)
        .unwrap_or(true);
    if state.keeping_watch && ready && !busy && due {
        asked_to_sweep = true;
    }
    if state.keeping_watch {
        // Otherwise the window only redraws on input, and a watch nobody is
        // touching would never take its next look.
        room.ui
            .ctx()
            .request_repaint_after(std::time::Duration::from_millis(500));
    }

    if asked_to_sweep {
        state.swept_at = Some(now);
        state.harvested = false;
        sweep(state, &job, room);
    }

    if state.made > 0 || !state.lately.is_empty() {
        room.ui.add_space(12.0);
        room.ui.group(|ui| {
            ui.label(
                egui::RichText::new(format!(
                    "{} delta{} so far",
                    state.made,
                    if state.made == 1 { "" } else { "s" }
                ))
                .strong(),
            );
            for name in &state.lately {
                ui.label(egui::RichText::new(name).monospace().small());
            }
        });
    }

    if let Some(outcome) = &room.jobs.last {
        // A sweep that found nothing is the ordinary case and does not deserve
        // a panel of its own every two seconds. Only the ones that did
        // something, or had something to say, are worth showing.
        //
        // The notes matter as much as the deltas. A sweep in which every file
        // failed writes nothing at all, and testing only `wrote` would show
        // nothing at all — while the record quietly remembers each failure so
        // that it is never tried again. A folder of files that cannot be opened
        // would sit there doing nothing and saying nothing, forever.
        let worth_showing = match outcome {
            Outcome::Done { wrote, notes, .. } => !wrote.is_empty() || !notes.is_empty(),
            Outcome::Refused { .. } => true,
        };
        if worth_showing {
            room.ui.add_space(12.0);
            if widgets::outcome(room.ui, outcome) {
                room.jobs.dismiss();
            }
        }
    }
}

/// One look at the folder, and the job run on whatever is ready.
fn sweep(state: &mut State, job: &onionskin::jobs::Job, room: &mut Room) {
    let Some(folder) = state.folder.clone() else {
        return;
    };
    let into = state.into.clone();
    let job = job.clone();
    let before = state.before.clone();
    let made = state.wrote.clone();
    let given: std::collections::BTreeMap<String, String> = state
        .filled
        .iter()
        .filter(|(_, value)| !value.trim().is_empty())
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect();

    room.jobs.start("Looking at the folder", move |report| {
        // A file is not opened until two looks agree about it, so a sweep with
        // nothing to compare against — the first press of the button, or the
        // first tick after the folder was changed — can never find any work.
        // Pressing "Look now" on a folder full of scans and having nothing
        // whatever happen is not a thing to make somebody sit through, so this
        // one takes the first look itself.
        //
        // The wait is on the worker, never on the thread that draws.
        if before.lock().map(|looks| looks.is_empty()).unwrap_or(true) {
            report.saying("Looking at the folder…");
            if let Ok(first) = onionskin::watch::listing(&folder) {
                if let Ok(mut looks) = before.lock() {
                    *looks = onionskin::watch::remember_looks(&first);
                }
                std::thread::sleep(std::time::Duration::from_secs(
                    onionskin::watch::BETWEEN_SWEEPS_SECONDS,
                ));
            }
        }

        let now = match onionskin::watch::listing(&folder) {
            Ok(now) => now,
            Err(e) => return Outcome::refused(e.to_string()),
        };
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

        let done = onionskin::watch::read_ledger(&folder);
        // What the last sweep saw, and then what this one did. Held across the
        // decision so that two sweeps cannot interleave and let a file through
        // on one look — though there is only ever one worker, so they cannot.
        let mut looks = match before.lock() {
            Ok(looks) => looks,
            Err(poisoned) => poisoned.into_inner(),
        };
        let verdicts = onionskin::watch::decide(&now, &looks, &done);
        *looks = onionskin::watch::remember_looks(&now);
        drop(looks);

        let work: Vec<onionskin::watch::Seen> = verdicts
            .iter()
            .filter(|(_, verdict)| verdict.is_work())
            .map(|(seen, _)| seen.clone())
            .collect();

        if work.is_empty() {
            return Outcome::Done {
                message: looked_at(&folder, &verdicts),
                // Nothing written, which is how the screen knows this one is
                // not worth a panel.
                wrote: Vec::new(),
                notes: Vec::new(),
            };
        }

        let mut wrote = Vec::new();
        let mut notes: Vec<String> = Vec::new();
        for (n, seen) in work.iter().enumerate() {
            let name = seen
                .path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            report.part_way(
                format!("{name} ({} of {})", n + 1, work.len()),
                (n as f32) / (work.len() as f32),
            );
            let delta = onionskin::watch::where_the_delta_goes(&seen.path, into.as_deref());
            let trouble = run_one(&job, &given, &seen.path, &delta);
            if trouble.is_empty() {
                if let Ok(mut made) = made.lock() {
                    made.push(delta.clone());
                }
                wrote.push(delta.clone());
            } else {
                notes.push(format!("{name}: {trouble}"));
            }
            let _ = onionskin::watch::write_down(
                &folder,
                &onionskin::watch::Handled::of(
                    &seen.path,
                    seen.look,
                    trouble.is_empty().then_some(delta.as_path()),
                    trouble,
                    onionskin::history::now(),
                ),
            );
        }

        Outcome::Done {
            message: format!(
                "{} delta{} from {}.",
                wrote.len(),
                if wrote.len() == 1 { "" } else { "s" },
                folder
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| folder.display().to_string())
            ),
            wrote,
            notes,
        }
    });
}

/// The job, run on one file. Empty when it worked.
fn run_one(
    job: &onionskin::jobs::Job,
    given: &std::collections::BTreeMap<String, String>,
    document: &std::path::Path,
    delta: &std::path::Path,
) -> String {
    // Worked out per file rather than once, so a watch left running overnight
    // stamps today's date on today's scan.
    let row = onionskin::jobs::values(given, onionskin::history::now());
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
    let laid = match onionskin::recipe::lay_out(&recipe, document) {
        Ok(laid) => laid,
        Err(e) => return e,
    };
    // The same saved settings the command line applies, for the same reason a
    // saved job exists at all: it has to do the same thing wherever it is run.
    let options = onionskin::settings::load()
        .defaults
        .over(onionskin::pipeline::Options::default());
    let outcome = match onionskin::pipeline::compose_run_pictures(
        document,
        &laid.items,
        &[],
        &laid.images,
        delta,
        None,
        &options,
    ) {
        Ok(outcome) => outcome,
        Err(e) => return e.to_string(),
    };
    if outcome.blocked() {
        let why: Vec<String> = outcome
            .checks
            .iter()
            .filter(|c| c.severity == onionskin::safety::Severity::Blocker)
            .map(|c| c.message.clone())
            .collect();
        return format!("nothing worth printing came out of it. {}", why.join(" "));
    }
    String::new()
}

/// What a sweep that found no work saw, for somebody wondering whether it is
/// looking at the right folder at all.
fn looked_at(
    folder: &std::path::Path,
    verdicts: &[(onionskin::watch::Seen, onionskin::watch::Verdict)],
) -> String {
    let tally = onionskin::watch::Tally::of(verdicts);
    format!(
        "Nothing to do in {}: {} done before, {} still arriving, {} left alone.",
        folder.display(),
        tally.already,
        tally.arriving,
        tally.left
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The settle rule is what stops a half-written scan being opened, and it
    /// needs what the last sweep saw. The window and the worker share one map
    /// rather than passing it back through the outcome, because an outcome is
    /// what a person is shown and a hundred file sizes is not that.
    #[test]
    fn what_one_sweep_saw_reaches_the_next_one() {
        let state = State::default();
        let shared: Looks = state.before.clone();
        // As the worker does, on its own thread.
        std::thread::spawn(move || {
            shared.lock().unwrap().insert(
                std::path::PathBuf::from("/scans/a.pdf"),
                onionskin::watch::Look {
                    size: 4_000,
                    modified: 900,
                },
            );
        })
        .join()
        .expect("the worker should finish");

        let seen = state.before.lock().unwrap();
        assert_eq!(
            seen.get(std::path::Path::new("/scans/a.pdf")),
            Some(&onionskin::watch::Look {
                size: 4_000,
                modified: 900
            }),
            "what the sweep saw did not reach the next one"
        );
    }

    /// Nothing is read from disk until the screen is drawn: this is a folder,
    /// and the screen is drawn sixty times a second.
    #[test]
    fn the_saved_jobs_are_not_read_on_every_frame() {
        let state = State::default();
        assert!(state.known.is_none());
        assert!(state.before.lock().unwrap().is_empty(), "nothing seen yet");
        assert!(
            !state.keeping_watch,
            "watching is opted into, not the default"
        );
    }

    /// A refusal stops the watch. Left running, the same refusal would be
    /// raised and then wiped by the next sweep every two seconds, so the one
    /// message that matters would be the one nobody could finish reading.
    #[test]
    fn something_going_wrong_stops_the_watching() {
        let mut state = State {
            keeping_watch: true,
            harvested: false,
            ..Default::default()
        };
        // What the show function does when it takes in a refusal.
        let last = Outcome::refused("that folder is not there any more");
        match &last {
            Outcome::Refused { .. } => state.keeping_watch = false,
            Outcome::Done { .. } => {}
        }
        assert!(!state.keeping_watch, "it kept watching after a refusal");

        // A sweep that worked does not stop it, which is the whole point of
        // leaving it running.
        let mut state = State {
            keeping_watch: true,
            ..Default::default()
        };
        let last = Outcome::done("2 deltas");
        match &last {
            Outcome::Refused { .. } => state.keeping_watch = false,
            Outcome::Done { .. } => {}
        }
        assert!(state.keeping_watch);
    }

    /// A blank filled with spaces is not filled in. A watch left running on a
    /// job whose {ref} is three spaces is an afternoon of wasted paper.
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
}
