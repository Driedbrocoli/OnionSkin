//! What works on this machine, and what is missing.
//!
//! The first screen anybody should look at when something will not go, and the
//! reason it exists is that "it did not work" is not a fault report. This turns
//! it into one: the renderer is missing, or LibreOffice is not installed, or
//! there is no font to write other alphabets with — each with what to do about
//! it, in a sentence.

use eframe::egui;

use super::Room;
use crate::theme;
use crate::widgets;

#[derive(Default)]
pub struct State {
    /// Filled in on the first frame, and again when asked. Held rather than
    /// recomputed each frame because looking for LibreOffice means walking a
    /// dozen paths on disk, sixty times a second.
    checks: Option<Vec<Check>>,
}

/// One thing that either works or does not.
struct Check {
    what: String,
    verdict: Verdict,
    detail: String,
}

enum Verdict {
    Works,
    /// Usable, but something is worth knowing.
    Caution,
    Missing,
}

pub fn show(state: &mut State, room: &mut Room) {
    widgets::title(
        room.ui,
        "This machine",
        "What Onionskin can do here, and what it cannot.",
    );

    if state.checks.is_none() {
        state.checks = Some(look());
    }
    if room.ui.button("Check again").clicked() {
        state.checks = Some(look());
    }
    room.ui.add_space(10.0);

    let Some(checks) = &state.checks else { return };
    for check in checks {
        let (colour, mark) = match check.verdict {
            Verdict::Works => (theme::DONE, "✔"),
            Verdict::Caution => (theme::CAUTION, "!"),
            Verdict::Missing => (theme::REFUSED, "✘"),
        };
        room.ui.horizontal_top(|ui| {
            ui.label(egui::RichText::new(mark).color(colour).strong().size(17.0));
            ui.vertical(|ui| {
                ui.label(egui::RichText::new(&check.what).strong());
                if !check.detail.is_empty() {
                    for line in check.detail.lines() {
                        widgets::hint(ui, line);
                    }
                }
            });
        });
        room.ui.add_space(8.0);
    }

    room.ui.separator();
    room.ui.add_space(6.0);
    widgets::hint(
        room.ui,
        "Onionskin never phones home: no telemetry, no update check, nothing \
         about your documents leaving this machine. It opens a socket when you \
         name a printer or a scanner, and when you ask it to find them — and \
         then it asks this network only, and talks to nothing beyond it.",
    );
}

/// Ask the same questions `onionskin doctor` asks.
fn look() -> Vec<Check> {
    let mut checks = Vec::new();

    checks.push(match onionskin::render::engine() {
        Ok(_) => Check {
            what: "Drawing PDF pages".into(),
            verdict: Verdict::Works,
            detail: String::new(),
        },
        Err(e) => Check {
            what: "Drawing PDF pages".into(),
            verdict: Verdict::Missing,
            detail: e.to_string(),
        },
    });

    checks.push(match onionskin::render::find_soffice() {
        Some(path) => Check {
            what: "Opening Word and OpenDocument files".into(),
            verdict: Verdict::Works,
            detail: format!("LibreOffice at {}", path.display()),
        },
        None => Check {
            what: "Opening Word and OpenDocument files".into(),
            verdict: Verdict::Works,
            detail: "LibreOffice is not installed, so Onionskin reads .docx, .odt and\n\
                     plain text itself. The words, tables and lists are all there;\n\
                     lines may not break exactly where Word does.\n\
                     Older formats — .doc, .rtf, spreadsheets, slides — still need\n\
                     LibreOffice: libreoffice.org/download."
                .into(),
        },
    });

    checks.push(match onionskin::font::suggest_system_font() {
        Some(path) => Check {
            what: "Writing in other alphabets".into(),
            verdict: Verdict::Works,
            detail: format!("A font with wide coverage at {}", path.display()),
        },
        None => Check {
            what: "Writing in other alphabets".into(),
            verdict: Verdict::Caution,
            detail: "No font with wide coverage was found. Western European text\n\
                     works without one; Greek, Cyrillic and the rest need a font file."
                .into(),
        },
    });

    checks.push(Check {
        what: "Printing and scanning over the network".into(),
        verdict: Verdict::Works,
        detail: "Onionskin speaks to printers and scanners directly, so nothing\n\
                 has to be installed for either."
            .into(),
    });

    checks
}
