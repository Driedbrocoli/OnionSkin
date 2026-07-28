//! Label stock by its code: `--stock l7160` instead of four measurements.
//!
//! [`crate::labels`] asks for the grid and the sizes, and gives its reason:
//! label codes mean different things in different countries and change between
//! years, so a table of them would be wrong for somebody, silently, on paper.
//!
//! That reason is right about the danger and wrong about the remedy. Nobody
//! reads the box. They type the code they bought, and if the program will not
//! take it they measure a label with a ruler and get it wrong by a millimetre —
//! which is the same failure, arrived at more slowly.
//!
//! So the codes are here, and the silence is not. Give `--stock l7160` and
//! Onionskin says what it filled in, in millimetres, and where the numbers came
//! from:
//!
//! ```text
//! --stock l7160: Avery L7160 — address labels, 21 to a sheet.
//!   A4, 3 across by 7 down, labels 63.5 × 38.1 mm,
//!   7.2 mm in from the left, 15.1 mm down from the top, 2.5 mm between columns.
//!   These are the published measurements. The box is the authority: print one
//!   sheet on plain paper and hold it up to the stock before committing the box.
//! ```
//!
//! Every measurement stays overridable — `--label`, `--margin-x` and the rest
//! win over the code — so a sheet that is a millimetre out is corrected rather
//! than abandoned.
//!
//! # How the numbers are checked
//!
//! Not by trusting them. Label stock is symmetrical: the margin on the left of
//! the sheet equals the margin on the right, and the top equals the bottom,
//! because the labels are die-cut in the middle of the paper. So every entry
//! below is arithmetic that has to come out even, and the tests do that sum on
//! all of them. A digit typed wrongly moves one margin and not the other, and
//! the sum stops.

/// One kind of label stock, as its maker publishes it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stock {
    /// The code as it is printed on the box, lower-cased.
    pub code: &'static str,
    /// The same stock's other codes. Avery sells one label under four numbers.
    pub also: &'static [&'static str],
    /// What it is for, in the words somebody would use asking for it.
    pub what_for: &'static str,
    /// The paper, as [`crate::geometry`] names it.
    pub paper: &'static str,
    pub across: usize,
    pub down: usize,
    /// One label: width then height, in millimetres.
    pub label_mm: (f64, f64),
    /// To the first label: from the left edge, then from the top.
    pub margin_mm: (f64, f64),
    /// Between one label and the next: across, then down.
    pub gap_mm: (f64, f64),
}

impl Stock {
    pub fn per_sheet(&self) -> usize {
        self.across * self.down
    }

    /// The width the labels and the gaps between them take up.
    pub fn across_mm(&self) -> f64 {
        self.across as f64 * self.label_mm.0
            + (self.across.saturating_sub(1)) as f64 * self.gap_mm.0
    }

    pub fn down_mm(&self) -> f64 {
        self.down as f64 * self.label_mm.1 + (self.down.saturating_sub(1)) as f64 * self.gap_mm.1
    }

    /// What was filled in, said out loud.
    ///
    /// The whole point of allowing a code at all: the numbers it stands for are
    /// on the screen before any paper moves, so a code that means something
    /// else where you live is caught by somebody reading, rather than by a
    /// sheet of ruined stock.
    pub fn describe(&self) -> String {
        let mut said = format!(
            "{} — {}.\n  {}, {} across by {} down, labels {} × {} mm,\n  \
             {} mm in from the left, {} mm down from the top",
            self.name(),
            self.what_for,
            self.paper_named(),
            self.across,
            self.down,
            trim(self.label_mm.0),
            trim(self.label_mm.1),
            trim(self.margin_mm.0),
            trim(self.margin_mm.1),
        );
        if self.gap_mm.0 > 0.0 {
            said.push_str(&format!(", {} mm between columns", trim(self.gap_mm.0)));
        }
        if self.gap_mm.1 > 0.0 {
            said.push_str(&format!(", {} mm between rows", trim(self.gap_mm.1)));
        }
        said.push_str(
            ".\n  These are the published measurements. The box is the \
             authority: print one\n  sheet on plain paper and hold it up to the \
             stock before committing the box.",
        );
        said
    }

    /// The maker and the code, the way it is written on the packet.
    pub fn name(&self) -> String {
        format!("Avery {}", self.code.to_uppercase())
    }

    fn paper_named(&self) -> &'static str {
        match self.paper {
            "letter" => "US Letter",
            other => {
                // A4 and the rest, capitalised the way people write them.
                if other == "a4" {
                    "A4"
                } else {
                    other
                }
            }
        }
    }
}

/// Drop the trailing nought from a whole number of millimetres.
fn trim(mm: f64) -> String {
    let said = format!("{mm:.1}");
    said.strip_suffix(".0").unwrap_or(&said).to_string()
}

/// The stock Onionskin knows, by code.
///
/// Deliberately short. Every entry is one somebody can buy in a supermarket,
/// with measurements its maker publishes; a long list padded out with codes
/// nobody checked would be the silent-wrongness this module exists to avoid.
/// Anything not here is still four numbers and `--label`.
pub const KNOWN: &[Stock] = &[
    // ---- A4, the European sheet -------------------------------------------
    Stock {
        code: "l7159",
        also: &["j8159", "l7159-25"],
        what_for: "address labels, 24 to a sheet",
        paper: "a4",
        across: 3,
        down: 8,
        label_mm: (63.5, 33.9),
        margin_mm: (7.2, 12.9),
        gap_mm: (2.5, 0.0),
    },
    Stock {
        code: "l7160",
        also: &["j8160", "l7160-25", "7160"],
        what_for: "address labels, 21 to a sheet",
        paper: "a4",
        across: 3,
        down: 7,
        label_mm: (63.5, 38.1),
        margin_mm: (7.2, 15.1),
        gap_mm: (2.5, 0.0),
    },
    Stock {
        code: "l7161",
        also: &["j8161"],
        what_for: "address labels, 18 to a sheet",
        paper: "a4",
        across: 3,
        down: 6,
        label_mm: (63.5, 46.6),
        margin_mm: (7.2, 8.7),
        gap_mm: (2.5, 0.0),
    },
    Stock {
        code: "l7162",
        also: &["j8162"],
        what_for: "address labels, 16 to a sheet",
        paper: "a4",
        across: 2,
        down: 8,
        label_mm: (99.1, 33.9),
        margin_mm: (4.65, 12.9),
        gap_mm: (2.5, 0.0),
    },
    Stock {
        code: "l7163",
        also: &["j8163", "7163"],
        what_for: "address labels, 14 to a sheet",
        paper: "a4",
        across: 2,
        down: 7,
        label_mm: (99.1, 38.1),
        margin_mm: (4.65, 15.15),
        gap_mm: (2.5, 0.0),
    },
    Stock {
        code: "l7165",
        also: &["j8165"],
        what_for: "parcel labels, 8 to a sheet",
        paper: "a4",
        across: 2,
        down: 4,
        label_mm: (99.1, 67.7),
        margin_mm: (4.65, 13.1),
        gap_mm: (2.5, 0.0),
    },
    Stock {
        code: "l7651",
        also: &["j8651"],
        what_for: "small labels, 65 to a sheet",
        paper: "a4",
        across: 5,
        down: 13,
        label_mm: (38.1, 21.2),
        margin_mm: (4.75, 10.7),
        gap_mm: (2.5, 0.0),
    },
    // ---- US Letter --------------------------------------------------------
    Stock {
        code: "5160",
        also: &["8160", "5960", "8460", "5260"],
        what_for: "address labels, 30 to a sheet",
        paper: "letter",
        across: 3,
        down: 10,
        label_mm: (66.7, 25.4),
        margin_mm: (4.7, 12.7),
        gap_mm: (3.2, 0.0),
    },
    Stock {
        code: "5161",
        also: &["8161", "5961"],
        what_for: "address labels, 20 to a sheet",
        paper: "letter",
        across: 2,
        down: 10,
        label_mm: (101.6, 25.4),
        margin_mm: (3.95, 12.7),
        gap_mm: (4.8, 0.0),
    },
    Stock {
        code: "5162",
        also: &["8162", "5962"],
        what_for: "address labels, 14 to a sheet",
        paper: "letter",
        across: 2,
        down: 7,
        // A third of an inch, not the 33.9 the catalogue rounds it to: seven
        // of those down the sheet is two tenths of a millimetre out, and the
        // last row lands on the cut.
        label_mm: (101.6, 33.87),
        margin_mm: (3.95, 21.15),
        gap_mm: (4.8, 0.0),
    },
    Stock {
        code: "5163",
        also: &["8163", "5963"],
        what_for: "shipping labels, 10 to a sheet",
        paper: "letter",
        across: 2,
        down: 5,
        label_mm: (101.6, 50.8),
        margin_mm: (3.95, 12.7),
        gap_mm: (4.8, 0.0),
    },
    Stock {
        code: "5164",
        also: &["8164", "5964"],
        what_for: "shipping labels, 6 to a sheet",
        paper: "letter",
        across: 2,
        down: 3,
        label_mm: (101.6, 84.7),
        margin_mm: (3.95, 12.65),
        gap_mm: (4.8, 0.0),
    },
];

/// A stock by its code, or by any of the codes the same stock is sold under.
///
/// Case and spacing are ignored, and so is a leading "avery": somebody typing
/// what is on the box types `Avery L7160`, and refusing that would be a program
/// being difficult about nothing.
pub fn find(given: &str) -> Option<&'static Stock> {
    let wanted = tidy(given);
    KNOWN
        .iter()
        .find(|stock| stock.code == wanted || stock.also.iter().any(|other| tidy(other) == wanted))
}

fn tidy(code: &str) -> String {
    let lowered = code.trim().to_lowercase().replace([' ', '-', '_'], "");
    lowered
        .strip_prefix("avery")
        .unwrap_or(&lowered)
        .to_string()
}

/// Every code known, for a message that has to say what is on offer.
pub fn every_code() -> Vec<String> {
    KNOWN
        .iter()
        .map(|stock| {
            format!(
                "{:<8} {} — {}",
                stock.code,
                stock.paper_named(),
                stock.what_for
            )
        })
        .collect()
}

/// What to say when a code is not known.
pub fn not_known(given: &str) -> String {
    format!(
        "'{given}' is not a label stock Onionskin knows.\n\nIt knows:\n  {}\n\n\
         Anything else is four numbers off the box:\n  \
         --grid 3x7 --label 63.5x38.1 --margin-x 7.2 --margin-y 15.1",
        every_code().join("\n  ")
    )
}

#[cfg(test)]
#[path = "stock/tests.rs"]
mod tests;
