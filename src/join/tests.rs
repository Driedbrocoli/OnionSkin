use super::*;

use crate::pdf::{Font, LineFont, PlacedLine};

/// A one-page document with a word on it, at a given size of paper.
fn a_page(dir: &Path, name: &str, word: &str, paper: PageSize) -> PathBuf {
    a_document(dir, name, word, paper, Font::Helvetica, 1)
}

/// A document of `pages` pages, each with a word on it in a given face.
fn a_document(
    dir: &Path,
    name: &str,
    word: &str,
    paper: PageSize,
    font: Font,
    pages: usize,
) -> PathBuf {
    let path = dir.join(name);
    let sizes = vec![paper; pages];
    let lines: Vec<Vec<PlacedLine>> = (0..pages)
        .map(|_| {
            vec![PlacedLine {
                text: word.to_string(),
                x_mm: 20.0,
                y_mm: 40.0,
                size_pt: 12.0,
                font: LineFont::Builtin(font),
                colour: (0.0, 0.0, 0.0),
                rotation_deg: 0.0,
            }]
        })
        .collect();
    crate::pdf::write_delta(&path, &sizes, &lines, "test", None).unwrap();
    path
}

fn a4() -> PageSize {
    PageSize::new(210.0, 297.0)
}

fn letter() -> PageSize {
    PageSize::new(215.9, 279.4)
}

/// A scratch directory. Kept alive by its handle, deleted when it drops.
fn scratch() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

/// The whole point: three one-page files become one three-page file, in the
/// order they were given, each page still carrying its own words.
#[test]
fn three_files_become_one_document_of_three_pages() {
    let held = scratch();
    let dir = held.path();
    let one = a_page(dir, "one.pdf", "First", a4());
    let two = a_page(dir, "two.pdf", "Second", a4());
    let three = a_page(dir, "three.pdf", "Third", a4());
    let out = dir.join("stack.pdf");

    let joined = join(&[one, two, three], &out, "stack").unwrap();
    assert_eq!(joined.pages, 3);

    // Read back with a different reader than the one that wrote it, so this
    // is a claim about the file and not about our own bookkeeping.
    let read = lopdf::Document::load(&out).unwrap();
    assert_eq!(read.get_pages().len(), 3);

    // And the words are on the pages they started on, in order.
    let pages: Vec<u32> = read.get_pages().into_keys().collect();
    for (index, wanted) in ["First", "Second", "Third"].iter().enumerate() {
        let text = read.extract_text(&[pages[index]]).unwrap();
        assert!(
            text.contains(wanted),
            "page {} does not say {wanted}: {text:?}",
            index + 1
        );
    }
}

/// A joined stack is a real PDF to something that is not us, which is the only
/// test of a PDF writer that counts.
#[test]
fn the_joined_file_renders() {
    let held = scratch();
    let dir = held.path();
    let one = a_page(dir, "one.pdf", "First", a4());
    let two = a_page(dir, "two.pdf", "Second", a4());
    let out = dir.join("stack.pdf");
    join(&[one, two], &out, "stack").unwrap();

    // Rendered by pdfium, which is a different program's idea of a PDF.
    let engine = crate::render::engine().unwrap();
    let doc = engine.open(&out).unwrap();
    assert_eq!(doc.len(), 2, "the stack did not open as two pages");
    // Blank would be white everywhere; one line of words is a few hundred
    // dark pixels at 150 dpi. A page that failed to carry its content across
    // would render, and render empty.
    for index in 0..2 {
        let page = doc.render_gray(index, 150.0).unwrap();
        let inked = page.gray.iter().filter(|&&value| value < 128).count();
        assert!(
            inked > 100,
            "page {} rendered nearly blank: {inked}",
            index + 1
        );
    }
}

/// Mixed paper is fine in a stack, unlike a merge. Each page keeps its own
/// size, and both sizes are reported.
#[test]
fn a4_and_letter_join_and_each_keeps_its_own_size() {
    let held = scratch();
    let dir = held.path();
    let one = a_page(dir, "a4.pdf", "Metric", a4());
    let two = a_page(dir, "letter.pdf", "Imperial", letter());
    let out = dir.join("stack.pdf");

    let joined = join(&[one, two], &out, "stack").unwrap();
    assert_eq!(joined.sizes().len(), 2, "{:?}", joined.sizes());
    assert!(
        joined.describe().contains("mixed paper"),
        "{}",
        joined.describe()
    );

    // Measured off the written file, not off what we told ourselves.
    let read = lopdf::Document::load(&out).unwrap();
    let ids: Vec<lopdf::ObjectId> = read.get_pages().into_values().collect();
    let first = crate::merge::sheet_of(&read, ids[0]).unwrap().size();
    let second = crate::merge::sheet_of(&read, ids[1]).unwrap().size();
    assert!((first.width_mm - 210.0).abs() < 0.5, "{first:?}");
    assert!((second.width_mm - 215.9).abs() < 0.5, "{second:?}");
}

/// Two files' fonts are different objects with the same name inside them. If
/// the copy were shallow, or the names reused, page two would come out set in
/// page one's face — or with no face at all.
#[test]
fn each_page_keeps_its_own_resources() {
    let held = scratch();
    let dir = held.path();
    let courier = a_document(dir, "courier.pdf", "Courier", a4(), Font::Courier, 1);
    let helvetica = a_page(dir, "helvetica.pdf", "Helvetica", a4());
    let out = dir.join("stack.pdf");
    join(&[courier, helvetica], &out, "stack").unwrap();

    let read = lopdf::Document::load(&out).unwrap();
    let ids: Vec<lopdf::ObjectId> = read.get_pages().into_values().collect();
    let faces = |page: lopdf::ObjectId| -> Vec<String> {
        let mut found = Vec::new();
        let resources = crate::merge::inherited(&read, page, b"Resources").unwrap();
        let resources = match resources {
            Object::Reference(id) => read.get_object(id).unwrap().clone(),
            other => other,
        };
        let fonts = resources.as_dict().unwrap().get(b"Font").unwrap().clone();
        let fonts = match fonts {
            Object::Reference(id) => read.get_object(id).unwrap().clone(),
            other => other,
        };
        for (_, font) in fonts.as_dict().unwrap().iter() {
            let font = match font {
                Object::Reference(id) => read.get_object(*id).unwrap().clone(),
                other => other.clone(),
            };
            if let Ok(name) = font.as_dict().unwrap().get(b"BaseFont") {
                found.push(String::from_utf8_lossy(name.as_name().unwrap()).into_owned());
            }
        }
        found
    };
    assert!(faces(ids[0]).iter().any(|name| name.contains("Courier")));
    assert!(faces(ids[1]).iter().any(|name| name.contains("Helvetica")));
}

/// Every page in the joined file must hang off the new page tree. A page whose
/// Parent still pointed at its old document opens in some readers and not
/// others, which is the worst of both.
#[test]
fn every_page_belongs_to_the_new_document() {
    let held = scratch();
    let dir = held.path();
    let one = a_page(dir, "one.pdf", "First", a4());
    let two = a_page(dir, "two.pdf", "Second", a4());
    let out = dir.join("stack.pdf");
    join(&[one, two], &out, "stack").unwrap();

    let read = lopdf::Document::load(&out).unwrap();
    let root = read.catalog().unwrap();
    let tree = root.get(b"Pages").unwrap().as_reference().unwrap();
    for id in read.get_pages().into_values() {
        let parent = read
            .get_dictionary(id)
            .unwrap()
            .get(b"Parent")
            .unwrap()
            .as_reference()
            .unwrap();
        assert_eq!(parent, tree, "a page still belongs to its old document");
    }
    let count = read
        .get_dictionary(tree)
        .unwrap()
        .get(b"Count")
        .unwrap()
        .as_i64()
        .unwrap();
    assert_eq!(count, 2, "the page tree miscounts its own pages");
}

/// A page that inherited its size from the tree above it must have that size
/// written onto it, because the tree is not coming along.
#[test]
fn a_size_kept_on_the_page_tree_comes_across_with_the_page() {
    let held = scratch();
    let dir = held.path();
    let plain = a_page(dir, "plain.pdf", "Inherited", a4());

    // Move MediaBox off the page and onto the tree above it — legal, ordinary,
    // and exactly what would be lost by a copy that only took the page.
    let mut doc = lopdf::Document::load(&plain).unwrap();
    let page_id = doc.get_pages().into_values().next().unwrap();
    let parent = doc
        .get_dictionary(page_id)
        .unwrap()
        .get(b"Parent")
        .unwrap()
        .as_reference()
        .unwrap();
    let box_pt = doc
        .get_dictionary(page_id)
        .unwrap()
        .get(b"MediaBox")
        .unwrap()
        .clone();
    doc.get_dictionary_mut(page_id).unwrap().remove(b"MediaBox");
    doc.get_dictionary_mut(parent)
        .unwrap()
        .set("MediaBox", box_pt);
    let up_the_tree = dir.join("up-the-tree.pdf");
    doc.save(&up_the_tree).unwrap();

    let other = a_page(dir, "other.pdf", "Other", a4());
    let out = dir.join("stack.pdf");
    join(&[up_the_tree, other], &out, "stack").unwrap();

    let read = lopdf::Document::load(&out).unwrap();
    let first = read.get_pages().into_values().next().unwrap();
    assert!(
        read.get_dictionary(first).unwrap().get(b"MediaBox").is_ok(),
        "the page lost the size it was inheriting"
    );
    let size = crate::merge::sheet_of(&read, first).unwrap().size();
    assert!((size.width_mm - 210.0).abs() < 0.5, "{size:?}");
}

/// A page with no size anywhere is refused rather than guessed at: the guess
/// would decide what paper the printer is asked for.
#[test]
fn a_page_with_no_size_anywhere_is_refused() {
    let held = scratch();
    let dir = held.path();
    let plain = a_page(dir, "plain.pdf", "Sized", a4());
    let mut doc = lopdf::Document::load(&plain).unwrap();
    let page_id = doc.get_pages().into_values().next().unwrap();
    let parent = doc
        .get_dictionary(page_id)
        .unwrap()
        .get(b"Parent")
        .unwrap()
        .as_reference()
        .unwrap();
    doc.get_dictionary_mut(page_id).unwrap().remove(b"MediaBox");
    doc.get_dictionary_mut(parent).unwrap().remove(b"MediaBox");
    let sizeless = dir.join("sizeless.pdf");
    doc.save(&sizeless).unwrap();

    let other = a_page(dir, "other.pdf", "Other", a4());
    let out = dir.join("stack.pdf");
    let refused = join(&[sizeless, other], &out, "stack").unwrap_err();
    assert!(refused.to_string().contains("what size paper"), "{refused}");
    assert!(!out.exists(), "half a stack was written before the refusal");
}

/// Where each file's pages ended up, so somebody can find page 14 again.
#[test]
fn the_report_says_which_pages_came_from_which_file() {
    let held = scratch();
    let dir = held.path();
    let one = a_page(dir, "one.pdf", "First", a4());
    let two = a_page(dir, "two.pdf", "Second", a4());
    let out = dir.join("stack.pdf");
    let joined = join(&[one.clone(), two.clone()], &out, "stack").unwrap();

    assert_eq!(joined.from[0].first_page, 1);
    assert_eq!(joined.from[1].first_page, 2);
    let said = joined.describe();
    assert!(said.contains("page 1"), "{said}");
    assert!(said.contains("page 2"), "{said}");
}

/// The same file twice is a stack with the page in it twice — which is a real
/// request, so it is done and mentioned, not refused.
#[test]
fn the_same_file_twice_is_allowed_and_said() {
    let held = scratch();
    let dir = held.path();
    let one = a_page(dir, "one.pdf", "First", a4());
    let out = dir.join("stack.pdf");
    let joined = join(&[one.clone(), one.clone()], &out, "stack").unwrap();

    assert_eq!(joined.pages, 2, "a file given twice must appear twice");
    assert_eq!(joined.from[1].same_as.as_ref(), Some(&one));
    assert!(joined.describe().contains("the same file as"));
}

/// One file is not a join, and saying so is better than writing a copy of it
/// under a new name.
#[test]
fn one_file_is_not_a_join() {
    let held = scratch();
    let dir = held.path();
    let one = a_page(dir, "one.pdf", "First", a4());
    let out = dir.join("stack.pdf");
    let refused = join(&[one], &out, "stack").unwrap_err();
    assert!(refused.to_string().contains("at least two"), "{refused}");
}

/// A shell sorts page-10 before page-2, and a stack in that order is a stack
/// in the wrong order. Not refused — the join is exactly as asked — but far
/// cheaper to notice here than after twenty sheets have gone through.
#[test]
fn files_numbered_out_of_order_are_noticed() {
    let held = scratch();
    let dir = held.path();
    let one = a_page(dir, "page-1.pdf", "One", a4());
    let ten = a_page(dir, "page-10.pdf", "Ten", a4());
    let two = a_page(dir, "page-2.pdf", "Two", a4());
    let out = dir.join("stack.pdf");

    let shell_order = join(&[one.clone(), ten.clone(), two.clone()], &out, "stack").unwrap();
    let said = shell_order
        .out_of_order()
        .expect("10 before 2 went unnoticed");
    assert!(said.contains("page-10"), "{said}");

    let right_order = join(&[one, two, ten], &out, "stack").unwrap();
    assert_eq!(
        right_order.out_of_order(),
        None,
        "1, 2, 10 was called out of order"
    );
}

/// Files that are not a numbered sequence are not second-guessed. "cover.pdf
/// terms.pdf" is in the order somebody typed, and there is nothing to compare
/// it against.
#[test]
fn files_that_are_not_numbered_are_left_alone() {
    let held = scratch();
    let dir = held.path();
    let cover = a_page(dir, "cover.pdf", "Cover", a4());
    let terms = a_page(dir, "terms.pdf", "Terms", a4());
    let out = dir.join("stack.pdf");
    let joined = join(&[cover, terms], &out, "stack").unwrap();
    assert_eq!(joined.out_of_order(), None);
}

/// The number is the last run of digits, because a scan folder is full of
/// names like "2024-invoice-7.pdf" and the year is not the page number.
#[test]
fn the_number_is_the_last_one_in_the_name() {
    assert_eq!(number_in(Path::new("2024-invoice-7.pdf")), Some(7));
    assert_eq!(number_in(Path::new("page-10.pdf")), Some(10));
    assert_eq!(number_in(Path::new("scan007.pdf")), Some(7));
    assert_eq!(number_in(Path::new("cover.pdf")), None);
}

/// Bookmarks belong to a document, not to a page, and three documents' outlines
/// do not splice into one. Saying so beats losing them quietly.
#[test]
fn what_cannot_come_along_is_named_rather_than_dropped_in_silence() {
    let held = scratch();
    let dir = held.path();
    let plain = a_page(dir, "plain.pdf", "Bookmarked", a4());
    let mut doc = lopdf::Document::load(&plain).unwrap();
    let page_id = doc.get_pages().into_values().next().unwrap();
    let item = doc.add_object(lopdf::dictionary! {
        "Title" => Object::string_literal("Chapter one"),
        "Dest" => vec![Object::Reference(page_id), "Fit".into()],
    });
    let outlines = doc.add_object(lopdf::dictionary! {
        "Type" => "Outlines",
        "First" => item,
        "Last" => item,
        "Count" => 1,
    });
    let root = doc.catalog().unwrap().clone();
    let root_id = doc.trailer.get(b"Root").unwrap().as_reference().unwrap();
    let mut root = root;
    root.set("Outlines", outlines);
    doc.objects.insert(root_id, Object::Dictionary(root));
    let bookmarked = dir.join("bookmarked.pdf");
    doc.save(&bookmarked).unwrap();

    let other = a_page(dir, "other.pdf", "Other", a4());
    let out = dir.join("stack.pdf");
    let joined = join(&[bookmarked.clone(), other], &out, "stack").unwrap();
    assert_eq!(joined.left_behind.len(), 1, "{:?}", joined.left_behind);
    assert!(joined.left_behind[0].1.contains("bookmarks"));
    assert_eq!(joined.left_behind[0].0, bookmarked);
}

/// An empty outline dictionary is not bookmarks. Warning about nothing teaches
/// people to ignore warnings.
#[test]
fn an_empty_outline_is_not_reported_as_lost_bookmarks() {
    let held = scratch();
    let dir = held.path();
    let plain = a_page(dir, "plain.pdf", "Plain", a4());
    let mut doc = lopdf::Document::load(&plain).unwrap();
    let outlines = doc.add_object(lopdf::dictionary! {
        "Type" => "Outlines",
        "Count" => 0,
    });
    let root_id = doc.trailer.get(b"Root").unwrap().as_reference().unwrap();
    let mut root = doc.catalog().unwrap().clone();
    root.set("Outlines", outlines);
    doc.objects.insert(root_id, Object::Dictionary(root));
    let empty = dir.join("empty-outline.pdf");
    doc.save(&empty).unwrap();

    let other = a_page(dir, "other.pdf", "Other", a4());
    let out = dir.join("stack.pdf");
    let joined = join(&[empty, other], &out, "stack").unwrap();
    assert!(joined.left_behind.is_empty(), "{:?}", joined.left_behind);
}

/// A file that will not open is said so before anything is written, so a bad
/// name in a list of twenty does not leave half a stack behind.
#[test]
fn a_file_that_will_not_open_is_refused_before_anything_is_written() {
    let held = scratch();
    let dir = held.path();
    let one = a_page(dir, "one.pdf", "First", a4());
    let rubbish = dir.join("rubbish.pdf");
    std::fs::write(&rubbish, b"this is not a PDF").unwrap();
    let out = dir.join("stack.pdf");

    let refused = join(&[one, rubbish.clone()], &out, "stack").unwrap_err();
    assert!(refused.to_string().contains("rubbish.pdf"), "{refused}");
    assert!(!out.exists(), "half a stack was written before the refusal");
}

/// A multi-page file joined with a one-page file gives every page, in order —
/// the case that made this necessary, since `onionskin stack` wants one
/// document however many files the pages arrived in.
#[test]
fn a_multi_page_file_contributes_all_of_its_pages() {
    let held = scratch();
    let dir = held.path();
    let first = a_page(dir, "first.pdf", "One", a4());
    let second = a_page(dir, "second.pdf", "Two", a4());
    let third = a_page(dir, "third.pdf", "Three", a4());
    let pair = dir.join("pair.pdf");
    join(&[first, second], &pair, "pair").unwrap();

    let out = dir.join("stack.pdf");
    let joined = join(&[pair, third], &out, "stack").unwrap();
    assert_eq!(joined.pages, 3);
    assert_eq!(joined.from[0].pages, 2);
    assert_eq!(joined.from[1].first_page, 3);

    let read = lopdf::Document::load(&out).unwrap();
    let pages: Vec<u32> = read.get_pages().into_keys().collect();
    assert_eq!(pages.len(), 3);
    let last = read.extract_text(&[pages[2]]).unwrap();
    assert!(last.contains("Three"), "{last:?}");
}

/// A link annotation names the page it sits on, and that page names the
/// annotation: a loop. Following it naively copies the whole old document in
/// behind one link, and leaves the annotation pointing at a page that is not
/// in the stack. Both are invisible until somebody clicks the link.
#[test]
fn an_annotation_that_points_back_at_its_own_page_lands_on_the_right_one() {
    let held = scratch();
    let dir = held.path();
    let plain = a_page(dir, "plain.pdf", "Linked", a4());
    let mut doc = lopdf::Document::load(&plain).unwrap();
    let page_id = doc.get_pages().into_values().next().unwrap();
    let link = doc.add_object(lopdf::dictionary! {
        "Type" => "Annot",
        "Subtype" => "Link",
        "Rect" => vec![20.into(), 20.into(), 80.into(), 40.into()],
        // The page this annotation is on — the reference that closes the loop.
        "P" => page_id,
        "A" => lopdf::dictionary! {
            "S" => "URI",
            "URI" => Object::string_literal("https://example.invalid/"),
        },
    });
    doc.get_dictionary_mut(page_id)
        .unwrap()
        .set("Annots", vec![Object::Reference(link)]);
    let linked = dir.join("linked.pdf");
    doc.save(&linked).unwrap();

    let other = a_page(dir, "other.pdf", "Other", a4());
    let out = dir.join("stack.pdf");
    join(&[linked, other], &out, "stack").unwrap();

    let read = lopdf::Document::load(&out).unwrap();
    let first = read.get_pages().into_values().next().unwrap();
    let annots = read
        .get_dictionary(first)
        .unwrap()
        .get(b"Annots")
        .unwrap()
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(annots.len(), 1, "the link did not come across");
    let annot = read
        .get_dictionary(annots[0].as_reference().unwrap())
        .unwrap();
    assert_eq!(
        annot.get(b"P").unwrap().as_reference().unwrap(),
        first,
        "the link points at a page that is not in the stack"
    );

    // And nothing came in behind it: two pages in, two page objects out.
    let pages = read
        .objects
        .values()
        .filter(|object| {
            object
                .as_dict()
                .ok()
                .and_then(|dict| dict.get(b"Type").ok())
                .and_then(|kind| kind.as_name().ok())
                == Some(b"Page")
        })
        .count();
    assert_eq!(pages, 2, "a page came across more than once");
}
