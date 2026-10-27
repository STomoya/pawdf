//! Optional content (§8.11): visibility under the default configuration only.

use std::collections::HashSet;

use pawdf_core::{Diagnostic, DiagnosticKind, Diagnostics, Dict, File, Location, Object, Ref};

use crate::dest::entry;
use crate::document::Inner;

/// Groups and expression nodes evaluated per pass. Visibility expressions can share
/// subexpressions by reference, so the depth limit alone does not bound the work.
const MAX_NODES: usize = 1 << 22;
const MAX_DEPTH: usize = 32;

/// The groups of `/OCProperties` and which of them the default configuration turns off.
#[derive(Default)]
pub(crate) struct Config {
    groups: HashSet<Ref>,
    off: HashSet<Ref>,
}

/// Reads the catalog's `/OCProperties` (§8.11.4.2) and its `/D` (§8.11.4.3).
pub(crate) fn read(file: &File) -> (Config, Vec<Diagnostic>) {
    let mut diags = Diagnostics::new();
    let mut config = Config::default();
    let root = file.trailer().get(b"Root");
    let at = root
        .and_then(Object::as_ref)
        .map_or(Location::File(0), Location::Object);
    let Some(props) = root
        .and_then(|r| file.deref(r).as_dict())
        .and_then(|c| entry(c, b"OCProperties"))
    else {
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
        let refs: Vec<Ref> = items.iter().filter_map(Object::as_ref).collect();
        if refs.len() < items.len() {
            bad(diags);
        }
        refs
    };
    config.groups = refs(props, b"OCGs", &mut diags).into_iter().collect();
    let Some(d) = entry(props, b"D").and_then(|d| file.deref(d).as_dict()) else {
        bad(&mut diags);
        return (config, diags.into_vec());
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
    (config, diags.into_vec())
}

/// Evaluates `/OC` entries for one pass.
pub(crate) struct Visibility<'a> {
    file: &'a File,
    config: &'a Config,
    nodes_left: usize,
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
            nodes_left: MAX_NODES,
        }
    });
    if vis.nodes_left == 0 {
        return false;
    }
    let on = vis.on(oc, at, diags);
    if vis.nodes_left == 0 {
        diags.push(DiagnosticKind::RecursionLimit, at);
        return false;
    }
    !on
}

impl Visibility<'_> {
    /// Spends one node; `false` once the budget is gone.
    fn tick(&mut self) -> bool {
        self.nodes_left = self.nodes_left.saturating_sub(1);
        self.nodes_left > 0
    }

    fn bad(diags: &mut Diagnostics, at: Location) {
        diags.push(DiagnosticKind::BadOptionalContent, at);
    }

    fn on(&mut self, oc: &Object, at: Location, diags: &mut Diagnostics) -> bool {
        let Some(d) = self.file.deref(oc).as_dict() else {
            Self::bad(diags, at);
            return true;
        };
        let ocmd = match d.get(b"Type").and_then(|t| self.file.deref(t).as_name()) {
            Some(b"OCMD") => true,
            Some(b"OCG") => false,
            _ => d.get(b"OCGs").is_some() || d.get(b"VE").is_some(),
        };
        if !ocmd {
            return self.group(oc.as_ref(), at, diags);
        }
        // §8.11.2.2: `/VE`, when present, overrides `/OCGs` and `/P`.
        if let Some(ve) = entry(d, b"VE") {
            return match self.expr(ve, 0, at, diags) {
                Ok(on) => on,
                Err(kind) => {
                    diags.push(kind, at);
                    true
                }
            };
        }
        let states: Vec<bool> = match entry(d, b"OCGs") {
            None => return true,
            Some(Object::Ref(r)) if self.file.resolve(*r).as_dict().is_some() => {
                vec![self.group(Some(*r), at, diags)]
            }
            Some(g) => match self.file.deref(g) {
                // Deleted groups (null) have no effect (Table 99).
                Object::Array(a) => a
                    .iter()
                    .filter(|g| !matches!(self.file.deref(g), Object::Null))
                    .map(|g| self.group(g.as_ref(), at, diags))
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

    /// The state of a group by reference; a group not in `/OCGs` is reported and on.
    fn group(&mut self, r: Option<Ref>, at: Location, diags: &mut Diagnostics) -> bool {
        if !self.tick() {
            return true;
        }
        match r {
            Some(r) if self.config.groups.contains(&r) => !self.config.off.contains(&r),
            _ => {
                Self::bad(diags, at);
                true
            }
        }
    }

    /// A visibility expression (§8.11.2.2): a group, or `[/And|/Or|/Not operands…]`.
    fn expr(
        &mut self,
        e: &Object,
        depth: usize,
        at: Location,
        diags: &mut Diagnostics,
    ) -> Result<bool, DiagnosticKind> {
        if depth > MAX_DEPTH {
            return Err(DiagnosticKind::RecursionLimit);
        }
        let items = match self.file.deref(e) {
            Object::Array(a) => a,
            Object::Dict(_) => return Ok(self.group(e.as_ref(), at, diags)),
            _ => return Err(DiagnosticKind::BadOptionalContent),
        };
        if !self.tick() {
            return Ok(true);
        }
        let Some((op, args)) = items.split_first() else {
            return Err(DiagnosticKind::BadOptionalContent);
        };
        let (mut all, mut any) = (true, false);
        for a in args {
            let v = self.expr(a, depth + 1, at, diags)?;
            all &= v;
            any |= v;
        }
        match (self.file.deref(op).as_name(), args.len()) {
            (Some(b"Not"), 1) => Ok(!all),
            (Some(b"And"), 1..) => Ok(all),
            (Some(b"Or"), 1..) => Ok(any),
            _ => Err(DiagnosticKind::BadOptionalContent),
        }
    }
}
