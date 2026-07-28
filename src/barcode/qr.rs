//! QR codes, worked out here.
//!
//! A QR code is the one barcode people photograph with a telephone, which makes
//! it the one worth having on a sheet somebody is holding: a link to the form
//! that comes next, a reference number that must not be typed in by hand, the
//! address of the file this printout came from.
//!
//! # What is going on
//!
//! Five stages, and each is arithmetic:
//!
//!   1. **The text becomes bits.** Three ways of doing it — digits packed three
//!      to ten bits, capitals and a few marks packed two to eleven, and anything
//!      at all a byte at a time. The narrowest one that can hold the text wins,
//!      because it is the one that fits in the smallest square.
//!   2. **A version is chosen**: the smallest square the bits fit in. Version 1
//!      is 21 modules across, version 40 is 177, and each step adds four.
//!   3. **Error correction is worked out.** Reed–Solomon over the field of 256,
//!      which is what lets a QR code survive a crease, a coffee ring, or a logo
//!      printed over the middle of it. The data and the correction are split
//!      into blocks and interleaved so that damage in one place is spread thin.
//!   4. **The square is drawn**: the three big eyes, the little ones, the dotted
//!      lines between them, and then the data threaded up and down the rest in a
//!      zigzag two modules wide.
//!   5. **A mask is chosen.** The data is XORed against one of eight patterns
//!      and the one that leaves the fewest awkward features — long runs, big
//!      blocks, anything resembling an eye — is kept. This is what stops a
//!      number of a particular shape drawing something a scanner mistakes for
//!      part of the frame.
//!
//! # How much correction
//!
//! Four levels, and the choice is real. [`Ecc::Low`] spends 7% of the square on
//! correction, [`Ecc::High`] spends 30% — which is a bigger square for the same
//! text, and a code that still reads with a third of it destroyed. On paper that
//! is going into an envelope, through a scanner, or onto a shelf for two years,
//! [`Ecc::Medium`] is the sensible middle and is what this defaults to.

use super::Symbol;

/// How much of the square is spent on surviving damage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ecc {
    /// About 7% recoverable. The smallest square, for a code that will be
    /// scanned off a screen or fresh paper and thrown away.
    Low,
    /// About 15%. The default, and the right answer for anything printed.
    Medium,
    /// About 25%.
    Quartile,
    /// About 30%. For a label that will be handled, or one with something
    /// printed over the middle of it.
    High,
}

impl Ecc {
    /// Where this level sits in the tables below.
    fn row(self) -> usize {
        match self {
            Ecc::Low => 0,
            Ecc::Medium => 1,
            Ecc::Quartile => 2,
            Ecc::High => 3,
        }
    }

    /// The two bits the format information carries. Not the same order as the
    /// tables, which is a quirk of the standard rather than a mistake here.
    fn format_bits(self) -> u32 {
        match self {
            Ecc::Low => 1,
            Ecc::Medium => 0,
            Ecc::Quartile => 3,
            Ecc::High => 2,
        }
    }

    pub fn parse(name: &str) -> Option<Ecc> {
        Some(match name.trim().to_ascii_lowercase().as_str() {
            "l" | "low" => Ecc::Low,
            "m" | "medium" => Ecc::Medium,
            "q" | "quartile" => Ecc::Quartile,
            "h" | "high" => Ecc::High,
            _ => return None,
        })
    }

    pub fn describe(self) -> &'static str {
        match self {
            Ecc::Low => "low: about 7% of it can be lost",
            Ecc::Medium => "medium: about 15% of it can be lost",
            Ecc::Quartile => "quartile: about 25% of it can be lost",
            Ecc::High => "high: about 30% of it can be lost",
        }
    }
}

/// How the text is packed into bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    Numeric,
    Alphanumeric,
    Byte,
}

impl Mode {
    fn indicator(self) -> u32 {
        match self {
            Mode::Numeric => 1,
            Mode::Alphanumeric => 2,
            Mode::Byte => 4,
        }
    }

    /// How many bits the character count takes, which grows with the version
    /// because a bigger square holds more characters to count.
    pub(crate) fn count_bits(self, version: usize) -> usize {
        let band = match version {
            1..=9 => 0,
            10..=26 => 1,
            _ => 2,
        };
        match self {
            Mode::Numeric => [10, 12, 14][band],
            Mode::Alphanumeric => [9, 11, 13][band],
            Mode::Byte => [8, 16, 16][band],
        }
    }
}

/// The characters alphanumeric mode can pack, in the order it numbers them.
const ALPHANUMERIC: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ $%*+-./:";

/// The mode indicator that says "a character set follows".
const ECI_MODE: u32 = 7;

/// The number the standard gives UTF-8 in its register of character sets.
const ECI_UTF8: u32 = 26;

/// Whether this text has to say out loud which character set it is in.
///
/// A QR code's byte mode carries bytes, and says nothing about what they mean.
/// The standard's default is Latin-1, so a decoder handed the two bytes of an
/// é has to guess — and they guess differently. zbar reads `café` back as
/// `caf矇`, having decided the bytes were part of a Chinese codepage; another
/// reader will say `cafÃ©`; a telephone will probably get it right.
///
/// None of them is wrong, because nothing in the code said. So when the text is
/// not plain ASCII, twelve bits go in front of it that name UTF-8, and the
/// guessing stops. Plain ASCII is left alone: it means the same in every
/// character set anybody would guess, and the twelve bits are twelve bits.
fn needs_a_character_set(text: &str, mode: Mode) -> bool {
    mode == Mode::Byte && !text.is_ascii()
}

/// Error-correction codewords per block, by level then version.
const ECC_PER_BLOCK: [[u8; 41]; 4] = [
    // Padding at index 0: there is no version 0, and counting from 1 here is
    // worth more than the byte it costs.
    [
        0, 7, 10, 15, 20, 26, 18, 20, 24, 30, 18, 20, 24, 26, 30, 22, 24, 28, 30, 28, 28, 28, 28,
        30, 30, 26, 28, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30,
    ],
    [
        0, 10, 16, 26, 18, 24, 16, 18, 22, 22, 26, 30, 22, 22, 24, 24, 28, 28, 26, 26, 26, 26, 28,
        28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28, 28,
    ],
    [
        0, 13, 22, 18, 26, 18, 24, 18, 22, 20, 24, 28, 26, 24, 20, 30, 24, 28, 28, 26, 30, 28, 30,
        30, 30, 30, 28, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30,
    ],
    [
        0, 17, 28, 22, 16, 22, 28, 26, 26, 24, 28, 24, 28, 22, 24, 24, 30, 28, 28, 26, 28, 30, 24,
        30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30, 30,
    ],
];

/// How many blocks the data is split into, by level then version.
const BLOCKS: [[u8; 41]; 4] = [
    [
        0, 1, 1, 1, 1, 1, 2, 2, 2, 2, 4, 4, 4, 4, 4, 6, 6, 6, 6, 7, 8, 8, 9, 9, 10, 12, 12, 12, 13,
        14, 15, 16, 17, 18, 19, 19, 20, 21, 22, 24, 25,
    ],
    [
        0, 1, 1, 1, 2, 2, 4, 4, 4, 5, 5, 5, 8, 9, 9, 10, 10, 11, 13, 14, 16, 17, 17, 18, 20, 21,
        23, 25, 26, 28, 29, 31, 33, 35, 37, 38, 40, 43, 45, 47, 49,
    ],
    [
        0, 1, 1, 2, 2, 4, 4, 6, 6, 8, 8, 8, 10, 12, 16, 12, 17, 16, 18, 21, 20, 23, 23, 25, 27, 29,
        34, 34, 35, 38, 40, 43, 45, 48, 51, 53, 56, 59, 62, 65, 68,
    ],
    [
        0, 1, 1, 2, 4, 4, 4, 5, 6, 8, 8, 11, 11, 16, 16, 18, 16, 19, 21, 25, 25, 25, 34, 30, 32,
        35, 37, 40, 42, 45, 48, 51, 54, 57, 60, 63, 66, 70, 74, 77, 81,
    ],
];

/// Quiet zone, in modules. The standard asks for four on every side.
const QUIET: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QrError {
    Empty,
    /// More than the largest QR code holds at this level of correction.
    TooLong {
        bits: usize,
        level: Ecc,
    },
}

impl std::fmt::Display for QrError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            QrError::Empty => write!(f, "there is nothing to put in a QR code"),
            QrError::TooLong { bits, .. } => write!(
                f,
                "that is {} characters' worth, which is more than the largest QR \
                 code holds. A lower level of error correction fits more in; \
                 better still, put a short web address on the paper and the long \
                 text behind it.",
                bits / 8
            ),
        }
    }
}

impl std::error::Error for QrError {}

/// A QR code holding this text.
pub fn encode(text: &str, level: Ecc) -> Result<Symbol, QrError> {
    build(text, level).map(|(symbol, _, _)| symbol)
}

/// The same, with the mask that was chosen and what all eight scored.
///
/// The choice is the one part of this with no right answer written down — eight
/// are tried and the least awkward wins — so it is the part worth being able to
/// check from outside.
fn build(text: &str, level: Ecc) -> Result<(Symbol, u32, [u32; 8]), QrError> {
    if text.is_empty() {
        return Err(QrError::Empty);
    }
    let mode = narrowest_mode(text);
    let (version, data) = fit(text, mode, level)?;
    let codewords = with_correction(&data, version, level);

    let size = 4 * version + 17;
    let mut dark = vec![false; size * size];
    let mut reserved = vec![false; size * size];
    draw_frame(&mut dark, &mut reserved, size, version);
    lay_out(&mut dark, &reserved, size, &codewords);

    // Every mask is tried and the least awkward kept. Skipping this is how a
    // code with a run of the same digit comes out with a stripe through it that
    // a scanner reads as part of the frame.
    let mut best = 0;
    let mut penalties = [u32::MAX; 8];
    for mask in 0..8u32 {
        let mut tried = dark.clone();
        apply_mask(&mut tried, &reserved, size, mask);
        draw_format(&mut tried, size, level, mask);
        penalties[mask as usize] = penalty_of(&tried, size);
        if penalties[mask as usize] < penalties[best as usize] {
            best = mask;
        }
    }
    apply_mask(&mut dark, &reserved, size, best);
    draw_format(&mut dark, size, level, best);

    Ok((
        Symbol {
            dark,
            width: size,
            height: size,
            quiet: QUIET,
            text: text.to_string(),
        },
        best,
        penalties,
    ))
}

/// The narrowest way of packing this text: fewer bits, smaller square.
fn narrowest_mode(text: &str) -> Mode {
    if text.bytes().all(|b| b.is_ascii_digit()) {
        return Mode::Numeric;
    }
    if text.bytes().all(|b| ALPHANUMERIC.contains(&b)) {
        return Mode::Alphanumeric;
    }
    Mode::Byte
}

/// The smallest version this text fits in, and the data codewords to put in it.
///
/// The version has to be settled before the bits can be counted, because the
/// character count field is wider in a bigger square — so the versions are tried
/// in order rather than worked out in one go.
fn fit(text: &str, mode: Mode, level: Ecc) -> Result<(usize, Vec<u8>), QrError> {
    let header = match needs_a_character_set(text, mode) {
        true => 12,
        false => 0,
    };
    for version in 1..=40usize {
        let capacity = data_codewords(version, level) * 8;
        let needed = header + 4 + mode.count_bits(version) + payload_bits(text, mode);
        if needed <= capacity {
            return Ok((version, bits_for(text, mode, version, capacity)));
        }
    }
    Err(QrError::TooLong {
        bits: payload_bits(text, Mode::Byte),
        level,
    })
}

/// How many bits the text itself takes, before the header.
fn payload_bits(text: &str, mode: Mode) -> usize {
    let n = match mode {
        Mode::Byte => text.len(),
        _ => text.chars().count(),
    };
    match mode {
        // Three digits to ten bits; a leftover pair takes seven and a single
        // takes four.
        Mode::Numeric => 10 * (n / 3) + [0, 4, 7][n % 3],
        // Two characters to eleven bits; a leftover takes six.
        Mode::Alphanumeric => 11 * (n / 2) + 6 * (n % 2),
        Mode::Byte => 8 * n,
    }
}

/// The finished data codewords: header, text, terminator, padding.
fn bits_for(text: &str, mode: Mode, version: usize, capacity: usize) -> Vec<u8> {
    let mut bits = Bits::new();
    if needs_a_character_set(text, mode) {
        bits.push(ECI_MODE, 4);
        bits.push(ECI_UTF8, 8);
    }
    bits.push(mode.indicator(), 4);
    let count = match mode {
        Mode::Byte => text.len(),
        _ => text.chars().count(),
    };
    bits.push(count as u32, mode.count_bits(version));

    match mode {
        Mode::Numeric => {
            let digits: Vec<u32> = text.bytes().map(|b| (b - b'0') as u32).collect();
            for group in digits.chunks(3) {
                let value = group.iter().fold(0, |acc, d| acc * 10 + d);
                bits.push(value, [0, 4, 7, 10][group.len()]);
            }
        }
        Mode::Alphanumeric => {
            let values: Vec<u32> = text
                .bytes()
                .map(|b| ALPHANUMERIC.iter().position(|c| *c == b).unwrap_or(0) as u32)
                .collect();
            for pair in values.chunks(2) {
                match pair {
                    [a, b] => bits.push(a * 45 + b, 11),
                    [a] => bits.push(*a, 6),
                    _ => unreachable!(),
                }
            }
        }
        Mode::Byte => {
            for byte in text.bytes() {
                bits.push(byte as u32, 8);
            }
        }
    }

    // The terminator is up to four zero bits, and fewer if there is no room —
    // a full square ends without one, which is allowed and is what a decoder
    // expects.
    let terminator = 4.min(capacity - bits.len());
    bits.push(0, terminator);
    // Then out to a whole byte, then the two padding bytes alternating. They
    // are 11101100 and 00010001, which are simply the two the standard picked.
    let to_byte = (8 - bits.len() % 8) % 8;
    bits.push(0, to_byte);
    for (index, _) in (bits.len()..capacity).step_by(8).enumerate() {
        bits.push(if index % 2 == 0 { 0xEC } else { 0x11 }, 8);
    }
    bits.bytes
}

/// A stream of bits being built into bytes, most significant first.
struct Bits {
    bytes: Vec<u8>,
    bits: usize,
}

impl Bits {
    fn new() -> Bits {
        Bits {
            bytes: Vec::new(),
            bits: 0,
        }
    }

    fn len(&self) -> usize {
        self.bits
    }

    fn push(&mut self, value: u32, width: usize) {
        for shift in (0..width).rev() {
            if self.bits % 8 == 0 {
                self.bytes.push(0);
            }
            if (value >> shift) & 1 == 1 {
                let at = self.bytes.len() - 1;
                self.bytes[at] |= 0x80 >> (self.bits % 8);
            }
            self.bits += 1;
        }
    }
}

/// How many modules of a version hold data, before correction is taken out.
fn raw_data_modules(version: usize) -> usize {
    let mut modules = (16 * version + 128) * version + 64;
    if version >= 2 {
        let alignments = version / 7 + 2;
        modules -= (25 * alignments - 10) * alignments - 55;
        if version >= 7 {
            // The version information, printed twice.
            modules -= 36;
        }
    }
    modules
}

/// How many codewords of the version are the text rather than the correction.
pub(crate) fn data_codewords(version: usize, level: Ecc) -> usize {
    let total = raw_data_modules(version) / 8;
    let blocks = BLOCKS[level.row()][version] as usize;
    let per_block = ECC_PER_BLOCK[level.row()][version] as usize;
    total - per_block * blocks
}

/// The data split into blocks, each given its correction, then interleaved.
///
/// Interleaving is the point of the whole arrangement: a thumbprint over one
/// corner damages a few codewords of every block rather than all of one, and
/// each block can only mend a few.
fn with_correction(data: &[u8], version: usize, level: Ecc) -> Vec<u8> {
    let blocks = BLOCKS[level.row()][version] as usize;
    let ecc_len = ECC_PER_BLOCK[level.row()][version] as usize;
    let total = raw_data_modules(version) / 8;
    // The short blocks come first and the long ones after, each long one
    // holding exactly one byte more.
    let short_blocks = blocks - total % blocks;
    let short_len = total / blocks - ecc_len;

    let generator = generator_polynomial(ecc_len);
    let mut pieces: Vec<(Vec<u8>, Vec<u8>)> = Vec::with_capacity(blocks);
    let mut at = 0;
    for index in 0..blocks {
        let len = short_len + usize::from(index >= short_blocks);
        let piece = data[at..at + len].to_vec();
        at += len;
        let ecc = remainder(&piece, &generator);
        pieces.push((piece, ecc));
    }

    let mut out = Vec::with_capacity(total);
    for column in 0..short_len + 1 {
        for (piece, _) in &pieces {
            if let Some(byte) = piece.get(column) {
                out.push(*byte);
            }
        }
    }
    for column in 0..ecc_len {
        for (_, ecc) in &pieces {
            out.push(ecc[column]);
        }
    }
    out
}

/// The longest plain-byte text a version holds at this level.
///
/// Public within the crate because it is what the tests use to land on a
/// particular version deliberately: every one of the three hundred numbers in
/// the two tables above was copied in by hand, and a wrong one produces a code
/// of exactly the right shape that no scanner can read.
#[cfg(test)]
pub(crate) fn longest_at(version: usize, level: Ecc) -> usize {
    let bits = data_codewords(version, level) * 8;
    (bits - 4 - Mode::Byte.count_bits(version)) / 8
}

// ---------------------------------------------------------------------------
// Reed–Solomon, over the field of 256
// ---------------------------------------------------------------------------

/// Multiply in GF(2^8), the field the standard uses: bytes as polynomials,
/// reduced modulo x^8 + x^4 + x^3 + x^2 + 1.
fn multiply(a: u8, b: u8) -> u8 {
    let mut result = 0u8;
    for shift in (0..8).rev() {
        // Doubling in this field is a shift, and a carry out of the top is
        // where the reduction happens.
        result = (result << 1) ^ (((result >> 7) as i8 as u8 & 1) * 0x1D);
        result ^= ((b >> shift) & 1) * a;
    }
    result
}

/// The divisor for `ecc_len` correction bytes: (x - 2^0)(x - 2^1)…
fn generator_polynomial(ecc_len: usize) -> Vec<u8> {
    let mut poly = vec![0u8; ecc_len];
    poly[ecc_len - 1] = 1;
    let mut root = 1u8;
    for _ in 0..ecc_len {
        for index in 0..ecc_len {
            poly[index] = multiply(poly[index], root);
            if index + 1 < ecc_len {
                poly[index] ^= poly[index + 1];
            }
        }
        root = multiply(root, 2);
    }
    poly
}

/// What is left after dividing the data by the generator: the correction bytes.
fn remainder(data: &[u8], generator: &[u8]) -> Vec<u8> {
    let mut result = vec![0u8; generator.len()];
    for byte in data {
        let factor = byte ^ result.remove(0);
        result.push(0);
        for (index, coefficient) in generator.iter().enumerate() {
            result[index] ^= multiply(*coefficient, factor);
        }
    }
    result
}

// ---------------------------------------------------------------------------
// Drawing the square
// ---------------------------------------------------------------------------

/// Everything that is the frame rather than the message: the eyes, the dotted
/// lines, the little eyes, and the space kept for the format information.
fn draw_frame(dark: &mut [bool], reserved: &mut [bool], size: usize, version: usize) {
    let mut set = |x: usize, y: usize, on: bool, keep: bool| {
        dark[y * size + x] = on;
        reserved[y * size + x] = keep;
    };

    // The dotted lines between the eyes.
    for at in 0..size {
        let on = at % 2 == 0;
        set(6, at, on, true);
        set(at, 6, on, true);
    }

    // The three eyes, and the blank ring around each.
    for (cx, cy) in [
        (3isize, 3isize),
        (size as isize - 4, 3),
        (3, size as isize - 4),
    ] {
        for dy in -4isize..=4 {
            for dx in -4isize..=4 {
                let (x, y) = (cx + dx, cy + dy);
                if x < 0 || y < 0 || x >= size as isize || y >= size as isize {
                    continue;
                }
                let reach = dx.abs().max(dy.abs());
                set(x as usize, y as usize, reach != 2 && reach <= 3, true);
            }
        }
    }

    // The little eyes, at every crossing of the alignment rows and columns
    // except where one would sit on top of a big one.
    let centres = alignment_centres(version);
    for (index_y, cy) in centres.iter().enumerate() {
        for (index_x, cx) in centres.iter().enumerate() {
            // The three corners already have a big eye on them, and a little
            // one on top of a big one is not a marking either of them.
            let last = centres.len() - 1;
            let corner = (index_x == 0 && (index_y == 0 || index_y == last))
                || (index_x == last && index_y == 0);
            if corner {
                continue;
            }
            for dy in -2isize..=2 {
                for dx in -2isize..=2 {
                    set(
                        (*cx as isize + dx) as usize,
                        (*cy as isize + dy) as usize,
                        dx.abs().max(dy.abs()) != 1,
                        true,
                    );
                }
            }
        }
    }

    // Room for the format information, which is written after the mask is
    // chosen. Kept clear now so the data does not land on it.
    //
    // One copy sits around the top-left eye — nine modules down and nine
    // across. The other is split: eight beside the top-right eye, seven above
    // the bottom-left one. Seven and not eight, because the eighth module of
    // that column is the dark one below, which belongs to nothing.
    for at in 0..9 {
        set(8, at, false, true);
        set(at, 8, false, true);
    }
    for at in 0..8 {
        set(size - 1 - at, 8, false, true);
    }
    for at in 0..7 {
        set(8, size - 1 - at, false, true);
    }

    // The one module that is always dark, whatever is encoded. Set after the
    // format information is reserved, because the two are next to each other
    // and clearing that column would clear this too.
    set(8, size - 8, true, true);

    // From version 7 the version itself is written into the square, twice.
    if version >= 7 {
        let mut remainder = version as u32;
        for _ in 0..12 {
            remainder = (remainder << 1) ^ ((remainder >> 11) * 0x1F25);
        }
        let bits = (version as u32) << 12 | remainder;
        for at in 0..18 {
            let on = (bits >> at) & 1 == 1;
            let a = size - 11 + at % 3;
            let b = at / 3;
            set(a, b, on, true);
            set(b, a, on, true);
        }
    }
}

/// Where the little eyes go, which is a rule rather than a table.
fn alignment_centres(version: usize) -> Vec<usize> {
    if version == 1 {
        return Vec::new();
    }
    let count = version / 7 + 2;
    // Evenly spaced, rounded up to an even number, working back from the last
    // one — which is why the first gap is the odd one out.
    let step = if version == 32 {
        26
    } else {
        (version * 4 + count * 2 + 1) / (count * 2 - 2) * 2
    };
    let mut centres = vec![6];
    let mut at = version * 4 + 10;
    while centres.len() < count {
        centres.insert(1, at);
        // Stepping back past the first one would go off the square, and the
        // first one is already there — so the walk stops when it is full.
        if centres.len() == count {
            break;
        }
        at -= step;
    }
    centres
}

/// The codewords threaded through everything the frame left, in a zigzag two
/// modules wide starting at the bottom right.
fn lay_out(dark: &mut [bool], reserved: &[bool], size: usize, codewords: &[u8]) {
    let mut bit = 0;
    let mut right = size - 1;
    loop {
        // Column six is the dotted line and is stepped over rather than used.
        if right == 6 {
            right = 5;
        }
        for row in 0..size {
            for across in 0..2 {
                let x = right - across;
                // Every other pair of columns is travelled upwards, which is
                // what makes it a zigzag rather than a raster.
                let upward = ((right + 1) & 2) == 0;
                let y = if upward { size - 1 - row } else { row };
                if reserved[y * size + x] {
                    continue;
                }
                if bit < codewords.len() * 8 {
                    let byte = codewords[bit / 8];
                    dark[y * size + x] = (byte >> (7 - bit % 8)) & 1 == 1;
                    bit += 1;
                }
                // Anything past the end stays light. A version whose data does
                // not fill it exactly has a few spare modules, and the standard
                // says they are zero.
            }
        }
        if right < 2 {
            break;
        }
        right -= 2;
    }
}

/// Flip the data modules against one of the eight patterns.
fn apply_mask(dark: &mut [bool], reserved: &[bool], size: usize, mask: u32) {
    for y in 0..size {
        for x in 0..size {
            if reserved[y * size + x] {
                continue;
            }
            let flip = match mask {
                0 => (x + y) % 2 == 0,
                1 => y % 2 == 0,
                2 => x % 3 == 0,
                3 => (x + y) % 3 == 0,
                4 => (y / 2 + x / 3) % 2 == 0,
                5 => (x * y) % 2 + (x * y) % 3 == 0,
                6 => ((x * y) % 2 + (x * y) % 3) % 2 == 0,
                _ => ((x + y) % 2 + (x * y) % 3) % 2 == 0,
            };
            dark[y * size + x] ^= flip;
        }
    }
}

/// The fifteen bits saying which level and which mask, written twice so that
/// losing one corner does not lose the ability to read the rest.
fn draw_format(dark: &mut [bool], size: usize, level: Ecc, mask: u32) {
    let data = level.format_bits() << 3 | mask;
    let mut remainder = data;
    for _ in 0..10 {
        remainder = (remainder << 1) ^ ((remainder >> 9) * 0x537);
    }
    let bits = (data << 10 | remainder) ^ 0x5412;

    for at in 0..8 {
        let on = (bits >> at) & 1 == 1;
        // Down the left of the top-right eye.
        dark[8 * size + (size - 1 - at)] = on;
        // And beside the top-left one, stepping over the dotted line.
        let y = if at < 6 { at } else { at + 1 };
        dark[y * size + 8] = on;
    }
    for at in 8..15 {
        let on = (bits >> at) & 1 == 1;
        let x = if at < 9 { 14 - at } else { 14 - at + 1 };
        dark[8 * size + x] = on;
        dark[(size - 15 + at) * size + 8] = on;
    }
}

// ---------------------------------------------------------------------------
// Choosing a mask
// ---------------------------------------------------------------------------

/// How awkward a masked square is. The lowest wins.
///
/// Four rules, all of them about what confuses a scanner: long runs of one
/// colour, blocks of it, anything shaped like an eye, and a square that is much
/// more of one colour than the other.
fn penalty_of(dark: &[bool], size: usize) -> u32 {
    let at = |x: usize, y: usize| dark[y * size + x];
    let mut score = 0u32;

    // Runs of five or more, along and down.
    for line in 0..size {
        for across in [true, false] {
            let mut run = 1;
            let mut previous = if across { at(0, line) } else { at(line, 0) };
            for step in 1..size {
                let this = if across {
                    at(step, line)
                } else {
                    at(line, step)
                };
                if this == previous {
                    run += 1;
                } else {
                    if run >= 5 {
                        score += 3 + (run - 5);
                    }
                    run = 1;
                    previous = this;
                }
            }
            if run >= 5 {
                score += 3 + (run - 5);
            }
        }
    }

    // Two by two of one colour.
    for y in 0..size - 1 {
        for x in 0..size - 1 {
            let corner = at(x, y);
            if at(x + 1, y) == corner && at(x, y + 1) == corner && at(x + 1, y + 1) == corner {
                score += 3;
            }
        }
    }

    // Anything shaped like the eyes: dark-light-dark-dark-dark-light-dark with
    // four light either side of it.
    const EYE: [bool; 7] = [true, false, true, true, true, false, true];
    for line in 0..size {
        for start in 0..size {
            for across in [true, false] {
                let read = |offset: usize| -> Option<bool> {
                    let step = start + offset;
                    (step < size).then(|| {
                        if across {
                            at(step, line)
                        } else {
                            at(line, step)
                        }
                    })
                };
                if (0..7).any(|i| read(i) != Some(EYE[i])) {
                    continue;
                }
                let clear_before = (1..=4).all(|back| match start.checked_sub(back) {
                    Some(step) => {
                        !(if across {
                            at(step, line)
                        } else {
                            at(line, step)
                        })
                    }
                    // Off the edge counts as clear: the quiet zone is there.
                    None => true,
                });
                let clear_after = (7..11).all(|forward| match read(forward) {
                    Some(on) => !on,
                    None => true,
                });
                if clear_before || clear_after {
                    score += 40;
                }
            }
        }
    }

    // And how far off half-and-half it is.
    let darkness = dark.iter().filter(|on| **on).count() * 100 / dark.len();
    let off = darkness.abs_diff(50);
    score += (off as u32 / 5) * 10;

    score
}

#[cfg(test)]
#[path = "qr/tests.rs"]
mod tests;
