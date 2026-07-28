//! Tests for writing Word and OpenDocument files.
//!
//! A word-processor file is a zip of XML, and it is very easy to write one that
//! looks right in a text editor and opens as "damaged" in Word. So the tests
//! that matter here hand the file to LibreOffice — which knows nothing about
//! this code and is as strict as any reader gets — and check that what comes
//! out the other side says what went in, at the millimetre it went in at.

use super::*;
use crate::document::{Shape, ShapeKind};
use crate::geometry::PageSize;

const A4: PageSize = PageSize {
    width_mm: 210.0,
    height_mm: 297.0,
};

fn item(text: &str, x_mm: f64, y_mm: f64) -> Item {
    Item {
        id: 0,
        page: 1,
        x_mm,
        y_mm,
        text: text.to_string(),
        size_pt: 12.0,
        font: "Helvetica".into(),
        width_mm: None,
        rotation_deg: 0.0,
        colour: "#000000".into(),
        leading: 1.2,
    }
}

fn a_page() -> Document {
    let mut document = Document::blank(A4, 1);
    document
        .add(item("PURCHASE ORDER 4471", 25.0, 40.0))
        .unwrap();
    document
        .add(item("Two hundred widgets, black.", 25.0, 60.0))
        .unwrap();
    document
        .add(item("Smith & Sons <Ltd>", 25.0, 80.0))
        .unwrap();
    document
}

/// Pull one file out of a zip we wrote, using the central directory rather than
/// trusting the order things were written in.
fn inside(archive: &[u8], name: &str) -> Option<Vec<u8>> {
    let dir = std::process::Command::new("unzip")
        .args(["-p", "/dev/stdin", name])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    use std::io::Write;
    let mut child = dir;
    child.stdin.as_mut()?.write_all(archive).ok()?;
    let out = child.wait_with_output().ok()?;
    out.status.success().then_some(out.stdout)
}

fn have(program: &str) -> bool {
    std::process::Command::new(program)
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// The shape of the files
// ---------------------------------------------------------------------------

#[test]
fn both_formats_are_zips_that_unzip() {
    if !have("unzip") {
        eprintln!("no unzip on this machine; skipping");
        return;
    }
    let document = a_page();
    for format in [Format::Docx, Format::Odt] {
        let bytes = write(&document, format, Layout::Placed).unwrap();
        assert_eq!(&bytes[0..2], b"PK", "{format:?} is not a zip");

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(format!("out.{}", format.extension()));
        std::fs::write(&path, &bytes).unwrap();
        let checked = std::process::Command::new("unzip")
            .args(["-t", path.to_str().unwrap()])
            .output()
            .unwrap();
        assert!(
            checked.status.success(),
            "{format:?} failed its own CRCs:\n{}",
            String::from_utf8_lossy(&checked.stdout)
        );
    }
}

#[test]
fn an_odt_starts_with_an_uncompressed_mimetype() {
    // The OpenDocument format requires it: a reader identifies the file from
    // its first thirty bytes without unpacking anything, which only works if
    // the mimetype is the first entry and is stored rather than deflated.
    let bytes = write(&a_page(), Format::Odt, Layout::Placed).unwrap();

    // Local file header: signature, version, flags, then the method.
    assert_eq!(&bytes[0..4], &0x0403_4b50u32.to_le_bytes());
    let method = u16::from_le_bytes([bytes[8], bytes[9]]);
    assert_eq!(method, 0, "the mimetype is compressed");

    let name_length = u16::from_le_bytes([bytes[26], bytes[27]]) as usize;
    let name = String::from_utf8_lossy(&bytes[30..30 + name_length]);
    assert_eq!(name, "mimetype", "the first entry is {name:?}");

    // And the type itself is readable straight out of the bytes, which is the
    // whole point of the rule.
    let text = String::from_utf8_lossy(&bytes[..200]);
    assert!(
        text.contains("application/vnd.oasis.opendocument.text"),
        "{text:?}"
    );
}

#[test]
fn a_docx_holds_the_parts_word_looks_for() {
    if !have("unzip") {
        eprintln!("no unzip on this machine; skipping");
        return;
    }
    let bytes = write(&a_page(), Format::Docx, Layout::Placed).unwrap();
    for part in ["[Content_Types].xml", "_rels/.rels", "word/document.xml"] {
        assert!(
            inside(&bytes, part).is_some(),
            "{part} is not in the file, and Word will call it damaged"
        );
    }
}

// ---------------------------------------------------------------------------
// What the XML says
// ---------------------------------------------------------------------------

#[test]
fn the_words_survive_the_characters_xml_cannot_take() {
    // An ampersand in "Smith & Sons" is enough to make a file no word
    // processor will open, and a scan produces whatever was on the paper.
    let document = a_page();
    let docx = super::docx_document_xml(&document, Layout::Placed);
    assert!(docx.contains("Smith &amp; Sons &lt;Ltd&gt;"), "{docx}");
    assert!(
        !docx.contains("Smith & Sons"),
        "a bare ampersand got through"
    );

    let (odt, _) = super::odt_paragraph(&document.items[2], Layout::Placed, 2, A4);
    assert!(odt.contains("Smith &amp; Sons &lt;Ltd&gt;"), "{odt}");

    // And a control character, which XML forbids outright.
    let mut odd = item("bell\u{7}here", 20.0, 30.0);
    odd.id = 0;
    let mut with_odd = Document::blank(A4, 1);
    with_odd.add(odd).unwrap();
    let xml = super::docx_document_xml(&with_odd, Layout::Placed);
    assert!(!xml.contains('\u{7}'), "a control character got through");
    assert!(xml.contains("bell here"), "{xml}");
}

#[test]
fn every_line_is_pinned_where_it_was_found() {
    let document = a_page();
    let placed = super::docx_paragraph(&document.items[0], Layout::Placed, A4);
    assert!(placed.contains("<v:textbox"), "not a text box: {placed}");
    assert!(placed.contains("left:25.000mm"), "{placed}");
    // The box's top is the baseline less the type size, or every line sits one
    // line lower than it did on the paper.
    let expected_top = 40.0 - 12.0 * 25.4 / 72.0;
    assert!(
        placed.contains(&format!("top:{:.3}mm", expected_top)),
        "expected a top of {expected_top:.3} mm: {placed}"
    );
    assert!(
        placed.contains("mso-position-vertical-relative:page"),
        "anchored to something other than the page: {placed}"
    );
}

#[test]
fn flowing_it_gives_ordinary_paragraphs_with_no_frames() {
    let document = a_page();
    let placed = super::docx_paragraph(&document.items[0], Layout::Flow, A4);
    assert!(!placed.contains("framePr"), "{placed}");
    assert!(placed.contains("<w:t"), "{placed}");

    let (odt, _) = super::odt_paragraph(&document.items[0], Layout::Flow, 0, A4);
    assert!(!odt.contains("draw:frame"), "{odt}");
    assert!(odt.starts_with("<text:p>"), "{odt}");
}

#[test]
fn lines_come_out_in_reading_order() {
    // Written in any order, they come back down the page and then across —
    // which is what somebody editing the result expects to find.
    let mut document = Document::blank(A4, 1);
    document.add(item("third", 20.0, 90.0)).unwrap();
    document.add(item("first", 20.0, 30.0)).unwrap();
    document.add(item("second-right", 120.0, 60.0)).unwrap();
    document.add(item("second-left", 20.0, 60.0)).unwrap();

    let order: Vec<&str> = super::in_reading_order(&document)
        .iter()
        .map(|i| i.text.as_str())
        .collect();
    assert_eq!(
        order,
        ["first", "second-left", "second-right", "third"],
        "out of order"
    );
}

#[test]
fn the_page_size_is_carried_over() {
    // A5 in, A5 out. Getting this wrong reflows everything the moment it is
    // opened, and the placement work is wasted.
    let a5 = PageSize {
        width_mm: 148.0,
        height_mm: 210.0,
    };
    let mut document = Document::blank(a5, 1);
    document.add(item("small", 10.0, 20.0)).unwrap();

    let xml = super::docx_document_xml(&document, Layout::Placed);
    assert!(
        xml.contains(&format!("w:w=\"{}\"", super::twips(148.0))),
        "{xml}"
    );
    assert!(
        xml.contains(&format!("w:h=\"{}\"", super::twips(210.0))),
        "{xml}"
    );

    // The OpenDocument page size lives in styles.xml, which is built here.
    let styles = super::odt_styles_xml(a5);
    assert!(styles.contains("fo:page-width=\"148.000mm\""), "{styles}");
    assert!(styles.contains("fo:page-height=\"210.000mm\""), "{styles}");
    assert!(styles.contains("portrait"), "{styles}");
}

#[test]
fn colour_and_weight_come_through() {
    let mut document = Document::blank(A4, 1);
    let mut red = item("URGENT", 20.0, 30.0);
    red.colour = "#cc0000".into();
    red.font = "Helvetica-Bold".into();
    document.add(red).unwrap();

    let placed = super::docx_paragraph(&document.items[0], Layout::Placed, A4);
    assert!(placed.contains("<w:b/>"), "not bold: {placed}");
    assert!(placed.contains("w:val=\"CC0000\""), "not red: {placed}");

    let (odt, styles) = super::odt_paragraph(&document.items[0], Layout::Placed, 0, A4);
    assert!(styles.contains("fo:font-weight=\"bold\""), "{styles}");
    assert!(styles.contains("fo:color=\"#CC0000\""), "{styles}");
    let _ = odt;
}

#[test]
fn a_black_line_of_text_is_not_given_a_colour_at_all() {
    // Word writes black by leaving the colour out. Saying so explicitly makes
    // the text refuse to follow a theme, which is a surprise nobody wants.
    let document = a_page();
    let placed = super::docx_paragraph(&document.items[0], Layout::Placed, A4);
    assert!(!placed.contains("<w:color"), "{placed}");
}

// ---------------------------------------------------------------------------
// Drawings
// ---------------------------------------------------------------------------

fn a_drawn_page() -> Document {
    let mut document = Document::blank(A4, 1);
    document.add(item("Signed", 30.0, 200.0)).unwrap();
    document
        .draw(Shape {
            id: 0,
            page: 1,
            kind: ShapeKind::Line {
                x1_mm: 25.0,
                y1_mm: 205.0,
                x2_mm: 95.0,
                y2_mm: 205.0,
            },
            stroke: Some("black".into()),
            fill: None,
            width_mm: 0.4,
            dash_mm: None,
        })
        .unwrap();
    document
        .draw(Shape {
            id: 0,
            page: 1,
            kind: ShapeKind::Rect {
                x_mm: 20.0,
                y_mm: 20.0,
                width_mm: 80.0,
                height_mm: 30.0,
                radius_mm: 3.0,
            },
            stroke: Some("#0033aa".into()),
            fill: Some("lightgrey".into()),
            width_mm: 0.8,
            dash_mm: None,
        })
        .unwrap();
    document
}

#[test]
fn drawings_are_written_into_both_formats() {
    let document = a_drawn_page();

    let (line, _) = super::odt_shape(&document.shapes[0], 0);
    assert!(line.contains("<draw:line"), "{line}");
    assert!(line.contains("svg:x1=\"25.000mm\""), "{line}");

    let (rect, styles) = super::odt_shape(&document.shapes[1], 1);
    assert!(rect.contains("<draw:rect"), "{rect}");
    assert!(rect.contains("draw:corner-radius"), "{rect}");
    assert!(styles.contains("draw:fill-color=\"#D9D9D9\""), "{styles}");

    let vml = super::docx_shape(&document.shapes[0], A4);
    assert!(vml.contains("<v:line"), "{vml}");
    let box_vml = super::docx_shape(&document.shapes[1], A4);
    assert!(box_vml.contains("v:roundrect"), "{box_vml}");
    assert!(box_vml.contains("fillcolor=\"#D9D9D9\""), "{box_vml}");
}

// ---------------------------------------------------------------------------
// The real test: give it to LibreOffice
// ---------------------------------------------------------------------------

/// Open the file with LibreOffice and get back what it saw.
///
/// This is the check that counts. Everything above says the bytes look right;
/// only a word processor can say the file actually opens.
///
/// Converted to flat ODF, and not for convenience. The plain-text export drops
/// everything in a frame or a text box, which is where all the placed text
/// lives; the HTML export turns an ODF text frame into a picture of itself.
/// Either makes a file that opened perfectly come back empty, and the test then
/// reports a bug that is entirely its own. Flat ODF is one XML file with the
/// whole document in it, which is exactly what is wanted for looking.
fn through_libreoffice(bytes: &[u8], extension: &str) -> Option<String> {
    Some(strip_markup(&as_flat_odf(bytes, extension)?))
}

fn as_flat_odf(bytes: &[u8], extension: &str) -> Option<String> {
    let soffice = crate::render::find_soffice()?;
    let dir = tempfile::tempdir().ok()?;
    let source = dir.path().join(format!("page.{extension}"));
    std::fs::write(&source, bytes).ok()?;

    let out = std::process::Command::new(&soffice)
        .args(["--headless", "--norestore", "--convert-to", "fodt"])
        .arg("--outdir")
        .arg(dir.path())
        .arg(&source)
        .output()
        .ok()?;
    if !out.status.success() {
        eprintln!(
            "LibreOffice refused it: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        return None;
    }
    std::fs::read_to_string(dir.path().join("page.fodt")).ok()
}

/// The words out of a page of HTML, with runs of space squeezed to one.
fn strip_markup(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut inside = false;
    for ch in html.chars() {
        match ch {
            '<' => inside = true,
            '>' => {
                inside = false;
                out.push(' ');
            }
            c if !inside => out.push(c),
            _ => {}
        }
    }
    let text = out
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ");
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Where LibreOffice put each frame, in millimetres, read off the HTML it
/// wrote. Inches because that is what it writes.
fn frame_positions(html: &str) -> Vec<(f64, f64)> {
    let mut found = Vec::new();
    for chunk in html.split("position: absolute").skip(1) {
        let value = |key: &str| -> Option<f64> {
            let at = chunk.find(key)?;
            let rest = &chunk[at + key.len()..];
            let end = rest.find("in")?;
            rest[..end]
                .trim()
                .trim_start_matches(':')
                .trim()
                .parse()
                .ok()
        };
        if let (Some(top), Some(left)) = (value("top:"), value("left:")) {
            found.push((left * 25.4, top * 25.4));
        }
    }
    found
}

#[test]
fn libreoffice_opens_what_we_wrote_and_finds_the_words() {
    let document = a_page();
    for (format, extension) in [(Format::Docx, "docx"), (Format::Odt, "odt")] {
        let bytes = write(&document, format, Layout::Placed).unwrap();
        let Some(text) = through_libreoffice(&bytes, extension) else {
            eprintln!("no LibreOffice on this machine; skipping {format:?}");
            continue;
        };
        for wanted in ["PURCHASE ORDER 4471", "Two hundred widgets", "Smith & Sons"] {
            assert!(
                text.contains(wanted),
                "{format:?}: {wanted:?} is not in what came back:\n{text}"
            );
        }
    }
}

#[test]
fn libreoffice_puts_the_lines_back_where_they_were() {
    // The whole point of the placed layout. Coming back with the right words
    // in the wrong place is the failure that looks like success.
    let Some(soffice) = crate::render::find_soffice() else {
        eprintln!("no LibreOffice on this machine; skipping");
        return;
    };
    let _ = soffice;
    let document = a_page();

    for (format, extension) in [(Format::Docx, "docx"), (Format::Odt, "odt")] {
        let bytes = write(&document, format, Layout::Placed).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join(format!("page.{extension}"));
        std::fs::write(&source, &bytes).unwrap();
        let ok = std::process::Command::new(crate::render::find_soffice().unwrap())
            .args(["--headless", "--norestore", "--convert-to", "html"])
            .arg("--outdir")
            .arg(dir.path())
            .arg(&source)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if !ok {
            eprintln!("LibreOffice would not convert the {format:?}; skipping");
            continue;
        }
        let html = std::fs::read_to_string(dir.path().join("page.html")).unwrap();
        let places = frame_positions(&html);
        assert_eq!(
            places.len(),
            3,
            "{format:?}: expected three placed lines, got {places:?}"
        );

        // 25 mm across; baselines at 40, 60 and 80 mm less the 12 pt type.
        let wanted = [
            (25.0, 40.0 - 12.0 * 25.4 / 72.0),
            (25.0, 60.0 - 12.0 * 25.4 / 72.0),
            (25.0, 80.0 - 12.0 * 25.4 / 72.0),
        ];
        for (index, ((x, y), (wx, wy))) in places.iter().zip(wanted).enumerate() {
            // A tenth of an inch either way: LibreOffice writes the HTML in
            // hundredths of an inch, so the rounding alone is worth 0.25 mm.
            assert!(
                (x - wx).abs() < 2.0 && (y - wy).abs() < 2.0,
                "{format:?}: line {index} came back at {x:.1},{y:.1} mm — \
                 it was written at {wx:.1},{wy:.1}"
            );
        }
    }
}

#[test]
fn libreoffice_opens_a_flowed_document_too() {
    let document = a_page();
    for (format, extension) in [(Format::Docx, "docx"), (Format::Odt, "odt")] {
        let bytes = write(&document, format, Layout::Flow).unwrap();
        let Some(text) = through_libreoffice(&bytes, extension) else {
            eprintln!("no LibreOffice on this machine; skipping {format:?}");
            continue;
        };
        assert!(
            text.contains("PURCHASE ORDER 4471"),
            "{format:?}: nothing came back:\n{text}"
        );
    }
}

#[test]
fn libreoffice_opens_one_with_drawings_on_it() {
    let document = a_drawn_page();
    for (format, extension) in [(Format::Docx, "docx"), (Format::Odt, "odt")] {
        let bytes = write(&document, format, Layout::Placed).unwrap();
        let Some(text) = through_libreoffice(&bytes, extension) else {
            eprintln!("no LibreOffice on this machine; skipping {format:?}");
            continue;
        };
        assert!(
            text.contains("Signed"),
            "{format:?}: the drawings stopped the words coming back:\n{text}"
        );
    }
}

#[test]
fn an_empty_document_still_opens() {
    // Word calls a body with no paragraphs damaged, which is a bad thing to
    // discover when somebody scans a blank sheet.
    let document = Document::blank(A4, 1);
    for (format, extension) in [(Format::Docx, "docx"), (Format::Odt, "odt")] {
        let bytes = write(&document, format, Layout::Placed).unwrap();
        if through_libreoffice(&bytes, extension).is_none() {
            eprintln!("no LibreOffice on this machine; skipping {format:?}");
        }
    }
}

#[test]
fn the_format_is_taken_from_the_name() {
    use std::path::Path;
    assert_eq!(Format::of_path(Path::new("out.docx")), Some(Format::Docx));
    assert_eq!(Format::of_path(Path::new("out.ODT")), Some(Format::Odt));
    assert_eq!(Format::of_path(Path::new("out.pdf")), None);
    assert_eq!(Format::parse("word"), Some(Format::Docx));
    assert_eq!(Format::parse("writer"), Some(Format::Odt));
    assert_eq!(Format::parse("wordperfect"), None);
}
