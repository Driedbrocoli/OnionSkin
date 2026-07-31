//! Several PDFs, one after another.
//!
//! [`crate::merge`] puts deltas on top of each other: three files, one sheet,
//! everything drawn in the same place. This does the other thing — three files,
//! three sheets, one after the other — and until now Onionskin could not do it
//! at all.
//!
//! ```text
//! onionskin join page-1.pdf page-2.pdf page-3.pdf --out stack.pdf
//! ```
//!
//! # Why it is needed
//!
//! Almost everything upstream of this program produces one page at a time. A
//! flatbed scanner does; so does a phone. Someone who has scanned twenty sheets
//! has twenty files, and `onionskin stack` — which reads a scanned stack and
//! matches each sheet to a page — wants one document with twenty pages in it.
//! Without a way to make that document, the whole stack workflow is out of
//! reach unless you already have some other PDF tool, which is precisely the
//! assumption this program exists to avoid.
//!
//! # Mixed paper is fine here
//!
//! Merging refuses to put an A4 delta and a Letter delta on the same sheet,
//! because one of them would print off the edge. Joining has no such problem:
//! each page keeps its own size, and a stack of A4 with one Letter page in it
//! is an ordinary thing to have. The sizes are reported, not refused.
//!
//! # What is carried across
//!
//! The page and everything it points at: its content, its fonts, its images,
//! its annotations. Object numbers are a document's private business, so
//! everything is renumbered on the way over, using the same deep copy the
//! merger uses. Sizes and rotations that a file kept on its page tree rather
//! than on the page itself are written onto the page as it lands, because the
//! tree it was inheriting from is left behind.
//!
//! What is *not* carried: the things that live above a page rather than on it —
//! bookmarks, a document-wide form, named destinations. A file that has any is
//! said so, rather than quietly losing them.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use lopdf::{dictionary, Document, Object, ObjectId};

use crate::geometry::PageSize;
use crate::merge::{import, inherited, is_locked, newest_version, sheet_of};

/// Things a page may inherit from the page tree above it.
///
/// The tree is not coming with it, so anything it was relying on has to be
/// written onto the page before it leaves.
const INHERITED: [&[u8]; 4] = [b"MediaBox", b"Resources", b"Rotate", b"CropBox"];

/// What one file put into the stack.
#[derive(Debug, Clone, PartialEq)]
pub struct Part {
    pub path: PathBuf,
    pub pages: usize,
    /// Where its pages ended up in the joined file, counting from one.
    pub first_page: usize,
    /// The size of its first page.
    pub page: PageSize,
    /// An earlier file in the list this one is byte-for-byte identical to.
    ///
    /// Unlike a merge — where the same delta twice means every letter printed
    /// twice in the same place, which is never wanted — the same file twice in
    /// a stack is a perfectly ordinary request. Two copies of a cover sheet is
    /// two copies of a cover sheet. Worth saying, not worth refusing.
    pub same_as: Option<PathBuf>,
}

/// What came of a join.
#[derive(Debug, Clone, PartialEq)]
pub struct Joined {
    pub pages: usize,
    pub from: Vec<Part>,
    /// Files that had something above the page level — bookmarks, a form,
    /// named destinations — which a stack of pages cannot carry.
    pub left_behind: Vec<(PathBuf, String)>,
}

impl Joined {
    /// Every distinct paper size in the stack, in the order first seen.
    pub fn sizes(&self) -> Vec<PageSize> {
        let mut seen: Vec<PageSize> = Vec::new();
        for part in &self.from {
            let known = seen.iter().any(|size| {
                (size.width_mm - part.page.width_mm).abs() < 0.5
                    && (size.height_mm - part.page.height_mm).abs() < 0.5
            });
            if !known {
                seen.push(part.page);
            }
        }
        seen
    }

    /// Whether the files look like they arrived in the shell's order rather
    /// than their own.
    ///
    /// `page-*.pdf` expands to `page-1 page-10 page-2`, because a shell sorts
    /// by character and 1 comes before 2. Nothing about that is wrong, and the
    /// join is exactly as asked — but a stack in that order is a stack in the
    /// wrong order, and it is far easier to notice here than after twenty
    /// sheets have gone through the printer.
    pub fn out_of_order(&self) -> Option<String> {
        let numbers: Vec<u64> = self
            .from
            .iter()
            .filter_map(|part| number_in(&part.path))
            .collect();
        // Only worth saying when every one of them is numbered: a stack of
        // "cover.pdf terms.pdf page-1.pdf" is not a numbered sequence at all.
        if numbers.len() != self.from.len() || numbers.len() < 2 {
            return None;
        }
        let broken = numbers.windows(2).position(|pair| pair[0] > pair[1])?;
        Some(format!(
            "{} comes before {} here, but {} is the larger number. \
             A shell expanding page-*.pdf sorts 10 before 2.",
            self.from[broken].path.display(),
            self.from[broken + 1].path.display(),
            numbers[broken],
        ))
    }

    pub fn describe(&self) -> String {
        let sizes = self.sizes();
        let paper = match sizes.len() {
            0 => "no pages".to_string(),
            1 => sizes[0].describe(),
            _ => {
                let named: Vec<String> = sizes.iter().map(PageSize::describe).collect();
                format!("mixed paper: {}", named.join(", "))
            }
        };
        let mut lines = vec![format!(
            "{} page(s) of {paper}, from {} file(s):",
            self.pages,
            self.from.len()
        )];
        for part in &self.from {
            let last = part.first_page + part.pages - 1;
            let where_it_went = if part.pages == 1 {
                format!("page {}", part.first_page)
            } else {
                format!("pages {}–{last}", part.first_page)
            };
            let mut line = format!("  {where_it_went}  {}", part.path.display());
            if let Some(same) = &part.same_as {
                line.push_str(&format!("  — the same file as {}", same.display()));
            }
            lines.push(line);
        }
        lines.join("\n")
    }
}

#[derive(Debug, thiserror::Error)]
pub enum JoinError {
    #[error("joining needs at least two files; {0} was given.")]
    TooFew(usize),
    #[error("could not read {path}: {source}")]
    Unreadable { path: PathBuf, source: lopdf::Error },
    #[error("{path} has no pages in it.")]
    Blank { path: PathBuf },
    #[error(
        "{path} is a protected PDF — its contents are encrypted, and this cannot \
         read them.\n    Every reader opens a file like this without asking for a \
         password, so it looks ordinary; what it carries is a restriction on \
         copying and changing, and joining is changing.\n    Open it in a PDF \
         reader and save or print it to a fresh PDF, then join that."
    )]
    Locked { path: PathBuf },
    #[error(
        "page {page} of {path} does not say what size paper it is for, and \
         neither does anything above it. Onionskin will not guess: the guess \
         would decide what size paper that page asks the printer for."
    )]
    NoPageBox { path: PathBuf, page: usize },
    #[error("could not write {path}: {source}")]
    NotWritten {
        path: PathBuf,
        source: std::io::Error,
    },
}

/// Put several PDFs into one, page after page, in the order given.
pub fn join(inputs: &[PathBuf], out: &Path, title: &str) -> Result<Joined, JoinError> {
    if inputs.len() < 2 {
        return Err(JoinError::TooFew(inputs.len()));
    }

    // Read them all before writing anything, so a file that cannot be read
    // does not leave half a stack behind.
    let mut sources: Vec<(PathBuf, Document, Vec<ObjectId>)> = Vec::new();
    for path in inputs {
        let doc = Document::load(path).map_err(|source| JoinError::Unreadable {
            path: path.clone(),
            source,
        })?;
        // See `merge::is_locked`: the bytes come across verbatim here, so a
        // protected file joins to a page that no reader will open — and nothing
        // said so.
        if is_locked(&doc) {
            return Err(JoinError::Locked { path: path.clone() });
        }
        let pages: Vec<ObjectId> = doc.get_pages().into_values().collect();
        if pages.is_empty() {
            return Err(JoinError::Blank { path: path.clone() });
        }
        sources.push((path.clone(), doc, pages));
    }

    // Every page measured up front, for the same reason the merger does it: a
    // page whose size cannot be read is refused before anything is written,
    // rather than landing on whatever paper the printer happens to hold.
    let mut sheets: Vec<Vec<crate::merge::Sheet>> = Vec::new();
    for (path, doc, pages) in &sources {
        let mut measured = Vec::new();
        for (page, id) in pages.iter().enumerate() {
            measured.push(sheet_of(doc, *id).ok_or_else(|| JoinError::NoPageBox {
                path: path.clone(),
                page: page + 1,
            })?);
        }
        sheets.push(measured);
    }

    let mut stack = Document::with_version(newest_version(&sources));
    let pages_id = stack.new_object_id();
    let mut kids: Vec<Object> = Vec::new();

    for (_, doc, pages) in &sources {
        // One import map per file, shared across its pages: a font used on
        // twenty pages crosses once rather than twenty times.
        let mut carried: BTreeMap<ObjectId, ObjectId> = BTreeMap::new();
        for page_id in pages {
            let landed = carry_page(doc, *page_id, &mut stack, pages_id, &mut carried);
            kids.push(Object::Reference(landed));
        }
    }

    let count = kids.len() as i64;
    stack.objects.insert(
        pages_id,
        Object::Dictionary(dictionary! {
            "Type" => "Pages",
            "Kids" => kids,
            "Count" => count,
        }),
    );
    let catalog_id = stack.add_object(dictionary! {
        "Type" => "Catalog",
        "Pages" => pages_id,
    });
    let info_id = stack.add_object(dictionary! {
        "Title" => Object::string_literal(title),
        "Producer" => Object::string_literal("Onionskin"),
    });
    stack.trailer.set("Root", catalog_id);
    stack.trailer.set("Info", info_id);

    stack.compress();
    stack.save(out).map_err(|source| JoinError::NotWritten {
        path: out.to_path_buf(),
        source,
    })?;

    Ok(Joined {
        pages: count as usize,
        from: parts(&sources, &sheets),
        left_behind: left_behind(&sources),
    })
}

/// Copy one page into the joined document, and return where it landed.
///
/// The page's own entries come across as they are, except `Parent`, which
/// pointed into a page tree that is being left behind. Anything the page was
/// inheriting from that tree is written onto the page itself on the way over,
/// because there will be nothing above it to inherit from.
fn carry_page(
    from: &Document,
    page_id: ObjectId,
    into: &mut Document,
    pages_id: ObjectId,
    carried: &mut BTreeMap<ObjectId, ObjectId>,
) -> ObjectId {
    let landed = into.new_object_id();
    // Recorded before anything is followed. An annotation that names the page
    // it sits on — which is ordinary, and required for a link — would otherwise
    // drag the old page in behind it, tree and all.
    carried.insert(page_id, landed);

    let mut page = lopdf::Dictionary::new();
    if let Ok(original) = from.get_dictionary(page_id) {
        for (key, value) in original.iter() {
            if key.as_slice() == b"Parent" {
                continue;
            }
            page.set(key.clone(), import(from, into, value, carried));
        }
    }
    for key in INHERITED {
        if page.get(key).is_ok() {
            continue;
        }
        if let Some(found) = inherited(from, page_id, key) {
            page.set(key.to_vec(), import(from, into, &found, carried));
        }
    }
    page.set("Type", "Page");
    page.set("Parent", pages_id);
    into.objects.insert(landed, Object::Dictionary(page));
    landed
}

/// Where each file's pages ended up, and which files are the same file.
fn parts(
    sources: &[(PathBuf, Document, Vec<ObjectId>)],
    sheets: &[Vec<crate::merge::Sheet>],
) -> Vec<Part> {
    let marks: Vec<Option<String>> = sources
        .iter()
        .map(|(path, _, _)| crate::history::fingerprint(path))
        .collect();
    let mut at = 1;
    let mut built = Vec::new();
    for (index, (path, _, pages)) in sources.iter().enumerate() {
        built.push(Part {
            path: path.clone(),
            pages: pages.len(),
            first_page: at,
            page: sheets[index]
                .first()
                .map(|sheet| sheet.size())
                .unwrap_or_else(|| PageSize::new(0.0, 0.0)),
            same_as: marks[index].as_ref().and_then(|mark| {
                marks[..index]
                    .iter()
                    .position(|earlier| earlier.as_deref() == Some(mark.as_str()))
                    .map(|earlier| sources[earlier].0.clone())
            }),
        });
        at += pages.len();
    }
    built
}

/// What each file had above the page level that a stack of pages cannot carry.
///
/// Bookmarks belong to a document, not to a page, and there is no honest way to
/// splice three documents' outlines into one. Nor is there one for a form: two
/// files can both have a field called `Name`, meaning different fields. Rather
/// than merge them wrongly or drop them silently, they are named.
fn left_behind(sources: &[(PathBuf, Document, Vec<ObjectId>)]) -> Vec<(PathBuf, String)> {
    let mut noted = Vec::new();
    for (path, doc, _) in sources {
        let Ok(root) = doc.catalog() else { continue };
        let mut lost = Vec::new();
        if root.get(b"Outlines").is_ok() && has_bookmarks(doc, root) {
            lost.push("bookmarks");
        }
        if has_form_fields(doc, root) {
            lost.push("form fields");
        }
        if root.get(b"Names").is_ok() {
            lost.push("named destinations");
        }
        if !lost.is_empty() {
            noted.push((path.clone(), lost.join(", ")));
        }
    }
    noted
}

/// An `Outlines` entry with nothing under it is not bookmarks.
///
/// Plenty of writers leave an empty outline dictionary behind. Reporting that
/// as lost bookmarks would be a warning about nothing, and a warning about
/// nothing teaches people to ignore warnings.
fn has_bookmarks(doc: &Document, root: &lopdf::Dictionary) -> bool {
    let Some(outlines) = resolve(doc, root.get(b"Outlines").ok()) else {
        return false;
    };
    let Ok(dict) = outlines.as_dict() else {
        return false;
    };
    dict.get(b"First").is_ok()
}

fn has_form_fields(doc: &Document, root: &lopdf::Dictionary) -> bool {
    let Some(form) = resolve(doc, root.get(b"AcroForm").ok()) else {
        return false;
    };
    let Ok(dict) = form.as_dict() else {
        return false;
    };
    match resolve(doc, dict.get(b"Fields").ok()) {
        Some(fields) => fields
            .as_array()
            .map(|list| !list.is_empty())
            .unwrap_or(false),
        None => false,
    }
}

/// Follow a reference once, so a value written either way reads the same.
fn resolve(doc: &Document, object: Option<&Object>) -> Option<Object> {
    match object? {
        Object::Reference(id) => doc.get_object(*id).ok().cloned(),
        other => Some(other.clone()),
    }
}

/// The last run of digits in a file's name.
///
/// The last, not the first: `2024-invoice-7.pdf` is number seven, and a scan
/// folder is full of names like that.
fn number_in(path: &Path) -> Option<u64> {
    let name = path.file_stem()?.to_string_lossy().into_owned();
    let digits: String = name
        .chars()
        .rev()
        .skip_while(|c| !c.is_ascii_digit())
        .take_while(char::is_ascii_digit)
        .collect();
    if digits.is_empty() {
        return None;
    }
    digits.chars().rev().collect::<String>().parse().ok()
}

#[cfg(test)]
#[path = "join/tests.rs"]
mod tests;
