//! Onionskin — add words to a page that is already printed.
//!
//! This binary covers the scanned-page workflow: you have a sheet in your hand
//! and an image of it, and you want to write something onto the paper itself.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{CommandFactory, Parser, Subcommand};

use onionskin::acquire::{
    acquire, list_devices, scanning_available, unavailable_reason, AcquireOptions, PLACEMENT_ADVICE,
};
use onionskin::calibrate;
use onionskin::discover;
use onionskin::document::{Document, Item};
use onionskin::font::{suggest_system_font, EmbeddedFont};
use onionskin::geometry::{parse_page, PageSize};
use onionskin::install;
use onionskin::letters;
use onionskin::package;
use onionskin::pdf::{write_delta, Font, LineFont, PlacedImage, PlacedLine};
use onionskin::pipeline;
use onionskin::printer;
use onionskin::scan::{register, ScanOptions, ScanRegistration};

#[derive(Parser)]
#[command(
    name = "onionskin",
    version,
    about = "Add words to a page that is already printed.",
    long_about = None,
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// Write over a file Onionskin did not make, instead of stopping.
    ///
    /// Kept apart from the `--force` that some commands have. That one means
    /// "print it anyway, I have read the warning"; this one means "yes, that
    /// file of mine can go". Two different things to be sure about, so two
    /// different flags to be sure with.
    #[arg(long, global = true)]
    overwrite: bool,
}

#[derive(Subcommand)]
enum Command {
    /// Report how the sheet sits in a scan, without writing anything.
    Inspect(InspectArgs),
    /// Write a delta PDF that adds words to the scanned sheet.
    Add(AddArgs),
    /// Scan a sheet, ready to add words to.
    Acquire(AcquireArgs),
    /// Find the scanners on this machine and on this network.
    Scanners,
    /// List the fonts available.
    Fonts(FontsArgs),

    /// Start a new document from a blank page.
    New(NewArgs),
    /// Put words on a document.
    Write(WriteArgs),
    /// Draw lines, boxes, circles and paths on a document.
    Draw(DrawArgs),
    /// List what is on a document, with the number of each piece of text.
    Show(ShowArgs),
    /// Change a piece of text: its words, where it sits, how it is set.
    Edit(EditArgs),
    /// Take a piece of text off the page.
    Erase(EraseArgs),
    /// Put a document back as it was before the last change.
    Undo(UndoArgs),
    /// Put back a change that undo took away.
    Redo(UndoArgs),
    /// Delete the deltas Onionskin is holding on to.
    Tidy,
    /// One sheet each for everybody on a list: certificates, tickets, forms.
    Batch(BatchArgs),
    /// Print solid over something on a sheet you have to hand over.
    Cover(CoverArgs),
    /// Write a document out as a PDF, whole or as a delta onto the printed sheet.
    Print(PrintArgs),
    /// Read the letters off a scanned page.
    Read(ReadArgs),

    /// Compare two documents and write a delta of what the edit added.
    Delta(DeltaArgs),
    /// Compare two documents and report, without writing anything.
    Compare(CompareArgs),
    /// Check a printed sheet came out the way the delta asked.
    Verify(VerifyArgs),
    /// See the sheet with the delta on it, before printing either.
    Proof(ProofArgs),
    /// Put several deltas onto one, so the sheet goes through once.
    Merge(MergeArgs),
    /// Find the places on a form where something can be written.
    Blanks(BlanksArgs),
    /// What was added to which sheet, and when.
    History(HistoryArgs),
    /// Jobs you have saved, to run again on another document.
    #[command(subcommand)]
    Job(JobCommand),
    /// A sheet of labels from a list: addresses, files, shelves.
    Labels(LabelsArgs),
    /// Measure a printer's second-pass registration, once per printer.
    #[command(subcommand)]
    Calibrate(CalibrateCommand),
    /// Check this machine has what Onionskin needs.
    Doctor,
    /// Open the browser interface, on this machine only.
    Serve(ServeArgs),

    /// Find the printers on this machine and on this network.
    Printers(PrintersArgs),
    /// Send a PDF straight to a printer, at 100% and without scaling.
    Send(SendArgs),
    /// Scan a sheet from a multifunction printer over the network.
    Fetch(FetchArgs),

    /// Put Onionskin somewhere your computer can find it.
    Install(InstallArgs),
    /// Take it off again.
    Uninstall(InstallArgs),
    /// Build the archive people download. For making a release.
    Package(PackageArgs),
    /// Build an apt repository, so `apt install onionskin` works from your
    /// own server. For making a release.
    AptRepo(AptRepoArgs),
    /// Print a completion script for your shell, so Tab knows every command.
    Completions(CompletionsArgs),
    /// Choose your own defaults, so you stop typing the same flags.
    Config(ConfigArgs),
}

#[derive(clap::Args)]
struct ConfigArgs {
    #[command(subcommand)]
    command: Option<ConfigCommand>,
}

#[derive(Subcommand)]
enum ConfigCommand {
    /// List every setting, and what it is.
    Show,
    /// Change one: `onionskin config set dpi 300`.
    Set { name: String, value: String },
    /// Go back to Onionskin's own choice for one setting.
    Unset { name: String },
    /// Go back to Onionskin's own choices for all of them.
    Reset,
}

#[derive(clap::Args)]
struct CompletionsArgs {
    /// bash, zsh, fish or powershell. Left out, it guesses from $SHELL.
    shell: Option<String>,
}

#[derive(clap::Args)]
struct AptRepoArgs {
    /// The .deb files to put in it. At least one.
    #[arg(long = "deb", value_name = "FILE", required = true)]
    debs: Vec<PathBuf>,
    /// The folder to build the repository in. Serve this folder over HTTPS.
    #[arg(long, default_value = "apt")]
    out: PathBuf,
    /// The address the repository will be served from, for the instructions.
    #[arg(long, default_value = "https://example.com/apt")]
    url: String,
    /// The suite name, which is the word in the sources line.
    #[arg(long, default_value = "stable")]
    suite: String,
    /// The component name.
    #[arg(long, default_value = "main")]
    component: String,
    /// Who publishes it.
    #[arg(long, default_value = "Onionskin")]
    origin: String,
    /// A short label for it.
    #[arg(long, default_value = "Onionskin")]
    label: String,
    /// A sentence describing it.
    #[arg(long, default_value = "Onionskin packages")]
    description: String,
}

#[derive(clap::Args)]
struct FontsArgs {
    /// Also list every font file found on this machine, not just the built-ins.
    #[arg(long)]
    all: bool,
    /// Look in this folder for fonts from now on, and remember it. Point it at
    /// LibreOffice's fonts folder, or wherever you keep the ones you bought.
    #[arg(long, value_name = "FOLDER")]
    add_folder: Option<PathBuf>,
    /// Stop looking in a folder that was added.
    #[arg(long, value_name = "FOLDER")]
    forget_folder: Option<PathBuf>,
    /// List the folders being searched, and stop.
    #[arg(long)]
    folders: bool,
}

#[derive(clap::Args)]
struct PackageArgs {
    /// Which platform the binary is for. The default is this machine's.
    #[arg(long)]
    platform: Option<String>,
    /// The compiled binary to package. The default is the running one.
    #[arg(long)]
    binary: Option<PathBuf>,
    /// The desktop window to package. The default is whatever sits beside the
    /// command line program.
    #[arg(long)]
    desktop: Option<PathBuf>,
    /// The PDF renderer to bundle. The default is whatever sits beside it.
    #[arg(long)]
    library: Option<PathBuf>,
    /// The licence text. The default is LICENSE in the current directory.
    #[arg(long)]
    licence: Option<PathBuf>,
    /// The version to name the archive after.
    #[arg(long, default_value = env!("CARGO_PKG_VERSION"))]
    version: String,
    /// Where to write the archives.
    #[arg(long, default_value = "dist")]
    out: PathBuf,
}

#[derive(clap::Args)]
struct InstallArgs {
    /// Install here instead of the usual per-user place.
    #[arg(long)]
    prefix: Option<PathBuf>,
    /// Do not touch any shell profile.
    #[arg(long)]
    keep_path: bool,
    /// Do not add an applications-menu entry.
    #[arg(long)]
    no_menu: bool,
}

#[derive(clap::Args)]
struct PrintersArgs {
    /// The print server. The default is CUPS on this machine, which is where a
    /// USB printer appears; give a printer's own address to ask it directly.
    #[arg(long, default_value = "ipp://127.0.0.1:631/")]
    server: String,
    /// Do not look on the network, only ask the print server.
    #[arg(long)]
    no_network: bool,
    /// How long to listen for printers announcing themselves, in seconds.
    #[arg(long, default_value_t = 2.0)]
    listen: f64,
}

#[derive(clap::Args)]
struct SendArgs {
    /// The PDF to print.
    file: PathBuf,
    /// The printer: 'ipp://printer.local/ipp/print', or the name of one from
    /// `onionskin printers`.
    #[arg(long)]
    printer: String,
    /// The print server to look a name up on.
    #[arg(long, default_value = "ipp://127.0.0.1:631/")]
    server: String,
    #[arg(long, default_value_t = 1)]
    copies: u32,
    /// The paper by its IPP name, such as iso_a4_210x297mm. The printer's own
    /// default if not given.
    #[arg(long)]
    media: Option<String>,
    /// What to call the job in the queue.
    #[arg(long, default_value = "Onionskin delta")]
    job_name: String,
    /// Print on both sides. Off by default — a delta on the back of the sheet
    /// it was meant for is a wasted sheet.
    #[arg(long)]
    two_sided: bool,
}

#[derive(clap::Args)]
struct FetchArgs {
    /// Where to write the scan.
    #[arg(short, long)]
    output: PathBuf,
    /// Open the scan when it is written.
    #[arg(long)]
    open: bool,
    /// The scanner: 'http://printer.local/eSCL'.
    #[arg(long)]
    scanner: String,
    /// Dots per inch.
    #[arg(long, default_value_t = 300)]
    resolution: u32,
    /// Scan in colour rather than greyscale.
    #[arg(long)]
    colour: bool,
    /// Take the sheet from the document feeder rather than the glass.
    #[arg(long)]
    feeder: bool,
    /// The paper size to scan, so the whole sheet is captured.
    #[arg(long, default_value_t = default_page())]
    page: String,
    /// Report what the scanner can do, and take no scan.
    #[arg(long)]
    capabilities: bool,
}

#[derive(clap::Args)]
struct ServeArgs {
    /// Which address to listen on. Anything but 127.0.0.1 lets other machines
    /// in, and there is no password.
    #[arg(long, default_value = "127.0.0.1")]
    host: String,
    #[arg(long, default_value_t = 8737)]
    port: u16,
}

#[derive(clap::Args)]
struct DeltaArgs {
    /// The document as it was printed.
    original: PathBuf,
    /// The edited copy.
    edited: PathBuf,
    /// Delta PDF to write. Without it, beside the edited copy as NAME-delta.pdf.
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Open the delta when it is written.
    #[arg(long)]
    open: bool,

    /// How to build it: 'raster' prints exactly the new pixels and can never
    /// re-print existing ink; 'vector' keeps the text as text but clips to
    /// rectangles. Without it, your setting, then 'raster'.
    #[arg(long)]
    mode: Option<String>,
    /// Rendering resolution. Higher is more exact and slower. Without it, your
    /// setting, then 400.
    #[arg(long)]
    dpi: Option<f64>,
    /// A calibration profile (see `onionskin calibrate list`).
    #[arg(long)]
    profile: Option<String>,
    /// Warn about additions closer than this to an edge, in mm.
    #[arg(long)]
    margin: Option<f64>,
    /// Write proof images here, showing where the new ink lands.
    #[arg(long)]
    preview: Option<PathBuf>,
    /// Draw a box round every change, so what was added is easy to see. The
    /// box is printed onto the paper along with the change.
    #[arg(long)]
    outline: bool,
    /// And this turns it off again, for when your settings have it on.
    #[arg(long, conflicts_with = "outline")]
    no_outline: bool,
    /// The colour of those boxes: red, green, blue, black, or 'R,G,B' with
    /// each from 0 to 1.
    #[arg(long)]
    outline_colour: Option<String>,
    /// Write the delta even when a check blocks it.
    #[arg(long)]
    force: bool,
    /// Report as JSON instead of for reading.
    #[arg(long)]
    json: bool,

    // Expert. Everything below changes how the comparison itself is made.
    // The defaults are right for paper; these are here because somebody
    // working with an unusual document should not have to fork the program to
    // change a number, and because a program that hides its own workings is
    // asking to be trusted rather than checked.
    /// How dark a pixel must be to count as ink, 1 to 254. Lower catches
    /// fainter marks and more scanner noise with them.
    #[arg(long, value_name = "1-254")]
    ink_threshold: Option<u8>,
    /// How far apart two pieces of new ink can be and still be called one
    /// change, in millimetres.
    #[arg(long, value_name = "MM")]
    group: Option<f64>,
    /// Ignore changes smaller than this, in square millimetres. Raise it to
    /// throw away specks; lower it to catch a full stop.
    #[arg(long, value_name = "MM2")]
    min_region: Option<f64>,
    /// How far outside the changed ink a vector delta's clip box reaches, in
    /// millimetres. Only used with --mode vector.
    #[arg(long, value_name = "MM")]
    pad: Option<f64>,
    /// How far a piece of ink may move and still count as unchanged, in
    /// millimetres.
    #[arg(long, value_name = "MM")]
    tolerance: Option<f64>,
}

#[derive(clap::Args)]
struct CompareArgs {
    /// The document as it was printed.
    original: PathBuf,
    /// The edited copy.
    edited: PathBuf,
    /// Rendering resolution. Without it, your setting, then 400.
    #[arg(long)]
    dpi: Option<f64>,
    /// Warn about ink closer than this to an edge, in mm.
    #[arg(long)]
    margin: Option<f64>,
    #[arg(long)]
    json: bool,
}

#[derive(Subcommand)]
enum CalibrateCommand {
    /// Write the two-pass target to print twice on one sheet.
    Target(TargetArgs),
    /// Measure the printed sheet from a scan of it, and save the profile.
    Measure(MeasureArgs),
    /// Learn from an ordinary job: scan the sheet you just printed.
    Learn(LearnArgs),
    /// Turn readings you took by hand into a stored profile.
    Solve(SolveArgs),
    /// List the profiles on this machine.
    List,
    /// Show one profile in full.
    Show(ProfileName),
    /// Delete a profile.
    Delete(ProfileName),
}

/// Learning the printer's error from a job that was printed anyway.
///
/// The target sheet exists because crosshairs are easy to find. But every
/// delta that was ever printed is also a set of marks in known places, so a
/// scan of the sheet afterwards says where they really went — which makes
/// calibration something that happens by using the program rather than
/// something somebody has to sit down and do.
#[derive(clap::Args)]
struct LearnArgs {
    /// A scan of the sheet after the delta was printed onto it.
    scan: PathBuf,
    /// The delta that was printed onto it.
    #[arg(long)]
    delta: PathBuf,
    /// Which profile to teach. Without it, the one in your settings.
    #[arg(long)]
    name: Option<String>,
    /// The paper it was printed on.
    #[arg(long, default_value_t = default_page())]
    page: String,
    /// The scan is already cropped to the sheet.
    #[arg(long)]
    cropped: bool,
    /// The sheet is square in the scan; do not look for skew.
    #[arg(long)]
    square: bool,
    /// Say what was measured without saving anything.
    #[arg(long)]
    dry_run: bool,
}

/// Checking a sheet actually came out right.
///
/// Overprinting is the one operation where nobody finds out it went wrong until
/// somebody looks at the paper — a delta that was written perfectly can still
/// land two millimetres low, or not print at all because the sheet went in the
/// wrong way round, and the file on disk says nothing about any of it. Sixty
/// certificates go in the envelopes and the mistake surfaces at the far end.
///
/// So: scan the first one, and be told.
#[derive(clap::Args)]
struct VerifyArgs {
    /// A scan of the sheet after the delta was printed onto it.
    scan: PathBuf,
    /// The delta that was printed onto it.
    #[arg(long)]
    delta: PathBuf,
    /// The paper it was printed on.
    #[arg(long, default_value_t = default_page())]
    page: String,
    /// The scan is already cropped to the sheet.
    #[arg(long)]
    cropped: bool,
    /// The sheet is square in the scan; do not look for skew.
    #[arg(long)]
    square: bool,
    /// How far out an addition may be, in millimetres, before it is wrong.
    #[arg(long, default_value_t = 1.0)]
    tolerance: f64,
    /// Also teach the profile of this name from what was measured.
    #[arg(long, value_name = "NAME")]
    learn: Option<String>,
}

/// Several deltas, one pass through the printer.
///
/// A day's work on one document arrives as more than one delta: the stamp is a
/// saved job, the signature is a picture, the reference came out of a
/// spreadsheet. Printing three of them means feeding the same sheet through the
/// printer three times, and every pass is a chance to jam it, skew it, or lose
/// it — on a sheet that already has the letterhead on it and cannot be
/// reprinted. Merged first, it goes through once.
#[derive(clap::Args)]
struct MergeArgs {
    /// The deltas to merge, in the order they should be drawn.
    #[arg(required = true, num_args = 2..)]
    deltas: Vec<PathBuf>,
    /// Where to write the merged delta. Without it, beside the first.
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Send it straight to this printer once it is written: the name of one
    /// from `onionskin printers`, or a URI.
    #[arg(long)]
    print_to: Option<String>,
    /// The print server to look that name up on.
    #[arg(long, default_value = "ipp://127.0.0.1:631/")]
    server: String,
    /// Open it when it is written.
    #[arg(long)]
    open: bool,
}

/// Looking at the finished sheet before committing paper to it.
///
/// The delta on its own tells you nothing: it is a nearly blank page, and
/// whether "Approved" lands in the box or across the line under it is not
/// visible in it. The sheet on its own tells you nothing either. The two
/// together are the only honest preview, and the only way to get one used to
/// be to print it.
#[derive(clap::Args)]
struct ProofArgs {
    /// The sheet as it is now: the PDF that was printed.
    sheet: PathBuf,
    /// The delta that would be printed onto it.
    #[arg(long)]
    delta: PathBuf,
    /// Where to write the proof. Without it, beside the delta.
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Show the sheet as a faint hint, as though holding it up to the light.
    #[arg(long)]
    tracing: bool,
    /// What colour to draw the additions in.
    #[arg(long, default_value = "red")]
    colour: String,
    /// How finely to draw it.
    #[arg(long, default_value_t = 150.0)]
    dpi: f64,
    /// Open it when it is written.
    #[arg(long)]
    open: bool,
}

/// Working out where on a form there is room to write.
///
/// The commonest thing anybody does with this program is fill in a printed
/// form, and the first thing they have to do is find the coordinates — with a
/// ruler against the paper, or by reading pixels off the scan in an image
/// editor and converting them, for every box on the page. The page can be asked
/// instead, and what comes back pastes straight into `--at`.
#[derive(clap::Args)]
struct BlanksArgs {
    /// The form: a PDF, or a scan of one.
    form: PathBuf,
    /// The paper it is on.
    #[arg(long, default_value_t = default_page())]
    page: String,
    /// The narrowest gap worth reporting, in millimetres.
    #[arg(long, default_value_t = 20.0)]
    min_width: f64,
    /// The shortest clear band worth reporting, in millimetres.
    #[arg(long, default_value_t = 3.5)]
    min_height: f64,
    /// How far to stay from the paper's edge.
    #[arg(long, default_value_t = onionskin::safety::DEFAULT_MARGIN_MM)]
    margin: f64,
    /// How dark a pixel has to be to count as something already printed.
    #[arg(long, default_value_t = 128)]
    ink_threshold: u8,
    /// The scan is already cropped to the sheet.
    #[arg(long)]
    cropped: bool,
    /// Do not look for skew; take the sheet as square to the scan.
    #[arg(long)]
    square: bool,
    /// Report them as JSON.
    #[arg(long)]
    json: bool,
}

/// Printing onto a sheet of pre-cut label stock.
///
/// The one thing here that is not an overlay on something already printed:
/// label stock is blank and there is nothing to compare against, so nothing is
/// rendered and nothing is diffed. The hard part is the same hard part —
/// getting ink onto a particular rectangle of a particular sheet, in
/// millimetres, and being right about it.
#[derive(clap::Args)]
struct LabelsArgs {
    /// The list, as a CSV file. Its first line names the columns.
    #[arg(long = "from", value_name = "LIST.csv")]
    from: PathBuf,
    /// What goes on each label, with {column} for each row's own. Use \n for
    /// a line break: '{name}\n{address}'.
    #[arg(long, value_name = "TEXT", allow_hyphen_values = true)]
    text: String,
    /// How the stock is cut: columns across by rows down.
    #[arg(long, value_name = "COLSxROWS")]
    grid: String,
    /// The paper the labels are on.
    #[arg(long, default_value_t = default_page())]
    page: String,
    /// Each label's size in millimetres, as WIDTHxHEIGHT. Without it, the
    /// labels are made to fill the page inside the margins.
    #[arg(long, value_name = "WxH")]
    label: Option<String>,
    /// From the paper's left edge to the first label.
    #[arg(long, default_value_t = 7.0)]
    margin_x: f64,
    /// From the paper's top edge to the first label.
    #[arg(long, default_value_t = 15.0)]
    margin_y: f64,
    /// Between one label and the next, across.
    #[arg(long, default_value_t = 2.5)]
    gap_x: f64,
    /// Between one label and the next, down.
    #[arg(long, default_value_t = 0.0)]
    gap_y: f64,
    /// Start at this label, counting from 1 — for a sheet with the first few
    /// already peeled off.
    #[arg(long, default_value_t = 1)]
    start: usize,
    /// White space inside each label, so the words are not against the edge.
    #[arg(long, default_value_t = 3.0)]
    pad: f64,
    /// Type size in points.
    #[arg(long, default_value_t = 10.0)]
    size: f64,
    /// A built-in font's name (see `onionskin fonts`).
    #[arg(long, default_value = "Helvetica")]
    font: String,
    /// Space between lines, as a multiple of the type size.
    #[arg(long, default_value_t = 1.2)]
    leading: f64,
    /// Colour as #rrggbb.
    #[arg(long, default_value = "#000000")]
    colour: String,
    /// Where to write the PDF.
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Stop after this many, to try one sheet before committing the box.
    #[arg(long, value_name = "N")]
    first: Option<usize>,
    /// Open it when it is written.
    #[arg(long)]
    open: bool,
}

#[derive(clap::Subcommand)]
enum JobCommand {
    /// List the jobs saved on this machine.
    List,
    /// Show one in full, including what it will ask for.
    Show(JobName),
    /// Run one on a document.
    Run(RunJobArgs),
    /// Delete one.
    Delete(JobName),
}

#[derive(clap::Args)]
struct JobName {
    /// The job's name.
    name: String,
}

/// Doing the same thing to today's document that was done to yesterday's.
///
/// An office does the same thing to the same form every day: the paid stamp at
/// 150,40 in nine point, the received date under the third line. Working that
/// out once is fine; working it out again every Monday out of a note in
/// somebody's head is how a box of letterhead gets reprinted.
#[derive(clap::Args)]
struct RunJobArgs {
    /// The job to run.
    name: String,
    /// The document to run it on.
    document: PathBuf,
    /// Fill in one of the job's blanks: --set ref=4471.
    #[arg(long = "set", value_name = "NAME=VALUE")]
    set: Vec<String>,
    /// Delta PDF to write. Without it, beside the document.
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Say what it would do without writing anything.
    #[arg(long)]
    dry_run: bool,
    /// Open the delta when it is written.
    #[arg(long)]
    open: bool,
    #[command(flatten)]
    tuning: Tuning,
}

/// The record of what has been added to sheets of paper.
///
/// "What did we add to that invoice, and when" is a question somebody asks
/// months later about a sheet in a filing cabinet, and until now the answer was
/// nowhere. Where the files were and how much went on them is kept — never the
/// words themselves, which would make a far more sensitive file than any
/// document it described.
#[derive(clap::Args)]
struct HistoryArgs {
    /// How many to show.
    #[arg(long, default_value_t = 20)]
    limit: usize,
    /// Delete the record.
    #[arg(long)]
    forget: bool,
    /// Report it as JSON.
    #[arg(long)]
    json: bool,
}

#[derive(clap::Args)]
struct MeasureArgs {
    /// A scan of the sheet with both passes printed on it.
    scan: PathBuf,
    /// What to call this printer.
    #[arg(long)]
    name: String,
    /// The paper the target was printed on.
    #[arg(long, default_value_t = default_page())]
    page: String,
    /// The inset the target was drawn with, if it was not the default.
    #[arg(long)]
    inset: Option<f64>,
    /// Anything worth remembering about this printer.
    #[arg(long, default_value = "")]
    notes: String,
    /// The image is exactly the sheet: skip finding the paper's edges.
    #[arg(long)]
    cropped: bool,
    /// Do not look for skew; take the sheet as square to the scan.
    #[arg(long)]
    square: bool,
    /// Show what was measured and what it works out to, and save nothing.
    #[arg(long)]
    dry_run: bool,
}

#[derive(clap::Args)]
struct TargetArgs {
    /// Where to write the target.
    #[arg(short, long)]
    output: PathBuf,
    /// The paper it will be printed on.
    #[arg(long, default_value_t = default_page())]
    page: String,
    /// How far in to place the corner crosshairs, in mm.
    #[arg(long)]
    inset: Option<f64>,
    /// Open the target when it is written.
    #[arg(long)]
    open: bool,
}

// Readings are routinely negative, and a leading minus would otherwise be
// taken for the start of another flag.
#[derive(clap::Args)]
#[command(allow_negative_numbers = true)]
struct SolveArgs {
    /// What to call this printer.
    #[arg(long)]
    name: String,
    /// A reading off the sheet: 'P1:dx,dy' in millimetres, right and down
    /// positive. Give one per crosshair.
    #[arg(long = "point", value_name = "P1:DX,DY", allow_hyphen_values = true)]
    points: Vec<String>,
    /// The paper the target was printed on.
    #[arg(long, default_value_t = default_page())]
    page: String,
    /// The inset the target was drawn with, if it was not the default.
    #[arg(long)]
    inset: Option<f64>,
    /// Anything worth remembering about this printer.
    #[arg(long, default_value = "")]
    notes: String,
}

#[derive(clap::Args)]
struct ProfileName {
    /// The profile's name.
    name: String,
}

#[derive(clap::Args)]
struct NewArgs {
    /// Where to keep the document.
    document: PathBuf,
    /// Size of the paper: a4, letter, legal… or a size in mm like 100x150.
    #[arg(long, default_value_t = default_page())]
    page: String,
    /// How many sheets.
    #[arg(long, default_value_t = 1)]
    pages: usize,
    /// Replace a document that is already there.
    #[arg(long)]
    force: bool,
}

// Positions on a page are routinely negative when someone is nudging
// something, and a leading minus would otherwise read as another flag.
/// The delta settings worth overriding for one run.
///
/// Flattened into `write`, `draw` and `batch` rather than repeated in each.
/// Deliberately only two: the calibration profile and the resolution, which
/// are the settings these commands actually act on. Boxes round the changes
/// are not here, because only `delta` draws them — offering a flag that does
/// nothing would be worse than offering none.
///
/// They exist because the stored settings are now honoured here at all. A
/// setting that cannot be departed from for one run is not a preference, it
/// is a rule, and stating a preference must not cost the ability to break it.
#[derive(clap::Args, Clone, Default)]
struct Tuning {
    /// Which calibration profile to use, overriding any saved one.
    #[arg(long, value_name = "NAME")]
    profile: Option<String>,
    /// Rendering resolution, overriding any saved one.
    #[arg(long, value_name = "DPI")]
    dpi: Option<f64>,
}

/// Covering something up on a sheet that is already printed.
///
/// A separate command rather than a note in `draw`'s help, because it is a
/// separate intention and it carries a warning that a filled rectangle does
/// not: a printer can only add toner, so this hides what is underneath from
/// the eye and from a photocopier — it does not remove it from the paper, and
/// somebody holding the sheet up to a strong light may still make it out.
#[derive(clap::Args)]
#[command(allow_negative_numbers = true)]
struct CoverArgs {
    /// The sheet to cover something on.
    document: PathBuf,
    /// What to cover: 'X,Y:WIDTHxHEIGHT', with X,Y its top-left corner, in
    /// millimetres.
    #[arg(long = "over", value_name = "X,Y:WxH", allow_hyphen_values = true)]
    over: Vec<String>,
    /// Cover whatever a word sits on, rather than measuring it: 'Salary'.
    #[arg(long = "word", value_name = "WORDS")]
    word: Vec<String>,
    /// How much beyond the words to cover, in millimetres.
    #[arg(long, default_value_t = 1.0)]
    pad: f64,
    /// Which page, counted from 1.
    #[arg(long, default_value_t = 1)]
    page: usize,
    /// Where to write the delta.
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Write proof images here, showing what will be covered.
    #[arg(long)]
    preview: Option<PathBuf>,
    /// Open the delta when it is written.
    #[arg(long)]
    open: bool,
    #[command(flatten)]
    tuning: Tuning,
}

/// One sheet each, from a spreadsheet.
///
/// Deliberately the same placement flags as `write`, because it is the same
/// question — where do the words go — asked once for two hundred sheets
/// rather than once for one. The only new idea is `{column}`.
#[derive(clap::Args)]
#[command(allow_negative_numbers = true)]
struct BatchArgs {
    /// The printed sheet everybody's copy goes onto: the blank certificate,
    /// the form, the ticket.
    document: PathBuf,
    /// The list, as a CSV file. Its first line names the columns.
    #[arg(long = "from", value_name = "LIST.csv")]
    from: PathBuf,
    /// Where the words go, in millimetres, with {column} standing for each
    /// person's own: '60,120:{name}'.
    #[arg(long = "at", value_name = "X,Y:WORDS", allow_hyphen_values = true)]
    at: Vec<String>,
    /// Just after something already printed on the sheet: 'Awarded to:{name}'.
    #[arg(long = "after", value_name = "ANCHOR:WORDS", allow_hyphen_values = true)]
    after: Vec<String>,
    /// One line below it: 'Name:{name}'.
    #[arg(long = "below", value_name = "ANCHOR:WORDS", allow_hyphen_values = true)]
    below: Vec<String>,
    /// A picture on every sheet, its file name able to name a column so that
    /// each one gets its own: 'signatures/{name}.png:120,240:40'.
    #[arg(long = "image", value_name = "FILE:X,Y:SIZE")]
    images: Vec<String>,
    /// Which page of the sheet, counted from 1.
    #[arg(long, default_value_t = 1)]
    page: usize,
    /// Stop after this many, to try a few before committing the whole stack.
    #[arg(long, value_name = "N")]
    first: Option<usize>,
    /// Type size in points.
    #[arg(long, default_value_t = 11.0)]
    size: f64,
    /// A built-in font's name (see `onionskin fonts`).
    #[arg(long, default_value_t = String::from("Helvetica"))]
    font: String,
    /// Wrap the words at this many millimetres.
    #[arg(long)]
    width: Option<f64>,
    /// Turn the words, degrees clockwise on the page.
    #[arg(long, default_value_t = 0.0)]
    rotation: f64,
    /// Colour as #rrggbb. Most printers only have black.
    #[arg(long, default_value_t = String::from("#000000"))]
    colour: String,
    /// Space between wrapped lines, as a multiple of the type size.
    #[arg(long, default_value_t = 1.2)]
    leading: f64,
    /// Where to write the stack of deltas.
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Write proof images here, showing where the words land.
    #[arg(long)]
    preview: Option<PathBuf>,
    /// Open the stack when it is written.
    #[arg(long)]
    open: bool,
    #[command(flatten)]
    tuning: Tuning,
}

#[derive(clap::Args)]
#[command(allow_negative_numbers = true)]
struct WriteArgs {
    /// The document to write on.
    document: PathBuf,
    /// Where the words go and what they say, in millimetres from the top-left
    /// corner of the paper: 'X,Y:the words'. Y is the baseline — where the
    /// letters sit. Use \n in the text for a line break.
    #[arg(long = "at", value_name = "X,Y:WORDS", allow_hyphen_values = true)]
    at: Vec<String>,
    /// Words placed just after something already on the document, so no
    /// measuring is needed: 'Received:Approved 27 July' puts the words after
    /// "Received:".
    #[arg(long = "after", value_name = "ANCHOR:WORDS", allow_hyphen_values = true)]
    after: Vec<String>,
    /// The same, one line below the anchor and starting where it starts:
    /// 'Signature:J. Bezzina'.
    #[arg(long = "below", value_name = "ANCHOR:WORDS", allow_hyphen_values = true)]
    below: Vec<String>,
    /// A picture to put on the page: a signature, a stamp, a logo.
    ///
    /// 'FILE:X,Y:WIDTH' — the file, where its top-left corner goes in
    /// millimetres, and how wide it is in millimetres. The height follows the
    /// picture's own shape, so it is never squashed. Give 'WIDTHxHEIGHT' to
    /// set both: 'sign.png:120,240:40x15'.
    #[arg(long = "image", value_name = "FILE:X,Y:SIZE")]
    image: Vec<String>,
    /// Which page, counted from 1.
    #[arg(long, default_value_t = 1)]
    page: usize,
    /// Type size in points.
    #[arg(long, default_value_t = 11.0)]
    size: f64,
    /// A built-in font's name (see `onionskin fonts`), or 'file' for the one
    /// passed with --font-file when printing.
    #[arg(long, default_value = "Helvetica")]
    font: String,
    /// Wrap the words at this many millimetres.
    #[arg(long)]
    width: Option<f64>,
    /// Turn the words, degrees clockwise on the page.
    #[arg(long, default_value_t = 0.0)]
    rotation: f64,
    /// Colour as #rrggbb. Most printers only have black.
    #[arg(long, default_value = "#000000")]
    colour: String,
    /// Space between wrapped lines, as a multiple of the type size.
    #[arg(long, default_value_t = 1.2)]
    leading: f64,

    // The three below apply only when writing on a document Onionskin did not
    // make. Writing on one of its own changes that document, so there is
    // nothing to name and nothing to open.
    /// Delta PDF to write, when writing on a Word file, PDF or scan. Without
    /// it, beside the document as NAME-delta.pdf.
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Write proof images here, showing where the words land.
    #[arg(long)]
    preview: Option<PathBuf>,
    /// Open the delta when it is written.
    #[arg(long)]
    open: bool,
    /// Keep this as a named job, to run again on another document.
    #[arg(long = "save-as", value_name = "NAME")]
    save_as: Option<String>,
    #[command(flatten)]
    tuning: Tuning,
}

#[derive(clap::Args)]
struct DrawArgs {
    /// The document to draw on.
    document: PathBuf,
    /// A straight line, in millimetres from the top-left: 'X1,Y1:X2,Y2'.
    #[arg(long = "line", value_name = "X1,Y1:X2,Y2", allow_hyphen_values = true)]
    line: Vec<String>,
    /// A box: 'X,Y:WIDTHxHEIGHT', with X,Y its top-left corner.
    #[arg(long = "box", value_name = "X,Y:WxH", allow_hyphen_values = true)]
    boxes: Vec<String>,
    /// An ellipse: 'X,Y:RADIUSxRADIUS', with X,Y its centre. One radius makes
    /// a circle.
    #[arg(long = "circle", value_name = "X,Y:RxR", allow_hyphen_values = true)]
    circles: Vec<String>,
    /// A run of points joined in order: 'X1,Y1 X2,Y2 X3,Y3'.
    #[arg(long = "path", value_name = "X,Y X,Y ...", allow_hyphen_values = true)]
    paths: Vec<String>,
    /// Join the last point of each --path back to the first.
    #[arg(long)]
    close: bool,
    /// Which page, counted from 1.
    #[arg(long, default_value_t = 1)]
    page: usize,
    /// The outline's colour: #rrggbb, or a name like red or black.
    #[arg(long, default_value = "black")]
    colour: String,
    /// Fill the inside with this colour. Boxes, circles and closed paths only.
    #[arg(long)]
    fill: Option<String>,
    /// Draw the outline this thick, in millimetres.
    #[arg(long, default_value_t = 0.35)]
    width: f64,
    /// Leave the outline off and fill only.
    #[arg(long)]
    no_outline: bool,
    /// Dash the outline: 'DASH,GAP' in millimetres, for example '2,1'.
    #[arg(long, value_name = "DASH,GAP")]
    dash: Option<String>,
    /// Round a box's corners by this many millimetres.
    #[arg(long, default_value_t = 0.0)]
    radius: f64,

    // The three below apply only when drawing on a document Onionskin did not
    // make — a Word file, a PDF, a scan. Drawing on one of its own documents
    // changes that document, so there is nothing to name and nothing to open.
    /// Delta PDF to write, when drawing on a Word file, PDF or scan. Without
    /// it, beside the document as NAME-delta.pdf.
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Write proof images here, showing where the drawing lands.
    #[arg(long)]
    preview: Option<PathBuf>,
    /// Open the delta when it is written.
    #[arg(long)]
    open: bool,
    #[command(flatten)]
    tuning: Tuning,
}

#[derive(clap::Args)]
struct UndoArgs {
    /// The document to put back.
    document: PathBuf,
}

#[derive(clap::Args)]
struct ShowArgs {
    /// The document to look at.
    document: PathBuf,
    /// Report as JSON instead of for reading.
    #[arg(long)]
    json: bool,
}

#[derive(clap::Args)]
#[command(allow_negative_numbers = true)]
struct EditArgs {
    /// The document to change.
    document: PathBuf,
    /// Which piece of text, by the number `onionskin show` gives it.
    item: u32,
    /// New words. Use \n for a line break.
    #[arg(long, allow_hyphen_values = true)]
    text: Option<String>,
    /// Move it to this position, in millimetres: 'X,Y'.
    #[arg(long, value_name = "X,Y", allow_hyphen_values = true)]
    at: Option<String>,
    /// Nudge it by this much, in millimetres: 'X,Y'.
    #[arg(long, value_name = "X,Y", allow_hyphen_values = true)]
    by: Option<String>,
    /// Move it to another page.
    #[arg(long)]
    page: Option<usize>,
    #[arg(long)]
    size: Option<f64>,
    #[arg(long)]
    font: Option<String>,
    #[arg(long)]
    width: Option<f64>,
    /// Stop wrapping, and let the text run on one line.
    #[arg(long, conflicts_with = "width")]
    no_width: bool,
    #[arg(long)]
    rotation: Option<f64>,
    #[arg(long)]
    colour: Option<String>,
    #[arg(long)]
    leading: Option<f64>,
}

#[derive(clap::Args)]
struct EraseArgs {
    /// The document to change.
    document: PathBuf,
    /// Which piece of text, by the number `onionskin show` gives it.
    item: u32,
}

#[derive(clap::Args)]
struct PrintArgs {
    /// The document to print.
    document: PathBuf,
    /// PDF to write. Without it, beside the document, named after it.
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Open the PDF when it is written.
    #[arg(long)]
    open: bool,
    /// Print only what has been added since the sheet was last printed, so it
    /// can go back through the printer onto the same paper.
    #[arg(long)]
    delta: bool,
    /// Note that this document is now on paper, so a later --delta knows what
    /// is already there.
    #[arg(long)]
    printed: bool,
    /// A font file, for text set in 'file'.
    #[arg(long)]
    font_file: Option<PathBuf>,
    /// Which face inside a font collection.
    #[arg(long, default_value_t = 0)]
    font_index: u32,
    /// Print a delta even though something already on the sheet has changed.
    #[arg(long)]
    force: bool,
}

#[derive(clap::Args)]
struct ReadArgs {
    /// The scan: PNG, JPEG, TIFF or BMP.
    scan: PathBuf,
    /// Size of the paper that was scanned.
    #[arg(long, default_value_t = default_page())]
    page: String,
    /// A font file to read the letters against. Without one, Onionskin reports
    /// where every letter is but not which letter it is.
    #[arg(long)]
    font_file: Option<PathBuf>,
    /// Which face inside a font collection.
    #[arg(long, default_value_t = 0)]
    font_index: u32,
    /// Look only for these characters. The default is everything the font can
    /// draw, which covers whatever language the page is in.
    #[arg(long)]
    letters: Option<String>,
    /// The image is exactly the sheet: skip detection and straightening.
    #[arg(long)]
    cropped: bool,
    /// Do not look for skew; take the sheet as square to the scan.
    #[arg(long)]
    square: bool,
    /// Report as JSON instead of for reading.
    #[arg(long)]
    json: bool,
    /// Also write the page out as something editable: a .docx, an .odt, or an
    /// .onion document. Needs --font-file, or there are no words to write.
    #[arg(long = "to", value_name = "FILE")]
    to: Option<PathBuf>,
    /// Write the words as ordinary paragraphs instead of pinning each line
    /// where it was found on the paper.
    #[arg(long)]
    flow: bool,
    /// Open what was written, when --to wrote something.
    #[arg(long)]
    open: bool,
}

#[derive(clap::Args)]
struct AcquireArgs {
    /// Where to write the scan.
    #[arg(short, long)]
    output: PathBuf,
    /// Open the scan when it is written.
    #[arg(long)]
    open: bool,
    /// Which scanner, when there is more than one (see `onionskin scanners`).
    #[arg(long)]
    device: Option<String>,
    /// Dots per inch.
    #[arg(long, default_value_t = 300)]
    resolution: u32,
    /// Scan in colour rather than greyscale.
    #[arg(long)]
    colour: bool,
    /// The paper size, so the scan can be checked once it is taken.
    #[arg(long, default_value_t = default_page())]
    page: String,
}

#[derive(clap::Args)]
#[command(allow_negative_numbers = true)]
struct InspectArgs {
    /// The scan: PNG, JPEG, TIFF or BMP.
    scan: PathBuf,
    /// Size of the paper that was scanned.
    #[arg(long, default_value_t = default_page())]
    page: String,
    /// The image is exactly the sheet: skip detection and straightening.
    #[arg(long)]
    cropped: bool,
    /// Do not look for skew; take the sheet as square to the scan.
    #[arg(long)]
    square: bool,
}

// Angles and page coordinates are routinely negative, and a leading minus
// would otherwise be read as the start of another flag.
#[derive(clap::Args)]
#[command(allow_negative_numbers = true)]
struct AddArgs {
    /// The scan: PNG, JPEG, TIFF or BMP.
    scan: PathBuf,
    /// Delta PDF to write. Without it, beside the scan as NAME-delta.pdf.
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Open the delta when it is written.
    #[arg(long)]
    open: bool,

    /// Words to place, positioned by where they appear in the SCAN, in pixels:
    /// 'X,Y:the words'. This is what you read off an image viewer.
    // allow_hyphen_values: a placement is a string that may start with a minus
    // ("-20,-30:note"), which allow_negative_numbers does not cover since the
    // whole value is not a number.
    #[arg(long = "at", value_name = "X,Y:WORDS", allow_hyphen_values = true)]
    at_scan: Vec<String>,

    /// Words positioned by where they sit on the PAPER, in millimetres from the
    /// top-left corner: 'X,Y:the words'. Use when you measured with a ruler.
    #[arg(long = "at-mm", value_name = "X,Y:WORDS", allow_hyphen_values = true)]
    at_page: Vec<String>,
    /// Words placed just after something already on the page, so no measuring
    /// is needed: 'Received:Approved 27 July' puts the words after "Received".
    #[arg(long = "after", value_name = "ANCHOR:WORDS", allow_hyphen_values = true)]
    after: Vec<String>,
    /// The same, one line below the anchor and starting where it starts:
    /// 'Signature:J. Bezzina'.
    #[arg(long = "below", value_name = "ANCHOR:WORDS", allow_hyphen_values = true)]
    below: Vec<String>,

    /// Size of the paper that was scanned.
    #[arg(long, default_value_t = default_page())]
    page: String,
    /// Type size in points. Without it, measured off the words already on the
    /// page.
    #[arg(long)]
    size: Option<f64>,
    /// One of the built-in fonts (see `onionskin fonts`). Without it, matched
    /// against the words already on the page.
    #[arg(long)]
    font: Option<String>,
    /// Do not look at the page: use Helvetica at 11 pt unless told otherwise.
    #[arg(long)]
    no_match_font: bool,
    /// A .ttf or .ttc to write with, carried inside the delta. Needed for any
    /// alphabet the built-in fonts do not cover.
    #[arg(long)]
    font_file: Option<PathBuf>,
    /// Which face to take from a .ttc collection.
    #[arg(long, default_value_t = 0)]
    font_index: u32,
    /// Turn the words, degrees clockwise on the page.
    #[arg(long, default_value_t = 0.0)]
    rotation: f64,
    /// Follow the sheet's own skew instead of the paper edges.
    #[arg(long)]
    follow_skew: bool,
    /// The image is exactly the sheet: skip detection and straightening.
    #[arg(long)]
    cropped: bool,
    /// Do not look for skew; take the sheet as square to the scan.
    #[arg(long)]
    square: bool,
    /// Write a proof image marking where each addition will land.
    #[arg(long)]
    preview: Option<PathBuf>,
    /// Warn about additions closer than this to an edge, in mm.
    #[arg(long, default_value_t = 5.0)]
    margin: f64,
}

/// Refuse to write over a file we are reading from.
///
/// `onionskin add scan.png -o scan.png` is an easy thing to type, and without
/// this it destroys the scan — quite possibly the only copy of a sheet that has
/// already been through the printer once. The same goes for the proof image.
fn refuse_to_clobber(output: &Path, label: &str, inputs: &[(&Path, &str)]) -> Result<(), String> {
    let target = same_file_key(output);
    for (path, name) in inputs {
        if same_file_key(path) == target {
            return Err(format!(
                "refusing to write the {label} over '{}' — that is the {name}, and \
                 overwriting it would destroy it. Choose a different path.",
                path.display()
            ));
        }
    }
    Ok(())
}

/// A comparable identity for a path, whether or not it exists yet.
///
/// Two files we are about to write cannot be canonicalised, since neither is
/// there — but the proof quietly landing on top of the delta is exactly the
/// kind of mistake worth catching before the work is done, not after.
fn same_file_key(path: &Path) -> PathBuf {
    if let Ok(real) = path.canonicalize() {
        return real;
    }
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(path)
    };
    // Resolve the parent where possible, so ./out/x.pdf and out/x.pdf agree.
    match (absolute.parent(), absolute.file_name()) {
        (Some(parent), Some(name)) => parent
            .canonicalize()
            .unwrap_or_else(|_| parent.to_path_buf())
            .join(name),
        _ => absolute,
    }
}

/// Make sure a file can actually be written before doing the work.
/// Whether `--overwrite` was given. See [`Cli::overwrite`].
static OVERWRITE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn check_writable(path: &Path, label: &str) -> Result<(), String> {
    if path.is_dir() {
        return Err(format!(
            "the {label} path '{}' is a directory. Give it a file name.",
            path.display()
        ));
    }
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() && !parent.is_dir() {
            return Err(format!(
                "the folder '{}' does not exist, so the {label} cannot be written there.",
                parent.display()
            ));
        }
    }

    // Do not write over somebody else's file without being told to.
    //
    // `refuse_to_clobber` runs before this at every call site, and must: an
    // output that is also an input has to be refused outright, and this one
    // would instead offer `--overwrite` — which for `add scan.png -o
    // scan.png` is an invitation to destroy the only copy of the sheet.
    //
    // Onionskin stamps everything it writes, so the ordinary loop — run a
    // command, look at the delta, edit, run it again — asks nothing. What is
    // refused is the case that costs something: an output name that happens
    // to be a file Onionskin did not make, which until now was destroyed in
    // silence.
    if !may_write_over(path, OVERWRITE.load(std::sync::atomic::Ordering::Relaxed)) {
        return Err(format!(
            "'{}' is already there, and Onionskin did not write it — so it has \
             been left alone.\n    Write over it:  add --overwrite\n    Keep \
             it:        choose another name for the {label}",
            path.display()
        ));
    }
    Ok(())
}

/// May this path be written to, given whether `--overwrite` was asked for?
///
/// Split out from `check_writable` so it can be tested without reaching for
/// the process-wide flag, which the tests run in parallel around.
fn may_write_over(path: &Path, overwrite: bool) -> bool {
    overwrite || !path.exists() || ours_to_replace(path)
}

/// Did Onionskin write this file?
///
/// Its PDFs carry `/Producer (Onionskin)` and its documents are its own
/// format, so both are recognisable without keeping a list of what it has
/// written. Anything it cannot positively claim is treated as somebody
/// else's, which is the safe way round: the cost of being wrong here is a
/// question, and the cost of being wrong the other way is their file.
fn ours_to_replace(path: &Path) -> bool {
    if Document::is_one(path) {
        return true;
    }
    let Ok(bytes) = std::fs::read(path) else {
        return false;
    };
    if !bytes.starts_with(b"%PDF") {
        return false;
    }
    bytes
        .windows(PRODUCER.len())
        .any(|window| window == PRODUCER)
}

/// What `pdf`'s document info writes into every PDF Onionskin produces.
///
/// Written without a space after the key, which is how the PDF is serialised
/// — worth matching exactly rather than on the word "Onionskin" alone, which
/// could appear in the text of a page Onionskin had nothing to do with.
const PRODUCER: &[u8] = b"/Producer(Onionskin)";

/// Say what printing only the additions saved.
///
/// The whole argument for the program, in one line. Only said when there is
/// something on the page to have saved against — a percentage of a blank
/// sheet reads as a bug, and is one.
fn report_saving(outcome: &pipeline::Outcome) {
    let Some(saving) = outcome.saving() else {
        return;
    };
    let fraction = saving.ink_fraction();

    // Where the page is nearly as bare as the delta there is nothing to
    // claim, and claiming it anyway would be the kind of number that makes a
    // reader stop believing the others.
    if fraction > 0.9 {
        return;
    }

    let per_cent = fraction * 100.0;
    let ink = if per_cent < 0.1 {
        // "0%" would be a rounding claim rather than a measurement.
        "under 0.1%".to_string()
    } else if per_cent < 10.0 {
        format!("{per_cent:.1}%")
    } else {
        format!("{per_cent:.0}%")
    };

    println!("\nThis uses {ink} of the ink that printing it whole would.");
}

/// Which sheets actually need to go back through the printer.
///
/// A forty-page document with three changes needs three sheets fed, not
/// forty — and until this said so, working out which three meant opening the
/// delta and looking at every page for ink.
fn report_sheets_to_feed(outcome: &pipeline::Outcome) {
    let carrying = outcome.pages_with_additions();
    let sheets = outcome.pages.len();
    if sheets < 2 || carrying.is_empty() || carrying.len() == sheets {
        return;
    }
    println!(
        "\nOnly {} of the {sheets} sheets has anything on it: {}",
        if carrying.len() == 1 {
            "one".to_string()
        } else {
            carrying.len().to_string()
        },
        describe_sheets(&carrying)
    );
    println!("  The rest of the delta is blank — feeding them would do nothing.");
}

/// Page numbers as somebody would say them: "3, 7 and 8", or "3 to 9".
fn describe_sheets(pages: &[usize]) -> String {
    // Runs collapse, because "4 to 21" is a thing a person can act on and
    // eighteen comma-separated numbers is not.
    let mut runs: Vec<(usize, usize)> = Vec::new();
    for page in pages {
        match runs.last_mut() {
            Some(run) if run.1 + 1 == *page => run.1 = *page,
            _ => runs.push((*page, *page)),
        }
    }
    let parts: Vec<String> = runs
        .iter()
        .map(|(first, last)| match last - first {
            0 => first.to_string(),
            1 => format!("{first} and {last}"),
            _ => format!("{first} to {last}"),
        })
        .collect();
    match parts.len() {
        0 => String::new(),
        1 => parts[0].clone(),
        _ => format!(
            "{} and {}",
            parts[..parts.len() - 1].join(", "),
            parts[parts.len() - 1]
        ),
    }
}

const PRINT_INSTRUCTIONS: &str = "\
Printing the delta
  0. Look at it first, if you like — the sheet with the additions on it, in a
     PDF, before any paper is involved:
       onionskin proof <the sheet> --delta <the delta>
  1. Put the scanned sheet back in the tray. Check which way up and which end
     goes first — a page printed upside down is the usual first mistake.
  2. Print at 100% / \"Actual size\". Turn OFF \"Fit to page\"; it scales by a few
     percent and nothing will line up.
  3. Do one sheet first and hold it against the original before committing more.
     Or scan that one sheet and be told, which is stricter than an eye is:
       onionskin verify scan.png --delta <the delta>";

fn main() -> ExitCode {
    allow_a_closed_pipe();
    match run() {
        Ok(code) => code,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::from(1)
        }
    }
}

/// Let `onionskin show ... | head` end quietly.
///
/// Rust turns off SIGPIPE at startup, so writing to a pipe whose reader has
/// gone gives an error instead of killing the process — and `println!` panics
/// on an error it cannot report. Piping any of this into `head` or quitting
/// `less` early therefore prints a panic and a backtrace, which looks exactly
/// like a crash and is nothing of the sort.
fn allow_a_closed_pipe() {
    #[cfg(unix)]
    // SAFETY: `signal` with SIG_DFL is async-signal-safe and this runs before
    // any thread is started. Restoring the default is what every other command
    // line program does, and is precisely the behaviour a shell expects.
    unsafe {
        libc_signal(13 /* SIGPIPE */, 0 /* SIG_DFL */);
    }
}

#[cfg(unix)]
extern "C" {
    #[link_name = "signal"]
    fn libc_signal(signum: i32, handler: usize) -> usize;
}

fn run() -> Result<ExitCode, String> {
    let cli = Cli::parse();
    // Read once here and kept where `check_writable` can reach it, rather
    // than threaded through fourteen call sites that have no other reason to
    // know about it. It is set before any command runs and never again.
    OVERWRITE.store(cli.overwrite, std::sync::atomic::Ordering::Relaxed);
    let Some(command) = cli.command else {
        greet();
        return Ok(ExitCode::SUCCESS);
    };
    match command {
        Command::Fonts(args) => cmd_fonts(args),
        Command::Inspect(args) => cmd_inspect(args),
        Command::Add(args) => cmd_add(args),
        Command::Acquire(args) => cmd_acquire(args),
        Command::Scanners => cmd_scanners(),
        Command::New(args) => cmd_new(args),
        Command::Write(args) => cmd_write(args),
        Command::Draw(args) => cmd_draw(args),
        Command::Show(args) => cmd_show(args),
        Command::Edit(args) => cmd_edit(args),
        Command::Erase(args) => cmd_erase(args),
        Command::Undo(args) => cmd_undo(args),
        Command::Redo(args) => cmd_redo(args),
        Command::Tidy => cmd_tidy(),
        Command::Batch(args) => cmd_batch(args),
        Command::Cover(args) => cmd_cover(args),
        Command::Print(args) => cmd_print(args),
        Command::Read(args) => cmd_read(args),
        Command::Delta(args) => cmd_delta(args),
        Command::Compare(args) => cmd_compare(args),
        Command::Verify(args) => cmd_verify(args),
        Command::Proof(args) => cmd_proof(args),
        Command::Merge(args) => cmd_merge(args),
        Command::Blanks(args) => cmd_blanks(args),
        Command::History(args) => cmd_history(args),
        Command::Job(command) => cmd_job(command),
        Command::Labels(args) => cmd_labels(args),
        Command::Calibrate(command) => cmd_calibrate(command),
        Command::Doctor => cmd_doctor(),
        Command::Serve(args) => {
            onionskin::web::serve(&args.host, args.port).map_err(|e| e.to_string())?;
            Ok(ExitCode::SUCCESS)
        }
        Command::Printers(args) => cmd_printers(args),
        Command::Send(args) => cmd_send(args),
        Command::Fetch(args) => cmd_fetch(args),
        Command::Install(args) => cmd_install(args),
        Command::Uninstall(args) => cmd_uninstall(args),
        Command::Package(args) => cmd_package(args),
        Command::AptRepo(args) => cmd_apt_repo(args),
        Command::Completions(args) => cmd_completions(args),
        Command::Config(args) => cmd_config(args),
    }
}

// ---------------------------------------------------------------------------
// Making a document, and editing it
// ---------------------------------------------------------------------------

fn cmd_new(args: NewArgs) -> Result<ExitCode, String> {
    let page = parse_page(&args.page).map_err(|e| e.to_string())?;
    if args.pages == 0 {
        return Err("a document has at least one page".into());
    }
    // `new --force` has always meant "start that name again from blank",
    // which is the same permission `--overwrite` grants. Honour it as both,
    // so the flag this command documents is not contradicted by the general
    // one a moment later.
    if args.force {
        OVERWRITE.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    check_writable(&args.document, "document")?;
    if args.document.exists() && !args.force {
        return Err(format!(
            "'{}' is already there. Use --force to start it again from blank, \
             or pick another name.",
            args.document.display()
        ));
    }

    let document = Document::blank(page, args.pages);
    document.save(&args.document).map_err(|e| e.to_string())?;

    println!(
        "{}: a blank {} document, {} page{}.",
        args.document.display(),
        page.describe(),
        args.pages,
        if args.pages == 1 { "" } else { "s" }
    );
    if let Some(kind) = misleading_name(&args.document) {
        println!(
            "\nNote: the name ends as though it were {kind}, and this is an \
             Onionskin document —\nOnionskin opens it whatever it is called, \
             but other programs will not. When you want a PDF:\n  onionskin \
             print {} -o {}",
            args.document.display(),
            beside(&args.document, "-printed", "pdf").display()
        );
    }
    println!(
        "\nPut words on it:\n  onionskin write {} --at '25,40:Dear Sir'",
        args.document.display()
    );
    Ok(ExitCode::SUCCESS)
}

/// The names Onionskin fills in without being asked: {today} and its family.
///
/// One place rather than three, because a `{today}` that works when the words
/// are anchored and not when they are placed by millimetre is worse than one
/// that never worked — nobody would report it, they would just stop using it.
fn today_row() -> onionskin::rows::Row {
    onionskin::jobs::values(
        &std::collections::BTreeMap::new(),
        onionskin::history::now(),
    )
}

fn cmd_write(args: WriteArgs) -> Result<ExitCode, String> {
    if args.at.is_empty()
        && args.after.is_empty()
        && args.below.is_empty()
        && args.image.is_empty()
    {
        return Err(
            "nothing to write. Say where the words go — easiest first:\n    \
             --after 'Received:Approved 27 July'   just after something \
             already there\n    --below 'Signature:J. Bezzina'        one \
             line under it\n    --at '25,40:Dear Sir'                 \
             millimetres measured on the paper\n    \
             --image 'sign.png:120,240:40'        a signature, stamp or logo"
                .into(),
        );
    }
    // Anything Onionskin can open can be written on, not only its own
    // documents. What comes out then is a delta rather than an altered file:
    // somebody's Word document is theirs, and putting words on the printed
    // sheet is a different thing from editing what made it.
    if is_document(&args.document) {
        return write_on_document(&args);
    }
    // A picture on one of Onionskin's own documents would have to be stored
    // in it, and the document format has no place for one yet. Saying so is
    // better than writing the words and quietly dropping the signature.
    if !args.image.is_empty() {
        return Err(format!(
            "a picture cannot be stored in an Onionskin document yet — only \
             printed onto one.\n    Print it first, then put the picture on \
             that:\n    onionskin print {} -o sheet.pdf\n    onionskin write \
             sheet.pdf --image '{}'",
            args.document.display(),
            args.image[0]
        ));
    }

    let mut document = Document::load(&args.document).map_err(|e| e.to_string())?;

    // Where the anchored words land, worked out before anything is added so
    // that a run naming an anchor that is not there changes nothing at all.
    // Half a page of new words and then a refusal would be the worst of both.
    let anchored = anchored_places(&document, &args)?;

    let today = today_row();
    let mut added = Vec::new();
    for placement in &args.at {
        let ((x_mm, y_mm), text) =
            parse_placement(&onionskin::rows::fill(placement, &today))?;
        let item = Item {
            id: 0,
            page: args.page,
            x_mm,
            y_mm,
            text: unescape(&text),
            size_pt: args.size,
            font: args.font.clone(),
            width_mm: args.width,
            rotation_deg: args.rotation,
            colour: args.colour.clone(),
            leading: args.leading,
        };
        added.push(document.add(item).map_err(|e| e.to_string())?);
    }
    for (x_mm, y_mm, text) in anchored {
        let item = Item {
            id: 0,
            page: args.page,
            x_mm,
            y_mm,
            text,
            size_pt: args.size,
            font: args.font.clone(),
            width_mm: args.width,
            rotation_deg: args.rotation,
            colour: args.colour.clone(),
            leading: args.leading,
        };
        added.push(document.add(item).map_err(|e| e.to_string())?);
    }
    document.save(&args.document).map_err(|e| e.to_string())?;
    save_the_job(&args);

    for id in &added {
        let item = document.get(*id).expect("just added");
        println!(
            "{id}: page {}, {:.1},{:.1} mm — {}",
            item.page,
            item.x_mm,
            item.y_mm,
            first_line(&item.text)
        );
    }
    warn_off_the_page(&document);
    Ok(ExitCode::SUCCESS)
}

/// One sheet each for everybody on a list.
///
/// The anchors are worked out once, against the sheet itself, because the
/// sheet is the same for everybody — only the words change. Two hundred
/// certificates therefore cost one reading of the page, not two hundred.
/// Print solid over something on a sheet that has to be handed over.
///
/// The words can be named instead of measured, because somebody redacting a
/// payslip knows they want the salary hidden and does not know it starts
/// 46.2 mm across.
fn cmd_cover(args: CoverArgs) -> Result<ExitCode, String> {
    if args.over.is_empty() && args.word.is_empty() {
        return Err(
            "nothing to cover. Say what to hide:\n    \
             --word 'Salary'            cover whatever that word sits on\n    \
             --over '40,100:70x8'       cover a rectangle, in millimetres"
                .into(),
        );
    }
    let output = args
        .output
        .clone()
        .unwrap_or_else(|| beside(&args.document, "-covered", "pdf"));
    refuse_to_clobber(&output, "delta", &[(&args.document, "sheet")])?;
    check_writable(&output, "delta")?;

    let mut boxes: Vec<(f64, f64, f64, f64)> = Vec::new();
    for spec in &args.over {
        let ((x_mm, y_mm), size) = parse_placement(spec)?;
        let (w, h) = parse_size(&size)?;
        boxes.push((x_mm, y_mm, w, h));
    }

    // Named words are found by reading the page, the same way anchors are.
    if !args.word.is_empty() {
        let page_text = read_a_document(&args.document)?;
        for wanted in &args.word {
            let found = onionskin::anchor::boxes_for(&page_text, wanted);
            if found.is_empty() {
                return Err(format!(
                    "nothing on the page reads as '{wanted}', so there is \
                     nothing to cover.\n    Run `onionskin read` to see what \
                     is on it, or give millimetres with --over."
                ));
            }
            for rect in found {
                boxes.push((
                    rect.x_mm - args.pad,
                    rect.y_mm - args.pad,
                    rect.width_mm + args.pad * 2.0,
                    rect.height_mm + args.pad * 2.0,
                ));
            }
        }
    }

    let shapes: Vec<(usize, onionskin::pdf::PlacedShape)> = boxes
        .iter()
        .map(|(x, y, w, h)| {
            (
                args.page,
                onionskin::pdf::PlacedShape {
                    drawing: onionskin::pdf::Drawing::Rect {
                        x_mm: *x,
                        y_mm: *y,
                        width_mm: *w,
                        height_mm: *h,
                        radius_mm: 0.0,
                    },
                    stroke: None,
                    fill: Some((0.0, 0.0, 0.0)),
                    width_mm: 0.0,
                    dash_mm: None,
                },
            )
        })
        .collect();

    let options = options_from_settings(args.preview.clone(), &args.tuning)?;
    let outcome =
        pipeline::compose_run_drawing(&args.document, &[], &shapes, &output, None, &options)
            .map_err(|e| e.to_string())?;

    report_checks(&outcome.checks);
    if outcome.blocked() {
        eprintln!("\nBlocked — see above. Nothing worth printing was produced.");
        return Ok(ExitCode::from(2));
    }
    println!(
        "\n{}: {} area{} covered.",
        output.display(),
        shapes.len(),
        if shapes.len() == 1 { "" } else { "s" }
    );
    for path in &outcome.previews {
        println!("proof: {}", path.display());
    }
    println!(
        "\nWhat this does, and what it does not\n  \
         Printing this lays solid toner over those areas. It hides them from \
         the eye\n  and from a photocopier. It does not take the old ink off \
         the paper —\n  a strong light behind the sheet may still show it \
         through.\n  \
         For anything that must not be recoverable, print a fresh page \
         without it."
    );
    open_if_asked(args.open, &output);
    Ok(ExitCode::SUCCESS)
}

fn cmd_batch(args: BatchArgs) -> Result<ExitCode, String> {
    if args.at.is_empty() && args.after.is_empty() && args.below.is_empty() && args.images.is_empty()
    {
        return Err(
            "nothing to put on them. Say where the words go, with {column} \
             standing for each person's own:\n    \
             --after 'Awarded to:{name}'\n    --at '60,120:{name}'\n    \
             --image 'signatures/{name}.png:120,240:40'"
                .into(),
        );
    }
    let list = onionskin::rows::List::read(&args.from).map_err(|e| e.to_string())?;

    // Every template checked against the columns before a single sheet is
    // made. Two hundred certificates reading "{nmae}" is a discovery to make
    // now, not at the printer.
    let templates: Vec<String> = args
        .at
        .iter()
        .chain(args.after.iter())
        .chain(args.below.iter())
        // The picture paths too: "signatures/{nmae}.png" is the same mistake
        // as "{nmae}" in a line of text, and is worth finding in the same
        // breath rather than at the two hundredth sheet.
        .chain(args.images.iter())
        .cloned()
        .collect();
    let unknown: Vec<String> = onionskin::rows::unknown_columns(&templates, &list)
        .into_iter()
        // {today} is not a column and is not a mistake.
        .filter(|name| !onionskin::jobs::known_without_asking(name))
        .collect();
    if !unknown.is_empty() {
        return Err(format!(
            "{} has no column called {}.\n    It has: {}\n    \
             {{number}} also works, and counts the sheets for you.",
            args.from.display(),
            unknown
                .iter()
                .map(|name| format!("'{name}'"))
                .collect::<Vec<_>>()
                .join(" or "),
            list.describe_columns()
        ));
    }

    let output = args
        .output
        .clone()
        .unwrap_or_else(|| beside(&args.document, "-batch", "pdf"));
    refuse_to_clobber(&output, "stack", &[(&args.document, "sheet"), (&args.from, "list")])?;
    check_writable(&output, "stack")?;

    // Where the anchored words land, worked out once against the sheet.
    // Every copy is the same page, so the answer cannot differ between them.
    let anchors = batch_anchors(&args)?;

    if args.first == Some(0) {
        return Err(
            "--first 0 would make no sheets at all. Leave it out to make one \
             for everybody, or give the number to try first — --first 2."
                .into(),
        );
    }
    let wanted = args.first.unwrap_or(list.rows.len()).min(list.rows.len());
    if args.first.is_some_and(|first| first < list.rows.len()) {
        println!(
            "Making the first {wanted} of {}, because --first says so.",
            list.rows.len()
        );
    }

    // {today} and its relatives, available to every sheet alongside the
    // columns. A certificate saying "Awarded {today}" should not need a column
    // in the spreadsheet holding the same date two hundred times.
    let known = onionskin::jobs::what_the_day_is(onionskin::history::now());

    let mut per_sheet: Vec<Vec<Item>> = Vec::with_capacity(wanted);
    let mut pictures_per_sheet: Vec<Vec<PlacedImage>> = Vec::with_capacity(wanted);
    for row in list.rows.iter().take(wanted) {
        // A column of the same name wins: a list that carries its own dates,
        // one per row, means them.
        let row = &{
            let mut values = known.clone();
            values.extend(row.values.clone());
            onionskin::rows::Row {
                values,
                number: row.number,
            }
        };
        let mut items = Vec::new();
        for placement in &args.at {
            let ((x_mm, y_mm), text) = parse_placement(&onionskin::rows::fill(placement, row))?;
            items.push(batch_item(&args, x_mm, y_mm, unescape(&text)));
        }
        for (x_mm, y_mm, template) in &anchors {
            let text = onionskin::rows::fill(template, row);
            items.push(batch_item(&args, *x_mm, *y_mm, unescape(&text)));
        }
        per_sheet.push(items);

        // Each row's own picture, if the file name named a column. Loaded
        // here rather than shared, because for a signature per person they
        // are all different files — and if they are all the same file, one
        // read of a signature is not worth arranging around.
        let filled: Vec<String> = args
            .images
            .iter()
            .map(|spec| onionskin::rows::fill(spec, row))
            .collect();
        let mine = placed_images(&filled, args.page).map_err(|e| {
            format!(
                "{e}\n    (making the sheet for row {} of {})",
                row.number,
                list.rows.len()
            )
        })?;
        pictures_per_sheet.push(mine.into_iter().map(|(_, image)| image).collect());
    }

    let options = options_from_settings(args.preview.clone(), &args.tuning)?;
    let outcome = pipeline::compose_sheets_with_pictures(
        &args.document,
        args.page,
        &per_sheet,
        &pictures_per_sheet,
        &output,
        None,
        &options,
    )
    .map_err(|e| e.to_string())?;

    report_checks(&outcome.checks);
    if outcome.blocked() {
        eprintln!("\nBlocked — see above. Nothing worth printing was produced.");
        return Ok(ExitCode::from(2));
    }
    println!(
        "\n{}: {wanted} sheet{}, {} addition{} in all.",
        output.display(),
        if wanted == 1 { "" } else { "s" },
        outcome.total_regions(),
        if outcome.total_regions() == 1 { "" } else { "s" }
    );
    for path in &outcome.previews {
        println!("proof: {}", path.display());
    }
    println!(
        "\nPrinting a stack\n  \
         1. Put {wanted} blank sheet{} in the tray, the same way up as the \
         first one\n     you printed.\n  \
         2. Print at 100% / \"Actual size\", with \"Fit to page\" off.\n  \
         3. Try --first 2 and hold them against a real sheet before \
         committing the stack.",
        if wanted == 1 { "" } else { "s" }
    );
    open_if_asked(args.open, &output);
    Ok(ExitCode::SUCCESS)
}

/// One piece of text on a batch sheet, set the way the flags asked for.
fn batch_item(args: &BatchArgs, x_mm: f64, y_mm: f64, text: String) -> Item {
    Item {
        id: 0,
        page: 1,
        x_mm,
        y_mm,
        text,
        size_pt: args.size,
        font: args.font.clone(),
        width_mm: args.width,
        rotation_deg: args.rotation,
        colour: args.colour.clone(),
        leading: args.leading,
    }
}

/// Resolve `--after` and `--below` against the sheet, once.
///
/// Returns where each one lands and the template that goes there. The anchor
/// is read off the sheet itself — the words already printed on it — so it is
/// the same for everybody on the list.
fn batch_anchors(args: &BatchArgs) -> Result<Vec<(f64, f64, String)>, String> {
    if args.after.is_empty() && args.below.is_empty() {
        return Ok(Vec::new());
    }
    let page_text = read_a_document(&args.document)?;
    let gap_mm = onionskin::geometry::pt_to_mm(args.size * 0.3);
    let step_mm = onionskin::geometry::pt_to_mm(args.size * 1.15);

    let mut out = Vec::new();
    for (flag, put) in [
        (&args.after, onionskin::anchor::Where::After),
        (&args.below, onionskin::anchor::Where::Below),
    ] {
        for spec in flag {
            let (anchor, text) = split_anchor(spec)?;
            let placed = onionskin::anchor::place(&page_text, &anchor, put, gap_mm, step_mm)
                .map_err(|e| e.to_string())?;
            out.push((placed.x_mm, placed.y_mm, text));
        }
    }
    Ok(out)
}

/// Work out where `--after` and `--below` put their words on a document.
///
/// The document's own layout is used rather than a rendering of it, so the
/// answer is exact: it already knows where every word sits, to the millimetre
/// it will print at. Nothing is added here — the caller does that once every
/// anchor has been found, so a run that cannot find one leaves the document
/// exactly as it was.
fn anchored_places(
    document: &Document,
    args: &WriteArgs,
) -> Result<Vec<(f64, f64, String)>, String> {
    if args.after.is_empty() && args.below.is_empty() {
        return Ok(Vec::new());
    }
    let pages = document
        .layout(None)
        .map_err(|e| format!("could not work out where the words already are: {e}"))?;
    let lines = pages
        .get(args.page.saturating_sub(1))
        .map(|lines| lines.as_slice())
        .unwrap_or(&[]);
    let rows = onionskin::anchor::rows_from_lines(lines);
    if rows.is_empty() {
        // Naming the page matters: the usual cause is a --page that is not
        // the one the words are on, and "there is no text on this page" does
        // not tell somebody which page Onionskin looked at.
        return Err(format!(
            "there is nothing on page {} to place anything against. The \
             document has {} page{}.",
            args.page,
            document.pages,
            if document.pages == 1 { "" } else { "s" }
        ));
    }

    let gap_mm = onionskin::geometry::pt_to_mm(args.size * 0.3);
    let step_mm = onionskin::geometry::pt_to_mm(args.size * 1.15);

    // The same {today} the placed-by-millimetre route fills in. The anchor
    // itself is filled too: "Invoice {year}:" is a perfectly ordinary thing to
    // be looking for on a page.
    let today = today_row();

    let mut out = Vec::new();
    for (flag, put) in [
        (&args.after, onionskin::anchor::Where::After),
        (&args.below, onionskin::anchor::Where::Below),
    ] {
        for wanted in flag {
            let (anchor, text) = split_anchor(&onionskin::rows::fill(wanted, &today))?;
            let placed = onionskin::anchor::place_in(&rows, &anchor, put, gap_mm, step_mm)
                .map_err(|e| e.to_string())?;
            out.push((placed.x_mm, placed.y_mm, unescape(&text)));
        }
    }
    Ok(out)
}

/// Read `FILE:X,Y:WIDTH` or `FILE:X,Y:WIDTHxHEIGHT` into a placed picture.
///
/// The file name comes first and may itself contain colons — `C:\\scans\\a.png`
/// on Windows does — so the two that matter are found from the *end*: the
/// size after the last colon, the position after the one before it, and
/// whatever is left in front is the name.
///
/// Giving only a width is the ordinary case. The height then follows the
/// picture's own shape, because a signature squashed into a box it was not
/// drawn for is worse than no signature at all.
fn parse_image(spec: &str) -> Result<ImageSpec, String> {
    let bad = || {
        format!(
            "bad picture '{spec}'. Expected 'FILE:X,Y:WIDTH' — the file, where \
             its top-left corner goes in millimetres, and how wide it is:\n    \
             --image 'signature.png:120,240:40'"
        )
    };
    let (rest, size) = spec.rsplit_once(':').ok_or_else(bad)?;
    let (file, position) = rest.rsplit_once(':').ok_or_else(bad)?;
    if file.trim().is_empty() {
        return Err(bad());
    }

    let (x, y) = position.split_once(',').ok_or_else(bad)?;
    let x_mm: f64 = x.trim().parse().map_err(|_| bad())?;
    let y_mm: f64 = y.trim().parse().map_err(|_| bad())?;

    let (width, height) = match size.split_once(['x', 'X']) {
        Some((w, h)) => (w.trim(), Some(h.trim())),
        None => (size.trim(), None),
    };
    let width_mm: Option<f64> = if width.is_empty() {
        None
    } else {
        Some(width.parse().map_err(|_| bad())?)
    };
    let height_mm: Option<f64> = match height {
        Some(h) if !h.is_empty() => Some(h.parse().map_err(|_| bad())?),
        _ => None,
    };
    if width_mm.is_none() && height_mm.is_none() {
        return Err(bad());
    }
    for measure in [width_mm, height_mm].into_iter().flatten() {
        if !(measure.is_finite() && measure > 0.0) {
            return Err(format!(
                "a picture cannot be {measure} mm across. Give a size greater \
                 than nothing."
            ));
        }
    }
    Ok(ImageSpec {
        path: PathBuf::from(file),
        x_mm,
        y_mm,
        width_mm,
        height_mm,
    })
}

/// A `--image` as it was typed: which file, where its top-left corner goes,
/// and whichever of the two measurements were given.
#[derive(Debug, PartialEq)]
struct ImageSpec {
    path: PathBuf,
    x_mm: f64,
    y_mm: f64,
    /// `None` when only a height was given, and the width is to follow the
    /// picture's own shape.
    width_mm: Option<f64>,
    /// `None` when only a width was given, which is the ordinary case.
    height_mm: Option<f64>,
}

/// Load every `--image` and work out the box each one fills.
fn placed_images(specs: &[String], page: usize) -> Result<Vec<(usize, PlacedImage)>, String> {
    let mut out = Vec::new();
    for spec in specs {
        let ImageSpec {
            path,
            x_mm,
            y_mm,
            width_mm,
            height_mm,
        } = parse_image(spec)?;
        let picture = onionskin::picture::load(&path).map_err(|e| e.to_string())?;
        // Whichever measurement was left out follows the picture's own shape.
        let (width_mm, height_mm) = match (width_mm, height_mm) {
            (Some(w), Some(h)) => (w, h),
            (Some(w), None) => (w, w / picture.aspect()),
            (None, Some(h)) => (h * picture.aspect(), h),
            (None, None) => unreachable!("parse_image refuses both missing"),
        };
        out.push((
            page,
            PlacedImage {
                picture,
                x_mm,
                y_mm,
                width_mm,
                height_mm,
                rotation_deg: 0.0,
            },
        ));
    }
    Ok(out)
}

/// Split `ANCHOR:WORDS` into the two, on the first colon.
///
/// The same rule `add` uses, so the same flag means the same thing on both
/// commands. It also leaves a colon inside the *words* alone, which matters
/// for the times and ratios people write, and the anchor rarely needs its own
/// — matching forgives punctuation, so "Received" finds "Received:".
fn split_anchor(given: &str) -> Result<(String, String), String> {
    let (anchor, text) = given.split_once(':').ok_or_else(|| {
        format!(
            "bad placement '{given}'. Expected 'ANCHOR:the words' — the thing \
             already on the page, a colon, then what to add."
        )
    })?;
    if anchor.trim().is_empty() {
        return Err(format!("'{given}' does not say what to look for"));
    }
    if text.trim().is_empty() {
        return Err(format!("'{given}' does not say what to write"));
    }
    Ok((anchor.to_string(), text.to_string()))
}

fn cmd_show(args: ShowArgs) -> Result<ExitCode, String> {
    let document = Document::load(&args.document).map_err(|e| e.to_string())?;

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&document).map_err(|e| e.to_string())?
        );
        return Ok(ExitCode::SUCCESS);
    }

    println!(
        "{} — {}, {} page{}",
        args.document.display(),
        document.page.describe(),
        document.pages,
        if document.pages == 1 { "" } else { "s" }
    );
    if document.items.is_empty() {
        println!("\nNothing on it yet.");
        return Ok(ExitCode::SUCCESS);
    }

    for page in 1..=document.pages {
        let items: Vec<_> = document.on_page(page).collect();
        println!("\nPage {page}:");
        if items.is_empty() {
            println!("  (blank)");
            continue;
        }
        for item in items {
            let wrapped = item
                .width_mm
                .map(|w| format!(", wrapped at {w:.0} mm"))
                .unwrap_or_default();
            println!(
                "  {:>3}  {:>6.1},{:<6.1} {:>5.1} pt {}{}",
                item.id, item.x_mm, item.y_mm, item.size_pt, item.font, wrapped
            );
            println!("       {}", first_line(&item.text));
        }
    }

    if document.has_been_printed() {
        let added = document.added_since_printing().len();
        println!(
            "\nPrinted once. {} since then.",
            match added {
                0 => "Nothing added".to_string(),
                1 => "One piece of text added".to_string(),
                n => format!("{n} pieces of text added"),
            }
        );
        let problems = document.overlay_problems();
        if !problems.is_empty() {
            println!(
                "{} already on the sheet {} changed, so a delta cannot be printed.",
                problems.len(),
                if problems.len() == 1 { "has" } else { "have" }
            );
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_edit(args: EditArgs) -> Result<ExitCode, String> {
    let mut document = Document::load(&args.document).map_err(|e| e.to_string())?;
    if args.at.is_some() && args.by.is_some() {
        return Err("--at and --by both move the text; use one or the other".into());
    }

    let position = args.at.as_deref().map(parse_point).transpose()?;
    let nudge = args.by.as_deref().map(parse_point).transpose()?;

    {
        let item = document.get_mut(args.item).map_err(|e| e.to_string())?;
        if let Some(text) = &args.text {
            item.text = unescape(text);
        }
        if let Some((x, y)) = position {
            item.x_mm = x;
            item.y_mm = y;
        }
        if let Some((dx, dy)) = nudge {
            item.x_mm += dx;
            item.y_mm += dy;
        }
        if let Some(page) = args.page {
            item.page = page;
        }
        if let Some(size) = args.size {
            item.size_pt = size;
        }
        if let Some(font) = &args.font {
            item.font = font.clone();
        }
        if let Some(width) = args.width {
            item.width_mm = Some(width);
        }
        if args.no_width {
            item.width_mm = None;
        }
        if let Some(rotation) = args.rotation {
            item.rotation_deg = rotation;
        }
        if let Some(colour) = &args.colour {
            item.colour = colour.clone();
        }
        if let Some(leading) = args.leading {
            item.leading = leading;
        }
    }
    if let Some(page) = args.page {
        document.pages = document.pages.max(page);
    }
    document.save(&args.document).map_err(|e| e.to_string())?;

    let item = document.get(args.item).expect("just edited");
    println!(
        "{}: page {}, {:.1},{:.1} mm — {}",
        item.id,
        item.page,
        item.x_mm,
        item.y_mm,
        first_line(&item.text)
    );
    warn_off_the_page(&document);
    report_overlay_problems(&document, false);
    Ok(ExitCode::SUCCESS)
}

fn cmd_erase(args: EraseArgs) -> Result<ExitCode, String> {
    let mut document = Document::load(&args.document).map_err(|e| e.to_string())?;
    let gone = document.remove(args.item).map_err(|e| e.to_string())?;
    document.save(&args.document).map_err(|e| e.to_string())?;

    println!("{}: erased — {}", gone.id, first_line(&gone.text));
    report_overlay_problems(&document, false);
    Ok(ExitCode::SUCCESS)
}

/// Where a result goes when nobody says where.
///
/// Beside the file it came from, named after it, with what it is on the end.
/// That is where a person looks for it and roughly what they would have called
/// it — and it removes the one flag that had to be typed every single time.
fn beside(source: &Path, tail: &str, extension: &str) -> PathBuf {
    let stem = source
        .file_stem()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "onionskin".to_string());
    // A bare file name has an *empty* parent rather than none, and joining
    // onto an empty path gives back a bare name — so "scan.png" becomes
    // "scan-delta.pdf" and not "./scan-delta.pdf".
    let folder = source.parent().unwrap_or(Path::new(""));
    folder.join(format!("{stem}{tail}.{extension}"))
}

/// Open a finished file, if asked and if there is anything here to open it.
fn open_if_asked(asked: bool, path: &Path) {
    if !asked {
        return;
    }
    if !onionskin::install::open_with_desktop(path) {
        // Not an error. There is no desktop on a server or over SSH, and the
        // path has just been printed anyway.
        eprintln!(
            "note: nothing on this machine would open {}",
            path.display()
        );
    }
}

fn cmd_print(args: PrintArgs) -> Result<ExitCode, String> {
    let mut document = Document::load(&args.document).map_err(|e| e.to_string())?;
    let output = args.output.clone().unwrap_or_else(|| {
        beside(
            &args.document,
            if args.delta { "-delta" } else { "" },
            "pdf",
        )
    });
    refuse_to_clobber(&output, "PDF", &[(&args.document, "document")])?;
    check_writable(&output, "PDF")?;

    let font = load_font(args.font_file.as_deref(), args.font_index)?;

    let drawings = if args.delta {
        document.shape_layout(&document.shapes_added_since_printing())
    } else {
        document.shape_layout(&document.shapes.iter().collect::<Vec<_>>())
    };

    let pages = if args.delta {
        let problems = document.overlay_problems();
        if !problems.is_empty() && !args.force {
            for problem in &problems {
                eprintln!("{}\n", problem.format());
            }
            eprintln!(
                "Nothing was written. Print the whole document instead (drop --delta), \
                 or --force if you know what you are doing."
            );
            return Ok(ExitCode::from(2));
        }
        if !document.has_been_printed() {
            eprintln!(
                "note: this document has not been printed yet, so the delta is all \
                 of it. Add --printed when you print it, and a later --delta will \
                 carry only what you added afterwards."
            );
        }
        document.delta_layout(font.as_ref())
    } else {
        document.layout(font.as_ref())
    }
    .map_err(|e| e.to_string())?;

    let written: usize = pages.iter().map(|p| p.len()).sum();
    let drawn: usize = drawings.iter().map(|p| p.len()).sum();
    if written == 0 && drawn == 0 {
        eprintln!(
            "note: nothing to print — the {} is empty.",
            if args.delta { "delta" } else { "document" }
        );
    }

    onionskin::pdf::write_page_content(
        &output,
        &document.page_sizes(),
        &pages,
        &drawings,
        "Onionskin document",
        font.as_ref(),
    )
    .map_err(|e| e.to_string())?;

    println!(
        "{}: {} page{}, {written} line{}{}.",
        output.display(),
        document.pages,
        if document.pages == 1 { "" } else { "s" },
        if written == 1 { "" } else { "s" },
        match drawn {
            0 => String::new(),
            1 => ", 1 drawing".to_string(),
            n => format!(", {n} drawings"),
        }
    );

    if args.printed {
        document.mark_printed();
        document.save(&args.document).map_err(|e| e.to_string())?;
        println!(
            "Noted as printed. Add more words, then:\n  onionskin print {} --delta",
            args.document.display()
        );
    }
    if args.delta {
        println!("\n{PRINT_INSTRUCTIONS}");
    }
    open_if_asked(args.open, &output);
    Ok(ExitCode::SUCCESS)
}

/// Say when text has been placed off the paper, without refusing it.
///
/// Refusing would be wrong: someone may be laying a page out and mean to move
/// it in a moment. Saying nothing would be worse — it prints blank and the
/// reason is invisible.
fn warn_off_the_page(document: &Document) {
    for item in &document.items {
        let page = document.page;
        if item.x_mm < 0.0
            || item.y_mm < 0.0
            || item.x_mm > page.width_mm
            || item.y_mm > page.height_mm
        {
            eprintln!(
                "warning: item {} sits at {:.1},{:.1} mm, which is off a {} sheet. \
                 It will not print.",
                item.id,
                item.x_mm,
                item.y_mm,
                page.describe()
            );
        }
    }
}

fn report_overlay_problems(document: &Document, verbose: bool) {
    let problems = document.overlay_problems();
    if problems.is_empty() {
        return;
    }
    if verbose {
        for problem in &problems {
            eprintln!("{}\n", problem.format());
        }
    }
    eprintln!(
        "note: {} piece{} of text already on the printed sheet {} changed. \
         A delta cannot undo ink, so that page has to be printed fresh.",
        problems.len(),
        if problems.len() == 1 { "" } else { "s" },
        if problems.len() == 1 { "has" } else { "have" }
    );
}

/// Load a font file, if one was named.
fn load_font(path: Option<&Path>, index: u32) -> Result<Option<EmbeddedFont>, String> {
    match path {
        Some(path) => EmbeddedFont::load_indexed(path, index)
            .map(Some)
            .map_err(|e| e.to_string()),
        None => Ok(None),
    }
}

/// `\n` typed at a shell prompt, meant as a line break.
fn unescape(text: &str) -> String {
    text.replace("\\n", "\n").replace("\\t", "\t")
}

fn first_line(text: &str) -> String {
    let first = text.lines().next().unwrap_or("");
    let shown: String = first.chars().take(56).collect();
    let more = if shown.chars().count() < first.chars().count() || text.contains('\n') {
        " …"
    } else {
        ""
    };
    format!("{shown}{more}")
}

fn parse_point(text: &str) -> Result<(f64, f64), String> {
    let (x, y) = text
        .split_once(',')
        .ok_or_else(|| format!("'{text}' should be two numbers: 'X,Y' in millimetres"))?;
    let number = |part: &str, which: &str| -> Result<f64, String> {
        part.trim()
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite())
            .ok_or_else(|| format!("'{}' is not a {which} position in millimetres", part.trim()))
    };
    Ok((number(x, "horizontal")?, number(y, "vertical")?))
}

// ---------------------------------------------------------------------------
// Reading the letters off a scan
// ---------------------------------------------------------------------------

fn cmd_read(args: ReadArgs) -> Result<ExitCode, String> {
    let page = parse_page(&args.page).map_err(|e| e.to_string())?;
    let image = image::open(&args.scan)
        .map_err(|e| format!("could not read '{}': {e}", args.scan.display()))?;

    let options = ScanOptions {
        page,
        assume_cropped: args.cropped,
        assume_square: args.square,
        ..ScanOptions::new(page)
    };
    let registration = register(&image, options).map_err(|e| e.to_string())?;
    let gray = image.to_luma8();

    let font = load_font(args.font_file.as_deref(), args.font_index)?;

    // Without a font named, work out what the page is set in and read it with
    // that. Asking somebody to name the font a scan was set in is asking a
    // question the page can answer: the reader tries each face it has, and the
    // one that accounts for the most ink is the one the page is in. Naming a
    // font is still how you read an alphabet the built-in faces do not cover,
    // which is why the option stays.
    let mut matched: Option<String> = None;
    let text = match &font {
        Some(font) => letters::read_with_font(
            &gray,
            &registration,
            &letters::ReadOptions::default(),
            font,
            args.letters.as_deref(),
        )
        .map_err(|e| e.to_string())?,
        None => match onionskin::typeface::read_and_match(
            &args.scan,
            &args.page,
            args.cropped,
            args.square,
        ) {
            Some((text, found)) => {
                matched = Some(match found {
                    Some(face) => face.describe(),
                    None => "read against the faces on this machine; the page did \
                             not say clearly which one it is set in"
                        .to_string(),
                });
                text
            }
            // Nothing on this machine to read against. Where every letter is
            // is still worth having, and is what this used to give always.
            None => letters::read(&gray, &registration, &letters::ReadOptions::default())
                .map_err(|e| e.to_string())?,
        },
    };

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&text).map_err(|e| e.to_string())?
        );
        return Ok(ExitCode::SUCCESS);
    }

    println!("{}", registration.describe());
    println!(
        "\n{} letter{} in {} word{} on {} line{}.",
        text.letter_count(),
        if text.letter_count() == 1 { "" } else { "s" },
        text.word_count(),
        if text.word_count() == 1 { "" } else { "s" },
        text.lines.len(),
        if text.lines.len() == 1 { "" } else { "s" }
    );
    match (&font, &matched) {
        (Some(_), _) => {}
        (None, Some(said)) => println!("Read automatically: {said}"),
        (None, None) => println!(
            "Nothing on this machine to read the letters against, so this is where \
             they are\nrather than which they are. Install a common face — DejaVu, \
             Liberation — or pass\n--font-file with the font the page was set in."
        ),
    }

    // The words are there whether the font was named or worked out, and the
    // second case is the common one now — gating this on a font file having
    // been passed would have read the page and then declined to show it.
    let letters_were_read = font.is_some() || matched.is_some();
    for line in &text.lines {
        println!(
            "\n  {:>6.1} mm  ({:.1}–{:.1} mm across)",
            line.baseline_mm,
            line.rect.x_mm,
            line.rect.right_mm()
        );
        if letters_were_read {
            println!("     {}", line.text_lossy());
        }
    }
    if text.discarded > 0 {
        println!(
            "\n{} mark{} set aside as too small or too large to be a letter.",
            text.discarded,
            if text.discarded == 1 { "" } else { "s" }
        );
    }

    if let Some(destination) = &args.to {
        export_page(&text, page, destination, args.flow, letters_were_read)?;
        open_if_asked(args.open, destination);
    }
    Ok(ExitCode::SUCCESS)
}

/// Turn what was read off the scan into something with a cursor in it.
fn export_page(
    text: &letters::PageText,
    page: PageSize,
    destination: &Path,
    flow: bool,
    read_the_letters: bool,
) -> Result<(), String> {
    use onionskin::office::{self, Format, Layout};

    if !read_the_letters {
        return Err(format!(
            "I can see where the ink is but not what it says, so there is \
             nothing to write into {}.\n    Pass --font-file with the font the \
             page was set in, and run it again.",
            destination.display()
        ));
    }
    check_writable(destination, "document")?;

    // Every line becomes one piece of text, at the millimetre it was found.
    let document = office::document_from_page(text, page).map_err(|e| e.to_string())?;
    let layout = if flow { Layout::Flow } else { Layout::Placed };
    match Format::of_path(destination) {
        Some(format) => {
            let bytes = office::write(&document, format, layout).map_err(|e| e.to_string())?;
            std::fs::write(destination, bytes)
                .map_err(|e| format!("could not write {}: {e}", destination.display()))?;
            println!(
                "\n{}: {} — {} line{}, {}.",
                destination.display(),
                format.describe(),
                document.items.len(),
                if document.items.len() == 1 { "" } else { "s" },
                if flow {
                    "as paragraphs"
                } else {
                    "each where it was on the paper"
                }
            );
            println!("Open it in Word or LibreOffice and edit it like anything else.");
        }
        None => {
            // An Onionskin document, which is what everything else here takes.
            document.save(destination).map_err(|e| e.to_string())?;
            println!(
                "\n{}: an Onionskin document — {} line{}.",
                destination.display(),
                document.items.len(),
                if document.items.len() == 1 { "" } else { "s" }
            );
            println!(
                "  onionskin show {}\n  onionskin write {} --at '20,150:and this'",
                destination.display(),
                destination.display()
            );
        }
    }
    Ok(())
}

fn cmd_scanners() -> Result<ExitCode, String> {
    // Anything attached to this machine, through SANE. An error here means no
    // scanning tool is installed, which is worth saying once at the end rather
    // than instead of the network scanners that may well be there — but it
    // must still be said. Dropping it left a machine with no SANE on it being
    // told to check the scanner was switched on.
    let (attached, no_tool) = match list_devices() {
        Ok(devices) => (devices, None),
        Err(why) => (Vec::new(), Some(why.to_string())),
    };
    if !attached.is_empty() {
        println!("Attached to this machine:");
        for device in &attached {
            println!("  {}", device.description);
            println!("    --device {}", device.name);
        }
    }

    if attached.is_empty() {
        println!("Looking for scanners…");
    }
    let network = discover::scanners(discover::LISTEN_FOR);
    if !network.is_empty() {
        println!("\nAnnouncing themselves on this network:");
        for found in &network {
            println!("\n  {}", found.name);
            if let Some(model) = found.model() {
                println!("    {model}");
            }
            println!("    --scanner {}", found.uri);
        }
    }

    if attached.is_empty() && network.is_empty() {
        println!("No scanners found.");
        match &no_tool {
            // The tool is missing, so nothing attached could have been found
            // whatever is plugged in. Say that, rather than sending somebody
            // to check a cable that is fine.
            Some(why) => println!("\n{why}"),
            None => println!(
                "\nCheck it is switched on, and plugged in or on this network. Onionskin\n\
                 found SANE's 'scanimage' but it reported no scanner attached.\n\
                 You can also scan with any program you like and pass the image file\n\
                 instead."
            ),
        }
        return Ok(ExitCode::from(1));
    }
    Ok(ExitCode::SUCCESS)
}

fn cmd_acquire(args: AcquireArgs) -> Result<ExitCode, String> {
    // Nothing has been scanned yet, so a bad destination costs nothing to
    // catch here and a whole scan to discover later.
    check_writable(&args.output, "scan")?;
    parse_page(&args.page)?;
    if !(72..=2400).contains(&args.resolution) {
        return Err(format!(
            "{} dpi is outside what a scanner will do (72 to 2400)",
            args.resolution
        ));
    }
    // Before telling anyone how to lay the sheet on the glass.
    if !scanning_available() {
        return Err(unavailable_reason());
    }

    println!("{PLACEMENT_ADVICE}\n");
    println!("Scanning at {} dpi…", args.resolution);

    let options = AcquireOptions {
        device: args.device,
        resolution: args.resolution,
        colour: args.colour,
    };
    let path = acquire(&options, &args.output).map_err(|e| e.to_string())?;
    println!("Wrote {}", path.display());

    // Say straight away whether the scan is usable, rather than letting the
    // trouble surface after the sheet has been taken off the glass.
    match load_registration(&path, &args.page, false, false) {
        Ok((registration, dimensions)) => {
            println!("  image      : {} × {} px", dimensions.0, dimensions.1);
            println!("  paper      : {}", registration.page.describe());
            println!("  resolution : {:.0} dpi", registration.dpi());
            println!("  skew       : {:+.2}°", registration.skew_deg);
            println!(
                "\nNow pick a spot on it and add your words:\n  \
                 onionskin add {} --at 'X,Y:the words'",
                path.display()
            );
            open_if_asked(args.open, &path);
            Ok(ExitCode::SUCCESS)
        }
        Err(message) => {
            println!(
                "\nThe scan was saved, but Onionskin cannot measure the sheet in it:\n  \
                 {message}\n\nThe sheet is still on the glass — it is usually quicker to \
                 fix the placement\nand scan again than to work around it."
            );
            // Still worth opening: seeing the scan is how somebody works out
            // what went wrong with it.
            open_if_asked(args.open, &path);
            Ok(ExitCode::from(1))
        }
    }
}

fn load_registration(
    scan: &Path,
    page_spec: &str,
    cropped: bool,
    square: bool,
) -> Result<(ScanRegistration, (u32, u32)), String> {
    let page = parse_page(page_spec)?;
    if !scan.is_file() {
        return Err(format!("no such file: {}", scan.display()));
    }
    let image = image::open(scan).map_err(|e| format!("could not read {}: {e}", scan.display()))?;
    let dimensions = (image.width(), image.height());

    let mut options = ScanOptions::new(page);
    options.assume_cropped = cropped;
    options.assume_square = square;

    let registration = register(&image, options).map_err(|e| e.to_string())?;
    Ok((registration, dimensions))
}

fn cmd_inspect(args: InspectArgs) -> Result<ExitCode, String> {
    let (registration, dimensions) =
        load_registration(&args.scan, &args.page, args.cropped, args.square)?;

    println!("{}", args.scan.display());
    println!("  image        : {} × {} px", dimensions.0, dimensions.1);
    println!("  paper        : {}", registration.page.describe());
    println!("  resolution   : {:.0} dpi", registration.dpi());
    println!(
        "  sheet corner : ({:.0}, {:.0}) px",
        registration.origin_px.0, registration.origin_px.1
    );
    println!("  skew         : {:+.2}°", registration.skew_deg);

    if registration.skew_deg.abs() > 2.0 {
        println!(
            "\nnote: the sheet is quite crooked in this scan. Onionskin corrects for it,\n\
             but a straighter scan leaves less to correct."
        );
    }
    Ok(ExitCode::SUCCESS)
}

/// Parse `X,Y:the words` into a position and its text.
fn parse_placement(spec: &str) -> Result<((f64, f64), String), String> {
    let (position, text) = spec.split_once(':').ok_or_else(|| {
        format!("bad placement '{spec}'. Expected 'X,Y:the words', e.g. '60,150:Approved'")
    })?;
    if text.trim().is_empty() {
        return Err(format!("the placement '{spec}' has no words in it"));
    }
    let (x, y) = position
        .split_once(',')
        .ok_or_else(|| format!("bad position in '{spec}'. Expected 'X,Y'"))?;
    let x: f64 = x
        .trim()
        .parse()
        .map_err(|_| format!("'{}' is not a number, in '{spec}'", x.trim()))?;
    let y: f64 = y
        .trim()
        .parse()
        .map_err(|_| format!("'{}' is not a number, in '{spec}'", y.trim()))?;
    if !(x.is_finite() && y.is_finite()) {
        return Err(format!("the position in '{spec}' is not a real number"));
    }
    Ok(((x, y), text.to_string()))
}

/// A pair of sizes: `WIDTHxHEIGHT`, or one number meaning both.
///
/// One number for a circle is the common case — `--circle '105,150:20'` reads
/// better than making somebody write the radius twice.
fn parse_size(spec: &str) -> Result<(f64, f64), String> {
    let spec = spec.trim();
    let (w, h) = match spec.split_once(['x', 'X', '*']) {
        Some(pair) => pair,
        None => (spec, spec),
    };
    let number = |text: &str| -> Result<f64, String> {
        text.trim()
            .parse::<f64>()
            .ok()
            .filter(|v| v.is_finite())
            .ok_or_else(|| format!("'{}' is not a size in millimetres", text.trim()))
    };
    Ok((number(w)?, number(h)?))
}

/// Where something goes and how big it is, both in millimetres.
type PlacedSize = ((f64, f64), (f64, f64));

/// `X,Y:WxH` — where it goes, and how big it is.
fn parse_placed_size(spec: &str, what: &str, example: &str) -> Result<PlacedSize, String> {
    let (position, size) = spec
        .split_once(':')
        .ok_or_else(|| format!("bad {what} '{spec}'. Expected '{example}'"))?;
    Ok((parse_point(position)?, parse_size(size)?))
}

fn cmd_draw(args: DrawArgs) -> Result<ExitCode, String> {
    use onionskin::document::{Shape, ShapeKind};

    if args.line.is_empty() && args.boxes.is_empty() && args.circles.is_empty() && args.paths.is_empty()
    {
        return Err("nothing to draw. Say what to draw, for example:\n    \
             --line '20,100:190,100'        a rule across the page\n    \
             --box '20,40:80x30'            a box 80 by 30 mm\n    \
             --circle '105,150:20'          a circle of radius 20 mm\n    \
             --path '20,20 60,50 100,20'    a run of points"
            .into());
    }
    if args.no_outline && args.fill.is_none() {
        return Err("--no-outline with no --fill would draw nothing at all.".into());
    }

    // Checked here so a bad colour is reported once, against the word that was
    // typed, rather than four times against each shape it was applied to.
    let stroke = if args.no_outline {
        None
    } else {
        onionskin::document::parse_colour(&args.colour).map_err(|e| e.to_string())?;
        Some(args.colour.clone())
    };
    if let Some(fill) = &args.fill {
        onionskin::document::parse_colour(fill).map_err(|e| e.to_string())?;
    }
    let dash = match &args.dash {
        Some(spec) => {
            let (on, off) = parse_point(spec)
                .map_err(|_| format!("bad dash '{spec}'. Expected 'DASH,GAP', for example '2,1'"))?;
            if on <= 0.0 || off < 0.0 {
                return Err(format!("bad dash '{spec}': the dash must be longer than nothing"));
            }
            Some((on, off))
        }
        None => None,
    };

    let mut kinds: Vec<ShapeKind> = Vec::new();
    for spec in &args.line {
        let (from, to) = spec
            .split_once(':')
            .ok_or_else(|| format!("bad line '{spec}'. Expected 'X1,Y1:X2,Y2'"))?;
        let (x1_mm, y1_mm) = parse_point(from)?;
        let (x2_mm, y2_mm) = parse_point(to)?;
        kinds.push(ShapeKind::Line {
            x1_mm,
            y1_mm,
            x2_mm,
            y2_mm,
        });
    }
    for spec in &args.boxes {
        let ((x_mm, y_mm), (width_mm, height_mm)) =
            parse_placed_size(spec, "box", "X,Y:WIDTHxHEIGHT")?;
        kinds.push(ShapeKind::Rect {
            x_mm,
            y_mm,
            width_mm,
            height_mm,
            radius_mm: args.radius,
        });
    }
    for spec in &args.circles {
        let ((x_mm, y_mm), (radius_x_mm, radius_y_mm)) =
            parse_placed_size(spec, "circle", "X,Y:RADIUS")?;
        kinds.push(ShapeKind::Ellipse {
            x_mm,
            y_mm,
            radius_x_mm,
            radius_y_mm,
        });
    }
    for spec in &args.paths {
        let mut points = Vec::new();
        for part in spec.split_whitespace() {
            points.push(parse_point(part)?);
        }
        if points.len() < 2 {
            return Err(format!(
                "the path '{spec}' has {} point(s); it takes two to draw a line",
                points.len()
            ));
        }
        kinds.push(ShapeKind::Path {
            points,
            closed: args.close,
        });
    }

    // Anything Onionskin can open can be drawn on, not only its own documents.
    // Somebody ringing a figure on a statement should not first have to convert
    // their file into a format they have never heard of.
    if is_document(&args.document) {
        let shapes: Vec<Shape> = kinds
            .into_iter()
            .map(|kind| Shape {
                id: 0,
                page: args.page,
                kind,
                stroke: stroke.clone(),
                fill: args.fill.clone(),
                width_mm: args.width,
                dash_mm: dash,
            })
            .collect();
        return draw_on_document(&args, &shapes);
    }

    let mut document = Document::load(&args.document).map_err(|e| e.to_string())?;
    let mut drawn = Vec::new();
    for kind in kinds {
        let shape = Shape {
            id: 0,
            page: args.page,
            kind,
            stroke: stroke.clone(),
            fill: args.fill.clone(),
            width_mm: args.width,
            dash_mm: dash,
        };
        drawn.push(document.draw(shape).map_err(|e| e.to_string())?);
    }
    document.save(&args.document).map_err(|e| e.to_string())?;

    for id in &drawn {
        let shape = document
            .shapes
            .iter()
            .find(|s| s.id == *id)
            .expect("just drawn");
        let (x0, y0, x1, y1) = shape.bounds();
        println!(
            "{id}: page {}, {} — {:.1},{:.1} to {:.1},{:.1} mm",
            shape.page,
            shape.describe(),
            x0,
            y0,
            x1,
            y1
        );
    }
    warn_drawings_off_the_page(&document);
    Ok(ExitCode::SUCCESS)
}

/// Say so when a drawing runs off the paper, which prints as a cut-off edge.
fn warn_drawings_off_the_page(document: &Document) {
    let page = document.page;
    for shape in &document.shapes {
        let (x0, y0, x1, y1) = shape.bounds();
        if x0 < 0.0 || y0 < 0.0 || x1 > page.width_mm || y1 > page.height_mm {
            eprintln!(
                "warning: drawing {} runs off the paper ({:.1},{:.1} to {:.1},{:.1} mm on a \
                 {:.0}×{:.0} mm sheet). It will print with its edge cut off.",
                shape.id, x0, y0, x1, y1, page.width_mm, page.height_mm
            );
        }
    }
}

fn cmd_add(args: AddArgs) -> Result<ExitCode, String> {
    // Onionskin's own document is neither a scan nor a file to print onto, so
    // it belongs on neither path below. Falling through to the scan one told
    // people their document "was not recognized as an image format", which is
    // true and no help at all.
    if Document::is_one(&args.scan) {
        return Err(format!(
            "{} is an Onionskin document, so there is a better way than a \
             delta measured off a scan.\n    Add the words:  onionskin write \
             {} --at '25,40:the words'\n    Then print only those:  onionskin \
             print {} -o delta.pdf --delta",
            args.scan.display(),
            args.scan.display(),
            args.scan.display()
        ));
    }
    // A PDF or a Word file is a document, not a photograph of one: it already
    // knows its own page size and needs no registering. Only the scanned-image
    // path has a sheet to find.
    if is_document(&args.scan) {
        return add_to_document(args);
    }
    if args.at_scan.is_empty()
        && args.at_page.is_empty()
        && args.after.is_empty()
        && args.below.is_empty()
    {
        return Err(
            "nothing to add. Say where the words go — easiest first:\n    \
             --after 'Received:Approved 27 July'   just after something on the page\n    \
             --below 'Signature:J. Bezzina'        one line under it\n    \
             --at-mm '45,63:Approved'              millimetres measured on the paper\n    \
             --at '620,870:Approved'               pixels read off the scan"
                .into(),
        );
    }
    if let Some(size) = args.size {
        if !(size.is_finite() && size > 0.0 && size <= 400.0) {
            return Err(format!("type size {size} pt is out of range (1 to 400)"));
        }
    }
    if !args.rotation.is_finite() {
        return Err("rotation must be a real number".into());
    }
    // A supplied font wins: asking for one and silently getting Helvetica is
    // how the Python side once made --font-file appear to do nothing.
    let embedded = match &args.font_file {
        Some(path) => {
            Some(EmbeddedFont::load_indexed(path, args.font_index).map_err(|e| e.to_string())?)
        }
        None => None,
    };

    // Nothing said, so read it off the page. Somebody adding a line to a form
    // wants it to look like the rest of the form, and the page itself knows
    // what the rest of the form is set in — asking them to name a font is
    // asking a question the program can answer.
    //
    // The same read serves both questions. Placing words after something
    // already printed needs the words; matching the font needs the
    // measurements; reading the page is the expensive part and is done once.
    let asked = args.font.as_deref().map(str::to_string);
    let anchoring = !args.after.is_empty() || !args.below.is_empty();
    let want_font = !(embedded.is_some()
        || args.no_match_font
        || (asked.is_some() && args.size.is_some()));
    let read = if anchoring || want_font {
        onionskin::typeface::read_and_match(&args.scan, &args.page, args.cropped, args.square)
    } else {
        None
    };
    if anchoring && read.is_none() {
        return Err(
            "nothing could be read off this page, so there is nothing to place words \
             against.\nRun 'onionskin read <scan>' to see what it can make out, or \
             give millimetres with --at-mm."
                .into(),
        );
    }
    let (page_text, matched) = match read {
        Some((text, found)) => (Some(text), found.filter(|_| want_font)),
        None => (None, None),
    };
    if let Some(found) = &matched {
        println!("Matched the page: {}", found.describe());
        println!("  Say --font or --size to choose for yourself.\n");
    }

    let chosen = asked.unwrap_or_else(|| {
        matched
            .map(|found| found.font.base_name().to_string())
            .unwrap_or_else(|| "Helvetica".to_string())
    });
    let size = args
        .size
        .or_else(|| matched.map(|found| found.size_pt))
        .unwrap_or(11.0);

    let line_font = match &embedded {
        Some(_) => LineFont::Embedded,
        None => LineFont::Builtin(Font::parse(&chosen).ok_or_else(|| {
            let names: Vec<&str> = Font::all().iter().map(|f| f.base_name()).collect();
            format!(
                "unknown font '{}'. Available: {}\n\
                 For another alphabet, pass --font-file with a .ttf.",
                chosen,
                names.join(", ")
            )
        })?),
    };

    // Check the destinations before doing any work, so a mistake costs a
    // message rather than the scan.
    let output = args
        .output
        .clone()
        .unwrap_or_else(|| beside(&args.scan, "-delta", "pdf"));
    refuse_to_clobber(&output, "delta", &[(&args.scan, "scan")])?;
    check_writable(&output, "delta")?;
    if let Some(preview) = &args.preview {
        refuse_to_clobber(
            preview,
            "proof",
            &[(&args.scan, "scan"), (&output, "delta")],
        )?;
        check_writable(preview, "proof")?;
    }

    let (registration, _) = load_registration(&args.scan, &args.page, args.cropped, args.square)?;
    let page: PageSize = registration.page;

    // Words follow the paper's edges by default. The sheet's skew is the
    // scanner's doing, not the printer's — the sheet itself is straight, so
    // copying its apparent tilt would print crooked text onto a straight page.
    let base_rotation = if args.follow_skew {
        args.rotation + registration.skew_deg
    } else {
        args.rotation
    };

    let mut lines: Vec<PlacedLine> = Vec::new();
    let mut placements: Vec<((f64, f64), String)> = Vec::new();

    for spec in &args.at_scan {
        let (pixel, text) = parse_placement(spec)?;
        placements.push((registration.pixel_to_page_mm(pixel), text));
    }
    for spec in &args.at_page {
        placements.push(parse_placement(spec)?);
    }

    // Anchored placements: found on the page rather than measured onto it.
    if !args.after.is_empty() || !args.below.is_empty() {
        let text = page_text.as_ref().expect("checked above");
        // A space's worth of room after the anchor, and a line's worth below
        // it, both taken from the size the new words will be set at.
        let gap_mm = onionskin::geometry::pt_to_mm(size * 0.3);
        let step_mm = onionskin::geometry::pt_to_mm(size * 1.15);
        for (specs, put) in [
            (&args.after, onionskin::anchor::Where::After),
            (&args.below, onionskin::anchor::Where::Below),
        ] {
            for spec in specs {
                let (anchor, words) = spec.split_once(':').ok_or_else(|| {
                    format!(
                        "bad placement '{spec}'. Expected 'ANCHOR:the words' — the \
                         thing already on the page, a colon, then what to add."
                    )
                })?;
                let found = onionskin::anchor::place(text, anchor, put, gap_mm, step_mm)
                    .map_err(|e| e.to_string())?;
                println!(
                    "Found \"{}\" on the line: {}\n  putting the words at {:.1}, {:.1} mm",
                    anchor.trim(),
                    found.line,
                    found.x_mm,
                    found.y_mm
                );
                placements.push(((found.x_mm, found.y_mm), unescape(words)));
            }
        }
        println!();
    }

    for (position_mm, text) in &placements {
        for (index, part) in text.split("\\n").enumerate() {
            if part.trim().is_empty() {
                continue;
            }
            // Successive lines step down by the type size; the y given is the
            // baseline of the first.
            let step = onionskin::geometry::pt_to_mm(size * 1.15) * index as f64;
            lines.push(PlacedLine {
                text: part.to_string(),
                x_mm: position_mm.0,
                y_mm: position_mm.1 + step,
                size_pt: size,
                font: line_font,
                rotation_deg: base_rotation,
                colour: (0.0, 0.0, 0.0),
            });
        }
    }

    if lines.is_empty() {
        return Err("every placement was blank, so the delta would print nothing".into());
    }

    write_delta(
        &output,
        &[page],
        &[lines.clone()],
        "Onionskin delta",
        embedded.as_ref(),
    )
    .map_err(|message| {
        // Point at a font that is actually on this machine, rather than
        // leaving someone to hunt for one.
        let text = message.to_string();
        match (
            text.contains("cannot write these characters"),
            suggest_system_font(),
        ) {
            (true, Some(path)) => format!(
                "{text}\n    There is one on this machine: --font-file {}",
                path.display()
            ),
            _ => text,
        }
    })?;

    println!("Wrote {}", output.display());
    println!("  paper      : {}", page.describe());
    if let Some(font) = &embedded {
        println!(
            "  font       : {} embedded ({} KB)",
            font.name,
            font.program().len() / 1024
        );
    }
    println!(
        "  scan       : {:.0} dpi, sheet turned {:+.2}°",
        registration.dpi(),
        registration.skew_deg
    );
    println!("  additions  : {}", lines.len());
    for line in &lines {
        println!(
            "    \"{}\" at ({:.1}, {:.1}) mm",
            truncate(&line.text, 40),
            line.x_mm,
            line.y_mm
        );
    }

    // Off the sheet entirely and merely close to the edge are different
    // problems: one prints nothing at all, the other prints and may be clipped.
    let off_page = lines
        .iter()
        .filter(|line| {
            line.x_mm < 0.0
                || line.y_mm < 0.0
                || line.x_mm > page.width_mm
                || line.y_mm > page.height_mm
        })
        .count();
    if off_page > 0 {
        println!(
            "\nWARNING: {off_page} addition(s) fall outside the {} sheet altogether,\n\
             so nothing of them will be printed. Check the coordinates.",
            page.describe()
        );
    }

    let near_edge = lines
        .iter()
        .filter(|line| {
            line.x_mm >= 0.0
                && line.y_mm >= 0.0
                && line.x_mm <= page.width_mm
                && line.y_mm <= page.height_mm
                && (line.x_mm < args.margin
                    || line.y_mm < args.margin
                    || line.x_mm > page.width_mm - args.margin
                    || line.y_mm > page.height_mm - args.margin)
        })
        .count();
    if near_edge > 0 {
        println!(
            "\nWARNING: {near_edge} addition(s) sit within {} mm of the edge. Most\n\
             printers cannot place ink there and will clip it.",
            args.margin
        );
    }

    if let Some(preview_path) = &args.preview {
        write_preview(&args.scan, &registration, &lines, preview_path)?;
        println!("\n  proof      : {}", preview_path.display());
    }

    println!("\n{PRINT_INSTRUCTIONS}");
    open_if_asked(args.open, &output);
    Ok(ExitCode::SUCCESS)
}

fn truncate(text: &str, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text.to_string();
    }
    let kept: String = text.chars().take(limit).collect();
    format!("{kept}…")
}

/// Mark on the scan where each addition will land.
///
/// The proof is the thing that actually saves paper: it shows the new words
/// against what is already on the sheet, so a mistake costs a glance rather
/// than a page.
fn write_preview(
    scan: &Path,
    registration: &ScanRegistration,
    lines: &[PlacedLine],
    out: &Path,
) -> Result<(), String> {
    let mut image = image::open(scan)
        .map_err(|e| format!("could not re-read the scan: {e}"))?
        .to_rgb8();
    let (width, height) = (image.width() as i64, image.height() as i64);

    let mark = image::Rgb([214u8, 51, 51]);
    let arm = (registration.px_per_mm * 3.0).round() as i64;
    let thickness = ((registration.px_per_mm * 0.35).round() as i64).max(1);

    for line in lines {
        let (cx, cy) = registration.page_mm_to_pixel((line.x_mm, line.y_mm));
        let (cx, cy) = (cx.round() as i64, cy.round() as i64);

        let mut put = |x: i64, y: i64| {
            if x >= 0 && y >= 0 && x < width && y < height {
                image.put_pixel(x as u32, y as u32, mark);
            }
        };

        for step in -arm..=arm {
            for t in 0..thickness {
                put(cx + step, cy + t);
                put(cx + t, cy + step);
            }
        }
        // A short bar along the baseline, so the direction is visible too.
        let baseline = (registration.px_per_mm * 12.0) as i64;
        for step in 0..baseline {
            put(cx + step, cy + thickness);
        }
    }

    image
        .save(out)
        .map_err(|e| format!("could not write the proof image: {e}"))
}

// ---------------------------------------------------------------------------
// Comparing two documents
// ---------------------------------------------------------------------------

fn delta_options(
    mode: &str,
    dpi: f64,
    margin: f64,
    profile: Option<String>,
    preview: Option<PathBuf>,
    outline: Option<onionskin::delta::Outline>,
) -> Result<pipeline::Options, String> {
    let mode = pipeline::Mode::parse(mode)
        .ok_or_else(|| format!("mode must be 'raster' or 'vector', not '{mode}'"))?;
    Ok(pipeline::Options {
        dpi,
        mode,
        margin_mm: margin,
        profile,
        preview_dir: preview,
        outline,
        ..Default::default()
    })
}

/// Options for the commands that make a delta without the full expert flags:
/// `write`, `draw`, `batch`, and adding to a document.
///
/// The point is the middle step. A flag beats what this person chose for
/// themselves, which beats Onionskin's own answer — and three commands were
/// skipping the middle one entirely, so somebody who had calibrated their
/// printer and set it as their default was told they had no profile and got
/// two millimetres of error instead of half of one.
fn options_from_settings(
    preview: Option<PathBuf>,
    tuning: &Tuning,
) -> Result<pipeline::Options, String> {
    let mine = onionskin::settings::load().defaults;
    let mut options = pipeline::Options {
        preview_dir: preview,
        ..Default::default()
    };

    // Onionskin's own answer, then this person's, then the flag — each one
    // written over the last, so the flag is what survives.
    if let Some(dpi) = mine.dpi {
        options.dpi = dpi;
    }
    if let Some(dpi) = tuning.dpi {
        options.dpi = dpi;
    }
    if !(50.0..=1200.0).contains(&options.dpi) {
        return Err("dpi must be between 50 and 1200".into());
    }
    if let Some(margin) = mine.margin_mm {
        options.margin_mm = margin;
    }
    if let Some(mode) = mine.mode.as_deref().and_then(pipeline::Mode::parse) {
        options.mode = mode;
    }
    if let Some(profile) = mine.profile.clone() {
        options.profile = Some(profile);
    }
    if let Some(profile) = tuning.profile.clone() {
        options.profile = Some(profile);
    }

    // The `outline` setting is deliberately not applied here. Boxes round the
    // changes are drawn by the raster delta writer, which only the
    // compare-two-documents path uses — so setting it and honouring it here
    // would produce nothing, and a setting that quietly does nothing is worse
    // than one that plainly does not apply. `delta` is where it works.
    Ok(options)
}

/// Apply the expert settings, leaving anything not given at its default.
///
/// Separate from [`delta_options`] so that the ordinary path reads as the
/// ordinary path: five arguments somebody might reasonably choose, and then a
/// line that says the fine adjustments were left alone unless asked for.
fn expert_options(mut options: pipeline::Options, args: &DeltaArgs) -> Result<pipeline::Options, String> {
    if let Some(threshold) = args.ink_threshold {
        if threshold == 0 || threshold == 255 {
            return Err("--ink-threshold must be between 1 and 254".into());
        }
        options.diff.ink_threshold = threshold;
    }
    for (value, name) in [
        (args.group, "--group"),
        (args.min_region, "--min-region"),
        (args.pad, "--pad"),
        (args.tolerance, "--tolerance"),
    ] {
        if let Some(value) = value {
            if !value.is_finite() || value < 0.0 {
                return Err(format!("{name} must be zero or more millimetres"));
            }
        }
    }
    if let Some(group) = args.group {
        options.diff.group_mm = group;
    }
    if let Some(smallest) = args.min_region {
        options.diff.min_region_mm2 = smallest;
    }
    if let Some(pad) = args.pad {
        options.pad_mm = pad;
    }
    if let Some(tolerance) = args.tolerance {
        options.diff.tolerance_mm = tolerance;
    }
    Ok(options)
}

/// The paper to assume when none was named.
///
/// Somebody working in a country that uses Letter should say so once rather
/// than on every command they ever type. Onionskin's own answer is A4, which
/// is right for most of the world and wrong for a great many people every day.
fn default_page() -> String {
    onionskin::settings::load()
        .defaults
        .page
        .unwrap_or_else(|| "a4".to_string())
}

/// A colour by name, or as three numbers.
///
/// Names first because that is what somebody types, and the three-number form
/// underneath because somebody marking up a proof in a particular house colour
/// should not have to settle for approximately red.
fn parse_colour(text: &str) -> Result<(f64, f64, f64), String> {
    let text = text.trim();
    let named = match text.to_ascii_lowercase().as_str() {
        "red" => Some((0.80, 0.10, 0.10)),
        "green" => Some((0.00, 0.55, 0.20)),
        "blue" => Some((0.10, 0.30, 0.85)),
        "orange" => Some((0.95, 0.45, 0.00)),
        "magenta" | "pink" => Some((0.85, 0.10, 0.60)),
        "black" => Some((0.0, 0.0, 0.0)),
        "grey" | "gray" => Some((0.45, 0.45, 0.45)),
        _ => None,
    };
    if let Some(colour) = named {
        return Ok(colour);
    }

    let parts: Vec<&str> = text.split(',').map(str::trim).collect();
    if parts.len() == 3 {
        let mut channels = [0.0f64; 3];
        for (slot, part) in channels.iter_mut().zip(&parts) {
            let value: f64 = part
                .parse()
                .map_err(|_| format!("'{part}' is not a number between 0 and 1"))?;
            if !(0.0..=1.0).contains(&value) {
                return Err(format!("{value} is outside 0 to 1"));
            }
            *slot = value;
        }
        return Ok((channels[0], channels[1], channels[2]));
    }

    Err(format!(
        "I do not know the colour '{text}'. Try red, green, blue, orange, \
         magenta, black or grey — or three numbers like '0.8,0.1,0.1'."
    ))
}

/// Print the checks. Anything worse than a note goes to stderr, so a script
/// piping stdout to a file still sees the warnings on the terminal.
fn report_checks(checks: &[onionskin::safety::Check]) {
    let out = Pen::for_stdout();
    let err = Pen::for_stderr();
    for check in checks {
        use onionskin::safety::Severity;
        // The first line carries the severity, so that is what is coloured.
        // The detail under it is prose and stays plain — a paragraph in red is
        // harder to read, not easier, and the point is to be found quickly and
        // then read.
        let text = check.format();
        let (first, rest) = match text.split_once('\n') {
            Some((first, rest)) => (first, Some(rest)),
            None => (text.as_str(), None),
        };
        match check.severity {
            Severity::Note => {
                println!("{}", out.dim(first));
                if let Some(rest) = rest {
                    println!("{}", out.dim(rest));
                }
            }
            Severity::Warning => {
                eprintln!("{}", err.caution(first));
                if let Some(rest) = rest {
                    eprintln!("{rest}");
                }
            }
            _ => {
                eprintln!("{}", err.alarm(first));
                if let Some(rest) = rest {
                    eprintln!("{rest}");
                }
            }
        }
    }
}

/// Say where a long job has got to, on one line that rewrites itself.
///
/// Only when there is a terminal to draw on. Piped into a file or another
/// program, a carriage return every page turns the output into a single
/// unreadable line — so a script gets nothing extra and a person gets a
/// program that is visibly still working.
fn progress_on_a_terminal() -> impl FnMut(pipeline::Step) {
    use std::io::{IsTerminal, Write};
    let live = std::io::stderr().is_terminal();
    let mut widest = 0usize;
    move |step: pipeline::Step| {
        if !live {
            return;
        }
        let line = step.describe();
        // Padded to the longest line so far, or the tail of a longer previous
        // line stays on screen after a shorter one is written over it.
        widest = widest.max(line.chars().count());
        let mut err = std::io::stderr();
        let _ = write!(err, "\r{line:<widest$}");
        let _ = err.flush();
    }
}

/// Take the progress line off the screen before anything else is printed.
fn clear_progress() {
    use std::io::{IsTerminal, Write};
    if !std::io::stderr().is_terminal() {
        return;
    }
    let mut err = std::io::stderr();
    let _ = write!(err, "\r{:<72}\r", "");
    let _ = err.flush();
}

fn cmd_delta(args: DeltaArgs) -> Result<ExitCode, String> {
    // Beside the edited copy, because that is the document somebody was just
    // looking at and where they will look for what came out.
    let output = args
        .output
        .clone()
        .unwrap_or_else(|| beside(&args.edited, "-delta", "pdf"));
    check_writable(&output, "delta")?;
    // The flag, then what this person chose for themselves, then Onionskin's
    // own answer. Never the other way about: stating a preference must not
    // cost the ability to depart from it for one run.
    let mine = onionskin::settings::load().defaults;
    let wants_outline = args.outline || (!args.no_outline && mine.outline.unwrap_or(false));
    let outline = wants_outline
        .then(|| {
            let named = args
                .outline_colour
                .clone()
                .or_else(|| mine.outline_colour.clone())
                .unwrap_or_else(|| "red".to_string());
            parse_colour(&named).map(|colour| onionskin::delta::Outline {
                colour,
                ..Default::default()
            })
        })
        .transpose()?;
    let options = delta_options(
        &args
            .mode
            .clone()
            .or_else(|| mine.mode.clone())
            .unwrap_or_else(|| "raster".to_string()),
        args.dpi
            .or(mine.dpi)
            .unwrap_or(onionskin::pipeline::DEFAULT_DPI),
        args.margin
            .or(mine.margin_mm)
            .unwrap_or(onionskin::safety::DEFAULT_MARGIN_MM),
        args.profile.clone().or_else(|| mine.profile.clone()),
        args.preview.clone(),
        outline,
    )?;
    let options = expert_options(options, &args)?;

    let outcome = pipeline::run_watched(
        &args.original,
        &args.edited,
        &output,
        &options,
        &mut progress_on_a_terminal(),
    )
    .map_err(|e| e.to_string());
    clear_progress();
    let outcome = outcome?;

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&outcome.to_json()).map_err(|e| e.to_string())?
        );
        return Ok(if outcome.blocked() && !args.force {
            ExitCode::from(2)
        } else {
            ExitCode::SUCCESS
        });
    }

    report_checks(&outcome.checks);

    if outcome.blocked() && !args.force {
        // The file was written — the checks are about whether it is safe to
        // print, not whether it could be built — so say plainly that it is
        // there, and why it should not go in the tray.
        eprintln!(
            "\nBlocked. '{}' was written, but printing it onto the existing sheet \
             will not line up.\nPrint the affected pages fresh, or --force if you \
             know better.",
            output.display()
        );
        return Ok(ExitCode::from(2));
    }

    let pages = outcome.pages_with_additions();
    println!(
        "\n{}: {} addition{} on page{} {}.",
        output.display(),
        outcome.total_regions(),
        if outcome.total_regions() == 1 {
            ""
        } else {
            "s"
        },
        if pages.len() == 1 { "" } else { "s" },
        pages
            .iter()
            .map(|p| p.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );
    report_saving(&outcome);
    report_sheets_to_feed(&outcome);
    for path in &outcome.previews {
        println!("proof: {}", path.display());
    }
    note_the_delta(&args.edited, &output, outcome.total_regions(), outcome.pages.len());
    println!("\n{PRINT_INSTRUCTIONS}");
    open_if_asked(args.open, &output);
    Ok(ExitCode::SUCCESS)
}

fn cmd_compare(args: CompareArgs) -> Result<ExitCode, String> {
    // Nothing is written at all, and nothing is built to be thrown away
    // either — see `pipeline::examine`.
    let mine = onionskin::settings::load().defaults;
    let options = delta_options(
        "raster",
        args.dpi
            .or(mine.dpi)
            .unwrap_or(onionskin::pipeline::DEFAULT_DPI),
        args.margin
            .or(mine.margin_mm)
            .unwrap_or(onionskin::safety::DEFAULT_MARGIN_MM),
        mine.profile.clone(),
        None,
        None,
    )?;
    let outcome =
        pipeline::examine(&args.original, &args.edited, &options).map_err(|e| e.to_string())?;

    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&outcome.to_json()).map_err(|e| e.to_string())?
        );
    } else {
        println!(
            "{} page{}, {} addition{}, {:.1} mm² of new ink.",
            outcome.pages.len(),
            if outcome.pages.len() == 1 { "" } else { "s" },
            outcome.total_regions(),
            if outcome.total_regions() == 1 {
                ""
            } else {
                "s"
            },
            outcome.total_added_mm2()
        );
        for page in &outcome.pages {
            if !page.has_additions() {
                continue;
            }
            println!("\nPage {}:", page.index + 1);
            for region in &page.added_regions {
                println!(
                    "  {:>6.1},{:<6.1} mm  {:>5.1} × {:<5.1} mm",
                    region.x0_mm,
                    region.y0_mm,
                    region.width_mm(),
                    region.height_mm()
                );
            }
        }
        println!();
        report_checks(&outcome.checks);
    }
    Ok(if outcome.blocked() {
        ExitCode::from(2)
    } else {
        ExitCode::SUCCESS
    })
}

/// Is this a document Onionskin can lay text onto directly?
///
/// A PDF or a Word file already knows its own page size. An image is a
/// photograph of a sheet, and has to be measured before anything can be placed
/// on it — a different job, with different ways to go wrong.
///
/// What the file *is* beats what it is called. One of Onionskin's own
/// documents is edited in place whatever somebody named it: calling it
/// `letter.pdf` is a naming choice, not a request to be told the file is a
/// damaged PDF.
fn is_document(path: &Path) -> bool {
    if Document::is_one(path) {
        return false;
    }
    let suffix = path
        .extension()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    onionskin::render::CONVERTIBLE.contains(&suffix.as_str())
        || onionskin::render::PASSTHROUGH.contains(&suffix.as_str())
}

/// Names that promise a file of a kind this is not.
///
/// Not refused — the name is the user's to choose, and Onionskin opens it
/// again perfectly well. But a document called `letter.pdf` will not open in
/// anything else, and finding that out from a PDF viewer's error message is a
/// worse way to learn it than being told here.
fn misleading_name(path: &Path) -> Option<String> {
    let suffix = path
        .extension()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    let kind = match suffix.as_str() {
        "pdf" => "a PDF",
        "docx" | "doc" => "a Word file",
        "odt" => "an OpenDocument file",
        "png" | "jpg" | "jpeg" | "tif" | "tiff" | "bmp" => "an image",
        _ => return None,
    };
    Some(kind.to_string())
}

/// Type words onto a document at millimetres measured on the paper.
fn add_to_document(args: AddArgs) -> Result<ExitCode, String> {
    if !args.at_scan.is_empty() {
        return Err(
            "--at takes coordinates read off a scanned image, and this is a \
             document.\nUse --at-mm with millimetres measured on the paper: \
             --at-mm '45,63:Approved'"
                .into(),
        );
    }
    if args.at_page.is_empty() && args.after.is_empty() && args.below.is_empty() {
        return Err(
            "nothing to add. Say where the words go — easiest first:\n    \
             --after 'Received:Approved 27 July'   just after something in the document\n    \
             --below 'Signature:J. Bezzina'        one line under it\n    \
             --at-mm '45,63:Approved'              millimetres measured on the paper"
                .into(),
        );
    }
    let output = args
        .output
        .clone()
        .unwrap_or_else(|| beside(&args.scan, "-delta", "pdf"));
    check_writable(&output, "delta")?;
    let font = load_font(args.font_file.as_deref(), args.font_index)?;
    let face = if font.is_some() {
        "file".to_string()
    } else {
        // A document is not a scan: there is no ink to measure, so a face that
        // was not named falls back rather than being guessed at.
        args.font.clone().unwrap_or_else(|| "Helvetica".to_string())
    };
    let size = args.size.unwrap_or(11.0);

    let mut items = Vec::new();

    // Anchored placements work on a document exactly as they do on a scan:
    // the page is drawn and then read, which is what a person does when they
    // look at it. Anything Onionskin can open can be written on this way.
    if !args.after.is_empty() || !args.below.is_empty() {
        let page_text = read_a_document(&args.scan)?;
        let gap_mm = onionskin::geometry::pt_to_mm(size * 0.3);
        let step_mm = onionskin::geometry::pt_to_mm(size * 1.15);
        for (specs, put) in [
            (&args.after, onionskin::anchor::Where::After),
            (&args.below, onionskin::anchor::Where::Below),
        ] {
            for spec in specs {
                let (anchor, words) = spec.split_once(':').ok_or_else(|| {
                    format!(
                        "bad placement '{spec}'. Expected 'ANCHOR:the words' — the \
                         thing already in the document, a colon, then what to add."
                    )
                })?;
                let found = onionskin::anchor::place(&page_text, anchor, put, gap_mm, step_mm)
                    .map_err(|e| e.to_string())?;
                println!(
                    "Found \"{}\" on the line: {}\n  putting the words at {:.1}, {:.1} mm",
                    anchor.trim(),
                    found.line,
                    found.x_mm,
                    found.y_mm
                );
                items.push(Item {
                    id: 0,
                    page: 1,
                    x_mm: found.x_mm,
                    y_mm: found.y_mm,
                    text: unescape(words),
                    size_pt: size,
                    font: face.clone(),
                    width_mm: None,
                    rotation_deg: args.rotation,
                    colour: "#000000".into(),
                    leading: 1.2,
                });
            }
        }
        println!();
    }

    for placement in &args.at_page {
        let ((x_mm, y_mm), text) = parse_placement(placement)?;
        items.push(Item {
            id: 0,
            page: 1,
            x_mm,
            y_mm,
            text: unescape(&text),
            size_pt: size,
            font: face.clone(),
            width_mm: None,
            rotation_deg: args.rotation,
            colour: "#000000".into(),
            leading: 1.2,
        });
    }

    let mut options = options_from_settings(args.preview.clone(), &Tuning::default())?;
    // `add` has a margin flag of its own, which beats the stored one.
    options.margin_mm = args.margin;
    let outcome = pipeline::compose_run(&args.scan, &items, &output, font.as_ref(), &options)
        .map_err(|e| e.to_string())?;

    report_checks(&outcome.checks);
    if outcome.blocked() {
        eprintln!("\nBlocked — see above. Nothing worth printing was produced.");
        return Ok(ExitCode::from(2));
    }
    println!(
        "\n{}: {} addition{}.",
        output.display(),
        outcome.total_regions(),
        if outcome.total_regions() == 1 {
            ""
        } else {
            "s"
        }
    );
    report_saving(&outcome);
    report_sheets_to_feed(&outcome);
    for path in &outcome.previews {
        println!("proof: {}", path.display());
    }
    note_the_delta(&args.scan, &output, outcome.total_regions(), outcome.pages.len());
    println!("\n{PRINT_INSTRUCTIONS}");
    open_if_asked(args.open, &output);
    Ok(ExitCode::SUCCESS)
}

/// Build an apt repository from one or more .deb files.
///
/// `sudo apt install onionskin` cannot be made to work without one: apt does
/// not install from a URL, it installs from an archive with an index and a
/// signature. Being in Debian's own archive means a sponsor and months of
/// waiting; hosting the same thing yourself takes a directory and two lines
/// typed once by whoever wants it.
///
/// The signing is left to `gpg` on purpose. Everything else here is Rust down
/// to the SHA-256, but key custody is not a thing to hand-roll, and the
/// private key should live where its owner already keeps their keys.
fn cmd_apt_repo(args: AptRepoArgs) -> Result<ExitCode, String> {
    for deb in &args.debs {
        if !deb.is_file() {
            return Err(format!("there is no file at {}", deb.display()));
        }
    }
    let options = onionskin::apt::RepoOptions {
        suite: args.suite.clone(),
        component: args.component.clone(),
        origin: args.origin.clone(),
        label: args.label.clone(),
        description: args.description.clone(),
    };
    let built = onionskin::apt::build(
        &args.debs,
        &args.out,
        &options,
        std::time::SystemTime::now(),
    )
    .map_err(|e| e.to_string())?;

    println!("Built an apt repository in {}", built.root.display());
    println!(
        "  {} package{}, for {}",
        built.packages.len(),
        if built.packages.len() == 1 { "" } else { "s" },
        built.architectures.join(", ")
    );
    for package in &built.packages {
        println!("    {package}");
    }
    println!("  index:   {}", built.release.display());
    println!("\n{}", onionskin::apt::instructions(&options, &args.url));
    Ok(ExitCode::SUCCESS)
}

/// What to say to somebody who typed `onionskin` and nothing else.
///
/// The full help lists twenty-eight commands, which is the right answer to
/// "what can this do" and a wall to somebody meeting the program for the first
/// time. Almost everybody arriving here wants one of three things, so those go
/// first, spelled out as commands that can be copied. The other twenty-five are
/// one line away and said to be there.
fn greet() {
    let pen = Pen::for_stdout();
    println!("{}", pen.strong("Onionskin — add words to a page that is already printed."));
    println!();
    println!("You have a printed sheet and want to add something to it without");
    println!("reprinting the whole page. Onionskin writes the additions on their own.");
    println!();
    println!("{}", pen.strong("The three things people do"));
    println!();
    println!("  Print only what changed between two versions of a document");
    println!("      {}", pen.command("onionskin delta before.docx after.docx -o delta.pdf"));
    println!();
    println!("  Type onto a form you only have as a scan");
    println!(
        "      {}",
        pen.command("onionskin add scan.png --after 'Received:Approved 27 July'")
    );
    println!();
    println!("  Use the window instead of the terminal");
    println!("      {}", pen.command("onionskin-desktop"));
    println!();
    println!("{}", pen.dim("  onionskin doctor      what works on this machine, and what is missing"));
    println!("{}", pen.dim("  onionskin --help      all of it — there are twenty-five more commands"));
}

/// Colour, when there is a terminal to put it on.
///
/// Piped into a file or another program, every one of these is the empty
/// string, so a script sees exactly the bytes it saw before colour existed and
/// a log file has no escape sequences in it. `NO_COLOR` is honoured because it
/// is the one thing everybody agrees on, and some terminals genuinely cannot
/// show it.
struct Pen {
    colour: bool,
}

impl Pen {
    fn for_stdout() -> Pen {
        use std::io::IsTerminal;
        Pen {
            colour: wants_colour(std::io::stdout().is_terminal()),
        }
    }

    fn for_stderr() -> Pen {
        use std::io::IsTerminal;
        Pen {
            colour: wants_colour(std::io::stderr().is_terminal()),
        }
    }

    fn wrap(&self, code: &str, text: &str) -> String {
        if self.colour {
            format!("\x1b[{code}m{text}\x1b[0m")
        } else {
            text.to_string()
        }
    }

    fn strong(&self, text: &str) -> String {
        self.wrap("1", text)
    }
    fn dim(&self, text: &str) -> String {
        self.wrap("2", text)
    }
    fn command(&self, text: &str) -> String {
        self.wrap("36", text)
    }
    fn alarm(&self, text: &str) -> String {
        self.wrap("1;31", text)
    }
    fn caution(&self, text: &str) -> String {
        self.wrap("33", text)
    }
}

fn wants_colour(is_terminal: bool) -> bool {
    if std::env::var_os("NO_COLOR").is_some() {
        return false;
    }
    if std::env::var("TERM").map(|term| term == "dumb").unwrap_or(false) {
        return false;
    }
    is_terminal
}

/// Put a document back as it was before the last change.
///
/// `erase` takes a piece of text off a page and there was no way back from it —
/// nor from an `edit` that replaced the wrong item, nor a `write` at the wrong
/// millimetre. Every command that changes a document now sets the previous
/// version aside first, and this puts it back.
///
/// It swaps the two rather than merely restoring, so an undo can itself be
/// undone: somebody who goes back one step too many should not have to redo the
/// work by hand.
fn cmd_undo(args: UndoArgs) -> Result<ExitCode, String> {
    if !args.document.is_file() {
        return Err(format!("no such document: {}", args.document.display()));
    }
    onionskin::document::undo(&args.document).map_err(|e| e.to_string())?;

    let document = Document::load(&args.document).map_err(|e| e.to_string())?;
    println!("{} is back as it was.", args.document.display());
    println!(
        "  {} page{}, {} piece{} of text, {} drawing{}",
        document.pages,
        if document.pages == 1 { "" } else { "s" },
        document.items.len(),
        if document.items.len() == 1 { "" } else { "s" },
        document.shapes.len(),
        if document.shapes.len() == 1 { "" } else { "s" },
    );
    let back = onionskin::document::steps_back(&args.document);
    if back > 0 {
        println!(
            "\n{back} more step{} back, if this was not far enough.",
            if back == 1 { "" } else { "s" }
        );
    }
    println!("Forward again: onionskin redo {}", args.document.display());
    Ok(ExitCode::SUCCESS)
}

/// Put back a change that `undo` took away.
fn cmd_redo(args: UndoArgs) -> Result<ExitCode, String> {
    if !args.document.is_file() {
        return Err(format!("no such document: {}", args.document.display()));
    }
    onionskin::document::redo(&args.document).map_err(|e| e.to_string())?;

    let document = Document::load(&args.document).map_err(|e| e.to_string())?;
    println!("{} is forward again.", args.document.display());
    println!(
        "  {} page{}, {} piece{} of text, {} drawing{}",
        document.pages,
        if document.pages == 1 { "" } else { "s" },
        document.items.len(),
        if document.items.len() == 1 { "" } else { "s" },
        document.shapes.len(),
        if document.shapes.len() == 1 { "" } else { "s" },
    );
    let forward = onionskin::document::steps_forward(&args.document);
    if forward > 0 {
        println!(
            "\n{forward} more step{} forward.",
            if forward == 1 { "" } else { "s" }
        );
    }
    Ok(ExitCode::SUCCESS)
}

// ---------------------------------------------------------------------------
// Settings somebody chose for themselves
// ---------------------------------------------------------------------------

/// Show or change the defaults.
///
/// Onionskin has to choose *something* when it is not told — four hundred dots
/// an inch, a five millimetre margin, no box round the changes — and those
/// choices are right for most people most of the time and wrong for somebody
/// every day. A person who always wants three hundred dots and the boxes drawn
/// should say so once rather than in every command they ever type.
///
/// A flag still beats a setting, always. Somebody who has stated a preference
/// has not given up the ability to depart from it for one run, which would be
/// a strange thing for a preference to cost.
fn cmd_config(args: ConfigArgs) -> Result<ExitCode, String> {
    let pen = Pen::for_stdout();
    match args.command.unwrap_or(ConfigCommand::Show) {
        ConfigCommand::Show => {
            let defaults = onionskin::settings::load().defaults;
            println!("{}", pen.strong("Your settings"));
            println!();
            let mut any = false;
            for (name, value, what) in defaults.each() {
                match value {
                    Some(value) => {
                        any = true;
                        println!("  {name:<16} {value:<12} {}", pen.dim(what));
                    }
                    None => println!(
                        "  {} {}",
                        pen.dim(&format!("{name:<16} {:<12}", "—")),
                        pen.dim(what)
                    ),
                }
            }
            println!();
            if any {
                println!("A flag on the command line still beats any of these.");
            } else {
                println!("Nothing set — Onionskin is choosing everything.");
            }
            println!("{}", pen.dim("  onionskin config set dpi 300"));
            println!("{}", pen.dim("  onionskin config set outline yes"));
            println!("{}", pen.dim("  onionskin config unset dpi"));
        }
        ConfigCommand::Set { name, value } => {
            onionskin::settings::set_default(&name, Some(&value))?;
            println!("{name} is {value} from now on, unless a flag says otherwise.");
        }
        ConfigCommand::Unset { name } => {
            onionskin::settings::set_default(&name, None)?;
            println!("{name} is back to Onionskin's own choice.");
        }
        ConfigCommand::Reset => {
            onionskin::settings::clear_defaults();
            println!("Every setting is back to Onionskin's own choice.");
        }
    }
    Ok(ExitCode::SUCCESS)
}

// ---------------------------------------------------------------------------
// Shell completions
// ---------------------------------------------------------------------------

/// One subcommand, as a completion script needs to know it.
struct Described {
    name: String,
    about: String,
    flags: Vec<String>,
}

/// Every subcommand and its long options, read out of the command tree itself.
///
/// Read rather than written down. A completion script kept by hand is wrong
/// within a month — a flag is added and Tab never learns about it, or one is
/// removed and Tab keeps offering it — and the wrongness is invisible to the
/// person who added the flag. Taking it from the same definition `--help`
/// comes from means it cannot drift.
fn command_tree() -> Vec<Described> {
    let cli = Cli::command();
    // A global argument is declared once on the root and accepted by every
    // subcommand, but only after clap propagates it at parse time — asking a
    // subcommand for its arguments here does not show it. Without this,
    // `--overwrite` worked everywhere and completed nowhere.
    let global: Vec<String> = cli
        .get_arguments()
        .filter(|arg| arg.is_global_set())
        .filter_map(|arg| arg.get_long().map(|long| format!("--{long}")))
        .collect();

    cli.get_subcommands()
        .map(|sub| {
            let mut flags: Vec<String> = sub
                .get_arguments()
                .filter_map(|arg| arg.get_long().map(|long| format!("--{long}")))
                .collect();
            flags.extend(global.iter().cloned());
            Described {
                name: sub.get_name().to_string(),
                about: sub
                    .get_about()
                    .map(|about| about.to_string())
                    .unwrap_or_default(),
                flags,
            }
        })
        .collect()
}

/// Print a completion script for a shell.
///
/// Written out here rather than pulled in from `clap_complete`, which would be
/// a dependency and a build of it on every machine for four shell scripts that
/// do not change. The scripts are generated from the command tree, so they are
/// exactly as correct as `--help` is.
fn cmd_completions(args: CompletionsArgs) -> Result<ExitCode, String> {
    let shell = match &args.shell {
        Some(named) => named.trim().to_ascii_lowercase(),
        None => guess_the_shell(),
    };
    let tree = command_tree();

    let script = match shell.as_str() {
        "bash" => bash_completions(&tree),
        "zsh" => zsh_completions(&tree),
        "fish" => fish_completions(&tree),
        "powershell" | "pwsh" => powershell_completions(&tree),
        other => {
            return Err(format!(
                "I do not know the shell '{other}'. Onionskin can write \
                 completions for bash, zsh, fish and powershell.\n\
                 Leave the name out and it will guess from $SHELL."
            ))
        }
    };
    print!("{script}");
    // Where to put it, on stderr so that piping the script into a file still
    // shows the instructions and does not put them in the file.
    eprintln!("{}", where_to_put_it(&shell));
    Ok(ExitCode::SUCCESS)
}

/// Which shell this probably is.
fn guess_the_shell() -> String {
    let shell = std::env::var("SHELL").unwrap_or_default();
    for known in ["zsh", "fish", "bash"] {
        if shell.ends_with(known) {
            return known.to_string();
        }
    }
    if cfg!(windows) {
        return "powershell".to_string();
    }
    "bash".to_string()
}

fn where_to_put_it(shell: &str) -> String {
    match shell {
        "zsh" => "\n# Save this as _onionskin somewhere on your $fpath, for example:\n\
                  #   onionskin completions zsh > ~/.zsh/completions/_onionskin\n\
                  # and make sure that folder is on the path zsh looks down:\n\
                  #   fpath=(~/.zsh/completions $fpath)  in ~/.zshrc, before compinit"
            .to_string(),
        "fish" => "\n# Save this where fish looks for completions:\n\
                   #   onionskin completions fish > ~/.config/fish/completions/onionskin.fish\n\
                   # fish picks it up straight away — no reloading."
            .to_string(),
        "powershell" | "pwsh" => "\n# Add this to your profile:\n\
                                  #   onionskin completions powershell >> $PROFILE"
            .to_string(),
        _ => "\n# Save this where bash looks for completions:\n\
              #   onionskin completions bash > ~/.local/share/bash-completion/completions/onionskin\n\
              # or source it from ~/.bashrc:\n\
              #   onionskin completions bash > ~/.onionskin-completions.bash\n\
              #   echo 'source ~/.onionskin-completions.bash' >> ~/.bashrc"
            .to_string(),
    }
}

fn bash_completions(tree: &[Described]) -> String {
    let names: Vec<&str> = tree.iter().map(|sub| sub.name.as_str()).collect();
    let mut cases = String::new();
    for sub in tree {
        cases.push_str(&format!(
            "        {})\n            options=\"{}\"\n            ;;\n",
            sub.name,
            sub.flags.join(" ")
        ));
    }
    format!(
        "# Onionskin completions for bash. Generated by `onionskin completions bash`.\n\
         _onionskin() {{\n\
         \x20   local current previous subcommand options\n\
         \x20   current=\"${{COMP_WORDS[COMP_CWORD]}}\"\n\
         \x20   previous=\"${{COMP_WORDS[COMP_CWORD-1]}}\"\n\
         \x20   subcommand=\"\"\n\
         \x20   local index\n\
         \x20   for ((index = 1; index < COMP_CWORD; index++)); do\n\
         \x20       case \"${{COMP_WORDS[index]}}\" in\n\
         \x20           -*) ;;\n\
         \x20           *) subcommand=\"${{COMP_WORDS[index]}}\"; break ;;\n\
         \x20       esac\n\
         \x20   done\n\
         \n\
         \x20   # A flag that takes a path: let the shell offer files.\n\
         \x20   case \"$previous\" in\n\
         \x20       -o|--output|--preview|--font-file|--deb|--out|--licence|--binary|--desktop|--library|--prefix|--add-folder|--forget-folder)\n\
         \x20           COMPREPLY=($(compgen -f -- \"$current\"))\n\
         \x20           return 0\n\
         \x20           ;;\n\
         \x20   esac\n\
         \n\
         \x20   if [ -z \"$subcommand\" ]; then\n\
         \x20       COMPREPLY=($(compgen -W \"{commands} --help --version\" -- \"$current\"))\n\
         \x20       return 0\n\
         \x20   fi\n\
         \n\
         \x20   options=\"\"\n\
         \x20   case \"$subcommand\" in\n{cases}\
         \x20   esac\n\
         \x20   if [[ \"$current\" == -* ]]; then\n\
         \x20       COMPREPLY=($(compgen -W \"$options --help\" -- \"$current\"))\n\
         \x20   else\n\
         \x20       COMPREPLY=($(compgen -f -- \"$current\"))\n\
         \x20   fi\n\
         }}\n\
         complete -F _onionskin onionskin\n",
        commands = names.join(" "),
        cases = cases,
    )
}

fn zsh_completions(tree: &[Described]) -> String {
    let mut commands = String::new();
    for sub in tree {
        commands.push_str(&format!(
            "        '{}:{}'\n",
            sub.name,
            escape_for_zsh(&sub.about)
        ));
    }
    let mut cases = String::new();
    for sub in tree {
        let flags: Vec<String> = sub
            .flags
            .iter()
            .map(|flag| format!("'{flag}'"))
            .collect();
        cases.push_str(&format!(
            "            {})\n                _arguments {} '*:file:_files'\n                ;;\n",
            sub.name,
            flags.join(" ")
        ));
    }
    format!(
        "#compdef onionskin\n\
         # Onionskin completions for zsh. Generated by `onionskin completions zsh`.\n\
         _onionskin() {{\n\
         \x20   local -a commands\n\
         \x20   commands=(\n{commands}\
         \x20   )\n\
         \x20   _arguments -C '1:command:->command' '*::arguments:->arguments'\n\
         \x20   case $state in\n\
         \x20       command) _describe 'onionskin command' commands ;;\n\
         \x20       arguments)\n\
         \x20           case $words[1] in\n{cases}\
         \x20           esac\n\
         \x20           ;;\n\
         \x20   esac\n\
         }}\n\
         _onionskin \"$@\"\n"
    )
}

fn fish_completions(tree: &[Described]) -> String {
    let mut out = String::from(
        "# Onionskin completions for fish. Generated by `onionskin completions fish`.\n",
    );
    for sub in tree {
        out.push_str(&format!(
            "complete -c onionskin -n __fish_use_subcommand -a {} -d '{}'\n",
            sub.name,
            escape_for_fish(&sub.about)
        ));
    }
    out.push('\n');
    for sub in tree {
        for flag in &sub.flags {
            out.push_str(&format!(
                "complete -c onionskin -n '__fish_seen_subcommand_from {}' -l {}\n",
                sub.name,
                flag.trim_start_matches("--")
            ));
        }
    }
    out
}

fn powershell_completions(tree: &[Described]) -> String {
    let names: Vec<String> = tree.iter().map(|sub| format!("'{}'", sub.name)).collect();
    let mut cases = String::new();
    for sub in tree {
        let flags: Vec<String> = sub.flags.iter().map(|flag| format!("'{flag}'")).collect();
        cases.push_str(&format!(
            "        '{}' {{ @({}) }}\n",
            sub.name,
            flags.join(", ")
        ));
    }
    format!(
        "# Onionskin completions for PowerShell.\n\
         Register-ArgumentCompleter -Native -CommandName onionskin -ScriptBlock {{\n\
         \x20   param($wordToComplete, $commandAst, $cursorPosition)\n\
         \x20   $words = $commandAst.CommandElements | ForEach-Object {{ $_.ToString() }}\n\
         \x20   $sub = $words | Select-Object -Skip 1 | Where-Object {{ -not $_.StartsWith('-') }} | Select-Object -First 1\n\
         \x20   $candidates = if (-not $sub) {{ @({commands}) }} else {{\n\
         \x20       switch ($sub) {{\n{cases}\
         \x20           default {{ @() }}\n\
         \x20       }}\n\
         \x20   }}\n\
         \x20   $candidates | Where-Object {{ $_ -like \"$wordToComplete*\" }} |\n\
         \x20       ForEach-Object {{ [System.Management.Automation.CompletionResult]::new($_, $_, 'ParameterValue', $_) }}\n\
         }}\n",
        commands = names.join(", "),
        cases = cases,
    )
}

/// Quote a description so a shell script cannot be broken by it.
fn escape_for_zsh(text: &str) -> String {
    text.replace('\'', "'\\''").replace(':', " -")
}

fn escape_for_fish(text: &str) -> String {
    text.replace('\\', "\\\\").replace('\'', "\\'")
}

/// Read the words on the first page of a document, by drawing it and looking.
///
/// A Word file knows where its words are, but only after it has been laid out,
/// and Onionskin's answer to laying out a document is to render it — which is
/// what it already does for every delta. So the page is drawn to pixels and
/// then read exactly as a scan is read. The registration is trivial in this
/// case, because the image *is* the page: no skew to find, no paper edge to
/// look for, and the resolution is whatever was asked for.
fn read_a_document(path: &Path) -> Result<onionskin::letters::PageText, String> {
    let engine = onionskin::render::engine().map_err(|e| e.to_string())?;
    let workspace = onionskin::render::Workspace::new(false).map_err(|e| e.to_string())?;
    let (pdf, _, _) = onionskin::render::to_pdf_noting(path, &workspace.path, 180)
        .map_err(|e| e.to_string())?;
    let document = engine.open(&pdf).map_err(|e| e.to_string())?;

    // Enough resolution to read small print, and not so much that a hundred
    // megapixels are matched against a font for the sake of one anchor.
    const DPI: f64 = 300.0;
    let drawn = document.render(0, DPI).map_err(|e| e.to_string())?;
    let image = image::GrayImage::from_raw(drawn.width as u32, drawn.height as u32, drawn.gray)
        .ok_or("the page could not be turned into an image")?;
    let registration = onionskin::scan::ScanRegistration {
        page: drawn.size,
        px_per_mm: DPI / 25.4,
        skew_deg: 0.0,
        origin_px: (0.0, 0.0),
    };

    let reference = suggest_system_font()
        .or_else(|| onionskin::font::installed_fonts().first().map(|f| f.path.clone()))
        .ok_or(
            "there is no font on this machine to read the document against, so \
             words cannot be placed by what is already in it. Use --at-mm with \
             millimetres instead.",
        )?;
    let reference = EmbeddedFont::load(&reference).map_err(|e| e.to_string())?;

    letters::read_with_font(
        &image,
        &registration,
        &letters::ReadOptions::default(),
        &reference,
        Some(letters::COMMON_LATIN),
    )
    .map_err(|e| e.to_string())
}

/// Check a printed sheet came out the way the delta asked.
///
/// The same measurement `calibrate learn` takes, asked a different question.
/// That one wants to know what the printer does in general and needs three
/// marks spread across the page to say it. This one wants to know whether
/// *this sheet* is right, which one addition can answer — and answers it before
/// the other fifty-nine go through.
fn cmd_verify(args: VerifyArgs) -> Result<ExitCode, String> {
    let page = parse_page(&args.page).map_err(|e| e.to_string())?;
    let asked = calibrate::marks_on_delta(&args.delta).map_err(|e| e.to_string())?;
    if asked.is_empty() {
        return Err(format!(
            "{} has nothing on it, so there is nothing to check for.",
            args.delta.display()
        ));
    }

    let image = image::open(&args.scan)
        .map_err(|e| format!("could not read '{}': {e}", args.scan.display()))?;
    let registration = register(
        &image,
        ScanOptions {
            page,
            assume_cropped: args.cropped,
            assume_square: args.square,
            ..ScanOptions::new(page)
        },
    )
    .map_err(|e| e.to_string())?;
    println!("{}", registration.describe());

    let gray = image.to_luma8();
    let landings = calibrate::measure_landings(
        &gray,
        &registration,
        &asked,
        calibrate::ink_threshold(),
    );

    println!("\n{} addition(s) asked for:", landings.len());
    let report = calibrate::PrintReport::of(landings, args.tolerance);
    for line in report.lines() {
        println!("{line}");
    }
    println!("\n{}", report.verdict());
    if report.adrift > 0 {
        println!(
            "    Calibrate this printer and it will come down: \
             onionskin calibrate learn {} --delta {}",
            args.scan.display(),
            args.delta.display()
        );
    }

    // Learning from the same sheet, for somebody who scanned it to check and
    // may as well get the measurement out of it too.
    if let Some(name) = &args.learn {
        println!();
        match calibrate::learn_from_landings(&report.landings, page, name) {
            Ok(profile) => {
                let path = calibrate::save_profile(&profile).map_err(|e| e.to_string())?;
                println!("Profile '{name}' saved in {}.", path.display());
                println!("{}", profile.describe());
            }
            // Not a failure of the check. The sheet is still whatever it is,
            // and the exit code below is about the sheet.
            Err(e) => println!("Nothing learnt from it: {e}"),
        }
    }

    // Exit 2 for a sheet that is not right, so this can go in a script between
    // the first sheet and the rest of the stack.
    Ok(if report.good() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(2)
    })
}

/// A sheet of labels from a list.
fn cmd_labels(args: LabelsArgs) -> Result<ExitCode, String> {
    let page = parse_page(&args.page).map_err(|e| e.to_string())?;
    let (columns, rows) = parse_grid(&args.grid)?;
    let label = match &args.label {
        Some(spec) => {
            let (width_mm, height_mm) = parse_size(spec)?;
            if width_mm <= 0.0 || height_mm <= 0.0 {
                return Err(format!(
                    "a label cannot be {width_mm} by {height_mm} mm. Give the size \
                     off the box the labels came in: --label 63.5x33.9"
                ));
            }
            Some((width_mm, height_mm))
        }
        None => None,
    };
    let grid = onionskin::labels::Grid {
        page,
        columns,
        rows,
        margin_x_mm: args.margin_x,
        margin_y_mm: args.margin_y,
        gap_x_mm: args.gap_x,
        gap_y_mm: args.gap_y,
        label,
    };
    grid.check()?;

    if args.start == 0 {
        return Err(
            "--start counts labels from 1, so --start 1 is a fresh sheet and \
             --start 6 skips five already peeled off."
                .into(),
        );
    }
    let skip = args.start - 1;
    if skip >= grid.per_sheet() {
        return Err(format!(
            "--start {} is past the end of a sheet — there are only {} labels \
             on one.",
            args.start,
            grid.per_sheet()
        ));
    }

    let list = onionskin::rows::List::read(&args.from).map_err(|e| e.to_string())?;
    let unknown: Vec<String> =
        onionskin::rows::unknown_columns(std::slice::from_ref(&args.text), &list)
            .into_iter()
            .filter(|name| !onionskin::jobs::known_without_asking(name))
            .collect();
    if !unknown.is_empty() {
        return Err(format!(
            "{} has no column called {}.\n    It has: {}",
            args.from.display(),
            unknown
                .iter()
                .map(|name| format!("'{name}'"))
                .collect::<Vec<_>>()
                .join(" or "),
            list.describe_columns()
        ));
    }

    let wanted = args.first.unwrap_or(list.rows.len()).min(list.rows.len());
    if wanted == 0 {
        return Err(format!(
            "{} has no rows, so there is nothing to put on any label.",
            args.from.display()
        ));
    }

    let output = args
        .output
        .clone()
        .unwrap_or_else(|| beside(&args.from, "-labels", "pdf"));
    refuse_to_clobber(&output, "labels", &[(&args.from, "list")])?;
    check_writable(&output, "labels")?;

    // What the words are, before anything about paper. A label that cannot
    // hold its own lines is worth saying now rather than printing.
    let known = onionskin::jobs::what_the_day_is(onionskin::history::now());
    // The colour parser the rest of the text commands use, so #rrggbb works
    // here exactly as it does on `write` and `draw`. There are two in this
    // program and picking the narrower one by accident is how `--colour
    // '#000000'` came to be refused on a flag whose own default was #000000.
    let colour = onionskin::document::parse_colour(&args.colour)
        .map_err(|e| format!("--colour {}: {e}", args.colour))?;
    let face = Font::parse(&args.font).ok_or_else(|| {
        format!(
            "no built-in face called '{}'. onionskin fonts lists them.",
            args.font
        )
    })?;
    let sheets = grid.sheets_needed(wanted, skip);
    let mut per_page: Vec<Vec<onionskin::pdf::PlacedLine>> = vec![Vec::new(); sheets];
    let mut overfull = 0usize;

    for (n, row) in list.rows.iter().take(wanted).enumerate() {
        let row = &{
            let mut values = known.clone();
            values.extend(row.values.clone());
            onionskin::rows::Row {
                values,
                number: row.number,
            }
        };
        let (sheet, at) = grid.place(n, skip);
        let cell = grid.cell(at).expect("place stays inside the sheet");
        let text = unescape(&onionskin::rows::fill(&args.text, row));
        let lines: Vec<&str> = text.split('\n').collect();
        if lines.len() > cell.lines_that_fit(args.size, args.leading, args.pad) {
            overfull += 1;
        }
        for (line, words) in lines.iter().enumerate() {
            if words.trim().is_empty() {
                continue;
            }
            let (x_mm, y_mm) = cell.line_at(line, args.size, args.leading, args.pad);
            per_page[sheet].push(onionskin::pdf::PlacedLine {
                text: (*words).to_string(),
                x_mm,
                y_mm,
                size_pt: args.size,
                font: onionskin::pdf::LineFont::Builtin(face),
                rotation_deg: 0.0,
                colour,
            });
        }
    }

    let sizes: Vec<onionskin::geometry::PageSize> = (0..sheets).map(|_| page).collect();
    let nothing: Vec<Vec<onionskin::pdf::PlacedShape>> = sizes.iter().map(|_| Vec::new()).collect();
    onionskin::pdf::write_page_content(
        &output,
        &sizes,
        &per_page,
        &nothing,
        "Onionskin labels",
        None,
    )
    .map_err(|e| e.to_string())?;

    println!(
        "{}: {wanted} label(s) on {sheets} sheet(s) — {}.",
        output.display(),
        grid.describe()
    );
    if skip > 0 {
        println!(
            "Starting at label {}, so the first {skip} on the first sheet are left \
             blank for the ones already peeled off.",
            args.start
        );
    }
    if overfull > 0 {
        println!(
            "\nWARNING: {overfull} label(s) have more lines than fit at {} pt. \
             They will\n  run onto the label below, or onto the backing paper. \
             Use --size 8, or fewer lines.",
            args.size
        );
    }
    println!(
        "\nPrint at 100% / \"Actual size\", with \"Fit to page\" off — it scales by \
         a few\npercent and nothing will line up with the cuts. Try one sheet on \
         plain paper\nfirst and hold it against the label stock."
    );
    open_if_asked(args.open, &output);
    Ok(ExitCode::SUCCESS)
}

/// `COLUMNSxROWS`, as a sheet of labels is described.
fn parse_grid(spec: &str) -> Result<(usize, usize), String> {
    let bad = || {
        format!(
            "bad grid '{spec}'. Expected COLUMNSxROWS — how the stock is cut: \
             --grid 3x8"
        )
    };
    let (columns, rows) = spec.trim().split_once(['x', 'X', '*']).ok_or_else(bad)?;
    let columns: usize = columns.trim().parse().map_err(|_| bad())?;
    let rows: usize = rows.trim().parse().map_err(|_| bad())?;
    if columns == 0 || rows == 0 {
        return Err("a sheet needs at least one column and one row of labels.".into());
    }
    Ok((columns, rows))
}

/// Jobs saved on this machine.
fn cmd_job(command: JobCommand) -> Result<ExitCode, String> {
    match command {
        JobCommand::List => {
            let jobs = onionskin::jobs::list();
            if jobs.is_empty() {
                println!(
                    "No jobs saved yet.\n\n\
                     Add --save-as NAME to a write command and it is kept, ready to \
                     run on\nanother document:\n  \
                     onionskin write invoice.pdf --at '150,40:PAID {{today}}' --size 9 --save-as paid\n  \
                     onionskin job run paid invoice-4472.pdf"
                );
                return Ok(ExitCode::SUCCESS);
            }
            println!("Saved jobs:\n");
            for job in &jobs {
                let wants = job.wants();
                println!(
                    "  {:<20} {} placement(s){}",
                    job.name,
                    job.templates().len(),
                    if wants.is_empty() {
                        String::new()
                    } else {
                        format!(", asks for {}", wants.join(", "))
                    }
                );
            }
            println!("\nonionskin job show NAME   to see one in full");
            Ok(ExitCode::SUCCESS)
        }

        JobCommand::Show(args) => {
            let job = onionskin::jobs::load(&args.name).map_err(|e| e.to_string())?;
            println!("{}", job.describe());
            println!("\nKept in {}", onionskin::jobs::path_of(&job.name).display());
            Ok(ExitCode::SUCCESS)
        }

        JobCommand::Delete(args) => {
            if onionskin::jobs::delete(&args.name).map_err(|e| e.to_string())? {
                println!("Deleted job '{}'.", args.name);
            } else {
                println!("There was no job called '{}'.", args.name);
            }
            Ok(ExitCode::SUCCESS)
        }

        JobCommand::Run(args) => cmd_job_run(args),
    }
}

/// Run a saved job on a document.
fn cmd_job_run(args: RunJobArgs) -> Result<ExitCode, String> {
    let job = onionskin::jobs::load(&args.name).map_err(|e| e.to_string())?;

    let mut given = std::collections::BTreeMap::new();
    for pair in &args.set {
        let (name, value) = pair.split_once('=').ok_or_else(|| {
            format!(
                "bad --set '{pair}'. Expected NAME=VALUE, as in --set ref=4471."
            )
        })?;
        if name.trim().is_empty() {
            return Err(format!("bad --set '{pair}'. It has no name before the '='."));
        }
        given.insert(name.trim().to_string(), value.to_string());
    }

    // Everything the job needs, checked before a single word is placed. "You
    // did not say what {ref} is" belongs at the keyboard, not on a hundred
    // sheets of paper reading {ref}.
    let missing = job.missing(&given);
    if !missing.is_empty() {
        return Err(format!(
            "job '{}' needs {} filled in.\n    {}\n    \
             onionskin job show {}   says what it wants and why",
            job.name,
            missing
                .iter()
                .map(|name| format!("{{{name}}}"))
                .collect::<Vec<_>>()
                .join(" and "),
            missing
                .iter()
                .map(|name| format!("--set {name}=…"))
                .collect::<Vec<_>>()
                .join(" "),
            job.name
        ));
    }

    let row = onionskin::jobs::values(&given, onionskin::history::now());
    let fill = |templates: &[String]| -> Vec<String> {
        templates
            .iter()
            .map(|template| onionskin::rows::fill(template, &row))
            .collect()
    };

    let written = WriteArgs {
        document: args.document.clone(),
        at: fill(&job.at),
        after: fill(&job.after),
        below: fill(&job.below),
        image: fill(&job.images),
        page: job.page,
        size: job.size_pt,
        font: job.font.clone(),
        width: job.width_mm,
        rotation: job.rotation_deg,
        colour: job.colour.clone(),
        leading: job.leading,
        output: args.output.clone(),
        preview: None,
        open: args.open,
        // Running a saved job does not re-save it. Saving on every run would
        // make "the job" whatever it was last used for, which is the one thing
        // a saved job must not be.
        save_as: None,
        tuning: args.tuning,
    };

    if args.dry_run {
        println!("job '{}' on {}:\n", job.name, args.document.display());
        for placement in &written.at {
            println!("  at       {placement}");
        }
        for anchor in &written.after {
            println!("  after    {anchor}");
        }
        for anchor in &written.below {
            println!("  below    {anchor}");
        }
        for image in &written.image {
            println!("  picture  {image}");
        }
        println!("\nNothing written — --dry-run.");
        return Ok(ExitCode::SUCCESS);
    }

    println!("Running job '{}' on {}.", job.name, args.document.display());
    write_on_document(&written)
}

/// Keep what was just written as a named job, if asked.
fn save_the_job(args: &WriteArgs) {
    let Some(name) = &args.save_as else { return };
    let job = onionskin::jobs::Job {
        name: name.clone(),
        at: args.at.clone(),
        after: args.after.clone(),
        below: args.below.clone(),
        images: args.image.clone(),
        size_pt: args.size,
        font: args.font.clone(),
        colour: args.colour.clone(),
        width_mm: args.width,
        rotation_deg: args.rotation,
        leading: args.leading,
        page: args.page,
        notes: String::new(),
        created: onionskin::history::now(),
    };
    match onionskin::jobs::save(&job) {
        Ok(path) => {
            println!("\nSaved as job '{name}' in {}.", path.display());
            println!("  onionskin job run {name} <another document>");
            let wants = job.wants();
            if !wants.is_empty() {
                println!(
                    "  It will ask for {} — the words in braces were kept as blanks.",
                    wants
                        .iter()
                        .map(|w| format!("{{{w}}}"))
                        .collect::<Vec<_>>()
                        .join(" and ")
                );
            }
        }
        // Not a failure of the delta, which is written and correct. Saying so
        // and carrying on beats throwing away work that succeeded.
        Err(e) => eprintln!("\nThe delta was written, but the job was not saved: {e}"),
    }
}

/// Show what has been added to sheets of paper.
fn cmd_history(args: HistoryArgs) -> Result<ExitCode, String> {
    if args.forget {
        let had = onionskin::history::forget();
        println!(
            "Forgotten {had} entr{}. Nothing else on this machine was touched.",
            if had == 1 { "y" } else { "ies" }
        );
        return Ok(ExitCode::SUCCESS);
    }

    let entries = onionskin::history::recent(args.limit);
    if args.json {
        println!(
            "{}",
            serde_json::to_string_pretty(&entries).map_err(|e| e.to_string())?
        );
        return Ok(ExitCode::SUCCESS);
    }

    if entries.is_empty() {
        println!(
            "Nothing written yet, or the record was forgotten.\n\n\
             Every delta Onionskin writes goes in here, so a sheet in a filing \
             cabinet can\nbe asked what was added to it and when."
        );
        return Ok(ExitCode::SUCCESS);
    }

    println!("What has been added, most recent first:\n");
    for entry in &entries {
        println!("  {}", entry.describe());
    }
    println!(
        "\nKept in {}. The words themselves are not — only what they were \
         written onto,\nand a fingerprint that recognises the same delta twice.",
        onionskin::history::path().display()
    );
    println!("Forget the lot with:  onionskin history --forget");
    Ok(ExitCode::SUCCESS)
}

/// Remember this delta, and say so if it is one that was written before.
///
/// Toner does not come off paper, so a delta printed twice onto the same sheet
/// puts every letter down twice and cannot be undone. It is an easy mistake:
/// the delta is a file like any other, it prints without complaint, and the
/// second time looks exactly like the first. Nothing here refuses — printing
/// the same delta onto a hundred *different* sheets is what a hundred
/// certificates are — but the question is worth being asked.
fn note_the_delta(source: &Path, delta: &Path, additions: usize, pages: usize) {
    note_the_delta_from(&source.display().to_string(), delta, additions, pages)
}

/// The same, where what the delta came from is not one file.
///
/// A merge came from several, and "stamp.pdf" alone in the record would read
/// months later as though that had been the sheet.
fn note_the_delta_from(source: &str, delta: &Path, additions: usize, pages: usize) {
    let Some(fingerprint) = onionskin::history::fingerprint(delta) else {
        return;
    };
    let entry = onionskin::history::Entry {
        at: onionskin::history::now(),
        source: source.to_string(),
        delta: delta.display().to_string(),
        pages,
        additions,
        fingerprint,
    };
    let Some(before) = onionskin::history::remember(entry) else {
        return;
    };
    println!(
        "\nNOTE: this is the same delta you wrote {} ({}), as {}.\n  \
         Printing it onto a sheet that already has it puts the ink down twice, \
         and that\n  cannot be undone. Onto a fresh sheet it is exactly right.\n  \
         Everything written so far:  onionskin history",
        before.how_long_ago(),
        before.when(),
        before.delta
    );
}

/// Say where on a form there is room to write.
fn cmd_blanks(args: BlanksArgs) -> Result<ExitCode, String> {
    let page = parse_page(&args.page).map_err(|e| e.to_string())?;
    let options = onionskin::blanks::BlankOptions {
        ink_threshold: args.ink_threshold,
        min_width_mm: args.min_width,
        min_height_mm: args.min_height,
        margin_mm: args.margin,
    };

    // A PDF is rendered; anything else is a picture of a sheet and has to be
    // found on its glass first. Both end up as the same thing: a page of grey
    // at a known number of pixels to the millimetre.
    let (gray, width, dpi, page) = page_in_grey(&args.form, page, args.cropped, args.square)?;
    let found = onionskin::blanks::find(&gray, width, dpi, page, &options);

    if args.json {
        let listed: Vec<serde_json::Value> = found
            .iter()
            .map(|blank| {
                serde_json::json!({
                    "x_mm": blank.x_mm,
                    "y_mm": blank.y_mm,
                    "width_mm": blank.width_mm,
                    "height_mm": blank.height_mm,
                    "beside_text": blank.beside_text,
                    "fits_pt": blank.fits_pt(),
                    "fits_characters": blank.fits_characters(),
                    "at": blank.placement(),
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&listed).map_err(|e| e.to_string())?
        );
        return Ok(ExitCode::SUCCESS);
    }

    if found.is_empty() {
        println!(
            "Nothing on {} is clear enough to write in at these settings.\n  \
             Try --min-width 10 for narrower boxes, or --margin 3 if the form \
             runs close to the edge.",
            args.form.display()
        );
        return Ok(ExitCode::SUCCESS);
    }

    // Beside a label first, because those are the places the form is asking
    // about, and the empty half of the page is only ever a guess.
    println!(
        "{} place(s) to write on {}:\n",
        found.len(),
        args.form.display()
    );
    for blank in &found {
        println!("  {}", blank.describe());
    }
    let best = &found[0];
    println!(
        "\nUse one by pasting its millimetres in:\n  onionskin write {} --at '{}:Your words' --size {:.0}",
        args.form.display(),
        best.placement(),
        best.fits_pt()
    );
    Ok(ExitCode::SUCCESS)
}

/// A page of grey at a known resolution, from a PDF or from a scan of paper.
fn page_in_grey(
    path: &Path,
    page: PageSize,
    cropped: bool,
    square: bool,
) -> Result<(Vec<u8>, usize, f64, PageSize), String> {
    let looks_like_a_picture = matches!(
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref(),
        Some("png" | "jpg" | "jpeg" | "tif" | "tiff" | "bmp" | "gif" | "webp")
    );

    if looks_like_a_picture {
        let image = image::open(path)
            .map_err(|e| format!("could not read '{}': {e}", path.display()))?;
        let registration = register(
            &image,
            ScanOptions {
                page,
                assume_cropped: cropped,
                assume_square: square,
                ..ScanOptions::new(page)
            },
        )
        .map_err(|e| e.to_string())?;
        println!("{}", registration.describe());

        // Straightened onto the paper's own grid, so a millimetre in the
        // answer is a millimetre on the sheet however crookedly it was
        // scanned.
        // Coarse on purpose: this is looking for empty regions several
        // millimetres across, which a thumbnail settles.
        let dpi = 100.0;
        let flat = registration.flatten(&image.to_luma8(), dpi);
        let width = flat.width() as usize;
        return Ok((flat.into_raw(), width, dpi, page));
    }

    let engine = onionskin::render::engine().map_err(|e| e.to_string())?;
    let doc = engine.open(path).map_err(|e| e.to_string())?;
    let dpi = 100.0;
    let drawn = doc.render_gray(0, dpi).map_err(|e| e.to_string())?;
    Ok((drawn.gray, drawn.width, dpi, drawn.size))
}

/// Draw the sheet with the delta on it, so it can be looked at.
fn cmd_proof(args: ProofArgs) -> Result<ExitCode, String> {
    let output = args
        .output
        .clone()
        .unwrap_or_else(|| beside(&args.delta, "-proof", "pdf"));
    refuse_to_clobber(
        &output,
        "proof",
        &[(&args.sheet, "sheet"), (&args.delta, "delta")],
    )?;
    check_writable(&output, "proof")?;

    let added = onionskin::document::parse_colour(&args.colour)
        .map_err(|e| format!("--colour {}: {e}", args.colour))?;
    let mut options = onionskin::proof::ProofOptions {
        dpi: args.dpi,
        added: [
            (added.0 * 255.0).round() as u8,
            (added.1 * 255.0).round() as u8,
            (added.2 * 255.0).round() as u8,
        ],
        ..Default::default()
    };
    if args.tracing {
        options = options.tracing();
    }

    let pages = onionskin::proof::write_proof(&args.sheet, &args.delta, &output, &options)
        .map_err(|e| e.to_string())?;

    println!(
        "{}: {pages} page(s), the sheet in grey and what would be added in {}.",
        output.display(),
        args.colour
    );
    println!(
        "Look at it before you print the delta. Nothing here goes near the printer."
    );
    open_if_asked(args.open, &output);
    Ok(ExitCode::SUCCESS)
}

/// Put several deltas onto one, so the sheet goes through the printer once.
fn cmd_merge(args: MergeArgs) -> Result<ExitCode, String> {
    for delta in &args.deltas {
        if !delta.is_file() {
            return Err(format!("no such file: {}", delta.display()));
        }
    }
    let output = args
        .output
        .clone()
        .unwrap_or_else(|| beside(&args.deltas[0], "-merged", "pdf"));

    let inputs: Vec<(&Path, &str)> = args
        .deltas
        .iter()
        .map(|delta| (delta.as_path(), "delta"))
        .collect();
    refuse_to_clobber(&output, "merged delta", &inputs)?;
    check_writable(&output, "merged delta")?;

    let merged = onionskin::merge::merge(&args.deltas, &output, "Onionskin merged delta")
        .map_err(|e| e.to_string())?;

    println!("{}", output.display());
    println!("{}", merged.describe());

    // The same delta twice puts every letter down twice in the same place. Not
    // refused — the merge is still a valid file — but it is never what anybody
    // meant, so it is said before the paper goes in rather than after.
    for repeat in merged.repeats() {
        if let Some(same) = &repeat.same_as {
            println!(
                "\nNOTE: {} is the same file as {}. Everything in it will be \
                 printed twice,\n  in the same place, which comes out heavier \
                 and blurred. Drop one of them.",
                repeat.path.display(),
                same.display()
            );
        }
    }

    println!(
        "\nOne file, one pass. Print this instead of the deltas it was made \
         from — printing\n  both it and them puts the ink down twice."
    );

    // Remembered like any other delta, so writing the same merge twice is
    // noticed by the same machinery that notices any other repeat.
    let made_of: Vec<String> = args
        .deltas
        .iter()
        .map(|delta| {
            delta
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| delta.display().to_string())
        })
        .collect();
    note_the_delta_from(
        &made_of.join(" + "),
        &output,
        merged.from.len(),
        merged.pages,
    );

    if let Some(printer) = &args.print_to {
        let uri = resolve_printer(printer, &args.server)?;
        let options = printer::PrintOptions {
            job_name: "Onionskin merged delta".to_string(),
            ..Default::default()
        };
        let job = printer::print_file(&uri, &output, &options).map_err(|e| e.to_string())?;
        println!(
            "\nSent to {uri}{}.",
            if job > 0 {
                format!(" as job {job}")
            } else {
                String::new()
            }
        );
    }
    open_if_asked(args.open, &output);
    Ok(ExitCode::SUCCESS)
}

/// Learn the printer's error from a job that was printed anyway.
///
/// The target sheet exists because crosshairs are easy to find, and printing
/// one is an errand somebody has to decide to run. Most never will, so most
/// deltas land within about two millimetres for want of a measurement that was
/// sitting in the out tray the whole time: a delta is also a set of marks in
/// known places, and the sheet it printed onto says where they really went.
///
/// Nothing here is asked of the user that the job did not already produce.
fn cmd_calibrate_learn(args: LearnArgs) -> Result<ExitCode, String> {
    let page = parse_page(&args.page).map_err(|e| e.to_string())?;

    // Which profile is being taught. Naming none teaches the one already in
    // use, which is the case that makes this automatic rather than a chore.
    let name = match args.name.clone() {
        Some(name) => name,
        None => onionskin::settings::load()
            .defaults
            .profile
            .ok_or_else(|| {
                "no profile named, and none saved. Give --name, or set one \
                 with `onionskin config set profile NAME`."
                    .to_string()
            })?,
    };
    // Where the delta asked for ink. Read off the delta itself rather than
    // remembered, so this works on any delta Onionskin ever wrote — and so a
    // correction the delta was already written with is in the measurement
    // rather than needing to be remembered and taken off again.
    let intended = calibrate::marks_on_delta(&args.delta).map_err(|e| e.to_string())?;
    if intended.is_empty() {
        return Err(format!(
            "{} has nothing on it, so there is nothing to measure.",
            args.delta.display()
        ));
    }

    let image = image::open(&args.scan)
        .map_err(|e| format!("could not read '{}': {e}", args.scan.display()))?;
    let registration = register(
        &image,
        ScanOptions {
            page,
            assume_cropped: args.cropped,
            assume_square: args.square,
            ..ScanOptions::new(page)
        },
    )
    .map_err(|e| e.to_string())?;
    println!("{}", registration.describe());

    let gray = image.to_luma8();
    let landings = calibrate::measure_landings(
        &gray,
        &registration,
        &intended,
        calibrate::ink_threshold(),
    );

    println!("\nWhere the additions landed:");
    for landing in &landings {
        println!(
            "  {:>6.1},{:<6.1} mm   {}",
            landing.intended.0,
            landing.intended.1,
            landing.describe()
        );
    }

    let learnt =
        calibrate::learn_from_landings(&landings, page, &name).map_err(|e| e.to_string())?;

    println!("\nWhat this printer does:");
    println!("{}", learnt.describe());

    if args.dry_run {
        println!("\nNothing saved — --dry-run.");
        return Ok(ExitCode::SUCCESS);
    }
    let path = calibrate::save_profile(&learnt).map_err(|e| e.to_string())?;
    println!("\nSaved as '{name}' in {}.", path.display());
    println!("Every delta from now on is corrected by it, and every job you scan back makes it better.");
    Ok(ExitCode::SUCCESS)
}


fn cmd_calibrate_measure(args: MeasureArgs) -> Result<ExitCode, String> {
    let page = parse_page(&args.page).map_err(|e| e.to_string())?;
    let image = image::open(&args.scan)
        .map_err(|e| format!("could not read '{}': {e}", args.scan.display()))?;
    let registration = register(
        &image,
        ScanOptions {
            page,
            assume_cropped: args.cropped,
            assume_square: args.square,
            ..ScanOptions::new(page)
        },
    )
    .map_err(|e| e.to_string())?;

    println!("{}", registration.describe());

    let gray = image.to_luma8();
    let (profile, readings) = calibrate::calibrate_from_scan(
        &gray,
        &registration,
        page,
        args.inset,
        &args.name,
        &args.notes,
    )
    .map_err(|e| e.to_string())?;

    println!("\nMeasured off the sheet:");
    for reading in &readings {
        println!(
            "  P{:<2} {:+.2}, {:+.2} mm{}",
            reading.index,
            reading.dx_mm,
            reading.dy_mm,
            if reading.confidence < 0.6 {
                "   (less sure of this one)"
            } else {
                ""
            }
        );
    }

    println!("\n{}", profile.correction().describe());
    if let Some(rms) = profile.rms_residual_mm {
        println!("  the fit misses each crosshair by {rms:.2} mm on average");
    }
    if let Some(worst) = profile.max_residual_mm {
        println!("  and by {worst:.2} mm at worst");
    }

    if args.dry_run {
        println!("\nNothing was saved — this was --dry-run.");
        return Ok(ExitCode::SUCCESS);
    }

    let path = calibrate::save_profile(&profile).map_err(|e| e.to_string())?;
    println!("\nSaved as '{}' in {}", profile.name, path.display());
    println!("Use it with:  onionskin delta a.pdf b.pdf -o delta.pdf --profile {}", profile.name);
    Ok(ExitCode::SUCCESS)
}

/// Write words on a document Onionskin did not make: a Word file, a PDF, a scan.
///
/// The source is opened and measured, never altered. What comes out is a delta
/// — the words on an otherwise blank page of the same size — ready to print
/// onto the sheet that already carries the document.
fn write_on_document(args: &WriteArgs) -> Result<ExitCode, String> {
    let output = args
        .output
        .clone()
        .unwrap_or_else(|| beside(&args.document, "-delta", "pdf"));
    refuse_to_clobber(&output, "delta", &[(&args.document, "document")])?;
    check_writable(&output, "delta")?;

    // {today} and its relatives, filled in before anything is placed. It is
    // the commonest thing anybody stamps onto a piece of paper, it is
    // different every day, and somebody typing it by hand eventually stamps
    // yesterday's. Anything else in braces is left visible, so a name that
    // stands for nothing is obvious rather than silently blank.
    let today = today_row();
    let dated = |templates: &[String]| -> Vec<String> {
        templates
            .iter()
            .map(|template| onionskin::rows::fill(template, &today))
            .collect()
    };

    let mut items = Vec::new();
    for placement in &dated(&args.at) {
        let ((x_mm, y_mm), text) = parse_placement(placement)?;
        items.push(Item {
            id: 0,
            page: args.page,
            x_mm,
            y_mm,
            text: unescape(&text),
            size_pt: args.size,
            font: args.font.clone(),
            width_mm: args.width,
            rotation_deg: args.rotation,
            colour: args.colour.clone(),
            leading: args.leading,
        });
    }

    let images = placed_images(&dated(&args.image), args.page)?;
    let options = options_from_settings(args.preview.clone(), &args.tuning)?;
    let outcome = pipeline::compose_run_pictures(
        &args.document,
        &items,
        &[],
        &images,
        &output,
        None,
        &options,
    )
    .map_err(|e| e.to_string())?;

    report_checks(&outcome.checks);
    if outcome.blocked() {
        eprintln!("\nBlocked — see above. Nothing worth printing was produced.");
        return Ok(ExitCode::from(2));
    }
    println!(
        "\n{}: {} addition{}.",
        output.display(),
        outcome.total_regions(),
        if outcome.total_regions() == 1 { "" } else { "s" }
    );
    report_saving(&outcome);
    report_sheets_to_feed(&outcome);
    for path in &outcome.previews {
        println!("proof: {}", path.display());
    }
    note_the_delta(&args.document, &output, outcome.total_regions(), outcome.pages.len());
    save_the_job(args);
    println!("\n{PRINT_INSTRUCTIONS}");
    open_if_asked(args.open, &output);
    Ok(ExitCode::SUCCESS)
}

/// Draw on a document Onionskin did not make: a Word file, a PDF, a scan.
///
/// The source is opened and measured, never altered. What comes out is a delta
/// — the shapes on an otherwise blank page of the same size — ready to print
/// onto the sheet that already carries the document, which is the same bargain
/// every other part of the program offers.
fn draw_on_document(args: &DrawArgs, shapes: &[onionskin::document::Shape]) -> Result<ExitCode, String> {
    let output = args
        .output
        .clone()
        .unwrap_or_else(|| beside(&args.document, "-delta", "pdf"));
    refuse_to_clobber(&output, "delta", &[(&args.document, "document")])?;
    check_writable(&output, "delta")?;

    let placed: Vec<(usize, onionskin::pdf::PlacedShape)> =
        shapes.iter().map(|shape| (shape.page, shape.placed())).collect();

    let options = options_from_settings(args.preview.clone(), &args.tuning)?;
    let outcome =
        pipeline::compose_run_drawing(&args.document, &[], &placed, &output, None, &options)
            .map_err(|e| e.to_string())?;

    report_checks(&outcome.checks);
    if outcome.blocked() {
        eprintln!("\nBlocked — see above. Nothing worth printing was produced.");
        return Ok(ExitCode::from(2));
    }
    println!(
        "\n{}: {} drawing{}.",
        output.display(),
        outcome.total_regions(),
        if outcome.total_regions() == 1 { "" } else { "s" }
    );
    report_saving(&outcome);
    report_sheets_to_feed(&outcome);
    for path in &outcome.previews {
        println!("proof: {}", path.display());
    }
    note_the_delta(&args.document, &output, outcome.total_regions(), outcome.pages.len());
    println!("\n{PRINT_INSTRUCTIONS}");
    open_if_asked(args.open, &output);
    Ok(ExitCode::SUCCESS)
}

// ---------------------------------------------------------------------------
// Calibration
// ---------------------------------------------------------------------------

fn cmd_calibrate(command: CalibrateCommand) -> Result<ExitCode, String> {
    match command {
        CalibrateCommand::Target(args) => {
            let page = parse_page(&args.page).map_err(|e| e.to_string())?;
            check_writable(&args.output, "target")?;
            calibrate::make_target(&args.output, page, args.inset).map_err(|e| e.to_string())?;

            println!(
                "{}: a {} target, two pages.",
                args.output.display(),
                page.describe()
            );
            println!(
                "\n1. Print PAGE 1 on blank paper at 100% / \"Actual size\", with \
                 \"Fit to page\" off.\n\
                 2. Put that same sheet back in the tray, the same way up, and print \
                 PAGE 2\n   onto it. Each crosshair now has a cross from the first \
                 pass and a diamond\n   from the second, and the gap between them is \
                 the printer's error.\n\
                 3. Scan the sheet, and let Onionskin read it:\n\
                 \x20     onionskin calibrate measure sheet.png --name office\n\n\
                 Or read the offsets off the printed scales yourself — right and down \
                 positive —\nand type them in:\n\
                 \x20     onionskin calibrate solve --name office --point 'P1:+0.40,-0.15' ..."
            );
            open_if_asked(args.open, &args.output);
            Ok(ExitCode::SUCCESS)
        }

        CalibrateCommand::Measure(args) => cmd_calibrate_measure(args),
        CalibrateCommand::Learn(args) => cmd_calibrate_learn(args),

        CalibrateCommand::Solve(args) => {
            if args.points.len() < 2 {
                return Err(
                    "at least two readings are needed to fit anything. With one point \
                     only a shift can be seen, and rotation and scale are what \
                     calibration is for."
                        .into(),
                );
            }
            let page = parse_page(&args.page).map_err(|e| e.to_string())?;
            let mut offsets = Vec::new();
            for spec in &args.points {
                offsets.push(calibrate::parse_point(spec).map_err(|e| e.to_string())?);
            }

            let fit = calibrate::solve_from_offsets(&offsets, page, args.inset)
                .map_err(|e| e.to_string())?;
            let profile = calibrate::Profile {
                name: args.name.clone(),
                error: fit.transform,
                page,
                rms_residual_mm: Some(fit.rms_residual_mm),
                max_residual_mm: Some(fit.max_residual_mm),
                n_points: offsets.len(),
                created: calibrate::now(),
                notes: args.notes.clone(),
            };
            let path = calibrate::save_profile(&profile).map_err(|e| e.to_string())?;

            println!("{}", profile.describe());
            println!("\nsaved to {}", path.display());

            // A fit that does not fit is worth saying out loud: it usually
            // means a reading was taken off the wrong crosshair.
            if fit.max_residual_mm > 0.3 {
                eprintln!(
                    "\nwarning: one reading is {:.2} mm away from the fitted \
                     transform.\n    That is more than the ruler's resolution, so \
                     check the readings — most often two\n    have been swapped, or \
                     one was taken off the wrong crosshair.",
                    fit.max_residual_mm
                );
            }
            println!(
                "\nUse it:\n  onionskin delta a.docx b.docx -o delta.pdf --profile {}",
                args.name
            );
            Ok(ExitCode::SUCCESS)
        }

        CalibrateCommand::List => {
            let profiles = calibrate::list_profiles().map_err(|e| e.to_string())?;
            if profiles.is_empty() {
                println!(
                    "No calibration profiles yet.\n\n\
                     From a job you were printing anyway — scan the sheet after it \
                     goes through:\n  onionskin calibrate learn scan.png --delta \
                     delta.pdf --name office\n\n\
                     Or up front, from a target sheet:\n  onionskin calibrate \
                     target -o target.pdf"
                );
                return Ok(ExitCode::SUCCESS);
            }
            println!("Calibration profiles:");
            for profile in &profiles {
                println!(
                    "  {:<16} {:<28} {}",
                    profile.name,
                    profile.page.describe(),
                    profile.error.describe()
                );
            }
            Ok(ExitCode::SUCCESS)
        }

        CalibrateCommand::Show(args) => {
            let profile = calibrate::load_profile(&args.name).map_err(|e| e.to_string())?;
            println!("{}", profile.describe());
            Ok(ExitCode::SUCCESS)
        }

        CalibrateCommand::Delete(args) => {
            if calibrate::delete_profile(&args.name).map_err(|e| e.to_string())? {
                println!("deleted profile '{}'", args.name);
                Ok(ExitCode::SUCCESS)
            } else {
                Err(format!("no calibration profile '{}'", args.name))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Checking the machine
// ---------------------------------------------------------------------------

/// Say what works here and what does not, before anyone wastes a sheet.
/// Which copy of Onionskin the shell actually runs, and whether there is more
/// than one.
///
/// The failure this catches has no symptom. There are two ways to install —
/// `onionskin install` puts a copy in your own account, the `.deb` puts one in
/// `/usr/bin` — and doing both leaves two, with the shell silently preferring
/// whichever directory comes first on PATH. Somebody downloads a new version,
/// installs it, runs it, and nothing has changed. Nothing is broken and nothing
/// says anything, because the old program is working perfectly; it simply is
/// not the one they just installed. Without this the only way to find out is to
/// already suspect it.
fn report_which_copy_is_running() {
    let copies = onionskin::install::every_binary(if cfg!(windows) {
        "onionskin.exe"
    } else {
        "onionskin"
    });
    let running = std::env::current_exe().ok();

    match copies.split_first() {
        // None on the path at all means this is a program being run from
        // wherever it was unpacked or built, which is a perfectly ordinary
        // thing to do and worth naming rather than complaining about.
        None | Some((_, [])) => {
            if let Some(shown) = copies.first().or(running.as_ref()) {
                println!("  This copy       {}", shown.display());
            }
        }
        Some((first, rest)) => {
            println!("  ATTENTION       {} copies are installed.", copies.len());
            println!("      Your shell runs this one: {}", first.display());
            for other in rest {
                println!("      and there is also:        {}", other.display());
            }
            println!(
                "      If you installed a new version and nothing changed, this is \
                 why: the\n      copy that runs is not the copy you installed. Keep \
                 one of them.\n        - the one in your own account came from \
                 'onionskin install', and\n          'onionskin uninstall' takes it \
                 away again\n        - the one under /usr came from the .deb, and \
                 'sudo apt remove onionskin'\n          takes that away"
            );
        }
    }

    // Worth saying only when the copy that would run is not the copy that just
    // reported — which is the whole confusion, stated as a fact about two
    // paths. Compared through the filesystem, because /usr/bin/onionskin and a
    // symlink to it are the same program under two names and saying so twice
    // would invent a problem.
    if let (Some(first), Some(running)) = (copies.first(), &running) {
        let settle = |p: &PathBuf| p.canonicalize().unwrap_or_else(|_| p.clone());
        if settle(first) != settle(running) {
            println!("      (this report came from {})", running.display());
        }
    }
    println!();
}

fn cmd_doctor() -> Result<ExitCode, String> {
    let mut everything_works = true;
    println!("Onionskin {}\n", env!("CARGO_PKG_VERSION"));
    report_which_copy_is_running();

    // Rendering: needed by everything except the document editor.
    match onionskin::render::engine() {
        Ok(_) => println!("  PDF rendering   ok"),
        Err(e) => {
            everything_works = false;
            println!("  PDF rendering   MISSING\n      {e}");
        }
    }

    // The window. Not needed by anything on the command line, so a machine
    // without it is not broken — but somebody who double-clicks the desktop
    // program and gets nothing deserves to be told why here.
    let missing = onionskin::install::desktop_needs();
    if missing.is_empty() {
        println!("  The window      ok — run onionskin-desktop");
    } else {
        println!(
            "  The window      not available\n      \
             The desktop window needs {} from this system, which {} not here.\n    \
             Everything on the command line works without {}.\n      {}",
            missing.join(", "),
            if missing.len() == 1 { "is" } else { "are" },
            if missing.len() == 1 { "it" } else { "them" },
            onionskin::install::how_to_install_desktop_needs()
        );
    }

    // LibreOffice: an improvement rather than a requirement, since Onionskin
    // reads the common formats itself.
    match onionskin::render::find_soffice() {
        Some(path) => println!("  Word documents  ok ({})", path.display()),
        None => println!(
            "  Word documents  ok, read by Onionskin itself\n      LibreOffice was \
             not found, so .docx, .odt and plain text are opened here instead.\n      \
             The words, tables and lists are all there; lines may not break exactly \
             where\n      Word does. Older formats (.doc, .rtf, spreadsheets, slides) \
             still need it:\n      https://www.libreoffice.org/download/"
        ),
    }

    // Scanning over the network needs nothing installed at all — the protocol
    // is spoken here — so it is always available, whatever SANE's state.
    println!(
        "  Scanning        ok, from any network printer\n      onionskin fetch -o \
         scan.png --scanner http://printer.local/eSCL"
    );

    // Scanning through SANE, for a scanner attached to this machine.
    if scanning_available() {
        match list_devices() {
            Ok(devices) if !devices.is_empty() => {
                println!("  Attached scanner ok ({} found)", devices.len());
                for device in &devices {
                    println!("      {}", device.description);
                }
            }
            _ => println!(
                "  Attached scanner none\n      SANE is installed, but nothing is \
                 plugged in and switched on."
            ),
        }
    } else {
        println!(
            "  Attached scanner not available\n      {}",
            unavailable_reason()
        );
    }

    // Printing straight to a printer, which also needs nothing installed.
    println!(
        "  Printing        ok, to any network printer\n      onionskin send \
         delta.pdf --printer ipp://printer.local/ipp/print"
    );

    // Fonts: the built-ins always work; a system font is needed only for other
    // alphabets.
    match suggest_system_font() {
        Some(path) => println!("  Other alphabets ok ({})", path.display()),
        None => println!(
            "  Other alphabets no system font found\n      Western European text \
             works regardless. For anything else, pass\n      --font-file with a \
             .ttf or .otf that covers your language."
        ),
    }

    // Profiles: not a fault, but worth knowing.
    match calibrate::list_profiles() {
        Ok(profiles) if profiles.is_empty() => println!(
            "  Calibration     none yet (expect about ±2 mm)\n      onionskin \
             calibrate target -o target.pdf"
        ),
        Ok(profiles) => println!(
            "  Calibration     {} profile(s): {}",
            profiles.len(),
            profiles
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Err(e) => println!("  Calibration     unreadable\n      {e}"),
    }

    report_what_is_kept();

    // Worth being exact about rather than sweeping: Onionskin does now open a
    // socket, and pretending otherwise would be the kind of half-truth this
    // program is meant not to tell.
    println!(
        "\nOnionskin never phones home: no telemetry, no update check, nothing \
         about\nyour documents leaving this machine. It opens a socket when you \
         name a\nprinter or a scanner, and when you ask it to find them — and \
         then it asks\nthis network only, and talks to nothing beyond it."
    );
    Ok(if everything_works {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    })
}

// ---------------------------------------------------------------------------
// Talking to the printer
// ---------------------------------------------------------------------------

fn cmd_printers(args: PrintersArgs) -> Result<ExitCode, String> {
    // The print server first, because that is where a printer plugged into
    // this machine by USB appears, and asking it costs nothing.
    let queues = printer::printers(&args.server).unwrap_or_default();
    if !queues.is_empty() {
        println!("Set up on this machine, including anything plugged in by USB:");
        for p in &queues {
            println!("\n  {}", p.name);
            for line in [&p.model, &p.location, &p.state] {
                if !line.is_empty() {
                    println!("    {line}");
                }
            }
            println!(
                "    --printer {}",
                if p.uri.is_empty() { &p.name } else { &p.uri }
            );
        }
    }

    // Then the network, where a printer that was never set up on this machine
    // announces itself and can be printed to with nothing installed at all.
    let network = if args.no_network {
        Vec::new()
    } else {
        if queues.is_empty() {
            println!("Looking for printers…");
        }
        discover::printers(std::time::Duration::from_secs_f64(args.listen.max(0.1)))
    };
    if !network.is_empty() {
        println!("\nAnnouncing themselves on this network:");
        for found in &network {
            println!("\n  {}", found.name);
            if let Some(model) = found.model() {
                println!("    {model}");
            }
            if let Some(where_) = found.location() {
                println!("    {where_}");
            }
            println!("    --printer {}", found.plain_uri());
        }
    }

    if queues.is_empty() && network.is_empty() {
        println!(
            "No printers found.\n\n\
             Check it is switched on, and on the same network or plugged in. If it\n\
             is on the network but does not announce itself, give its address:\n  \
             onionskin printers --server ipp://printer.local/"
        );
        return Ok(ExitCode::from(1));
    }
    Ok(ExitCode::SUCCESS)
}

/// Turn whatever was typed into a printer URI.
///
/// A name is looked up on the print server; anything with a scheme or a dot in
/// it is taken as an address. Somebody who reads a name out of
/// `onionskin printers` should be able to type it straight back in.
fn resolve_printer(printer: &str, server: &str) -> Result<String, String> {
    if printer.contains("://") {
        return Ok(printer.to_string());
    }
    match printer::printers(server) {
        Ok(found) => {
            if let Some(p) = found.iter().find(|p| p.name == printer) {
                return Ok(if p.uri.is_empty() {
                    printer.to_string()
                } else {
                    p.uri.clone()
                });
            }
            if found.is_empty() {
                return Ok(printer.to_string());
            }
            Err(format!(
                "no printer called '{printer}' on {server}.\nThere is: {}",
                found
                    .iter()
                    .map(|p| p.name.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        }
        // No print server to ask. The name may still be a hostname.
        Err(_) => Ok(printer.to_string()),
    }
}

fn cmd_send(args: SendArgs) -> Result<ExitCode, String> {
    if !args.file.is_file() {
        return Err(format!("no such file: {}", args.file.display()));
    }
    let uri = resolve_printer(&args.printer, &args.server)?;
    let options = printer::PrintOptions {
        copies: args.copies,
        job_name: args.job_name.clone(),
        media: args.media.clone(),
        two_sided: args.two_sided,
    };

    let job = printer::print_file(&uri, &args.file, &options).map_err(|e| e.to_string())?;
    println!(
        "{} sent to {}{}.",
        args.file.display(),
        uri,
        if job > 0 {
            format!(" as job {job}")
        } else {
            String::new()
        }
    );
    println!(
        "\nSent with 'do not scale', so the page goes on the paper at its true \
         size.\nThat is the setting every print dialogue gets wrong."
    );
    Ok(ExitCode::SUCCESS)
}

fn cmd_fetch(args: FetchArgs) -> Result<ExitCode, String> {
    if args.capabilities {
        let capabilities = printer::capabilities(&args.scanner).map_err(|e| e.to_string())?;
        println!("{}", args.scanner);
        if !capabilities.make_and_model.is_empty() {
            println!("  {}", capabilities.make_and_model);
        }
        println!(
            "  glass: {}    feeder: {}",
            if capabilities.has_platen { "yes" } else { "no" },
            if capabilities.has_feeder { "yes" } else { "no" }
        );
        if !capabilities.resolutions.is_empty() {
            println!(
                "  resolutions: {}",
                capabilities
                    .resolutions
                    .iter()
                    .map(|r| r.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        return Ok(ExitCode::SUCCESS);
    }

    let page = parse_page(&args.page).map_err(|e| e.to_string())?;
    check_writable(&args.output, "scan")?;
    println!("{PLACEMENT_ADVICE}\n");

    let request = printer::ScanRequest {
        resolution: args.resolution,
        colour: args.colour,
        feeder: args.feeder,
        // The whole sheet, with a little over: the paper's outline is what the
        // page is measured from, and a scan cropped to the paper has lost it.
        area_mm: Some((page.width_mm + 6.0, page.height_mm + 6.0)),
    };
    let written =
        printer::scan_to(&args.scanner, &request, &args.output).map_err(|e| e.to_string())?;

    println!("{}: scanned at {} dpi.", written.display(), args.resolution);
    println!(
        "\nCheck it before adding anything:\n  onionskin inspect {} --page {}",
        written.display(),
        args.page
    );
    open_if_asked(args.open, &written);
    Ok(ExitCode::SUCCESS)
}

// ---------------------------------------------------------------------------
// Installing
// ---------------------------------------------------------------------------

fn install_options(args: &InstallArgs) -> install::Options {
    install::Options {
        prefix: args.prefix.clone(),
        keep_path: args.keep_path,
        no_menu: args.no_menu,
    }
}

fn cmd_install(args: InstallArgs) -> Result<ExitCode, String> {
    let options = install_options(&args);
    let report = install::install(&options).map_err(|e| e.to_string())?;

    if let Some(binary) = &report.binary {
        println!("Installed to {}", binary.display());
    }
    if let Some(window) = &report.desktop {
        println!("  with the window: {}", window.display());
    }
    if let Some(library) = &report.library {
        println!("  with the PDF renderer: {}", library.display());
    }
    if let Some(entry) = &report.menu_entry {
        println!("  and a menu entry: {}", entry.display());
    }
    if let Some(profile) = &report.profile {
        println!("  and a line in {}", profile.display());
        println!("\nOpen a new terminal, or run:  . {}", profile.display());
    } else if report.already_on_path {
        println!("\nThat folder is already on your path, so it is ready to use.");
    }
    for note in &report.notes {
        println!("\n{note}");
    }
    warn_about_the_other_copy(report.binary.as_deref());

    println!("\nTry it:\n  onionskin doctor");
    println!("\nTo remove it later:  onionskin uninstall");
    Ok(ExitCode::SUCCESS)
}

/// Say so when what was just installed is not what will run.
///
/// The moment a second copy comes into existence is the moment to mention it,
/// because afterwards there is nothing to notice: the older copy keeps running
/// and keeps working, and the only symptom is that a version somebody just
/// installed does not appear to have installed. Said here, it costs two lines
/// and is read by the one person who can act on it, while they are still
/// looking at the terminal they typed the command into.
fn warn_about_the_other_copy(installed: Option<&Path>) {
    let Some(installed) = installed else { return };
    let copies = onionskin::install::every_binary(if cfg!(windows) {
        "onionskin.exe"
    } else {
        "onionskin"
    });
    let settle = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    let Some(first) = copies.first() else { return };
    if settle(first) == settle(installed) {
        return;
    }

    println!(
        "\nATTENTION: this is not the copy your shell will run.\n  \
         it runs   {}\n  \
         you just installed   {}\n\
         Both work; the first one wins because its folder comes first on your \
         PATH. Until\none of them goes, `onionskin` will keep being the older \
         one — which looks exactly\nlike the new version not having installed. \
         Remove whichever you do not want:\n  \
         sudo apt remove onionskin      (the one under /usr, from the .deb)\n  \
         onionskin uninstall            (the one in your own account)",
        first.display(),
        installed.display(),
    );
}

fn cmd_uninstall(args: InstallArgs) -> Result<ExitCode, String> {
    let options = install_options(&args);
    let (binary, installed) = install::status(&options);
    if !installed {
        println!("Nothing installed at {}.", binary.display());
        return Ok(ExitCode::SUCCESS);
    }

    let report = install::uninstall(&options).map_err(|e| e.to_string())?;
    for (what, path) in [
        ("removed", report.binary.as_ref()),
        ("removed", report.desktop.as_ref()),
        ("removed", report.library.as_ref()),
        ("removed", report.menu_entry.as_ref()),
        ("tidied", report.profile.as_ref()),
    ] {
        if let Some(path) = path {
            println!("{what} {}", path.display());
        }
    }
    for note in &report.notes {
        println!("\n{note}");
    }
    Ok(ExitCode::SUCCESS)
}

// ---------------------------------------------------------------------------
// Fonts
// ---------------------------------------------------------------------------

fn cmd_fonts(args: FontsArgs) -> Result<ExitCode, String> {
    // Changing where fonts are looked for is a thing you do once, so it says
    // what it did and stops rather than also printing a page of font names.
    if let Some(folder) = &args.add_folder {
        if !folder.is_dir() {
            return Err(format!("there is no folder at {}", folder.display()));
        }
        let count = onionskin::font::installed_fonts().len();
        if onionskin::settings::add_font_folder(folder) {
            let now = onionskin::font::installed_fonts().len();
            println!("Looking in {} from now on.", folder.display());
            match now.saturating_sub(count) {
                0 => println!("  It added no fonts Onionskin could not already see."),
                1 => println!("  One font more is now available."),
                more => println!("  {more} fonts more are now available."),
            }
        } else {
            println!("Already looking in {}.", folder.display());
        }
        return Ok(ExitCode::SUCCESS);
    }
    if let Some(folder) = &args.forget_folder {
        if onionskin::settings::forget_font_folder(folder) {
            println!("No longer looking in {}.", folder.display());
        } else {
            println!(
                "{} was not one of the folders being searched.",
                folder.display()
            );
        }
        return Ok(ExitCode::SUCCESS);
    }
    if args.folders {
        println!("Looking for fonts in:");
        for folder in onionskin::font::font_folders() {
            println!("  {}", folder.display());
        }
        println!(
            "\nAdd another with:  onionskin fonts --add-folder <FOLDER>\n\
             LibreOffice keeps its own fonts inside its installation, which is why\n\
             a face that works in Writer is not always one Onionskin can see."
        );
        return Ok(ExitCode::SUCCESS);
    }

    println!("Fonts built into every PDF reader:");
    for font in Font::all() {
        println!("  {}", font.base_name());
    }
    println!(
        "\nThese cover Western European text only, and need nothing installed \
         anywhere.\nFor any other alphabet, or to match a particular face, pass \
         --font-file with a\n.ttf or .ttc and it will be carried inside the delta."
    );

    if args.all {
        let installed = onionskin::font::installed_fonts();
        if installed.is_empty() {
            println!("\nNo font files were found on this machine.");
        } else {
            println!("\nFonts on this machine ({}):", installed.len());
            for font in &installed {
                println!("  {:-40} {}", font.family, font.path.display());
            }
        }
    } else {
        let count = onionskin::font::installed_fonts().len();
        if count > 0 {
            println!("\n{count} font files were found on this machine.");
            println!("  onionskin fonts --all        list them");
            println!("  onionskin fonts --folders    where they were looked for");
        }
    }
    Ok(ExitCode::SUCCESS)
}

// ---------------------------------------------------------------------------
// Packaging
/// Delete every delta Onionskin is holding, and say what went.
///
/// It tidies as it goes already — see `delta::tidy_scratch` — so this is for
/// somebody who wants it gone now rather than at the next run, and for
/// anybody who would simply rather decide themselves.
fn cmd_tidy() -> Result<ExitCode, String> {
    let folder = onionskin::calibrate::home_dir().join("deltas");
    let (count, bytes) = scratch_deltas(&folder);
    if count == 0 {
        println!("Nothing to tidy — Onionskin is holding no deltas.");
        return Ok(ExitCode::SUCCESS);
    }
    onionskin::delta::tidy_scratch(None);
    let (left, _) = scratch_deltas(&folder);
    let gone = count - left;
    println!(
        "{gone} delta{} deleted, {} freed.",
        if gone == 1 { "" } else { "s" },
        describe_size(bytes)
    );
    if left > 0 {
        println!(
            "{left} could not be deleted. They are in {}, if you want to look.",
            folder.display()
        );
    }
    println!("\nNothing else was touched — your own files are never in here.");
    Ok(ExitCode::SUCCESS)
}

/// What Onionskin keeps on this machine, and where.
///
/// A program that stores things in a hidden folder should be willing to say
/// so without being asked twice. Everything here can be deleted by hand with
/// no harm beyond losing what it says it holds — which is the point of
/// listing it rather than merely promising to be tidy.
fn report_what_is_kept() {
    let home = onionskin::calibrate::home_dir();
    println!("\nWhat Onionskin keeps, all under {}:", home.display());

    let settings = onionskin::settings::path();
    println!(
        "  settings   {}",
        if settings.is_file() {
            "your defaults — onionskin config show".to_string()
        } else {
            "nothing yet".to_string()
        }
    );

    match onionskin::calibrate::list_profiles() {
        Ok(profiles) if profiles.is_empty() => println!("  profiles   none yet"),
        Ok(profiles) => println!(
            "  profiles   {} — {}",
            profiles.len(),
            profiles
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Err(_) => println!("  profiles   none yet"),
    }

    let (count, bytes) = scratch_deltas(&home.join("deltas"));
    if count == 0 {
        println!("  deltas     none — they are deleted once printed");
    } else {
        println!(
            "  deltas     {count} kept back ({}), from runs that asked to keep them",
            describe_size(bytes)
        );
        println!("             remove them:  onionskin tidy");
    }
}

/// How many scratch deltas are sitting there, and how much they come to.
fn scratch_deltas(folder: &Path) -> (usize, u64) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return (0, 0);
    };
    entries
        .flatten()
        .filter_map(|entry| entry.metadata().ok())
        .filter(|meta| meta.is_file())
        .fold((0, 0), |(count, bytes), meta| {
            (count + 1, bytes + meta.len())
        })
}

/// A size somebody can read, rather than a number of bytes.
fn describe_size(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{} kB", bytes / 1024)
    } else {
        format!("{bytes} bytes")
    }
}

// ---------------------------------------------------------------------------

fn cmd_package(args: PackageArgs) -> Result<ExitCode, String> {
    let platform = match &args.platform {
        Some(text) => package::Platform::parse(text)
            .ok_or_else(|| format!("I do not know the platform '{text}'. Try linux, macos or windows."))?,
        None => this_platform(),
    };

    // The running binary, unless told otherwise. Packaging on the machine that
    // built it is the ordinary case, and asking for a path there would only be
    // a chance to give the wrong one.
    let binary = match args.binary {
        Some(path) => path,
        None => std::env::current_exe().map_err(|e| format!("could not find this program: {e}"))?,
    };
    if !binary.is_file() {
        return Err(format!("There is no file at {}.", binary.display()));
    }

    // The renderer, if it is to hand. Not finding one is worth saying out loud
    // rather than shipping an archive that quietly cannot compare documents.
    let library = match args.library {
        Some(path) => {
            if !path.is_file() {
                return Err(format!("There is no file at {}.", path.display()));
            }
            Some(path)
        }
        None => binary
            .parent()
            .map(|dir| dir.join(platform.library_name()))
            .filter(|path| path.is_file()),
    };

    // The licence has to travel with the binary — that is the whole reason
    // this command exists rather than a shell script somebody runs by hand.
    let licence = args.licence.unwrap_or_else(|| PathBuf::from("LICENSE"));
    if !licence.is_file() {
        return Err(format!(
            "I could not find the licence at {}.\n\
             Onionskin is MIT and the licence must ship with it, so I will not \
             build an archive without one.\n\
             Run this from the source directory, or say --licence <path>.",
            licence.display()
        ));
    }

    // The window, if one was built. Looked for beside the command line program
    // by the name cargo gives it, so the ordinary case needs no arguments.
    let desktop = match args.desktop {
        Some(path) => {
            if !path.is_file() {
                return Err(format!("There is no file at {}.", path.display()));
            }
            Some(path)
        }
        None => binary
            .parent()
            .map(|dir| {
                dir.join(if platform == package::Platform::Windows {
                    "onionskin-desktop.exe"
                } else {
                    "onionskin-desktop"
                })
            })
            .filter(|path| path.is_file()),
    };

    let written = package::build_with_window(
        platform,
        &binary,
        desktop.as_deref(),
        library.as_deref(),
        &licence,
        &args.version,
        &args.out,
    )
    .map_err(|e| e.to_string())?;

    println!("Packaged Onionskin {} for {}:", args.version, platform.name());
    for path in &written {
        let size = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        println!("  {}  ({})", path.display(), human_size(size));
    }
    match &library {
        Some(path) => println!("\nWith the PDF renderer from {}.", path.display()),
        None => println!(
            "\nWithout a PDF renderer: none was beside the binary.\n\
             Everything works except comparing two documents. Put {} next to the \
             binary, or say --library <path>.",
            platform.library_name()
        ),
    }
    println!(
        "\nInside each: the program, LICENCE, THIRD-PARTY-LICENCES and a README\n\
         saying to run '{} install'.",
        platform.binary_name()
    );
    Ok(ExitCode::SUCCESS)
}

/// The platform this copy was compiled for.
fn this_platform() -> package::Platform {
    if cfg!(windows) {
        package::Platform::Windows
    } else if cfg!(target_os = "macos") {
        package::Platform::MacOs
    } else {
        package::Platform::Linux
    }
}

fn human_size(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{} kB", bytes / 1024)
    } else {
        format!("{bytes} bytes")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colour_is_only_for_a_terminal_and_only_when_wanted() {
        // Piped into a file or another program, a script must see exactly the
        // bytes it saw before colour existed, and a log file must have no
        // escape sequences in it.
        let plain = Pen { colour: false };
        assert_eq!(plain.alarm("BLOCKER"), "BLOCKER");
        assert_eq!(plain.dim("note"), "note");
        assert_eq!(plain.command("onionskin doctor"), "onionskin doctor");

        let coloured = Pen { colour: true };
        assert!(coloured.alarm("BLOCKER").contains("BLOCKER"));
        assert!(coloured.alarm("BLOCKER").starts_with('\x1b'));
        assert!(coloured.alarm("BLOCKER").ends_with("\x1b[0m"));
    }

    #[test]
    fn a_terminal_is_not_enough_if_the_person_said_no() {
        // NO_COLOR is the one thing everybody agrees on, and some terminals
        // genuinely cannot show it.
        assert!(!wants_colour(false), "colour on a pipe");
        // Not asserted with the variable set, because the environment is
        // process-wide and this test suite runs in threads — but the branch
        // above it is what the flag guards, and it is one line.
    }

    #[test]
    fn the_command_line_itself_is_valid() {
        // clap checks a great deal about the command tree, but only when
        // asked. Without this, a duplicated flag or a `requires` naming an
        // argument that does not exist is a panic the first time somebody
        // runs the program, and never during a build.
        Cli::command().debug_assert();
    }

    #[test]
    fn the_completions_are_taken_from_the_real_commands() {
        // The whole reason for generating them: a script kept by hand is
        // wrong within a month — a flag is added and Tab never learns of it —
        // and the wrongness is invisible to whoever added the flag.
        let tree = command_tree();
        assert!(tree.len() > 20, "only {} commands found", tree.len());

        let delta = tree
            .iter()
            .find(|sub| sub.name == "delta")
            .expect("no delta command");
        for flag in ["--outline", "--mode", "--ink-threshold", "--profile"] {
            assert!(
                delta.flags.iter().any(|had| had == flag),
                "delta is missing {flag}: {:?}",
                delta.flags
            );
        }
        assert!(!delta.about.is_empty());
    }

    #[test]
    fn a_flag_declared_once_for_every_command_reaches_every_command() {
        // `--overwrite` is declared on the root and accepted everywhere, but
        // clap only hands it to a subcommand at parse time — so asking one
        // for its arguments does not mention it. It worked everywhere and
        // completed nowhere, which is exactly the invisible wrongness these
        // generated scripts exist to prevent.
        let tree = command_tree();
        for sub in &tree {
            assert!(
                sub.flags.iter().any(|flag| flag == "--overwrite"),
                "{} cannot complete --overwrite: {:?}",
                sub.name,
                sub.flags
            );
        }

        // And it is in the scripts themselves, not merely in the tree.
        for script in [
            bash_completions(&tree),
            zsh_completions(&tree),
            fish_completions(&tree),
            powershell_completions(&tree),
        ] {
            assert!(script.contains("overwrite"), "a script lost --overwrite");
        }
    }

    #[test]
    fn every_shell_gets_a_script_naming_every_command() {
        let tree = command_tree();
        let scripts = [
            bash_completions(&tree),
            zsh_completions(&tree),
            fish_completions(&tree),
            powershell_completions(&tree),
        ];
        for script in &scripts {
            assert!(!script.is_empty());
            for sub in &tree {
                assert!(
                    script.contains(&sub.name),
                    "a script never mentions '{}'",
                    sub.name
                );
            }
        }
    }

    #[test]
    fn a_description_cannot_break_the_script_it_goes_into() {
        // Descriptions are prose written by whoever added the command, and
        // prose contains apostrophes. One unescaped quote turns a completion
        // script into a syntax error in somebody's shell startup, which is a
        // memorable way to meet a program for the first time.
        assert_eq!(escape_for_fish("don't"), "don\\'t");
        assert!(escape_for_zsh("a 'quoted' word").contains("'\\''"));
        // zsh reads a colon as the end of the name, so it cannot survive one.
        assert!(!escape_for_zsh("Print: a thing").contains(':'));
    }

    #[test]
    fn an_unknown_shell_is_refused_by_name() {
        let said = cmd_completions(CompletionsArgs {
            shell: Some("tcsh".into()),
        })
        .unwrap_err();
        assert!(said.contains("tcsh"), "{said}");
        assert!(said.contains("bash"), "{said}");
    }

    #[test]
    fn a_result_goes_beside_the_file_it_came_from() {
        // The commonest command should not need a flag typed every time.
        assert_eq!(
            beside(Path::new("/work/report-v2.docx"), "-delta", "pdf"),
            PathBuf::from("/work/report-v2-delta.pdf")
        );
        assert_eq!(
            beside(Path::new("scan.png"), "-delta", "pdf"),
            PathBuf::from("scan-delta.pdf")
        );
        // Printing a document whole keeps its own name.
        assert_eq!(
            beside(Path::new("/work/order.onionskin"), "", "pdf"),
            PathBuf::from("/work/order.pdf")
        );
    }

    #[test]
    fn a_file_with_no_name_still_gets_one() {
        // Nothing sensible to name it after, and refusing would be worse than
        // a dull name.
        let named = beside(Path::new("/"), "-delta", "pdf");
        assert!(
            named.to_string_lossy().ends_with("onionskin-delta.pdf"),
            "{}",
            named.display()
        );
    }

    #[test]
    fn a_name_with_dots_in_it_keeps_all_but_the_last() {
        assert_eq!(
            beside(Path::new("minutes.2026.03.docx"), "-delta", "pdf"),
            PathBuf::from("minutes.2026.03-delta.pdf")
        );
    }
}

#[cfg(test)]
mod naming_tests {
    use super::*;

    #[test]
    fn onionskins_own_document_is_edited_in_place_whatever_it_is_called() {
        // The trap this closes: `onionskin new letter.pdf` printed the very
        // command to run next, and that command then said the file was a
        // damaged PDF. Onionskin had written it itself, one line earlier.
        let dir = tempfile::tempdir().unwrap();
        for name in ["letter.pdf", "letter.docx", "letter.odt", "letter.png"] {
            let path = dir.path().join(name);
            Document::blank(onionskin::calibrate::A4, 1)
                .save(&path)
                .unwrap();
            assert!(!is_document(&path), "{name} was sent down the delta path");
        }
    }

    #[test]
    fn a_real_pdf_or_word_file_still_gets_a_delta() {
        // And the direction that matters more: somebody's own file is theirs,
        // and must never be edited in place.
        let dir = tempfile::tempdir().unwrap();
        for (name, magic) in [
            ("theirs.pdf", &b"%PDF-1.7\n"[..]),
            ("theirs.docx", &b"PK\x03\x04"[..]),
        ] {
            let path = dir.path().join(name);
            std::fs::write(&path, magic).unwrap();
            assert!(is_document(&path), "{name} would have been edited");
        }
        // A name with no file behind it keeps deciding by the extension,
        // because that is all there is to go on.
        assert!(is_document(&dir.path().join("not-yet.pdf")));
        assert!(!is_document(&dir.path().join("not-yet.onion")));
    }

    /// `onionskin write <doc>` with the given flags, straight through the
    /// real parser so the test exercises what somebody would actually type.
    fn write_run(args: &[&str]) -> Result<ExitCode, String> {
        let mut argv = vec!["onionskin", "write"];
        argv.extend_from_slice(args);
        let Some(Command::Write(parsed)) = Cli::parse_from(argv).command else {
            panic!("write did not parse");
        };
        cmd_write(parsed)
    }

    #[test]
    fn words_can_be_anchored_to_a_documents_own_words_too() {
        // "Place words next to what is already on the page" worked on a scan
        // and on somebody's PDF, but not on Onionskin's own documents — the
        // one format where their position is known exactly rather than read
        // off a picture.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("form.onionskin");
        let name = path.to_str().unwrap();
        Document::blank(onionskin::calibrate::A4, 1).save(&path).unwrap();
        write_run(&[name, "--at", "20,40:Received:"]).unwrap();

        write_run(&[name, "--after", "Received:27 July"]).unwrap();
        let doc = Document::load(&path).unwrap();
        let added = doc.items.iter().find(|i| i.text == "27 July").unwrap();
        // On the same line, and past the anchor rather than on top of it.
        assert!((added.y_mm - 40.0).abs() < 1e-9, "{added:?}");
        assert!(added.x_mm > 20.0, "{added:?}");

        write_run(&[name, "--below", "Received:next line"]).unwrap();
        let doc = Document::load(&path).unwrap();
        let below = doc.items.iter().find(|i| i.text == "next line").unwrap();
        assert!(below.y_mm > 40.0, "{below:?}");
        assert!((below.x_mm - 20.0).abs() < 1e-9, "{below:?}");
    }

    #[test]
    fn an_anchor_that_is_not_there_leaves_the_document_untouched() {
        // Half a page of new words and then a refusal would be the worst of
        // both, so the anchors are all found before anything is added.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("form.onionskin");
        let name = path.to_str().unwrap();
        Document::blank(onionskin::calibrate::A4, 1).save(&path).unwrap();
        write_run(&[name, "--at", "20,40:Received:"]).unwrap();

        let said = write_run(&[
            name,
            "--at",
            "20,90:this should not land",
            "--after",
            "Telephone:0123",
        ])
        .unwrap_err();
        assert!(said.contains("Telephone"), "{said}");

        let doc = Document::load(&path).unwrap();
        assert_eq!(doc.items.len(), 1, "something was written anyway");
    }

    /// `onionskin batch …`, straight through the real parser.
    fn batch_run(args: &[&str]) -> Result<ExitCode, String> {
        let mut argv = vec!["onionskin", "batch"];
        argv.extend_from_slice(args);
        let Some(Command::Batch(parsed)) = Cli::parse_from(argv).command else {
            panic!("batch did not parse");
        };
        cmd_batch(parsed)
    }

    /// A blank sheet to print onto, and a list of people.
    fn a_sheet_and_a_list(dir: &Path) -> (PathBuf, PathBuf) {
        let doc = dir.join("blank.onionskin");
        Document::blank(onionskin::calibrate::A4, 1).save(&doc).unwrap();
        let sheet = dir.join("sheet.pdf");
        let Some(Command::Print(print)) = Cli::parse_from([
            "onionskin",
            "print",
            doc.to_str().unwrap(),
            "-o",
            sheet.to_str().unwrap(),
        ])
        .command
        else {
            panic!("print did not parse");
        };
        cmd_print(print).unwrap();

        let list = dir.join("people.csv");
        std::fs::write(&list, "name,course\nA. One,Bookbinding\nB. Two,Letterpress\n").unwrap();
        (sheet, list)
    }

    #[test]
    fn the_sheets_to_feed_are_named_the_way_somebody_would_say_them() {
        assert_eq!(describe_sheets(&[3]), "3");
        assert_eq!(describe_sheets(&[3, 7]), "3 and 7");
        assert_eq!(describe_sheets(&[3, 7, 9]), "3, 7 and 9");
        // A run collapses, because "4 to 21" is something a person can act
        // on and eighteen comma-separated numbers is not.
        assert_eq!(describe_sheets(&[4, 5, 6, 7]), "4 to 7");
        assert_eq!(describe_sheets(&[1, 2]), "1 and 2");
        assert_eq!(describe_sheets(&[1, 2, 3, 9, 11, 12, 13]), "1 to 3, 9 and 11 to 13");
        assert_eq!(describe_sheets(&[]), "");
    }

    #[test]
    fn a_saved_setting_reaches_the_commands_that_make_a_delta() {
        // `write`, `draw` and `batch` were building their options from
        // Onionskin's own defaults and skipping the middle step entirely, so
        // somebody who had calibrated their printer and saved it as their
        // default was told they had no profile — and got two millimetres of
        // error where they should have had half of one.
        //
        // The settings file is process-wide, so this drives the pure part
        // rather than the file: what matters is that a flag beats a stored
        // value and a stored value beats nothing.
        let nothing = Tuning::default();
        let base = options_from_settings(None, &nothing).unwrap();
        assert!(base.dpi >= 50.0);

        let asked = Tuning {
            profile: Some("office".to_string()),
            dpi: Some(250.0),
        };
        let tuned = options_from_settings(None, &asked).unwrap();
        assert_eq!(tuned.profile.as_deref(), Some("office"));
        assert!((tuned.dpi - 250.0).abs() < 1e-9);
    }

    #[test]
    fn a_silly_resolution_is_refused_where_it_is_typed() {
        for silly in [0.0, 49.0, 1201.0] {
            let asked = Tuning {
                profile: None,
                dpi: Some(silly),
            };
            let said = options_from_settings(None, &asked).unwrap_err();
            assert!(said.contains("between 50 and 1200"), "{silly}: {said}");
        }
    }

    #[test]
    fn a_picture_is_read_from_the_end_so_a_windows_path_still_works() {
        // The file name comes first and may hold colons of its own, so the
        // two parts that matter are found from the end.
        let spec = parse_image("signature.png:120,240:40").unwrap();
        assert_eq!(spec.path, PathBuf::from("signature.png"));
        assert_eq!((spec.x_mm, spec.y_mm), (120.0, 240.0));
        assert_eq!((spec.width_mm, spec.height_mm), (Some(40.0), None));

        let spec = parse_image(r"C:\scans\sign.png:10,20:30").unwrap();
        assert_eq!(spec.path, PathBuf::from(r"C:\scans\sign.png"));

        // Both measurements, when somebody wants the box exactly.
        let spec = parse_image("s.png:10,20:40x15").unwrap();
        assert_eq!((spec.width_mm, spec.height_mm), (Some(40.0), Some(15.0)));
    }

    #[test]
    fn a_picture_with_no_size_or_a_silly_one_is_refused_by_name() {
        for bad in [
            "signature.png",
            "signature.png:120,240",
            ":120,240:40",
            "s.png:120:40",
            "s.png:a,b:40",
            "s.png:10,20:wide",
        ] {
            assert!(parse_image(bad).is_err(), "{bad} was accepted");
        }
        for silly in ["s.png:10,20:0", "s.png:10,20:-5"] {
            let said = parse_image(silly).unwrap_err();
            assert!(said.contains("greater than nothing"), "{said}");
        }
    }

    #[test]
    fn the_measurement_left_out_follows_the_pictures_own_shape() {
        // A signature squashed into a box it was not drawn for is worse than
        // no signature at all.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wide.png");
        // Four across, one down: four times as wide as it is tall.
        let mut img = image::RgbImage::new(4, 1);
        for pixel in img.pixels_mut() {
            *pixel = image::Rgb([0, 0, 0]);
        }
        img.save(&path).unwrap();

        let spec = format!("{}:10,20:40", path.to_str().unwrap());
        let placed = placed_images(&[spec], 1).unwrap();
        assert_eq!(placed.len(), 1);
        let image = &placed[0].1;
        assert!((image.width_mm - 40.0).abs() < 1e-9);
        assert!((image.height_mm - 10.0).abs() < 1e-9, "{image:?}");

        // And giving only a height works the other way round.
        let spec = format!("{}:10,20:x10", path.to_str().unwrap());
        let placed = placed_images(&[spec], 1).unwrap();
        assert!((placed[0].1.width_mm - 40.0).abs() < 1e-9, "{:?}", placed[0].1);
    }

    #[test]
    fn a_picture_that_is_not_there_says_so_rather_than_writing_a_blank_page() {
        let said = placed_images(&["nowhere.png:10,20:40".to_string()], 1).unwrap_err();
        assert!(said.contains("nowhere.png"), "{said}");
    }

    #[test]
    fn a_misspelt_column_is_caught_before_a_single_sheet_is_made() {
        // Two hundred certificates reading "{nmae}" is a discovery to make
        // now rather than at the printer, so this must not even reach the
        // point of writing a file.
        let dir = tempfile::tempdir().unwrap();
        let (sheet, list) = a_sheet_and_a_list(dir.path());
        let out = dir.path().join("stack.pdf");

        let said = batch_run(&[
            sheet.to_str().unwrap(),
            "--from",
            list.to_str().unwrap(),
            "--at",
            "60,140:{nmae}",
            "-o",
            out.to_str().unwrap(),
        ])
        .unwrap_err();
        assert!(said.contains("nmae"), "{said}");
        // And says what it could have meant.
        assert!(said.contains("{name}"), "{said}");
        assert!(!out.exists(), "a stack was written anyway");
    }

    #[test]
    fn first_makes_only_that_many() {
        // Try two before committing two hundred sheets of paper.
        let dir = tempfile::tempdir().unwrap();
        let (sheet, list) = a_sheet_and_a_list(dir.path());
        let out = dir.path().join("stack.pdf");
        batch_run(&[
            sheet.to_str().unwrap(),
            "--from",
            list.to_str().unwrap(),
            "--at",
            "60,140:{name}",
            "--first",
            "1",
            "-o",
            out.to_str().unwrap(),
        ])
        .unwrap();
        assert!(out.is_file());

        // The real question is whose names are on it: the first person's and
        // not the second's. Counting pages would pass even if both names had
        // been printed on the one sheet.
        let written = String::from_utf8_lossy(&std::fs::read(&out).unwrap()).into_owned();
        assert!(written.contains("A. One"), "the first name is missing");
        assert!(
            !written.contains("B. Two"),
            "--first 1 made a sheet for the second person too"
        );
    }

    #[test]
    fn batch_needs_to_be_told_where_the_words_go() {
        let dir = tempfile::tempdir().unwrap();
        let (sheet, list) = a_sheet_and_a_list(dir.path());
        let said = batch_run(&[
            sheet.to_str().unwrap(),
            "--from",
            list.to_str().unwrap(),
            "-o",
            dir.path().join("stack.pdf").to_str().unwrap(),
        ])
        .unwrap_err();
        assert!(said.contains("where the words go"), "{said}");
    }

    #[test]
    fn a_page_with_nothing_on_it_says_which_page_that_was() {
        // The usual cause is a --page that is not the one the words are on,
        // and "there is no text on this page" does not say which page was
        // looked at, which is the whole question.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("form.onionskin");
        let name = path.to_str().unwrap();
        let mut doc = Document::blank(onionskin::calibrate::A4, 3);
        doc.pages = 3;
        doc.save(&path).unwrap();
        write_run(&[name, "--at", "20,40:Received:"]).unwrap();

        let said = write_run(&[name, "--after", "Received:x", "--page", "2"]).unwrap_err();
        assert!(said.contains("page 2"), "{said}");
        assert!(said.contains("3 pages"), "{said}");

        // And the page it is on still works.
        assert!(write_run(&[name, "--after", "Received:x", "--page", "1"]).is_ok());
    }

    #[test]
    fn an_anchor_and_its_words_are_split_the_way_add_splits_them() {
        // The same flag on two commands must mean the same thing.
        assert_eq!(
            split_anchor("Received:27 July").unwrap(),
            ("Received".to_string(), "27 July".to_string())
        );
        // A colon inside the words is left alone — times and ratios have one.
        assert_eq!(
            split_anchor("Meeting:12:30 in room 4").unwrap(),
            ("Meeting".to_string(), "12:30 in room 4".to_string())
        );
        // And the halves that say nothing are refused by name.
        assert!(split_anchor("no colon here").is_err());
        assert!(split_anchor(":only words").is_err());
        assert!(split_anchor("only anchor:").is_err());
    }

    #[test]
    fn what_is_kept_is_counted_and_sized_in_words_somebody_reads() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(scratch_deltas(dir.path()), (0, 0));
        // A folder that is not there at all is not an error, it is nothing.
        assert_eq!(scratch_deltas(&dir.path().join("never-made")), (0, 0));

        std::fs::write(dir.path().join("a.pdf"), vec![0u8; 3000]).unwrap();
        std::fs::write(dir.path().join("b.pdf"), vec![0u8; 1000]).unwrap();
        assert_eq!(scratch_deltas(dir.path()), (2, 4000));

        assert_eq!(describe_size(0), "0 bytes");
        assert_eq!(describe_size(999), "999 bytes");
        assert_eq!(describe_size(4000), "3 kB");
        assert_eq!(describe_size(5 * 1024 * 1024), "5.0 MB");
    }

    #[test]
    fn a_file_onionskin_did_not_write_is_not_written_over() {
        // `onionskin print doc -o report.pdf` used to destroy a report.pdf
        // that had nothing to do with Onionskin, in silence and with a
        // cheerful success message.
        let dir = tempfile::tempdir().unwrap();

        let theirs = dir.path().join("report.pdf");
        std::fs::write(&theirs, b"%PDF-1.7\n...somebody's own work...").unwrap();
        assert!(!may_write_over(&theirs, false), "their PDF was not protected");
        assert!(may_write_over(&theirs, true), "--overwrite was not honoured");

        // Not a PDF at all, and not ours either.
        let notes = dir.path().join("notes.txt");
        std::fs::write(&notes, b"shopping list").unwrap();
        assert!(!may_write_over(&notes, false));
    }

    #[test]
    fn onionskins_own_output_is_replaced_without_asking() {
        // Run a command, look at the delta, edit, run it again. That loop is
        // the ordinary way to use this and must not ask anything.
        let dir = tempfile::tempdir().unwrap();

        let ours = dir.path().join("delta.pdf");
        std::fs::write(&ours, b"%PDF-1.4\n<</Title(x)/Producer(Onionskin)>>").unwrap();
        assert!(may_write_over(&ours, false), "our own delta was protected");

        // A document of ours, likewise — and whatever it is called.
        let doc = dir.path().join("letter.pdf");
        Document::blank(onionskin::calibrate::A4, 1).save(&doc).unwrap();
        assert!(may_write_over(&doc, false));

        // A name with nothing behind it is always free.
        assert!(may_write_over(&dir.path().join("not-there.pdf"), false));
    }

    #[test]
    fn the_producer_line_is_matched_exactly_not_merely_mentioned() {
        // A PDF of somebody's essay about Onionskin says the word on its
        // pages. That is not a claim to have been written by it.
        let dir = tempfile::tempdir().unwrap();
        let essay = dir.path().join("essay.pdf");
        std::fs::write(
            &essay,
            b"%PDF-1.7\n(Onionskin is a program for adding words to printed pages)",
        )
        .unwrap();
        assert!(!may_write_over(&essay, false), "an essay was claimed as ours");
    }

    #[test]
    fn add_points_at_write_rather_than_calling_a_document_a_bad_image() {
        // `add` measures a delta off a scan. Handed one of Onionskin's own
        // documents it used to fall through to the image path and report that
        // the file "was not recognized as an image format" — true, and no
        // help at all when there is a command that does exactly what was
        // wanted.
        let dir = tempfile::tempdir().unwrap();
        for name in ["letter.onionskin", "letter.pdf"] {
            let path = dir.path().join(name);
            Document::blank(onionskin::calibrate::A4, 1)
                .save(&path)
                .unwrap();
            let cli = Cli::parse_from([
                "onionskin",
                "add",
                path.to_str().unwrap(),
                "--at-mm",
                "20,80:Approved",
            ]);
            let Some(Command::Add(args)) = cli.command else {
                panic!("add did not parse");
            };
            let said = cmd_add(args).unwrap_err();
            assert!(said.contains("Onionskin document"), "{said}");
            assert!(said.contains("onionskin write"), "{said}");
            assert!(!said.contains("image format"), "{said}");
        }
    }

    #[test]
    fn a_name_that_promises_another_kind_of_file_is_mentioned_once() {
        assert_eq!(misleading_name(Path::new("a.pdf")).as_deref(), Some("a PDF"));
        assert_eq!(
            misleading_name(Path::new("a.DOCX")).as_deref(),
            Some("a Word file")
        );
        assert_eq!(
            misleading_name(Path::new("a.jpeg")).as_deref(),
            Some("an image")
        );
        // The names that promise nothing say nothing.
        assert_eq!(misleading_name(Path::new("letter.onion")), None);
        assert_eq!(misleading_name(Path::new("letter")), None);
    }
}
