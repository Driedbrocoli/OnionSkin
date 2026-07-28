use super::*;

const A4: PageSize = PageSize {
    width_mm: 210.0,
    height_mm: 297.0,
};

/// The corners of the ink, measured by walking the word from its start point
/// the way the writer will.
///
/// The *baseline* is not what anybody looks at, and it is deliberately not
/// centred — the letters stand above it, so a baseline through the middle of
/// the paper puts the word's body above the middle. What has to be centred, and
/// what has to stay on the paper, is the ink.
fn ink_corners(mark: &Watermark, font: Font) -> Vec<(f64, f64)> {
    let width = builtin_width_mm(font, &mark.text, mark.size_pt);
    let cap = mark.size_pt * 0.7 * 25.4 / 72.0;
    let radians = mark.rotation_deg.to_radians();
    let along = (radians.cos(), radians.sin());
    // Up out of the baseline, which is where the letters are.
    let up = (radians.sin(), -radians.cos());
    let at = |a: f64, u: f64| {
        (
            mark.x_mm + along.0 * a + up.0 * u,
            mark.y_mm + along.1 * a + up.1 * u,
        )
    };
    vec![at(0.0, 0.0), at(width, 0.0), at(width, cap), at(0.0, cap)]
}

/// The middle of the ink.
fn ink_middle(mark: &Watermark, font: Font) -> (f64, f64) {
    let corners = ink_corners(mark, font);
    let n = corners.len() as f64;
    (
        corners.iter().map(|c| c.0).sum::<f64>() / n,
        corners.iter().map(|c| c.1).sum::<f64>() / n,
    )
}

/// The whole point: the word crosses the middle of the paper, not a corner of
/// it. A watermark two centimetres long in the middle of A4 is not a watermark.
#[test]
fn the_word_runs_across_the_middle_of_the_sheet() {
    let mark = across("DRAFT", A4, Font::Helvetica, None, None).unwrap();
    let middle = ink_middle(&mark, Font::Helvetica);
    assert!(
        (middle.0 - A4.width_mm / 2.0).abs() < 1.0,
        "not centred across the paper: {middle:?}"
    );
    assert!(
        (middle.1 - A4.height_mm / 2.0).abs() < 1.0,
        "not centred down the paper: {middle:?}"
    );

    // And it goes bottom-left to top-right, which is the way every watermark
    // anybody has seen runs: rightwards, and up the page.
    let corners = ink_corners(&mark, Font::Helvetica);
    assert!(
        corners[1].0 > corners[0].0,
        "it does not run rightwards: {corners:?}"
    );
    assert!(
        corners[1].1 < corners[0].1,
        "it does not run up the page: {corners:?}"
    );
}

/// It has to stay on the paper. A word running off the edge is a word with its
/// first and last letters missing, and the corner is where the printer's grip
/// is.
#[test]
fn the_word_stays_on_the_paper() {
    // Every shape of word, on both ways up of the paper.
    for page in [A4, PageSize::new(297.0, 210.0), PageSize::new(200.0, 200.0)] {
        for text in [
            "I",
            "VOID",
            "DRAFT",
            "NOT FOR CIRCULATION",
            "COPY — DO NOT FILE",
        ] {
            let mark = across(text, page, Font::Helvetica, None, None).unwrap();
            for corner in ink_corners(&mark, Font::Helvetica) {
                assert!(
                    corner.0 > 0.0 && corner.0 < page.width_mm,
                    "'{text}' on {page:?} runs off the side at {corner:?}"
                );
                assert!(
                    corner.1 > 0.0 && corner.1 < page.height_mm,
                    "'{text}' on {page:?} runs off the top or bottom at {corner:?}"
                );
            }
        }
    }
}

/// A short word and a long one both fill the page, because the size comes off
/// the word rather than being a number somebody has to guess.
#[test]
fn a_short_word_is_set_larger_than_a_long_one_so_both_fill_the_page() {
    let short = across("VOID", A4, Font::Helvetica, None, None).unwrap();
    let long = across("NOT FOR CIRCULATION", A4, Font::Helvetica, None, None).unwrap();
    assert!(
        short.size_pt > long.size_pt * 2.0,
        "four letters were set at {} pt and nineteen at {}",
        short.size_pt,
        long.size_pt
    );

    // Both fill about the same amount of the paper, which is the point of
    // sizing at all — measured as the box the ink really occupies.
    for mark in [&short, &long] {
        let corners = ink_corners(mark, Font::Helvetica);
        let across_mm = corners.iter().map(|c| c.0).fold(f64::MIN, f64::max)
            - corners.iter().map(|c| c.0).fold(f64::MAX, f64::min);
        assert!(
            across_mm > A4.width_mm * 0.7,
            "'{}' only spans {across_mm:.0} mm of a {} mm sheet",
            mark.text,
            A4.width_mm
        );
    }
}

/// The angle follows the paper, so it really is corner to corner rather than a
/// flat forty-five degrees that misses on anything but a square.
#[test]
fn the_angle_is_the_papers_own_diagonal() {
    let portrait = across("DRAFT", A4, Font::Helvetica, None, None).unwrap();
    // A4 is taller than it is wide, so the diagonal is steeper than 45°.
    assert!(
        portrait.rotation_deg < -50.0 && portrait.rotation_deg > -60.0,
        "an A4 diagonal came out at {}°",
        portrait.rotation_deg
    );

    let landscape = across(
        "DRAFT",
        PageSize::new(297.0, 210.0),
        Font::Helvetica,
        None,
        None,
    )
    .unwrap();
    assert!(
        landscape.rotation_deg > -40.0 && landscape.rotation_deg < -30.0,
        "a landscape diagonal came out at {}°",
        landscape.rotation_deg
    );

    // And a square page is the flat forty-five everybody pictures.
    let square = across(
        "DRAFT",
        PageSize::new(200.0, 200.0),
        Font::Helvetica,
        None,
        None,
    )
    .unwrap();
    assert!(
        (square.rotation_deg + 45.0).abs() < 0.01,
        "{}",
        square.rotation_deg
    );
}

/// Somebody overruling either of the two worked-out numbers is taken at their
/// word, because the escape hatch is the whole reason to have one.
#[test]
fn what_somebody_asks_for_beats_what_the_page_works_out() {
    let mark = across("DRAFT", A4, Font::Helvetica, Some(48.0), Some(0.2)).unwrap();
    assert_eq!(mark.size_pt, 48.0);
    assert!((mark.grey - 0.2).abs() < 1e-9);
    // Still centred, at whatever size it was told.
    let middle = ink_middle(&mark, Font::Helvetica);
    assert!((middle.0 - A4.width_mm / 2.0).abs() < 1.0, "{middle:?}");
    assert!((middle.1 - A4.height_mm / 2.0).abs() < 1.0, "{middle:?}");
}

/// Toner goes on top of what is already printed. A dark watermark does not sit
/// behind the text the way a word processor's does — it sits over it, and the
/// text stops being readable.
#[test]
fn a_watermark_dark_enough_to_hide_the_page_is_noticed() {
    assert!(!too_dark_to_read_through(GREY), "the default is too dark");
    assert!(
        too_dark_to_read_through(0.1),
        "near-black was called readable"
    );
    assert!(!too_dark_to_read_through(0.9));

    // And the colour that comes out is a grey, not a colour.
    let mark = across("DRAFT", A4, Font::Helvetica, None, None).unwrap();
    let (r, g, b) = mark.colour();
    assert_eq!(r, g);
    assert_eq!(g, b);
    assert!((r - GREY).abs() < 1e-9);
}

/// A grey outside the range is brought back into it rather than producing a
/// colour no printer can name.
#[test]
fn a_grey_that_is_not_a_grey_is_brought_back() {
    let too_light = across("D", A4, Font::Helvetica, None, Some(5.0)).unwrap();
    assert!((too_light.grey - 1.0).abs() < 1e-9);
    let too_dark = across("D", A4, Font::Helvetica, None, Some(-2.0)).unwrap();
    assert!((too_dark.grey - 0.0).abs() < 1e-9);
}

/// Nothing to say, nothing to write — and a page of no size is not a page.
#[test]
fn nothing_is_refused_rather_than_placed_nowhere() {
    assert!(across("", A4, Font::Helvetica, None, None).is_none());
    assert!(across("   ", A4, Font::Helvetica, None, None).is_none());
    assert!(across(
        "DRAFT",
        PageSize::new(0.0, 297.0),
        Font::Helvetica,
        None,
        None
    )
    .is_none());
    assert!(across(
        "DRAFT",
        PageSize::new(210.0, -1.0),
        Font::Helvetica,
        None,
        None
    )
    .is_none());
}

/// What it says of itself is what somebody checks before printing.
#[test]
fn it_says_what_it_will_do() {
    let said = across("DRAFT", A4, Font::Helvetica, None, None)
        .unwrap()
        .describe();
    assert!(said.contains("DRAFT"), "{said}");
    assert!(said.contains("pt"), "{said}");
    assert!(said.contains("grey"), "{said}");
}

/// A size nothing could be set at is refused rather than written.
///
/// `--size 0` used to write a delta with a nought-point word on it: a sheet
/// through the printer for no ink, and a report saying "at 0 pt" as though that
/// were a thing.
#[test]
fn a_size_that_is_not_a_size_is_refused() {
    for size in [0.0, -12.0, f64::NAN, f64::INFINITY] {
        assert!(
            across("DRAFT", A4, Font::Helvetica, Some(size), None).is_none(),
            "{size} was accepted as a type size"
        );
    }
    // And a real one still is.
    assert!(across("DRAFT", A4, Font::Helvetica, Some(48.0), None).is_some());
}
