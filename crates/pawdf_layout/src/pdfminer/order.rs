//! Reading order: `group_textboxes`, the group sort keys, `IndexAssigner` and `getkey`.

use std::cmp::{Ordering, Reverse};
use std::collections::BinaryHeap;

use pawdf_reader::{Diagnostic, Rect};

use super::blocks::RawBlock;
use super::input::{cmp_f64, py_max, py_min};
use super::lines::{EMPTY, expand, height, width};
use super::plane::{OverBudget, Plane, Work};
use super::{MAX_BLOCKS, Params, limit};
use crate::Orientation;

/// A `group_textboxes` heap entry. pdfminer orders by `(skip_isany, dist, id(obj1),
/// id(obj2))`, so ties fall to memory addresses; the ids here are creation order instead.
#[derive(Debug, Clone, Copy)]
pub(crate) struct HeapEntry {
    pub skip_isany: bool,
    pub dist: f64,
    pub id1: usize,
    pub id2: usize,
}

impl Ord for HeapEntry {
    fn cmp(&self, o: &Self) -> Ordering {
        self.skip_isany
            .cmp(&o.skip_isany)
            .then_with(|| cmp_f64(self.dist, o.dist))
            .then_with(|| self.id1.cmp(&o.id1))
            .then_with(|| self.id2.cmp(&o.id2))
    }
}

impl PartialOrd for HeapEntry {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

impl PartialEq for HeapEntry {
    fn eq(&self, o: &Self) -> bool {
        self.cmp(o) == Ordering::Equal
    }
}

impl Eq for HeapEntry {}

/// A text box (`block` set) or a group of two elements.
struct Elem {
    bbox: Rect,
    vertical: bool,
    children: Vec<usize>,
    block: Option<usize>,
}

fn dist(a: &Rect, b: &Rect) -> f64 {
    let x0 = py_min(a.x0, b.x0);
    let y0 = py_min(a.y0, b.y0);
    let x1 = py_max(a.x1, b.x1);
    let y1 = py_max(a.y1, b.y1);
    (x1 - x0) * (y1 - y0) - width(a) * height(a) - width(b) * height(b)
}

fn union(a: &Rect, b: &Rect) -> Rect {
    Rect {
        x0: py_min(a.x0, b.x0),
        y0: py_min(a.y0, b.y0),
        x1: py_max(a.x1, b.x1),
        y1: py_max(a.y1, b.y1),
    }
}

/// `LTLayoutContainer.group_textboxes`, then the groups' `analyze` sorts and
/// `IndexAssigner`: block indices in reading order.
pub(crate) fn group_textboxes(
    blocks: &[RawBlock],
    bound: Rect,
    flow: f64,
    work: &mut Work,
) -> Result<Vec<usize>, OverBudget> {
    let n = blocks.len();
    if n <= 1 {
        return Ok((0..n).collect());
    }
    work.spend((n as u64).saturating_mul(n as u64) / 2)?;
    let mut elems: Vec<Elem> = blocks
        .iter()
        .enumerate()
        .map(|(i, b)| Elem {
            bbox: b.bbox,
            vertical: b.orientation == Orientation::Vertical,
            children: Vec::new(),
            block: Some(i),
        })
        .collect();
    let mut pairs = Vec::with_capacity(n * (n - 1) / 2);
    for i in 0..n {
        for j in i + 1..n {
            let dist = dist(&elems[i].bbox, &elems[j].bbox);
            pairs.push(Reverse(HeapEntry {
                skip_isany: false,
                dist,
                id1: i,
                id2: j,
            }));
        }
    }
    let mut heap = BinaryHeap::from(pairs);
    let mut plane = Plane::new(bound);
    for (i, e) in elems.iter().enumerate() {
        plane.add(i, e.bbox, work)?;
    }
    let mut done = vec![false; n];
    while let Some(Reverse(entry)) = heap.pop() {
        let HeapEntry {
            skip_isany,
            id1,
            id2,
            ..
        } = entry;
        if done[id1] || done[id2] {
            continue;
        }
        if !skip_isany {
            let area = union(&elems[id1].bbox, &elems[id2].bbox);
            if plane
                .find(area, work)?
                .iter()
                .any(|&o| o != id1 && o != id2)
            {
                heap.push(Reverse(HeapEntry {
                    skip_isany: true,
                    ..entry
                }));
                continue;
            }
        }
        let bbox = expand(&expand(&EMPTY, &elems[id1].bbox), &elems[id2].bbox);
        let group = elems.len();
        elems.push(Elem {
            bbox,
            vertical: elems[id1].vertical || elems[id2].vertical,
            children: vec![id1, id2],
            block: None,
        });
        done.push(false);
        plane.remove(id1, work)?;
        plane.remove(id2, work)?;
        done[id1] = true;
        done[id2] = true;
        // Iterating walks every object ever added, removed ones included.
        work.spend(plane.added() as u64)?;
        let others: Vec<usize> = plane.iter().collect();
        work.spend(others.len() as u64)?;
        for other in others {
            let dist = dist(&bbox, &elems[other].bbox);
            heap.push(Reverse(HeapEntry {
                skip_isany: false,
                dist,
                id1: group,
                id2: other,
            }));
        }
        plane.add(group, bbox, work)?;
    }
    for i in n..elems.len() {
        let mut children = std::mem::take(&mut elems[i].children);
        let key = |c: &usize| {
            let b = &elems[*c].bbox;
            if elems[i].vertical {
                -(1.0 + flow) * (b.x0 + b.x1) - (1.0 - flow) * b.y1
            } else {
                (1.0 - flow) * b.x0 - (1.0 + flow) * (b.y0 + b.y1)
            }
        };
        children.sort_by(|a, b| cmp_f64(key(a), key(b)));
        elems[i].children = children;
    }
    let mut out = Vec::with_capacity(n);
    let mut stack: Vec<usize> = plane.iter().collect();
    stack.reverse();
    while let Some(e) = stack.pop() {
        match elems[e].block {
            Some(b) => out.push(b),
            None => stack.extend(elems[e].children.iter().rev()),
        }
    }
    Ok(out)
}

/// `analyze`'s order when `boxes_flow` is `None`.
pub(crate) fn getkey_order(blocks: &mut [RawBlock]) {
    let key = |b: &RawBlock| match b.orientation {
        Orientation::Vertical => (0, -b.bbox.x1, -b.bbox.y0),
        Orientation::Horizontal => (1, -b.bbox.y0, b.bbox.x0),
    };
    blocks.sort_by(|a, b| {
        let (ka, kb) = (key(a), key(b));
        ka.0.cmp(&kb.0)
            .then_with(|| cmp_f64(ka.1, kb.1))
            .then_with(|| cmp_f64(ka.2, kb.2))
    });
}

/// Orders blocks per `boxes_flow`, falling back to `getkey` order with one `LayoutLimit`
/// diagnostic when a limit is hit.
pub(crate) fn order_blocks(
    blocks: &mut Vec<RawBlock>,
    bound: Rect,
    params: &Params,
    work: &mut Work,
    diags: &mut Vec<Diagnostic>,
) {
    let Some(flow) = params.boxes_flow else {
        getkey_order(blocks);
        return;
    };
    if blocks.len() > MAX_BLOCKS {
        diags.push(limit());
        getkey_order(blocks);
        return;
    }
    match group_textboxes(blocks, bound, flow, work) {
        Ok(order) => {
            let mut taken: Vec<Option<RawBlock>> = blocks.drain(..).map(Some).collect();
            blocks.extend(order.into_iter().filter_map(|i| taken[i].take()));
        }
        Err(OverBudget) => {
            diags.push(limit());
            getkey_order(blocks);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pawdf_reader::DiagnosticKind;

    fn block(x: f64, y: f64) -> RawBlock {
        RawBlock {
            bbox: Rect {
                x0: x,
                y0: y,
                x1: x + 10.,
                y1: y + 10.,
            },
            orientation: Orientation::Horizontal,
            lines: vec![],
        }
    }

    fn e(skip_isany: bool, dist: f64, id1: usize, id2: usize) -> HeapEntry {
        HeapEntry {
            skip_isany,
            dist,
            id1,
            id2,
        }
    }

    #[test]
    fn heap_entry_ties_use_creation_order() {
        assert!(e(false, 1.0, 0, 2) < e(false, 1.0, 1, 2));
        assert!(e(false, 1.0, 1, 2) < e(false, 1.0, 1, 3));
        assert!(e(false, 9.0, 5, 6) < e(true, 1.0, 0, 1));
        assert!(e(false, 1.0, 9, 9) < e(false, f64::NAN, 0, 0));
        assert_eq!(
            e(false, 0.0, 1, 2).cmp(&e(false, -0.0, 1, 2)),
            std::cmp::Ordering::Equal
        );
    }

    #[test]
    fn getkey_puts_vertical_first_then_top_down() {
        let mut v = block(50., 0.);
        v.orientation = Orientation::Vertical;
        let mut blocks = vec![block(0., 0.), block(10., 100.), v, block(0., 100.)];
        getkey_order(&mut blocks);
        let got: Vec<_> = blocks
            .iter()
            .map(|b| (b.orientation, b.bbox.x0, b.bbox.y0))
            .collect();
        assert_eq!(
            got,
            [
                (Orientation::Vertical, 50., 0.),
                (Orientation::Horizontal, 0., 100.),
                (Orientation::Horizontal, 10., 100.),
                (Orientation::Horizontal, 0., 0.),
            ]
        );
    }

    #[test]
    fn budget_exhausted_while_ordering() {
        let mut blocks = vec![block(0., 0.), block(100., 100.), block(0., 100.)];
        let mut diags = Vec::new();
        let bound = Rect {
            x0: 0.,
            y0: 0.,
            x1: 300.,
            y1: 300.,
        };
        order_blocks(
            &mut blocks,
            bound,
            &Params::default(),
            &mut Work::new(2),
            &mut diags,
        );
        let got: Vec<_> = blocks.iter().map(|b| (b.bbox.x0, b.bbox.y0)).collect();
        assert_eq!(got, [(0., 100.), (100., 100.), (0., 0.)]);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].kind, DiagnosticKind::LayoutLimit);
    }

    #[test]
    fn single_block_needs_no_grouping() {
        let mut blocks = vec![block(0., 0.)];
        let mut diags = Vec::new();
        let bound = Rect {
            x0: 0.,
            y0: 0.,
            x1: 300.,
            y1: 300.,
        };
        order_blocks(
            &mut blocks,
            bound,
            &Params::default(),
            &mut Work::new(0),
            &mut diags,
        );
        assert_eq!(blocks.len(), 1);
        assert!(diags.is_empty());
    }
}
