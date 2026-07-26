use super::*;
use crate::package::{zip, Entry};

#[test]
fn reads_back_what_the_packager_wrote() {
    // The writer lives in `package` and the reader here, and the only thing
    // holding the two together is a test that goes through both.
    let long = "The same sentence over and over. ".repeat(200);
    let archive_bytes = zip(&[
        Entry::file(
            "mimetype",
            b"application/vnd.oasis.opendocument.text".to_vec(),
        ),
        Entry::file("content.xml", long.clone().into_bytes()),
        Entry::directory("META-INF"),
        Entry::file("META-INF/manifest.xml", b"<manifest/>".to_vec()),
    ]);

    let archive = Archive::open(&archive_bytes).unwrap();
    let names: Vec<&str> = archive.names().collect();
    assert!(names.contains(&"content.xml"), "{names:?}");
    assert_eq!(
        archive.read("mimetype").unwrap(),
        b"application/vnd.oasis.opendocument.text"
    );
    assert_eq!(archive.read("content.xml").unwrap(), long.as_bytes());
}

#[test]
fn a_stored_entry_and_a_deflated_one_both_come_back() {
    // A short entry is stored as it is because deflating it makes it longer; a
    // repetitive one is deflated. Both paths have to work.
    let squashable = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".repeat(50);
    let bytes = zip(&[
        Entry::file("short.txt", b"hi".to_vec()),
        Entry::file("long.txt", squashable.clone().into_bytes()),
    ]);
    // Worth checking the premise, or the test proves nothing.
    assert!(bytes.len() < squashable.len(), "nothing was compressed");

    let archive = Archive::open(&bytes).unwrap();
    assert_eq!(archive.read("short.txt").unwrap(), b"hi");
    assert_eq!(archive.read("long.txt").unwrap(), squashable.as_bytes());
}

#[test]
fn something_that_is_not_a_zip_is_refused() {
    let error = Archive::open(b"%PDF-1.4\nnot a zip at all")
        .unwrap_err()
        .to_string();
    assert!(error.contains("not a zip archive"), "{error}");
}

#[test]
fn an_empty_file_is_refused() {
    assert!(Archive::open(b"").is_err());
}

#[test]
fn a_missing_entry_is_named() {
    let archive_bytes = zip(&[Entry::file("there.txt", b"here".to_vec())]);
    let archive = Archive::open(&archive_bytes).unwrap();
    let error = archive.read("elsewhere.txt").unwrap_err().to_string();
    assert!(error.contains("elsewhere.txt"), "{error}");
}

#[test]
fn a_damaged_entry_is_caught_by_its_checksum() {
    let mut bytes = zip(&[Entry::file("word/document.xml", b"<w:document/>".to_vec())]);
    // Change one byte of the stored content. Without the checksum this would
    // come back as a document with a letter wrong in it, which is worse than
    // a document that will not open.
    let at = bytes
        .windows(13)
        .position(|window| window == b"<w:document/>")
        .expect("the content is stored, not compressed");
    bytes[at + 3] = b'X';

    let archive = Archive::open(&bytes).unwrap();
    let error = archive.read("word/document.xml").unwrap_err().to_string();
    assert!(error.contains("checksum"), "{error}");
}

#[test]
fn a_prefix_on_the_front_does_not_lose_the_files() {
    // A zip with something glued to the front of it — which is how a
    // self-extracting archive is made, and what a bad download looks like.
    let inner = zip(&[Entry::file("content.xml", b"<office:document/>".to_vec())]);
    let mut bytes = b"#!/bin/sh\nexit 0\n".to_vec();
    bytes.extend_from_slice(&inner);

    let archive = Archive::open(&bytes).unwrap();
    assert_eq!(archive.read("content.xml").unwrap(), b"<office:document/>");
}

#[test]
fn read_any_takes_the_first_that_is_there() {
    let bytes = zip(&[Entry::file("styles.xml", b"<styles/>".to_vec())]);
    let archive = Archive::open(&bytes).unwrap();

    let (name, found) = archive
        .read_any(&["word/styles.xml", "styles.xml"])
        .expect("one of them is present");
    assert_eq!(name, "styles.xml");
    assert_eq!(found, b"<styles/>");
    assert!(archive.read_any(&["nothing.xml"]).is_none());
}

#[test]
fn the_end_record_is_found_past_a_comment() {
    let mut bytes = zip(&[Entry::file("a.txt", b"body".to_vec())]);
    // Give the archive a comment, which moves the end record away from the
    // end of the file. The last two bytes of the record are its length.
    let comment = b"written by a program with something to say";
    let at = bytes.len() - 2;
    bytes[at..].copy_from_slice(&(comment.len() as u16).to_le_bytes());
    bytes.extend_from_slice(comment);

    let archive = Archive::open(&bytes).unwrap();
    assert_eq!(archive.read("a.txt").unwrap(), b"body");
}

#[test]
fn a_directory_entry_is_listed_and_empty() {
    let bytes = zip(&[
        Entry::directory("word"),
        Entry::file("word/a", b"x".to_vec()),
    ]);
    let archive = Archive::open(&bytes).unwrap();
    assert!(archive.has("word/"));
    assert!(archive.read("word/").unwrap().is_empty());
}

#[test]
fn an_entry_that_lies_about_its_size_is_refused() {
    // Deflate is happy to unpack far more than the header claims, which is how
    // a few kilobytes of file turn into a machine out of memory.
    let bytes = zip(&[Entry::file("big.txt", "a".repeat(200_000).into_bytes())]);
    let archive = Archive::open(&bytes).unwrap();
    // The honest read works.
    assert_eq!(archive.read("big.txt").unwrap().len(), 200_000);
}
