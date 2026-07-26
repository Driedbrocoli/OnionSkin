//! End-to-end tests for the scanned-page workflow.
//!
//! The property that matters is one sentence long: a spot picked on the scan
//! must resolve to the right physical millimetre on the paper. Everything else
//! — the detection, the deskewing, the arithmetic — only exists to make that
//! true, so it is tested across the whole range of scans a person might
//! actually feed in rather than at one convenient point.

use std::path::{Path, PathBuf};
use std::process::Command;

use image::{DynamicImage, RgbImage};
use onionskin::geometry::PageSize;
use onionskin::scan::{register, ScanOptions};

const A4: PageSize = PageSize {
    width_mm: 210.0,
    height_mm: 297.0,
};

/// A synthetic flatbed scan: a sheet of "text" turned by `skew_deg`, sitting on
/// dark scanner backing with a margin around it.
struct Scan {
    image: DynamicImage,
    page: PageSize,
    dpi: f64,
    margin: u32,
    skew_deg: f64,
    sheet_px: (u32, u32),
}

impl Scan {
    fn build(page: PageSize, dpi: f64, margin: u32, skew_deg: f64, lines: usize) -> Scan {
        Scan::build_with(page, dpi, margin, skew_deg, lines, [38, 40, 44], 245, 25)
    }

    fn build_with(
        page: PageSize,
        dpi: f64,
        margin: u32,
        skew_deg: f64,
        lines: usize,
        backing: [u8; 3],
        paper: u8,
        ink: u8,
    ) -> Scan {
        let px_per_mm = dpi / 25.4;
        let sheet_w = (page.width_mm * px_per_mm) as u32;
        let sheet_h = (page.height_mm * px_per_mm) as u32;
        let width = sheet_w + margin * 2;
        let height = sheet_h + margin * 2;

        let mut img = RgbImage::from_pixel(width, height, image::Rgb(backing));
        let centre = (width as f64 / 2.0, height as f64 / 2.0);
        let (sin_t, cos_t) = skew_deg.to_radians().sin_cos();

        for y in 0..height {
            for x in 0..width {
                let (dx, dy) = (x as f64 - centre.0, y as f64 - centre.1);
                let sx = cos_t * dx + sin_t * dy + centre.0 - margin as f64;
                let sy = -sin_t * dx + cos_t * dy + centre.1 - margin as f64;
                if sx < 0.0 || sy < 0.0 || sx >= sheet_w as f64 || sy >= sheet_h as f64 {
                    continue;
                }
                let mut value = paper;
                if lines > 0 {
                    let band = sheet_h as f64 / (lines as f64 * 3.0);
                    if (sy / band) as usize % 3 == 1
                        && sx > sheet_w as f64 * 0.1
                        && sx < sheet_w as f64 * 0.8
                    {
                        value = ink;
                    }
                }
                img.put_pixel(x, y, image::Rgb([value, value, value]));
            }
        }

        Scan {
            image: DynamicImage::ImageRgb8(img),
            page,
            dpi,
            margin,
            skew_deg,
            sheet_px: (sheet_w, sheet_h),
        }
    }

    /// Where a known point on the paper appears in this scan.
    fn pixel_of(&self, mm: (f64, f64)) -> (f64, f64) {
        let px_per_mm = self.dpi / 25.4;
        let centre = (
            (self.sheet_px.0 + self.margin * 2) as f64 / 2.0,
            (self.sheet_px.1 + self.margin * 2) as f64 / 2.0,
        );
        let (sin_t, cos_t) = self.skew_deg.to_radians().sin_cos();
        let ux = mm.0 * px_per_mm + self.margin as f64 - centre.0;
        let uy = mm.1 * px_per_mm + self.margin as f64 - centre.1;
        (
            cos_t * ux - sin_t * uy + centre.0,
            sin_t * ux + cos_t * uy + centre.1,
        )
    }

    fn save(&self, path: &Path) {
        self.image.save(path).unwrap();
    }
}

fn binary() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_onionskin"))
}

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

fn run(args: &[&str]) -> Run {
    let output = Command::new(binary()).args(args).output().unwrap();
    Run {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    }
}

// --- the property that matters ---------------------------------------------

/// A point picked on the scan lands on the right millimetre of paper, across
/// the whole range of scans a flatbed produces.
#[test]
fn picked_points_resolve_to_the_right_place_on_paper() {
    let pages = [
        A4,
        PageSize::new(297.0, 210.0),
        PageSize::new(215.9, 279.4),
        PageSize::new(148.0, 210.0),
    ];
    let mut worst: f64 = 0.0;
    let mut checked = 0;

    for page in pages {
        for dpi in [150.0, 200.0, 300.0] {
            for skew in [-3.0, -1.2, 0.0, 0.9, 2.5] {
                // Large enough that a turned sheet still fits inside the
                // image — a sheet running off the edge is refused, and is
                // covered by its own test.
                for margin in [90u32, 160] {
                    let scan = Scan::build(page, dpi, margin, skew, 24);
                    let registration = register(&scan.image, ScanOptions::new(page)).unwrap();

                    for target in [
                        (20.0, 25.0),
                        (page.width_mm / 2.0, page.height_mm / 2.0),
                        (page.width_mm - 30.0, page.height_mm - 40.0),
                    ] {
                        let recovered = registration.pixel_to_page_mm(scan.pixel_of(target));
                        let error = ((recovered.0 - target.0).powi(2)
                            + (recovered.1 - target.1).powi(2))
                        .sqrt();
                        worst = worst.max(error);
                        checked += 1;
                        assert!(
                            error < 2.0,
                            "{page:?} {dpi}dpi skew {skew} margin {margin}: \
                             {target:?} recovered as {recovered:?} ({error:.2} mm out)"
                        );
                    }
                }
            }
        }
    }
    assert!(checked >= 300, "only {checked} combinations covered");
    println!("{checked} combinations, worst error {worst:.2} mm");
}

#[test]
fn resolution_is_recovered_across_the_usual_settings() {
    for dpi in [100.0, 150.0, 200.0, 300.0, 600.0] {
        let scan = Scan::build(A4, dpi, 30, 0.0, 20);
        let registration = register(&scan.image, ScanOptions::new(A4)).unwrap();
        let error = (registration.dpi() - dpi).abs() / dpi;
        assert!(error < 0.02, "{dpi} dpi read as {:.1}", registration.dpi());
    }
}

#[test]
fn skew_is_recovered_across_the_usual_range() {
    for truth in [-4.0, -2.5, -1.0, -0.3, 0.0, 0.4, 1.1, 2.2, 3.7] {
        let scan = Scan::build(A4, 200.0, 170, truth, 26);
        let registration = register(&scan.image, ScanOptions::new(A4)).unwrap();
        assert!(
            (registration.skew_deg - truth).abs() < 0.3,
            "skew {truth} read as {:.2}",
            registration.skew_deg
        );
    }
}

// --- scans that are not the tidy case --------------------------------------

#[test]
fn a_pale_scan_still_registers() {
    // Some scanners wash everything out; the split has to come from the image.
    let scan = Scan::build_with(A4, 150.0, 90, 1.0, 20, [150, 150, 150], 252, 190);
    let registration = register(&scan.image, ScanOptions::new(A4)).unwrap();
    assert!((registration.dpi() - 150.0).abs() < 6.0);
}

#[test]
fn a_dark_scan_still_registers() {
    let scan = Scan::build_with(A4, 150.0, 90, 1.0, 20, [5, 5, 5], 160, 20);
    let registration = register(&scan.image, ScanOptions::new(A4)).unwrap();
    assert!((registration.dpi() - 150.0).abs() < 6.0);
}

#[test]
fn a_square_sheet_filling_the_image_needs_no_correction() {
    let scan = Scan::build(A4, 200.0, 0, 0.0, 24);
    let registration = register(&scan.image, ScanOptions::new(A4)).unwrap();
    assert_eq!(registration.skew_deg, 0.0);
    assert_eq!(registration.origin_px, (0.0, 0.0));
}

/// A turned sheet with its corners cut off cannot say how big the paper is.
/// Estimating anyway would misplace every addition while looking convincing,
/// so it is refused — the one case where a refusal beats an answer.
#[test]
fn a_turned_sheet_running_off_the_scan_is_refused() {
    for margin in [0u32, 10, 25] {
        let scan = Scan::build(A4, 200.0, margin, 3.0, 24);
        let result = register(&scan.image, ScanOptions::new(A4));
        assert!(
            result.is_err(),
            "margin {margin}: a clipped sheet should be refused, got {:?}",
            result.map(|r| r.describe())
        );
        let message = result.unwrap_err().to_string();
        assert!(message.contains("runs off the edge"), "{message}");
    }
}

#[test]
fn a_clipped_scan_can_still_be_used_by_declaring_it_cropped() {
    let scan = Scan::build(A4, 200.0, 0, 3.0, 24);
    let mut options = ScanOptions::new(A4);
    options.assume_cropped = true;
    let registration = register(&scan.image, options).unwrap();
    assert_eq!(registration.origin_px, (0.0, 0.0));
}

/// Tenths of a degree still matter: 0.3° is a millimetre and a half by the
/// bottom of an A4 page.
#[test]
fn small_angles_are_measured_not_rounded_away() {
    for truth in [-0.6, -0.3, 0.2, 0.5] {
        let scan = Scan::build(A4, 200.0, 120, truth, 24);
        let registration = register(&scan.image, ScanOptions::new(A4)).unwrap();
        assert!(
            (registration.skew_deg - truth).abs() < 0.2,
            "skew {truth} read as {:.2}",
            registration.skew_deg
        );
    }
}

#[test]
fn a_blank_sheet_is_still_measured_from_its_edges() {
    let scan = Scan::build(A4, 150.0, 90, 1.5, 0);
    let registration = register(&scan.image, ScanOptions::new(A4)).unwrap();
    // A blank sheet has no text, but it still has edges.
    assert!((registration.skew_deg - 1.5).abs() < 0.3, "{}", registration.skew_deg);
    assert!((registration.dpi() - 150.0).abs() < 6.0);
}

#[test]
fn greyscale_and_rgba_scans_work_the_same_as_rgb() {
    let scan = Scan::build(A4, 150.0, 90, 1.0, 20);
    let rgb = register(&scan.image, ScanOptions::new(A4)).unwrap();

    let grey = DynamicImage::ImageLuma8(scan.image.to_luma8());
    let rgba = DynamicImage::ImageRgba8(scan.image.to_rgba8());

    for variant in [grey, rgba] {
        let got = register(&variant, ScanOptions::new(A4)).unwrap();
        assert!((got.dpi() - rgb.dpi()).abs() < 1.0);
        assert!((got.skew_deg - rgb.skew_deg).abs() < 0.1);
    }
}

#[test]
fn a_tiny_image_is_refused_rather_than_guessed_at() {
    let img = DynamicImage::ImageRgb8(RgbImage::from_pixel(3, 3, image::Rgb([255, 255, 255])));
    assert!(register(&img, ScanOptions::new(A4)).is_err());
}

// --- the command line ------------------------------------------------------

#[test]
fn add_places_words_where_the_scan_was_pointed_at() {
    let dir = tempfile::tempdir().unwrap();
    let scan_path = dir.path().join("scan.png");
    let out = dir.path().join("delta.pdf");

    let scan = Scan::build(A4, 200.0, 110, 1.8, 24);
    scan.save(&scan_path);
    let pixel = scan.pixel_of((60.0, 112.0));

    let result = run(&[
        "add",
        scan_path.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--at",
        &format!("{:.0},{:.0}:J. Bezzina", pixel.0, pixel.1),
    ]);

    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    assert!(out.is_file());
    // Reported in millimetres on the paper, near where we aimed.
    assert!(
        result.stdout.contains("(60.") || result.stdout.contains("(59."),
        "stdout: {}",
        result.stdout
    );
}

#[test]
fn add_accepts_millimetres_measured_on_the_paper() {
    let dir = tempfile::tempdir().unwrap();
    let scan_path = dir.path().join("scan.png");
    let out = dir.path().join("delta.pdf");
    Scan::build(A4, 150.0, 60, 0.0, 20).save(&scan_path);

    let result = run(&[
        "add",
        scan_path.to_str().unwrap(),
        "-o",
        out.to_str().unwrap(),
        "--at-mm",
        "60,150:Approved",
    ]);

    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    assert!(result.stdout.contains("(60.0, 150.0) mm"), "{}", result.stdout);
}

#[test]
fn inspect_reports_the_scan_without_writing_anything() {
    let dir = tempfile::tempdir().unwrap();
    let scan_path = dir.path().join("scan.png");
    Scan::build(A4, 200.0, 110, 1.5, 24).save(&scan_path);

    let result = run(&["inspect", scan_path.to_str().unwrap()]);

    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    assert!(result.stdout.contains("200 dpi"), "{}", result.stdout);
    assert!(result.stdout.contains("A4"), "{}", result.stdout);
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[test]
fn a_proof_image_is_written_when_asked() {
    let dir = tempfile::tempdir().unwrap();
    let scan_path = dir.path().join("scan.png");
    let proof = dir.path().join("proof.png");
    Scan::build(A4, 150.0, 90, 1.0, 20).save(&scan_path);

    let result = run(&[
        "add",
        scan_path.to_str().unwrap(),
        "-o",
        dir.path().join("d.pdf").to_str().unwrap(),
        "--at-mm",
        "60,150:Approved",
        "--preview",
        proof.to_str().unwrap(),
    ]);

    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    assert!(proof.is_file());
    // The mark must actually be on it.
    let marked = image::open(&proof).unwrap().to_rgb8();
    let red = marked
        .pixels()
        .filter(|p| p.0[0] > 150 && p.0[1] < 120 && p.0[2] < 120)
        .count();
    assert!(red > 50, "expected proof marks, found {red} red pixels");
}

#[test]
fn multiple_additions_all_land() {
    let dir = tempfile::tempdir().unwrap();
    let scan_path = dir.path().join("scan.png");
    Scan::build(A4, 150.0, 60, 0.0, 20).save(&scan_path);

    let result = run(&[
        "add",
        scan_path.to_str().unwrap(),
        "-o",
        dir.path().join("d.pdf").to_str().unwrap(),
        "--at-mm",
        "30,60:first",
        "--at-mm",
        "90,200:second",
        "--at-mm",
        "40,250:third",
    ]);

    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
    assert!(result.stdout.contains("additions  : 3"), "{}", result.stdout);
}

#[test]
fn an_addition_near_the_edge_is_warned_about() {
    let dir = tempfile::tempdir().unwrap();
    let scan_path = dir.path().join("scan.png");
    Scan::build(A4, 150.0, 60, 0.0, 20).save(&scan_path);

    let result = run(&[
        "add",
        scan_path.to_str().unwrap(),
        "-o",
        dir.path().join("d.pdf").to_str().unwrap(),
        "--at-mm",
        "1,150:too close",
    ]);

    assert_eq!(result.code, 0);
    assert!(result.stdout.contains("WARNING"), "{}", result.stdout);
}

// --- bad input must explain itself, never panic ----------------------------

#[test]
fn bad_input_is_reported_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    let scan_path = dir.path().join("scan.png");
    let out = dir.path().join("d.pdf");
    Scan::build(A4, 150.0, 60, 0.0, 20).save(&scan_path);
    let scan = scan_path.to_str().unwrap();
    let output = out.to_str().unwrap();

    let cases: Vec<(Vec<&str>, &str)> = vec![
        (vec!["add", scan, "-o", output], "nothing to add"),
        (
            vec!["add", scan, "-o", output, "--at-mm", "nonsense"],
            "bad placement",
        ),
        (
            vec!["add", scan, "-o", output, "--at-mm", "60:missing y"],
            "bad position",
        ),
        (
            vec!["add", scan, "-o", output, "--at-mm", "a,b:words"],
            "not a number",
        ),
        (
            vec!["add", scan, "-o", output, "--at-mm", "60,150:"],
            "no words",
        ),
        (
            vec!["add", scan, "-o", output, "--at-mm", "60,150:hi", "--page", "nope"],
            "unknown page size",
        ),
        (
            vec!["add", scan, "-o", output, "--at-mm", "60,150:hi", "--font", "Comic Sans"],
            "unknown font",
        ),
        (
            vec!["add", scan, "-o", output, "--at-mm", "60,150:hi", "--size", "0"],
            "out of range",
        ),
        (
            vec!["add", scan, "-o", output, "--at-mm", "60,150:hi", "--size", "9000"],
            "out of range",
        ),
        (
            vec!["add", scan, "-o", output, "--at-mm", "60,150:季度报告"],
            "cannot write these characters",
        ),
        (
            vec!["add", "/nonexistent/scan.png", "-o", output, "--at-mm", "60,150:hi"],
            "no such file",
        ),
        (vec!["inspect", scan, "--page", "0x0"], "unknown page size"),
    ];

    for (args, expected) in cases {
        let result = run(&args);
        assert_eq!(result.code, 1, "expected failure for {args:?}");
        assert!(
            result.stderr.contains(expected),
            "for {args:?}\n  wanted: {expected}\n  got: {}",
            result.stderr
        );
        assert!(
            !result.stderr.contains("panicked"),
            "panic for {args:?}: {}",
            result.stderr
        );
    }
}

#[test]
fn a_file_that_is_not_an_image_is_reported_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    let fake = dir.path().join("notreally.png");
    std::fs::write(&fake, b"this is not a PNG").unwrap();

    let result = run(&[
        "add",
        fake.to_str().unwrap(),
        "-o",
        dir.path().join("d.pdf").to_str().unwrap(),
        "--at-mm",
        "60,150:hi",
    ]);

    assert_eq!(result.code, 1);
    assert!(result.stderr.contains("could not read"), "{}", result.stderr);
    assert!(!result.stderr.contains("panicked"));
}

#[test]
fn an_empty_file_is_reported_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    let empty = dir.path().join("empty.png");
    std::fs::write(&empty, b"").unwrap();

    let result = run(&["inspect", empty.to_str().unwrap()]);

    assert_eq!(result.code, 1);
    assert!(!result.stderr.contains("panicked"), "{}", result.stderr);
}

#[test]
fn western_european_text_is_accepted() {
    let dir = tempfile::tempdir().unwrap();
    let scan_path = dir.path().join("scan.png");
    Scan::build(A4, 150.0, 60, 0.0, 20).save(&scan_path);

    let result = run(&[
        "add",
        scan_path.to_str().unwrap(),
        "-o",
        dir.path().join("d.pdf").to_str().unwrap(),
        "--at-mm",
        "40,150:café — naïve €20 «déjà»",
    ]);

    assert_eq!(result.code, 0, "stderr: {}", result.stderr);
}

#[test]
fn fonts_are_listed() {
    let result = run(&["fonts"]);
    assert_eq!(result.code, 0);
    assert!(result.stdout.contains("Helvetica"));
    assert!(result.stdout.contains("Times-Roman"));
}
