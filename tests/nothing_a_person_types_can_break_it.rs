//! Every parser that takes a string a person typed, given strings nobody would.
//!
//! A command line is an open door: whatever somebody types reaches a parser, and
//! a parser that panics turns a typing mistake into a stack trace. None of these
//! inputs is realistic and that is the point — the realistic ones are covered by
//! the tests beside the code, and this is for the rest.
//!
//! It is a sweep rather than a set of cases, so the list of nasty strings is
//! shared and every parser gets all of them. Adding a parser here costs one
//! line, which is the only way a check like this stays current.
use std::panic::catch_unwind;

fn nasty() -> Vec<String> {
    let mut out: Vec<String> = vec![
        "",
        " ",
        ",",
        ":",
        "::",
        ",,",
        ":,",
        "-",
        "--",
        ".",
        "e",
        "E",
        "nan",
        "NaN",
        "inf",
        "-inf",
        "Infinity",
        "1e400",
        "-1e400",
        "0x10",
        "1,",
        ",1",
        "1:",
        ":1",
        "1,2",
        "1,2:",
        ":1,2",
        "1,2:3",
        "a,b:c",
        "999999999999999999999999,1:x",
        "1,999999999999999999999999:x",
        "-0,-0:x",
        "1e-400,1e-400:x",
        "١٢٣,٤٥٦:x",
        "𝟙,𝟚:x",
        "x".repeat(10000).as_str(),
        "\u{0}",
        "\u{feff}",
        "a\nb",
        "a\tb",
        "/",
        "//",
        "\\",
        "..",
        "../..",
        "C:\\",
        "~",
        "%s",
        "{}",
        "{n}",
        "A=B=C",
        "A/below/below",
        "A/number/number",
        "/below",
        "=",
        "==",
    ]
    .into_iter()
    .map(str::to_string)
    .collect();
    out.push(format!("{},{}:x", f64::MAX, f64::MIN));
    out
}

macro_rules! never_panics {
    ($name:literal, $body:expr) => {{
        for text in nasty() {
            let t = text.clone();
            if catch_unwind(move || {
                let _ = $body(&t);
            })
            .is_err()
            {
                panic!("{} panicked on {:?}", $name, text);
            }
        }
    }};
}

#[test]
fn no_parser_panics_on_anything_a_person_could_type() {
    never_panics!("recipe::parse_placement", |t: &str| {
        onionskin::recipe::parse_placement(t)
    });
    never_panics!("recipe::parse_image", |t: &str| {
        onionskin::recipe::parse_image(t)
    });
    never_panics!("harvest::Field::parse", |t: &str| {
        onionskin::harvest::Field::parse(t)
    });
    never_panics!("geometry::parse_page", |t: &str| {
        onionskin::geometry::parse_page(t)
    });
    never_panics!("duplex::Feed::parse", |t: &str| {
        onionskin::duplex::Feed::parse(t)
    });
    never_panics!("qr::Ecc::parse", |t: &str| {
        onionskin::barcode::qr::Ecc::parse(t)
    });
    never_panics!("pdf::Font::parse", |t: &str| onionskin::pdf::Font::parse(t));
    never_panics!("document::parse_colour", |t: &str| {
        onionskin::document::parse_colour(t)
    });
    never_panics!("code128::encode", |t: &str| {
        onionskin::barcode::code128::encode(t)
    });
    never_panics!("qr::encode", |t: &str| onionskin::barcode::qr::encode(
        t,
        onionskin::barcode::qr::Ecc::Low
    ));
    never_panics!("rows::List::parse", |t: &str| onionskin::rows::List::parse(
        t,
        std::path::Path::new("list.csv")
    ));
    never_panics!("rows::fill", |t: &str| onionskin::rows::fill(
        t,
        &onionskin::jobs::values(
            &std::collections::BTreeMap::new(),
            onionskin::history::now()
        )
    ));
}

/// The readers that are handed a whole file rather than a typed string.
///
/// A spreadsheet is a zip full of XML, and a zip is a format with lengths and
/// offsets in it that point at other parts of itself. Every one of those is a
/// number somebody could have written down wrongly, or on purpose — so the
/// reader is handed archives that lie about their own shape and asked only to
/// come back.
#[test]
fn no_file_reader_panics_on_a_file_that_lies_about_itself() {
    let mut files: Vec<Vec<u8>> = vec![
        Vec::new(),
        b"PK".to_vec(),
        b"PK\x03\x04".to_vec(),
        // A local header promising more than follows it.
        b"PK\x03\x04\xff\xff\xff\xff\xff\xff\xff\xff".to_vec(),
        // An end-of-directory record claiming entries that are not there.
        b"PK\x05\x06\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff\xff"
            .to_vec(),
        b"PK\x05\x06\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00\x00"
            .to_vec(),
        (0u8..=255).collect(),
        b"%PDF-1.4 pretending to be a spreadsheet".to_vec(),
        b"<?xml version=\"1.0\"?><worksheet/>".to_vec(),
    ];
    files.push(vec![0u8; 100_000]);
    files.push(b"PK\x03\x04".iter().copied().cycle().take(50_000).collect());

    // A picture is a whole file too, and one people are handed by other people:
    // a signature scanned by somebody else, a logo out of an email.
    let dir = tempfile::tempdir().expect("somewhere to work");

    for bytes in &files {
        let one = bytes.clone();
        let picture = dir.path().join("picture.png");
        std::fs::write(&picture, bytes).expect("writing the file");
        if catch_unwind(move || {
            let _ = onionskin::sheets::is_a_spreadsheet(&one);
            let _ = onionskin::sheets::read(&one);
            let _ = onionskin::office::read::docx::read(&one);
            let _ = onionskin::office::read::odt::read(&one);
            let _ = onionskin::picture::load(&picture);
        })
        .is_err()
        {
            panic!(
                "a reader panicked on {} bytes starting {:?}",
                bytes.len(),
                &bytes[..bytes.len().min(8)]
            );
        }
    }

    // And the two that take text rather than bytes.
    let long = "<".repeat(50_000);
    for text in ["", "\u{0}", "<a><b></a>", long.as_str(), "\u{feff}x"] {
        let owned = text.to_string();
        if catch_unwind(move || {
            let _ = onionskin::office::read::odt::read_flat(&owned);
            let _ = onionskin::office::read::plain::read(&owned, "txt");
        })
        .is_err()
        {
            panic!(
                "a text reader panicked on {:?}",
                &text[..text.len().min(40)]
            );
        }
    }
}
