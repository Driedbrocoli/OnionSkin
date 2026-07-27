//! Autocalibration, end to end: print a job, scan it back, learn the printer.
//!
//! Calibration used to mean sitting down to do calibration — print the target,
//! find a ruler, read eight offsets in tenths of a millimetre, type them in.
//! Most people were never going to. This is the same measurement taken from a
//! job that was being printed anyway, so the printer gets measured by somebody
//! using the program rather than by somebody setting it up.
//!
//! The test simulates the one thing that cannot be simulated in a unit test:
//! the round trip. A real delta is written by the real command, rendered as the
//! printer would put it on paper, laid down a known distance from where it was
//! asked for, and photographed onto a sheet with margins and a bit of skew. If
//! the number that comes back out is the distance that went in, the whole chain
//! — placement, rendering, registration, centroid, fit — is right.
//!
//! The same round trip covers `verify`, which asks the other question the scan
//! can answer: not "what does this printer do" but "did this sheet come out
//! right", which one addition can settle and which wants settling before the
//! other fifty-nine go through.

use std::path::{Path, PathBuf};
use std::process::Command;

use image::{DynamicImage, RgbImage};
use onionskin::geometry::PageSize;

const A4: PageSize = PageSize {
    width_mm: 210.0,
    height_mm: 297.0,
};

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_onionskin"))
}

struct Run {
    ok: bool,
    code: i32,
    stdout: String,
    stderr: String,
}

impl Run {
    /// The whole of what the user saw, for an assertion message worth reading.
    fn said(&self) -> String {
        format!("{}{}", self.stdout, self.stderr)
    }
}

/// Run the command with a home of its own, so a test never reads or writes the
/// profiles of whoever is running it.
fn run(home: &Path, args: &[&str]) -> Run {
    let output = Command::new(binary())
        .args(args)
        .env("ONIONSKIN_HOME", home)
        .output()
        .expect("the binary should run");
    Run {
        ok: output.status.success(),
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    }
}

/// How the printer behaved on the second pass, and how the sheet went into the
/// scanner afterwards.
struct Printed {
    /// What the printer did to the delta, in millimetres of paper.
    off_by: (f64, f64),
    /// How crooked the sheet sat on the glass.
    skew_deg: f64,
    /// The resolution of the scan, which is not the resolution of the print.
    dpi: f64,
    /// Dark scanner backing around the sheet.
    margin: u32,
}

impl Printed {
    fn shifted(dx: f64, dy: f64) -> Printed {
        Printed {
            off_by: (dx, dy),
            skew_deg: 0.0,
            dpi: 200.0,
            margin: 60,
        }
    }

    /// The shift this simulation can actually deliver.
    ///
    /// Ink is laid down on whole pixels of the scan, so asking for 1.2 mm at
    /// 200 dpi puts 1.143 mm on the paper. Measuring against what was asked for
    /// rather than what was laid down would leave a tenth of a millimetre of
    /// slack in every assertion here — which is most of the accuracy this
    /// feature is for.
    fn really_off_by(&self) -> (f64, f64) {
        let px_per_mm = self.dpi / 25.4;
        (
            (self.off_by.0 * px_per_mm).round() / px_per_mm,
            (self.off_by.1 * px_per_mm).round() / px_per_mm,
        )
    }
}

/// A scan of the sheet as it came out of the printer.
///
/// The delta is rendered exactly as a printer would raster it, then stamped
/// onto blank paper `off_by` millimetres from where it asked to be — which is
/// the error the printer is about to be measured for.
fn scan_of_the_printed_sheet(delta: &Path, how: &Printed) -> DynamicImage {
    let engine = onionskin::render::engine().expect("pdfium should be available");
    let doc = engine.open(delta).expect("the delta should open");
    let page = doc
        .render_gray(0, how.dpi)
        .expect("the delta should render");

    let px_per_mm = how.dpi / 25.4;
    let sheet_w = (A4.width_mm * px_per_mm) as u32;
    let sheet_h = (A4.height_mm * px_per_mm) as u32;
    let width = sheet_w + how.margin * 2;
    let height = sheet_h + how.margin * 2;

    // Paper, then ink, then the whole sheet turned as it sits on the glass.
    let mut sheet = vec![245u8; (sheet_w * sheet_h) as usize];
    let shift_x = (how.off_by.0 * px_per_mm).round() as i64;
    let shift_y = (how.off_by.1 * px_per_mm).round() as i64;
    for y in 0..page.height {
        for x in 0..page.width {
            let value = page.gray[y * page.width + x];
            if value >= 200 {
                continue; // the delta's blank page is not ink
            }
            let tx = x as i64 + shift_x;
            let ty = y as i64 + shift_y;
            if tx < 0 || ty < 0 || tx >= sheet_w as i64 || ty >= sheet_h as i64 {
                continue;
            }
            sheet[ty as usize * sheet_w as usize + tx as usize] = value.min(30);
        }
    }

    let mut img = RgbImage::from_pixel(width, height, image::Rgb([38, 40, 44]));
    let centre = (width as f64 / 2.0, height as f64 / 2.0);
    let (sin_t, cos_t) = how.skew_deg.to_radians().sin_cos();
    for y in 0..height {
        for x in 0..width {
            let (dx, dy) = (x as f64 - centre.0, y as f64 - centre.1);
            let sx = cos_t * dx + sin_t * dy + centre.0 - how.margin as f64;
            let sy = -sin_t * dx + cos_t * dy + centre.1 - how.margin as f64;
            if sx < 0.0 || sy < 0.0 || sx >= sheet_w as f64 || sy >= sheet_h as f64 {
                continue;
            }
            let value = sheet[sy as usize * sheet_w as usize + sx as usize];
            img.put_pixel(x, y, image::Rgb([value, value, value]));
        }
    }
    DynamicImage::ImageRgb8(img)
}

/// The sheet that is already in the tray: an ordinary PDF, printed once.
fn a_sheet_already_printed(dir: &Path) -> PathBuf {
    let doc = dir.join("letter.onionskin");
    let pdf = dir.join("letter.pdf");
    let made = run(dir, &["new", doc.to_str().unwrap()]);
    assert!(made.ok, "could not start a document: {}", made.said());
    let printed = run(
        dir,
        &["print", doc.to_str().unwrap(), "-o", pdf.to_str().unwrap()],
    );
    assert!(printed.ok, "could not print it: {}", printed.said());
    pdf
}

/// A delta with words spread across the sheet, which is what a fit needs: marks
/// close together say where the paper sits but nothing about rotation or scale.
///
/// `extra` is where the profile goes when the job is being printed corrected.
fn a_job_worth_learning_from(dir: &Path, source: &Path, out: &Path, extra: &[&str]) {
    let mut args = vec![
        "write",
        source.to_str().unwrap(),
        "--at",
        "30,40:Received",
        "--at",
        "150,45:14 March",
        "--at",
        "30,250:Signed",
        "--at",
        "140,255:J Bezzina",
        "--size",
        "14",
        "-o",
        out.to_str().unwrap(),
    ];
    args.extend_from_slice(extra);
    let written = run(dir, &args);
    assert!(written.ok, "could not write the delta: {}", written.said());
}

/// Pull "+1.20 mm" style numbers out of the line describing the printer error.
fn error_line(said: &str) -> String {
    said.lines()
        .find(|line| line.contains("printer error"))
        .unwrap_or_else(|| panic!("no printer error was reported in:\n{said}"))
        .to_string()
}

// ---------------------------------------------------------------------------

/// The measurement that makes calibration automatic: a job printed 1.2 mm to
/// the right and 0.7 mm down is reported as a printer that shifts by that much.
#[test]
fn a_printers_shift_is_read_back_off_the_job_it_printed() {
    let dir = tempfile::tempdir().unwrap();
    let source = a_sheet_already_printed(dir.path());
    let delta = dir.path().join("delta.pdf");
    a_job_worth_learning_from(dir.path(), &source, &delta, &[]);
    let how = Printed::shifted(1.2, 0.7);
    let scan = dir.path().join("scan.png");
    scan_of_the_printed_sheet(&delta, &how).save(&scan).unwrap();

    let learnt = run(
        dir.path(),
        &[
            "calibrate",
            "learn",
            scan.to_str().unwrap(),
            "--delta",
            delta.to_str().unwrap(),
            "--name",
            "office",
        ],
    );
    assert!(learnt.ok, "learning failed: {}", learnt.said());

    let profile = {
        let saved = run(dir.path(), &["calibrate", "show", "office"]);
        assert!(saved.ok, "the profile was not saved: {}", saved.said());
        saved.stdout
    };

    // Read the numbers back off the stored profile rather than the printout,
    // because the printout is prose and the profile is the thing every later
    // delta is placed by.
    let stored: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(dir.path().join("profiles").join("office.json")).unwrap(),
    )
    .unwrap();
    let dx = stored["error"]["dx_mm"].as_f64().unwrap();
    let dy = stored["error"]["dy_mm"].as_f64().unwrap();
    let (want_x, want_y) = how.really_off_by();
    // A tenth of a millimetre. Loose enough for the rounding a raster imposes,
    // tight enough to fail if the two halves of a landing are ever measured
    // differently again — that cost a quarter of a millimetre when it was so.
    assert!(
        (dx - want_x).abs() < 0.1 && (dy - want_y).abs() < 0.1,
        "printed {want_x:.3},{want_y:.3} off but learnt {dx:.2},{dy:.2}\n{profile}"
    );
    // And the fit should say it fits, because a pure shift is exactly what a
    // similarity can represent.
    let rms = stored["rms_residual_mm"].as_f64().unwrap();
    assert!(
        rms < 0.12,
        "the fit missed by {rms:.3} mm on average\n{profile}"
    );
    assert!(
        learnt.stdout.contains("out by"),
        "the user was not told where the additions landed:\n{}",
        learnt.stdout
    );
}

/// The sheet nearly always goes onto the glass crooked, and that crookedness
/// is the scanner's, not the printer's.
///
/// If it were learnt as rotation the printer introduces, every delta afterwards
/// would be turned by however carelessly one sheet was laid down — which is
/// worse than not calibrating at all, because it is confidently wrong.
#[test]
fn a_crooked_scan_does_not_become_a_crooked_printer() {
    let dir = tempfile::tempdir().unwrap();
    let source = a_sheet_already_printed(dir.path());
    let delta = dir.path().join("delta.pdf");
    a_job_worth_learning_from(dir.path(), &source, &delta, &[]);

    let how = Printed {
        off_by: (1.0, 0.5),
        skew_deg: 2.0,
        dpi: 200.0,
        margin: 90,
    };
    let scan = dir.path().join("scan.png");
    scan_of_the_printed_sheet(&delta, &how).save(&scan).unwrap();

    let learnt = run(
        dir.path(),
        &[
            "calibrate",
            "learn",
            scan.to_str().unwrap(),
            "--delta",
            delta.to_str().unwrap(),
            "--name",
            "office",
        ],
    );
    assert!(learnt.ok, "learning failed: {}", learnt.said());

    let stored: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(dir.path().join("profiles").join("office.json")).unwrap(),
    )
    .unwrap();
    let rotation = stored["error"]["rotation_deg"].as_f64().unwrap();
    assert!(
        rotation.abs() < 0.5,
        "a 2° crooked scan was learnt as a {rotation:.2}° crooked printer\n{}",
        learnt.stdout
    );
    let (dx, dy) = (
        stored["error"]["dx_mm"].as_f64().unwrap(),
        stored["error"]["dy_mm"].as_f64().unwrap(),
    );
    let (want_x, want_y) = how.really_off_by();
    assert!(
        (dx - want_x).abs() < 0.25 && (dy - want_y).abs() < 0.25,
        "shift read as {dx:.2},{dy:.2} off a crooked scan, not {want_x:.3},{want_y:.3}\n{}",
        learnt.stdout
    );
}

/// Nothing is written to disk on a dry run, so somebody can look at what a job
/// says about their printer before letting it change how everything prints.
#[test]
fn a_dry_run_measures_without_changing_anything() {
    let dir = tempfile::tempdir().unwrap();
    let source = a_sheet_already_printed(dir.path());
    let delta = dir.path().join("delta.pdf");
    a_job_worth_learning_from(dir.path(), &source, &delta, &[]);
    let scan = dir.path().join("scan.png");
    scan_of_the_printed_sheet(&delta, &Printed::shifted(0.9, -0.6))
        .save(&scan)
        .unwrap();

    let learnt = run(
        dir.path(),
        &[
            "calibrate",
            "learn",
            scan.to_str().unwrap(),
            "--delta",
            delta.to_str().unwrap(),
            "--name",
            "office",
            "--dry-run",
        ],
    );
    assert!(learnt.ok, "learning failed: {}", learnt.said());
    assert!(!error_line(&learnt.stdout).is_empty());
    assert!(
        !dir.path().join("profiles").join("office.json").exists(),
        "--dry-run saved a profile anyway"
    );
}

/// A correction already in force is not thrown away.
///
/// This is the difference between calibration that improves and calibration
/// that oscillates. Once a profile is correcting 1.2 mm and the printer is
/// therefore landing on the mark, a second learn must conclude "the printer
/// still shifts by 1.2 mm" — not "the printer is perfect", which would drop the
/// correction and put the error straight back.
#[test]
fn learning_again_keeps_the_correction_that_is_already_working() {
    let dir = tempfile::tempdir().unwrap();
    let source = a_sheet_already_printed(dir.path());
    let delta = dir.path().join("delta.pdf");
    a_job_worth_learning_from(dir.path(), &source, &delta, &[]);
    let scan = dir.path().join("scan.png");

    // Round one: measured cold, off by 1.2, 0.7.
    scan_of_the_printed_sheet(&delta, &Printed::shifted(1.2, 0.7))
        .save(&scan)
        .unwrap();
    let first = run(
        dir.path(),
        &[
            "calibrate",
            "learn",
            scan.to_str().unwrap(),
            "--delta",
            delta.to_str().unwrap(),
            "--name",
            "office",
        ],
    );
    assert!(first.ok, "{}", first.said());

    // Round two: the delta now goes out corrected, so it lands on the mark and
    // the scan shows no error at all.
    let corrected = dir.path().join("delta-2.pdf");
    a_job_worth_learning_from(dir.path(), &source, &corrected, &["--profile", "office"]);
    scan_of_the_printed_sheet(&corrected, &Printed::shifted(1.2, 0.7))
        .save(&scan)
        .unwrap();
    let second = run(
        dir.path(),
        &[
            "calibrate",
            "learn",
            scan.to_str().unwrap(),
            "--delta",
            corrected.to_str().unwrap(),
            "--name",
            "office",
        ],
    );
    assert!(second.ok, "{}", second.said());

    let stored: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(dir.path().join("profiles").join("office.json")).unwrap(),
    )
    .unwrap();
    let dx = stored["error"]["dx_mm"].as_f64().unwrap();
    let dy = stored["error"]["dy_mm"].as_f64().unwrap();
    let (want_x, want_y) = Printed::shifted(1.2, 0.7).really_off_by();
    assert!(
        (dx - want_x).abs() < 0.15 && (dy - want_y).abs() < 0.15,
        "the correction was thrown away: printed {want_x:.3},{want_y:.3} off, \
         but learnt {dx:.2},{dy:.2} the second time"
    );
}

/// A delta with nothing on it is refused by name, rather than fitting a
/// transform to no points and saving nonsense over a good profile.
#[test]
fn an_empty_delta_is_refused_rather_than_learnt_from() {
    let dir = tempfile::tempdir().unwrap();
    let doc = dir.path().join("blank.onionskin");
    assert!(run(dir.path(), &["new", doc.to_str().unwrap()]).ok);
    let empty = dir.path().join("empty.pdf");
    let made = run(
        dir.path(),
        &[
            "print",
            doc.to_str().unwrap(),
            "-o",
            empty.to_str().unwrap(),
        ],
    );
    assert!(made.ok, "{}", made.said());

    let scan = dir.path().join("scan.png");
    scan_of_the_printed_sheet(&empty, &Printed::shifted(0.0, 0.0))
        .save(&scan)
        .unwrap();
    let learnt = run(
        dir.path(),
        &[
            "calibrate",
            "learn",
            scan.to_str().unwrap(),
            "--delta",
            empty.to_str().unwrap(),
            "--name",
            "office",
        ],
    );
    assert!(!learnt.ok, "an empty delta was accepted");
    assert!(
        learnt.said().contains("nothing"),
        "unhelpful refusal: {}",
        learnt.said()
    );
}

// ---------------------------------------------------------------------------
// Checking one sheet before committing the rest of the stack
// ---------------------------------------------------------------------------

/// A sheet that came out right says so, and exits 0 so a script can act on it.
#[test]
fn a_sheet_that_printed_properly_passes() {
    let dir = tempfile::tempdir().unwrap();
    let source = a_sheet_already_printed(dir.path());
    let delta = dir.path().join("delta.pdf");
    a_job_worth_learning_from(dir.path(), &source, &delta, &[]);

    // A fifth of a millimetre out, which is a well-behaved printer.
    let scan = dir.path().join("scan.png");
    scan_of_the_printed_sheet(&delta, &Printed::shifted(0.2, 0.1))
        .save(&scan)
        .unwrap();

    let checked = run(
        dir.path(),
        &[
            "verify",
            scan.to_str().unwrap(),
            "--delta",
            delta.to_str().unwrap(),
        ],
    );
    assert_eq!(checked.code, 0, "{}", checked.said());
    assert!(
        checked.stdout.contains("Everything printed"),
        "{}",
        checked.stdout
    );
}

/// A sheet that landed too far out is refused, with the distance and what to
/// do about it — and exits 2, so fifty-nine more do not follow it.
#[test]
fn a_sheet_that_landed_too_far_out_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let source = a_sheet_already_printed(dir.path());
    let delta = dir.path().join("delta.pdf");
    a_job_worth_learning_from(dir.path(), &source, &delta, &[]);

    let scan = dir.path().join("scan.png");
    scan_of_the_printed_sheet(&delta, &Printed::shifted(3.0, 2.0))
        .save(&scan)
        .unwrap();

    let checked = run(
        dir.path(),
        &[
            "verify",
            scan.to_str().unwrap(),
            "--delta",
            delta.to_str().unwrap(),
        ],
    );
    assert_eq!(checked.code, 2, "{}", checked.said());
    assert!(
        checked.stdout.contains("landed more than"),
        "{}",
        checked.stdout
    );
    // And it points at the thing that fixes it, rather than only complaining.
    assert!(
        checked.stdout.contains("calibrate learn"),
        "{}",
        checked.stdout
    );
}

/// The tolerance is the caller's to set: the same sheet passes when a
/// signature is being placed and fails when a pre-printed box is being filled.
#[test]
fn how_close_is_close_enough_is_the_callers_to_say() {
    let dir = tempfile::tempdir().unwrap();
    let source = a_sheet_already_printed(dir.path());
    let delta = dir.path().join("delta.pdf");
    a_job_worth_learning_from(dir.path(), &source, &delta, &[]);
    let scan = dir.path().join("scan.png");
    scan_of_the_printed_sheet(&delta, &Printed::shifted(1.5, 0.0))
        .save(&scan)
        .unwrap();

    let check = |tolerance: &str| {
        run(
            dir.path(),
            &[
                "verify",
                scan.to_str().unwrap(),
                "--delta",
                delta.to_str().unwrap(),
                "--tolerance",
                tolerance,
            ],
        )
    };
    assert_eq!(check("3.0").code, 0, "{}", check("3.0").said());
    assert_eq!(check("0.5").code, 2, "{}", check("0.5").said());
}

/// A sheet that went through the printer the wrong way up carries none of the
/// delta at all, and that is the failure worth naming rather than measuring.
#[test]
fn a_sheet_with_nothing_on_it_says_the_additions_did_not_print() {
    let dir = tempfile::tempdir().unwrap();
    let source = a_sheet_already_printed(dir.path());
    let delta = dir.path().join("delta.pdf");
    a_job_worth_learning_from(dir.path(), &source, &delta, &[]);

    // Blank paper: the delta never reached it.
    let blank = dir.path().join("blank.pdf");
    let doc = dir.path().join("nothing.onionskin");
    assert!(run(dir.path(), &["new", doc.to_str().unwrap()]).ok);
    assert!(
        run(
            dir.path(),
            &[
                "print",
                doc.to_str().unwrap(),
                "-o",
                blank.to_str().unwrap()
            ],
        )
        .ok
    );
    let scan = dir.path().join("scan.png");
    scan_of_the_printed_sheet(&blank, &Printed::shifted(0.0, 0.0))
        .save(&scan)
        .unwrap();

    let checked = run(
        dir.path(),
        &[
            "verify",
            scan.to_str().unwrap(),
            "--delta",
            delta.to_str().unwrap(),
        ],
    );
    assert_eq!(checked.code, 2, "{}", checked.said());
    assert!(
        checked.stdout.contains("did not print"),
        "{}",
        checked.stdout
    );
    assert!(
        checked.stdout.contains("wrong way up"),
        "the most likely cause was not named:\n{}",
        checked.stdout
    );
}

/// Checking and learning off the same scan, because somebody who went to the
/// trouble of scanning the sheet may as well get the measurement out of it.
#[test]
fn the_same_scan_can_check_the_sheet_and_teach_the_printer() {
    let dir = tempfile::tempdir().unwrap();
    let source = a_sheet_already_printed(dir.path());
    let delta = dir.path().join("delta.pdf");
    a_job_worth_learning_from(dir.path(), &source, &delta, &[]);
    let how = Printed::shifted(1.2, 0.7);
    let scan = dir.path().join("scan.png");
    scan_of_the_printed_sheet(&delta, &how).save(&scan).unwrap();

    let checked = run(
        dir.path(),
        &[
            "verify",
            scan.to_str().unwrap(),
            "--delta",
            delta.to_str().unwrap(),
            "--learn",
            "office",
        ],
    );
    // The sheet is out of tolerance and says so…
    assert_eq!(checked.code, 2, "{}", checked.said());
    // …and the profile was still saved, because that is a different question.
    let stored: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(dir.path().join("profiles").join("office.json")).unwrap(),
    )
    .unwrap();
    let (want_x, want_y) = how.really_off_by();
    let dx = stored["error"]["dx_mm"].as_f64().unwrap();
    let dy = stored["error"]["dy_mm"].as_f64().unwrap();
    assert!(
        (dx - want_x).abs() < 0.1 && (dy - want_y).abs() < 0.1,
        "learnt {dx:.2},{dy:.2} rather than {want_x:.3},{want_y:.3}\n{}",
        checked.stdout
    );
}
