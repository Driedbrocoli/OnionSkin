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
use std::sync::{Arc, Mutex};

use crate::job::Outcome;
use crate::widgets;
use onionskin::{discover, printer};

/// What a search turned up, filled in by the worker thread and read by the
/// window on its next frame.
///
/// Shared rather than sent back through the job's own channel because that
/// channel carries a message for a person to read, and this is a list for the
/// window to draw — two different things that happen to arrive together.
type Found = Arc<Mutex<Vec<Device>>>;

/// One printer or scanner somebody could choose.
#[derive(Clone)]
struct Device {
    name: String,
    detail: String,
    uri: String,
    scanner: bool,
}

pub struct State {
    server: String,
    found: Found,

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
            found: Arc::new(Mutex::new(Vec::new())),
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
    if room
        .ui
        .add_enabled(
            !room.jobs.busy(),
            egui::Button::new(egui::RichText::new("Find my printers and scanners").strong()),
        )
        .clicked()
    {
        let server = state.server.clone();
        let into = Arc::clone(&state.found);
        into.lock().map(|mut list| list.clear()).ok();
        room.jobs.start("Looking", move |report| {
            let mut devices = Vec::new();

            // What is already set up here, which is where a printer plugged in
            // by USB appears. Asking costs nothing and answers immediately.
            report.saying("Asking this machine…");
            for printer in printer::printers(&server).unwrap_or_default() {
                let uri = if printer.uri.is_empty() {
                    printer.name.clone()
                } else {
                    printer.uri.clone()
                };
                devices.push(Device {
                    name: printer.name.clone(),
                    detail: [printer.model.as_str(), printer.location.as_str()]
                        .iter()
                        .filter(|part| !part.is_empty())
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(" — "),
                    uri,
                    scanner: false,
                });
            }

            // Anything plugged into this machine, through SANE. A scanner on
            // a USB cable does not announce itself on any network, so without
            // this it would be the one device the search could not see.
            for device in onionskin::acquire::list_devices().unwrap_or_default() {
                devices.push(Device {
                    name: device.description.clone(),
                    detail: "plugged into this machine".to_string(),
                    uri: device.name.clone(),
                    scanner: true,
                });
            }

            // Then whatever announces itself on the network, which needs
            // nothing to have been set up anywhere.
            report.saying("Listening on the network…");
            for one in discover::find(discover::LISTEN_FOR) {
                devices.push(Device {
                    name: one.name.clone(),
                    detail: one.model().unwrap_or("").to_string(),
                    uri: one.plain_uri(),
                    scanner: one.kind == discover::Kind::Scanner,
                });
            }

            let count = devices.len();
            if let Ok(mut list) = into.lock() {
                *list = devices;
            }
            match count {
                0 => Outcome::done(
                    "Nothing found. Check it is switched on, and plugged in or on \
                     this network. A printer that does not announce itself can \
                     still be typed in below."
                        .to_string(),
                ),
                1 => Outcome::done("One found. Choose it below.".to_string()),
                many => Outcome::done(format!("{many} found. Choose one below.")),
            }
        });
    }

    // The list, with the choosing done by clicking rather than by copying a
    // URI out of it — which is the whole reason for finding them.
    let devices: Vec<Device> = state
        .found
        .lock()
        .map(|list| list.clone())
        .unwrap_or_default();
    if !devices.is_empty() {
        room.ui.add_space(8.0);
        for device in &devices {
            room.ui.horizontal(|ui| {
                let what = if device.scanner {
                    "Scan with"
                } else {
                    "Print to"
                };
                if ui.button(what).clicked() {
                    if device.scanner {
                        state.scanner_uri = device.uri.clone();
                    } else {
                        state.printer_uri = device.uri.clone();
                    }
                }
                ui.label(egui::RichText::new(&device.name).strong());
                if !device.detail.is_empty() {
                    widgets::hint(ui, &device.detail);
                }
            });
            room.ui.horizontal(|ui| {
                ui.add_space(24.0);
                widgets::hint(ui, &device.uri);
            });
        }
    }

    room.ui.add_space(8.0);
    room.ui.collapsing("Type an address instead", |ui| {
        ui.label("Print server");
        ui.text_edit_singleline(&mut state.server);
        widgets::hint(
            ui,
            "The default is the print server on this machine, where a printer \
                 plugged in by USB appears. A printer of its own is named \
                 directly, for example ipp://printer.local/ipp/print",
        );
    });

    room.ui.add_space(14.0);
    room.ui.separator();

    // --------------------------------------------------------------- print
    room.ui.add_space(10.0);
    room.ui
        .label(egui::RichText::new("Send a PDF to print").strong());
    widgets::file_row(
        room.ui,
        room.picker,
        "The PDF",
        &mut state.to_print,
        &["pdf"],
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
        ui.add(
            egui::DragValue::new(&mut state.dpi)
                .range(75..=1200)
                .suffix(" dpi"),
        );
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
        // A scanner on a cable is not a scanner on the network, and the two
        // are driven by completely different means: eSCL over HTTP for one,
        // SANE for the other. Which one this is can be told from the name —
        // an address has a scheme, a SANE device is `plustek:libusb:001:004`.
        let attached = !uri.starts_with("http://") && !uri.starts_with("https://");
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
            let outcome = if attached {
                onionskin::acquire::acquire(
                    &onionskin::acquire::AcquireOptions {
                        device: Some(uri.clone()),
                        resolution: request.resolution,
                        colour: request.colour,
                    },
                    &target,
                )
                .map(|_| ())
                .map_err(|e| e.to_string())
            } else {
                printer::scan_to(&uri, &request, &target)
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            };
            match outcome {
                Ok(()) => Outcome::wrote("Scanned.", vec![target]),
                Err(e) => Outcome::refused(e),
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
        "This is the only screen that uses the network. Finding devices asks \
         this network and no other; printing and scanning talk to the machine \
         you chose and to nothing else. Nothing about your documents goes \
         anywhere near the internet.",
    );
}
