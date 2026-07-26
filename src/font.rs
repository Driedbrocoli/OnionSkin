//! Embedding a font, so the delta can be written in any alphabet.
//!
//! The fonts built into every PDF reader cover Western European text and
//! nothing else. That is fine for a signature and useless for most of the
//! world, and the wrong answer — letting the reader substitute — prints a row
//! of solid blocks onto a sheet that may be someone's only copy.
//!
//! So a font file can be supplied and is carried inside the delta. The glyphs
//! travel with the file, which means the printer needs nothing installed and
//! cannot pick a different face.

use std::path::{Path, PathBuf};

use ttf_parser::Face;

#[derive(Debug, thiserror::Error)]
pub enum FontError {
    #[error("no font file at {0}")]
    Missing(PathBuf),
    #[error("could not read {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("{0}")]
    Unusable(String),
}

/// A font file loaded and ready to embed.
///
/// The whole file is kept: PDF wants the font programme itself, and Onionskin
/// does not subset it. Subsetting would shrink a CJK delta from megabytes to
/// kilobytes, but getting it wrong drops glyphs from someone's document, and a
/// large file that prints correctly beats a small one that does not.
pub struct EmbeddedFont {
    pub name: String,
    data: Vec<u8>,
    units_per_em: f64,
    ascender: f64,
    descender: f64,
    cap_height: f64,
    italic_angle: f64,
    bbox: (f64, f64, f64, f64),
    /// Index into a font collection (`.ttc`/`.otc`); zero for a plain file.
    index: u32,
    outlines: Outlines,
}

/// How a font describes its glyph shapes.
///
/// It decides how the font must be written into a PDF, and the two forms are
/// not interchangeable: a PostScript-flavoured font embedded as if it were
/// TrueType produces a file that reads as valid and prints nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outlines {
    /// Quadratic outlines in a `glyf` table — `.ttf`, `.ttc`.
    TrueType,
    /// Cubic PostScript outlines in a `CFF` table — most `.otf`, and the fonts
    /// Word leans on such as Calibri and Cambria.
    PostScript,
}

impl std::fmt::Debug for EmbeddedFont {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Deliberately not the font programme itself: it is megabytes.
        f.debug_struct("EmbeddedFont")
            .field("name", &self.name)
            .field("bytes", &self.data.len())
            .finish()
    }
}

impl EmbeddedFont {
    pub fn load(path: &Path) -> Result<EmbeddedFont, FontError> {
        Self::load_indexed(path, 0)
    }

    pub fn load_indexed(path: &Path, index: u32) -> Result<EmbeddedFont, FontError> {
        if !path.is_file() {
            return Err(FontError::Missing(path.to_path_buf()));
        }
        let data = std::fs::read(path).map_err(|source| FontError::Io {
            path: path.to_path_buf(),
            source,
        })?;

        let face = Face::parse(&data, index).map_err(|err| {
            FontError::Unusable(format!(
                "{} is not a font Onionskin can read ({err}).",
                path.display()
            ))
        })?;

        // Which kind of outlines the font carries decides how a PDF must hold
        // it. Both are supported; they simply take different shapes.
        let outlines = if face.tables().glyf.is_some() {
            Outlines::TrueType
        } else if face.tables().cff.is_some() {
            Outlines::PostScript
        } else {
            return Err(FontError::Unusable(format!(
                "{} has no outlines Onionskin can use — it is neither a TrueType \
                 nor a PostScript-flavoured font.",
                path.display()
            )));
        };

        let units_per_em = face.units_per_em() as f64;
        if units_per_em <= 0.0 {
            return Err(FontError::Unusable(format!(
                "{} declares a nonsensical em size",
                path.display()
            )));
        }

        let bbox = face.global_bounding_box();
        let scale = 1000.0 / units_per_em;
        let name = font_name(&face).unwrap_or_else(|| {
            path.file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "EmbeddedFont".to_string())
        });

        Ok(EmbeddedFont {
            name: sanitise_name(&name),
            units_per_em,
            ascender: face.ascender() as f64 * scale,
            descender: face.descender() as f64 * scale,
            cap_height: face
                .capital_height()
                .map(|v| v as f64 * scale)
                .unwrap_or(face.ascender() as f64 * scale * 0.7),
            italic_angle: face.italic_angle() as f64,
            bbox: (
                bbox.x_min as f64 * scale,
                bbox.y_min as f64 * scale,
                bbox.x_max as f64 * scale,
                bbox.y_max as f64 * scale,
            ),
            data,
            index,
            outlines,
        })
    }

    fn face(&self) -> Face<'_> {
        // Parsed once already in `load`, so this cannot fail.
        Face::parse(&self.data, self.index).expect("font re-parse")
    }

    pub fn program(&self) -> &[u8] {
        &self.data
    }

    pub fn outlines(&self) -> Outlines {
        self.outlines
    }

    /// How many glyphs the font holds, for sizing the widths table.
    pub fn glyph_count(&self) -> u16 {
        self.face().number_of_glyphs()
    }

    pub fn ascender(&self) -> f64 {
        self.ascender
    }
    pub fn descender(&self) -> f64 {
        self.descender
    }
    pub fn cap_height(&self) -> f64 {
        self.cap_height
    }
    pub fn italic_angle(&self) -> f64 {
        self.italic_angle
    }
    pub fn bbox(&self) -> (f64, f64, f64, f64) {
        self.bbox
    }
    /// Font design units per em — the scale the outlines are drawn in.
    pub fn units_per_em(&self) -> f64 {
        self.units_per_em
    }

    /// Does the font have a glyph for this character?
    pub fn has(&self, ch: char) -> bool {
        self.face().glyph_index(ch).is_some()
    }

    /// Every character the font can draw.
    ///
    /// This is what makes reading a page language-agnostic: the alphabet to
    /// look for is not a list somebody wrote down in English, it is whatever
    /// the font that set the page actually contains. Point Onionskin at a
    /// Greek font and it looks for Greek; at a CJK font and it looks for han.
    pub fn coverage(&self) -> Vec<char> {
        let face = self.face();
        let mut characters = Vec::new();
        if let Some(cmap) = face.tables().cmap {
            for subtable in cmap.subtables {
                // Symbol and Mac-Roman subtables map bytes, not Unicode; a
                // codepoint read out of them means something else entirely.
                if !subtable.is_unicode() {
                    continue;
                }
                subtable.codepoints(|codepoint| {
                    if let Some(ch) = char::from_u32(codepoint) {
                        characters.push(ch);
                    }
                });
            }
        }
        characters.sort_unstable();
        characters.dedup();
        characters
    }

    /// The shape of a character, for comparing against ink on a scan.
    ///
    /// Curves are flattened to line segments here rather than by the caller:
    /// the tolerance belongs with the outline, and everything downstream wants
    /// polygons. Coordinates are font units, y upwards from the baseline —
    /// both conversions are the reader's, since only they know the size.
    pub fn outline(&self, ch: char) -> Option<Vec<Vec<(f64, f64)>>> {
        let face = self.face();
        let id = face.glyph_index(ch)?;
        let mut sink = Flattener::default();
        // A glyph with no outline — a space — reports a bounding box and no
        // contours. That is a real answer, so keep it rather than failing.
        face.outline_glyph(id, &mut sink)?;
        sink.finish_contour();
        Some(sink.contours)
    }
}

/// Collects a glyph's outline, turning curves into line segments.
#[derive(Default)]
struct Flattener {
    contours: Vec<Vec<(f64, f64)>>,
    current: Vec<(f64, f64)>,
    at: (f64, f64),
}

impl Flattener {
    /// Segments per curve. Twenty holds a 2000-unit em to well under a pixel
    /// at any size a page is scanned at, and costs nothing worth counting.
    const STEPS: usize = 20;

    fn finish_contour(&mut self) {
        if self.current.len() >= 3 {
            let done = std::mem::take(&mut self.current);
            self.contours.push(done);
        } else {
            self.current.clear();
        }
    }

    fn push(&mut self, x: f64, y: f64) {
        self.current.push((x, y));
        self.at = (x, y);
    }
}

impl ttf_parser::OutlineBuilder for Flattener {
    fn move_to(&mut self, x: f32, y: f32) {
        self.finish_contour();
        self.push(x as f64, y as f64);
    }

    fn line_to(&mut self, x: f32, y: f32) {
        self.push(x as f64, y as f64);
    }

    fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
        let (x0, y0) = self.at;
        let (x1, y1, x, y) = (x1 as f64, y1 as f64, x as f64, y as f64);
        for step in 1..=Self::STEPS {
            let t = step as f64 / Self::STEPS as f64;
            let u = 1.0 - t;
            self.current.push((
                u * u * x0 + 2.0 * u * t * x1 + t * t * x,
                u * u * y0 + 2.0 * u * t * y1 + t * t * y,
            ));
        }
        self.at = (x, y);
    }

    fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
        let (x0, y0) = self.at;
        let (x1, y1) = (x1 as f64, y1 as f64);
        let (x2, y2) = (x2 as f64, y2 as f64);
        let (x, y) = (x as f64, y as f64);
        for step in 1..=Self::STEPS {
            let t = step as f64 / Self::STEPS as f64;
            let u = 1.0 - t;
            let (uu, tt) = (u * u, t * t);
            self.current.push((
                uu * u * x0 + 3.0 * uu * t * x1 + 3.0 * u * tt * x2 + tt * t * x,
                uu * u * y0 + 3.0 * uu * t * y1 + 3.0 * u * tt * y2 + tt * t * y,
            ));
        }
        self.at = (x, y);
    }

    fn close(&mut self) {
        self.finish_contour();
    }
}

impl EmbeddedFont {
    /// Turn text into the glyphs that will be drawn.
    ///
    /// Characters the font has no glyph for are reported rather than dropped:
    /// silently losing a word is exactly the failure this whole path exists to
    /// avoid, and the person can then pick a font that covers their language.
    pub fn shape(&self, text: &str) -> Result<Vec<Glyph>, FontError> {
        let face = self.face();
        let scale = 1000.0 / self.units_per_em;
        let mut glyphs = Vec::new();
        let mut missing: Vec<char> = Vec::new();

        for ch in text.chars() {
            // A line of PDF text has no tab stops and no line endings.
            let ch = match ch {
                '\t' => ' ',
                '\r' | '\n' => continue,
                other => other,
            };
            match face.glyph_index(ch) {
                Some(id) => {
                    let advance = face
                        .glyph_hor_advance(id)
                        .map(|a| a as f64 * scale)
                        .unwrap_or(0.0);
                    glyphs.push(Glyph {
                        id: id.0,
                        advance_1000: advance,
                    });
                }
                None => {
                    if ch != '\u{FEFF}' && !missing.contains(&ch) {
                        missing.push(ch);
                    }
                }
            }
        }

        if !missing.is_empty() {
            let shown: String = missing
                .iter()
                .take(8)
                .map(|c| c.to_string())
                .collect::<Vec<_>>()
                .join(" ");
            let more = if missing.len() > 8 { " …" } else { "" };
            return Err(FontError::Unusable(format!(
                "'{}' has no glyph for these characters: {shown}{more}\n\
                 Choose a font that covers the language you are writing in.",
                self.name
            )));
        }
        Ok(glyphs)
    }

    /// How wide a string will be, in millimetres, at a given type size.
    pub fn width_mm(&self, text: &str, size_pt: f64) -> Result<f64, FontError> {
        let total: f64 = self.shape(text)?.iter().map(|g| g.advance_1000).sum();
        Ok(crate::geometry::pt_to_mm(total * size_pt / 1000.0))
    }
}

/// One glyph to draw, and how far the pen moves after it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Glyph {
    /// The glyph's index in the font. With Identity-H encoding this is also
    /// the character code written into the PDF.
    pub id: u16,
    /// Advance width in PDF text units, where the em is 1000.
    pub advance_1000: f64,
}

fn font_name(face: &Face<'_>) -> Option<String> {
    face.names()
        .into_iter()
        .find(|name| name.name_id == ttf_parser::name_id::POST_SCRIPT_NAME)
        .and_then(|name| name.to_string())
        .or_else(|| {
            face.names()
                .into_iter()
                .find(|name| name.name_id == ttf_parser::name_id::FULL_NAME)
                .and_then(|name| name.to_string())
        })
}

/// PDF names cannot carry spaces or delimiters.
fn sanitise_name(raw: &str) -> String {
    let cleaned: String = raw
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(60)
        .collect();
    if cleaned.is_empty() {
        "EmbeddedFont".to_string()
    } else {
        cleaned
    }
}

/// Look for a usable font on this machine, for the error messages to suggest.
///
/// A person told "supply a font" and left there is stuck; told "try this file,
/// which you already have" they are not.
pub fn suggest_system_font() -> Option<PathBuf> {
    const CANDIDATES: &[&str] = &[
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
        "/usr/share/fonts/TTF/DejaVuSans.ttf",
        "/Library/Fonts/Arial Unicode.ttf",
        "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
        "C:\\Windows\\Fonts\\arial.ttf",
    ];
    CANDIDATES.iter().map(PathBuf::from).find(|p| p.is_file())
}

/// Every folder worth looking in for fonts on this machine.
///
/// The user's own folders come first, so a face they installed deliberately
/// beats one that happened to be on the machine. After those, the places the
/// system keeps fonts — and the places LibreOffice keeps its own, which are
/// not the same places and are the reason a document can look right in Writer
/// and have Onionskin say it has never heard of the font.
pub fn font_folders() -> Vec<PathBuf> {
    let mut folders = crate::settings::font_folders();

    const SYSTEM: &[&str] = &[
        // Linux
        "/usr/share/fonts",
        "/usr/local/share/fonts",
        // LibreOffice, which ships its own and does not install them system-wide
        "/usr/lib/libreoffice/share/fonts",
        "/usr/lib64/libreoffice/share/fonts",
        "/opt/libreoffice/share/fonts",
        "/snap/libreoffice/current/lib/libreoffice/share/fonts",
        "/var/lib/flatpak/app/org.libreoffice.LibreOffice/current/active/files/lib/libreoffice/share/fonts",
        // macOS
        "/Library/Fonts",
        "/System/Library/Fonts",
        "/System/Library/Fonts/Supplemental",
        "/Applications/LibreOffice.app/Contents/Resources/fonts/truetype",
        // Windows
        "C:\\Windows\\Fonts",
        "C:\\Program Files\\LibreOffice\\share\\fonts\\truetype",
        "C:\\Program Files (x86)\\LibreOffice\\share\\fonts\\truetype",
    ];
    for path in SYSTEM {
        folders.push(PathBuf::from(path));
    }

    // And the per-user places, which is where a font somebody installed by
    // double-clicking it actually lands.
    let home = crate::install::home();
    for tail in [
        ".fonts",
        ".local/share/fonts",
        "Library/Fonts",
        "AppData/Local/Microsoft/Windows/Fonts",
    ] {
        folders.push(home.join(tail));
    }

    let mut seen: Vec<PathBuf> = Vec::new();
    folders.retain(|folder| {
        if !folder.is_dir() {
            return false;
        }
        let key = folder.canonicalize().unwrap_or_else(|_| folder.clone());
        if seen.contains(&key) {
            return false;
        }
        seen.push(key);
        true
    });
    folders
}

/// A font file found on this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    pub path: PathBuf,
    /// What the font calls itself — "DejaVu Sans", not "DejaVuSans.ttf".
    pub family: String,
}

/// Every font file Onionskin can find, named the way its maker named it.
///
/// The name is read out of the file rather than taken from the filename,
/// because the two often disagree and it is the one inside that a person
/// recognises. Reading every font on a machine takes a moment, so this is for
/// listing and for matching by name — not for anything on a hot path.
///
/// Unreadable files are skipped in silence. A fonts folder with something odd
/// in it is completely ordinary and is not the user's problem to hear about.
pub fn installed_fonts() -> Vec<Installed> {
    let mut found: Vec<Installed> = Vec::new();
    for folder in font_folders() {
        collect_fonts(&folder, 0, &mut found);
    }
    found.sort_by(|a, b| {
        a.family
            .to_lowercase()
            .cmp(&b.family.to_lowercase())
            .then_with(|| a.path.cmp(&b.path))
    });
    found.dedup_by(|a, b| a.family == b.family);
    found
}

/// Walk a folder for font files, a few levels down.
///
/// Bounded rather than unlimited: `/usr/share/fonts` is two or three deep, and
/// a folder somebody names by mistake — their home directory, say — should
/// cost a moment rather than a walk of the whole disk.
fn collect_fonts(folder: &Path, depth: usize, found: &mut Vec<Installed>) {
    const DEEPEST: usize = 4;
    if depth > DEEPEST {
        return;
    }
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_fonts(&path, depth + 1, found);
            continue;
        }
        let is_font = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| {
                let e = e.to_ascii_lowercase();
                e == "ttf" || e == "otf" || e == "ttc" || e == "otc"
            })
            .unwrap_or(false);
        if !is_font {
            continue;
        }
        if let Ok(font) = EmbeddedFont::load(&path) {
            found.push(Installed {
                family: font.name.clone(),
                path,
            });
        }
    }
}

/// Find a font by the name somebody would type.
///
/// Matched loosely on purpose: "Liberation Serif", "liberationserif" and
/// "LiberationSerif-Regular" are the same request as far as anybody asking is
/// concerned, and refusing over a space would be pedantry.
pub fn find_font(name: &str) -> Option<PathBuf> {
    let wanted = squash(name);
    if wanted.is_empty() {
        return None;
    }
    let installed = installed_fonts();
    // An exact name first, so asking for "Arial" cannot land on "Arial Black".
    installed
        .iter()
        .find(|font| squash(&font.family) == wanted)
        .or_else(|| {
            installed
                .iter()
                .find(|font| squash(&font.family).contains(&wanted))
        })
        .map(|font| font.path.clone())
}

/// A name with everything that people vary about it taken out.
fn squash(name: &str) -> String {
    name.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn dejavu_path() -> Option<PathBuf> {
        let path = PathBuf::from("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf");
        path.is_file().then_some(path)
    }

    /// A font with PostScript outlines — the shape Word's own faces take.
    pub(crate) fn postscript_font() -> Option<PathBuf> {
        const CANDIDATES: [&str; 4] = [
            "/usr/share/fonts/opentype/tlwg/Loma.otf",
            "/usr/share/fonts/opentype/unifont/unifont_sample.otf",
            "/Library/Fonts/Optima.ttc",
            "C:/Windows/Fonts/calibri.ttf",
        ];
        CANDIDATES
            .iter()
            .map(PathBuf::from)
            .filter(|p| p.is_file())
            .find(|p| {
                EmbeddedFont::load(p)
                    .map(|f| f.outlines() == Outlines::PostScript)
                    .unwrap_or(false)
            })
    }

    #[test]
    fn a_font_folder_the_user_names_is_searched() {
        // The point of the setting: a face that is not where the system keeps
        // fonts — LibreOffice's own, or one somebody bought — is found anyway.
        let _home = crate::calibrate::borrow_home(
            &tempfile::tempdir().expect("a temporary home").keep(),
        );
        let Some(source) = dejavu_path() else {
            return;
        };
        let folder = tempfile::tempdir().unwrap();
        let planted = folder.path().join("Planted.ttf");
        std::fs::copy(&source, &planted).unwrap();

        assert!(
            !font_folders().contains(&folder.path().to_path_buf()),
            "the folder should not be searched before it is added"
        );
        crate::settings::add_font_folder(folder.path());
        let searched = font_folders();
        assert!(
            searched.iter().any(|f| f.starts_with(folder.path())
                || folder.path().starts_with(f.as_path())),
            "the added folder is not being searched: {searched:?}"
        );
    }

    #[test]
    fn a_font_is_found_by_the_name_a_person_would_type() {
        // "Liberation Serif", "liberationserif" and "LiberationSerif" are the
        // same request. Refusing over a space would be pedantry.
        assert_eq!(squash("Liberation Serif"), "liberationserif");
        assert_eq!(squash("LiberationSerif-Regular"), "liberationserifregular");
        assert_eq!(squash("  DejaVu  Sans  "), "dejavusans");
        assert!(squash("").is_empty());
        // An empty request must not match the first font on the machine.
        assert!(find_font("").is_none());
    }

    #[test]
    fn a_folder_full_of_things_that_are_not_fonts_is_no_trouble() {
        // A fonts folder with a stray README, a broken file and a directory in
        // it is completely ordinary, and none of it is the user's problem.
        let folder = tempfile::tempdir().unwrap();
        std::fs::write(folder.path().join("README"), b"not a font").unwrap();
        std::fs::write(folder.path().join("broken.ttf"), b"not a font either").unwrap();
        std::fs::create_dir(folder.path().join("subfolder")).unwrap();

        let mut found = Vec::new();
        collect_fonts(folder.path(), 0, &mut found);
        assert!(found.is_empty(), "{found:?}");
    }

    #[test]
    fn a_missing_file_is_reported() {
        let err = EmbeddedFont::load(Path::new("/nonexistent/font.ttf")).unwrap_err();
        assert!(err.to_string().contains("no font file"));
    }

    #[test]
    fn a_file_that_is_not_a_font_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notafont.ttf");
        std::fs::write(&path, b"this is not a font at all").unwrap();

        let err = EmbeddedFont::load(&path).unwrap_err();
        assert!(err.to_string().contains("not a font"), "{err}");
    }

    #[test]
    fn a_real_font_loads_with_sane_metrics() {
        let Some(path) = dejavu_path() else { return };
        let font = EmbeddedFont::load(&path).unwrap();

        assert!(!font.name.is_empty());
        assert!(font.ascender() > 0.0 && font.ascender() < 2000.0);
        assert!(font.descender() < 0.0);
        assert!(!font.program().is_empty());
        // A PDF name may not contain spaces or delimiters.
        assert!(font
            .name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'));
    }

    #[test]
    fn text_becomes_glyphs_that_advance() {
        let Some(path) = dejavu_path() else { return };
        let font = EmbeddedFont::load(&path).unwrap();

        let glyphs = font.shape("Hello").unwrap();
        assert_eq!(glyphs.len(), 5);
        assert!(glyphs.iter().all(|g| g.advance_1000 > 0.0));
        // A space is narrower than a capital.
        let space = font.shape(" ").unwrap()[0].advance_1000;
        let capital = font.shape("H").unwrap()[0].advance_1000;
        assert!(space < capital);
    }

    #[test]
    fn cyrillic_and_greek_are_carried() {
        let Some(path) = dejavu_path() else { return };
        let font = EmbeddedFont::load(&path).unwrap();

        for text in ["Утверждено", "Έγκριση", "café — naïve"] {
            let glyphs = font.shape(text).unwrap();
            assert!(!glyphs.is_empty(), "{text}");
        }
    }

    #[test]
    fn characters_the_font_lacks_are_named_not_dropped() {
        let Some(path) = dejavu_path() else { return };
        let font = EmbeddedFont::load(&path).unwrap();

        // DejaVu has no CJK.
        let err = font.shape("季度报告").unwrap_err().to_string();
        assert!(err.contains("no glyph for these characters"), "{err}");
        assert!(err.contains("季"), "{err}");
    }

    #[test]
    fn tabs_and_line_endings_are_taken_literally_enough() {
        let Some(path) = dejavu_path() else { return };
        let font = EmbeddedFont::load(&path).unwrap();

        assert_eq!(font.shape("a\tb").unwrap().len(), 3); // tab became a space
        assert_eq!(font.shape("a\r\nb").unwrap().len(), 2);
    }

    #[test]
    fn width_grows_with_the_text_and_the_size() {
        let Some(path) = dejavu_path() else { return };
        let font = EmbeddedFont::load(&path).unwrap();

        let short = font.width_mm("Hi", 12.0).unwrap();
        let long = font.width_mm("Hi there, everyone", 12.0).unwrap();
        let bigger = font.width_mm("Hi", 24.0).unwrap();

        assert!(long > short * 3.0);
        assert!((bigger - short * 2.0).abs() < 0.01);
    }

    #[test]
    fn truetype_outlines_are_recognised() {
        let Some(path) = dejavu_path() else { return };
        let font = EmbeddedFont::load(&path).unwrap();
        assert_eq!(font.outlines(), Outlines::TrueType);
    }

    #[test]
    fn postscript_outlines_load_rather_than_being_refused() {
        // Word's own faces — Calibri, Cambria — are PostScript-flavoured, so
        // refusing this format would refuse the fonts most documents use.
        let Some(path) = postscript_font() else {
            return;
        };
        let font = EmbeddedFont::load(&path).unwrap();

        assert_eq!(font.outlines(), Outlines::PostScript);
        assert!(font.ascender() > 0.0);
        assert!(!font.program().is_empty());
        assert!(!font.shape("Approved").unwrap().is_empty());
        assert!(font.width_mm("Approved", 12.0).unwrap() > 0.0);
    }

    #[test]
    fn a_font_with_no_outlines_at_all_is_refused() {
        // A colour-emoji font carries bitmaps and nothing to draw with.
        let path = PathBuf::from("/usr/share/fonts/truetype/noto/NotoColorEmoji.ttf");
        if !path.is_file() {
            return;
        }
        let Err(err) = EmbeddedFont::load(&path) else {
            // Some builds do carry outlines; then loading is correct.
            return;
        };
        assert!(err.to_string().contains("no outlines"), "{err}");
    }

    #[test]
    fn glyph_count_covers_the_glyphs_shaping_returns() {
        let Some(path) = dejavu_path() else { return };
        let font = EmbeddedFont::load(&path).unwrap();

        let count = font.glyph_count();
        assert!(count > 0);
        for glyph in font.shape("Approved 25 July").unwrap() {
            assert!(glyph.id < count, "glyph {} of {count}", glyph.id);
        }
    }
}
