use super::*;

#[test]
fn a_placement_is_a_position_and_some_words() {
    assert_eq!(
        parse_placement("60,150:Approved").unwrap(),
        ((60.0, 150.0), "Approved".to_string())
    );
    // Spaces round the numbers are how people type, and a colon in the words
    // is how people write times.
    assert_eq!(
        parse_placement(" 20 , 40 :Due at 9:30").unwrap(),
        ((20.0, 40.0), "Due at 9:30".to_string())
    );
}

#[test]
fn a_placement_with_nothing_in_it_is_refused() {
    for bad in ["60,150:", "60,150", "x,150:Hello", "60:Hello"] {
        assert!(
            parse_placement(bad).is_err(),
            "'{bad}' should not have been accepted"
        );
    }
}

/// The anchor takes the first colon and the words keep the rest, so "Due:at
/// 9:30" is an anchor of "Due" and words of "at 9:30" — not the other way
/// about, which would look for something no page has on it.
#[test]
fn an_anchor_takes_the_first_colon_and_the_words_keep_the_rest() {
    assert_eq!(
        split_anchor("Received:Approved").unwrap(),
        ("Received".to_string(), "Approved".to_string())
    );
    assert_eq!(
        split_anchor("Due:at 9:30").unwrap(),
        ("Due".to_string(), "at 9:30".to_string())
    );
}

#[test]
fn an_anchor_missing_either_half_is_refused() {
    for bad in ["Received", "Received:", ":Approved", ":"] {
        assert!(
            split_anchor(bad).is_err(),
            "'{bad}' should not have been accepted"
        );
    }
}

#[test]
fn the_two_escapes_worth_having_are_understood() {
    assert_eq!(unescape("one\\ntwo"), "one\ntwo");
    assert_eq!(unescape("a\\tb"), "a\tb");
    // A backslash before anything else is left alone: a Windows path in the
    // middle of a sentence should survive being written down.
    assert_eq!(unescape("C:\\Users\\me"), "C:\\Users\\me");
}

/// A recipe that asks for nothing has to say so, or the composer is handed an
/// empty page and reports that the two documents render identically — which is
/// a diagnosis for a different question.
#[test]
fn a_recipe_that_asks_for_nothing_knows_it() {
    assert!(Recipe::default().is_empty());
    let one = Recipe {
        at: vec!["20,40:Hello".into()],
        ..Default::default()
    };
    assert!(!one.is_empty());
}

/// Reading the page is the slow part, and it is only needed for the two
/// placements that match against what is printed. A recipe of millimetres and
/// pictures must not pay for it.
#[test]
fn only_anchored_placements_make_the_page_worth_reading() {
    let measured = Recipe {
        at: vec!["20,40:Hello".into()],
        images: vec!["sign.png:10,10:30".into()],
        ..Default::default()
    };
    assert!(!measured.needs_reading());

    for anchored in [
        Recipe {
            after: vec!["Received:Yes".into()],
            ..Default::default()
        },
        Recipe {
            below: vec!["Signature:Me".into()],
            ..Default::default()
        },
    ] {
        assert!(anchored.needs_reading());
    }
}

/// What an anchor matched is shown before anything is printed, whole line and
/// all — because an anchor is a guess, and a page with two "Total"s on it is
/// how somebody stamps the wrong one.
#[test]
fn what_an_anchor_matched_is_reported_with_the_line_it_was_on() {
    let found = Found {
        anchor: "Total".into(),
        line: "Total: £420.00".into(),
        x_mm: 45.25,
        y_mm: 150.0,
    };
    let said = found.describe();
    assert!(said.contains("Total: £420.00"), "{said}");
    assert!(said.contains("45.2"), "{said}");
    assert!(said.contains("150.0"), "{said}");
}

/// Millimetre placements need no document at all, so laying them out must not
/// touch one — a recipe of positions works on a path that has not been opened.
#[test]
fn placements_in_millimetres_are_laid_out_without_reading_anything() {
    let recipe = Recipe {
        at: vec!["20,40:First".into(), "20,60:Second".into()],
        page: 2,
        size_pt: 9.0,
        font: "Times-Roman".into(),
        colour: "#112233".into(),
        leading: 1.4,
        ..Default::default()
    };
    let laid = lay_out(&recipe, std::path::Path::new("/no/such/document.pdf")).unwrap();

    assert_eq!(laid.items.len(), 2);
    assert!(laid.found.is_empty(), "nothing was anchored");
    assert_eq!(laid.items[0].x_mm, 20.0);
    assert_eq!(laid.items[1].y_mm, 60.0);
    // How it is set comes from the recipe, once, for every placement in it.
    for item in &laid.items {
        assert_eq!(item.page, 2);
        assert_eq!(item.size_pt, 9.0);
        assert_eq!(item.font, "Times-Roman");
        assert_eq!(item.colour, "#112233");
        assert_eq!(item.leading, 1.4);
    }
}

/// A bad placement stops the whole thing rather than laying out the good ones
/// and dropping this. Half a delta is worse than none: it prints.
#[test]
fn one_bad_placement_stops_the_lot() {
    let recipe = Recipe {
        at: vec!["20,40:Fine".into(), "not a placement".into()],
        ..Default::default()
    };
    assert!(lay_out(&recipe, std::path::Path::new("/no/such.pdf")).is_err());
}

// Moved here from the command-line binary along with the code they test.
// A picture spec is parsed by the library now, because the window places
// pictures too, and a test that lives beside a copy tests the copy.

#[test]
fn a_picture_is_read_from_the_end_so_a_windows_path_still_works() {
    // The file name comes first and may hold colons of its own, so the
    // two parts that matter are found from the end.
    let spec = parse_image("signature.png:120,240:40").unwrap();
    assert_eq!(spec.path, PathBuf::from("signature.png"));
    assert_eq!((spec.x_mm, spec.y_mm), (120.0, 240.0));
    assert_eq!((spec.width_mm, spec.height_mm), (Some(40.0), None));

    let spec = parse_image(r"C:\scans\sign.png:10,20:30").unwrap();
    assert_eq!(spec.path, PathBuf::from(r"C:\scans\sign.png"));

    // Both measurements, when somebody wants the box exactly.
    let spec = parse_image("s.png:10,20:40x15").unwrap();
    assert_eq!((spec.width_mm, spec.height_mm), (Some(40.0), Some(15.0)));
}

#[test]
fn a_picture_with_no_size_or_a_silly_one_is_refused_by_name() {
    for bad in [
        "signature.png",
        "signature.png:120,240",
        ":120,240:40",
        "s.png:120:40",
        "s.png:a,b:40",
        "s.png:10,20:wide",
    ] {
        assert!(parse_image(bad).is_err(), "{bad} was accepted");
    }
    for silly in ["s.png:10,20:0", "s.png:10,20:-5"] {
        let said = parse_image(silly).unwrap_err();
        assert!(said.contains("greater than nothing"), "{said}");
    }
}

#[test]
fn the_measurement_left_out_follows_the_pictures_own_shape() {
    // A signature squashed into a box it was not drawn for is worse than
    // no signature at all.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("wide.png");
    // Four across, one down: four times as wide as it is tall.
    let mut img = image::RgbImage::new(4, 1);
    for pixel in img.pixels_mut() {
        *pixel = image::Rgb([0, 0, 0]);
    }
    img.save(&path).unwrap();

    let spec = format!("{}:10,20:40", path.to_str().unwrap());
    let placed = placed_images(&[spec], 1).unwrap();
    assert_eq!(placed.len(), 1);
    let image = &placed[0].1;
    assert!((image.width_mm - 40.0).abs() < 1e-9);
    assert!((image.height_mm - 10.0).abs() < 1e-9, "{image:?}");

    // And giving only a height works the other way round.
    let spec = format!("{}:10,20:x10", path.to_str().unwrap());
    let placed = placed_images(&[spec], 1).unwrap();
    assert!(
        (placed[0].1.width_mm - 40.0).abs() < 1e-9,
        "{:?}",
        placed[0].1
    );
}

#[test]
fn a_picture_that_is_not_there_says_so_rather_than_writing_a_blank_page() {
    let said = placed_images(&["nowhere.png:10,20:40".to_string()], 1).unwrap_err();
    assert!(said.contains("nowhere.png"), "{said}");
}
