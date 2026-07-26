//! Small pieces of window used on more than one screen.

use eframe::egui;

use super::job::Outcome;
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
