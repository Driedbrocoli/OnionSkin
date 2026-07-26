//! Turning a scan into something with a cursor in it.
//!
//! Onionskin finds every letter on a scanned page, but only *reads* them
//! against the font the page was set in — matching ink against that font's
//! own glyphs is what tells an O from a 0, and it is both more accurate and
//! far less code than guessing from shape alone. Without the font there is
//! nowhere to put the words, so this screen refuses rather than write a
//! document that is really just guesses at where the ink is.

use std::path::PathBuf;

use eframe::egui;

use super::Room;
use crate::job::Outcome;
use crate::widgets;
use onionskin::font;
use onionskin::geometry;
use onionskin::letters;
use onionskin::office;
use onionskin::scan;

pub struct State {
    scan: Option<PathBuf>,
    font_file: Option<PathBuf>,
    page: String,
    format: OutputKind,
    layout: office::Layout,
    output: Option<PathBuf>,
}

impl Default for State {
    fn default() -> Self {
        State {
            scan: None,
            font_file: None,
            page: "a4".to_string(),
            format: OutputKind::Docx,
            layout: office::Layout::Placed,
            output: None,
        }
    }
}

/// What kind of file to write. Kept as a choice on screen rather than read
/// back off the save path's extension, so picking "Word document" and typing
/// a plain name still gets a Word document.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OutputKind {
    Docx,
    Odt,
    Onionskin,
}

impl OutputKind {
    const ALL: [OutputKind; 3] = [OutputKind::Docx, OutputKind::Odt, OutputKind::Onionskin];

    fn label(self) -> &'static str {
        match self {
            OutputKind::Docx => "Word document (.docx)",
            OutputKind::Odt => "OpenDocument text (.odt)",
            OutputKind::Onionskin => "Onionskin document (.onionskin)",
        }
    }

    fn suggested_name(self) -> &'static str {
        match self {
            OutputKind::Docx => "read.docx",
            OutputKind::Odt => "read.odt",
            OutputKind::Onionskin => "read.onionskin",
        }
    }

    fn extensions(self) -> &'static [&'static str] {
        match self {
            OutputKind::Docx => &["docx"],
            OutputKind::Odt => &["odt"],
            OutputKind::Onionskin => &["onionskin"],
        }
    }
}

pub fn show(state: &mut State, room: &mut Room) {
    let screen = super::Screen::Read;
    widgets::title(room.ui, screen.name(), screen.lede());

    widgets::file_row(
        room.ui,
        room.picker,
        "The scan",
        &mut state.scan,
        &["png", "jpg", "jpeg", "tif", "tiff", "bmp"],
    );

    room.ui.horizontal(|ui| {
        ui.label("Paper size");
        ui.text_edit_singleline(&mut state.page);
    });
    widgets::hint(
        room.ui,
        "a4, letter, legal, a5, or a custom size such as 210x297 (mm).",
    );
    room.ui.add_space(6.0);

    widgets::file_row(
        room.ui,
        room.picker,
        "The font the page was set in",
        &mut state.font_file,
        &["ttf", "otf", "ttc"],
    );
    widgets::hint(
        room.ui,
        "Required. Without it, Onionskin can see where the ink is on the page \
         but not which letters it is — there would be nothing to write down.",
    );

    room.ui.add_space(6.0);
    room.ui.collapsing("Settings", |ui| {
        ui.horizontal(|ui| {
            ui.label("Save as");
            egui::ComboBox::from_id_salt("read-format")
                .selected_text(state.format.label())
                .show_ui(ui, |ui| {
                    for kind in OutputKind::ALL {
                        ui.selectable_value(&mut state.format, kind, kind.label());
                    }
                });
        });
        widgets::hint(
            ui,
            "An Onionskin document can be edited again with 'onionskin write', \
             and turned into Word or OpenDocument later.",
        );

        ui.horizontal(|ui| {
            ui.label("Layout");
            egui::ComboBox::from_id_salt("read-layout")
                .selected_text(match state.layout {
                    office::Layout::Placed => "Placed — each line where it was on the paper",
                    office::Layout::Flow => "Flow — ordinary paragraphs",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(
                        &mut state.layout,
                        office::Layout::Placed,
                        "Placed — each line where it was on the paper",
                    );
                    ui.selectable_value(
                        &mut state.layout,
                        office::Layout::Flow,
                        "Flow — ordinary paragraphs",
                    );
                });
        });
        widgets::hint(
            ui,
            "Placed keeps the page looking like the scan. Flow reads more \
             naturally in a word processor, but loses that layout.",
        );
    });

    room.ui.add_space(10.0);
    widgets::save_row(
        room.ui,
        room.picker,
        "Where to save it",
        &mut state.output,
        state.format.suggested_name(),
        state.format.extensions(),
        "beside the scan, named after it",
    );

    room.ui.add_space(6.0);
    let ready = state.scan.is_some() && state.font_file.is_some();
    let busy = room.jobs.busy();
    if room
        .ui
        .add_enabled(
            ready && !busy,
            egui::Button::new(egui::RichText::new("Read the scan").strong()),
        )
        .clicked()
    {
        start(state, room);
    }
    if !ready {
        widgets::hint(room.ui, "Choose the scan and the font it was set in first.");
    }

    if let Some(outcome) = &room.jobs.last {
        room.ui.add_space(12.0);
        if widgets::outcome(room.ui, outcome) {
            room.jobs.dismiss();
        }
    }

    room.ui.add_space(16.0);
    widgets::caution(
        room.ui,
        "A letter Onionskin cannot match with confidence comes back as '?', not \
         a guess. Read the result against the scan before relying on it.",
    );
}

fn start(state: &mut State, room: &mut Room) {
    let (Some(scan_path), Some(font_file)) = (state.scan.clone(), state.font_file.clone()) else {
        return;
    };
    let page_spec = state.page.clone();
    let format = state.format;
    let layout = state.layout;

    // Beside the scan unless somebody has said otherwise, because that is the
    // folder they are already working in.
    let output = state.output.clone().unwrap_or_else(|| {
        let mut path = scan_path.clone();
        path.set_file_name(format.suggested_name());
        path
    });

    room.previews.forget(&output);
    let target = output.clone();
    room.jobs.start("Reading the scan", move |report| {
        let page = match geometry::parse_page(&page_spec) {
            Ok(page) => page,
            Err(e) => return Outcome::refused(e),
        };

        report.saying("Registering the scan…");
        let image = match image::open(&scan_path) {
            Ok(image) => image,
            Err(e) => {
                return Outcome::refused(format!(
                    "could not read {}: {e}",
                    scan_path.display()
                ));
            }
        };
        let registration = match scan::register(&image, scan::ScanOptions::new(page)) {
            Ok(registration) => registration,
            // The library's own wording points at a command-line flag this
            // screen does not have. Its diagnosis is still the useful part —
            // keep that, and give the next step this screen can actually take.
            Err(scan::ScanError::Detection(why)) => {
                return Outcome::refused(format!(
                    "{why}\n\nThis screen expects the whole sheet visible in the \
                     scan, with a little of the scanner's own background \
                     showing round it, so the edge can be found and the page \
                     straightened."
                ));
            }
            Err(e) => return Outcome::refused(e.to_string()),
        };

        report.saying("Reading the letters…");
        let font = match font::EmbeddedFont::load(&font_file) {
            Ok(font) => font,
            Err(e) => return Outcome::refused(e.to_string()),
        };
        let gray = image.to_luma8();
        let text = match letters::read_with_font(
            &gray,
            &registration,
            &letters::ReadOptions::default(),
            &font,
            None,
        ) {
            Ok(text) => text,
            Err(e) => return Outcome::refused(e.to_string()),
        };

        report.saying("Writing the document…");
        let document = match office::document_from_page(&text, page) {
            Ok(document) => document,
            Err(e) => return Outcome::refused(e.to_string()),
        };
        let write_result = match format {
            OutputKind::Onionskin => document.save(&target).map_err(|e| e.to_string()),
            OutputKind::Docx | OutputKind::Odt => {
                let office_format = if format == OutputKind::Docx {
                    office::Format::Docx
                } else {
                    office::Format::Odt
                };
                office::write(&document, office_format, layout)
                    .map_err(|e| e.to_string())
                    .and_then(|bytes| {
                        std::fs::write(&target, bytes)
                            .map_err(|e| format!("could not write {}: {e}", target.display()))
                    })
            }
        };
        if let Err(e) = write_result {
            return Outcome::refused(e);
        }

        let lines = text.lines.len();
        let words = text.word_count();
        let mut notes = Vec::new();
        if text.discarded > 0 {
            notes.push(format!(
                "{} mark{} on the scan were set aside as too small or too large \
                 to be a letter.",
                text.discarded,
                if text.discarded == 1 { "" } else { "s" }
            ));
        }

        Outcome::Done {
            message: format!(
                "{lines} line{}, {words} word{} read from the scan.",
                if lines == 1 { "" } else { "s" },
                if words == 1 { "" } else { "s" }
            ),
            wrote: vec![target],
            notes,
        }
    });
}
