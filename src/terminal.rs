//! Looking at a page without a window.
//!
//! Everything in this program that says "look at it before you print" ends in a
//! file somebody opens: a proof PDF, a preview PNG, the window itself. On a
//! server over SSH there is nothing to open it with. `onionskin serve` runs
//! there, and so do scheduled jobs and `onionskin watch`, and the person
//! looking after them has a terminal and nothing else.
//!
//! So a page can be drawn in the terminal. It is not a substitute for the proof
//! — nobody should approve a run off eighty characters of text — but it answers
//! the question that actually gets asked from an SSH session, which is *is the
//! stamp roughly where I meant, or is it off the paper.* That question is
//! answerable at this resolution and is otherwise unanswerable at all.
//!
//! # How it draws
//!
//! Each character cell covers a rectangle of the page, and the cell is chosen by
//! how much ink is in it. Terminal cells are about twice as tall as they are
//! wide, so the rectangle is twice as tall as it is wide too — otherwise an A4
//! page comes out looking like a bus ticket.
//!
//! Two things it deliberately does not do. It does not use colour: the output
//! goes into logs, over screen readers, and onto terminals with a dozen
//! different palettes, and a page drawn in grey text is legible in all of them.
//! And it does not use half-block characters, which are prettier and are missing
//! from the fonts on exactly the machines this exists for — a page of tofu
//! boxes is worse than a page of hashes.

use crate::geometry::PageSize;

/// The shades, lightest first. Chosen so that the *coverage* they suggest goes
/// up roughly evenly: a page of `:` should read as lighter than a page of `#`
/// on any font somebody has.
pub const SHADES: &[char] = &[' ', '.', ':', '-', '=', '+', '*', '#', '@'];

/// How much taller a terminal cell is than it is wide.
///
/// Two is right for nearly every terminal font. Without it, a portrait A4 page
/// is drawn twice as wide as it is tall, and a stamp measured off it is wrong
/// in the one direction somebody was checking.
pub const CELL_TALLER_BY: usize = 2;

/// The most rows a drawing may have.
///
/// Two hundred is already several screens; past that nobody is reading it, and
/// the number exists to stop a page of an absurd shape asking for a drawing
/// the machine cannot hold.
pub const DOWN_AT_MOST: usize = 200;

/// How wide a drawing is, in characters, when nobody says.
///
/// Eighty is the width every terminal has had since 1928, and it is the width a
/// line of output has to fit in to be safe in a log file.
pub const ACROSS: usize = 78;

/// A page drawn as characters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Drawing {
    pub lines: Vec<String>,
    pub across: usize,
    pub down: usize,
}

impl Drawing {
    /// The whole thing, ready to print.
    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    /// With a frame round it, so the edge of the paper can be told from the
    /// edge of the terminal — which is the whole question when somebody is
    /// asking whether a stamp has fallen off the sheet.
    pub fn framed(&self, caption: &str) -> String {
        let rule = "─".repeat(self.across);
        let mut out = format!("┌{rule}┐\n");
        for line in &self.lines {
            out.push_str(&format!("│{line:<width$}│\n", width = self.across));
        }
        out.push_str(&format!("└{rule}┘"));
        if !caption.is_empty() {
            out.push_str(&format!("\n{caption}"));
        }
        out
    }
}

/// Draw a page of grey pixels as characters.
///
/// `across` is how many characters wide; the height follows from the paper's
/// own shape, so a landscape page comes out landscape.
pub fn draw(gray: &[u8], width: usize, height: usize, page: PageSize, across: usize) -> Drawing {
    let across = across.clamp(8, 400);
    // The page's own shape, not the picture's: a render at a strange resolution
    // must not change what the paper looks like.
    let tall = if page.width_mm > 0.0 {
        page.height_mm / page.width_mm
    } else {
        1.0
    };
    // Clamped at both ends. A page a millimetre wide and a metre tall is a
    // real shape somebody can ask for by mistake, and unclamped it asks for
    // hundreds of thousands of rows; a width that has gone subnormal makes the
    // ratio infinite, and `as usize` saturates rather than failing, so what
    // comes out is a request to allocate usize::MAX rows and a panic reading
    // "capacity overflow".
    let down = ((across as f64 * tall) / CELL_TALLER_BY as f64)
        .round()
        .clamp(1.0, DOWN_AT_MOST as f64) as usize;

    if width == 0 || height == 0 || gray.len() < width * height {
        return Drawing {
            lines: vec![" ".repeat(across); down],
            across,
            down,
        };
    }

    let mut lines = Vec::with_capacity(down);
    for row in 0..down {
        let from_y = row * height / down;
        let to_y = (((row + 1) * height) / down).max(from_y + 1).min(height);
        let mut line = String::with_capacity(across);
        for column in 0..across {
            let from_x = column * width / across;
            let to_x = (((column + 1) * width) / across).max(from_x + 1).min(width);
            line.push(shade_for(darkness(gray, width, from_x, to_x, from_y, to_y)));
        }
        // Trailing spaces are invisible and make every line the full width in
        // a log file for no reason.
        lines.push(line.trim_end().to_string());
    }
    Drawing {
        lines,
        across,
        down,
    }
}

/// How dark a rectangle of the page is, from 0 (paper) to 1 (solid ink).
///
/// The *darkest* pixel counts for more than the average would. A line of type
/// covers perhaps a tenth of the cell it sits in, and averaging makes it
/// invisible — which for a program whose whole business is where the words
/// landed is the one thing the drawing must not do. So the answer is the mean
/// of the darkest quarter, which keeps thin marks visible without turning a
/// speck of dust into a black cell.
fn darkness(
    gray: &[u8],
    width: usize,
    from_x: usize,
    to_x: usize,
    from_y: usize,
    to_y: usize,
) -> f64 {
    let mut values: Vec<u8> = Vec::with_capacity((to_x - from_x) * (to_y - from_y));
    for y in from_y..to_y {
        let row = y * width;
        values.extend_from_slice(&gray[row + from_x..row + to_x]);
    }
    if values.is_empty() {
        return 0.0;
    }
    values.sort_unstable();
    let quarter = (values.len() / 4).max(1);
    let darkest: u32 = values[..quarter].iter().map(|v| u32::from(*v)).sum();
    let mean = darkest as f64 / quarter as f64;
    (255.0 - mean) / 255.0
}

/// Which character stands for that much ink.
fn shade_for(darkness: f64) -> char {
    // Paper is paper. Without this, the faintest smudge in a scan turns the
    // whole page into a wash of dots and nothing stands out.
    if darkness < 0.06 {
        return SHADES[0];
    }
    let step = ((darkness * SHADES.len() as f64).floor() as usize).clamp(1, SHADES.len() - 1);
    SHADES[step]
}

/// A ruler along the bottom, so a position can be read off the drawing rather
/// than guessed at — which is what turns "roughly there" into "150 mm across".
///
/// Never wider than the drawing it sits under. The unit goes on the first
/// label, as `0mm`, rather than after the last one: a trailing " mm across"
/// pushes the line past the width of the terminal it was drawn to fit, and a
/// ruler that wraps onto a second line is worse than no ruler.
///
/// Marks every 50 mm — enough to place something on A4, and few enough that the
/// labels do not run into one another at eighty characters.
pub fn ruler(page: PageSize, across: usize) -> String {
    let across = across.clamp(8, 400);
    // A page with no width cannot say where anything is along it, and dividing
    // by it would give NaN rather than an answer.
    if !page.width_mm.is_finite() || page.width_mm <= 0.0 {
        return String::new();
    }
    const EVERY_MM: f64 = 50.0;

    let at = |at_mm: f64| ((at_mm / page.width_mm) * (across - 1) as f64).round() as usize;
    let mut line = vec![' '; across];
    let mut at_mm = 0.0;
    while at_mm <= page.width_mm {
        let column = at(at_mm);
        if column < across {
            line[column] = '|';
        }
        at_mm += EVERY_MM;
    }
    let drawn: String = line.into_iter().collect();

    let mut marks = String::new();
    // Where the last label ended, or nothing when none has been written. The
    // difference matters at the left edge: the first label starts at column
    // nought, and a rule that simply demanded a higher column than the last
    // would drop it.
    let mut ended: Option<usize> = None;
    let mut at_mm = 0.0;
    while at_mm <= page.width_mm {
        let column = at(at_mm);
        // A label that would run into the one before it is left out rather
        // than printed over it. On a narrow drawing that is most of them, and
        // the marks still say where the fifties are.
        let label = if at_mm == 0.0 {
            "0mm".to_string()
        } else {
            format!("{}", at_mm as usize)
        };
        // With a space between: two labels that abut run together into one
        // number that says nothing — "100" ending where "150" begins reads as
        // 100150, and somebody measuring off it has no idea which is which.
        let room = match ended {
            None => true,
            Some(ended) => column > ended,
        };
        if room && column + label.len() <= across {
            marks.push_str(&" ".repeat(column - ended.unwrap_or(0)));
            marks.push_str(&label);
            ended = Some(column + label.len());
        }
        at_mm += EVERY_MM;
    }
    format!("{}\n{}", drawn.trim_end(), marks.trim_end())
}

#[cfg(test)]
#[path = "terminal/tests.rs"]
mod tests;
