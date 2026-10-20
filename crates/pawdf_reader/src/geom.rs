//! Points, rectangles, and affine matrices in PDF conventions (ISO 32000-2 §8.3).

/// A point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Point {
    /// Horizontal coordinate.
    pub x: f64,
    /// Vertical coordinate.
    pub y: f64,
}

/// An axis-aligned rectangle with `x0 <= x1` and `y0 <= y1`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    /// Left.
    pub x0: f64,
    /// Bottom (in PDF user space).
    pub y0: f64,
    /// Right.
    pub x1: f64,
    /// Top (in PDF user space).
    pub y1: f64,
}

impl Rect {
    /// The rectangle spanned by two corners, in any order.
    pub fn from_corners(x0: f64, y0: f64, x1: f64, y1: f64) -> Rect {
        Rect {
            x0: x0.min(x1),
            y0: y0.min(y1),
            x1: x0.max(x1),
            y1: y0.max(y1),
        }
    }

    /// The overlap; a zero-area rectangle when they do not overlap.
    pub fn intersect(&self, other: &Rect) -> Rect {
        let x0 = self.x0.max(other.x0);
        let y0 = self.y0.max(other.y0);
        Rect {
            x0,
            y0,
            x1: self.x1.min(other.x1).max(x0),
            y1: self.y1.min(other.y1).max(y0),
        }
    }

    pub(crate) fn union(&self, other: &Rect) -> Rect {
        Rect {
            x0: self.x0.min(other.x0),
            y0: self.y0.min(other.y0),
            x1: self.x1.max(other.x1),
            y1: self.y1.max(other.y1),
        }
    }
}

/// An affine matrix `[a b c d e f]`, applied to row vectors: `(x, y) ↦ (a·x + c·y + e, b·x + d·y + f)`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Matrix {
    /// Row 1, column 1.
    pub a: f64,
    /// Row 1, column 2.
    pub b: f64,
    /// Row 2, column 1.
    pub c: f64,
    /// Row 2, column 2.
    pub d: f64,
    /// Horizontal translation.
    pub e: f64,
    /// Vertical translation.
    pub f: f64,
}

impl Matrix {
    /// The identity matrix.
    pub const IDENTITY: Matrix = Matrix {
        a: 1.,
        b: 0.,
        c: 0.,
        d: 1.,
        e: 0.,
        f: 0.,
    };

    /// `self × other`: the transform that applies `self`, then `other`.
    pub fn concat(&self, o: &Matrix) -> Matrix {
        Matrix {
            a: self.a * o.a + self.b * o.c,
            b: self.a * o.b + self.b * o.d,
            c: self.c * o.a + self.d * o.c,
            d: self.c * o.b + self.d * o.d,
            e: self.e * o.a + self.f * o.c + o.e,
            f: self.e * o.b + self.f * o.d + o.f,
        }
    }

    /// Transforms a point.
    pub fn apply(&self, p: Point) -> Point {
        Point {
            x: self.a * p.x + self.c * p.y + self.e,
            y: self.b * p.x + self.d * p.y + self.f,
        }
    }

    /// The axis-aligned bound of the transformed corners of `r`.
    pub fn transform_rect(&self, r: Rect) -> Rect {
        let pts = [(r.x0, r.y0), (r.x1, r.y0), (r.x0, r.y1), (r.x1, r.y1)]
            .map(|(x, y)| self.apply(Point { x, y }));
        let (mut out_x0, mut out_y0) = (f64::INFINITY, f64::INFINITY);
        let (mut out_x1, mut out_y1) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
        for p in pts {
            out_x0 = out_x0.min(p.x);
            out_y0 = out_y0.min(p.y);
            out_x1 = out_x1.max(p.x);
            out_y1 = out_y1.max(p.y);
        }
        Rect {
            x0: out_x0,
            y0: out_y0,
            x1: out_x1,
            y1: out_y1,
        }
    }
}

/// Page rotation, clockwise when displayed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rotation {
    /// None.
    R0,
    /// 90°.
    R90,
    /// 180°.
    R180,
    /// 270°.
    R270,
}

impl Rotation {
    /// The rotation in degrees.
    pub fn degrees(self) -> u16 {
        match self {
            Rotation::R0 => 0,
            Rotation::R90 => 90,
            Rotation::R180 => 180,
            Rotation::R270 => 270,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concat_order() {
        let t = Matrix {
            e: 10.,
            ..Matrix::IDENTITY
        };
        let s = Matrix {
            a: 2.,
            d: 2.,
            ..Matrix::IDENTITY
        };
        assert_eq!(
            t.concat(&s).apply(Point { x: 1., y: 1. }),
            Point { x: 22., y: 2. }
        );
    }

    #[test]
    fn rect_intersect() {
        let a = Rect::from_corners(0., 0., 10., 10.);
        assert_eq!(
            a.intersect(&Rect::from_corners(5., 5., 20., 20.)),
            Rect::from_corners(5., 5., 10., 10.)
        );
        assert_eq!(
            a.intersect(&Rect::from_corners(20., 20., 30., 30.)),
            Rect::from_corners(20., 20., 20., 20.)
        );
    }

    #[test]
    fn transform_rect_bounds_rotation() {
        let rot = Matrix {
            a: 0.,
            b: 1.,
            c: -1.,
            d: 0.,
            ..Matrix::IDENTITY
        };
        let r = rot.transform_rect(Rect::from_corners(0., 0., 2., 1.));
        assert_eq!(r, Rect::from_corners(-1., 0., 0., 2.));
    }
}
