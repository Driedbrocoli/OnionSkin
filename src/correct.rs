//! Fixing a mistake on a page that is already printed.
//!
//! The wrong figure went out on two hundred invoices, or a name is misspelt on
//! a certificate somebody is waiting for. The whole document is fine apart from
//! four characters, and the answer everywhere else is to print it again.
//!
//! Onionskin already has both halves of something better. `cover` lays solid
//! toner over words that should not be read; `write` puts new words down. What
//! it has not had is the join — and the join is the hard part, because doing it
//! by hand means measuring the box, guessing the type size, naming the face,
//! and getting all three right on a sheet there is only one of.
//!
//! The page can answer all three. It knows where the words are, how big they
//! are set, and which face they are in.
//!
//! # What this is honest about
//!
//! Covering is not erasing. Solid toner hides the old text from the eye and
//! from a photocopier; it does not take it off the paper, and a strong light
//! behind the sheet may still show it through. For anything that must not be
//! recoverable, the answer is still a fresh page — and the command says so.
//!
//! A phrase that appears twice on the page is refused rather than guessed at.
//! Covering the wrong "Total" is precisely the failure this is meant to
//! prevent, and there is no undo at a printer.

use crate::letters::PageText;
use crate::typeface::Typeface;

/// One thing that is wrong, and what it should say instead.
#[derive(Debug, Clone, PartialEq)]
pub struct Mistake {
    /// The words as they are printed on the sheet.
    pub was: String,
    /// What should be there instead.
    pub now: String,
}

/// How much beyond the old words to cover.
///
/// Enough to take the ink with it, including the parts of letters that reach
/// past the box a reader measures — and not so much that the box swallows the
/// line above. Under a millimetre either way is invisible on paper.
pub const PAD_MM: f64 = 0.6;

/// One correction, worked out against the page.
#[derive(Debug, Clone)]
pub struct Correction {
    pub was: String,
    pub now: String,
    /// The box to lay toner over: x, y, width, height, in millimetres.
    pub cover_mm: (f64, f64, f64, f64),
    /// Where the new words start, which is where the old ones started.
    pub x_mm: f64,
    /// The baseline the old words sat on, so the new ones sit level with
    /// whatever is beside them.
    pub baseline_mm: f64,
    pub size_pt: f64,
    pub font: String,
    /// Whether the size was measured off the page or worked out from the
    /// height of the line, which is a guess and worth saying so.
    pub size_measured: bool,
    /// The whole line the mistake was found on, so somebody can see that the
    /// right one was found before any paper moves.
    pub line: String,
}

impl Correction {
    pub fn describe(&self) -> String {
        format!(
            "\"{}\" → \"{}\"\n    on the line: {}\n    covering {:.0},{:.0} \
             {:.0}×{:.0} mm, writing at {:.0},{:.0} in {} at {:.0} pt{}",
            self.was.trim(),
            self.now.trim(),
            self.line,
            self.cover_mm.0,
            self.cover_mm.1,
            self.cover_mm.2,
            self.cover_mm.3,
            self.x_mm,
            self.baseline_mm,
            self.font,
            self.size_pt,
            if self.size_measured {
                ""
            } else {
                " (guessed from the height of the line — check it, or give --size)"
            }
        )
    }

    /// Whether the new words are wider than the space the old ones had.
    ///
    /// Not a refusal: an invoice that grows from "120.00" to "1,120.00" is a
    /// perfectly ordinary correction, and it will run into whatever is to its
    /// right. Worth saying before it is printed rather than after.
    pub fn wider_than_what_it_replaces(&self) -> Option<f64> {
        let was = self.cover_mm.2 - PAD_MM * 2.0;
        // A rough average of the built-in faces, the same one `blanks` uses to
        // say "about forty characters".
        let per_character_mm = self.size_pt * 25.4 / 72.0 * 0.5;
        let now = self.now.chars().count() as f64 * per_character_mm;
        (now > was).then_some(now - was)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CorrectError {
    #[error(
        "nothing on the page reads as '{wanted}'.\n    Run `onionskin read` on \
         it to see what is actually there — a scan of a printed page is read \
         letter by letter, and a smudged one may not say quite what you expect."
    )]
    NotFound { wanted: String },
    #[error(
        "'{wanted}' is on the page {count} times, so there is no telling which \
         one you mean.\n    Covering the wrong one cannot be undone. Give more \
         of the line to pick it out — '{wanted} 120.00' rather than '{wanted}'."
    )]
    FoundTwice { wanted: String, count: usize },
    #[error("'{0}' does not say what to put there instead")]
    NothingToPut(String),
}

/// Work out what to cover and what to write, from the page itself.
///
/// `face` is what the reader made of the page's typeface, where it could tell.
/// `size_pt` and `font`, when given, are somebody overruling that — which is
/// the escape hatch for a page the reader reads wrongly.
pub fn plan(
    text: &PageText,
    face: Option<&Typeface>,
    mistakes: &[Mistake],
    size_pt: Option<f64>,
    font: Option<&str>,
) -> Result<Vec<Correction>, CorrectError> {
    let mut planned = Vec::new();
    for mistake in mistakes {
        if mistake.now.trim().is_empty() {
            return Err(CorrectError::NothingToPut(mistake.was.clone()));
        }
        let found = crate::anchor::boxes_for(text, &mistake.was);
        match found.len() {
            0 => {
                return Err(CorrectError::NotFound {
                    wanted: mistake.was.clone(),
                })
            }
            1 => {}
            count => {
                return Err(CorrectError::FoundTwice {
                    wanted: mistake.was.clone(),
                    count,
                })
            }
        }
        let box_mm = found[0];

        // The line the mistake sits on, which carries the baseline the new
        // words have to share and the height they are measured against.
        let line = text
            .lines
            .iter()
            .min_by(|a, b| {
                let mine = (a.rect.y_mm - box_mm.y_mm).abs();
                let theirs = (b.rect.y_mm - box_mm.y_mm).abs();
                mine.partial_cmp(&theirs)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .ok_or(CorrectError::NotFound {
                wanted: mistake.was.clone(),
            })?;

        // The size comes off the line the mistake is on, not off the page.
        //
        // The reader fits one size through the whole page, which is the right
        // answer to a different question: a heading at fourteen point and a
        // body at twelve average to something that is neither, and a
        // correction set at the average is visibly wrong beside the words it
        // replaces. The tallest letter on *this* line is a direct measurement
        // of this line — cap height, which is about seven tenths of the type
        // size in every face there is.
        let tallest = line
            .words
            .iter()
            .flat_map(|word| word.letters.iter())
            .map(|letter| letter.rect.height_mm)
            .fold(0.0f64, f64::max);
        let (size, measured) = match (size_pt, tallest) {
            (Some(given), _) => (given, true),
            (None, tallest) if tallest > 0.0 => (tallest / 0.7 * 72.0 / 25.4, true),
            // Nothing on the line was read, so there is nothing to measure.
            // Whatever the page said of itself is better than nothing.
            (None, _) => (face.map(|face| face.size_pt).unwrap_or(11.0), false),
        };

        planned.push(Correction {
            was: mistake.was.clone(),
            now: mistake.now.clone(),
            cover_mm: (
                box_mm.x_mm - PAD_MM,
                box_mm.y_mm - PAD_MM,
                box_mm.width_mm + PAD_MM * 2.0,
                box_mm.height_mm + PAD_MM * 2.0,
            ),
            x_mm: box_mm.x_mm,
            baseline_mm: line.baseline_mm,
            size_pt: size,
            font: font
                .map(|name| name.to_string())
                .or_else(|| face.map(|face| face.font.base_name().to_string()))
                .unwrap_or_else(|| "Helvetica".to_string()),
            size_measured: measured,
            line: line.text_lossy(),
        });
    }
    Ok(planned)
}

/// Split `WAS:NOW` into the two, on the first colon.
///
/// The same rule `--after` uses, so a colon inside the replacement is left
/// alone — which matters, because "Total:120.00" is exactly the sort of thing
/// somebody is correcting.
pub fn parse_mistake(given: &str) -> Result<Mistake, String> {
    let (was, now) = given.split_once(':').ok_or_else(|| {
        format!(
            "bad correction '{given}'. Expected 'WAS:NOW' — the words as they \
             are printed, a colon, then what should be there instead."
        )
    })?;
    if was.trim().is_empty() {
        return Err(format!("'{given}' does not say what is wrong"));
    }
    if now.trim().is_empty() {
        return Err(format!("'{given}' does not say what to put there instead"));
    }
    Ok(Mistake {
        was: was.to_string(),
        now: now.to_string(),
    })
}

#[cfg(test)]
#[path = "correct/tests.rs"]
mod tests;
