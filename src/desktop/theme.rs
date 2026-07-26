//! How the window looks.
//!
//! Restrained on purpose. This is a program about putting ink exactly where it
//! belongs, and a window full of gradients and animation makes a person less
//! sure of that, not more. The one place colour is spent is where it carries
//! meaning: the ink already on the paper in grey, the ink about to be added in
//! red, a refusal in red, a warning in amber.

use eframe::egui;

/// Ink already on the sheet, in a preview.
pub const EXISTING: egui::Color32 = egui::Color32::from_rgb(0x8a, 0x8a, 0x8a);
/// Ink the delta will add.
pub const ADDED: egui::Color32 = egui::Color32::from_rgb(0xd6, 0x33, 0x33);
/// Something went wrong, or was refused.
pub const REFUSED: egui::Color32 = egui::Color32::from_rgb(0xc0, 0x28, 0x28);
/// Worth reading, but nothing is broken.
pub const CAUTION: egui::Color32 = egui::Color32::from_rgb(0xb0, 0x76, 0x00);
/// It worked.
pub const DONE: egui::Color32 = egui::Color32::from_rgb(0x1f, 0x7a, 0x3d);

/// The paper in a preview, and the desk it sits on.
pub const PAPER: egui::Color32 = egui::Color32::from_rgb(0xff, 0xff, 0xff);
pub const DESK_LIGHT: egui::Color32 = egui::Color32::from_rgb(0xd8, 0xd6, 0xd2);
pub const DESK_DARK: egui::Color32 = egui::Color32::from_rgb(0x2a, 0x2a, 0x2c);

pub fn desk(ui: &egui::Ui) -> egui::Color32 {
    if ui.visuals().dark_mode {
        DESK_DARK
    } else {
        DESK_LIGHT
    }
}

/// Set the window's look once, at startup.
///
/// Applied to both the light and the dark style, because egui keeps one of
/// each and follows the desktop's setting. Setting only the current one leaves
/// somebody who switches their machine to dark mode with a window that
/// suddenly has different type sizes in it.
pub fn apply(ctx: &egui::Context) {
    use egui::FontFamily::{Monospace, Proportional};
    use egui::{FontId, TextStyle};

    ctx.all_styles_mut(|style| {
        // Bigger than egui's default, which is sized for a developer tool on a
        // large monitor. This is for somebody signing an invoice, possibly on
        // a laptop, possibly not twenty-five years old.
        style.text_styles = [
            (TextStyle::Heading, FontId::new(22.0, Proportional)),
            (TextStyle::Body, FontId::new(15.0, Proportional)),
            (TextStyle::Button, FontId::new(15.0, Proportional)),
            (TextStyle::Small, FontId::new(13.0, Proportional)),
            (TextStyle::Monospace, FontId::new(13.5, Monospace)),
        ]
        .into();

        // Room to breathe, and targets big enough to hit.
        style.spacing.item_spacing = egui::vec2(10.0, 9.0);
        style.spacing.button_padding = egui::vec2(12.0, 7.0);
        style.spacing.interact_size.y = 30.0;
        style.spacing.indent = 20.0;

        // A visible focus outline. Somebody working by keyboard has to be able
        // to see where they are, and the default is nearly invisible on a pale
        // background.
        style.visuals.selection.stroke.width = 2.0;
        style.visuals.widgets.hovered.expansion = 1.0;
    });
}
