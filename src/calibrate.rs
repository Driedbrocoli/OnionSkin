//! Per-printer registration calibration.
//!
//! Uncalibrated, a second pass through a sheet-fed printer lands within about
//! ±2 mm — fine for a signature, useless for filling a pre-printed box. The fix
//! is one sheet printed twice:
//!
//! 1. `onionskin calibrate target` writes a target of two pages. Page 1 is a
//!    set of crosshairs at known positions, each with a fine ruler running
//!    right and down from it. Page 2 puts a *different* mark — an open diamond
//!    — at those same positions.
//! 2. Print page 1 on blank paper at 100%.
//! 3. Put that same sheet back in the tray and print page 2 onto it.
//! 4. Every crosshair now has a diamond beside it. How far that diamond sits
//!    from where it was asked to go *is* the error the printer will apply to
//!    your delta.
//! 5. Either scan the sheet and let [`measure_from_scan`] read all five offsets
//!    off it, or read them against the printed rulers by eye. Either way,
//!    [`solve_from_offsets`] fits shift, rotation and scale to those readings
//!    and the profile is stored.
//!
//! Deltas then get the inverse of that transform, so the ink lands where the
//! document says it should.
//!
//! # Why the two passes no longer print the same thing
//!
//! They used to, and for a person that is fine: someone reading a ruler knows
//! which impression they printed second, because they watched it happen. A scan
//! does not. Two identical crosshairs a third of a millimetre apart are just
//! two crosshairs, and nothing in the picture says which one came second — so
//! an automatic reading would recover the offset's size but toss a coin for its
//! sign, and a correction applied backwards is twice the error of no correction
//! at all. Giving the second pass a mark of its own settles it in the image
//! itself, which is the only place a scanned sheet can be asked.

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

/// The ring at the middle of each crosshair. This is the mark a scan measures
/// the first pass by, so its size is a compromise: big enough that a few
/// hundred pixels land on it at any sane scanning resolution, small enough that
/// the second pass's diamond can sit clear of it.
const RING_MM: f64 = 1.6;

/// How far out from the crosshair's centre its arms begin.
///
/// It used to be 0.8 mm, which was only ever a gap for the eye. It is now wide
/// enough that the ring stands completely free of the arms, because that is
/// what lets a scan measure the first pass at all: ink that touches the arms is
/// ink joined to the rulers, the rulers being crossed by the arms by design,
/// and the middle of *that* shape is nowhere near the middle of the crosshair.
/// A free ring is a small closed curve, symmetric about the point it marks, so
/// the average of its pixels is that point and nothing else has to be true.
const ARM_GAP_MM: f64 = 3.4;

/// Half the diagonal of the second pass's diamond.
const DIAMOND_MM: f64 = 2.6;

/// How far up and to the right of its crosshair the diamond is drawn.
///
/// Not on top of it, which is the obvious thing to do and cannot be made to
/// work. Any closed shape drawn around the crosshair's centre has to cross the
/// four arms to get there, and ink that touches is ink that cannot be told
/// apart. Nor can it hide between the ring and the arms: to survive the ±2 mm
/// the printer is allowed to be out it would need a couple of millimetres of
/// clear paper on both sides, and there is nowhere near that much room.
///
/// The diagonal is the way out. The crosshair's arms lie on the axes and its
/// rulers sit below and to the left, so the quadrant up and to the right is
/// empty, and a mark placed squarely in it is 5 mm clear of the nearest ink in
/// every direction — which is the widest berth anything within reach of the
/// rulers can be given.
const DIAMOND_OFFSET_MM: f64 = 5.0;

/// Where the second pass's reading arms start.
///
/// Further out than the first pass's, so that a sheet whose second pass landed
/// a couple of millimetres high still leaves the first pass's ring standing on
/// its own.
const SECOND_ARM_GAP_MM: f64 = 5.0;

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

    /// A closed outline through the given corners.
    ///
    /// One path rather than a line per side, because four separately stroked
    /// lines meet at square ends and leave a notch at every corner. On paper
    /// that is invisible; in a scan of that paper it is four strokes instead of
    /// one shape, and the measurement that has to find one shape finds nothing.
    fn closed(&mut self, corners: &[(f64, f64)]) {
        let Some((first, rest)) = corners.split_first() else {
            return;
        };
        let start = self.at(first.0, first.1);
        self.ops
            .push_str(&format!("{:.3} {:.3} m ", start.0, start.1));
        for corner in rest {
            let point = self.at(corner.0, corner.1);
            self.ops
                .push_str(&format!("{:.3} {:.3} l ", point.0, point.1));
        }
        self.ops.push_str("h S\n");
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

/// Draw the first pass's mark: a ring with four arms radiating from it.
///
/// The ring marks the point and the arms reach across the rulers, which is how
/// a reading is taken by eye. They are drawn apart rather than joined so that a
/// scan has one small closed shape to measure the pass by — see [`ARM_GAP_MM`].
fn draw_crosshair(ink: &mut Ink, x_mm: f64, y_mm: f64) {
    ink.width(0.35);
    ink.line((x_mm - ARM_MM, y_mm), (x_mm - ARM_GAP_MM, y_mm));
    ink.line((x_mm + ARM_GAP_MM, y_mm), (x_mm + ARM_MM, y_mm));
    ink.line((x_mm, y_mm - ARM_MM), (x_mm, y_mm - ARM_GAP_MM));
    ink.line((x_mm, y_mm + ARM_GAP_MM), (x_mm, y_mm + ARM_MM));
    ink.circle((x_mm, y_mm), RING_MM);
}

/// Draw the second pass's mark: an open diamond, plus the two arms a person
/// reads the rulers with.
///
/// The diamond is a square stood on its corner, and it is the shape it is for
/// three reasons. It is closed and symmetric about both axes, so the average of
/// its pixels is its centre however heavily it printed and however the scanner
/// spread the ink — and that centre is the entire measurement. It is nothing
/// like the ring on page 1: ink at a constant distance from the middle says
/// ring, ink whose distance swings by a factor of the square root of two
/// between the flats and the corners says diamond, and no amount of blur turns
/// one into the other. And it has no straight edge lying along either axis, so
/// it can never be mistaken for a fragment of an arm or a ruler.
///
/// There is deliberately no tick attached to it. Anything joined to the diamond
/// drags the average of its pixels away from its middle by a fixed amount — and
/// a fixed displacement of every mark on the sheet is exactly what a printer
/// that shifts the page does. The two are indistinguishable, so the one thing
/// this mark must not have is decoration.
fn draw_diamond_mark(ink: &mut Ink, x_mm: f64, y_mm: f64) {
    ink.width(0.35);
    // The arms, for the eye: down across the x scale, left across the y scale.
    // Only those two, both because they are the only two a scale is read
    // against and because a second full crosshair is the one thing this mark
    // must not look like.
    ink.line((x_mm, y_mm + SECOND_ARM_GAP_MM), (x_mm, y_mm + ARM_MM));
    ink.line((x_mm - ARM_MM, y_mm), (x_mm - SECOND_ARM_GAP_MM, y_mm));

    // The diamond, for the scanner.
    let (cx, cy) = (x_mm + DIAMOND_OFFSET_MM, y_mm - DIAMOND_OFFSET_MM);
    ink.closed(&[
        (cx, cy - DIAMOND_MM),
        (cx + DIAMOND_MM, cy),
        (cx, cy + DIAMOND_MM),
        (cx - DIAMOND_MM, cy),
    ]);
}

/// The first pass: crosshairs, their rulers and labels, and the instructions.
fn first_pass(page: PageSize, inset: f64) -> String {
    let mut ink = Ink::new(page);
    for (index, (x_mm, y_mm)) in fiducials(page, inset).into_iter().enumerate() {
        draw_crosshair(&mut ink, x_mm, y_mm);
        draw_scales(&mut ink, x_mm, y_mm);

        // Above the crosshair: the x scale is below it and the y scale left,
        // and the second pass's diamond takes the corner between them, so this
        // is the one side left free.
        ink.text_centred(
            (x_mm, y_mm - ARM_MM - 2.5),
            6.0,
            "Helvetica-Bold",
            &format!("P{}   {}, {}", index + 1, trim(x_mm), trim(y_mm)),
        );
    }

    let centre_x = page.width_mm / 2.0;
    if page.width_mm < 170.0 || page.height_mm < 230.0 {
        // A small sheet has no room for the prose, and a target you can read
        // beats a target with printed prose over the crosshairs.
        ink.text_centred(
            (centre_x, short_prose_y(page)),
            5.5,
            "Helvetica",
            "Onionskin target - print page 1 at 100%, re-feed the sheet, print page 2.",
        );
        return ink.ops;
    }

    let text_y = prose_y(page);
    ink.text_centred(
        (centre_x, text_y),
        9.0,
        "Helvetica-Bold",
        "Onionskin - printer calibration target",
    );
    let body = [
        "1.  Print PAGE 1 on blank paper at 100% / Actual size. Turn OFF 'Fit to page'.",
        "2.  Put that same sheet back in the tray, same way up, and print PAGE 2 onto it.",
        "3.  Scan the finished sheet at 300 dpi or better and let Onionskin measure it.",
        "     It reads the offsets far more finely than an eye can read a printed ruler.",
        "4.  Or read them yourself: each diamond's arms cross the scales below and to the",
        "     left of its crosshair. Right and down are positive. Then:",
        "5.  onionskin calibrate solve --point 'P1:+0.4,-0.2' --point 'P2:...' ...",
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
    ink.ops
}

/// The second pass: a diamond and two reading arms at every fiducial.
///
/// No labels and no rulers. This page is printed onto a sheet that already
/// carries page 1, so anything it repeats lands on top of what is there and
/// leaves both unreadable. Its one line of prose goes below the middle
/// crosshair, in a band page 1 leaves empty.
fn second_pass(page: PageSize, inset: f64) -> String {
    let mut ink = Ink::new(page);
    for (x_mm, y_mm) in fiducials(page, inset) {
        draw_diamond_mark(&mut ink, x_mm, y_mm);
    }

    let centre_x = page.width_mm / 2.0;
    let small = page.width_mm < 170.0 || page.height_mm < 230.0;
    let (y, size) = if small {
        (page.height_mm / 2.0 + 20.0, 5.5)
    } else {
        (page.height_mm / 2.0 + 45.0, 7.0)
    };
    ink.text_centred(
        (centre_x, y),
        size,
        "Helvetica",
        "PAGE 2 - the second pass. Print this onto the sheet that already carries page 1.",
    );
    ink.ops
}

/// Where the block of instructions goes on a sheet with room for it.
///
/// Well above the middle. It used to sit 22 mm above the centre of the sheet,
/// which put six lines of prose straight across the middle crosshair and its
/// label — P5 was unreadable, by eye or by scan, on every target Onionskin has
/// ever written. The block is about 31 mm deep and a fiducial reaches 17 mm
/// above its centre, so it starts far enough up to leave a clear band of paper
/// between the last line and the label below it.
fn prose_y(page: PageSize) -> f64 {
    page.height_mm / 2.0 - 53.0
}

/// Where the one line a small sheet gets goes.
fn short_prose_y(page: PageSize) -> f64 {
    page.height_mm / 2.0 - 20.0
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

    let pages = [first_pass(page, inset), second_pass(page, inset)];
    write_pages(out_path, page, &pages)?;
    Ok(out_path.to_path_buf())
}

/// Put the pages of operators into one PDF.
///
/// One file rather than two, because the two passes must agree about the paper
/// size and the crosshair positions to the tenth of a millimetre, and two files
/// are two things to lose track of. The printer's own page range picks which
/// pass to print.
fn write_pages(path: &Path, page: PageSize, pages: &[String]) -> Result<(), CalibrateError> {
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

    let mut page_ids: Vec<Object> = Vec::new();
    for ops in pages {
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
            "Resources" => dictionary! { "Font" => Object::Dictionary(fonts.clone()) },
            "Contents" => content_id,
        });
        page_ids.push(Object::Reference(page_id));
    }

    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Count" => page_ids.len() as i64,
            "Kids" => Object::Array(page_ids),
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

// ---------------------------------------------------------------------------
// Reading the sheet back off a scan
// ---------------------------------------------------------------------------

/// One crosshair, measured off a scan.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Reading {
    /// Which crosshair this is, numbered from one exactly as the sheet labels
    /// them, so it can go to [`solve_from_offsets`] unchanged.
    pub index: usize,
    /// How far right of the first pass the second pass landed, in millimetres.
    pub dx_mm: f64,
    /// How far below the first pass the second pass landed, in millimetres.
    pub dy_mm: f64,
    /// How far this reading is worth believing, from 0 to 1.
    ///
    /// It is not an error bar. It is a tally of everything that could have made
    /// this reading the wrong one — ink in the window that is neither mark, two
    /// shapes that were hard to tell apart, an offset larger than a printer
    /// plausibly makes — and it exists so that a reading which is probably fine
    /// and a reading which is probably nonsense do not look alike in a list.
    pub confidence: f64,
}

impl Reading {
    pub fn describe(&self) -> String {
        format!(
            "P{}: {:+.2}, {:+.2} mm ({:.0}% sure)",
            self.index,
            self.dx_mm,
            self.dy_mm,
            self.confidence * 100.0
        )
    }
}

/// How far the reading window stops short on the two sides the rulers are on.
///
/// The scales sit 7 mm below and to the left of every crosshair, their printed
/// numbers 11 mm out, and the `P1   25, 25` label 15 mm above. None of that is
/// part of the measurement, and all of it would arrive as shapes to argue
/// about — the numbers especially, since a small closed glyph at the right size
/// is exactly what a mark looks like. So the window is cut off before any of it
/// and the exclusion is deliberate, not incidental.
const WINDOW_NEAR_MM: f64 = 5.0;

/// How far the window reaches the other way, up and to the right.
///
/// Far enough for the diamond, which is drawn 5 mm out with a 2.6 mm reach of
/// its own, plus the two or three millimetres the printer is allowed to be out
/// before the reading is refused anyway.
const WINDOW_FAR_MM: f64 = 10.5;

/// The coarsest scan worth trying to measure, in pixels per millimetre — about
/// 140 dpi.
///
/// The target is drawn in hairlines: a third of a *point*, which is an eighth
/// of a millimetre, so even a good scan has barely a pixel and a half across a
/// mark. It survives that because the ink is weighed rather than counted, but
/// below about this the line stops being continuous at all — the threshold
/// breaks it into beads — and a broken ring is a ring whose middle is somewhere
/// else. Refusing is better than measuring the gaps.
const MIN_PX_PER_MM: f64 = 5.5;

/// The radius nine tenths of a mark's ink lies inside, for each mark.
///
/// The ring's ink is all at one radius, so that is simply its radius. The
/// diamond's runs from the middle of an edge out to the corners, and nine
/// tenths of the way along by length is about 0.95 of the half-diagonal.
const RING_SPAN_MM: f64 = RING_MM;
const DIAMOND_SPAN_MM: f64 = DIAMOND_MM * 0.95;

/// The ratio of the radius a tenth of a mark's ink lies inside to the radius
/// nine tenths of it does — the number that tells the two marks apart.
///
/// A ratio, so that it says the same thing at 200 dpi as at 600, and so that a
/// scan of a sheet the printer shrank by half a percent still reads as the
/// shape it is. A ring would score 1 were its line infinitely thin; a real one
/// measures between about 0.84 and 0.94, the fatter the ink the lower. A
/// diamond's ink reaches from 1/√2 of the half-diagonal at the middle of each
/// edge out to the full distance at the corners, and measures around 0.73
/// however heavily it printed. Those two numbers are what the shapes were
/// chosen to make different, and the gap between them is what the confidence is
/// scaled against.
const RING_ROUNDNESS: f64 = 0.90;
const DIAMOND_ROUNDNESS: f64 = 0.73;

/// How much better a shape must match one mark than the other before the
/// reading is credited in full.
const SHAPE_MARGIN: f64 = 0.35;

/// How well a patch of ink must match one of the two marks to count as one.
const MIN_SHAPE_SCORE: f64 = 0.20;

/// Ink smaller than this is dirt, not a mark, and not worth doubting a reading
/// over.
///
/// Well under what a mark lays down — a ring is 1.2 to 1.7 mm² of ink depending
/// on how thickly it printed and how coarsely it was scanned, a diamond half as
/// much again. The margin is deliberate and it points this way on purpose: dirt
/// counted as a mark costs a little confidence, while a mark dismissed as dirt
/// costs the whole fiducial.
const SPECK_MM2: f64 = 0.35;

/// An offset up to this is an ordinary printer being an ordinary printer, and
/// costs a reading nothing.
const PLAUSIBLE_OFFSET_MM: f64 = 1.5;

/// An offset beyond this is not a re-feed error. A sheet-fed printer three
/// millimetres out would be visibly chewing the paper, so a number that large
/// is far likelier to be two marks paired up wrongly — and a wrong pair fitted
/// with a straight face is the failure this whole module exists to avoid.
/// Refuse it and let the fit run on the fiducials that made sense.
///
/// The window has the same say in this from the other direction: a diamond
/// further out than about three millimetres runs into the edge of it, and
/// anything touching that edge is thrown away unread. The two limits are set to
/// agree so that a reading is never refused for one reason while looking as
/// though it were refused for the other.
const MAX_OFFSET_MM: f64 = 3.0;

/// Below this a reading is not a reading, and is dropped rather than reported.
const MIN_CONFIDENCE: f64 = 0.25;

/// The patch of sheet one crosshair is read inside, in page millimetres.
#[derive(Debug, Clone, Copy)]
struct Window {
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
}

impl Window {
    /// The window around one crosshair, lopsided towards the diamond.
    ///
    /// The crosshair's own arms run out of it on all four sides. That is
    /// expected and it is why every patch of ink that reaches the edge of the
    /// window is thrown away: an arm cut off by the window has its middle
    /// wherever the window happens to be, not wherever the printer put it.
    fn around(centre: (f64, f64)) -> Window {
        Window {
            x0: centre.0 - WINDOW_NEAR_MM,
            y0: centre.1 - WINDOW_FAR_MM,
            x1: centre.0 + WINDOW_FAR_MM,
            y1: centre.1 + WINDOW_NEAR_MM,
        }
    }

    fn holds(&self, mm: (f64, f64)) -> bool {
        mm.0 >= self.x0 && mm.0 <= self.x1 && mm.1 >= self.y0 && mm.1 <= self.y1
    }

    /// The block of scan pixels that could hold any of this window.
    ///
    /// The sheet may be turned in the scan, so the window is a tilted rectangle
    /// in the image; its four corners bound the pixels worth looking at, and
    /// [`Window::holds`] settles the rest one pixel at a time.
    fn pixel_bounds(
        &self,
        image: &image::GrayImage,
        mapping: &crate::scan::Mapping,
    ) -> Option<(u32, u32, u32, u32)> {
        let (width, height) = image.dimensions();
        if width == 0 || height == 0 {
            return None;
        }
        let corners = [
            mapping.page_mm_to_pixel((self.x0, self.y0)),
            mapping.page_mm_to_pixel((self.x1, self.y0)),
            mapping.page_mm_to_pixel((self.x0, self.y1)),
            mapping.page_mm_to_pixel((self.x1, self.y1)),
        ];
        let mut low = (f64::MAX, f64::MAX);
        let mut high = (f64::MIN, f64::MIN);
        for (x, y) in corners {
            if !(x.is_finite() && y.is_finite()) {
                return None;
            }
            low = (low.0.min(x), low.1.min(y));
            high = (high.0.max(x), high.1.max(y));
        }
        if high.0 < 0.0 || high.1 < 0.0 || low.0 > width as f64 || low.1 > height as f64 {
            return None;
        }
        let x0 = low.0.floor().max(0.0) as u32;
        let y0 = low.1.floor().max(0.0) as u32;
        let x1 = (high.0.ceil() as i64).min(width as i64 - 1).max(0) as u32;
        let y1 = (high.1.ceil() as i64).min(height as i64 - 1).max(0) as u32;
        if x1 <= x0 || y1 <= y0 {
            return None;
        }
        Some((x0, y0, x1, y1))
    }

    /// Every scan pixel that falls inside the window.
    fn walk(
        &self,
        image: &image::GrayImage,
        mapping: &crate::scan::Mapping,
        mut visit: impl FnMut((f64, f64), u8),
    ) {
        let Some((x0, y0, x1, y1)) = self.pixel_bounds(image, mapping) else {
            return;
        };
        for y in y0..=y1 {
            for x in x0..=x1 {
                let mm = mapping.pixel_to_page_mm((x as f64 + 0.5, y as f64 + 0.5));
                if self.holds(mm) {
                    visit(mm, image.get_pixel(x, y).0[0]);
                }
            }
        }
    }
}

/// One connected patch of ink inside a window.
struct Blob {
    /// Where the middle of it is, in page millimetres.
    centre_mm: (f64, f64),
    /// The radius a tenth of its ink lies inside, and the radius nine tenths
    /// does. Percentiles rather than the smallest and largest, because one
    /// stray pixel would otherwise decide the shape.
    near_mm: f64,
    far_mm: f64,
    area_mm2: f64,
    /// True when the patch runs into the edge of the window, so what is here is
    /// only part of it and its middle means nothing.
    clipped: bool,
}

impl Blob {
    /// `points` are the pixels that came out darker than the threshold;
    /// `centre_mm` is the middle of the ink, weighed rather than counted.
    fn new(points: &[(f64, f64)], centre_mm: (f64, f64), px_per_mm: f64, clipped: bool) -> Blob {
        let mut radii: Vec<f64> = points
            .iter()
            .map(|p| (p.0 - centre_mm.0).hypot(p.1 - centre_mm.1))
            .collect();
        radii.sort_by(f64::total_cmp);
        let at = |fraction: f64| {
            let last = radii.len() - 1;
            radii[((last as f64 * fraction).round() as usize).min(last)]
        };
        Blob {
            centre_mm,
            near_mm: at(0.10),
            far_mm: at(0.90),
            area_mm2: points.len() as f64 / (px_per_mm * px_per_mm),
            clipped,
        }
    }

    fn roundness(&self) -> f64 {
        if self.far_mm > 0.0 {
            self.near_mm / self.far_mm
        } else {
            0.0
        }
    }

    /// How much this patch looks like a mark of the given size and shape, from
    /// 0 to 1.
    ///
    /// Size and shape both count, and both are forgiving: a scanner spreads ink
    /// and a printer lays it on thicker or thinner than asked. What is not
    /// forgiving is the pair of them together, because the ways a ring can be
    /// mistaken for a diamond and the ways it can be mistaken for one of the
    /// right size hardly ever happen at once.
    fn resembles(&self, span_mm: f64, roundness: f64) -> f64 {
        if self.clipped || self.area_mm2 < SPECK_MM2 {
            return 0.0;
        }
        nearness(self.far_mm, span_mm, span_mm * 0.30) * nearness(self.roundness(), roundness, 0.10)
    }
}

/// How nearly a measurement matches what was expected of it: 1 when it is
/// exactly that, falling away smoothly as it drifts off by multiples of
/// `tolerance`.
fn nearness(value: f64, want: f64, tolerance: f64) -> f64 {
    if tolerance <= 0.0 {
        return 0.0;
    }
    let off = (value - want) / tolerance;
    (-0.5 * off * off).exp()
}

/// The two grey levels a window is read against: what counts as ink, and what
/// the bare paper came out at.
#[derive(Debug, Clone, Copy)]
struct Levels {
    ink: u8,
    paper: u8,
}

/// The grey level that separates ink from paper, taken from the windows alone.
///
/// Not from the whole scan, because a flatbed's dark backing around the sheet
/// is a third cluster of greys and drags the split away from the one that
/// matters. The windows are all well inside the paper, so what is in the tally
/// is ink and paper and nothing else.
fn ink_levels(
    image: &image::GrayImage,
    mapping: &crate::scan::Mapping,
    windows: &[Window],
) -> Levels {
    let mut histogram = [0u64; 256];
    for window in windows {
        window.walk(image, mapping, |_, level| {
            histogram[level as usize] += 1;
        });
    }
    let split = crate::scan::otsu_of_histogram(&histogram);

    // Nine tenths of what is in a window is bare paper, so the level nine
    // tenths of the pixels are lighter than is the paper itself.
    let total: u64 = histogram.iter().sum();
    let mut seen = 0u64;
    let mut paper = 255u8;
    for (level, count) in histogram.iter().enumerate() {
        seen += count;
        if seen * 10 >= total * 9 {
            paper = level as u8;
            break;
        }
    }

    // Otsu always finds a split, even in a picture of blank paper — where the
    // best one available is somewhere in the middle of the paper's own noise,
    // and half the sheet then reads as ink. Insisting that ink be markedly
    // darker than the paper turns that case into "no marks here", which is the
    // truth, rather than into a window full of enormous shapes.
    Levels {
        ink: split.min(paper.saturating_sub(45)),
        paper,
    }
}

/// Every connected patch of ink in one window.
///
/// Eight-way connected, not four: the diamond's edges run at 45°, and a
/// diagonal line of pixels touching only at their corners is one stroke to the
/// eye and four hundred separate specks to a four-way search.
///
/// The middle of each patch is weighed rather than counted: every pixel in and
/// around it counts for as much as it is darker than the paper. Counting whole
/// pixels instead sounds harmless and is not. A printed line is a couple of
/// pixels wide, no edge of it lands neatly on a pixel boundary, and where a
/// threshold falls decides whether the last row of pixels along an edge is in
/// or out — so one side of a diamond comes out a pixel fatter than the other,
/// and the middle of the ink moves half a pixel with it. Half a pixel at 300
/// dpi is 0.04 mm, which is most of the accuracy this whole exercise was for.
/// Weighing by how much ink each pixel holds puts the edge back where the ink
/// really stops, between the pixels.
fn ink_blobs(
    image: &image::GrayImage,
    mapping: &crate::scan::Mapping,
    px_per_mm: f64,
    levels: Levels,
    window: &Window,
) -> Vec<Blob> {
    let Some((x0, y0, x1, y1)) = window.pixel_bounds(image, mapping) else {
        return Vec::new();
    };
    let width = (x1 - x0 + 1) as usize;
    let height = (y1 - y0 + 1) as usize;

    let mut inside = vec![false; width * height];
    let mut ink = vec![false; width * height];
    for row in 0..height {
        for column in 0..width {
            let (px, py) = ((x0 + column as u32), (y0 + row as u32));
            let mm = mapping.pixel_to_page_mm((px as f64 + 0.5, py as f64 + 0.5));
            if !window.holds(mm) {
                continue;
            }
            let cell = row * width + column;
            inside[cell] = true;
            ink[cell] = image.get_pixel(px, py).0[0] < levels.ink;
        }
    }

    // Where a pixel is, in millimetres on the sheet.
    let place = |cell: usize| {
        mapping.pixel_to_page_mm((
            (x0 + (cell % width) as u32) as f64 + 0.5,
            (y0 + (cell / width) as u32) as f64 + 0.5,
        ))
    };

    let mut taken = vec![false; width * height];
    // Which patch each pixel has already been weighed for, so that the fringe
    // of one is not counted twice. Numbered from one, since zero means never.
    let mut weighed = vec![0u32; width * height];
    let mut blobs: Vec<Blob> = Vec::new();
    let mut stack: Vec<usize> = Vec::new();
    let mut cells: Vec<usize> = Vec::new();

    for seed in 0..width * height {
        if !ink[seed] || taken[seed] {
            continue;
        }
        taken[seed] = true;
        stack.clear();
        cells.clear();
        stack.push(seed);
        let mut clipped = false;

        while let Some(cell) = stack.pop() {
            cells.push(cell);
            let (row, column) = (cell / width, cell % width);
            for step_y in -1i64..=1 {
                for step_x in -1i64..=1 {
                    if step_x == 0 && step_y == 0 {
                        continue;
                    }
                    let (nx, ny) = (column as i64 + step_x, row as i64 + step_y);
                    if nx < 0 || ny < 0 || nx >= width as i64 || ny >= height as i64 {
                        clipped = true;
                        continue;
                    }
                    let next = ny as usize * width + nx as usize;
                    if !inside[next] {
                        // The window's own edge, which is what "clipped" means:
                        // there may be more of this shape outside, and there is
                        // no way to know from in here.
                        clipped = true;
                        continue;
                    }
                    if ink[next] && !taken[next] {
                        taken[next] = true;
                        stack.push(next);
                    }
                }
            }
        }

        // Weigh the patch and the ring of half-inked pixels around it, each
        // pixel counting for as much as it is darker than the paper.
        let patch = blobs.len() as u32 + 1;
        let (mut weight, mut sum_x, mut sum_y) = (0.0f64, 0.0f64, 0.0f64);
        for &cell in &cells {
            let (row, column) = (cell / width, cell % width);
            for step_y in -1i64..=1 {
                for step_x in -1i64..=1 {
                    let (nx, ny) = (column as i64 + step_x, row as i64 + step_y);
                    if nx < 0 || ny < 0 || nx >= width as i64 || ny >= height as i64 {
                        continue;
                    }
                    let next = ny as usize * width + nx as usize;
                    if !inside[next] || weighed[next] == patch {
                        continue;
                    }
                    weighed[next] = patch;
                    let level = image.get_pixel(x0 + nx as u32, y0 + ny as u32).0[0];
                    let darkness = (levels.paper as f64 - level as f64).max(0.0);
                    if darkness <= 0.0 {
                        continue;
                    }
                    let mm = place(next);
                    weight += darkness;
                    sum_x += darkness * mm.0;
                    sum_y += darkness * mm.1;
                }
            }
        }

        let points: Vec<(f64, f64)> = cells.iter().map(|cell| place(*cell)).collect();
        let centre = if weight > 0.0 {
            (sum_x / weight, sum_y / weight)
        } else {
            (
                points.iter().map(|p| p.0).sum::<f64>() / points.len() as f64,
                points.iter().map(|p| p.1).sum::<f64>() / points.len() as f64,
            )
        };
        blobs.push(Blob::new(&points, centre, px_per_mm, clipped));
    }
    blobs
}

/// Measure one crosshair. `None` when this fiducial cannot be read, which is
/// always better than a number nobody should have trusted.
fn read_fiducial(
    image: &image::GrayImage,
    mapping: &crate::scan::Mapping,
    px_per_mm: f64,
    levels: Levels,
    index: usize,
    centre_mm: (f64, f64),
) -> Option<Reading> {
    let window = Window::around(centre_mm);
    let blobs = ink_blobs(image, mapping, px_per_mm, levels, &window);

    let (mut ring, mut ring_score, mut ring_as_diamond) = (None, 0.0f64, 0.0f64);
    let (mut diamond, mut diamond_score, mut diamond_as_ring) = (None, 0.0f64, 0.0f64);
    let (mut rings, mut diamonds, mut strays) = (0usize, 0usize, 0usize);

    for blob in &blobs {
        let as_ring = blob.resembles(RING_SPAN_MM, RING_ROUNDNESS);
        let as_diamond = blob.resembles(DIAMOND_SPAN_MM, DIAMOND_ROUNDNESS);
        if as_ring.max(as_diamond) < MIN_SHAPE_SCORE {
            // Ink that is neither mark. A speck of dust or a paper fibre is not
            // worth a moment's doubt; anything the size of a mark is.
            if !blob.clipped && blob.area_mm2 >= SPECK_MM2 {
                strays += 1;
            }
            continue;
        }
        if as_ring >= as_diamond {
            rings += 1;
            if as_ring > ring_score {
                ring = Some(blob);
                ring_score = as_ring;
                ring_as_diamond = as_diamond;
            }
        } else {
            diamonds += 1;
            if as_diamond > diamond_score {
                diamond = Some(blob);
                diamond_score = as_diamond;
                diamond_as_ring = as_ring;
            }
        }
    }

    // One of each, or nothing. Two rings in one window means either the sheet
    // was printed twice from page 1 or something on the paper looks like a
    // mark, and in both cases picking whichever scored higher would be picking
    // at random — the reading has to be left out instead.
    let (ring, diamond) = (ring?, diamond?);
    if rings != 1 || diamonds != 1 {
        return None;
    }

    // The diamond is drawn up and to the right of its crosshair by a known
    // amount. Take that back off and what is left is what the printer did.
    let dx_mm = diamond.centre_mm.0 - DIAMOND_OFFSET_MM - ring.centre_mm.0;
    let dy_mm = diamond.centre_mm.1 + DIAMOND_OFFSET_MM - ring.centre_mm.1;
    let apart = dx_mm.hypot(dy_mm);
    if !apart.is_finite() || apart > MAX_OFFSET_MM {
        return None;
    }

    let quality = (ring_score.min(1.0) * diamond_score.min(1.0)).sqrt();
    let separation = ((ring_score - ring_as_diamond) / SHAPE_MARGIN)
        .clamp(0.0, 1.0)
        .min(((diamond_score - diamond_as_ring) / SHAPE_MARGIN).clamp(0.0, 1.0));
    let crowding = 0.8f64.powi(strays.min(3) as i32);
    let plausibility = 1.0
        - 0.7
            * ((apart - PLAUSIBLE_OFFSET_MM) / (MAX_OFFSET_MM - PLAUSIBLE_OFFSET_MM))
                .clamp(0.0, 1.0);
    let confidence = quality * separation * crowding * plausibility;
    if !confidence.is_finite() || confidence < MIN_CONFIDENCE {
        return None;
    }

    Some(Reading {
        index,
        dx_mm,
        dy_mm,
        confidence,
    })
}

/// Read every offset off a scan of a printed two-pass target.
///
/// The sheet carries its own reference: page 1's ring says where the first pass
/// landed and page 2's diamond says where the second one did, and the answer is
/// the distance between them. Nothing here depends on the scanner having found
/// the paper's edge to better than a millimetre, because an error in the
/// registration moves both marks the same way and cancels in the subtraction.
/// What the registration is needed for is the scale — how many pixels make a
/// millimetre — and the sheet's lean, so that "right" and "down" mean what they
/// mean on the paper rather than on the glass.
///
/// Fiducials that could not be read are left out of the list rather than
/// guessed at. [`solve_from_offsets`] is happy with a subset, and a fitted
/// transform is only as good as its worst reading.
pub fn measure_from_scan(
    image: &image::GrayImage,
    registration: &crate::scan::ScanRegistration,
    page: crate::geometry::PageSize,
    inset_mm: Option<f64>,
) -> Result<Vec<Reading>, CalibrateError> {
    let (width, height) = image.dimensions();
    if width == 0 || height == 0 {
        return Err(CalibrateError::Invalid("the scan is empty".into()));
    }
    let px_per_mm = registration.px_per_mm;
    if !(px_per_mm.is_finite() && px_per_mm > 0.0) {
        return Err(CalibrateError::Invalid(
            "this scan's resolution does not make sense, so nothing on it can be measured".into(),
        ));
    }
    if px_per_mm < MIN_PX_PER_MM {
        return Err(CalibrateError::Invalid(format!(
            "this scan is only {:.0} dpi. The marks on a calibration sheet are drawn in \
             hairlines, about an eighth of a millimetre thick, so scan it again at 300 \
             dpi or more.",
            registration.dpi()
        )));
    }
    // The registration is what turns pixels into millimetres, and it was told a
    // paper size when it was made. If that is not the paper this target was
    // drawn for, every crosshair is looked for in the wrong place — and the
    // measurement would fail with nothing to say why.
    if (registration.page.width_mm - page.width_mm).abs() > 1.0
        || (registration.page.height_mm - page.height_mm).abs() > 1.0
    {
        return Err(CalibrateError::Invalid(format!(
            "this scan was registered as {} but the target was drawn for {}. \
             Measure it against the paper it was printed on.",
            registration.page.describe(),
            page.describe()
        )));
    }

    let inset = inset_mm.unwrap_or_else(|| default_inset(page));
    let mapping = registration.mapping();
    let points = fiducials(page, inset);
    let windows: Vec<Window> = points.iter().map(|p| Window::around(*p)).collect();
    let levels = ink_levels(image, &mapping, &windows);

    let mut readings = Vec::new();
    for (index, centre) in points.into_iter().enumerate() {
        if let Some(reading) = read_fiducial(image, &mapping, px_per_mm, levels, index + 1, centre)
        {
            readings.push(reading);
        }
    }
    Ok(readings)
}

/// Measure a scanned target and fit a profile to it, in one step.
///
/// The profile is handed back rather than saved, so that whatever asked for it
/// can show the fit and its residuals to somebody before it becomes the thing
/// every future delta is corrected by. [`save_profile`] stores it.
pub fn calibrate_from_scan(
    image: &image::GrayImage,
    registration: &crate::scan::ScanRegistration,
    page: crate::geometry::PageSize,
    inset_mm: Option<f64>,
    name: &str,
    notes: &str,
) -> Result<(Profile, Vec<Reading>), CalibrateError> {
    let readings = measure_from_scan(image, registration, page, inset_mm)?;
    let inset = inset_mm.unwrap_or_else(|| default_inset(page));
    let total = fiducials(page, inset).len();

    // Two points and a similarity is not a fit — it is four numbers through
    // four numbers, which passes exactly through both readings and says nothing
    // about whether either was any good. Three is the fewest that can disagree,
    // and disagreement is the only evidence there is that the readings mean
    // something.
    if readings.len() < 3 {
        return Err(CalibrateError::Invalid(format!(
            "only {} of the {total} crosshairs on this sheet could be measured, and \
             three is the fewest a shift, a rotation and a scale can be fitted from.\n    \
             Check that page 1 and page 2 of the target were both printed on this sheet, \
             that the scan is 300 dpi or better, and that the whole sheet is in the image.",
            readings.len()
        )));
    }

    let offsets: Vec<(usize, f64, f64)> = readings
        .iter()
        .map(|r| (r.index, r.dx_mm, r.dy_mm))
        .collect();
    let fit = solve_from_offsets(&offsets, page, inset_mm)?;

    let profile = Profile {
        name: name.to_string(),
        error: fit.transform,
        page,
        rms_residual_mm: Some(fit.rms_residual_mm),
        max_residual_mm: Some(fit.max_residual_mm),
        n_points: readings.len(),
        created: now(),
        notes: notes.to_string(),
    };
    Ok((profile, readings))
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
