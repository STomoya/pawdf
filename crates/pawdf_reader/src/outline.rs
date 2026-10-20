use std::collections::{HashMap, HashSet};

use pawdf_core::{
    DiagnosticKind, Diagnostics, Dict, File, Location, Object, Ref, decode_text_string,
};

use crate::document::{Inner, MAX_TREE_DEPTH};
use crate::model::{Destination, Outline, OutlineItem};

struct Cx<'a> {
    file: &'a File,
    pages: HashMap<Ref, usize>,
    page_count: usize,
    visited: HashSet<u32>,
    diags: Diagnostics,
}

impl Cx<'_> {
    fn bad(&mut self, at: Location) {
        self.diags.push(DiagnosticKind::BadOutline, at);
    }
}

/// `None` for an absent or `null` entry, which §7.3.7 treats alike.
fn entry<'a>(dict: &'a Dict, key: &[u8]) -> Option<&'a Object> {
    dict.get(key).filter(|o| !matches!(o, Object::Null))
}

pub(crate) fn read(doc: &Inner) -> Outline {
    let file = &doc.file;
    let mut pages = HashMap::new();
    for (i, p) in doc.pages.iter().enumerate() {
        pages.entry(p.obj).or_insert(i);
    }
    let mut cx = Cx {
        file,
        pages,
        page_count: doc.pages.len(),
        visited: HashSet::new(),
        diags: Diagnostics::new(),
    };
    let root = file.trailer().get(b"Root");
    let catalog_at = root
        .and_then(Object::as_ref)
        .map_or(Location::File(0), Location::Object);
    let outlines = root
        .and_then(|c| file.deref(c).as_dict())
        .and_then(|c| entry(c, b"Outlines"));
    let mut items = Vec::new();
    if let Some(o) = outlines {
        let at = o.as_ref().map_or(catalog_at, Location::Object);
        match file.deref(o).as_dict() {
            Some(d) => items = chain(&mut cx, entry(d, b"First"), at, 0),
            None => cx.bad(catalog_at),
        }
    }
    Outline {
        items,
        diagnostics: cx.diags.into_vec(),
    }
}

/// Reads the sibling chain starting at `link`, held by the object at `holder`. Siblings
/// are a loop, so only nesting (capped) recurses.
fn chain<'a>(
    cx: &mut Cx<'a>,
    mut link: Option<&'a Object>,
    mut holder: Location,
    depth: usize,
) -> Vec<OutlineItem> {
    let file = cx.file;
    let mut out = Vec::new();
    while let Some(obj) = link {
        // One set for the whole outline: an item reached twice (a cycle, or a child shared by
        // two parents) is cut where the second link is.
        if let Some(r) = obj.as_ref()
            && !cx.visited.insert(r.num)
        {
            cx.bad(holder);
            break;
        }
        let Some(dict) = file.deref(obj).as_dict() else {
            cx.bad(holder);
            break;
        };
        let at = obj.as_ref().map_or(holder, Location::Object);
        let title = match dict
            .get(b"Title")
            .map(|t| file.deref(t))
            .and_then(Object::as_string)
        {
            Some(t) => decode_text_string(t),
            None => {
                cx.bad(at);
                String::new()
            }
        };
        let dest = destination(cx, dict, at);
        let children = match entry(dict, b"First") {
            Some(_) if depth >= MAX_TREE_DEPTH => {
                cx.bad(at);
                Vec::new()
            }
            first => chain(cx, first, at, depth + 1),
        };
        out.push(OutlineItem {
            title,
            dest,
            children,
        });
        link = entry(dict, b"Next");
        holder = at;
    }
    out
}

/// The item's `/Dest`, else the `/D` of a GoTo `/A` (§12.3.3, Table 151).
fn destination(cx: &mut Cx, item: &Dict, at: Location) -> Option<Destination> {
    let action = entry(item, b"A");
    if let Some(d) = entry(item, b"Dest") {
        // Table 151: /Dest "shall not be present if an A entry is present".
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
    if action.get(b"S").and_then(Object::as_name) != Some(b"GoTo") {
        return None;
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
fn target(cx: &mut Cx, value: &Object, at: Location) -> Option<Destination> {
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
        _ => {
            cx.bad(at);
            None
        }
    }
}

/// `[page /Form operands…]` (§12.3.2.2, Table 149).
fn explicit(cx: &mut Cx, arr: &[Object], at: Location) -> Option<Destination> {
    let page = match arr.first() {
        Some(Object::Ref(r)) => cx.pages.get(r).copied(),
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
