//! PDF syntax layer: lexer, objects, cross-reference data, stream filters, and diagnostics.
#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod diag;
mod error;
mod lexer;
mod object;
mod parser;

pub use diag::{Diagnostic, DiagnosticKind, Diagnostics, Location, Origin};
pub use error::{Error, ErrorKind};
pub use lexer::{Lexer, Token, hex_value, is_delimiter, is_regular, is_whitespace};
pub use object::{Dict, Name, Object, Ref, Stream};
pub use parser::{MAX_DEPTH, parse_indirect, parse_object, parse_object_from};
