//! Not built yet.
//!
//! The screen is named and reachable so that the shape of the program is
//! honest: somebody can see what it will do and that it does not do it yet,
//! which is better than a menu that hides half the application.

use super::Room;
use crate::widgets;

#[derive(Default)]
pub struct State;

pub fn show(_state: &mut State, room: &mut Room) {
    let screen = super::Screen::Draw;
    widgets::title(room.ui, screen.name(), screen.lede());
    widgets::caution(
        room.ui,
        "This screen is not built yet.\n\nEverything it will do can be done          from the command line today — run 'onionskin --help' to see how.",
    );
}
