//! Onionskin — add words to a page that is already printed.
//!
//! This binary covers the scanned-page workflow: you have a sheet in your hand
//! and an image of it, and you want to write something onto the paper itself.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};

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
use onionskin::pdf::{write_delta, Font, LineFont, PlacedLine};
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
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Report how the sheet sits in a scan, without writing anything.
    Inspect(InspectArgs),
    /// Write a delta PDF that adds words to the scanned sheet.
    Add(AddArgs),
    /// Scan a sheet, ready to add words to.
    Acquire(AcquireArgs),
    /// List the scanners this machine can see.
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
    /// Write a document out as a PDF, whole or as a delta onto the printed sheet.
    Print(PrintArgs),
    /// Read the letters off a scanned page.
    Read(ReadArgs),

    /// Compare two documents and write a delta of what the edit added.
    Delta(DeltaArgs),
    /// Compare two documents and report, without writing anything.
    Compare(CompareArgs),
    /// Measure a printer's second-pass registration, once per printer.
    #[command(subcommand)]
    Calibrate(CalibrateCommand),
    /// Check this machine has what Onionskin needs.
    Doctor,
    /// Open the browser interface, on this machine only.
    Serve(ServeArgs),

    /// List the printers a print server knows about.
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
    #[arg(long, default_value = "a4")]
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
    /// rectangles.
    #[arg(long, default_value = "raster")]
    mode: String,
    /// Rendering resolution. Higher is more exact and slower.
    #[arg(long, default_value_t = onionskin::pipeline::DEFAULT_DPI)]
    dpi: f64,
    /// A calibration profile (see `onionskin calibrate list`).
    #[arg(long)]
    profile: Option<String>,
    /// Warn about additions closer than this to an edge, in mm.
    #[arg(long, default_value_t = onionskin::safety::DEFAULT_MARGIN_MM)]
    margin: f64,
    /// Write proof images here, showing where the new ink lands.
    #[arg(long)]
    preview: Option<PathBuf>,
    /// Draw a box round every change, so what was added is easy to see. The
    /// box is printed onto the paper along with the change.
    #[arg(long)]
    outline: bool,
    /// The colour of those boxes: red, green, blue, black, or 'R,G,B' with
    /// each from 0 to 1.
    #[arg(long, default_value = "red", requires = "outline")]
    outline_colour: String,
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
    #[arg(long, default_value_t = onionskin::pipeline::DEFAULT_DPI)]
    dpi: f64,
    #[arg(long, default_value_t = onionskin::safety::DEFAULT_MARGIN_MM)]
    margin: f64,
    #[arg(long)]
    json: bool,
}

#[derive(Subcommand)]
enum CalibrateCommand {
    /// Write the two-pass target to print twice on one sheet.
    Target(TargetArgs),
    /// Measure the printed sheet from a scan of it, and save the profile.
    Measure(MeasureArgs),
    /// Turn readings you took by hand into a stored profile.
    Solve(SolveArgs),
    /// List the profiles on this machine.
    List,
    /// Show one profile in full.
    Show(ProfileName),
    /// Delete a profile.
    Delete(ProfileName),
}

#[derive(clap::Args)]
struct MeasureArgs {
    /// A scan of the sheet with both passes printed on it.
    scan: PathBuf,
    /// What to call this printer.
    #[arg(long)]
    name: String,
    /// The paper the target was printed on.
    #[arg(long, default_value = "a4")]
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
    #[arg(long, default_value = "a4")]
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
    #[arg(long, default_value = "a4")]
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
    #[arg(long, default_value = "a4")]
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
    #[arg(long, default_value = "a4")]
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
    #[arg(long, default_value = "a4")]
    page: String,
}

#[derive(clap::Args)]
#[command(allow_negative_numbers = true)]
struct InspectArgs {
    /// The scan: PNG, JPEG, TIFF or BMP.
    scan: PathBuf,
    /// Size of the paper that was scanned.
    #[arg(long, default_value = "a4")]
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

    /// Size of the paper that was scanned.
    #[arg(long, default_value = "a4")]
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
    Ok(())
}

const PRINT_INSTRUCTIONS: &str = "\
Printing the delta
  1. Put the scanned sheet back in the tray. Check which way up and which end
     goes first — a page printed upside down is the usual first mistake.
  2. Print at 100% / \"Actual size\". Turn OFF \"Fit to page\"; it scales by a few
     percent and nothing will line up.
  3. Do one sheet first and hold it against the original before committing more.";

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
    match Cli::parse().command {
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
        Command::Print(args) => cmd_print(args),
        Command::Read(args) => cmd_read(args),
        Command::Delta(args) => cmd_delta(args),
        Command::Compare(args) => cmd_compare(args),
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
    println!(
        "\nPut words on it:\n  onionskin write {} --at '25,40:Dear Sir'",
        args.document.display()
    );
    Ok(ExitCode::SUCCESS)
}

fn cmd_write(args: WriteArgs) -> Result<ExitCode, String> {
    if args.at.is_empty() {
        return Err(
            "nothing to write. Say where the words go, for example:\n    \
             --at '25,40:Dear Sir'"
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

    let mut document = Document::load(&args.document).map_err(|e| e.to_string())?;

    let mut added = Vec::new();
    for placement in &args.at {
        let ((x_mm, y_mm), text) = parse_placement(placement)?;
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
    document.save(&args.document).map_err(|e| e.to_string())?;

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
    check_writable(&output, "PDF")?;
    refuse_to_clobber(&output, "PDF", &[(&args.document, "document")])?;

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
    let text = match &font {
        Some(font) => letters::read_with_font(
            &gray,
            &registration,
            &letters::ReadOptions::default(),
            font,
            args.letters.as_deref(),
        ),
        None => letters::read(&gray, &registration, &letters::ReadOptions::default()),
    }
    .map_err(|e| e.to_string())?;

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
    if font.is_none() {
        println!(
            "No font was given, so this is where the letters are, not which they \
             are.\nPass --font-file with the font the page was set in to read them."
        );
    }

    for line in &text.lines {
        println!(
            "\n  {:>6.1} mm  ({:.1}–{:.1} mm across)",
            line.baseline_mm,
            line.rect.x_mm,
            line.rect.right_mm()
        );
        if font.is_some() {
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
        export_page(&text, page, destination, args.flow, font.is_some())?;
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
    // than instead of the network scanners that may well be there.
    let attached = list_devices().unwrap_or_default();
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
        println!(
            "No scanners found.\n\n\
             Check it is switched on, and plugged in or on this network. Onionskin\n\
             drives an attached scanner through SANE's 'scanimage' — install it with\n\
             your package manager, for example:  sudo apt install sane-utils"
        );
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
    // A PDF or a Word file is a document, not a photograph of one: it already
    // knows its own page size and needs no registering. Only the scanned-image
    // path has a sheet to find.
    if is_document(&args.scan) {
        return add_to_document(args);
    }
    if args.at_scan.is_empty() && args.at_page.is_empty() {
        return Err(
            "nothing to add. Use --at 'X,Y:the words' with coordinates read off the \
             scan, or --at-mm 'X,Y:the words' with millimetres measured on the paper."
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
    let asked = args.font.as_deref().map(str::to_string);
    let matched = if embedded.is_some() || args.no_match_font || (asked.is_some() && args.size.is_some())
    {
        None
    } else {
        onionskin::typeface::match_scan(&args.scan, &args.page, args.cropped, args.square)
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
    check_writable(&output, "delta")?;
    refuse_to_clobber(&output, "delta", &[(&args.scan, "scan")])?;
    if let Some(preview) = &args.preview {
        check_writable(preview, "proof")?;
        refuse_to_clobber(
            preview,
            "proof",
            &[(&args.scan, "scan"), (&output, "delta")],
        )?;
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
    for check in checks {
        match check.severity {
            onionskin::safety::Severity::Note => println!("{}", check.format()),
            _ => eprintln!("{}", check.format()),
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
    let outline = args
        .outline
        .then(|| {
            parse_colour(&args.outline_colour).map(|colour| onionskin::delta::Outline {
                colour,
                ..Default::default()
            })
        })
        .transpose()?;
    let options = delta_options(
        &args.mode,
        args.dpi,
        args.margin,
        args.profile.clone(),
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
    for path in &outcome.previews {
        println!("proof: {}", path.display());
    }
    println!("\n{PRINT_INSTRUCTIONS}");
    open_if_asked(args.open, &output);
    Ok(ExitCode::SUCCESS)
}

fn cmd_compare(args: CompareArgs) -> Result<ExitCode, String> {
    // Somewhere to put the delta that is thrown away afterwards, so that
    // "report, write nothing" really does write nothing anyone will find.
    let scratch = onionskin::render::Workspace::new(false).map_err(|e| e.to_string())?;
    let options = delta_options("raster", args.dpi, args.margin, None, None, None)?;

    let outcome = pipeline::run(
        &args.original,
        &args.edited,
        &scratch.path.join("delta.pdf"),
        &options,
    )
    .map_err(|e| e.to_string())?;

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
fn is_document(path: &Path) -> bool {
    let suffix = path
        .extension()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    onionskin::render::CONVERTIBLE.contains(&suffix.as_str())
        || onionskin::render::PASSTHROUGH.contains(&suffix.as_str())
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
    if args.at_page.is_empty() {
        return Err(
            "nothing to add. Say where the words go, in millimetres from the \
             top-left of the paper:\n    --at-mm '45,63:Approved'"
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

    let options = pipeline::Options {
        margin_mm: args.margin,
        preview_dir: args.preview.clone(),
        ..Default::default()
    };
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
    for path in &outcome.previews {
        println!("proof: {}", path.display());
    }
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

/// Measure a printed calibration sheet from a scan of it.
///
/// The part of calibration that was a chore. Reading eight offsets off paper
/// with a ruler, in tenths of a millimetre, is unpleasant to do and easy to do
/// badly — and the numbers it produces are the ones every later delta is
/// placed by. The scanner can read them instead, and it reads them better.
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
    check_writable(&output, "delta")?;
    refuse_to_clobber(&output, "delta", &[(&args.document, "document")])?;

    let mut items = Vec::new();
    for placement in &args.at {
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

    let options = pipeline::Options {
        preview_dir: args.preview.clone(),
        ..Default::default()
    };
    let outcome = pipeline::compose_run(&args.document, &items, &output, None, &options)
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
    for path in &outcome.previews {
        println!("proof: {}", path.display());
    }
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
    check_writable(&output, "delta")?;
    refuse_to_clobber(&output, "delta", &[(&args.document, "document")])?;

    let placed: Vec<(usize, onionskin::pdf::PlacedShape)> =
        shapes.iter().map(|shape| (shape.page, shape.placed())).collect();

    let options = pipeline::Options {
        preview_dir: args.preview.clone(),
        ..Default::default()
    };
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
    for path in &outcome.previews {
        println!("proof: {}", path.display());
    }
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
                    "No calibration profiles yet.\n\nMake one:\n  onionskin calibrate \
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
fn cmd_doctor() -> Result<ExitCode, String> {
    let mut everything_works = true;
    println!("Onionskin {}\n", env!("CARGO_PKG_VERSION"));

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

    println!("\nTry it:\n  onionskin doctor");
    println!("\nTo remove it later:  onionskin uninstall");
    Ok(ExitCode::SUCCESS)
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
