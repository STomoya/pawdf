//! The content-stream interpreter (§8, §9).

pub(crate) mod color;
pub(crate) mod state;
pub(crate) mod text;

use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use pawdf_core::{
    DiagnosticKind, Diagnostics, Dict, File, Location, MAX_DECODED_LEN, Object, Operation, Origin,
    Ref, Stream, parse_content,
};

use crate::document::{Inner, Page, numbers};
use crate::font::Font;
use crate::geom::{Matrix, Point, Rect};
use crate::image::{ImageParts, expand_inline, read_image};
use crate::model::{FillRule, Image, PageContent, Path, Segment};
use color::{Space, device_space, resolve_space};
use state::{FontKey, GraphicsState, matrix, name, nums};

const MAX_Q_DEPTH: usize = 256;
const MAX_FORM_DEPTH: usize = 32;
/// Operations executed per pass, counting every form invocation. Forms that draw each
/// other several times form a DAG that cycle and depth checks do not bound.
const MAX_OPERATIONS: usize = 1 << 22;
/// Image bytes returned per pass. Each placement owns its data, so a small file placing
/// one large image many times would otherwise ask for unbounded memory.
const MAX_IMAGE_BYTES: usize = MAX_DECODED_LEN;

impl Page {
    /// Extracts glyphs, painted paths, and images. Never fails: problems become
    /// [`PageContent::diagnostics`].
    pub fn content(&self) -> PageContent {
        let file = &self.doc.file;
        let resources = self.info.resources.as_deref().and_then(Object::as_dict);
        let mut interp = Interp::new(&self.doc, resources);
        let streams = content_streams(file, self.info.obj);
        let mut carry: Vec<Object> = Vec::new();
        for r in streams {
            let Some(parsed) = interp.parse_stream(r) else {
                continue;
            };
            if carry.is_empty() && parsed.last().is_none_or(|o| !o.operator.is_empty()) {
                interp.run(&parsed);
                continue;
            }
            let mut ops = Vec::clone(&parsed);
            if !carry.is_empty()
                && let Some(first) = ops.first_mut()
            {
                carry.append(&mut first.operands);
                first.operands = std::mem::take(&mut carry);
            }
            if ops.last().is_some_and(|o| o.operator.is_empty())
                && let Some(last) = ops.pop()
            {
                carry = last.operands;
            }
            interp.run(&ops);
        }
        interp.finish()
    }
}

/// The page's `/Contents` stream references, in order.
fn content_streams(file: &File, page: Ref) -> Vec<Ref> {
    let Some(dict) = file.resolve(page).as_dict() else {
        return Vec::new();
    };
    match dict.get(b"Contents") {
        Some(Object::Ref(r)) => match file.resolve(*r) {
            Object::Array(a) => a.iter().filter_map(Object::as_ref).collect(),
            _ => vec![*r],
        },
        Some(Object::Array(a)) => a.iter().filter_map(Object::as_ref).collect(),
        _ => Vec::new(),
    }
}

pub(crate) struct Interp<'a> {
    pub doc: &'a Inner,
    pub file: &'a File,
    pub gs: GraphicsState,
    stack: Vec<GraphicsState>,
    pub out: PageContent,
    pub diags: Diagnostics,
    /// Active `/Resources`, innermost last.
    pub resources: Vec<Option<&'a Dict>>,
    path: Vec<Segment>,
    current: Option<Point>,
    subpath_start: Option<Point>,
    /// `BX`/`EX` nesting: unknown operators are expected inside (§8.2).
    compat: usize,
    q_limit_reported: bool,
    pub in_text: bool,
    pub tm: Matrix,
    pub tlm: Matrix,
    pub fallback: Arc<Font>,
    pub font_index: HashMap<FontKey, usize>,
    /// Fonts already used in this pass.
    pub pass_fonts: HashMap<FontKey, Arc<Font>>,
    pub unmapped_reported: HashSet<FontKey>,
    pub no_font_reported: bool,
    /// Forms being drawn, innermost last, to cut cycles.
    form_stack: Vec<Ref>,
    /// `Q` cannot pop states saved outside the current form.
    stack_floor: usize,
    /// Parsed streams by object, so repeated forms and `/Contents` entries decode once.
    parsed: HashMap<Ref, Option<Rc<Vec<Operation>>>>,
    ops_left: usize,
    /// Decoded image XObjects by object, so repeated placements decode once.
    images: HashMap<Ref, Option<Rc<ImageParts>>>,
    image_bytes_left: usize,
    image_budget_reported: bool,
}

impl<'a> Interp<'a> {
    pub fn new(doc: &'a Inner, resources: Option<&'a Dict>) -> Self {
        Interp {
            doc,
            file: &doc.file,
            gs: GraphicsState::default(),
            stack: Vec::new(),
            out: PageContent::default(),
            diags: Diagnostics::new(),
            resources: vec![resources],
            path: Vec::new(),
            current: None,
            subpath_start: None,
            compat: 0,
            q_limit_reported: false,
            in_text: false,
            tm: Matrix::IDENTITY,
            tlm: Matrix::IDENTITY,
            fallback: Arc::new(Font::fallback()),
            font_index: HashMap::new(),
            pass_fonts: HashMap::new(),
            unmapped_reported: HashSet::new(),
            no_font_reported: false,
            form_stack: Vec::new(),
            stack_floor: 0,
            parsed: HashMap::new(),
            ops_left: MAX_OPERATIONS,
            images: HashMap::new(),
            image_bytes_left: MAX_IMAGE_BYTES,
            image_budget_reported: false,
        }
    }

    pub fn finish(mut self) -> PageContent {
        self.out.diagnostics = self.diags.into_vec();
        self.out
    }

    /// Decodes and parses one content stream; `None` if it is not a usable stream.
    pub fn parse_stream(&mut self, r: Ref) -> Option<Rc<Vec<Operation>>> {
        if let Some(cached) = self.parsed.get(&r) {
            return cached.clone();
        }
        let parsed = self.parse_stream_uncached(r).map(Rc::new);
        self.parsed.insert(r, parsed.clone());
        parsed
    }

    fn parse_stream_uncached(&mut self, r: Ref) -> Option<Vec<Operation>> {
        let at = Location::Stream { obj: r, offset: 0 };
        let Object::Stream(s) = self.file.resolve(r) else {
            self.diags.push(DiagnosticKind::BadObject, at);
            return None;
        };
        let before = self.diags.as_slice().len();
        let decoded = s.decode(at, &mut self.diags);
        let unknown = self
            .diags
            .as_slice()
            .get(before..)
            .unwrap_or_default()
            .iter()
            .any(|d| d.kind == DiagnosticKind::UnknownFilter);
        // Undecodable bytes would only yield garbage operators; partial Flate output is kept.
        if unknown {
            return None;
        }
        if decoded.codec.is_some() {
            self.diags.push(DiagnosticKind::UnknownFilter, at);
            return None;
        }
        Some(parse_content(
            &decoded.data,
            Origin::Stream(r),
            &mut self.diags,
        ))
    }

    /// The named entry of a resource category, resolved.
    pub fn resource(&self, category: &[u8], key: &[u8]) -> Option<&'a Object> {
        Some(self.file.deref(self.resource_entry(category, key)?))
    }

    /// The named entry of a resource category as written (possibly a reference).
    pub fn resource_entry(&self, category: &[u8], key: &[u8]) -> Option<&'a Object> {
        let res = (*self.resources.last()?)?;
        self.file.deref(res.get(category)?).as_dict()?.get(key)
    }

    pub fn run(&mut self, ops: &[Operation]) {
        for op in ops {
            if self.ops_left == 0 {
                return;
            }
            self.ops_left -= 1;
            if self.ops_left == 0 {
                self.bad(DiagnosticKind::RecursionLimit, op);
            }
            self.op(op);
        }
    }

    fn bad(&mut self, kind: DiagnosticKind, op: &Operation) {
        self.diags.push(kind, op.location);
    }

    fn op(&mut self, op: &Operation) {
        if let Some(ok) = self.text_op(op) {
            if !ok {
                self.bad(DiagnosticKind::BadOperands, op);
            }
            return;
        }
        let args = op.operands.as_slice();
        let ok = match op.operator.as_slice() {
            b"q" => {
                if self.stack.len() < MAX_Q_DEPTH {
                    self.stack.push(self.gs.clone());
                } else if !self.q_limit_reported {
                    self.q_limit_reported = true;
                    self.bad(DiagnosticKind::RecursionLimit, op);
                }
                true
            }
            b"Q" => {
                let saved = if self.stack.len() > self.stack_floor {
                    self.stack.pop()
                } else {
                    None
                };
                match saved {
                    Some(gs) => self.gs = gs,
                    None => self.bad(DiagnosticKind::UnbalancedRestore, op),
                }
                true
            }
            b"Do" => match name(args) {
                Some(n) => {
                    self.do_xobject(n, op);
                    true
                }
                None => false,
            },
            b"BI" => match args {
                [Object::Stream(s)] => {
                    let s = Stream::new(expand_inline(&s.dict), s.raw().to_vec());
                    self.place_image(&s, None, op.location, None);
                    true
                }
                _ => false,
            },
            b"cm" => nums::<6>(args)
                .map(|m| self.gs.ctm = matrix(m).concat(&self.gs.ctm))
                .is_some(),
            b"w" => nums::<1>(args).map(|[w]| self.gs.line_width = w).is_some(),
            b"gs" => match name(args) {
                Some(n) => {
                    self.ext_gstate(n, op);
                    true
                }
                None => false,
            },
            b"CS" | b"cs" => match name(args) {
                Some(n) => {
                    self.set_space(n, op.operator == b"CS", op);
                    true
                }
                None => false,
            },
            b"SC" | b"SCN" => self.set_color(args, true),
            b"sc" | b"scn" => self.set_color(args, false),
            b"G" | b"g" | b"RG" | b"rg" | b"K" | b"k" => {
                let space = match op.operator.as_slice() {
                    b"G" | b"g" => Space::Gray,
                    b"RG" | b"rg" => Space::Rgb,
                    _ => Space::Cmyk,
                };
                let stroke = op.operator[0].is_ascii_uppercase();
                match space.color(args) {
                    Some(c) => {
                        self.put_color(space, c, stroke);
                        true
                    }
                    None => false,
                }
            }
            b"m" => nums::<2>(args)
                .map(|[x, y]| {
                    let p = Point { x, y };
                    self.path.push(Segment::MoveTo(p));
                    self.current = Some(p);
                    self.subpath_start = Some(p);
                })
                .is_some(),
            b"l" => match (nums::<2>(args), self.current) {
                (Some([x, y]), Some(_)) => {
                    let p = Point { x, y };
                    self.path.push(Segment::LineTo(p));
                    self.current = Some(p);
                    true
                }
                _ => false,
            },
            b"c" | b"v" | b"y" => self.curve(op.operator[0], args),
            b"h" => {
                if self.current.is_some() {
                    self.path.push(Segment::Close);
                    self.current = self.subpath_start;
                }
                true
            }
            b"re" => nums::<4>(args)
                .map(|[x, y, w, h]| {
                    let p = |x, y| Point { x, y };
                    self.path.extend([
                        Segment::MoveTo(p(x, y)),
                        Segment::LineTo(p(x + w, y)),
                        Segment::LineTo(p(x + w, y + h)),
                        Segment::LineTo(p(x, y + h)),
                        Segment::Close,
                    ]);
                    self.current = Some(p(x, y));
                    self.subpath_start = Some(p(x, y));
                })
                .is_some(),
            b"S" => self.paint(false, true, None),
            b"s" => self.paint(true, true, None),
            b"f" | b"F" => self.paint(false, false, Some(FillRule::NonZero)),
            b"f*" => self.paint(false, false, Some(FillRule::EvenOdd)),
            b"B" => self.paint(false, true, Some(FillRule::NonZero)),
            b"B*" => self.paint(false, true, Some(FillRule::EvenOdd)),
            b"b" => self.paint(true, true, Some(FillRule::NonZero)),
            b"b*" => self.paint(true, true, Some(FillRule::EvenOdd)),
            b"n" => {
                self.clear_path();
                true
            }
            b"BX" => {
                self.compat += 1;
                true
            }
            b"EX" => {
                self.compat = self.compat.saturating_sub(1);
                true
            }
            // Clipping (not tracked), marked content, shading, and line style are ignored.
            b"W" | b"W*" | b"sh" | b"BMC" | b"BDC" | b"EMC" | b"MP" | b"DP" | b"d" | b"i"
            | b"j" | b"J" | b"M" | b"ri" | b"d0" | b"d1" => true,
            _ => {
                if self.compat == 0 {
                    self.bad(DiagnosticKind::UnknownOperator, op);
                }
                true
            }
        };
        if !ok {
            self.bad(DiagnosticKind::BadOperands, op);
        }
    }

    fn do_xobject(&mut self, key: &[u8], op: &Operation) {
        let stream = match self.resource_entry(b"XObject", key) {
            Some(Object::Ref(r)) => match self.file.resolve(*r) {
                Object::Stream(s) => Some((*r, s)),
                _ => None,
            },
            _ => None,
        };
        let Some((r, s)) = stream else {
            self.bad(DiagnosticKind::MissingResource, op);
            return;
        };
        match s.dict.get(b"Subtype").and_then(Object::as_name) {
            Some(b"Image") => {
                let name = String::from_utf8_lossy(key).into_owned();
                self.place_image(
                    s,
                    Some(name),
                    Location::Stream { obj: r, offset: 0 },
                    Some(r),
                );
            }
            Some(b"Form") => self.run_form(r, s, op),
            _ => {}
        }
    }

    /// Decodes an image stream, resolving named color spaces in the current resources.
    fn read_image(&mut self, s: &Stream, at: Location) -> Option<ImageParts> {
        let file = self.file;
        let mut diags = std::mem::take(&mut self.diags);
        let mut space = |obj: &Object, diags: &mut Diagnostics| match file.deref(obj) {
            Object::Name(n) => device_space(n.as_bytes()).or_else(|| {
                let cs = self.resource(b"ColorSpace", n.as_bytes())?;
                resolve_space(file, cs, diags)
            }),
            other => resolve_space(file, other, diags),
        };
        let parts = read_image(file, s, at, &mut space, true, &mut diags);
        self.diags = diags;
        parts
    }

    /// Emits one image placement. `name` is `None` for inline images.
    fn place_image(&mut self, s: &Stream, name: Option<String>, at: Location, obj: Option<Ref>) {
        let cached = obj.and_then(|r| self.images.get(&r).cloned());
        let parts = match cached {
            Some(parts) => parts,
            None => {
                let parts = self.read_image(s, at).map(Rc::new);
                if let Some(r) = obj {
                    self.images.insert(r, parts.clone());
                }
                parts
            }
        };
        let Some(p) = parts else {
            return;
        };
        let bytes = p.data_len();
        if bytes > self.image_bytes_left {
            if !self.image_budget_reported {
                self.image_budget_reported = true;
                self.diags.push(DiagnosticKind::BadImage, at);
            }
            return;
        }
        self.image_bytes_left -= bytes;
        // ponytail: each placement owns a copy of the decoded data, bounded by the per-pass
        // byte budget; share it (Arc in the public type) if memory matters.
        let p = ImageParts::clone(&p);
        let ctm = self.gs.ctm;
        self.out.images.push(Image {
            inline: name.is_none(),
            name,
            ctm,
            bbox: ctm.transform_rect(Rect {
                x0: 0.,
                y0: 0.,
                x1: 1.,
                y1: 1.,
            }),
            width: p.width,
            height: p.height,
            bpc: p.bpc,
            color_space: p.color_space,
            image_mask: p.image_mask,
            decode: p.decode,
            data: p.data,
            smask: p.smask,
        });
    }

    /// Draws a Form XObject (§8.10) with its own `/Matrix` and resources.
    fn run_form(&mut self, r: Ref, s: &'a Stream, op: &Operation) {
        if self.form_stack.contains(&r) || self.form_stack.len() >= MAX_FORM_DEPTH {
            self.bad(DiagnosticKind::RecursionLimit, op);
            return;
        }
        let file = self.file;
        let m = s
            .dict
            .get(b"Matrix")
            .and_then(|m| numbers::<6>(file, m))
            .map(matrix)
            .unwrap_or(Matrix::IDENTITY);
        let res = match s
            .dict
            .get(b"Resources")
            .and_then(|o| file.deref(o).as_dict())
        {
            Some(d) => Some(d),
            None => self.resources.last().copied().flatten(),
        };
        let Some(ops) = self.parse_stream(r) else {
            return;
        };
        let saved = (
            self.gs.clone(),
            self.stack_floor,
            self.in_text,
            self.tm,
            self.tlm,
        );
        self.stack_floor = self.stack.len();
        self.gs.ctm = m.concat(&self.gs.ctm);
        self.resources.push(res);
        self.form_stack.push(r);
        self.run(&ops);
        self.form_stack.pop();
        self.resources.pop();
        self.stack.truncate(self.stack_floor);
        (self.gs, self.stack_floor, self.in_text, self.tm, self.tlm) = saved;
    }

    fn curve(&mut self, kind: u8, args: &[Object]) -> bool {
        let Some(cur) = self.current else {
            return false;
        };
        let p = |x, y| Point { x, y };
        let seg = match kind {
            b'c' => nums::<6>(args).map(|[a, b, c, d, e, f]| (p(a, b), p(c, d), p(e, f))),
            b'v' => nums::<4>(args).map(|[c, d, e, f]| (cur, p(c, d), p(e, f))),
            _ => nums::<4>(args).map(|[a, b, e, f]| (p(a, b), p(e, f), p(e, f))),
        };
        match seg {
            Some((c1, c2, end)) => {
                self.path.push(Segment::CurveTo(c1, c2, end));
                self.current = Some(end);
                true
            }
            None => false,
        }
    }

    fn clear_path(&mut self) {
        self.path.clear();
        self.current = None;
        self.subpath_start = None;
    }

    fn paint(&mut self, close: bool, stroke: bool, fill: Option<FillRule>) -> bool {
        if close && self.path.last().is_some_and(|s| *s != Segment::Close) {
            self.path.push(Segment::Close);
        }
        if !self.path.is_empty() {
            let ctm = self.gs.ctm;
            let t = |p: Point| ctm.apply(p);
            let segments = self
                .path
                .iter()
                .map(|s| match *s {
                    Segment::MoveTo(p) => Segment::MoveTo(t(p)),
                    Segment::LineTo(p) => Segment::LineTo(t(p)),
                    Segment::CurveTo(a, b, c) => Segment::CurveTo(t(a), t(b), t(c)),
                    Segment::Close => Segment::Close,
                })
                .collect();
            self.out.paths.push(Path {
                segments,
                stroke,
                fill,
                line_width: self.gs.line_width,
                ctm,
                fill_color: self.gs.fill.clone(),
                stroke_color: self.gs.stroke.clone(),
                fill_alpha: self.gs.fill_alpha,
                stroke_alpha: self.gs.stroke_alpha,
            });
        }
        self.clear_path();
        true
    }

    fn ext_gstate(&mut self, key: &[u8], op: &Operation) {
        let Some(dict) = self.resource(b"ExtGState", key).and_then(Object::as_dict) else {
            self.bad(DiagnosticKind::MissingResource, op);
            return;
        };
        let file = self.file;
        let num = |k: &[u8]| {
            dict.get(k)
                .and_then(|o| file.deref(o).as_number())
                .filter(|n| n.is_finite())
        };
        if let Some(w) = num(b"LW") {
            self.gs.line_width = w;
        }
        if let Some(a) = num(b"ca") {
            self.gs.fill_alpha = a;
        }
        if let Some(a) = num(b"CA") {
            self.gs.stroke_alpha = a;
        }
        if let Some([font, size]) = dict
            .get(b"Font")
            .and_then(|f| file.deref(f).as_array())
            .and_then(|a| <&[Object; 2]>::try_from(a).ok())
            && let Some(size) = file.deref(size).as_number().filter(|s| s.is_finite())
        {
            self.set_font_ref(font, size, op.location);
        }
    }

    fn set_space(&mut self, key: &[u8], stroke: bool, op: &Operation) {
        let space = match device_space(key) {
            Some(s) => Some(s),
            None => match self.resource(b"ColorSpace", key) {
                Some(obj) => resolve_space(self.file, obj, &mut self.diags),
                None => None,
            },
        };
        match space {
            Some(s) => {
                let c = s.initial_color();
                self.put_color(s, c, stroke);
            }
            None => self.bad(DiagnosticKind::MissingResource, op),
        }
    }

    fn set_color(&mut self, args: &[Object], stroke: bool) -> bool {
        let space = if stroke {
            &self.gs.stroke_space
        } else {
            &self.gs.fill_space
        };
        match space.color(args) {
            Some(c) => {
                if stroke {
                    self.gs.stroke = c;
                } else {
                    self.gs.fill = c;
                }
                true
            }
            None => false,
        }
    }

    fn put_color(&mut self, space: Space, color: crate::model::Color, stroke: bool) {
        if stroke {
            self.gs.stroke_space = space;
            self.gs.stroke = color;
        } else {
            self.gs.fill_space = space;
            self.gs.fill = color;
        }
    }
}
