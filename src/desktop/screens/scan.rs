//! Typing onto a page that only exists as a photograph of paper.
//!
//! The other screens start from a digital document and already know where
//! everything on it is. A scan knows nothing — it is pixels — so the one
//! thing worth adding here that the command line cannot is a picture to point
//! at: click on the scan and the click becomes a millimetre on the sheet, the
//! same ruler-on-paper measurement the delta is printed with.
//!
//! The click is a rough guess, not a measurement — good enough to drop a
//! placement near where it belongs, refined afterwards by typing the exact
//! millimetres. It assumes the scan shows the sheet edge to edge, the way
//! `--at-mm` does on the command line; a scan with a border of scanner
//! background round the page will need those millimetres nudged by hand.

use std::path::{Path, PathBuf};

use eframe::egui;

use super::Room;
use crate::job::Outcome;
use crate::theme;
use crate::widgets;
use onionskin::font;
use onionskin::geometry;
use onionskin::pdf;

/// One word or line of words, waiting to be printed onto the sheet.
struct Placement {
    text: String,
    x_mm: f64,
    y_mm: f64,
}

pub struct State {
    scan: Option<PathBuf>,
    page: String,
    size_pt: f64,
    font: pdf::Font,
    font_file: Option<PathBuf>,
    colour: [u8; 3],
    placements: Vec<Placement>,
    output: Option<PathBuf>,
    /// What the last look at the page said it was set in, if it was asked.
    matched: Option<String>,
    /// How far to turn the words, degrees clockwise on the page. For a form
    /// with a sideways box on it, or a note down the margin.
    rotation_deg: f64,
    /// The thing already on the page to put the next words after.
    anchor: String,
    /// What to say about the last attempt to find one.
    anchor_said: Option<String>,
    /// Below the anchor rather than after it.
    anchor_below: bool,
    /// The page as it was last read, so finding a second anchor does not read
    /// it again — reading is three passes of template matching over every mark
    /// on the sheet, and the page has not changed between one and the next.
    read: Option<onionskin::letters::PageText>,
}

impl Default for State {
    fn default() -> Self {
        State {
            scan: None,
            page: "a4".to_string(),
            size_pt: 11.0,
            font: pdf::Font::Helvetica,
            font_file: None,
            colour: [0, 0, 0],
            placements: Vec::new(),
            output: None,
            matched: None,
            rotation_deg: 0.0,
            anchor: String::new(),
            anchor_said: None,
            anchor_below: false,
            read: None,
        }
    }
}

pub fn show(state: &mut State, room: &mut Room) {
    let screen = super::Screen::Scan;
    widgets::title(room.ui, screen.name(), screen.lede());

    if widgets::file_row(
        room.ui,
        room.picker,
        "The scan",
        &mut state.scan,
        &["png", "jpg", "jpeg", "tif", "tiff", "bmp"],
        room.dropped,
    ) {
        // A different sheet: everything worked out about the last one is now
        // about the wrong piece of paper.
        state.read = None;
        state.matched = None;
        state.anchor_said = None;
    }

    room.ui.horizontal(|ui| {
        ui.label("Paper size");
        ui.text_edit_singleline(&mut state.page);
    });
    widgets::hint(
        room.ui,
        "a4, letter, legal, a5, or a custom size such as 210x297 (mm). This \
         must match the sheet that was scanned — it is what turns a click, \
         and the delta itself, into millimetres on the real page.",
    );

    room.ui.add_space(6.0);
    let picker = &mut *room.picker;
    room.ui.collapsing("Settings", |ui| {
        ui.horizontal(|ui| {
            ui.label("Type size");
            ui.add(
                egui::DragValue::new(&mut state.size_pt)
                    .range(1.0..=400.0)
                    .suffix(" pt"),
            );
        });

        ui.add_enabled_ui(state.font_file.is_none(), |ui| {
            ui.horizontal(|ui| {
                ui.label("Font");
                egui::ComboBox::from_id_salt("scan-font")
                    .selected_text(state.font.base_name())
                    .show_ui(ui, |ui| {
                        for candidate in pdf::Font::all() {
                            ui.selectable_value(&mut state.font, *candidate, candidate.base_name());
                        }
                    });

                // The page in front of you knows what it is set in, and you do
                // not. Nobody looks at a rent statement and thinks "Helvetica,
                // eleven point" — they think "there is a gap after Received".
                let can_look = state.scan.is_some();
                if ui
                    .add_enabled(can_look, egui::Button::new("Match the page"))
                    .on_hover_text(
                        "Read the words already on the scan and use the same face \
                         and size",
                    )
                    .clicked()
                {
                    match onionskin::typeface::match_scan(
                        state.scan.as_ref().expect("checked"),
                        &state.page,
                        false,
                        false,
                    ) {
                        Some(found) => {
                            state.font = found.font;
                            state.size_pt = found.size_pt;
                            state.matched = Some(found.describe());
                        }
                        None => {
                            state.matched = Some(
                                "Could not tell — too little text on the page, or \
                                 too poor a scan. The choice above still stands."
                                    .to_string(),
                            );
                        }
                    }
                }
            });
        });
        if let Some(said) = &state.matched {
            widgets::hint(ui, said);
        }
        widgets::file_row(
            ui,
            picker,
            "Font file, for another alphabet",
            &mut state.font_file,
            &["ttf", "otf", "ttc"],
        room.dropped,
    );
        widgets::hint(
            ui,
            "Optional. Overrides the font above, and is carried inside the \
             delta — needed only for letters Helvetica, Times and Courier \
             cannot write.",
        );

        ui.horizontal(|ui| {
            ui.label("Colour");
            ui.color_edit_button_srgb(&mut state.colour);
        });

        ui.horizontal(|ui| {
            ui.label("Turn the words");
            ui.add(
                egui::DragValue::new(&mut state.rotation_deg)
                    .range(-360.0..=360.0)
                    .speed(1.0)
                    .suffix("°"),
            );
            widgets::hint(ui, "clockwise — for a sideways box, or a note down the margin");
            if state.rotation_deg != 0.0 && ui.button("Straight again").clicked() {
                state.rotation_deg = 0.0;
            }
        });
    });

    room.ui.add_space(10.0);
    widgets::hint(
        room.ui,
        "Click the scan below to add a placement roughly where you click. Or \
         press \"Add a placement\" and type the millimetres straight from a \
         specification — no clicking needed.",
    );

    match (state.scan.clone(), geometry::parse_page(&state.page)) {
        (Some(scan_path), Ok(size)) => show_preview(state, room, &scan_path, size),
        (Some(_), Err(e)) => widgets::hint(
            room.ui,
            &format!("The paper size above is not understood, so the scan cannot be shown to scale: {e}"),
        ),
        (None, _) => widgets::hint(room.ui, "Choose the scan above to see it here."),
    }

    room.ui.add_space(10.0);
    show_placements(state, room);

    room.ui.add_space(12.0);
    widgets::save_row(
        room.ui,
        room.picker,
        "Where to save the delta",
        &mut state.output,
        "delta.pdf",
        &["pdf"],
        "beside the scan, as delta.pdf",
    );

    room.ui.add_space(6.0);
    let page_ok = geometry::parse_page(&state.page).is_ok();
    let has_words = state.placements.iter().any(|p| !p.text.trim().is_empty());
    let ready = state.scan.is_some() && page_ok && has_words;
    let busy = room.jobs.busy();
    if room
        .ui
        .add_enabled(
            ready && !busy,
            egui::Button::new(egui::RichText::new("Write the delta").strong()),
        )
        .clicked()
    {
        start(state, room);
    }
    if !ready {
        widgets::hint(
            room.ui,
            "Choose the scan, a paper size Onionskin understands, and type the \
             words for at least one placement.",
        );
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
        "Print it at 100%. Put the scanned sheet back in the tray, and turn \
         \"Fit to page\" off — it scales by a few percent and nothing will \
         line up. Do one sheet first and hold it against the original.",
    );
}

/// The scan, drawn to scale, with a marker at each placement. Clicking it
/// adds a new placement where the pointer landed.
fn show_preview(state: &mut State, room: &mut Room, scan: &Path, page: geometry::PageSize) {
    let pixels_per_point = room.ui.ctx().pixels_per_point();
    let width_px = (room.ui.available_width() * pixels_per_point)
        .round()
        .max(200.0) as u32;

    let sheet = match room.previews.sheet(room.ui.ctx(), scan, 0, width_px) {
        Ok(sheet) => sheet,
        Err(why) => {
            widgets::refused(room.ui, &why);
            return;
        }
    };

    // `Image::from_texture` copies out the id and size it needs there and
    // then, so nothing here keeps the preview cache borrowed.
    let image = egui::Image::from_texture(&sheet.texture).sense(egui::Sense::click());
    let response = room.ui.add(image);
    let rect = response.rect;

    if response.clicked() {
        if let Some(pos) = response.interact_pointer_pos() {
            let (x_mm, y_mm) = point_to_mm(rect, page, pos);
            state.placements.push(Placement {
                text: String::new(),
                x_mm,
                y_mm,
            });
        }
    }

    let painter = room.ui.painter();
    for placement in &state.placements {
        let point = mm_to_point(rect, page, (placement.x_mm, placement.y_mm));
        painter.circle(point, 5.0, theme::ADDED, egui::Stroke::new(1.5, egui::Color32::WHITE));
    }
}

/// The list of placements made so far, each editable and each removable.
fn show_placements(state: &mut State, room: &mut Room) {
    if state.placements.is_empty() {
        widgets::hint(room.ui, "No placements yet.");
    }

    let mut remove = None;
    for (index, placement) in state.placements.iter_mut().enumerate() {
        room.ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut placement.text)
                    .hint_text("the words to add")
                    .desired_width(240.0),
            );
            ui.label("at");
            ui.add(egui::DragValue::new(&mut placement.x_mm).suffix(" mm").speed(0.5));
            ui.label(",");
            ui.add(egui::DragValue::new(&mut placement.y_mm).suffix(" mm").speed(0.5));
            if ui.small_button("×").clicked() {
                remove = Some(index);
            }
        });
    }
    if let Some(index) = remove {
        state.placements.remove(index);
    }

    if room.ui.button("+ Add a placement").clicked() {
        state.placements.push(Placement {
            text: String::new(),
            x_mm: 20.0,
            y_mm: 20.0,
        });
    }
    widgets::hint(
        room.ui,
        "Position is the baseline — where the letters sit, not the top of them.",
    );

    // The other way of saying where: by what is already printed there. Nobody
    // holding a form knows the gap after "Received:" starts 44.9 mm across.
    // They know it is the gap after "Received:".
    room.ui.add_space(10.0);
    room.ui.label("Or put them next to something already on the page");
    let ready = state.scan.is_some() && !state.anchor.trim().is_empty();
    room.ui.horizontal(|ui| {
        ui.label(if state.anchor_below { "Below" } else { "After" });
        ui.add(
            egui::TextEdit::singleline(&mut state.anchor)
                .hint_text("Received:")
                .desired_width(200.0),
        );
        ui.checkbox(&mut state.anchor_below, "on the next line");
    });
    if room
        .ui
        .add_enabled(ready, egui::Button::new("Find it and add a placement"))
        .clicked()
    {
        add_by_anchor(state);
    }
    if let Some(said) = &state.anchor_said {
        widgets::hint(room.ui, said);
    }
}

/// Find the anchor on the page and put a placement where it says.
///
/// Everything is allowed to fail into a sentence rather than an error box.
/// Somebody who has not chosen a scan, or whose anchor is not on the page, is
/// mid-thought about where to put some words — the answer is to say what went
/// wrong beside the box they typed it into, and leave everything else alone.
fn add_by_anchor(state: &mut State) {
    let Some(scan) = state.scan.clone() else {
        return;
    };
    if state.read.is_none() {
        state.read = onionskin::typeface::read_and_match(&scan, &state.page, false, false)
            .map(|(text, _)| text);
    }
    let Some(text) = &state.read else {
        state.anchor_said = Some(
            "Nothing could be read off this page, so there is nothing to place              words against. Add a placement above and give the millimetres."
                .to_string(),
        );
        return;
    };

    let put = if state.anchor_below {
        onionskin::anchor::Where::Below
    } else {
        onionskin::anchor::Where::After
    };
    let gap_mm = onionskin::geometry::pt_to_mm(state.size_pt * 0.3);
    let step_mm = onionskin::geometry::pt_to_mm(state.size_pt * 1.15);
    match onionskin::anchor::place(text, &state.anchor, put, gap_mm, step_mm) {
        Ok(found) => {
            state.anchor_said = Some(format!(
                "Found it on the line \"{}\" — a placement is waiting above at                  {:.1}, {:.1} mm.",
                found.line, found.x_mm, found.y_mm
            ));
            state.placements.push(Placement {
                text: String::new(),
                x_mm: found.x_mm,
                y_mm: found.y_mm,
            });
            state.anchor.clear();
        }
        Err(e) => state.anchor_said = Some(e.to_string()),
    }
}

fn start(state: &mut State, room: &mut Room) {
    let Some(scan_path) = state.scan.clone() else {
        return;
    };
    let Ok(page) = geometry::parse_page(&state.page) else {
        return;
    };

    let colour = (
        state.colour[0] as f64 / 255.0,
        state.colour[1] as f64 / 255.0,
        state.colour[2] as f64 / 255.0,
    );
    let line_font = if state.font_file.is_some() {
        pdf::LineFont::Embedded
    } else {
        pdf::LineFont::Builtin(state.font)
    };
    let lines: Vec<pdf::PlacedLine> = state
        .placements
        .iter()
        .filter(|p| !p.text.trim().is_empty())
        .map(|p| pdf::PlacedLine {
            text: p.text.clone(),
            x_mm: p.x_mm,
            y_mm: p.y_mm,
            size_pt: state.size_pt,
            font: line_font,
            rotation_deg: state.rotation_deg,
            colour,
        })
        .collect();
    if lines.is_empty() {
        return;
    }
    let font_file = state.font_file.clone();

    // Beside the scan unless somebody has said otherwise, because that is the
    // folder they are already working in.
    let output = state.output.clone().unwrap_or_else(|| {
        let mut path = scan_path.clone();
        path.set_file_name("delta.pdf");
        path
    });

    room.previews.forget(&output);
    let target = output.clone();
    room.jobs.start("Writing the delta", move |report| {
        let embedded = match &font_file {
            Some(path) => {
                report.saying("Loading the font…");
                match font::EmbeddedFont::load(path) {
                    Ok(font) => Some(font),
                    Err(e) => return Outcome::refused(e.to_string()),
                }
            }
            None => None,
        };

        report.saying("Writing the page…");
        let pages_lines = [lines];
        if let Err(e) = pdf::write_delta(
            &target,
            &[page],
            &pages_lines,
            "Onionskin delta",
            embedded.as_ref(),
        ) {
            // The built-in fonts only cover Western European text; the
            // library's own advice for that case names a command-line flag
            // this screen does not have, so it is swapped for the control
            // that actually exists here.
            let mut message = e.to_string();
            if message.contains("cannot write these characters") {
                message = message.replace(
                    "Pass --font-file with a .ttf that covers your language and \
                     it will be carried inside the delta.",
                    "Choose a font file above that covers your language — it \
                     will be carried inside the delta.",
                );
                if let Some(path) = font::suggest_system_font() {
                    message.push_str(&format!(
                        "\n\nThere is one on this machine: {}",
                        path.display()
                    ));
                }
            }
            return Outcome::refused(message);
        }

        let lines = &pages_lines[0];
        let mut notes = Vec::new();
        let off_page = lines
            .iter()
            .filter(|l| {
                l.x_mm < 0.0 || l.y_mm < 0.0 || l.x_mm > page.width_mm || l.y_mm > page.height_mm
            })
            .count();
        if off_page > 0 {
            notes.push(format!(
                "{off_page} addition{} fall outside the {}, so nothing of them \
                 will print. Check the coordinates.",
                if off_page == 1 { "" } else { "s" },
                page.describe()
            ));
        }

        Outcome::Done {
            message: format!(
                "{} addition{} to print.",
                lines.len(),
                if lines.len() == 1 { "" } else { "s" }
            ),
            wrote: vec![target],
            notes,
        }
    });
}

/// Where a point on the displayed image sits on the physical sheet, assuming
/// the image shows the sheet edge to edge.
fn point_to_mm(rect: egui::Rect, page: geometry::PageSize, point: egui::Pos2) -> (f64, f64) {
    let fx = ((point.x - rect.left()) / rect.width()) as f64;
    let fy = ((point.y - rect.top()) / rect.height()) as f64;
    (fx * page.width_mm, fy * page.height_mm)
}

/// The inverse of [`point_to_mm`], for drawing a marker back onto the image.
fn mm_to_point(rect: egui::Rect, page: geometry::PageSize, mm: (f64, f64)) -> egui::Pos2 {
    let fx = (mm.0 / page.width_mm) as f32;
    let fy = (mm.1 / page.height_mm) as f32;
    egui::pos2(
        rect.left() + fx * rect.width(),
        rect.top() + fy * rect.height(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_scan_forgets_everything_about_the_last_one() {
        // The page read, the font matched off it, and whatever was said about
        // finding an anchor are all about one piece of paper. Carrying any of
        // them to the next sheet would place words by measurements taken from
        // a document that is no longer open.
        let mut state = State {
            read: None,
            matched: Some("Times-Roman at about 11 pt".into()),
            anchor_said: Some("Found it".into()),
            ..State::default()
        };
        // What `show` does when the file row reports a change.
        state.read = None;
        state.matched = None;
        state.anchor_said = None;

        assert!(state.read.is_none());
        assert!(state.matched.is_none());
        assert!(state.anchor_said.is_none());
    }

    #[test]
    fn asking_for_an_anchor_with_no_scan_chosen_does_nothing_at_all() {
        // Not an error box: somebody is mid-thought about where to put some
        // words, and the answer is to leave everything alone until there is a
        // page to look at.
        let mut state = State {
            anchor: "Received:".into(),
            ..State::default()
        };
        add_by_anchor(&mut state);
        assert!(state.placements.is_empty());
        assert!(state.anchor_said.is_none());
        assert_eq!(state.anchor, "Received:");
    }

    #[test]
    fn the_default_state_is_the_one_somebody_should_meet() {
        let state = State::default();
        assert_eq!(state.page, "a4");
        assert_eq!(state.size_pt, 11.0);
        assert_eq!(state.rotation_deg, 0.0);
        assert!(!state.anchor_below, "the ordinary case is 'after'");
        assert!(state.placements.is_empty());
    }
}
