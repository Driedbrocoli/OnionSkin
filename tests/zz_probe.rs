use onionskin::barcode::{code128, qr};

#[test]
fn boundaries_and_nasty_inputs() {
    // Code 128 right at its limit and one past it.
    for n in [1usize, 79, 80, 81] {
        let text = "A".repeat(n);
        let got = code128::encode(&text);
        println!(
            "code128 {n} chars -> {}",
            if got.is_ok() { "ok" } else { "refused" }
        );
    }
    // Text with every awkward character.
    for text in [" ", "  ", "\t", "a\nb", "é", "\u{7f}", "~"] {
        let got = code128::encode(text);
        println!(
            "code128 {:?} -> {}",
            text,
            got.map(|s| s.width)
                .map_err(|e| e.to_string())
                .map_or_else(|e| e, |w| format!("{w} modules"))
        );
    }
    // QR at each mode boundary and each capacity edge.
    for (name, text) in [
        ("one digit", "1".to_string()),
        ("one letter", "A".to_string()),
        ("one byte", "é".to_string()),
        ("a space", " ".to_string()),
        ("newline", "a\nb".to_string()),
        ("nul", "\u{0}".to_string()),
        ("max-ish", "8".repeat(2953)),
        ("one past", "8".repeat(7090)),
    ] {
        let got = qr::encode(&text, qr::Ecc::Low);
        println!(
            "qr {name} -> {}",
            got.map(|s| format!("{} modules", s.width))
                .unwrap_or_else(|e| e.to_string())
        );
    }
}

#[test]
fn duplex_with_extreme_paper() {
    use onionskin::duplex::*;
    use onionskin::geometry::PageSize;
    for page in [
        PageSize::new(1.0, 1.0),
        PageSize::new(2000.0, 2000.0),
        PageSize::new(0.0, 0.0),
    ] {
        let turned = turn_a_placement(20.0, 40.0, 0.0, page, Feed::TurnedAround);
        println!("duplex {page:?} -> {turned:?}");
    }
    println!("sheets_for(usize::MAX) = {}", sheets_for(usize::MAX));
    println!(
        "page_of(usize::MAX/2, Back) = {}",
        page_of(usize::MAX / 2 - 1, Side::Back)
    );
}
