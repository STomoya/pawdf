//! Route 3: tables from how words line up, for tables drawn without rules. Follows the "text"
//! strategy of pdfplumber's `TableFinder` (`pdfplumber/table.py`) and its defaults; results are
//! not meant to match pdfplumber's exactly.

use std::cmp::Reverse;

use pawdf_reader::{Matrix, PageContent, Rect, Rotation};

use super::{Tables, centres, finish, grid, overlaps, placed, slot, union, whole, words};
use crate::Error;
use crate::pdfminer::{self, Params};

/// Words line up when their tops, or their lefts, rights or centres, are chained this close, in
/// points; pdfplumber's fixed tolerance.
const ALIGN_TOLERANCE: f64 = 1.0;

/// Overlap checks between column candidates allowed per page. A page of 1,000 words needs at
/// most about 10^6.
const MAX_WORK: u64 = 1 << 23;

/// Cells allowed in a table. A 100 × 100 table has 10^4.
const MAX_CELLS: usize = 1 << 20;

/// Settings for [`borderless`], with pdfplumber's names and defaults.
#[derive(Debug, Clone, PartialEq)]
pub struct BorderlessParams {
    /// Words that must share a left, right or centre to make a column boundary.
    pub min_words_vertical: usize,
    /// Words that must share a top to make a row.
    pub min_words_horizontal: usize,
}

impl Default for BorderlessParams {
    fn default() -> Self {
        BorderlessParams {
            min_words_vertical: 3,
            min_words_horizontal: 1,
        }
    }
}

/// The table laid out by the page's text alone, for tables drawn without ruling lines. Weaker
/// than [`ruled`](super::ruled): without a `region` it reads any page with aligned words as a
/// table, ordinary paragraphs included, so call it only when a table is expected.
///
/// Words come from [`pdfminer`]. Column boundaries are the lefts of groups of at least
/// `min_words_vertical` words whose lefts, rights or centres line up, the group with most words
/// winning where two overlap, plus the right of the rightmost group. Each group of at least
/// `min_words_horizontal` words sharing a top is a row, bounded by its top and bottom. Bands of
/// this grid holding no glyph are then merged away, so the gaps between rows and the space
/// between the text and a loose region don't become empty rows and columns; two neighbouring
/// rows or columns meet halfway across the gap between them. Every cell spans one row and one
/// column.
///
/// Without a region, the table spans the column groups, and a table of one cell is left out.
/// With a `region`, the extent of a table, only words that overlap it count, the region is the
/// table's box, and one table is always returned. Words that overlap the region are kept whole:
/// each of their glyphs goes to the cell containing its centre after the centre is moved inside
/// the table.
///
/// A glyph belongs to the cell containing its box's centre; at a shared edge it goes to the
/// cell to the right or below. Boxes are in display space (see [the module](super)); `display`
/// is the page's [`display_matrix`](pawdf_reader::Page::display_matrix). Hidden glyphs are left
/// out.
///
/// A page whose words need more than a fixed amount of work, or whose table would have too many
/// cells, gets no tables and a `LayoutLimit` diagnostic.
pub fn borderless(
    content: &PageContent,
    media_box: Rect,
    rotate: Rotation,
    display: &Matrix,
    region: Option<Rect>,
    align: &BorderlessParams,
    params: &Params,
) -> Result<Tables, Error> {
    params.validate()?;
    let region = super::region(region)?;
    let words: Vec<_> = words(content, media_box, rotate, display, params)?
        .into_iter()
        .filter(|(b, _)| super::finite(b) && region.is_none_or(|r| overlaps(b, &r)))
        .collect();
    let boxes: Vec<Rect> = words.iter().map(|(b, _)| *b).collect();
    let none = |diagnostics| {
        Ok(Tables {
            tables: Vec::new(),
            diagnostics,
        })
    };
    let Some(cols) = columns(&boxes, align.min_words_vertical) else {
        return none(vec![pdfminer::limit()]);
    };
    let rows = rows(&boxes, align.min_words_horizontal);
    let extent = match region {
        Some(r) => r,
        None if rows.is_empty() => return none(Vec::new()),
        None => match cols.iter().copied().reduce(|a, b| union(&a, &b)) {
            Some(c) => c,
            None => return none(Vec::new()),
        },
    };
    let col_right = cols.iter().map(|c| c.x1).fold(f64::NEG_INFINITY, f64::max);
    let xs = bounds(
        (extent.x0, extent.x1),
        cols.iter().map(|c| c.x0).chain([col_right]),
    );
    let ys = bounds(
        (extent.y0, extent.y1),
        rows.iter().flat_map(|r| [r.y0, r.y1]),
    );
    let centres = centres(content, display);
    let whole = whole(&words, region, centres.len());
    let mut used_x = vec![false; xs.len().saturating_sub(1)];
    let mut used_y = vec![false; ys.len().saturating_sub(1)];
    for (_, p) in placed(extent, &centres, &whole) {
        if let Some(u) = used_x.get_mut(slot(&xs, p.x)) {
            *u = true;
        }
        if let Some(u) = used_y.get_mut(slot(&ys, p.y)) {
            *u = true;
        }
    }
    let xs = compress((extent.x0, extent.x1), &xs, &used_x);
    let ys = compress((extent.y0, extent.y1), &ys, &used_y);
    let (ncols, nrows) = (xs.len() - 1, ys.len() - 1);
    if ncols.saturating_mul(nrows) > MAX_CELLS {
        return none(vec![pdfminer::limit()]);
    }
    if region.is_none() && ncols * nrows == 1 {
        return none(Vec::new());
    }
    let cells: Vec<Rect> = ys
        .windows(2)
        .flat_map(|y| {
            xs.windows(2).map(move |x| Rect {
                x0: x[0],
                y0: y[0],
                x1: x[1],
                y1: y[1],
            })
        })
        .collect();
    let mut sets = vec![Vec::new(); cells.len()];
    for (g, p) in placed(extent, &centres, &whole) {
        sets[slot(&ys, p.y) * ncols + slot(&xs, p.x)].push(g);
    }
    Ok(finish(
        content,
        media_box,
        rotate,
        vec![grid(&cells)],
        &sets,
        params,
    ))
}

/// Indices into `v`, grouped where each value is within [`ALIGN_TOLERANCE`] of the one before
/// in sorted order, as pdfplumber's `cluster_list`.
fn cluster(v: &[f64]) -> Vec<Vec<usize>> {
    let mut idx: Vec<usize> = (0..v.len()).collect();
    idx.sort_by(|&a, &b| v[a].total_cmp(&v[b]));
    let mut out: Vec<Vec<usize>> = Vec::new();
    let mut last = f64::NEG_INFINITY;
    for i in idx {
        match out.last_mut() {
            Some(c) if v[i] <= last + ALIGN_TOLERANCE => c.push(i),
            _ => out.push(vec![i]),
        }
        last = v[i];
    }
    out
}

fn boxes_of(c: &[usize], boxes: &[Rect]) -> Option<Rect> {
    c.iter().map(|&i| boxes[i]).reduce(|a, b| union(&a, &b))
}

/// The boxes of the groups of at least `min` words sharing a top, top to bottom.
fn rows(boxes: &[Rect], min: usize) -> Vec<Rect> {
    let tops: Vec<f64> = boxes.iter().map(|b| b.y0).collect();
    cluster(&tops)
        .iter()
        .filter(|c| c.len() >= min)
        .filter_map(|c| boxes_of(c, boxes))
        .collect()
}

/// The boxes of the groups of at least `min` words whose lefts, rights or centres line up, left
/// to right, dropping each that touches a bigger one; `None` past the work limit.
fn columns(boxes: &[Rect], min: usize) -> Option<Vec<Rect>> {
    let keys: [fn(&Rect) -> f64; 3] = [|b| b.x0, |b| b.x1, |b| b.x0 * 0.5 + b.x1 * 0.5];
    let mut groups: Vec<Vec<usize>> = keys
        .iter()
        .flat_map(|k| cluster(&boxes.iter().map(k).collect::<Vec<_>>()))
        .filter(|c| c.len() >= min)
        .collect();
    groups.sort_by_key(|c| Reverse(c.len()));
    let mut kept: Vec<Rect> = Vec::new();
    let mut work = 0u64;
    for b in groups.iter().filter_map(|c| boxes_of(c, boxes)) {
        work += kept.len() as u64 + 1;
        if work > MAX_WORK {
            return None;
        }
        if !kept.iter().any(|k| touches(k, &b)) {
            kept.push(b);
        }
    }
    kept.sort_by(|a, b| a.x0.total_cmp(&b.x0));
    Some(kept)
}

/// Whether the boxes share area or a stretch of edge, as pdfplumber's `get_bbox_overlap`.
fn touches(a: &Rect, b: &Rect) -> bool {
    let w = a.x1.min(b.x1) - a.x0.max(b.x0);
    let h = a.y1.min(b.y1) - a.y0.max(b.y0);
    w >= 0.0 && h >= 0.0 && w + h > 0.0
}

/// The sorted, distinct boundaries strictly inside `lo..hi`, with both ends.
fn bounds((lo, hi): (f64, f64), inner: impl Iterator<Item = f64>) -> Vec<f64> {
    let mut b: Vec<f64> = inner
        .filter(|&v| lo < v && v < hi)
        .chain([lo, hi])
        .collect();
    b.sort_by(f64::total_cmp);
    b.dedup();
    b
}

/// The boundaries with every band not `used` merged into its neighbours: the ends stay, and the
/// run of unused bands between two used ones is split at its middle.
fn compress((lo, hi): (f64, f64), bounds: &[f64], used: &[bool]) -> Vec<f64> {
    let mut out = vec![lo];
    let mut prev: Option<usize> = None;
    for k in (0..used.len()).filter(|&k| used[k]) {
        if let Some(p) = prev {
            out.push(bounds[p + 1] * 0.5 + bounds[k] * 0.5);
        }
        prev = Some(k);
    }
    out.push(hi);
    out
}
