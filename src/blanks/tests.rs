//! Tests for finding the writable places on a form.

use super::*;

const A4: PageSize = PageSize {
    width_mm: 210.0,
    height_mm: 297.0,
};
const DPI: f64 = 100.0;

/// A blank page, and a brush for putting printed ink on it.
struct Sheet {
    gray: Vec<u8>,
    width: usize,
}

impl Sheet {
    fn blank() -> Sheet {
        let px_per_mm = DPI / 25.4;
        let width = (A4.width_mm * px_per_mm) as usize;
        let height = (A4.height_mm * px_per_mm) as usize;
        Sheet {
            gray: vec![255; width * height],
            width,
        }
    }

    /// Ink in a box given in millimetres.
    fn ink(&mut self, x0: f64, y0: f64, x1: f64, y1: f64) -> &mut Sheet {
        let px_per_mm = DPI / 25.4;
        let at = |mm: f64| (mm * px_per_mm).round().max(0.0) as usize;
        let height = self.gray.len() / self.width;
        for y in at(y0)..at(y1).min(height) {
            for x in at(x0)..at(x1).min(self.width) {
                self.gray[y * self.width + x] = 0;
            }
        }
        self
    }

    fn find(&self, options: &BlankOptions) -> Vec<Blank> {
        super::find(&self.gray, self.width, DPI, A4, options)
    }
}

/// The thing this exists for: "Name:" printed on a form, and six centimetres
/// of nothing after it that somebody has to find with a ruler.
#[test]
fn the_gap_after_a_label_is_found_and_reported_in_millimetres() {
    let mut sheet = Sheet::blank();
    sheet.ink(20.0, 60.0, 45.0, 65.0); // "Name:"
    let blanks = sheet.find(&BlankOptions::default());

    let beside = blanks
        .iter()
        .find(|b| b.beside_text)
        .expect("no gap beside the label was found");
    // It starts at the label's right edge, not at the margin. Within a pixel:
    // at this resolution one is a quarter of a millimetre, and the edge of the
    // ink falls wherever inside it that lands.
    assert!((beside.x_mm - 45.0).abs() < 0.5, "{beside:?}");
    // And runs to the far margin.
    assert!(beside.width_mm > 150.0, "{beside:?}");
    // On the label's own baseline, so what goes in it sits level with it.
    assert!((beside.y_mm - 65.0).abs() < 1.0, "{beside:?}");
}

/// A gap between two printed things is as useful as one after the last of
/// them — a form with a box in the middle of a line is an ordinary form.
#[test]
fn a_gap_between_two_printed_things_is_found_too() {
    let mut sheet = Sheet::blank();
    sheet.ink(20.0, 60.0, 45.0, 65.0);
    sheet.ink(150.0, 60.0, 190.0, 65.0);
    let blanks = sheet.find(&BlankOptions::default());

    let middle = blanks
        .iter()
        .find(|b| b.beside_text && b.x_mm > 44.0 && b.x_mm < 60.0)
        .expect("the gap between them was not found");
    assert!(middle.width_mm > 90.0 && middle.width_mm < 110.0, "{middle:?}");
}

/// Below the last line there is usually most of a page, and it is a perfectly
/// good place to write.
#[test]
fn the_empty_part_of_the_page_is_offered_as_one_place() {
    let mut sheet = Sheet::blank();
    sheet.ink(20.0, 30.0, 190.0, 35.0);
    let blanks = sheet.find(&BlankOptions::default());

    let open = blanks
        .iter()
        .find(|b| !b.beside_text && b.y_mm > 100.0)
        .expect("the empty page below was not offered");
    assert!(open.height_mm > 200.0, "{open:?}");
    // The baseline sits inside the band rather than on its bottom edge, so
    // what is written lands on the paper and not past it.
    assert!(open.y_mm < 297.0 - BlankOptions::default().margin_mm, "{open:?}");
}

/// The gaps between words on a line of prose are not places to write, and
/// reporting them would bury the one gap that is.
#[test]
fn the_spaces_between_words_are_not_offered_as_blanks() {
    let mut sheet = Sheet::blank();
    // Six "words" with ordinary spaces between them, right across the page.
    let mut x = 20.0;
    while x < 185.0 {
        sheet.ink(x, 60.0, x + 22.0, 65.0);
        x += 26.0;
    }
    let blanks = sheet.find(&BlankOptions::default());
    assert!(
        !blanks.iter().any(|b| b.beside_text),
        "four-millimetre word spaces were offered as somewhere to write: {blanks:?}"
    );
}

/// Nothing is offered in the border a printer cannot reach.
#[test]
fn the_unprintable_border_is_not_offered() {
    let sheet = Sheet::blank();
    let options = BlankOptions {
        margin_mm: 10.0,
        ..Default::default()
    };
    for blank in sheet.find(&options) {
        assert!(blank.x_mm >= 9.9, "{blank:?}");
        assert!(blank.x_mm + blank.width_mm <= 200.1, "{blank:?}");
        assert!(blank.y_mm <= 287.1, "{blank:?}");
    }
}

/// A gap beside a label comes before the empty half of the page, however much
/// bigger the empty half is. The form is asking for one and not the other.
#[test]
fn the_places_the_form_asks_about_come_before_the_ones_it_does_not() {
    let mut sheet = Sheet::blank();
    sheet.ink(20.0, 60.0, 45.0, 65.0);
    let blanks = sheet.find(&BlankOptions::default());
    assert!(blanks.len() >= 2, "{blanks:?}");
    assert!(blanks[0].beside_text, "an open area was listed first: {blanks:?}");
    // And the open areas really are wider, so this is not width in disguise.
    assert!(
        blanks.iter().any(|b| !b.beside_text && b.width_mm > blanks[0].width_mm),
        "{blanks:?}"
    );
}

/// The widest is first, because the place with the most room is the one being
/// looked for and page order buries it.
#[test]
fn the_roomiest_place_is_listed_first() {
    let mut sheet = Sheet::blank();
    sheet.ink(20.0, 60.0, 120.0, 65.0); // leaves a narrow gap to its right
    sheet.ink(20.0, 90.0, 40.0, 95.0); // leaves a wide one
    let blanks = sheet.find(&BlankOptions::default());
    assert!(blanks.len() >= 2);
    assert!(
        blanks[0].width_mm >= blanks[1].width_mm,
        "{:?}",
        &blanks[..2]
    );
}

#[test]
fn a_gap_says_what_would_fit_in_it_rather_than_only_how_wide_it_is() {
    let blank = Blank {
        x_mm: 50.0,
        y_mm: 65.0,
        width_mm: 100.0,
        // The ink of an ordinary eleven-point line: capitals about seven tenths
        // of the type size, which is 2.7 mm.
        height_mm: 2.7,
        beside_text: true,
    };
    assert_eq!(blank.placement(), "50,65");
    // And what is offered is the size of the line it would sit beside.
    assert!(blank.fits_pt() > 9.0 && blank.fits_pt() < 13.0, "{}", blank.fits_pt());
    // A hundred millimetres at that size is a good few words.
    assert!(blank.fits_characters() > 30, "{}", blank.fits_characters());
    assert!(blank.describe().contains("50,65"), "{}", blank.describe());
}

/// A very tall empty area does not offer to write in seventy-point type.
#[test]
fn the_size_offered_stays_within_reason() {
    let huge = Blank {
        x_mm: 20.0,
        y_mm: 200.0,
        width_mm: 170.0,
        height_mm: 200.0,
        beside_text: false,
    };
    assert!(huge.fits_pt() <= 24.0, "{}", huge.fits_pt());

    let sliver = Blank {
        x_mm: 20.0,
        y_mm: 60.0,
        width_mm: 100.0,
        height_mm: 0.5,
        beside_text: true,
    };
    // A hairline of ink is a rule, not type, and nothing readable is smaller
    // than six point.
    assert!(sliver.fits_pt() >= 6.0, "{}", sliver.fits_pt());
}

#[test]
fn nothing_to_look_at_finds_nothing_rather_than_panicking() {
    assert!(find(&[], 0, DPI, A4, &BlankOptions::default()).is_empty());
    assert!(find(&[255; 100], 10, 0.0, A4, &BlankOptions::default()).is_empty());
    // A page smaller than its own margins.
    let tiny = PageSize::new(4.0, 4.0);
    assert!(find(&[255; 100], 10, DPI, tiny, &BlankOptions::default()).is_empty());
}
