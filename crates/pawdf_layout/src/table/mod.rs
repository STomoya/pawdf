//! Table detection, without rendering. [`tagged`] reads the structure tree of a tagged PDF;
//! [`ruled`] builds the grid from the page's ruling lines; [`borderless`] infers it from how the
//! words line up.
//!
//! Boxes are in display space, as for [`pdfminer::analyze_regions`], so a cell box can be
//! passed back to it: the origin is the top-left of the rotated crop box and y points down, in
//! points. Each cell's text is grouped by [`pdfminer`], with its boxes in pdfminer's page space.

mod borderless;
mod ruled;
mod tagged;

pub use borderless::{BorderlessParams, borderless};
pub use ruled::{RuledParams, ruled};
pub use tagged::tagged;

use pawdf_reader::{Diagnostic, Matrix, PageContent, Point, Rect, Rotation};

use crate::pdfminer::{self, Params};
use crate::{Error, Layout};

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
    /// Whether it is a header cell (`TH`); always false for a ruled or borderless table.
    pub header: bool,
    /// In display space. A tagged cell's is the union of its glyphs' boxes on this page, `None`
    /// when it has none; a ruled cell's is the rectangle its lines enclose; a borderless cell's
    /// is its band of the grid.
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
struct Grid {
    bbox: Rect,
    xs: Vec<f64>,
    ys: Vec<f64>,
    cells: Vec<GridCell>,
}

struct GridCell {
    rect: Rect,
    row: usize,
    col: usize,
    row_span: usize,
    col_span: usize,
}

/// Rows and columns from the cells' distinct edges; cells sorted by row, then column.
fn grid(cells: &[Rect]) -> Grid {
    let bounds = |f: &dyn Fn(&Rect) -> [f64; 2]| {
        let mut b: Vec<f64> = cells.iter().flat_map(f).collect();
        b.sort_by(f64::total_cmp);
        b.dedup();
        b
    };
    let xs = bounds(&|c| [c.x0, c.x1]);
    let ys = bounds(&|c| [c.y0, c.y1]);
    let at = |b: &[f64], v: f64| b.partition_point(|&x| x < v);
    let mut out: Vec<GridCell> = cells
        .iter()
        .map(|&rect| {
            let (row, col) = (at(&ys, rect.y0), at(&xs, rect.x0));
            GridCell {
                rect,
                row,
                col,
                row_span: at(&ys, rect.y1) - row,
                col_span: at(&xs, rect.x1) - col,
            }
        })
        .collect();
    out.sort_by_key(|c| (c.row, c.col));
    let bbox = cells
        .iter()
        .copied()
        .reduce(|a, b| union(&a, &b))
        .unwrap_or(Rect {
            x0: 0.,
            y0: 0.,
            x1: 0.,
            y1: 0.,
        });
    Grid {
        bbox,
        xs,
        ys,
        cells: out,
    }
}

fn centres(content: &PageContent, display: &Matrix) -> Vec<Point> {
    content
        .glyphs
        .iter()
        .map(|g| pdfminer::centre(display, &g.bbox))
        .collect()
}

/// Each word's glyphs and the union of their boxes in display space.
fn words(
    content: &PageContent,
    media_box: Rect,
    rotate: Rotation,
    display: &Matrix,
    params: &Params,
) -> Result<Vec<(Rect, Vec<usize>)>, Error> {
    Ok(pdfminer::analyze(content, media_box, rotate, params)?
        .blocks
        .into_iter()
        .flat_map(|b| b.lines)
        .flat_map(|l| l.words)
        .filter_map(|w| {
            let b = w
                .glyphs
                .iter()
                .filter_map(|&g| content.glyphs.get(g))
                .map(|g| display.transform_rect(g.bbox))
                .reduce(|a, b| union(&a, &b))?;
            Some((b, w.glyphs))
        })
        .collect())
}

/// Which of `n` glyphs belong to a word overlapping the region, so they are kept with it.
fn whole(words: &[(Rect, Vec<usize>)], region: Option<Rect>, n: usize) -> Vec<bool> {
    let mut whole = vec![false; n];
    for (_, gs) in words
        .iter()
        .filter(|(b, _)| region.is_some_and(|r| overlaps(b, &r)))
    {
        for &g in gs {
            if let Some(w) = whole.get_mut(g) {
                *w = true;
            }
        }
    }
    whole
}

/// The glyphs a table with this box takes, with their centres: those inside it and those kept
/// whole.
fn placed<'a>(
    bbox: Rect,
    centres: &'a [Point],
    whole: &'a [bool],
) -> impl Iterator<Item = (usize, Point)> + 'a {
    centres
        .iter()
        .zip(whole)
        .enumerate()
        .filter(move |(_, (p, w))| {
            let inside = bbox.x0 <= p.x && p.x < bbox.x1 && bbox.y0 <= p.y && p.y < bbox.y1;
            (inside || **w) && p.x.is_finite() && p.y.is_finite()
        })
        .map(|(g, (p, _))| (g, *p))
}

/// The band of sorted `bounds` holding `at`; a value outside goes to the nearest band.
fn slot(bounds: &[f64], at: f64) -> usize {
    bounds
        .partition_point(|&b| b <= at)
        .saturating_sub(1)
        .min(bounds.len().saturating_sub(2))
}

/// Groups each cell's glyphs, `sets` holding one set per cell of every grid in order.
fn finish(
    content: &PageContent,
    media_box: Rect,
    rotate: Rotation,
    grids: Vec<Grid>,
    sets: &[Vec<usize>],
    params: &Params,
) -> Tables {
    let (layouts, diagnostics) = pdfminer::analyze_sets(content, media_box, rotate, sets, params);
    let mut layouts = layouts.into_iter();
    let tables = grids
        .into_iter()
        .map(|t| Table {
            bbox: t.bbox,
            rows: t.ys.len() - 1,
            cols: t.xs.len() - 1,
            cells: t
                .cells
                .into_iter()
                .zip(layouts.by_ref())
                .map(|(c, layout)| Cell {
                    row: c.row,
                    col: c.col,
                    row_span: c.row_span,
                    col_span: c.col_span,
                    header: false,
                    bbox: Some(c.rect),
                    layout,
                })
                .collect(),
        })
        .collect();
    Tables {
        tables,
        diagnostics,
    }
}
