use std::collections::{HashMap, HashSet};

use pawdf_core::{DiagnosticKind, Dict, Location, Object, Ref};

use crate::annotation;
use crate::dest::{Cx, entry, text};
use crate::document::{Inner, MAX_TREE_DEPTH, PageInfo};
use crate::model::{MarkedContentRef, ObjectRef, StructElement, StructKid, StructTree};

/// The page content items of an element default to (Table 355).
#[derive(Clone, Copy)]
struct Pg {
    page: Option<usize>,
    /// Taken from an ancestor because the element has no usable `/Pg` of its own.
    inherited: bool,
}

struct Walk<'a> {
    cx: Cx<'a>,
    pages: &'a [PageInfo],
    role_map: Option<&'a Dict>,
    /// Elements and `/K` arrays already reached, so a cycle or a shared element is read once.
    visited: HashSet<u32>,
    /// Index into `Annotations::items` by object number, per page, built on the first lookup.
    annots: HashMap<usize, HashMap<u32, usize>>,
}

pub(crate) fn read(doc: &Inner) -> StructTree {
    let cx = Cx::new(doc, DiagnosticKind::BadStructTree);
    let (file, catalog, catalog_at) = (cx.file, cx.catalog, cx.catalog_at);
    let mut w = Walk {
        cx,
        pages: &doc.pages,
        role_map: None,
        visited: HashSet::new(),
        annots: HashMap::new(),
    };
    let mut roots = Vec::new();
    if let Some(t) = catalog.and_then(|c| entry(c, b"StructTreeRoot")) {
        let at = t.as_ref().map_or(catalog_at, Location::Object);
        match file.deref(t).as_dict() {
            Some(root) => {
                if let Some(r) = t.as_ref() {
                    w.visited.insert(r.num);
                }
                w.role_map = match entry(root, b"RoleMap").map(|m| file.deref(m)) {
                    None => None,
                    Some(Object::Dict(m)) => Some(m),
                    Some(_) => {
                        w.cx.bad(at);
                        None
                    }
                };
                let pg = Pg {
                    page: None,
                    inherited: true,
                };
                roots = w
                    .kids(root, at, t.as_ref(), pg, 0)
                    .into_iter()
                    .filter_map(|k| match k {
                        StructKid::Element(e) => Some(e),
                        _ => None,
                    })
                    .collect();
            }
            None => w.cx.bad(catalog_at),
        }
    }
    StructTree {
        roots,
        diagnostics: w.cx.diags.into_vec(),
    }
}

impl<'a> Walk<'a> {
    /// The kids in `/K` of `d`, held by the object at `at` (`me` when indirect); only elements
    /// at depth 0, the root. Only nesting (capped) recurses.
    fn kids(
        &mut self,
        d: &'a Dict,
        at: Location,
        me: Option<Ref>,
        pg: Pg,
        depth: usize,
    ) -> Vec<StructKid> {
        let file = self.cx.file;
        let Some(k) = entry(d, b"K") else {
            return Vec::new();
        };
        let items = match file.deref(k) {
            Object::Array(a) => {
                // Direct elements in a shared array would otherwise each walk it again.
                if let Some(r) = k.as_ref()
                    && !self.visited.insert(r.num)
                {
                    self.cx.bad(at);
                    return Vec::new();
                }
                a.as_slice()
            }
            _ => std::slice::from_ref(k),
        };
        let mut out = Vec::new();
        for item in items {
            let item_at = item.as_ref().map_or(at, Location::Object);
            match file.deref(item) {
                Object::Int(n) if depth > 0 => match u32::try_from(*n) {
                    Ok(mcid) => out.push(StructKid::MarkedContent(MarkedContentRef {
                        page: self.item_page(None, pg, at),
                        mcid,
                        stream: None,
                    })),
                    Err(_) => self.cx.bad(at),
                },
                Object::Dict(kd) => match kd
                    .get(b"Type")
                    .map(|t| file.deref(t))
                    .and_then(Object::as_name)
                {
                    Some(b"MCR" | b"OBJR") if depth == 0 => self.cx.bad(at),
                    Some(b"MCR") => {
                        if let Some(m) = self.mcr(kd, pg, item_at) {
                            out.push(StructKid::MarkedContent(m));
                        }
                    }
                    Some(b"OBJR") => {
                        if let Some(o) = self.objr(kd, pg, item_at) {
                            out.push(StructKid::Object(o));
                        }
                    }
                    // Table 355: a dictionary without `/Type` is a structure element.
                    _ => {
                        if let Some(r) = item.as_ref()
                            && !self.visited.insert(r.num)
                        {
                            self.cx.bad(at);
                        } else if depth >= MAX_TREE_DEPTH {
                            self.cx.bad(item_at);
                        } else {
                            let e = self.element(kd, item_at, item.as_ref(), me, pg, depth + 1);
                            out.push(StructKid::Element(e));
                        }
                    }
                },
                // A dangling reference is already reported by the file.
                Object::Null => {}
                _ => self.cx.bad(at),
            }
        }
        out
    }

    fn element(
        &mut self,
        d: &'a Dict,
        at: Location,
        me: Option<Ref>,
        parent: Option<Ref>,
        pg: Pg,
        depth: usize,
    ) -> StructElement {
        let file = self.cx.file;
        // Table 355: `/P` is required and indirect. Generations are ignored, as `File::resolve` does.
        match (entry(d, b"P").and_then(Object::as_ref), parent) {
            (Some(p), Some(q)) if p.num == q.num => {}
            (Some(_), None) => {}
            _ => self.cx.bad(at),
        }
        let role = match d.get(b"S").map(|s| file.deref(s)).and_then(Object::as_name) {
            Some(s) => s,
            None => {
                self.cx.bad(at);
                b""
            }
        };
        let standard = self.standard(role, at);
        let page = entry(d, b"Pg").and_then(|p| self.page(p, at));
        let kids_pg = match page {
            Some(_) => Pg {
                page,
                inherited: false,
            },
            None => Pg {
                inherited: true,
                ..pg
            },
        };
        StructElement {
            role: String::from_utf8_lossy(role).into_owned(),
            standard,
            title: text(&mut self.cx, d, b"T", at),
            alt: text(&mut self.cx, d, b"Alt", at),
            actual_text: text(&mut self.cx, d, b"ActualText", at),
            lang: text(&mut self.cx, d, b"Lang", at),
            expansion: text(&mut self.cx, d, b"E", at),
            page,
            children: self.kids(d, at, me, kids_pg, depth),
        }
    }

    /// Follows `/RoleMap` from `role` until a name is not mapped.
    fn standard(&mut self, role: &'a [u8], at: Location) -> String {
        let file = self.cx.file;
        let mut name = role;
        if let Some(map) = self.role_map {
            // After as many steps as there are keys, a still-mapped name is in a cycle.
            let mut steps = 0;
            while let Some(v) = map.get(name) {
                match file.deref(v).as_name() {
                    Some(n) if steps < map.len() => name = n,
                    _ => {
                        self.cx.bad(at);
                        break;
                    }
                }
                steps += 1;
            }
        }
        String::from_utf8_lossy(name).into_owned()
    }

    /// The page index of a `/Pg` entry; `None` and reported when it is not a page here.
    fn page(&mut self, pg: &Object, at: Location) -> Option<usize> {
        let i = pg.as_ref().and_then(|r| self.cx.page_index(r.num));
        if i.is_none() {
            self.cx.bad(at);
        }
        i
    }

    /// The page of a content item: its own `/Pg` (from an `/MCR` or `/OBJR` dictionary `d`),
    /// else its element's. An ancestor's is a recovery, as Table 355 requires the element's.
    fn item_page(&mut self, d: Option<&Dict>, pg: Pg, at: Location) -> Option<usize> {
        if let Some(p) = d.and_then(|d| entry(d, b"Pg")) {
            return self.page(p, at).or(pg.page);
        }
        if pg.inherited || pg.page.is_none() {
            self.cx.bad(at);
        }
        pg.page
    }

    fn mcr(&mut self, d: &Dict, pg: Pg, at: Location) -> Option<MarkedContentRef> {
        let file = self.cx.file;
        let Some(mcid) = entry(d, b"MCID")
            .and_then(|m| file.deref(m).as_int())
            .and_then(|n| u32::try_from(n).ok())
        else {
            self.cx.bad(at);
            return None;
        };
        let stream = entry(d, b"Stm").and_then(|s| {
            let r = s.as_ref();
            if r.is_none() {
                self.cx.bad(at);
            }
            r
        });
        Some(MarkedContentRef {
            page: self.item_page(Some(d), pg, at),
            mcid,
            stream,
        })
    }

    fn objr(&mut self, d: &Dict, pg: Pg, at: Location) -> Option<ObjectRef> {
        let Some(obj) = entry(d, b"Obj").and_then(Object::as_ref) else {
            self.cx.bad(at);
            return None;
        };
        let page = self.item_page(Some(d), pg, at);
        Some(ObjectRef {
            object: obj,
            page,
            annotation: page.and_then(|p| self.annotation(p, obj.num)),
        })
    }

    fn annotation(&mut self, page: usize, num: u32) -> Option<usize> {
        let (file, pages) = (self.cx.file, self.pages);
        self.annots
            .entry(page)
            .or_insert_with(|| {
                let annots = pages.get(page).and_then(|p| annotation::annots(file, p));
                let Some(Object::Array(annots)) = annots else {
                    return HashMap::new();
                };
                // Malformed entries are reported by `Page::annotations`.
                annotation::listed(file, annots, || {})
                    .iter()
                    .enumerate()
                    .filter_map(|(i, (a, _))| Some((a.as_ref()?.num, i)))
                    .collect()
            })
            .get(&num)
            .copied()
    }
}
