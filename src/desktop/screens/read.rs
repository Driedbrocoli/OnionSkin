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
    /// Whether a file already at the destination may be written over.
    ///
    /// Off, and asked rather than assumed, because reading a scan a second
    /// time — after naming the font, or choosing Flow instead of Placed — is
    /// an ordinary thing to do, and the first thing somebody does with the
    /// result is edit it. Replacing it without a word throws that away.
    replace: bool,
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
            replace: false,
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

    fn extension(self) -> &'static str {
        match self {
            OutputKind::Docx => "docx",
            OutputKind::Odt => "odt",
            OutputKind::Onionskin => "onionskin",
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
        room.dropped,
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

    room.ui.collapsing("Read it in a particular font", |ui| {
        widgets::hint(
            ui,
            "Leave this alone and Onionskin works out what the page is set in \
             by itself. Name a font only to read an alphabet the built-in faces \
             do not cover — Greek, Cyrillic, Arabic — or to overrule what it \
             decided.",
        );
        widgets::file_row(
            ui,
            room.picker,
            "The font the page was set in",
            &mut state.font_file,
            &["ttf", "otf", "ttc"],
            room.dropped,
        );
    });

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
    room.ui
        .checkbox(&mut state.replace, "Replace it if it is already there");
    widgets::hint(
        room.ui,
        "Off, so reading a second scan cannot quietly write over the first one's \
         result — or over a file of your own that happens to have the same name.",
    );

    room.ui.add_space(6.0);
    let ready = state.scan.is_some();
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
        widgets::hint(room.ui, "Choose a scan to read.");
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

/// Where the result goes when nobody has said: beside the scan, named after
/// it. `invoice.png` becomes `invoice-read.docx`, so a folder of scans read
/// one after another ends up with a result per scan rather than one file
/// replaced over and over.
fn beside_the_scan(scan: &std::path::Path, format: OutputKind) -> PathBuf {
    let stem = scan
        .file_stem()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "scan".to_string());
    let mut path = scan.to_path_buf();
    path.set_file_name(format!("{stem}-read.{}", format.extension()));
    path
}

fn start(state: &mut State, room: &mut Room) {
    let Some(scan_path) = state.scan.clone() else {
        return;
    };
    let font_file = state.font_file.clone();
    let page_spec = state.page.clone();
    let format = state.format;
    let layout = state.layout;

    // Beside the scan and named after it, unless somebody has said otherwise,
    // because that is the folder they are already working in and that is what
    // the box on screen says will happen.
    //
    // It used to be a fixed `read.docx`, which is a name a person may well
    // have of their own — and it was written with no guard at all. Reading a
    // second scan replaced the first one's result, and a `read.docx` somebody
    // wrote themselves was destroyed outright. Neither said anything.
    let output = state
        .output
        .clone()
        .unwrap_or_else(|| beside_the_scan(&scan_path, format));

    // Nothing marks a Word or OpenDocument file as Onionskin's own, so there
    // is no way to tell one this screen wrote from one somebody typed. The
    // question therefore has to be asked rather than guessed at, and asked
    // before the reading rather than after it.
    if !state.replace && output.is_file() {
        room.jobs.refuse(format!(
            "'{}' is already there.\n\nTick 'Replace it if it is already there' to write over \
             it, or choose another name under 'Where to save it'.",
            output.display()
        ));
        return;
    }

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
                return Outcome::refused(format!("could not read {}: {e}", scan_path.display()));
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

        // A font named is a font used; otherwise the page is asked what it is
        // set in. Nobody looking at a scan of a letter knows, or should have
        // to know, which of three faces it was typed in — but somebody reading
        // a page of Greek does know that the built-in faces will not do, and
        // for them the choice is still there.
        let gray = image.to_luma8();
        let (text, matched) = match &font_file {
            Some(path) => {
                report.saying("Reading the letters…");
                let face = match font::EmbeddedFont::load(path) {
                    Ok(face) => face,
                    Err(e) => return Outcome::refused(e.to_string()),
                };
                match letters::read_with_font(
                    &gray,
                    &registration,
                    &letters::ReadOptions::default(),
                    &face,
                    None,
                ) {
                    Ok(text) => (text, None),
                    Err(e) => return Outcome::refused(e.to_string()),
                }
            }
            None => {
                report.saying("Working out what the page is set in…");
                match onionskin::typeface::read_and_match(&scan_path, &page_spec, false, false) {
                    Some((text, found)) => (text, Some(found)),
                    None => {
                        return Outcome::refused(
                            "There is no face on this machine to read the letters \
                             against, so the words cannot be made out — only where \
                             the ink is.\n\nInstall a common font (DejaVu or \
                             Liberation), or name the font the page was set in \
                             under 'Read it in a particular font'."
                                .to_string(),
                        );
                    }
                }
            }
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
        // What it decided, first, because it is the thing the person did not
        // have to answer and the thing most worth checking if the result looks
        // wrong.
        match &matched {
            Some(Some(face)) => notes.push(format!("Read as {}.", face.describe())),
            Some(None) => notes.push(
                "The page did not say clearly what it is set in, so the closest \
                 face on this machine was used. If the words look wrong, name the \
                 font under 'Read it in a particular font'."
                    .to_string(),
            ),
            None => {}
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A folder of scans read one after another has to end up with a result
    /// per scan.
    ///
    /// The name used to be a fixed `read.docx`, put in the scan's own folder.
    /// So reading `invoice-01.png` and then `invoice-02.png` left one file,
    /// holding the second — and anybody who had edited the first lost the
    /// edit. The box on screen said "beside the scan, named after it" the
    /// whole time, which was half true.
    #[test]
    fn the_result_is_named_after_the_scan_it_came_from() {
        let one = beside_the_scan(
            std::path::Path::new("/home/jo/scans/invoice-01.png"),
            OutputKind::Docx,
        );
        let two = beside_the_scan(
            std::path::Path::new("/home/jo/scans/invoice-02.png"),
            OutputKind::Docx,
        );
        assert_eq!(
            one,
            std::path::Path::new("/home/jo/scans/invoice-01-read.docx")
        );
        assert_ne!(one, two, "two scans were given one name");
        // Beside the scan, which is the folder they are already working in.
        assert_eq!(one.parent(), std::path::Path::new("/home/jo/scans").into());

        // Each kind gets its own extension, so choosing OpenDocument does not
        // hand somebody a `.docx` full of OpenDocument.
        for (kind, tail) in [
            (OutputKind::Docx, "invoice-01-read.docx"),
            (OutputKind::Odt, "invoice-01-read.odt"),
            (OutputKind::Onionskin, "invoice-01-read.onionskin"),
        ] {
            let made = beside_the_scan(std::path::Path::new("/home/jo/scans/invoice-01.png"), kind);
            assert_eq!(made.file_name().unwrap().to_string_lossy(), tail);
        }
    }

    /// A scan whose name is nothing but an extension still gets a result.
    #[test]
    fn a_scan_with_no_name_to_speak_of_still_gets_one() {
        let made = beside_the_scan(std::path::Path::new("/tmp/.png"), OutputKind::Docx);
        // `.png` is all extension and no stem to most people, and a file stem
        // to Rust. Either way the result is a name, not an empty one.
        let name = made.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.ends_with("-read.docx"), "{name}");
        assert!(name.len() > "-read.docx".len(), "{name}");
    }
}
