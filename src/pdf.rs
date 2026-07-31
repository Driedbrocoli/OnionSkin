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

use crate::font::{EmbeddedFont, Outlines};
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

mod metrics;

/// How wide a run of text is in one of the built-in fonts, in millimetres.
///
/// Exact rather than estimated: these are the widths every PDF reader uses for
/// these fonts, so a line measured here breaks in the same place on the page.
/// A character the font cannot write counts as nothing, because it will not be
/// written — [`encode_winansi`] refuses the line before it reaches paper.
pub fn builtin_width_mm(font: Font, text: &str, size_pt: f64) -> f64 {
    let widths = font.widths();
    let thousandths: u32 = text
        .chars()
        .map(|ch| match ch {
            '\t' => ' ',
            other => other,
        })
        .filter_map(winansi_byte)
        .map(|byte| widths[byte as usize] as u32)
        .sum();
    thousandths as f64 / 1000.0 * size_pt * 25.4 / 72.0
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

/// A picture, positioned in page space (mm from the top-left).
///
/// `y_mm` is the *top* edge, not a baseline: a picture has no baseline, and
/// somebody placing a signature is thinking about the box it fills rather
/// than a line it sits on.
#[derive(Debug, Clone, PartialEq)]
pub struct PlacedImage {
    pub picture: crate::picture::Picture,
    pub x_mm: f64,
    pub y_mm: f64,
    pub width_mm: f64,
    pub height_mm: f64,
    /// Turned clockwise on the page, about its top-left corner.
    pub rotation_deg: f64,
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

/// A shape, positioned in page space (mm from the top-left, y downwards).
///
/// Everything here is measured in millimetres on the paper, like the rest of
/// Onionskin, and turned into PDF points at the last moment. A drawing put at
/// 30 mm from the top lands 30 mm from the top of the sheet whatever the paper.
#[derive(Debug, Clone, PartialEq)]
pub struct PlacedShape {
    pub drawing: Drawing,
    /// The outline's colour, or `None` to leave the shape unoutlined.
    pub stroke: Option<(f64, f64, f64)>,
    /// The inside's colour, or `None` to leave the shape hollow.
    pub fill: Option<(f64, f64, f64)>,
    /// How thick the outline is. Under about 0.2 mm a laser printer starts to
    /// drop parts of it, which is why nothing here defaults thinner.
    pub width_mm: f64,
    /// Dash pattern: how long a dash, how long the gap, both in millimetres.
    pub dash_mm: Option<(f64, f64)>,
}

/// What to draw.
#[derive(Debug, Clone, PartialEq)]
pub enum Drawing {
    Line {
        from: (f64, f64),
        to: (f64, f64),
    },
    /// `radius_mm` rounds the corners; zero leaves them square.
    Rect {
        x_mm: f64,
        y_mm: f64,
        width_mm: f64,
        height_mm: f64,
        radius_mm: f64,
    },
    Ellipse {
        centre: (f64, f64),
        radius_x_mm: f64,
        radius_y_mm: f64,
    },
    /// A run of points, joined in order. Closed makes it a polygon.
    Path {
        points: Vec<(f64, f64)>,
        closed: bool,
    },
}

impl PlacedShape {
    /// A hairline outline in black, which is what most drawing on a page is.
    pub fn outline(drawing: Drawing) -> PlacedShape {
        PlacedShape {
            drawing,
            stroke: Some((0.0, 0.0, 0.0)),
            fill: None,
            width_mm: 0.35,
            dash_mm: None,
        }
    }
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
    write_page_content(path, pages, lines_per_page, &[], title, embedded)
}

/// The same, with drawings as well as words.
///
/// Shapes are drawn before the text on each page, so a filled box behind a
/// label does not cover the label. Anyone who wants it the other way round can
/// say so by ordering the pages differently; putting the words on top is the
/// answer that is right nearly always.
pub fn write_page_content(
    path: &Path,
    pages: &[PageSize],
    lines_per_page: &[Vec<PlacedLine>],
    shapes_per_page: &[Vec<PlacedShape>],
    title: &str,
    embedded: Option<&EmbeddedFont>,
) -> Result<(), PdfError> {
    write_page_content_with_pictures(
        path,
        pages,
        lines_per_page,
        shapes_per_page,
        &[],
        title,
        embedded,
    )
}

/// The same, with pictures as well as words and drawings.
#[allow(clippy::too_many_arguments)]
pub fn write_page_content_with_pictures(
    path: &Path,
    pages: &[PageSize],
    lines_per_page: &[Vec<PlacedLine>],
    shapes_per_page: &[Vec<PlacedShape>],
    images_per_page: &[Vec<PlacedImage>],
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

    // One PDF object per distinct picture, however many pages use it. A logo
    // on all two hundred sheets of a batch is carried once, not two hundred
    // times — which is the difference between a sensible file and one nobody
    // can email.
    // Matched on a fingerprint rather than by comparing the pictures
    // themselves. Two hundred sheets carrying the same five-megabyte logo
    // would otherwise be two hundred five-megabyte comparisons, and two
    // hundred *different* pictures would be twenty thousand of them.
    let mut pictures: BTreeMap<u64, String> = BTreeMap::new();
    let mut xobjects = lopdf::Dictionary::new();
    for images in images_per_page {
        for image in images {
            let mark = fingerprint(&image.picture);
            if pictures.contains_key(&mark) {
                continue;
            }
            let name = format!("Im{}", pictures.len());
            let id = add_picture(&mut doc, &image.picture);
            xobjects.set(name.clone(), Object::Reference(id));
            pictures.insert(mark, name);
        }
    }
    let picture_key = |picture: &crate::picture::Picture| -> String {
        pictures
            .get(&fingerprint(picture))
            .cloned()
            .unwrap_or_else(|| "Im0".to_string())
    };

    // No fonts, no `/Font` — rather than an empty dictionary that says nothing
    // and means nothing. It matters for one caller in particular: `redact`
    // writes a document that must have no way of showing text in it, and
    // somebody auditing that file will search it for `/Font`. An empty entry
    // is a true answer to a question nobody asked and a frightening one to the
    // person asking it.
    let mut resources = lopdf::Dictionary::new();
    if !font_dict.is_empty() {
        resources.set("Font", font_dict);
    }
    if !xobjects.is_empty() {
        resources.set("XObject", xobjects);
    }
    let resources_id = doc.add_object(resources);

    let mut page_ids: Vec<Object> = Vec::new();
    for (index, size) in pages.iter().enumerate() {
        let empty: Vec<PlacedLine> = Vec::new();
        let lines = lines_per_page.get(index).unwrap_or(&empty);
        let no_glyphs: Vec<Option<Vec<crate::font::Glyph>>> = Vec::new();
        let glyphs = shaped.get(index).unwrap_or(&no_glyphs);
        let no_drawings: Vec<PlacedShape> = Vec::new();
        let drawings = shapes_per_page.get(index).unwrap_or(&no_drawings);
        let no_pictures: Vec<PlacedImage> = Vec::new();
        let page_pictures = images_per_page.get(index).unwrap_or(&no_pictures);
        let named: Vec<(String, &PlacedImage)> = page_pictures
            .iter()
            .map(|image| (picture_key(&image.picture), image))
            .collect();
        let content = page_content(
            size,
            lines,
            glyphs,
            drawings,
            &named,
            &font_key,
            EMBEDDED_KEY,
        )?;
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

    // The two outline formats are held differently, and swapping them produces
    // a file that reads as valid and prints nothing. TrueType goes in as a
    // plain font programme; PostScript-flavoured fonts — most .otf, and the
    // faces Word leans on — go in whole, as OpenType.
    let (stream_key, subtype, cid_subtype) = match font.outlines() {
        Outlines::TrueType => ("FontFile2", None, "CIDFontType2"),
        Outlines::PostScript => ("FontFile3", Some("OpenType"), "CIDFontType0"),
    };

    let mut stream_dict = dictionary! { "Length1" => program.len() as i64 };
    if let Some(subtype) = subtype {
        // Length1 has no meaning for FontFile3, and /Subtype names the format.
        stream_dict.remove(b"Length1");
        stream_dict.set("Subtype", Object::Name(subtype.as_bytes().to_vec()));
    }
    let mut stream = Stream::new(stream_dict, program.to_vec());
    // Compressing the font programme is what keeps a delta sane: the file is
    // often several megabytes and the printer has to receive all of it.
    let _ = stream.compress();
    let file_id = doc.add_object(stream);

    let (x0, y0, x1, y1) = font.bbox();
    let mut descriptor = dictionary! {
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
    };
    descriptor.set(stream_key, file_id);
    let descriptor_id = doc.add_object(descriptor);

    // Widths for the glyphs actually drawn, one entry each.
    let mut widths: Vec<Object> = Vec::new();
    for (glyph, advance) in used_glyphs {
        widths.push(Object::Integer(*glyph as i64));
        widths.push(Object::Array(vec![Object::Real(*advance as f32)]));
    }

    let mut cid_font = dictionary! {
        "Type" => "Font",
        "Subtype" => Object::Name(cid_subtype.as_bytes().to_vec()),
        "BaseFont" => Object::Name(font.name.clone().into_bytes()),
        "CIDSystemInfo" => dictionary! {
            "Registry" => Object::string_literal("Adobe"),
            "Ordering" => Object::string_literal("Identity"),
            "Supplement" => 0_i64,
        },
        "FontDescriptor" => descriptor_id,
        "DW" => 1000_i64,
        "W" => Object::Array(widths),
    };
    if font.outlines() == Outlines::TrueType {
        // Only a Type2 CID font maps CIDs to glyphs; a Type0 one is addressed
        // by glyph already, and the key is not allowed there.
        cid_font.set("CIDToGIDMap", Object::Name(b"Identity".to_vec()));
    }
    let cid_id = doc.add_object(cid_font);

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

fn real(value: f64) -> Object {
    Object::Real(round6(value) as f32)
}

/// Set a colour, in the fewest components that say it.
///
/// A grey written as three equal numbers is a colour as far as a printer is
/// concerned, and some will run the colour heads for it, or refuse the job on a
/// machine that is out of cyan. Written as one number it is unambiguously black
/// ink. Every shade of grey — including black itself — therefore goes out on
/// the greyscale operator, and only actual colour uses RGB.
fn colour_operation(colour: (f64, f64, f64), stroking: bool) -> Operation {
    let (r, g, b) = colour;
    if (r - g).abs() < 1e-9 && (g - b).abs() < 1e-9 {
        Operation::new(if stroking { "G" } else { "g" }, vec![real(r)])
    } else {
        Operation::new(
            if stroking { "RG" } else { "rg" },
            vec![real(r), real(g), real(b)],
        )
    }
}

/// How far along a Bézier control point sits to make a quarter of a circle.
///
/// PDF has no arc operator, so every curve is a cubic Bézier, and a circle
/// cannot be drawn by one exactly. This is the constant that makes four of them
/// agree with a circle everywhere except a few parts in ten thousand, which on
/// paper is a good deal finer than the toner.
const KAPPA: f64 = 0.552_284_749_831;

/// The operators that draw one shape.
///
/// Write a picture into the document, and hand back the object to draw it by.
///
/// A JPEG goes in as its own bytes with `DCTDecode`, which is PDF asking the
/// reader to do the decoding it already knows how to do. Anything else goes
/// in as plain samples, deflated — the only shape that can carry a PNG's
/// transparency.
///
/// Transparency becomes an `/SMask`: a second, greyscale picture the same
/// size where white shows the picture and black shows the paper. Without it a
/// signature saved on a see-through background prints inside a white box, and
/// the box covers the line it is meant to be sitting on.
fn add_picture(doc: &mut Document, picture: &crate::picture::Picture) -> lopdf::ObjectId {
    use crate::picture::Picture;
    match picture {
        Picture::Jpeg {
            bytes,
            width,
            height,
            grey,
        } => doc.add_object(Stream::new(
            dictionary! {
                "Type" => "XObject",
                "Subtype" => "Image",
                "Width" => *width as i64,
                "Height" => *height as i64,
                "ColorSpace" => if *grey { "DeviceGray" } else { "DeviceRGB" },
                "BitsPerComponent" => 8,
                "Filter" => "DCTDecode",
            },
            bytes.clone(),
        )),
        Picture::Samples {
            width,
            height,
            rgb,
            alpha,
        } => {
            let mask = alpha.as_ref().map(|alpha| {
                doc.add_object(Stream::new(
                    dictionary! {
                        "Type" => "XObject",
                        "Subtype" => "Image",
                        "Width" => *width as i64,
                        "Height" => *height as i64,
                        "ColorSpace" => "DeviceGray",
                        "BitsPerComponent" => 8,
                        "Filter" => "FlateDecode",
                    },
                    deflate(alpha),
                ))
            });
            let mut entries = dictionary! {
                "Type" => "XObject",
                "Subtype" => "Image",
                "Width" => *width as i64,
                "Height" => *height as i64,
                "ColorSpace" => "DeviceRGB",
                "BitsPerComponent" => 8,
                "Filter" => "FlateDecode",
            };
            if let Some(mask) = mask {
                entries.set("SMask", Object::Reference(mask));
            }
            doc.add_object(Stream::new(entries, deflate(rgb)))
        }
    }
}

/// A number that stands for a picture, so two of them can be told apart
/// without comparing every byte twice.
///
/// The size goes in as well as the pixels, so two pictures that happen to
/// hash alike still have to differ in shape to collide — and a collision
/// would only mean one picture drawn where another was meant, never a broken
/// file. `DefaultHasher` is in the standard library, so this costs no
/// dependency.
fn fingerprint(picture: &crate::picture::Picture) -> u64 {
    use std::hash::Hasher;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    hasher.write_u32(picture.width());
    hasher.write_u32(picture.height());
    match picture {
        crate::picture::Picture::Jpeg { bytes, .. } => {
            hasher.write_u8(1);
            hasher.write(bytes);
        }
        crate::picture::Picture::Samples { rgb, alpha, .. } => {
            hasher.write_u8(2);
            hasher.write(rgb);
            if let Some(alpha) = alpha {
                hasher.write(alpha);
            }
        }
    }
    hasher.finish()
}

/// Deflate, which is what `FlateDecode` means.
fn deflate(bytes: &[u8]) -> Vec<u8> {
    use flate2::write::ZlibEncoder;
    use std::io::Write;
    let mut encoder = ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    // Writing to a Vec cannot fail, and neither can finishing it.
    let _ = encoder.write_all(bytes);
    encoder.finish().unwrap_or_default()
}

/// Where a picture goes and how big, as PDF operators.
///
/// PDF draws every image into the unit square and lets the matrix say where
/// that square lands, so the width and height *are* the matrix. Page space is
/// y-down from the top-left and PDF is y-up from the bottom, so the top edge
/// given here becomes the bottom edge of the square down there.
fn image_operations(size: &PageSize, name: &str, image: &PlacedImage) -> Vec<Operation> {
    let w = mm_to_pt(image.width_mm);
    let h = mm_to_pt(image.height_mm);
    let x = mm_to_pt(image.x_mm);
    // The picture hangs down from its top edge, so its bottom is that much
    // lower — and in PDF's y-up world, that is where the unit square starts.
    let y = size.height_pt() - mm_to_pt(image.y_mm) - h;

    let mut operations = vec![Operation::new("q", vec![])];
    if image.rotation_deg.abs() > 1e-9 {
        // Turned about its top-left corner, which is the corner somebody
        // placed. Page-space clockwise is counter-clockwise in y-up PDF.
        let theta = (-image.rotation_deg).to_radians();
        let (sin_t, cos_t) = theta.sin_cos();
        let top = size.height_pt() - mm_to_pt(image.y_mm);
        operations.push(Operation::new(
            "cm",
            vec![
                Object::Real(cos_t as f32),
                Object::Real(sin_t as f32),
                Object::Real(-sin_t as f32),
                Object::Real(cos_t as f32),
                Object::Real(x as f32),
                Object::Real(top as f32),
            ],
        ));
        // Inside the turned frame the picture starts at the origin and hangs
        // downwards, so its square begins one height below.
        operations.push(Operation::new(
            "cm",
            vec![
                Object::Real(w as f32),
                Object::Real(0.0),
                Object::Real(0.0),
                Object::Real(h as f32),
                Object::Real(0.0),
                Object::Real(-h as f32),
            ],
        ));
    } else {
        operations.push(Operation::new(
            "cm",
            vec![
                Object::Real(w as f32),
                Object::Real(0.0),
                Object::Real(0.0),
                Object::Real(h as f32),
                Object::Real(x as f32),
                Object::Real(y as f32),
            ],
        ));
    }
    operations.push(Operation::new(
        "Do",
        vec![Object::Name(name.as_bytes().to_vec())],
    ));
    operations.push(Operation::new("Q", vec![]));
    operations
}

/// Page space is y-down from the top-left and PDF is y-up from the bottom, so
/// every y is turned over here — once, in one place, rather than at each call.
fn shape_operations(size: &PageSize, shape: &PlacedShape) -> Vec<Operation> {
    let mut ops: Vec<Operation> = Vec::new();
    // Nothing to see: no outline and no fill would emit a path that draws
    // nothing, which is a waste of bytes and confusing in a content stream.
    if shape.stroke.is_none() && shape.fill.is_none() {
        return ops;
    }

    ops.push(Operation::new("q", vec![]));
    if let Some(colour) = shape.fill {
        ops.push(colour_operation(colour, false));
    }
    if let Some(colour) = shape.stroke {
        ops.push(colour_operation(colour, true));
        ops.push(Operation::new(
            "w",
            vec![real(mm_to_pt(shape.width_mm.max(0.0)))],
        ));
        // Round ends and joins. A drawn line with square ends overshoots its
        // own corner by half its width, which is visible on anything thick.
        ops.push(Operation::new("J", vec![Object::Integer(1)]));
        ops.push(Operation::new("j", vec![Object::Integer(1)]));
        if let Some((on, off)) = shape.dash_mm {
            ops.push(Operation::new(
                "d",
                vec![
                    Object::Array(vec![real(mm_to_pt(on)), real(mm_to_pt(off))]),
                    Object::Integer(0),
                ],
            ));
        }
    }

    let mut path = PathBuilder::new(size);
    match &shape.drawing {
        Drawing::Line { from, to } => {
            path.move_to(from.0, from.1);
            path.line_to(to.0, to.1);
        }
        Drawing::Rect {
            x_mm,
            y_mm,
            width_mm,
            height_mm,
            radius_mm,
        } => {
            // A rectangle given from any corner: negative sizes are somebody
            // dragging up and to the left, and refusing that would be silly.
            let left = x_mm.min(x_mm + width_mm);
            let top = y_mm.min(y_mm + height_mm);
            let w = width_mm.abs();
            let h = height_mm.abs();
            let r = radius_mm.min(w / 2.0).min(h / 2.0).max(0.0);
            if r <= 0.0 {
                path.rect(left, top, w, h);
            } else {
                path.rounded_rect(left, top, w, h, r);
            }
        }
        Drawing::Ellipse {
            centre,
            radius_x_mm,
            radius_y_mm,
        } => path.ellipse(*centre, radius_x_mm.abs(), radius_y_mm.abs()),
        Drawing::Path { points, closed } => {
            let Some(first) = points.first() else {
                return Vec::new();
            };
            path.move_to(first.0, first.1);
            for point in &points[1..] {
                path.line_to(point.0, point.1);
            }
            if *closed {
                path.close();
            }
        }
    }
    ops.extend(path.ops);

    // A closed shape can be filled; a line cannot, whatever it was asked for.
    let fillable = !matches!(shape.drawing, Drawing::Line { .. })
        && !matches!(shape.drawing, Drawing::Path { closed: false, .. });
    let paint = match (shape.fill.is_some() && fillable, shape.stroke.is_some()) {
        (true, true) => "B",
        (true, false) => "f",
        _ => "S",
    };
    ops.push(Operation::new(paint, vec![]));
    ops.push(Operation::new("Q", vec![]));
    ops
}

/// A path in page space, built as millimetre points and turned over once.
///
/// Written as a builder rather than a set of closures because every one of
/// these needs to append to the same list while reading the same coordinate
/// mapping, which is the one shape a closure cannot take.
struct PathBuilder {
    height_pt: f64,
    ops: Vec<Operation>,
}

impl PathBuilder {
    fn new(size: &PageSize) -> PathBuilder {
        PathBuilder {
            height_pt: size.height_pt(),
            ops: Vec::new(),
        }
    }

    /// Page space is y-down from the top-left; PDF is y-up from the bottom.
    fn at(&self, x_mm: f64, y_mm: f64) -> (Object, Object) {
        (real(mm_to_pt(x_mm)), real(self.height_pt - mm_to_pt(y_mm)))
    }

    fn move_to(&mut self, x: f64, y: f64) {
        let (px, py) = self.at(x, y);
        self.ops.push(Operation::new("m", vec![px, py]));
    }

    fn line_to(&mut self, x: f64, y: f64) {
        let (px, py) = self.at(x, y);
        self.ops.push(Operation::new("l", vec![px, py]));
    }

    fn curve_to(&mut self, c1: (f64, f64), c2: (f64, f64), end: (f64, f64)) {
        let (a, b) = self.at(c1.0, c1.1);
        let (c, d) = self.at(c2.0, c2.1);
        let (e, f) = self.at(end.0, end.1);
        self.ops.push(Operation::new("c", vec![a, b, c, d, e, f]));
    }

    fn close(&mut self) {
        self.ops.push(Operation::new("h", vec![]));
    }

    fn rect(&mut self, left: f64, top: f64, w: f64, h: f64) {
        let (px, py) = self.at(left, top + h);
        self.ops.push(Operation::new(
            "re",
            vec![px, py, real(mm_to_pt(w)), real(mm_to_pt(h))],
        ));
    }

    /// Four Bézier arcs, starting at the rightmost point.
    fn ellipse(&mut self, centre: (f64, f64), rx: f64, ry: f64) {
        let (cx, cy) = centre;
        let (ox, oy) = (rx * KAPPA, ry * KAPPA);
        self.move_to(cx + rx, cy);
        self.curve_to((cx + rx, cy + oy), (cx + ox, cy + ry), (cx, cy + ry));
        self.curve_to((cx - ox, cy + ry), (cx - rx, cy + oy), (cx - rx, cy));
        self.curve_to((cx - rx, cy - oy), (cx - ox, cy - ry), (cx, cy - ry));
        self.curve_to((cx + ox, cy - ry), (cx + rx, cy - oy), (cx + rx, cy));
        self.close();
    }

    /// A rectangle with its corners rounded off. `top` is the edge nearer the
    /// top of the paper.
    fn rounded_rect(&mut self, left: f64, top: f64, w: f64, h: f64, r: f64) {
        let right = left + w;
        let bottom = top + h;
        let k = r * KAPPA;

        self.move_to(left + r, top);
        self.line_to(right - r, top);
        self.curve_to((right - r + k, top), (right, top + r - k), (right, top + r));
        self.line_to(right, bottom - r);
        self.curve_to(
            (right, bottom - r + k),
            (right - r + k, bottom),
            (right - r, bottom),
        );
        self.line_to(left + r, bottom);
        self.curve_to(
            (left + r - k, bottom),
            (left, bottom - r + k),
            (left, bottom - r),
        );
        self.line_to(left, top + r);
        self.curve_to((left, top + r - k), (left + r - k, top), (left + r, top));
        self.close();
    }
}

#[allow(clippy::too_many_arguments)]
fn page_content(
    size: &PageSize,
    lines: &[PlacedLine],
    glyphs_per_line: &[Option<Vec<crate::font::Glyph>>],
    drawings: &[PlacedShape],
    images: &[(String, &PlacedImage)],
    font_key: &BTreeMap<&str, String>,
    embedded_key: &str,
) -> Result<Vec<u8>, PdfError> {
    let mut operations: Vec<Operation> = Vec::new();

    // Drawings first, so a label written over a filled box stays readable.
    for shape in drawings {
        operations.extend(shape_operations(size, shape));
    }

    // Then pictures, then words: a signature goes over the ruled line it sits
    // on, and anything written stays on top of everything.
    for (name, image) in images {
        operations.extend(image_operations(size, name, image));
    }

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
                let glyphs = glyphs_per_line
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
        operations.push(Operation::new("Tj", vec![Object::String(encoded, format)]));
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

    /// Write one line in an embedded font and hand back the loaded document.
    fn delta_with(path: &Path, font: &EmbeddedFont) -> Document {
        let mut placed = line("Approved 25 July");
        placed.font = LineFont::Embedded;
        write_delta(
            path,
            &[PageSize::new(210.0, 297.0)],
            &[vec![placed]],
            "t",
            Some(font),
        )
        .unwrap();
        Document::load(path).unwrap()
    }

    /// The only dictionary in the file with a `/Type /FontDescriptor`.
    fn descriptor(doc: &Document) -> lopdf::Dictionary {
        doc.objects
            .values()
            .filter_map(|object| object.as_dict().ok())
            .find(|dict| {
                dict.get(b"Type")
                    .and_then(|t| t.as_name())
                    .map(|name| name == b"FontDescriptor")
                    .unwrap_or(false)
            })
            .expect("no font descriptor was written")
            .clone()
    }

    /// The descendant CID font, which names the flavour of the programme.
    fn cid_font(doc: &Document) -> lopdf::Dictionary {
        doc.objects
            .values()
            .filter_map(|object| object.as_dict().ok())
            .find(|dict| {
                dict.get(b"Subtype")
                    .and_then(|t| t.as_name())
                    .map(|name| name.starts_with(b"CIDFontType"))
                    .unwrap_or(false)
            })
            .expect("no CID font was written")
            .clone()
    }

    fn name_of(dict: &lopdf::Dictionary, key: &[u8]) -> String {
        String::from_utf8_lossy(dict.get(key).unwrap().as_name().unwrap()).into_owned()
    }

    #[test]
    fn a_truetype_font_is_embedded_as_a_plain_programme() {
        let Some(path) = crate::font::tests::dejavu_path() else {
            return;
        };
        let font = EmbeddedFont::load(&path).unwrap();
        let dir = tempdir().unwrap();
        let doc = delta_with(&dir.path().join("d.pdf"), &font);

        let descriptor = descriptor(&doc);
        assert!(descriptor.has(b"FontFile2"), "{descriptor:?}");
        assert!(!descriptor.has(b"FontFile3"));

        let cid = cid_font(&doc);
        assert_eq!(name_of(&cid, b"Subtype"), "CIDFontType2");
        // A Type2 CID font is addressed by CID, so the map must be there.
        assert_eq!(name_of(&cid, b"CIDToGIDMap"), "Identity");

        // The programme itself carries Length1, which only FontFile2 wants.
        let file_id = descriptor
            .get(b"FontFile2")
            .unwrap()
            .as_reference()
            .unwrap();
        let stream = doc.get_object(file_id).unwrap().as_stream().unwrap();
        assert!(stream.dict.has(b"Length1"));
        assert!(!stream.dict.has(b"Subtype"));
    }

    #[test]
    fn a_postscript_font_is_embedded_whole_as_opentype() {
        // Getting this wrong is silent: a CFF font written as if it were
        // TrueType gives a file that opens fine and prints a blank page.
        let Some(path) = crate::font::tests::postscript_font() else {
            return;
        };
        let font = EmbeddedFont::load(&path).unwrap();
        let dir = tempdir().unwrap();
        let doc = delta_with(&dir.path().join("d.pdf"), &font);

        let descriptor = descriptor(&doc);
        assert!(descriptor.has(b"FontFile3"), "{descriptor:?}");
        assert!(!descriptor.has(b"FontFile2"));

        let cid = cid_font(&doc);
        assert_eq!(name_of(&cid, b"Subtype"), "CIDFontType0");
        // CIDToGIDMap has no meaning for a Type0 CID font and is not allowed.
        assert!(!cid.has(b"CIDToGIDMap"), "{cid:?}");

        let file_id = descriptor
            .get(b"FontFile3")
            .unwrap()
            .as_reference()
            .unwrap();
        let stream = doc.get_object(file_id).unwrap().as_stream().unwrap();
        assert_eq!(name_of(&stream.dict, b"Subtype"), "OpenType");
        // Length1 counts a TrueType programme's bytes and means nothing here.
        assert!(!stream.dict.has(b"Length1"), "{:?}", stream.dict);
    }

    #[test]
    fn either_flavour_carries_the_whole_font_programme() {
        let candidates = [
            crate::font::tests::dejavu_path(),
            crate::font::tests::postscript_font(),
        ];
        for path in candidates.into_iter().flatten() {
            let font = EmbeddedFont::load(&path).unwrap();
            let expected = font.program().len();
            let dir = tempdir().unwrap();
            let doc = delta_with(&dir.path().join("d.pdf"), &font);

            let descriptor = descriptor(&doc);
            let key: &[u8] = if descriptor.has(b"FontFile2") {
                b"FontFile2"
            } else {
                b"FontFile3"
            };
            let file_id = descriptor.get(key).unwrap().as_reference().unwrap();
            let stream = doc.get_object(file_id).unwrap().as_stream().unwrap();
            let program = stream.decompressed_content().unwrap();
            assert_eq!(
                program.len(),
                expected,
                "{} arrived truncated",
                path.display()
            );
        }
    }

    /// Two operator lists draw the same thing if every operator matches with
    /// identical operands, in order — used to prove that two different ways
    /// of describing one rectangle produce the same path.
    fn same_operations(a: &[Operation], b: &[Operation]) -> bool {
        a.len() == b.len()
            && a.iter()
                .zip(b)
                .all(|(x, y)| x.operator == y.operator && x.operands == y.operands)
    }

    /// The paint operator a shape ends on: it sits just before the closing
    /// "Q" that every non-empty shape's operators finish with.
    fn paint_operator(shape: &PlacedShape) -> String {
        let a4 = PageSize::new(210.0, 297.0);
        let ops = shape_operations(&a4, shape);
        ops[ops.len() - 2].operator.clone()
    }

    #[test]
    fn a_point_near_the_top_of_the_page_lands_near_the_top_of_pdf_space() {
        // Page space counts millimetres down from the top of the sheet; PDF
        // counts points up from the bottom. Get the flip wrong here and every
        // shape prints upside down relative to the text sitting on the same
        // page.
        let a4 = PageSize::new(210.0, 297.0);
        let path = PathBuilder::new(&a4);
        let (_, y) = path.at(20.0, 20.0);
        let got = y.as_float().unwrap() as f64;
        let expected = mm_to_pt(297.0 - 20.0);
        assert!(
            (got - expected).abs() < 1e-3,
            "expected {expected} pt for y=20mm on a 297mm-tall page, got {got}"
        );
    }

    #[test]
    fn a_grey_is_written_on_the_greyscale_operator_not_as_three_numbers() {
        // A grey written as three equal numbers is still a colour as far as
        // a printer is concerned: some run the colour heads for it, or
        // refuse the job outright on a machine that is out of cyan. Every
        // shade where r==g==b, including pure black and pure white, must go
        // out on the one-number greyscale operator instead.
        for shade in [0.0, 1.0, 0.5, 0.25] {
            let fill_op = colour_operation((shade, shade, shade), false);
            assert_eq!(fill_op.operator, "g", "shade {shade}: {fill_op:?}");
            assert_eq!(fill_op.operands.len(), 1, "shade {shade}: {fill_op:?}");

            let stroke_op = colour_operation((shade, shade, shade), true);
            assert_eq!(stroke_op.operator, "G", "shade {shade}: {stroke_op:?}");
            assert_eq!(stroke_op.operands.len(), 1, "shade {shade}: {stroke_op:?}");
        }
    }

    #[test]
    fn an_actual_colour_is_written_as_three_numbers() {
        let fill_op = colour_operation((0.8, 0.1, 0.2), false);
        assert_eq!(fill_op.operator, "rg", "{fill_op:?}");
        assert_eq!(fill_op.operands.len(), 3, "{fill_op:?}");

        let stroke_op = colour_operation((0.8, 0.1, 0.2), true);
        assert_eq!(stroke_op.operator, "RG", "{stroke_op:?}");
        assert_eq!(stroke_op.operands.len(), 3, "{stroke_op:?}");
    }

    #[test]
    fn a_stroke_alone_paints_with_the_stroke_operator() {
        let shape = PlacedShape::outline(Drawing::Rect {
            x_mm: 10.0,
            y_mm: 10.0,
            width_mm: 20.0,
            height_mm: 30.0,
            radius_mm: 0.0,
        });
        assert_eq!(paint_operator(&shape), "S", "{shape:?}");
    }

    #[test]
    fn a_fill_alone_on_a_closed_shape_paints_with_the_fill_operator() {
        let shape = PlacedShape {
            drawing: Drawing::Rect {
                x_mm: 10.0,
                y_mm: 10.0,
                width_mm: 20.0,
                height_mm: 30.0,
                radius_mm: 0.0,
            },
            stroke: None,
            fill: Some((0.2, 0.2, 0.2)),
            width_mm: 0.35,
            dash_mm: None,
        };
        assert_eq!(paint_operator(&shape), "f", "{shape:?}");
    }

    #[test]
    fn a_stroke_and_a_fill_together_paint_with_the_fill_and_stroke_operator() {
        let shape = PlacedShape {
            drawing: Drawing::Rect {
                x_mm: 10.0,
                y_mm: 10.0,
                width_mm: 20.0,
                height_mm: 30.0,
                radius_mm: 0.0,
            },
            stroke: Some((0.0, 0.0, 0.0)),
            fill: Some((1.0, 1.0, 1.0)),
            width_mm: 0.35,
            dash_mm: None,
        };
        assert_eq!(paint_operator(&shape), "B", "{shape:?}");
    }

    #[test]
    fn a_line_is_never_filled_even_when_a_fill_colour_is_given() {
        // A line has no inside. If a caller sets a fill as well as a stroke
        // — wrongly, but it will happen — the safe reading is still to
        // stroke it, not to silently "fill" a shape with no enclosed area.
        let shape = PlacedShape {
            drawing: Drawing::Line {
                from: (10.0, 10.0),
                to: (50.0, 10.0),
            },
            stroke: Some((0.0, 0.0, 0.0)),
            fill: Some((0.0, 0.0, 0.0)),
            width_mm: 0.35,
            dash_mm: None,
        };
        assert_eq!(paint_operator(&shape), "S", "{shape:?}");
    }

    #[test]
    fn an_open_path_is_never_filled_even_when_a_fill_colour_is_given() {
        // Same reasoning as an open line: a path that never closes has no
        // enclosed area either, so it must still come out stroked.
        let shape = PlacedShape {
            drawing: Drawing::Path {
                points: vec![(10.0, 10.0), (30.0, 10.0), (30.0, 30.0)],
                closed: false,
            },
            stroke: Some((0.0, 0.0, 0.0)),
            fill: Some((0.0, 0.0, 0.0)),
            width_mm: 0.35,
            dash_mm: None,
        };
        assert_eq!(paint_operator(&shape), "S", "{shape:?}");
    }

    #[test]
    fn a_shape_with_no_stroke_and_no_fill_draws_nothing() {
        // Nothing asked for means nothing drawn. Anything else would put
        // bytes into the content stream for a shape nobody can see, and it
        // would be a live bug the moment something else gets placed there.
        let a4 = PageSize::new(210.0, 297.0);
        let shape = PlacedShape {
            drawing: Drawing::Rect {
                x_mm: 10.0,
                y_mm: 10.0,
                width_mm: 20.0,
                height_mm: 30.0,
                radius_mm: 0.0,
            },
            stroke: None,
            fill: None,
            width_mm: 0.35,
            dash_mm: None,
        };
        let ops = shape_operations(&a4, &shape);
        assert!(ops.is_empty(), "{ops:?}");
    }

    #[test]
    fn a_rectangle_dragged_up_and_left_matches_the_same_rectangle_from_its_other_corner() {
        // Negative width/height mean someone dragged the rectangle from its
        // bottom-right corner rather than its top-left one; the box that
        // ends up on the page must land in exactly the same place either
        // way.
        let a4 = PageSize::new(210.0, 297.0);
        let dragged = PlacedShape::outline(Drawing::Rect {
            x_mm: 50.0,
            y_mm: 60.0,
            width_mm: -20.0,
            height_mm: -30.0,
            radius_mm: 0.0,
        });
        let plain = PlacedShape::outline(Drawing::Rect {
            x_mm: 30.0,
            y_mm: 30.0,
            width_mm: 20.0,
            height_mm: 30.0,
            radius_mm: 0.0,
        });
        let dragged_ops = shape_operations(&a4, &dragged);
        let plain_ops = shape_operations(&a4, &plain);
        assert!(
            same_operations(&dragged_ops, &plain_ops),
            "dragged rect {dragged_ops:?} did not match plain rect {plain_ops:?}"
        );
    }

    #[test]
    fn an_ellipse_is_four_curves_between_a_move_and_a_close() {
        let a4 = PageSize::new(210.0, 297.0);
        let mut path = PathBuilder::new(&a4);
        path.ellipse((100.0, 100.0), 20.0, 15.0);
        let operators: Vec<&str> = path.ops.iter().map(|op| op.operator.as_str()).collect();
        assert_eq!(
            operators,
            vec!["m", "c", "c", "c", "c", "h"],
            "{operators:?}"
        );
    }

    #[test]
    fn the_first_ellipse_control_point_sits_at_the_kappa_offset_from_the_start() {
        // KAPPA is the constant that makes four cubic Béziers agree with a
        // circle; get the control point wrong and the "circle" prints
        // visibly lens-shaped or egg-shaped once ink is on paper.
        let a4 = PageSize::new(210.0, 297.0);
        let (cx, cy) = (100.0, 120.0);
        let r = 20.0;
        let mut path = PathBuilder::new(&a4);
        path.ellipse((cx, cy), r, r);

        // The curve leaves the start point (cx+r, cy) heading for the top of
        // the circle; its first control point must sit KAPPA of the radius
        // along that tangent.
        let expected = path.at(cx + r, cy + r * KAPPA);
        let first_curve = &path.ops[1];
        assert_eq!(first_curve.operator, "c", "{first_curve:?}");
        assert_eq!(
            first_curve.operands[0], expected.0,
            "control point x was {:?}, expected {:?}",
            first_curve.operands[0], expected.0
        );
        assert_eq!(
            first_curve.operands[1], expected.1,
            "control point y was {:?}, expected {:?}",
            first_curve.operands[1], expected.1
        );
    }

    #[test]
    fn a_rounded_rect_radius_larger_than_the_shape_is_clamped_not_broken() {
        // Somebody will type a corner radius bigger than the box without
        // thinking about it; the result must still be a sane closed path
        // rather than a self-intersecting or missing outline.
        let a4 = PageSize::new(210.0, 297.0);
        let huge = PlacedShape::outline(Drawing::Rect {
            x_mm: 10.0,
            y_mm: 10.0,
            width_mm: 20.0,
            height_mm: 10.0,
            radius_mm: 1000.0,
        });
        let clamped = PlacedShape::outline(Drawing::Rect {
            x_mm: 10.0,
            y_mm: 10.0,
            width_mm: 20.0,
            height_mm: 10.0,
            radius_mm: 5.0, // half of the shorter, 10mm side
        });
        let huge_ops = shape_operations(&a4, &huge);
        let clamped_ops = shape_operations(&a4, &clamped);
        assert!(
            same_operations(&huge_ops, &clamped_ops),
            "radius 1000mm gave {huge_ops:?}, expected it clamped to match {clamped_ops:?}"
        );
    }

    #[test]
    fn a_zero_radius_rect_uses_the_cheap_rectangle_operator_not_beziers() {
        // The plain "re" operator is one line of the content stream; four
        // Béziers for square corners is eight times the bytes for a printer
        // to parse for no visible difference on the page.
        let a4 = PageSize::new(210.0, 297.0);
        let shape = PlacedShape::outline(Drawing::Rect {
            x_mm: 10.0,
            y_mm: 10.0,
            width_mm: 20.0,
            height_mm: 30.0,
            radius_mm: 0.0,
        });
        let ops = shape_operations(&a4, &shape);
        let operators: Vec<&str> = ops.iter().map(|op| op.operator.as_str()).collect();
        assert!(operators.contains(&"re"), "{operators:?}");
        assert!(!operators.contains(&"c"), "{operators:?}");
    }

    #[test]
    fn a_dash_pattern_is_converted_to_points_not_left_in_millimetres() {
        let a4 = PageSize::new(210.0, 297.0);
        let shape = PlacedShape {
            drawing: Drawing::Line {
                from: (10.0, 10.0),
                to: (60.0, 10.0),
            },
            stroke: Some((0.0, 0.0, 0.0)),
            fill: None,
            width_mm: 0.5,
            dash_mm: Some((3.0, 1.5)),
        };
        let ops = shape_operations(&a4, &shape);
        let dash_op = ops
            .iter()
            .find(|op| op.operator == "d")
            .expect("no dash operator was emitted");
        let array = dash_op.operands[0]
            .as_array()
            .expect("dash operand is not an array");
        let on = array[0].as_float().unwrap() as f64;
        let off = array[1].as_float().unwrap() as f64;
        assert!(
            (on - mm_to_pt(3.0)).abs() < 1e-3,
            "expected on-length {} pt, got {on}",
            mm_to_pt(3.0)
        );
        assert!(
            (off - mm_to_pt(1.5)).abs() < 1e-3,
            "expected off-length {} pt, got {off}",
            mm_to_pt(1.5)
        );
    }

    #[test]
    fn the_line_width_is_written_in_points_not_millimetres() {
        // A 2mm line written as "2 w" would be the better part of an inch of
        // black on the finished page, since PDF widths are always in
        // user-space points, never in the page's own units.
        let a4 = PageSize::new(210.0, 297.0);
        let shape = PlacedShape {
            drawing: Drawing::Line {
                from: (10.0, 10.0),
                to: (60.0, 10.0),
            },
            stroke: Some((0.0, 0.0, 0.0)),
            fill: None,
            width_mm: 2.0,
            dash_mm: None,
        };
        let ops = shape_operations(&a4, &shape);
        let width_op = ops
            .iter()
            .find(|op| op.operator == "w")
            .expect("no width operator was emitted");
        let got = width_op.operands[0].as_float().unwrap() as f64;
        let expected = mm_to_pt(2.0);
        assert!(
            (got - expected).abs() < 1e-3,
            "expected {expected} pt, got {got}"
        );
    }

    #[test]
    fn an_empty_path_draws_nothing_and_does_not_panic() {
        // A path can arrive with no points if whatever built it was
        // cancelled partway through; this must be a no-op, not a crash that
        // takes an otherwise-finished print job down with it.
        let a4 = PageSize::new(210.0, 297.0);
        let shape = PlacedShape {
            drawing: Drawing::Path {
                points: vec![],
                closed: false,
            },
            stroke: Some((0.0, 0.0, 0.0)),
            fill: Some((0.0, 0.0, 0.0)),
            width_mm: 0.35,
            dash_mm: None,
        };
        let ops = shape_operations(&a4, &shape);
        assert!(ops.is_empty(), "{ops:?}");
    }

    #[test]
    fn write_page_content_with_shapes_produces_a_loadable_pdf() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("d.pdf");
        let a4 = PageSize::new(210.0, 297.0);

        let shapes = vec![
            PlacedShape::outline(Drawing::Rect {
                x_mm: 20.0,
                y_mm: 20.0,
                width_mm: 50.0,
                height_mm: 30.0,
                radius_mm: 3.0,
            }),
            PlacedShape {
                drawing: Drawing::Ellipse {
                    centre: (100.0, 150.0),
                    radius_x_mm: 15.0,
                    radius_y_mm: 10.0,
                },
                stroke: Some((0.0, 0.0, 0.0)),
                fill: Some((0.9, 0.2, 0.2)),
                width_mm: 0.5,
                dash_mm: Some((2.0, 1.0)),
            },
        ];

        write_page_content(
            &path,
            &[a4],
            &[vec![line("Approved")]],
            &[shapes],
            "t",
            None,
        )
        .unwrap();

        let doc = Document::load(&path).unwrap();
        let pages = doc.get_pages();
        assert_eq!(pages.len(), 1, "{pages:?}");
    }
}
