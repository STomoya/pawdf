use std::sync::Arc;

use pawdf_core::{Object, Ref};

use crate::content::color::Space;
use crate::font::Font;
use crate::geom::Matrix;
use crate::model::{Color, RenderMode};

/// The parts of the graphics state (§8.4.1) that extraction reports.
#[derive(Debug, Clone)]
pub(crate) struct GraphicsState {
    pub ctm: Matrix,
    pub line_width: f64,
    pub fill_space: Space,
    pub stroke_space: Space,
    pub fill: Color,
    pub stroke: Color,
    pub fill_alpha: f64,
    pub stroke_alpha: f64,
    pub text: TextState,
}

/// Identifies a font for page-local indexing and flood control.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum FontKey {
    Ref(Ref),
    /// A direct font dictionary, by its address in the resource tree.
    Direct(usize),
    Fallback,
}

/// Text state parameters (§9.3), saved and restored with the graphics state.
#[derive(Debug, Clone)]
pub(crate) struct TextState {
    pub char_spacing: f64,
    pub word_spacing: f64,
    /// `Tz / 100`.
    pub hscale: f64,
    pub leading: f64,
    pub font: Option<(FontKey, Arc<Font>)>,
    pub size: f64,
    pub render_mode: RenderMode,
    pub rise: f64,
}

impl Default for TextState {
    fn default() -> Self {
        TextState {
            char_spacing: 0.,
            word_spacing: 0.,
            hscale: 1.,
            leading: 0.,
            font: None,
            size: 0.,
            render_mode: RenderMode::Fill,
            rise: 0.,
        }
    }
}

impl Default for GraphicsState {
    fn default() -> Self {
        GraphicsState {
            ctm: Matrix::IDENTITY,
            line_width: 1.,
            fill_space: Space::Gray,
            stroke_space: Space::Gray,
            fill: Color::Gray(0.),
            stroke: Color::Gray(0.),
            fill_alpha: 1.,
            stroke_alpha: 1.,
            text: TextState::default(),
        }
    }
}

/// Exactly `N` finite numbers.
pub(crate) fn nums<const N: usize>(ops: &[Object]) -> Option<[f64; N]> {
    if ops.len() != N {
        return None;
    }
    let mut out = [0.; N];
    for (slot, o) in out.iter_mut().zip(ops) {
        *slot = o.as_number().filter(|n| n.is_finite())?;
    }
    Some(out)
}

/// A single name operand.
pub(crate) fn name(ops: &[Object]) -> Option<&[u8]> {
    match ops {
        [o] => o.as_name(),
        _ => None,
    }
}

pub(crate) fn matrix(m: [f64; 6]) -> Matrix {
    let [a, b, c, d, e, f] = m;
    Matrix { a, b, c, d, e, f }
}
