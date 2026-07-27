//! Make a document from a blank page, and keep adding to it.
//!
//! Onionskin's own file format, not a scan or a pair of Word files: choose
//! the paper, put words on it, print it, then keep adding — and print only
//! what was added, onto the sheet already in the tray. The delta here is
//! exact rather than measured, because the document remembers precisely
//! which words were on the page when it was printed. See `onionskin::document`
//! for why that makes this workflow different from the other two.
//!
//! This file also holds the page canvas — painting the paper on its desk,
//! and the millimetre-to-pixel mapping — shared with [`super::draw`], which
//! draws the same page while drawing on it.

use std::collections::HashSet;
use std::path::PathBuf;

use eframe::egui;
use egui::epaint::EllipseShape;

use onionskin::document::{Document, Item, Shape, ShapeKind};
use onionskin::geometry::{parse_page, PageSize, PAGE_PRESETS};
use onionskin::pdf;

use super::Room;
use crate::job::Outcome;
use crate::theme;
use crate::widgets;

pub struct State {
    /// Where the document lives on disk. `None` until one is opened or made
    /// — there is nothing to show or edit before then.
    path: Option<PathBuf>,
    doc: Option<Document>,

    open_pick: Option<PathBuf>,
    open_error: Option<String>,

    new_page: String,
    new_pages: usize,
    new_path: Option<PathBuf>,
    new_error: Option<String>,

    /// Which page is on screen. Not part of the document itself — two people
    /// looking at the same file might be looking at different pages of it.
    page: usize,
    selected: Option<u32>,
    dragging: Option<u32>,

    /// The item the form below is editing, or `None` while it is adding a
    /// new one instead.
    editing: Option<u32>,
    draft: ItemDraft,

    save_error: Option<String>,

    output: Option<PathBuf>,
    delta: bool,
    mark_printed: bool,
    /// Set when a print job starts with "note this as printed" ticked. Held
    /// here, rather than acted on immediately, because the write has not
    /// happened yet — only once the job reports success is it true that this
    /// is what is on the paper.
    pending_mark: Option<Document>,
}

impl Default for State {
    fn default() -> Self {
        State {
            path: None,
            doc: None,
            open_pick: None,
            open_error: None,
            new_page: "a4".to_string(),
            new_pages: 1,
            new_path: None,
            new_error: None,
            page: 1,
            selected: None,
            dragging: None,
            editing: None,
            draft: ItemDraft::default(),
            save_error: None,
            output: None,
            delta: false,
            mark_printed: true,
            pending_mark: None,
        }
    }
}

/// What the add/edit form is holding, kept apart from [`Item`] because it has
/// to represent things an `Item` cannot — a wrap width that is turned off
/// rather than merely absent, for one — and because text mid-edit is not
/// always a valid item yet.
struct ItemDraft {
    text: String,
    x_mm: f64,
    y_mm: f64,
    size_pt: f64,
    font: String,
    colour: String,
    wrap: bool,
    width_mm: f64,
    rotation_deg: f64,
    leading: f64,
}

impl Default for ItemDraft {
    fn default() -> Self {
        ItemDraft {
            text: String::new(),
            x_mm: 25.0,
            y_mm: 40.0,
            size_pt: 11.0,
            font: pdf::Font::Helvetica.base_name().to_string(),
            colour: "black".to_string(),
            wrap: false,
            width_mm: 160.0,
            rotation_deg: 0.0,
            leading: 1.2,
        }
    }
}

impl ItemDraft {
    fn from_item(item: &Item) -> ItemDraft {
        ItemDraft {
            text: item.text.clone(),
            x_mm: item.x_mm,
            y_mm: item.y_mm,
            size_pt: item.size_pt,
            font: item.font.clone(),
            colour: item.colour.clone(),
            wrap: item.width_mm.is_some(),
            width_mm: item.width_mm.unwrap_or(160.0),
            rotation_deg: item.rotation_deg,
            leading: item.leading,
        }
    }

    /// A fresh item from the current field values. `id` is set by
    /// [`Document::add`], not here.
    fn build(&self, page: usize) -> Item {
        Item {
            id: 0,
            page,
            x_mm: self.x_mm,
            y_mm: self.y_mm,
            text: self.text.clone(),
            size_pt: self.size_pt,
            font: self.font.clone(),
            width_mm: if self.wrap { Some(self.width_mm) } else { None },
            rotation_deg: self.rotation_deg,
            colour: self.colour.clone(),
            leading: self.leading,
        }
    }
}

pub fn show(state: &mut State, room: &mut Room) {
    let screen = super::Screen::Document;
    widgets::title(room.ui, screen.name(), screen.lede());

    if state.doc.is_none() {
        show_picker(state, room);
        return;
    }

    // Taken for the frame, so the rest of this function can read and mutate
    // it directly without fighting the borrow checker over a field of
    // `state` that the same closures also need. Put back below, unless the
    // person asked to close it.
    let mut doc = state.doc.take().expect("checked above");
    apply_pending_mark(state, &mut doc, room);
    let close = show_editor(state, &mut doc, room);
    if close {
        state.path = None;
        reset_editor_state(state);
    } else {
        state.doc = Some(doc);
    }
}

fn show_picker(state: &mut State, room: &mut Room) {
    room.ui.label(egui::RichText::new("Open a document already started").strong());
    if widgets::file_row(room.ui, room.picker, "Document", &mut state.open_pick, &["onionskin"],
        room.dropped,
    ) {
        match state.open_pick.clone() {
            Some(path) => match Document::load(&path) {
                Ok(doc) => {
                    reset_editor_state(state);
                    state.path = Some(path);
                    state.doc = Some(doc);
                }
                Err(e) => {
                    state.open_error = Some(e.to_string());
                    state.open_pick = None;
                }
            },
            None => state.open_error = None,
        }
    }
    if let Some(err) = &state.open_error {
        widgets::refused(room.ui, err);
    }

    room.ui.add_space(18.0);
    room.ui.separator();
    room.ui.add_space(10.0);

    room.ui.label(egui::RichText::new("Or start a new one").strong());
    room.ui.horizontal(|ui| {
        ui.label("Paper");
        egui::ComboBox::from_id_salt("new-doc-page")
            .selected_text(state.new_page.clone())
            .show_ui(ui, |ui| {
                for preset in PAGE_PRESETS {
                    let name = preset.0;
                    ui.selectable_value(&mut state.new_page, name.to_string(), name);
                }
            });
        ui.add(egui::TextEdit::singleline(&mut state.new_page).desired_width(90.0));
        widgets::hint(ui, "a name above, or WIDTHxHEIGHT in mm");
    });
    room.ui.horizontal(|ui| {
        ui.label("Sheets");
        ui.add(egui::DragValue::new(&mut state.new_pages).range(1..=999));
    });
    widgets::save_row(
        room.ui,
        room.picker,
        "Where to keep it",
        &mut state.new_path,
        "document.onionskin",
        &["onionskin"],
        "choose where to keep it — Create needs a place to put it",
    );

    let page = parse_page(&state.new_page);
    if let Err(e) = &page {
        widgets::hint(room.ui, e);
    }
    let ready = state.new_path.is_some() && page.is_ok();
    if room
        .ui
        .add_enabled(ready, egui::Button::new("Create"))
        .clicked()
    {
        if let (Ok(page), Some(path)) = (page, state.new_path.clone()) {
            let doc = Document::blank(page, state.new_pages);
            match doc.save(&path) {
                Ok(()) => {
                    reset_editor_state(state);
                    state.path = Some(path);
                    state.doc = Some(doc);
                }
                Err(e) => state.new_error = Some(e.to_string()),
            }
        }
    }
    if let Some(err) = &state.new_error {
        widgets::refused(room.ui, err);
    }
}

/// Apply a print job's success to the document it was printed from, once —
/// not the moment the job starts, because nothing is really printed until it
/// has finished; and not from inside the worker thread, because the document
/// on screen may have kept changing while the write was in flight.
fn apply_pending_mark(state: &mut State, doc: &mut Document, room: &Room) {
    if room.jobs.busy() {
        return;
    }
    let Some(marked) = state.pending_mark.take() else {
        return;
    };
    if matches!(room.jobs.last, Some(Outcome::Done { .. })) {
        doc.printed = marked.printed;
        doc.printed_shapes = marked.printed_shapes;
        save(state, doc);
    }
}

fn show_editor(state: &mut State, doc: &mut Document, room: &mut Room) -> bool {
    let mut close = false;
    room.ui.horizontal(|ui| {
        widgets::hint(
            ui,
            &state
                .path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
        );
        if ui.small_button("Close").clicked() {
            close = true;
        }
    });
    if close {
        return true;
    }
    room.ui.add_space(4.0);

    // The document may have fewer pages than the last time this screen had
    // it open — someone could edit the file by hand between sessions.
    state.page = state.page.clamp(1, doc.pages);

    let problems = doc.overlay_problems();
    if !problems.is_empty() {
        let mut message = String::from(
            "The sheet in your hand no longer matches this document, and no \
             amount of adding to it will fix that — toner does not come off \
             paper.\n\n",
        );
        message.push_str(&problems.iter().map(|p| p.format()).collect::<Vec<_>>().join("\n\n"));
        message.push_str("\n\nPrint the page fresh, then carry on adding to it.");
        widgets::refused(room.ui, &message);
        room.ui.add_space(10.0);
    }

    room.ui.horizontal(|ui| {
        ui.label("Page");
        if ui
            .add_enabled(state.page > 1, egui::Button::new("◀"))
            .clicked()
        {
            state.page -= 1;
            deselect(state);
        }
        ui.label(format!("{} of {}", state.page, doc.pages));
        if ui
            .add_enabled(state.page < doc.pages, egui::Button::new("▶"))
            .clicked()
        {
            state.page += 1;
            deselect(state);
        }
        // Deleting a piece of text is one click here, which makes a way back
        // matter more than it does on the command line. The button appears
        // only when there is something to go back to, so it never offers to
        // undo a document nobody has touched.
        if let Some(path) = &state.path {
            let back = onionskin::document::steps_back(path);
            let forward = onionskin::document::steps_forward(path);
            if ui
                .add_enabled(back > 0, egui::Button::new("Undo"))
                .on_hover_text(format!(
                    "Go back a step. {back} to go back through."
                ))
                .on_disabled_hover_text("Nothing has changed since this was opened")
                .clicked()
            {
                step_back(state, doc);
            }
            if ui
                .add_enabled(forward > 0, egui::Button::new("Redo"))
                .on_hover_text(format!("Come forward again. {forward} to come through."))
                .on_disabled_hover_text("Nothing has been undone")
                .clicked()
            {
                step_forward(state, doc);
            }
        }
        if ui.button("Add a blank page").clicked() {
            doc.pages += 1;
            state.page = doc.pages;
            deselect(state);
            save(state, doc);
        }
    });
    room.ui.add_space(6.0);

    show_canvas(state, doc, room);

    room.ui.add_space(12.0);
    show_item_list(state, doc, room);

    room.ui.add_space(14.0);
    room.ui.separator();
    room.ui.add_space(8.0);
    show_item_form(state, doc, room);

    room.ui.add_space(14.0);
    room.ui.separator();
    room.ui.add_space(8.0);
    show_print(state, doc, room);

    false
}

fn show_canvas(state: &mut State, doc: &mut Document, room: &mut Room) {
    let canvas = page_canvas(room.ui, doc.page);
    let painter = &canvas.painter;
    let transform = &canvas.transform;

    // Drawings first, so a piece of text written beside one sits on top of
    // it rather than under it — matching how the PDF itself layers the two.
    let shapes_added: HashSet<u32> = doc
        .shapes_added_since_printing()
        .iter()
        .map(|s| s.id)
        .collect();
    for shape in doc.shapes.iter().filter(|s| s.page == state.page) {
        let tint = if shapes_added.contains(&shape.id) {
            theme::ADDED
        } else {
            theme::EXISTING
        };
        draw_shape(painter, transform, shape, Some(tint));
    }

    let items_added: HashSet<u32> = doc.added_since_printing().iter().map(|i| i.id).collect();
    let mut hitboxes: Vec<(u32, egui::Rect)> = Vec::new();
    let mut unreadable = 0usize;
    for item in doc.on_page(state.page) {
        let colour = if items_added.contains(&item.id) {
            theme::ADDED
        } else {
            theme::EXISTING
        };
        match draw_item(painter, transform, item, colour) {
            Ok(rects) => hitboxes.extend(rects.into_iter().map(|r| (item.id, r))),
            Err(()) => unreadable += 1,
        }
    }

    // A highlight round whatever the form below is about to change, so the
    // two are unambiguously the same piece of text.
    if let Some(id) = state.selected {
        let boxed = hitboxes.iter().filter(|(i, _)| *i == id).map(|(_, r)| *r);
        if let Some(rect) = boxed.reduce(|a, b| a.union(b)) {
            painter.rect_stroke(
                rect.expand(3.0),
                2,
                egui::Stroke::new(2.0, room.ui.visuals().selection.stroke.color),
                egui::StrokeKind::Outside,
            );
        }
    }

    let hit_at = |pos: egui::Pos2| hitboxes.iter().rev().find(|(_, r)| r.contains(pos)).map(|(id, _)| *id);

    if canvas.response.drag_started() {
        if let Some(pos) = canvas.response.interact_pointer_pos() {
            let hit = hit_at(pos);
            state.dragging = hit;
            if hit.is_some() {
                state.selected = hit;
                select_for_editing(state, doc);
            }
        }
    }
    if let Some(id) = state.dragging {
        if canvas.response.dragged() {
            let delta = canvas.response.drag_delta();
            let (dx, dy) = (transform.mm(delta.x), transform.mm(delta.y));
            if let Ok(item) = doc.get_mut(id) {
                item.x_mm += dx;
                item.y_mm += dy;
                if state.editing == Some(id) {
                    state.draft.x_mm = item.x_mm;
                    state.draft.y_mm = item.y_mm;
                }
            }
        }
        if canvas.response.drag_stopped() {
            state.dragging = None;
            save(state, doc);
        }
    } else if canvas.response.clicked() {
        if let Some(pos) = canvas.response.interact_pointer_pos() {
            state.selected = hit_at(pos);
            select_for_editing(state, doc);
        }
    }

    if unreadable > 0 {
        widgets::hint(
            room.ui,
            &format!(
                "{unreadable} item(s) are set in a font file this screen cannot load, so \
                 they are not shown here. They still print."
            ),
        );
    }
}

fn show_item_list(state: &mut State, doc: &mut Document, room: &mut Room) {
    room.ui.label(egui::RichText::new("On this page").strong());
    let ids: Vec<u32> = doc.on_page(state.page).map(|i| i.id).collect();
    if ids.is_empty() {
        widgets::hint(room.ui, "Nothing here yet — add a piece of text below.");
        return;
    }

    let mut erase = None;
    egui::ScrollArea::vertical()
        .max_height(180.0)
        .show(room.ui, |ui| {
            for id in ids {
                let Some(item) = doc.get(id) else { continue };
                let label = format!(
                    "{:>3}   {:>6.1}, {:<6.1} mm   {}",
                    item.id,
                    item.x_mm,
                    item.y_mm,
                    first_line(&item.text)
                );
                let is_selected = state.selected == Some(id);

                let mut row_clicked = false;
                let mut delete_clicked = false;
                ui.horizontal(|ui| {
                    if ui.selectable_label(is_selected, label).clicked() {
                        row_clicked = true;
                    }
                    if ui.small_button("×").clicked() {
                        delete_clicked = true;
                    }
                });
                if row_clicked {
                    state.selected = Some(id);
                    select_for_editing(state, doc);
                }
                if delete_clicked {
                    erase = Some(id);
                }
            }
        });

    if let Some(id) = erase {
        let _ = doc.remove(id);
        if state.selected == Some(id) {
            state.selected = None;
        }
        if state.editing == Some(id) {
            state.editing = None;
            state.draft = ItemDraft::default();
        }
        save(state, doc);
    }
}

fn show_item_form(state: &mut State, doc: &mut Document, room: &mut Room) {
    let adding = state.editing.is_none();
    room.ui.label(
        egui::RichText::new(if adding {
            "Add a piece of text"
        } else {
            "Edit the selected text"
        })
        .strong(),
    );

    room.ui.add(
        egui::TextEdit::multiline(&mut state.draft.text)
            .desired_rows(3)
            .hint_text("What the sheet should say"),
    );

    room.ui.horizontal(|ui| {
        ui.label("At");
        ui.add(egui::DragValue::new(&mut state.draft.x_mm).speed(0.5).suffix(" mm"));
        ui.label(",");
        ui.add(egui::DragValue::new(&mut state.draft.y_mm).speed(0.5).suffix(" mm"));
        widgets::hint(ui, "from the top-left corner; the second number is the baseline");
    });

    room.ui.horizontal(|ui| {
        ui.label("Size");
        ui.add(
            egui::DragValue::new(&mut state.draft.size_pt)
                .range(1.0..=400.0)
                .suffix(" pt"),
        );
        ui.label("Font");
        egui::ComboBox::from_id_salt("item-font")
            .selected_text(state.draft.font.clone())
            .show_ui(ui, |ui| {
                for font in pdf::Font::all() {
                    ui.selectable_value(
                        &mut state.draft.font,
                        font.base_name().to_string(),
                        font.base_name(),
                    );
                }
            });
    });

    colour_field(room.ui, "Colour", &mut state.draft.colour);

    room.ui.collapsing("More", |ui| {
        ui.horizontal(|ui| {
            ui.checkbox(&mut state.draft.wrap, "Wrap at");
            ui.add_enabled_ui(state.draft.wrap, |ui| {
                ui.add(
                    egui::DragValue::new(&mut state.draft.width_mm)
                        .range(1.0..=2000.0)
                        .suffix(" mm"),
                );
            });
        });
        ui.horizontal(|ui| {
            ui.label("Rotation");
            ui.add(egui::DragValue::new(&mut state.draft.rotation_deg).suffix("°"));
            ui.label("Line spacing");
            ui.add(
                egui::DragValue::new(&mut state.draft.leading)
                    .range(0.5..=4.0)
                    .speed(0.05),
            );
        });
    });

    if state.draft.x_mm < 0.0
        || state.draft.y_mm < 0.0
        || state.draft.x_mm > doc.page.width_mm
        || state.draft.y_mm > doc.page.height_mm
    {
        widgets::hint(room.ui, "That is off the edge of the paper — it will not print.");
    }
    if overflows_page(&state.draft, doc.page) {
        widgets::hint(
            room.ui,
            "This runs past the right edge of the paper at this size and position.",
        );
    }

    let problem = validate_draft(&state.draft);
    room.ui.add_space(6.0);
    room.ui.horizontal(|ui| {
        let label = if adding { "Add" } else { "Save changes" };
        if ui
            .add_enabled(problem.is_none(), egui::Button::new(label))
            .clicked()
        {
            if adding {
                let item = state.draft.build(state.page);
                match doc.add(item) {
                    Ok(id) => {
                        state.selected = Some(id);
                        state.draft.text.clear();
                        save(state, doc);
                    }
                    Err(e) => state.save_error = Some(e.to_string()),
                }
            } else if let Some(id) = state.editing {
                if let Ok(existing) = doc.get_mut(id) {
                    let fresh = state.draft.build(existing.page);
                    existing.text = fresh.text;
                    existing.x_mm = fresh.x_mm;
                    existing.y_mm = fresh.y_mm;
                    existing.size_pt = fresh.size_pt;
                    existing.font = fresh.font;
                    existing.width_mm = fresh.width_mm;
                    existing.rotation_deg = fresh.rotation_deg;
                    existing.colour = fresh.colour;
                    existing.leading = fresh.leading;
                    save(state, doc);
                }
            }
        }
        if !adding {
            if ui.button("Delete").clicked() {
                if let Some(id) = state.editing {
                    let _ = doc.remove(id);
                    state.editing = None;
                    state.selected = None;
                    state.draft = ItemDraft::default();
                    save(state, doc);
                }
            }
            if ui.button("Add new instead").clicked() {
                state.editing = None;
                state.selected = None;
                state.draft = ItemDraft::default();
            }
        }
    });
    if let Some(problem) = &problem {
        widgets::hint(room.ui, problem);
    }
    if let Some(err) = &state.save_error {
        room.ui.add_space(6.0);
        widgets::refused(room.ui, err);
    }
}

fn show_print(state: &mut State, doc: &mut Document, room: &mut Room) {
    room.ui.label(egui::RichText::new("Print").strong());

    widgets::save_row(
        room.ui,
        room.picker,
        "Write a PDF to",
        &mut state.output,
        "document.pdf",
        &["pdf"],
        "beside the document, as document.pdf",
    );

    if doc.has_been_printed() {
        room.ui.horizontal(|ui| {
            ui.label("What to print");
            egui::ComboBox::from_id_salt("print-mode")
                .selected_text(if state.delta {
                    "Only what has been added since it was printed"
                } else {
                    "The whole document"
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut state.delta, false, "The whole document");
                    ui.selectable_value(
                        &mut state.delta,
                        true,
                        "Only what has been added since it was printed",
                    );
                });
        });
    } else {
        state.delta = false;
    }

    room.ui
        .checkbox(&mut state.mark_printed, "Note this as printed once it is written");
    widgets::hint(room.ui, "so a later print can offer just what is added from here on.");

    let problems = if state.delta { doc.overlay_problems() } else { Vec::new() };
    let blocked = state.delta && !problems.is_empty();
    if blocked {
        room.ui.add_space(6.0);
        widgets::refused(
            room.ui,
            "What is already on the sheet has changed since it was printed, so a \
             delta cannot go safely onto that paper. Print the whole document \
             instead, or undo what changed.",
        );
    }

    let ready = state.output.is_some() && !blocked;
    let busy = room.jobs.busy();
    room.ui.add_space(6.0);
    if room
        .ui
        .add_enabled(
            ready && !busy,
            egui::Button::new(egui::RichText::new("Write the PDF").strong()),
        )
        .clicked()
    {
        start_print(state, doc, room);
    }
    if state.output.is_none() {
        widgets::hint(room.ui, "Choose where to write it first.");
    }

    if let Some(outcome) = &room.jobs.last {
        room.ui.add_space(10.0);
        if widgets::outcome(room.ui, outcome) {
            room.jobs.dismiss();
        }
    }
}

fn start_print(state: &mut State, doc: &Document, room: &mut Room) {
    let Some(output) = state.output.clone() else { return };
    let snapshot = doc.clone();
    let delta = state.delta;

    // Recorded now, against exactly what is being sent to the writer — not
    // whatever the document happens to hold once the job reports back, which
    // may already have more added to it.
    state.pending_mark = if state.mark_printed {
        let mut marked = snapshot.clone();
        marked.mark_printed();
        Some(marked)
    } else {
        None
    };

    room.previews.forget(&output);
    let target = output;
    room.jobs.start("Writing the PDF", move |_report| {
        let lines = if delta {
            snapshot.delta_layout(None)
        } else {
            snapshot.layout(None)
        };
        let lines = match lines {
            Ok(lines) => lines,
            Err(e) => return Outcome::refused(e.to_string()),
        };
        let drawings = if delta {
            snapshot.shape_layout(&snapshot.shapes_added_since_printing())
        } else {
            snapshot.shape_layout(&snapshot.shapes.iter().collect::<Vec<_>>())
        };
        let written: usize = lines.iter().map(|p| p.len()).sum();
        let drawn: usize = drawings.iter().map(|p| p.len()).sum();

        match pdf::write_page_content(
            &target,
            &snapshot.page_sizes(),
            &lines,
            &drawings,
            "Onionskin document",
            None,
        ) {
            Ok(()) => Outcome::wrote(
                format!(
                    "{} page{}, {written} line{}{}.",
                    snapshot.pages,
                    if snapshot.pages == 1 { "" } else { "s" },
                    if written == 1 { "" } else { "s" },
                    match drawn {
                        0 => String::new(),
                        1 => ", 1 drawing".to_string(),
                        n => format!(", {n} drawings"),
                    }
                ),
                vec![target],
            ),
            Err(e) => Outcome::refused(e.to_string()),
        }
    });
}

/// Write the document back to disk. Called after every change: there is no
/// separate Save button, because a `.onionskin` file is JSON and saving it is
/// fast enough to do inline, and a screen that can show something the file on
/// disk disagrees with is worse than one extra write.
fn save(state: &mut State, doc: &Document) {
    let Some(path) = &state.path else { return };
    match doc.save(path) {
        Ok(()) => state.save_error = None,
        Err(e) => state.save_error = Some(e.to_string()),
    }
}

/// Put the document back as it was before the last change, and show it.
///
/// The document on screen has to be re-read afterwards rather than kept: what
/// is in memory is the version being undone, and drawing it over the restored
/// file would put the mistake straight back the next time anything was saved.
///
/// The restored document goes into `doc` — the one the caller took out of
/// `state` for the frame — and *not* into `state.doc`. Writing it to
/// `state.doc` looks right and is not: `show` puts its own `doc` back when
/// this returns, which would quietly overwrite the restored version with the
/// mistake and undo the undo.
fn step_back(state: &mut State, doc: &mut Document) {
    step(state, doc, onionskin::document::undo)
}

/// Come forward again, the other half of going back.
fn step_forward(state: &mut State, doc: &mut Document) {
    step(state, doc, onionskin::document::redo)
}

/// Move the document one step along its history, either way.
fn step(
    state: &mut State,
    doc: &mut Document,
    which: fn(&std::path::Path) -> Result<(), onionskin::document::DocumentError>,
) {
    let Some(path) = state.path.clone() else {
        return;
    };
    match which(&path) {
        Ok(()) => match Document::load(&path) {
            Ok(restored) => {
                *doc = restored;
                state.save_error = None;
                deselect(state);
                // The page being looked at may not exist in the version that
                // came back.
                state.page = state.page.clamp(1, doc.pages.max(1));
            }
            Err(e) => state.save_error = Some(e.to_string()),
        },
        Err(e) => state.save_error = Some(e.to_string()),
    }
}

/// What is wrong with the draft, if anything — checked before the button is
/// enabled, so a person sees why before Onionskin merely refuses.
fn validate_draft(draft: &ItemDraft) -> Option<String> {
    if draft.text.trim().is_empty() {
        return Some("There is nothing to write yet.".to_string());
    }
    if !(draft.size_pt.is_finite() && draft.size_pt > 0.0) {
        return Some("The type size must be a number greater than nothing.".to_string());
    }
    if onionskin::document::parse_colour(&draft.colour).is_err() {
        return Some(format!("{:?} is not a colour Onionskin understands.", draft.colour));
    }
    if draft.wrap && !(draft.width_mm.is_finite() && draft.width_mm > 0.0) {
        return Some("The wrap width must be a number greater than nothing.".to_string());
    }
    if pdf::Font::parse(&draft.font).is_none() {
        return Some(format!("{:?} is not one of the built-in fonts.", draft.font));
    }
    None
}

/// Whether this text would print past the right edge of the page — using the
/// same widths the PDF itself uses, so this warns about exactly the overflow
/// that would happen and not an approximation of it.
fn overflows_page(draft: &ItemDraft, page: PageSize) -> bool {
    if draft.wrap {
        return draft.x_mm + draft.width_mm > page.width_mm;
    }
    let Some(font) = pdf::Font::parse(&draft.font) else {
        return false;
    };
    draft.text.split('\n').any(|paragraph| {
        draft.x_mm + pdf::builtin_width_mm(font, paragraph, draft.size_pt) > page.width_mm
    })
}

fn first_line(text: &str) -> String {
    let first = text.lines().next().unwrap_or("");
    let shown: String = first.chars().take(50).collect();
    if shown.chars().count() < first.chars().count() || text.contains('\n') {
        format!("{shown}…")
    } else {
        shown
    }
}

/// Load the selected item's real values into the form, unless it is already
/// there — overwriting on every frame would throw away whatever is being
/// typed mid-edit.
fn select_for_editing(state: &mut State, doc: &Document) {
    let Some(id) = state.selected else { return };
    if state.editing == Some(id) {
        return;
    }
    if let Some(item) = doc.get(id) {
        state.draft = ItemDraft::from_item(item);
        state.editing = Some(id);
    }
}

/// Clear the selection when the visible page changes, so the form is not
/// left open on something no longer in view.
fn deselect(state: &mut State) {
    state.selected = None;
    state.dragging = None;
    state.editing = None;
    state.draft = ItemDraft::default();
}

/// Reset everything that belongs to one open document, so a document opened
/// afterwards does not inherit a selection, a drag, or a half-written form
/// pointing at ids that mean something else now.
fn reset_editor_state(state: &mut State) {
    state.page = 1;
    deselect(state);
    state.save_error = None;
    state.output = None;
    state.delta = false;
    state.mark_printed = true;
    state.pending_mark = None;
    state.open_pick = None;
    state.open_error = None;
    state.new_path = None;
    state.new_error = None;
}

// ---------------------------------------------------------------------------
// The page canvas, shared with `draw`.
//
// Nothing below this line knows about the add/edit form above it — it only
// draws a page and turns clicks into millimetres, which is exactly as true
// on the drawing screen as it is here.
// ---------------------------------------------------------------------------

/// Millimetres on the paper, converted to pixels on screen, and back — the
/// only place a screen pixel appears at all, since everything else in
/// Onionskin is measured in millimetres.
#[derive(Clone, Copy)]
pub struct Transform {
    origin: egui::Pos2,
    px_per_mm: f32,
}

impl Transform {
    pub fn to_screen(self, x_mm: f64, y_mm: f64) -> egui::Pos2 {
        egui::pos2(
            self.origin.x + x_mm as f32 * self.px_per_mm,
            self.origin.y + y_mm as f32 * self.px_per_mm,
        )
    }

    pub fn to_mm(self, point: egui::Pos2) -> (f64, f64) {
        (
            ((point.x - self.origin.x) / self.px_per_mm) as f64,
            ((point.y - self.origin.y) / self.px_per_mm) as f64,
        )
    }

    /// A distance in screen pixels, as millimetres on the page — for turning
    /// a drag's motion into a move in document space.
    pub fn mm(&self, px: f32) -> f64 {
        (px / self.px_per_mm) as f64
    }

    /// A distance in millimetres, as screen pixels.
    pub fn px(&self, mm: f64) -> f32 {
        mm as f32 * self.px_per_mm
    }
}

/// A page, painted and ready for a screen to put its own ink on.
pub struct Canvas {
    pub painter: egui::Painter,
    pub response: egui::Response,
    pub transform: Transform,
}

/// The widest and tallest a page is drawn on screen. A page taller than this
/// shrinks to fit rather than pushing the rest of the screen off the bottom —
/// what matters here is a working canvas, not a ruler-accurate one.
const MAX_CANVAS_WIDTH: f32 = 640.0;
const MAX_CANVAS_HEIGHT: f32 = 520.0;

/// Paint the paper on its desk, sized to fit the space available, and hand
/// back an interactive area plus the millimetre-to-pixel mapping so a screen
/// can put its own ink on top.
pub fn page_canvas(ui: &mut egui::Ui, page: PageSize) -> Canvas {
    let available = ui.available_width().clamp(120.0, MAX_CANVAS_WIDTH);
    let px_per_mm = (available / page.width_mm as f32).min(MAX_CANVAS_HEIGHT / page.height_mm as f32);
    let paper_size = egui::vec2(
        page.width_mm as f32 * px_per_mm,
        page.height_mm as f32 * px_per_mm,
    );
    let desk_size = egui::vec2(ui.available_width(), paper_size.y + 24.0);

    let (rect, response) = ui.allocate_exact_size(desk_size, egui::Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0, theme::desk(ui));

    let paper_min = egui::pos2(
        rect.min.x + (rect.width() - paper_size.x) / 2.0,
        rect.min.y + 12.0,
    );
    let paper_rect = egui::Rect::from_min_size(paper_min, paper_size);
    painter.rect_filled(paper_rect, 1, theme::PAPER);
    painter.rect_stroke(
        paper_rect,
        1,
        egui::Stroke::new(1.0, theme::desk(ui).gamma_multiply(0.7)),
        egui::StrokeKind::Outside,
    );

    Canvas {
        painter,
        response,
        transform: Transform {
            origin: paper_rect.min,
            px_per_mm,
        },
    }
}

/// Draw one item's text, and hand back the on-screen box each of its lines
/// landed in — the only reliable way to know what a click on a wrapped,
/// multi-line, any-size piece of text actually hit.
///
/// `Err` means the item is set in a font file this screen has no way to load
/// — only `onionskin print --font-file` can supply one. The item is real and
/// will still print; it simply cannot be shown here.
///
/// Rotation is not drawn: a rotated preview needs its own pivot arithmetic to
/// match the PDF exactly, rotation is rare, and a wrongly-pivoted rotation
/// would mislead more than an unrotated preview does. What prints is decided
/// by `onionskin::pdf`, not by this canvas.
pub fn draw_item(
    painter: &egui::Painter,
    transform: &Transform,
    item: &Item,
    colour: egui::Color32,
) -> Result<Vec<egui::Rect>, ()> {
    let lines = item.lines(None).map_err(|_| ())?;
    let mut rects = Vec::with_capacity(lines.len());
    for line in &lines {
        let size_mm = onionskin::geometry::pt_to_mm(line.size_pt);
        let font = egui::FontId::new(transform.px(size_mm).max(4.0), egui::FontFamily::Proportional);
        let pos = transform.to_screen(line.x_mm, line.y_mm);
        let mut rect = painter.text(pos, egui::Align2::LEFT_BOTTOM, &line.text, font.clone(), colour);
        // A deliberately blank line paints nothing and returns a rect with no
        // area, which cannot be clicked — pad it so an empty line someone
        // left on purpose can still be selected and moved.
        if rect.width() < 1.0 && rect.height() < 1.0 {
            rect = egui::Rect::from_center_size(pos, egui::vec2(6.0, font.size.max(4.0)));
        }
        rects.push(rect);
    }
    Ok(rects)
}

/// Draw one shape.
///
/// `tint` replaces the shape's own colours with a single one — used on this
/// screen to show a drawing in [`theme::EXISTING`] or [`theme::ADDED`], the
/// same distinction its text is shown in. `None` draws the shape as it will
/// actually print, which is what choosing a colour on the drawing screen
/// needs to show.
pub fn draw_shape(
    painter: &egui::Painter,
    transform: &Transform,
    shape: &Shape,
    tint: Option<egui::Color32>,
) {
    let stroke_colour = match tint {
        Some(c) => Some(c),
        None => shape.stroke.as_deref().and_then(colour32),
    };
    let fill_colour = match tint {
        // A fill tinted as solid as the outline would hide anything drawn
        // inside it — the point of tinting at all is to show new ink without
        // hiding what it lands on.
        Some(c) => shape.fill.as_ref().map(|_| c.gamma_multiply(0.35)),
        None => shape.fill.as_deref().and_then(colour32),
    };
    let has_stroke = stroke_colour.is_some();
    let stroke = egui::Stroke::new(
        transform.px(shape.width_mm).max(1.0),
        stroke_colour.unwrap_or(egui::Color32::TRANSPARENT),
    );

    match &shape.kind {
        ShapeKind::Line {
            x1_mm,
            y1_mm,
            x2_mm,
            y2_mm,
        } => {
            if !has_stroke {
                return;
            }
            let path = [
                transform.to_screen(*x1_mm, *y1_mm),
                transform.to_screen(*x2_mm, *y2_mm),
            ];
            match shape.dash_mm {
                Some((on, off)) => {
                    painter.extend(egui::Shape::dashed_line(&path, stroke, transform.px(on), transform.px(off)));
                }
                None => {
                    painter.line_segment(path, stroke);
                }
            }
        }
        ShapeKind::Rect {
            x_mm,
            y_mm,
            width_mm,
            height_mm,
            radius_mm,
        } => {
            let rect = egui::Rect::from_two_pos(
                transform.to_screen(*x_mm, *y_mm),
                transform.to_screen(x_mm + width_mm, y_mm + height_mm),
            );
            let radius = transform.px(*radius_mm).max(0.0);
            match shape.dash_mm {
                Some((on, off)) => {
                    // A dashed rounded corner needs its own arc length to
                    // space the dashes evenly round it; squaring the corners
                    // here is the difference between that arithmetic and
                    // none, and at preview size nobody will see it.
                    let points = vec![
                        rect.left_top(),
                        rect.right_top(),
                        rect.right_bottom(),
                        rect.left_bottom(),
                        rect.left_top(),
                    ];
                    if let Some(fill) = fill_colour {
                        painter.add(egui::epaint::PathShape::convex_polygon(
                            points.clone(),
                            fill,
                            egui::Stroke::NONE,
                        ));
                    }
                    if has_stroke {
                        painter.extend(egui::Shape::dashed_line(&points, stroke, transform.px(on), transform.px(off)));
                    }
                }
                None => {
                    if let Some(fill) = fill_colour {
                        painter.rect_filled(rect, radius, fill);
                    }
                    if has_stroke {
                        painter.rect_stroke(rect, radius, stroke, egui::StrokeKind::Inside);
                    }
                }
            }
        }
        ShapeKind::Ellipse {
            x_mm,
            y_mm,
            radius_x_mm,
            radius_y_mm,
        } => {
            let centre = transform.to_screen(*x_mm, *y_mm);
            let radius = egui::vec2(
                transform.px(*radius_x_mm).abs(),
                transform.px(*radius_y_mm).abs(),
            );
            match shape.dash_mm {
                Some((on, off)) => {
                    let mut points = ellipse_points(centre, radius);
                    if let Some(fill) = fill_colour {
                        painter.add(egui::epaint::PathShape::convex_polygon(
                            points.clone(),
                            fill,
                            egui::Stroke::NONE,
                        ));
                    }
                    if has_stroke {
                        points.push(points[0]);
                        painter.extend(egui::Shape::dashed_line(&points, stroke, transform.px(on), transform.px(off)));
                    }
                }
                None => {
                    painter.add(EllipseShape {
                        center: centre,
                        radius,
                        fill: fill_colour.unwrap_or(egui::Color32::TRANSPARENT),
                        stroke: if has_stroke { stroke } else { egui::Stroke::NONE },
                        angle: 0.0,
                    });
                }
            }
        }
        ShapeKind::Path { points, closed } => {
            let screen: Vec<egui::Pos2> = points.iter().map(|(x, y)| transform.to_screen(*x, *y)).collect();
            match shape.dash_mm {
                Some((on, off)) => {
                    if *closed {
                        if let Some(fill) = fill_colour {
                            painter.add(egui::epaint::PathShape::convex_polygon(
                                screen.clone(),
                                fill,
                                egui::Stroke::NONE,
                            ));
                        }
                    }
                    if has_stroke {
                        let mut path = screen.clone();
                        if *closed {
                            path.push(screen[0]);
                        }
                        painter.extend(egui::Shape::dashed_line(&path, stroke, transform.px(on), transform.px(off)));
                    }
                }
                None if *closed => {
                    let fill = fill_colour.unwrap_or(egui::Color32::TRANSPARENT);
                    let path_stroke: egui::epaint::PathStroke =
                        if has_stroke { stroke.into() } else { egui::epaint::PathStroke::NONE };
                    painter.add(egui::epaint::PathShape::convex_polygon(screen, fill, path_stroke));
                }
                None => {
                    if has_stroke {
                        painter.add(egui::epaint::PathShape::line(screen, stroke));
                    }
                }
            }
        }
    }
}

fn ellipse_points(centre: egui::Pos2, radius: egui::Vec2) -> Vec<egui::Pos2> {
    const SEGMENTS: usize = 64;
    (0..SEGMENTS)
        .map(|i| {
            let t = i as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
            egui::pos2(centre.x + radius.x * t.cos(), centre.y + radius.y * t.sin())
        })
        .collect()
}

/// The names `parse_colour` accepts as words, so nobody has to remember or
/// type a hex triple for an ordinary colour.
const COLOUR_NAMES: &[&str] = &[
    "black", "white", "grey", "lightgrey", "red", "green", "blue", "yellow", "orange",
];

/// The three numbers `parse_colour` returns, as something egui can paint
/// with — kept as one function so the preview and the print can never
/// disagree about what a name or a hex string means.
pub fn colour32(text: &str) -> Option<egui::Color32> {
    let (r, g, b) = onionskin::document::parse_colour(text).ok()?;
    Some(egui::Color32::from_rgb(
        (r * 255.0).round().clamp(0.0, 255.0) as u8,
        (g * 255.0).round().clamp(0.0, 255.0) as u8,
        (b * 255.0).round().clamp(0.0, 255.0) as u8,
    ))
}

/// A colour chooser: the names above in a menu, a field for anything else (a
/// hex triple, or a name typed by hand), and a swatch of what was actually
/// understood — so a choice is checked by eye, not by reading six hex digits.
///
/// Returns true if the value changed.
pub fn colour_field(ui: &mut egui::Ui, label: &str, value: &mut String) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(label);
        let current = value.trim().to_ascii_lowercase();
        egui::ComboBox::from_id_salt(label)
            .selected_text(if COLOUR_NAMES.contains(&current.as_str()) {
                current.clone()
            } else {
                "custom".to_string()
            })
            .show_ui(ui, |ui| {
                for name in COLOUR_NAMES {
                    if ui.selectable_label(current == *name, *name).clicked() {
                        *value = (*name).to_string();
                        changed = true;
                    }
                }
            });
        if ui.add(egui::TextEdit::singleline(value).desired_width(70.0)).changed() {
            changed = true;
        }
        swatch(ui, value);
    });
    changed
}

/// The same, for a colour that can be turned off altogether — a shape's fill,
/// which is often left hollow.
pub fn colour_field_optional(
    ui: &mut egui::Ui,
    label: &str,
    toggle_label: &str,
    value: &mut Option<String>,
    default: &str,
) -> bool {
    let mut changed = false;
    let mut enabled = value.is_some();
    if ui.checkbox(&mut enabled, toggle_label).changed() {
        *value = if enabled { Some(default.to_string()) } else { None };
        changed = true;
    }
    if let Some(inner) = value {
        if colour_field(ui, label, inner) {
            changed = true;
        }
    }
    changed
}

fn swatch(ui: &mut egui::Ui, value: &str) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::hover());
    match colour32(value) {
        Some(colour) => {
            ui.painter().rect_filled(rect, 3, colour);
            ui.painter().rect_stroke(
                rect,
                3,
                egui::Stroke::new(1.0, ui.visuals().weak_text_color()),
                egui::StrokeKind::Outside,
            );
        }
        None => {
            ui.painter().rect_stroke(rect, 3, egui::Stroke::new(1.0, theme::REFUSED), egui::StrokeKind::Outside);
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "?",
                egui::FontId::default(),
                theme::REFUSED,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    //! `egui::__run_test_ui` builds a bare `Context` and runs one frame with
    //! no simulated pointer input at all — which is exactly what lets these
    //! reach every render path (the loaded editor, the overlay-problem
    //! banner, the edit form) without ever going near a real file dialog:
    //! `widgets::file_row` only opens one when its button is *clicked*, and
    //! nothing is clicked when there is no input to click with.
    use super::*;
    use crate::job::Jobs;
    use crate::preview::Previews;

    const A4: PageSize = PageSize {
        width_mm: 210.0,
        height_mm: 297.0,
    };

    fn temp_path(name: &str) -> PathBuf {
        static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "onionskin-desktop-test-document-{name}-{}-{n}.onionskin",
            std::process::id()
        ))
    }

    fn text_item(text: &str, x_mm: f64, y_mm: f64) -> Item {
        Item {
            id: 0,
            page: 1,
            x_mm,
            y_mm,
            text: text.to_string(),
            size_pt: 11.0,
            font: "Helvetica".to_string(),
            width_mm: None,
            rotation_deg: 0.0,
            colour: "#000000".to_string(),
            leading: 1.2,
        }
    }

    /// A plain outlined box, valid enough for `Document::draw` to accept.
    fn box_shape(x_mm: f64, y_mm: f64) -> Shape {
        Shape {
            id: 0,
            page: 1,
            kind: ShapeKind::Rect {
                x_mm,
                y_mm,
                width_mm: 40.0,
                height_mm: 20.0,
                radius_mm: 0.0,
            },
            stroke: Some("black".to_string()),
            fill: None,
            width_mm: 0.5,
            dash_mm: None,
        }
    }

    /// Run the whole screen once against whatever `state` already holds.
    fn render(state: &mut State) {
        egui::__run_test_ui(|ui| {
            let mut jobs = Jobs::new(ui.ctx());
            let mut previews = Previews::default();
            let mut room = Room {
                ui,
                picker: &mut crate::picker::Picker::default(),
                jobs: &mut jobs,
                previews: &mut previews,
                dropped: &mut Vec::new(),
            };
            show(state, &mut room);
        });
    }

    #[test]
    fn shows_the_picker_when_nothing_is_open() {
        let mut state = State::default();
        render(&mut state);
        assert!(state.doc.is_none());
    }

    #[test]
    fn renders_a_document_with_existing_and_added_ink() {
        let mut doc = Document::blank(A4, 1);
        doc.add(text_item("Dear Sir", 25.0, 40.0)).unwrap();
        doc.draw(box_shape(20.0, 60.0)).unwrap();
        doc.mark_printed();
        doc.add(text_item("Yours faithfully", 25.0, 80.0)).unwrap();
        doc.draw(box_shape(20.0, 100.0)).unwrap();

        let mut state = State {
            path: Some(temp_path("existing-and-added")),
            doc: Some(doc),
            ..State::default()
        };
        render(&mut state);
        assert!(state.doc.is_some(), "a document that renders cleanly stays open");
        let _ = std::fs::remove_file(state.path.take().unwrap());
    }

    #[test]
    fn renders_the_edit_form_for_a_selected_item() {
        let mut doc = Document::blank(A4, 1);
        let id = doc.add(text_item("Dear Sir", 25.0, 40.0)).unwrap();
        let draft = ItemDraft::from_item(doc.get(id).unwrap());

        let mut state = State {
            path: Some(temp_path("edit-form")),
            doc: Some(doc),
            selected: Some(id),
            editing: Some(id),
            draft,
            ..State::default()
        };
        render(&mut state);
        let _ = std::fs::remove_file(state.path.take().unwrap());
    }

    #[test]
    fn renders_the_overlay_problem_banner_once_printed_ink_is_touched() {
        let mut doc = Document::blank(A4, 1);
        let id = doc.add(text_item("Dear Sir", 25.0, 40.0)).unwrap();
        doc.mark_printed();
        doc.get_mut(id).unwrap().text = "Dear Madam".to_string();
        assert!(
            !doc.overlay_problems().is_empty(),
            "the fixture should provoke the problem this test is about"
        );

        let mut state = State {
            path: Some(temp_path("overlay-problem")),
            doc: Some(doc),
            ..State::default()
        };
        render(&mut state);
        let _ = std::fs::remove_file(state.path.take().unwrap());
    }

    #[test]
    fn renders_the_print_section_in_delta_mode_when_blocked() {
        let mut doc = Document::blank(A4, 1);
        let id = doc.add(text_item("Dear Sir", 25.0, 40.0)).unwrap();
        doc.mark_printed();
        doc.get_mut(id).unwrap().x_mm += 5.0; // moved since printing: blocks a delta

        let mut state = State {
            path: Some(temp_path("print-blocked")),
            doc: Some(doc),
            delta: true,
            output: Some(temp_path("print-blocked-output").with_extension("pdf")),
            ..State::default()
        };
        render(&mut state);
        let _ = std::fs::remove_file(state.path.take().unwrap());
    }

    #[test]
    fn draw_item_returns_one_box_per_wrapped_line() {
        egui::__run_test_ui(|ui| {
            let item = text_item("one\ntwo", 20.0, 30.0);
            let transform = Transform {
                origin: egui::pos2(0.0, 0.0),
                px_per_mm: 3.0,
            };
            let rects = draw_item(ui.painter(), &transform, &item, theme::EXISTING);
            assert_eq!(rects.unwrap().len(), 2);
        });
    }

    #[test]
    fn draw_item_reports_items_set_in_a_font_file_this_screen_cannot_load() {
        egui::__run_test_ui(|ui| {
            let mut item = text_item("wide margins need wrapping to check", 20.0, 30.0);
            item.font = "file".to_string();
            item.width_mm = Some(100.0); // wrapping needs to measure the text, which needs the embedded font
            let transform = Transform {
                origin: egui::pos2(0.0, 0.0),
                px_per_mm: 3.0,
            };
            assert!(draw_item(ui.painter(), &transform, &item, theme::EXISTING).is_err());
        });
    }

    #[test]
    fn colour32_understands_names_and_hex_and_refuses_nonsense() {
        assert_eq!(colour32("black"), Some(egui::Color32::from_rgb(0, 0, 0)));
        assert_eq!(colour32("white"), Some(egui::Color32::from_rgb(255, 255, 255)));
        let red = colour32("#ff0000").unwrap();
        assert_eq!((red.r(), red.g(), red.b()), (255, 0, 0));
        assert_eq!(colour32("not a colour"), None);
    }

    #[test]
    fn validate_draft_needs_words_a_real_size_a_known_colour_and_a_real_font() {
        let mut draft = ItemDraft {
            text: "  ".to_string(),
            ..ItemDraft::default()
        };
        assert!(validate_draft(&draft).is_some(), "blank text is refused");

        draft.text = "hello".to_string();
        draft.size_pt = 0.0;
        assert!(validate_draft(&draft).is_some(), "a zero type size is refused");

        draft.size_pt = 11.0;
        draft.colour = "not-a-colour".to_string();
        assert!(validate_draft(&draft).is_some(), "an unknown colour is refused");

        draft.colour = "black".to_string();
        draft.wrap = true;
        draft.width_mm = 0.0;
        assert!(validate_draft(&draft).is_some(), "a zero wrap width is refused");

        draft.wrap = false;
        draft.font = "Not A Font".to_string();
        assert!(validate_draft(&draft).is_some(), "an unknown font is refused");

        draft.font = pdf::Font::Helvetica.base_name().to_string();
        assert!(validate_draft(&draft).is_none(), "a filled-in, valid draft is accepted");
    }

    #[test]
    fn overflows_page_flags_text_that_would_print_past_the_right_edge() {
        let mut draft = ItemDraft {
            x_mm: 190.0,
            text: "a line long enough to run off a narrow sheet of paper".to_string(),
            ..ItemDraft::default()
        };
        assert!(overflows_page(&draft, A4));

        draft.x_mm = 20.0;
        assert!(!overflows_page(&draft, A4));

        draft.wrap = true;
        draft.x_mm = 190.0;
        draft.width_mm = 100.0;
        assert!(overflows_page(&draft, A4), "the wrap box itself hangs off the edge");
    }

    #[test]
    fn first_line_shows_only_the_first_line_and_marks_the_rest() {
        assert_eq!(first_line("Dear Sir"), "Dear Sir");
        assert_eq!(first_line("Dear Sir\nYours faithfully"), "Dear Sir…");
        let long = "x".repeat(80);
        assert!(first_line(&long).ends_with('…'));
    }
}

#[cfg(test)]
mod undo_tests {
    use super::*;

    /// A document on disk with one piece of text on it.
    fn a_document(dir: &std::path::Path) -> (State, PathBuf) {
        let path = dir.join("d.onionskin");
        let mut doc = Document::blank(onionskin::geometry::parse_page("a4").unwrap(), 1);
        doc.add(onionskin::document::Item {
            id: 0,
            page: 1,
            x_mm: 25.0,
            y_mm: 40.0,
            text: "First".into(),
            size_pt: 11.0,
            font: "Helvetica".into(),
            width_mm: None,
            rotation_deg: 0.0,
            colour: "#000000".into(),
            leading: 1.2,
        })
        .unwrap();
        doc.save(&path).unwrap();
        let state = State {
            path: Some(path.clone()),
            doc: Some(Document::load(&path).unwrap()),
            ..State::default()
        };
        (state, path)
    }

    /// Run `undo_last` the way `show` really runs it: on the document taken
    /// out of `state` for the frame, and put back into `state` afterwards.
    ///
    /// The first version of this test called `undo_last(&mut state)` and
    /// asserted on `state.doc`. It passed, and the button was still broken —
    /// `show` put its own copy back over the restored one the moment the
    /// frame ended, so the file went back and the screen did not. Anything
    /// testing this has to go through the same take-and-put-back.
    fn step_as_the_button_does(state: &mut State) {
        let mut doc = state.doc.take().expect("a document is open");
        step_back(state, &mut doc);
        state.doc = Some(doc);
    }

    #[test]
    fn undoing_reloads_rather_than_keeping_what_was_on_screen() {
        // What is in memory is the version being undone. Drawing it over the
        // restored file would put the mistake straight back the next time
        // anything was saved.
        let dir = tempfile::tempdir().unwrap();
        let (mut state, path) = a_document(dir.path());

        let mut doc = state.doc.clone().unwrap();
        doc.remove(1).unwrap();
        doc.save(&path).unwrap();
        state.doc = Some(doc);
        assert_eq!(state.doc.as_ref().unwrap().items.len(), 0);

        step_as_the_button_does(&mut state);
        assert_eq!(
            state.doc.as_ref().unwrap().items.len(),
            1,
            "the screen still shows the version that was undone"
        );
        assert_eq!(Document::load(&path).unwrap().items.len(), 1);
        assert!(state.save_error.is_none());
    }

    #[test]
    fn undoing_with_nothing_to_go_back_to_says_so_and_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let (mut state, _) = a_document(dir.path());
        step_as_the_button_does(&mut state);
        assert!(state.save_error.is_some(), "no complaint was made");
        assert_eq!(state.doc.as_ref().unwrap().items.len(), 1);
    }

    #[test]
    fn the_page_being_looked_at_survives_a_document_that_shrank() {
        // The version that comes back may have fewer pages than the one on
        // screen, and page four of a two-page document is nowhere.
        let dir = tempfile::tempdir().unwrap();
        let (mut state, path) = a_document(dir.path());
        let mut doc = state.doc.clone().unwrap();
        doc.pages = 4;
        doc.save(&path).unwrap();
        state.doc = Some(doc);
        state.page = 4;

        step_as_the_button_does(&mut state);
        let pages = state.doc.as_ref().unwrap().pages;
        assert!(state.page >= 1 && state.page <= pages, "{} of {pages}", state.page);
    }

    #[test]
    fn what_is_on_screen_after_an_undo_is_what_a_save_would_write() {
        // The bug this closes: the file went back, the window did not, and
        // the next save wrote the undone mistake straight back to disk. The
        // round trip is the only assertion that catches it.
        let dir = tempfile::tempdir().unwrap();
        let (mut state, path) = a_document(dir.path());

        let mut doc = state.doc.clone().unwrap();
        doc.remove(1).unwrap();
        doc.save(&path).unwrap();
        state.doc = Some(doc);

        step_as_the_button_does(&mut state);

        // Save what the window is holding, exactly as any later edit would.
        state.doc.as_ref().unwrap().save(&path).unwrap();
        assert_eq!(
            Document::load(&path).unwrap().items.len(),
            1,
            "saving after an undo put the mistake back"
        );
    }
}
