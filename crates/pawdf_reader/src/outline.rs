use std::collections::HashSet;

use pawdf_core::{DiagnosticKind, Location, Object, decode_text_string};

use crate::dest::{Cx, destination, entry};
use crate::document::{Inner, MAX_TREE_DEPTH};
use crate::model::{Outline, OutlineItem};

pub(crate) fn read(doc: &Inner) -> Outline {
    let mut cx = Cx::new(doc, DiagnosticKind::BadOutline);
    let (file, catalog, catalog_at) = (cx.file, cx.catalog, cx.catalog_at);
    let mut visited = HashSet::new();
    let mut items = Vec::new();
    if let Some(o) = catalog.and_then(|c| entry(c, b"Outlines")) {
        let at = o.as_ref().map_or(catalog_at, Location::Object);
        match file.deref(o).as_dict() {
            Some(d) => items = chain(&mut cx, &mut visited, entry(d, b"First"), at, 0),
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
    visited: &mut HashSet<u32>,
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
            && !visited.insert(r.num)
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
            first => chain(cx, visited, first, at, depth + 1),
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
