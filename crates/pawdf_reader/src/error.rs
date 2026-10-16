use std::fmt;

/// Why an operation failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorKind {
    /// The file could not be opened.
    Core(pawdf_core::Error),
    /// A page index past the end of the document.
    PageOutOfRange {
        /// The requested index.
        index: usize,
        /// The number of pages.
        count: usize,
    },
}

/// A reader error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    /// What went wrong.
    pub kind: ErrorKind,
}

impl From<pawdf_core::Error> for Error {
    fn from(e: pawdf_core::Error) -> Self {
        Error {
            kind: ErrorKind::Core(e),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            ErrorKind::Core(e) => e.fmt(f),
            ErrorKind::PageOutOfRange { index, count } => {
                write!(f, "page index {index} out of range ({count} pages)")
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match &self.kind {
            ErrorKind::Core(e) => Some(e),
            ErrorKind::PageOutOfRange { .. } => None,
        }
    }
}
