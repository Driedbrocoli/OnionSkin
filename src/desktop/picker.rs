//! Choosing a file, without asking the operating system.
//!
//! The obvious way is a native dialog, and Onionskin used one until it turned
//! out to bring the program down. On Linux a native dialog is not part of the
//! window system at all: it is a request over D-Bus to a *desktop portal*, a
//! separate service that a full desktop runs and a minimal one does not. Where
//! it is missing — a tiling window manager, a stripped virtual machine, a
//! container — the program has no dialog and, in the version that shipped
//! first, no window either: it panicked and vanished the moment somebody
//! pressed Choose.
//!
//! So the file browser is drawn here, in the same window as everything else.
//! It costs a few hundred lines and it removes a dependency, a system service,
//! and an entire way for the program to disappear. It also behaves identically
//! on all three platforms, which the native dialogs emphatically do not.
//!
//! Clicking through it works, but it is not the only way in: the rest of this
//! file is what makes it drivable from the keyboard too — arrows and paging to
//! move, letters to jump to a name, a path typed or pasted straight in. None
//! of it needs a crate that is not already here.

use std::path::{Component, Path, PathBuf};

use eframe::egui;

/// Which control asked for a file, so the answer goes back to the right one.
pub type Asked = egui::Id;

/// Opening a file, or choosing where to write one.
#[derive(PartialEq, Eq, Clone, Copy)]
enum Purpose {
    Open,
    Save,
}

#[derive(Default)]
pub struct Picker {
    open: Option<Browsing>,
    /// The path chosen, waiting to be collected by whoever asked.
    answered: Option<(Asked, PathBuf)>,
}

struct Browsing {
    who: Asked,
    purpose: Purpose,
    title: String,
    /// The folder being looked at.
    at: PathBuf,
    /// Extensions worth showing. Empty means everything.
    kinds: Vec<String>,
    /// What the file will be called, when saving.
    name: String,
    /// The row the keyboard is on, so Enter and the arrows work.
    chosen: Option<usize>,
    /// Set when a name would overwrite something, until it is confirmed.
    confirm_overwrite: bool,
    error: Option<String>,
    /// The text sitting in the path box at the top.
    ///
    /// Kept apart from `at` rather than bound straight to it, so that
    /// clicking a breadcrumb mid-sentence does not yank a half-typed path
    /// out from under somebody, and so that a path which turns out not to
    /// exist leaves what was typed on screen instead of wiping it. It only
    /// ever changes in the two directions described on `navigate_to` and the
    /// path box's own Enter handling.
    path_box: String,
    /// Why the path box's last errand did not work, shown quietly beside it
    /// rather than in place of what was typed.
    path_problem: Option<String>,
    /// What has been typed so far for jump-to-first-match.
    type_ahead: String,
    /// The `ctx.input(|i| i.time)` reading when the last letter landed in
    /// `type_ahead`, so a pause of about a second can start the next search
    /// fresh. See `prefix_expired`.
    type_ahead_at: f64,
    /// Whether the name field still owes itself the keyboard this time the
    /// dialog is open.
    ///
    /// Set once, in `begin`, and cleared the first frame `show` actually
    /// claims it — a plain `bool` rather than doing it unconditionally every
    /// frame, because the latter would also snatch the keyboard back the
    /// moment somebody clicked away from the name field on purpose.
    focus_name: bool,
}

/// One row in the list.
struct Row {
    name: String,
    path: PathBuf,
    is_dir: bool,
}

/// Where a typed or pasted path leads.
#[derive(Debug, PartialEq)]
enum Destination {
    /// A folder to show.
    Folder(PathBuf),
    /// A file to show already selected, inside the folder that holds it.
    File { folder: PathBuf, file: PathBuf },
}

/// Where pressing Enter on the row that is selected leads.
#[derive(Debug, PartialEq)]
enum Entered {
    /// Go into this folder.
    Folder(PathBuf),
    /// This file was the row's whole answer; what to do with it depends on
    /// why the dialog is open, and is decided where this is matched.
    File(PathBuf),
}

/// What answering Save should do.
#[derive(Debug, PartialEq)]
enum SaveOutcome {
    /// Nothing is in the way, or it was and this is the second attempt at
    /// the same name: here is where to write.
    Answer(PathBuf),
    /// Something already exists at that name and this is the first attempt.
    /// Ask again before it is overwritten.
    NeedsConfirmation,
}

/// A keyboard nudge to the selection: what Up, Down, Home, End, Page Up and
/// Page Down each ask the list to do.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Step {
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
}

impl Picker {
    /// Ask for a file to open.
    pub fn open(&mut self, who: Asked, title: &str, kinds: &[&str], start: Option<&Path>) {
        self.begin(who, Purpose::Open, title, kinds, start, "");
    }

    /// Ask where to write one.
    pub fn save(
        &mut self,
        who: Asked,
        title: &str,
        kinds: &[&str],
        start: Option<&Path>,
        suggested: &str,
    ) {
        self.begin(who, Purpose::Save, title, kinds, start, suggested);
    }

    fn begin(
        &mut self,
        who: Asked,
        purpose: Purpose,
        title: &str,
        kinds: &[&str],
        start: Option<&Path>,
        suggested: &str,
    ) {
        let at = start
            .map(|p| {
                if p.is_dir() {
                    p.to_path_buf()
                } else {
                    p.parent().unwrap_or(Path::new(".")).to_path_buf()
                }
            })
            .filter(|p| p.is_dir())
            .unwrap_or_else(somewhere_sensible);

        self.open = Some(Browsing {
            who,
            purpose,
            title: title.to_string(),
            path_box: at.to_string_lossy().into_owned(),
            at,
            kinds: kinds.iter().map(|k| k.to_string()).collect(),
            name: suggested.to_string(),
            chosen: None,
            confirm_overwrite: false,
            error: None,
            path_problem: None,
            type_ahead: String::new(),
            type_ahead_at: 0.0,
            // Only Save has anything to type a name into; Open has nothing
            // that should steal the keyboard away from the list.
            focus_name: purpose == Purpose::Save,
        });
    }

    /// The path chosen by `who`, once. Returns `None` on every other frame.
    pub fn taken(&mut self, who: Asked) -> Option<PathBuf> {
        match &self.answered {
            Some((asked, _)) if *asked == who => self.answered.take().map(|(_, path)| path),
            _ => None,
        }
    }

    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// Draw it, if anything asked. Called once a frame, after the screens.
    pub fn show(&mut self, ctx: &egui::Context) {
        let Some(browsing) = &mut self.open else {
            return;
        };

        let mut close = false;
        let mut answer: Option<PathBuf> = None;
        let home = onionskin::install::home();

        let modal = egui::Modal::new(egui::Id::new("file-picker")).show(ctx, |ui| {
            ui.set_width(660.0);
            ui.heading(&browsing.title);
            ui.add_space(6.0);

            // Somewhere to type or paste a path straight in, for whoever
            // already knows where their file is. It sits above the
            // breadcrumbs rather than replacing them, because the two are
            // good at opposite things: a breadcrumb is one click to go up
            // several folders at once, which typing cannot beat, while this
            // box can go straight to a folder nowhere near the breadcrumb
            // trail — from a clipboard, or from memory — which no amount of
            // clicking can beat. They do not fight because the box only ever
            // pushes a change out, on its own Enter, and only ever has one
            // pushed back in, by `navigate_to`; a breadcrumb click updates
            // what the box shows rather than the box deciding what a
            // breadcrumb click does.
            let mut select_after_navigate: Option<PathBuf> = None;
            ui.horizontal_wrapped(|ui| {
                ui.label("Go to");
                let response =
                    ui.add(egui::TextEdit::singleline(&mut browsing.path_box).desired_width(460.0));
                if response.changed() {
                    browsing.path_problem = None;
                }
                // A consuming check, not the usual "lost_focus() &&
                // key_pressed()" peek: this runs before the list's own Enter
                // handling further down in this same frame, and a `TextEdit`
                // losing focus does not remove the key press that caused it
                // from `ctx.input().events` — only `consume_key` does that.
                // Without this, the Enter that submits a path here would
                // still be sitting in the queue for the list to also act on
                // a few lines below, entering or opening whatever row
                // happens to be selected.
                if response.lost_focus()
                    && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter))
                {
                    match resolve_path_input(&browsing.path_box, &browsing.at, &home) {
                        Ok(Destination::Folder(at)) => browsing.navigate_to(at),
                        Ok(Destination::File { folder, file }) => {
                            browsing.navigate_to(folder);
                            select_after_navigate = Some(file);
                        }
                        Err(problem) => browsing.path_problem = Some(problem),
                    }
                }
                if let Some(problem) = &browsing.path_problem {
                    ui.label(egui::RichText::new(problem).small().weak());
                }
            });
            ui.add_space(4.0);

            // Where we are, and a way back up. The path is a row of buttons
            // rather than text, because going up three folders is otherwise a
            // matter of typing.
            ui.horizontal_wrapped(|ui| {
                for (label, path) in breadcrumbs(&browsing.at) {
                    if ui.small_button(label).clicked() {
                        browsing.navigate_to(path);
                    }
                }
            });
            ui.add_space(4.0);

            // The usual places, so nobody has to walk up to the root to get
            // to their own documents.
            ui.horizontal_wrapped(|ui| {
                for (label, path) in places() {
                    if ui.small_button(label).clicked() {
                        browsing.navigate_to(path);
                    }
                }
            });
            ui.separator();

            let rows = read_folder(&browsing.at, &browsing.kinds);

            // The path box asked to land on a particular file; now that the
            // folder it lives in has actually been read, find it in the same
            // listing a click would have found it in.
            if let Some(target) = select_after_navigate {
                if let Some((index, row)) =
                    rows.iter().enumerate().find(|(_, row)| row.path == target)
                {
                    browsing.chosen = Some(index);
                    browsing.confirm_overwrite = false;
                    browsing.name = row.name.clone();
                }
            }

            let mut scroll_to_selected = false;
            // Every one of these keys means something to a text field too:
            // Backspace deletes a character, the arrows move the cursor,
            // Home and End jump within the line, and a letter is just a
            // letter. egui does not remove an event from `ctx.input().events`
            // just because a focused `TextEdit` already used it, so without
            // this guard, typing a path or a Save name would also drive the
            // list underneath it. `text_edit_focused` asks specifically
            // whether the widget holding the keyboard right now is a
            // `TextEdit` — the coarser `egui_wants_keyboard_input` would
            // also be true for, say, a focused button, and there is no
            // reason for these keys to stand down for one of those.
            if !ctx.text_edit_focused() {
                let pressed_up = ctx.input(|i| i.key_pressed(egui::Key::ArrowUp));
                let pressed_down = ctx.input(|i| i.key_pressed(egui::Key::ArrowDown));
                let pressed_home = ctx.input(|i| i.key_pressed(egui::Key::Home));
                let pressed_end = ctx.input(|i| i.key_pressed(egui::Key::End));
                let pressed_page_up = ctx.input(|i| i.key_pressed(egui::Key::PageUp));
                let pressed_page_down = ctx.input(|i| i.key_pressed(egui::Key::PageDown));
                let pressed_backspace = ctx.input(|i| i.key_pressed(egui::Key::Backspace));
                let pressed_enter = ctx.input(|i| i.key_pressed(egui::Key::Enter));
                let (typed, now) = ctx.input(|i| {
                    let typed: String = i
                        .events
                        .iter()
                        .filter_map(|event| match event {
                            egui::Event::Text(text) => Some(text.as_str()),
                            _ => None,
                        })
                        .collect();
                    (typed, i.time)
                });

                let step = if pressed_up {
                    Some(Step::Up)
                } else if pressed_down {
                    Some(Step::Down)
                } else if pressed_home {
                    Some(Step::Home)
                } else if pressed_end {
                    Some(Step::End)
                } else if pressed_page_up {
                    Some(Step::PageUp)
                } else if pressed_page_down {
                    Some(Step::PageDown)
                } else {
                    None
                };
                if let Some(step) = step {
                    browsing.chosen = step_selection(browsing.chosen, rows.len(), step);
                    browsing.type_ahead.clear();
                    scroll_to_selected = true;
                }

                if pressed_backspace {
                    if let Some(parent) = browsing.at.parent() {
                        browsing.navigate_to(parent.to_path_buf());
                    }
                }

                if pressed_enter {
                    match enter_row(&rows, browsing.chosen) {
                        Some(Entered::Folder(path)) => browsing.navigate_to(path),
                        Some(Entered::File(path)) if browsing.purpose == Purpose::Open => {
                            answer = Some(path);
                        }
                        Some(Entered::File(_)) => {
                            // Saving is only ever armed by the name field's
                            // own Enter, below, never by the list's: arrowing
                            // past a file on the way to a folder further in
                            // must not be able to set up the overwrite
                            // prompt by accident. This just selects it, the
                            // same as a click would.
                            if let Some(row) = browsing.chosen.and_then(|i| rows.get(i)) {
                                browsing.name = row.name.clone();
                                browsing.confirm_overwrite = false;
                            }
                        }
                        None => {}
                    }
                }

                if !typed.is_empty() {
                    if prefix_expired(browsing.type_ahead_at, now) {
                        browsing.type_ahead.clear();
                    }
                    browsing.type_ahead.push_str(&typed);
                    browsing.type_ahead_at = now;
                    if let Some(index) = jump_to_prefix(&rows, &browsing.type_ahead) {
                        browsing.chosen = Some(index);
                        scroll_to_selected = true;
                    }
                } else if !browsing.type_ahead.is_empty()
                    && prefix_expired(browsing.type_ahead_at, now)
                {
                    browsing.type_ahead.clear();
                }
            }

            if !browsing.type_ahead.is_empty() {
                // Small and out of the way, but there for the asking: typing
                // and having the list jump around with no explanation of why
                // is the mysterious part, not this one line of text.
                ui.label(
                    egui::RichText::new(format!("Jumping to \"{}\"", browsing.type_ahead))
                        .small()
                        .weak(),
                );
            }

            egui::ScrollArea::vertical()
                .max_height(340.0)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    if rows.is_empty() {
                        ui.add_space(12.0);
                        ui.label(
                            egui::RichText::new(if browsing.kinds.is_empty() {
                                "This folder is empty."
                            } else {
                                "Nothing here that Onionskin can use."
                            })
                            .weak(),
                        );
                    }
                    for (index, row) in rows.iter().enumerate() {
                        let picked = browsing.chosen == Some(index);
                        // A trailing slash rather than an icon. A folder emoji
                        // needs a font that has it, and where that font is
                        // missing every row shows an empty box instead — which
                        // looks like a broken program rather than a folder.
                        let label = if row.is_dir {
                            format!("{}/", row.name)
                        } else {
                            format!("   {}", row.name)
                        };
                        let response = ui.selectable_label(
                            picked,
                            if row.is_dir {
                                egui::RichText::new(label).strong()
                            } else {
                                egui::RichText::new(label)
                            },
                        );

                        if picked && scroll_to_selected {
                            // Only on the frame the keyboard moved here.
                            // Doing this every frame would fight somebody
                            // scrolling the list by hand to look at
                            // something else while the selection stays put.
                            response.scroll_to_me(None);
                        }

                        // One click goes into a folder. A folder is never the
                        // answer here — only a file is — so there is nothing
                        // to be gained by making somebody select it first and
                        // then double-click, and double-click is the gesture
                        // people miss.
                        if response.clicked() {
                            if row.is_dir {
                                browsing.navigate_to(row.path.clone());
                            } else {
                                browsing.chosen = Some(index);
                                browsing.confirm_overwrite = false;
                                browsing.name = row.name.clone();
                            }
                        }
                        // A double click on a file takes it and closes, which
                        // is what the gesture means everywhere else.
                        if response.double_clicked()
                            && !row.is_dir
                            && browsing.purpose == Purpose::Open
                        {
                            answer = Some(row.path.clone());
                        }
                    }
                });

            ui.separator();
            if browsing.purpose == Purpose::Save {
                ui.horizontal(|ui| {
                    ui.label("Call it");
                    let response =
                        ui.add(egui::TextEdit::singleline(&mut browsing.name).desired_width(360.0));
                    if response.changed() {
                        browsing.confirm_overwrite = false;
                    }
                    // Saving is what somebody almost always came here to do,
                    // and typing a name is the very next thing that happens,
                    // so the dialog opens with this field already holding the
                    // keyboard rather than making that a click of its own.
                    // `request_focus` only needs calling the once — see
                    // `focus_name`.
                    if browsing.focus_name {
                        response.request_focus();
                        browsing.focus_name = false;
                    }
                    // Consuming, like the path box above, though strictly
                    // this one does not have to be: it runs after the
                    // list's own Enter check rather than before, so nothing
                    // downstream would see this key again regardless.
                    // Consuming anyway keeps the two fields behaving alike,
                    // rather than one working only because of where it
                    // happens to sit in the function.
                    if response.lost_focus()
                        && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter))
                    {
                        let name_given = !browsing.name.trim().is_empty();
                        if name_given {
                            match decide_save(
                                &browsing.at,
                                &browsing.name,
                                browsing.confirm_overwrite,
                            ) {
                                SaveOutcome::Answer(path) => answer = Some(path),
                                SaveOutcome::NeedsConfirmation => browsing.confirm_overwrite = true,
                            }
                        }
                    }
                });
            }

            if let Some(error) = &browsing.error {
                ui.add_space(4.0);
                ui.label(egui::RichText::new(error).color(crate::theme::REFUSED));
            }
            if browsing.confirm_overwrite {
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new(
                        "There is already a file of that name here. Press Save \
                         again to write over it.",
                    )
                    .color(crate::theme::CAUTION),
                );
            }

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let verb = match browsing.purpose {
                    Purpose::Open => "Open",
                    Purpose::Save => "Save",
                };
                let ready = match browsing.purpose {
                    Purpose::Open => browsing
                        .chosen
                        .and_then(|i| rows.get(i))
                        .map(|r| !r.is_dir)
                        .unwrap_or(false),
                    Purpose::Save => !browsing.name.trim().is_empty(),
                };
                if ui
                    .add_enabled(ready, egui::Button::new(egui::RichText::new(verb).strong()))
                    .clicked()
                {
                    match browsing.purpose {
                        Purpose::Open => {
                            if let Some(row) = browsing.chosen.and_then(|i| rows.get(i)) {
                                answer = Some(row.path.clone());
                            }
                        }
                        Purpose::Save => {
                            match decide_save(
                                &browsing.at,
                                &browsing.name,
                                browsing.confirm_overwrite,
                            ) {
                                SaveOutcome::Answer(path) => answer = Some(path),
                                SaveOutcome::NeedsConfirmation => browsing.confirm_overwrite = true,
                            }
                        }
                    }
                }
                if ui.button("Cancel").clicked() {
                    close = true;
                }
            });
        });

        // The modal already knows how to close itself on a click outside it
        // or on Escape — the second of which is exactly what "Escape cancels
        // the whole picker" means — so long as it is actually asked.
        if modal.should_close() {
            close = true;
        }

        if let Some(path) = answer {
            self.answered = Some((browsing.who, path));
            close = true;
        }
        if close {
            self.open = None;
        }
    }
}

impl Browsing {
    /// Move to a folder: the one place `at` changes, so the path box and the
    /// selection change with it instead of drifting out of step with each
    /// other or with something left over from wherever the dialog was
    /// before.
    fn navigate_to(&mut self, at: PathBuf) {
        self.path_box = at.to_string_lossy().into_owned();
        self.at = at;
        self.chosen = None;
        self.confirm_overwrite = false;
        self.path_problem = None;
        self.type_ahead.clear();
    }
}

/// Somewhere to start when nothing better is known.
fn somewhere_sensible() -> PathBuf {
    std::env::current_dir()
        .ok()
        .filter(|p| p.is_dir())
        .unwrap_or_else(onionskin::install::home)
}

/// The path as a row of clickable parts.
fn breadcrumbs(at: &Path) -> Vec<(String, PathBuf)> {
    let mut crumbs = Vec::new();
    let mut so_far = PathBuf::new();
    for part in at.components() {
        so_far.push(part.as_os_str());
        let label = part.as_os_str().to_string_lossy().into_owned();
        crumbs.push((
            if label == "/" { "/".to_string() } else { label },
            so_far.clone(),
        ));
    }
    if crumbs.is_empty() {
        crumbs.push(("/".to_string(), PathBuf::from("/")));
    }
    crumbs
}

/// The folders people actually keep things in.
fn places() -> Vec<(&'static str, PathBuf)> {
    let home = onionskin::install::home();
    let mut places = vec![("Home", home.clone())];
    for (label, name) in [
        ("Desktop", "Desktop"),
        ("Documents", "Documents"),
        ("Downloads", "Downloads"),
    ] {
        let path = home.join(name);
        if path.is_dir() {
            places.push((label, path));
        }
    }
    if let Ok(here) = std::env::current_dir() {
        places.push(("This folder", here));
    }
    places
}

/// What is in a folder, folders first and then files, both by name.
///
/// Anything unreadable is left out rather than reported: a folder somebody
/// cannot open is not an error, it is simply not where their file is.
fn read_folder(at: &Path, kinds: &[String]) -> Vec<Row> {
    let Ok(entries) = std::fs::read_dir(at) else {
        return Vec::new();
    };
    let mut folders = Vec::new();
    let mut files = Vec::new();

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        // Hidden files are hidden. Somebody who wants one can type its name.
        if name.starts_with('.') {
            continue;
        }
        let path = entry.path();
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);

        if is_dir {
            folders.push(Row { name, path, is_dir });
        } else if kinds.is_empty() || has_kind(&path, kinds) {
            files.push(Row { name, path, is_dir });
        }
    }
    folders.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    files.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    folders.extend(files);
    folders
}

fn has_kind(path: &Path, kinds: &[String]) -> bool {
    let Some(extension) = path.extension().and_then(|e| e.to_str()) else {
        return false;
    };
    let extension = extension.to_ascii_lowercase();
    kinds
        .iter()
        .any(|kind| kind.eq_ignore_ascii_case(&extension))
}

/// Work out what somebody meant by typing, or pasting, `input` into the path
/// box.
///
/// A bare `~`, or `~` followed by a slash, means `home`. Anything else that is
/// not already absolute is taken from `at` — the folder currently on screen —
/// rather than from the program's own working directory, because a box that
/// lives inside a particular folder ought to read paths the way somebody
/// standing in that folder would write them.
///
/// Only that one shape of `~` is expanded. `~fred`, which a shell would read
/// as somebody else's home directory, is left as an ordinary — and almost
/// certainly non-existent — file name instead: guessing which `fred` was
/// meant and sending somebody into that folder would be worse than an error.
fn resolve_path_input(input: &str, at: &Path, home: &Path) -> Result<Destination, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("Type a path first.".to_string());
    }

    let expanded = if trimmed == "~" {
        home.to_path_buf()
    } else if let Some(rest) = trimmed.strip_prefix("~/") {
        home.join(rest)
    } else {
        PathBuf::from(trimmed)
    };

    let joined = if expanded.is_absolute() {
        expanded
    } else {
        at.join(expanded)
    };
    let path = normalise(&joined);

    if path.is_dir() {
        Ok(Destination::Folder(path))
    } else if path.is_file() {
        // `parent` only fails to answer for a root or a prefix on its own,
        // and a file can never be either, so the fallback is not expected to
        // be reached — it is here because falling back to the file itself
        // is still safer than a `Path` method this crate has no business
        // being certain about.
        let folder = path.parent().unwrap_or(&path).to_path_buf();
        Ok(Destination::File { folder, file: path })
    } else {
        Err(format!("There is nothing at {}.", path.to_string_lossy()))
    }
}

/// Fold `.` and `..` out of a path lexically, without asking the filesystem.
///
/// `Path::join` does not do this — `/a/b`.join("../c") is `/a/b/../c`, not
/// `/a/c` — and leaving the `..` in is what would then turn up in the path
/// box and the breadcrumbs after going there, reading as though the program
/// does not actually know where it is.
fn normalise(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for part in path.components() {
        match part {
            Component::ParentDir => match result.components().next_back() {
                Some(Component::Normal(_)) => {
                    result.pop();
                }
                Some(Component::RootDir) | Some(Component::Prefix(_)) => {
                    // Already at the top: ".." from "/" is still "/".
                }
                _ => {
                    // A relative path with nothing behind it yet to cancel.
                    result.push("..");
                }
            },
            Component::CurDir => {}
            other => result.push(other.as_os_str()),
        }
    }
    result
}

/// Where Up, Down, Home, End, Page Up and Page Down put the selection, given
/// how many rows there are and where it is now.
///
/// Nothing wraps: pressing Up on the first row, or Down on the last, leaves
/// it exactly where it was. A list that wrapped would mean a single stray
/// keystroke could silently move the selection from the top of a folder with
/// five hundred files in it to the bottom, which is a worse surprise than not
/// moving at all.
///
/// With nothing selected yet, every one of these lands on the first row
/// rather than the last: the first press of any of them means "start
/// looking", not "start at the end".
fn step_selection(current: Option<usize>, count: usize, step: Step) -> Option<usize> {
    if count == 0 {
        return None;
    }
    let last = count - 1;
    Some(match step {
        Step::Up => current.map_or(0, |i| i.saturating_sub(1)),
        Step::Down => current.map_or(0, |i| (i + 1).min(last)),
        Step::PageUp => current.map_or(0, |i| i.saturating_sub(10)),
        Step::PageDown => current.map_or(0, |i| (i + 10).min(last)),
        Step::Home => 0,
        Step::End => last,
    })
}

/// The first row whose name starts with `prefix`, matching without regard to
/// case — the same way the list is already sorted, in `read_folder`, so
/// "typing in order" and "reading in order" agree with each other.
fn jump_to_prefix(rows: &[Row], prefix: &str) -> Option<usize> {
    if prefix.is_empty() {
        return None;
    }
    let prefix = prefix.to_lowercase();
    rows.iter()
        .position(|row| row.name.to_lowercase().starts_with(&prefix))
}

/// Whether enough silence has passed since `last` that the next letter
/// should start a fresh type-ahead search rather than extend the old one.
///
/// A second is long enough to type two or three letters on purpose, and
/// short enough that coming back to the keyboard after a pause searches from
/// scratch instead of jumping around from wherever an old, forgotten prefix
/// happened to leave off.
fn prefix_expired(last: f64, now: f64) -> bool {
    now - last > 1.0
}

/// What pressing Enter on the list does with the row that is selected: go
/// into it if it is a folder, or hand it back if it is a file — mirroring a
/// click, which is what Enter means in every other file browser.
fn enter_row(rows: &[Row], chosen: Option<usize>) -> Option<Entered> {
    let row = rows.get(chosen?)?;
    Some(if row.is_dir {
        Entered::Folder(row.path.clone())
    } else {
        Entered::File(row.path.clone())
    })
}

/// Decide what pressing Save — the button, or Enter in the name field —
/// should do: answer with the path, unless it already exists and
/// `already_confirmed` is not yet true, in which case the caller should arm
/// the confirmation and wait to be asked again before writing over it.
fn decide_save(at: &Path, name: &str, already_confirmed: bool) -> SaveOutcome {
    let path = at.join(name.trim());
    if path.exists() && !already_confirmed {
        SaveOutcome::NeedsConfirmation
    } else {
        SaveOutcome::Answer(path)
    }
}

#[cfg(test)]
mod tests;
