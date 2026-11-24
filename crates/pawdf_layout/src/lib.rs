//! Layout analysis: groups a page's glyphs into blocks, lines and words in reading order.
//!
//! Each algorithm lives in its own module and returns the shared [`Layout`] types. The only
//! algorithm so far is [`pdfminer`], a port of pdfminer.six's layout analysis.
#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod pdfminer;

use std::fmt;

use pawdf_reader::{Diagnostic, Rect};

/// The blocks of one page, in reading order.
#[derive(Debug, Clone, PartialEq)]
pub struct Layout {
    /// Text blocks in reading order.
    pub blocks: Vec<Block>,
    /// Recoveries made by layout analysis only; reading the page reports its own.
    pub diagnostics: Vec<Diagnostic>,
}

/// A group of neighbouring lines.
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    /// Union of its lines' boxes, in the algorithm's page space.
    pub bbox: Rect,
    /// Direction of its lines.
    pub orientation: Orientation,
    /// Its lines' text joined with `\n`.
    pub text: String,
    /// Lines in reading order.
    pub lines: Vec<Line>,
}

/// One line of text.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    /// Union of its glyphs' boxes, in the algorithm's page space.
    pub bbox: Rect,
    /// Writing direction.
    pub orientation: Orientation,
    /// Its glyphs' text, with a space wherever the algorithm inserted one.
    pub text: String,
    /// Words in order.
    pub words: Vec<Word>,
    /// Indices into [`PageContent::glyphs`](pawdf_reader::PageContent::glyphs) of all its
    /// glyphs, including whitespace glyphs that belong to no word.
    pub glyphs: Vec<usize>,
}

/// A run of glyphs between spaces.
#[derive(Debug, Clone, PartialEq)]
pub struct Word {
    /// Union of its glyphs' boxes, in the algorithm's page space.
    pub bbox: Rect,
    /// Its glyphs' text, concatenated.
    pub text: String,
    /// Indices into [`PageContent::glyphs`](pawdf_reader::PageContent::glyphs).
    pub glyphs: Vec<usize>,
}

/// Direction of a line or block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Orientation {
    /// Left to right.
    Horizontal,
    /// Top to bottom.
    Vertical,
}

/// A layout error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// A setting was out of range or not finite.
    InvalidParam {
        /// The setting's name.
        name: &'static str,
    },
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::InvalidParam { name } => write!(f, "invalid layout setting `{name}`"),
        }
    }
}

impl std::error::Error for Error {}
