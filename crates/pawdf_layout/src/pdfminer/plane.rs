//! pdfminer's `Plane` (`pdfminer/utils.py`), with a work budget.

use std::collections::{HashMap, HashSet};

use pawdf_reader::Rect;

use super::input::{py_max, py_min};

const GRID: i64 = 50;

/// Work charged per cell [`Plane::add`] touches. A new grid entry costs ~160 bytes against
/// a few bytes for a walk, so this keeps the grid of a page within ~40 MB.
const CELL_COST: u64 = 32;

/// The page's work budget ran out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OverBudget;

/// A per-page work budget, charged before the work is done so that a huge page bound
/// can't allocate first.
#[derive(Debug)]
pub(crate) struct Work {
    left: u64,
}

impl Work {
    pub(crate) fn new(left: u64) -> Work {
        Work { left }
    }

    pub(crate) fn spend(&mut self, n: u64) -> Result<(), OverBudget> {
        match self.left.checked_sub(n) {
            Some(left) => {
                self.left = left;
                Ok(())
            }
            None => {
                self.left = 0;
                Err(OverBudget)
            }
        }
    }
}

/// `drange(v0, v1, 50)` as a half-open range: `int()` truncates, `//` floors.
pub(crate) fn drange(v0: f64, v1: f64) -> (i64, i64) {
    (
        (v0 as i64).div_euclid(GRID),
        ((v1 + GRID as f64) as i64).div_euclid(GRID),
    )
}

type Cells = ((i64, i64), (i64, i64));

fn count((xr, yr): Cells) -> u64 {
    let n = |(lo, hi): (i64, i64)| hi.saturating_sub(lo).max(0) as u64;
    n(xr).saturating_mul(n(yr))
}

/// The cells of a range; nothing at all when either axis is empty, so that an empty x range
/// does not walk the y range for free (`count` charges zero for it).
fn each((xr, yr): Cells) -> impl Iterator<Item = (i64, i64)> {
    let rows = if xr.0 < xr.1 { yr.0..yr.1 } else { 0..0 };
    rows.flat_map(move |y| (xr.0..xr.1).map(move |x| (x, y)))
}

/// Objects on a 50-unit grid, found by area. Objects are ids chosen by the caller.
#[derive(Debug)]
pub(crate) struct Plane {
    bound: Rect,
    seq: Vec<usize>,
    live: HashSet<usize>,
    rects: HashMap<usize, Rect>,
    grid: HashMap<(i64, i64), Vec<usize>>,
}

impl Plane {
    pub(crate) fn new(bound: Rect) -> Plane {
        Plane {
            bound,
            seq: Vec::new(),
            live: HashSet::new(),
            rects: HashMap::new(),
            grid: HashMap::new(),
        }
    }

    /// `_getrange`: `None` for an area entirely outside the bound.
    fn range(&self, r: &Rect) -> Option<Cells> {
        let b = &self.bound;
        if r.x1 <= b.x0 || b.x1 <= r.x0 || r.y1 <= b.y0 || b.y1 <= r.y0 {
            return None;
        }
        let x0 = py_max(b.x0, r.x0);
        let y0 = py_max(b.y0, r.y0);
        let x1 = py_min(b.x1, r.x1);
        let y1 = py_min(b.y1, r.y1);
        Some((drange(x0, x1), drange(y0, y1)))
    }

    pub(crate) fn add(&mut self, id: usize, r: Rect, work: &mut Work) -> Result<(), OverBudget> {
        if let Some(cells) = self.range(&r) {
            work.spend(count(cells).saturating_mul(CELL_COST))?;
            for k in each(cells) {
                self.grid.entry(k).or_default().push(id);
            }
        }
        self.seq.push(id);
        self.live.insert(id);
        self.rects.insert(id, r);
        Ok(())
    }

    pub(crate) fn remove(&mut self, id: usize, work: &mut Work) -> Result<(), OverBudget> {
        if let Some(cells) = self.rects.get(&id).and_then(|r| self.range(r)) {
            work.spend(count(cells))?;
            for k in each(cells) {
                if let Some(v) = self.grid.get_mut(&k) {
                    work.spend(v.len() as u64)?;
                    if let Some(pos) = v.iter().position(|&o| o == id) {
                        v.remove(pos);
                    }
                }
            }
        }
        self.live.remove(&id);
        Ok(())
    }

    /// Objects overlapping `r`, in the order cells first yield them.
    pub(crate) fn find(&self, r: Rect, work: &mut Work) -> Result<Vec<usize>, OverBudget> {
        let mut out = Vec::new();
        let Some(cells) = self.range(&r) else {
            return Ok(out);
        };
        work.spend(count(cells))?;
        let mut done = HashSet::new();
        for k in each(cells) {
            let Some(v) = self.grid.get(&k) else {
                continue;
            };
            work.spend(v.len() as u64)?;
            for &o in v {
                if !done.insert(o) {
                    continue;
                }
                let Some(b) = self.rects.get(&o) else {
                    continue;
                };
                if b.x1 <= r.x0 || r.x1 <= b.x0 || b.y1 <= r.y0 || r.y1 <= b.y0 {
                    continue;
                }
                out.push(o);
            }
        }
        Ok(out)
    }

    /// How many objects were ever added, which is what [`Plane::iter`] walks.
    pub(crate) fn added(&self) -> usize {
        self.seq.len()
    }

    /// Live objects in the order they were added.
    pub(crate) fn iter(&self) -> impl Iterator<Item = usize> + '_ {
        self.seq.iter().copied().filter(|o| self.live.contains(o))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x0: f64, y0: f64, x1: f64, y1: f64) -> Rect {
        Rect { x0, y0, x1, y1 }
    }

    const BOUND: Rect = Rect {
        x0: 0.,
        y0: 0.,
        x1: 300.,
        y1: 300.,
    };

    #[test]
    fn drange_matches_python() {
        assert_eq!(drange(0., 49.9), (0, 1));
        assert_eq!(drange(0., 50.), (0, 2));
        assert_eq!(drange(-60.5, 0.), (-2, 1));
    }

    #[test]
    fn find_returns_first_seen_order() {
        let mut w = Work::new(1000);
        let mut p = Plane::new(BOUND);
        p.add(0, r(60., 0., 70., 10.), &mut w).unwrap();
        p.add(1, r(0., 0., 70., 10.), &mut w).unwrap();
        p.add(2, r(0., 0., 10., 10.), &mut w).unwrap();
        assert_eq!(p.find(r(0., 0., 100., 10.), &mut w).unwrap(), [1, 2, 0]);
        assert_eq!(p.iter().collect::<Vec<_>>(), [0, 1, 2]);
    }

    #[test]
    fn off_page_object_is_iterated_but_not_found() {
        let mut w = Work::new(1000);
        let mut p = Plane::new(BOUND);
        p.add(0, r(-50., 0., -40., 10.), &mut w).unwrap();
        assert!(
            p.find(r(-100., -100., 400., 400.), &mut w)
                .unwrap()
                .is_empty()
        );
        assert_eq!(p.iter().collect::<Vec<_>>(), [0]);
    }

    #[test]
    fn removed_object_is_not_found_or_iterated() {
        let mut w = Work::new(1000);
        let mut p = Plane::new(BOUND);
        p.add(0, r(0., 0., 10., 10.), &mut w).unwrap();
        p.add(1, r(0., 0., 10., 10.), &mut w).unwrap();
        p.remove(0, &mut w).unwrap();
        assert_eq!(p.find(BOUND, &mut w).unwrap(), [1]);
        assert_eq!(p.iter().collect::<Vec<_>>(), [1]);
    }

    #[test]
    fn touching_edges_are_not_found() {
        let mut w = Work::new(1000);
        let mut p = Plane::new(BOUND);
        p.add(0, r(0., 0., 10., 10.), &mut w).unwrap();
        assert!(p.find(r(10., 0., 20., 10.), &mut w).unwrap().is_empty());
    }

    #[test]
    fn empty_x_range_does_not_walk_the_y_range() {
        // Without the early return this walks i64::MAX empty rows and never finishes.
        assert_eq!(each(((5, 0), (0, i64::MAX))).next(), None);
    }

    #[test]
    fn spend_fails_at_the_budget() {
        let mut w = Work::new(3);
        assert!(w.spend(3).is_ok());
        assert!(w.spend(1).is_err());
        let mut p = Plane::new(Rect {
            x0: 0.,
            y0: 0.,
            x1: 1e9,
            y1: 1e9,
        });
        assert!(p.add(0, r(0., 0., 1e9, 1e9), &mut Work::new(1000)).is_err());
    }

    #[test]
    fn page_budget_caps_grid_memory() {
        // 1024 × 1024 cells is well under the page budget counted in cells, but would
        // allocate ~170 MB of grid entries.
        let side = 1023. * GRID as f64;
        let big = r(0., 0., side, side);
        let mut p = Plane::new(big);
        let mut w = Work::new(crate::pdfminer::MAX_WORK);
        assert!(p.add(0, big, &mut w).is_err());
        assert!(p.grid.is_empty());
    }
}
