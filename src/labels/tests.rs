//! Tests for label sheets.

use super::*;

const A4: PageSize = PageSize {
    width_mm: 210.0,
    height_mm: 297.0,
};

/// A common address-label sheet: three across, eight down, measured off the
/// box the way somebody would type it in.
fn three_by_eight() -> Grid {
    Grid {
        page: A4,
        columns: 3,
        rows: 8,
        margin_x_mm: 7.0,
        margin_y_mm: 15.0,
        gap_x_mm: 2.5,
        gap_y_mm: 0.0,
        label: Some((63.5, 33.9)),
    }
}

#[test]
fn the_labels_land_where_the_stock_is_cut() {
    let grid = three_by_eight();
    grid.check().unwrap();
    assert_eq!(grid.per_sheet(), 24);

    // The first is at the margins.
    let first = grid.cell(0).unwrap();
    assert_eq!((first.x_mm, first.y_mm), (7.0, 15.0));
    assert_eq!((first.width_mm, first.height_mm), (63.5, 33.9));

    // The second is one label and one gap to the right.
    let second = grid.cell(1).unwrap();
    assert!((second.x_mm - (7.0 + 63.5 + 2.5)).abs() < 1e-9, "{second:?}");
    assert_eq!(second.y_mm, 15.0);

    // The fourth begins the second row, back at the left margin.
    let fourth = grid.cell(3).unwrap();
    assert_eq!(fourth.x_mm, 7.0);
    assert!((fourth.y_mm - (15.0 + 33.9)).abs() < 1e-9, "{fourth:?}");

    // And there is no twenty-fifth.
    assert!(grid.cell(24).is_none());
}

/// Across then down, because that is the order labels are peeled off and
/// therefore the order a half-used sheet is used up in.
#[test]
fn the_labels_are_filled_across_before_down() {
    let grid = three_by_eight();
    let row_of = |index: usize| grid.cell(index).unwrap().y_mm;
    assert_eq!(row_of(0), row_of(1));
    assert_eq!(row_of(1), row_of(2));
    assert!(row_of(3) > row_of(2), "the fourth should start a new row");
}

/// Nobody ever uses a whole sheet. There is always one in the drawer with the
/// first five peeled off, and this is the difference between the feature
/// being useful and people going back to a word processor.
#[test]
fn a_half_used_sheet_can_be_started_part_way_in() {
    let grid = three_by_eight();
    // Five already peeled off: the first name goes on the sixth label.
    assert_eq!(grid.place(0, 5), (0, 5));
    assert_eq!(grid.place(1, 5), (0, 6));
    // And it runs onto the second sheet when the first is used up.
    assert_eq!(grid.place(19, 5), (1, 0));
}

#[test]
fn how_many_sheets_are_needed_counts_the_ones_already_peeled_off() {
    let grid = three_by_eight(); // 24 to a sheet
    assert_eq!(grid.sheets_needed(0, 0), 0);
    assert_eq!(grid.sheets_needed(1, 0), 1);
    assert_eq!(grid.sheets_needed(24, 0), 1);
    assert_eq!(grid.sheets_needed(25, 0), 2);
    // Starting five in, nineteen still fit on the first sheet.
    assert_eq!(grid.sheets_needed(19, 5), 1);
    assert_eq!(grid.sheets_needed(20, 5), 2);
}

/// A grid that runs off the sheet does not fail — it prints, onto the backing
/// paper, and costs a sheet of labels. So it is refused before anything is
/// written, and the refusal says by how much and out of what.
#[test]
fn a_grid_that_runs_off_the_paper_is_refused_with_the_numbers() {
    let too_wide = Grid {
        columns: 4,
        ..three_by_eight()
    };
    let said = too_wide.check().unwrap_err();
    assert!(said.contains("right-hand edge"), "{said}");
    assert!(said.contains("4 columns"), "{said}");
    assert!(said.contains("210.0 mm"), "{said}");

    let too_tall = Grid {
        rows: 12,
        ..three_by_eight()
    };
    let said = too_tall.check().unwrap_err();
    assert!(said.contains("off the bottom"), "{said}");
    assert!(said.contains("12 rows"), "{said}");
}

/// Label stock is measured to the tenth of a millimetre, and a grid that comes
/// out exactly flush must not be refused for a rounding error.
#[test]
fn a_grid_that_exactly_fills_the_paper_is_allowed() {
    let exact = Grid {
        page: A4,
        columns: 2,
        rows: 2,
        margin_x_mm: 0.0,
        margin_y_mm: 0.0,
        gap_x_mm: 0.0,
        gap_y_mm: 0.0,
        label: Some((105.0, 148.5)),
    };
    exact.check().unwrap();
}

#[test]
fn labels_with_no_size_given_are_made_to_fill_the_page() {
    let grid = Grid {
        page: A4,
        columns: 2,
        rows: 4,
        margin_x_mm: 10.0,
        margin_y_mm: 10.0,
        gap_x_mm: 5.0,
        gap_y_mm: 5.0,
        label: None,
    };
    grid.check().unwrap();
    let (width_mm, height_mm) = grid.label_size();
    // Two columns, 10 mm each side, one 5 mm gap: (210 - 20 - 5) / 2.
    assert!((width_mm - 92.5).abs() < 1e-9, "{width_mm}");
    // Four rows, 10 mm top and bottom, three 5 mm gaps: (297 - 20 - 15) / 4.
    assert!((height_mm - 65.5).abs() < 1e-9, "{height_mm}");
    // And what it works out has to be a grid that fits.
    let last = grid.cell(grid.per_sheet() - 1).unwrap();
    assert!(last.x_mm + last.width_mm <= A4.width_mm + 0.1, "{last:?}");
    assert!(last.y_mm + last.height_mm <= A4.height_mm + 0.1, "{last:?}");
}

#[test]
fn a_grid_with_no_room_left_for_labels_says_so_rather_than_making_none() {
    let crowded = Grid {
        page: A4,
        columns: 40,
        rows: 4,
        margin_x_mm: 10.0,
        margin_y_mm: 10.0,
        gap_x_mm: 10.0,
        gap_y_mm: 5.0,
        label: None,
    };
    let said = crowded.check().unwrap_err();
    assert!(said.contains("no room left"), "{said}");

    assert!(Grid {
        columns: 0,
        ..three_by_eight()
    }
    .check()
    .is_err());
}

// ---------------------------------------------------------------------------
// Words inside a label
// ---------------------------------------------------------------------------

#[test]
fn the_lines_of_a_label_sit_inside_it() {
    let cell = Cell {
        x_mm: 7.0,
        y_mm: 15.0,
        width_mm: 63.5,
        height_mm: 33.9,
    };
    let (x_mm, y_mm) = cell.line_at(0, 11.0, 1.2, 3.0);
    assert_eq!(x_mm, 10.0);
    // The first baseline is below the top edge, not on it — otherwise the
    // letters hang above the label.
    assert!(y_mm > cell.y_mm + 3.0, "{y_mm}");
    assert!(y_mm < cell.y_mm + 10.0, "{y_mm}");

    // Each line after it is one leading further down.
    let (_, second) = cell.line_at(1, 11.0, 1.2, 3.0);
    let step = crate::geometry::pt_to_mm(11.0 * 1.2);
    assert!((second - y_mm - step).abs() < 1e-9, "{second} {y_mm}");
}

/// A label silently overfilled prints its last line onto the next label, or
/// onto the backing paper. Knowing how many fit is what lets that be said.
#[test]
fn how_many_lines_fit_is_known() {
    let cell = Cell {
        x_mm: 7.0,
        y_mm: 15.0,
        width_mm: 63.5,
        height_mm: 33.9,
    };
    // 33.9 mm less 3 mm of padding each side, at 11 pt × 1.2 ≈ 4.65 mm a line.
    assert_eq!(cell.lines_that_fit(11.0, 1.2, 3.0), 5);
    // Bigger type, fewer lines.
    assert!(cell.lines_that_fit(24.0, 1.2, 3.0) < 5);
    // And a padding that swallows the label fits nothing rather than panicking.
    assert_eq!(cell.lines_that_fit(11.0, 1.2, 20.0), 0);
    assert_eq!(cell.lines_that_fit(11.0, 0.0, 3.0), 0);
}

#[test]
fn a_grid_says_what_it_is_in_words() {
    let said = three_by_eight().describe();
    assert!(said.contains("3 × 8"), "{said}");
    assert!(said.contains("63.5"), "{said}");
    assert!(said.contains("A4"), "{said}");
}
