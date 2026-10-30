//! Marked content (§14.6): `BMC`, `BDC`, `EMC`.

use pawdf_core::{DiagnosticKind, Dict, Location, Object, Operation, decode_text_string};

use super::Interp;
use crate::model::MarkedContent;

const MAX_MARKED_DEPTH: usize = 256;
/// Sequences returned per pass; a form of `BMC EMC` pairs drawn many times would otherwise
/// ask for unbounded memory.
const MAX_MARKED_CONTENTS: usize = 1 << 20;
/// Bytes of tags, `/ActualText` and artifact names returned per pass. Each span owns its
/// strings, so one long indirect `/ActualText` reused by many spans would otherwise ask for
/// unbounded memory.
pub(super) const MAX_MARKED_TEXT: usize = 1 << 24;

struct Open {
    /// Index into `PageContent::marked_contents`.
    slot: usize,
    actual: bool,
    /// An `/OC` sequence that is off, counted in `Interp::hidden`.
    hides: bool,
}

#[derive(Default)]
pub(super) struct Stack {
    open: Vec<Open>,
    /// Sequences opened beyond a limit and not stored. They are always the innermost, so `EMC`
    /// closes them first.
    dropped: usize,
    /// `dropped` before the outermost dropped sequence that hides was opened. Only that one is
    /// counted in `Interp::hidden`, so memory stays bounded.
    dropped_hiding: Option<usize>,
}

impl Interp<'_> {
    /// Runs a marked-content operator; `None` if `op` is not one.
    pub(super) fn marked_op(&mut self, op: &Operation) -> Option<bool> {
        let args = op.operands.as_slice();
        let tag = args.first().and_then(Object::as_name);
        let ok = match op.operator.as_slice() {
            b"BMC" => {
                self.begin_marked(tag, None, op.location);
                args.len() == 1 && tag.is_some()
            }
            b"BDC" => {
                self.begin_marked(tag, args.get(1), op.location);
                args.len() == 2 && tag.is_some()
            }
            b"EMC" => {
                self.end_marked(op.location);
                args.is_empty()
            }
            _ => return None,
        };
        Some(ok)
    }

    /// Opens a sequence. It is opened even when malformed, so that its `EMC` balances.
    fn begin_marked(&mut self, tag: Option<&[u8]>, props: Option<&Object>, at: Location) {
        let dropping = self.marked.dropped > 0
            || self.marked.open.len() >= MAX_MARKED_DEPTH
            || self.out.marked_contents.len() >= MAX_MARKED_CONTENTS;
        let hides = tag == Some(b"OC") && self.marked_oc_hides(props, at, dropping);
        if dropping {
            if hides && self.marked.dropped_hiding.is_none() {
                self.marked.dropped_hiding = Some(self.marked.dropped);
                self.hidden += 1;
            }
            self.marked.dropped += 1;
            self.marked_limit(at);
            return;
        }
        self.hidden += usize::from(hides);
        let mut span = self.read_span(tag, props, at);
        let cost = span.tag.len()
            + [
                &span.actual_text,
                &span.artifact_type,
                &span.artifact_subtype,
            ]
            .iter()
            .map(|s| s.as_ref().map_or(0, String::len))
            .sum::<usize>();
        match self.marked_text_left.checked_sub(cost) {
            Some(left) => self.marked_text_left = left,
            None => {
                self.marked_text_left = 0;
                span.tag.clear();
                span.actual_text = None;
                span.artifact_type = None;
                span.artifact_subtype = None;
                self.marked_limit(at);
            }
        }
        let actual = span.actual_text.is_some();
        self.actual_open += usize::from(actual);
        self.marked.open.push(Open {
            slot: self.out.marked_contents.len(),
            actual,
            hides,
        });
        self.out.marked_contents.push(span);
    }

    /// §8.11.3.2: `/OC /name BDC` names an optional content group or membership dictionary in
    /// `/Properties`. An inline dictionary or an indirect reference is accepted too. Missing or malformed properties
    /// are reported by `read_span`, or here when `report`, for a sequence it does not read.
    fn marked_oc_hides(&mut self, props: Option<&Object>, at: Location, report: bool) -> bool {
        let file = self.file;
        let oc = match props {
            Some(Object::Name(n)) => {
                let oc = self
                    .resource_entry(b"Properties", n.as_bytes())
                    .filter(|oc| file.deref(oc).as_dict().is_some());
                if oc.is_none() && report {
                    self.diags.push(DiagnosticKind::MissingResource, at);
                }
                oc
            }
            Some(p @ Object::Ref(_)) if report => {
                self.diags.push(DiagnosticKind::BadOperands, at);
                Some(p)
            }
            Some(p @ (Object::Dict(_) | Object::Ref(_))) => Some(p),
            Some(_) if report => {
                self.diags.push(DiagnosticKind::BadOperands, at);
                None
            }
            _ => None,
        };
        oc.is_some_and(|oc| self.oc_hides(oc, at))
    }

    fn marked_limit(&mut self, at: Location) {
        if !self.marked_limit_reported {
            self.marked_limit_reported = true;
            self.diags.push(DiagnosticKind::RecursionLimit, at);
        }
    }

    /// The span for `BMC`/`BDC`, with empty ranges at the current counts. Its strings are
    /// left out once the text budget is spent, so that they are not even decoded.
    fn read_span(
        &mut self,
        tag: Option<&[u8]>,
        props: Option<&Object>,
        at: Location,
    ) -> MarkedContent {
        let (g, p, i) = self.counts();
        let mut span = MarkedContent {
            tag: String::new(),
            mcid: None,
            actual_text: None,
            artifact_type: None,
            artifact_subtype: None,
            glyphs: g..g,
            paths: p..p,
            images: i..i,
        };
        if self.marked_text_left > 0 {
            span.tag = tag
                .map(|t| String::from_utf8_lossy(t).into_owned())
                .unwrap_or_default();
        }
        let file = self.file;
        let dict: Option<&Dict> = match props {
            None => None,
            Some(Object::Name(n)) => {
                let d = self
                    .resource(b"Properties", n.as_bytes())
                    .and_then(Object::as_dict);
                if d.is_none() {
                    self.diags.push(DiagnosticKind::MissingResource, at);
                }
                d
            }
            Some(p) => {
                // §7.8.2: content streams hold no indirect references; one is followed anyway.
                let d = file.deref(p).as_dict();
                if d.is_none() || matches!(p, Object::Ref(_)) {
                    self.diags.push(DiagnosticKind::BadOperands, at);
                }
                d
            }
        };
        let get = |k: &[u8]| dict.and_then(|d| d.get(k)).map(|o| file.deref(o));
        let text = self.marked_text_left > 0;
        span.actual_text = match get(b"ActualText") {
            // The outermost /ActualText replaces everything inside it.
            Some(Object::String(s)) => {
                (text && self.actual_open == 0).then(|| decode_text_string(s))
            }
            Some(_) => {
                self.diags.push(DiagnosticKind::BadOperands, at);
                None
            }
            None => None,
        };
        span.mcid = match get(b"MCID") {
            Some(o) => {
                let id = o.as_int().and_then(|n| u32::try_from(n).ok());
                if id.is_none() {
                    self.diags.push(DiagnosticKind::BadOperands, at);
                }
                id
            }
            None => None,
        };
        let artifact = text && tag == Some(b"Artifact");
        let artifact_name = |k: &[u8]| {
            get(k)
                .filter(|_| artifact)
                .and_then(Object::as_name)
                .map(|n| String::from_utf8_lossy(n).into_owned())
        };
        span.artifact_type = artifact_name(b"Type");
        span.artifact_subtype = artifact_name(b"Subtype");
        span
    }

    fn end_marked(&mut self, at: Location) {
        if self.marked.dropped > 0 {
            self.marked.dropped -= 1;
            if self.marked.dropped_hiding == Some(self.marked.dropped) {
                self.marked.dropped_hiding = None;
                self.hidden -= 1;
            }
            return;
        }
        match self.marked.open.pop() {
            Some(open) => self.close_span(open),
            None => self.diags.push(DiagnosticKind::UnbalancedMarkedContent, at),
        }
    }

    /// Closes every sequence still open at the end of the page's content or of a form XObject.
    pub(super) fn close_marked(&mut self, at: Location) {
        if self.marked.open.is_empty() && self.marked.dropped == 0 {
            return;
        }
        self.diags.push(DiagnosticKind::UnbalancedMarkedContent, at);
        self.marked.dropped = 0;
        if self.marked.dropped_hiding.take().is_some() {
            self.hidden -= 1;
        }
        while let Some(open) = self.marked.open.pop() {
            self.close_span(open);
        }
    }

    fn close_span(&mut self, open: Open) {
        let (g, p, i) = self.counts();
        if let Some(m) = self.out.marked_contents.get_mut(open.slot) {
            m.glyphs.end = g;
            m.paths.end = p;
            m.images.end = i;
        }
        if open.actual {
            self.actual_open = self.actual_open.saturating_sub(1);
        }
        self.hidden -= usize::from(open.hides);
    }

    fn counts(&self) -> (usize, usize, usize) {
        (
            self.out.glyphs.len(),
            self.out.paths.len(),
            self.out.images.len(),
        )
    }
}
