//! Route 2: tables from ruling lines. Follows the "lines" strategy of pdfplumber's
//! `TableFinder` (`pdfplumber/table.py`) and its default tolerances; results are not meant to
//! match pdfplumber's exactly, as its edges come from pdfminer's path objects.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::collections::hash_map::Entry;

use pawdf_reader::{Matrix, PageContent, Point, Rect, Rotation, Segment};

use super::{Cell, Table, Tables, overlaps, union};
use crate::Error;
use crate::pdfminer::{self, Params};

/// Horizontal × vertical edge pairs checked for crossings per page. Each crossing is kept in a
/// map, so this also bounds memory. A 100 × 100 grid needs about 10^4.
const MAX_PAIRS: u64 = 1 << 20;

/// Cell-search steps allowed per page. A 100 × 100 grid needs about 2 × 10^4.
const MAX_WORK: u64 = 1 << 23;

/// A segment is horizontal (vertical) when its ends differ by at most this in y (x), in points,
/// which absorbs rounding from the transforms.
const AXIS_TOLERANCE: f64 = 0.01;

/// Settings for [`ruled`], with pdfplumber's names and defaults. All are in points.
#[derive(Debug, Clone, PartialEq)]
pub struct RuledParams {
    /// Parallel edges this close are moved to their mean position.
    pub snap_tolerance: f64,
    /// Collinear edges with a gap up to this are joined.
    pub join_tolerance: f64,
    /// Edges shorter than this after joining are dropped.
    pub edge_min_length: f64,
    /// A horizontal and a vertical edge cross when each comes this close to the other.
    pub intersection_tolerance: f64,
}

impl Default for RuledParams {
    fn default() -> Self {
        RuledParams {
            snap_tolerance: 3.0,
            join_tolerance: 3.0,
            edge_min_length: 3.0,
            intersection_tolerance: 3.0,
        }
    }
}

impl RuledParams {
    fn validate(&self) -> Result<(), Error> {
        let checks = [
            ("snap_tolerance", self.snap_tolerance),
            ("join_tolerance", self.join_tolerance),
            ("edge_min_length", self.edge_min_length),
            ("intersection_tolerance", self.intersection_tolerance),
        ];
        match checks.iter().find(|(_, v)| !(v.is_finite() && *v >= 0.0)) {
            Some((name, _)) => Err(Error::InvalidParam { name }),
            None => Ok(()),
        }
    }
}

/// A horizontal edge at y = `pos` from x = `start` to `end`, or a vertical one at x = `pos`.
#[derive(Debug, Clone, Copy)]
struct Edge {
    pos: f64,
    start: f64,
    end: f64,
}

/// The tables drawn with ruling lines on a page, top to bottom, then left to right.
///
/// Every visible painted path gives edges: its horizontal and vertical straight segments,
/// including the sides of filled rectangles, so a thin filled bar acts as a rule and a shaded
/// cell has borders. Curves are skipped; edges are clipped to the path's clip bound. Edges are
/// then snapped, joined and filtered by length, and cells are the smallest rectangles whose four
/// corners are crossings joined by edges. Cells that share a corner form a table; a table of one
/// cell is left out.
///
/// With a `region`, the extent of a table, edges are clipped to it and its border is added, so
/// a table ruled only between rows still gets its frame. The table that starts at the region's
/// top-left corner is returned, even with one cell. Words that overlap the region are kept
/// whole: each of their glyphs goes to the cell containing its centre after the centre is moved
/// inside the table.
///
/// A glyph belongs to the cell containing its box's centre; at a shared edge it goes to the
/// cell to the right or below. Rows and columns come from the cells' distinct edges, so a merged
/// cell gets a span. Boxes are in display space (see [the module](super)); `display` is the
/// page's [`display_matrix`](pawdf_reader::Page::display_matrix). Hidden glyphs are left out.
///
/// A page whose edges need more than a fixed amount of work gets no tables and a `LayoutLimit`
/// diagnostic.
#[allow(clippy::too_many_arguments)]
pub fn ruled(
    content: &PageContent,
    media_box: Rect,
    rotate: Rotation,
    display: &Matrix,
    region: Option<Rect>,
    ruling: &RuledParams,
    params: &Params,
) -> Result<Tables, Error> {
    params.validate()?;
    ruling.validate()?;
    let region = super::region(region)?;
    let (mut h, mut v) = edges(content, display);
    if let Some(r) = region {
        h.retain_mut(|e| clip(e, (r.y0, r.y1), (r.x0, r.x1)));
        v.retain_mut(|e| clip(e, (r.x0, r.x1), (r.y0, r.y1)));
        let edge = |pos, start, end| Edge { pos, start, end };
        h.extend([edge(r.y0, r.x0, r.x1), edge(r.y1, r.x0, r.x1)]);
        v.extend([edge(r.x0, r.y0, r.y1), edge(r.x1, r.y0, r.y1)]);
    }
    let (h, v) = (merge(h, ruling), merge(v, ruling));
    let Some(mut grids) = find_tables(&h, &v, ruling.intersection_tolerance) else {
        return Ok(Tables {
            tables: Vec::new(),
            diagnostics: vec![pdfminer::limit()],
        });
    };
    match region {
        Some(_) => grids.truncate(1),
        None => grids.retain(|g| g.len() > 1),
    }
    if grids.is_empty() {
        return Ok(Tables {
            tables: Vec::new(),
            diagnostics: Vec::new(),
        });
    }
    let centres: Vec<Point> = content
        .glyphs
        .iter()
        .map(|g| pdfminer::centre(display, &g.bbox))
        .collect();
    let mut whole = vec![false; content.glyphs.len()];
    if let Some(r) = region {
        for w in pdfminer::analyze(content, media_box, rotate, params)?
            .blocks
            .iter()
            .flat_map(|b| &b.lines)
            .flat_map(|l| &l.words)
        {
            let b = w
                .glyphs
                .iter()
                .filter_map(|&g| content.glyphs.get(g))
                .map(|g| display.transform_rect(g.bbox))
                .reduce(|a, b| union(&a, &b));
            if b.is_some_and(|b| overlaps(&b, &r)) {
                for &g in &w.glyphs {
                    whole[g] = true;
                }
            }
        }
    }
    let tables: Vec<_> = grids.iter().map(|g| grid(g)).collect();
    let mut sets = Vec::new();
    for t in &tables {
        let first = sets.len();
        sets.resize(first + t.cells.len(), Vec::new());
        // ponytail: every table scans every glyph and every cell, O(glyphs × cells); index
        // the slots if pages with huge ruled tables get slow.
        for (g, p) in centres.iter().enumerate() {
            let inside = t.bbox.x0 <= p.x && p.x < t.bbox.x1 && t.bbox.y0 <= p.y && p.y < t.bbox.y1;
            if !(inside || whole[g]) || !p.x.is_finite() || !p.y.is_finite() {
                continue;
            }
            let slot = |bounds: &[f64], at: f64| {
                bounds
                    .partition_point(|&b| b <= at)
                    .clamp(1, bounds.len() - 1)
                    - 1
            };
            let (row, col) = (slot(&t.ys, p.y), slot(&t.xs, p.x));
            if let Some(i) = t.cells.iter().position(|c| {
                (c.row..c.row + c.row_span).contains(&row)
                    && (c.col..c.col + c.col_span).contains(&col)
            }) {
                sets[first + i].push(g);
            }
        }
    }
    let (layouts, diagnostics) = pdfminer::analyze_sets(content, media_box, rotate, &sets, params);
    let mut layouts = layouts.into_iter();
    let tables = tables
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
    Ok(Tables {
        tables,
        diagnostics,
    })
}

/// Horizontal and vertical edges of the visible paths, in display space.
fn edges(content: &PageContent, display: &Matrix) -> (Vec<Edge>, Vec<Edge>) {
    let (mut h, mut v) = (Vec::new(), Vec::new());
    for p in content.paths.iter().filter(|p| !p.hidden) {
        let clip_box = p.clip.map(|c| display.transform_rect(c));
        if clip_box.is_some_and(|c| !(c.x0 < c.x1 && c.y0 < c.y1)) {
            continue;
        }
        let (mut start, mut last) = (None, None);
        // Filling implicitly closes every open subpath (ISO 32000-2 §8.5.3.3.1).
        let fill_close = p.fill.map(|_| Segment::Close);
        for s in p.segments.iter().copied().chain(fill_close) {
            let seg = match s {
                Segment::MoveTo(q) => {
                    let close = p.fill.and(last.zip(start));
                    start = Some(q);
                    last = Some(q);
                    close
                }
                Segment::LineTo(q) => last.replace(q).map(|a| (a, q)),
                Segment::CurveTo(_, _, q) => {
                    last = Some(q);
                    None
                }
                Segment::Close => {
                    let a = last;
                    last = start;
                    a.zip(start)
                }
            };
            let Some((a, b)) = seg else { continue };
            let (a, b) = (display.apply(a), display.apply(b));
            if ![a.x, a.y, b.x, b.y].iter().all(|c| c.is_finite()) {
                continue;
            }
            let (dx, dy) = ((a.x - b.x).abs(), (a.y - b.y).abs());
            let (mut e, out, pos_range, span_range) = if dy <= AXIS_TOLERANCE && dx > AXIS_TOLERANCE
            {
                let e = Edge {
                    pos: a.y * 0.5 + b.y * 0.5,
                    start: a.x.min(b.x),
                    end: a.x.max(b.x),
                };
                (
                    e,
                    &mut h,
                    clip_box.map(|c| (c.y0, c.y1)),
                    clip_box.map(|c| (c.x0, c.x1)),
                )
            } else if dx <= AXIS_TOLERANCE && dy > AXIS_TOLERANCE {
                let e = Edge {
                    pos: a.x * 0.5 + b.x * 0.5,
                    start: a.y.min(b.y),
                    end: a.y.max(b.y),
                };
                (
                    e,
                    &mut v,
                    clip_box.map(|c| (c.x0, c.x1)),
                    clip_box.map(|c| (c.y0, c.y1)),
                )
            } else {
                continue;
            };
            if let (Some(pr), Some(sr)) = (pos_range, span_range)
                && !clip(&mut e, pr, sr)
            {
                continue;
            }
            out.push(e);
        }
    }
    (h, v)
}

/// Clips `e` to a box given as its range across and along the edge; false when nothing is left.
fn clip(e: &mut Edge, across: (f64, f64), along: (f64, f64)) -> bool {
    e.start = e.start.max(along.0);
    e.end = e.end.min(along.1);
    (across.0..=across.1).contains(&e.pos) && e.start < e.end
}

/// pdfplumber's `merge_edges` then `filter_edges`: snaps, joins, and drops short edges.
fn merge(mut edges: Vec<Edge>, p: &RuledParams) -> Vec<Edge> {
    edges.sort_by(|a, b| a.pos.total_cmp(&b.pos));
    let mut i = 0;
    while i < edges.len() {
        let mut j = i + 1;
        while j < edges.len() && edges[j].pos - edges[j - 1].pos <= p.snap_tolerance {
            j += 1;
        }
        let n = (j - i) as f64;
        // Dividing first keeps the sum finite in most cases, and clamping to the cluster's range
        // catches the rest and the rounding drift that would move equal positions; `+ 0.0` turns
        // -0.0 into 0.0, so positions compare and hash alike.
        let (lo, hi) = (edges[i].pos, edges[j - 1].pos);
        let mean = edges[i..j]
            .iter()
            .map(|e| e.pos / n)
            .sum::<f64>()
            .max(lo)
            .min(hi)
            + 0.0;
        for e in &mut edges[i..j] {
            e.pos = mean;
        }
        i = j;
    }
    edges.sort_by(|a, b| a.pos.total_cmp(&b.pos).then(a.start.total_cmp(&b.start)));
    let mut out: Vec<Edge> = Vec::new();
    for e in edges {
        match out.last_mut() {
            Some(l) if l.pos == e.pos && e.start <= l.end + p.join_tolerance => {
                l.end = l.end.max(e.end);
            }
            _ => out.push(e),
        }
    }
    out.retain(|e| e.end - e.start >= p.edge_min_length);
    out
}

type Key = (u64, u64);

fn key(x: f64, y: f64) -> Key {
    (x.to_bits(), y.to_bits())
}

/// The edges through a crossing: indices of horizontal edges, then of vertical ones.
type Crossings = HashMap<Key, (Vec<usize>, Vec<usize>)>;

/// Whether two sorted index lists share an entry; each comparison is charged to `work`.
fn shares(a: &[usize], b: &[usize], work: &mut u64) -> bool {
    let (mut i, mut j) = (0, 0);
    while i < a.len() && j < b.len() {
        *work += 1;
        match a[i].cmp(&b[j]) {
            Ordering::Less => i += 1,
            Ordering::Greater => j += 1,
            Ordering::Equal => return true,
        }
    }
    false
}

/// Cells as rectangles between crossings (pdfplumber's `intersections_to_cells`), grouped into
/// tables by shared corners and sorted by their top-left cell; `None` past the work limit.
fn find_tables(h: &[Edge], v: &[Edge], tol: f64) -> Option<Vec<Vec<Rect>>> {
    if (h.len() as u64).saturating_mul(v.len() as u64) > MAX_PAIRS {
        return None;
    }
    let mut crossings = Crossings::new();
    for (vi, ve) in v.iter().enumerate() {
        for (hi, he) in h.iter().enumerate() {
            if ve.start <= he.pos + tol
                && ve.end >= he.pos - tol
                && ve.pos >= he.start - tol
                && ve.pos <= he.end + tol
            {
                let c = crossings.entry(key(ve.pos, he.pos)).or_default();
                c.0.push(hi);
                c.1.push(vi);
            }
        }
    }
    // Unjoined collinear edges can meet at one crossing and push the same index many times.
    for (hs, vs) in crossings.values_mut() {
        hs.sort_unstable();
        hs.dedup();
        vs.dedup();
    }
    let mut points: Vec<(f64, f64)> = crossings
        .keys()
        .map(|&(x, y)| (f64::from_bits(x), f64::from_bits(y)))
        .collect();
    points.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1)));
    let mut columns: HashMap<u64, Vec<f64>> = HashMap::new();
    let mut rows: HashMap<u64, Vec<f64>> = HashMap::new();
    for &(x, y) in &points {
        columns.entry(x.to_bits()).or_default().push(y);
        rows.entry(y.to_bits()).or_default().push(x);
    }
    for r in rows.values_mut() {
        r.sort_by(f64::total_cmp);
    }
    let edges_at = |x: f64, y: f64| crossings.get(&key(x, y));
    let mut cells = Vec::new();
    let mut work = 0u64;
    for &(x, y) in &points {
        let (Some(here), Some(col), Some(row)) = (
            edges_at(x, y),
            columns.get(&x.to_bits()),
            rows.get(&y.to_bits()),
        ) else {
            continue;
        };
        let below = &col[col.partition_point(|&b| b <= y)..];
        let right = &row[row.partition_point(|&r| r <= x)..];
        'search: for &by in below {
            work += 1;
            if work > MAX_WORK {
                return None;
            }
            let Some(down) = edges_at(x, by).filter(|d| shares(&here.1, &d.1, &mut work)) else {
                continue;
            };
            for &rx in right {
                work += 1;
                if work > MAX_WORK {
                    return None;
                }
                let Some(across) = edges_at(rx, y).filter(|a| shares(&here.0, &a.0, &mut work))
                else {
                    continue;
                };
                if edges_at(rx, by).is_some_and(|c| {
                    shares(&c.1, &across.1, &mut work) && shares(&c.0, &down.0, &mut work)
                }) {
                    cells.push(Rect {
                        x0: x,
                        y0: y,
                        x1: rx,
                        y1: by,
                    });
                    break 'search;
                }
            }
        }
    }
    Some(group(cells))
}

/// Groups cells that share a corner, as pdfplumber's `cells_to_tables` does, sorted by their
/// top-left corner.
fn group(cells: Vec<Rect>) -> Vec<Vec<Rect>> {
    fn root(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    let mut parent: Vec<usize> = (0..cells.len()).collect();
    let mut owner: HashMap<Key, usize> = HashMap::new();
    for (i, c) in cells.iter().enumerate() {
        for k in [
            key(c.x0, c.y0),
            key(c.x1, c.y0),
            key(c.x0, c.y1),
            key(c.x1, c.y1),
        ] {
            match owner.entry(k) {
                Entry::Occupied(o) => {
                    let (a, b) = (root(&mut parent, *o.get()), root(&mut parent, i));
                    parent[a.max(b)] = a.min(b);
                }
                Entry::Vacant(e) => {
                    e.insert(i);
                }
            }
        }
    }
    let mut tables: Vec<Vec<Rect>> = Vec::new();
    let mut index: HashMap<usize, usize> = HashMap::new();
    for (i, c) in cells.into_iter().enumerate() {
        let r = root(&mut parent, i);
        let t = *index.entry(r).or_insert_with(|| {
            tables.push(Vec::new());
            tables.len() - 1
        });
        tables[t].push(c);
    }
    let top_left = |t: &Vec<Rect>| {
        t.iter().fold((f64::INFINITY, f64::INFINITY), |m, c| {
            if (c.y0, c.x0) < m { (c.y0, c.x0) } else { m }
        })
    };
    tables.sort_by(|a, b| {
        let (a, b) = (top_left(a), top_left(b));
        a.0.total_cmp(&b.0).then(a.1.total_cmp(&b.1))
    });
    tables
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
