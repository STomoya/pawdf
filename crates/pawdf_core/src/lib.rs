//! PDF syntax layer: lexer, objects, cross-reference data, stream filters, and diagnostics.
#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod diag;
mod error;
mod object;

pub use diag::{Diagnostic, DiagnosticKind, Diagnostics, Location, Origin};
pub use error::{Error, ErrorKind};
pub use object::{Dict, Name, Object, Ref, Stream};
