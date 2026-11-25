//! Route 1: tables from the structure tree of a tagged PDF.

use std::collections::HashMap;
use std::ops::Range;

use pawdf_reader::{
    MarkedContentRef, Matrix, PageContent, Rect, Rotation, StructElement, StructKid, StructTree,
};

use super::{Cell, Table, Tables, finite, overlaps, union};
use crate::Error;
use crate::pdfminer::{self, Params};

/// The tables of page `page` in a tagged PDF, from the `Table`, `TR`, `TH` and `TD` elements of
/// `tree` (§14.8.4), in tree order. A table nested in a cell is returned on its own as
/// well, and its text is also in the enclosing cell.
///
/// Rows are `TR` elements in the table, directly or in `THead`, `TBody` or `TFoot`; cells are
/// the `TH` and `TD` elements of a row. Cells take the next free column, after the slots taken
/// by spanning cells, as in HTML. A cell's content is all marked content under it on this
/// page.
///
/// A table is returned when it has glyphs on this page. Rows with content only on other pages
/// are left out, and row indices count every row, so the parts of a table split across pages
/// can be joined. Marked content with no page, or with no sequence on this page, is ignored.
///
/// Boxes are in display space (see [the module](super)); `display` is the page's
/// [`display_matrix`](pawdf_reader::Page::display_matrix). With a `region`, also in display
/// space with corners in any order, only tables whose box overlaps it are returned. Hidden
/// glyphs are left out.
#[allow(clippy::too_many_arguments)]
pub fn tagged(
    content: &PageContent,
    tree: &StructTree,
    page: usize,
    media_box: Rect,
    rotate: Rotation,
    display: &Matrix,
    region: Option<Rect>,
    params: &Params,
) -> Result<Tables, Error> {
    params.validate()?;
    let region = super::region(region)?;
    let mut sequences: Sequences = HashMap::new();
    for m in &content.marked_contents {
        if let Some(mcid) = m.mcid {
            sequences
                .entry((m.stream.map(|r| r.num), mcid))
                .or_default()
                .push(m.glyphs.clone());
        }
    }
    let mut tables = Vec::new();
    for e in &tree.roots {
        find_tables(e, &mut tables);
    }
    let mut kept = Vec::new();
    for p in tables.iter().filter_map(|t| place(t, page, &sequences)) {
        let boxes: Vec<_> = p
            .cells
            .iter()
            .map(|c| cell_box(content, display, &c.glyphs))
            .collect();
        let Some(bbox) = boxes.iter().flatten().copied().reduce(|a, b| union(&a, &b)) else {
            continue;
        };
        if region.is_some_and(|r| !overlaps(&r, &bbox)) {
            continue;
        }
        kept.push((p, boxes, bbox));
    }
    if kept.is_empty() {
        return Ok(Tables {
            tables: Vec::new(),
            diagnostics: Vec::new(),
        });
    }
    let sets: Vec<Vec<usize>> = kept
        .iter()
        .flat_map(|(p, _, _)| p.cells.iter())
        .map(|c| c.glyphs.iter().flat_map(Range::clone).collect())
        .collect();
    let (layouts, diagnostics) = pdfminer::analyze_sets(content, media_box, rotate, &sets, params);
    let mut layouts = layouts.into_iter();
    let mut out = Vec::new();
    for (p, boxes, bbox) in kept {
        let cells = p
            .cells
            .into_iter()
            .zip(boxes)
            .zip(layouts.by_ref())
            .map(|((c, bbox), layout)| Cell {
                row: c.row,
                col: c.col,
                row_span: c.row_span,
                col_span: c.col_span,
                header: c.header,
                bbox,
                layout,
            })
            .collect();
        out.push(Table {
            bbox,
            rows: p.rows,
            cols: p.cols,
            cells,
        });
    }
    Ok(Tables {
        tables: out,
        diagnostics,
    })
}

/// Every `Table` element at or under `e`, in tree order. Recursion is bounded by the reader's
/// depth limit.
fn find_tables<'a>(e: &'a StructElement, out: &mut Vec<&'a StructElement>) {
    if e.standard == "Table" {
        out.push(e);
    }
    for k in elements(e) {
        find_tables(k, out);
    }
}

fn elements(e: &StructElement) -> impl Iterator<Item = &StructElement> {
    e.children.iter().filter_map(|k| match k {
        StructKid::Element(e) => Some(e),
        _ => None,
    })
}

fn marked_refs<'a>(e: &'a StructElement, out: &mut Vec<&'a MarkedContentRef>) {
    for k in &e.children {
        match k {
            StructKid::Element(e) => marked_refs(e, out),
            StructKid::MarkedContent(m) => out.push(m),
            StructKid::Object(_) => {}
        }
    }
}

struct Part {
    rows: usize,
    cols: usize,
    cells: Vec<RawCell>,
}

struct RawCell {
    row: usize,
    col: usize,
    row_span: usize,
    col_span: usize,
    header: bool,
    /// Glyph index ranges, sorted and merged, so content referenced twice is counted once.
    glyphs: Vec<Range<usize>>,
}

/// Glyph ranges of the marked-content sequences on a page, by `(stream object number, MCID)`.
/// Generations are ignored, as `File::resolve` does.
type Sequences = HashMap<(Option<u32>, u32), Vec<Range<usize>>>;

/// Places the table's cells on its grid and keeps the rows with content on `page`; `None` when
/// no row has.
fn place(table: &StructElement, page: usize, sequences: &Sequences) -> Option<Part> {
    let rows: Vec<&StructElement> = elements(table)
        .flat_map(|e| match e.standard.as_str() {
            "TR" => vec![e],
            "THead" | "TBody" | "TFoot" => elements(e).filter(|r| r.standard == "TR").collect(),
            _ => vec![],
        })
        .collect();
    // Cells spanning down from rows above: the row they end before, and their columns.
    // ponytail: each placement scans the open spans, O(cells × spans) at worst; use an
    // interval tree if huge tagged tables get slow.
    let mut spanning: Vec<(usize, Range<usize>)> = Vec::new();
    let mut cols = 0;
    let mut cells = Vec::new();
    for (r, row) in rows.iter().enumerate() {
        spanning.retain(|(until, _)| *until > r);
        let mut col = 0;
        let mut row_cells = Vec::new();
        let mut opened = Vec::new();
        let mut here = false;
        for cell in elements(row).filter(|c| matches!(c.standard.as_str(), "TH" | "TD")) {
            while let Some((_, t)) = spanning.iter().find(|(_, t)| t.contains(&col)) {
                col = t.end;
            }
            let row_span = (cell.row_span as usize).clamp(1, rows.len() - r);
            let col_span = cell.col_span.max(1) as usize;
            let end = col.saturating_add(col_span);
            if row_span > 1 {
                opened.push((r + row_span, col..end));
            }
            let mut refs = Vec::new();
            marked_refs(cell, &mut refs);
            let ranges: Vec<_> = refs
                .into_iter()
                .filter(|m| m.page == Some(page))
                .filter_map(|m| sequences.get(&(m.stream.map(|r| r.num), m.mcid)))
                .flatten()
                .cloned()
                .collect();
            here |= !ranges.is_empty();
            row_cells.push(RawCell {
                row: r,
                col,
                row_span,
                col_span,
                header: cell.standard == "TH",
                glyphs: merge(ranges),
            });
            cols = cols.max(end);
            col = end;
        }
        spanning.extend(opened);
        if here {
            cells.extend(row_cells);
        }
    }
    (!cells.is_empty()).then_some(Part {
        rows: rows.len(),
        cols,
        cells,
    })
}

fn merge(mut ranges: Vec<Range<usize>>) -> Vec<Range<usize>> {
    ranges.retain(|r| !r.is_empty());
    ranges.sort_unstable_by_key(|r| (r.start, r.end));
    let mut out: Vec<Range<usize>> = Vec::new();
    for r in ranges {
        match out.last_mut() {
            Some(last) if r.start <= last.end => last.end = last.end.max(r.end),
            _ => out.push(r),
        }
    }
    out
}

/// Union of the boxes of a cell's visible glyphs with finite corners, in display space.
fn cell_box(content: &PageContent, display: &Matrix, glyphs: &[Range<usize>]) -> Option<Rect> {
    glyphs
        .iter()
        .flat_map(Range::clone)
        .map_while(|g| content.glyphs.get(g))
        .filter(|g| !g.hidden)
        .map(|g| display.transform_rect(g.bbox))
        .filter(finite)
        .reduce(|a, b| union(&a, &b))
}
