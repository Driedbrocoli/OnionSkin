//! Onionskin, as a window.
//!
//! A separate program from the command line one, and not for tidiness: Windows
//! decides at link time whether an executable owns a console. A window built as
//! a console program flashes a black box behind itself on every launch; a
//! console program built as a window prints to nowhere. One file cannot be both,
//! so there are two, and they share every line of the work underneath.
//!
//! There is no web view here. egui draws each widget itself onto an OpenGL
//! surface, so the program remains one file that runs on a machine with nothing
//! installed — which is the whole promise, and a bundled browser would break it
//! twice over: a hundred megabytes to download, and a network stack sitting
//! inside a program that says it never uses the network.

// No console window behind the app on Windows. Left on for a debug build,
// because that is where being able to see a panic is worth more.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
#[allow(dead_code)]
mod job;
// Some of this is used only by screens still being built.
#[allow(dead_code)]
mod preview;
mod screens;
#[allow(dead_code)]
mod theme;
#[allow(dead_code)]
mod widgets;

use eframe::egui;

/// The window's smallest useful size. Below this the page preview stops being
/// a preview and starts being a postage stamp.
const MINIMUM: [f32; 2] = [900.0, 620.0];
const STARTING: [f32; 2] = [1180.0, 820.0];

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size(STARTING)
            .with_min_inner_size(MINIMUM)
            .with_title("Onionskin")
            .with_app_id("onionskin")
            .with_icon(icon()),
        ..Default::default()
    };

    eframe::run_native(
        "Onionskin",
        options,
        Box::new(|cc| Ok(Box::new(app::Onionskin::new(cc)))),
    )
}

/// The window and taskbar icon, drawn here rather than loaded from a file.
///
/// A separate icon file is one more thing that can fail to be installed, and
/// then the program runs with the operating system's grey default and looks
/// broken. Sixty-four squares of arithmetic cannot go missing.
fn icon() -> egui::IconData {
    const SIZE: usize = 64;
    let mut rgba = Vec::with_capacity(SIZE * SIZE * 4);

    for y in 0..SIZE {
        for x in 0..SIZE {
            let (fx, fy) = (x as f32 / SIZE as f32, y as f32 / SIZE as f32);
            // A sheet of paper, and a second sheet laid over it and offset —
            // which is what the name means and what the program does.
            let lower = (0.16..0.80).contains(&fx) && (0.20..0.84).contains(&fy);
            let upper = (0.28..0.92).contains(&fx) && (0.10..0.74).contains(&fy);

            let pixel: [u8; 4] = if upper {
                // The delta: nearly clear, with one line of new ink on it.
                let ink = (0.34..0.84).contains(&fx)
                    && ((0.24..0.275).contains(&fy) || (0.34..0.375).contains(&fy));
                if ink {
                    [0xd6, 0x33, 0x33, 0xff]
                } else {
                    [0xff, 0xff, 0xff, 0xe6]
                }
            } else if lower {
                let ink = (0.22..0.72).contains(&fx)
                    && ((0.46..0.49).contains(&fy)
                        || (0.56..0.59).contains(&fy)
                        || (0.66..0.69).contains(&fy));
                if ink {
                    [0x33, 0x33, 0x33, 0xff]
                } else {
                    [0xf4, 0xf1, 0xea, 0xff]
                }
            } else {
                [0, 0, 0, 0]
            };
            rgba.extend_from_slice(&pixel);
        }
    }

    egui::IconData {
        rgba,
        width: SIZE as u32,
        height: SIZE as u32,
    }
}
