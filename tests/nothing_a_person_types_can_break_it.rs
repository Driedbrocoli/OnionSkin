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
