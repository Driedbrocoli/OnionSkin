//! Printers and scanners, spoken to directly.
//!
//! Onionskin talks IPP to a printer and eSCL to a scanner itself, so neither
//! needs a driver, a spooler or anything else installed. That is worth saying
//! on the screen, because somebody who has spent an afternoon installing
//! printer software will not otherwise believe an address is all that is
//! wanted.
//!
//! This is also the only screen that opens a socket, and it says so.

use eframe::egui;

use super::Room;
use crate::job::Outcome;
use crate::widgets;
use onionskin::printer;

pub struct State {
    server: String,
    printers: Option<Result<Vec<printer::Printer>, String>>,

    /// The printer to send to, and what to send it.
    printer_uri: String,
    to_print: Option<std::path::PathBuf>,
    copies: u32,

    scanner_uri: String,
    dpi: u32,
    colour: bool,
    scan_to: Option<std::path::PathBuf>,
}

impl Default for State {
    fn default() -> Self {
        State {
            // CUPS on this machine, which is where a printer plugged in by USB
            // turns up. A printer of its own is named directly.
            server: "ipp://127.0.0.1:631/".into(),
            printers: None,
            printer_uri: String::new(),
            to_print: None,
            copies: 1,
            scanner_uri: String::new(),
            dpi: 300,
            colour: false,
            scan_to: None,
        }
    }
}

pub fn show(state: &mut State, room: &mut Room) {
    widgets::title(
        room.ui,
        "Printers and scanners",
        "Spoken to directly, so neither needs anything installed.",
    );

    // ---------------------------------------------------------------- find
    room.ui.label(egui::RichText::new("Where to look").strong());
    room.ui.horizontal(|ui| {
        ui.text_edit_singleline(&mut state.server);
        if ui
            .add_enabled(!room.jobs.busy(), egui::Button::new("List printers"))
            .clicked()
        {
            let server = state.server.clone();
            state.printers = None;
            room.jobs.start("Asking for printers", move |report| {
                report.saying(format!("Asking {server}…"));
                match printer::printers(&server) {
                    Ok(found) => Outcome::done(format!(
                        "{} printer{} found.",
                        found.len(),
                        if found.len() == 1 { "" } else { "s" }
                    )),
                    Err(e) => Outcome::refused(e.to_string()),
                }
            });
        }
    });
    widgets::hint(
        room.ui,
        "The default is the print server on this machine. A printer of its own \
         is named directly, for example ipp://printer.local/ipp/print",
    );

    room.ui.add_space(14.0);
    room.ui.separator();

    // --------------------------------------------------------------- print
    room.ui.add_space(10.0);
    room.ui.label(egui::RichText::new("Send a PDF to print").strong());
    widgets::file_row(room.ui, room.picker, "The PDF", &mut state.to_print, &["pdf"],
        room.dropped,
    );
    room.ui.horizontal(|ui| {
        ui.label("Printer");
        ui.text_edit_singleline(&mut state.printer_uri);
    });
    room.ui.horizontal(|ui| {
        ui.label("Copies");
        ui.add(egui::DragValue::new(&mut state.copies).range(1..=99));
    });

    let can_print = state.to_print.is_some() && !state.printer_uri.trim().is_empty();
    if room
        .ui
        .add_enabled(
            can_print && !room.jobs.busy(),
            egui::Button::new(egui::RichText::new("Print it").strong()),
        )
        .clicked()
    {
        let file = state.to_print.clone().expect("checked");
        let uri = state.printer_uri.trim().to_string();
        let options = printer::PrintOptions {
            copies: state.copies,
            ..Default::default()
        };
        room.jobs.start("Printing", move |report| {
            report.saying(format!("Sending to {uri}…"));
            match printer::print_file(&uri, &file, &options) {
                Ok(_) => Outcome::done(
                    "Sent to the printer.\n\nOnionskin asked for no scaling, so the \
                     delta lands where it was measured. If the printer's own \
                     settings override that, nothing will line up.",
                ),
                Err(e) => Outcome::refused(e.to_string()),
            }
        });
    }
    if !can_print {
        widgets::hint(room.ui, "Choose a PDF and name a printer first.");
    }

    room.ui.add_space(14.0);
    room.ui.separator();

    // ---------------------------------------------------------------- scan
    room.ui.add_space(10.0);
    room.ui
        .label(egui::RichText::new("Scan a sheet from a printer").strong());
    room.ui.horizontal(|ui| {
        ui.label("Scanner");
        ui.text_edit_singleline(&mut state.scanner_uri);
    });
    widgets::hint(room.ui, "for example http://printer.local/eSCL");
    room.ui.horizontal(|ui| {
        ui.label("Resolution");
        ui.add(egui::DragValue::new(&mut state.dpi).range(75..=1200).suffix(" dpi"));
        ui.checkbox(&mut state.colour, "In colour");
    });
    widgets::hint(
        room.ui,
        "300 dpi is the one to use. Higher is slower and reads no better.",
    );
    widgets::save_row(
        room.ui,
        room.picker,
        "Save the scan as",
        &mut state.scan_to,
        "scan.png",
        &["png", "jpg", "tiff"],
        "if you do not choose, it goes in this folder as scan.png",
    );

    let can_scan = !state.scanner_uri.trim().is_empty();
    if room
        .ui
        .add_enabled(
            can_scan && !room.jobs.busy(),
            egui::Button::new(egui::RichText::new("Scan").strong()),
        )
        .clicked()
    {
        let uri = state.scanner_uri.trim().to_string();
        let target = state
            .scan_to
            .clone()
            .unwrap_or_else(|| std::path::PathBuf::from("scan.png"));
        let request = printer::ScanRequest {
            resolution: state.dpi,
            colour: state.colour,
            ..Default::default()
        };
        room.previews.forget(&target);
        room.jobs.start("Scanning", move |report| {
            // The head has to travel the length of the sheet, which is ten or
            // fifteen seconds and looks like nothing happening.
            report.saying("Waiting for the scanner — this takes a few seconds…");
            match printer::scan_to(&uri, &request, &target) {
                Ok(_) => Outcome::wrote("Scanned.", vec![target]),
                Err(e) => Outcome::refused(e.to_string()),
            }
        });
    }
    if !can_scan {
        widgets::hint(room.ui, "Name a scanner first.");
    }

    // -------------------------------------------------------------- result
    if let Some(outcome) = &room.jobs.last {
        room.ui.add_space(12.0);
        if widgets::outcome(room.ui, outcome) {
            room.jobs.dismiss();
        }
    }

    room.ui.add_space(16.0);
    widgets::caution(
        room.ui,
        "This is the only screen that uses the network, and it talks to the \
         machine you name and to nothing else. Nothing about your documents \
         goes anywhere near the internet.",
    );
}
