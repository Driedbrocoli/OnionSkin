//! Whether the delta belongs on the sheet in your hand, before any paper moves.
//!
//! The verify screen answers this afterwards, which is the right check one
//! sheet too late. The mistake it cannot help with is the wrong form in the
//! tray: the delta prints perfectly, onto a document it was never made for, and
//! the additions land across whatever that sheet happens to say. On a stack of
//! headed paper, a run of numbered certificates, somebody's signed contract —
//! the ink does not come off, and there is no undo at a printer.
//!
//! So the delta is held against the sheet and what is under each addition is
//! looked at. On the right sheet the additions land on clear paper, because
//! that is what an overlay is for.
//!
//! # Two ways to land on ink
//!
//! They look the same and want opposite things done. The wrong sheet has
//! somebody else's text under the additions, and wants swapping. The *right*
//! sheet, fed a second time, has the additions' own ink under them, and wants
//! somebody to stop and think — printing it again lays every letter down twice
//! in the same place.
//!
//! The second is said, not refused. A faint first pass is a real reason to want
//! a second, and nothing here cannot be undone by looking at the paper.

use eframe::egui;

use super::Room;
use crate::job::Outcome;
use crate::widgets;
use onionskin::calibrate;

pub struct State {
    /// The delta about to be printed.
    delta: Option<std::path::PathBuf>,
    /// The sheet it would go onto: a scan, a photograph, or the PDF the form
    /// arrived as.
    sheet: Option<std::path::PathBuf>,
    page: String,
    /// Which page of a multi-page document the sheet is.
    sheet_number: usize,
}

impl Default for State {
    fn default() -> Self {
        State {
            delta: None,
            sheet: None,
            page: "a4".into(),
            sheet_number: 1,
        }
    }
}

pub fn show(state: &mut State, room: &mut Room) {
    widgets::title(
        room.ui,
        "Check the sheet before you feed it",
        "Hold the delta against the sheet and see what is underneath.",
    );

    widgets::hint(
        room.ui,
        "The wrong form in the tray prints perfectly onto the wrong document, \
         and the ink does not come off. This looks at what sits under each \
         addition: on the right sheet they land on clear paper.\n\nA PDF of the \
         form is better than a scan of it, having no skew to measure.",
    );
    room.ui.add_space(10.0);

    widgets::file_row(
        room.ui,
        room.picker,
        "The delta about to be printed",
        &mut state.delta,
        &["pdf"],
        room.dropped,
    );
    super::last_delta_button(room.ui, &mut state.delta);
    widgets::file_row(
        room.ui,
        room.picker,
        "The sheet it would go onto",
        &mut state.sheet,
        &["pdf", "png", "jpg", "jpeg", "tif", "tiff", "bmp"],
        room.dropped,
    );

    room.ui.collapsing("If the sheet is a scan", |ui| {
        widgets::hint(
            ui,
            "A scan has to be measured against a known paper size; a PDF says \
             its own and ignores this.",
        );
        ui.horizontal(|ui| {
            ui.label("Paper");
            ui.add(egui::TextEdit::singleline(&mut state.page).desired_width(80.0));
            ui.label("Page of the document");
            ui.add(egui::DragValue::new(&mut state.sheet_number).range(1..=999));
        });
    });

    room.ui.add_space(10.0);
    let ready = state.delta.is_some() && state.sheet.is_some();
    if room
        .ui
        .add_enabled(
            ready && !room.jobs.busy(),
            egui::Button::new(egui::RichText::new("Hold them together").strong()),
        )
        .clicked()
    {
        check(state, room);
    }
    if !ready {
        widgets::hint(room.ui, "Give the delta and the sheet it would go onto.");
    }

    if let Some(outcome) = &room.jobs.last {
        room.ui.add_space(12.0);
        if widgets::outcome(room.ui, outcome) {
            room.jobs.dismiss();
        }
    }
}

/// How finely the sheet is looked at.
///
/// The same resolution the command line uses, so the two cannot disagree about
/// whether an addition lands on ink.
const LOOKING_AT_DPI: f64 = 150.0;

fn check(state: &mut State, room: &mut Room) {
    let (Some(delta), Some(sheet)) = (state.delta.clone(), state.sheet.clone()) else {
        return;
    };
    let page_text = state.page.clone();
    let sheet_number = state.sheet_number;

    room.jobs.start("Holding them together", move |report| {
        report.saying("Reading what the delta asks for…");
        let asked = match calibrate::marks_on_delta(&delta) {
            Ok(marks) => marks,
            Err(e) => return Outcome::refused(e.to_string()),
        };
        if asked.is_empty() {
            return Outcome::refused(
                "There is nothing on that delta, so there is nothing to hold \
                 against the sheet."
                    .to_string(),
            );
        }
        let delta_page = match onionskin::recipe::draw_page(&delta, 1) {
            Ok((_, registration)) => registration.page,
            Err(e) => return Outcome::refused(e),
        };

        report.saying("Looking at the sheet…");
        let drawn = if is_a_document(&sheet) {
            onionskin::recipe::draw_page(&sheet, sheet_number).map_err(Outcome::refused)
        } else {
            read_a_picture(&sheet, &page_text)
        };
        let (gray, registration) = match drawn {
            Ok(both) => both,
            Err(refused) => return refused,
        };

        report.saying("Looking at what is under each addition…");
        let fit = onionskin::fits::against(
            &asked,
            gray.as_raw(),
            gray.width() as usize,
            registration.px_per_mm * 25.4,
            delta_page,
            registration.page,
            onionskin::diff::DiffOptions::default().ink_threshold,
        );

        let message = fit.describe();
        if fit.belongs() {
            return Outcome::Done {
                message,
                wrote: Vec::new(),
                notes: vec![
                    "This looks like the sheet the delta was made for. Print at \
                     100%, with \"Fit to page\" off."
                        .to_string(),
                ],
            };
        }
        // The sheet that has been through already is the right sheet. Saying
        // "refused" about it would be telling somebody to swap a sheet that
        // needs no swapping.
        if fit.already_stamped() && fit.paper_matches() {
            return Outcome::Done {
                message,
                wrote: Vec::new(),
                notes: vec!["This sheet has been through already. Printing it again is \
                     allowed — a faint first pass is a real reason to want it — \
                     but it is rarely what somebody meant. Look at the paper \
                     before you feed it."
                    .to_string()],
            };
        }
        Outcome::refused(format!(
            "{message}\n\nDo not print this onto that sheet without looking at \
             it first. The proof screen will show you both together."
        ))
    });
}

/// Whether the sheet arrived as a document rather than a picture of one.
fn is_a_document(path: &std::path::Path) -> bool {
    matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("pdf" | "docx" | "odt" | "doc" | "rtf" | "txt" | "md")
    )
}

/// A scan, straightened onto the paper's own grid.
///
/// Measured off the raw scan instead, every addition would be out by the width
/// of whatever border the scanner left round the paper — which is exactly the
/// error this screen exists to find, arriving from the wrong direction.
fn read_a_picture(
    sheet: &std::path::Path,
    page_text: &str,
) -> Result<(image::GrayImage, onionskin::scan::ScanRegistration), Outcome> {
    let page =
        onionskin::geometry::parse_page(page_text).map_err(|e| Outcome::refused(e.to_string()))?;
    let image = image::open(sheet)
        .map_err(|e| Outcome::refused(format!("could not read that sheet: {e}")))?;
    let registration = onionskin::scan::register(&image, onionskin::scan::ScanOptions::new(page))
        .map_err(|e| Outcome::refused(e.to_string()))?;
    let flat = registration.flatten(&image.to_luma8(), LOOKING_AT_DPI);
    let squared = onionskin::scan::ScanRegistration {
        px_per_mm: LOOKING_AT_DPI / 25.4,
        skew_deg: 0.0,
        origin_px: (0.0, 0.0),
        ..registration
    };
    Ok((flat, squared))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// A PDF of the form is read as a document; a photograph of it has to be
    /// registered first. Getting this the wrong way round measures a scan's
    /// border as though it were the paper.
    #[test]
    fn a_document_is_told_from_a_picture_of_one() {
        for document in ["form.pdf", "FORM.PDF", "letter.docx", "notes.odt"] {
            assert!(is_a_document(Path::new(document)), "{document}");
        }
        for picture in ["scan.png", "scan.JPG", "photo.jpeg", "fax.tif"] {
            assert!(!is_a_document(Path::new(picture)), "{picture}");
        }
    }

    /// This screen and the command line must agree about how closely the sheet
    /// is looked at, or the two will disagree about whether an addition lands
    /// on ink — on the one question where disagreeing costs a sheet of paper.
    #[test]
    fn the_screen_looks_at_the_sheet_as_closely_as_the_command_line_does() {
        assert_eq!(LOOKING_AT_DPI, 150.0);
    }

    /// The screen has to tell the two collisions apart, because they want
    /// opposite things done: the wrong sheet wants swapping, and the one that
    /// has been through already wants somebody to stop and think.
    #[test]
    fn a_sheet_that_has_been_through_is_not_reported_as_the_wrong_sheet() {
        use onionskin::fits::{Fit, Landing};
        let a4 = onionskin::geometry::PageSize::new(210.0, 297.0);
        let stamped = |x: f64| Landing {
            box_mm: (x, 40.0, x + 18.0, 44.0),
            // The addition's own ink, plus the form's rule underneath.
            under_mm2: 14.0,
            clearance_mm: 0.0,
            asked_mm2: 10.0,
        };
        let fit = Fit {
            landings: vec![stamped(60.0), stamped(100.0)],
            delta_page: a4,
            sheet_page: a4,
        };
        assert!(fit.already_stamped());
        assert!(!fit.belongs(), "a stamped sheet is not clear to print onto");
        assert!(fit.describe().contains("already has this delta on it"));

        // And the wrong sheet still reads as the wrong sheet.
        let wrong = Fit {
            landings: vec![
                Landing {
                    box_mm: (60.0, 40.0, 78.0, 44.0),
                    under_mm2: 40.0,
                    clearance_mm: 0.0,
                    asked_mm2: 10.0,
                },
                Landing {
                    box_mm: (100.0, 40.0, 118.0, 44.0),
                    under_mm2: 0.0,
                    clearance_mm: 20.0,
                    asked_mm2: 10.0,
                },
            ],
            delta_page: a4,
            sheet_page: a4,
        };
        assert!(!wrong.already_stamped());
        assert!(wrong.describe().contains("wrong sheet is in the tray"));
    }
}
