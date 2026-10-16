use crate::object::Ref;

/// What kind of recovery happened.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiagnosticKind {
    /// No `%PDF-` header in the first 1024 bytes.
    MissingHeader,
    /// The cross-reference data was unusable and was rebuilt by scanning the file.
    XrefRepaired,
    /// A cross-reference entry or section was malformed and was skipped or corrected.
    BadXrefEntry,
    /// A referenced object does not exist.
    ObjectNotFound,
    /// An xref offset did not point at the expected `N G obj` header.
    ObjectMismatch,
    /// Syntax error inside an object; the parsable part was kept.
    BadObject,
    /// `/Length` was missing or wrong; the data was delimited by `endstream` instead.
    BadStreamLength,
    /// A stream filter is not supported; decoding stopped there.
    UnknownFilter,
    /// Filtered data was corrupt; the data decoded so far was kept.
    FilterError,
    /// Decoded data exceeded the size limit and was truncated.
    StreamTooLarge,
    /// Nesting or reference chains were too deep and were cut off.
    RecursionLimit,
    /// The diagnostic limit was reached; later diagnostics were dropped.
    TooManyDiagnostics,
}

/// Where a diagnostic happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Location {
    /// Byte offset in the file.
    File(u64),
    /// Byte offset inside the decoded data of a stream object.
    Stream {
        /// The stream object.
        obj: Ref,
        /// Offset in its decoded data.
        offset: u64,
    },
}

/// A recovery from malformed input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Diagnostic {
    /// What was recovered from.
    pub kind: DiagnosticKind,
    /// Where.
    pub location: Location,
}

/// Where the bytes being parsed come from, so offsets can become [`Location`]s.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// The file itself.
    File,
    /// The decoded data of this stream object.
    Stream(Ref),
}

impl Origin {
    /// The location of `offset` within this origin.
    pub fn at(self, offset: usize) -> Location {
        let offset = offset as u64;
        match self {
            Origin::File => Location::File(offset),
            Origin::Stream(obj) => Location::Stream { obj, offset },
        }
    }
}

/// A capped collector of diagnostics.
#[derive(Debug, Clone, Default)]
pub struct Diagnostics {
    items: Vec<Diagnostic>,
}

impl Diagnostics {
    /// Maximum number of diagnostics kept before `TooManyDiagnostics` is recorded.
    pub const CAP: usize = 10_000;

    /// An empty collector.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records a diagnostic, subject to [`Diagnostics::CAP`].
    pub fn push(&mut self, kind: DiagnosticKind, location: Location) {
        if self.items.len() < Self::CAP {
            self.items.push(Diagnostic { kind, location });
        } else if self.items.len() == Self::CAP {
            self.items.push(Diagnostic {
                kind: DiagnosticKind::TooManyDiagnostics,
                location,
            });
        }
    }

    /// Moves all of `other` into `self`, subject to the cap.
    pub fn append(&mut self, other: Diagnostics) {
        for d in other.items {
            self.push(d.kind, d.location);
        }
    }

    /// The recorded diagnostics.
    pub fn as_slice(&self) -> &[Diagnostic] {
        &self.items
    }

    /// The recorded diagnostics, by value.
    pub fn into_vec(self) -> Vec<Diagnostic> {
        self.items
    }

    /// Whether nothing was recorded.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cap_appends_one_marker_then_drops() {
        let mut d = Diagnostics::new();
        for i in 0..Diagnostics::CAP + 5 {
            d.push(DiagnosticKind::BadObject, Location::File(i as u64));
        }
        let v = d.into_vec();
        assert_eq!(v.len(), Diagnostics::CAP + 1);
        assert_eq!(v[Diagnostics::CAP].kind, DiagnosticKind::TooManyDiagnostics);
    }

    #[test]
    fn origin_maps_offsets() {
        let r = Ref {
            num: 3,
            generation: 0,
        };
        assert_eq!(Origin::File.at(7), Location::File(7));
        assert_eq!(
            Origin::Stream(r).at(7),
            Location::Stream { obj: r, offset: 7 }
        );
    }
}
