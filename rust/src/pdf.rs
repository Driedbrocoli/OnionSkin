//! Writing the delta PDF.
//!
//! The delta is deliberately plain: one page per sheet, at exactly the sheet's
//! size, blank except for the words being added. It goes straight to a printer
//! driver, so it uses the fonts every PDF reader already has rather than
//! embedding anything, and it carries no transparency, no soft masks and no
//! features a tired old driver might refuse.

use std::collections::BTreeMap;
use std::path::Path;

use lopdf::content::{Content, Operation};
use lopdf::{dictionary, Document, Object, Stream};

use crate::font::EmbeddedFont;
use crate::geometry::{mm_to_pt, PageSize};

/// The fonts built into every PDF reader.
///
/// Using one of these keeps the delta a few kilobytes and removes a whole
/// class of printer trouble, at the cost of Western European characters only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Font {
    Helvetica,
    HelveticaBold,
    HelveticaOblique,
    TimesRoman,
    TimesBold,
    TimesItalic,
    Courier,
    CourierBold,
}

impl Font {
    pub fn base_name(&self) -> &'static str {
        match self {
            Font::Helvetica => "Helvetica",
            Font::HelveticaBold => "Helvetica-Bold",
            Font::HelveticaOblique => "Helvetica-Oblique",
            Font::TimesRoman => "Times-Roman",
            Font::TimesBold => "Times-Bold",
            Font::TimesItalic => "Times-Italic",
            Font::Courier => "Courier",
            Font::CourierBold => "Courier-Bold",
        }
    }

    pub fn parse(name: &str) -> Option<Font> {
        Some(match name.trim().to_ascii_lowercase().as_str() {
            "helvetica" => Font::Helvetica,
            "helvetica-bold" | "helvetica_bold" | "bold" => Font::HelveticaBold,
            "helvetica-oblique" | "italic" | "oblique" => Font::HelveticaOblique,
            "times-roman" | "times" => Font::TimesRoman,
            "times-bold" => Font::TimesBold,
            "times-italic" => Font::TimesItalic,
            "courier" | "mono" => Font::Courier,
            "courier-bold" => Font::CourierBold,
            _ => return None,
        })
    }

    pub fn all() -> &'static [Font] {
        &[
            Font::Helvetica,
            Font::HelveticaBold,
            Font::HelveticaOblique,
            Font::TimesRoman,
            Font::TimesBold,
            Font::TimesItalic,
            Font::Courier,
            Font::CourierBold,
        ]
    }
}

/// Which face a line is set in.
///
/// One embedded font at a time is enough for the job — a person adding words
/// to a form writes them in one hand — and it keeps the delta to a single
/// copy of what is often a very large file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineFont {
    /// One of the faces every PDF reader already has.
    Builtin(Font),
    /// The font supplied alongside, carried inside the delta.
    Embedded,
}

/// A single line of text, positioned in page space (mm from the top-left).
///
/// `y_mm` is the text *baseline*, not the top of the block — the caller has
/// already decided where the line sits, and keeping this type dumb means the
/// PDF layer never second-guesses the layout.
#[derive(Debug, Clone)]
pub struct PlacedLine {
    pub text: String,
    pub x_mm: f64,
    pub y_mm: f64,
    pub size_pt: f64,
    pub font: LineFont,
    pub rotation_deg: f64,
    pub colour: (f64, f64, f64),
}

#[derive(Debug, thiserror::Error)]
pub enum PdfError {
    #[error("{0}")]
    Text(String),
    #[error("{0}")]
    Font(#[from] crate::font::FontError),
    #[error("a line asks for the supplied font, but none was given")]
    NoEmbeddedFont,
    #[error("could not write the PDF: {0}")]
    Io(#[from] std::io::Error),
    #[error("could not build the PDF: {0}")]
    Lopdf(#[from] lopdf::Error),
}

/// Encode text the way a base-14 font expects it.
///
/// These fonts are WinAnsi-encoded, so anything outside Western European text
/// simply has no glyph. Substituting silently is the worst outcome — it prints
/// a row of solid blocks onto a sheet that may be someone's only copy — so
/// unencodable characters are refused by name.
pub fn encode_winansi(text: &str) -> Result<Vec<u8>, PdfError> {
    let mut out = Vec::with_capacity(text.len());
    let mut missing: Vec<char> = Vec::new();

    for ch in text.chars() {
        // Text pasted out of a document arrives with tabs and carriage
        // returns in it. A line of PDF text has no tab stops to honour and no
        // notion of a line ending, so the sensible readings are a space and
        // nothing — complaining about them as if they were foreign alphabets
        // would be both wrong and unhelpful.
        match ch {
            '\t' => {
                out.push(b' ');
                continue;
            }
            '\r' | '\n' => continue,
            _ => {}
        }
        if (ch as u32) < 0x20 || ch as u32 == 0x7F {
            return Err(PdfError::Text(format!(
                "the text contains a control character (code {}), which cannot be \
                 printed. Remove it, or retype the line.",
                ch as u32
            )));
        }
        match winansi_byte(ch) {
            Some(byte) => out.push(byte),
            None => {
                if !missing.contains(&ch) {
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
        return Err(PdfError::Text(format!(
            "the built-in fonts cannot write these characters: {shown}{more}\n\
             They cover Western European text only, and Onionskin will not print \
             a row of blocks in their place. Pass --font-file with a .ttf that \
             covers your language and it will be carried inside the delta."
        )));
    }
    Ok(out)
}

/// Map a character to its WinAnsiEncoding byte.
fn winansi_byte(ch: char) -> Option<u8> {
    let code = ch as u32;
    // ASCII and the Latin-1 upper range map straight through.
    if (0x20..=0x7E).contains(&code) || (0xA0..=0xFF).contains(&code) {
        return Some(code as u8);
    }
    // WinAnsi fills 0x80-0x9F with typographic characters that Latin-1 leaves
    // as control codes; these are exactly the ones real documents use.
    Some(match ch {
        '\u{20AC}' => 0x80, // euro
        '\u{201A}' => 0x82,
        '\u{0192}' => 0x83,
        '\u{201E}' => 0x84,
        '\u{2026}' => 0x85, // ellipsis
        '\u{2020}' => 0x86,
        '\u{2021}' => 0x87,
        '\u{02C6}' => 0x88,
        '\u{2030}' => 0x89,
        '\u{0160}' => 0x8A,
        '\u{2039}' => 0x8B,
        '\u{0152}' => 0x8C,
        '\u{017D}' => 0x8E,
        '\u{2018}' => 0x91, // curly quotes
        '\u{2019}' => 0x92,
        '\u{201C}' => 0x93,
        '\u{201D}' => 0x94,
        '\u{2022}' => 0x95, // bullet
        '\u{2013}' => 0x96, // en dash
        '\u{2014}' => 0x97, // em dash
        '\u{02DC}' => 0x98,
        '\u{2122}' => 0x99, // trademark
        '\u{0161}' => 0x9A,
        '\u{203A}' => 0x9B,
        '\u{0153}' => 0x9C,
        '\u{017E}' => 0x9E,
        '\u{0178}' => 0x9F,
        _ => return None,
    })
}

/// Build a delta PDF: blank pages of the given sizes, carrying only these lines.
///
/// `embedded` is the font supplied by the user, if any; lines marked
/// [`LineFont::Embedded`] are set in it and it travels inside the file.
pub fn write_delta(
    path: &Path,
    pages: &[PageSize],
    lines_per_page: &[Vec<PlacedLine>],
    title: &str,
    embedded: Option<&EmbeddedFont>,
) -> Result<(), PdfError> {
    let mut doc = Document::with_version("1.4");
    let pages_id = doc.new_object_id();

    // One font object per built-in face actually used, shared across pages.
    let mut font_ids: BTreeMap<&'static str, Object> = BTreeMap::new();
    let mut uses_embedded = false;
    for lines in lines_per_page {
        for line in lines {
            match line.font {
                LineFont::Builtin(font) => {
                    let name = font.base_name();
                    font_ids.entry(name).or_insert_with(|| {
                        Object::Reference(doc.add_object(dictionary! {
                            "Type" => "Font",
                            "Subtype" => "Type1",
                            "BaseFont" => name,
                            "Encoding" => "WinAnsiEncoding",
                        }))
                    });
                }
                LineFont::Embedded => uses_embedded = true,
            }
        }
    }
    if uses_embedded && embedded.is_none() {
        return Err(PdfError::NoEmbeddedFont);
    }

    let font_key: BTreeMap<&str, String> = font_ids
        .keys()
        .enumerate()
        .map(|(index, name)| (*name, format!("F{index}")))
        .collect();

    let mut font_dict = lopdf::Dictionary::new();
    for (name, id) in font_ids.iter() {
        font_dict.set(font_key[name].clone(), id.clone());
    }

    // Shape every embedded line up front: it validates the text against the
    // font before anything is written, and collects the widths the PDF needs.
    let mut shaped: Vec<Vec<Option<Vec<crate::font::Glyph>>>> = Vec::new();
    let mut used_glyphs: BTreeMap<u16, f64> = BTreeMap::new();
    for lines in lines_per_page {
        let mut page_shapes = Vec::new();
        for line in lines {
            match line.font {
                LineFont::Embedded => {
                    let font = embedded.ok_or(PdfError::NoEmbeddedFont)?;
                    let glyphs = font.shape(&line.text)?;
                    for glyph in &glyphs {
                        used_glyphs.insert(glyph.id, glyph.advance_1000);
                    }
                    page_shapes.push(Some(glyphs));
                }
                LineFont::Builtin(_) => page_shapes.push(None),
            }
        }
        shaped.push(page_shapes);
    }

    const EMBEDDED_KEY: &str = "FE";
    if uses_embedded {
        let font = embedded.expect("checked above");
        let id = add_embedded_font(&mut doc, font, &used_glyphs);
        font_dict.set(EMBEDDED_KEY, id);
    }

    let resources_id = doc.add_object(dictionary! { "Font" => font_dict });

    let mut page_ids: Vec<Object> = Vec::new();
    for (index, size) in pages.iter().enumerate() {
        let empty: Vec<PlacedLine> = Vec::new();
        let lines = lines_per_page.get(index).unwrap_or(&empty);
        let no_shapes: Vec<Option<Vec<crate::font::Glyph>>> = Vec::new();
        let shapes = shaped.get(index).unwrap_or(&no_shapes);
        let content = page_content(size, lines, shapes, &font_key, EMBEDDED_KEY)?;
        let content_id = doc.add_object(Stream::new(dictionary! {}, content));

        let page_id = doc.add_object(dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content_id,
            "Resources" => resources_id,
            "MediaBox" => vec![
                0.into(),
                0.into(),
                Object::Real(round6(size.width_pt()) as f32),
                Object::Real(round6(size.height_pt()) as f32),
            ],
        });
        page_ids.push(page_id.into());
    }

    let count = page_ids.len() as i64;
    doc.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => page_ids,
            "Count" => count,
        }),
    );

    let catalog_id = doc.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    let info_id = doc.add_object(dictionary! {
        "Title" => Object::string_literal(title),
        "Producer" => Object::string_literal("Onionskin"),
    });
    doc.trailer.set("Root", catalog_id);
    doc.trailer.set("Info", info_id);

    doc.compress();
    doc.save(path)?;
    Ok(())
}

/// Write the supplied font into the document as a composite font.
///
/// Identity-H encoding means the character codes in the content stream are the
/// font's own glyph numbers, which sidesteps every question of what encoding
/// the text was in — the glyphs have already been chosen.
fn add_embedded_font(
    doc: &mut Document,
    font: &EmbeddedFont,
    used_glyphs: &BTreeMap<u16, f64>,
) -> Object {
    let program = font.program();
    let mut stream = Stream::new(
        dictionary! { "Length1" => program.len() as i64 },
        program.to_vec(),
    );
    // Compressing the font programme is what keeps a delta sane: the file is
    // often several megabytes and the printer has to receive all of it.
    let _ = stream.compress();
    let file_id = doc.add_object(stream);

    let (x0, y0, x1, y1) = font.bbox();
    let descriptor_id = doc.add_object(dictionary! {
        "Type" => "FontDescriptor",
        "FontName" => Object::Name(font.name.clone().into_bytes()),
        // Symbolic: the text is addressed by glyph, not by a named encoding.
        "Flags" => 4_i64,
        "FontBBox" => vec![
            Object::Real(x0 as f32), Object::Real(y0 as f32),
            Object::Real(x1 as f32), Object::Real(y1 as f32),
        ],
        "ItalicAngle" => Object::Real(font.italic_angle() as f32),
        "Ascent" => Object::Real(font.ascender() as f32),
        "Descent" => Object::Real(font.descender() as f32),
        "CapHeight" => Object::Real(font.cap_height() as f32),
        // Nominal: nothing in a print path reads it, but it is required.
        "StemV" => 80_i64,
        "FontFile2" => file_id,
    });

    // Widths for the glyphs actually drawn, one entry each.
    let mut widths: Vec<Object> = Vec::new();
    for (glyph, advance) in used_glyphs {
        widths.push(Object::Integer(*glyph as i64));
        widths.push(Object::Array(vec![Object::Real(*advance as f32)]));
    }

    let cid_id = doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "CIDFontType2",
        "BaseFont" => Object::Name(font.name.clone().into_bytes()),
        "CIDSystemInfo" => dictionary! {
            "Registry" => Object::string_literal("Adobe"),
            "Ordering" => Object::string_literal("Identity"),
            "Supplement" => 0_i64,
        },
        "FontDescriptor" => descriptor_id,
        "DW" => 1000_i64,
        "W" => Object::Array(widths),
        "CIDToGIDMap" => "Identity",
    });

    Object::Reference(doc.add_object(dictionary! {
        "Type" => "Font",
        "Subtype" => "Type0",
        "BaseFont" => Object::Name(font.name.clone().into_bytes()),
        "Encoding" => "Identity-H",
        "DescendantFonts" => vec![Object::Reference(cid_id)],
    }))
}

fn round6(value: f64) -> f64 {
    (value * 1e6).round() / 1e6
}

fn page_content(
    size: &PageSize,
    lines: &[PlacedLine],
    shapes: &[Option<Vec<crate::font::Glyph>>],
    font_key: &BTreeMap<&str, String>,
    embedded_key: &str,
) -> Result<Vec<u8>, PdfError> {
    let mut operations: Vec<Operation> = Vec::new();

    for (index, line) in lines.iter().enumerate() {
        // Character codes, and the resource name to set them in.
        let (encoded, name, hex) = match line.font {
            LineFont::Builtin(font) => {
                let bytes = encode_winansi(&line.text)?;
                let name = font_key
                    .get(font.base_name())
                    .cloned()
                    .unwrap_or_else(|| "F0".to_string());
                (bytes, name, false)
            }
            LineFont::Embedded => {
                let glyphs = shapes
                    .get(index)
                    .and_then(|g| g.as_ref())
                    .ok_or(PdfError::NoEmbeddedFont)?;
                // Identity-H: two bytes per glyph, big-endian.
                let mut bytes = Vec::with_capacity(glyphs.len() * 2);
                for glyph in glyphs {
                    bytes.push((glyph.id >> 8) as u8);
                    bytes.push((glyph.id & 0xFF) as u8);
                }
                (bytes, embedded_key.to_string(), true)
            }
        };
        if encoded.is_empty() {
            continue;
        }

        // Page space is y-down from the top-left; PDF is y-up from the bottom.
        let x = mm_to_pt(line.x_mm);
        let y = size.height_pt() - mm_to_pt(line.y_mm);

        operations.push(Operation::new("q", vec![]));
        operations.push(Operation::new(
            "rg",
            vec![
                Object::Real(line.colour.0 as f32),
                Object::Real(line.colour.1 as f32),
                Object::Real(line.colour.2 as f32),
            ],
        ));
        operations.push(Operation::new("BT", vec![]));
        operations.push(Operation::new(
            "Tf",
            vec![
                Object::Name(name.into_bytes()),
                Object::Real(line.size_pt as f32),
            ],
        ));

        if line.rotation_deg.abs() > 1e-9 {
            // Page-space clockwise is counter-clockwise in y-up PDF space.
            let theta = (-line.rotation_deg).to_radians();
            let (sin_t, cos_t) = theta.sin_cos();
            operations.push(Operation::new(
                "Tm",
                vec![
                    Object::Real(cos_t as f32),
                    Object::Real(sin_t as f32),
                    Object::Real(-sin_t as f32),
                    Object::Real(cos_t as f32),
                    Object::Real(x as f32),
                    Object::Real(y as f32),
                ],
            ));
        } else {
            operations.push(Operation::new(
                "Td",
                vec![Object::Real(x as f32), Object::Real(y as f32)],
            ));
        }

        let format = if hex {
            lopdf::StringFormat::Hexadecimal
        } else {
            lopdf::StringFormat::Literal
        };
        operations.push(Operation::new(
            "Tj",
            vec![Object::String(encoded, format)],
        ));
        operations.push(Operation::new("ET", vec![]));
        operations.push(Operation::new("Q", vec![]));
    }

    Ok(Content { operations }.encode()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn line(text: &str) -> PlacedLine {
        PlacedLine {
            text: text.to_string(),
            x_mm: 20.0,
            y_mm: 40.0,
            size_pt: 12.0,
            font: LineFont::Builtin(Font::Helvetica),
            rotation_deg: 0.0,
            colour: (0.0, 0.0, 0.0),
        }
    }

    #[test]
    fn writes_a_readable_pdf_at_the_right_size() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("d.pdf");
        let a4 = PageSize::new(210.0, 297.0);

        write_delta(&path, &[a4], &[vec![line("Approved")]], "test", None).unwrap();

        let doc = Document::load(&path).unwrap();
        let pages = doc.get_pages();
        assert_eq!(pages.len(), 1);

        let page_id = *pages.values().next().unwrap();
        let media = doc
            .get_dictionary(page_id)
            .unwrap()
            .get(b"MediaBox")
            .unwrap()
            .as_array()
            .unwrap();
        let width = media[2].as_float().unwrap();
        assert!((width - a4.width_pt() as f32).abs() < 0.5, "width {width}");
    }

    #[test]
    fn a_page_with_no_lines_is_blank_but_present() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("d.pdf");
        let a4 = PageSize::new(210.0, 297.0);

        write_delta(&path, &[a4, a4], &[vec![line("x")], vec![]], "test", None).unwrap();

        let doc = Document::load(&path).unwrap();
        assert_eq!(doc.get_pages().len(), 2);
    }

    #[test]
    fn pages_may_differ_in_size() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("d.pdf");
        let sizes = [
            PageSize::new(210.0, 297.0),
            PageSize::new(297.0, 210.0),
            PageSize::new(215.9, 355.6),
        ];

        write_delta(&path, &sizes, &[vec![], vec![], vec![]], "t", None).unwrap();

        let doc = Document::load(&path).unwrap();
        let pages = doc.get_pages();
        assert_eq!(pages.len(), 3);
        for (index, page_id) in pages.values().enumerate() {
            let media = doc
                .get_dictionary(*page_id)
                .unwrap()
                .get(b"MediaBox")
                .unwrap()
                .as_array()
                .unwrap();
            let width = media[2].as_float().unwrap() as f64;
            assert!((width - sizes[index].width_pt()).abs() < 0.5);
        }
    }

    #[test]
    fn tabs_and_line_endings_are_taken_literally_enough() {
        // Pasting from a document brings these along; neither means anything
        // inside a single run of PDF text.
        assert_eq!(encode_winansi("a\tb").unwrap(), b"a b".to_vec());
        assert_eq!(encode_winansi("a\r\nb").unwrap(), b"ab".to_vec());
    }

    #[test]
    fn a_control_character_is_named_as_such() {
        let message = encode_winansi("a\u{7}b").unwrap_err().to_string();
        assert!(message.contains("control character"), "{message}");
    }

    #[test]
    fn western_european_text_encodes() {
        assert!(encode_winansi("café — naïve «déjà» £50 €20").is_ok());
        assert_eq!(encode_winansi("A").unwrap(), vec![b'A']);
        assert_eq!(encode_winansi("\u{2014}").unwrap(), vec![0x97]);
    }

    #[test]
    fn unwritable_characters_are_named_not_substituted() {
        for text in ["季度报告", "Утверждено", "تمت", "Approved ✅"] {
            let err = encode_winansi(text).unwrap_err();
            let message = err.to_string();
            assert!(
                message.contains("cannot write these characters"),
                "unexpected: {message}"
            );
            // The advice must name the way out, which now exists.
            assert!(message.contains("--font-file"), "{message}");
        }
    }

    #[test]
    fn parentheses_and_backslashes_survive() {
        // These are the literal-string delimiters; lopdf must escape them.
        let dir = tempdir().unwrap();
        let path = dir.path().join("d.pdf");
        write_delta(
            &path,
            &[PageSize::new(210.0, 297.0)],
            &[vec![line(r"a (b) \c\ (((")]],
            "t",
            None,
        )
        .unwrap();
        assert!(Document::load(&path).is_ok());
    }

    #[test]
    fn font_names_round_trip() {
        for font in Font::all() {
            assert_eq!(Font::parse(font.base_name()), Some(*font));
        }
        assert_eq!(Font::parse("Comic Sans"), None);
    }
}
