//! The window: a list of things to do down the side, the chosen one in the
//! middle, and a line along the bottom saying what is happening.

use eframe::egui;

use super::job::Jobs;
use super::picker;
use super::preview::Previews;
use super::screens::{self, Room, Screen};
use super::theme;

pub struct Onionskin {
    screen: Screen,
    jobs: Jobs,
    previews: Previews,
    picker: picker::Picker,

    /// Files dropped on the window, held until a control claims them.
    dropped: Vec<std::path::PathBuf>,
    /// Something to say along the bottom, and when it was said.
    said: Option<(String, f64)>,

    compare: screens::compare::State,
    scan: screens::scan::State,
    document: screens::document::State,
    draw: screens::draw::State,
    read: screens::read::State,
    verify: screens::verify::State,
    blanks: screens::blanks::State,
    proof: screens::proof::State,
    merge: screens::merge::State,
    join: screens::join::State,
    fits: screens::fits::State,
    stack: screens::stack::State,
    correct: screens::correct::State,
    cover: screens::cover::State,
    watermark: screens::watermark::State,
    barcode: screens::barcode::State,
    back: screens::back::State,
    harvest: screens::harvest::State,
    batch: screens::batch::State,
    labels: screens::labels::State,
    jobs_screen: screens::jobs::State,
    watch: screens::watch::State,
    history: screens::history::State,
    devices: screens::devices::State,
    calibrate: screens::calibrate::State,
    doctor: screens::doctor::State,
}

impl Onionskin {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Onionskin {
        theme::apply(&cc.egui_ctx);
        // Where somebody was last time. Opening on the screen they left is a
        // small thing that makes the program feel like it was waiting.
        let screen = onionskin::settings::load()
            .last_screen
            .as_deref()
            .and_then(Screen::from_key)
            .unwrap_or(Screen::Compare);
        Onionskin {
            screen,
            jobs: Jobs::new(&cc.egui_ctx),
            previews: Previews::default(),
            picker: picker::Picker::default(),
            dropped: Vec::new(),
            said: None,
            compare: Default::default(),
            join: Default::default(),
            fits: Default::default(),
            stack: Default::default(),
            correct: Default::default(),
            cover: Default::default(),
            watermark: Default::default(),
            barcode: Default::default(),
            back: Default::default(),
            harvest: Default::default(),
            batch: Default::default(),
            scan: Default::default(),
            document: Default::default(),
            draw: Default::default(),
            read: Default::default(),
            verify: Default::default(),
            blanks: Default::default(),
            proof: Default::default(),
            merge: Default::default(),
            labels: Default::default(),
            jobs_screen: Default::default(),
            watch: Default::default(),
            history: Default::default(),
            devices: Default::default(),
            calibrate: Default::default(),
            doctor: Default::default(),
        }
    }
}

impl eframe::App for Onionskin {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.jobs.poll();
        self.take_dropped_files(ui.ctx());

        // Panels are nested outermost-first, and the central one comes last.
        egui::Panel::left("what-to-do")
            .exact_size(262.0)
            .resizable(false)
            .show(ui, |ui| self.sidebar(ui));

        egui::Panel::bottom("what-is-happening")
            .resizable(false)
            .show(ui, |ui| self.status(ui));

        egui::CentralPanel::default().show(ui, |ui| {
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    // Long lines of text are hard to read. The page stays a
                    // comfortable width however wide the window is opened.
                    ui.set_max_width(740.0);
                    let mut room = Room {
                        picker: &mut self.picker,
                        jobs: &mut self.jobs,
                        previews: &mut self.previews,
                        dropped: &mut self.dropped,
                        ui,
                    };
                    match self.screen {
                        Screen::Compare => screens::compare::show(&mut self.compare, &mut room),
                        Screen::Scan => screens::scan::show(&mut self.scan, &mut room),
                        Screen::Document => screens::document::show(&mut self.document, &mut room),
                        Screen::Draw => screens::draw::show(&mut self.draw, &mut room),
                        Screen::Read => screens::read::show(&mut self.read, &mut room),
                        Screen::Verify => screens::verify::show(&mut self.verify, &mut room),
                        Screen::Blanks => screens::blanks::show(&mut self.blanks, &mut room),
                        Screen::Proof => screens::proof::show(&mut self.proof, &mut room),
                        Screen::Merge => screens::merge::show(&mut self.merge, &mut room),
                        Screen::Join => screens::join::show(&mut self.join, &mut room),
                        Screen::Fits => screens::fits::show(&mut self.fits, &mut room),
                        Screen::Stack => screens::stack::show(&mut self.stack, &mut room),
                        Screen::Correct => screens::correct::show(&mut self.correct, &mut room),
                        Screen::Cover => screens::cover::show(&mut self.cover, &mut room),
                        Screen::Watermark => {
                            screens::watermark::show(&mut self.watermark, &mut room)
                        }
                        Screen::Barcode => screens::barcode::show(&mut self.barcode, &mut room),
                        Screen::Back => screens::back::show(&mut self.back, &mut room),
                        Screen::Harvest => screens::harvest::show(&mut self.harvest, &mut room),
                        Screen::Batch => screens::batch::show(&mut self.batch, &mut room),
                        Screen::Labels => screens::labels::show(&mut self.labels, &mut room),
                        Screen::Jobs => screens::jobs::show(&mut self.jobs_screen, &mut room),
                        Screen::Watch => screens::watch::show(&mut self.watch, &mut room),
                        Screen::History => screens::history::show(&mut self.history, &mut room),
                        Screen::Devices => screens::devices::show(&mut self.devices, &mut room),
                        Screen::Calibrate => {
                            screens::calibrate::show(&mut self.calibrate, &mut room)
                        }
                        Screen::Doctor => screens::doctor::show(&mut self.doctor, &mut room),
                    }
                });
        });

        // Drawn last and over everything, because it is a question that has to
        // be answered before anything else can be got on with.
        self.picker.show(ui.ctx());

        // A file nothing on this screen could use is dropped — but silently
        // dropping it after saying "drop it to use it here" is a small lie, so
        // it is said out loud along the bottom instead.
        if !self.dropped.is_empty() {
            let names: Vec<String> = self
                .dropped
                .iter()
                .filter_map(|path| path.file_name())
                .map(|name| name.to_string_lossy().into_owned())
                .collect();
            self.said = Some((
                format!(
                    "Nothing on this screen can use {}",
                    if names.is_empty() {
                        "that".to_string()
                    } else {
                        names.join(", ")
                    }
                ),
                ui.ctx().input(|input| input.time),
            ));
            // Neither of these happens by itself: the window only redraws when
            // something arrives, and nothing arrives to take a message away.
            ui.ctx().request_repaint();
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_secs(SAY_FOR_SECONDS as u64 + 1));
        }
        self.dropped.clear();
    }
}

/// How long a passing message stays along the bottom. Long enough to read
/// twice, short enough not to be mistaken for the state of the program.
const SAY_FOR_SECONDS: f64 = 6.0;

impl Onionskin {
    /// Whatever is worth saying along the bottom just now, if anything.
    fn something_to_say(&self, ctx: &egui::Context) -> Option<String> {
        let (said, at) = self.said.as_ref()?;
        let now = ctx.input(|input| input.time);
        (now - at < SAY_FOR_SECONDS).then(|| said.clone())
    }

    /// Collect anything dropped on the window, and say so while it is hovering.
    ///
    /// Dragging a file onto a program is how most people would rather open one,
    /// and it costs nothing here: the window is already watching for it, and a
    /// dropped path is the same thing the file browser hands back.
    fn take_dropped_files(&mut self, ctx: &egui::Context) {
        let (dropped, hovering) = ctx.input(|input| {
            (
                input
                    .raw
                    .dropped_files
                    .iter()
                    .filter_map(|file| file.path.clone())
                    .collect::<Vec<_>>(),
                input.raw.hovered_files.len(),
            )
        });
        if !dropped.is_empty() {
            if let Some(first) = dropped.first() {
                onionskin::settings::remember_folder(first);
            }
            self.dropped = dropped;
        }

        // While something is over the window, say what will happen. Silence
        // here reads as "this program does not take dropped files".
        if hovering > 0 {
            egui::Area::new(egui::Id::new("drop-hint"))
                .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 24.0))
                .order(egui::Order::Tooltip)
                .show(ctx, |ui| {
                    egui::Frame::popup(ui.style()).show(ui, |ui| {
                        ui.label(
                            egui::RichText::new(if hovering == 1 {
                                "Drop it to use it here".to_string()
                            } else {
                                format!("Drop {hovering} files to use them here")
                            })
                            .strong(),
                        );
                    });
                });
        }
    }

    /// The list of screens down the left, and the two lines at the bottom.
    ///
    /// The order here is load-bearing. The footer is claimed first, as a panel
    /// of its own, and the list scrolls in whatever is left — because a
    /// scrolling area takes all the height there is, and anything drawn after
    /// it is pushed off the end of the window.
    ///
    /// There was no scrolling area at all until now, and there are twenty-odd
    /// screens each with a heading and a wrapped sentence under it. On a
    /// laptop, that is a list about half as tall again as the window it is in:
    /// the ones at the bottom — measuring the printer, saved jobs, what works
    /// on this machine — could not be clicked, and there was nothing on the
    /// screen to suggest they were there. A whole third of the program was
    /// unreachable from the window, and nobody would have known to look.
    fn sidebar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("who-this-is")
            .resizable(false)
            .show(ui, |ui| {
                ui.add_space(10.0);
                ui.label(
                    egui::RichText::new(
                        "Nothing leaves this machine.\nIt speaks to printers, and to nothing else.",
                    )
                    .small()
                    .weak(),
                );
                ui.add_space(6.0);
                ui.label(
                    egui::RichText::new(concat!("Onionskin ", env!("CARGO_PKG_VERSION")))
                        .small()
                        .weak(),
                );
                ui.add_space(6.0);
            });

        ui.add_space(12.0);
        ui.heading("Onionskin");
        ui.label(
            egui::RichText::new("Add words to a page that is already printed.")
                .small()
                .weak(),
        );
        ui.add_space(14.0);

        // Written down rather than acted on inside the loop, because changing
        // the screen borrows `self` and the loop is already holding it.
        let mut chose: Option<Screen> = None;
        let here = self.screen;
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for screen in Screen::ALL {
                    let chosen = here == *screen;
                    let response =
                        ui.selectable_label(chosen, egui::RichText::new(screen.name()).strong());
                    if response.clicked() {
                        chose = Some(*screen);
                    }
                    // The sentence beneath is what lets somebody pick the right
                    // screen without opening all six to find out what they do.
                    // Wrapped, or the longer ones run off the edge of the panel
                    // and lose their last word — which is usually the one that
                    // distinguishes them.
                    ui.indent(screen.name(), |ui| {
                        ui.add(
                            egui::Label::new(egui::RichText::new(screen.lede()).small().weak())
                                .wrap(),
                        );
                    });
                    ui.add_space(4.0);
                }
            });

        if let Some(screen) = chose {
            self.screen = screen;
            let key = screen.key();
            onionskin::settings::remember(|settings| settings.last_screen = Some(key.to_string()));
        }
    }

    fn status(&mut self, ui: &mut egui::Ui) {
        ui.add_space(3.0);
        ui.horizontal(|ui| match self.jobs.doing() {
            Some((what, progress, elapsed)) => {
                ui.spinner();
                ui.label(egui::RichText::new(what).strong());
                ui.label(&progress.doing);
                // Not decoration: the seconds ticking up are what tell somebody
                // the program is working rather than stuck.
                ui.label(
                    egui::RichText::new(format!("{:.0}s", elapsed.as_secs_f32()))
                        .small()
                        .weak(),
                );
                if let Some(fraction) = progress.fraction {
                    ui.add(egui::ProgressBar::new(fraction).desired_width(150.0));
                }
            }
            None => match self.something_to_say(ui.ctx()) {
                Some(said) => {
                    ui.label(egui::RichText::new(said).small().color(theme::CAUTION));
                }
                None => {
                    ui.label(egui::RichText::new("Ready").small().weak());
                }
            },
        });
        ui.add_space(3.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The list of screens has to scroll, and there is no way to find out from
    /// a test whether it does.
    ///
    /// egui draws to a window, and Onionskin has no harness that opens one —
    /// getting one means another dependency, which this program does not take.
    /// So the check is on the source, which is crude, and the alternative is
    /// nothing at all. That is a bad trade only if the fault is easy to spot
    /// by other means, and this one is the opposite: the program compiles,
    /// starts, and looks perfectly well, and a third of it simply cannot be
    /// reached. Nobody reports a screen they never knew was there.
    ///
    /// The arithmetic below is the reason, kept next to the rule so that if
    /// the sidebar ever becomes short enough not to need scrolling, this fails
    /// and says so rather than quietly enforcing a habit.
    #[test]
    fn the_list_of_screens_can_be_scrolled() {
        // A heading, a wrapped sentence under it, and the space between: this
        // is a floor, not an estimate, so the conclusion holds even if the
        // rows are drawn tighter than they are today.
        const AT_LEAST_PER_SCREEN: f32 = 38.0;
        // The window's own minimum, less the status strip along the bottom and
        // the heading at the top of the panel. Generous, for the same reason.
        const ROOM_AT_MOST: f32 = 620.0 - 30.0 - 60.0;

        let needed = Screen::ALL.len() as f32 * AT_LEAST_PER_SCREEN;
        assert!(
            needed > ROOM_AT_MOST,
            "{} screens now fit without scrolling ({needed} px in {ROOM_AT_MOST} px), \
             so this test is no longer about anything real",
            Screen::ALL.len()
        );

        let source = include_str!("app.rs");
        let from = source
            .find("fn sidebar(")
            .expect("the sidebar is drawn by a function called sidebar");
        let to = source[from..]
            .find("\n    fn ")
            .map(|at| from + at)
            .unwrap_or(source.len());
        let drawing = &source[from..to];
        assert!(
            drawing.contains("ScrollArea"),
            "the sidebar draws {} screens and does not scroll, so the ones at \
             the bottom cannot be reached",
            Screen::ALL.len()
        );
        // And the footer is a panel of its own rather than something drawn
        // after the scrolling area, which would put it past the bottom edge.
        let scrolls_at = drawing.find("ScrollArea").expect("just checked");
        let footer_at = drawing
            .find("Panel::bottom")
            .expect("the footer is claimed as a panel before the list scrolls");
        assert!(
            footer_at < scrolls_at,
            "the footer is drawn after the scrolling area, which pushes it off \
             the bottom of the window"
        );
    }
}
