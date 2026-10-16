use std::fmt;

/// Why a file could not be opened at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// The trailer has an `/Encrypt` entry. Decryption is not supported.
    Encrypted,
    /// Neither the xref nor a full-file repair scan found a `/Root` catalog.
    Unrecoverable,
}

/// A fatal error, with the byte offset it relates to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    /// What went wrong.
    pub kind: ErrorKind,
    /// Byte offset in the file (the xref section, or 0 when none was usable).
    pub offset: u64,
}

impl Error {
    pub(crate) fn new(kind: ErrorKind, offset: u64) -> Self {
        Self { kind, offset }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let what = match self.kind {
            ErrorKind::Encrypted => "encrypted PDF files are not supported",
            ErrorKind::Unrecoverable => "no document catalog found",
        };
        write!(f, "{what} (at byte {})", self.offset)
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_includes_offset() {
        let e = Error::new(ErrorKind::Encrypted, 42);
        assert_eq!(
            e.to_string(),
            "encrypted PDF files are not supported (at byte 42)"
        );
    }
}
