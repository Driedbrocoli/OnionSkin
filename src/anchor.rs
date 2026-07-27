//! Placing words by what is already on the page, rather than by measuring it.
//!
//! Everything else in Onionskin wants a position in millimetres from the
//! top-left corner of the paper. That is the honest unit — it is what the
//! printer works in and what the delta is written in — and it is a miserable
//! thing to have to supply. Somebody holding a form does not know that the gap
//! after `Received:` starts 62.4 mm across and 96.1 mm down. They know it is
//! the gap after `Received:`.
//!
//! So the page is read, the words on it are found, and the new text is put
//! where a person would have put it:
//!
//! ```text
//! onionskin add form.png --after 'Received:Approved 27 July'
//! onionskin add form.png --below 'Signature:J. Bezzina'
//! ```
//!
//! # Why this is allowed to be approximate
//!
//! The anchor is found by reading letters off an image, which is never perfect.
//! A wrong answer here is not the same kind of wrong as a wrong answer in the
//! delta itself: the words land a few millimetres from where they were wanted,
//! which somebody notices on the proof and corrects. So the matching is
//! deliberately forgiving — case is ignored, runs of spaces count as one, and
//! an anchor that appears as part of a longer line still counts.
//!
//! What it will not do is guess. An anchor that appears nowhere is an error
//! naming what *was* on the page, and an anchor that appears several times is
//! an error saying so and asking which one — because putting the words next to
//! the first of five `Date:` fields is a coin toss, and a coin toss that ruins
//! a sheet of paper is worse than a question.

use crate::letters::{PageText, Rect};

/// Where the new words go, relative to the anchor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Where {
    /// On the same line, just after the anchor's last letter.
    After,
    /// One line down, starting where the anchor starts.
    Below,
    /// One line down, starting where the anchor *ends* — for filling the
    /// second line of a box whose label is on the first.
    BelowEnd,
}

impl Where {
    pub fn parse(text: &str) -> Option<Where> {
        Some(match text.trim().to_ascii_lowercase().as_str() {
            "after" => Where::After,
            "below" => Where::Below,
            "below-end" | "belowend" => Where::BelowEnd,
            _ => return None,
        })
    }
}

/// A place on the paper, in millimetres, and how it was arrived at.
#[derive(Debug, Clone, PartialEq)]
pub struct Placed {
    /// Millimetres from the left edge of the paper.
    pub x_mm: f64,
    /// The baseline, in millimetres down from the top edge — which is what
    /// the rest of Onionskin means by a text position.
    pub y_mm: f64,
    /// The whole line the anchor was found on, for saying what was matched.
    pub line: String,
    /// How tall the anchor's letters were, in millimetres. A caller with no
    /// size of its own can set text to match what is already there.
    pub letter_height_mm: f64,
}

#[derive(Debug, thiserror::Error)]
pub enum AnchorError {
    #[error("nothing on the page reads as '{wanted}'.{nearby}")]
    NotFound { wanted: String, nearby: String },
    #[error(
        "'{wanted}' appears {count} times on this page, so there is no way to know \
         which one you mean.\n{lines}\nUse more of the surrounding words to say \
         which — the whole line if need be."
    )]
    Ambiguous {
        wanted: String,
        count: usize,
        lines: String,
    },
    #[error("there is no text on this page to place anything against")]
    NothingRead,
}

/// One line of a page, as the matcher needs to see it.
///
/// A [`crate::letters::PageText`] is an awkward thing to build by hand — its
/// letters carry the straightened bitmap they were matched from — which would
/// leave this logic tested only through the thing that produces it. So the
/// matching works on this instead, and reading a page into it is one short
/// function anybody can check by eye.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    /// Where the letters sit, in millimetres down the page.
    pub baseline_mm: f64,
    /// Each word: what it reads as, where its ink sits, and how tall its
    /// tallest letter is.
    pub words: Vec<(String, Rect, f64)>,
}

impl Row {
    fn text(&self) -> String {
        self.words
            .iter()
            .map(|(text, _, _)| text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    }
}

/// Read a page into rows the matcher can work on.
pub fn rows(page: &PageText) -> Vec<Row> {
    page.lines
        .iter()
        .map(|line| Row {
            baseline_mm: line.baseline_mm,
            words: line
                .words
                .iter()
                .map(|word| {
                    let tallest = word
                        .letters
                        .iter()
                        .map(|letter| letter.rect.height_mm)
                        .fold(0.0f64, f64::max);
                    (word.text_lossy(), word.rect, tallest)
                })
                .collect(),
        })
        .collect()
}

/// Find `wanted` on the page and work out where the new words should start.
///
/// `gap_mm` is how much room to leave after the anchor when placing on the same
/// line — a space, more or less. `line_step_mm` is how far down a line is.
/// Both are given rather than assumed because only the caller knows what size
/// the new text will be set at.
pub fn place(
    page: &PageText,
    wanted: &str,
    put: Where,
    gap_mm: f64,
    line_step_mm: f64,
) -> Result<Placed, AnchorError> {
    place_in(&rows(page), wanted, put, gap_mm, line_step_mm)
}

/// The same, from rows rather than from a page.
pub fn place_in(
    rows: &[Row],
    wanted: &str,
    put: Where,
    gap_mm: f64,
    line_step_mm: f64,
) -> Result<Placed, AnchorError> {
    if rows.is_empty() {
        return Err(AnchorError::NothingRead);
    }
    let wanted_key = squash(wanted);
    if wanted_key.is_empty() {
        return Err(AnchorError::NotFound {
            wanted: wanted.to_string(),
            nearby: String::new(),
        });
    }

    // Exactly first. Only if nothing on the page reads as the anchor is a
    // near miss considered, so a page that says both "Date" and "Rate" is
    // never resolved by charity when one of them is exactly right.
    let mut hits: Vec<Hit> = Vec::new();
    for row in rows {
        if let Some(hit) = find_in_row(row, &wanted_key, 0) {
            hits.push(hit);
        }
    }
    if hits.is_empty() {
        let allowed = slack(&wanted_key);
        if allowed > 0 {
            for row in rows {
                if let Some(hit) = find_in_row(row, &wanted_key, allowed) {
                    hits.push(hit);
                }
            }
        }
    }

    match hits.len() {
        0 => Err(AnchorError::NotFound {
            wanted: wanted.to_string(),
            nearby: suggest(rows, &wanted_key),
        }),
        1 => Ok(position(&hits[0], put, gap_mm, line_step_mm)),
        count => Err(AnchorError::Ambiguous {
            wanted: wanted.to_string(),
            count,
            lines: hits
                .iter()
                .map(|hit| format!("    {:.0} mm down: {}", hit.baseline_mm, hit.line))
                .collect::<Vec<_>>()
                .join("\n"),
        }),
    }
}

/// One place the anchor was found.
#[derive(Debug, Clone)]
struct Hit {
    /// Where the matched run of words starts and ends, in millimetres.
    start_mm: f64,
    end_mm: f64,
    baseline_mm: f64,
    letter_height_mm: f64,
    line: String,
}

fn position(hit: &Hit, put: Where, gap_mm: f64, line_step_mm: f64) -> Placed {
    let (x_mm, y_mm) = match put {
        Where::After => (hit.end_mm + gap_mm, hit.baseline_mm),
        Where::Below => (hit.start_mm, hit.baseline_mm + line_step_mm),
        Where::BelowEnd => (hit.end_mm, hit.baseline_mm + line_step_mm),
    };
    Placed {
        x_mm,
        y_mm,
        line: hit.line.clone(),
        letter_height_mm: hit.letter_height_mm,
    }
}

/// Look for the anchor among one line's words.
///
/// Matched against runs of consecutive words rather than the line as a whole,
/// so that the *extent* of the match is known — which is the whole point,
/// since "after" means after the last letter of it and not after the line.
fn find_in_row(row: &Row, wanted_key: &str, slack: usize) -> Option<Hit> {
    if row.words.is_empty() {
        return None;
    }
    for first in 0..row.words.len() {
        let mut joined = String::new();
        for last in first..row.words.len() {
            joined.push_str(&squash(&row.words[last].0));
            if within(&joined, wanted_key, slack) {
                let start = row.words[first].1;
                let end = row.words[last].1;
                // The tallest letter in the run, which is a better guide to
                // the size of the text than the whole line — a line with a
                // parenthesis in it is taller than its letters are.
                let height = row.words[first..=last]
                    .iter()
                    .map(|(_, _, tallest)| *tallest)
                    .fold(0.0f64, f64::max);
                return Some(Hit {
                    start_mm: start.x_mm,
                    end_mm: end.right_mm(),
                    baseline_mm: row.baseline_mm,
                    letter_height_mm: height,
                    line: row.text(),
                });
            }
            // Longer than what is wanted: nothing further along can shorten
            // it. The slack is allowed for, since a run that is short by a
            // letter may still be the anchor read badly.
            if joined.chars().count() > wanted_key.chars().count() + slack {
                break;
            }
        }
    }
    None
}

/// How many wrong letters to forgive in an anchor of this length.
///
/// A scan is never read perfectly. `Received:` comes back as `Peoeived:` on a
/// poor one — the shapes of `R` and `P` differ by very little once a fax has
/// had them — and refusing over that sends somebody back to measuring with a
/// ruler, which is the thing this exists to avoid.
///
/// A quarter of the letters, and nothing at all under five. `Date` and `Rate`
/// are both four letters and both plausible labels on the same form; forgiving
/// one wrong letter there would be a coin toss dressed up as helpfulness. A
/// quarter rather than a fifth because a poor scan really does get two letters
/// of a nine-letter word wrong — `Received:` reading as `Peoeived:` is a
/// measured result and not a hypothetical — while two wrong letters is still
/// nowhere near enough to turn one form label into another.
fn slack(wanted_key: &str) -> usize {
    let letters = wanted_key.chars().count();
    if letters < 5 {
        0
    } else {
        letters / 4
    }
}

/// Are these the same string, allowing `slack` letters to be wrong?
///
/// A bounded edit distance — insertions, deletions and substitutions — that
/// gives up as soon as it is certain the answer exceeds the budget, so a long
/// line costs nothing to reject.
fn within(found: &str, wanted: &str, slack: usize) -> bool {
    if slack == 0 {
        return found == wanted;
    }
    let a: Vec<char> = found.chars().collect();
    let b: Vec<char> = wanted.chars().collect();
    if a.len().abs_diff(b.len()) > slack {
        return false;
    }

    // One row of the distance table at a time; only `slack + 1` values on
    // either side of the diagonal can ever be under the budget.
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    for (i, &left) in a.iter().enumerate() {
        let mut current = vec![usize::MAX; b.len() + 1];
        current[0] = i + 1;
        let from = (i + 1).saturating_sub(slack + 1);
        let to = (i + 1 + slack + 1).min(b.len());
        let mut best = usize::MAX;
        for j in from.max(1)..=to {
            let substitute = previous[j - 1].saturating_add(usize::from(left != b[j - 1]));
            let delete = previous[j].saturating_add(1);
            let insert = current[j - 1].saturating_add(1);
            current[j] = substitute.min(delete).min(insert);
            best = best.min(current[j]);
        }
        // Every reachable cell in this row is already over budget, so no
        // continuation of it can come back under.
        if best > slack {
            return false;
        }
        previous = current;
    }
    previous[b.len()] <= slack
}

/// Lines that look a bit like what was asked for, to put in the error.
///
/// Somebody who mistyped an anchor, or whose scan read `Recerved`, is far
/// better served by "here is what is actually on the page" than by "not
/// found". Three lines, because a whole page of them is not help either.
fn suggest(rows: &[Row], wanted_key: &str) -> String {
    let mut scored: Vec<(usize, String)> = rows
        .iter()
        .map(|row| {
            let text = row.text();
            (shared_run(&squash(&text), wanted_key), text)
        })
        .filter(|(score, _)| *score >= 3)
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0));
    scored.truncate(3);

    if scored.is_empty() {
        let some: Vec<String> = rows
            .iter()
            .take(3)
            .map(|row| format!("    {}", row.text()))
            .collect();
        if some.is_empty() {
            return String::new();
        }
        return format!("\nWhat is on the page:\n{}", some.join("\n"));
    }
    format!(
        "\nDid you mean one of these?\n{}",
        scored
            .iter()
            .map(|(_, text)| format!("    {text}"))
            .collect::<Vec<_>>()
            .join("\n")
    )
}

/// The longest run of characters the two have in common.
///
/// Not a proper edit distance, which would be more code for an answer nobody
/// measures: this only has to sort three suggestions into a helpful order.
fn shared_run(haystack: &str, needle: &str) -> usize {
    let hay: Vec<char> = haystack.chars().collect();
    let want: Vec<char> = needle.chars().collect();
    let mut best = 0;
    for start in 0..hay.len() {
        for offset in 0..want.len() {
            let mut run = 0;
            while start + run < hay.len()
                && offset + run < want.len()
                && hay[start + run] == want[offset + run]
            {
                run += 1;
            }
            best = best.max(run);
        }
    }
    best
}

/// A string with everything people vary about it taken out.
///
/// Case, spacing and punctuation all go. A scan that read `Received:` as
/// `Received;` should still match `Received:` — the colon is not what anybody
/// meant by the anchor, and refusing over one would be pedantry paid for in
/// sheets of paper.
fn squash(text: &str) -> String {
    text.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

#[cfg(test)]
mod tests;
