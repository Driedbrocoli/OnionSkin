//! Per-printer registration calibration.
//!
//! Uncalibrated, a second pass through a sheet-fed printer lands within about
//! ±2 mm — fine for a signature, useless for filling a pre-printed box. The fix
//! needs no scanner:
//!
//! 1. `onionskin calibrate target` writes a page of crosshairs at known
//!    positions, each with a fine ruler running right and down from it.
//! 2. Print it on blank paper at 100%.
//! 3. Put that same sheet back in the tray and print the *same file again*.
//! 4. Every crosshair now has two impressions. Read the offset of the second
//!    from the first against the printed ruler — that offset *is* the error the
//!    printer will apply to your delta.
//! 5. `onionskin calibrate solve` fits shift, rotation and scale to those
//!    readings and stores the profile.
//!
//! Deltas then get the inverse of that transform, so the ink lands where the
//! document says it should.

use std::path::{Path, PathBuf};

use lopdf::{dictionary, Object, Stream};
use serde::{Deserialize, Serialize};

use crate::geometry::{mm_to_pt, solve_similarity, PageSize, Similarity, SimilarityFit};

pub const A4: PageSize = PageSize {
    width_mm: 210.0,
    height_mm: 297.0,
};

/// How far the measuring rulers extend from each crosshair.
pub const RULER_REACH_MM: f64 = 4.0;
pub const RULER_STEP_MM: f64 = 0.25;

/// How far each scale sits from the crosshair centre. Far enough that the two
/// scales cannot overlap each other, close enough that the second impression's
/// arms still reach across them.
const SCALE_OFFSET_MM: f64 = 7.0;

/// Crosshair arms must be longer than `SCALE_OFFSET_MM`, or the second
/// impression's arms would never reach the scale they are read against.
const ARM_MM: f64 = 12.0;

#[derive(Debug, thiserror::Error)]
pub enum CalibrateError {
    #[error("{0}")]
    Invalid(String),
    #[error("no calibration profile '{name}' (available: {available})")]
    NoProfile { name: String, available: String },
    #[error("could not read {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("profile {path} is unreadable: {source}")]
    Malformed {
        path: PathBuf,
        source: serde_json::Error,
    },
    #[error("could not write the target: {0}")]
    Pdf(#[from] lopdf::Error),
}

/// A stored measurement of one printer's second-pass registration error.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    /// The error the printer introduces. Deltas get its inverse.
    pub error: Similarity,
    #[serde(default = "a4")]
    pub page: PageSize,
    #[serde(default)]
    pub rms_residual_mm: Option<f64>,
    #[serde(default)]
    pub max_residual_mm: Option<f64>,
    #[serde(default)]
    pub n_points: usize,
    /// Seconds since the epoch.
    #[serde(default)]
    pub created: u64,
    #[serde(default)]
    pub notes: String,
}

fn a4() -> PageSize {
    A4
}

impl Profile {
    /// What to apply to a delta: the opposite of what the printer will do.
    pub fn correction(&self) -> Similarity {
        self.error.inverse()
    }

    pub fn describe(&self) -> String {
        let mut lines = vec![
            format!("profile '{}'", self.name),
            format!("  printer error : {}", self.error.describe()),
            format!("  correction    : {}", self.correction().describe()),
            format!("  page          : {}", self.page.describe()),
        ];
        if let (Some(rms), Some(max)) = (self.rms_residual_mm, self.max_residual_mm) {
            lines.push(format!(
                "  fit           : {} points, rms {rms:.3} mm, max {max:.3} mm",
                self.n_points
            ));
        }
        if !self.notes.is_empty() {
            lines.push(format!("  notes         : {}", self.notes));
        }
        lines.join("\n")
    }
}

/// Where profiles live.
pub fn home_dir() -> PathBuf {
    if let Ok(set) = std::env::var("ONIONSKIN_HOME") {
        return PathBuf::from(set);
    }
    home().join(".onionskin")
}

/// Point `ONIONSKIN_HOME` at a directory of a test's own, and hold everything
/// else off until it is done.
///
/// One variable for the whole process, and tests run beside one another — so
/// two of them changing it at once see each other's answers, which is a test
/// that fails once in three runs and passes when it is looked at. Everything
/// that redirects the home directory takes this first.
#[cfg(test)]
pub(crate) fn borrow_home(path: &Path) -> std::sync::MutexGuard<'static, ()> {
    static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());
    // A test that panicked while holding it left it poisoned; the next test
    // sets the variable itself anyway, so there is nothing to recover.
    let held = ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    std::env::set_var("ONIONSKIN_HOME", path);
    held
}

/// The user's home directory, without a crate to ask.
fn home() -> PathBuf {
    for key in ["HOME", "USERPROFILE"] {
        if let Ok(value) = std::env::var(key) {
            if !value.is_empty() {
                return PathBuf::from(value);
            }
        }
    }
    // Windows splits it in two.
    if let (Ok(drive), Ok(path)) = (std::env::var("HOMEDRIVE"), std::env::var("HOMEPATH")) {
        return PathBuf::from(format!("{drive}{path}"));
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

pub fn profiles_dir() -> Result<PathBuf, CalibrateError> {
    let path = home_dir().join("profiles");
    std::fs::create_dir_all(&path).map_err(|source| CalibrateError::Io {
        path: path.clone(),
        source,
    })?;
    // A profile says which printer someone uses and how it is set up. Not a
    // secret, but not other accounts' business either.
    crate::render::restrict(&path);
    if let Some(parent) = path.parent() {
        crate::render::restrict(parent);
    }
    Ok(path)
}

/// A file name that cannot escape the profiles directory.
pub fn profile_path(name: &str) -> Result<PathBuf, CalibrateError> {
    let safe: String = name
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect();
    let safe = if safe.is_empty() { "default" } else { &safe };
    Ok(profiles_dir()?.join(format!("{safe}.json")))
}

pub fn save_profile(profile: &Profile) -> Result<PathBuf, CalibrateError> {
    let path = profile_path(&profile.name)?;
    let text = serde_json::to_string_pretty(profile)
        .map_err(|e| CalibrateError::Invalid(e.to_string()))?;
    std::fs::write(&path, text).map_err(|source| CalibrateError::Io {
        path: path.clone(),
        source,
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(path)
}

pub fn load_profile(name: &str) -> Result<Profile, CalibrateError> {
    let path = profile_path(name)?;
    if !path.is_file() {
        let names: Vec<String> = list_profiles()?.into_iter().map(|p| p.name).collect();
        return Err(CalibrateError::NoProfile {
            name: name.to_string(),
            available: if names.is_empty() {
                "none".into()
            } else {
                names.join(", ")
            },
        });
    }
    let text = std::fs::read_to_string(&path).map_err(|source| CalibrateError::Io {
        path: path.clone(),
        source,
    })?;
    serde_json::from_str(&text).map_err(|source| CalibrateError::Malformed { path, source })
}

pub fn list_profiles() -> Result<Vec<Profile>, CalibrateError> {
    let dir = profiles_dir()?;
    let mut found: Vec<Profile> = Vec::new();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Ok(found);
    };
    let mut paths: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map(|s| s == "json").unwrap_or(false))
        .collect();
    paths.sort();

    for path in paths {
        // A profile someone has hand-edited into nonsense should not stop the
        // others from being listed.
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Ok(profile) = serde_json::from_str::<Profile>(&text) {
                found.push(profile);
            }
        }
    }
    Ok(found)
}

pub fn delete_profile(name: &str) -> Result<bool, CalibrateError> {
    let path = profile_path(name)?;
    if path.is_file() {
        std::fs::remove_file(&path).map_err(|source| CalibrateError::Io {
            path: path.clone(),
            source,
        })?;
        return Ok(true);
    }
    Ok(false)
}

/// Crosshair positions: four corners plus centre.
///
/// Spread matters. Rotation and scale are only observable from points far
/// apart, so clustering them would leave those terms unconstrained.
pub fn fiducials(page: PageSize, inset_mm: f64) -> Vec<(f64, f64)> {
    let (w, h) = (page.width_mm, page.height_mm);
    vec![
        (inset_mm, inset_mm),
        (w - inset_mm, inset_mm),
        (inset_mm, h - inset_mm),
        (w - inset_mm, h - inset_mm),
        (w / 2.0, h / 2.0),
    ]
}

/// How far in to place the corner crosshairs on a given sheet.
///
/// 25 mm suits office paper, but a small sheet needs the fiducials pulled in
/// proportionally or their scales would hang off the edge. Spread still has to
/// be as wide as the sheet allows, since rotation and scale are only observable
/// from points far apart.
pub fn default_inset(page: PageSize) -> f64 {
    let shortest = page.width_mm.min(page.height_mm);
    (shortest / 4.0).clamp(15.0, 25.0)
}

/// Fit the printer's error from per-fiducial `(index, dx_mm, dy_mm)` readings.
///
/// The inset must match the target that was printed, or the fitted rotation and
/// scale will be wrong — hence the shared default.
pub fn solve_from_offsets(
    offsets: &[(usize, f64, f64)],
    page: PageSize,
    inset_mm: Option<f64>,
) -> Result<SimilarityFit, CalibrateError> {
    let inset = inset_mm.unwrap_or_else(|| default_inset(page));
    let points = fiducials(page, inset);
    let mut nominal = Vec::new();
    let mut observed = Vec::new();

    for &(index, dx, dy) in offsets {
        if index < 1 || index > points.len() {
            return Err(CalibrateError::Invalid(format!(
                "P{index} is not on the target (it has {} points)",
                points.len()
            )));
        }
        if !(dx.is_finite() && dy.is_finite()) {
            return Err(CalibrateError::Invalid(format!(
                "the offsets for P{index} are not real numbers"
            )));
        }
        let (px, py) = points[index - 1];
        nominal.push((px, py));
        observed.push((px + dx, py + dy));
    }
    solve_similarity(&nominal, &observed, &page).map_err(|e| CalibrateError::Invalid(e.to_string()))
}

/// Parse `P1:+0.40,-0.15` into `(1, 0.40, -0.15)`.
pub fn parse_point(spec: &str) -> Result<(usize, f64, f64), CalibrateError> {
    let raw = spec.trim();
    let Some((label, values)) = raw.split_once(':') else {
        return Err(CalibrateError::Invalid(format!(
            "bad point '{spec}'. Expected 'P1:dx,dy', e.g. 'P1:+0.40,-0.15'"
        )));
    };
    let label = label.trim().trim_start_matches(['P', 'p']);
    let index: usize = label
        .parse()
        .map_err(|_| CalibrateError::Invalid(format!("bad point label in '{spec}'")))?;

    let cleaned: String = values.chars().filter(|c| !c.is_whitespace()).collect();
    let parts: Vec<&str> = cleaned.split(',').collect();
    if parts.len() != 2 {
        return Err(CalibrateError::Invalid(format!(
            "bad offsets in '{spec}'. Expected 'dx,dy' in mm"
        )));
    }
    let number = |text: &str| -> Result<f64, CalibrateError> {
        text.parse::<f64>()
            .ok()
            .filter(|v| v.is_finite())
            .ok_or_else(|| CalibrateError::Invalid(format!("offsets in '{spec}' are not numbers")))
    };
    Ok((index, number(parts[0])?, number(parts[1])?))
}

// ---------------------------------------------------------------------------
// Drawing the target
// ---------------------------------------------------------------------------

/// A page of lines and text, built up as PDF operators.
///
/// Small on purpose. The target is the only thing in Onionskin that draws
/// anything but text, and it needs exactly lines, a circle and some labels.
struct Ink {
    ops: String,
    page: PageSize,
}

impl Ink {
    fn new(page: PageSize) -> Ink {
        Ink {
            ops: String::from("0 G 0 g\n"),
            page,
        }
    }

    /// Page millimetres to PDF points, flipping y to point up.
    fn at(&self, x_mm: f64, y_mm: f64) -> (f64, f64) {
        (mm_to_pt(x_mm), self.page.height_pt() - mm_to_pt(y_mm))
    }

    fn width(&mut self, pt: f64) {
        self.ops.push_str(&format!("{pt:.3} w\n"));
    }

    fn line(&mut self, from: (f64, f64), to: (f64, f64)) {
        let a = self.at(from.0, from.1);
        let b = self.at(to.0, to.1);
        self.ops.push_str(&format!(
            "{:.3} {:.3} m {:.3} {:.3} l S\n",
            a.0, a.1, b.0, b.1
        ));
    }

    /// A circle, as the four Bézier arcs every drawing program uses.
    fn circle(&mut self, centre: (f64, f64), radius_mm: f64) {
        let (cx, cy) = self.at(centre.0, centre.1);
        let r = mm_to_pt(radius_mm);
        // The magic constant that makes a cubic Bézier hug a quarter circle.
        let k = r * 0.552_284_749_8;
        self.ops.push_str(&format!(
            "{:.3} {:.3} m {:.3} {:.3} {:.3} {:.3} {:.3} {:.3} c \
             {:.3} {:.3} {:.3} {:.3} {:.3} {:.3} c \
             {:.3} {:.3} {:.3} {:.3} {:.3} {:.3} c \
             {:.3} {:.3} {:.3} {:.3} {:.3} {:.3} c S\n",
            cx + r,
            cy,
            cx + r,
            cy + k,
            cx + k,
            cy + r,
            cx,
            cy + r,
            cx - k,
            cy + r,
            cx - r,
            cy + k,
            cx - r,
            cy,
            cx - r,
            cy - k,
            cx - k,
            cy - r,
            cx,
            cy - r,
            cx + k,
            cy - r,
            cx + r,
            cy - k,
            cx + r,
            cy,
        ));
    }

    /// Text, positioned by where its left edge sits.
    fn text(&mut self, at: (f64, f64), size_pt: f64, font: &str, body: &str) {
        let (x, y) = self.at(at.0, at.1);
        self.ops.push_str(&format!(
            "BT /{font} {size_pt:.2} Tf {x:.3} {y:.3} Td ({}) Tj ET\n",
            escape(body)
        ));
    }

    /// Text centred on a point. Measured properly rather than guessed, so a
    /// label sits over its crosshair rather than beside it.
    fn text_centred(&mut self, at: (f64, f64), size_pt: f64, font: &str, body: &str) {
        let width = crate::pdf::builtin_width_mm(face(font), body, size_pt);
        self.text((at.0 - width / 2.0, at.1), size_pt, font, body);
    }

    /// Text whose right edge sits at the point.
    fn text_right(&mut self, at: (f64, f64), size_pt: f64, font: &str, body: &str) {
        let width = crate::pdf::builtin_width_mm(face(font), body, size_pt);
        self.text((at.0 - width, at.1), size_pt, font, body);
    }
}

fn face(name: &str) -> crate::pdf::Font {
    crate::pdf::Font::parse(name).unwrap_or(crate::pdf::Font::Helvetica)
}

fn escape(text: &str) -> String {
    text.replace('\\', "\\\\")
        .replace('(', "\\(")
        .replace(')', "\\)")
}

/// How long a tick is, by what it marks.
fn tick_length(offset: f64) -> f64 {
    if (offset - offset.round()).abs() < 1e-9 {
        1.7 // a whole millimetre
    } else if (offset * 2.0 - (offset * 2.0).round()).abs() < 1e-9 {
        1.1 // a half
    } else {
        0.6
    }
}

/// Draw the two measuring scales for one crosshair.
///
/// The x scale sits *below* the fiducial and the y scale to its *left*, in
/// strips that cannot overlap. You read an offset where the second impression's
/// arm crosses a scale: its vertical arm cuts the x scale, its horizontal arm
/// cuts the y scale.
fn draw_scales(ink: &mut Ink, x_mm: f64, y_mm: f64) {
    let steps = (RULER_REACH_MM / RULER_STEP_MM) as i64;

    // The x scale: a baseline below the fiducial, ticks hanging down from it.
    ink.width(0.25);
    ink.line(
        (x_mm - RULER_REACH_MM, y_mm + SCALE_OFFSET_MM),
        (x_mm + RULER_REACH_MM, y_mm + SCALE_OFFSET_MM),
    );
    ink.width(0.2);
    for i in -steps..=steps {
        let offset = i as f64 * RULER_STEP_MM;
        let at = (x_mm + offset, y_mm + SCALE_OFFSET_MM);
        ink.line(at, (at.0, at.1 + tick_length(offset)));
    }
    for mark in [-4i32, -2, 2, 4] {
        ink.text_centred(
            (x_mm + mark as f64, y_mm + SCALE_OFFSET_MM + 3.6 + 1.0),
            3.6,
            "Helvetica",
            &format!("{mark:+}"),
        );
    }
    ink.text_centred(
        (x_mm, y_mm + SCALE_OFFSET_MM + 3.6 + 1.0),
        3.6,
        "Helvetica",
        "x",
    );

    // The y scale: a baseline left of the fiducial, ticks running left from it.
    ink.width(0.25);
    ink.line(
        (x_mm - SCALE_OFFSET_MM, y_mm - RULER_REACH_MM),
        (x_mm - SCALE_OFFSET_MM, y_mm + RULER_REACH_MM),
    );
    ink.width(0.2);
    for i in -steps..=steps {
        let offset = i as f64 * RULER_STEP_MM;
        let at = (x_mm - SCALE_OFFSET_MM, y_mm + offset);
        ink.line(at, (at.0 - tick_length(offset), at.1));
    }
    for mark in [-4i32, -2, 2, 4] {
        ink.text_right(
            (x_mm - SCALE_OFFSET_MM - 2.4, y_mm + mark as f64 + 0.5),
            3.6,
            "Helvetica",
            &format!("{mark:+}"),
        );
    }
    ink.text_right(
        (x_mm - SCALE_OFFSET_MM - 2.4, y_mm + 0.5),
        3.6,
        "Helvetica",
        "y",
    );
}

/// Write the two-pass calibration target.
pub fn make_target(
    out_path: &Path,
    page: PageSize,
    inset_mm: Option<f64>,
) -> Result<PathBuf, CalibrateError> {
    let inset = inset_mm.unwrap_or_else(|| default_inset(page));
    if let Some(parent) = out_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|source| CalibrateError::Io {
                path: parent.to_path_buf(),
                source,
            })?;
        }
    }

    let mut ink = Ink::new(page);
    for (index, (x_mm, y_mm)) in fiducials(page, inset).into_iter().enumerate() {
        // A crosshair with a gap at its middle, so the two impressions can be
        // told apart where they nearly coincide.
        ink.width(0.35);
        ink.line((x_mm - ARM_MM, y_mm), (x_mm - 0.8, y_mm));
        ink.line((x_mm + 0.8, y_mm), (x_mm + ARM_MM, y_mm));
        ink.line((x_mm, y_mm - ARM_MM), (x_mm, y_mm - 0.8));
        ink.line((x_mm, y_mm + 0.8), (x_mm, y_mm + ARM_MM));
        ink.circle((x_mm, y_mm), 1.6);

        draw_scales(&mut ink, x_mm, y_mm);

        // Above the crosshair: the x scale is below it and the y scale left, so
        // this is the one side left free.
        ink.text_centred(
            (x_mm, y_mm - ARM_MM - 2.5),
            6.0,
            "Helvetica-Bold",
            &format!("P{}   {}, {}", index + 1, trim(x_mm), trim(y_mm)),
        );
    }

    // Instructions, kept clear of the fiducials and their rulers. A small sheet
    // has no room for them, and a target you can read beats a target with
    // printed prose over the crosshairs.
    let text_y = page.height_mm / 2.0 - 22.0;
    let centre_x = page.width_mm / 2.0;

    if page.width_mm < 170.0 || page.height_mm < 230.0 {
        ink.text_centred(
            (centre_x, text_y + 2.0),
            5.5,
            "Helvetica",
            "Onionskin target - print at 100%, re-feed the sheet, print again.",
        );
    } else {
        ink.text_centred(
            (centre_x, text_y),
            9.0,
            "Helvetica-Bold",
            "Onionskin - printer calibration target",
        );
        let body = [
            "1.  Print this page on blank paper at 100% / Actual size. Turn OFF 'Fit to page'.",
            "2.  Put that same sheet back in the tray, same way up, and print this file AGAIN.",
            "3.  Each crosshair now has two impressions. For each one, read where the second",
            "     impression's arms cross the scales: its vertical arm on the x scale below,",
            "     its horizontal arm on the y scale to the left. Right and down are positive.",
            "4.  onionskin calibrate solve --point 'P1:+0.4,-0.2' --point 'P2:...' ...",
        ];
        for (i, line) in body.iter().enumerate() {
            ink.text_centred(
                (centre_x, text_y + 4.0 + i as f64 * 3.4),
                7.0,
                "Helvetica",
                line,
            );
        }
        ink.text_centred(
            (centre_x, text_y + 4.0 + body.len() as f64 * 3.4 + 2.5),
            6.0,
            "Helvetica-Oblique",
            &format!(
                "{} - ruler ticks are {RULER_STEP_MM} mm",
                page.describe().replace('×', "x")
            ),
        );
    }

    write_page(out_path, page, &ink.ops)?;
    Ok(out_path.to_path_buf())
}

/// Put one page of operators into a PDF.
fn write_page(path: &Path, page: PageSize, ops: &str) -> Result<(), CalibrateError> {
    let mut doc = lopdf::Document::with_version("1.4");
    let pages_id = doc.new_object_id();

    let mut fonts = lopdf::Dictionary::new();
    for (key, base) in [
        ("Helvetica", "Helvetica"),
        ("Helvetica-Bold", "Helvetica-Bold"),
        ("Helvetica-Oblique", "Helvetica-Oblique"),
    ] {
        let id = doc.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => Object::Name(base.as_bytes().to_vec()),
            "Encoding" => "WinAnsiEncoding",
        });
        fonts.set(key, Object::Reference(id));
    }

    let mut content = Stream::new(dictionary! {}, ops.as_bytes().to_vec());
    let _ = content.compress();
    let content_id = doc.add_object(content);

    let page_id = doc.add_object(dictionary! {
        "Type" => "Page",
        "Parent" => pages_id,
        "MediaBox" => Object::Array(vec![
            Object::Real(0.0),
            Object::Real(0.0),
            Object::Real(page.width_pt() as f32),
            Object::Real(page.height_pt() as f32),
        ]),
        "Resources" => dictionary! { "Font" => Object::Dictionary(fonts) },
        "Contents" => content_id,
    });

    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Count" => 1_i64,
            "Kids" => Object::Array(vec![Object::Reference(page_id)]),
        }),
    );
    let catalog_id = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    let info_id = doc.add_object(dictionary! {
        "Title" => Object::string_literal("Onionskin calibration target"),
        "Producer" => Object::string_literal("Onionskin"),
    });
    doc.trailer.set("Root", catalog_id);
    doc.trailer.set("Info", info_id);
    doc.compress();
    doc.save(path).map_err(|source| CalibrateError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(())
}

/// `25` rather than `25.0`, the way the label reads on paper.
fn trim(value: f64) -> String {
    if (value - value.round()).abs() < 1e-9 {
        format!("{}", value.round() as i64)
    } else {
        format!("{value:.1}")
    }
}

/// Seconds since the epoch, for stamping a profile.
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests;
