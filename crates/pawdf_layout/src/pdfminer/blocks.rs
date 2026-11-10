//! Blocks: `group_textlines` and `LTTextLine{Horizontal,Vertical}.find_neighbors`.

use std::collections::{HashMap, HashSet};

use pawdf_reader::Rect;

use super::Params;
use super::input::cmp_f64;
use super::lines::{EMPTY, RawLine, expand, height, width};
use super::plane::{OverBudget, Plane, Work};
use crate::Orientation;

#[cfg(test)]
use super::lines::is_empty_line;

/// `LTTextBoxHorizontal` or `LTTextBoxVertical`: indices into the line slice, sorted as its
/// `analyze` sorts them.
#[derive(Debug, Clone)]
pub(crate) struct RawBlock {
    pub bbox: Rect,
    pub orientation: Orientation,
    pub lines: Vec<usize>,
}

fn find_neighbors(
    lines: &[RawLine],
    i: usize,
    plane: &Plane,
    ratio: f64,
    work: &mut Work,
) -> Result<Vec<usize>, OverBudget> {
    let s = &lines[i];
    let b = &s.bbox;
    let near = |a: f64, c: f64, d: f64| (a - c).abs() <= d;
    let found = match s.orientation {
        Orientation::Horizontal => {
            let d = ratio * height(b);
            let objs = plane.find(
                Rect {
                    x0: b.x0,
                    y0: b.y0 - d,
                    x1: b.x1,
                    y1: b.y1 + d,
                },
                work,
            )?;
            objs.into_iter()
                .filter(|&o| {
                    let (l, ob) = (&lines[o], &lines[o].bbox);
                    l.orientation == Orientation::Horizontal
                        && near(height(ob), height(b), d)
                        && (near(ob.x0, b.x0, d)
                            || near(ob.x1, b.x1, d)
                            || near((ob.x0 + ob.x1) / 2.0, (b.x0 + b.x1) / 2.0, d))
                })
                .collect()
        }
        Orientation::Vertical => {
            let d = ratio * width(b);
            let objs = plane.find(
                Rect {
                    x0: b.x0 - d,
                    y0: b.y0,
                    x1: b.x1 + d,
                    y1: b.y1,
                },
                work,
            )?;
            objs.into_iter()
                .filter(|&o| {
                    let (l, ob) = (&lines[o], &lines[o].bbox);
                    l.orientation == Orientation::Vertical
                        && near(width(ob), width(b), d)
                        && (near(ob.y0, b.y0, d)
                            || near(ob.y1, b.y1, d)
                            || near((ob.y0 + ob.y1) / 2.0, (b.y0 + b.y1) / 2.0, d))
                })
                .collect()
        }
    };
    Ok(found)
}

/// `LTLayoutContainer.group_textlines`, for lines that are not empty.
pub(crate) fn group_textlines(
    lines: &[RawLine],
    bound: Rect,
    p: &Params,
    work: &mut Work,
) -> Result<Vec<RawBlock>, OverBudget> {
    let mut plane = Plane::new(bound);
    for (i, l) in lines.iter().enumerate() {
        plane.add(i, l.bbox, work)?;
    }
    let mut boxes: HashMap<usize, usize> = HashMap::new();
    let mut members_of: Vec<Vec<usize>> = Vec::new();
    let mut kinds: Vec<Orientation> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let mut members = vec![i];
        for n in find_neighbors(lines, i, &plane, p.line_margin, work)? {
            members.push(n);
            if let Some(b) = boxes.remove(&n) {
                // A popped box's members all join the new box, so nothing refers to it any
                // more; a second pop of it in this loop would only add duplicates.
                let old = std::mem::take(&mut members_of[b]);
                work.spend(old.len() as u64)?;
                members.extend(old);
            }
        }
        let id = members_of.len();
        let mut seen = HashSet::new();
        members.retain(|m| seen.insert(*m));
        for &m in &members {
            boxes.insert(m, id);
        }
        members_of.push(members);
        kinds.push(line.orientation);
    }
    let mut done = HashSet::new();
    let mut out = Vec::new();
    for i in 0..lines.len() {
        let Some(&b) = boxes.get(&i) else {
            continue;
        };
        if !done.insert(b) {
            continue;
        }
        let mut members = std::mem::take(&mut members_of[b]);
        let bbox = members
            .iter()
            .fold(EMPTY, |acc, &m| expand(&acc, &lines[m].bbox));
        if width(&bbox) <= 0.0 || height(&bbox) <= 0.0 {
            continue;
        }
        let orientation = kinds[b];
        members.sort_by(|&a, &c| match orientation {
            Orientation::Horizontal => cmp_f64(-lines[a].bbox.y1, -lines[c].bbox.y1),
            Orientation::Vertical => cmp_f64(-lines[a].bbox.x1, -lines[c].bbox.x1),
        });
        out.push(RawBlock {
            bbox,
            orientation,
            lines: members,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pdfminer::input::Char;
    use crate::pdfminer::lines::{group_objects, line_text};

    const BOUND: Rect = Rect {
        x0: 0.,
        y0: 0.,
        x1: 300.,
        y1: 300.,
    };

    fn ch(text: &'static str, x0: f64, y0: f64, x1: f64, y1: f64) -> Char<'static> {
        Char {
            bbox: Rect { x0, y0, x1, y1 },
            glyph: 0,
            text,
        }
    }

    fn g(text: &'static str, x: f64, y: f64) -> Char<'static> {
        ch(text, x, y, x + 10., y + 10.)
    }

    /// (bbox, line texts) per block, as pdfminer 20260107 reports them.
    fn run(chars: &[Char], bound: Rect) -> Vec<(Rect, Vec<String>)> {
        let p = Params::default();
        let lines: Vec<_> = group_objects(chars, &p)
            .into_iter()
            .filter(|l| !is_empty_line(l, chars))
            .collect();
        let blocks = group_textlines(&lines, bound, &p, &mut Work::new(1 << 20)).unwrap();
        blocks
            .iter()
            .map(|b| {
                (
                    b.bbox,
                    b.lines
                        .iter()
                        .map(|&i| line_text(&lines[i], chars))
                        .collect(),
                )
            })
            .collect()
    }

    fn close_lines() -> Vec<Char<'static>> {
        vec![
            g("a", 0., 20.),
            g("b", 11., 20.),
            g("c", 0., 8.),
            g("d", 11., 8.),
        ]
    }

    #[test]
    fn close_lines_merge() {
        assert_eq!(
            run(&close_lines(), BOUND),
            [(
                Rect {
                    x0: 0.,
                    y0: 8.,
                    x1: 21.,
                    y1: 30.
                },
                vec!["ab".to_string(), "cd".into()]
            )]
        );
    }

    #[test]
    fn off_page_lines_stay_apart() {
        let b = run(&[g("a", -50., 20.), g("b", -50., 8.)], BOUND);
        assert_eq!(b.len(), 2);
        assert_eq!(b[0].1, ["a"]);
    }

    #[test]
    fn zero_width_lines_are_dropped() {
        assert!(run(&[ch("z", 0., 0., 0., 10.), ch("y", 0., 0., 0., 10.)], BOUND).is_empty());
    }

    #[test]
    fn whitespace_lines_are_dropped() {
        assert!(run(&[g(" ", 0., 0.), g(" ", 11., 0.)], BOUND).is_empty());
    }

    #[test]
    fn zero_area_page_bound_separates_all_lines() {
        let zero = Rect {
            x0: 0.,
            y0: 0.,
            x1: 0.,
            y1: 0.,
        };
        assert_eq!(run(&close_lines(), zero).len(), 2);
    }

    #[test]
    fn different_heights_do_not_merge() {
        let chars = [ch("a", 0., 45., 21., 55.), ch("b", 0., 12., 21., 42.)];
        assert_eq!(run(&chars, BOUND).len(), 2);
    }

    #[test]
    fn block_sorts_lines_top_down() {
        let chars = [
            g("c", 0., 8.),
            g("d", 11., 8.),
            g("a", 0., 20.),
            g("b", 11., 20.),
        ];
        assert_eq!(run(&chars, BOUND)[0].1, ["ab", "cd"]);
    }
}
