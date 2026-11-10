//! A port of pdfminer.six's layout analysis (`pdfminer/layout.py`, version 20260107).
//!
//! Given the same glyph boxes, [`analyze`] produces the same lines, blocks and reading order
//! as pdfminer's `LTLayoutContainer.analyze`. Words are the runs of a line's glyphs between
//! the spaces pdfminer inserts and whitespace glyphs.
//!
//! Results on a whole PDF can still differ from running pdfminer on it:
//! - glyph boxes come from `pawdf_reader`, which measures them differently;
//! - text inside form XObjects is grouped with the rest of the page, whereas pdfminer keeps
//!   it in an ungrouped figure by default;
//! - pdfminer breaks ties between equally distant blocks by memory address, so its order
//!   can change between runs; this port breaks them by creation order.
//!
//! Result boxes are in pdfminer's upright page space: the media box's lower-left corner is
//! the origin, `/Rotate` is applied, and y points up. On rotated or shifted pages they differ
//! from [`Glyph::bbox`](pawdf_reader::Glyph::bbox).
//!
//! `/ActualText` is not applied. Glyph indices together with
//! [`PageContent::marked_contents`] let callers apply it.
//!
//! The port is distributed under pdfminer.six's MIT license as well; see `LICENSE-pdfminer`
//! in this crate.

#[allow(dead_code)] // Temporary: used once lines land.
mod input;

use pawdf_reader::{PageContent, Rect, Rotation};

use crate::{Error, Layout};

/// pdfminer's `LAParams`, without `all_texts`: form XObject text is always grouped.
#[derive(Debug, Clone, PartialEq)]
pub struct Params {
    /// Two glyphs whose overlap exceeds this fraction of the smaller height (width for
    /// vertical text) can share a line.
    pub line_overlap: f64,
    /// Two glyphs closer than this multiple of the wider glyph's width can share a line.
    pub char_margin: f64,
    /// Two lines closer than this multiple of the line height can share a block.
    pub line_margin: f64,
    /// A gap wider than this multiple of a glyph's larger dimension inserts a space.
    pub word_margin: f64,
    /// How much horizontal (-1.0) versus vertical (+1.0) position decides block order, within
    /// [-1, 1]. `None` sorts blocks by their lower-left corner instead.
    pub boxes_flow: Option<f64>,
    /// Whether vertical lines are detected.
    pub detect_vertical: bool,
}

impl Default for Params {
    fn default() -> Self {
        Params {
            line_overlap: 0.5,
            char_margin: 2.0,
            line_margin: 0.5,
            word_margin: 0.1,
            boxes_flow: Some(0.5),
            detect_vertical: false,
        }
    }
}

impl Params {
    fn validate(&self) -> Result<(), Error> {
        let checks = [
            ("line_overlap", self.line_overlap.is_finite()),
            ("char_margin", self.char_margin.is_finite()),
            ("line_margin", self.line_margin.is_finite()),
            ("word_margin", self.word_margin.is_finite()),
            (
                "boxes_flow",
                self.boxes_flow.is_none_or(|f| (-1.0..=1.0).contains(&f)),
            ),
        ];
        match checks.iter().find(|(_, ok)| !ok) {
            Some((name, _)) => Err(Error::InvalidParam { name }),
            None => Ok(()),
        }
    }
}

/// Groups a page's glyphs into blocks, lines and words, in reading order.
///
/// `media_box` and `rotate` are the page's, as from [`Page`](pawdf_reader::Page). Hidden
/// glyphs (optional content that is off) are left out; invisible text is kept.
pub fn analyze(
    content: &PageContent,
    media_box: Rect,
    rotate: Rotation,
    params: &Params,
) -> Result<Layout, Error> {
    params.validate()?;
    let mut diagnostics = Vec::new();
    let (m, _bound) = input::page_space(media_box, rotate);
    let _chars = input::select(content, &m, &mut diagnostics);
    Ok(Layout {
        blocks: Vec::new(),
        diagnostics,
    })
}
