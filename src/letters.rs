//! Reading the letters off a scanned page.
//!
//! Registration tells Onionskin where the *sheet* is. This tells it where the
//! *words* are: every mark of ink on the paper, grouped into letters, words and
//! lines, and reported in millimetres from the top-left corner — the same
//! ruler-on-the-page coordinates everything else in Onionskin speaks.
//!
//! Two things fall out of that, and both matter more than the reading itself:
//!
//! * you can point at a gap and know it really is a gap, rather than
//!   discovering at the printer that your new line lands on top of a footnote;
//! * you can find the label — the "Date:" or the "Signed:" — and put the words
//!   beside it, without measuring anything.
//!
//! Recognising *which* letter each mark is comes second, and only when a font
//! is supplied. That is not a limitation dressed up as a virtue: a page is set
//! in some font, and comparing ink against the glyphs of the font it was set in
//! is both far more accurate and far less code than guessing from scratch. Give
//! it the wrong font and it says so — every letter comes back with how well it
//! actually matched, and a poor match is reported as unread rather than as a
//! confident wrong answer. Onionskin is a tool for putting ink on paper that
//! may be someone's only copy; a plausible guess is worse than a blank.

use image::GrayImage;

use crate::font::EmbeddedFont;
use crate::scan::{otsu_of_histogram, Mapping, ScanRegistration};

/// A rectangle on the paper, in millimetres from the top-left corner.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize)]
pub struct Rect {
    pub x_mm: f64,
    pub y_mm: f64,
    pub width_mm: f64,
    pub height_mm: f64,
}

impl Rect {
    pub fn right_mm(&self) -> f64 {
        self.x_mm + self.width_mm
    }
    pub fn bottom_mm(&self) -> f64 {
        self.y_mm + self.height_mm
    }
    pub fn centre(&self) -> (f64, f64) {
        (
            self.x_mm + self.width_mm / 2.0,
            self.y_mm + self.height_mm / 2.0,
        )
    }

    /// The smallest rectangle holding both.
    pub fn union(&self, other: &Rect) -> Rect {
        let x0 = self.x_mm.min(other.x_mm);
        let y0 = self.y_mm.min(other.y_mm);
        let x1 = self.right_mm().max(other.right_mm());
        let y1 = self.bottom_mm().max(other.bottom_mm());
        Rect {
            x_mm: x0,
            y_mm: y0,
            width_mm: x1 - x0,
            height_mm: y1 - y0,
        }
    }

    /// How much of the shorter one's height the two share, 0 to 1.
    fn vertical_overlap(&self, other: &Rect) -> f64 {
        let top = self.y_mm.max(other.y_mm);
        let bottom = self.bottom_mm().min(other.bottom_mm());
        let shared = (bottom - top).max(0.0);
        let shorter = self.height_mm.min(other.height_mm);
        if shorter <= 0.0 {
            0.0
        } else {
            shared / shorter
        }
    }

    /// How much of the narrower one's width the two share, 0 to 1.
    fn horizontal_overlap(&self, other: &Rect) -> f64 {
        let left = self.x_mm.max(other.x_mm);
        let right = self.right_mm().min(other.right_mm());
        let shared = (right - left).max(0.0);
        let narrower = self.width_mm.min(other.width_mm);
        if narrower <= 0.0 {
            0.0
        } else {
            shared / narrower
        }
    }

    /// Do the two rectangles touch at all?
    pub fn intersects(&self, other: &Rect) -> bool {
        self.x_mm < other.right_mm()
            && other.x_mm < self.right_mm()
            && self.y_mm < other.bottom_mm()
            && other.y_mm < self.bottom_mm()
    }
}

/// One letter's worth of ink on the sheet.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Letter {
    /// Where it sits on the paper.
    pub rect: Rect,
    /// How much ink it is, in square millimetres. A hollow `o` is far less
    /// than its box; that difference is what tells the two apart.
    pub ink_mm2: f64,
    /// What it was read as, if a font was given and the match was good enough.
    pub text: Option<char>,
    /// How well the ink matched the glyph, 0 to 1. Zero when nothing was tried.
    pub confidence: f64,
    /// The mark itself, straightened, for matching. Not part of the report.
    #[serde(skip)]
    stamp: Stamp,
}

/// A run of letters with no space in it.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Word {
    pub rect: Rect,
    pub letters: Vec<Letter>,
}

impl Word {
    /// The word as text, if every letter in it was read.
    pub fn text(&self) -> Option<String> {
        self.letters.iter().map(|l| l.text).collect()
    }

    /// The word as text, with an unread letter standing in as `?`.
    pub fn text_lossy(&self) -> String {
        self.letters.iter().map(|l| l.text.unwrap_or('?')).collect()
    }
}

/// A row of words sharing a baseline.
#[derive(Debug, Clone, serde::Serialize)]
pub struct TextLine {
    pub rect: Rect,
    /// Where the letters sit, in millimetres down the page. Text is placed
    /// from its baseline, so this is the number to give `add --at-mm`.
    pub baseline_mm: f64,
    pub words: Vec<Word>,
}

impl TextLine {
    pub fn text_lossy(&self) -> String {
        self.words
            .iter()
            .map(|w| w.text_lossy())
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub fn letters(&self) -> impl Iterator<Item = &Letter> {
        self.words.iter().flat_map(|w| w.letters.iter())
    }
}

/// Everything found on one scanned sheet.
#[derive(Debug, Clone, serde::Serialize)]
pub struct PageText {
    pub lines: Vec<TextLine>,
    /// Marks that were thrown away as too small or too large to be a letter.
    /// Counted rather than listed, so a page of dust does not pass unnoticed.
    pub discarded: usize,
}

impl PageText {
    pub fn letters(&self) -> impl Iterator<Item = &Letter> {
        self.lines.iter().flat_map(|l| l.letters())
    }

    pub fn letter_count(&self) -> usize {
        self.letters().count()
    }

    pub fn word_count(&self) -> usize {
        self.lines.iter().map(|l| l.words.len()).sum()
    }

    /// How many letters were actually recognised.
    pub fn read_count(&self) -> usize {
        self.letters().filter(|l| l.text.is_some()).count()
    }

    /// The page as text, one line per line, unread letters as `?`.
    pub fn text_lossy(&self) -> String {
        self.lines
            .iter()
            .map(|l| l.text_lossy())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Every rectangle that already has ink in it, for keeping clear of.
    pub fn occupied(&self) -> Vec<Rect> {
        self.letters().map(|l| l.rect).collect()
    }

    /// Is this rectangle clear of existing ink?
    ///
    /// The question a delta actually has to answer before it prints: adding a
    /// word on top of a word makes both unreadable, and unlike everything else
    /// here that mistake cannot be undone.
    pub fn is_clear(&self, area: &Rect) -> bool {
        !self.letters().any(|l| l.rect.intersects(area))
    }

    /// The line whose text most nearly matches, for finding a labelled field.
    pub fn find_line(&self, needle: &str) -> Option<&TextLine> {
        let needle = needle.to_lowercase();
        self.lines
            .iter()
            .find(|line| line.text_lossy().to_lowercase().contains(&needle))
    }
}

/// Tuning for [`read`]. The defaults suit ordinary printed text.
#[derive(Debug, Clone, Copy)]
pub struct ReadOptions {
    /// Smallest mark to keep, millimetres tall. Below this is dust on the
    /// glass and sensor noise: at 300 dpi that is under seven pixels.
    pub min_height_mm: f64,
    /// Largest mark to treat as a letter. Above this it is a photograph, a
    /// logo or a box rule — real ink, but not a letter.
    pub max_height_mm: f64,
    /// Ignore ink within this far of the paper's edge. A scanner's own shadow
    /// falls exactly there and reads as a long thin letter otherwise.
    pub edge_margin_mm: f64,
    /// How well ink must match a glyph to be read as it, 0 to 1. Below this
    /// the letter is reported as found but unread.
    pub min_confidence: f64,
}

impl Default for ReadOptions {
    fn default() -> Self {
        Self {
            min_height_mm: 0.5,
            max_height_mm: 25.0,
            edge_margin_mm: 2.0,
            min_confidence: 0.55,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    #[error("the scan has no pixels")]
    Empty,
    #[error("{0}")]
    Unusable(String),
}

/// Find every letter on a registered scan.
pub fn read(
    image: &GrayImage,
    registration: &ScanRegistration,
    options: &ReadOptions,
) -> Result<PageText, ReadError> {
    let (width, height) = image.dimensions();
    if width == 0 || height == 0 {
        return Err(ReadError::Empty);
    }

    // Only the paper counts. A flatbed's backing is darker than any ink on the
    // sheet, so leaving it in makes the threshold split paper from backing
    // instead of ink from paper — and then the whole border is one enormous
    // "letter" and the actual text is not ink at all.
    let inside = paper_mask(image, registration, options);
    let mut histogram = [0u64; 256];
    for (value, &on_paper) in image.as_raw().iter().zip(inside.iter()) {
        if on_paper {
            histogram[*value as usize] += 1;
        }
    }
    let threshold = otsu_of_histogram(&histogram);

    let ink: Vec<bool> = image
        .as_raw()
        .iter()
        .zip(inside.iter())
        .map(|(&value, &on_paper)| on_paper && value <= threshold)
        .collect();

    let (labels, count) = label_components(&ink, width as usize, height as usize);
    let boxes = component_boxes(&labels, count, width as usize, height as usize);

    let mapping = registration.mapping();
    let px_per_mm = registration.px_per_mm;
    // Judged in pixels first. A component's millimetre box costs a pass over
    // its own area, and the components that are obviously not letters are
    // exactly the enormous ones — a photograph, a border, a scan that came out
    // black. Measuring those precisely before throwing them away is the one
    // thing here that can turn a page into a minute.
    let smallest_px = options.min_height_mm * px_per_mm;
    let largest_px = options.max_height_mm * px_per_mm;

    let mut marks = Vec::new();
    let mut discarded = 0usize;
    for (label, patch) in boxes.iter().enumerate() {
        let Some(patch) = patch else { continue };
        let box_width = (patch.x1 - patch.x0 + 1) as f64;
        let box_height = (patch.y1 - patch.y0 + 1) as f64;
        // Generous either way: the skew still has to be taken out, and this
        // only exists to skip work, not to make the decision.
        if box_height > largest_px * 1.2 || box_width.max(box_height) < smallest_px * 0.8 {
            discarded += 1;
            continue;
        }
        match to_mark(
            patch,
            label as u32 + 1,
            &labels,
            width as usize,
            &mapping,
            px_per_mm,
        ) {
            Some(mark) if keeps(&mark, registration, options) => marks.push(mark),
            _ => discarded += 1,
        }
    }

    // Pictures are drawn only now, with each letter whole: merging first and
    // drawing second is what puts the dot of an `i` into the `i`.
    let marks = merge_accents(marks);
    let drawn: Vec<(Mark, Stamp)> = marks
        .into_iter()
        .map(|mark| {
            let stamp = Stamp::sample(&labels, width as usize, &mark.parts, &mapping, &mark.rect);
            (mark, stamp)
        })
        .collect();

    let lines = group_lines(drawn);
    Ok(PageText { lines, discarded })
}

/// Which pixels of the scan are paper Onionskin should look at.
///
/// The margin comes off here rather than later so that the scanner's own
/// shadow along the paper's edge never reaches the threshold: a band of dark
/// pixels the length of the sheet would otherwise pull the split towards it.
fn paper_mask(
    image: &GrayImage,
    registration: &ScanRegistration,
    options: &ReadOptions,
) -> Vec<bool> {
    let (width, height) = image.dimensions();
    let page = registration.page;
    let margin = options.edge_margin_mm;
    let mapping = registration.mapping();
    let mut mask = vec![false; (width as usize) * (height as usize)];

    for y in 0..height as usize {
        for x in 0..width as usize {
            let (mx, my) = mapping.pixel_to_page_mm((x as f64 + 0.5, y as f64 + 0.5));
            mask[y * width as usize + x] = mx >= margin
                && my >= margin
                && mx <= page.width_mm - margin
                && my <= page.height_mm - margin;
        }
    }
    mask
}

/// Find the letters, then read them against a font.
pub fn read_with_font(
    image: &GrayImage,
    registration: &ScanRegistration,
    options: &ReadOptions,
    font: &EmbeddedFont,
    alphabet: Option<&str>,
) -> Result<PageText, ReadError> {
    let mut page = read(image, registration, options)?;
    // Told nothing, look for everything the font can draw.
    let every = alphabet.is_none().then(|| alphabet_of(font));
    let alphabet = alphabet.or(every.as_deref()).unwrap_or("");
    recognise(&mut page, font, alphabet, options.min_confidence);
    Ok(page)
}

/// The Latin characters worth trying first.
///
/// Ordered so the commoner shapes come first, which only matters for ties: an
/// exact draw between `O` and `0` has to break somewhere, and breaking it
/// towards the letter is right far more often than not.
///
/// This is a head start, not the alphabet — see [`alphabet_of`].
pub const COMMON_LATIN: &str = "etaoinshrdlcumwfgypbvkjxqz\
                                ETAOINSHRDLCUMWFGYPBVKJXQZ\
                                0123456789\
                                .,:;'\"!?-–—()[]/&@#£$€%+=*_";

/// What to look for on a page set in this font: everything it can draw.
///
/// The alphabet is taken from the font rather than written down here, and that
/// is the whole of Onionskin's answer to "which languages does it read?". A
/// page is set in some font; that font contains exactly the characters the page
/// can possibly be showing. Hand it a Greek font and it looks for Greek, a
/// Devanagari font and it looks for Devanagari, without a list of languages
/// existing anywhere in the program to be left incomplete.
///
/// Two honest limits, both of them about scripts rather than about size:
///
/// * **Cursive scripts** — Arabic, N'Ko, and handwriting — join their letters
///   up, so a run of joined letters is one connected mark and comes back as
///   one unread letter rather than four read ones. Finding *where* the ink is
///   still works, which is what placing a delta needs.
/// * **Combining marks** are folded into the letter they sit on, so a page of
///   Devanagari or Thai reads as its base letters. Again the positions hold.
pub fn alphabet_of(font: &EmbeddedFont) -> String {
    let mut alphabet = String::new();
    let mut taken: std::collections::BTreeSet<char> = std::collections::BTreeSet::new();

    // The common Latin shapes first, so ties break towards them.
    for ch in COMMON_LATIN.chars() {
        if font.has(ch) && taken.insert(ch) {
            alphabet.push(ch);
        }
    }
    for ch in font.coverage() {
        // A control character or a space has no shape to match against, and a
        // combining mark is never found on its own — it was merged into the
        // letter it sits on before any of this.
        if ch.is_control() || ch.is_whitespace() || is_specialist_notation(ch) {
            continue;
        }
        if taken.insert(ch) {
            alphabet.push(ch);
        }
    }
    alphabet
}

/// Characters that exist to be written *about* text rather than in it.
///
/// Unicode holds several complete second copies of the Latin alphabet — small
/// capitals and phonetic letters for linguistics, subscripts and superscripts
/// for notation, four mathematical variants for equations, roman numerals,
/// fullwidth forms for setting Latin among CJK. Every one of them is drawn as
/// an ordinary letter, and none of them appears in ordinary printed text.
///
/// Left in the default alphabet they are not a rare nuisance but a constant
/// one: `Paid` comes back as `ᴘaid` because a small-capital P is a P, and no
/// amount of looking at the ink will ever say otherwise. So the default leaves
/// them out. A linguist setting a page in phonetic script passes the letters
/// they want and gets them — this is a default, not a limit.
fn is_specialist_notation(ch: char) -> bool {
    let code = ch as u32;
    matches!(code,
        // Private use: a codepoint here means whatever the font's author
        // decided, and nothing at all outside that font. Reading a letter as
        // one is not a wrong answer so much as no answer written down as if it
        // were one.
        0xE000..=0xF8FF
        | 0xF0000..=0xFFFFD
        | 0x100000..=0x10FFFD
        | 0x02B0..=0x02FF   // spacing modifier letters
        | 0x1D00..=0x1DBF // phonetic extensions: small capitals and the rest
        | 0x2070..=0x209F // superscripts and subscripts
        | 0x2100..=0x214F // letterlike symbols
        | 0x2150..=0x218F // number forms: roman numerals
        | 0xA700..=0xA71F // modifier tone letters
        | 0xFF00..=0xFFEF // fullwidth and halfwidth forms
        | 0x1D400..=0x1D7FF // mathematical alphanumeric symbols
        | 0x1F100..=0x1F1FF // enclosed alphanumeric supplement
    )
}

// ---------------------------------------------------------------------------
// Finding the marks
// ---------------------------------------------------------------------------

/// One mark of ink, before it is grouped with any other.
///
/// It carries the components it is made of rather than a picture of them,
/// because a letter is not finished until its accent has been merged in: the
/// dot of an `i` is its own component, and a picture drawn before the merge
/// shows a bare stem sitting low in a box that has room for a dot — which
/// matches a dotless `ı` better than an `i`, and reads "Paid" as "Paıd".
#[derive(Debug, Clone)]
struct Mark {
    rect: Rect,
    ink_mm2: f64,
    /// The connected components this mark is made of: the letter, and any
    /// accent that turned out to belong to it.
    parts: Vec<u32>,
}

/// A pixel bounding box and ink count for one connected component.
#[derive(Debug, Clone, Copy)]
struct Patch {
    x0: usize,
    y0: usize,
    x1: usize,
    y1: usize,
    pixels: u32,
}

/// Connected-component labelling, eight-connected, two passes over the image.
///
/// Two passes rather than a flood fill because a flood fill's stack is the
/// size of the component, and a scan of a solid black page is one component of
/// eight million pixels. This one's memory is fixed by the image, not by what
/// happens to be on it.
fn label_components(ink: &[bool], width: usize, height: usize) -> (Vec<u32>, u32) {
    let mut labels = vec![0u32; ink.len()];
    // Union-find over provisional labels. Index 0 is "no label".
    let mut parent: Vec<u32> = vec![0];

    for y in 0..height {
        for x in 0..width {
            let here = y * width + x;
            if !ink[here] {
                continue;
            }
            // West, north-west, north, north-east: every already-labelled
            // neighbour under eight-connectivity.
            let mut neighbours = [0u32; 4];
            let mut seen = 0usize;
            if x > 0 {
                neighbours[seen] = labels[here - 1];
                seen += 1;
            }
            if y > 0 {
                let up = here - width;
                if x > 0 {
                    neighbours[seen] = labels[up - 1];
                    seen += 1;
                }
                neighbours[seen] = labels[up];
                seen += 1;
                if x + 1 < width {
                    neighbours[seen] = labels[up + 1];
                    seen += 1;
                }
            }

            let smallest = neighbours[..seen]
                .iter()
                .copied()
                .filter(|&label| label != 0)
                .min()
                .unwrap_or(0);

            if smallest == 0 {
                let fresh = parent.len() as u32;
                parent.push(fresh);
                labels[here] = fresh;
            } else {
                labels[here] = smallest;
                // Every other neighbour is the same blob reached another way.
                for &other in &neighbours[..seen] {
                    if other != 0 && other != smallest {
                        union(&mut parent, smallest, other);
                    }
                }
            }
        }
    }

    // Second pass: resolve every provisional label to its root, and renumber
    // the roots into a dense range so the caller can index by them.
    let mut dense = vec![0u32; parent.len()];
    let mut next = 0u32;
    for label in labels.iter_mut() {
        if *label == 0 {
            continue;
        }
        let root = find(&mut parent, *label);
        if dense[root as usize] == 0 {
            next += 1;
            dense[root as usize] = next;
        }
        *label = dense[root as usize];
    }
    (labels, next)
}

fn find(parent: &mut [u32], mut label: u32) -> u32 {
    while parent[label as usize] != label {
        // Path halving: shortens the tree without a second walk.
        let grandparent = parent[parent[label as usize] as usize];
        parent[label as usize] = grandparent;
        label = grandparent;
    }
    label
}

fn union(parent: &mut [u32], a: u32, b: u32) {
    let (ra, rb) = (find(parent, a), find(parent, b));
    if ra != rb {
        // Always point the larger label at the smaller, so roots stay stable.
        let (small, large) = if ra < rb { (ra, rb) } else { (rb, ra) };
        parent[large as usize] = small;
    }
}

fn component_boxes(labels: &[u32], count: u32, width: usize, height: usize) -> Vec<Option<Patch>> {
    let mut boxes: Vec<Option<Patch>> = vec![None; count as usize];
    for y in 0..height {
        for x in 0..width {
            let label = labels[y * width + x];
            if label == 0 {
                continue;
            }
            let slot = &mut boxes[label as usize - 1];
            match slot {
                None => {
                    *slot = Some(Patch {
                        x0: x,
                        y0: y,
                        x1: x,
                        y1: y,
                        pixels: 1,
                    })
                }
                Some(patch) => {
                    patch.x0 = patch.x0.min(x);
                    patch.x1 = patch.x1.max(x);
                    patch.y1 = y;
                    patch.pixels += 1;
                }
            }
        }
    }
    boxes
}

/// Turn one component into a mark placed on the paper.
///
/// The scan is turned by a degree or two and the paper is not, so a pixel box
/// is not a page box. Each ink pixel is placed individually and the box taken
/// around those: mapping the four corners of the pixel box instead would
/// inflate every letter by the skew, which at 2° is a tenth of a millimetre on
/// a capital and enough to make two adjacent letters look like they touch.
fn to_mark(
    patch: &Patch,
    label: u32,
    labels: &[u32],
    width: usize,
    mapping: &Mapping,
    px_per_mm: f64,
) -> Option<Mark> {
    let (mut x0, mut y0) = (f64::INFINITY, f64::INFINITY);
    let (mut x1, mut y1) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    let mut pixels = 0u32;

    for y in patch.y0..=patch.y1 {
        for x in patch.x0..=patch.x1 {
            if labels[y * width + x] != label {
                continue;
            }
            let (mx, my) = mapping.pixel_to_page_mm((x as f64 + 0.5, y as f64 + 0.5));
            x0 = x0.min(mx);
            y0 = y0.min(my);
            x1 = x1.max(mx);
            y1 = y1.max(my);
            pixels += 1;
        }
    }
    if pixels == 0 {
        return None;
    }

    // Half a pixel each way: the loop measured pixel centres, and the mark
    // covers the pixels themselves.
    let half = 0.5 / px_per_mm;
    let rect = Rect {
        x_mm: x0 - half,
        y_mm: y0 - half,
        width_mm: (x1 - x0) + 2.0 * half,
        height_mm: (y1 - y0) + 2.0 * half,
    };
    let mm2_per_pixel = 1.0 / (px_per_mm * px_per_mm);

    Some(Mark {
        rect,
        ink_mm2: pixels as f64 * mm2_per_pixel,
        parts: vec![label],
    })
}

/// Is this mark worth keeping, or is it dust, a rule, or the scanner's shadow?
fn keeps(mark: &Mark, registration: &ScanRegistration, options: &ReadOptions) -> bool {
    let rect = &mark.rect;
    if rect.height_mm < options.min_height_mm && rect.width_mm < options.min_height_mm {
        return false;
    }
    if rect.height_mm > options.max_height_mm {
        return false;
    }

    let page = registration.page;
    let margin = options.edge_margin_mm;
    if rect.x_mm < margin
        || rect.y_mm < margin
        || rect.right_mm() > page.width_mm - margin
        || rect.bottom_mm() > page.height_mm - margin
    {
        return false;
    }

    // A rule: long, thin, and solid. Underscores are exactly this and so are
    // table borders, and neither is a letter however much a `_` looks like one.
    let long = rect.width_mm > 25.0 || rect.height_mm > 25.0;
    let thin = rect.height_mm < 0.8 || rect.width_mm < 0.8;
    if long && thin {
        return false;
    }
    true
}

// ---------------------------------------------------------------------------
// Grouping: marks to letters, letters to words, words to lines
// ---------------------------------------------------------------------------

/// Put the dot back on the i.
///
/// A tittle, an accent, the two dots of an umlaut and the bar of a Polish `ł`
/// are all separate ink from the letter they belong to, so a connected-
/// component pass finds each of them as its own mark. Left alone they read as
/// a page with twice as many letters as it has, and every one of them in the
/// wrong place.
fn merge_accents(mut marks: Vec<Mark>) -> Vec<Mark> {
    // Tallest first, so a base letter is always the one absorbing.
    marks.sort_by(|a, b| {
        b.rect
            .height_mm
            .partial_cmp(&a.rect.height_mm)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    // Which letter each accent belongs to, decided by looking at every letter
    // that could claim it and taking the nearest.
    //
    // Taking the first that fits is not good enough, and fails in a way that
    // looks like nothing at all: in "Paid" the `d` is tall, so it is considered
    // before the short `i`, and it is close enough to the `i`'s tittle to claim
    // it. The word then reads as three letters with a `d` that starts to the
    // left of the `i` — and nothing anywhere says so.
    let mut claimed_by: Vec<Option<usize>> = vec![None; marks.len()];
    for accent in 0..marks.len() {
        let mut best: Option<(f64, usize)> = None;
        for base in 0..marks.len() {
            if base == accent || !belongs_to(&marks[base].rect, &marks[accent].rect) {
                continue;
            }
            // Nearest by centre. An accent sits over its own letter, so when
            // two letters could take it, the closer one is right.
            let apart = (marks[base].rect.centre().0 - marks[accent].rect.centre().0).abs();
            if best.map(|(d, _)| apart < d).unwrap_or(true) {
                best = Some((apart, base));
            }
        }
        claimed_by[accent] = best.map(|(_, base)| base);
    }
    // An accent may not itself be claimed as one — otherwise the two dots of a
    // diaeresis could chain, one onto the other and away from the letter.
    for accent in 0..marks.len() {
        if let Some(base) = claimed_by[accent] {
            if claimed_by[base].is_some() {
                claimed_by[accent] = None;
            }
        }
    }

    let mut merged: Vec<Mark> = Vec::with_capacity(marks.len());
    for base in 0..marks.len() {
        if claimed_by[base].is_some() {
            continue;
        }
        let mut rect = marks[base].rect;
        let mut ink = marks[base].ink_mm2;
        let mut parts = marks[base].parts.clone();
        for (accent, owner) in claimed_by.iter().enumerate() {
            if *owner == Some(base) {
                rect = rect.union(&marks[accent].rect);
                ink += marks[accent].ink_mm2;
                parts.extend_from_slice(&marks[accent].parts);
            }
        }
        merged.push(Mark {
            rect,
            ink_mm2: ink,
            parts,
        });
    }
    merged
}

/// Is the smaller mark an accent on the larger one?
fn belongs_to(base: &Rect, mark: &Rect) -> bool {
    // Only something distinctly smaller can be an accent. Without this, two
    // letters of the same size on consecutive lines would swallow each other.
    if mark.height_mm > base.height_mm * 0.55 {
        return false;
    }

    // It has to sit over the letter — but "over" means over the letter's
    // *width on the page*, not over its ink. The two dots of an `ï` straddle a
    // stem less than half a millimetre wide and touch none of it, so a test
    // against the stem alone leaves them behind as two letters of their own.
    let reach = base.height_mm * 0.3;
    let span = Rect {
        x_mm: base.x_mm - reach,
        width_mm: base.width_mm + reach * 2.0,
        ..*base
    };
    if span.horizontal_overlap(mark) < 0.5 {
        return false;
    }

    // Clear above the letter, or clear below it for a cedilla. Requiring it to
    // be clear is what keeps a full stop from being swallowed by the letter it
    // follows: punctuation sits on the baseline, which is where the letter
    // ends, and an accent never does.
    let above = mark.bottom_mm() <= base.y_mm + base.height_mm * 0.25;
    let below = mark.y_mm >= base.bottom_mm() - base.height_mm * 0.05;

    if above {
        base.y_mm - mark.bottom_mm() < base.height_mm * 0.5
    } else if below {
        mark.y_mm - base.bottom_mm() < base.height_mm * 0.3
    } else {
        false
    }
}

/// Sort marks into rows sharing a baseline, then rows into words.
fn group_lines(mut marks: Vec<(Mark, Stamp)>) -> Vec<TextLine> {
    if marks.is_empty() {
        return Vec::new();
    }
    // Top-down, then left-to-right: reading order for everything that follows.
    marks.sort_by(|a, b| {
        a.0.rect
            .y_mm
            .partial_cmp(&b.0.rect.y_mm)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut rows: Vec<Vec<(Mark, Stamp)>> = Vec::new();
    for mark in marks {
        // A mark joins the last row it shares vertical space with. Rows are
        // built top-down, so that is the lowest one that still reaches it.
        let home = rows.iter().rposition(|row: &Vec<(Mark, Stamp)>| {
            row.iter()
                .any(|other| other.0.rect.vertical_overlap(&mark.0.rect) > 0.35)
        });
        match home {
            Some(index) => rows[index].push(mark),
            None => rows.push(vec![mark]),
        }
    }

    let mut lines: Vec<TextLine> = rows.into_iter().map(build_line).collect();
    lines.sort_by(|a, b| {
        a.baseline_mm
            .partial_cmp(&b.baseline_mm)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    lines
}

fn build_line(mut row: Vec<(Mark, Stamp)>) -> TextLine {
    row.sort_by(|a, b| {
        a.0.rect
            .x_mm
            .partial_cmp(&b.0.rect.x_mm)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let rect = row
        .iter()
        .map(|m| m.0.rect)
        .reduce(|a, b| a.union(&b))
        .expect("a row is never empty");

    // The baseline is where most letters end. Descenders on `g` and `y` reach
    // below it and would drag a mean down, so take the commonest bottom edge
    // rather than the average of them.
    let baseline_mm = commonest_bottom(&row);

    let gap_limit = word_gap_limit(&row);
    let mut words: Vec<Word> = Vec::new();
    let mut current: Vec<Letter> = Vec::new();
    let mut previous_right = f64::NEG_INFINITY;

    for (mark, stamp) in row {
        let gap = mark.rect.x_mm - previous_right;
        if !current.is_empty() && gap > gap_limit {
            words.push(finish_word(std::mem::take(&mut current)));
        }
        previous_right = previous_right.max(mark.rect.right_mm());
        current.push(Letter {
            rect: mark.rect,
            ink_mm2: mark.ink_mm2,
            text: None,
            confidence: 0.0,
            stamp,
        });
    }
    if !current.is_empty() {
        words.push(finish_word(current));
    }

    TextLine {
        rect,
        baseline_mm,
        words,
    }
}

fn finish_word(letters: Vec<Letter>) -> Word {
    let rect = letters
        .iter()
        .map(|l| l.rect)
        .reduce(|a, b| a.union(&b))
        .expect("a word is never empty");
    Word { rect, letters }
}

/// Where the line's letters sit, from the commonest bottom edge.
fn commonest_bottom(row: &[(Mark, Stamp)]) -> f64 {
    let mut bottoms: Vec<f64> = row.iter().map(|m| m.0.rect.bottom_mm()).collect();
    bottoms.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    // The densest cluster within a quarter of a millimetre — tight enough to
    // separate a baseline from a descender, loose enough to survive a scan.
    let window = 0.25;
    let (mut best_start, mut best_len) = (0usize, 0usize);
    for start in 0..bottoms.len() {
        let mut end = start;
        while end + 1 < bottoms.len() && bottoms[end + 1] - bottoms[start] <= window {
            end += 1;
        }
        if end - start + 1 > best_len {
            best_len = end - start + 1;
            best_start = start;
        }
    }
    let cluster = &bottoms[best_start..best_start + best_len];
    cluster.iter().sum::<f64>() / cluster.len() as f64
}

/// How wide a gap has to be before it is a space rather than letter spacing.
///
/// Derived from the line itself rather than fixed, because the same page
/// carries an 8 pt footnote and a 24 pt heading and a millimetre means
/// something different in each. The quarter-point of the gaps is a reliable
/// read on the letter spacing even when half the line is spaces.
fn word_gap_limit(row: &[(Mark, Stamp)]) -> f64 {
    let mut gaps: Vec<f64> = Vec::new();
    let mut right = f64::NEG_INFINITY;
    for (mark, _) in row {
        if right.is_finite() {
            gaps.push((mark.rect.x_mm - right).max(0.0));
        }
        right = right.max(mark.rect.right_mm());
    }

    // A floor from the type size, for a line with too few gaps to have a
    // distribution at all.
    //
    // Measured against the line's x-height rather than its tallest mark. The
    // tallest mark is whatever happens to be on the line — a capital on one, a
    // `J` reaching below the baseline on the next — and the two differ by a
    // third, which is more than the whole margin between a wide letter gap and
    // a narrow space. The lower quartile of the heights lands on the plain
    // small letters, which is a steady measure of the type size.
    let mut heights: Vec<f64> = row.iter().map(|m| m.0.rect.height_mm).collect();
    heights.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let x_height = heights[heights.len() / 4];
    // A space is about 0.32 of the em and an x-height about 0.53, so a space
    // is around 0.6 of an x-height. Letter spacing runs to about 0.35 of one.
    let floor = (x_height * 0.45).max(0.3);

    if gaps.len() < 4 {
        return floor;
    }
    gaps.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    // Twice the typical gap. On a line with spaces the median still sits among
    // the letter gaps — half a line is never spaces — and on a line without
    // any, nothing comes close to twice it.
    let median = gaps[gaps.len() / 2];
    (median * 2.2).max(floor)
}

// ---------------------------------------------------------------------------
// Recognition: matching ink against the glyphs of a known font
// ---------------------------------------------------------------------------

/// A small square picture of one mark, straightened and normalised.
///
/// Everything is compared at this size, so a letter scanned at 600 dpi and a
/// glyph drawn from an outline meet on the same ground.
#[derive(Debug, Clone, PartialEq)]
struct Stamp {
    cells: Vec<f32>,
}

/// The side of a stamp, in cells. Thirty-two holds the difference between an
/// `e` and a `c` — a bar a tenth of the letter tall — with pixels to spare,
/// and 1024 comparisons per candidate is nothing.
const STAMP: usize = 32;

impl Stamp {
    fn blank() -> Stamp {
        Stamp {
            cells: vec![0.0; STAMP * STAMP],
        }
    }

    fn get(&self, x: usize, y: usize) -> f32 {
        self.cells[y * STAMP + x]
    }

    /// Draw a component into a stamp, straightening the scan's skew.
    ///
    /// Sampled from the page's frame back into the scan rather than the other
    /// way round, so every cell gets a value and the letter arrives upright
    /// whatever angle the sheet was laid at.
    fn sample(
        labels: &[u32],
        width: usize,
        wanted: &[u32],
        mapping: &Mapping,
        rect: &Rect,
    ) -> Stamp {
        let mut stamp = Stamp::blank();
        if rect.width_mm <= 0.0 || rect.height_mm <= 0.0 {
            return stamp;
        }
        let height = labels.len() / width.max(1);

        // Several samples per cell: a letter is often only a dozen pixels
        // across, and one sample per cell turns a stem into a dotted line.
        const SUB: usize = 3;
        let weight = 1.0 / (SUB * SUB) as f32;

        for cy in 0..STAMP {
            for cx in 0..STAMP {
                let mut hits = 0.0f32;
                for sy in 0..SUB {
                    for sx in 0..SUB {
                        let u = (cx as f64 + (sx as f64 + 0.5) / SUB as f64) / STAMP as f64;
                        let v = (cy as f64 + (sy as f64 + 0.5) / SUB as f64) / STAMP as f64;
                        let mm = (
                            rect.x_mm + u * rect.width_mm,
                            rect.y_mm + v * rect.height_mm,
                        );
                        let (px, py) = mapping.page_mm_to_pixel(mm);
                        if px < 0.0 || py < 0.0 {
                            continue;
                        }
                        let (px, py) = (px as usize, py as usize);
                        if px >= width || py >= height {
                            continue;
                        }
                        // Only this letter's own ink — its body and its
                        // accent. A neighbour clipping the corner of the box
                        // is not part of it.
                        if wanted.contains(&labels[py * width + px]) {
                            hits += weight;
                        }
                    }
                }
                stamp.cells[cy * STAMP + cx] = hits;
            }
        }
        stamp
    }

    /// Draw a glyph outline into a stamp, filled the way a printer would.
    fn from_outline(contours: &[Vec<(f64, f64)>]) -> Option<Stamp> {
        let (mut x0, mut y0) = (f64::INFINITY, f64::INFINITY);
        let (mut x1, mut y1) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
        for contour in contours {
            for &(x, y) in contour {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
        if !x0.is_finite() || x1 <= x0 || y1 <= y0 {
            return None;
        }

        let mut stamp = Stamp::blank();
        // Scanlines through the middle of each sub-row, filled by the even-odd
        // rule. Fonts are drawn non-zero, but a glyph's counters — the hole in
        // an `o` — are wound the opposite way in both conventions, so for the
        // one question being asked here the two agree.
        const SUB: usize = 3;
        let weight = 1.0 / (SUB * SUB) as f32;

        for cy in 0..STAMP {
            for sy in 0..SUB {
                let v = (cy as f64 + (sy as f64 + 0.5) / SUB as f64) / STAMP as f64;
                // Outlines run y-upwards; a stamp runs y-downwards.
                let y = y1 - v * (y1 - y0);

                let mut crossings: Vec<f64> = Vec::new();
                for contour in contours {
                    for index in 0..contour.len() {
                        let (ax, ay) = contour[index];
                        let (bx, by) = contour[(index + 1) % contour.len()];
                        if (ay > y) == (by > y) {
                            continue;
                        }
                        let t = (y - ay) / (by - ay);
                        crossings.push(ax + t * (bx - ax));
                    }
                }
                if crossings.len() < 2 {
                    continue;
                }
                crossings.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

                for span in crossings.chunks_exact(2) {
                    let (left, right) = (span[0], span[1]);
                    for cx in 0..STAMP {
                        for sx in 0..SUB {
                            let u = (cx as f64 + (sx as f64 + 0.5) / SUB as f64) / STAMP as f64;
                            let x = x0 + u * (x1 - x0);
                            if x >= left && x < right {
                                stamp.cells[cy * STAMP + cx] += weight;
                            }
                        }
                    }
                }
            }
        }
        Some(stamp)
    }

    /// How alike two stamps are, 0 to 1.
    ///
    /// Ink-over-union rather than a plain pixel count: two mostly-white
    /// pictures agree about the white whatever they are, and scoring that
    /// makes every letter look like every other. Only the ink is evidence.
    fn similarity(&self, other: &Stamp) -> f64 {
        let mut shared = 0.0f64;
        let mut total = 0.0f64;
        for (a, b) in self.cells.iter().zip(other.cells.iter()) {
            shared += a.min(*b) as f64;
            total += a.max(*b) as f64;
        }
        if total <= 0.0 {
            0.0
        } else {
            shared / total
        }
    }

    /// The best similarity over a small search, for a letter placed a cell or
    /// two off by the threshold's choice of edge.
    fn best_similarity(&self, other: &Stamp) -> f64 {
        let mut best = self.similarity(other);
        for (dx, dy) in [(1i32, 0i32), (-1, 0), (0, 1), (0, -1)] {
            best = best.max(self.similarity(&other.shifted(dx, dy)));
        }
        best
    }

    fn shifted(&self, dx: i32, dy: i32) -> Stamp {
        let mut moved = Stamp::blank();
        for y in 0..STAMP {
            for x in 0..STAMP {
                let sx = x as i32 - dx;
                let sy = y as i32 - dy;
                if sx < 0 || sy < 0 || sx >= STAMP as i32 || sy >= STAMP as i32 {
                    continue;
                }
                moved.cells[y * STAMP + x] = self.get(sx as usize, sy as usize);
            }
        }
        moved
    }

    fn ink(&self) -> f64 {
        self.cells.iter().map(|&c| c as f64).sum()
    }

    /// The same picture at a quarter of the resolution, for a first look.
    ///
    /// A CJK font holds twenty thousand glyphs, and comparing a mark against
    /// every one of them at full size is a thousand times more arithmetic than
    /// a page can afford. A blurred version costs a sixteenth as much and is
    /// enough to throw out everything that is not close, leaving a handful to
    /// look at properly. Nothing is decided here — only narrowed.
    fn coarse(&self) -> Coarse {
        const SCALE: usize = STAMP / COARSE;
        let mut cells = [0.0f32; COARSE * COARSE];
        let weight = 1.0 / (SCALE * SCALE) as f32;
        for y in 0..STAMP {
            for x in 0..STAMP {
                cells[(y / SCALE) * COARSE + (x / SCALE)] += self.get(x, y) * weight;
            }
        }
        Coarse { cells }
    }
}

/// The side of a blurred stamp, in cells.
const COARSE: usize = 8;

/// A blurred stamp, for narrowing a large alphabet down to a few candidates.
#[derive(Debug, Clone, Copy)]
struct Coarse {
    cells: [f32; COARSE * COARSE],
}

impl Coarse {
    fn similarity(&self, other: &Coarse) -> f64 {
        let mut shared = 0.0f64;
        let mut total = 0.0f64;
        for (a, b) in self.cells.iter().zip(other.cells.iter()) {
            shared += a.min(*b) as f64;
            total += a.max(*b) as f64;
        }
        if total <= 0.0 {
            0.0
        } else {
            shared / total
        }
    }
}

/// A glyph prepared for comparison.
struct Candidate {
    ch: char,
    stamp: Stamp,
    coarse: Coarse,
    /// Width over height of the glyph's own ink, for the cheap first pass.
    aspect: f64,
    /// Where the glyph sits about the baseline, as a fraction of its height:
    /// 0 means it sits entirely on the line, 1 entirely below it.
    below_baseline: f64,
    /// Cap height fraction: how tall it is against the font's capitals.
    relative_height: f64,
}

/// How many candidates survive the blurred first pass and are looked at
/// properly. Generous: the point of the first pass is to make a large alphabet
/// affordable, not to make the decision.
const SHORTLIST: usize = 24;

/// How far a glyph's height may differ from the mark's, against the line's cap
/// height, before it is not worth comparing.
const HEIGHT_REACH: f64 = 0.45;

/// How far a glyph may sit from the mark against the baseline, as a fraction of
/// the mark's own height. This is what separates `p` from `P`.
const BASELINE_REACH: f64 = 0.35;

/// Read the letters against a font's glyphs.
///
/// The page is matched line by line, because size is a line-level property:
/// once one line's type size is known every letter on it is measured against
/// glyphs at that size, and `o` stops being confusable with `O`.
pub fn recognise(page: &mut PageText, font: &EmbeddedFont, alphabet: &str, min_confidence: f64) {
    let candidates = build_candidates(font, alphabet);
    if candidates.is_empty() {
        return;
    }

    // Read it once with no opinion about language, see what language it turned
    // out to be, then read it again knowing.
    //
    // Latin `o`, Greek `ο` and Lao `໐` are three characters drawn as one
    // circle. No amount of looking at the ink will separate them, because the
    // ink is the same — the only evidence is the rest of the page. Deciding it
    // by a rule ("prefer Latin") would read every Greek page as Latin; the
    // page itself is the better authority, and most letters in any script have
    // no lookalike at all, so the first pass settles the question comfortably.
    match_page(page, &candidates, font, min_confidence, None);
    if let Some(script) = dominant_script(page) {
        match_page(page, &candidates, font, min_confidence, Some(script));
    }

    for line in page.lines.iter_mut() {
        order_words(line);
    }
}

/// The script most of the page turned out to be written in.
///
/// Weighted by how well each letter matched, and counting only letters that
/// belong to a script: digits and punctuation are at home anywhere and would
/// otherwise outvote the alphabet on a page of figures.
fn dominant_script(page: &PageText) -> Option<Script> {
    let mut tally: std::collections::BTreeMap<Script, f64> = std::collections::BTreeMap::new();
    for letter in page.letters() {
        if let Some(ch) = letter.text {
            let script = script_of(ch);
            if script != Script::Common {
                *tally.entry(script).or_default() += letter.confidence;
            }
        }
    }
    tally
        .into_iter()
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(script, _)| script)
}

/// How much a letter in the page's own script is favoured over a lookalike
/// from another. Small: it settles ties and near-ties and nothing else, so a
/// genuinely better match from another script still wins.
const HOME_SCRIPT_BONUS: f64 = 0.04;

fn match_page(
    page: &mut PageText,
    candidates: &[Candidate],
    font: &EmbeddedFont,
    min_confidence: f64,
    home: Option<Script>,
) {
    let cap_height = font.cap_height().max(1.0) / 1000.0;
    let mut shortlist: Vec<(f64, usize)> = Vec::with_capacity(candidates.len());

    for line in page.lines.iter_mut() {
        // The type size, read off the line: the commonest letter height that
        // reaches the baseline is a cap height, and the font says what
        // fraction of the em that is.
        let em_mm = line_em_mm(line, cap_height);
        if em_mm <= 0.0 {
            continue;
        }
        let baseline = line.baseline_mm;

        for word in line.words.iter_mut() {
            for letter in word.letters.iter_mut() {
                if letter.stamp.ink() <= 0.0 {
                    continue;
                }
                // Where this letter sits relative to the line, which is what
                // separates `p` from `P` and `,` from `'`.
                let below =
                    ((letter.rect.bottom_mm() - baseline) / letter.rect.height_mm).clamp(-1.0, 2.0);
                let relative = letter.rect.height_mm / (em_mm * cap_height).max(1e-6);
                let aspect = letter.rect.width_mm / letter.rect.height_mm.max(1e-6);
                let coarse = letter.stamp.coarse();

                // First pass: measurements, then a blurred picture. Both are
                // cheap, and between them they take an alphabet of thousands
                // down to a couple of dozen worth comparing properly.
                shortlist.clear();
                for (index, candidate) in candidates.iter().enumerate() {
                    // A letter whose box is twice as wide as the glyph's
                    // cannot become it however the ink falls.
                    if (candidate.aspect / aspect).max(aspect / candidate.aspect) > 2.2 {
                        continue;
                    }
                    if (candidate.relative_height - relative).abs() > HEIGHT_REACH {
                        continue;
                    }
                    if (candidate.below_baseline - below).abs() > BASELINE_REACH {
                        continue;
                    }
                    shortlist.push((coarse.similarity(&candidate.coarse), index));
                }
                if shortlist.len() > SHORTLIST {
                    // Partial sort: only the head is needed, and the tail is
                    // most of an alphabet.
                    shortlist.sort_unstable_by(|a, b| {
                        b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal)
                    });
                    shortlist.truncate(SHORTLIST);
                }

                let mut best: Option<(f64, f64, char)> = None;
                for &(_, index) in shortlist.iter() {
                    let candidate = &candidates[index];
                    // Shape is most of the evidence but not all of it. A
                    // subscript `ₚ` is the same drawing as a `p`, and only its
                    // size says otherwise; scoring size as well as shape is
                    // what keeps the two apart. Each measurement can cost a
                    // third, so shape still decides between real rivals.
                    let height_off = (candidate.relative_height - relative).abs() / HEIGHT_REACH;
                    let sits_off = (candidate.below_baseline - below).abs() / BASELINE_REACH;
                    let shape = letter.stamp.best_similarity(&candidate.stamp);
                    let mut score = shape
                        * (1.0 - 0.35 * height_off.min(1.0))
                        * (1.0 - 0.35 * sits_off.min(1.0));
                    // A letter in the language the page is written in, over an
                    // identical-looking one from a language it is not.
                    if let Some(home) = home {
                        let script = script_of(candidate.ch);
                        if script == home || script == Script::Common {
                            score *= 1.0 + HOME_SCRIPT_BONUS;
                        }
                    }
                    // Strictly better, so a tie falls to whichever came first
                    // in the alphabet — which is ordered by how common the
                    // letter is, and `O` beats `0` more often than not.
                    if best.map(|(s, _, _)| score > s).unwrap_or(true) {
                        best = Some((score, shape, candidate.ch));
                    }
                }

                if let Some((_, shape, ch)) = best {
                    // Reported as how well the ink matched the glyph. The
                    // preference above decides *which* glyph; it is not
                    // evidence, so it must not inflate the answer.
                    letter.confidence = shape;
                    letter.text = (shape >= min_confidence).then_some(ch);
                }
            }
        }
    }
}

/// Put the words of a line in the order they are read, not the order they sit.
///
/// Hebrew and Arabic run right to left, so a line of them found left-to-right
/// comes back with its words reversed — the same mistake as reading an English
/// line backwards, and just as invisible to anyone who does not read the
/// script. Decided from what the line actually says rather than from a setting:
/// the characters know which way they go.
fn order_words(line: &mut TextLine) {
    let (mut rtl, mut ltr) = (0usize, 0usize);
    for letter in line.words.iter().flat_map(|w| w.letters.iter()) {
        match letter.text.map(direction) {
            Some(Direction::RightToLeft) => rtl += 1,
            Some(Direction::LeftToRight) => ltr += 1,
            _ => {}
        }
    }
    if rtl > ltr {
        line.words.reverse();
        for word in line.words.iter_mut() {
            word.letters.reverse();
        }
    }
}

/// A coarse script identity, enough to tell homoglyphs apart by context.
///
/// Not a Unicode script database — those are large, and none of this needs to
/// be exact. All it has to do is put `o`, `ο` and `໐` in three different piles
/// so that whichever pile the rest of the page is in can claim them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Script {
    /// Digits, punctuation and symbols — at home on a page in any language,
    /// so never counted for or against one.
    Common,
    Latin,
    Greek,
    Cyrillic,
    Hebrew,
    Arabic,
    Devanagari,
    Thai,
    Lao,
    Han,
    Kana,
    Hangul,
    /// Everything else, kept apart by which part of Unicode it lives in.
    /// Coarse on purpose: a script split across two of these piles only makes
    /// the preference weaker, never wrong.
    Elsewhere(u32),
}

fn script_of(ch: char) -> Script {
    let code = ch as u32;
    // Digits, ASCII punctuation, and the general punctuation and symbol
    // blocks. A Greek page still numbers its pages with 1, 2, 3.
    if ch.is_ascii_digit()
        || (ch.is_ascii_punctuation())
        || (0x2000..=0x206F).contains(&code)
        || (0x20A0..=0x20CF).contains(&code)
        || (0x2190..=0x2BFF).contains(&code)
    {
        return Script::Common;
    }
    match code {
        0x0041..=0x024F | 0x1E00..=0x1EFF | 0xA720..=0xA7FF => Script::Latin,
        0x0370..=0x03FF | 0x1F00..=0x1FFF => Script::Greek,
        0x0400..=0x052F => Script::Cyrillic,
        0x0590..=0x05FF | 0xFB1D..=0xFB4F => Script::Hebrew,
        0x0600..=0x06FF | 0x0750..=0x077F | 0xFB50..=0xFDFF | 0xFE70..=0xFEFF => Script::Arabic,
        0x0900..=0x097F => Script::Devanagari,
        0x0E00..=0x0E7F => Script::Thai,
        0x0E80..=0x0EFF => Script::Lao,
        0x3400..=0x9FFF | 0xF900..=0xFAFF | 0x20000..=0x2FFFF => Script::Han,
        0x3040..=0x30FF | 0x31F0..=0x31FF => Script::Kana,
        0x1100..=0x11FF | 0xAC00..=0xD7AF => Script::Hangul,
        other => Script::Elsewhere(other >> 7),
    }
}

/// Which way a character's script runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Direction {
    LeftToRight,
    RightToLeft,
    /// Digits and punctuation, which take the direction of what surrounds them.
    Neutral,
}

/// The right-to-left blocks: Hebrew, Arabic, Syriac, Thaana, N'Ko, Samaritan,
/// Mandaic, and the Arabic presentation and supplement ranges.
fn direction(ch: char) -> Direction {
    let code = ch as u32;
    let right_to_left = (0x0590..=0x08FF).contains(&code)
        || (0xFB1D..=0xFDFF).contains(&code)
        || (0xFE70..=0xFEFF).contains(&code)
        || (0x10800..=0x10FFF).contains(&code)
        || (0x1E800..=0x1EFFF).contains(&code);
    if right_to_left {
        Direction::RightToLeft
    } else if ch.is_alphabetic() {
        Direction::LeftToRight
    } else {
        Direction::Neutral
    }
}

fn build_candidates(font: &EmbeddedFont, alphabet: &str) -> Vec<Candidate> {
    let cap_height = font.cap_height().max(1.0) / 1000.0;
    let units_per_em = font.units_per_em();
    let mut seen: std::collections::BTreeSet<char> = std::collections::BTreeSet::new();
    let mut candidates = Vec::new();

    for ch in alphabet.chars() {
        if ch.is_whitespace() || !seen.insert(ch) {
            continue;
        }
        let Some(contours) = font.outline(ch) else {
            continue;
        };
        let Some(stamp) = Stamp::from_outline(&contours) else {
            continue;
        };

        let (mut x0, mut y0) = (f64::INFINITY, f64::INFINITY);
        let (mut x1, mut y1) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
        for contour in &contours {
            for &(x, y) in contour {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
        let height = (y1 - y0) / units_per_em;
        if height <= 0.0 {
            continue;
        }
        candidates.push(Candidate {
            ch,
            coarse: stamp.coarse(),
            stamp,
            aspect: (x1 - x0) / (y1 - y0),
            // The outline's y runs up from the baseline, so a negative
            // bottom edge is a descender.
            below_baseline: (-y0 / units_per_em) / height,
            relative_height: height / cap_height,
        });
    }
    drop_identical_shapes(candidates)
}

/// Keep one character per shape, and let the commonest one have it.
///
/// Unicode is full of characters that are drawn the same: Latin `a` and
/// Cyrillic `а`, Latin `o` and Greek `ο`, and every letter twice more in the
/// mathematical alphabets. They are different characters and mean different
/// things, and no amount of looking at ink will ever tell them apart, because
/// the ink is identical. Left in, they do not merely lose the coin toss — they
/// win it about as often as not, and a page comes back as Latin text with a
/// Cyrillic `а` sprinkled through it.
///
/// So where two candidates draw the same shape at the same proportions, the
/// first is kept. The alphabet is built commonest-first, which makes that the
/// right one.
fn drop_identical_shapes(candidates: Vec<Candidate>) -> Vec<Candidate> {
    /// Shapes are only compared within a bucket of near-identical proportions,
    /// which turns a quadratic sweep over thousands of glyphs into a linear
    /// one. A tenth is fine enough to keep unrelated letters apart and coarse
    /// enough that two drawings of the same letter land together.
    fn bucket(candidate: &Candidate) -> (i32, i32, i32) {
        (
            (candidate.aspect * 10.0).round() as i32,
            (candidate.relative_height * 10.0).round() as i32,
            (candidate.below_baseline * 10.0).round() as i32,
        )
    }

    let mut buckets: std::collections::HashMap<(i32, i32, i32), Vec<usize>> =
        std::collections::HashMap::new();
    let mut kept: Vec<Candidate> = Vec::with_capacity(candidates.len());

    for candidate in candidates {
        let key = bucket(&candidate);
        // Rounding puts two all-but-identical glyphs in neighbouring buckets
        // as often as the same one, so look at the neighbours too.
        let duplicate = [-1i32, 0, 1].iter().any(|dx| {
            [-1i32, 0, 1].iter().any(|dy| {
                buckets
                    .get(&(key.0 + dx, key.1 + dy, key.2))
                    .map(|indexes| {
                        indexes.iter().any(|&index| {
                            // Coarse first: it settles most pairs for a
                            // sixteenth of the arithmetic.
                            candidate.coarse.similarity(&kept[index].coarse) > 0.985
                                && candidate.stamp.similarity(&kept[index].stamp) > 0.975
                        })
                    })
                    .unwrap_or(false)
            })
        });
        if duplicate {
            continue;
        }
        buckets.entry(key).or_default().push(kept.len());
        kept.push(candidate);
    }
    kept
}

/// The line's em size in millimetres, from the letters that reach its baseline.
fn line_em_mm(line: &TextLine, cap_height: f64) -> f64 {
    let mut heights: Vec<f64> = line
        .letters()
        .filter(|l| (l.rect.bottom_mm() - line.baseline_mm).abs() < 0.3)
        .map(|l| l.rect.height_mm)
        .collect();
    if heights.is_empty() {
        heights = line.letters().map(|l| l.rect.height_mm).collect();
    }
    if heights.is_empty() {
        return 0.0;
    }
    heights.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    // The tallest quarter: capitals and ascenders, which is what cap height
    // measures. The median would land on an x-height letter instead.
    let tall = heights[heights.len() * 3 / 4];
    tall / cap_height.max(1e-6)
}

#[cfg(test)]
mod tests;
