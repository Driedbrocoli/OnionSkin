//! A barcode or a QR code, on a sheet that is otherwise already printed.
//!
//! Both are worked out on this machine. There is no library behind them and
//! nothing is fetched, which matters here more than it sounds: the ordinary way
//! to get a barcode is to type what you want encoded into somebody's website,
//! and an asset number or a patient reference is not a thing to hand over in
//! exchange for a picture of it.
//!
//! # The one thing this screen has to say out loud
//!
//! A barcode has to go on blank paper. Toner goes on top of what is already
//! printed, and printing showing through the bars changes their widths — which
//! does not produce a barcode that reads wrongly, it produces one that does not
//! read at all. So the warning is above the button, and the screen points at
//! "Find the gaps on a form" rather than leaving somebody to find out with sixty
//! sheets printed.

use eframe::egui;

use super::{beside, Room};
use crate::job::Outcome;
use crate::widgets;

/// How wide one module is by default, in millimetres — the same numbers the
/// command line uses, so the same job done either way comes out the same.
const BARCODE_MODULE_MM: f64 = 0.4;
const QR_MODULE_MM: f64 = 0.8;

pub struct State {
    document: Option<std::path::PathBuf>,
    output: Option<std::path::PathBuf>,
    text: String,
    /// A square read by a telephone, rather than bars read by a scanner.
    qr: bool,
    x_mm: f64,
    y_mm: f64,
    module_mm: f64,
    /// Left alone, the module size is the one that suits the kind of code.
    own_module: bool,
    height_mm: f64,
    level: String,
    caption: bool,
    page: usize,
}

impl Default for State {
    fn default() -> Self {
        State {
            document: None,
            output: None,
            text: String::new(),
            qr: false,
            x_mm: 20.0,
            y_mm: 40.0,
            module_mm: BARCODE_MODULE_MM,
            own_module: false,
            height_mm: 15.0,
            level: "medium".to_string(),
            caption: true,
            page: 1,
        }
    }
}

impl State {
    /// The module size in force: the one asked for, or the one that suits.
    fn module(&self) -> f64 {
        match self.own_module {
            true => self.module_mm,
            false if self.qr => QR_MODULE_MM,
            false => BARCODE_MODULE_MM,
        }
    }

    /// The symbol as it stands, so the screen can say how big it comes out
    /// before anybody prints it.
    fn symbol(&self) -> Option<onionskin::barcode::Symbol> {
        let text = self.text.trim();
        if text.is_empty() {
            return None;
        }
        match self.qr {
            true => onionskin::barcode::qr::encode(
                text,
                onionskin::barcode::qr::Ecc::parse(&self.level)?,
            )
            .ok(),
            false => onionskin::barcode::code128::encode(text).ok(),
        }
    }

    /// How big it comes out on paper: across, and down.
    fn size_mm(&self) -> Option<(f64, f64)> {
        let symbol = self.symbol()?;
        let module = self.module();
        Some(match self.qr {
            true => (symbol.width_mm(module), symbol.height_mm(module)),
            false => (
                symbol.width_mm(module),
                self.height_mm + symbol.quiet as f64 * module * 2.0,
            ),
        })
    }
}

pub fn show(state: &mut State, room: &mut Room) {
    widgets::title(
        room.ui,
        "A barcode or a QR code",
        "An asset number, a file reference, a link to what comes next.",
    );

    widgets::hint(
        room.ui,
        "Worked out on this machine. Nothing is sent anywhere, which is the \
         point when the thing being encoded is a reference somebody cares about.",
    );
    room.ui.add_space(10.0);

    widgets::file_row(
        room.ui,
        room.picker,
        "The sheet",
        &mut state.document,
        &[
            "pdf", "png", "jpg", "jpeg", "tif", "tiff", "bmp", "docx", "odt",
        ],
        room.dropped,
    );
    widgets::save_row(
        room.ui,
        room.picker,
        "Write the delta to",
        &mut state.output,
        "barcode.pdf",
        &["pdf"],
        "beside the sheet, as NAME-barcode.pdf",
    );

    room.ui.add_space(8.0);
    room.ui.horizontal(|ui| {
        ui.label(egui::RichText::new("What it says").strong());
        ui.add(
            egui::TextEdit::singleline(&mut state.text)
                .hint_text("INV-2024-00817")
                .desired_width(280.0),
        );
    });

    room.ui.add_space(4.0);
    room.ui.horizontal(|ui| {
        ui.selectable_value(&mut state.qr, false, "Barcode");
        ui.selectable_value(&mut state.qr, true, "QR code");
    });
    widgets::hint(
        room.ui,
        match state.qr {
            true => {
                "A square, read by a telephone camera. Holds a web address, \
                     or a good deal of text."
            }
            false => {
                "Bars, read by a handheld scanner. Letters, digits and \
                      punctuation — nothing accented."
            }
        },
    );

    room.ui.add_space(8.0);
    room.ui.horizontal(|ui| {
        ui.label("Top-left corner");
        millimetres(ui, &mut state.x_mm);
        ui.label(",");
        millimetres(ui, &mut state.y_mm);
        ui.label("Page");
        ui.add(egui::DragValue::new(&mut state.page).range(1..=9999));
    });

    // How big it comes out, while there is still time to move it.
    match state.size_mm() {
        Some((across, down)) => {
            room.ui.label(
                egui::RichText::new(format!(
                    "It comes out {across:.0} x {down:.0} mm, so it covers \
                     {:.0},{:.0} to {:.0},{:.0}.",
                    state.x_mm,
                    state.y_mm,
                    state.x_mm + across,
                    state.y_mm + down
                ))
                .weak(),
            );
        }
        None if !state.text.trim().is_empty() => {
            widgets::hint(
                room.ui,
                match state.qr {
                    true => "That is more than the largest QR code holds.",
                    false => {
                        "A barcode holds letters, digits and punctuation \
                              without accents. A QR code takes anything."
                    }
                },
            );
        }
        None => {}
    }

    room.ui.add_space(8.0);
    room.ui.collapsing("More", |ui| {
        ui.checkbox(&mut state.own_module, "Set the module size myself");
        ui.add_enabled_ui(state.own_module, |ui| {
            ui.horizontal(|ui| {
                ui.label("One module");
                ui.add(
                    egui::DragValue::new(&mut state.module_mm)
                        .speed(0.05)
                        .range(0.1..=5.0)
                        .suffix(" mm"),
                );
            });
        });
        if onionskin::barcode::too_small_to_print(state.module()) {
            widgets::hint(
                ui,
                "Under about a quarter of a millimetre a laser printer stops \
                 putting a module down the same width twice, which is exactly \
                 what a scanner cannot survive.",
            );
        }

        ui.add_enabled_ui(!state.qr, |ui| {
            ui.horizontal(|ui| {
                ui.label("Bars are");
                millimetres(ui, &mut state.height_mm);
                ui.label("tall");
            });
        });

        ui.add_enabled_ui(state.qr, |ui| {
            ui.horizontal(|ui| {
                ui.label("How much can be lost");
                egui::ComboBox::from_id_salt("barcode-level")
                    .selected_text(state.level.clone())
                    .show_ui(ui, |ui| {
                        for name in ["low", "medium", "quartile", "high"] {
                            ui.selectable_value(&mut state.level, name.to_string(), name);
                        }
                    });
            });
            if let Some(level) = onionskin::barcode::qr::Ecc::parse(&state.level) {
                widgets::hint(ui, level.describe());
            }
        });

        ui.checkbox(&mut state.caption, "Print what it says underneath");
        widgets::hint(
            ui,
            "So a person can type it in when the scanner will not read it.",
        );
    });

    // Above the button. Somebody deciding where to put a barcode needs this
    // while they are still deciding.
    room.ui.add_space(8.0);
    widgets::hint(room.ui, onionskin::barcode::Symbol::over_printing());

    room.ui.add_space(10.0);
    let ready = state.document.is_some() && state.symbol().is_some();
    if room
        .ui
        .add_enabled(
            ready && !room.jobs.busy(),
            egui::Button::new(egui::RichText::new("Write the delta").strong()),
        )
        .clicked()
    {
        make(state, room);
    }
    if !ready {
        widgets::hint(room.ui, "Give the sheet, and something to encode.");
    }

    if let Some(outcome) = &room.jobs.last {
        room.ui.add_space(12.0);
        if widgets::outcome(room.ui, outcome) {
            room.jobs.dismiss();
        }
    }
}

fn millimetres(ui: &mut egui::Ui, value: &mut f64) {
    ui.add(
        egui::DragValue::new(value)
            .speed(0.5)
            .range(0.0..=2000.0)
            .suffix(" mm"),
    );
}

fn make(state: &mut State, room: &mut Room) {
    let Some(document) = state.document.clone() else {
        return;
    };
    let Some(symbol) = state.symbol() else {
        return;
    };
    let module = state.module();
    let (qr, caption, page) = (state.qr, state.caption, state.page);
    let (x_mm, y_mm, height_mm) = (state.x_mm, state.y_mm, state.height_mm);
    let text = state.text.trim().to_string();
    let output = state
        .output
        .clone()
        .unwrap_or_else(|| beside(&document, "-barcode"));

    room.jobs.start("Writing the code", move |report| {
        report.saying("Reading the sheet…");
        let pages = onionskin::recipe::pages_in(&document).unwrap_or(1);
        if page == 0 || page > pages {
            return Outcome::refused(format!(
                "{} has {pages} page(s), so there is no page {page} to put a \
                 code on.",
                document.display()
            ));
        }
        let mut sizes = Vec::with_capacity(pages);
        for at in 1..=pages {
            match onionskin::recipe::draw_page(&document, at) {
                Ok(both) => sizes.push(both.1.page),
                Err(why) => return Outcome::refused(why),
            }
        }

        let paper = sizes[page - 1];
        let across_mm = symbol.width_mm(module);
        let down_mm = match qr {
            true => symbol.height_mm(module),
            false => height_mm + symbol.quiet as f64 * module * 2.0,
        };
        if x_mm + across_mm > paper.width_mm || y_mm + down_mm > paper.height_mm {
            return Outcome::refused(format!(
                "That code comes to {across_mm:.0} x {down_mm:.0} mm, and at \
                 {x_mm:.0},{y_mm:.0} it runs off a {} sheet. Move it, or make \
                 the modules smaller.",
                paper.describe()
            ));
        }

        report.saying("Writing the delta…");
        let mut shapes: Vec<Vec<onionskin::pdf::PlacedShape>> = vec![Vec::new(); pages];
        let mut lines: Vec<Vec<onionskin::pdf::PlacedLine>> = vec![Vec::new(); pages];
        let boxes = match qr {
            true => symbol.rectangles(module),
            false => symbol.bars(module, height_mm),
        };
        for (bx, by, width_mm, height_mm) in boxes {
            shapes[page - 1].push(onionskin::pdf::PlacedShape {
                drawing: onionskin::pdf::Drawing::Rect {
                    x_mm: x_mm + bx,
                    y_mm: y_mm + by,
                    width_mm,
                    height_mm,
                    radius_mm: 0.0,
                },
                stroke: None,
                fill: Some((0.0, 0.0, 0.0)),
                width_mm: 0.0,
                dash_mm: None,
            });
        }
        if caption {
            // Small, because it is there for when the scanner fails rather than
            // instead of it — and set one cap height below the symbol, so the tops
            // of the letters sit exactly where the quiet zone ends.
            const CAPTION_PT: f64 = 8.0;
            let width = onionskin::pdf::builtin_width_mm(
                onionskin::pdf::Font::Helvetica,
                &text,
                CAPTION_PT,
            );
            lines[page - 1].push(onionskin::pdf::PlacedLine {
                text: text.clone(),
                x_mm: x_mm + (across_mm - width) / 2.0,
                y_mm: y_mm + down_mm + CAPTION_PT * 0.7 * 25.4 / 72.0,
                size_pt: CAPTION_PT,
                font: onionskin::pdf::LineFont::Builtin(onionskin::pdf::Font::Helvetica),
                colour: (0.0, 0.0, 0.0),
                rotation_deg: 0.0,
            });
        }

        if let Err(why) = onionskin::pdf::write_page_content(
            &output,
            &sizes,
            &lines,
            &shapes,
            "Onionskin code",
            None,
        ) {
            return Outcome::refused(why.to_string());
        }

        Outcome::Done {
            message: format!(
                "{}, {across_mm:.0} x {down_mm:.0} mm at {x_mm:.0},{y_mm:.0}.",
                match qr {
                    true => "A QR code",
                    false => "A Code 128 barcode",
                }
            ),
            wrote: vec![output],
            notes: vec![onionskin::barcode::Symbol::over_printing().to_string()],
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The module sizes match the command line's, so the same job done either
    /// way comes out the same size on paper.
    #[test]
    fn the_defaults_are_the_command_lines_defaults() {
        let mut state = State::default();
        assert_eq!(state.module(), 0.4);
        state.qr = true;
        assert_eq!(state.module(), 0.8);
        // And neither is small enough to print badly.
        assert!(!onionskin::barcode::too_small_to_print(state.module()));
        state.qr = false;
        assert!(!onionskin::barcode::too_small_to_print(state.module()));
    }

    /// Nothing typed is not a code, and the button stays off.
    #[test]
    fn nothing_typed_is_not_a_code() {
        let mut state = State {
            document: Some(std::path::PathBuf::from("/tmp/a.pdf")),
            ..Default::default()
        };
        assert!(state.symbol().is_none());
        state.text = "   ".into();
        assert!(state.symbol().is_none());
        state.text = "INV-1".into();
        assert!(state.symbol().is_some());
    }

    /// A barcode cannot hold an accented letter and a QR code can. Switching
    /// between the two is the answer the screen gives, so it has to be true.
    #[test]
    fn what_a_barcode_refuses_a_qr_code_accepts() {
        let mut state = State {
            text: "café".into(),
            ..Default::default()
        };
        assert!(
            state.symbol().is_none(),
            "a barcode took an accented letter"
        );
        state.qr = true;
        assert!(
            state.symbol().is_some(),
            "a QR code refused an accented letter"
        );
    }

    /// The size shown is the size that comes out, quiet zone included — which
    /// is the number somebody uses to decide where it goes.
    #[test]
    fn the_size_shown_includes_the_paper_around_it() {
        let state = State {
            text: "INV-2024-00817".into(),
            ..Default::default()
        };
        let symbol = state.symbol().unwrap();
        let (across, down) = state.size_mm().unwrap();
        // Wider than the bars alone, by the quiet zone on both sides.
        assert!(
            across > symbol.width as f64 * state.module(),
            "the quiet zone was left out of {across}"
        );
        assert!(
            down > state.height_mm,
            "the quiet zone was left out of {down}"
        );
    }

    /// A QR code is square and a barcode is not, which is the difference that
    /// decides whether the height control means anything.
    #[test]
    fn a_qr_code_comes_out_square() {
        let state = State {
            text: "https://example.org".into(),
            qr: true,
            ..Default::default()
        };
        let (across, down) = state.size_mm().unwrap();
        assert!((across - down).abs() < 1e-9, "{across} by {down}");
    }

    /// Every level the list offers is one the encoder knows.
    #[test]
    fn the_levels_on_offer_are_real() {
        for name in ["low", "medium", "quartile", "high"] {
            assert!(onionskin::barcode::qr::Ecc::parse(name).is_some(), "{name}");
        }
        assert!(onionskin::barcode::qr::Ecc::parse(&State::default().level).is_some());
    }

    #[test]
    fn the_delta_lands_beside_the_sheet() {
        assert_eq!(
            beside(std::path::Path::new("/tmp/assets.pdf"), "-barcode"),
            std::path::Path::new("/tmp/assets-barcode.pdf")
        );
    }
}
