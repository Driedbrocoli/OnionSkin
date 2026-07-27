//! Tests for merging deltas.
//!
//! These build real deltas with the real writer and merge the real files,
//! because the whole risk in this feature is in the PDF itself: a merged file
//! that parses but sets the second delta's words in the first delta's face
//! would pass any test that only counted pages.

use super::*;

use crate::geometry::PageSize;
use crate::pdf::{Font, LineFont, PlacedLine};

const A4: PageSize = PageSize {
    width_mm: 210.0,
    height_mm: 297.0,
};

fn line(text: &str, font: Font, x_mm: f64, y_mm: f64) -> PlacedLine {
    PlacedLine {
        text: text.to_string(),
        x_mm,
        y_mm,
        size_pt: 12.0,
        font: LineFont::Builtin(font),
        colour: (0.0, 0.0, 0.0),
        rotation_deg: 0.0,
    }
}

/// A delta of `pages` pages, each carrying one line in the given face.
fn a_delta(at: &Path, words: &str, font: Font, pages: usize) -> PathBuf {
    let sizes = vec![A4; pages];
    let lines: Vec<Vec<PlacedLine>> = (0..pages)
        .map(|page| vec![line(&format!("{words} {page}"), font, 20.0, 40.0)])
        .collect();
    crate::pdf::write_delta(at, &sizes, &lines, "test", None).unwrap();
    at.to_path_buf()
}

/// Everything drawn on a page of a written file, as the operators themselves.
///
/// Following the form references is the point: the merged page's own content
/// is three lines long, and what actually prints is inside the forms.
fn what_page_draws(path: &Path, page: usize) -> String {
    let doc = Document::load(path).unwrap();
    let ids: Vec<ObjectId> = doc.get_pages().into_values().collect();
    let page_id = ids[page];
    let mut out = String::from_utf8_lossy(&doc.get_page_content(page_id).unwrap()).into_owned();

    // …and whatever the forms it names draw, with their own font names
    // rewritten to the actual face so a swap cannot hide.
    let (resources, _) = doc.get_page_resources(page_id);
    let forms = resources
        .and_then(|dict| dict.get(b"XObject").ok())
        .and_then(|object| object.as_dict().ok())
        .cloned()
        .unwrap_or_default();
    for (_, reference) in forms.iter() {
        let Ok(id) = reference.as_reference() else {
            continue;
        };
        let Ok(stream) = doc.get_object(id).and_then(lopdf::Object::as_stream) else {
            continue;
        };
        let bytes = stream
            .decompressed_content()
            .unwrap_or_else(|_| stream.content.clone());
        let mut drawn = String::from_utf8_lossy(&bytes).into_owned();
        for (key, face) in faces_in(&doc, &stream.dict) {
            drawn = drawn.replace(&format!("/{key} "), &format!("/{face} "));
        }
        out.push('\n');
        out.push_str(&drawn);
    }
    out
}

/// The real base font behind each font name in a form's resources.
fn faces_in(doc: &Document, dict: &lopdf::Dictionary) -> Vec<(String, String)> {
    let Some(fonts) = dict
        .get(b"Resources")
        .ok()
        .and_then(|object| doc.dereference(object).ok())
        .and_then(|(_, object)| object.as_dict().ok())
        .and_then(|resources| resources.get(b"Font").ok())
        .and_then(|object| doc.dereference(object).ok())
        .and_then(|(_, object)| object.as_dict().ok())
    else {
        return Vec::new();
    };
    fonts
        .iter()
        .filter_map(|(key, reference)| {
            let (_, font) = doc.dereference(reference).ok()?;
            let base = font.as_dict().ok()?.get(b"BaseFont").ok()?;
            Some((
                String::from_utf8_lossy(key).into_owned(),
                base.as_name_str().ok()?.to_string(),
            ))
        })
        .collect()
}

// ---------------------------------------------------------------------------
// The point of the whole thing
// ---------------------------------------------------------------------------

/// One sheet, one pass, both deltas on it.
#[test]
fn two_deltas_come_out_as_one_page_carrying_both() {
    let dir = tempfile::tempdir().unwrap();
    let stamp = a_delta(&dir.path().join("stamp.pdf"), "PAID", Font::Helvetica, 1);
    let sign = a_delta(&dir.path().join("sign.pdf"), "SIGNED", Font::Courier, 1);
    let out = dir.path().join("both.pdf");

    let merged = merge(&[stamp, sign], &out, "both").unwrap();
    assert_eq!(merged.pages, 1);
    assert!(merged.page.matches(&A4, 0.1), "{:?}", merged.page);

    let drawn = what_page_draws(&out, 0);
    assert!(drawn.contains("PAID 0"), "{drawn}");
    assert!(drawn.contains("SIGNED 0"), "{drawn}");
}

/// The reason each page goes in as a form rather than as glued-on content:
/// both deltas call their first font `F0` and they mean different faces.
/// Concatenated, the second delta's words would come out in the first's face —
/// wrong, and close enough to right to go unnoticed.
#[test]
fn each_delta_keeps_its_own_typeface() {
    let dir = tempfile::tempdir().unwrap();
    let helvetica = a_delta(&dir.path().join("a.pdf"), "HELV", Font::Helvetica, 1);
    let courier = a_delta(&dir.path().join("b.pdf"), "COUR", Font::Courier, 1);
    let out = dir.path().join("both.pdf");
    merge(&[helvetica, courier], &out, "both").unwrap();

    // Both files name their font F0. In the merged file the two forms must
    // still reach different faces.
    let drawn = what_page_draws(&out, 0);
    assert!(drawn.contains("Helvetica"), "{drawn}");
    assert!(drawn.contains("Courier"), "{drawn}");
}

/// A one-page stamp onto the front of a five-page invoice is an ordinary thing
/// to want, so the short file simply stops contributing rather than failing.
#[test]
fn a_file_that_runs_out_stops_rather_than_failing() {
    let dir = tempfile::tempdir().unwrap();
    let long = a_delta(&dir.path().join("long.pdf"), "PAGE", Font::Helvetica, 3);
    let short = a_delta(&dir.path().join("short.pdf"), "ONCE", Font::Courier, 1);
    let out = dir.path().join("both.pdf");

    let merged = merge(&[long, short.clone()], &out, "both").unwrap();
    assert_eq!(merged.pages, 3, "the longest decides");

    let first = what_page_draws(&out, 0);
    assert!(first.contains("ONCE 0"), "{first}");
    let last = what_page_draws(&out, 2);
    assert!(last.contains("PAGE 2"), "{last}");
    assert!(!last.contains("ONCE"), "{last}");

    // And it is said out loud, because the other reading is a wrong file.
    let short_ones: Vec<&PathBuf> = merged.short().iter().map(|from| &from.path).collect();
    assert_eq!(short_ones, vec![&short]);
}

#[test]
fn three_deltas_merge_as_readily_as_two() {
    let dir = tempfile::tempdir().unwrap();
    let files = vec![
        a_delta(&dir.path().join("a.pdf"), "ONE", Font::Helvetica, 1),
        a_delta(&dir.path().join("b.pdf"), "TWO", Font::Courier, 1),
        a_delta(&dir.path().join("c.pdf"), "THREE", Font::TimesRoman, 1),
    ];
    let out = dir.path().join("all.pdf");
    let merged = merge(&files, &out, "all").unwrap();
    assert_eq!(merged.from.len(), 3);

    let drawn = what_page_draws(&out, 0);
    for words in ["ONE 0", "TWO 0", "THREE 0"] {
        assert!(drawn.contains(words), "{words} missing from {drawn}");
    }
}

// ---------------------------------------------------------------------------
// What is refused, and what is merely reported
// ---------------------------------------------------------------------------

/// Merging a letter's delta with an invoice's would print one of them off the
/// edge of the paper. Asked before the paper goes in, not after.
#[test]
fn deltas_for_different_paper_are_refused_with_both_sizes() {
    let dir = tempfile::tempdir().unwrap();
    let a4 = a_delta(&dir.path().join("a4.pdf"), "A4", Font::Helvetica, 1);
    let letter = dir.path().join("letter.pdf");
    crate::pdf::write_delta(
        &letter,
        &[PageSize::new(215.9, 279.4)],
        &[vec![line("US", Font::Helvetica, 20.0, 40.0)]],
        "letter",
        None,
    )
    .unwrap();

    let out = dir.path().join("both.pdf");
    let said = merge(&[a4, letter], &out, "both").unwrap_err().to_string();
    assert!(said.contains("A4"), "{said}");
    assert!(said.contains("Letter") || said.contains("215.9"), "{said}");
    assert!(said.contains("off the edge"), "{said}");
    assert!(!out.exists(), "a refused merge still wrote a file");
}

/// Reading every file before writing anything: a mismatch found in the last
/// file must not leave half a merged document behind.
#[test]
fn nothing_is_written_until_every_file_has_been_read() {
    let dir = tempfile::tempdir().unwrap();
    let good = a_delta(&dir.path().join("good.pdf"), "OK", Font::Helvetica, 1);
    let missing = dir.path().join("not-here.pdf");
    let out = dir.path().join("both.pdf");

    assert!(merge(&[good, missing], &out, "both").is_err());
    assert!(!out.exists(), "a failed merge left a file behind");
}

#[test]
fn merging_one_file_is_refused_because_there_is_nothing_to_merge() {
    let dir = tempfile::tempdir().unwrap();
    let only = a_delta(&dir.path().join("only.pdf"), "ONE", Font::Helvetica, 1);
    let out = dir.path().join("out.pdf");
    let said = merge(&[only], &out, "out").unwrap_err().to_string();
    assert!(said.contains("at least two"), "{said}");
}

/// The same delta twice puts every letter down twice in the same place: a
/// little heavier, a little blurred, and never what anybody meant. Not refused
/// — there is no harm in it beyond the ink — but said.
#[test]
fn the_same_delta_given_twice_is_noticed() {
    let dir = tempfile::tempdir().unwrap();
    let one = a_delta(&dir.path().join("one.pdf"), "PAID", Font::Helvetica, 1);
    let copy = dir.path().join("copy.pdf");
    std::fs::copy(&one, &copy).unwrap();
    let out = dir.path().join("both.pdf");

    let merged = merge(&[one.clone(), copy], &out, "both").unwrap();
    let repeats = merged.repeats();
    assert_eq!(repeats.len(), 1, "{:?}", merged.from);
    assert_eq!(repeats[0].same_as.as_ref(), Some(&one));
    assert!(
        merged.describe().contains("the same file as"),
        "{}",
        merged.describe()
    );

    // Two different deltas are not reported as repeats.
    let a = a_delta(&dir.path().join("a.pdf"), "A", Font::Helvetica, 1);
    let b = a_delta(&dir.path().join("b.pdf"), "B", Font::Helvetica, 1);
    let plain = merge(&[a, b], &dir.path().join("ab.pdf"), "ab").unwrap();
    assert!(plain.repeats().is_empty(), "{:?}", plain.from);
}

// ---------------------------------------------------------------------------
// PDFs that are not ours
// ---------------------------------------------------------------------------

/// A page box that does not start at zero is legal and does happen. The same
/// sheet of paper, measured from a different corner, must not shift the words
/// — and must not be refused either, because it is only arithmetic.
#[test]
fn a_page_box_that_starts_somewhere_else_is_corrected_not_refused() {
    let dir = tempfile::tempdir().unwrap();
    let plain = a_delta(&dir.path().join("plain.pdf"), "PLAIN", Font::Helvetica, 1);

    // The same delta, with its box moved to start at (10, 20) — so its own
    // content is 10 pt right and 20 pt up from where the merged page has it.
    let shifted = dir.path().join("shifted.pdf");
    {
        let mut doc = Document::load(&plain).unwrap();
        let page_id = doc.get_pages().into_values().next().unwrap();
        let box_pt = vec![
            Object::Real(10.0),
            Object::Real(20.0),
            Object::Real(10.0 + A4.width_pt() as f32),
            Object::Real(20.0 + A4.height_pt() as f32),
        ];
        doc.get_dictionary_mut(page_id)
            .unwrap()
            .set("MediaBox", box_pt);
        doc.save(&shifted).unwrap();
    }

    let out = dir.path().join("both.pdf");
    let merged = merge(&[plain, shifted], &out, "both").unwrap();
    assert_eq!(merged.pages, 1);

    // The merged page keeps the first file's box, and the second's form is
    // shifted back onto it.
    let doc = Document::load(&out).unwrap();
    let page_id = doc.get_pages().into_values().next().unwrap();
    let (resources, _) = doc.get_page_resources(page_id);
    let forms = resources
        .and_then(|dict| dict.get(b"XObject").ok())
        .and_then(|object| object.as_dict().ok())
        .cloned()
        .unwrap();
    let matrices: Vec<Option<Vec<f64>>> = forms
        .iter()
        .map(|(_, reference)| {
            let id = reference.as_reference().unwrap();
            let stream = doc.get_object(id).unwrap().as_stream().unwrap();
            stream.dict.get(b"Matrix").ok().map(|object| {
                object
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|number| as_number(&doc, number).unwrap())
                    .collect()
            })
        })
        .collect();
    // One form sits where it was; the other is moved back by exactly the
    // difference between the two boxes.
    assert!(matrices.iter().any(|m| m.is_none()), "{matrices:?}");
    let moved = matrices
        .iter()
        .flatten()
        .find(|m| m[4] != 0.0 || m[5] != 0.0)
        .expect("nothing was moved");
    assert!((moved[4] + 10.0).abs() < 1e-4, "{moved:?}");
    assert!((moved[5] + 20.0).abs() < 1e-4, "{moved:?}");
}

/// A page turned by the reader, merged with one that is not, lands sideways.
/// Refused, saying which is which, rather than printed.
#[test]
fn pages_that_are_not_the_same_way_up_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let upright = a_delta(&dir.path().join("upright.pdf"), "UP", Font::Helvetica, 1);
    let turned = dir.path().join("turned.pdf");
    {
        let mut doc = Document::load(&upright).unwrap();
        let page_id = doc.get_pages().into_values().next().unwrap();
        doc.get_dictionary_mut(page_id).unwrap().set("Rotate", 90);
        doc.save(&turned).unwrap();
    }

    let out = dir.path().join("both.pdf");
    let said = merge(&[upright, turned], &out, "both")
        .unwrap_err()
        .to_string();
    assert!(said.contains("90°"), "{said}");
    assert!(said.contains("sideways"), "{said}");
}

/// Both turned the same way is not a mismatch, and the merged page has to be
/// turned too or it prints across the sheet.
#[test]
fn two_pages_turned_the_same_way_keep_their_turn() {
    let dir = tempfile::tempdir().unwrap();
    let mut turned = Vec::new();
    for (index, words) in ["ONE", "TWO"].iter().enumerate() {
        let plain = a_delta(
            &dir.path().join(format!("p{index}.pdf")),
            words,
            Font::Helvetica,
            1,
        );
        let mut doc = Document::load(&plain).unwrap();
        let page_id = doc.get_pages().into_values().next().unwrap();
        doc.get_dictionary_mut(page_id).unwrap().set("Rotate", 270);
        doc.save(&plain).unwrap();
        turned.push(plain);
    }

    let out = dir.path().join("both.pdf");
    merge(&turned, &out, "both").unwrap();

    let doc = Document::load(&out).unwrap();
    let page_id = doc.get_pages().into_values().next().unwrap();
    let rotate = doc
        .get_dictionary(page_id)
        .unwrap()
        .get(b"Rotate")
        .unwrap()
        .as_i64()
        .unwrap();
    assert_eq!(rotate, 270);
}

/// A page that keeps its size and resources on the page tree rather than on
/// the page is perfectly ordinary, and both have to be found there.
#[test]
fn a_page_that_inherits_its_size_from_the_tree_still_merges() {
    let dir = tempfile::tempdir().unwrap();
    let plain = a_delta(&dir.path().join("plain.pdf"), "INHERIT", Font::Helvetica, 1);
    let moved_up = dir.path().join("inherited.pdf");
    {
        let mut doc = Document::load(&plain).unwrap();
        let page_id = doc.get_pages().into_values().next().unwrap();
        let page = doc.get_dictionary(page_id).unwrap();
        let parent = page.get(b"Parent").unwrap().as_reference().unwrap();
        let media = page.get(b"MediaBox").unwrap().clone();
        let resources = page.get(b"Resources").unwrap().clone();

        let page = doc.get_dictionary_mut(page_id).unwrap();
        page.remove(b"MediaBox");
        page.remove(b"Resources");
        let tree = doc.get_dictionary_mut(parent).unwrap();
        tree.set("MediaBox", media);
        tree.set("Resources", resources);
        doc.save(&moved_up).unwrap();
    }

    let other = a_delta(&dir.path().join("other.pdf"), "OTHER", Font::Courier, 1);
    let out = dir.path().join("both.pdf");
    let merged = merge(&[moved_up, other], &out, "both").unwrap();
    assert!(merged.page.matches(&A4, 0.1), "{:?}", merged.page);

    let drawn = what_page_draws(&out, 0);
    assert!(drawn.contains("INHERIT 0"), "{drawn}");
    assert!(drawn.contains("OTHER 0"), "{drawn}");
    // The inherited font came across with it.
    assert!(drawn.contains("Helvetica"), "{drawn}");
}

/// A signature is a picture with a see-through background, and it has to come
/// across whole: the pixels, and the transparency mask that is a second object
/// the picture points at. Get the renumbering wrong and the mask is lost, which
/// prints the signature inside a white box covering the line it sits on.
#[test]
fn a_picture_and_its_transparency_come_across_together() {
    use crate::picture::Picture;

    let dir = tempfile::tempdir().unwrap();
    let with_picture = dir.path().join("signed.pdf");
    let placed = crate::pdf::PlacedImage {
        picture: Picture::Samples {
            width: 4,
            height: 2,
            rgb: vec![10; 4 * 2 * 3],
            alpha: Some(vec![0, 64, 128, 255, 255, 128, 64, 0]),
        },
        x_mm: 40.0,
        y_mm: 150.0,
        width_mm: 60.0,
        height_mm: 30.0,
        rotation_deg: 0.0,
    };
    crate::pdf::write_page_content_with_pictures(
        &with_picture,
        &[A4],
        &[Vec::new()],
        &[],
        &[vec![placed]],
        "signature",
        None,
    )
    .unwrap();

    let words = a_delta(&dir.path().join("words.pdf"), "PAID", Font::Helvetica, 1);
    let out = dir.path().join("both.pdf");
    merge(&[with_picture, words], &out, "both").unwrap();

    // The picture is in the merged file, and so is the mask it points at —
    // which is the object that would go missing if renumbering were wrong.
    let doc = Document::load(&out).unwrap();
    let mut pictures = 0;
    let mut masks = 0;
    for object in doc.objects.values() {
        let Object::Stream(stream) = object else {
            continue;
        };
        if stream
            .dict
            .get(b"Subtype")
            .and_then(Object::as_name_str)
            .ok()
            != Some("Image")
        {
            continue;
        }
        pictures += 1;
        if let Ok(mask) = stream.dict.get(b"SMask").and_then(Object::as_reference) {
            // Not merely a number: the object it names has to be there too.
            assert!(
                doc.get_object(mask).is_ok(),
                "the transparency mask was renumbered to nothing"
            );
            masks += 1;
        }
    }
    assert_eq!(pictures, 2, "one picture and one mask, both images");
    assert_eq!(masks, 1, "the transparency was lost");

    // And the words are still there beside it.
    assert!(what_page_draws(&out, 0).contains("PAID 0"));
}

/// A page that does not say what size paper it is for is refused, not skipped.
///
/// Skipping it would drop that delta's ink without saying so — and if it were
/// not the last page, it would leave a hole that shifts every later page up
/// one, so page three of the invoice would get page four's additions.
#[test]
fn a_page_with_no_size_anywhere_is_refused_rather_than_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let plain = a_delta(&dir.path().join("plain.pdf"), "PLAIN", Font::Helvetica, 2);
    let sizeless = dir.path().join("sizeless.pdf");
    {
        let mut doc = Document::load(&plain).unwrap();
        // Off the page and off the tree above it, so there is nowhere left to
        // inherit it from.
        let ids: Vec<ObjectId> = doc.get_pages().into_values().collect();
        let parent = doc
            .get_dictionary(ids[0])
            .unwrap()
            .get(b"Parent")
            .unwrap()
            .as_reference()
            .unwrap();
        for id in &ids {
            doc.get_dictionary_mut(*id).unwrap().remove(b"MediaBox");
        }
        doc.get_dictionary_mut(parent).unwrap().remove(b"MediaBox");
        doc.save(&sizeless).unwrap();
    }

    let other = a_delta(&dir.path().join("other.pdf"), "OTHER", Font::Courier, 2);
    let out = dir.path().join("both.pdf");
    let said = merge(&[other, sizeless], &out, "both")
        .unwrap_err()
        .to_string();
    assert!(said.contains("what size paper"), "{said}");
    assert!(said.contains("sizeless.pdf"), "{said}");
    assert!(!out.exists(), "a refused merge still wrote a file");
}

/// A file with no pages in it says so, rather than merging to nothing.
#[test]
fn a_file_with_no_pages_is_named() {
    let dir = tempfile::tempdir().unwrap();
    let good = a_delta(&dir.path().join("good.pdf"), "OK", Font::Helvetica, 1);
    let empty = dir.path().join("empty.pdf");
    crate::pdf::write_delta(&empty, &[], &[], "empty", None).unwrap();

    let out = dir.path().join("both.pdf");
    let said = merge(&[good, empty], &out, "both").unwrap_err().to_string();
    assert!(said.contains("no pages"), "{said}");
}

/// The merged file must survive a round trip through a reader that is not
/// lopdf, which is what actually matters — so it is rendered.
#[test]
fn the_merged_file_renders() {
    let dir = tempfile::tempdir().unwrap();
    let a = a_delta(&dir.path().join("a.pdf"), "ALPHA", Font::Helvetica, 1);
    let b = a_delta(&dir.path().join("b.pdf"), "BETA", Font::Courier, 1);
    let out = dir.path().join("both.pdf");
    merge(&[a, b], &out, "both").unwrap();

    let engine = crate::render::engine().unwrap();
    let doc = engine.open(&out).unwrap();
    assert_eq!(doc.len(), 1);
    let page = doc.render_gray(0, 150.0).unwrap();
    assert!(page.size.matches(&A4, 0.2), "{:?}", page.size);
    // Something was drawn: a blank page would be white everywhere. Both
    // deltas put one line on, so a form that failed to draw would roughly
    // halve this rather than empty it — hence the count rather than a test
    // for any ink at all.
    let inked = page.gray.iter().filter(|&&value| value < 128).count();
    assert!(
        inked > 100,
        "the merged page rendered nearly blank: {inked}"
    );
}

// ---------------------------------------------------------------------------
// Saying what happened
// ---------------------------------------------------------------------------

#[test]
fn a_merge_says_what_it_did_in_words() {
    let dir = tempfile::tempdir().unwrap();
    let long = a_delta(&dir.path().join("long.pdf"), "PAGE", Font::Helvetica, 2);
    let short = a_delta(&dir.path().join("short.pdf"), "ONCE", Font::Courier, 1);
    let out = dir.path().join("both.pdf");
    let said = merge(&[long, short], &out, "both").unwrap().describe();

    assert!(said.contains("2 page(s) of A4"), "{said}");
    assert!(said.contains("long.pdf"), "{said}");
    assert!(said.contains("nothing on the pages after that"), "{said}");
}
