//! Small pieces of window used on more than one screen.

use eframe::egui;

use super::job::Outcome;
use super::picker::Picker;
use super::theme;

/// Take the first dropped file this control can use, if any.
///
/// A row that accepts anything takes the first file outright. A row with a
/// list of kinds only takes one it recognises, so a scan dropped on a screen
/// that wants a document is left for the control that wants a scan.
fn claim(dropped: &mut Vec<std::path::PathBuf>, kinds: &[&str]) -> Option<std::path::PathBuf> {
    let at = dropped.iter().position(|path| {
        if kinds.is_empty() {
            return true;
        }
        path.extension()
            .and_then(|kind| kind.to_str())
            .map(|kind| kinds.iter().any(|wanted| wanted.eq_ignore_ascii_case(kind)))
            .unwrap_or(false)
    })?;
    Some(dropped.remove(at))
}

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
                    // Looking at what came out is what a person does next, and
                    // hunting for it by hand is the tax for not offering.
                    if ui.button("Open it").clicked() {
                        onionskin::install::open_with_desktop(path);
                    }
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
///
/// It also claims a file dropped on the window: each row takes the first one it
/// can use and leaves the rest, so two documents dropped on the comparing
/// screen fill both slots in the order the rows are drawn.
pub fn file_row(
    ui: &mut egui::Ui,
    picker: &mut Picker,
    label: &str,
    slot: &mut Option<std::path::PathBuf>,
    kinds: &[&str],
    dropped: &mut Vec<std::path::PathBuf>,
) -> bool {
    let mut changed = false;
    let who = ui.make_persistent_id(("file-row", label));
    if let Some(chosen) = picker.taken(who) {
        onionskin::settings::remember_folder(&chosen);
        *slot = Some(chosen);
        changed = true;
    }
    if let Some(claimed) = claim(dropped, kinds) {
        *slot = Some(claimed);
        changed = true;
    }

    ui.label(egui::RichText::new(label).strong());
    ui.horizontal(|ui| {
        if ui.button("Choose…").clicked() {
            let start = onionskin::settings::start_in(slot.as_deref());
            picker.open(who, label, kinds, Some(&start));
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
        onionskin::settings::remember_output_folder(&chosen);
        *slot = Some(chosen);
    }

    ui.label(egui::RichText::new(label).strong());
    ui.horizontal(|ui| {
        if ui.button("Save as…").clicked() {
            // Where the last result went, which is nearly always where the
            // next one should go too.
            let start = slot
                .clone()
                .or_else(|| onionskin::settings::load().last_output_folder)
                .unwrap_or_else(|| onionskin::settings::start_in(None));
            picker.save(who, label, kinds, Some(&start), suggested);
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
/// The work is in [`onionskin::install::open_with_desktop`], which the command
/// line uses too — "which command opens a file on this operating system" is
/// not a question worth answering twice.
pub fn show_in_folder(path: &std::path::Path) {
    onionskin::install::open_with_desktop(path.parent().unwrap_or(path));
}

#[cfg(test)]
mod tests;
