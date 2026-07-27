//! Putting Onionskin where the operating system can find it.
//!
//! The whole installer is the program itself. Download one file, run
//! `onionskin install`, and it copies itself somewhere on the path, brings the
//! rendering library along, and adds a menu entry. There is no separate setup
//! program to sign, to trust, or to keep in step with the thing it installs.
//!
//! Two rules run through all of it:
//!
//! * **Nothing needs administrator rights.** Installing into your own home
//!   directory is the default everywhere. A program that demands a password to
//!   put a file on your own computer teaches people to give passwords to
//!   programs, and Onionskin has no business doing that.
//! * **Everything is reversible.** `onionskin uninstall` removes exactly what
//!   was put there and says what it removed. Anything the installer changed
//!   that it cannot safely change back — a line in a shell profile — is marked
//!   so it can be found by eye.

use std::path::{Path, PathBuf};

/// The line written into a shell profile, marked so it can be found again.
const MARKER: &str = "# added by onionskin install";

#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("could not work out where this program is: {0}")]
    Whereami(std::io::Error),
    #[error("could not {doing} {path}: {source}")]
    Io {
        doing: &'static str,
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{0}")]
    Refused(String),
}

fn io<'a>(
    doing: &'static str,
    path: &'a Path,
) -> impl FnOnce(std::io::Error) -> InstallError + 'a {
    move |source| InstallError::Io {
        doing,
        path: path.to_path_buf(),
        source,
    }
}

/// What an install did, or would do.
#[derive(Debug, Default, PartialEq)]
pub struct Report {
    pub binary: Option<PathBuf>,
    /// The desktop window, if one was installed alongside.
    pub desktop: Option<PathBuf>,
    pub library: Option<PathBuf>,
    pub menu_entry: Option<PathBuf>,
    /// The profile a PATH line was added to, if one was needed.
    pub profile: Option<PathBuf>,
    /// Whether the destination is already somewhere the shell looks.
    pub already_on_path: bool,
    /// Things that were left alone, and why.
    pub notes: Vec<String>,
}

/// Where to put it.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Install here instead of the usual place.
    pub prefix: Option<PathBuf>,
    /// Do not touch any shell profile.
    pub keep_path: bool,
    /// Do not add a menu entry.
    pub no_menu: bool,
}

/// Where Onionskin installs itself when nobody says otherwise.
///
/// A per-user directory on every platform. `/usr/local/bin` would need a
/// password, and would put one person's choice on everybody's machine.
pub fn default_prefix() -> PathBuf {
    if cfg!(windows) {
        // %LOCALAPPDATA%\Onionskin — the ordinary home for a per-user program.
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            if !local.is_empty() {
                return PathBuf::from(local).join("Onionskin");
            }
        }
        home().join("AppData").join("Local").join("Onionskin")
    } else {
        // ~/.local/bin, which every modern shell puts on the path already.
        home().join(".local").join("bin")
    }
}

pub fn home() -> PathBuf {
    for key in ["HOME", "USERPROFILE"] {
        if let Ok(value) = std::env::var(key) {
            if !value.is_empty() {
                return PathBuf::from(value);
            }
        }
    }
    if let (Ok(drive), Ok(path)) = (std::env::var("HOMEDRIVE"), std::env::var("HOMEPATH")) {
        return PathBuf::from(format!("{drive}{path}"));
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

/// What the installed program is called on this platform.
pub fn binary_name() -> &'static str {
    if cfg!(windows) {
        "onionskin.exe"
    } else {
        "onionskin"
    }
}

/// What the window is called.
pub fn desktop_name() -> &'static str {
    if cfg!(windows) {
        "onionskin-desktop.exe"
    } else {
        "onionskin-desktop"
    }
}

/// What the desktop window needs from the operating system before it will open.
///
/// The window draws with OpenGL and reads the keyboard through the desktop's
/// own libraries, and it loads them when it starts rather than linking them, so
/// a machine without them gets a program that quits with a line about a file
/// nobody has heard of instead of a window.
///
/// Every desktop Linux has these. A server install, a container and a minimal
/// virtual machine do not — which is exactly where somebody is most likely to
/// be puzzled by it, and least likely to guess the answer.
#[cfg(target_os = "linux")]
pub fn desktop_needs() -> Vec<&'static str> {
    // Wayland or X11 will do, so the two are checked as alternatives. Missing
    // *both* is what stops the window opening.
    let display = ["libwayland-client.so.0", "libX11.so.6"];
    let mut missing = Vec::new();
    if !display.iter().any(|name| library_is_here(name)) {
        missing.push("libX11.so.6");
    }
    for name in ["libxkbcommon.so.0", "libxkbcommon-x11.so.0"] {
        if !library_is_here(name) {
            missing.push(name);
        }
    }
    // Either the older GL front end or the newer one is enough to get a
    // surface to draw on.
    if !["libGL.so.1", "libEGL.so.1"]
        .iter()
        .any(|name| library_is_here(name))
    {
        missing.push("libGL.so.1");
    }
    missing
}

#[cfg(not(target_os = "linux"))]
pub fn desktop_needs() -> Vec<&'static str> {
    // Windows and macOS ship their own graphics and keyboard handling, and
    // there has never been anything to install for either.
    Vec::new()
}

/// The one command that installs what is missing, for this kind of machine.
///
/// Worked out from which package manager is actually here rather than from the
/// name in `/etc/os-release`, because the file lies on derivatives and the
/// binary on disk does not.
#[cfg(target_os = "linux")]
pub fn how_to_install_desktop_needs() -> &'static str {
    for (tool, command) in [
        (
            "apt-get",
            "sudo apt install libxkbcommon-x11-0 libgl1 libxcursor1 libxrandr2 libxi6",
        ),
        (
            "dnf",
            "sudo dnf install libxkbcommon-x11 mesa-libGL libXcursor libXrandr libXi",
        ),
        (
            "pacman",
            "sudo pacman -S libxkbcommon-x11 libglvnd libxcursor libxrandr libxi",
        ),
        (
            "zypper",
            "sudo zypper install libxkbcommon-x11-0 Mesa-libGL1 libXcursor1 libXrandr2 libXi6",
        ),
        ("apk", "sudo apk add libxkbcommon mesa-gl libxcursor libxrandr libxi"),
    ] {
        if which_binary(tool).is_some() {
            return command;
        }
    }
    "install the X11, xkbcommon and OpenGL runtime libraries with your package manager"
}

#[cfg(not(target_os = "linux"))]
pub fn how_to_install_desktop_needs() -> &'static str {
    ""
}

/// Is a shared library anywhere the loader would look?
#[cfg(target_os = "linux")]
fn library_is_here(name: &str) -> bool {
    // The ordinary places, plus whatever the loader has been told about. Not
    // exhaustive by design: a false "missing" costs somebody one wasted
    // package install, and a false "present" costs them a program that will
    // not start and no idea why.
    let mut roots: Vec<PathBuf> = vec![
        PathBuf::from("/lib"),
        PathBuf::from("/lib64"),
        PathBuf::from("/usr/lib"),
        PathBuf::from("/usr/lib64"),
        PathBuf::from("/usr/local/lib"),
        // Debian and Ubuntu put nearly everything here.
        PathBuf::from("/lib/x86_64-linux-gnu"),
        PathBuf::from("/usr/lib/x86_64-linux-gnu"),
        PathBuf::from("/usr/lib/aarch64-linux-gnu"),
    ];
    if let Ok(extra) = std::env::var("LD_LIBRARY_PATH") {
        roots.extend(std::env::split_paths(&extra));
    }
    roots.iter().any(|root| root.join(name).exists())
}

/// Is this program on the path?
#[cfg(target_os = "linux")]
fn which_binary(name: &str) -> Option<PathBuf> {
    every_binary(name).into_iter().next()
}

/// Every copy of a program on the path, in the order the shell would find
/// them — so the first is the one that actually runs.
///
/// There are two ways to install Onionskin and they land in different places:
/// `onionskin install` puts a copy in the user's own account, and the `.deb`
/// puts one in `/usr/bin`. Do both and there are two, and the shell silently
/// prefers whichever directory comes first on PATH. Somebody who downloads a
/// new version, installs it, and finds nothing has changed has almost always
/// hit this — and there is nothing on the screen to tell them, because the old
/// program is working perfectly. It just is not the one they installed.
pub fn every_binary(name: &str) -> Vec<PathBuf> {
    match std::env::var_os("PATH") {
        Some(path) => every_binary_on(&path, name),
        None => Vec::new(),
    }
}

/// The same, against a given path rather than the process's own.
///
/// Split out so the searching can be tested without setting PATH, which is a
/// process-wide thing and therefore not something a test running beside other
/// tests should be touching.
fn every_binary_on(path: &std::ffi::OsStr, name: &str) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = Vec::new();
    for dir in std::env::split_paths(path) {
        let candidate = dir.join(name);
        if !candidate.is_file() {
            continue;
        }
        // The same directory can be on PATH twice, and ~/.local/bin and
        // /home/someone/.local/bin are the same place under two names.
        let settled = candidate.canonicalize().unwrap_or_else(|_| candidate.clone());
        if found
            .iter()
            .any(|seen| seen.canonicalize().unwrap_or_else(|_| seen.clone()) == settled)
        {
            continue;
        }
        found.push(candidate);
    }
    found
}

/// The rendering library's name here, in the order worth trying.
fn library_names() -> &'static [&'static str] {
    if cfg!(windows) {
        &["pdfium.dll", "libpdfium.dll"]
    } else if cfg!(target_os = "macos") {
        &["libpdfium.dylib"]
    } else {
        &["libpdfium.so"]
    }
}

/// Is `directory` somewhere the shell already looks for programs?
pub fn on_path(directory: &Path) -> bool {
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|entry| {
        // Compare resolved paths where possible: ~/.local/bin and
        // /home/someone/.local/bin are the same directory.
        match (entry.canonicalize(), directory.canonicalize()) {
            (Ok(a), Ok(b)) => a == b,
            _ => entry == directory,
        }
    })
}

/// Which shell profile to add a PATH line to.
///
/// The one the user's shell actually reads, which is not the same file
/// everywhere: zsh is the default on macOS and reads `.zprofile`, bash reads
/// `.bash_profile` if it exists and `.profile` otherwise.
pub fn shell_profile() -> PathBuf {
    let home = home();
    let shell = std::env::var("SHELL").unwrap_or_default();

    if shell.ends_with("zsh") {
        return home.join(".zprofile");
    }
    if shell.ends_with("fish") {
        return home.join(".config/fish/config.fish");
    }
    let bash_profile = home.join(".bash_profile");
    if bash_profile.is_file() {
        return bash_profile;
    }
    home.join(".profile")
}

/// The line that puts `directory` on the path, in this shell's syntax.
pub fn path_line(directory: &Path, profile: &Path) -> String {
    let shown = directory.display();
    if profile.to_string_lossy().contains("fish") {
        format!("fish_add_path {shown}  {MARKER}\n")
    } else {
        format!("export PATH=\"{shown}:$PATH\"  {MARKER}\n")
    }
}

/// A `.desktop` entry, so Onionskin appears in the applications menu.
/// `window` is the desktop program if one was installed, and `binary` the
/// command line one otherwise.
///
/// Which it is changes the entry completely. A window is launched directly and
/// needs no terminal; the command line program has to open one and start the
/// browser interface, because a menu entry that runs a program with no visible
/// output looks exactly like a menu entry that does nothing.
fn desktop_entry(binary: &Path, window: Option<&Path>) -> String {
    let (exec, terminal) = match window {
        Some(window) => (window.display().to_string(), "false"),
        None => (format!("{} serve", binary.display()), "true"),
    };
    format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name=Onionskin\n\
         GenericName=Overprinting\n\
         Comment=Add words to a page that is already printed\n\
         Exec={exec}\n\
         Terminal={terminal}\n\
         Categories=Office;Publishing;Scanning;\n\
         Keywords=print;pdf;scan;delta;overprint;\n\
         StartupWMClass=onionskin\n"
    )
}

/// Copy a file, keeping it executable.
fn place(from: &Path, to: &Path) -> Result<(), InstallError> {
    // Copying a file onto itself truncates it, and running `onionskin install`
    // from the place it installs to is an easy thing to do by accident.
    if let (Ok(a), Ok(b)) = (from.canonicalize(), to.canonicalize()) {
        if a == b {
            return Ok(());
        }
    }
    std::fs::copy(from, to).map_err(io("write", to))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(from)
            .map(|m| m.permissions().mode())
            .unwrap_or(0o755);
        let _ = std::fs::set_permissions(to, std::fs::Permissions::from_mode(mode | 0o700));
    }
    Ok(())
}

/// Install Onionskin for the person running it.
pub fn install(options: &Options) -> Result<Report, InstallError> {
    let me = std::env::current_exe().map_err(InstallError::Whereami)?;
    let here = me.parent().unwrap_or(Path::new(".")).to_path_buf();
    let prefix = options.prefix.clone().unwrap_or_else(default_prefix);

    std::fs::create_dir_all(&prefix).map_err(io("create", &prefix))?;
    let mut report = Report::default();

    // The program itself.
    let target = prefix.join(binary_name());
    place(&me, &target)?;
    report.binary = Some(target.clone());

    // The window, if it came along. Installed under its own name so the menu
    // entry can launch it directly: a menu entry that opens a terminal is not
    // what anybody means by an application.
    let window = here.join(desktop_name());
    if window.is_file() {
        let to = prefix.join(desktop_name());
        place(&window, &to)?;
        report.desktop = Some(to);
    }

    // The rendering library, if it travelled alongside. Onionskin looks beside
    // its own binary first, so putting it there is all that is needed.
    for name in library_names() {
        let beside = here.join(name);
        if beside.is_file() {
            let to = prefix.join(name);
            place(&beside, &to)?;
            report.library = Some(to);
            break;
        }
    }
    if report.library.is_none() {
        report.notes.push(
            "No PDF rendering library was found next to this file, so none was \
             installed.\n    Everything works except comparing two documents; run \
             'onionskin doctor' to see."
                .into(),
        );
    }

    // The path.
    report.already_on_path = on_path(&prefix);
    if !report.already_on_path && !options.keep_path {
        if cfg!(windows) {
            // Editing the registry needs a Windows crate, and getting it wrong
            // damages a setting the whole account depends on. Saying the one
            // command that does it is safer, and the person can see what it
            // will do before they run it.
            report.notes.push(format!(
                "To run 'onionskin' from anywhere, add it to your path:\n    \
                 setx PATH \"%PATH%;{}\"\n    Then open a new terminal.",
                prefix.display()
            ));
        } else {
            let profile = shell_profile();
            let line = path_line(&prefix, &profile);
            let existing = std::fs::read_to_string(&profile).unwrap_or_default();
            // Never twice: someone who installs again should not collect a
            // second copy of the line.
            if !existing.contains(&line) {
                if let Some(parent) = profile.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                let mut text = existing;
                if !text.is_empty() && !text.ends_with('\n') {
                    text.push('\n');
                }
                text.push_str(&line);
                std::fs::write(&profile, text).map_err(io("write", &profile))?;
            }
            report.profile = Some(profile);
        }
    }

    // A menu entry, on the one platform where it is a plain file.
    if !options.no_menu && cfg!(target_os = "linux") {
        let applications = home().join(".local/share/applications");
        if std::fs::create_dir_all(&applications).is_ok() {
            let entry = applications.join("onionskin.desktop");
            if std::fs::write(&entry, desktop_entry(&target, report.desktop.as_deref())).is_ok() {
                report.menu_entry = Some(entry);
            }
        }
    }

    Ok(report)
}

/// Take it all off again.
pub fn uninstall(options: &Options) -> Result<Report, InstallError> {
    let prefix = options.prefix.clone().unwrap_or_else(default_prefix);
    let mut report = Report::default();

    let binary = prefix.join(binary_name());
    if binary.is_file() {
        // Removing the running program is fine on Unix — the file goes, the
        // process carries on from the copy already in memory. Windows holds
        // the file open, so the same move fails there and has to be said out
        // loud rather than reported as a success.
        match std::fs::remove_file(&binary) {
            Ok(()) => report.binary = Some(binary),
            Err(e) if cfg!(windows) => report.notes.push(format!(
                "{} is in use and could not be removed ({e}).\n    Close Onionskin \
                 and delete that file by hand.",
                binary.display()
            )),
            Err(e) => return Err(io("remove", &binary)(e)),
        }
    }

    // The window, which is the larger of the two files and the one somebody
    // would most notice being left behind.
    let window = prefix.join(desktop_name());
    if window.is_file() {
        match std::fs::remove_file(&window) {
            Ok(()) => report.desktop = Some(window),
            Err(e) if cfg!(windows) => report.notes.push(format!(
                "{} is in use and could not be removed ({e}).\n    Close the \
                 Onionskin window and delete that file by hand.",
                window.display()
            )),
            Err(e) => return Err(io("remove", &window)(e)),
        }
    }

    for name in library_names() {
        let library = prefix.join(name);
        if library.is_file() && std::fs::remove_file(&library).is_ok() {
            report.library = Some(library);
        }
    }

    let entry = home().join(".local/share/applications/onionskin.desktop");
    if entry.is_file() && std::fs::remove_file(&entry).is_ok() {
        report.menu_entry = Some(entry);
    }

    // The profile line is taken out only if it is exactly the one that was
    // added. A profile is somebody's own file, and a program that edits it
    // loosely will one day delete a line it did not write.
    let profile = shell_profile();
    if let Ok(text) = std::fs::read_to_string(&profile) {
        if text.contains(MARKER) {
            let kept: String = text
                .lines()
                .filter(|line| !line.contains(MARKER))
                .map(|line| format!("{line}\n"))
                .collect();
            if std::fs::write(&profile, kept).is_ok() {
                report.profile = Some(profile);
            }
        }
    }

    // What is deliberately left behind, because it is the person's own work.
    let profiles = crate::calibrate::home_dir();
    if profiles.exists() {
        report.notes.push(format!(
            "Left alone: {} — your calibration profiles.\n    Delete that folder \
             too if you want no trace.",
            profiles.display()
        ));
    }
    Ok(report)
}

/// Hand a file or a folder to whatever this desktop opens it with.
///
/// Returns whether anything was launched, so a caller can say "it is at ..."
/// instead when nothing was. Best effort by design: on a server, in a
/// container, or over SSH there is nothing to open, and failing quietly is
/// right — the path has just been printed, so nothing is lost.
///
/// Lives here rather than in the window because both want it, and two copies
/// of "which command opens a file on this operating system" is one too many.
pub fn open_with_desktop(path: &Path) -> bool {
    // The one on Windows is a shell built-in rather than a program, which is
    // why it is spelled as an argument to the shell.
    let mut command = if cfg!(target_os = "macos") {
        let mut command = std::process::Command::new("open");
        command.arg(path);
        command
    } else if cfg!(windows) {
        let mut command = std::process::Command::new("cmd");
        command.args(["/C", "start", ""]).arg(path);
        command
    } else {
        let mut command = std::process::Command::new("xdg-open");
        command.arg(path);
        command
    };
    // Its output is not ours: a viewer that writes a warning to the terminal
    // would otherwise appear in the middle of Onionskin's own report.
    command
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    command.spawn().is_ok()
}

/// Where an installed copy would be, and whether it is there.
pub fn status(options: &Options) -> (PathBuf, bool) {
    let prefix = options.prefix.clone().unwrap_or_else(default_prefix);
    let binary = prefix.join(binary_name());
    let installed = binary.is_file();
    (binary, installed)
}

#[cfg(test)]
mod tests;
