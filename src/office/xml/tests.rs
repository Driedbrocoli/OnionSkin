use super::*;

/// Every event a piece of XML produces, for comparing against by hand.
fn events(text: &str) -> Vec<Event> {
    Reader::new(text).collect()
}

/// Just the text, which is what most of the callers want.
fn text_of(text: &str) -> String {
    Reader::new(text)
        .filter_map(|event| match event {
            Event::Text(found) => Some(found),
            _ => None,
        })
        .collect()
}

#[test]
fn a_prefix_is_not_part_of_the_name() {
    let found = events("<w:p><w:r>hello</w:r></w:p>");
    let Event::Start(tag) = &found[0] else {
        panic!("expected a start, got {found:?}");
    };
    assert_eq!(tag.name, "p");
    assert_eq!(tag.prefix, "w");
    assert_eq!(found.last(), Some(&Event::End("p".to_string())));
}

#[test]
fn a_tag_that_closes_itself_starts_and_ends() {
    let found = events("<w:br/>");
    assert_eq!(found.len(), 2);
    let Event::Start(tag) = &found[0] else {
        panic!("expected a start");
    };
    assert!(tag.empty);
    assert_eq!(found[1], Event::End("br".to_string()));
}

#[test]
fn attributes_come_back_by_their_local_name() {
    let found = events("<w:sz w:val='36' xml:space=\"preserve\" bare/>");
    let Event::Start(tag) = &found[0] else {
        panic!("expected a start");
    };
    assert_eq!(tag.get("val"), Some("36"));
    assert_eq!(tag.number("val"), Some(36.0));
    assert_eq!(tag.get("space"), Some("preserve"));
    assert_eq!(tag.get("bare"), Some(""));
    assert_eq!(tag.get("nothing"), None);
}

#[test]
fn a_flag_is_on_unless_it_says_otherwise() {
    let on = |markup: &str| {
        let found = events(markup);
        let Event::Start(tag) = &found[0] else {
            panic!("expected a start");
        };
        tag.on()
    };
    assert!(on("<w:b/>"));
    assert!(on("<w:b w:val=\"1\"/>"));
    assert!(on("<w:b w:val=\"true\"/>"));
    assert!(!on("<w:b w:val=\"0\"/>"));
    assert!(!on("<w:b w:val=\"false\"/>"));
    assert!(!on("<w:b w:val=\"off\"/>"));
}

#[test]
fn the_five_entities_come_back_as_characters() {
    assert_eq!(
        text_of("<t>Smith &amp; Sons &lt;Ltd&gt; said &quot;yes&quot; &apos;now&apos;</t>"),
        "Smith & Sons <Ltd> said \"yes\" 'now'"
    );
}

#[test]
fn numeric_entities_come_back_as_characters() {
    // The last one is a combining accent, which is its own character and not
    // the same string as the letter it sits over.
    assert_eq!(
        text_of("<t>caf&#233; &#x2014; a&#x300;</t>"),
        "caf\u{e9} \u{2014} a\u{300}"
    );
}

#[test]
fn an_entity_nobody_defined_is_left_as_it_was_written() {
    // Better than dropping it: whatever it was, it is still visible.
    assert_eq!(text_of("<t>10&nbsp;km &amp; more</t>"), "10&nbsp;km & more");
}

#[test]
fn a_bare_ampersand_does_not_eat_the_rest_of_the_line() {
    assert_eq!(text_of("<t>Fish & chips</t>"), "Fish & chips");
}

/// A bare ampersand near an accented letter used to bring the whole program
/// down.
///
/// The search for the `;` that ends an entity was bounded at twelve *bytes*,
/// and slicing a string at a byte offset panics outright if that offset is
/// inside a character. `&` then eleven bytes then anything non-ASCII put the
/// cut in the middle of it. Nothing exotic: a French or German name in a Word
/// document, with an `&` a dozen characters before it.
#[test]
fn an_ampersand_near_an_accented_letter_does_not_bring_it_down() {
    // The `€` begins at byte 11 and runs to byte 13, so a twelve-byte cut
    // lands inside it.
    assert_eq!(text_of("<t>&abcdefghij€</t>"), "&abcdefghij€");
    // The same again for every width of character, and at every offset it
    // could be cut at, so no single position is what the test rests on.
    for filler in 0..16 {
        for tail in ["é", "€", "—", "𝄞", "日本語"] {
            let body = format!("<t>&{}{tail} and more</t>", "x".repeat(filler));
            let expected = format!("&{}{tail} and more", "x".repeat(filler));
            assert_eq!(text_of(&body), expected, "filler {filler}, tail {tail}");
        }
    }
    // And a real entity is still read when an accented letter follows it.
    assert_eq!(text_of("<t>caf&#233; &amp; th&#233;</t>"), "café & thé");
}

#[test]
fn an_attribute_value_may_hold_the_end_of_a_tag() {
    let found = events("<w:t w:val=\"a > b\">body</w:t>");
    let Event::Start(tag) = &found[0] else {
        panic!("expected a start");
    };
    assert_eq!(tag.get("val"), Some("a > b"));
    assert_eq!(found[1], Event::Text("body".to_string()));
}

#[test]
fn comments_and_declarations_are_not_events() {
    let found =
        events("<?xml version=\"1.0\"?><!DOCTYPE thing><!-- a note --><t>x</t><!-- another -->");
    assert_eq!(
        found,
        vec![
            Event::Start(Tag {
                name: "t".into(),
                prefix: String::new(),
                attrs: Vec::new(),
                empty: false,
            }),
            Event::Text("x".into()),
            Event::End("t".into()),
        ]
    );
}

#[test]
fn a_thousand_comments_in_a_row_do_not_run_out_of_stack() {
    let markup = format!("{}<t>end</t>", "<!-- nothing to see -->".repeat(50_000));
    assert_eq!(text_of(&markup), "end");
}

#[test]
fn cdata_is_text() {
    assert_eq!(
        text_of("<t><![CDATA[a < b & c > d]]>!</t>"),
        "a < b & c > d!"
    );
}

#[test]
fn skipping_an_element_counts_its_own_kind() {
    // A paragraph inside a paragraph is what every text box in every Word
    // document looks like. Stopping at the first end tag would put the rest of
    // the document inside the box.
    let mut reader =
        Reader::new("<body><w:p>outer<w:p>inner</w:p>still outer</w:p><w:t>after</w:t></body>");
    // Walk to the start of the outer paragraph.
    for event in reader.by_ref() {
        if matches!(&event, Event::Start(tag) if tag.name == "p") {
            break;
        }
    }
    reader.skip_element("p");

    let rest: Vec<Event> = reader.collect();
    let Some(Event::Start(tag)) = rest.first() else {
        panic!("expected the tag after the paragraph, got {rest:?}");
    };
    assert_eq!(tag.name, "t");
    assert_eq!(rest[1], Event::Text("after".into()));
}

#[test]
fn a_truncated_file_stops_rather_than_spinning() {
    let found = events("<w:p><w:r><w:t>half a docum");
    assert!(found
        .iter()
        .any(|event| event == &Event::Text("half a docum".into())));
    let found = events("<w:p><w:r unfinished=");
    assert!(!found.is_empty());
}

#[test]
fn utf16_is_read_as_well_as_utf8() {
    let mut bytes = vec![0xFF, 0xFE];
    for unit in "<t>héllo</t>".encode_utf16() {
        bytes.extend_from_slice(&unit.to_le_bytes());
    }
    assert_eq!(text_of(&decode(&bytes)), "héllo");

    let mut big = vec![0xFE, 0xFF];
    for unit in "<t>héllo</t>".encode_utf16() {
        big.extend_from_slice(&unit.to_be_bytes());
    }
    assert_eq!(text_of(&decode(&big)), "héllo");

    assert_eq!(decode("plain".as_bytes()), "plain");
    assert_eq!(decode("\u{feff}plain".as_bytes()), "\u{feff}plain");
}

#[test]
fn a_byte_order_mark_is_not_a_tag() {
    assert_eq!(text_of("\u{feff}<t>x</t>"), "x");
}

#[test]
fn local_strips_whatever_is_before_the_colon() {
    assert_eq!(local("w:tblPr"), "tblPr");
    assert_eq!(local("tblPr"), "tblPr");
    assert_eq!(local(""), "");
}

#[test]
fn whitespace_between_tags_is_kept() {
    // Word writes `<w:t xml:space="preserve"> </w:t>`, and that space is a
    // word gap somebody typed.
    assert_eq!(text_of("<w:t xml:space=\"preserve\"> </w:t>"), " ");
}
