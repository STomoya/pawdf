//! Document- and page-level PDF reading on top of `pawdf_core`.
#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod content;
mod data;
mod document;
mod error;
mod geom;
mod model;

pub use document::{Document, Page};
pub use error::{Error, ErrorKind};
pub use geom::{Matrix, Point, Rect, Rotation};
pub use model::{
    CcittParams, Color, ColorSpace, ColorSpaceFamily, FillRule, FontInfo, FontKind, Glyph, Image,
    ImageData, PageContent, Path, RenderMode, Segment, SoftMask, WritingMode,
};
