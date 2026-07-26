//! Word and OpenDocument files: writing them, and reading them.
//!
//! Reading a scan gives you letters at millimetre positions. That is exactly
//! what a delta needs and nothing like what a person needs when they say "I
//! want to edit it" — they mean in Word, or in LibreOffice Writer, with a
//! cursor. So the same page goes out as `.docx` or `.odt`. That is this file.
//!
//! Both formats are a zip of XML files, which is why they are written here
//! rather than shelled out to something: the zip writer already exists for the
//! packaging, the XML is a page and a half, and the alternative is making a
//! word processor a requirement for *producing* a document.
//!
//! The same reasoning, followed one step further, is why they are now *read*
//! here too. [`read`] opens a `.docx`, a `.odt` or plain text and sets it on
//! paper, with [`unzip`] and [`xml`] underneath — so LibreOffice is what
//! improves the result rather than what makes it possible.
//!
//! # Why frames, and not paragraphs
//!
//! The obvious way to write a scanned page out is one paragraph per line, and
//! it is the wrong way. Onionskin knows where every line sits to a fraction of
//! a millimetre; flowing it into paragraphs throws that away, and a form with
//! its boxes filled in comes out as a column of disconnected phrases. So each
//! line goes into a frame anchored to the page at the place it was found. Open
//! it and it looks like the paper, because it is where it was on the paper.
//!
//! The cost is that the result is a page of frames rather than flowing text.
//! For a scanned form, a letter or an invoice — which is what people scan —
//! that is what they wanted. `--flow` gives ordinary paragraphs to anyone who
//! would rather have them.

use crate::document::{Document, Item, Shape, ShapeKind};
use crate::geometry::PageSize;
use crate::package::{zip, Entry};

pub mod read;
pub mod unzip;
pub mod xml;

/// Which word-processor format to write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// Microsoft Word, and everything that reads it.
    Docx,
    /// OpenDocument Text: LibreOffice, OpenOffice, and Word since 2007.
    Odt,
}

impl Format {
    pub fn parse(text: &str) -> Option<Format> {
        match text.trim().to_ascii_lowercase().trim_start_matches('.') {
            "docx" | "word" | "doc" => Some(Format::Docx),
            "odt" | "odf" | "libreoffice" | "writer" => Some(Format::Odt),
            _ => None,
        }
    }

    /// Work it out from what the file is to be called.
    pub fn of_path(path: &std::path::Path) -> Option<Format> {
        path.extension()
            .and_then(|e| e.to_str())
            .and_then(Format::parse)
    }

    pub fn extension(&self) -> &'static str {
        match self {
            Format::Docx => "docx",
            Format::Odt => "odt",
        }
    }

    pub fn describe(&self) -> &'static str {
        match self {
            Format::Docx => "a Word document",
            Format::Odt => "an OpenDocument text",
        }
    }
}

/// How the page should be laid out in the result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Layout {
    /// Each line pinned where it was found on the paper.
    #[default]
    Placed,
    /// Ordinary paragraphs, one per line, in reading order.
    Flow,
}

// ---------------------------------------------------------------------------
// Units
// ---------------------------------------------------------------------------

/// Twentieths of a point, which is what Word measures in.
fn twips(mm: f64) -> i64 {
    (mm / 25.4 * 1440.0).round() as i64
}

/// Half-points, which is how Word writes a type size.
fn half_points(pt: f64) -> i64 {
    (pt * 2.0).round().max(2.0) as i64
}

fn mm(value: f64) -> String {
    format!("{:.3}mm", round3(value))
}

fn round3(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

/// XML has five characters it cannot take literally, and a scan can produce any
/// of them — an ampersand in "Smith & Sons" is enough to make a file that Word
/// refuses to open at all.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            // Control characters are not allowed in XML at all, and a scan
            // that produced one would otherwise write an unopenable file.
            c if (c as u32) < 0x20 && c != '\n' && c != '\t' => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

/// `#rrggbb` without the hash, which is how both formats write a colour.
fn hex_colour(text: &str) -> String {
    crate::document::parse_colour(text)
        .map(|(r, g, b)| {
            format!(
                "{:02X}{:02X}{:02X}",
                (r * 255.0).round() as u8,
                (g * 255.0).round() as u8,
                (b * 255.0).round() as u8
            )
        })
        .unwrap_or_else(|_| "000000".to_string())
}

/// The font name a word processor should use for an item.
fn face_name(item: &Item) -> &str {
    match item.font.to_ascii_lowercase().as_str() {
        "file" => "Arial",
        name if name.starts_with("times") => "Times New Roman",
        name if name.starts_with("courier") => "Courier New",
        _ => "Arial",
    }
}

fn is_bold(item: &Item) -> bool {
    item.font.to_ascii_lowercase().contains("bold")
}

fn is_italic(item: &Item) -> bool {
    let name = item.font.to_ascii_lowercase();
    name.contains("italic") || name.contains("oblique")
}

// ---------------------------------------------------------------------------
// The public door
// ---------------------------------------------------------------------------

/// Write a document out as `.docx` or `.odt`.
pub fn write(
    document: &Document,
    format: Format,
    layout: Layout,
) -> Result<Vec<u8>, std::io::Error> {
    Ok(match format {
        Format::Docx => docx(document, layout),
        Format::Odt => odt(document, layout),
    })
}

/// Turn what was read off a scan into a document, one line at a time.
///
/// Lives here rather than in the command that calls it because the browser
/// interface wants exactly the same thing, and two copies of "how big was that
/// type, roughly?" would drift apart the first time either was touched.
pub fn document_from_page(
    text: &crate::letters::PageText,
    page: PageSize,
) -> Result<Document, crate::document::DocumentError> {
    let mut document = Document::blank(page, 1);
    for line in &text.lines {
        let said = line.text_lossy();
        if said.trim().is_empty() {
            continue;
        }
        document.add(Item {
            id: 0,
            page: 1,
            x_mm: line.rect.x_mm,
            y_mm: line.baseline_mm,
            text: said,
            size_pt: type_size_of(line),
            font: "Helvetica".into(),
            width_mm: None,
            rotation_deg: 0.0,
            colour: "#000000".into(),
            leading: 1.2,
        })?;
    }
    Ok(document)
}

/// How big the type on a line was, judged from the ink.
///
/// The tall letters of a line stand about a cap height, and a cap height is
/// about seven tenths of the em in nearly every typeface. It will be a point
/// out here and there — which matters far less than every line coming back at
/// 11 pt when the heading was set in 24.
fn type_size_of(line: &crate::letters::TextLine) -> f64 {
    let tall_mm = line
        .letters()
        .map(|l| l.rect.height_mm)
        .fold(0.0f64, f64::max);
    let size_pt = (tall_mm / 0.7 * 72.0 / 25.4).clamp(5.0, 96.0);
    // To the nearest half point, because 11.03 pt in a word processor's size
    // box looks like something has gone wrong even when nothing has.
    (size_pt * 2.0).round() / 2.0
}

/// The items of a document in reading order: down the page, then across.
fn in_reading_order(document: &Document) -> Vec<&Item> {
    let mut items: Vec<&Item> = document.items.iter().collect();
    items.sort_by(|a, b| {
        (a.page, ordered(a.y_mm), ordered(a.x_mm)).cmp(&(b.page, ordered(b.y_mm), ordered(b.x_mm)))
    });
    items
}

/// A float that can be sorted, to the tenth of a millimetre. Two lines within a
/// tenth of a millimetre of each other are the same line as far as order goes,
/// and then what decides is which is further left.
fn ordered(value: f64) -> i64 {
    (value * 10.0).round() as i64
}

// ---------------------------------------------------------------------------
// Word
// ---------------------------------------------------------------------------

fn docx(document: &Document, layout: Layout) -> Vec<u8> {
    let content_types = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
         <Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">\
         <Default Extension=\"rels\" \
         ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>\
         <Default Extension=\"xml\" ContentType=\"application/xml\"/>\
         <Override PartName=\"/word/document.xml\" \
         ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/>\
         </Types>";

    let rels = "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
         <Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\">\
         <Relationship Id=\"rId1\" \
         Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument\" \
         Target=\"word/document.xml\"/></Relationships>";

    zip(&[
        Entry::file("[Content_Types].xml", content_types.as_bytes().to_vec()),
        Entry::file("_rels/.rels", rels.as_bytes().to_vec()),
        Entry::file(
            "word/document.xml",
            docx_document_xml(document, layout).into_bytes(),
        ),
    ])
}

/// `word/document.xml`: the whole of what Word will show.
fn docx_document_xml(document: &Document, layout: Layout) -> String {
    let page = document.page;
    let mut body = String::new();

    for item in in_reading_order(document) {
        body.push_str(&docx_paragraph(item, layout, page));
    }
    for shape in &document.shapes {
        body.push_str(&docx_shape(shape, page));
    }
    // Word wants at least one paragraph, and an empty body opens as a damaged
    // file rather than an empty one.
    if body.is_empty() {
        body.push_str("<w:p/>");
    }
    let section = docx_section(page);

    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\n\
         <w:document \
         xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\" \
         xmlns:v=\"urn:schemas-microsoft-com:vml\" \
         xmlns:w10=\"urn:schemas-microsoft-com:office:word\">\
         <w:body>{body}{section}</w:body></w:document>"
    )
}

/// The paper size, and no margins — everything is placed, so a margin would
/// only push it all inwards.
fn docx_section(page: PageSize) -> String {
    format!(
        "<w:sectPr><w:pgSz w:w=\"{}\" w:h=\"{}\"/>\
         <w:pgMar w:top=\"0\" w:right=\"0\" w:bottom=\"0\" w:left=\"0\" \
         w:header=\"0\" w:footer=\"0\" w:gutter=\"0\"/></w:sectPr>",
        twips(page.width_mm),
        twips(page.height_mm)
    )
}

fn docx_paragraph(item: &Item, layout: Layout, page: PageSize) -> String {
    let properties = "<w:spacing w:before=\"0\" w:after=\"0\"/>".to_string();

    let mut run_properties = format!(
        "<w:rFonts w:ascii=\"{face}\" w:hAnsi=\"{face}\" w:cs=\"{face}\"/>\
         <w:sz w:val=\"{size}\"/><w:szCs w:val=\"{size}\"/>",
        face = escape(face_name(item)),
        size = half_points(item.size_pt)
    );
    if is_bold(item) {
        run_properties.push_str("<w:b/>");
    }
    if is_italic(item) {
        run_properties.push_str("<w:i/>");
    }
    let colour = hex_colour(&item.colour);
    if colour != "000000" {
        run_properties.push_str(&format!("<w:color w:val=\"{colour}\"/>"));
    }

    // A line break inside an item is a line break in the paragraph, not a new
    // paragraph — the item is one piece of text and stays one.
    let mut runs = String::new();
    for (index, part) in item.text.split('\n').enumerate() {
        if index > 0 {
            runs.push_str("<w:br/>");
        }
        runs.push_str(&format!(
            "<w:t xml:space=\"preserve\">{}</w:t>",
            escape(part)
        ));
    }

    let paragraph = format!(
        "<w:p><w:pPr>{properties}<w:rPr>{run_properties}</w:rPr></w:pPr>\
         <w:r><w:rPr>{run_properties}</w:rPr>{runs}</w:r></w:p>"
    );
    if layout == Layout::Flow {
        return paragraph;
    }

    // A text box, anchored to the page at the millimetre the ink was found.
    //
    // The obvious tool is `w:framePr`, and it does not work: a run of framed
    // paragraphs is read as *one* frame taking the first paragraph's position,
    // so a page of twelve placed lines opened showing one. A text box is one
    // box per line and cannot be merged with its neighbour.
    //
    // `y_mm` is the baseline and the box's top is its top, so the type size
    // comes off — otherwise every line sits one line too low.
    let top = (item.y_mm - item.size_pt * 25.4 / 72.0).max(0.0);
    let lines = item.text.split('\n').count() as f64;
    let height = item.size_pt * 25.4 / 72.0 * item.leading.max(1.0) * lines + 1.0;
    let width = item
        .width_mm
        .unwrap_or_else(|| (page.width_mm - item.x_mm).max(10.0));

    format!(
        "<w:p><w:pPr><w:spacing w:before=\"0\" w:after=\"0\"/></w:pPr><w:r><w:pict>\
         <v:rect stroked=\"f\" filled=\"f\" style=\"position:absolute;\
         left:{left};top:{top};width:{width};height:{height};\
         mso-position-horizontal-relative:page;mso-position-vertical-relative:page;\
         mso-wrap-style:none;v-text-anchor:top\">\
         <v:textbox inset=\"0,0,0,0\" style=\"mso-fit-shape-to-text:t\">\
         <w:txbxContent>{paragraph}</w:txbxContent></v:textbox>\
         </v:rect></w:pict></w:r></w:p>",
        left = mm(item.x_mm),
        top = mm(top),
        width = mm(width),
        height = mm(height),
    )
}

/// A drawing, as VML — the old shape language, which both Word and LibreOffice
/// still read and which takes a tenth of the XML that DrawingML does.
fn docx_shape(shape: &Shape, page: PageSize) -> String {
    let (x0, y0, x1, y1) = shape.bounds();
    let stroke = match &shape.stroke {
        Some(colour) => format!(
            "stroked=\"t\" strokecolor=\"#{}\" strokeweight=\"{}mm\"",
            hex_colour(colour),
            round3(shape.width_mm)
        ),
        None => "stroked=\"f\"".to_string(),
    };
    let fill = match &shape.fill {
        Some(colour) => format!("filled=\"t\" fillcolor=\"#{}\"", hex_colour(colour)),
        None => "filled=\"f\"".to_string(),
    };
    let dash = match shape.dash_mm {
        Some(_) => "<v:stroke dashstyle=\"dash\"/>",
        None => "",
    };

    let body = match &shape.kind {
        ShapeKind::Line {
            x1_mm,
            y1_mm,
            x2_mm,
            y2_mm,
        } => format!(
            "<v:line from=\"{},{}\" to=\"{},{}\" {stroke} \
             style=\"position:absolute;mso-position-horizontal-relative:page;\
             mso-position-vertical-relative:page\">{dash}</v:line>",
            mm(*x1_mm),
            mm(*y1_mm),
            mm(*x2_mm),
            mm(*y2_mm)
        ),
        ShapeKind::Rect { radius_mm, .. } => {
            // VML rounds corners as a fraction of the shorter side, not as a
            // length, so the radius has to be turned into one.
            let shorter = (x1 - x0).min(y1 - y0).max(1e-6);
            let arc = (radius_mm / shorter).clamp(0.0, 0.5);
            let element = if *radius_mm > 0.0 {
                format!("<v:roundrect arcsize=\"{:.4}\"", arc)
            } else {
                "<v:rect".to_string()
            };
            format!(
                "{element} {stroke} {fill} style=\"position:absolute;\
                 left:{};top:{};width:{};height:{};\
                 mso-position-horizontal-relative:page;\
                 mso-position-vertical-relative:page\">{dash}</{}>",
                mm(x0),
                mm(y0),
                mm(x1 - x0),
                mm(y1 - y0),
                if *radius_mm > 0.0 { "v:roundrect" } else { "v:rect" }
            )
        }
        ShapeKind::Ellipse { .. } => format!(
            "<v:oval {stroke} {fill} style=\"position:absolute;\
             left:{};top:{};width:{};height:{};\
             mso-position-horizontal-relative:page;\
             mso-position-vertical-relative:page\">{dash}</v:oval>",
            mm(x0),
            mm(y0),
            mm(x1 - x0),
            mm(y1 - y0)
        ),
        ShapeKind::Path { points, closed } => {
            // VML paths are in the shape's own coordinate space, so the points
            // are scaled into a box of a thousand and the box placed on the
            // page. Every point moves together, so the drawing keeps its shape.
            let (w, h) = ((x1 - x0).max(1e-6), (y1 - y0).max(1e-6));
            let mut path = String::new();
            for (index, (px, py)) in points.iter().enumerate() {
                let cx = ((px - x0) / w * 1000.0).round() as i64;
                let cy = ((py - y0) / h * 1000.0).round() as i64;
                path.push_str(&format!(
                    "{}{cx},{cy}",
                    if index == 0 { "m " } else { " l " }
                ));
            }
            path.push_str(if *closed { " x e" } else { " e" });
            format!(
                "<v:shape {stroke} {fill} coordsize=\"1000,1000\" path=\"{path}\" \
                 style=\"position:absolute;left:{};top:{};width:{};height:{};\
                 mso-position-horizontal-relative:page;\
                 mso-position-vertical-relative:page\">{dash}</v:shape>",
                mm(x0),
                mm(y0),
                mm(w),
                mm(h)
            )
        }
    };
    let _ = page;
    format!("<w:p><w:pPr><w:spacing w:before=\"0\" w:after=\"0\"/></w:pPr><w:r><w:pict>{body}</w:pict></w:r></w:p>")
}

// ---------------------------------------------------------------------------
// OpenDocument
// ---------------------------------------------------------------------------

fn odt(document: &Document, layout: Layout) -> Vec<u8> {
    let page = document.page;
    let mut body = String::new();
    let mut styles = String::new();

    for (index, item) in in_reading_order(document).into_iter().enumerate() {
        let (text, style) = odt_paragraph(item, layout, index, page);
        body.push_str(&text);
        styles.push_str(&style);
    }
    for (index, shape) in document.shapes.iter().enumerate() {
        let (drawing, style) = odt_shape(shape, index);
        body.push_str(&drawing);
        styles.push_str(&style);
    }
    if body.is_empty() {
        body.push_str("<text:p/>");
    }

    let content = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <office:document-content \
         xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" \
         xmlns:style=\"urn:oasis:names:tc:opendocument:xmlns:style:1.0\" \
         xmlns:text=\"urn:oasis:names:tc:opendocument:xmlns:text:1.0\" \
         xmlns:draw=\"urn:oasis:names:tc:opendocument:xmlns:drawing:1.0\" \
         xmlns:fo=\"urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0\" \
         xmlns:svg=\"urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0\" \
         office:version=\"1.3\">\
         <office:automatic-styles>{styles}</office:automatic-styles>\
         <office:body><office:text>{body}</office:text></office:body>\
         </office:document-content>"
    );

    let manifest = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <manifest:manifest \
         xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\" \
         manifest:version=\"1.3\">\
         <manifest:file-entry manifest:full-path=\"/\" \
         manifest:media-type=\"application/vnd.oasis.opendocument.text\"/>\
         <manifest:file-entry manifest:full-path=\"content.xml\" \
         manifest:media-type=\"text/xml\"/>\
         <manifest:file-entry manifest:full-path=\"styles.xml\" \
         manifest:media-type=\"text/xml\"/>\
         </manifest:manifest>";

    // The mimetype comes first and uncompressed, which is how a reader knows
    // what the file is by looking at its first thirty bytes. It is short
    // enough that the zip writer stores it rather than deflating it — which is
    // what the format requires — and there is a test that says so.
    zip(&[
        Entry::file(
            "mimetype",
            b"application/vnd.oasis.opendocument.text".to_vec(),
        ),
        Entry::file("META-INF/manifest.xml", manifest.as_bytes().to_vec()),
        Entry::file("content.xml", content.into_bytes()),
        Entry::file("styles.xml", odt_styles_xml(page).into_bytes()),
    ])
}

/// `styles.xml`: the paper, and nothing else. Everything on the page carries
/// its own style, written beside it.
fn odt_styles_xml(page: PageSize) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <office:document-styles \
         xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" \
         xmlns:style=\"urn:oasis:names:tc:opendocument:xmlns:style:1.0\" \
         xmlns:fo=\"urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0\" \
         office:version=\"1.3\">\
         <office:automatic-styles>\
         <style:page-layout style:name=\"pm1\"><style:page-layout-properties \
         fo:page-width=\"{width}\" fo:page-height=\"{height}\" \
         fo:margin-top=\"0mm\" fo:margin-bottom=\"0mm\" \
         fo:margin-left=\"0mm\" fo:margin-right=\"0mm\" \
         style:print-orientation=\"{orientation}\"/></style:page-layout>\
         </office:automatic-styles>\
         <office:master-styles><style:master-page style:name=\"Standard\" \
         style:page-layout-name=\"pm1\"/></office:master-styles>\
         </office:document-styles>",
        width = mm(page.width_mm),
        height = mm(page.height_mm),
        orientation = if page.width_mm > page.height_mm {
            "landscape"
        } else {
            "portrait"
        }
    )
}

fn odt_paragraph(item: &Item, layout: Layout, index: usize, page: PageSize) -> (String, String) {
    let text_style = format!("T{index}");
    let mut properties = format!(
        "fo:font-family=\"{face}\" style:font-name=\"{face}\" fo:font-size=\"{size}pt\"",
        face = escape(face_name(item)),
        size = round3(item.size_pt)
    );
    if is_bold(item) {
        properties.push_str(" fo:font-weight=\"bold\"");
    }
    if is_italic(item) {
        properties.push_str(" fo:font-style=\"italic\"");
    }
    let colour = hex_colour(&item.colour);
    if colour != "000000" {
        properties.push_str(&format!(" fo:color=\"#{colour}\""));
    }

    let mut styles = format!(
        "<style:style style:name=\"{text_style}\" style:family=\"text\">\
         <style:text-properties {properties}/></style:style>"
    );

    // A line break inside the item stays inside the paragraph.
    let mut runs = String::new();
    for (line, part) in item.text.split('\n').enumerate() {
        if line > 0 {
            runs.push_str("<text:line-break/>");
        }
        runs.push_str(&format!(
            "<text:span text:style-name=\"{text_style}\">{}</text:span>",
            escape(part)
        ));
    }

    if layout == Layout::Flow {
        return (format!("<text:p>{runs}</text:p>"), styles);
    }

    let frame_style = format!("fr{index}");
    styles.push_str(&format!(
        "<style:style style:name=\"{frame_style}\" style:family=\"graphic\">\
         <style:graphic-properties fo:padding=\"0mm\" fo:border=\"none\" \
         style:vertical-pos=\"from-top\" style:vertical-rel=\"page\" \
         style:horizontal-pos=\"from-left\" style:horizontal-rel=\"page\" \
         draw:fill=\"none\" draw:stroke=\"none\" \
         fo:min-height=\"0mm\" draw:auto-grow-height=\"true\"/></style:style>"
    ));

    let top = (item.y_mm - item.size_pt * 25.4 / 72.0).max(0.0);
    let width = item
        .width_mm
        .unwrap_or_else(|| (page.width_mm - item.x_mm).max(10.0));
    let frame = format!(
        "<text:p><draw:frame draw:style-name=\"{frame_style}\" \
         text:anchor-type=\"page\" text:anchor-page-number=\"{page_number}\" \
         svg:x=\"{x}\" svg:y=\"{y}\" svg:width=\"{w}\" \
         draw:z-index=\"{index}\"><draw:text-box>\
         <text:p>{runs}</text:p></draw:text-box></draw:frame></text:p>",
        page_number = item.page.max(1),
        x = mm(item.x_mm),
        y = mm(top),
        w = mm(width)
    );
    (frame, styles)
}

fn odt_shape(shape: &Shape, index: usize) -> (String, String) {
    let name = format!("gr{index}");
    let stroke = match &shape.stroke {
        Some(colour) => format!(
            "draw:stroke=\"{}\" svg:stroke-width=\"{}\" svg:stroke-color=\"#{}\"",
            if shape.dash_mm.is_some() {
                "dash"
            } else {
                "solid"
            },
            mm(shape.width_mm),
            hex_colour(colour)
        ),
        None => "draw:stroke=\"none\"".to_string(),
    };
    let fill = match &shape.fill {
        Some(colour) => format!(
            "draw:fill=\"solid\" draw:fill-color=\"#{}\"",
            hex_colour(colour)
        ),
        None => "draw:fill=\"none\"".to_string(),
    };
    let styles = format!(
        "<style:style style:name=\"{name}\" style:family=\"graphic\">\
         <style:graphic-properties {stroke} {fill} \
         style:vertical-pos=\"from-top\" style:vertical-rel=\"page\" \
         style:horizontal-pos=\"from-left\" style:horizontal-rel=\"page\"/>\
         </style:style>"
    );

    let anchor = format!(
        "draw:style-name=\"{name}\" text:anchor-type=\"page\" \
         text:anchor-page-number=\"{}\" draw:z-index=\"{}\"",
        shape.page.max(1),
        1000 + index
    );
    let (x0, y0, x1, y1) = shape.bounds();

    let drawing = match &shape.kind {
        ShapeKind::Line {
            x1_mm,
            y1_mm,
            x2_mm,
            y2_mm,
        } => format!(
            "<draw:line {anchor} svg:x1=\"{}\" svg:y1=\"{}\" svg:x2=\"{}\" svg:y2=\"{}\"/>",
            mm(*x1_mm),
            mm(*y1_mm),
            mm(*x2_mm),
            mm(*y2_mm)
        ),
        ShapeKind::Rect {
            x_mm,
            y_mm,
            width_mm,
            height_mm,
            radius_mm,
        } => {
            let corner = if *radius_mm > 0.0 {
                format!(" draw:corner-radius=\"{}\"", mm(*radius_mm))
            } else {
                String::new()
            };
            format!(
                "<draw:rect {anchor} svg:x=\"{}\" svg:y=\"{}\" svg:width=\"{}\" \
                 svg:height=\"{}\"{corner}/>",
                mm(x_mm.min(x_mm + width_mm)),
                mm(y_mm.min(y_mm + height_mm)),
                mm(width_mm.abs()),
                mm(height_mm.abs())
            )
        }
        ShapeKind::Ellipse {
            radius_x_mm,
            radius_y_mm,
            ..
        } => format!(
            "<draw:ellipse {anchor} svg:x=\"{}\" svg:y=\"{}\" svg:width=\"{}\" \
             svg:height=\"{}\"/>",
            mm(x0),
            mm(y0),
            mm(radius_x_mm.abs() * 2.0),
            mm(radius_y_mm.abs() * 2.0)
        ),
        ShapeKind::Path { points, closed } => {
            // A polyline's points are in its own space, sized here to a
            // hundredth of a millimetre so nothing is lost to rounding.
            let (w, h) = ((x1 - x0).max(1e-6), (y1 - y0).max(1e-6));
            let scale = 100.0;
            let coords: Vec<String> = points
                .iter()
                .map(|(px, py)| {
                    format!(
                        "{},{}",
                        ((px - x0) * scale).round() as i64,
                        ((py - y0) * scale).round() as i64
                    )
                })
                .collect();
            let element = if *closed { "polygon" } else { "polyline" };
            format!(
                "<draw:{element} {anchor} svg:x=\"{}\" svg:y=\"{}\" svg:width=\"{}\" \
                 svg:height=\"{}\" svg:viewBox=\"0 0 {} {}\" draw:points=\"{}\"/>",
                mm(x0),
                mm(y0),
                mm(w),
                mm(h),
                (w * scale).round() as i64,
                (h * scale).round() as i64,
                coords.join(" ")
            )
        }
    };
    (drawing, styles)
}

#[cfg(test)]
mod tests;
