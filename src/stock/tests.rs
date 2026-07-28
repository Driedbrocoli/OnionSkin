use super::*;

use crate::geometry::PageSize;

fn paper_of(stock: &Stock) -> PageSize {
    crate::geometry::parse_page(stock.paper)
        .unwrap_or_else(|_| panic!("{} names a paper size nothing knows", stock.code))
}

/// The arithmetic every entry has to satisfy, and the reason this table can be
/// trusted at all.
///
/// Label stock is die-cut in the middle of the sheet: the margin on the left
/// equals the margin on the right, and the top equals the bottom. So the
/// measurements are not four independent numbers to be taken on faith — they
/// are a sum that comes out even, and a digit typed wrongly moves one margin
/// and not the other.
#[test]
fn every_stock_sits_squarely_on_its_paper() {
    for stock in KNOWN {
        let paper = paper_of(stock);
        let right = paper.width_mm - stock.margin_mm.0 - stock.across_mm();
        let bottom = paper.height_mm - stock.margin_mm.1 - stock.down_mm();

        assert!(
            right > 0.0,
            "{}: {} across runs {:.1} mm off the right of the paper",
            stock.code,
            stock.across,
            -right
        );
        assert!(
            bottom > 0.0,
            "{}: {} down runs {:.1} mm off the bottom of the paper",
            stock.code,
            stock.down,
            -bottom
        );
        // A tenth of a millimetre, which is the precision the published
        // figures are given to.
        assert!(
            (right - stock.margin_mm.0).abs() < 0.15,
            "{}: {:.2} mm at the left and {:.2} at the right — label stock is \
             cut down the middle, so one of these numbers is wrong",
            stock.code,
            stock.margin_mm.0,
            right
        );
        assert!(
            (bottom - stock.margin_mm.1).abs() < 0.15,
            "{}: {:.2} mm at the top and {:.2} at the bottom — one of these \
             numbers is wrong",
            stock.code,
            stock.margin_mm.1,
            bottom
        );
    }
}

/// The count in the description must be the count the grid gives. "21 to a
/// sheet" beside a 3 × 8 grid is somebody having edited one and not the other.
#[test]
fn the_count_in_the_words_is_the_count_in_the_grid() {
    for stock in KNOWN {
        let said = stock
            .what_for
            .split_whitespace()
            .find_map(|word| word.parse::<usize>().ok())
            .unwrap_or_else(|| panic!("{} does not say how many to a sheet", stock.code));
        assert_eq!(
            said,
            stock.per_sheet(),
            "{} says {said} to a sheet but its grid is {} × {}",
            stock.code,
            stock.across,
            stock.down
        );
    }
}

/// Two entries claiming the same code would make `find` depend on the order of
/// the table, which is not a thing anybody should have to think about.
#[test]
fn no_code_is_claimed_twice() {
    let mut seen: Vec<String> = Vec::new();
    for stock in KNOWN {
        for code in std::iter::once(stock.code).chain(stock.also.iter().copied()) {
            let tidied = tidy(code);
            assert!(
                !seen.contains(&tidied),
                "{code} is claimed by more than one stock"
            );
            seen.push(tidied);
        }
    }
}

/// Somebody types what is on the box, and what is on the box is "Avery L7160".
#[test]
fn a_code_is_found_however_it_is_written() {
    let wanted = find("l7160").expect("l7160 is not known");
    for spelling in [
        "L7160",
        "l7160",
        "  L7160 ",
        "Avery L7160",
        "avery-l7160",
        "AVERY L7160",
        "l 7160",
        "L7160-25",
        "j8160",
        "7160",
    ] {
        assert_eq!(
            find(spelling),
            Some(wanted),
            "'{spelling}' did not find L7160"
        );
    }
}

#[test]
fn a_code_nobody_has_is_not_invented() {
    assert!(find("l9999").is_none());
    assert!(find("").is_none());
    assert!(find("avery").is_none());
}

/// A code that is not known says what is, and how to go on without it —
/// because somebody with an unlisted box still has a job to do.
#[test]
fn an_unknown_code_names_the_ones_that_are_known_and_the_way_round_it() {
    let said = not_known("l9999");
    assert!(said.contains("l9999"), "{said}");
    assert!(said.contains("l7160"), "{said}");
    assert!(said.contains("5160"), "{said}");
    assert!(said.contains("--label"), "{said}");
    assert!(said.contains("--grid"), "{said}");
}

/// The whole reason a code is allowed at all: the numbers it stands for are on
/// the screen before any paper moves.
#[test]
fn what_a_code_filled_in_is_said_out_loud() {
    let said = find("l7160").unwrap().describe();
    assert!(said.contains("Avery L7160"), "{said}");
    assert!(said.contains("A4"), "{said}");
    assert!(said.contains("3 across by 7 down"), "{said}");
    assert!(said.contains("63.5 × 38.1 mm"), "{said}");
    assert!(said.contains("7.2 mm in from the left"), "{said}");
    assert!(said.contains("15.1 mm down from the top"), "{said}");
    assert!(said.contains("2.5 mm between columns"), "{said}");
    // And that the box, not this table, is the authority.
    assert!(said.contains("box is the"), "{said}");
    assert!(said.contains("plain paper"), "{said}");
}

/// A gap of nothing is not worth a clause.
#[test]
fn a_gap_of_nothing_is_not_mentioned() {
    let said = find("l7160").unwrap().describe();
    assert!(
        !said.contains("between rows"),
        "a zero gap was described: {said}"
    );
}

/// A whole number of millimetres reads as a whole number.
#[test]
fn measurements_read_the_way_somebody_would_say_them() {
    assert_eq!(trim(38.0), "38");
    assert_eq!(trim(38.1), "38.1");
    assert_eq!(trim(4.65), "4.7");
}

/// The listing is what somebody reads to find their box, so it has to name the
/// paper — an A4 code on Letter stock is the mistake that ruins a sheet.
#[test]
fn the_listing_names_the_paper_for_each_one() {
    let listed = every_code();
    assert_eq!(listed.len(), KNOWN.len());
    assert!(listed
        .iter()
        .any(|line| line.contains("l7160") && line.contains("A4")));
    assert!(listed
        .iter()
        .any(|line| line.contains("5160") && line.contains("US Letter")));
}

/// The sizes that are stated in inches on an American box must come back as
/// those inches. 5160 is 1 inch by 2⅝, and anything else is a wrong table.
#[test]
fn the_american_sizes_are_the_inches_they_are_sold_as() {
    let inch = 25.4;
    let five_one_six_zero = find("5160").unwrap();
    assert!((five_one_six_zero.label_mm.1 - inch).abs() < 0.1);
    assert!((five_one_six_zero.label_mm.0 - 2.625 * inch).abs() < 0.1);

    let shipping = find("5163").unwrap();
    assert!((shipping.label_mm.0 - 4.0 * inch).abs() < 0.1);
    assert!((shipping.label_mm.1 - 2.0 * inch).abs() < 0.1);
}
