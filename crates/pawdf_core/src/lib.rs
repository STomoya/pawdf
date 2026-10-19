//! PDF syntax layer: lexer, objects, cross-reference data, stream filters, and diagnostics.
#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod content;
mod diag;
mod error;
mod file;
mod filter;
mod lexer;
mod object;
mod parser;
mod text;
mod xref;

pub use content::{Operation, parse_content};
pub use diag::{Diagnostic, DiagnosticKind, Diagnostics, Location, Origin};
pub use error::{Error, ErrorKind};
pub use file::File;
pub use filter::{Codec, Decoded, MAX_DECODED_LEN};
pub use lexer::{hex_value, is_delimiter, is_regular, is_whitespace};
pub use object::{Dict, Name, Object, Ref, Stream};
pub use parser::{MAX_DEPTH, parse_indirect};
pub use text::decode_text_string;
