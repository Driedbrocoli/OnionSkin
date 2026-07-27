//! What was added to which sheet, and when.
//!
//! Overprinting is the one operation this program does that cannot be undone.
//! Toner does not come off paper, so a delta printed twice onto the same sheet
//! puts every letter down twice — a little heavier, a little blurred, and
//! unfixable. Nothing about the second time looks different from the first.
//!
//! So every delta is remembered by a fingerprint of the file itself, and this
//! is where somebody looks when they cannot remember whether they already
//! printed it. "What did we add to that invoice, and when" is a question people
//! ask months later about a sheet of paper in a filing cabinet, and the answer
//! used to be nowhere.
//!
//! **The words themselves are not kept.** A log of everything anybody ever
//! wrote onto anything would be a far more sensitive file than any document it
//! describes, sitting in a home directory being backed up.

use eframe::egui;

use super::Room;
use crate::widgets;

#[derive(Default)]
pub struct State {
    /// Loaded when the screen is first opened, and after anything changes it.
    entries: Option<Vec<onionskin::history::Entry>>,
    /// Set when Forget has been pressed once, so the second press means it.
    asked_to_forget: bool,
}

pub fn show(state: &mut State, room: &mut Room) {
    widgets::title(
        room.ui,
        "What was added, and when",
        "Every delta this machine has written, most recent first.",
    );

    let entries = state
        .entries
        .get_or_insert_with(|| onionskin::history::recent(onionskin::history::KEEP));

    if entries.is_empty() {
        widgets::hint(
            room.ui,
            "Nothing yet. Every delta Onionskin writes is recorded here, so you \
             can tell whether one has already been printed.",
        );
        return;
    }

    widgets::hint(
        room.ui,
        "Where the files were, how many pages and additions, and a fingerprint \
         of the delta. Not the words themselves — those stay in your documents.",
    );
    room.ui.add_space(8.0);

    room.ui.horizontal(|ui| {
        if ui.button("Refresh").clicked() {
            state.entries = None;
            state.asked_to_forget = false;
        }
        ui.add_space(12.0);
        if state.asked_to_forget {
            if ui
                .button(egui::RichText::new("Yes, forget all of it").strong())
                .clicked()
            {
                onionskin::history::forget();
                state.entries = None;
                state.asked_to_forget = false;
            }
            if ui.button("Keep it").clicked() {
                state.asked_to_forget = false;
            }
        } else if ui.button("Forget everything").clicked() {
            state.asked_to_forget = true;
        }
    });
    if state.asked_to_forget {
        widgets::caution(
            room.ui,
            "This only forgets the record. It does not un-print anything, and it \
             does not delete any delta.",
        );
    }

    room.ui.add_space(8.0);
    let Some(entries) = &state.entries else {
        return;
    };
    room.ui
        .label(egui::RichText::new(format!("{} remembered", entries.len())).strong());
    room.ui.add_space(4.0);

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(room.ui, |ui| {
            for entry in entries {
                ui.horizontal_wrapped(|ui| {
                    ui.label(egui::RichText::new(entry.when()).monospace().weak());
                    ui.label(egui::RichText::new(&entry.source).strong());
                    ui.label(format!(
                        "→ {} addition(s) on {} page(s)",
                        entry.additions, entry.pages
                    ));
                });
                ui.horizontal_wrapped(|ui| {
                    ui.add_space(12.0);
                    ui.label(egui::RichText::new(&entry.delta).monospace().small().weak());
                    ui.label(egui::RichText::new(entry.how_long_ago()).small().weak());
                });
                ui.separator();
            }
        });
}
