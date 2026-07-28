//! Code 128: the barcode on nearly every parcel and asset tag.
//!
//! It encodes all of printable ASCII, packs digits two to a symbol, and is what
//! a warehouse scanner expects when nobody has said otherwise. That combination
//! is why it is here rather than one of the older codes: Code 39 needs a third
//! more paper for the same characters and cannot manage lower case.
//!
//! # How it is built
//!
//! Each symbol is eleven modules wide, split into six runs — bar, space, bar,
//! space, bar, space — whose widths add to eleven. A symbol's value is looked up
//! in [`PATTERNS`]. The whole barcode is a start symbol, the data, a check
//! symbol, and a stop symbol whose pattern is thirteen modules because it ends
//! with an extra bar.
//!
//! The check symbol is the start value plus each data value times its position,
//! modulo 103. A scanner recomputes it; a barcode with the wrong one is not read
//! wrongly, it is not read at all.
//!
//! # Which code set
//!
//! There are three, and only two are worth having here. **B** covers ASCII 32
//! to 127, which is everything anybody puts on a label. **C** encodes *pairs* of
//! digits in one symbol, halving the paper a number takes. **A** exists for
//! control characters and the upper case its lower case displaces, which no
//! label needs.
//!
//! So this starts in whichever suits the front of the text and switches to C
//! whenever four or more digits run together — the point at which the switch
//! symbol has paid for itself. A twelve-digit reference comes out at seven
//! symbols instead of twelve, and "INV-2024-00817" gets the same treatment for
//! the digits at the end without giving up the letters at the front.

use super::Symbol;

/// The six run-lengths of each symbol, values 0 to 106.
///
/// Straight out of the standard. Every row sums to eleven except the last,
/// which is the stop pattern and has a seventh run.
pub const PATTERNS: [[u8; 6]; 107] = [
    [2, 1, 2, 2, 2, 2],
    [2, 2, 2, 1, 2, 2],
    [2, 2, 2, 2, 2, 1],
    [1, 2, 1, 2, 2, 3],
    [1, 2, 1, 3, 2, 2],
    [1, 3, 1, 2, 2, 2],
    [1, 2, 2, 2, 1, 3],
    [1, 2, 2, 3, 1, 2],
    [1, 3, 2, 2, 1, 2],
    [2, 2, 1, 2, 1, 3],
    [2, 2, 1, 3, 1, 2],
    [2, 3, 1, 2, 1, 2],
    [1, 1, 2, 2, 3, 2],
    [1, 2, 2, 1, 3, 2],
    [1, 2, 2, 2, 3, 1],
    [1, 1, 3, 2, 2, 2],
    [1, 2, 3, 1, 2, 2],
    [1, 2, 3, 2, 2, 1],
    [2, 2, 3, 2, 1, 1],
    [2, 2, 1, 1, 3, 2],
    [2, 2, 1, 2, 3, 1],
    [2, 1, 3, 2, 1, 2],
    [2, 2, 3, 1, 1, 2],
    [3, 1, 2, 1, 3, 1],
    [3, 1, 1, 2, 2, 2],
    [3, 2, 1, 1, 2, 2],
    [3, 2, 1, 2, 2, 1],
    [3, 1, 2, 2, 1, 2],
    [3, 2, 2, 1, 1, 2],
    [3, 2, 2, 2, 1, 1],
    [2, 1, 2, 1, 2, 3],
    [2, 1, 2, 3, 2, 1],
    [2, 3, 2, 1, 2, 1],
    [1, 1, 1, 3, 2, 3],
    [1, 3, 1, 1, 2, 3],
    [1, 3, 1, 3, 2, 1],
    [1, 1, 2, 3, 1, 3],
    [1, 3, 2, 1, 1, 3],
    [1, 3, 2, 3, 1, 1],
    [2, 1, 1, 3, 1, 3],
    [2, 3, 1, 1, 1, 3],
    [2, 3, 1, 3, 1, 1],
    [1, 1, 2, 1, 3, 3],
    [1, 1, 2, 3, 3, 1],
    [1, 3, 2, 1, 3, 1],
    [1, 1, 3, 1, 2, 3],
    [1, 1, 3, 3, 2, 1],
    [1, 3, 3, 1, 2, 1],
    [3, 1, 3, 1, 2, 1],
    [2, 1, 1, 3, 3, 1],
    [2, 3, 1, 1, 3, 1],
    [2, 1, 3, 1, 1, 3],
    [2, 1, 3, 3, 1, 1],
    [2, 1, 3, 1, 3, 1],
    [3, 1, 1, 1, 2, 3],
    [3, 1, 1, 3, 2, 1],
    [3, 3, 1, 1, 2, 1],
    [3, 1, 2, 1, 1, 3],
    [3, 1, 2, 3, 1, 1],
    [3, 3, 2, 1, 1, 1],
    [3, 1, 4, 1, 1, 1],
    [2, 2, 1, 4, 1, 1],
    [4, 3, 1, 1, 1, 1],
    [1, 1, 1, 2, 2, 4],
    [1, 1, 1, 4, 2, 2],
    [1, 2, 1, 1, 2, 4],
    [1, 2, 1, 4, 2, 1],
    [1, 4, 1, 1, 2, 2],
    [1, 4, 1, 2, 2, 1],
    [1, 1, 2, 2, 1, 4],
    [1, 1, 2, 4, 1, 2],
    [1, 2, 2, 1, 1, 4],
    [1, 2, 2, 4, 1, 1],
    [1, 4, 2, 1, 1, 2],
    [1, 4, 2, 2, 1, 1],
    [2, 4, 1, 2, 1, 1],
    [2, 2, 1, 1, 1, 4],
    [4, 1, 3, 1, 1, 1],
    [2, 4, 1, 1, 1, 2],
    [1, 3, 4, 1, 1, 1],
    [1, 1, 1, 2, 4, 2],
    [1, 2, 1, 1, 4, 2],
    [1, 2, 1, 2, 4, 1],
    [1, 1, 4, 2, 1, 2],
    [1, 2, 4, 1, 1, 2],
    [1, 2, 4, 2, 1, 1],
    [4, 1, 1, 2, 1, 2],
    [4, 2, 1, 1, 1, 2],
    [4, 2, 1, 2, 1, 1],
    [2, 1, 2, 1, 4, 1],
    [2, 1, 4, 1, 2, 1],
    [4, 1, 2, 1, 2, 1],
    [1, 1, 1, 1, 4, 3],
    [1, 1, 1, 3, 4, 1],
    [1, 3, 1, 1, 4, 1],
    [1, 1, 4, 1, 1, 3],
    [1, 1, 4, 3, 1, 1],
    [4, 1, 1, 1, 1, 3],
    [4, 1, 1, 3, 1, 1],
    [1, 1, 3, 1, 4, 1],
    [1, 1, 4, 1, 3, 1],
    [3, 1, 1, 1, 4, 1],
    [4, 1, 1, 1, 3, 1],
    [2, 1, 1, 4, 1, 2],
    [2, 1, 1, 2, 1, 4],
    [2, 1, 1, 2, 3, 2],
    [2, 3, 3, 1, 1, 1],
];

/// The stop symbol's seventh run: the extra bar that ends the barcode.
const STOP_TAIL: u8 = 2;

const START_B: usize = 104;
const START_C: usize = 105;
const STOP: usize = 106;
/// Switch to the other code set, from B and from C respectively.
const TO_C: usize = 99;
const TO_B: usize = 100;

/// Quiet zone, in modules.
///
/// The standard asks for ten, and ten is what is left. Scanners that manage on
/// less do so by luck, and the paper it costs at a quarter of a millimetre a
/// module is two and a half millimetres.
const QUIET: usize = 10;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Code128Error {
    /// Nothing to encode.
    Empty,
    /// A character Code 128 has no symbol for.
    OutOfRange { character: char, at: usize },
    /// Long enough that no scanner will manage it in one pass.
    TooLong { characters: usize },
}

impl std::fmt::Display for Code128Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Code128Error::Empty => write!(f, "there is nothing to put in a barcode"),
            Code128Error::OutOfRange { character, at } => write!(
                f,
                "Code 128 has no symbol for '{character}' (character {}). It \
                 covers the ASCII printing characters — letters without \
                 accents, digits, and punctuation. A QR code takes anything.",
                at + 1
            ),
            Code128Error::TooLong { characters } => write!(
                f,
                "{characters} characters is too long for one barcode. Much over \
                 forty and it is wider than the paper at a size a scanner can \
                 read; a QR code holds far more in a square."
            ),
        }
    }
}

impl std::error::Error for Code128Error {}

/// Beyond this a barcode is wider than a sheet of paper at a readable size.
///
/// At 0.33 mm a module — about the smallest a desktop laser is dependable at —
/// eighty characters of Code B comes to 300 mm, and A4 is 210. Refusing is
/// kinder than printing something off the edge of the page.
const MOST_CHARACTERS: usize = 80;

/// A Code 128 barcode.
pub fn encode(text: &str) -> Result<Symbol, Code128Error> {
    if text.is_empty() {
        return Err(Code128Error::Empty);
    }
    if text.chars().count() > MOST_CHARACTERS {
        return Err(Code128Error::TooLong {
            characters: text.chars().count(),
        });
    }
    for (at, character) in text.chars().enumerate() {
        let code = character as u32;
        if !(32..=126).contains(&code) {
            return Err(Code128Error::OutOfRange { character, at });
        }
    }

    let values = values_for(text.as_bytes());
    let mut modules: Vec<bool> = Vec::new();
    for (index, value) in values.iter().enumerate() {
        let pattern = PATTERNS[*value];
        // Bar, space, bar, space, bar, space — dark on the even runs.
        for (run, width) in pattern.iter().enumerate() {
            for _ in 0..*width {
                modules.push(run % 2 == 0);
            }
        }
        if index + 1 == values.len() {
            modules.resize(modules.len() + STOP_TAIL as usize, true);
        }
    }

    Ok(Symbol {
        width: modules.len(),
        height: 1,
        dark: modules,
        quiet: QUIET,
        text: text.to_string(),
    })
}

/// The symbol values: a start, the data, the check symbol, and the stop.
fn values_for(bytes: &[u8]) -> Vec<usize> {
    let mut values = Vec::new();
    // Start in C when the text opens with enough digits to be worth it, which
    // is the whole of a numeric reference and the commonest case there is.
    let mut in_c = digits_from(bytes, 0) >= 4 || (bytes.len() >= 2 && all_digits(bytes));
    values.push(if in_c { START_C } else { START_B });

    let mut at = 0;
    while at < bytes.len() {
        if in_c {
            // C takes digits two at a time and can do nothing else, so an odd
            // digit at the end drops back to B for that one character.
            if at + 1 < bytes.len() && bytes[at].is_ascii_digit() && bytes[at + 1].is_ascii_digit()
            {
                let pair = (bytes[at] - b'0') * 10 + (bytes[at + 1] - b'0');
                values.push(pair as usize);
                at += 2;
                continue;
            }
            values.push(TO_B);
            in_c = false;
            continue;
        }

        // Whether switching to C pays, counted rather than guessed. The switch
        // costs one symbol; each pair of digits saves one; coming back costs
        // another unless the digits run to the end of the text.
        //
        // So six digits in the middle saves one (1 + 3 + 1 = 5 against 6), and
        // four in the middle saves nothing (1 + 2 + 1 = 4 against 4) — which is
        // why the rule is six in the middle and four at the end, and not the
        // round number it looks like it should be.
        let run = digits_from(bytes, at);
        let to_the_end = at + run == bytes.len();
        let odd = run % 2 == 1;
        // Back to B: needed if a digit is left over at the end of the run, or
        // if there is anything at all after the digits.
        let back = usize::from(odd || !to_the_end);
        let cost = 1 + run / 2 + back + usize::from(odd);
        if run >= 4 && cost < run {
            values.push(TO_C);
            in_c = true;
            continue;
        }
        values.push((bytes[at] - 32) as usize);
        at += 1;
    }

    let start = values[0];
    let sum: usize = values
        .iter()
        .skip(1)
        .enumerate()
        .map(|(index, value)| (index + 1) * value)
        .sum();
    values.push((start + sum) % 103);
    values.push(STOP);
    values
}

fn all_digits(bytes: &[u8]) -> bool {
    bytes.iter().all(u8::is_ascii_digit)
}

/// How many digits run from here.
fn digits_from(bytes: &[u8], at: usize) -> usize {
    bytes[at..]
        .iter()
        .take_while(|b| b.is_ascii_digit())
        .count()
}
