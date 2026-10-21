//! Document- and page-level PDF reading on top of `pawdf_core`.
#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod cmap;
mod content;
mod data;
mod document;
mod error;
mod font;
mod geom;
mod image;
mod model;
mod outline;
mod ranges;
#[cfg(test)]
mod testutil;

pub use document::{Document, Page};
pub use error::{Error, ErrorKind};
pub use geom::{Matrix, Point, Rect, Rotation};
pub use model::{
    CcittParams, Color, ColorSpace, ColorSpaceFamily, Destination, FillRule, FontInfo, FontKind,
    Glyph, Image, ImageData, MarkedContent, Outline, OutlineItem, PageContent, Path, RenderMode,
    Segment, SoftMask, WritingMode,
};
pub use pawdf_core::{
    Diagnostic, DiagnosticKind, Encryption, Error as CoreError, ErrorKind as CoreErrorKind,
    Location, Method, PasswordKind, Permissions, Ref,
};
