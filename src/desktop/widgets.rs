//! Small pieces of window used on more than one screen.

use eframe::egui;

use super::job::Outcome;
use super::picker::Picker;
use super::theme;

/// A screen's title and the sentence under it.
pub fn title(ui: &mut egui::Ui, heading: &str, lede: &str) {
    ui.add_space(4.0);
    ui.heading(heading);
    ui.label(egui::RichText::new(lede).weak());
    ui.add_space(10.0);
}

/// Quieter text, for the explanation beside a control.
pub fn hint(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).small().weak());
}

/// A coloured band with a message in it.
fn band(ui: &mut egui::Ui, colour: egui::Color32, heading: &str, body: &str) {
    egui::Frame::NONE
        .fill(colour.gamma_multiply(0.10))
        .stroke(egui::Stroke::new(1.0, colour.gamma_multiply(0.6)))
        .inner_margin(egui::Margin::symmetric(12, 10))
        .corner_radius(6)
        .show(ui, |ui| {
            ui.vertical(|ui| {
                if !heading.is_empty() {
                    ui.label(egui::RichText::new(heading).color(colour).strong());
                }
                for line in body.lines() {
                    if line.trim().is_empty() {
                        ui.add_space(4.0);
                    } else {
                        ui.label(line);
                    }
                }
            });
        });
}

pub fn refused(ui: &mut egui::Ui, body: &str) {
    band(ui, theme::REFUSED, "That did not happen", body);
}

pub fn caution(ui: &mut egui::Ui, body: &str) {
    band(ui, theme::CAUTION, "", body);
}

/// What a finished job produced, with a way to dismiss it.
///
/// Returns true when the person has read it and wants it gone.
pub fn outcome(ui: &mut egui::Ui, outcome: &Outcome) -> bool {
    let mut dismissed = false;
    match outcome {
        Outcome::Done {
            message,
            wrote,
            notes,
        } => {
            band(ui, theme::DONE, "Done", message);
            for path in wrote {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(path.display().to_string()).monospace());
                    // Showing a file in its folder is what a person does next,
                    // and hunting for it by hand is the tax for not offering.
                    if ui.button("Show the folder").clicked() {
                        show_in_folder(path);
                    }
                });
            }
            for note in notes {
                ui.add_space(6.0);
                caution(ui, note);
            }
        }
        Outcome::Refused { message } => refused(ui, message),
    }
    ui.add_space(8.0);
    if ui.button("Dismiss").clicked() {
        dismissed = true;
    }
    dismissed
}

/// A labelled row with the chosen file and a button to change it.
///
/// Returns true when the choice changed, so a screen can throw away whatever
/// it worked out about the file it was looking at before.
///
/// The browser is drawn by the window itself, so the answer comes back a frame
/// or two later rather than from the call that asked for it.
pub fn file_row(
    ui: &mut egui::Ui,
    picker: &mut Picker,
    label: &str,
    slot: &mut Option<std::path::PathBuf>,
    kinds: &[&str],
) -> bool {
    let mut changed = false;
    let who = ui.make_persistent_id(("file-row", label));
    if let Some(chosen) = picker.taken(who) {
        *slot = Some(chosen);
        changed = true;
    }

    ui.label(egui::RichText::new(label).strong());
    ui.horizontal(|ui| {
        if ui.button("Choose…").clicked() {
            picker.open(who, label, kinds, slot.as_deref());
        }
        match slot {
            Some(path) => {
                ui.label(
                    egui::RichText::new(
                        path.file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default(),
                    )
                    .monospace(),
                )
                .on_hover_text(path.display().to_string());
                if ui.small_button("×").clicked() {
                    *slot = None;
                    changed = true;
                }
            }
            None => hint(ui, "nothing chosen"),
        }
    });
    ui.add_space(6.0);
    changed
}

/// Where a result should be written, with a Save-as button.
///
/// `when_empty` is what the row says before a place has been chosen, and it
/// has to be the truth. A row that reads "beside the original, as
/// document.onionskin" beside a button that will not work until a place *is*
/// chosen tells somebody there is a default when there is none, and leaves
/// them staring at a greyed-out button with no idea what it wants.
pub fn save_row(
    ui: &mut egui::Ui,
    picker: &mut Picker,
    label: &str,
    slot: &mut Option<std::path::PathBuf>,
    suggested: &str,
    kinds: &[&str],
    when_empty: &str,
) {
    let who = ui.make_persistent_id(("save-row", label));
    if let Some(chosen) = picker.taken(who) {
        *slot = Some(chosen);
    }

    ui.label(egui::RichText::new(label).strong());
    ui.horizontal(|ui| {
        if ui.button("Save as…").clicked() {
            picker.save(who, label, kinds, slot.as_deref(), suggested);
        }
        match slot {
            Some(path) => {
                ui.label(egui::RichText::new(path.display().to_string()).monospace());
                if ui.small_button("×").clicked() {
                    *slot = None;
                }
            }
            None => hint(ui, when_empty),
        }
    });
    ui.add_space(6.0);
}

/// Open the folder a file is in, using whatever the desktop provides.
///
/// Best effort by design: on a machine with no desktop session there is
/// nothing to open, and failing quietly is right — the path is on screen
/// beside the button, so nothing is lost.
pub fn show_in_folder(path: &std::path::Path) {
    let folder = path.parent().unwrap_or(path);
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(windows) {
        "explorer"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(opener).arg(folder).spawn();
}
