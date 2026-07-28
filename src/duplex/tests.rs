use super::*;

const A4: PageSize = PageSize {
    width_mm: 210.0,
    height_mm: 297.0,
};

/// Sheets and pages agree with each other, both ways, for as many sheets as
/// anybody feeds in one go.
#[test]
fn a_sheet_and_a_side_name_exactly_one_page() {
    for sheet in 1..=200usize {
        for side in [Side::Front, Side::Back] {
            let page = page_of(sheet, side);
            assert_eq!(
                sheet_and_side(page),
                (sheet, side),
                "sheet {sheet} {} came back as something else",
                side.describe()
            );
        }
    }
    // The ones anybody would check by hand.
    assert_eq!(page_of(1, Side::Front), 1);
    assert_eq!(page_of(1, Side::Back), 2);
    assert_eq!(page_of(3, Side::Front), 5);
    assert_eq!(page_of(3, Side::Back), 6);
    assert_eq!(sheet_and_side(6), (3, Side::Back));
}

/// Two pages to a sheet, and an odd page still uses a whole one.
#[test]
fn an_odd_page_still_costs_a_whole_sheet() {
    assert_eq!(sheets_for(1), 1);
    assert_eq!(sheets_for(2), 1);
    assert_eq!(sheets_for(3), 2);
    assert_eq!(sheets_for(20), 10);
    assert_eq!(sheets_for(0), 0);
}

/// A back that comes out the same way up needs nothing done to it.
#[test]
fn a_back_that_comes_out_upright_is_left_alone() {
    let placed = turn_a_placement(20.0, 40.0, 0.0, A4, Feed::SameWayUp);
    assert_eq!(placed, (20.0, 40.0, 0.0));
    let boxed = turn_a_box(20.0, 40.0, 60.0, 8.0, A4, Feed::SameWayUp);
    assert_eq!(boxed, (20.0, 40.0, 60.0, 8.0));
}

/// Turning it twice is not turning it at all, which is the arithmetic saying
/// the same thing the paper does.
#[test]
fn turning_a_placement_twice_puts_it_back() {
    for (x, y, rotation) in [
        (20.0, 40.0, 0.0),
        (150.0, 260.0, 30.0),
        (105.0, 148.5, -90.0),
    ] {
        let (x2, y2, r2) = turn_a_placement(x, y, rotation, A4, Feed::TurnedAround);
        let (x3, y3, r3) = turn_a_placement(x2, y2, r2, A4, Feed::TurnedAround);
        assert!((x3 - x).abs() < 1e-9, "{x3} is not {x}");
        assert!((y3 - y).abs() < 1e-9, "{y3} is not {y}");
        assert!(
            (r3 - rotation - 360.0).abs() < 1e-9,
            "{r3} against {rotation}"
        );
    }

    for area in [(20.0, 40.0, 60.0, 8.0), (0.0, 0.0, 210.0, 297.0)] {
        let once = turn_a_box(area.0, area.1, area.2, area.3, A4, Feed::TurnedAround);
        let twice = turn_a_box(once.0, once.1, once.2, once.3, A4, Feed::TurnedAround);
        assert_eq!(twice, area);
    }
}

/// A box keeps its size and swaps its corner for the opposite one, so it covers
/// the same part of the paper when the sheet is turned.
#[test]
fn a_turned_box_covers_the_same_part_of_the_paper() {
    let (x, y, width, height) = turn_a_box(20.0, 40.0, 60.0, 8.0, A4, Feed::TurnedAround);
    assert_eq!((width, height), (60.0, 8.0), "the box changed size");
    // Its far corner is where the near one was, measured from the other end.
    assert!((x + width - (A4.width_mm - 20.0)).abs() < 1e-9, "{x}");
    assert!((y + height - (A4.height_mm - 40.0)).abs() < 1e-9, "{y}");
}

/// The words somebody types are the words that work.
#[test]
fn the_answer_can_be_written_the_way_people_say_it() {
    for said in ["same", "SAME", " same-way-up ", "book", "long", "long-edge"] {
        assert_eq!(Feed::parse(said), Some(Feed::SameWayUp), "{said}");
    }
    for said in [
        "turned",
        "TURNED",
        "turned-around",
        "calendar",
        "short",
        "short-edge",
    ] {
        assert_eq!(Feed::parse(said), Some(Feed::TurnedAround), "{said}");
    }
    assert_eq!(Feed::parse("sideways"), None);
    // And what is written down comes back as itself.
    for feed in [Feed::SameWayUp, Feed::TurnedAround] {
        assert_eq!(Feed::parse(feed.key()), Some(feed));
    }
}

/// The word read off the test sheet is the answer, without anybody having to
/// translate it into a yes.
#[test]
fn the_word_on_the_test_sheet_is_the_answer() {
    assert_eq!(what_the_word_means("SAME"), Some(Feed::SameWayUp));
    assert_eq!(what_the_word_means(" turned "), Some(Feed::TurnedAround));
    assert_eq!(what_the_word_means("neither"), None);

    // And both words really are on the sheet, so there is something to read.
    let sheet = a_test_sheet(A4);
    let words: Vec<&str> = sheet.iter().map(|line| line.text.as_str()).collect();
    assert!(words.contains(&"SAME"), "{words:?}");
    assert!(words.contains(&"TURNED"), "{words:?}");
    for word in ["SAME", "TURNED"] {
        assert!(
            what_the_word_means(word).is_some(),
            "the sheet says '{word}' and nothing understands it"
        );
    }
}

/// One word upright near one end, the other upside down near the other — so
/// that whichever way the paper comes out, exactly one of them is readable and
/// at the top.
#[test]
fn the_test_sheet_reads_one_way_up_or_the_other_but_not_both() {
    let sheet = a_test_sheet(A4);
    let find = |text: &str| {
        sheet
            .iter()
            .find(|line| line.text == text)
            .unwrap_or_else(|| panic!("no '{text}' on the sheet"))
    };
    let same = find("SAME");
    let turned = find("TURNED");

    assert_eq!(same.rotation_deg, 0.0, "SAME is not upright");
    assert_eq!(turned.rotation_deg, 180.0, "TURNED is not upside down");
    assert!(same.y_mm < A4.height_mm / 2.0, "SAME is not near the top");
    assert!(
        turned.y_mm > A4.height_mm / 2.0,
        "TURNED is not near the bottom"
    );

    // Both well clear of the edge, where a printer's grip is.
    for line in &sheet {
        assert!(
            line.y_mm > 10.0 && line.y_mm < A4.height_mm - 10.0,
            "'{}' is in the margin at {}",
            line.text,
            line.y_mm
        );
        assert!(
            line.x_mm > 5.0 && line.x_mm < A4.width_mm - 5.0,
            "{}",
            line.text
        );
    }
}

/// Everything the two answers say about themselves is different, because a
/// person choosing between them reads exactly this.
#[test]
fn the_two_answers_describe_themselves_differently() {
    assert_ne!(
        Feed::SameWayUp.describe(),
        Feed::TurnedAround.describe(),
        "the two feeds say the same thing about themselves"
    );
    assert!(Feed::SameWayUp.describe().contains("upright"));
    assert!(Feed::TurnedAround.describe().contains("upside down"));
    assert_ne!(Feed::SameWayUp.key(), Feed::TurnedAround.key());
    // The default is the one most printers do and most people expect.
    assert_eq!(Feed::default(), Feed::SameWayUp);
}

/// The instructions name both of the words the sheet actually prints, and the
/// commands they lead to.
#[test]
fn the_instructions_match_the_sheet_they_are_about() {
    for word in ["SAME", "TURNED"] {
        assert!(
            a_test_sheet(A4).iter().any(|line| line.text == word),
            "the sheet does not say {word}"
        );
    }
    for feed in [Feed::SameWayUp, Feed::TurnedAround] {
        assert!(
            HOW_TO_USE_THE_TEST_SHEET.contains(&format!("config set feed {}", feed.key())),
            "the instructions do not say how to remember '{}'",
            feed.key()
        );
    }
}

/// The promise, checked on the paper: a word asked for at 20 mm in and 40 mm
/// down really is 20 mm in and 40 mm down when somebody holds the finished back
/// the right way up.
///
/// Every other test here is arithmetic agreeing with itself. This one draws the
/// page, turns the picture of it round the way a hand turns paper, and measures
/// where the ink actually is — which is the only version of the claim that
/// matters, because the arithmetic is what would be wrong.
#[test]
fn a_word_asked_for_at_a_place_lands_there_on_the_finished_back() {
    const ASKED_X: f64 = 20.0;
    const ASKED_Y: f64 = 40.0;

    for feed in [Feed::SameWayUp, Feed::TurnedAround] {
        let (x_mm, y_mm, rotation_deg) = turn_a_placement(ASKED_X, ASKED_Y, 0.0, A4, feed);
        let line = PlacedLine {
            text: "HERE".to_string(),
            x_mm,
            y_mm,
            size_pt: 24.0,
            font: LineFont::Builtin(Font::Helvetica),
            colour: (0.0, 0.0, 0.0),
            rotation_deg,
        };

        let dir = tempfile::tempdir().expect("somewhere to work");
        let pdf = dir.path().join("back.pdf");
        crate::pdf::write_delta(&pdf, &[A4], &[vec![line]], "Onionskin", None)
            .expect("the page should be written");

        const DPI: f64 = 100.0;
        let engine = crate::render::engine().expect("a renderer");
        let document = engine.open(&pdf).expect("it should open");
        let drawn = document.render_gray(0, DPI).expect("it should draw");

        // The printer lays this on the paper. Now do what a hand does: for a
        // feed that turns the paper around, turn the picture around too, so
        // that what is measured is what somebody looking at the back sees.
        let seen: Vec<u8> = match feed {
            Feed::SameWayUp => drawn.gray.clone(),
            Feed::TurnedAround => drawn.gray.iter().rev().copied().collect(),
        };

        let mm = |pixels: usize| (pixels as f64 + 0.5) * 25.4 / DPI;
        let ink: Vec<(f64, f64)> = (0..drawn.height)
            .flat_map(|y| (0..drawn.width).map(move |x| (x, y)))
            .filter(|(x, y)| seen[y * drawn.width + x] < 128)
            .map(|(x, y)| (mm(x), mm(y)))
            .collect();
        assert!(!ink.is_empty(), "nothing was printed for {feed:?}");

        let left = ink.iter().map(|spot| spot.0).fold(f64::MAX, f64::min);
        let bottom = ink.iter().map(|spot| spot.1).fold(f64::MIN, f64::max);

        // The left edge of the ink is the left of the H, a whisker right of
        // where the word was asked to start; the bottom of it is the baseline,
        // which is what 40 mm down means for a line of type.
        assert!(
            (left - ASKED_X).abs() < 2.0,
            "{feed:?}: the word starts {left:.1} mm in, not {ASKED_X}"
        );
        assert!(
            (bottom - ASKED_Y).abs() < 2.0,
            "{feed:?}: the word sits {bottom:.1} mm down, not {ASKED_Y}"
        );
    }
}

/// And the wrong answer really does put it in the wrong place — otherwise the
/// question would not be worth asking, and neither would the test sheet.
#[test]
fn getting_the_feed_wrong_puts_the_words_at_the_other_end_of_the_paper() {
    let upright = turn_a_placement(20.0, 40.0, 0.0, A4, Feed::SameWayUp);
    let turned = turn_a_placement(20.0, 40.0, 0.0, A4, Feed::TurnedAround);
    assert!(
        (upright.0 - turned.0).abs() > 100.0 && (upright.1 - turned.1).abs() > 100.0,
        "the two answers put the words in nearly the same place: {upright:?} \
         against {turned:?}"
    );
}
