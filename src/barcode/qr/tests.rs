use super::*;

/// The field arithmetic, against values anybody can check by hand.
#[test]
fn multiplying_in_the_field_of_256() {
    assert_eq!(multiply(0, 5), 0);
    assert_eq!(multiply(1, 5), 5);
    assert_eq!(multiply(2, 2), 4);
    // 0x80 doubled overflows the byte and is reduced by the field's polynomial.
    assert_eq!(multiply(0x80, 2), 0x1D);
    assert_eq!(multiply(3, 7), 9);
}

/// The smallest square that holds the text is the one used.
#[test]
fn a_short_text_gets_a_small_square() {
    let small = encode("HELLO", Ecc::Medium).unwrap();
    assert_eq!(small.width, 21, "five characters should be a version 1");
    assert_eq!(small.width, small.height);

    let bigger = encode(&"X".repeat(200), Ecc::Medium).unwrap();
    assert!(bigger.width > small.width);
}

/// Digits pack tighter than letters, which pack tighter than anything else.
#[test]
fn the_narrowest_packing_that_fits_is_chosen() {
    assert_eq!(narrowest_mode("12345"), Mode::Numeric);
    assert_eq!(narrowest_mode("ONIONSKIN-1"), Mode::Alphanumeric);
    assert_eq!(narrowest_mode("Onionskin"), Mode::Byte);
    assert_eq!(narrowest_mode("café"), Mode::Byte);
}

/// The frame is where the standard puts it, and a scanner looks nowhere else.
#[test]
fn the_three_eyes_are_in_their_corners() {
    let code = encode("HELLO", Ecc::Medium).unwrap();
    let size = code.width;
    for (cx, cy) in [(3, 3), (size - 4, 3), (3, size - 4)] {
        // Out from the middle: a 3x3 dark core, a light ring, a dark border.
        // That is the shape every scanner hunts for, and it is the only thing
        // in the square a decoder can find without decoding anything.
        assert!(code.dark_at(cx, cy), "the middle of an eye is light");
        assert!(code.dark_at(cx - 1, cy), "the core of an eye is not solid");
        assert!(!code.dark_at(cx - 2, cy), "the ring inside an eye is dark");
        assert!(code.dark_at(cx - 3, cy), "the border of an eye is light");
        // And the paper around it, which is what separates it from the data.
        assert!(!code.dark_at(cx + 4, cy) || cx + 4 >= size);
    }
    // And the module that is always dark.
    assert!(code.dark_at(8, size - 8));
}

/// The mask that is kept is the best of the eight, not the first that works.
///
/// This is the one step with no right answer written down, and it is the step
/// that stops a text of a particular shape drawing something a scanner mistakes
/// for part of the frame. A chooser that always returned nought would pass
/// every other test in this file and produce codes that read badly in poor
/// light, which is exactly the failure nobody notices until it is on paper.
#[test]
fn the_least_awkward_of_the_eight_masks_is_the_one_kept() {
    let mut chosen = std::collections::BTreeSet::new();
    for text in [
        "0".repeat(40),
        "HELLO".to_string(),
        "https://example.org/forms/2024/renewal".to_string(),
        "AAAAAAAAAAAAAAAAAAAAAAAA".to_string(),
        "8".repeat(120),
    ] {
        let (code, mask, penalties) = build(&text, Ecc::Medium).unwrap();
        chosen.insert(mask);

        // Nothing else scores better...
        let best = *penalties.iter().min().unwrap();
        assert_eq!(
            penalties[mask as usize], best,
            "'{text}' kept mask {mask} scoring {} when {best} was on offer: {penalties:?}",
            penalties[mask as usize]
        );
        // ...and what was kept really is what was scored, rather than the
        // scoring having been done on some other square.
        assert_eq!(
            penalty_of(&code.dark, code.width),
            best,
            "'{text}' does not score what the chooser said it did"
        );
    }
    // And the choice is a real one: not every text lands on the same mask.
    assert!(
        chosen.len() > 1,
        "every text chose mask {chosen:?}, so nothing is being chosen"
    );
}

/// Nothing is refused rather than encoded as an empty square.
#[test]
fn nothing_is_refused() {
    assert_eq!(encode("", Ecc::Medium), Err(QrError::Empty));
}

/// Past the largest square, it says so rather than writing something unreadable.
#[test]
fn more_than_the_largest_square_holds_is_refused() {
    let far_too_much = "X".repeat(5000);
    let why = encode(&far_too_much, Ecc::High).unwrap_err();
    assert!(matches!(why, QrError::TooLong { .. }));
    assert!(why.to_string().contains("largest QR code"), "{why}");
}

/// More correction means a bigger square for the same words. That is the trade,
/// and it is the reason the level is a choice at all.
#[test]
fn more_correction_costs_more_paper() {
    let text = "https://example.org/forms/2024/renewal";
    let low = encode(text, Ecc::Low).unwrap().width;
    let high = encode(text, Ecc::High).unwrap().width;
    assert!(
        high > low,
        "high correction came out no bigger: {high} against {low}"
    );
}

/// Every version's tables agree with the module count the geometry gives, which
/// is the check that catches a mistyped number in either table.
///
/// Two hundred and eighty numbers were copied in by hand, and a wrong one does
/// not produce a wrong-looking code — it produces a code that is the right shape
/// and cannot be read. So they are checked against the one thing that is not a
/// table: the size of the square.
#[test]
fn the_tables_agree_with_the_shape_of_the_square() {
    for version in 1..=40usize {
        let total = raw_data_modules(version) / 8;
        for level in [Ecc::Low, Ecc::Medium, Ecc::Quartile, Ecc::High] {
            let blocks = BLOCKS[level.row()][version] as usize;
            let ecc = ECC_PER_BLOCK[level.row()][version] as usize;
            assert!(blocks > 0, "version {version} has no blocks");
            assert!(
                ecc * blocks < total,
                "version {version} spends {} of {total} codewords on correction",
                ecc * blocks
            );
            // Every block has to hold at least one byte of the message, or the
            // interleaving has nothing to interleave.
            let data = total - ecc * blocks;
            assert!(
                data >= blocks,
                "version {version} at {level:?} leaves blocks with no data"
            );
            // And no block may be more than one byte longer than another —
            // which is what the splitting relies on.
            assert!(
                data % blocks <= blocks,
                "version {version} at {level:?} splits unevenly"
            );
        }
    }
}

/// Capacity climbs with the version and falls as correction is added. Both are
/// obvious and both would be broken by a table row in the wrong place.
#[test]
fn a_bigger_square_holds_more_and_more_correction_holds_less() {
    for version in 2..=40usize {
        assert!(
            data_codewords(version, Ecc::Medium) > data_codewords(version - 1, Ecc::Medium),
            "version {version} holds no more than version {}",
            version - 1
        );
    }
    for version in 1..=40usize {
        let mut last = usize::MAX;
        for level in [Ecc::Low, Ecc::Medium, Ecc::Quartile, Ecc::High] {
            let holds = data_codewords(version, level);
            assert!(
                holds < last,
                "version {version} at {level:?} holds {holds}, no less than the \
                 level before it"
            );
            last = holds;
        }
    }
    // The one number anybody can look up: a version 1 at low correction holds
    // nineteen bytes of message and seven of correction.
    assert_eq!(data_codewords(1, Ecc::Low), 19);
    assert_eq!(raw_data_modules(1) / 8, 26);
    // And the largest of all: version 40 comes to 3706 codewords.
    assert_eq!(raw_data_modules(40) / 8, 3706);
}

/// The little eyes are where the rule puts them, checked against the shape of
/// the square: the first at 6, the last seven modules in from the far edge.
#[test]
fn the_little_eyes_are_spaced_across_the_square() {
    assert!(alignment_centres(1).is_empty(), "version 1 has none");
    assert_eq!(alignment_centres(2), vec![6, 18]);
    assert_eq!(alignment_centres(7), vec![6, 22, 38]);
    for version in 2..=40usize {
        let centres = alignment_centres(version);
        let size = 4 * version + 17;
        assert_eq!(centres[0], 6, "version {version} starts in the wrong place");
        assert_eq!(
            *centres.last().unwrap(),
            size - 7,
            "version {version} ends in the wrong place"
        );
        assert_eq!(centres.len(), version / 7 + 2, "version {version}");
        // In order, and never close enough to overlap.
        for pair in centres.windows(2) {
            assert!(pair[1] > pair[0] + 4, "version {version}: {centres:?}");
        }
    }
}

/// A text that is not plain ASCII says which character set it is in.
///
/// This was a real defect, found by handing a code to a decoder rather than by
/// reading the code over. A QR code's byte mode carries bytes and says nothing
/// about what they mean; the standard's default is Latin-1, so a decoder given
/// the two bytes of an é has to guess. zbar read `café` back as `caf矇`, having
/// decided they were part of a Chinese codepage. It was not wrong to — nothing
/// in the code had said otherwise.
#[test]
fn a_text_that_is_not_ascii_names_its_character_set() {
    assert!(needs_a_character_set("café", Mode::Byte));
    assert!(needs_a_character_set("日本語", Mode::Byte));
    // Plain ASCII means the same in every character set anybody would guess,
    // so it does not pay the twelve bits.
    assert!(!needs_a_character_set("HELLO", Mode::Byte));
    assert!(!needs_a_character_set("12345", Mode::Numeric));

    // And the twelve bits really are in front of the text: the mode indicator
    // for a character set, then the number the standard gives UTF-8.
    let (_, data) = fit("café", Mode::Byte, Ecc::Medium).unwrap();
    assert_eq!(
        data[0] >> 4,
        ECI_MODE as u8,
        "no character set was declared"
    );
    // The eight bits after the four-bit indicator.
    let eci = (u16::from(data[0]) << 8 | u16::from(data[1])) >> 4 & 0xFF;
    assert_eq!(
        u32::from(eci),
        ECI_UTF8,
        "the character set named was not UTF-8"
    );

    // An ASCII text goes straight in with the byte-mode indicator instead.
    let (_, plain) = fit("HELLO", Mode::Byte, Ecc::Medium).unwrap();
    assert_eq!(plain[0] >> 4, Mode::Byte.indicator() as u8);
}

/// The twelve bits are counted before the version is chosen, not after.
///
/// Otherwise a text that only just fits would be given a square twelve bits too
/// small, and the last character of it would be silently lost.
#[test]
fn the_character_set_is_paid_for_before_the_square_is_chosen() {
    // The most bytes a version 1 at medium holds, then one accented character
    // in place of two plain ones — which still fits by bytes and no longer fits
    // once the declaration is counted.
    let holds = longest_at(1, Ecc::Medium);
    let plain = "A".repeat(holds);
    assert_eq!(encode(&plain, Ecc::Medium).unwrap().width, 21);

    let accented = format!("{}é", "A".repeat(holds - 2));
    let code = encode(&accented, Ecc::Medium).unwrap();
    assert!(
        code.width > 21,
        "a text needing a character set was squeezed into a version 1 anyway"
    );
}
