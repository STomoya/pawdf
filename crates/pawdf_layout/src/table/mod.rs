//! Table detection, without rendering. [`tagged`] reads the structure tree of a tagged PDF;
//! [`ruled`] builds the grid from the page's ruling lines.
//!
//! Boxes are in display space, as for [`pdfminer::analyze_regions`], so a cell box can be
//! passed back to it: the origin is the top-left of the rotated crop box and y points down, in
//! points. Each cell's text is grouped by [`pdfminer`], with its boxes in pdfminer's page space.

mod ruled;
mod tagged;

pub use ruled::{RuledParams, ruled};
pub use tagged::tagged;

use pawdf_reader::{Diagnostic, Rect};

use crate::Layout;
#[cfg(doc)]
use crate::pdfminer;

/// The tables found on one page.
#[derive(Debug, Clone, PartialEq)]
pub struct Tables {
    /// Tables, in the order of the detection route.
    pub tables: Vec<Table>,
    /// Recoveries made by table detection only, once for the page; reading the page reports its
    /// own. A cell's [`Layout`] holds only those made grouping its text.
    pub diagnostics: Vec<Diagnostic>,
}

/// A table on one page.
#[derive(Debug, Clone, PartialEq)]
pub struct Table {
    /// Union of its cells' boxes, in display space.
    pub bbox: Rect,
    /// Rows of the whole table; for a tagged table, including rows on other pages.
    pub rows: usize,
    /// Columns of the whole table, counting spans.
    pub cols: usize,
    /// Its cells on this page, row by row.
    pub cells: Vec<Cell>,
}

/// One table cell.
#[derive(Debug, Clone, PartialEq)]
pub struct Cell {
    /// 0-based row in the whole table; for a tagged table split across pages, the parts line up.
    pub row: usize,
    /// 0-based column.
    pub col: usize,
    /// Rows it spans.
    pub row_span: usize,
    /// Columns it spans.
    pub col_span: usize,
    /// Whether it is a header cell (`TH`); always false for a ruled table.
    pub header: bool,
    /// In display space. A tagged cell's is the union of its glyphs' boxes on this page, `None`
    /// when it has none; a ruled cell's is the rectangle its lines enclose.
    pub bbox: Option<Rect>,
    /// Its text, grouped by [`pdfminer`] with boxes in pdfminer's page space.
    pub layout: Layout,
}

/// The region with its corners normalized; `InvalidParam` when a corner is not finite.
fn region(region: Option<Rect>) -> Result<Option<Rect>, crate::Error> {
    match region {
        Some(r) if !finite(&r) => Err(crate::Error::InvalidParam { name: "region" }),
        r => Ok(r.map(|r| Rect::from_corners(r.x0, r.y0, r.x1, r.y1))),
    }
}

fn finite(r: &Rect) -> bool {
    [r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.is_finite())
}

fn union(a: &Rect, b: &Rect) -> Rect {
    Rect {
        x0: a.x0.min(b.x0),
        y0: a.y0.min(b.y0),
        x1: a.x1.max(b.x1),
        y1: a.y1.max(b.y1),
    }
}

/// Whether the boxes share some area; touching edges do not count.
fn overlaps(a: &Rect, b: &Rect) -> bool {
    a.x0 < b.x1 && b.x0 < a.x1 && a.y0 < b.y1 && b.y0 < a.y1
}
