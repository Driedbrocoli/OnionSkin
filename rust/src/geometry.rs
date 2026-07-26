//! Units, page geometry, and the similarity transform used for printer calibration.
//!
//! Two coordinate systems appear throughout Onionskin:
//!
//! **page space** — millimetres from the top-left corner of the sheet, x right,
//! y *down*. This is how a person measures a printed page with a ruler, so
//! every user-facing number (region positions, calibration offsets, margins) is
//! in page space.
//!
//! **PDF space** — points from the bottom-left corner, y *up*. Only the PDF
//! writers touch it.
//!
//! [`Similarity`] is defined in page space; [`Similarity::to_pdf_matrix`]
//! handles the flip.

pub const MM_PER_INCH: f64 = 25.4;
pub const PT_PER_INCH: f64 = 72.0;
pub const PT_PER_MM: f64 = PT_PER_INCH / MM_PER_INCH;

pub fn mm_to_pt(mm: f64) -> f64 {
    mm * PT_PER_MM
}

pub fn pt_to_mm(pt: f64) -> f64 {
    pt / PT_PER_MM
}

pub fn mm_to_px(mm: f64, dpi: f64) -> f64 {
    mm * dpi / MM_PER_INCH
}

pub fn px_to_mm(px: f64, dpi: f64) -> f64 {
    px * MM_PER_INCH / dpi
}

/// A page's physical size in millimetres.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PageSize {
    pub width_mm: f64,
    pub height_mm: f64,
}

impl PageSize {
    pub fn new(width_mm: f64, height_mm: f64) -> Self {
        Self {
            width_mm,
            height_mm,
        }
    }

    pub fn from_pt(width_pt: f64, height_pt: f64) -> Self {
        Self::new(pt_to_mm(width_pt), pt_to_mm(height_pt))
    }

    pub fn width_pt(&self) -> f64 {
        mm_to_pt(self.width_mm)
    }

    pub fn height_pt(&self) -> f64 {
        mm_to_pt(self.height_mm)
    }

    pub fn centre_mm(&self) -> (f64, f64) {
        (self.width_mm / 2.0, self.height_mm / 2.0)
    }

    /// Raster dimensions at `dpi`, rounded consistently for old and new.
    pub fn px_size(&self, dpi: f64) -> (u32, u32) {
        (
            mm_to_px(self.width_mm, dpi).round().max(1.0) as u32,
            mm_to_px(self.height_mm, dpi).round().max(1.0) as u32,
        )
    }

    pub fn matches(&self, other: &PageSize, tol_mm: f64) -> bool {
        (self.width_mm - other.width_mm).abs() <= tol_mm
            && (self.height_mm - other.height_mm).abs() <= tol_mm
    }

    pub fn describe(&self) -> String {
        let base = format!("{:.1}×{:.1} mm", self.width_mm, self.height_mm);
        match named_size(self) {
            Some(name) => format!("{name} ({base})"),
            None => base,
        }
    }
}

fn named_size(size: &PageSize) -> Option<&'static str> {
    const NAMED: &[(i64, i64, &str)] = &[
        (210, 297, "A4"),
        (297, 210, "A4 landscape"),
        (216, 279, "Letter"),
        (279, 216, "Letter landscape"),
        (216, 356, "Legal"),
        (148, 210, "A5"),
        (297, 420, "A3"),
    ];
    let w = size.width_mm.round() as i64;
    let h = size.height_mm.round() as i64;
    NAMED
        .iter()
        .find(|(nw, nh, _)| *nw == w && *nh == h)
        .map(|(_, _, name)| *name)
}

/// Every paper size Onionskin knows by name.
///
/// A printer it has never heard of is still fine — a size may be given as
/// `WIDTHxHEIGHT` in millimetres.
pub const PAGE_PRESETS: &[(&str, f64, f64)] = &[
    ("a3", 297.0, 420.0),
    ("a4", 210.0, 297.0),
    ("a5", 148.0, 210.0),
    ("a6", 105.0, 148.0),
    ("b5", 176.0, 250.0),
    ("letter", 215.9, 279.4),
    ("legal", 215.9, 355.6),
    ("tabloid", 279.4, 431.8),
    ("executive", 184.15, 266.7),
    ("statement", 139.7, 215.9),
];

/// Resolve a page name, or a custom `WIDTHxHEIGHT` in millimetres.
pub fn parse_page(spec: &str) -> Result<PageSize, String> {
    let text = spec.trim().to_ascii_lowercase();
    if let Some((_, w, h)) = PAGE_PRESETS.iter().find(|(name, _, _)| *name == text) {
        return Ok(PageSize::new(*w, *h));
    }

    let separator = if text.contains('x') {
        'x'
    } else if text.contains('*') {
        '*'
    } else {
        return Err(unknown_page(spec));
    };

    let parts: Vec<&str> = text.split(separator).collect();
    if parts.len() != 2 {
        return Err(unknown_page(spec));
    }
    let (width, height) = match (
        parts[0].trim().parse::<f64>(),
        parts[1].trim().parse::<f64>(),
    ) {
        (Ok(w), Ok(h)) => (w, h),
        _ => return Err(unknown_page(spec)),
    };
    if !(width.is_finite() && height.is_finite()) || width <= 0.0 || height <= 0.0 {
        return Err(unknown_page(spec));
    }
    if width > 2000.0 || height > 2000.0 {
        return Err(format!("{width}×{height} mm is not a paper size"));
    }
    Ok(PageSize::new(width, height))
}

fn unknown_page(spec: &str) -> String {
    let names: Vec<&str> = PAGE_PRESETS.iter().map(|(n, _, _)| *n).collect();
    format!(
        "unknown page size '{spec}'. Use one of {}, or a custom size like '210x297' (mm).",
        names.join(", ")
    )
}

/// A PDF content-stream matrix `[a b c d e f]`.
///
/// PDF uses the row-vector convention, so a point maps as
/// `x' = a·x + c·y + e`, `y' = b·x + d·y + f`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Matrix {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub e: f64,
    pub f: f64,
}

impl Matrix {
    pub const IDENTITY: Matrix = Matrix {
        a: 1.0,
        b: 0.0,
        c: 0.0,
        d: 1.0,
        e: 0.0,
        f: 0.0,
    };

    pub fn apply(&self, x: f64, y: f64) -> (f64, f64) {
        (
            self.a * x + self.c * y + self.e,
            self.b * x + self.d * y + self.f,
        )
    }

    /// Render as the operands of a `cm` operator.
    pub fn to_cm(&self) -> String {
        format!(
            "{:.9} {:.9} {:.9} {:.9} {:.6} {:.6} cm",
            self.a, self.b, self.c, self.d, self.e, self.f
        )
    }
}

/// A rigid-plus-uniform-scale transform of the printed page.
///
/// This is the full space of registration error a sheet-fed printer can
/// introduce on a second pass: the paper can land shifted, very slightly
/// rotated, and the imaging can be marginally over- or under-scaled. Shear is
/// not physically reachable, so it is deliberately not modelled.
///
/// The transform is applied *about the centre of the page*:
///
/// ```text
/// p' = centre + scale * R(rotation_deg) * (p - centre) + (dx_mm, dy_mm)
/// ```
///
/// `rotation_deg` is positive **clockwise** as you look at the sheet, which is
/// what people mean when they say "it came out slightly rotated right".
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Similarity {
    pub dx_mm: f64,
    pub dy_mm: f64,
    pub rotation_deg: f64,
    pub scale: f64,
}

impl Default for Similarity {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Similarity {
    pub const IDENTITY: Similarity = Similarity {
        dx_mm: 0.0,
        dy_mm: 0.0,
        rotation_deg: 0.0,
        scale: 1.0,
    };

    pub fn is_identity(&self) -> bool {
        self.dx_mm.abs() < 1e-9
            && self.dy_mm.abs() < 1e-9
            && self.rotation_deg.abs() < 1e-9
            && (self.scale - 1.0).abs() < 1e-12
    }

    pub fn apply(&self, point_mm: (f64, f64), page: &PageSize) -> (f64, f64) {
        let (cx, cy) = page.centre_mm();
        let theta = self.rotation_deg.to_radians();
        let (sin_t, cos_t) = theta.sin_cos();
        let (ax, ay) = (point_mm.0 - cx, point_mm.1 - cy);
        // y-down space: positive theta rotates +x toward +y, i.e. clockwise.
        let rx = cos_t * ax - sin_t * ay;
        let ry = sin_t * ax + cos_t * ay;
        (
            cx + self.scale * rx + self.dx_mm,
            cy + self.scale * ry + self.dy_mm,
        )
    }

    /// The transform that undoes this one.
    ///
    /// Derived from `p = c + sR(q - c) + t`: solving for `q` gives scale `1/s`,
    /// rotation `-theta` and translation `-(1/s) R^-1 t`.
    pub fn inverse(&self) -> Similarity {
        let inv_scale = 1.0 / self.scale;
        let theta = (-self.rotation_deg).to_radians();
        let (sin_t, cos_t) = theta.sin_cos();
        let rx = cos_t * self.dx_mm - sin_t * self.dy_mm;
        let ry = sin_t * self.dx_mm + cos_t * self.dy_mm;
        Similarity {
            dx_mm: -inv_scale * rx,
            dy_mm: -inv_scale * ry,
            rotation_deg: -self.rotation_deg,
            scale: inv_scale,
        }
    }

    /// Build the equivalent PDF content-stream matrix.
    ///
    /// PDF space is y-up, so a clockwise page-space rotation becomes a
    /// counter-clockwise PDF rotation and the y translation flips sign.
    pub fn to_pdf_matrix(&self, page: &PageSize) -> Matrix {
        let (centre_x, centre_y) = page.centre_mm();
        let cx = mm_to_pt(centre_x);
        let cy = page.height_pt() - mm_to_pt(centre_y);
        let dx = mm_to_pt(self.dx_mm);
        let dy = -mm_to_pt(self.dy_mm);

        let theta = self.rotation_deg.to_radians();
        let (sin_t, cos_t) = theta.sin_cos();
        // Scale then rotate by -theta in PDF space, as a row-vector matrix.
        let a = self.scale * cos_t;
        let b = -self.scale * sin_t;
        let c = self.scale * sin_t;
        let d = self.scale * cos_t;

        // Move the centre to the origin, transform, then put it back plus the
        // offset: p' = (p - centre)·L + centre + offset.
        Matrix {
            a,
            b,
            c,
            d,
            e: -(cx * a + cy * c) + cx + dx,
            f: -(cx * b + cy * d) + cy + dy,
        }
    }

    pub fn describe(&self) -> String {
        if self.is_identity() {
            return "identity (no correction)".to_string();
        }
        let mut parts: Vec<String> = Vec::new();
        if self.dx_mm.abs() >= 5e-4 || self.dy_mm.abs() >= 5e-4 {
            parts.push(format!("shift {:+.2}, {:+.2} mm", self.dx_mm, self.dy_mm));
        }
        if self.rotation_deg.abs() >= 5e-4 {
            parts.push(format!("rotate {:+.3}° cw", self.rotation_deg));
        }
        if (self.scale - 1.0).abs() >= 5e-7 {
            parts.push(format!(
                "scale {:.5} ({:+.3}%)",
                self.scale,
                (self.scale - 1.0) * 100.0
            ));
        }
        if parts.is_empty() {
            "identity (no correction)".to_string()
        } else {
            parts.join(", ")
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SimilarityFit {
    pub transform: Similarity,
    pub rms_residual_mm: f64,
    pub max_residual_mm: f64,
    pub n_points: usize,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum FitError {
    #[error("nominal and observed must have the same length")]
    LengthMismatch,
    #[error("need at least 2 measured points to solve for shift, rotation and scale")]
    TooFewPoints,
    #[error("measured points are coincident; spread them across the page")]
    Coincident,
}

/// Least-squares fit of a [`Similarity`] mapping `nominal -> observed`.
///
/// Uses the complex-number formulation: in 2D a scale-plus-rotation is just
/// multiplication by a complex number, so the least-squares solution has a
/// closed form and needs no iteration. Written out in real arithmetic here,
/// since the standard library has no complex type.
///
/// `nominal` are the coordinates Onionskin asked the printer for; `observed`
/// are where the ink actually landed, both in page-space mm.
pub fn solve_similarity(
    nominal: &[(f64, f64)],
    observed: &[(f64, f64)],
    page: &PageSize,
) -> Result<SimilarityFit, FitError> {
    if nominal.len() != observed.len() {
        return Err(FitError::LengthMismatch);
    }
    let n = nominal.len();
    if n < 2 {
        return Err(FitError::TooFewPoints);
    }

    let (cx, cy) = page.centre_mm();
    let a: Vec<(f64, f64)> = nominal.iter().map(|p| (p.0 - cx, p.1 - cy)).collect();
    let b: Vec<(f64, f64)> = observed.iter().map(|p| (p.0 - cx, p.1 - cy)).collect();

    let count = n as f64;
    let mean_a = (
        a.iter().map(|p| p.0).sum::<f64>() / count,
        a.iter().map(|p| p.1).sum::<f64>() / count,
    );
    let mean_b = (
        b.iter().map(|p| p.0).sum::<f64>() / count,
        b.iter().map(|p| p.1).sum::<f64>() / count,
    );

    // w = sum(conj(a - mean_a) * (b - mean_b)) / sum(|a - mean_a|^2)
    let mut num_re = 0.0;
    let mut num_im = 0.0;
    let mut den = 0.0;
    for (pa, pb) in a.iter().zip(b.iter()) {
        let (dax, day) = (pa.0 - mean_a.0, pa.1 - mean_a.1);
        let (dbx, dby) = (pb.0 - mean_b.0, pb.1 - mean_b.1);
        num_re += dax * dbx + day * dby;
        num_im += dax * dby - day * dbx;
        den += dax * dax + day * day;
    }
    if den < 1e-12 {
        return Err(FitError::Coincident);
    }
    let (w_re, w_im) = (num_re / den, num_im / den);

    // t = mean_b - w * mean_a
    let t = (
        mean_b.0 - (w_re * mean_a.0 - w_im * mean_a.1),
        mean_b.1 - (w_re * mean_a.1 + w_im * mean_a.0),
    );

    let mut sum_sq = 0.0;
    let mut max_residual: f64 = 0.0;
    for (pa, pb) in a.iter().zip(b.iter()) {
        let px = w_re * pa.0 - w_im * pa.1 + t.0;
        let py = w_re * pa.1 + w_im * pa.0 + t.1;
        let residual = ((px - pb.0).powi(2) + (py - pb.1).powi(2)).sqrt();
        sum_sq += residual * residual;
        max_residual = max_residual.max(residual);
    }

    Ok(SimilarityFit {
        transform: Similarity {
            dx_mm: t.0,
            dy_mm: t.1,
            rotation_deg: w_im.atan2(w_re).to_degrees(),
            scale: (w_re * w_re + w_im * w_im).sqrt(),
        },
        rms_residual_mm: (sum_sq / count).sqrt(),
        max_residual_mm: max_residual,
        n_points: n,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use approx::assert_relative_eq;

    const A4: PageSize = PageSize {
        width_mm: 210.0,
        height_mm: 297.0,
    };

    #[test]
    fn unit_roundtrip() {
        assert_relative_eq!(pt_to_mm(mm_to_pt(123.4)), 123.4, epsilon = 1e-12);
        assert_relative_eq!(mm_to_pt(25.4), 72.0, epsilon = 1e-12);
    }

    #[test]
    fn page_size_from_pt_recognises_a4() {
        let page = PageSize::from_pt(595.276, 841.89);
        assert_relative_eq!(page.width_mm, 210.0, epsilon = 0.05);
        assert!(page.describe().contains("A4"));
    }

    #[test]
    fn translation_moves_point() {
        let t = Similarity {
            dx_mm: 2.0,
            dy_mm: -3.0,
            ..Similarity::IDENTITY
        };
        let (x, y) = t.apply((100.0, 100.0), &A4);
        assert_relative_eq!(x, 102.0, epsilon = 1e-9);
        assert_relative_eq!(y, 97.0, epsilon = 1e-9);
    }

    #[test]
    fn rotation_is_clockwise_on_the_page() {
        // +90 must take a point above centre to the right of centre.
        let t = Similarity {
            rotation_deg: 90.0,
            ..Similarity::IDENTITY
        };
        let (cx, cy) = A4.centre_mm();
        let (x, y) = t.apply((cx, cy - 50.0), &A4);
        assert_relative_eq!(x, cx + 50.0, epsilon = 1e-9);
        assert_relative_eq!(y, cy, epsilon = 1e-9);
    }

    #[test]
    fn scale_is_about_the_page_centre() {
        let t = Similarity {
            scale: 2.0,
            ..Similarity::IDENTITY
        };
        let (cx, cy) = A4.centre_mm();
        let (x, y) = t.apply((cx, cy), &A4);
        assert_relative_eq!(x, cx, epsilon = 1e-9);
        assert_relative_eq!(y, cy, epsilon = 1e-9);
        let (x2, _) = t.apply((cx + 10.0, cy), &A4);
        assert_relative_eq!(x2, cx + 20.0, epsilon = 1e-9);
    }

    #[test]
    fn inverse_undoes_the_transform() {
        let cases = [
            Similarity {
                dx_mm: 0.7,
                dy_mm: -0.4,
                ..Similarity::IDENTITY
            },
            Similarity {
                rotation_deg: 0.35,
                ..Similarity::IDENTITY
            },
            Similarity {
                scale: 1.004,
                ..Similarity::IDENTITY
            },
            Similarity {
                dx_mm: -1.2,
                dy_mm: 0.9,
                rotation_deg: -0.22,
                scale: 0.997,
            },
        ];
        let points = [(10.0, 10.0), (200.0, 287.0), (105.0, 148.5), (55.0, 240.0)];
        for transform in cases {
            let inverse = transform.inverse();
            for point in points {
                let moved = transform.apply(point, &A4);
                let back = inverse.apply(moved, &A4);
                assert_relative_eq!(back.0, point.0, epsilon = 1e-9);
                assert_relative_eq!(back.1, point.1, epsilon = 1e-9);
            }
        }
    }

    #[test]
    fn solve_recovers_a_known_transform() {
        let truth = Similarity {
            dx_mm: 0.42,
            dy_mm: -0.31,
            rotation_deg: 0.18,
            scale: 1.0021,
        };
        let nominal = [
            (25.0, 25.0),
            (185.0, 25.0),
            (25.0, 272.0),
            (185.0, 272.0),
            (105.0, 148.5),
        ];
        let observed: Vec<(f64, f64)> = nominal.iter().map(|p| truth.apply(*p, &A4)).collect();

        let fit = solve_similarity(&nominal, &observed, &A4).unwrap();

        assert_relative_eq!(fit.transform.dx_mm, truth.dx_mm, epsilon = 1e-9);
        assert_relative_eq!(fit.transform.dy_mm, truth.dy_mm, epsilon = 1e-9);
        assert_relative_eq!(
            fit.transform.rotation_deg,
            truth.rotation_deg,
            epsilon = 1e-9
        );
        assert_relative_eq!(fit.transform.scale, truth.scale, epsilon = 1e-12);
        assert!(fit.rms_residual_mm < 1e-9);
        assert_eq!(fit.n_points, 5);
    }

    #[test]
    fn solve_is_robust_to_measurement_noise() {
        let truth = Similarity {
            dx_mm: 0.5,
            dy_mm: 0.3,
            rotation_deg: 0.1,
            scale: 1.001,
        };
        let nominal = [
            (25.0, 25.0),
            (185.0, 25.0),
            (25.0, 272.0),
            (185.0, 272.0),
            (105.0, 148.5),
        ];
        // A person reading a printed ruler resolves about a quarter millimetre.
        let noise = [
            (0.1, -0.1),
            (-0.1, 0.1),
            (0.12, 0.08),
            (-0.08, -0.12),
            (0.05, 0.05),
        ];
        let observed: Vec<(f64, f64)> = nominal
            .iter()
            .zip(noise.iter())
            .map(|(p, n)| {
                let moved = truth.apply(*p, &A4);
                (moved.0 + n.0, moved.1 + n.1)
            })
            .collect();

        let fit = solve_similarity(&nominal, &observed, &A4).unwrap();

        assert_relative_eq!(fit.transform.dx_mm, truth.dx_mm, epsilon = 0.15);
        assert_relative_eq!(fit.transform.dy_mm, truth.dy_mm, epsilon = 0.15);
        assert!(fit.rms_residual_mm < 0.2);
    }

    #[test]
    fn solve_rejects_degenerate_input() {
        assert_eq!(
            solve_similarity(&[(1.0, 1.0)], &[(1.0, 1.0)], &A4).unwrap_err(),
            FitError::TooFewPoints
        );
        let same = [(5.0, 5.0); 3];
        assert_eq!(
            solve_similarity(&same, &same, &A4).unwrap_err(),
            FitError::Coincident
        );
        assert_eq!(
            solve_similarity(&[(1.0, 1.0), (2.0, 2.0)], &[(1.0, 1.0)], &A4).unwrap_err(),
            FitError::LengthMismatch
        );
    }

    /// The PDF matrix must reproduce `apply()`, including the y-axis flip.
    #[test]
    fn pdf_matrix_matches_page_space() {
        let cases = [
            Similarity {
                dx_mm: 2.0,
                dy_mm: 3.0,
                ..Similarity::IDENTITY
            },
            Similarity {
                dx_mm: -0.6,
                dy_mm: 0.4,
                rotation_deg: 0.75,
                scale: 1.003,
            },
            Similarity {
                rotation_deg: -2.0,
                scale: 0.998,
                ..Similarity::IDENTITY
            },
        ];
        let points = [(20.0, 20.0), (190.0, 30.0), (105.0, 148.5), (50.0, 80.0)];
        for transform in cases {
            let matrix = transform.to_pdf_matrix(&A4);
            for point in points {
                let expected = transform.apply(point, &A4);
                let (x_pt, y_pt) = (mm_to_pt(point.0), A4.height_pt() - mm_to_pt(point.1));
                let (gx, gy) = matrix.apply(x_pt, y_pt);
                assert_relative_eq!(pt_to_mm(gx), expected.0, epsilon = 1e-9);
                assert_relative_eq!(pt_to_mm(A4.height_pt() - gy), expected.1, epsilon = 1e-9);
            }
        }
    }

    /// A clockwise page rotation is counter-clockwise in y-up PDF space.
    #[test]
    fn rotation_sign_survives_the_pdf_flip() {
        let matrix = Similarity {
            rotation_deg: 1.0,
            ..Similarity::IDENTITY
        }
        .to_pdf_matrix(&A4);
        assert_relative_eq!(matrix.b.atan2(matrix.a).to_degrees(), -1.0, epsilon = 1e-9);
    }

    #[test]
    fn identity_detection_and_description() {
        assert!(Similarity::IDENTITY.is_identity());
        assert!(!Similarity {
            dx_mm: 0.01,
            ..Similarity::IDENTITY
        }
        .is_identity());
        assert!(Similarity::IDENTITY.describe().contains("identity"));

        let text = Similarity {
            dx_mm: 0.4,
            dy_mm: -0.2,
            rotation_deg: 0.1,
            scale: 1.002,
        }
        .describe();
        assert!(text.contains("+0.40") && text.contains("-0.20"));
        assert!(text.contains("cw") && text.contains('%'));
    }

    #[test]
    fn page_sizes_by_name_or_measurement() {
        assert_eq!(parse_page("a4").unwrap(), PageSize::new(210.0, 297.0));
        assert_eq!(
            parse_page("  LETTER ").unwrap(),
            PageSize::new(215.9, 279.4)
        );
        assert_eq!(parse_page("legal").unwrap(), PageSize::new(215.9, 355.6));
        assert_eq!(parse_page("210x297").unwrap(), PageSize::new(210.0, 297.0));
        assert_eq!(parse_page("100*150").unwrap(), PageSize::new(100.0, 150.0));
    }

    #[test]
    fn impossible_page_sizes_are_refused() {
        for spec in [
            "",
            "nonsense",
            "a4x",
            "1x",
            "0x100",
            "-5x10",
            "9000x9000",
            "1x2x3",
        ] {
            assert!(parse_page(spec).is_err(), "{spec} should be refused");
        }
    }

    #[test]
    fn px_size_is_stable() {
        assert_eq!(A4.px_size(300.0), (2480, 3508));
        assert_eq!(PageSize::new(0.001, 0.001).px_size(72.0), (1, 1));
    }
}
