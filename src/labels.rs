//! Sheets of labels, from a list.
//!
//! Address labels, file labels, shelf labels. Every office prints them, the
//! stock comes pre-cut in a grid, and the job is always the same: take a
//! column of names and put one in each label.
//!
//! This is the one thing in Onionskin that is not an overlay on something
//! already printed — label stock is blank, and there is nothing to compare
//! against. So no page is rendered, nothing is diffed, and the PDF is written
//! straight out. It is here rather than in some other program because the hard
//! part is the same hard part: getting ink onto a particular rectangle of a
//! particular sheet of paper, in millimetres, and being right about it.
//!
//! # The half-used sheet
//!
//! Nobody ever uses a whole sheet of labels. There is always one in the drawer
//! with the first five peeled off, and printing onto it means starting at the
//! sixth. `--start 6` does that, and it is the difference between this being
//! useful and being a thing people go back to a word processor for.
//!
//! # The grid, and the code that stands for it
//!
//! Label stock is sold by a code — Avery 5160, L7160, and a hundred others —
//! and those codes mean different sizes in different countries and change
//! between years. So a table of them is a way to be wrong for somebody,
//! silently, on paper: the danger this module was originally written to avoid
//! by asking for the four numbers off the box instead.
//!
//! It avoided the danger and kept the failure. Nobody reads the box. They
//! measure a label with a ruler and are half a millimetre out, which is the
//! same ruined sheet arrived at more slowly.
//!
//! [`crate::stock`] has the codes, and takes away the silence rather than the
//! codes: `--stock l7160` prints the measurements it filled in, says the box is
//! the authority, and calls out anything you overrode. The four numbers still
//! work, and still win — which is what makes a code safe to offer.

use crate::geometry::PageSize;

/// One label's rectangle on the paper, in millimetres from the top-left.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cell {
    pub x_mm: f64,
    pub y_mm: f64,
    pub width_mm: f64,
    pub height_mm: f64,
}

impl Cell {
    /// Where a line of text sits inside it: indented from the left edge, and
    /// on a baseline `line` lines down from the top.
    pub fn line_at(&self, line: usize, size_pt: f64, leading: f64, pad_mm: f64) -> (f64, f64) {
        let step_mm = crate::geometry::pt_to_mm(size_pt * leading);
        // The first baseline sits one type-size below the top edge, so the
        // letters are inside the label rather than hanging above it.
        let first = self.y_mm + pad_mm + crate::geometry::pt_to_mm(size_pt);
        (self.x_mm + pad_mm, first + line as f64 * step_mm)
    }

    /// How many lines of this size fit, so a label is not overfilled in
    /// silence.
    pub fn lines_that_fit(&self, size_pt: f64, leading: f64, pad_mm: f64) -> usize {
        let step_mm = crate::geometry::pt_to_mm(size_pt * leading);
        if step_mm <= 0.0 {
            return 0;
        }
        let room = self.height_mm - pad_mm * 2.0;
        if room <= 0.0 {
            return 0;
        }
        (room / step_mm).floor().max(0.0) as usize
    }
}

/// The grid a sheet of label stock is cut into.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Grid {
    pub page: PageSize,
    pub columns: usize,
    pub rows: usize,
    /// From the left edge of the paper to the left edge of the first label.
    pub margin_x_mm: f64,
    /// From the top edge of the paper to the top edge of the first label.
    pub margin_y_mm: f64,
    /// Between one label and the next, across and down.
    pub gap_x_mm: f64,
    pub gap_y_mm: f64,
    /// Each label's size. Without it, the labels are made to fill whatever is
    /// left of the page after the margins and gaps.
    pub label: Option<(f64, f64)>,
}

impl Grid {
    /// How many labels are on one sheet.
    pub fn per_sheet(&self) -> usize {
        self.columns * self.rows
    }

    /// How big each label is, measured or worked out.
    pub fn label_size(&self) -> (f64, f64) {
        if let Some(size) = self.label {
            return size;
        }
        let across = self.page.width_mm - self.margin_x_mm * 2.0 + self.gap_x_mm
            - self.gap_x_mm * self.columns as f64;
        let down = self.page.height_mm - self.margin_y_mm * 2.0 + self.gap_y_mm
            - self.gap_y_mm * self.rows as f64;
        (
            (across / self.columns.max(1) as f64).max(0.0),
            (down / self.rows.max(1) as f64).max(0.0),
        )
    }

    /// The `index`-th label on a sheet, counted from 0, across then down.
    ///
    /// Across then down because that is the order the labels are peeled off,
    /// and therefore the order a half-used sheet is used up in.
    pub fn cell(&self, index: usize) -> Option<Cell> {
        if index >= self.per_sheet() {
            return None;
        }
        let (width_mm, height_mm) = self.label_size();
        let column = index % self.columns;
        let row = index / self.columns;
        Some(Cell {
            x_mm: self.margin_x_mm + column as f64 * (width_mm + self.gap_x_mm),
            y_mm: self.margin_y_mm + row as f64 * (height_mm + self.gap_y_mm),
            width_mm,
            height_mm,
        })
    }

    /// Does this grid actually fit on this paper?
    ///
    /// Checked before anything is written, because a grid that runs off the
    /// sheet does not fail — it prints, onto the part of the label stock that
    /// is backing paper, and the mistake is a wasted sheet rather than an
    /// error message.
    pub fn check(&self) -> Result<(), String> {
        if self.columns == 0 || self.rows == 0 {
            return Err("a grid needs at least one column and one row.".to_string());
        }
        let (width_mm, height_mm) = self.label_size();
        if width_mm <= 0.0 || height_mm <= 0.0 {
            return Err(format!(
                "there is no room left for the labels themselves: {} columns and \
                 {} rows, with {:.1} mm margins and {:.1} mm gaps, uses up more \
                 than the {} it has.",
                self.columns,
                self.rows,
                self.margin_x_mm,
                self.gap_x_mm,
                self.page.describe()
            ));
        }

        let across = self.margin_x_mm
            + self.columns as f64 * width_mm
            + (self.columns.saturating_sub(1)) as f64 * self.gap_x_mm;
        let down = self.margin_y_mm
            + self.rows as f64 * height_mm
            + (self.rows.saturating_sub(1)) as f64 * self.gap_y_mm;

        // A tenth of a millimetre of slack: label stock is measured to the
        // tenth and a grid that comes out exactly flush should not be refused
        // for a rounding error.
        if across > self.page.width_mm + 0.1 {
            return Err(format!(
                "the labels run {:.1} mm off the right-hand edge. {} columns of \
                 {:.1} mm, {:.1} mm apart, starting {:.1} mm in, needs {:.1} mm \
                 and the paper is {:.1} mm.",
                across - self.page.width_mm,
                self.columns,
                width_mm,
                self.gap_x_mm,
                self.margin_x_mm,
                across,
                self.page.width_mm
            ));
        }
        if down > self.page.height_mm + 0.1 {
            return Err(format!(
                "the labels run {:.1} mm off the bottom. {} rows of {:.1} mm, \
                 {:.1} mm apart, starting {:.1} mm down, needs {:.1} mm and the \
                 paper is {:.1} mm.",
                down - self.page.height_mm,
                self.rows,
                height_mm,
                self.gap_y_mm,
                self.margin_y_mm,
                down,
                self.page.height_mm
            ));
        }
        Ok(())
    }

    /// Which sheet and which label the `n`-th name goes on, given that the
    /// first `skip` labels of the first sheet have already been peeled off.
    pub fn place(&self, n: usize, skip: usize) -> (usize, usize) {
        let at = n + skip;
        (at / self.per_sheet(), at % self.per_sheet())
    }

    /// How many sheets `count` labels need, starting `skip` labels in.
    pub fn sheets_needed(&self, count: usize, skip: usize) -> usize {
        if count == 0 {
            return 0;
        }
        let per = self.per_sheet();
        (count + skip).div_ceil(per)
    }

    pub fn describe(&self) -> String {
        let (width_mm, height_mm) = self.label_size();
        format!(
            "{} × {} labels of {:.1} × {:.1} mm on {}",
            self.columns,
            self.rows,
            width_mm,
            height_mm,
            self.page.describe()
        )
    }
}

#[cfg(test)]
#[path = "labels/tests.rs"]
mod tests;
