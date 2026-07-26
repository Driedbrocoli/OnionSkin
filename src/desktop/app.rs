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

    compare: screens::compare::State,
    scan: screens::scan::State,
    document: screens::document::State,
    draw: screens::draw::State,
    read: screens::read::State,
    devices: screens::devices::State,
    calibrate: screens::calibrate::State,
    doctor: screens::doctor::State,
}

impl Onionskin {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Onionskin {
        theme::apply(&cc.egui_ctx);
        Onionskin {
            screen: Screen::Compare,
            jobs: Jobs::new(&cc.egui_ctx),
            previews: Previews::default(),
            picker: picker::Picker::default(),
            compare: Default::default(),
            scan: Default::default(),
            document: Default::default(),
            draw: Default::default(),
            read: Default::default(),
            devices: Default::default(),
            calibrate: Default::default(),
            doctor: Default::default(),
        }
    }
}

impl eframe::App for Onionskin {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.jobs.poll();

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
                        ui,
                    };
                    match self.screen {
                        Screen::Compare => screens::compare::show(&mut self.compare, &mut room),
                        Screen::Scan => screens::scan::show(&mut self.scan, &mut room),
                        Screen::Document => {
                            screens::document::show(&mut self.document, &mut room)
                        }
                        Screen::Draw => screens::draw::show(&mut self.draw, &mut room),
                        Screen::Read => screens::read::show(&mut self.read, &mut room),
                        Screen::Devices => {
                            screens::devices::show(&mut self.devices, &mut room)
                        }
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
    }
}

impl Onionskin {
    fn sidebar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(12.0);
        ui.heading("Onionskin");
        ui.label(
            egui::RichText::new("Add words to a page that is already printed.")
                .small()
                .weak(),
        );
        ui.add_space(14.0);

        for screen in Screen::ALL {
            let chosen = self.screen == *screen;
            let response = ui.selectable_label(chosen, egui::RichText::new(screen.name()).strong());
            if response.clicked() {
                self.screen = *screen;
            }
            // The sentence beneath is what lets somebody pick the right screen
            // without opening all six to find out what they do. Wrapped, or
            // the longer ones run off the edge of the panel and lose their
            // last word — which is usually the one that distinguishes them.
            ui.indent(screen.name(), |ui| {
                ui.add(
                    egui::Label::new(egui::RichText::new(screen.lede()).small().weak()).wrap(),
                );
            });
            ui.add_space(4.0);
        }

        ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
            ui.add_space(10.0);
            ui.label(
                egui::RichText::new("Never uses the network.\nEverything stays on this machine.")
                    .small()
                    .weak(),
            );
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new(concat!("Onionskin ", env!("CARGO_PKG_VERSION")))
                    .small()
                    .weak(),
            );
        });
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
            None => {
                ui.label(egui::RichText::new("Ready").small().weak());
            }
        });
        ui.add_space(3.0);
    }
}
