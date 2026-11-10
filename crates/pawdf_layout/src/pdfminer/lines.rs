//! Lines and words: `group_objects` and `LTTextLine{Horizontal,Vertical}.add`.

use pawdf_reader::Rect;

use super::Params;
use super::input::{Char, INF, py_max, py_min};
use crate::{Line, Orientation, Word};

/// A line's element: a glyph (index into the `Char` slice) or a space pdfminer inserted.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Item {
    Char(usize),
    Space,
}

/// `LTTextLineHorizontal` or `LTTextLineVertical`.
#[derive(Debug, Clone)]
pub(crate) struct RawLine {
    pub bbox: Rect,
    pub orientation: Orientation,
    pub items: Vec<Item>,
    /// `_x1` for horizontal lines, `_y0` for vertical ones.
    edge: f64,
}

pub(crate) fn width(r: &Rect) -> f64 {
    r.x1 - r.x0
}

pub(crate) fn height(r: &Rect) -> f64 {
    r.y1 - r.y0
}

/// `LTExpandableContainer.add`'s box update.
pub(crate) fn expand(acc: &Rect, r: &Rect) -> Rect {
    Rect {
        x0: py_min(acc.x0, r.x0),
        y0: py_min(acc.y0, r.y0),
        x1: py_max(acc.x1, r.x1),
        y1: py_max(acc.y1, r.y1),
    }
}

/// The starting box of `LTExpandableContainer`.
pub(crate) const EMPTY: Rect = Rect {
    x0: INF,
    y0: INF,
    x1: -INF,
    y1: -INF,
};

impl RawLine {
    fn new(orientation: Orientation) -> RawLine {
        RawLine {
            bbox: EMPTY,
            orientation,
            items: Vec::new(),
            edge: match orientation {
                Orientation::Horizontal => INF,
                Orientation::Vertical => -INF,
            },
        }
    }

    fn add(&mut self, chars: &[Char], i: usize, word_margin: f64) {
        let b = &chars[i].bbox;
        if word_margin != 0.0 {
            let margin = word_margin * py_max(width(b), height(b));
            let gap = match self.orientation {
                Orientation::Horizontal => self.edge < b.x0 - margin,
                Orientation::Vertical => b.y1 + margin < self.edge,
            };
            if gap {
                self.items.push(Item::Space);
            }
        }
        self.edge = match self.orientation {
            Orientation::Horizontal => b.x1,
            Orientation::Vertical => b.y0,
        };
        self.items.push(Item::Char(i));
        self.bbox = expand(&self.bbox, b);
    }

    fn chars(&self) -> impl Iterator<Item = usize> + '_ {
        self.items.iter().filter_map(|i| match i {
            Item::Char(c) => Some(*c),
            Item::Space => None,
        })
    }
}

fn is_voverlap(a: &Rect, b: &Rect) -> bool {
    b.y0 <= a.y1 && a.y0 <= b.y1
}

fn is_hoverlap(a: &Rect, b: &Rect) -> bool {
    b.x0 <= a.x1 && a.x0 <= b.x1
}

fn gap(overlap: bool, a0: f64, a1: f64, b0: f64, b1: f64) -> f64 {
    if overlap {
        py_min((a0 - b1).abs(), (a1 - b0).abs())
    } else {
        0.0
    }
}

/// `LTComponent.voverlap`, `hoverlap`, `vdistance` and `hdistance`.
fn voverlap(a: &Rect, b: &Rect) -> f64 {
    gap(is_voverlap(a, b), a.y0, a.y1, b.y0, b.y1)
}

fn hoverlap(a: &Rect, b: &Rect) -> f64 {
    gap(is_hoverlap(a, b), a.x0, a.x1, b.x0, b.x1)
}

fn vdistance(a: &Rect, b: &Rect) -> f64 {
    gap(!is_voverlap(a, b), a.y0, a.y1, b.y0, b.y1)
}

fn hdistance(a: &Rect, b: &Rect) -> f64 {
    gap(!is_hoverlap(a, b), a.x0, a.x1, b.x0, b.x1)
}

/// `LTLayoutContainer.group_objects`, branch for branch.
pub(crate) fn group_objects(chars: &[Char], p: &Params) -> Vec<RawLine> {
    let mut out = Vec::new();
    if chars.is_empty() {
        return out;
    }
    let mut line: Option<RawLine> = None;
    for i1 in 1..chars.len() {
        let i0 = i1 - 1;
        let (o0, o1) = (&chars[i0].bbox, &chars[i1].bbox);
        let halign = is_voverlap(o0, o1)
            && py_min(height(o0), height(o1)) * p.line_overlap < voverlap(o0, o1)
            && hdistance(o0, o1) < py_max(width(o0), width(o1)) * p.char_margin;
        let valign = p.detect_vertical
            && is_hoverlap(o0, o1)
            && py_min(width(o0), width(o1)) * p.line_overlap < hoverlap(o0, o1)
            && vdistance(o0, o1) < py_max(height(o0), height(o1)) * p.char_margin;
        let open = line.as_ref().map(|l| l.orientation);
        if (halign && open == Some(Orientation::Horizontal))
            || (valign && open == Some(Orientation::Vertical))
        {
            if let Some(l) = line.as_mut() {
                l.add(chars, i1, p.word_margin);
            }
        } else if let Some(l) = line.take() {
            out.push(l);
        } else if valign && !halign {
            let mut l = RawLine::new(Orientation::Vertical);
            l.add(chars, i0, p.word_margin);
            l.add(chars, i1, p.word_margin);
            line = Some(l);
        } else if halign && !valign {
            let mut l = RawLine::new(Orientation::Horizontal);
            l.add(chars, i0, p.word_margin);
            l.add(chars, i1, p.word_margin);
            line = Some(l);
        } else {
            let mut l = RawLine::new(Orientation::Horizontal);
            l.add(chars, i0, p.word_margin);
            out.push(l);
        }
    }
    out.push(line.unwrap_or_else(|| {
        let mut l = RawLine::new(Orientation::Horizontal);
        l.add(chars, chars.len() - 1, p.word_margin);
        l
    }));
    out
}

/// Python's `str.isspace`, which also counts U+001C..U+001F, unlike `char::is_whitespace`.
pub(crate) fn is_py_space(s: &str) -> bool {
    !s.is_empty()
        && s.chars().all(|c| {
            matches!(
                c,
                '\u{9}'..='\u{d}'
                    | '\u{1c}'..='\u{20}'
                    | '\u{85}'
                    | '\u{a0}'
                    | '\u{1680}'
                    | '\u{2000}'..='\u{200a}'
                    | '\u{2028}'
                    | '\u{2029}'
                    | '\u{202f}'
                    | '\u{205f}'
                    | '\u{3000}'
            )
        })
}

/// `get_text()` before `analyze` appends the trailing newline.
pub(crate) fn line_text(line: &RawLine, chars: &[Char]) -> String {
    line.items
        .iter()
        .map(|i| match i {
            Item::Char(c) => chars[*c].text,
            Item::Space => " ",
        })
        .collect()
}

/// `LTTextLine.is_empty`: no area, or only whitespace.
pub(crate) fn is_empty_line(line: &RawLine, chars: &[Char]) -> bool {
    width(&line.bbox) <= 0.0 || height(&line.bbox) <= 0.0 || is_py_space(&line_text(line, chars))
}

/// Splits a line into words at inserted spaces and whitespace glyphs.
pub(crate) fn finish_line(line: &RawLine, chars: &[Char]) -> Line {
    let mut words = Vec::new();
    let mut word: Option<Word> = None;
    for item in &line.items {
        let c = match item {
            Item::Char(c) if !is_py_space(chars[*c].text) => &chars[*c],
            _ => {
                words.extend(word.take());
                continue;
            }
        };
        match word.as_mut() {
            Some(w) => {
                w.bbox = expand(&w.bbox, &c.bbox);
                w.text.push_str(c.text);
                w.glyphs.push(c.glyph);
            }
            None => {
                word = Some(Word {
                    bbox: c.bbox,
                    text: c.text.to_string(),
                    glyphs: vec![c.glyph],
                })
            }
        }
    }
    words.extend(word);
    Line {
        bbox: line.bbox,
        orientation: line.orientation,
        text: line_text(line, chars),
        words,
        glyphs: line.chars().map(|c| chars[c].glyph).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn g(text: &'static str, x: f64, y: f64) -> Char<'static> {
        sized(text, x, y, 10.)
    }

    fn sized(text: &'static str, x: f64, y: f64, w: f64) -> Char<'static> {
        Char {
            bbox: Rect {
                x0: x,
                y0: y,
                x1: x + w,
                y1: y + 10.,
            },
            glyph: 0,
            text,
        }
    }

    fn indexed(mut chars: Vec<Char<'static>>) -> Vec<Char<'static>> {
        for (i, c) in chars.iter_mut().enumerate() {
            c.glyph = i;
        }
        chars
    }

    fn lines(chars: &[Char], params: &Params) -> Vec<Line> {
        group_objects(chars, params)
            .iter()
            .map(|l| finish_line(l, chars))
            .collect()
    }

    fn texts(chars: Vec<Char<'static>>, params: &Params) -> Vec<String> {
        lines(&indexed(chars), params)
            .into_iter()
            .map(|l| l.text)
            .collect()
    }

    fn words(l: &Line) -> Vec<&str> {
        l.words.iter().map(|w| w.text.as_str()).collect()
    }

    fn d() -> Params {
        Params::default()
    }

    #[test]
    fn adjacent_glyphs_share_a_line() {
        let ls = group_objects(&[g("a", 0., 0.), g("b", 11., 0.)], &d());
        assert_eq!(ls.len(), 1);
        assert!(ls[0].items.iter().all(|i| matches!(i, Item::Char(_))));
        assert_eq!(texts(vec![g("a", 0., 0.), g("b", 11., 0.)], &d()), ["ab"]);
    }

    #[test]
    fn word_gap_inserts_a_space() {
        let chars = indexed(vec![g("a", 0., 0.), g("b", 11., 0.), g("c", 22.5, 0.)]);
        let ls = lines(&chars, &d());
        assert_eq!(ls[0].text, "ab c");
        assert_eq!(words(&ls[0]), ["ab", "c"]);
    }

    #[test]
    fn word_margin_boundary_is_strict() {
        // The running edge 10 equals x0 - margin = 11 - 1 exactly: no space.
        assert_eq!(texts(vec![g("a", 0., 0.), g("b", 11., 0.)], &d()), ["ab"]);
    }

    #[test]
    fn word_margin_zero_inserts_none() {
        let p = Params {
            word_margin: 0.,
            ..d()
        };
        assert_eq!(texts(vec![g("a", 0., 0.), g("b", 25., 0.)], &p), ["ab"]);
    }

    #[test]
    fn char_margin_boundary_is_strict() {
        assert_eq!(
            texts(vec![g("a", 0., 0.), g("b", 30., 0.)], &d()),
            ["a", "b"]
        );
    }

    #[test]
    fn line_overlap_boundary_is_strict() {
        assert_eq!(
            texts(vec![g("a", 0., 0.), g("b", 11., 5.)], &d()),
            ["a", "b"]
        );
    }

    #[test]
    fn ending_a_line_only_closes_it() {
        let chars = vec![
            g("a", 0., 0.),
            g("b", 11., 0.),
            g("c", 100., 0.),
            g("d", 111., 0.),
        ];
        assert_eq!(texts(chars, &d()), ["ab", "cd"]);
    }

    #[test]
    fn single_far_glyphs_get_own_lines() {
        let chars = vec![g("a", 0., 0.), g("b", 100., 0.), g("c", 200., 0.)];
        assert_eq!(texts(chars, &d()), ["a", "b", "c"]);
    }

    #[test]
    fn vertical_stack_needs_detect_vertical() {
        let chars = indexed(vec![g("a", 0., 20.), g("b", 0., 9.)]);
        assert_eq!(lines(&chars, &d()).len(), 2);
        let ls = lines(
            &chars,
            &Params {
                detect_vertical: true,
                ..d()
            },
        );
        assert_eq!(ls.len(), 1);
        assert_eq!(ls[0].orientation, Orientation::Vertical);
        assert_eq!(ls[0].text, "ab");
        assert_eq!(
            ls[0].bbox,
            Rect {
                x0: 0.,
                y0: 9.,
                x1: 10.,
                y1: 30.
            }
        );
    }

    #[test]
    fn vertical_gap_inserts_a_space() {
        let chars = indexed(vec![g("a", 0., 40.), g("b", 0., 28.5)]);
        let ls = lines(
            &chars,
            &Params {
                detect_vertical: true,
                ..d()
            },
        );
        assert_eq!(ls[0].text, "a b");
    }

    #[test]
    fn whitespace_glyph_breaks_words_and_stays_in_line() {
        let chars = indexed(vec![g("a", 0., 0.), g(" ", 11., 0.), g("b", 22., 0.)]);
        let ls = lines(&chars, &d());
        assert_eq!(words(&ls[0]), ["a", "b"]);
        assert_eq!(ls[0].glyphs, [0, 1, 2]);
        assert_eq!(ls[0].words[1].glyphs, [2]);
    }

    #[test]
    fn py_space_differs_from_rust() {
        for c in [
            '\u{1c}', '\u{1d}', '\u{1e}', '\u{1f}', '\u{a0}', '\u{3000}', '\u{85}',
        ] {
            assert!(is_py_space(&c.to_string()), "{c:?}");
        }
        assert!(!is_py_space(""));
        assert!(!is_py_space(" a"));
        assert!(!is_py_space("\u{200b}"));
        assert!(is_py_space(" \t"));
    }

    #[test]
    fn ligature_is_one_glyph() {
        let chars = indexed(vec![g("fi", 0., 0.), g("x", 11., 0.)]);
        let ls = lines(&chars, &d());
        assert_eq!(ls[0].words[0].text, "fix");
        assert_eq!(ls[0].words[0].glyphs, [0, 1]);
    }

    #[test]
    fn empty_text_glyph_is_not_whitespace() {
        let chars = indexed(vec![g("a", 0., 0.), g("", 11., 0.), g("b", 22., 0.)]);
        let ls = lines(&chars, &d());
        assert_eq!(words(&ls[0]), ["ab"]);
        assert_eq!(ls[0].words[0].glyphs, [0, 1, 2]);
    }

    #[test]
    fn zero_width_glyph_stays_in_a_normal_line() {
        let chars = indexed(vec![
            g("a", 0., 0.),
            sized("\u{301}", 10., 0., 0.),
            g("b", 11., 0.),
        ]);
        let ls = lines(&chars, &d());
        assert_eq!(ls.len(), 1);
        assert_eq!(ls[0].glyphs, [0, 1, 2]);
    }

    #[test]
    fn word_bbox_is_union_of_its_glyphs() {
        let chars = indexed(vec![g("a", 0., 0.), g("b", 11., 2.)]);
        let ls = lines(&chars, &d());
        assert_eq!(
            ls[0].words[0].bbox,
            Rect {
                x0: 0.,
                y0: 0.,
                x1: 21.,
                y1: 12.
            }
        );
    }

    #[test]
    fn empty_lines() {
        let chars = indexed(vec![
            g(" ", 0., 0.),
            g(" ", 11., 0.),
            sized("a", 50., 0., 0.),
        ]);
        let ls = group_objects(&chars, &d());
        assert!(ls.iter().all(|l| is_empty_line(l, &chars)));
        let one = [g("a", 0., 0.)];
        assert!(!is_empty_line(&group_objects(&one, &d())[0], &one));
    }
}
