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
    /// Index into a font collection (`.ttc`); zero for a plain file.
    index: u32,
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

        // A PDF carries TrueType outlines as a plain font programme. Fonts with
        // PostScript (CFF) outlines need a different, much more involved
        // arrangement, so say so plainly rather than emitting a file that
        // reads as valid and prints nothing.
        if face.tables().glyf.is_none() {
            return Err(FontError::Unusable(format!(
                "{} has PostScript outlines, which Onionskin cannot embed yet.\n\
                 Use a TrueType font — a .ttf, or a .ttc collection — which most \
                 systems have for every alphabet.",
                path.display()
            )));
        }

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
        })
    }

    fn face(&self) -> Face<'_> {
        // Parsed once already in `load`, so this cannot fail.
        Face::parse(&self.data, self.index).expect("font re-parse")
    }

    pub fn program(&self) -> &[u8] {
        &self.data
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

#[cfg(test)]
mod tests {
    use super::*;

    fn dejavu() -> Option<PathBuf> {
        let path = PathBuf::from("/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf");
        path.is_file().then_some(path)
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
        let Some(path) = dejavu() else { return };
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
        let Some(path) = dejavu() else { return };
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
        let Some(path) = dejavu() else { return };
        let font = EmbeddedFont::load(&path).unwrap();

        for text in ["Утверждено", "Έγκριση", "café — naïve"] {
            let glyphs = font.shape(text).unwrap();
            assert!(!glyphs.is_empty(), "{text}");
        }
    }

    #[test]
    fn characters_the_font_lacks_are_named_not_dropped() {
        let Some(path) = dejavu() else { return };
        let font = EmbeddedFont::load(&path).unwrap();

        // DejaVu has no CJK.
        let err = font.shape("季度报告").unwrap_err().to_string();
        assert!(err.contains("no glyph for these characters"), "{err}");
        assert!(err.contains("季"), "{err}");
    }

    #[test]
    fn tabs_and_line_endings_are_taken_literally_enough() {
        let Some(path) = dejavu() else { return };
        let font = EmbeddedFont::load(&path).unwrap();

        assert_eq!(font.shape("a\tb").unwrap().len(), 3); // tab became a space
        assert_eq!(font.shape("a\r\nb").unwrap().len(), 2);
    }

    #[test]
    fn width_grows_with_the_text_and_the_size() {
        let Some(path) = dejavu() else { return };
        let font = EmbeddedFont::load(&path).unwrap();

        let short = font.width_mm("Hi", 12.0).unwrap();
        let long = font.width_mm("Hi there, everyone", 12.0).unwrap();
        let bigger = font.width_mm("Hi", 24.0).unwrap();

        assert!(long > short * 3.0);
        assert!((bigger - short * 2.0).abs() < 0.01);
    }
}
