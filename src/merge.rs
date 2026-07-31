//! Several deltas, one pass through the printer.
//!
//! A day's work on one document arrives as more than one delta. The paid stamp
//! is a saved job; the signature is a picture; the reference number came out of
//! a spreadsheet. Each of those is a delta, each of them prints, and printing
//! three of them onto one sheet means feeding that sheet through the printer
//! three times.
//!
//! Every pass is a chance to lose the sheet. It can go in crooked, it can jam,
//! it can pick up the one underneath it as well, and it lands a little
//! differently each time — that is the entire reason this program has a
//! calibration step. Three passes are three of those chances on a piece of
//! paper that already has the letterhead printed on it and cannot be reprinted.
//!
//! So the deltas are merged first, and the sheet goes through once:
//!
//! ```text
//! onionskin merge stamp.pdf signature.pdf reference.pdf --out all.pdf
//! ```
//!
//! # Why a form, and not glued-together content
//!
//! Two deltas both call their first font `F0`, and they mean different fonts.
//! Concatenating the two content streams would set the second delta's words in
//! the first delta's face, silently, and it would look almost right.
//!
//! So each page goes into the merged file as a *form* — a self-contained
//! parcel with its own resource names inside it — and the merged page is three
//! lines long: draw the first, draw the second, draw the third. Nothing is
//! renamed, nothing is reinterpreted, and a delta written by some other program
//! entirely merges just as well as one of ours.
//!
//! # What is checked before anything is written
//!
//! That the pages are the same size. A delta for a letter merged with a delta
//! for an A4 invoice is somebody's mistake, and the merged file would print
//! half of one of them off the edge of the paper. It is refused, with both
//! sizes named, because that is a question best asked before the paper goes in
//! rather than after.
//!
//! Where the sheets are the same size but their origins differ — which happens
//! when a PDF's page box does not start at zero — the difference is corrected
//! rather than refused. It is arithmetic, and there is no need to trouble
//! anybody with it.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use lopdf::{dictionary, Document, Object, ObjectId, Stream};

use crate::geometry::PageSize;

/// How far apart two page sizes may be and still count as the same sheet.
///
/// A point is about a third of a millimetre: loose enough for the rounding
/// that happens when millimetres go into a PDF and come back out, tight enough
/// that A4 and US Letter are never confused for one another.
pub const SAME_SHEET_PT: f64 = 1.0;

/// One page of one of the files being merged.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Sheet {
    /// The page box, in points: left, bottom, right, top.
    box_pt: (f64, f64, f64, f64),
    /// Quarter turns the reader is asked to apply, normalised to 0, 90, 180
    /// or 270.
    rotate: i64,
}

impl Sheet {
    pub(crate) fn size(&self) -> PageSize {
        let (left, bottom, right, top) = self.box_pt;
        PageSize::from_pt(right - left, top - bottom)
    }

    /// The same piece of paper?
    ///
    /// Size only. Where the two boxes are the same size but start from
    /// different corners the difference is corrected when the page is drawn,
    /// so it is not a reason to refuse.
    fn same_paper_as(&self, other: &Sheet) -> bool {
        let (mine, theirs) = (self.size(), other.size());
        (mine.width_pt() - theirs.width_pt()).abs() <= SAME_SHEET_PT
            && (mine.height_pt() - theirs.height_pt()).abs() <= SAME_SHEET_PT
    }
}

/// What one of the merged files brought to the merge.
#[derive(Debug, Clone, PartialEq)]
pub struct Contribution {
    pub path: PathBuf,
    pub pages: usize,
    /// The file earlier in the list this one is byte-for-byte identical to.
    ///
    /// Worth saying: the same delta merged twice puts every letter down twice
    /// in the same place, which comes out heavier and blurred and is never
    /// what anybody meant.
    pub same_as: Option<PathBuf>,
}

/// What came of a merge.
#[derive(Debug, Clone, PartialEq)]
pub struct Merged {
    pub pages: usize,
    /// The size of the first page. Pages are checked against each other file's
    /// page in the same position, not against page one, so a document whose
    /// pages are not all the same size merges correctly even though only one
    /// size is reported here.
    pub page: PageSize,
    pub from: Vec<Contribution>,
}

impl Merged {
    /// Whether any file was given twice.
    pub fn repeats(&self) -> Vec<&Contribution> {
        self.from
            .iter()
            .filter(|from| from.same_as.is_some())
            .collect()
    }

    /// Files that ran out before the merged document did.
    ///
    /// Not an error — a one-page stamp merged onto page one of a five-page
    /// invoice is a perfectly ordinary thing to want — but worth saying,
    /// because the other reading is that somebody named the wrong file.
    pub fn short(&self) -> Vec<&Contribution> {
        self.from
            .iter()
            .filter(|from| from.pages < self.pages)
            .collect()
    }

    pub fn describe(&self) -> String {
        let mut lines = vec![format!(
            "{} page(s) of {}, from {} file(s):",
            self.pages,
            self.page.describe(),
            self.from.len()
        )];
        for from in &self.from {
            let name = from.path.display();
            let mut line = format!("  {} page(s)  {name}", from.pages);
            if let Some(same) = &from.same_as {
                line.push_str(&format!("  — the same file as {}", same.display()));
            } else if from.pages < self.pages {
                line.push_str("  — nothing on the pages after that");
            }
            lines.push(line);
        }
        lines.join("\n")
    }
}

#[derive(Debug, thiserror::Error)]
pub enum MergeError {
    #[error("merging needs at least two deltas; {0} was given.")]
    TooFew(usize),
    #[error("could not read {path}: {source}")]
    Unreadable { path: PathBuf, source: lopdf::Error },
    #[error("{path} has no pages in it.")]
    Blank { path: PathBuf },
    #[error(
        "{path} is a protected PDF — its contents are encrypted, and this cannot \
         read them.\n    Every reader opens a file like this without asking for a \
         password, so it looks ordinary; what it carries is a restriction on \
         copying and changing, and merging is changing.\n    Open it in a PDF \
         reader and save or print it to a fresh PDF, then merge that."
    )]
    Locked { path: PathBuf },
    #[error(
        "page {page} of {path} does not say what size paper it is for, and \
         neither does anything above it. Onionskin will not guess: the guess \
         would decide where every word on that page lands."
    )]
    NoPageBox { path: PathBuf, page: usize },
    #[error(
        "these are not deltas for the same sheet of paper. Page {page} of \
         {first} is {first_size}, and page {page} of {other} is {other_size}. \
         Merging them would print one of them off the edge."
    )]
    DifferentPaper {
        page: usize,
        first: PathBuf,
        first_size: String,
        other: PathBuf,
        other_size: String,
    },
    #[error(
        "page {page} of {other} is turned {other_deg}° and page {page} of \
         {first} is turned {first_deg}°. Merge them and one of them lands \
         sideways. Straighten one of them first."
    )]
    DifferentWayUp {
        page: usize,
        first: PathBuf,
        first_deg: i64,
        other: PathBuf,
        other_deg: i64,
    },
    #[error("could not write {path}: {source}")]
    NotWritten {
        path: PathBuf,
        source: std::io::Error,
    },
}

/// Whether a PDF's contents are encrypted.
///
/// Not "is it password protected" as anybody would mean it. A file with an
/// *owner* password and an empty user password opens in every reader with no
/// prompt at all — it looks like an ordinary PDF, and it is the common case,
/// because it is what "restrict copying and editing" produces in Acrobat and in
/// half the online tools people use.
///
/// Its streams are still encrypted, and the library this reads structure with
/// does not decrypt them. What came back was an empty page: `merge` wrote a form
/// of zero length and reported success, then advised printing the merged file
/// *instead of* the deltas it was made from — so the second delta was gone, and
/// the advice was to throw away the only copy of it. `join` kept the bytes and
/// wrote a page no reader can open.
///
/// Neither is something to do quietly. Refusing costs somebody one step —
/// re-save the file — and the alternative costs them a sheet of paper they only
/// have one of.
pub(crate) fn is_locked(doc: &Document) -> bool {
    doc.trailer.get(b"Encrypt").is_ok()
}

/// Merge several delta PDFs into one, page for page.
///
/// The merged file has as many pages as the longest of them; a file that runs
/// out simply stops contributing, which is what makes a one-page stamp merge
/// onto the front of a five-page invoice.
pub fn merge(inputs: &[PathBuf], out: &Path, title: &str) -> Result<Merged, MergeError> {
    if inputs.len() < 2 {
        return Err(MergeError::TooFew(inputs.len()));
    }

    // Read them all before writing anything, so a mismatch found in the last
    // file does not leave half a merged document behind.
    let mut sources: Vec<(PathBuf, Document, Vec<ObjectId>)> = Vec::new();
    for path in inputs {
        let doc = Document::load(path).map_err(|source| MergeError::Unreadable {
            path: path.clone(),
            source,
        })?;
        if is_locked(&doc) {
            return Err(MergeError::Locked { path: path.clone() });
        }
        let pages: Vec<ObjectId> = doc.get_pages().into_values().collect();
        if pages.is_empty() {
            return Err(MergeError::Blank { path: path.clone() });
        }
        sources.push((path.clone(), doc, pages));
    }

    let deepest = sources
        .iter()
        .map(|(_, _, ids)| ids.len())
        .max()
        .unwrap_or(0);

    // Every page of every file, measured. A page whose size cannot be read is
    // refused here rather than skipped: skipping it would drop its ink without
    // saying so, and — worse — leave a hole that shifts every later page up one.
    let mut sheets: Vec<Vec<Sheet>> = Vec::new();
    for (path, doc, pages) in &sources {
        let mut measured = Vec::new();
        for (page, id) in pages.iter().enumerate() {
            measured.push(sheet_of(doc, *id).ok_or_else(|| MergeError::NoPageBox {
                path: path.clone(),
                page: page + 1,
            })?);
        }
        sheets.push(measured);
    }

    // Each page checked against the first file that has a page there.
    for page in 0..deepest {
        let Some((first_index, (first_path, _, _))) = sources
            .iter()
            .enumerate()
            .find(|(index, _)| page < sheets[*index].len())
        else {
            continue;
        };
        let first_sheet = sheets[first_index][page];
        for (index, (path, _, _)) in sources.iter().enumerate().skip(first_index + 1) {
            let Some(sheet) = sheets[index].get(page).copied() else {
                continue;
            };
            if sheet.rotate != first_sheet.rotate {
                return Err(MergeError::DifferentWayUp {
                    page: page + 1,
                    first: first_path.clone(),
                    first_deg: first_sheet.rotate,
                    other: path.clone(),
                    other_deg: sheet.rotate,
                });
            }
            if !sheet.same_paper_as(&first_sheet) {
                return Err(MergeError::DifferentPaper {
                    page: page + 1,
                    first: first_path.clone(),
                    first_size: first_sheet.size().describe(),
                    other: path.clone(),
                    other_size: sheet.size().describe(),
                });
            }
        }
    }

    let mut merged = Document::with_version(newest_version(&sources));
    let pages_id = merged.new_object_id();

    // One import map per source file, shared across that file's pages, so a
    // font used on twenty pages is carried across once rather than twenty
    // times.
    let mut carried: Vec<BTreeMap<ObjectId, ObjectId>> =
        sources.iter().map(|_| BTreeMap::new()).collect();

    let mut page_ids: Vec<Object> = Vec::new();
    let mut first_size = None;
    for page in 0..deepest {
        // The page box this merged page is drawn in: the first file that has
        // something on this page decides it.
        let Some(page_box) = sheets.iter().find_map(|file| file.get(page).copied()) else {
            continue;
        };
        if first_size.is_none() {
            first_size = Some(page_box.size());
        }

        let mut xobjects = lopdf::Dictionary::new();
        let mut draw = Vec::new();
        for (index, (_, doc, pages)) in sources.iter().enumerate() {
            let Some(page_id) = pages.get(page).copied() else {
                continue;
            };
            let sheet = sheets[index][page];
            let form = form_of(
                doc,
                page_id,
                sheet,
                page_box,
                &mut merged,
                &mut carried[index],
            );
            let name = format!("Fm{index}");
            xobjects.set(name.clone(), Object::Reference(form));
            // Saved and restored around each one, so a delta that leaves the
            // graphics state untidy cannot disturb the one drawn after it.
            draw.push(format!("q /{name} Do Q"));
        }

        let content = merged.add_object(Stream::new(dictionary! {}, draw.join("\n").into_bytes()));
        let (left, bottom, right, top) = page_box.box_pt;
        let mut page_dict = dictionary! {
            "Type" => "Page",
            "Parent" => pages_id,
            "Contents" => content,
            "Resources" => dictionary! { "XObject" => xobjects },
            "MediaBox" => vec![
                Object::Real(left as f32),
                Object::Real(bottom as f32),
                Object::Real(right as f32),
                Object::Real(top as f32),
            ],
        };
        if page_box.rotate != 0 {
            page_dict.set("Rotate", page_box.rotate);
        }
        page_ids.push(merged.add_object(page_dict).into());
    }

    let count = page_ids.len() as i64;
    merged.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => page_ids,
            "Count" => count,
        }),
    );
    let catalog_id = merged.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    let info_id = merged.add_object(dictionary! {
        "Title" => Object::string_literal(title),
        "Producer" => Object::string_literal("Onionskin"),
    });
    merged.trailer.set("Root", catalog_id);
    merged.trailer.set("Info", info_id);

    merged.compress();
    merged.save(out).map_err(|source| MergeError::NotWritten {
        path: out.to_path_buf(),
        source,
    })?;

    Ok(Merged {
        pages: count as usize,
        page: first_size.unwrap_or(PageSize::new(0.0, 0.0)),
        from: told_apart(&sources),
    })
}

/// Which of the files given are the same file, by their bytes.
fn told_apart(sources: &[(PathBuf, Document, Vec<ObjectId>)]) -> Vec<Contribution> {
    let marks: Vec<Option<String>> = sources
        .iter()
        .map(|(path, _, _)| crate::history::fingerprint(path))
        .collect();
    sources
        .iter()
        .enumerate()
        .map(|(index, (path, _, pages))| Contribution {
            path: path.clone(),
            pages: pages.len(),
            same_as: marks[index].as_ref().and_then(|mark| {
                marks[..index]
                    .iter()
                    .position(|earlier| earlier.as_deref() == Some(mark.as_str()))
                    .map(|earlier| sources[earlier].0.clone())
            }),
        })
        .collect()
}

/// The newest PDF version among the files being merged.
///
/// A merged file that claims to be older than its contents is a small lie that
/// some readers take seriously, so it claims the newest of them instead.
pub(crate) fn newest_version(sources: &[(PathBuf, Document, Vec<ObjectId>)]) -> String {
    let ordered = |version: &str| -> (u32, u32) {
        let mut parts = version.split('.');
        let major = parts.next().and_then(|p| p.parse().ok()).unwrap_or(1);
        let minor = parts.next().and_then(|p| p.parse().ok()).unwrap_or(4);
        (major, minor)
    };
    sources
        .iter()
        .map(|(_, doc, _)| doc.version.clone())
        .max_by_key(|version| ordered(version))
        .unwrap_or_else(|| "1.4".to_string())
}

/// The page box and rotation of one page, taking both from the page tree above
/// it when the page itself does not say.
pub(crate) fn sheet_of(doc: &Document, page_id: ObjectId) -> Option<Sheet> {
    let media = inherited(doc, page_id, b"MediaBox")?;
    // The box itself may be a reference to an array, which is legal and does
    // happen — a producer that shares one box between five hundred pages.
    let media = match media {
        Object::Reference(id) => doc.get_object(id).ok()?.clone(),
        other => other,
    };
    let numbers = media.as_array().ok()?;
    if numbers.len() != 4 {
        return None;
    }
    let mut edges = [0.0f64; 4];
    for (slot, number) in edges.iter_mut().zip(numbers) {
        *slot = as_number(doc, number)?;
    }
    // A page box may be given with its corners either way round.
    let box_pt = (
        edges[0].min(edges[2]),
        edges[1].min(edges[3]),
        edges[0].max(edges[2]),
        edges[1].max(edges[3]),
    );
    let rotate = match inherited(doc, page_id, b"Rotate") {
        Some(Object::Integer(quarters)) => quarters,
        Some(Object::Reference(id)) => doc.get_object(id).and_then(Object::as_i64).unwrap_or(0),
        _ => 0,
    };
    Some(Sheet {
        box_pt,
        rotate: rotate.rem_euclid(360),
    })
}

/// An entry from a page or, failing that, from the page tree above it.
///
/// `MediaBox`, `Resources` and `Rotate` are all inheritable, and a PDF that
/// puts them on the page tree rather than the page is perfectly ordinary.
pub(crate) fn inherited(doc: &Document, page_id: ObjectId, key: &[u8]) -> Option<Object> {
    let mut at = page_id;
    // Bounded rather than followed to the end: a page tree with a loop in it
    // would otherwise hang here, and a broken file should not be able to do
    // that.
    for _ in 0..32 {
        let dict = doc.get_dictionary(at).ok()?;
        if let Ok(found) = dict.get(key) {
            return Some(found.clone());
        }
        at = dict.get(b"Parent").ok()?.as_reference().ok()?;
    }
    None
}

fn as_number(doc: &Document, object: &Object) -> Option<f64> {
    match object {
        Object::Integer(number) => Some(*number as f64),
        Object::Real(number) => Some(*number as f64),
        // A page box given as four separate objects is legal and does happen.
        Object::Reference(id) => match doc.get_object(*id).ok()? {
            Object::Integer(number) => Some(*number as f64),
            Object::Real(number) => Some(*number as f64),
            _ => None,
        },
        _ => None,
    }
}

/// Wrap one page of one file up as a form in the merged document.
///
/// The form carries its own resources, so its `F0` stays its own `F0`. Its
/// matrix corrects for a page box that does not start where the merged page's
/// does — the same sheet of paper, measured from a different corner.
fn form_of(
    from: &Document,
    page_id: ObjectId,
    sheet: Sheet,
    onto: Sheet,
    into: &mut Document,
    carried: &mut BTreeMap<ObjectId, ObjectId>,
) -> ObjectId {
    let content = from.get_page_content(page_id).unwrap_or_default();
    let (left, bottom, right, top) = sheet.box_pt;

    let mut entries = dictionary! {
        "Type" => "XObject",
        "Subtype" => "Form",
        "FormType" => 1,
        "BBox" => vec![
            Object::Real(left as f32),
            Object::Real(bottom as f32),
            Object::Real(right as f32),
            Object::Real(top as f32),
        ],
    };
    let (shift_x, shift_y) = (onto.box_pt.0 - left, onto.box_pt.1 - bottom);
    if shift_x.abs() > 1e-6 || shift_y.abs() > 1e-6 {
        entries.set(
            "Matrix",
            vec![
                Object::Real(1.0),
                Object::Real(0.0),
                Object::Real(0.0),
                Object::Real(1.0),
                Object::Real(shift_x as f32),
                Object::Real(shift_y as f32),
            ],
        );
    }
    if let Some(resources) = resources_of(from, page_id) {
        entries.set("Resources", import(from, into, &resources, carried));
    }
    into.add_object(Stream::new(entries, content))
}

/// Everything a page draws with, with the page's own entries winning over the
/// ones it inherits from the tree above it.
///
/// Strictly, a page that has any `Resources` of its own inherits none — so
/// collecting the tree's categories as well is more generous than the letter of
/// the spec. It can only ever add something the page's own content already
/// asked for by name and would otherwise not have found; anything the page does
/// define wins, so nothing is ever reinterpreted.
fn resources_of(doc: &Document, page_id: ObjectId) -> Option<Object> {
    let mut merged = lopdf::Dictionary::new();
    let mut at = page_id;
    let mut found = false;
    for _ in 0..32 {
        let Ok(dict) = doc.get_dictionary(at) else {
            break;
        };
        if let Some((_, Object::Dictionary(resources))) = dict
            .get(b"Resources")
            .ok()
            .and_then(|object| doc.dereference(object).ok())
        {
            found = true;
            for (key, value) in resources.iter() {
                // Nearest wins: the page's own before the tree's.
                if merged.get(key).is_err() {
                    merged.set(key.to_vec(), value.clone());
                }
            }
        }
        let Ok(parent) = dict.get(b"Parent").and_then(Object::as_reference) else {
            break;
        };
        at = parent;
    }
    found.then_some(Object::Dictionary(merged))
}

/// Copy an object, and everything it points at, from one document into another.
///
/// Object numbers are a document's own private business, so every reference has
/// to be renumbered on the way across. `carried` remembers what has already
/// made the trip: it keeps a font shared by twenty pages a single font, and —
/// because a number is written into it *before* what it points at is followed —
/// it stops a document that refers back to itself from going round forever.
///
/// Shared with [`crate::join`], which needs the same deep copy for a different
/// purpose: this one carries a page's furniture across so it can be drawn on
/// top of another, that one carries a whole page across so it can follow one.
/// Two spellings of "copy this object and its children" would be two places for
/// a cycle to hang or a font to go missing.
pub(crate) fn import(
    from: &Document,
    into: &mut Document,
    object: &Object,
    carried: &mut BTreeMap<ObjectId, ObjectId>,
) -> Object {
    match object {
        Object::Reference(id) => {
            if let Some(already) = carried.get(id) {
                return Object::Reference(*already);
            }
            let fresh = into.new_object_id();
            carried.insert(*id, fresh);
            let copied = match from.get_object(*id) {
                Ok(target) => import(from, into, target, carried),
                // A reference to nothing is written as nothing, rather than
                // failing the whole merge over a file that was already broken.
                Err(_) => Object::Null,
            };
            into.objects.insert(fresh, copied);
            Object::Reference(fresh)
        }
        Object::Array(items) => Object::Array(
            items
                .iter()
                .map(|item| import(from, into, item, carried))
                .collect(),
        ),
        Object::Dictionary(dict) => {
            let mut copy = lopdf::Dictionary::new();
            for (key, value) in dict.iter() {
                copy.set(key.to_vec(), import(from, into, value, carried));
            }
            Object::Dictionary(copy)
        }
        Object::Stream(stream) => {
            let mut copy = lopdf::Dictionary::new();
            for (key, value) in stream.dict.iter() {
                copy.set(key.to_vec(), import(from, into, value, carried));
            }
            // The bytes travel exactly as they are, filter and all, so an
            // already-compressed stream is not decoded and re-encoded on the
            // way — and an embedded font arrives byte for byte.
            Object::Stream(
                Stream::new(copy, stream.content.clone())
                    .with_compression(stream.allows_compression),
            )
        }
        other => other.clone(),
    }
}

#[cfg(test)]
#[path = "merge/tests.rs"]
mod tests;
