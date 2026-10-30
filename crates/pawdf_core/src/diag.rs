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
    /// Syntax error inside an object, or an entry of the wrong type or out of range; the
    /// parsable part or a default was used.
    BadObject,
    /// `/Length` was missing or wrong; the data was delimited by `endstream` instead.
    BadStreamLength,
    /// A stream filter is not supported; decoding stopped there.
    UnknownFilter,
    /// Filtered data was corrupt; the data decoded so far was kept.
    FilterError,
    /// Decoded data exceeded the size limit and was truncated.
    StreamTooLarge,
    /// Nesting, reference chains, or per-page work and output exceeded a limit and were cut off.
    RecursionLimit,
    /// The diagnostic limit was reached; later diagnostics were dropped.
    TooManyDiagnostics,
    /// The page tree was malformed (cycle, bad node, bad box, bad `/Rotate`, or a `/Contents`
    /// that is not a reference or an array, or an entry of it that is not a reference).
    BadPageTree,
    /// An operator had the wrong operand count or type, or a non-finite number; it was skipped.
    /// Also operands left at the end of the content with no operator, and a `TJ` element that
    /// is neither a string nor a finite number, which was skipped.
    /// Also a clip whose bounds became non-finite or inverted through the CTM; it was ignored.
    BadOperands,
    /// An unknown operator appeared outside `BX`/`EX`.
    UnknownOperator,
    /// A text operator appeared outside `BT`/`ET`, `BT` was nested, or `W`/`W*` was not
    /// followed by path construction or painting.
    MisplacedOperator,
    /// `Q` with an empty state stack, or `ET` without `BT`.
    UnbalancedRestore,
    /// `EMC` without an open marked-content sequence, or a sequence still open at the end of
    /// the page's content (all of `/Contents`, read as one stream) or of a form XObject.
    UnbalancedMarkedContent,
    /// A named font, XObject, color space, or ExtGState was not in the resources.
    MissingResource,
    /// A font dictionary was unusable; fallback metrics were used.
    BadFont,
    /// A character code has no Unicode mapping.
    UnmappedGlyph,
    /// A malformed CMap entry was skipped.
    BadCMap,
    /// An image had bad dimensions or short data, a Pattern colour space, or an inline image had
    /// no `EI`.
    BadImage,
    /// Encryption data was malformed (`/ID`, `/Length`, `/Perms`, AES padding, or an unknown
    /// crypt filter); the object or value was used as is.
    BadEncryption,
    /// The given password was wrong, but the empty user password opened the file.
    WrongPassword,
    /// An outline item or destination was malformed; the item was kept, cut, or left without
    /// a destination.
    BadOutline,
    /// An annotation or its destination was malformed; the annotation was skipped, or an entry
    /// was left empty or defaulted.
    BadAnnotation,
    /// A form field or its widget was malformed; the field was skipped, or an entry was left
    /// empty or defaulted.
    BadFormField,
    /// Optional content (§8.11) was malformed: a bad `/OCProperties` or default configuration,
    /// a group not listed in `/OCGs`, or a bad membership dictionary or visibility expression.
    /// The content was treated as visible.
    BadOptionalContent,
    /// The structure tree (§14.7) was malformed: a bad `/StructTreeRoot`, element, `/K` kid, or
    /// `/Type`, `/P` that is not the element's parent, `/Pg` that is not a page, a content item
    /// whose page came from an ancestor or is missing, a role map cycle or bad value, or an
    /// element reached twice or nested too deep. The element or kid was kept with the entry left
    /// empty or defaulted, or cut.
    BadStructTree,
}

impl DiagnosticKind {
    /// The kind as a snake_case name, as exposed to Python.
    pub fn name(self) -> &'static str {
        match self {
            DiagnosticKind::MissingHeader => "missing_header",
            DiagnosticKind::XrefRepaired => "xref_repaired",
            DiagnosticKind::BadXrefEntry => "bad_xref_entry",
            DiagnosticKind::ObjectNotFound => "object_not_found",
            DiagnosticKind::ObjectMismatch => "object_mismatch",
            DiagnosticKind::BadObject => "bad_object",
            DiagnosticKind::BadStreamLength => "bad_stream_length",
            DiagnosticKind::UnknownFilter => "unknown_filter",
            DiagnosticKind::FilterError => "filter_error",
            DiagnosticKind::StreamTooLarge => "stream_too_large",
            DiagnosticKind::RecursionLimit => "recursion_limit",
            DiagnosticKind::TooManyDiagnostics => "too_many_diagnostics",
            DiagnosticKind::BadPageTree => "bad_page_tree",
            DiagnosticKind::BadOperands => "bad_operands",
            DiagnosticKind::UnknownOperator => "unknown_operator",
            DiagnosticKind::MisplacedOperator => "misplaced_operator",
            DiagnosticKind::UnbalancedRestore => "unbalanced_restore",
            DiagnosticKind::UnbalancedMarkedContent => "unbalanced_marked_content",
            DiagnosticKind::MissingResource => "missing_resource",
            DiagnosticKind::BadFont => "bad_font",
            DiagnosticKind::UnmappedGlyph => "unmapped_glyph",
            DiagnosticKind::BadCMap => "bad_cmap",
            DiagnosticKind::BadImage => "bad_image",
            DiagnosticKind::BadEncryption => "bad_encryption",
            DiagnosticKind::WrongPassword => "wrong_password",
            DiagnosticKind::BadOutline => "bad_outline",
            DiagnosticKind::BadAnnotation => "bad_annotation",
            DiagnosticKind::BadFormField => "bad_form_field",
            DiagnosticKind::BadOptionalContent => "bad_optional_content",
            DiagnosticKind::BadStructTree => "bad_struct_tree",
        }
    }
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
    /// An object as a whole, when there is no byte position to point at.
    Object(Ref),
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
