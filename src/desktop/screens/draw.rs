//! Draw on a document: lines, boxes, circles and paths.
//!
//! The other half of what an Onionskin document can carry besides words — see
//! `onionskin::document::Shape`. Nothing here does slow work: a drawing is a
//! few numbers, saving it is the same fast JSON write as everything else in
//! `onionskin::document`, and there is no PDF written until the Print screen
//! — sorry, the Make a document screen's Print section — is asked for one,
//! because a drawing-only document still needs somewhere to be printed from.
//!
//! The page itself — paper on a desk, and the millimetre-to-pixel mapping —
//! is [`super::document::page_canvas`], shared rather than duplicated, since
//! a page is a page whichever screen is looking at it.

use std::path::PathBuf;

use eframe::egui;

use onionskin::document::{Document, Shape, ShapeKind};

use super::document;
use super::Room;
use crate::theme;
use crate::widgets;

pub struct State {
    path: Option<PathBuf>,
    doc: Option<Document>,

    open_pick: Option<PathBuf>,
    open_error: Option<String>,

    page: usize,

    tool: Tool,
    stroke: String,
    outline: bool,
    fill: Option<String>,
    width_mm: f64,
    radius_mm: f64,
    dash: bool,
    dash_on_mm: f64,
    dash_gap_mm: f64,
    close_path: bool,

    /// The shape being dragged out, if the pointer is down over the page.
    drafting: Option<Draft>,
    save_error: Option<String>,
}

impl Default for State {
    fn default() -> Self {
        State {
            path: None,
            doc: None,
            open_pick: None,
            open_error: None,
            page: 1,
            tool: Tool::Line,
            stroke: "black".to_string(),
            outline: true,
            fill: None,
            width_mm: 0.35,
            radius_mm: 0.0,
            dash: false,
            dash_on_mm: 2.0,
            dash_gap_mm: 1.0,
            close_path: false,
            drafting: None,
            save_error: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tool {
    Line,
    Box,
    Circle,
    Path,
}

impl Tool {
    const ALL: [Tool; 4] = [Tool::Line, Tool::Box, Tool::Circle, Tool::Path];

    fn name(&self) -> &'static str {
        match self {
            Tool::Line => "Line",
            Tool::Box => "Box",
            Tool::Circle => "Circle",
            Tool::Path => "Path",
        }
    }
}

/// A shape in progress: where the drag started and where the pointer is now
/// for a tool with two ends, or every point recorded so far for a path — which
/// follows the pointer's whole route rather than just picking out two ends of
/// it.
enum Draft {
    TwoPoint { start: (f64, f64), now: (f64, f64) },
    Path { points: Vec<(f64, f64)> },
}

pub fn show(state: &mut State, room: &mut Room) {
    let screen = super::Screen::Draw;
    widgets::title(room.ui, screen.name(), screen.lede());

    if state.doc.is_none() {
        show_picker(state, room);
        return;
    }

    let mut doc = state.doc.take().expect("checked above");
    let close = show_editor(state, &mut doc, room);
    if close {
        state.path = None;
        reset(state);
    } else {
        state.doc = Some(doc);
    }
}

fn show_picker(state: &mut State, room: &mut Room) {
    widgets::hint(
        room.ui,
        "Open a document to draw on. Start one on the Make a document screen \
         first, if there is not one yet.",
    );
    room.ui.add_space(6.0);
    if widgets::file_row(room.ui, room.picker, "Document", &mut state.open_pick, &["onionskin"]) {
        match state.open_pick.clone() {
            Some(path) => match Document::load(&path) {
                Ok(doc) => {
                    reset(state);
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
}

fn reset(state: &mut State) {
    state.page = 1;
    state.drafting = None;
    state.save_error = None;
    state.open_pick = None;
    state.open_error = None;
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

    state.page = state.page.clamp(1, doc.pages);

    room.ui.horizontal(|ui| {
        ui.label("Page");
        if ui.add_enabled(state.page > 1, egui::Button::new("◀")).clicked() {
            state.page -= 1;
            state.drafting = None;
        }
        ui.label(format!("{} of {}", state.page, doc.pages));
        if ui
            .add_enabled(state.page < doc.pages, egui::Button::new("▶"))
            .clicked()
        {
            state.page += 1;
            state.drafting = None;
        }
    });
    room.ui.add_space(8.0);

    show_tool_options(state, room);
    room.ui.add_space(10.0);

    show_canvas(state, doc, room);
    room.ui.add_space(12.0);

    show_shape_list(state, doc, room);

    if let Some(err) = &state.save_error {
        room.ui.add_space(8.0);
        widgets::refused(room.ui, err);
    }

    false
}

fn show_tool_options(state: &mut State, room: &mut Room) {
    room.ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Tool").strong());
        for tool in Tool::ALL {
            if ui.selectable_label(state.tool == tool, tool.name()).clicked() {
                state.tool = tool;
                state.drafting = None;
            }
        }
    });
    room.ui.add_space(4.0);

    room.ui.horizontal(|ui| {
        ui.checkbox(&mut state.outline, "Outline");
        ui.add_enabled_ui(state.outline, |ui| {
            document::colour_field(ui, "Stroke colour", &mut state.stroke);
        });
    });
    document::colour_field_optional(room.ui, "Fill colour", "Fill", &mut state.fill, "lightgrey");

    room.ui.horizontal(|ui| {
        ui.label("Line width");
        ui.add(
            egui::DragValue::new(&mut state.width_mm)
                .range(0.05..=20.0)
                .speed(0.05)
                .suffix(" mm"),
        );
        if state.tool == Tool::Box {
            ui.label("Corner radius");
            ui.add(
                egui::DragValue::new(&mut state.radius_mm)
                    .range(0.0..=100.0)
                    .speed(0.2)
                    .suffix(" mm"),
            );
        }
        if state.tool == Tool::Path {
            ui.checkbox(&mut state.close_path, "Close the path");
        }
    });

    room.ui.horizontal(|ui| {
        ui.checkbox(&mut state.dash, "Dashed");
        ui.add_enabled_ui(state.dash, |ui| {
            ui.add(
                egui::DragValue::new(&mut state.dash_on_mm)
                    .range(0.1..=50.0)
                    .speed(0.1)
                    .suffix(" mm dash"),
            );
            ui.add(
                egui::DragValue::new(&mut state.dash_gap_mm)
                    .range(0.1..=50.0)
                    .speed(0.1)
                    .suffix(" mm gap"),
            );
        });
    });

    if let Some(problem) = drawing_problem(state) {
        widgets::hint(room.ui, &problem);
    }
}

/// What stops the current settings from drawing a real shape, if anything —
/// checked before a drag is allowed to start, so it is clear beforehand why
/// dragging on the page did nothing, rather than discovering it after.
fn drawing_problem(state: &State) -> Option<String> {
    if !state.outline && state.fill.is_none() {
        return Some(
            "Turn on an outline or a fill first — a shape with neither would be \
             invisible on the page."
                .to_string(),
        );
    }
    if state.outline && document::colour32(&state.stroke).is_none() {
        return Some(format!("{:?} is not a colour Onionskin understands.", state.stroke));
    }
    if let Some(fill) = &state.fill {
        if document::colour32(fill).is_none() {
            return Some(format!("{fill:?} is not a colour Onionskin understands."));
        }
    }
    if !(state.width_mm.is_finite() && state.width_mm > 0.0) {
        return Some("The line width must be a number greater than nothing.".to_string());
    }
    if state.dash
        && !(state.dash_on_mm.is_finite() && state.dash_on_mm > 0.0 && state.dash_gap_mm.is_finite())
    {
        return Some("The dash length must be a number greater than nothing.".to_string());
    }
    None
}

fn show_canvas(state: &mut State, doc: &mut Document, room: &mut Room) {
    let canvas = document::page_canvas(room.ui, doc.page);

    // Everything already on the page, so a box can be drawn round a
    // paragraph without having to remember by heart where it sits.
    for item in doc.on_page(state.page) {
        let _ = document::draw_item(&canvas.painter, &canvas.transform, item, theme::EXISTING);
    }
    for shape in doc.shapes.iter().filter(|s| s.page == state.page) {
        document::draw_shape(&canvas.painter, &canvas.transform, shape, None);
    }

    let ready = drawing_problem(state).is_none();

    if ready {
        if canvas.response.drag_started() {
            if let Some(pos) = canvas.response.interact_pointer_pos() {
                let mm = canvas.transform.to_mm(pos);
                state.drafting = Some(if state.tool == Tool::Path {
                    Draft::Path { points: vec![mm] }
                } else {
                    Draft::TwoPoint { start: mm, now: mm }
                });
            }
        }
        if canvas.response.dragged() {
            if let Some(pos) = canvas.response.interact_pointer_pos() {
                let mm = canvas.transform.to_mm(pos);
                match &mut state.drafting {
                    Some(Draft::TwoPoint { now, .. }) => *now = mm,
                    Some(Draft::Path { points }) => {
                        // Kept only once the pointer has actually moved a
                        // little — otherwise a slow drag records the same
                        // spot hundreds of times over for nothing.
                        let step_mm = canvas.transform.mm(2.0);
                        let far_enough = points.last().map_or(true, |&(x, y)| {
                            let (dx, dy) = (x - mm.0, y - mm.1);
                            (dx * dx + dy * dy).sqrt() > step_mm
                        });
                        if far_enough {
                            points.push(mm);
                        }
                    }
                    None => {}
                }
            }
        }
    }

    if let Some(draft) = &state.drafting {
        if let Some(kind) = shape_kind(state.tool, state.close_path, state.radius_mm, draft) {
            let preview = build_shape(state, kind);
            document::draw_shape(&canvas.painter, &canvas.transform, &preview, None);
        }
    }

    if canvas.response.drag_stopped() {
        if let Some(draft) = state.drafting.take() {
            commit(state, doc, &draft);
        }
    }
}

/// Turn what has been dragged so far into the shape it would produce if
/// released now — the same construction the release itself uses, so the
/// preview never shows something letting go would not actually draw.
fn shape_kind(tool: Tool, close_path: bool, radius_mm: f64, draft: &Draft) -> Option<ShapeKind> {
    match (tool, draft) {
        (Tool::Line, Draft::TwoPoint { start, now }) => Some(ShapeKind::Line {
            x1_mm: start.0,
            y1_mm: start.1,
            x2_mm: now.0,
            y2_mm: now.1,
        }),
        (Tool::Box, Draft::TwoPoint { start, now }) => {
            let (x0, x1) = (start.0.min(now.0), start.0.max(now.0));
            let (y0, y1) = (start.1.min(now.1), start.1.max(now.1));
            Some(ShapeKind::Rect {
                x_mm: x0,
                y_mm: y0,
                width_mm: x1 - x0,
                height_mm: y1 - y0,
                radius_mm,
            })
        }
        (Tool::Circle, Draft::TwoPoint { start, now }) => Some(ShapeKind::Ellipse {
            x_mm: start.0,
            y_mm: start.1,
            radius_x_mm: (now.0 - start.0).abs(),
            radius_y_mm: (now.1 - start.1).abs(),
        }),
        (Tool::Path, Draft::Path { points }) if points.len() >= 2 => Some(ShapeKind::Path {
            points: points.clone(),
            closed: close_path,
        }),
        _ => None,
    }
}

/// The shape the current tool settings would draw, given its outline.
fn build_shape(state: &State, kind: ShapeKind) -> Shape {
    Shape {
        id: 0,
        page: state.page,
        kind,
        stroke: if state.outline { Some(state.stroke.clone()) } else { None },
        fill: state.fill.clone(),
        width_mm: state.width_mm,
        dash_mm: if state.dash {
            Some((state.dash_on_mm, state.dash_gap_mm))
        } else {
            None
        },
    }
}

/// Below this, a drag is a click that twitched rather than a request to draw
/// a shape of zero size — the paper is not going to show it either way.
const MIN_EXTENT_MM: f64 = 1.0;

fn big_enough(kind: &ShapeKind) -> bool {
    match kind {
        ShapeKind::Line {
            x1_mm,
            y1_mm,
            x2_mm,
            y2_mm,
        } => ((x2_mm - x1_mm).powi(2) + (y2_mm - y1_mm).powi(2)).sqrt() >= MIN_EXTENT_MM,
        ShapeKind::Rect { width_mm, height_mm, .. } => width_mm.max(*height_mm) >= MIN_EXTENT_MM,
        ShapeKind::Ellipse {
            radius_x_mm,
            radius_y_mm,
            ..
        } => radius_x_mm.max(*radius_y_mm) >= MIN_EXTENT_MM / 2.0,
        ShapeKind::Path { points, .. } => points.len() >= 2,
    }
}

/// Turn a finished drag into a shape on the document, unless it never became
/// one worth keeping.
fn commit(state: &mut State, doc: &mut Document, draft: &Draft) {
    let Some(kind) = shape_kind(state.tool, state.close_path, state.radius_mm, draft) else {
        return;
    };
    if !big_enough(&kind) || (!state.outline && state.fill.is_none()) {
        return;
    }
    let shape = build_shape(state, kind);
    match doc.draw(shape) {
        Ok(_) => save(state, doc),
        Err(e) => state.save_error = Some(e.to_string()),
    }
}

fn show_shape_list(state: &mut State, doc: &mut Document, room: &mut Room) {
    room.ui.label(egui::RichText::new("Drawings on this page").strong());
    let ids: Vec<u32> = doc
        .shapes
        .iter()
        .filter(|s| s.page == state.page)
        .map(|s| s.id)
        .collect();
    if ids.is_empty() {
        widgets::hint(room.ui, "Nothing drawn here yet — drag on the page above.");
        return;
    }

    let mut erase = None;
    egui::ScrollArea::vertical()
        .max_height(200.0)
        .show(room.ui, |ui| {
            for id in ids {
                let Some(shape) = doc.shapes.iter().find(|s| s.id == id) else { continue };
                let (x0, y0, x1, y1) = shape.bounds();
                let label = format!(
                    "{:>3}   {}   {:.1},{:.1} to {:.1},{:.1} mm",
                    shape.id,
                    shape.describe(),
                    x0,
                    y0,
                    x1,
                    y1
                );
                let swatches: Vec<egui::Color32> = [shape.stroke.as_deref(), shape.fill.as_deref()]
                    .into_iter()
                    .flatten()
                    .filter_map(document::colour32)
                    .collect();

                let mut delete_clicked = false;
                ui.horizontal(|ui| {
                    ui.label(label);
                    for colour in &swatches {
                        let (rect, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
                        ui.painter().rect_filled(rect, 2, *colour);
                    }
                    if ui.small_button("×").clicked() {
                        delete_clicked = true;
                    }
                });
                if delete_clicked {
                    erase = Some(id);
                }
            }
        });

    if let Some(id) = erase {
        let _ = doc.erase_shape(id);
        save(state, doc);
    }
}

/// Write the document back to disk. There is no separate Save button — see
/// `document::save` for why that is right rather than merely convenient.
fn save(state: &mut State, doc: &Document) {
    let Some(path) = &state.path else { return };
    match doc.save(path) {
        Ok(()) => state.save_error = None,
        Err(e) => state.save_error = Some(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    //! See the note at the top of `document`'s tests for why running a bare
    //! `egui::__run_test_ui` frame never touches a real file dialog.
    use super::*;
    use crate::job::Jobs;
    use crate::preview::Previews;
    use onionskin::document::Item;
    use onionskin::geometry::PageSize;

    const A4: PageSize = PageSize {
        width_mm: 210.0,
        height_mm: 297.0,
    };

    fn temp_path(name: &str) -> PathBuf {
        static COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "onionskin-desktop-test-draw-{name}-{}-{n}.onionskin",
            std::process::id()
        ))
    }

    fn render(state: &mut State) {
        egui::__run_test_ui(|ui| {
            let mut jobs = Jobs::new(ui.ctx());
            let mut previews = Previews::default();
            let mut room = Room {
                ui,
                picker: &mut crate::picker::Picker::default(),
                jobs: &mut jobs,
                previews: &mut previews,
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
    fn renders_a_document_with_one_of_each_shape_and_some_text() {
        let mut doc = Document::blank(A4, 1);
        doc.add(Item {
            id: 0,
            page: 1,
            x_mm: 20.0,
            y_mm: 30.0,
            text: "Sign here".to_string(),
            size_pt: 11.0,
            font: "Helvetica".to_string(),
            width_mm: None,
            rotation_deg: 0.0,
            colour: "#000000".to_string(),
            leading: 1.2,
        })
        .unwrap();
        doc.draw(Shape {
            id: 0,
            page: 1,
            kind: ShapeKind::Line {
                x1_mm: 10.0,
                y1_mm: 10.0,
                x2_mm: 190.0,
                y2_mm: 10.0,
            },
            stroke: Some("black".to_string()),
            fill: None,
            width_mm: 0.5,
            dash_mm: None,
        })
        .unwrap();
        doc.draw(Shape {
            id: 0,
            page: 1,
            kind: ShapeKind::Rect {
                x_mm: 20.0,
                y_mm: 40.0,
                width_mm: 60.0,
                height_mm: 30.0,
                radius_mm: 3.0,
            },
            stroke: Some("blue".to_string()),
            fill: Some("lightgrey".to_string()),
            width_mm: 0.5,
            dash_mm: Some((2.0, 1.0)),
        })
        .unwrap();
        doc.draw(Shape {
            id: 0,
            page: 1,
            kind: ShapeKind::Ellipse {
                x_mm: 100.0,
                y_mm: 100.0,
                radius_x_mm: 20.0,
                radius_y_mm: 10.0,
            },
            stroke: Some("red".to_string()),
            fill: None,
            width_mm: 0.5,
            dash_mm: Some((1.0, 1.0)),
        })
        .unwrap();
        doc.draw(Shape {
            id: 0,
            page: 1,
            kind: ShapeKind::Path {
                points: vec![(20.0, 150.0), (60.0, 180.0), (100.0, 150.0)],
                closed: true,
            },
            stroke: Some("green".to_string()),
            fill: Some("yellow".to_string()),
            width_mm: 0.5,
            dash_mm: None,
        })
        .unwrap();

        let mut state = State {
            path: Some(temp_path("all-shapes")),
            doc: Some(doc),
            ..State::default()
        };
        // Every tool changes what the canvas draws as a live preview, so
        // cycle through all four rather than trusting one to stand for all.
        for tool in Tool::ALL {
            state.tool = tool;
            render(&mut state);
        }
        assert!(state.doc.is_some(), "a document that renders cleanly stays open");
        let _ = std::fs::remove_file(state.path.take().unwrap());
    }

    #[test]
    fn shape_kind_builds_a_line_from_two_points() {
        let draft = Draft::TwoPoint {
            start: (10.0, 20.0),
            now: (30.0, 40.0),
        };
        let kind = shape_kind(Tool::Line, false, 0.0, &draft).unwrap();
        assert_eq!(
            kind,
            ShapeKind::Line {
                x1_mm: 10.0,
                y1_mm: 20.0,
                x2_mm: 30.0,
                y2_mm: 40.0
            }
        );
    }

    #[test]
    fn shape_kind_normalises_a_box_dragged_from_any_corner() {
        // Dragged up and to the left, from (50,60) to (10,20) — the box must
        // still come out with its top-left corner and a positive size.
        let draft = Draft::TwoPoint {
            start: (50.0, 60.0),
            now: (10.0, 20.0),
        };
        let kind = shape_kind(Tool::Box, false, 0.0, &draft).unwrap();
        assert_eq!(
            kind,
            ShapeKind::Rect {
                x_mm: 10.0,
                y_mm: 20.0,
                width_mm: 40.0,
                height_mm: 40.0,
                radius_mm: 0.0
            }
        );
    }

    #[test]
    fn shape_kind_builds_an_ellipse_from_centre_and_radius() {
        let draft = Draft::TwoPoint {
            start: (100.0, 100.0),
            now: (120.0, 90.0),
        };
        let kind = shape_kind(Tool::Circle, false, 0.0, &draft).unwrap();
        assert_eq!(
            kind,
            ShapeKind::Ellipse {
                x_mm: 100.0,
                y_mm: 100.0,
                radius_x_mm: 20.0,
                radius_y_mm: 10.0
            }
        );
    }

    #[test]
    fn shape_kind_needs_at_least_two_points_for_a_path() {
        let one_point = Draft::Path {
            points: vec![(1.0, 1.0)],
        };
        assert!(shape_kind(Tool::Path, false, 0.0, &one_point).is_none());

        let two_points = Draft::Path {
            points: vec![(1.0, 1.0), (2.0, 2.0)],
        };
        assert!(shape_kind(Tool::Path, true, 0.0, &two_points).is_some());
    }

    #[test]
    fn shape_kind_is_tool_specific_even_for_the_same_drag() {
        // A two-point drag means nothing to the path tool — it only ever
        // reads the point trail, never a start/now pair — regardless of how
        // far apart the two points are.
        let draft = Draft::TwoPoint {
            start: (0.0, 0.0),
            now: (50.0, 50.0),
        };
        assert!(shape_kind(Tool::Path, false, 0.0, &draft).is_none());
    }

    #[test]
    fn big_enough_rejects_a_stray_click_but_accepts_a_real_drag() {
        assert!(!big_enough(&ShapeKind::Line {
            x1_mm: 5.0,
            y1_mm: 5.0,
            x2_mm: 5.1,
            y2_mm: 5.1
        }));
        assert!(big_enough(&ShapeKind::Line {
            x1_mm: 5.0,
            y1_mm: 5.0,
            x2_mm: 20.0,
            y2_mm: 5.0
        }));
        assert!(!big_enough(&ShapeKind::Rect {
            x_mm: 0.0,
            y_mm: 0.0,
            width_mm: 0.2,
            height_mm: 0.2,
            radius_mm: 0.0
        }));
    }

    #[test]
    fn drawing_problem_requires_an_outline_or_a_fill() {
        let mut state = State {
            outline: false,
            fill: None,
            ..State::default()
        };
        assert!(drawing_problem(&state).is_some());

        state.fill = Some("red".to_string());
        assert!(drawing_problem(&state).is_none());

        state.fill = None;
        state.outline = true;
        assert!(drawing_problem(&state).is_none());
    }

    #[test]
    fn drawing_problem_catches_an_unknown_stroke_or_fill_colour() {
        let mut state = State {
            stroke: "not-a-colour".to_string(),
            ..State::default()
        };
        assert!(drawing_problem(&state).is_some());

        state.stroke = "black".to_string();
        state.fill = Some("also-not-a-colour".to_string());
        assert!(drawing_problem(&state).is_some());
    }

    #[test]
    fn commit_ignores_a_drag_too_small_to_be_real() {
        let mut doc = Document::blank(A4, 1);
        let mut state = State::default();
        let tiny = Draft::TwoPoint {
            start: (10.0, 10.0),
            now: (10.05, 10.02),
        };
        commit(&mut state, &mut doc, &tiny);
        assert!(doc.shapes.is_empty(), "a drag that never really moved draws nothing");
    }

    #[test]
    fn commit_adds_a_real_drag_to_the_document() {
        let mut doc = Document::blank(A4, 1);
        let mut state = State::default();
        let real = Draft::TwoPoint {
            start: (10.0, 10.0),
            now: (60.0, 10.0),
        };
        commit(&mut state, &mut doc, &real);
        assert_eq!(doc.shapes.len(), 1);
    }
}
