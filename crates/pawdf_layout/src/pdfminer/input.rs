//! Glyph selection and pdfminer's page space.

use std::cmp::Ordering;

use pawdf_reader::{Diagnostic, DiagnosticKind, Location, Matrix, PageContent, Rect, Rotation};

/// A glyph as layout sees it: its box in page space and its index in the page content.
#[derive(Debug, Clone)]
pub(crate) struct Char<'a> {
    pub bbox: Rect,
    pub glyph: usize,
    pub text: &'a str,
}

/// pdfminer's `INF`, the starting bound of its containers.
pub(crate) const INF: f64 = 2147483647.0;

/// Python's `min(a, b)`: `a` unless `b` is strictly smaller.
pub(crate) fn py_min(a: f64, b: f64) -> f64 {
    if b < a { b } else { a }
}

/// Python's `max(a, b)`: `a` unless `b` is strictly larger.
pub(crate) fn py_max(a: f64, b: f64) -> f64 {
    if b > a { b } else { a }
}

/// A total order for sort and heap keys: `0.0 == -0.0`, and NaN equals only NaN and ranks
/// after every number. Treating NaN as equal to everything is not a total order, and
/// `sort_by` may panic on one.
pub(crate) fn cmp_f64(a: f64, b: f64) -> Ordering {
    match (a.is_nan(), b.is_nan()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        (false, false) => a.partial_cmp(&b).unwrap_or(Ordering::Equal),
    }
}

/// pdfminer's `apply_matrix_rect`, with the same operand order and `min`/`max` semantics.
pub(crate) fn apply_rect(m: &Matrix, r: Rect) -> Rect {
    let pt = |x: f64, y: f64| (m.a * x + m.c * y + m.e, m.b * x + m.d * y + m.f);
    let (left1, bottom1) = pt(r.x0, r.y0);
    let (right1, bottom2) = pt(r.x1, r.y0);
    let (right2, top1) = pt(r.x1, r.y1);
    let (left2, top2) = pt(r.x0, r.y1);
    let min4 = |a, b, c, d| py_min(py_min(py_min(a, b), c), d);
    let max4 = |a, b, c, d| py_max(py_max(py_max(a, b), c), d);
    Rect {
        x0: min4(left1, left2, right1, right2),
        y0: min4(bottom1, bottom2, top1, top2),
        x1: max4(left1, left2, right1, right2),
        y1: max4(bottom1, bottom2, top1, top2),
    }
}

/// The CTM of `PDFPageInterpreter.process_page` and the page bound of
/// `PDFLayoutAnalyzer.begin_page`.
pub(crate) fn page_space(media: Rect, rotate: Rotation) -> (Matrix, Rect) {
    let Rect { x0, y0, x1, y1 } = media;
    let (a, b, c, d, e, f) = match rotate {
        Rotation::R0 => (1., 0., 0., 1., -x0, -y0),
        Rotation::R90 => (0., -1., 1., 0., -y0, x1),
        Rotation::R180 => (-1., 0., 0., -1., x1, y1),
        Rotation::R270 => (0., 1., -1., 0., y1, -x0),
    };
    let m = Matrix { a, b, c, d, e, f };
    let t = apply_rect(&m, media);
    let bound = Rect {
        x0: 0.,
        y0: 0.,
        x1: (t.x0 - t.x1).abs(),
        y1: (t.y0 - t.y1).abs(),
    };
    (m, bound)
}

fn finite(r: &Rect) -> bool {
    r.x0.is_finite() && r.y0.is_finite() && r.x1.is_finite() && r.y1.is_finite()
}

/// The glyphs layout uses, in content order, with boxes in page space. Hidden glyphs are
/// left out. So are non-finite boxes, reported once: pdfminer accepts them, but NaN makes its
/// ordering meaningless.
pub(crate) fn select<'a>(
    content: &'a PageContent,
    m: &Matrix,
    diags: &mut Vec<Diagnostic>,
) -> Vec<Char<'a>> {
    let mut bad = false;
    let mut out = Vec::new();
    for (i, g) in content.glyphs.iter().enumerate() {
        if g.hidden {
            continue;
        }
        let bbox = apply_rect(m, g.bbox);
        if !finite(&g.bbox) || !finite(&bbox) {
            bad = true;
            continue;
        }
        out.push(Char {
            bbox,
            glyph: i,
            text: &g.text,
        });
    }
    if bad {
        diags.push(Diagnostic {
            kind: DiagnosticKind::BadGlyphBox,
            location: Location::File(0),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use pawdf_reader::Rotation::{R0, R90, R180, R270};
    use std::cmp::Ordering::{Equal, Greater, Less};

    fn rect((x0, y0, x1, y1): (f64, f64, f64, f64)) -> Rect {
        Rect { x0, y0, x1, y1 }
    }

    #[test]
    fn page_space_matches_pdfminer() {
        let media = rect((10., 20., 110., 220.));
        let glyph = rect((10., 20., 20., 30.));
        let cases = [
            (R0, (0., 0., 10., 10.), (0., 0., 100., 200.)),
            (R90, (0., 90., 10., 100.), (0., 0., 200., 100.)),
            (R180, (90., 190., 100., 200.), (0., 0., 100., 200.)),
            (R270, (190., 0., 200., 10.), (0., 0., 200., 100.)),
        ];
        for (rotate, want_glyph, want_bound) in cases {
            let (m, bound) = page_space(media, rotate);
            assert_eq!(apply_rect(&m, glyph), rect(want_glyph), "{rotate:?}");
            assert_eq!(bound, rect(want_bound), "{rotate:?}");
        }
    }

    #[test]
    fn cmp_f64_is_a_total_order() {
        assert_eq!(cmp_f64(0.0, -0.0), Equal);
        assert_eq!(cmp_f64(f64::NAN, f64::NAN), Equal);
        assert_eq!(cmp_f64(f64::NAN, f64::INFINITY), Greater);
        assert_eq!(cmp_f64(1.0, f64::NAN), Less);
        assert_eq!(cmp_f64(1.0, 2.0), Less);
    }

    #[test]
    fn sort_with_nan_keys_does_not_panic() {
        for len in [24, 32, 48, 64, 100] {
            let mut v: Vec<f64> = (0..len)
                .map(|i| {
                    if i % 3 == 0 {
                        f64::NAN
                    } else {
                        ((i * 7919) % 101) as f64
                    }
                })
                .collect();
            v.sort_by(|a, b| cmp_f64(*a, *b));
            let first_nan = v.iter().position(|x| x.is_nan()).unwrap();
            assert!(v[first_nan..].iter().all(|x| x.is_nan()));
            assert!(v[..first_nan].windows(2).all(|w| w[0] <= w[1]));
        }
    }
}
