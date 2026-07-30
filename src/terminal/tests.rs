use super::*;

const A4: PageSize = PageSize {
    width_mm: 210.0,
    height_mm: 297.0,
};

const LANDSCAPE: PageSize = PageSize {
    width_mm: 297.0,
    height_mm: 210.0,
};

/// A page of nothing but paper, so a mark can be put on it and found.
fn blank(width: usize, height: usize) -> Vec<u8> {
    vec![255u8; width * height]
}

/// A black rectangle on the page, in pixels.
fn ink(gray: &mut [u8], width: usize, from_x: usize, from_y: usize, to_x: usize, to_y: usize) {
    for y in from_y..to_y {
        for x in from_x..to_x {
            gray[y * width + x] = 0;
        }
    }
}

/// Blank paper draws as blank paper — not as a wash of dots, which is what a
/// program that averaged naively would produce from a scan.
#[test]
fn an_empty_page_is_empty() {
    let drawn = draw(&blank(400, 566), 400, 566, A4, 40);
    assert!(
        drawn.lines.iter().all(|line| line.trim().is_empty()),
        "{}",
        drawn.text()
    );
    assert_eq!(drawn.across, 40);
}

/// A portrait page comes out portrait. Terminal cells are twice as tall as they
/// are wide, and without allowing for it A4 is drawn wider than it is tall — so
/// a stamp measured off the drawing is wrong in the one direction somebody was
/// checking.
#[test]
fn the_paper_keeps_its_shape() {
    let portrait = draw(&blank(200, 283), 200, 283, A4, 40);
    // 40 wide, 297/210 tall, halved for the cell shape: 28 rows.
    assert_eq!(portrait.down, 28);
    assert_eq!(portrait.lines.len(), 28);
    assert!(
        portrait.down < portrait.across,
        "a portrait page came out wider than it is tall"
    );

    let landscape = draw(&blank(283, 200), 283, 200, LANDSCAPE, 40);
    assert_eq!(landscape.down, 14);
    assert!(
        landscape.down < portrait.down,
        "landscape and portrait drew the same"
    );

    // The shape comes off the paper, not the picture: a render at a strange
    // resolution must not change what the sheet looks like.
    let odd = draw(&blank(50, 800), 50, 800, A4, 40);
    assert_eq!(odd.down, portrait.down);
}

/// The whole point: a mark in the top right of the page draws in the top right
/// of the terminal.
#[test]
fn a_mark_lands_where_it_is_on_the_paper() {
    let (width, height) = (400usize, 566usize);
    let mut gray = blank(width, height);
    // A stamp at about 150,40 mm on A4 — three quarters across, an eighth down.
    ink(&mut gray, width, 280, 60, 360, 90);

    let drawn = draw(&gray, width, height, A4, 40);
    let inked: Vec<(usize, usize)> = drawn
        .lines
        .iter()
        .enumerate()
        .flat_map(|(row, line)| {
            line.chars()
                .enumerate()
                .filter(|(_, c)| *c != ' ')
                .map(move |(column, _)| (row, column))
        })
        .collect();
    assert!(!inked.is_empty(), "the mark vanished:\n{}", drawn.text());

    let leftmost = inked.iter().map(|(_, c)| *c).min().unwrap();
    let rightmost = inked.iter().map(|(_, c)| *c).max().unwrap();
    let topmost = inked.iter().map(|(r, _)| *r).min().unwrap();
    let lowest = inked.iter().map(|(r, _)| *r).max().unwrap();

    // Three quarters across the paper is three quarters across the drawing,
    // within a character.
    assert!(
        (26..=29).contains(&leftmost),
        "{leftmost}: \n{}",
        drawn.text()
    );
    assert!(
        (35..=37).contains(&rightmost),
        "{rightmost}: \n{}",
        drawn.text()
    );
    // And an eighth down.
    assert!((2..=4).contains(&topmost), "{topmost}: \n{}", drawn.text());
    assert!(lowest < drawn.down / 3, "{lowest}: \n{}", drawn.text());
}

/// A line of type covers perhaps a tenth of the cell it sits in. Averaging
/// makes it invisible, which for a program whose whole business is where the
/// words landed is the one thing the drawing must not do.
#[test]
fn a_thin_line_of_type_does_not_average_away() {
    let (width, height) = (400usize, 566usize);
    let mut gray = blank(width, height);
    // Two pixels tall, the way a line of nine-point type is at this size.
    ink(&mut gray, width, 40, 200, 300, 202);

    let drawn = draw(&gray, width, height, A4, 40);
    assert!(
        drawn.lines.iter().any(|line| !line.trim().is_empty()),
        "a line of type disappeared entirely:\n{}",
        drawn.text()
    );
    // On the right row: 200 of 566 down the page, over 28 rows, is row 9 or 10.
    let row = drawn
        .lines
        .iter()
        .position(|line| !line.trim().is_empty())
        .unwrap();
    assert!((8..=11).contains(&row), "row {row}:\n{}", drawn.text());
}

/// More ink draws darker, which is the only thing the shades have to promise.
#[test]
fn more_ink_draws_darker() {
    let (width, height) = (80usize, 80usize);
    let mut seen: Vec<char> = Vec::new();
    for covered in [0usize, 20, 40, 60, 80] {
        let mut gray = blank(width, height);
        ink(&mut gray, width, 0, 0, width, covered);
        let drawn = draw(
            &gray,
            width,
            height,
            PageSize {
                width_mm: 100.0,
                height_mm: 100.0,
            },
            8,
        );
        seen.push(drawn.lines[0].chars().next().unwrap_or(' '));
    }
    // Each step is at least as dark as the one before, and the ends differ.
    let rank = |c: char| SHADES.iter().position(|s| *s == c).unwrap_or(0);
    for pair in seen.windows(2) {
        assert!(
            rank(pair[1]) >= rank(pair[0]),
            "{:?} went lighter as the ink went up",
            seen
        );
    }
    assert!(rank(seen[4]) > rank(seen[0]), "{seen:?}");
    assert_eq!(seen[0], ' ', "blank paper should be blank");
}

/// A frame, so the edge of the paper can be told from the edge of the terminal
/// — which is the whole question when somebody asks whether a stamp has fallen
/// off the sheet.
#[test]
fn the_edge_of_the_paper_is_visible() {
    let drawn = draw(&blank(100, 141), 100, 141, A4, 20);
    let framed = drawn.framed("page 1 of 3");
    let lines: Vec<&str> = framed.lines().collect();

    assert!(
        lines[0].starts_with('┌') && lines[0].ends_with('┐'),
        "{}",
        lines[0]
    );
    assert!(lines[lines.len() - 2].starts_with('└'), "{framed}");
    assert_eq!(lines.last().copied(), Some("page 1 of 3"));
    // Every drawn row is the same width, or the frame is ragged.
    let widths: Vec<usize> = lines[1..lines.len() - 2]
        .iter()
        .map(|line| line.chars().count())
        .collect();
    assert!(
        widths.iter().all(|w| *w == drawn.across + 2),
        "ragged frame: {widths:?}"
    );

    // No caption, no trailing blank line.
    assert!(!drawn.framed("").ends_with('\n'));
}

/// Everything drawn is plain ASCII except the frame, because this output goes
/// into logs and over SSH to terminals whose fonts have nothing else in them.
#[test]
fn the_page_itself_is_drawn_in_characters_every_terminal_has() {
    for shade in SHADES {
        assert!(shade.is_ascii(), "{shade:?} is not something to rely on");
    }
    let (width, height) = (60usize, 60usize);
    let mut gray = blank(width, height);
    ink(&mut gray, width, 5, 5, 55, 55);
    let drawn = draw(&gray, width, height, A4, 30);
    assert!(
        drawn.text().is_ascii(),
        "something not ASCII got into the page:\n{}",
        drawn.text()
    );
}

/// A picture with nothing in it, or one smaller than it claims to be, draws
/// blank paper rather than reading off the end of the buffer.
#[test]
fn a_picture_that_is_not_there_does_not_take_the_program_with_it() {
    for (gray, width, height) in [
        (Vec::new(), 0usize, 0usize),
        (Vec::new(), 10, 10),
        (vec![255u8; 10], 100, 100),
    ] {
        let drawn = draw(&gray, width, height, A4, 20);
        assert_eq!(drawn.across, 20);
        assert!(drawn.down > 0);
        assert!(drawn.lines.iter().all(|line| line.trim().is_empty()));
    }
}

/// Asked for something absurd, it draws something readable rather than
/// nothing, a panic, or a gigabyte of text.
#[test]
fn a_silly_width_is_brought_back_to_a_sensible_one() {
    for across in [0usize, 1, 3, 100_000] {
        let drawn = draw(&blank(50, 70), 50, 70, A4, across);
        assert!(
            (8..=400).contains(&drawn.across),
            "{across} → {}",
            drawn.across
        );
        assert!(drawn.down >= 1);
        assert_eq!(drawn.lines.len(), drawn.down);
    }
    // A page with no width at all is a page that cannot say what shape it is,
    // and it is drawn square rather than divided by zero.
    let square = draw(
        &blank(10, 10),
        10,
        10,
        PageSize {
            width_mm: 0.0,
            height_mm: 297.0,
        },
        20,
    );
    assert_eq!(square.down, 10);
}

/// A ruler, so a position can be read off the drawing rather than guessed at —
/// which is what turns "roughly there" into "150 mm across".
#[test]
fn there_is_a_scale_to_read_positions_off() {
    let said = ruler(A4, 40);
    let lines: Vec<&str> = said.lines().collect();
    assert_eq!(lines.len(), 2, "{said}");
    // A4 is 210 wide, so marks at 0, 50, 100, 150 and 200.
    assert_eq!(lines[0].matches('|').count(), 5, "{said}");
    assert!(lines[0].starts_with('|'), "{said}");
    for label in ["0mm", "50", "100", "150", "200"] {
        assert!(lines[1].contains(label), "{label} missing from: {said}");
    }

    // Never wider than the drawing it sits under: a ruler that wraps onto a
    // second line is worse than no ruler, and the unit is on the first label
    // rather than after the last one for exactly that reason.
    for across in [8usize, 20, 40, ACROSS, 120] {
        for page in [A4, LANDSCAPE] {
            for line in ruler(page, across).lines() {
                assert!(
                    line.chars().count() <= across,
                    "{across} wide, but a ruler line was {}: {line:?}",
                    line.chars().count()
                );
            }
        }
    }
}

/// A page with no width cannot say where anything is along it, and dividing by
/// it would give a row of NaN rather than an answer.
#[test]
fn a_page_with_no_width_gets_no_ruler_rather_than_nonsense() {
    for width_mm in [0.0, -5.0, f64::NAN, f64::INFINITY] {
        let said = ruler(
            PageSize {
                width_mm,
                height_mm: 297.0,
            },
            40,
        );
        assert!(said.is_empty(), "{width_mm}: {said:?}");
    }
}

/// A drawing too narrow to hold the labels still says where the fifties are,
/// rather than printing them over one another.
///
/// Two labels that abut run together into one number that says nothing —
/// "100" ending where "150" begins reads as 100150, and somebody measuring off
/// it has no idea which is which. So every width is checked: whatever labels
/// survive have a space between them and can be read back as the numbers they
/// were.
#[test]
fn no_two_labels_ever_run_together_into_one_number() {
    for across in [8usize, 12, 16, 20, 24, 30, 40, 60, ACROSS, 120, 200] {
        for page in [A4, LANDSCAPE] {
            let said = ruler(page, across);
            let lines: Vec<&str> = said.lines().collect();
            assert_eq!(lines.len(), 2, "{across}: {said}");
            assert!(lines[0].contains('|'), "{across}: {said}");

            // Every label that survived is one of the fifties, read back — not
            // two of them stuck together.
            let read: Vec<usize> = lines[1]
                .split_whitespace()
                .map(|word| {
                    word.trim_end_matches("mm")
                        .parse()
                        .unwrap_or_else(|_| panic!("{across}: {word:?} in {said}"))
                })
                .collect();
            for number in &read {
                assert_eq!(
                    number % 50,
                    0,
                    "{across}: {number} is two labels run together — {said}"
                );
                assert!(
                    *number as f64 <= page.width_mm,
                    "{across}: {number} is past the edge of the paper — {said}"
                );
            }
            assert!(
                read.windows(2).all(|pair| pair[0] < pair[1]),
                "{across}: out of order — {said}"
            );
            assert!(!read.is_empty(), "{across}: no labels at all — {said}");
        }
    }
}

/// A page of an absurd shape must not ask for a drawing the machine cannot
/// hold. A sliver a millimetre wide and a metre tall wants hundreds of
/// thousands of rows; a width that has gone subnormal makes the ratio infinite,
/// and `as usize` saturates rather than failing — so what comes out is a
/// request to allocate usize::MAX rows and a panic reading "capacity overflow".
#[test]
fn a_page_of_an_absurd_shape_is_drawn_rather_than_crashed_on() {
    let shapes = [
        (1.0, 1000.0),
        (0.001, 297.0),
        (f64::MIN_POSITIVE, 297.0),
        (1e-300, 1e300),
        (210.0, f64::MAX),
        (210.0, 0.0),
        (210.0, -297.0),
        (f64::MAX, 297.0),
    ];
    for (width_mm, height_mm) in shapes {
        let page = PageSize {
            width_mm,
            height_mm,
        };
        let drawn = draw(&blank(40, 40), 40, 40, page, 40);
        assert!(
            (1..=DOWN_AT_MOST).contains(&drawn.down),
            "{width_mm} × {height_mm} asked for {} rows",
            drawn.down
        );
        assert_eq!(drawn.lines.len(), drawn.down);
        // And the frame round it is still square.
        let framed = drawn.framed("");
        assert!(framed.lines().count() >= 3, "{width_mm} × {height_mm}");
    }
}
