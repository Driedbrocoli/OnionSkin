//! Onionskin — add words to a page that is already printed.
//!
//! This binary covers the scanned-page workflow: you have a sheet in your hand
//! and an image of it, and you want to write something onto the paper itself.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use onionskin::geometry::{parse_page, PageSize};
use onionskin::acquire::{
    acquire, list_devices, scanning_available, unavailable_reason, AcquireOptions,
    PLACEMENT_ADVICE,
};
use onionskin::font::{suggest_system_font, EmbeddedFont};
use onionskin::pdf::{write_delta, Font, LineFont, PlacedLine};
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
    Fonts,
}

#[derive(clap::Args)]
struct AcquireArgs {
    /// Where to write the scan.
    #[arg(short, long)]
    output: PathBuf,
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
    /// Delta PDF to write.
    #[arg(short, long)]
    output: PathBuf,

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
    /// Type size in points.
    #[arg(long, default_value_t = 11.0)]
    size: f64,
    /// One of the built-in fonts (see `onionskin fonts`).
    #[arg(long, default_value = "Helvetica")]
    font: String,
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
        std::env::current_dir()
            .unwrap_or_default()
            .join(path)
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
    match run() {
        Ok(code) => code,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::from(1)
        }
    }
}

fn run() -> Result<ExitCode, String> {
    match Cli::parse().command {
        Command::Fonts => {
            println!("Fonts built into every PDF reader:");
            for font in Font::all() {
                println!("  {}", font.base_name());
            }
            println!(
                "\nThese cover Western European text only. For any other alphabet, \
                 pass\n--font-file with a .ttf or .ttc and it will be carried inside \
                 the delta."
            );
            if let Some(path) = suggest_system_font() {
                println!("\nThere is one on this machine: {}", path.display());
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Inspect(args) => cmd_inspect(args),
        Command::Add(args) => cmd_add(args),
        Command::Acquire(args) => cmd_acquire(args),
        Command::Scanners => cmd_scanners(),
    }
}

fn cmd_scanners() -> Result<ExitCode, String> {
    let devices = list_devices().map_err(|e| e.to_string())?;
    if devices.is_empty() {
        println!("No scanners found. Check the scanner is switched on and plugged in.");
        return Ok(ExitCode::from(1));
    }
    println!("Scanners this machine can see:");
    for device in &devices {
        println!("  {}", device.description);
        println!("    --device {}", device.name);
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
                 onionskin add {} -o delta.pdf --at 'X,Y:the words'",
                path.display()
            );
            Ok(ExitCode::SUCCESS)
        }
        Err(message) => {
            println!(
                "\nThe scan was saved, but Onionskin cannot measure the sheet in it:\n  \
                 {message}\n\nThe sheet is still on the glass — it is usually quicker to \
                 fix the placement\nand scan again than to work around it."
            );
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

fn cmd_add(args: AddArgs) -> Result<ExitCode, String> {
    if args.at_scan.is_empty() && args.at_page.is_empty() {
        return Err(
            "nothing to add. Use --at 'X,Y:the words' with coordinates read off the \
             scan, or --at-mm 'X,Y:the words' with millimetres measured on the paper."
                .into(),
        );
    }
    if !(args.size.is_finite() && args.size > 0.0 && args.size <= 400.0) {
        return Err(format!(
            "type size {} pt is out of range (1 to 400)",
            args.size
        ));
    }
    if !args.rotation.is_finite() {
        return Err("rotation must be a real number".into());
    }
    // A supplied font wins: asking for one and silently getting Helvetica is
    // how the Python side once made --font-file appear to do nothing.
    let embedded = match &args.font_file {
        Some(path) => Some(
            EmbeddedFont::load_indexed(path, args.font_index).map_err(|e| e.to_string())?,
        ),
        None => None,
    };
    let line_font = match &embedded {
        Some(_) => LineFont::Embedded,
        None => LineFont::Builtin(Font::parse(&args.font).ok_or_else(|| {
            let names: Vec<&str> = Font::all().iter().map(|f| f.base_name()).collect();
            format!(
                "unknown font '{}'. Available: {}\n\
                 For another alphabet, pass --font-file with a .ttf.",
                args.font,
                names.join(", ")
            )
        })?),
    };

    // Check the destinations before doing any work, so a mistake costs a
    // message rather than the scan.
    check_writable(&args.output, "delta")?;
    refuse_to_clobber(&args.output, "delta", &[(&args.scan, "scan")])?;
    if let Some(preview) = &args.preview {
        check_writable(preview, "proof")?;
        refuse_to_clobber(
            preview,
            "proof",
            &[(&args.scan, "scan"), (&args.output, "delta")],
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
            let step = onionskin::geometry::pt_to_mm(args.size * 1.15) * index as f64;
            lines.push(PlacedLine {
                text: part.to_string(),
                x_mm: position_mm.0,
                y_mm: position_mm.1 + step,
                size_pt: args.size,
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
        &args.output,
        &[page],
        &[lines.clone()],
        "Onionskin delta",
        embedded.as_ref(),
    )
    .map_err(|message| {
        // Point at a font that is actually on this machine, rather than
        // leaving someone to hunt for one.
        let text = message.to_string();
        match (text.contains("cannot write these characters"), suggest_system_font()) {
            (true, Some(path)) => format!(
                "{text}\n    There is one on this machine: --font-file {}",
                path.display()
            ),
            _ => text,
        }
    })?;

    println!("Wrote {}", args.output.display());
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
