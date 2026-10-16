//! PDF syntax layer: lexer, objects, cross-reference data, stream filters, and diagnostics.
#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod diag;
mod error;
mod file;
mod filter;
mod lexer;
mod object;
mod parser;
mod xref;

pub use diag::{Diagnostic, DiagnosticKind, Diagnostics, Location, Origin};
pub use error::{Error, ErrorKind};
pub use file::File;
pub use filter::{Codec, Decoded, MAX_DECODED_LEN};
pub use lexer::{hex_value, is_delimiter, is_regular, is_whitespace};
pub use object::{Dict, Name, Object, Ref, Stream};
pub use parser::{MAX_DEPTH, parse_indirect};
