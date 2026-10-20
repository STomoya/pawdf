use std::fmt;

/// Why a file could not be opened at all.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ErrorKind {
    /// The file is encrypted and neither the given password nor the empty password
    /// authenticated.
    IncorrectPassword {
        /// Whether the caller passed a password.
        password_given: bool,
    },
    /// The file is encrypted with a security handler or method that is not supported. The
    /// string names it, e.g. `Adobe.PubSec` or `Standard R7`.
    UnsupportedEncryption(String),
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
        match &self.kind {
            ErrorKind::IncorrectPassword {
                password_given: false,
            } => f.write_str("a password is required")?,
            ErrorKind::IncorrectPassword {
                password_given: true,
            } => f.write_str("incorrect password")?,
            ErrorKind::UnsupportedEncryption(name) => write!(f, "unsupported encryption: {name}")?,
            ErrorKind::Unrecoverable => f.write_str("no document catalog found")?,
        }
        write!(f, " (at byte {})", self.offset)
    }
}

impl std::error::Error for Error {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_includes_offset() {
        let cases = [
            (
                ErrorKind::IncorrectPassword {
                    password_given: false,
                },
                "a password is required (at byte 42)",
            ),
            (
                ErrorKind::IncorrectPassword {
                    password_given: true,
                },
                "incorrect password (at byte 42)",
            ),
            (
                ErrorKind::UnsupportedEncryption("Standard R7".into()),
                "unsupported encryption: Standard R7 (at byte 42)",
            ),
            (
                ErrorKind::Unrecoverable,
                "no document catalog found (at byte 42)",
            ),
        ];
        for (kind, want) in cases {
            assert_eq!(Error::new(kind, 42).to_string(), want);
        }
    }
}
