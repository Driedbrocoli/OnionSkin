//! Working out what a page is already set in, from the letters read off it.
//!
//! Everything else in Onionskin has to be told a font. Reading the letters off
//! a scan needs one to compare the ink against, and adding a line of words
//! needs one to set them in. Asking the person holding the paper is asking the
//! wrong person entirely: the answer is printed on the sheet in their hand and
//! not written down anywhere in their head. Nobody looks at a rent statement
//! and thinks "Helvetica, eleven point". They look at it and think "there is a
//! gap after Received, and I want a date in it".
//!
//! Getting it wrong is not a small matter either. A sentence added in the wrong
//! face at the wrong size is the one thing on the sheet that announces itself
//! as an addition, and the sheet may well be somebody's only copy of it.
//!
//! So the font is read off the ink instead, from the one measurement a scan
//! gives away for nothing: how wide each word is. That is a surprisingly strong
//! clue, because [`crate::pdf::builtin_width_mm`] does not estimate the width
//! of a word in one of the eight built-in faces — it knows it, from the table
//! Adobe published and every PDF reader on every platform uses. A page's worth
//! of words is a page's worth of exact predictions to compare against.
//!
//! # How it works
//!
//! 1. Take every word where **every** letter was read ([`crate::letters::Word::text`]
//!    is `Some`) and there are at least two of them. The width to match is
//!    `word.rect.width_mm`, the box around the ink.
//! 2. For each of the eight faces, the width of word *w* at size *s* is
//!    `builtin_width_mm(font, w, 1.0) * s`. But that is the *advance* width —
//!    where the next word would start — and what a scan measures is the *ink*,
//!    which is narrower by the side bearing at each end of the word. So two
//!    numbers are fitted by ordinary least squares rather than one:
//!    `observed ≈ a * k + b`, with `k = builtin_width_mm(font, w, 1.0)`. Then
//!    `a` is the type size and `b` — a small negative number, roughly constant
//!    for a face — is the bearing the ink box loses.
//!
//!    Fitting only `a` is the obvious thing to do and it is wrong in a way that
//!    is easy to miss. The bearing loss does not disappear; it is absorbed into
//!    the size, which comes out several per cent small, and worse, every face
//!    is then wrong by the *same shape* of error. What ought to be a comparison
//!    between the eight width tables turns into a comparison of how much
//!    constant offset each one happens to leave, and the answer stops depending
//!    on the page at all. The second parameter takes that offset off the table
//!    so that only the shape of the widths is left to decide.
//! 3. Each face is scored by how far its line misses, relative to the size of
//!    the words: `sqrt(mean(residual²)) / mean(observed)`. Lowest wins.
//! 4. The confidence is how much better the winner is than the runner-up.
//! 5. Width says nothing about weight — see below — so bold is judged
//!    separately, from how much of each letter's box is filled with ink.
//! 6. And when there is not enough to go on, or the answer is absurd, it says
//!    nothing at all. A wrong font quietly asserted is worse than no answer,
//!    because no answer prompts a question and a wrong one does not.
//!
//! # What it cannot do, and will not pretend to
//!
//! * **It only knows eight faces.** A page set in Garamond comes back as
//!   whichever of the eight fits it best, not as "not one of these". The
//!   confidence is a comparison between the candidates and never a statement
//!   that the page really is set in one of them.
//! * **An oblique face has the same widths as its upright.** In Adobe's
//!   metrics Helvetica-Oblique is Helvetica width for width, and Courier-Bold
//!   is Courier width for width. No amount of measuring will separate those
//!   pairs, so ties are broken towards the upright and the regular, and the
//!   weight test below is the only thing that can promote Courier to
//!   Courier-Bold. Times-Italic is genuinely narrower than Times-Roman and is
//!   found on its own.
//! * **One page, one size.** A heading among body text is thrown out as an
//!   outlier and the body wins, which is the right answer for placing an
//!   addition. A page set half in one size and half in another has no single
//!   answer and this will give the larger half's, or neither.
//!
//! On measured pages the fit is better than it has any business being. With
//! word widths carrying ±0.1 mm of scanning error — about a pixel at 300 dpi —
//! twenty words recover the family every time and the size to within a
//! twentieth of a point. At ±0.3 mm, which is a poor scan of a poorly printed
//! sheet, the family is still right around nine times in ten and the confidence
//! falls to about a third, which is what confidence is for.

use std::cmp::Ordering;

use crate::letters::{PageText, Word};
use crate::pdf::{self, Font};

/// What a page turned out to be set in.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Typeface {
    /// The face that fitted best, of the eight built into every PDF reader.
    pub font: Font,
    /// The type size, in points.
    pub size_pt: f64,
    /// How much better the winner was than the best face that could have
    /// disagreed with it, 0 to 1.
    ///
    /// Zero means two faces fitted the page equally well and the tie was broken
    /// by nothing better than the order of the list. It does *not* mean the page
    /// is not set in the font named — only that these words could not tell.
    pub confidence: f64,
    /// How many words the answer was actually fitted through, after any that
    /// disagreed with all the others were set aside.
    pub words_measured: usize,
}

impl Typeface {
    /// One short sentence, for putting in front of somebody.
    ///
    /// "About" is not modesty. The size is fitted from ink boxes measured on a
    /// scan, so it lands within a few hundredths of a point of the truth and
    /// almost never exactly on it; printing `11.4728 pt` would suggest a
    /// precision the paper cannot support.
    pub fn describe(&self) -> String {
        format!(
            "{} at about {:.1} pt, from {} word{}",
            self.font.base_name(),
            self.size_pt,
            self.words_measured,
            if self.words_measured == 1 { "" } else { "s" }
        )
    }
}

/// The fewest words worth fitting a straight line through.
///
/// Two points and two unknowns is not a fit, it is a pair of simultaneous
/// equations: the line passes exactly through both whichever face is tried, so
/// all eight score zero and the winner is whichever came first in the list.
/// Three is the least that can disagree with itself.
const FEWEST_WORDS: usize = 3;

/// The smallest and largest type sizes worth believing.
///
/// Below about 4 pt a laser printer cannot hold the strokes of a letter apart,
/// and 200 pt is three inches tall, which is a poster rather than a page of
/// words. A fit landing outside this has not discovered an unusual size; it has
/// found nothing, and the widths it was handed were not the widths of one page
/// set in one size. A negative size — ink that gets *narrower* as the words get
/// longer — falls outside it too, which is the point.
const SMALLEST_PT: f64 = 4.0;
const LARGEST_PT: f64 = 200.0;

/// Work out what a scanned page is set in.
///
/// `None` when the page has not given enough away: fewer than three readable
/// words of two letters or more, or a size that comes out absurd.
pub fn detect(page: &PageText) -> Option<Typeface> {
    // A typewriter face is worth asking about first, because it can be
    // recognised from the geometry alone and the width fit cannot reliably
    // tell it apart from anything else. See [`monospaced`].
    if let Some(advance_mm) = monospaced(page) {
        let size_pt = advance_mm / COURIER_ADVANCE_EM * PT_PER_MM;
        if (SMALLEST_PT..=LARGEST_PT).contains(&size_pt) {
            let font = match page_coverage(page) {
                Some(ink) if ink > BOLD_ABOVE => pdf::Font::CourierBold,
                _ => pdf::Font::Courier,
            };
            return Some(Typeface {
                font,
                size_pt,
                // Evenly spaced letters are not something a proportional face
                // does by accident, so this is about as sure as this module
                // gets — but it is one measurement rather than two.
                confidence: 0.9,
                words_measured: measurable(page).len(),
            });
        }
    }
    detect_measured(&measurable(page), page_coverage(page))
}

/// Every letter in Courier is exactly this wide, in ems. Not approximately:
/// that is what makes it a typewriter face.
const COURIER_ADVANCE_EM: f64 = 0.6;

/// Points per millimetre, for turning a measured advance into a type size.
const PT_PER_MM: f64 = 72.0 / 25.4;

/// Is this page set in a typewriter face, and if so how wide is a letter?
///
/// Worth a separate measurement because the width fit cannot answer it well.
/// Reading letters off a scan means matching their shapes against some font,
/// and the only monospaced faces on most machines are sans ones — DejaVu Sans
/// Mono and its relatives — which look nothing like Courier's slab serifs. A
/// Courier page read against them comes back mostly unread, and a fit with
/// nothing to fit gives no answer at all.
///
/// The geometry gives it away without any font being involved. In a
/// proportional face an `i` and an `m` take very different amounts of room; in
/// a typewriter face every letter is on the same pitch, so the gaps between
/// the left edges of consecutive letters are all the same number. Measuring
/// how much those gaps vary separates the two cleanly, and does it on letters
/// that were never successfully read.
pub fn monospaced(page: &PageText) -> Option<f64> {
    monospaced_from(&letter_gaps(page))
}

/// The gap between the left edge of each letter and the next, within words.
///
/// Only within a word: the gap across a space is a space, not an advance.
fn letter_gaps(page: &PageText) -> Vec<f64> {
    let mut gaps = Vec::new();
    for line in &page.lines {
        for word in &line.words {
            for pair in word.letters.windows(2) {
                let gap = pair[1].rect.x_mm - pair[0].rect.x_mm;
                // A negative or absurd gap is a mis-segmented letter, not a
                // measurement of anything.
                if gap > 0.05 && gap < 40.0 {
                    gaps.push(gap);
                }
            }
        }
    }
    gaps
}

/// The same, from measured gaps rather than from a page.
///
/// Public for the same reason as [`detect_measured`]: this is where the
/// arithmetic is, and a page of letters is an awkward thing to build by hand,
/// which would otherwise leave the maths tested only through the thing that
/// produces it.
pub fn monospaced_from(gaps: &[f64]) -> Option<f64> {
    // Too few gaps and any number is a coincidence. Twelve is two short words.
    if gaps.len() < 12 {
        return None;
    }
    let mut gaps = gaps.to_vec();
    gaps.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
    let middle = gaps[gaps.len() / 2];
    if middle <= 0.0 {
        return None;
    }
    // Judged on the middle four-fifths. A scan always produces a few wild
    // gaps — two letters that touched and were read as one, a speck taken for
    // a full stop — and a handful of those should not outvote a page that is
    // otherwise perfectly even.
    let low = gaps.len() / 10;
    let high = gaps.len() - gaps.len() / 10;
    let trimmed = &gaps[low..high.max(low + 1)];
    let spread = trimmed
        .iter()
        .map(|gap| (gap - middle).abs() / middle)
        .sum::<f64>()
        / trimmed.len() as f64;

    // A tenth is generous for a typewriter face and far tighter than any
    // proportional one manages: Helvetica's own letters vary by about half
    // their own width from each other.
    (spread < 0.10).then_some(middle)
}

/// The same, from measurements rather than from a page.
///
/// This is where the arithmetic actually lives, and it is public for two
/// reasons. Widths do not have to come from a scan — anything that can measure
/// a word can ask this what it is looking at. And a page of text is an awkward
/// thing to build by hand, which would leave the maths tested only through the
/// thing that produces it.
///
/// `words` is the text of each word and the width of the box around its ink, in
/// millimetres. `coverage` is how much of a letter's box is ink on this page, if
/// it is known — see [`page_coverage`]; pass `None` and the weight is left to
/// the widths alone, which for the Courier pair means it stays regular.
pub fn detect_measured(words: &[(String, f64)], coverage: Option<f64>) -> Option<Typeface> {
    if words.len() < FEWEST_WORDS {
        return None;
    }

    let mut fits: Vec<(Font, Fit)> = Font::all()
        .iter()
        .filter_map(|&font| fit(font, words).map(|fitted| (font, fitted)))
        .collect();
    // Best first. The sort is stable, so faces that fit exactly as well as each
    // other keep the order of `Font::all()` — which puts the upright before the
    // oblique and the regular before the bold. That is the tie-break for the
    // two pairs that share a width table, and it breaks the right way: setting
    // an addition upright and regular when the page might be sloped or bold is
    // the quieter of the two mistakes.
    fits.sort_by(|a, b| a.1.score.partial_cmp(&b.1.score).unwrap_or(Ordering::Equal));
    let (winner, best) = fits.first()?;

    // The runner-up has to be a face that could have disagreed. Comparing the
    // winner against its own width table under another name — Helvetica against
    // Helvetica-Oblique — always gives a dead heat, and reporting that as no
    // confidence would mean a perfectly clear page of Helvetica came back with
    // a confidence of zero.
    let rival = fits
        .iter()
        .skip(1)
        .find(|(_, other)| tells_apart(&best.predicted, &other.predicted));
    let confidence = match rival {
        Some((_, other)) if other.score > 0.0 => (1.0 - best.score / other.score).clamp(0.0, 1.0),
        // Nothing on the list could have contradicted the winner, so there is
        // no evidence here at all, however well the line fitted.
        _ => 0.0,
    };

    // Weight is decided by the ink, and then the size is fitted again for the
    // face that decision landed on. Skipping the second fit would report the
    // size of one font beside the name of another: Helvetica-Bold is wider than
    // Helvetica for the same words, so its size read off Helvetica's table
    // comes out a few per cent large.
    let font = weigh(*winner, coverage);
    let fitted = if font == *winner {
        best.clone()
    } else {
        fit(font, words)?
    };

    if !(SMALLEST_PT..=LARGEST_PT).contains(&fitted.size_pt) {
        return None;
    }
    Some(Typeface {
        font,
        size_pt: fitted.size_pt,
        confidence,
        words_measured: fitted.words,
    })
}

// ---------------------------------------------------------------------------
// Which words are evidence
// ---------------------------------------------------------------------------

/// The words on a page whose width means something, and what they measure.
fn measurable(page: &PageText) -> Vec<(String, f64)> {
    page.lines
        .iter()
        .flat_map(|line| line.words.iter())
        .filter_map(measure)
        .collect()
}

/// One word, if it is worth measuring.
///
/// Three rules, and each of them is there because of what happens without it:
///
/// * **Every letter read.** A word with an unread letter in it still has a
///   width, but the text to predict that width from is missing a character, so
///   the prediction is short by a letter and the fit is pulled towards whatever
///   face is narrowest.
/// * **At least two letters.** A single mark on a form is as likely to be a
///   tick, a bullet or a speck read as an `l` as it is to be a word, and its
///   whole width is one glyph's bearings — the very thing the fit is trying to
///   hold constant.
/// * **Letters and digits only, and only ones the built-in fonts can write.**
///   A character outside WinAnsi has no advance width in these tables and would
///   silently count as nothing, taking a real piece of ink out of the
///   prediction while leaving it in the measurement. Punctuation is dropped for
///   a subtler reason: a full stop or a comma carries far more side bearing
///   than a letter does, so a word ending in one breaks the assumption that the
///   bearing loss is the same for every word.
fn measure(word: &Word) -> Option<(String, f64)> {
    let text = word.text()?;
    let width_mm = word.rect.width_mm;
    (worth_measuring(&text) && width_mm > 0.0).then_some((text, width_mm))
}

/// Is a word of this text worth measuring? See [`measure`] for why.
fn worth_measuring(text: &str) -> bool {
    text.chars().count() >= 2
        && text.chars().all(char::is_alphanumeric)
        && pdf::encode_winansi(text).is_ok()
}

// ---------------------------------------------------------------------------
// Fitting one face to the page
// ---------------------------------------------------------------------------

/// How well one face explains the widths, and what it says the page is.
#[derive(Debug, Clone)]
struct Fit {
    /// The fitted `a`: the type size, in points.
    ///
    /// The fitted `b` — what the ink box loses to the side bearings — is not
    /// kept. It is a nuisance parameter: it exists so that it can be subtracted
    /// off and stop corrupting the size, and having been subtracted off it has
    /// nothing more to say.
    size_pt: f64,
    /// How far the line misses, as a fraction of the average word width. Lower
    /// is better, and zero is a page that matches this face exactly.
    score: f64,
    /// How many words were left after the outliers went.
    words: usize,
    /// This face's predicted width for each word at 1 pt, kept so that two
    /// faces can be asked whether they could have disagreed at all.
    predicted: Vec<f64>,
}

/// Fit one face to the measured words.
fn fit(font: Font, words: &[(String, f64)]) -> Option<Fit> {
    let predicted: Vec<f64> = words
        .iter()
        .map(|(text, _)| pdf::builtin_width_mm(font, text, 1.0))
        .collect();
    let observed: Vec<f64> = words.iter().map(|(_, width)| *width).collect();

    let mut kept: Vec<usize> = (0..words.len()).collect();
    let (mut size_pt, mut bearing_mm) = solve(&predicted, &observed, &kept)?;

    // Then the same fit again without whatever disagreed with it wildly.
    //
    // This is for headings. A page is body text with a title on it, and the
    // title's words are two or three times the width their letters predict at
    // the body's size. Least squares has no defence against that — it is the
    // squares that do the damage, so the handful of words that miss by most
    // decide the answer — and a page of 11 pt with two words of 30 pt on it
    // fits at 24 pt, which is neither of them and is a size nothing on the
    // sheet is set in.
    //
    // The measure to clip against is the *middling* error rather than the
    // average one, and that distinction is the whole trick. Two gross outliers
    // in twenty inflate the average error enough to shelter themselves — they
    // are no longer three times an average they are themselves setting — while
    // the middling error still describes the ordinary words and leaves them
    // standing far outside it.
    for _ in 0..3 {
        let middling = middling_error(&predicted, &observed, &kept, size_pt, bearing_mm);
        if middling <= 0.0 {
            break;
        }
        let trimmed: Vec<usize> = kept
            .iter()
            .copied()
            .filter(|&index| {
                let miss = observed[index] - (size_pt * predicted[index] + bearing_mm);
                miss.abs() <= middling * OUTLIER
            })
            .collect();
        if trimmed.len() < FEWEST_WORDS || trimmed.len() == kept.len() {
            break;
        }
        let Some(again) = solve(&predicted, &observed, &trimmed) else {
            break;
        };
        kept = trimmed;
        (size_pt, bearing_mm) = again;
    }

    let mean = kept.iter().map(|&index| observed[index]).sum::<f64>() / kept.len() as f64;
    if !mean.is_finite() || mean <= 0.0 {
        return None;
    }
    let error = spread(&predicted, &observed, &kept, size_pt, bearing_mm);
    Some(Fit {
        size_pt,
        score: error / mean,
        words: kept.len(),
        predicted,
    })
}

/// How far out a word has to be, in middling errors, to be set aside.
///
/// Four, which on the pages this was tried against drops a 30 pt heading and
/// keeps every word of the 11 pt body. Six keeps the heading and gives an
/// answer of 24 pt that nothing on the sheet is set in; two and a half starts
/// eating perfectly good words. The margin between those is wide, which is
/// another way of saying a heading does not look remotely like body text once
/// the comparison is made against the right yardstick.
const OUTLIER: f64 = 4.0;

/// Ordinary least squares: the line `observed ≈ a * predicted + b`.
///
/// `None` when the words cannot pin the line down — which happens for real. On
/// a page set in Courier every character is the same width, so a face's
/// prediction depends on nothing but how many letters a word has; give it a
/// column of a table, where every word is the same length, and every prediction
/// is identical. Two unknowns and one distinct value of `k` is a line through a
/// single point, and any size at all fits it if the bearing is chosen to suit.
fn solve(predicted: &[f64], observed: &[f64], kept: &[usize]) -> Option<(f64, f64)> {
    if kept.len() < FEWEST_WORDS {
        return None;
    }
    let count = kept.len() as f64;
    let mut sum_k = 0.0;
    let mut sum_y = 0.0;
    let mut sum_kk = 0.0;
    let mut sum_ky = 0.0;
    for &index in kept {
        let (k, y) = (predicted[index], observed[index]);
        sum_k += k;
        sum_y += y;
        sum_kk += k * k;
        sum_ky += k * y;
    }

    let denominator = count * sum_kk - sum_k * sum_k;
    // Judged against the size of the numbers rather than against zero: these
    // are millimetres, and "small" only means anything next to what is being
    // subtracted.
    if denominator.abs() <= 1e-9 * (count * sum_kk).abs() {
        return None;
    }
    let a = (count * sum_ky - sum_k * sum_y) / denominator;
    let b = (sum_y - a * sum_k) / count;
    (a.is_finite() && b.is_finite()).then_some((a, b))
}

/// The root-mean-square of how far the line misses each word.
fn spread(
    predicted: &[f64],
    observed: &[f64],
    kept: &[usize],
    size_pt: f64,
    bearing_mm: f64,
) -> f64 {
    if kept.is_empty() {
        return 0.0;
    }
    let total: f64 = kept
        .iter()
        .map(|&index| {
            let miss = observed[index] - (size_pt * predicted[index] + bearing_mm);
            miss * miss
        })
        .sum();
    (total / kept.len() as f64).sqrt()
}

/// How far the line misses the middling word, which is what an outlier is
/// judged against.
fn middling_error(
    predicted: &[f64],
    observed: &[f64],
    kept: &[usize],
    size_pt: f64,
    bearing_mm: f64,
) -> f64 {
    if kept.is_empty() {
        return 0.0;
    }
    let mut misses: Vec<f64> = kept
        .iter()
        .map(|&index| (observed[index] - (size_pt * predicted[index] + bearing_mm)).abs())
        .collect();
    misses.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
    misses[misses.len() / 2]
}

/// Could these two faces have disagreed about this page?
///
/// Not "are they the same font" but "would they have predicted different
/// widths for these particular words", which is the question confidence turns
/// on. Two faces can share a width table outright, and two others can differ
/// only in characters this page does not happen to use.
fn tells_apart(one: &[f64], other: &[f64]) -> bool {
    one.len() != other.len()
        || one
            .iter()
            .zip(other.iter())
            .any(|(a, b)| (a - b).abs() > 1e-9 * a.abs().max(1.0))
}

// ---------------------------------------------------------------------------
// Weight: is the page bold?
// ---------------------------------------------------------------------------

/// The letters a page's weight is judged by.
///
/// All of them round or plain, and all of them x-height with nothing sticking
/// out. That is the point: what is being measured is how much of a letter's
/// box is filled with ink, and that depends far more on which letter it is than
/// on how heavy the face is. An `l` is a stem in a narrow box and fills half of
/// it in any weight; a `w` is mostly the white between its strokes. Comparing
/// like with like across a page means comparing the same handful of shapes.
const PLAIN_LETTERS: &str = "oenascum";

/// The fewest of those letters worth taking a median of.
///
/// A median of two is one of the two. A page of prose has hundreds of these —
/// they are the commonest letters in English, which is why they were chosen —
/// so needing eight costs a full page nothing and refuses to answer for a form
/// with three words on it.
const FEWEST_LETTERS: usize = 8;

/// How much of a letter's box has to be ink before the page is called bold, and
/// how little before it is called regular.
///
/// Measured, not guessed. Rendering `o e n a s c u m` from the metric-compatible
/// clones of the eight faces and thresholding them the way a scan is
/// thresholded gives median coverages of:
///
/// | face | regular | bold |
/// |---|---|---|
/// | Helvetica | 0.46 | 0.62 |
/// | Times | 0.40 | 0.55 |
/// | Courier | 0.35 | 0.59 |
///
/// The regulars top out at 0.46 and the bolds start at 0.55, so the two bands
/// are set just inside that gap and the space between them is left undecided
/// rather than split down the middle. Undecided means the widths keep their
/// answer, and that caution is deliberate, because **the threshold the scan was
/// binarised at moves every one of these numbers**. A dark scan fattens every
/// stroke and a light one thins it, by an amount that is easily a tenth of the
/// coverage — comparable with the whole gap between a regular face and a bold
/// one. So a page whose coverage lands in the middle is not a page of some
/// intermediate weight; it is a page that has not said.
const BOLD_ABOVE: f64 = 0.53;
const REGULAR_BELOW: f64 = 0.48;

/// How much of a letter's box is ink on this page, for the plain letters.
///
/// The middling one rather than the average, because a page has the odd letter
/// sitting in a box that is not its own: a mark merged with the one beside it,
/// or a comma swept up into a `c`. Those inflate an average and cannot move a
/// median.
pub fn page_coverage(page: &PageText) -> Option<f64> {
    median_coverage(page.letters().filter_map(|letter| {
        let text = letter.text?;
        let box_mm2 = letter.rect.width_mm * letter.rect.height_mm;
        (box_mm2 > 0.0).then(|| (text, letter.ink_mm2 / box_mm2))
    }))
}

/// The median coverage of the plain letters, given each letter and how much of
/// its box is ink.
fn median_coverage(letters: impl Iterator<Item = (char, f64)>) -> Option<f64> {
    let mut coverages: Vec<f64> = letters
        .filter(|(text, coverage)| {
            PLAIN_LETTERS.contains(*text) && coverage.is_finite() && *coverage > 0.0
        })
        .map(|(_, coverage)| coverage)
        .collect();
    if coverages.len() < FEWEST_LETTERS {
        return None;
    }
    coverages.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
    Some(coverages[coverages.len() / 2])
}

/// Which weight of the winning family the page is actually printed in.
///
/// Width is a poor witness to weight and for one of the three families it is no
/// witness at all: Courier and Courier-Bold have identical advance widths, so
/// the fit can never prefer one, and every page of typewriter text would come
/// back regular. Even where the tables do differ, a bold face is only a few per
/// cent wider than its regular, and the fit has a free size to spend — so it
/// absorbs most of the difference by calling the page slightly larger instead.
///
/// Ink coverage sees it immediately, because a bold face is not a wider letter
/// but a thicker stroke inside much the same box.
///
/// Nothing is changed when the ink has not spoken clearly, and the two sloped
/// faces are left alone in any case: there is no bold oblique or bold italic
/// among the eight to promote them to.
fn weigh(font: Font, coverage: Option<f64>) -> Font {
    let Some(coverage) = coverage else {
        return font;
    };
    let (regular, bold) = match font {
        Font::Helvetica | Font::HelveticaBold => (Font::Helvetica, Font::HelveticaBold),
        Font::TimesRoman | Font::TimesBold => (Font::TimesRoman, Font::TimesBold),
        Font::Courier | Font::CourierBold => (Font::Courier, Font::CourierBold),
        Font::HelveticaOblique | Font::TimesItalic => return font,
    };
    if coverage >= BOLD_ABOVE {
        bold
    } else if coverage <= REGULAR_BELOW {
        regular
    } else {
        font
    }
}

#[cfg(test)]
mod tests;
