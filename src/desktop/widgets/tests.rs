use super::*;
use std::path::PathBuf;

fn dropped(names: &[&str]) -> Vec<PathBuf> {
    names.iter().map(PathBuf::from).collect()
}

#[test]
fn a_row_takes_the_first_file_it_can_use() {
    let mut files = dropped(&["/tmp/holiday.png", "/tmp/report.docx"]);
    let taken = claim(&mut files, &["pdf", "docx"]);

    assert_eq!(taken, Some(PathBuf::from("/tmp/report.docx")));
    // And leaves the rest, so the row that wants a scan still gets one.
    assert_eq!(files, dropped(&["/tmp/holiday.png"]));
}

#[test]
fn two_rows_of_the_same_kind_fill_in_order() {
    // Dropping two documents on the comparing screen should fill the original
    // and then the edited copy, which is the order the rows are drawn.
    let mut files = dropped(&["/tmp/before.pdf", "/tmp/after.pdf"]);
    assert_eq!(
        claim(&mut files, &["pdf"]),
        Some(PathBuf::from("/tmp/before.pdf"))
    );
    assert_eq!(
        claim(&mut files, &["pdf"]),
        Some(PathBuf::from("/tmp/after.pdf"))
    );
    assert_eq!(claim(&mut files, &["pdf"]), None);
}

#[test]
fn a_row_leaves_a_file_it_cannot_use() {
    let mut files = dropped(&["/tmp/holiday.png"]);
    assert_eq!(claim(&mut files, &["pdf", "docx"]), None);
    assert_eq!(files.len(), 1, "it should still be there for another row");
}

#[test]
fn a_row_that_takes_anything_takes_the_first() {
    let mut files = dropped(&["/tmp/whatever", "/tmp/second"]);
    assert_eq!(claim(&mut files, &[]), Some(PathBuf::from("/tmp/whatever")));
}

#[test]
fn a_file_from_windows_may_be_shouted() {
    let mut files = dropped(&["/tmp/REPORT.PDF"]);
    assert_eq!(
        claim(&mut files, &["pdf"]),
        Some(PathBuf::from("/tmp/REPORT.PDF"))
    );
}

#[test]
fn a_file_with_no_extension_is_not_mistaken_for_one() {
    let mut files = dropped(&["/tmp/Makefile"]);
    assert_eq!(claim(&mut files, &["pdf"]), None);
}

#[test]
fn nothing_dropped_claims_nothing() {
    let mut files: Vec<PathBuf> = Vec::new();
    assert_eq!(claim(&mut files, &["pdf"]), None);
    assert_eq!(claim(&mut files, &[]), None);
}
