use std::collections::{HashMap, HashSet};

use pawdf_core::{
    DiagnosticKind, Diagnostics, Dict, File, Location, Object, Ref, decode_text_string,
};

use crate::document::{Inner, MAX_TREE_DEPTH};
use crate::model::{Destination, Outline, OutlineItem};

struct Cx<'a> {
    file: &'a File,
    pages: HashMap<Ref, usize>,
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

fn destination(_cx: &mut Cx, _item: &Dict, _at: Location) -> Option<Destination> {
    None
}
