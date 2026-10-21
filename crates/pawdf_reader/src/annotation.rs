use pawdf_core::{DiagnosticKind, Dict, Location, Object, decode_text_string};

use crate::dest::{Cx, destination, entry};
use crate::document::{Inner, PageInfo, rect};
use crate::geom::{Point, Rect};
use crate::model::{Annotation, Annotations};

pub(crate) fn read(doc: &Inner, page: &PageInfo) -> Annotations {
    let mut cx = Cx::new(doc, DiagnosticKind::BadAnnotation);
    let file = cx.file;
    let page_at = Location::Object(page.obj);
    let mut items = Vec::new();
    let annots = file
        .resolve(page.obj)
        .as_dict()
        .and_then(|p| entry(p, b"Annots"))
        .map(|a| file.deref(a));
    match annots {
        None => {}
        Some(Object::Array(annots)) => {
            for a in annots {
                match file.deref(a) {
                    Object::Dict(d) => {
                        let at = a.as_ref().map_or(page_at, Location::Object);
                        items.push(annotation(&mut cx, d, at));
                    }
                    // A dangling reference is already reported by the file.
                    Object::Null => {}
                    _ => cx.bad(page_at),
                }
            }
        }
        Some(_) => cx.bad(page_at),
    }
    Annotations {
        items,
        diagnostics: cx.diags.into_vec(),
    }
}

/// Table 166 entries, plus the link target (§12.5.6.5).
fn annotation<'a>(cx: &mut Cx<'a>, d: &'a Dict, at: Location) -> Annotation {
    let file = cx.file;
    let subtype = match d
        .get(b"Subtype")
        .map(|s| file.deref(s))
        .and_then(Object::as_name)
    {
        Some(s) => String::from_utf8_lossy(s).into_owned(),
        None => {
            cx.bad(at);
            String::new()
        }
    };
    let rect = match d.get(b"Rect").and_then(|r| rect(file, r)) {
        Some(r) => r,
        None => {
            cx.bad(at);
            Rect::from_corners(0., 0., 0., 0.)
        }
    };
    let flags = match entry(d, b"F").map(|f| file.deref(f)) {
        None => 0,
        Some(f) => match f.as_int().and_then(|n| u32::try_from(n).ok()) {
            Some(n) => n,
            None => {
                cx.bad(at);
                0
            }
        },
    };
    Annotation {
        subtype,
        rect,
        contents: text(cx, d, b"Contents", at),
        author: text(cx, d, b"T", at),
        name: text(cx, d, b"NM", at),
        flags,
        quad_points: quad_points(cx, d, at),
        dest: destination(cx, d, at),
        uri: uri(cx, d, at),
    }
}

fn text(cx: &mut Cx, d: &Dict, key: &[u8], at: Location) -> Option<String> {
    let v = cx.file.deref(entry(d, key)?);
    match v.as_string() {
        Some(s) => Some(decode_text_string(s)),
        None => {
            cx.bad(at);
            None
        }
    }
}

/// Groups of 8 numbers, as written: writers disagree with Figure 84 on the vertex order.
fn quad_points(cx: &mut Cx, d: &Dict, at: Location) -> Vec<[Point; 4]> {
    let file = cx.file;
    let Some(arr) = entry(d, b"QuadPoints").map(|q| file.deref(q)) else {
        return Vec::new();
    };
    let nums: Option<Vec<f64>> = arr.as_array().and_then(|a| {
        a.iter()
            .map(|n| file.deref(n).as_number().filter(|n| n.is_finite()))
            .collect()
    });
    let Some(nums) = nums else {
        cx.bad(at);
        return Vec::new();
    };
    let (quads, rest) = nums.as_chunks::<8>();
    if !rest.is_empty() {
        cx.bad(at);
    }
    quads
        .iter()
        .map(|q| {
            let p = |i: usize| Point {
                x: q[2 * i],
                y: q[2 * i + 1],
            };
            [p(0), p(1), p(2), p(3)]
        })
        .collect()
}

/// The `/URI` of a URI action (§12.6.4.8), which is UTF-8.
fn uri(cx: &mut Cx, d: &Dict, at: Location) -> Option<String> {
    let file = cx.file;
    let action = file.deref(entry(d, b"A")?).as_dict()?;
    if action
        .get(b"S")
        .map(|s| file.deref(s))
        .and_then(Object::as_name)
        != Some(b"URI")
    {
        return None;
    }
    let Some(bytes) = entry(action, b"URI").and_then(|u| file.deref(u).as_string()) else {
        cx.bad(at);
        return None;
    };
    if bytes.starts_with(b"\xFE\xFF") {
        cx.bad(at);
        return Some(decode_text_string(bytes));
    }
    match std::str::from_utf8(bytes) {
        Ok(s) => Some(s.to_owned()),
        Err(_) => {
            cx.bad(at);
            Some(String::from_utf8_lossy(bytes).into_owned())
        }
    }
}
