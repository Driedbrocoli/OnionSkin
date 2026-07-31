//! Turning source documents into PDFs, and PDFs into rasters.
//!
//! Word documents go through LibreOffice in headless mode where it is
//! installed. That matters more than it looks: both the original and the
//! edited file must be laid out by the *same* engine at the *same* version, or
//! the two renders will disagree about kerning and line breaks and every glyph
//! on the page will show up as a difference. Since both files go through this
//! module in the same run, that holds by construction.
//!
//! # When LibreOffice is not there
//!
//! It used to be the end of the matter: no LibreOffice, no `.docx`. That is a
//! fair thing to ask of somebody who already has it and an unreasonable thing
//! to ask of everybody else, so Onionskin now reads `.docx`, `.odt` and plain
//! text itself — see [`crate::office::read`]. It is not as good: it sets the
//! text, the tables and the lists, and it will not match Word to the
//! millimetre. Which engine did the work comes back with the PDF, because the
//! difference matters when the sheet already in the tray was printed from
//! Word.

use std::path::{Path, PathBuf};
use std::process::Command;

use lopdf::{Dictionary, Object};

use crate::geometry::PageSize;

/// Extensions LibreOffice will convert. Anything else is refused up front
/// rather than failing deep inside a subprocess.
///
/// Everything LibreOffice opens, not only the word-processor formats. A
/// spreadsheet and a slide deck are printed onto paper like anything else, and
/// somebody adding a line to a printed invoice does not care that the invoice
/// began life in Calc. Refusing those was never a decision — it was a list
/// written when the only thing being tested was Word.
pub const CONVERTIBLE: &[&str] = &[
    // Word processors.
    "doc", "docx", "docm", "dot", "dotx", "dotm", "odt", "ott", "fodt", "sxw", "stw", "rtf", "wpd",
    "wps", "abw", "lwp", "uot", "hwp", // Spreadsheets.
    "xls", "xlsx", "xlsm", "xlt", "xltx", "ods", "ots", "fods", "sxc", "csv", "tsv", "dif", "slk",
    "dbf", "numbers", // Presentations.
    "ppt", "pptx", "pptm", "pps", "ppsx", "pot", "potx", "odp", "otp", "fodp", "sxi", "key",
    // Drawings.
    "odg", "otg", "fodg", "sxd", "vsd", "vsdx", "pub", "cdr", "wmf", "emf",
    // Plain and marked-up text.
    "txt", "text", "html", "htm", "xhtml", "xml", "md", "markdown",
];

/// Formats that need no conversion at all.
pub const PASSTHROUGH: &[&str] = &["pdf"];

/// Pictures, which this does not open — named so it can say so properly.
///
/// A scan of a printed sheet is a picture, and it is one of the two things
/// somebody is most likely to be holding. It has commands of its own
/// (`onionskin add`, `onionskin read`), so being handed one here is not a
/// mistake to be listed at, it is a signpost to be given.
pub const PICTURES: &[&str] = &[
    "png", "jpg", "jpeg", "tif", "tiff", "bmp", "gif", "webp", "heic", "heif",
];

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    /// LibreOffice could not produce a PDF from the input.
    #[error("{0}")]
    Conversion(String),
    /// The input document is unusable for a delta.
    #[error("{0}")]
    Document(String),
    #[error("{0}")]
    Pdfium(String),
    #[error("could not read {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
}

/// Where LibreOffice lives on this machine.
///
/// Checked in the order someone would want: what they told us, what is on the
/// path, then the usual places each platform puts it.
pub fn find_soffice() -> Option<PathBuf> {
    if let Ok(set) = std::env::var("ONIONSKIN_SOFFICE") {
        let path = PathBuf::from(set);
        if path.exists() {
            return Some(path);
        }
    }
    for name in ["soffice", "libreoffice", "libreoffice7.6", "openoffice"] {
        if let Some(found) = which(name) {
            return Some(found);
        }
    }
    for candidate in [
        // macOS, including the two names the project has shipped under.
        "/Applications/LibreOffice.app/Contents/MacOS/soffice",
        "/Applications/OpenOffice.app/Contents/MacOS/soffice",
        // Linux, installed by a package manager.
        "/usr/lib/libreoffice/program/soffice",
        "/usr/lib64/libreoffice/program/soffice",
        "/usr/local/lib/libreoffice/program/soffice",
        "/opt/libreoffice/program/soffice",
        // Snap and Flatpak put it somewhere of their own, and a great many
        // desktop Linux installs get it that way now.
        "/snap/bin/libreoffice",
        "/var/lib/snapd/snap/bin/libreoffice",
        "/var/lib/flatpak/exports/bin/org.libreoffice.LibreOffice",
        "/usr/lib/openoffice/program/soffice",
        // Windows.
        "C:\\Program Files\\LibreOffice\\program\\soffice.exe",
        "C:\\Program Files (x86)\\LibreOffice\\program\\soffice.exe",
        "C:\\Program Files\\LibreOffice 7\\program\\soffice.exe",
        "C:\\Program Files\\OpenOffice\\program\\soffice.exe",
        "C:\\Program Files (x86)\\OpenOffice 4\\program\\soffice.exe",
    ] {
        let path = PathBuf::from(candidate);
        if path.exists() {
            return Some(path);
        }
    }

    // A per-user Flatpak, which lives under the home directory and so cannot
    // be written down as a fixed path.
    if let Ok(home) = std::env::var("HOME") {
        for tail in [
            ".local/share/flatpak/exports/bin/org.libreoffice.LibreOffice",
            "Applications/LibreOffice.app/Contents/MacOS/soffice",
        ] {
            let path = PathBuf::from(&home).join(tail);
            if path.exists() {
                return Some(path);
            }
        }
    }
    None
}

/// The first executable of this name on PATH.
fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for directory in std::env::split_paths(&path) {
        let candidate = directory.join(name);
        if candidate.is_file() {
            return Some(candidate);
        }
        // Windows keeps the extension on.
        for extension in ["exe", "bat", "cmd"] {
            let candidate = directory.join(format!("{name}.{extension}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// A `file://` URL for a path, the way LibreOffice wants it.
///
/// Not a string concatenation: on Windows a bare `file:///C:\Users\...` is not
/// a URL LibreOffice will accept, and a space anywhere in the path breaks it
/// everywhere.
fn file_url(path: &Path) -> String {
    let absolute = path
        .canonicalize()
        .unwrap_or_else(|_| std::env::current_dir().unwrap_or_default().join(path));
    let text = absolute.to_string_lossy().replace('\\', "/");
    let mut url = String::from("file://");
    if !text.starts_with('/') {
        url.push('/');
    }
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' | b':' => {
                url.push(byte as char)
            }
            other => url.push_str(&format!("%{other:02X}")),
        }
    }
    url
}

/// Which of the two ways of opening a document was used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Opener {
    /// It was already a PDF.
    Already,
    /// LibreOffice laid it out.
    LibreOffice,
    /// Onionskin read it itself.
    Onionskin,
}

/// Which opener to use, when both are possible.
///
/// `ONIONSKIN_OFFICE=onionskin` forces the built-in reader — useful for seeing
/// what somebody without LibreOffice will get, and for a machine where the
/// installed LibreOffice is broken or too slow to wait for.
fn preference() -> Option<Opener> {
    match std::env::var("ONIONSKIN_OFFICE")
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase()
        .as_str()
    {
        "onionskin" | "native" | "builtin" | "self" => Some(Opener::Onionskin),
        "libreoffice" | "soffice" | "lo" => Some(Opener::LibreOffice),
        _ => None,
    }
}

/// A PDF for `source`, converting it if it is not one already.
pub fn to_pdf(source: &Path, workdir: &Path, timeout_secs: u64) -> Result<PathBuf, RenderError> {
    to_pdf_noting(source, workdir, timeout_secs).map(|(path, _, _)| path)
}

/// The same, saying who did the work and what they had to approximate.
///
/// The notes are not decoration. Onionskin's own reader will not put a line
/// break exactly where Word does, and the person about to feed a printed sheet
/// back into a printer is the one who needs to know that.
pub fn to_pdf_noting(
    source: &Path,
    workdir: &Path,
    timeout_secs: u64,
) -> Result<(PathBuf, Opener, Vec<String>), RenderError> {
    if !source.is_file() {
        return Err(RenderError::Document(format!(
            "no such file: {}",
            source.display()
        )));
    }
    // One of Onionskin's own documents, whatever it has been named. Opening it
    // as though the extension were the truth ends in "this PDF is damaged",
    // which is both wrong and unhelpful: the file is fine, and there is a
    // command that does what was wanted.
    if crate::document::Document::is_one(source) {
        return Err(RenderError::Document(format!(
            "{} is an Onionskin document, not a file to print onto.\n    \
             To make a PDF of it:      onionskin print {} -o out.pdf\n    \
             To put words on it:       onionskin write {} --at '25,40:words'",
            source.display(),
            source.display(),
            source.display()
        )));
    }

    let suffix = source
        .extension()
        .map(|s| s.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    if PASSTHROUGH.contains(&suffix.as_str()) {
        return Ok((source.to_path_buf(), Opener::Already, Vec::new()));
    }
    if !CONVERTIBLE.contains(&suffix.as_str()) {
        // A picture is not an oversight, it is a different job, and the sixty
        // formats below do not include it — so a list of them reads as "your
        // file is unusual" when the real answer is "there is a command for
        // that". Somebody holding a scan of a printed sheet is Onionskin's
        // most ordinary visitor; sending them off to read a list is the worst
        // possible reply.
        if PICTURES.contains(&suffix.as_str()) {
            return Err(RenderError::Document(format!(
                "'{}' is a picture, and this needs a document.\n    To write on a scanned \
                 sheet, use:  onionskin add <picture> --at-mm '45,63:words'\n    To turn one \
                 into words first, use:  onionskin read <picture>",
                source.display()
            )));
        }
        let mut supported: Vec<String> = CONVERTIBLE
            .iter()
            .chain(PASSTHROUGH.iter())
            .map(|e| format!(".{e}"))
            .collect();
        supported.sort();
        return Err(RenderError::Document(format!(
            "unsupported file type '.{suffix}'. Supported: {}",
            supported.join(", ")
        )));
    }

    let ourselves = crate::office::read::can_read(&suffix);
    let soffice = find_soffice();
    // LibreOffice first where it is there, because it lays a document out the
    // way the program that wrote it would. Ours is the answer for a machine
    // that has not got it, not a replacement for it.
    let use_ourselves = match preference() {
        Some(Opener::Onionskin) => ourselves,
        Some(Opener::LibreOffice) => false,
        _ => ourselves && soffice.is_none(),
    };

    if use_ourselves {
        return open_ourselves(source, workdir);
    }

    let Some(soffice) = soffice else {
        return Err(RenderError::Conversion(refuse_without_libreoffice(&suffix)));
    };

    std::fs::create_dir_all(workdir).map_err(|source| RenderError::Io {
        path: workdir.to_path_buf(),
        source,
    })?;
    // A private profile per conversion: LibreOffice refuses to run two headless
    // instances against one profile, which would otherwise break two documents
    // being converted at once.
    let tag = unique_tag();
    let profile = workdir.join(format!("lo-profile-{tag}"));
    let outdir = workdir.join(format!("lo-out-{tag}"));
    std::fs::create_dir_all(&outdir).map_err(|source| RenderError::Io {
        path: outdir.clone(),
        source,
    })?;

    let resolved = source
        .canonicalize()
        .unwrap_or_else(|_| source.to_path_buf());
    let output = Command::new(&soffice)
        .arg(format!("-env:UserInstallation={}", file_url(&profile)))
        .args([
            "--headless",
            "--norestore",
            "--invisible",
            "--nolockcheck",
            "--convert-to",
            "pdf:writer_pdf_Export",
            "--outdir",
        ])
        .arg(&outdir)
        .arg(&resolved)
        .output();
    let _ = std::fs::remove_dir_all(&profile);

    let output = output.map_err(|e| {
        RenderError::Conversion(format!("could not run {}: {e}", soffice.display()))
    })?;

    let mut produced: Vec<PathBuf> = std::fs::read_dir(&outdir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| {
                    p.extension()
                        .map(|s| s.eq_ignore_ascii_case("pdf"))
                        .unwrap_or(false)
                })
                .collect()
        })
        .unwrap_or_default();
    produced.sort();

    if produced.is_empty() {
        let detail: String = String::from_utf8_lossy(&output.stderr)
            .chars()
            .chain(String::from_utf8_lossy(&output.stdout).chars())
            .take(500)
            .collect();
        let name = source.file_name().unwrap_or_default().to_string_lossy();
        return Err(RenderError::Conversion(if detail.trim().is_empty() {
            format!("LibreOffice produced no PDF for {name}")
        } else {
            format!("LibreOffice produced no PDF for {name}\n{}", detail.trim())
        }));
    }
    let _ = timeout_secs;
    Ok((produced.remove(0), Opener::LibreOffice, Vec::new()))
}

/// Open a document with Onionskin's own reader.
fn open_ourselves(
    source: &Path,
    workdir: &Path,
) -> Result<(PathBuf, Opener, Vec<String>), RenderError> {
    std::fs::create_dir_all(workdir).map_err(|error| RenderError::Io {
        path: workdir.to_path_buf(),
        source: error,
    })?;
    let stem = source
        .file_stem()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "document".into());
    let into = workdir.join(format!("{stem}-{}.pdf", unique_tag()));

    let mut notes = crate::office::read::to_pdf(source, &into)
        .map_err(|error| RenderError::Conversion(error.to_string()))?;

    // No file name in it, so that two documents opened the same way say this
    // once between them rather than twice. And the reason has to be the real
    // one: saying LibreOffice is missing on a machine where it is installed
    // and was simply not asked for is a small lie that costs somebody an
    // afternoon.
    let instead = if find_soffice().is_some() {
        "unset ONIONSKIN_OFFICE so LibreOffice is used"
    } else {
        "install LibreOffice (libreoffice.org/download)"
    };
    notes.insert(
        0,
        format!(
            "Onionskin read the document itself, without a word processor. The \
             words, the tables and the lists are all there, but the lines may not \
             break exactly where Word or Writer would break them. If the sheet in \
             your tray was printed from one of those, {instead}, or export the \
             document to PDF from the program that wrote it."
        ),
    );
    Ok((into, Opener::Onionskin, notes))
}

/// What to say when a document needs LibreOffice and it is not there.
fn refuse_without_libreoffice(suffix: &str) -> String {
    let readable: Vec<String> = crate::office::read::READABLE
        .iter()
        .map(|kind| format!(".{kind}"))
        .collect();
    format!(
        "a '.{suffix}' needs LibreOffice, and it was not found.\n\
         Install it (https://www.libreoffice.org/download/) or set \
         ONIONSKIN_SOFFICE to the soffice binary.\n\
         You can also export the document to PDF yourself and use that.\n\
         Onionskin opens {} without any of that.",
        readable.join(", ")
    )
}

/// A name nothing else will pick, without a random number generator.
fn unique_tag() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{:x}-{:x}", std::process::id(), nanos & 0xffff_ffff)
}

/// Where a page's content actually sits, in PDF user space.
///
/// A page is not always the simple case of "a box starting at (0,0), the right
/// way up". It can have a media box with a non-zero origin, a crop box smaller
/// than the media box, and a `/Rotate` that turns it a quarter turn for
/// display. All three are ordinary in the wild — phone scans and anything that
/// has been through a PDF editor — and all three move where ink lands on the
/// physical sheet.
///
/// Onionskin renders and diffs in *display space*: the page as you see it,
/// origin at the top-left, already cropped and turned. The delta must then be
/// written back into the source's own frame, or it will not line up with the
/// sheet in the tray.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageFrame {
    pub media: (f64, f64, f64, f64),
    pub crop: (f64, f64, f64, f64),
    pub rotate: i64,
}

impl PageFrame {
    pub fn crop_size_pt(&self) -> (f64, f64) {
        (self.crop.2 - self.crop.0, self.crop.3 - self.crop.1)
    }

    /// The page as rendered: cropped, and turned if `/Rotate` says so.
    pub fn display_size(&self) -> PageSize {
        let (mut width, mut height) = self.crop_size_pt();
        if self.rotate == 90 || self.rotate == 270 {
            std::mem::swap(&mut width, &mut height);
        }
        PageSize::from_pt(width, height)
    }

    /// True when display space and user space are the same thing.
    pub fn is_simple(&self) -> bool {
        self.rotate == 0
            && self.crop.0.abs() < 1e-6
            && self.crop.1.abs() < 1e-6
            && (self.crop.0 - self.media.0).abs() < 1e-6
            && (self.crop.1 - self.media.1).abs() < 1e-6
            && (self.crop.2 - self.media.2).abs() < 1e-6
            && (self.crop.3 - self.media.3).abs() < 1e-6
    }

    pub fn describe(&self) -> String {
        let mut bits = Vec::new();
        if self.rotate != 0 {
            bits.push(format!("rotated {}°", self.rotate));
        }
        if self.crop.0.abs() > 1e-6 || self.crop.1.abs() > 1e-6 {
            bits.push(format!(
                "origin at ({:.1}, {:.1}) pt",
                self.crop.0, self.crop.1
            ));
        }
        if (self.crop.0 - self.media.0).abs() > 1e-6
            || (self.crop.1 - self.media.1).abs() > 1e-6
            || (self.crop.2 - self.media.2).abs() > 1e-6
            || (self.crop.3 - self.media.3).abs() > 1e-6
        {
            bits.push("cropped".into());
        }
        if bits.is_empty() {
            "standard".into()
        } else {
            bits.join(", ")
        }
    }
}

/// Turn a library's complaint into something a person can act on.
pub fn unreadable(path: &Path, detail: &str) -> String {
    let name = path.file_name().unwrap_or_default().to_string_lossy();
    let lower = detail.to_lowercase();
    if lower.contains("password") || lower.contains("encrypt") {
        return format!(
            "{name} is password-protected. Open it in a PDF reader, save an \
             unprotected copy, and use that."
        );
    }
    if path.metadata().map(|m| m.len() == 0).unwrap_or(false) {
        return format!("{name} is empty (0 bytes).");
    }
    format!(
        "{name} could not be opened as a PDF. It may be damaged, incomplete, or \
         not really a PDF.\n    ({detail})"
    )
}

/// Read every page's box geometry, resolving inherited attributes.
///
/// MediaBox, CropBox and Rotate are inheritable: a page may not carry them at
/// all and take them from an ancestor in the page tree. A reader that only
/// looks at the page itself gets the default Letter-sized box, and every
/// measurement after that is wrong by however much the real page differs.
pub fn read_frames(pdf: &lopdf::Document) -> Result<Vec<PageFrame>, RenderError> {
    let mut frames = Vec::new();
    for (number, id) in pdf.get_pages() {
        let page = pdf
            .get_dictionary(id)
            .map_err(|e| RenderError::Document(format!("page {number} is unreadable: {e}")))?;

        let media = inherited_box(pdf, page, b"MediaBox")
            // A PDF with no MediaBox anywhere is malformed, but every reader
            // falls back to US Letter rather than refusing to open it.
            .unwrap_or((0.0, 0.0, 612.0, 792.0));
        let crop = inherited_box(pdf, page, b"CropBox").unwrap_or(media);

        let rotate = inherited_number(pdf, page, b"Rotate").unwrap_or(0.0) as i64;
        let rotate = rotate.rem_euclid(360);
        if rotate % 90 != 0 {
            return Err(RenderError::Document(format!(
                "page {number} is rotated {rotate}°, which the PDF specification \
                 does not allow (it must be a multiple of 90)"
            )));
        }

        // A crop box is only meaningful where it meets the media box.
        let mut crop = (
            crop.0.max(media.0),
            crop.1.max(media.1),
            crop.2.min(media.2),
            crop.3.min(media.3),
        );
        if crop.2 - crop.0 <= 0.0 || crop.3 - crop.1 <= 0.0 {
            crop = media;
        }
        frames.push(PageFrame {
            media,
            crop,
            rotate,
        });
    }
    Ok(frames)
}

/// A rectangle from this page or the nearest ancestor that has one.
fn inherited_box(
    pdf: &lopdf::Document,
    page: &Dictionary,
    key: &[u8],
) -> Option<(f64, f64, f64, f64)> {
    let object = inherited(pdf, page, key)?;
    let array = pdf.dereference(&object).ok()?.1.as_array().ok()?;
    if array.len() < 4 {
        return None;
    }
    let value = |index: usize| -> Option<f64> {
        pdf.dereference(&array[index])
            .ok()
            .and_then(|(_, o)| o.as_float().ok())
            .map(|v| v as f64)
    };
    let (a, b, c, d) = (value(0)?, value(1)?, value(2)?, value(3)?);
    // A rectangle may be given with either pair of corners first.
    Some((a.min(c), b.min(d), a.max(c), b.max(d)))
}

fn inherited_number(pdf: &lopdf::Document, page: &Dictionary, key: &[u8]) -> Option<f64> {
    let object = inherited(pdf, page, key)?;
    pdf.dereference(&object)
        .ok()
        .and_then(|(_, o)| o.as_float().ok())
        .map(|v| v as f64)
}

/// Walk up the page tree looking for an attribute.
fn inherited(pdf: &lopdf::Document, page: &Dictionary, key: &[u8]) -> Option<Object> {
    if let Ok(object) = page.get(key) {
        return Some(object.clone());
    }
    let mut node = page.clone();
    // Bounded: a page tree deeper than this is a cycle, not a document.
    for _ in 0..64 {
        let parent = node.get(b"Parent").ok()?.clone();
        let (_, resolved) = pdf.dereference(&parent).ok()?;
        let dictionary = resolved.as_dict().ok()?;
        if let Ok(object) = dictionary.get(key) {
            return Some(object.clone());
        }
        node = dictionary.clone();
    }
    None
}

/// One page turned into pixels, with only the grey of it.
///
/// The colour costs three bytes a pixel — forty-six megabytes on an A4 page at
/// four hundred dots an inch — and most of what Onionskin does with a page
/// never looks at it. Comparing two documents needs the grey and nothing else;
/// only the sheet being *written* is needed in colour, and only when a delta is
/// actually being built. Rendering the rest in grey alone is forty-six
/// megabytes a page not allocated, not filled in, and not handed back.
#[derive(Debug, Clone)]
pub struct GrayPage {
    pub index: usize,
    pub size: PageSize,
    pub width: usize,
    pub height: usize,
    /// One byte per pixel.
    pub gray: Vec<u8>,
}

/// One page turned into pixels.
#[derive(Debug, Clone)]
pub struct RenderedPage {
    pub index: usize,
    pub size: PageSize,
    pub width: usize,
    pub height: usize,
    /// Three bytes per pixel.
    pub rgb: Vec<u8>,
    /// One byte per pixel.
    pub gray: Vec<u8>,
}

/// Crop or pad an image to exactly `width` × `height`, paper-side out.
pub(crate) fn fit(
    source: &[u8],
    from: (usize, usize),
    to: (usize, usize),
    channels: usize,
    fill: u8,
) -> Vec<u8> {
    if from == to {
        return source.to_vec();
    }
    let mut out = vec![fill; to.0 * to.1 * channels];
    let rows = from.1.min(to.1);
    let columns = from.0.min(to.0);
    for y in 0..rows {
        let src = y * from.0 * channels;
        let dst = y * to.0 * channels;
        out[dst..dst + columns * channels].copy_from_slice(&source[src..src + columns * channels]);
    }
    out
}

/// The rendering engine, bound once for the life of the process.
///
/// pdfium is a C library loaded at run time, and binding it twice in one
/// process is not safe. It is also the one part of Onionskin that is not pure
/// Rust, so where it is found and what to do when it is missing are worth
/// saying out loud rather than leaving to a link error.
pub struct Engine {
    pdfium: pdfium_render::prelude::Pdfium,
}

impl std::fmt::Debug for Engine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Engine(pdfium)")
    }
}

/// The one engine this process gets.
///
/// pdfium initialises global state, and initialising it twice aborts. So it is
/// bound once, on first use, and everything shares it. The failure is cached
/// too: a machine without the library will not find it on the second attempt
/// either, and re-searching the filesystem per page would be pure waste.
static ENGINE: std::sync::OnceLock<Result<Shared, String>> = std::sync::OnceLock::new();

/// The engine, in a form that can live in a static.
///
/// pdfium's bindings are not marked as safe to share between threads, and on
/// the face of it that settles the matter. But the crate is built here with its
/// `thread_safe` feature, which puts every single call to the library behind
/// one mutex — so what reaches pdfium is a strictly serialised stream of calls
/// from one thread at a time, which is exactly the condition it asks for. The
/// marker is missing because it is a property of how the crate is configured
/// rather than of the type, and there is no way to say so in the type system.
struct Shared(Engine);

// SAFETY: every call into the library goes through the mutex that the crate's
// `thread_safe` feature installs, so no two threads are ever inside pdfium at
// once. Removing that feature from Cargo.toml would make this unsound, which is
// why it is not optional there.
unsafe impl Send for Shared {}
unsafe impl Sync for Shared {}

/// Held for as long as anyone is using the renderer.
///
/// The `thread_safe` feature makes each individual call into pdfium safe, and
/// that is not enough. Rendering a page is a *sequence* of calls — load the
/// document, take a page, draw it, drop it — and pdfium keeps state across
/// them. Two threads whose sequences interleave will segfault inside the
/// library, which is exactly what happened here the first time two documents
/// were opened at once. So a whole session gets the renderer to itself.
static IN_USE: std::sync::Mutex<()> = std::sync::Mutex::new(());

thread_local! {
    /// How many guards this thread is holding.
    ///
    /// The lock has to be re-entrant, because opening the original and the
    /// edited document at the same time is the ordinary case and a plain mutex
    /// would deadlock the moment it happened. A thread that already has the
    /// renderer simply keeps it.
    static DEPTH: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Exclusive use of the renderer, for as long as this lives.
pub struct EngineGuard {
    engine: &'static Engine,
    /// `None` when this thread already held the lock further up the stack.
    _lock: Option<std::sync::MutexGuard<'static, ()>>,
}

impl std::ops::Deref for EngineGuard {
    type Target = Engine;
    fn deref(&self) -> &Engine {
        self.engine
    }
}

impl Drop for EngineGuard {
    fn drop(&mut self) {
        DEPTH.with(|depth| depth.set(depth.get().saturating_sub(1)));
        // The mutex guard, if this was the outermost one, is released after
        // this — fields drop last, which is the order we want.
    }
}

/// The rendering engine, binding it if this is the first time.
///
/// Blocks while another thread is rendering. Hold the guard for as long as any
/// [`Document`] opened from it is alive — the borrow checker enforces that,
/// since a `Document` borrows from the guard.
pub fn engine() -> Result<EngineGuard, RenderError> {
    let engine = match ENGINE.get_or_init(|| Engine::bind().map(Shared).map_err(|e| e.to_string()))
    {
        Ok(shared) => &shared.0,
        Err(message) => return Err(RenderError::Pdfium(message.clone())),
    };

    let already_held = DEPTH.with(|depth| {
        let held = depth.get();
        depth.set(held + 1);
        held
    });
    let lock = if already_held == 0 {
        // A panic while rendering poisons the lock. The renderer is not left
        // in a state that matters — the next caller opens its own document —
        // so carrying on is better than refusing every later run.
        Some(
            IN_USE
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        )
    } else {
        None
    };
    Ok(EngineGuard {
        engine,
        _lock: lock,
    })
}

/// Where a system package puts the renderer.
///
/// A `.deb` cannot drop a private library into `/usr/bin` beside the binary —
/// Debian puts it in a directory of the package's own — so the packager's
/// layout and this list have to agree. When they do not, an installed copy
/// loses PDF rendering and nothing says why. There is a test in
/// [`crate::package`] holding the two together.
pub const PACKAGED_LIBRARY_PATHS: &[&str] = &[
    "/usr/lib/onionskin/libpdfium.so",
    "/usr/local/lib/onionskin/libpdfium.so",
    "/usr/lib/onionskin/libpdfium.dylib",
    "/usr/local/lib/onionskin/libpdfium.dylib",
];

impl Engine {
    /// Find and bind pdfium.
    ///
    /// Prefer [`engine`], which does this once. Calling this twice in one
    /// process is not safe — pdfium's own initialiser aborts on the second go.
    pub fn bind() -> Result<Engine, RenderError> {
        use pdfium_render::prelude::*;

        let mut tried: Vec<String> = Vec::new();
        let mut candidates: Vec<PathBuf> = Vec::new();
        if let Ok(set) = std::env::var("ONIONSKIN_PDFIUM") {
            candidates.push(PathBuf::from(set));
        }
        // Beside the binary is where a packaged build puts it.
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                for name in [
                    "libpdfium.so",
                    "libpdfium.dylib",
                    "pdfium.dll",
                    "libpdfium.dll",
                ] {
                    candidates.push(dir.join(name));
                }
            }
        }
        for path in PACKAGED_LIBRARY_PATHS {
            candidates.push(PathBuf::from(*path));
        }
        for path in [
            "/usr/lib/libpdfium.so",
            "/usr/local/lib/libpdfium.so",
            "/usr/lib/x86_64-linux-gnu/libpdfium.so",
            "/opt/homebrew/lib/libpdfium.dylib",
            "/usr/local/lib/python3.11/dist-packages/pypdfium2_raw/libpdfium.so",
        ] {
            candidates.push(PathBuf::from(path));
        }

        for candidate in candidates {
            if !candidate.exists() {
                continue;
            }
            match Pdfium::bind_to_library(&candidate) {
                Ok(bindings) => {
                    return Ok(Engine {
                        pdfium: Pdfium::new(bindings),
                    })
                }
                Err(e) => tried.push(format!("{}: {e}", candidate.display())),
            }
        }
        if let Ok(bindings) = Pdfium::bind_to_system_library() {
            return Ok(Engine {
                pdfium: Pdfium::new(bindings),
            });
        }

        let detail = if tried.is_empty() {
            String::new()
        } else {
            format!("\n    Tried: {}", tried.join("\n           "))
        };
        // Naming the download matters. "Put libpdfium next to the binary" is
        // useless advice to somebody who has never heard of pdfium and has no
        // idea where one comes from, and that is most people who will read it.
        let file = if cfg!(windows) {
            "pdfium.dll"
        } else if cfg!(target_os = "macos") {
            "libpdfium.dylib"
        } else {
            "libpdfium.so"
        };
        let build = if cfg!(windows) {
            "pdfium-win-x64.tgz"
        } else if cfg!(target_os = "macos") {
            "pdfium-mac-arm64.tgz (or -mac-x64 on an Intel Mac)"
        } else {
            "pdfium-linux-x64.tgz"
        };
        Err(RenderError::Pdfium(format!(
            "the PDF rendering library was not found.\n    \
             Onionskin draws PDF pages with pdfium, which is Google's renderer \
             from Chromium.\n    Everything works without it except comparing \
             two documents.\n\n    \
             Get it from https://github.com/bblanchon/pdfium-binaries/releases\n    \
             — download {build}, and put the {file} inside it next to the\n      \
             onionskin program. Or set ONIONSKIN_PDFIUM to wherever you keep it.\n\n    \
             It is BSD and Apache licensed, like Onionskin itself, so there is \
             nothing to agree to.{detail}"
        )))
    }

    /// Open a PDF for measuring and rasterising.
    pub fn open<'a>(&'a self, path: &Path) -> Result<Document<'a>, RenderError> {
        let bytes = std::fs::read(path).map_err(|source| RenderError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let structure = lopdf::Document::load_mem(&bytes)
            .map_err(|e| RenderError::Document(unreadable(path, &e.to_string())))?;
        let frames = read_frames(&structure)?;
        if frames.is_empty() {
            return Err(RenderError::Document(format!(
                "{} has no pages",
                path.file_name().unwrap_or_default().to_string_lossy()
            )));
        }

        let pdf = self
            .pdfium
            .load_pdf_from_file(path, None)
            .map_err(|e| RenderError::Document(unreadable(path, &e.to_string())))?;

        let seen = pdf.pages().len() as usize;
        if seen != frames.len() {
            return Err(RenderError::Document(format!(
                "{} is inconsistent: the renderer sees {seen} page(s), the page \
                 tree has {}",
                path.file_name().unwrap_or_default().to_string_lossy(),
                frames.len()
            )));
        }

        let page_sizes = frames.iter().map(|f| f.display_size()).collect();
        Ok(Document {
            path: path.to_path_buf(),
            pdf,
            frames,
            page_sizes,
        })
    }
}

/// A PDF opened for measurement and rasterising.
pub struct Document<'a> {
    pub path: PathBuf,
    pdf: pdfium_render::prelude::PdfDocument<'a>,
    pub frames: Vec<PageFrame>,
    pub page_sizes: Vec<PageSize>,
}

impl Document<'_> {
    pub fn len(&self) -> usize {
        self.page_sizes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.page_sizes.is_empty()
    }

    /// Draw one page at `dpi`.
    pub fn render(&self, index: usize, dpi: f64) -> Result<RenderedPage, RenderError> {
        let (width, height, size, rgb, gray) = self.raster(index, dpi, true)?;
        Ok(RenderedPage {
            index,
            size,
            width,
            height,
            rgb,
            gray,
        })
    }

    /// The words on a page, as the document itself carries them.
    ///
    /// Not read off a picture of the page — asked of the document, which is
    /// the only way to find text that is *there* without being visible. That
    /// is the whole point of it: a black rectangle drawn over a salary leaves
    /// the salary in the file, selectable, copyable and searchable, and no
    /// amount of looking at the page will show it. So a redaction is checked
    /// by asking this, after the fact, and a redaction that cannot be checked
    /// is not one.
    pub fn text_on(&self, index: usize) -> Result<String, RenderError> {
        let page = self
            .pdf
            .pages()
            .get(index as u16)
            .map_err(|e| RenderError::Pdfium(format!("page {} : {e}", index + 1)))?;
        Ok(page.text().map(|text| text.all()).unwrap_or_default())
    }

    /// The same page, in grey only.
    ///
    /// See [`GrayPage`]: the colour is three bytes a pixel that most callers
    /// never read.
    pub fn render_gray(&self, index: usize, dpi: f64) -> Result<GrayPage, RenderError> {
        let (width, height, size, _, gray) = self.raster(index, dpi, false)?;
        Ok(GrayPage {
            index,
            size,
            width,
            height,
            gray,
        })
    }

    /// With the colour or without it, decided at the call.
    ///
    /// For the one caller that cannot know until it is running whether the
    /// colour will be wanted: a delta needs it, a report does not, and both go
    /// down the same loop.
    pub fn render_either(
        &self,
        index: usize,
        dpi: f64,
        want_colour: bool,
    ) -> Result<RenderedPage, RenderError> {
        let (width, height, size, rgb, gray) = self.raster(index, dpi, want_colour)?;
        Ok(RenderedPage {
            index,
            size,
            width,
            height,
            rgb,
            gray,
        })
    }

    /// Draw a page, keeping the colour only if it was asked for.
    #[allow(clippy::type_complexity)]
    fn raster(
        &self,
        index: usize,
        dpi: f64,
        want_colour: bool,
    ) -> Result<(usize, usize, PageSize, Vec<u8>, Vec<u8>), RenderError> {
        use pdfium_render::prelude::*;

        let size = self.page_sizes[index];
        let page = self
            .pdf
            .pages()
            .get(index as u16)
            .map_err(|e| RenderError::Pdfium(format!("page {} : {e}", index + 1)))?;

        let (target_w, target_h) = size.px_size(dpi);
        let config = PdfRenderConfig::new()
            .set_target_width(target_w as i32)
            .set_target_height(target_h as i32)
            .render_form_data(false)
            .render_annotations(true);

        let bitmap = page
            .render_with_config(&config)
            .map_err(|e| RenderError::Pdfium(format!("page {} : {e}", index + 1)))?;

        let width = bitmap.width() as usize;
        let height = bitmap.height() as usize;
        let raw = bitmap.as_rgba_bytes();

        let mut rgb = Vec::with_capacity(if want_colour { width * height * 3 } else { 0 });
        let mut gray = Vec::with_capacity(width * height);
        for pixel in raw.chunks_exact(4) {
            let (r, g, b) = (pixel[0], pixel[1], pixel[2]);
            if want_colour {
                rgb.extend_from_slice(&[r, g, b]);
            }
            // The same luma weights every image library uses, so a page
            // rasterised here and one rasterised elsewhere agree about grey.
            gray.push(((r as u32 * 299 + g as u32 * 587 + b as u32 * 114) / 1000).min(255) as u8);
        }

        // pdfium rounds each axis independently, so a page can come back a
        // pixel off. Both documents must land on identical rasters for the diff
        // to be a straight comparison — but the difference is never more than a
        // pixel, so crop or pad rather than resample. Resampling a
        // thirteen-megapixel page to move it one pixel costs a fifth of the run
        // time and blurs every glyph edge.
        let target = (target_w as usize, target_h as usize);
        if (width, height) != target {
            if want_colour {
                rgb = fit(&rgb, (width, height), target, 3, 255);
            }
            gray = fit(&gray, (width, height), target, 1, 255);
        }

        Ok((target.0, target.1, size, rgb, gray))
    }
}

/// A scratch directory that cleans itself up.
#[derive(Debug)]
pub struct Workspace {
    pub path: PathBuf,
    pub keep: bool,
}

impl Workspace {
    pub fn new(keep: bool) -> Result<Workspace, RenderError> {
        let path = std::env::temp_dir().join(format!("onionskin-{}", unique_tag()));
        std::fs::create_dir_all(&path).map_err(|source| RenderError::Io {
            path: path.clone(),
            source,
        })?;
        // Owner-only: working files hold whole documents, and other accounts on
        // a shared machine have no business reading them.
        restrict(&path);
        Ok(Workspace { path, keep })
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        if !self.keep {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }
}

/// Make a directory readable only by its owner.
pub fn restrict(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700));
    }
    #[cfg(not(unix))]
    {
        // Windows inherits the parent's ACL, and the temp directory is already
        // per-user. There is nothing useful to tighten here.
        let _ = path;
    }
}

#[cfg(test)]
mod tests;
