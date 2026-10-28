//! Optional content (§8.11): visibility under the default configuration only.

use std::collections::{HashMap, HashSet};

use pawdf_core::{Diagnostic, DiagnosticKind, Diagnostics, Dict, File, Location, Object, Ref};

use crate::dest::entry;
use crate::document::{self, Inner, catalog};

const MAX_DEPTH: usize = 32;

/// The groups of `/OCProperties`, which of them the default configuration turns off, and
/// which it ignores for their intent.
#[derive(Default)]
pub(crate) struct Config {
    groups: HashSet<Ref>,
    off: HashSet<Ref>,
    ignored: HashSet<Ref>,
}

/// Reads the catalog's `/OCProperties` (§8.11.4.2) and its `/D` (§8.11.4.3).
pub(crate) fn read(file: &File) -> (Config, Vec<Diagnostic>) {
    let mut diags = Diagnostics::new();
    let mut config = Config::default();
    let (catalog, at) = catalog(file);
    let Some(props) = catalog.and_then(|c| entry(c, b"OCProperties")) else {
        return (config, Vec::new());
    };
    let bad = |diags: &mut Diagnostics| diags.push(DiagnosticKind::BadOptionalContent, at);
    let Some(props) = file.deref(props).as_dict() else {
        bad(&mut diags);
        return (config, diags.into_vec());
    };
    let refs = |d: &Dict, key: &[u8], diags: &mut Diagnostics| -> Vec<Ref> {
        let Some(v) = entry(d, key) else {
            return Vec::new();
        };
        let Some(items) = file.deref(v).as_array() else {
            bad(diags);
            return Vec::new();
        };
        // Deleted groups (null) have no effect (Table 99).
        document::refs(items.iter().filter(|o| !matches!(o, Object::Null)), || {
            bad(diags)
        })
    };
    if entry(props, b"OCGs").is_none() {
        bad(&mut diags);
    }
    config.groups = refs(props, b"OCGs", &mut diags).into_iter().collect();
    let Some(d) = entry(props, b"D").and_then(|d| file.deref(d).as_dict()) else {
        bad(&mut diags);
        return (config, diags.into_vec());
    };
    // §8.11.2.1, §8.11.4.3: a name or an array of names, `View` by default. An empty array is
    // meaningful (§8.11.2.3: no group is used), but one without a single name is malformed.
    let intents = |d: &Dict, diags: &mut Diagnostics| -> Vec<Vec<u8>> {
        let view = vec![b"View".to_vec()];
        match entry(d, b"Intent").map(|i| file.deref(i)) {
            None => view,
            Some(Object::Array(a)) => {
                let names: Vec<Vec<u8>> = a
                    .iter()
                    .filter_map(|n| file.deref(n).as_name())
                    .map(<[u8]>::to_vec)
                    .collect();
                if names.len() < a.len() {
                    bad(diags);
                }
                if names.is_empty() && !a.is_empty() {
                    view
                } else {
                    names
                }
            }
            Some(o) => match o.as_name() {
                Some(n) => vec![n.to_vec()],
                None => {
                    bad(diags);
                    view
                }
            },
        }
    };
    match entry(d, b"BaseState").map(|b| file.deref(b).as_name()) {
        None | Some(Some(b"ON" | b"Unchanged")) => {}
        Some(Some(b"OFF")) => config.off = config.groups.clone(),
        Some(_) => bad(&mut diags),
    }
    // pdf.js applies `/ON` then `/OFF`, so a group in both is off.
    for (key, on) in [(b"ON".as_slice(), true), (b"OFF", false)] {
        for r in refs(d, key, &mut diags) {
            if !config.groups.contains(&r) {
                bad(&mut diags);
            } else if on {
                config.off.remove(&r);
            } else {
                config.off.insert(r);
            }
        }
    }
    // §8.11.4.3: groups whose intent shares nothing with the configuration's are ignored.
    let wanted = intents(d, &mut diags);
    if !wanted.iter().any(|i| i == b"All") {
        config.ignored = config
            .groups
            .iter()
            .copied()
            .filter(|r| {
                file.resolve(*r)
                    .as_dict()
                    .is_some_and(|g| !intents(g, &mut diags).iter().any(|i| wanted.contains(i)))
            })
            .collect();
    }
    (config, diags.into_vec())
}

/// Evaluates `/OC` entries for one pass.
pub(crate) struct Visibility<'a> {
    file: &'a File,
    config: &'a Config,
    /// Results by object, so a membership dictionary or subexpression shared by reference is
    /// evaluated once per pass. This bounds the work by the size of the file.
    memberships: HashMap<Ref, bool>,
    /// Expression results with the depth they were evaluated at.
    exprs: HashMap<Ref, (usize, Result<Option<bool>, DiagnosticKind>)>,
}

/// Whether the `/OC` value `oc`, an optional content group or membership dictionary, is off.
/// Anything unusable counts as on. Creates `vis` on first use, reporting the configuration's
/// diagnostics into `diags` then.
pub(crate) fn hides<'a>(
    vis: &mut Option<Visibility<'a>>,
    doc: &'a Inner,
    oc: &Object,
    at: Location,
    diags: &mut Diagnostics,
) -> bool {
    let vis = vis.get_or_insert_with(|| {
        let (config, found) = doc.optional_content();
        for d in found {
            diags.push(d.kind, d.location);
        }
        Visibility {
            file: &doc.file,
            config,
            memberships: HashMap::new(),
            exprs: HashMap::new(),
        }
    });
    !vis.on(oc, at, diags)
}

impl Visibility<'_> {
    fn bad(diags: &mut Diagnostics, at: Location) {
        diags.push(DiagnosticKind::BadOptionalContent, at);
    }

    fn on(&mut self, oc: &Object, at: Location, diags: &mut Diagnostics) -> bool {
        let Some(r) = oc.as_ref() else {
            return self.on_uncached(oc, at, diags);
        };
        if let Some(&on) = self.memberships.get(&r) {
            return on;
        }
        let on = self.on_uncached(oc, at, diags);
        self.memberships.insert(r, on);
        on
    }

    fn on_uncached(&mut self, oc: &Object, at: Location, diags: &mut Diagnostics) -> bool {
        let Some(d) = self.file.deref(oc).as_dict() else {
            Self::bad(diags, at);
            return true;
        };
        let ocmd = match d.get(b"Type").and_then(|t| self.file.deref(t).as_name()) {
            Some(b"OCMD") => true,
            Some(b"OCG") => false,
            _ => entry(d, b"OCGs").is_some() || entry(d, b"VE").is_some(),
        };
        if !ocmd {
            return self.group(oc.as_ref(), at, diags).unwrap_or(true);
        }
        // §8.11.2.2: `/VE`, when present, overrides `/OCGs` and `/P`.
        if let Some(ve) = entry(d, b"VE") {
            return match self.expr(ve, 0, at, diags) {
                Ok(on) => on.unwrap_or(true),
                Err(kind) => {
                    diags.push(kind, at);
                    true
                }
            };
        }
        let states: Vec<bool> = match entry(d, b"OCGs") {
            None => return true,
            Some(Object::Ref(r)) if self.file.resolve(*r).as_dict().is_some() => {
                self.group(Some(*r), at, diags).into_iter().collect()
            }
            Some(g) => match self.file.deref(g) {
                // Deleted groups (null) have no effect (Table 99).
                Object::Array(a) => a
                    .iter()
                    .filter(|g| !matches!(self.file.deref(g), Object::Null))
                    .filter_map(|g| self.group(g.as_ref(), at, diags))
                    .collect(),
                Object::Null => return true,
                _ => {
                    Self::bad(diags, at);
                    return true;
                }
            },
        };
        if states.is_empty() {
            return true;
        }
        let policy = match entry(d, b"P").map(|p| self.file.deref(p).as_name()) {
            None => b"AnyOn".as_slice(),
            Some(Some(p @ (b"AllOn" | b"AnyOn" | b"AnyOff" | b"AllOff"))) => p,
            Some(_) => {
                Self::bad(diags, at);
                b"AnyOn"
            }
        };
        match policy {
            b"AllOn" => states.iter().all(|&s| s),
            b"AnyOff" => states.iter().any(|&s| !s),
            b"AllOff" => states.iter().all(|&s| !s),
            _ => states.iter().any(|&s| s),
        }
    }

    /// The state of a group by reference, or `None` when it has no effect: ignored for its
    /// intent, or not in `/OCGs` (reported).
    fn group(&self, r: Option<Ref>, at: Location, diags: &mut Diagnostics) -> Option<bool> {
        match r {
            Some(r) if self.config.groups.contains(&r) => {
                (!self.config.ignored.contains(&r)).then(|| !self.config.off.contains(&r))
            }
            _ => {
                Self::bad(diags, at);
                None
            }
        }
    }

    /// A visibility expression (§8.11.2.2): a group, or `[/And|/Or|/Not operands…]`. `None`
    /// when it has no effect.
    fn expr(
        &mut self,
        e: &Object,
        depth: usize,
        at: Location,
        diags: &mut Diagnostics,
    ) -> Result<Option<bool>, DiagnosticKind> {
        let Some(r) = e.as_ref() else {
            return self.expr_uncached(e, depth, at, diags);
        };
        // A depth limit hit deeper than this use may not recur here, so it is retried. Each
        // retry is shallower, so an object is evaluated at most `MAX_DEPTH + 1` times.
        if let Some(&(was, v)) = self.exprs.get(&r)
            && (v != Err(DiagnosticKind::RecursionLimit) || depth >= was)
        {
            return v;
        }
        let v = self.expr_uncached(e, depth, at, diags);
        self.exprs.insert(r, (depth, v));
        v
    }

    fn expr_uncached(
        &mut self,
        e: &Object,
        depth: usize,
        at: Location,
        diags: &mut Diagnostics,
    ) -> Result<Option<bool>, DiagnosticKind> {
        if depth > MAX_DEPTH {
            return Err(DiagnosticKind::RecursionLimit);
        }
        let items = match self.file.deref(e) {
            Object::Array(a) => a,
            Object::Dict(_) => return Ok(self.group(e.as_ref(), at, diags)),
            _ => return Err(DiagnosticKind::BadOptionalContent),
        };
        let Some((op, args)) = items.split_first() else {
            return Err(DiagnosticKind::BadOptionalContent);
        };
        // Operands without effect are left out; an operator left with none has none either.
        let (mut all, mut any, mut some) = (true, false, false);
        for a in args {
            if let Some(v) = self.expr(a, depth + 1, at, diags)? {
                all &= v;
                any |= v;
                some = true;
            }
        }
        let v = match (self.file.deref(op).as_name(), args.len()) {
            (Some(b"Not"), 1) => !all,
            (Some(b"And"), 1..) => all,
            (Some(b"Or"), 1..) => any,
            _ => return Err(DiagnosticKind::BadOptionalContent),
        };
        Ok(some.then_some(v))
    }
}
