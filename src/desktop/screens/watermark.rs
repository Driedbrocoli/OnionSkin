//! A word across the whole sheet: DRAFT, COPY, VOID.
//!
//! A word processor puts this *behind* the page's own printing. Onionskin
//! cannot — the sheet is already printed, and a printer adds toner and never
//! takes it away, so the word goes on top of whatever is there.
//!
//! That single fact is what the screen is built around. The grey is a slider
//! rather than a number in a manual, it starts light, and it says out loud what
//! a darker one costs. Somebody who wants a black VOID across a superseded form
//! can have one; somebody who wants a DRAFT they can still read through gets
//! that without having to know why.

use eframe::egui;

use super::{beside, Room};
use crate::job::Outcome;
use crate::widgets;

/// The words people actually stamp on paper, so the common case is a click.
const USUAL: [&str; 6] = [
    "DRAFT",
    "COPY",
    "VOID",
    "SAMPLE",
    "CONFIDENTIAL",
    "SUPERSEDED",
];

pub struct State {
    document: Option<std::path::PathBuf>,
    output: Option<std::path::PathBuf>,
    text: String,
    grey: f64,
    /// Left alone, the size is worked out from the word so it fills the sheet.
    own_size: bool,
    size_pt: f64,
    every_page: bool,
    page: usize,
    font: String,
}

impl Default for State {
    fn default() -> Self {
        State {
            document: None,
            output: None,
            text: "DRAFT".to_string(),
            grey: onionskin::watermark::GREY,
            own_size: false,
            size_pt: 96.0,
            every_page: true,
            page: 1,
            font: "helvetica".to_string(),
        }
    }
}

pub fn show(state: &mut State, room: &mut Room) {
    widgets::title(
        room.ui,
        "Stamp a word across it",
        "DRAFT, COPY, VOID — corner to corner, on the sheet you already printed.",
    );

    widgets::hint(
        room.ui,
        "The size works itself out from the word, so a short one and a long one \
         both fill the page.",
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
        "watermark.pdf",
        &["pdf"],
        "beside the sheet, as NAME-watermark.pdf",
    );

    room.ui.add_space(8.0);
    room.ui.horizontal(|ui| {
        ui.label(egui::RichText::new("The word").strong());
        ui.add(
            egui::TextEdit::singleline(&mut state.text)
                .hint_text("DRAFT")
                .desired_width(220.0),
        );
    });
    room.ui.horizontal_wrapped(|ui| {
        for word in USUAL {
            if ui.small_button(word).clicked() {
                state.text = word.to_string();
            }
        }
    });

    room.ui.add_space(8.0);
    room.ui.horizontal(|ui| {
        ui.label("How light");
        ui.add(
            egui::Slider::new(&mut state.grey, 0.0..=1.0)
                .show_value(false)
                .custom_formatter(|v, _| format!("{:.0}%", v * 100.0)),
        );
        ui.label(format!("{:.0}% grey", state.grey * 100.0));
    });

    // The whole design of the feature, said where somebody is still deciding.
    widgets::hint(
        room.ui,
        "Toner goes on top. This does not sit behind the page's own printing the \
         way a word processor's watermark does — it goes over it, and where it \
         crosses printing the printing wins.",
    );
    if onionskin::watermark::too_dark_to_read_through(state.grey) {
        widgets::hint(
            room.ui,
            "This dark, the words underneath will be hard to read. That is the \
             right answer for a form that has been superseded and the wrong one \
             for a draft somebody still has to read.",
        );
    }

    room.ui.add_space(8.0);
    room.ui.collapsing("More", |ui| {
        ui.checkbox(&mut state.every_page, "Every page");
        ui.add_enabled_ui(!state.every_page, |ui| {
            ui.horizontal(|ui| {
                ui.label("Page");
                ui.add(egui::DragValue::new(&mut state.page).range(1..=9999));
            });
        });

        ui.checkbox(&mut state.own_size, "Set the size myself");
        ui.add_enabled_ui(state.own_size, |ui| {
            ui.horizontal(|ui| {
                ui.label("Size");
                ui.add(
                    egui::DragValue::new(&mut state.size_pt)
                        .speed(1.0)
                        .range(6.0..=400.0)
                        .suffix(" pt"),
                );
            });
        });
        widgets::hint(
            ui,
            "Left alone, the size is whatever makes the word span the paper.",
        );

        ui.horizontal(|ui| {
            ui.label("Font");
            egui::ComboBox::from_id_salt("watermark-font")
                .selected_text(state.font.clone())
                .show_ui(ui, |ui| {
                    for name in ["helvetica", "helvetica-bold", "times", "courier"] {
                        ui.selectable_value(&mut state.font, name.to_string(), name);
                    }
                });
        });
    });

    room.ui.add_space(10.0);
    let ready = state.document.is_some() && !state.text.trim().is_empty();
    if room
        .ui
        .add_enabled(
            ready && !room.jobs.busy(),
            egui::Button::new(egui::RichText::new("Write the delta").strong()),
        )
        .clicked()
    {
        stamp(state, room);
    }
    if !ready {
        widgets::hint(room.ui, "Give the sheet, and a word to stamp on it.");
    }

    if let Some(outcome) = &room.jobs.last {
        room.ui.add_space(12.0);
        if widgets::outcome(room.ui, outcome) {
            room.jobs.dismiss();
        }
    }
}

fn stamp(state: &mut State, room: &mut Room) {
    let Some(document) = state.document.clone() else {
        return;
    };
    let text = state.text.trim().to_string();
    let grey = state.grey;
    let size = state.own_size.then_some(state.size_pt);
    let every_page = state.every_page;
    let page = state.page;
    let font = state.font.clone();
    let output = state
        .output
        .clone()
        .unwrap_or_else(|| beside(&document, "-watermark"));

    room.jobs.start("Stamping the sheet", move |report| {
        let Some(font) = onionskin::pdf::Font::parse(&font) else {
            return Outcome::refused(format!("'{font}' is not a built-in font."));
        };

        report.saying("Reading the sheet…");
        let pages = onionskin::recipe::pages_in(&document).unwrap_or(1);
        let wanted: Vec<usize> = match every_page {
            true => (1..=pages).collect(),
            false => vec![page],
        };
        if let Some(past) = wanted.iter().find(|page| **page > pages || **page == 0) {
            return Outcome::refused(format!(
                "{} has {pages} page(s), so there is no page {past} to mark.",
                document.display()
            ));
        }

        // Every page of the delta, blank where the sheet is not being marked —
        // a delta with fewer pages than the sheet would print page one's
        // watermark onto page two.
        let mut sizes = Vec::with_capacity(pages);
        let mut lines: Vec<Vec<onionskin::pdf::PlacedLine>> = vec![Vec::new(); pages];
        let mut said = Vec::new();
        for page in 1..=pages {
            let paper = match onionskin::recipe::draw_page(&document, page) {
                Ok(both) => both.1.page,
                Err(why) => return Outcome::refused(why),
            };
            sizes.push(paper);
            if !wanted.contains(&page) {
                continue;
            }
            let Some(mark) = onionskin::watermark::across(&text, paper, font, size, Some(grey))
            else {
                return Outcome::refused("There is nothing to write across the sheet.".to_string());
            };
            said.push(format!("Page {page}: {}", mark.describe()));
            lines[page - 1].push(onionskin::pdf::PlacedLine {
                text: mark.text.clone(),
                x_mm: mark.x_mm,
                y_mm: mark.y_mm,
                size_pt: mark.size_pt,
                font: onionskin::pdf::LineFont::Builtin(font),
                colour: mark.colour(),
                rotation_deg: mark.rotation_deg,
            });
        }

        report.saying("Writing the delta…");
        if let Err(why) =
            onionskin::pdf::write_delta(&output, &sizes, &lines, "Onionskin watermark", None)
        {
            return Outcome::refused(why.to_string());
        }

        let mut notes = vec![
            "Feed the printed sheets back in and print this. The word goes over \
             what is already there, so where it crosses printing the printing \
             wins and the mark reads as black."
                .to_string(),
        ];
        if onionskin::watermark::too_dark_to_read_through(grey) {
            notes.push(format!(
                "At {:.0}% grey the words underneath will be hard to read.",
                grey * 100.0
            ));
        }
        notes.extend(said.iter().cloned());

        Outcome::Done {
            message: format!("'{text}' across {} page(s).", wanted.len()),
            wrote: vec![output],
            notes,
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The screen opens on the light grey, because that is the one that leaves
    /// the page readable and the one somebody who does not know the difference
    /// should get.
    #[test]
    fn it_opens_light_enough_to_read_through() {
        let state = State::default();
        assert!(!onionskin::watermark::too_dark_to_read_through(state.grey));
        assert_eq!(state.grey, onionskin::watermark::GREY);
    }

    /// And on every page, because a two-page document stamped on page one only
    /// is almost never what somebody meant by "mark this DRAFT".
    #[test]
    fn it_opens_on_every_page() {
        assert!(State::default().every_page);
    }

    /// The size is worked out unless somebody says otherwise. A screen that
    /// opened on a fixed 96 pt would put "NOT FOR CIRCULATION" off both edges.
    #[test]
    fn the_size_works_itself_out_unless_told_not_to() {
        assert!(!State::default().own_size);
    }

    /// Every offered word really is a watermark on A4: placed, and big.
    ///
    /// "It was placed" is not the check worth making — the sizing clamps at
    /// 6 pt, so a word too long to fit comes back placed and unreadable rather
    /// than refused. So the box the word occupies is measured, and it has to
    /// span most of the paper. CONFIDENTIAL is the one that would go first.
    #[test]
    fn every_word_on_offer_fills_a_sheet() {
        let a4 = onionskin::geometry::PageSize::new(210.0, 297.0);
        for word in USUAL {
            let mark =
                onionskin::watermark::across(word, a4, onionskin::pdf::Font::Helvetica, None, None)
                    .unwrap_or_else(|| panic!("'{word}' could not be placed"));

            // The turned box: a word is a band as tall as its letters, and a
            // band laid across a diagonal is wider than the word itself.
            let turned = mark.rotation_deg.to_radians();
            let along = onionskin::pdf::builtin_width_mm(
                onionskin::pdf::Font::Helvetica,
                word,
                mark.size_pt,
            );
            let cap = mark.size_pt * 0.7 * 25.4 / 72.0;
            let across_mm = along * turned.cos().abs() + cap * turned.sin().abs();
            let down_mm = along * turned.sin().abs() + cap * turned.cos().abs();

            assert!(
                across_mm > a4.width_mm * 0.6,
                "'{word}' spans only {across_mm:.0} mm of a {} mm sheet",
                a4.width_mm
            );
            assert!(
                across_mm < a4.width_mm && down_mm < a4.height_mm,
                "'{word}' comes to {across_mm:.0}x{down_mm:.0} mm, off the paper"
            );
        }
    }

    /// The font names offered are ones `Font::parse` knows, which is the same
    /// check the command line does.
    #[test]
    fn the_fonts_on_offer_are_real() {
        for name in ["helvetica", "helvetica-bold", "times", "courier"] {
            assert!(
                onionskin::pdf::Font::parse(name).is_some(),
                "the list offers '{name}', which is not a font"
            );
        }
        assert!(onionskin::pdf::Font::parse(&State::default().font).is_some());
    }

    #[test]
    fn the_delta_lands_beside_the_sheet() {
        assert_eq!(
            beside(std::path::Path::new("/tmp/report.pdf"), "-watermark"),
            std::path::Path::new("/tmp/report-watermark.pdf")
        );
    }
}
