//! Document- and page-level PDF reading on top of `pawdf_core`.
#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod annotation;
mod cmap;
mod content;
mod data;
mod dest;
mod document;
mod error;
mod font;
mod form;
mod geom;
mod image;
mod model;
mod oc;
mod outline;
mod ranges;
mod structure;
#[cfg(test)]
mod testutil;

pub use document::{Document, Page};
pub use error::{Error, ErrorKind};
pub use geom::{Matrix, Point, Rect, Rotation};
pub use model::{
    Annotation, Annotations, CcittParams, Color, ColorSpace, ColorSpaceFamily, Destination,
    FieldKind, FillRule, FontInfo, FontKind, FormField, FormFields, Glyph, Image, ImageData,
    MarkedContent, MarkedContentRef, ObjectRef, Outline, OutlineItem, PageContent, Path,
    RenderMode, Segment, SoftMask, StructElement, StructKid, StructTree, Widget, WritingMode,
};
pub use pawdf_core::{
    Diagnostic, DiagnosticKind, Encryption, Error as CoreError, ErrorKind as CoreErrorKind,
    Location, Method, PasswordKind, Permissions, Ref,
};
