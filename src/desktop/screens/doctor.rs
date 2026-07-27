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
    /// What the last Delete-them did, shown until the screen is left. Kept so
    /// the button says something happened: the list it was next to goes to
    /// "none" straight away, which on its own reads like nothing occurred.
    tidied: Option<String>,
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
    show_what_is_kept(state, room);

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

/// What Onionskin is holding on this machine, and a way to be rid of it.
///
/// The same list `onionskin doctor` prints. A program that keeps things in a
/// hidden folder should say so in the place somebody looks to find out what
/// it is doing — and the people who never open a terminal are exactly the
/// ones who cannot go and look for themselves.
fn show_what_is_kept(state: &mut State, room: &mut Room) {
    let home = onionskin::calibrate::home_dir();
    room.ui
        .label(egui::RichText::new("What Onionskin keeps here").strong());
    widgets::hint(room.ui, &home.display().to_string());
    room.ui.add_space(4.0);

    let profiles = onionskin::calibrate::list_profiles().unwrap_or_default();
    widgets::hint(
        room.ui,
        &match profiles.len() {
            0 => "Calibration profiles: none yet".to_string(),
            1 => format!("Calibration profiles: 1 — {}", profiles[0].name),
            n => format!(
                "Calibration profiles: {n} — {}",
                profiles
                    .iter()
                    .map(|p| p.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        },
    );

    let (count, bytes) = kept_deltas(&home.join("deltas"));
    if count == 0 {
        widgets::hint(room.ui, "Deltas: none — they are deleted once printed");
    } else {
        room.ui.horizontal(|ui| {
            widgets::hint(
                ui,
                &format!("Deltas: {count} kept back, {}", describe_size(bytes)),
            );
            if ui.small_button("Delete them").clicked() {
                onionskin::delta::tidy_scratch(None);
                state.tidied = Some(format!("{} freed.", describe_size(bytes)));
            }
        });
    }
    // After the branch, not inside it: the moment the button works the count
    // above becomes "none", and a confirmation that only shows while there is
    // still something to delete is a confirmation nobody ever sees.
    if let Some(said) = &state.tidied {
        widgets::hint(room.ui, said);
    }
}

/// How many deltas are being held, and how much they come to.
fn kept_deltas(folder: &std::path::Path) -> (usize, u64) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return (0, 0);
    };
    entries
        .flatten()
        .filter_map(|entry| entry.metadata().ok())
        .filter(|meta| meta.is_file())
        .fold((0, 0), |(count, bytes), meta| {
            (count + 1, bytes + meta.len())
        })
}

/// A size somebody reads, rather than a number of bytes.
fn describe_size(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{} kB", bytes / 1024)
    } else {
        format!("{bytes} bytes")
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_kept_is_counted_and_sized_the_way_the_command_line_says_it() {
        // The window and `onionskin doctor` are two views of one answer, so
        // they must not disagree about how much is being held.
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(kept_deltas(dir.path()), (0, 0));
        assert_eq!(kept_deltas(&dir.path().join("never-made")), (0, 0));

        std::fs::write(dir.path().join("a.pdf"), vec![0u8; 3000]).unwrap();
        std::fs::write(dir.path().join("b.pdf"), vec![0u8; 1000]).unwrap();
        assert_eq!(kept_deltas(dir.path()), (2, 4000));

        assert_eq!(describe_size(0), "0 bytes");
        assert_eq!(describe_size(999), "999 bytes");
        assert_eq!(describe_size(4000), "3 kB");
        assert_eq!(describe_size(5 * 1024 * 1024), "5.0 MB");
    }

    #[test]
    fn a_folder_of_folders_is_not_counted_as_deltas() {
        // Only files are held deltas. A directory in there is not one, and
        // reporting it would offer to delete something this does not delete.
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("a-folder")).unwrap();
        std::fs::write(dir.path().join("real.pdf"), vec![0u8; 10]).unwrap();
        assert_eq!(kept_deltas(dir.path()), (1, 10));
    }
}
