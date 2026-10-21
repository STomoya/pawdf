//! Destination resolution (§12.3.2), shared by outlines and annotations.

use std::collections::{HashMap, HashSet};

use pawdf_core::{DiagnosticKind, Diagnostics, Dict, File, Location, Object};

use crate::document::{Inner, MAX_TREE_DEPTH};
use crate::model::Destination;

pub(crate) struct Cx<'a> {
    pub file: &'a File,
    /// Page index by object number; generations are ignored, as `File::resolve` does.
    pages: HashMap<u32, usize>,
    page_count: usize,
    pub diags: Diagnostics,
    /// What every recovery is reported as.
    kind: DiagnosticKind,
    pub catalog: Option<&'a Dict>,
    pub catalog_at: Location,
    /// The `/Names /Dests` tree, flattened on the first named lookup.
    names: Option<HashMap<&'a [u8], &'a Object>>,
}

impl<'a> Cx<'a> {
    pub fn new(doc: &'a Inner, kind: DiagnosticKind) -> Self {
        let file = &doc.file;
        let mut pages = HashMap::new();
        for (i, p) in doc.pages.iter().enumerate() {
            pages.entry(p.obj.num).or_insert(i);
        }
        let root = file.trailer().get(b"Root");
        Cx {
            file,
            pages,
            page_count: doc.pages.len(),
            diags: Diagnostics::new(),
            kind,
            catalog: root.and_then(|c| file.deref(c).as_dict()),
            catalog_at: root
                .and_then(Object::as_ref)
                .map_or(Location::File(0), Location::Object),
            names: None,
        }
    }

    pub fn bad(&mut self, at: Location) {
        self.diags.push(self.kind, at);
    }
}

/// `None` for an absent or `null` entry, which §7.3.7 treats alike.
pub(crate) fn entry<'a>(dict: &'a Dict, key: &[u8]) -> Option<&'a Object> {
    dict.get(key).filter(|o| !matches!(o, Object::Null))
}

/// The `/Dest` of an outline item or link annotation, else the `/D` of a GoTo `/A` (Tables 151
/// and 176).
pub(crate) fn destination<'a>(
    cx: &mut Cx<'a>,
    item: &'a Dict,
    at: Location,
) -> Option<Destination> {
    let action = entry(item, b"A");
    if let Some(d) = entry(item, b"Dest") {
        // Tables 151 and 176: /Dest "shall not be present if an A entry is present".
        if action.is_some() {
            cx.bad(at);
        }
        return target(cx, d, at);
    }
    let action = cx.file.deref(action?);
    let Some(action) = action.as_dict() else {
        cx.bad(at);
        return None;
    };
    match action
        .get(b"S")
        .map(|t| cx.file.deref(t))
        .and_then(Object::as_name)
    {
        Some(b"GoTo") => {}
        Some(_) => return None,
        None => {
            cx.bad(at);
            return None;
        }
    }
    match entry(action, b"D") {
        Some(d) => target(cx, d, at),
        None => {
            cx.bad(at);
            None
        }
    }
}

/// A destination value: an explicit array, a dictionary with `/D` (§12.3.2.4), or a name.
fn target<'a>(cx: &mut Cx<'a>, value: &'a Object, at: Location) -> Option<Destination> {
    let file = cx.file;
    match file.deref(value) {
        Object::Array(a) => explicit(cx, a, at),
        Object::Dict(d) => match d.get(b"D").map(|d| file.deref(d)) {
            Some(Object::Array(a)) => explicit(cx, a, at),
            _ => {
                cx.bad(at);
                None
            }
        },
        Object::String(key) => named(cx, key, at),
        Object::Name(key) => named(cx, key.as_bytes(), at),
        _ => {
            cx.bad(at);
            None
        }
    }
}

/// `[page /Form operands…]` (§12.3.2.2, Table 149).
fn explicit(cx: &mut Cx, arr: &[Object], at: Location) -> Option<Destination> {
    let page = match arr.first() {
        Some(Object::Ref(r)) => cx.pages.get(&r.num).copied(),
        // Only remote GoTo may use a page number, but pdf.js accepts it locally too.
        Some(Object::Int(n)) => match usize::try_from(*n).ok().filter(|&n| n < cx.page_count) {
            Some(n) => {
                cx.bad(at);
                Some(n)
            }
            None => None,
        },
        _ => None,
    };
    let Some(page) = page else {
        cx.bad(at);
        return None;
    };
    let file = cx.file;
    let mut operand = |i: usize| match arr.get(i).map(|o| file.deref(o)) {
        Some(Object::Null) => None,
        Some(o) if o.as_number().is_some_and(f64::is_finite) => o.as_number(),
        _ => {
            cx.bad(at);
            None
        }
    };
    let (left, top) = match arr.get(1).map(|o| file.deref(o)).and_then(Object::as_name) {
        Some(b"XYZ") => (operand(2), operand(3)),
        Some(b"FitH" | b"FitBH") => (None, operand(2)),
        Some(b"FitV" | b"FitBV") => (operand(2), None),
        Some(b"FitR") => (operand(2), operand(5)),
        Some(b"Fit" | b"FitB") => (None, None),
        _ => {
            cx.bad(at);
            (None, None)
        }
    };
    Some(Destination { page, left, top })
}

/// Looks `key` up in the `/Names /Dests` tree, then the legacy catalog `/Dests` (§12.3.2.4).
/// Strings and names with the same bytes match the same entry.
fn named<'a>(cx: &mut Cx<'a>, key: &[u8], at: Location) -> Option<Destination> {
    let file = cx.file;
    if cx.names.is_none() {
        let mut names = HashMap::new();
        if let Some(root) = cx
            .catalog
            .and_then(|c| entry(c, b"Names"))
            .and_then(|n| file.deref(n).as_dict())
            .and_then(|n| entry(n, b"Dests"))
        {
            flatten(cx, root, &mut names);
        }
        cx.names = Some(names);
    }
    let value = cx
        .names
        .as_ref()
        .and_then(|n| n.get(key).copied())
        .or_else(|| {
            cx.catalog
                .and_then(|c| entry(c, b"Dests"))
                .and_then(|d| file.deref(d).as_dict())
                .and_then(|d| d.get(key))
        });
    match value.map(|v| file.deref(v)) {
        // A name mapping to another name is not followed, so lookups cannot loop.
        Some(Object::Array(_) | Object::Dict(_)) => value.and_then(|v| target(cx, v, at)),
        _ => {
            cx.bad(at);
            None
        }
    }
}

/// Collects a name tree's leaves (§7.9.6). `/Limits` is ignored because writers get it
/// wrong; the first occurrence of a key wins.
fn flatten<'a>(cx: &mut Cx<'a>, root: &'a Object, out: &mut HashMap<&'a [u8], &'a Object>) {
    let file = cx.file;
    let mut visited = HashSet::new();
    let mut stack = vec![(root, cx.catalog_at, 0usize)];
    while let Some((obj, holder, depth)) = stack.pop() {
        if let Some(r) = obj.as_ref()
            && !visited.insert(r.num)
        {
            cx.bad(holder);
            continue;
        }
        let Some(node) = file.deref(obj).as_dict() else {
            cx.bad(holder);
            continue;
        };
        let at = obj.as_ref().map_or(holder, Location::Object);
        let names = entry(node, b"Names").map(|n| file.deref(n));
        let kids = entry(node, b"Kids").map(|k| file.deref(k));
        if names.is_none() && kids.is_none() {
            cx.bad(at);
        }
        if let Some(names) = names {
            let Some(names) = names.as_array() else {
                cx.bad(at);
                continue;
            };
            let (pairs, rest) = names.as_chunks::<2>();
            if !rest.is_empty() {
                cx.bad(at);
            }
            for [key, value] in pairs {
                match file.deref(key) {
                    Object::String(k) => {
                        out.entry(k.as_slice()).or_insert(value);
                    }
                    _ => cx.bad(at),
                }
            }
        }
        if let Some(kids) = kids {
            match kids.as_array() {
                Some(_) if depth >= MAX_TREE_DEPTH => cx.bad(at),
                Some(kids) => stack.extend(kids.iter().rev().map(|k| (k, at, depth + 1))),
                None => cx.bad(at),
            }
        }
    }
}
