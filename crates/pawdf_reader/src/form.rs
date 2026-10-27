use std::collections::{HashMap, HashSet};

use pawdf_core::{DiagnosticKind, Dict, Location, Object, decode_text_string};

use crate::dest::{Cx, entry, text};
use crate::document::{Inner, MAX_TREE_DEPTH, PageInfo, rect};
use crate::geom::Rect;
use crate::model::{FieldKind, FormField, FormFields, Widget};

/// Field flags (Table 227 and §12.7.5).
const RADIO: u32 = 1 << 15;
const PUSHBUTTON: u32 = 1 << 16;
const COMBO: u32 = 1 << 17;

/// Entries a field takes from its ancestors (Table 226), plus its name so far.
#[derive(Default, Clone)]
struct Inherited<'a> {
    /// `None` until some level has a `/T`.
    name: Option<String>,
    ft: Option<&'a Object>,
    ff: Option<&'a Object>,
    v: Option<&'a Object>,
}

struct Walk<'a> {
    cx: Cx<'a>,
    /// Field objects already reached, so a `/Kids` cycle or a shared kid is read once.
    visited: HashSet<u32>,
    items: Vec<FormField>,
    pages: &'a [PageInfo],
    /// Page index by the object number of each `/Annots` entry, built on the first lookup.
    annots: Option<HashMap<u32, usize>>,
}

pub(crate) fn read(doc: &Inner) -> FormFields {
    let cx = Cx::new(doc, DiagnosticKind::BadFormField);
    let (file, catalog, catalog_at) = (cx.file, cx.catalog, cx.catalog_at);
    let mut w = Walk {
        cx,
        visited: HashSet::new(),
        items: Vec::new(),
        pages: &doc.pages,
        annots: None,
    };
    if let Some(a) = catalog.and_then(|c| entry(c, b"AcroForm")) {
        let at = a.as_ref().map_or(catalog_at, Location::Object);
        match file.deref(a).as_dict() {
            Some(form) => match entry(form, b"Fields").map(|f| file.deref(f)) {
                None => {}
                Some(Object::Array(roots)) => {
                    for root in roots {
                        if w.first_visit(root, at) {
                            w.field(root, at, &Inherited::default(), 0);
                        }
                    }
                }
                Some(_) => w.cx.bad(at),
            },
            None => w.cx.bad(catalog_at),
        }
    }
    FormFields {
        items: w.items,
        diagnostics: w.cx.diags.into_vec(),
    }
}

impl<'a> Walk<'a> {
    fn first_visit(&mut self, obj: &Object, holder: Location) -> bool {
        match obj.as_ref() {
            Some(r) if !self.visited.insert(r.num) => {
                self.cx.bad(holder);
                false
            }
            _ => true,
        }
    }

    fn field(&mut self, obj: &'a Object, holder: Location, parent: &Inherited<'a>, depth: usize) {
        let file = self.cx.file;
        let d = match file.deref(obj) {
            Object::Dict(d) => d,
            // A dangling reference is already reported by the file.
            Object::Null => return,
            _ => {
                self.cx.bad(holder);
                return;
            }
        };
        let at = obj.as_ref().map_or(holder, Location::Object);
        let name = match (&parent.name, text(&mut self.cx, d, b"T", at)) {
            (Some(p), Some(t)) => Some(format!("{p}.{t}")),
            (_, Some(t)) => Some(t),
            (p, None) => p.clone(),
        };
        let inh = Inherited {
            name,
            ft: entry(d, b"FT").or(parent.ft),
            ff: entry(d, b"Ff").or(parent.ff),
            v: entry(d, b"V").or(parent.v),
        };
        let mut fields = Vec::new();
        let mut widgets = Vec::new();
        match entry(d, b"Kids").map(|k| file.deref(k)) {
            // A field without kids is its own widget when it has widget entries (§12.5.6.19).
            None if entry(d, b"Rect").is_some() || entry(d, b"Subtype").is_some() => {
                widgets.push((obj, d));
            }
            None => {}
            Some(Object::Array(_)) if depth >= MAX_TREE_DEPTH => {
                self.cx.bad(at);
                return;
            }
            Some(Object::Array(kids)) => {
                for kid in kids {
                    match file.deref(kid) {
                        // §12.7.4: kids are fields or widgets. `/T` is optional on fields
                        // (Table 226), so an unnamed kid with kids of its own is a field too.
                        Object::Dict(k)
                            if entry(k, b"Kids").is_some() || entry(k, b"T").is_some() =>
                        {
                            if self.first_visit(kid, at) {
                                fields.push(kid);
                            }
                        }
                        Object::Dict(k) => widgets.push((kid, k)),
                        Object::Null => {}
                        _ => self.cx.bad(at),
                    }
                }
            }
            Some(_) => self.cx.bad(at),
        }
        if !widgets.is_empty() || fields.is_empty() {
            let item = self.terminal(d, &inh, at, &widgets);
            self.items.push(item);
        }
        for kid in fields {
            self.field(kid, at, &inh, depth + 1);
        }
    }

    fn terminal(
        &mut self,
        d: &'a Dict,
        inh: &Inherited<'a>,
        at: Location,
        widgets: &[(&'a Object, &'a Dict)],
    ) -> FormField {
        let file = self.cx.file;
        let flags = match inh.ff.map(|f| file.deref(f)) {
            None => 0,
            Some(f) => match f.as_int().and_then(|n| u32::try_from(n).ok()) {
                Some(n) => n,
                None => {
                    self.cx.bad(at);
                    0
                }
            },
        };
        let kind = match inh.ft.map(|t| file.deref(t)).and_then(Object::as_name) {
            Some(b"Tx") => FieldKind::Text,
            Some(b"Sig") => FieldKind::Signature,
            Some(b"Ch") if flags & COMBO != 0 => FieldKind::ComboBox,
            Some(b"Ch") => FieldKind::ListBox,
            Some(b"Btn") if flags & PUSHBUTTON != 0 => FieldKind::PushButton,
            Some(b"Btn") if flags & RADIO != 0 => FieldKind::RadioButton,
            Some(b"Btn") => FieldKind::CheckBox,
            _ => {
                self.cx.bad(at);
                FieldKind::Unknown
            }
        };
        FormField {
            name: inh.name.clone().unwrap_or_default(),
            label: text(&mut self.cx, d, b"TU", at),
            kind,
            values: self.values(kind, inh.v, d, at),
            flags,
            widgets: widgets
                .iter()
                .map(|&(w, wd)| self.widget(w, wd, at))
                .collect(),
        }
    }

    /// `/V` by field type (§12.7.5). `/DV` is not a fallback: it is the reset value.
    fn values(
        &mut self,
        kind: FieldKind,
        v: Option<&Object>,
        d: &Dict,
        at: Location,
    ) -> Vec<String> {
        let file = self.cx.file;
        let Some(v) = v.map(|v| file.deref(v)) else {
            return Vec::new();
        };
        match (kind, v) {
            (FieldKind::Text | FieldKind::ComboBox | FieldKind::ListBox, Object::String(s)) => {
                vec![decode_text_string(s)]
            }
            // §12.7.5.3: a text field's value may be a text stream.
            (FieldKind::Text, Object::Stream(s)) => {
                let decoded = s.decode(at, &mut self.cx.diags);
                if decoded.codec.is_some() {
                    self.cx.bad(at);
                    return Vec::new();
                }
                vec![decode_text_string(&decoded.data)]
            }
            (FieldKind::ComboBox | FieldKind::ListBox, Object::Array(items)) => items
                .iter()
                .filter_map(|i| match file.deref(i) {
                    Object::String(s) => Some(decode_text_string(s)),
                    _ => {
                        self.cx.bad(at);
                        None
                    }
                })
                .collect(),
            (FieldKind::CheckBox | FieldKind::RadioButton, Object::Name(n)) => {
                let n = n.as_bytes();
                if n == b"Off" {
                    return Vec::new();
                }
                // §12.7.5.2.3: with `/Opt`, the appearance state names are indices into it.
                let opt = std::str::from_utf8(n)
                    .ok()
                    .and_then(|n| n.parse::<usize>().ok())
                    .and_then(|i| {
                        entry(d, b"Opt")
                            .and_then(|o| file.deref(o).as_array())
                            .and_then(|o| o.get(i))
                    })
                    .and_then(|o| file.deref(o).as_string());
                vec![match opt {
                    Some(s) => decode_text_string(s),
                    None => String::from_utf8_lossy(n).into_owned(),
                }]
            }
            (FieldKind::PushButton | FieldKind::Signature | FieldKind::Unknown, _) => Vec::new(),
            _ => {
                self.cx.bad(at);
                Vec::new()
            }
        }
    }

    fn widget(&mut self, obj: &Object, d: &Dict, holder: Location) -> Widget {
        let file = self.cx.file;
        let at = obj.as_ref().map_or(holder, Location::Object);
        let rect = match entry(d, b"Rect").and_then(|r| rect(file, r)) {
            Some(r) => r,
            None => {
                self.cx.bad(at);
                Rect::from_corners(0., 0., 0., 0.)
            }
        };
        Widget {
            page: self.page(obj, d, at),
            rect,
        }
    }

    /// The page whose `/Annots` lists the widget, else its `/P`, which is optional and less
    /// reliable (Table 166).
    fn page(&mut self, obj: &Object, d: &Dict, at: Location) -> Option<usize> {
        let file = self.cx.file;
        let pages = self.pages;
        let annots = self.annots.get_or_insert_with(|| {
            let mut map = HashMap::new();
            for (i, p) in pages.iter().enumerate() {
                let Some(list) = file
                    .resolve(p.obj)
                    .as_dict()
                    .and_then(|p| entry(p, b"Annots"))
                    .and_then(|a| file.deref(a).as_array())
                else {
                    continue;
                };
                for r in list.iter().filter_map(Object::as_ref) {
                    map.entry(r.num).or_insert(i);
                }
            }
            map
        });
        if let Some(&i) = obj.as_ref().and_then(|r| annots.get(&r.num)) {
            return Some(i);
        }
        self.cx.bad(at);
        let p = entry(d, b"P")?.as_ref()?;
        self.cx.page_index(p.num)
    }
}
