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
    pub font: Font,
    pub rotation_deg: f64,
    pub colour: (f64, f64, f64),
}

#[derive(Debug, thiserror::Error)]
pub enum PdfError {
    #[error("{0}")]
    Text(String),
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
             They cover Western European text only. Onionskin will not print a \
             row of blocks in their place, so this line is refused rather than \
             spoiled — writing in other alphabets needs an embedded font, which \
             the scanned-page command cannot do yet."
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
pub fn write_delta(
    path: &Path,
    pages: &[PageSize],
    lines_per_page: &[Vec<PlacedLine>],
    title: &str,
) -> Result<(), PdfError> {
    let mut doc = Document::with_version("1.4");
    let pages_id = doc.new_object_id();

    // One font object per base font actually used, shared across pages.
    let mut font_ids: BTreeMap<&'static str, Object> = BTreeMap::new();
    for lines in lines_per_page {
        for line in lines {
            let name = line.font.base_name();
            font_ids.entry(name).or_insert_with(|| {
                Object::Reference(doc.add_object(dictionary! {
                    "Type" => "Font",
                    "Subtype" => "Type1",
                    "BaseFont" => name,
                    "Encoding" => "WinAnsiEncoding",
                }))
            });
        }
    }

    let mut font_dict = lopdf::Dictionary::new();
    for (index, (name, id)) in font_ids.iter().enumerate() {
        let _ = name;
        font_dict.set(format!("F{index}"), id.clone());
    }
    let font_key: BTreeMap<&str, String> = font_ids
        .keys()
        .enumerate()
        .map(|(index, name)| (*name, format!("F{index}")))
        .collect();

    let resources_id = doc.add_object(dictionary! { "Font" => font_dict });

    let mut page_ids: Vec<Object> = Vec::new();
    for (index, size) in pages.iter().enumerate() {
        let empty: Vec<PlacedLine> = Vec::new();
        let lines = lines_per_page.get(index).unwrap_or(&empty);
        let content = page_content(size, lines, &font_key)?;
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

fn round6(value: f64) -> f64 {
    (value * 1e6).round() / 1e6
}

fn page_content(
    size: &PageSize,
    lines: &[PlacedLine],
    font_key: &BTreeMap<&str, String>,
) -> Result<Vec<u8>, PdfError> {
    let mut operations: Vec<Operation> = Vec::new();

    for line in lines {
        let encoded = encode_winansi(&line.text)?;
        if encoded.is_empty() {
            continue;
        }
        let name = font_key
            .get(line.font.base_name())
            .cloned()
            .unwrap_or_else(|| "F0".to_string());

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
            vec![Object::Name(name.into_bytes()), Object::Real(line.size_pt as f32)],
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

        operations.push(Operation::new("Tj", vec![Object::String(
            encoded,
            lopdf::StringFormat::Literal,
        )]));
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
            font: Font::Helvetica,
            rotation_deg: 0.0,
            colour: (0.0, 0.0, 0.0),
        }
    }

    #[test]
    fn writes_a_readable_pdf_at_the_right_size() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("d.pdf");
        let a4 = PageSize::new(210.0, 297.0);

        write_delta(&path, &[a4], &[vec![line("Approved")]], "test").unwrap();

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

        write_delta(&path, &[a4, a4], &[vec![line("x")], vec![]], "test").unwrap();

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

        write_delta(&path, &sizes, &[vec![], vec![], vec![]], "t").unwrap();

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
            // The advice must not name a way out that does not exist.
            assert!(!message.contains("--font-file"), "{message}");
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
