//! Table detection. So far only tagged PDFs are read, from their structure tree.

use std::collections::HashMap;
use std::ops::Range;

use pawdf_reader::{
    MarkedContentRef, Matrix, PageContent, Rect, Ref, Rotation, StructElement, StructKid,
    StructTree,
};

use crate::pdfminer::{self, Params};
use crate::{Error, Layout};

/// A table on one page.
#[derive(Debug, Clone, PartialEq)]
pub struct Table {
    /// Union of its cells' boxes, in display space.
    pub bbox: Rect,
    /// Rows of the whole table, including rows on other pages.
    pub rows: usize,
    /// Columns of the whole table: the widest row, counting spans.
    pub cols: usize,
    /// Cells of the rows on this page, row by row.
    pub cells: Vec<Cell>,
}

/// One table cell.
#[derive(Debug, Clone, PartialEq)]
pub struct Cell {
    /// 0-based row in the whole table, so parts of a table split across pages line up.
    pub row: usize,
    /// 0-based column.
    pub col: usize,
    /// Rows it spans, clipped to the table's last row.
    pub row_span: usize,
    /// Columns it spans.
    pub col_span: usize,
    /// Whether it is a header cell (`TH`).
    pub header: bool,
    /// Union of its glyphs' boxes on this page, in display space; `None` when it has none.
    pub bbox: Option<Rect>,
    /// Its text, grouped by [`pdfminer`] with boxes in pdfminer's page space.
    pub layout: Layout,
}

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
/// Boxes are in display space, as for [`pdfminer::analyze_regions`], so a cell box can be
/// passed back to it; `display` is the page's
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
) -> Result<Vec<Table>, Error> {
    params.validate()?;
    if region.is_some_and(|r| ![r.x0, r.y0, r.x1, r.y1].iter().all(|v| v.is_finite())) {
        return Err(Error::InvalidParam { name: "region" });
    }
    let mut sequences: HashMap<(Option<Ref>, u32), Vec<Range<usize>>> = HashMap::new();
    for m in &content.marked_contents {
        if let Some(mcid) = m.mcid {
            sequences
                .entry((m.stream, mcid))
                .or_default()
                .push(m.glyphs.clone());
        }
    }
    let mut tables = Vec::new();
    for e in &tree.roots {
        find_tables(e, &mut tables);
    }
    let parts: Vec<_> = tables
        .iter()
        .filter_map(|t| place(t, page, &sequences))
        .collect();
    let sets: Vec<Vec<usize>> = parts
        .iter()
        .flat_map(|p| p.cells.iter().map(|c| c.glyphs.clone()))
        .collect();
    let mut layouts = pdfminer::analyze_sets(content, media_box, rotate, &sets, params).into_iter();
    let region = region.map(|r| Rect::from_corners(r.x0, r.y0, r.x1, r.y1));
    let mut out = Vec::new();
    for p in parts {
        let cells: Vec<Cell> = p
            .cells
            .into_iter()
            .zip(layouts.by_ref())
            .map(|(c, layout)| Cell {
                row: c.row,
                col: c.col,
                row_span: c.row_span,
                col_span: c.col_span,
                header: c.header,
                bbox: c
                    .glyphs
                    .iter()
                    .filter_map(|&g| content.glyphs.get(g))
                    .filter(|g| !g.hidden)
                    .map(|g| display.transform_rect(g.bbox))
                    .filter(|b| [b.x0, b.y0, b.x1, b.y1].iter().all(|v| v.is_finite()))
                    .reduce(|a, b| union(&a, &b)),
                layout,
            })
            .collect();
        let Some(bbox) = cells
            .iter()
            .filter_map(|c| c.bbox)
            .reduce(|a, b| union(&a, &b))
        else {
            continue;
        };
        if region.is_some_and(|r| !overlaps(&r, &bbox)) {
            continue;
        }
        out.push(Table {
            bbox,
            rows: p.rows,
            cols: p.cols,
            cells,
        });
    }
    Ok(out)
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
    glyphs: Vec<usize>,
}

/// Places the table's cells on its grid and keeps the rows with content on `page`; `None` when
/// no row has.
fn place(
    table: &StructElement,
    page: usize,
    sequences: &HashMap<(Option<Ref>, u32), Vec<Range<usize>>>,
) -> Option<Part> {
    let rows: Vec<&StructElement> = elements(table)
        .flat_map(|e| match e.standard.as_str() {
            "TR" => vec![e],
            "THead" | "TBody" | "TFoot" => elements(e).filter(|r| r.standard == "TR").collect(),
            _ => vec![],
        })
        .collect();
    // Column ranges taken in each row by cells spanning down from rows above.
    // ponytail: a spanning cell marks every row it covers and each lookup scans the row's
    // ranges, O(cells × rows) at worst; use interval trees if huge tagged tables get slow.
    let mut taken: Vec<Vec<Range<usize>>> = vec![Vec::new(); rows.len()];
    let mut cols = 0;
    let mut cells = Vec::new();
    for (r, row) in rows.iter().enumerate() {
        let mut col = 0;
        let mut row_cells = Vec::new();
        let mut here = false;
        for cell in elements(row).filter(|c| matches!(c.standard.as_str(), "TH" | "TD")) {
            while let Some(t) = taken[r].iter().find(|t| t.contains(&col)) {
                col = t.end;
            }
            let row_span = (cell.row_span as usize).clamp(1, rows.len() - r);
            let col_span = cell.col_span.max(1) as usize;
            let end = col.saturating_add(col_span);
            for t in &mut taken[r..r + row_span] {
                t.push(col..end);
            }
            let mut refs = Vec::new();
            marked_refs(cell, &mut refs);
            let refs: Vec<_> = refs.into_iter().filter(|m| m.page == Some(page)).collect();
            here |= !refs.is_empty();
            let glyphs = refs
                .iter()
                .filter_map(|m| sequences.get(&(m.stream, m.mcid)))
                .flatten()
                .flat_map(Range::clone)
                .collect();
            row_cells.push(RawCell {
                row: r,
                col,
                row_span,
                col_span,
                header: cell.standard == "TH",
                glyphs,
            });
            cols = cols.max(end);
            col = end;
        }
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
