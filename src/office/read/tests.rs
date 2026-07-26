//! Reading documents that something else wrote.
//!
//! The tests beside each reader use files built by hand, which proves the
//! parsing and proves nothing about the real thing. These take a document
//! through LibreOffice first, so what is read is a file written by a word
//! processor rather than one written by the test — the two are not the same,
//! and the differences are exactly where a reader goes wrong.
//!
//! They skip themselves where LibreOffice is not installed, which is most
//! machines and, pointedly, the machines this feature exists for.

use super::*;

/// A document with one of everything, as HTML for LibreOffice to import.
const A_REAL_DOCUMENT: &str = "<html><body>\
     <h1>Quarterly report</h1>\
     <p>The <b>bold</b> claim and the <i>quieter</i> one, set in a paragraph \
     long enough that it has to break across more than a single line when it \
     is finally set on a page of A4 with ordinary margins round it.</p>\
     <h2>What was found</h2>\
     <ul><li>The first finding</li><li>The second finding</li></ul>\
     <ol><li>Step one</li><li>Step two</li></ol>\
     <table border=\"1\"><tr><td>Widgets</td><td>4471</td></tr>\
     <tr><td>Sprockets</td><td>12</td></tr></table>\
     <p style=\"text-align:center\">Signed, the auditor</p>\
     </body></html>";

/// Convert a document with LibreOffice, and hand back the bytes.
///
/// Both filters are named rather than inferred, and the reason is a trap worth
/// writing down: LibreOffice opens an HTML file as a *Writer/Web* document,
/// which has no Word export at all. Left to itself it produces nothing, says
/// so on standard error, and exits successfully — so the test skips, passes,
/// and proves nothing. Naming the input filter opens it as an ordinary Writer
/// document, and naming the output filter says which of the several Word
/// formats is wanted.
fn written_by_libreoffice(html: &str, into: &str) -> Option<Vec<u8>> {
    let soffice = crate::render::find_soffice()?;
    let dir = tempfile::tempdir().ok()?;
    let source = dir.path().join("source.html");
    std::fs::write(&source, html).ok()?;

    let filter = match into {
        "docx" => "docx:MS Word 2007 XML",
        "odt" => "odt:writer8",
        other => other,
    };

    // A profile of its own, because LibreOffice will not have two headless
    // instances sharing one — and these tests convert the same document twice.
    let out = std::process::Command::new(&soffice)
        .arg(format!(
            "-env:UserInstallation=file://{}",
            dir.path().join("profile").display()
        ))
        .args([
            "--headless",
            "--norestore",
            "--nolockcheck",
            "--infilter=HTML (StarWriter)",
            "--convert-to",
            filter,
        ])
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
    let produced = dir.path().join(format!("source.{into}"));
    if !produced.is_file() {
        // Exiting successfully having written nothing is LibreOffice's way of
        // reporting a missing filter, and a silent skip here would hide it.
        eprintln!(
            "LibreOffice wrote no .{into}: {}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        return None;
    }
    std::fs::read(produced).ok()
}

/// Everything a sheet says, with runs of whitespace squeezed to one space so
/// that a test is about the words rather than the formatting.
fn words(sheet: &Sheet) -> String {
    sheet
        .text()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Every phrase, in the order they appear, or the first one that is missing.
fn in_order(text: &str, phrases: &[&str]) -> Result<(), String> {
    let mut from = 0usize;
    for phrase in phrases {
        match text[from..].find(phrase) {
            Some(at) => from += at + phrase.len(),
            None => {
                return Err(format!(
                    "{phrase:?} is missing, or comes before what should precede it"
                ))
            }
        }
    }
    Ok(())
}

#[test]
fn reads_a_word_document_libreoffice_wrote() {
    let Some(bytes) = written_by_libreoffice(A_REAL_DOCUMENT, "docx") else {
        eprintln!("no LibreOffice on this machine; skipping");
        return;
    };
    let sheet = docx::read(&bytes).expect("a real .docx should open");
    let text = words(&sheet);

    in_order(
        &text,
        &[
            "Quarterly report",
            "bold",
            "quieter",
            "What was found",
            "The first finding",
            "The second finding",
            "Step one",
            "Step two",
            "Widgets",
            "4471",
            "Sprockets",
            "Signed, the auditor",
        ],
    )
    .unwrap_or_else(|missing| panic!("{missing}\nwhat was read: {text}"));

    // The table has to arrive as a table, or a two-column list of figures
    // comes out as a column of loose words.
    let tables = sheet
        .blocks
        .iter()
        .filter(|block| matches!(block, Block::Table(_)))
        .count();
    assert_eq!(tables, 1, "the table did not survive: {:?}", sheet.blocks);
}

#[test]
fn reads_an_opendocument_libreoffice_wrote() {
    let Some(bytes) = written_by_libreoffice(A_REAL_DOCUMENT, "odt") else {
        eprintln!("no LibreOffice on this machine; skipping");
        return;
    };
    let sheet = odt::read(&bytes).expect("a real .odt should open");
    let text = words(&sheet);

    in_order(
        &text,
        &[
            "Quarterly report",
            "bold",
            "quieter",
            "What was found",
            "The first finding",
            "The second finding",
            "Step one",
            "Step two",
            "Widgets",
            "4471",
            "Sprockets",
            "Signed, the auditor",
        ],
    )
    .unwrap_or_else(|missing| panic!("{missing}\nwhat was read: {text}"));

    let tables = sheet
        .blocks
        .iter()
        .filter(|block| matches!(block, Block::Table(_)))
        .count();
    assert_eq!(tables, 1, "the table did not survive: {:?}", sheet.blocks);
}

#[test]
fn the_two_formats_of_the_same_document_read_the_same() {
    // LibreOffice writes the same document twice, once each way. Onionskin has
    // two readers for them, and if the two disagree at least one is wrong.
    let (Some(word), Some(open)) = (
        written_by_libreoffice(A_REAL_DOCUMENT, "docx"),
        written_by_libreoffice(A_REAL_DOCUMENT, "odt"),
    ) else {
        eprintln!("no LibreOffice on this machine; skipping");
        return;
    };
    let from_word = words(&docx::read(&word).unwrap());
    let from_open = words(&odt::read(&open).unwrap());

    // Not byte for byte: the two formats number their lists differently — one
    // writes the marker into the file and the other leaves it to the reader —
    // so what is compared is the prose.
    for phrase in [
        "Quarterly report",
        "What was found",
        "The first finding",
        "Widgets",
        "Signed, the auditor",
    ] {
        assert!(from_word.contains(phrase), "missing from .docx: {phrase}");
        assert!(from_open.contains(phrase), "missing from .odt: {phrase}");
    }
}

#[test]
fn a_real_document_becomes_a_pdf_that_opens() {
    let Some(bytes) = written_by_libreoffice(A_REAL_DOCUMENT, "docx") else {
        eprintln!("no LibreOffice on this machine; skipping");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("report.docx");
    std::fs::write(&source, &bytes).unwrap();
    let into = dir.path().join("report.pdf");

    let notes = to_pdf(&source, &into).expect("it should convert");
    let pdf = lopdf::Document::load(&into).expect("what came out is not a PDF");
    assert!(!pdf.get_pages().is_empty());

    // Whatever it could not do, it has to have said so in words rather than
    // leaving somebody to notice.
    for note in &notes {
        assert!(
            note.ends_with('.') && note.chars().next().unwrap_or(' ').is_uppercase(),
            "a note should be a sentence: {note:?}"
        );
    }
}

/// A `.docx` holding these paragraphs, built without a word processor so that
/// the test runs on the machines this whole feature exists for.
fn a_word_document(paragraphs: &[&str]) -> Vec<u8> {
    let body: String = paragraphs
        .iter()
        .map(|text| format!("<w:p><w:r><w:t>{text}</w:t></w:r></w:p>"))
        .collect();
    let document = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\
         <w:document xmlns:w=\"http://schemas.openxmlformats.org/wordprocessingml/2006/main\">\
         <w:body>{body}<w:sectPr><w:pgSz w:w=\"11906\" w:h=\"16838\"/>\
         <w:pgMar w:top=\"1134\" w:right=\"1134\" w:bottom=\"1134\" w:left=\"1134\"/>\
         </w:sectPr></w:body></w:document>"
    );
    crate::package::zip(&[crate::package::Entry::file(
        "word/document.xml",
        document.into_bytes(),
    )])
}

#[test]
fn a_word_added_to_a_word_document_is_the_only_thing_in_the_delta() {
    // The whole point of the program, on a machine with no word processor on
    // it: two Word documents differing by one line, and a delta holding that
    // line and nothing else.
    if crate::render::engine().is_err() {
        eprintln!("no PDF renderer on this machine; skipping");
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let before = dir.path().join("note.docx");
    let after = dir.path().join("note-v2.docx");
    std::fs::write(&before, a_word_document(&["Purchase order 4471", ""])).unwrap();
    std::fs::write(
        &after,
        a_word_document(&["Purchase order 4471", "Approved by the auditor"]),
    )
    .unwrap();

    let first = dir.path().join("before.pdf");
    let second = dir.path().join("after.pdf");
    to_pdf(&before, &first).expect("the original should open");
    to_pdf(&after, &second).expect("the edited copy should open");

    let output = dir.path().join("delta.pdf");
    let outcome = crate::pipeline::run(
        &first,
        &second,
        &output,
        &crate::pipeline::Options {
            dpi: 150.0,
            ..Default::default()
        },
    )
    .expect("the delta should be made");

    assert!(
        !outcome.blocked(),
        "adding a line at the end moves nothing: {:?}",
        outcome
            .checks
            .iter()
            .map(|check| check.format())
            .collect::<Vec<_>>()
    );
    assert!(
        outcome.total_regions() > 0,
        "the added line should be in the delta"
    );
    assert_eq!(
        outcome.pages_with_additions(),
        vec![1],
        "and only on the page it was added to"
    );
    assert!(output.is_file());
}

#[test]
fn the_kinds_it_can_open_are_the_kinds_it_says_it_can() {
    for kind in READABLE {
        assert!(can_read(kind), "{kind} is listed and refused");
        assert!(
            can_read(&format!(".{kind}")),
            "a leading dot should not matter"
        );
        assert!(
            can_read(&kind.to_uppercase()),
            "a file from Windows may be shouted"
        );
        // Everything readable also has to be a kind the rest of the program
        // agrees is a document, or it will be refused before it gets here.
        assert!(
            crate::render::CONVERTIBLE.contains(kind),
            "{kind} can be read but is not on the list of what may be opened"
        );
    }
    assert!(!can_read("pdf"), "a PDF needs no reading");
    assert!(!can_read("rtf"), "that one is LibreOffice's job");
}

#[test]
fn it_can_read_the_placed_documents_it_writes_itself() {
    // A scan comes out of Onionskin as a page of frames, one per line, so that
    // each line opens where it was found on the paper. A reader that takes a
    // frame for a picture and skips it reads that whole document as blank —
    // which is what this one did until it was made to look inside.
    let mut document = crate::document::Document::blank(PageSize::new(210.0, 297.0), 1);
    for (index, text) in ["Invoice 4471", "Two hundred widgets"].iter().enumerate() {
        document
            .add(crate::document::Item {
                id: 0,
                page: 1,
                x_mm: 25.0,
                y_mm: 40.0 + index as f64 * 10.0,
                text: (*text).to_string(),
                size_pt: 12.0,
                font: "Helvetica".into(),
                width_mm: None,
                rotation_deg: 0.0,
                colour: "#000000".into(),
                leading: 1.2,
            })
            .unwrap();
    }

    for (format, describe) in [
        (crate::office::Format::Odt, "OpenDocument"),
        (crate::office::Format::Docx, "Word"),
    ] {
        let bytes = crate::office::write(&document, format, crate::office::Layout::Placed).unwrap();
        let sheet = match format {
            crate::office::Format::Odt => odt::read(&bytes),
            crate::office::Format::Docx => docx::read(&bytes),
        }
        .unwrap_or_else(|error| panic!("the {describe} it wrote would not open: {error}"));

        let text = words(&sheet);
        assert!(text.contains("Invoice 4471"), "{describe}: {text}");
        assert!(text.contains("Two hundred widgets"), "{describe}: {text}");
    }
}

#[test]
fn a_font_name_is_sorted_into_the_right_shape_of_type() {
    use Family::{Mono, Sans, Serif};
    for (name, expected) in [
        ("Arial", Sans),
        ("Calibri", Sans),
        ("Helvetica Neue", Sans),
        ("Comic Sans MS", Sans),
        ("Liberation Sans", Sans),
        ("Times New Roman", Serif),
        ("Georgia", Serif),
        ("Cambria", Serif),
        ("Liberation Serif", Serif),
        ("Noto Serif", Serif),
        ("Courier New", Mono),
        ("Consolas", Mono),
        ("DejaVu Sans Mono", Mono),
        // A script face whose name happens to start the same way as the
        // typewriter ones.
        ("Monotype Corsiva", Sans),
        ("something nobody has heard of", Sans),
    ] {
        assert_eq!(Family::of(name), expected, "{name}");
    }
}

#[test]
fn a_file_of_a_kind_it_does_not_know_is_refused_by_name() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("sheet.xlsx");
    std::fs::write(&path, b"not really a spreadsheet").unwrap();

    let error = read(&path).unwrap_err().to_string();
    assert!(error.contains(".xlsx"), "{error}");
}
