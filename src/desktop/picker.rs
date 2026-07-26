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

use std::path::{Path, PathBuf};

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
}

/// One row in the list.
struct Row {
    name: String,
    path: PathBuf,
    is_dir: bool,
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
            at,
            kinds: kinds.iter().map(|k| k.to_string()).collect(),
            name: suggested.to_string(),
            chosen: None,
            confirm_overwrite: false,
            error: None,
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

        egui::Modal::new(egui::Id::new("file-picker")).show(ctx, |ui| {
            ui.set_width(660.0);
            ui.heading(&browsing.title);
            ui.add_space(6.0);

            // Where we are, and a way back up. The path is a row of buttons
            // rather than text, because going up three folders is otherwise a
            // matter of typing.
            ui.horizontal_wrapped(|ui| {
                for (label, path) in breadcrumbs(&browsing.at) {
                    if ui.small_button(label).clicked() {
                        browsing.at = path;
                        browsing.chosen = None;
                    }
                }
            });
            ui.add_space(4.0);

            // The usual places, so nobody has to walk up to the root to get
            // to their own documents.
            ui.horizontal_wrapped(|ui| {
                for (label, path) in places() {
                    if ui.small_button(label).clicked() {
                        browsing.at = path;
                        browsing.chosen = None;
                    }
                }
            });
            ui.separator();

            let rows = read_folder(&browsing.at, &browsing.kinds);
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

                        // One click goes into a folder. A folder is never the
                        // answer here — only a file is — so there is nothing
                        // to be gained by making somebody select it first and
                        // then double-click, and double-click is the gesture
                        // people miss.
                        if response.clicked() {
                            if row.is_dir {
                                browsing.at = row.path.clone();
                                browsing.chosen = None;
                            } else {
                                browsing.chosen = Some(index);
                                browsing.confirm_overwrite = false;
                                browsing.name = row.name.clone();
                            }
                        }
                        // A double click on a file takes it and closes, which
                        // is what the gesture means everywhere else.
                        if response.double_clicked() && !row.is_dir && browsing.purpose == Purpose::Open
                        {
                            answer = Some(row.path.clone());
                        }
                    }
                });

            ui.separator();
            if browsing.purpose == Purpose::Save {
                ui.horizontal(|ui| {
                    ui.label("Call it");
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut browsing.name).desired_width(360.0),
                    );
                    if response.changed() {
                        browsing.confirm_overwrite = false;
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
                            let path = browsing.at.join(browsing.name.trim());
                            // Writing over somebody's file without asking is
                            // the one mistake here that cannot be undone.
                            if path.exists() && !browsing.confirm_overwrite {
                                browsing.confirm_overwrite = true;
                            } else {
                                answer = Some(path);
                            }
                        }
                    }
                }
                if ui.button("Cancel").clicked() {
                    close = true;
                }
            });
        });

        if let Some(path) = answer {
            self.answered = Some((browsing.who, path));
            close = true;
        }
        if close {
            self.open = None;
        }
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
    kinds.iter().any(|kind| kind.eq_ignore_ascii_case(&extension))
}
