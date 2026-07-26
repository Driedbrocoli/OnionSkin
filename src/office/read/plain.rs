//! Plain text, and Markdown read as plain text.
//!
//! A `.txt` needed LibreOffice to become a PDF, which was always absurd — it is
//! text, and a page of text is what this program is for. Markdown is read the
//! same way, with the headings picked out: `# Title` set larger and bold is
//! nearer to what the author meant than a line starting with a hash, and
//! nothing else in Markdown changes where the ink lands enough to be worth
//! guessing at.

use super::{Align, Block, Margins, Para, Piece, Sheet, Style};
use crate::geometry::PageSize;

/// Read a text file into paragraphs.
///
/// `suffix` decides whether Markdown's headings are honoured; anything that is
/// not Markdown is taken exactly as it is written.
pub fn read(text: &str, suffix: &str) -> Sheet {
    let markdown = matches!(suffix, "md" | "markdown");
    let mut sheet = Sheet::new(PageSize::new(210.0, 297.0), Margins::default());

    // Windows line endings, and the old Mac ones a very old file may still
    // have. Splitting on '\n' alone leaves a carriage return at the end of
    // every line, which prints as a missing glyph.
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    // An empty file is no lines at all. Splitting it would give one empty
    // line, which is a blank page with a paragraph nobody typed on it.
    if text.is_empty() {
        return sheet;
    }

    let mut mentioned_tables = false;
    for line in text.split('\n') {
        let mut para = Para {
            style: Style::default(),
            space_after_mm: 0.0,
            ..Para::default()
        };

        let mut body = line;
        if markdown {
            let hashes = line.len() - line.trim_start_matches('#').len();
            if hashes > 0 && hashes <= 6 && line[hashes..].starts_with(' ') {
                para.style.size_pt = heading_size(hashes);
                para.style.bold = true;
                para.space_before_mm = 3.0;
                para.space_after_mm = 1.5;
                body = line[hashes + 1..].trim_start();
            } else if let Some(item) = bullet(line) {
                para.marker = Some("•".into());
                para.indent_left_mm = 8.0;
                para.first_line_mm = -5.0;
                body = item;
            } else if line.trim_start().starts_with('|') && !mentioned_tables {
                sheet.note(
                    "This file has a Markdown table in it. Onionskin sets the rows as \
                     they are written rather than drawing a grid.",
                );
                mentioned_tables = true;
            }
        }

        if !body.is_empty() {
            para.pieces.push(Piece::Text(body.to_string(), para.style));
        }
        para.align = Align::Left;
        sheet.blocks.push(Block::Para(para));
    }

    // A file that ends with a newline gives one empty paragraph at the end,
    // which is a blank line nobody typed. Every editor adds that newline.
    if text.ends_with('\n') {
        sheet.blocks.pop();
    }
    sheet
}

/// How big a Markdown heading is set, which is roughly what a browser does.
fn heading_size(level: usize) -> f64 {
    match level {
        1 => 22.0,
        2 => 17.0,
        3 => 14.0,
        4 => 12.0,
        5 => 11.0,
        _ => 10.0,
    }
}

/// The text of a bulleted line, if it is one.
fn bullet(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    for mark in ["- ", "* ", "+ "] {
        if let Some(rest) = trimmed.strip_prefix(mark) {
            return Some(rest);
        }
    }
    None
}

#[cfg(test)]
mod tests;
