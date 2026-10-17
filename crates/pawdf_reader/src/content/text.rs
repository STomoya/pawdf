//! Text objects, text state, and glyph geometry (§9.3, §9.4).

use std::sync::Arc;

use pawdf_core::{DiagnosticKind, Location, Object, Operation};

use crate::content::Interp;
use crate::content::state::{FontKey, matrix, nums};
use crate::font::{Font, load_font};
use crate::geom::{Matrix, Point, Rect};
use crate::model::{Glyph, RenderMode, WritingMode};

fn translate(tx: f64, ty: f64) -> Matrix {
    Matrix {
        e: tx,
        f: ty,
        ..Matrix::IDENTITY
    }
}

impl<'a> Interp<'a> {
    /// Handles text operators. `None` if `op` is not one; otherwise whether its operands
    /// were usable.
    pub(crate) fn text_op(&mut self, op: &Operation) -> Option<bool> {
        let args = op.operands.as_slice();
        let ok = match op.operator.as_slice() {
            b"BT" => {
                if self.in_text {
                    self.diags
                        .push(DiagnosticKind::MisplacedOperator, op.location);
                }
                self.in_text = true;
                self.tm = Matrix::IDENTITY;
                self.tlm = Matrix::IDENTITY;
                true
            }
            b"ET" => {
                if !self.in_text {
                    self.diags
                        .push(DiagnosticKind::UnbalancedRestore, op.location);
                }
                self.in_text = false;
                true
            }
            b"Tc" => nums::<1>(args)
                .map(|[v]| self.gs.text.char_spacing = v)
                .is_some(),
            b"Tw" => nums::<1>(args)
                .map(|[v]| self.gs.text.word_spacing = v)
                .is_some(),
            b"Tz" => nums::<1>(args)
                .map(|[v]| self.gs.text.hscale = v / 100.)
                .is_some(),
            b"TL" => nums::<1>(args)
                .map(|[v]| self.gs.text.leading = v)
                .is_some(),
            b"Ts" => nums::<1>(args).map(|[v]| self.gs.text.rise = v).is_some(),
            b"Tr" => match args.first().and_then(Object::as_int).and_then(render_mode) {
                Some(m) if args.len() == 1 => {
                    self.gs.text.render_mode = m;
                    true
                }
                _ => false,
            },
            b"Tf" => match args {
                [Object::Name(n), size] => match size.as_number().filter(|s| s.is_finite()) {
                    Some(size) => {
                        let font = self.named_font(n.as_bytes(), op.location);
                        self.gs.text.font = Some(font);
                        self.gs.text.size = size;
                        true
                    }
                    None => false,
                },
                _ => false,
            },
            b"Td" | b"TD" | b"Tm" | b"T*" | b"Tj" | b"TJ" | b"'" | b"\"" => {
                if !self.in_text {
                    self.diags
                        .push(DiagnosticKind::MisplacedOperator, op.location);
                }
                self.positioning_or_show(op)
            }
            _ => return None,
        };
        Some(ok)
    }

    fn positioning_or_show(&mut self, op: &Operation) -> bool {
        let args = op.operands.as_slice();
        match op.operator.as_slice() {
            b"Td" => nums::<2>(args).map(|[x, y]| self.next_line(x, y)).is_some(),
            b"TD" => nums::<2>(args)
                .map(|[x, y]| {
                    self.gs.text.leading = -y;
                    self.next_line(x, y);
                })
                .is_some(),
            b"Tm" => nums::<6>(args)
                .map(|m| {
                    self.tm = matrix(m);
                    self.tlm = self.tm;
                })
                .is_some(),
            b"T*" => {
                self.next_line(0., -self.gs.text.leading);
                true
            }
            b"Tj" => match args {
                [Object::String(s)] => {
                    self.show(s, op.location);
                    true
                }
                _ => false,
            },
            b"'" => match args {
                [Object::String(s)] => {
                    self.next_line(0., -self.gs.text.leading);
                    self.show(s, op.location);
                    true
                }
                _ => false,
            },
            b"\"" => match args {
                [aw, ac, Object::String(s)] => {
                    match (aw.as_number(), ac.as_number()) {
                        (Some(aw), Some(ac)) if aw.is_finite() && ac.is_finite() => {
                            self.gs.text.word_spacing = aw;
                            self.gs.text.char_spacing = ac;
                        }
                        _ => return false,
                    }
                    self.next_line(0., -self.gs.text.leading);
                    self.show(s, op.location);
                    true
                }
                _ => false,
            },
            _ => match args {
                [Object::Array(items)] => {
                    for item in items {
                        match item {
                            Object::String(s) => self.show(s, op.location),
                            o => {
                                if let Some(n) = o.as_number().filter(|n| n.is_finite()) {
                                    let ts = &self.gs.text;
                                    let vertical = ts.font.as_ref().is_some_and(|(_, f)| {
                                        f.info.wmode == WritingMode::Vertical
                                    });
                                    // §9.4.4: in vertical writing the number moves along y.
                                    let shift = if vertical {
                                        translate(0., -n / 1000. * ts.size)
                                    } else {
                                        translate(-n / 1000. * ts.size * ts.hscale, 0.)
                                    };
                                    self.tm = shift.concat(&self.tm);
                                }
                            }
                        }
                    }
                    true
                }
                _ => false,
            },
        }
    }

    fn next_line(&mut self, x: f64, y: f64) {
        self.tlm = translate(x, y).concat(&self.tlm);
        self.tm = self.tlm;
    }

    /// The font a `Tf` (or `gs /Font`) name selects; the fallback if it is missing.
    fn named_font(&mut self, key: &[u8], at: Location) -> (FontKey, Arc<Font>) {
        match self.resource_entry(b"Font", key) {
            Some(Object::Ref(r)) => (FontKey::Ref(*r), self.ref_font(*r, at)),
            Some(Object::Dict(d)) => {
                let key = FontKey::Direct(d as *const _ as usize);
                if let Some(f) = self.pass_fonts.get(&key) {
                    return (key, Arc::clone(f));
                }
                let font = Arc::new(load_font(
                    self.file,
                    &self.doc.shared,
                    d,
                    at,
                    &mut self.diags,
                ));
                self.pass_fonts.insert(key, Arc::clone(&font));
                (key, font)
            }
            _ => {
                self.diags.push(DiagnosticKind::MissingResource, at);
                (FontKey::Fallback, Arc::clone(&self.fallback))
            }
        }
    }

    /// A document-cached font, reporting its load diagnostics once per pass.
    fn ref_font(&mut self, r: pawdf_core::Ref, at: Location) -> Arc<Font> {
        let key = FontKey::Ref(r);
        if let Some(f) = self.pass_fonts.get(&key) {
            return Arc::clone(f);
        }
        let (font, diags) = self.doc.font(r, at);
        for d in diags {
            self.diags.push(d.kind, d.location);
        }
        self.pass_fonts.insert(key, Arc::clone(&font));
        font
    }

    pub(crate) fn set_font_ref(&mut self, font: &Object, size: f64, at: Location) {
        let selected = match font {
            Object::Ref(r) => (FontKey::Ref(*r), self.ref_font(*r, at)),
            _ => {
                self.diags.push(DiagnosticKind::BadFont, at);
                (FontKey::Fallback, Arc::clone(&self.fallback))
            }
        };
        self.gs.text.font = Some(selected);
        self.gs.text.size = size;
    }

    fn show(&mut self, s: &[u8], at: Location) {
        let (key, font) = match &self.gs.text.font {
            Some(f) => f.clone(),
            None => {
                if !self.no_font_reported {
                    self.no_font_reported = true;
                    self.diags.push(DiagnosticKind::MissingResource, at);
                }
                (FontKey::Fallback, Arc::clone(&self.fallback))
            }
        };
        let index = match self.font_index.get(&key) {
            Some(&i) => i,
            None => {
                self.out.fonts.push(font.info.clone());
                let i = self.out.fonts.len() - 1;
                self.font_index.insert(key, i);
                i
            }
        };
        let ts = self.gs.text.clone();
        let ctm = self.gs.ctm;
        // One past the budget, so running out is still reported.
        for code in font.codes(s, self.glyphs_left.saturating_add(1)) {
            if self.glyphs_left == 0 {
                if !self.glyph_budget_reported {
                    self.glyph_budget_reported = true;
                    self.diags.push(DiagnosticKind::RecursionLimit, at);
                }
                return;
            }
            self.glyphs_left -= 1;
            let w0 = font.width(code);
            let scale = Matrix {
                a: ts.size * ts.hscale,
                d: ts.size,
                f: ts.rise,
                ..Matrix::IDENTITY
            };
            let user = self.tm.concat(&ctm);
            let trm = scale.concat(&user);
            if !code.matched && self.bad_code_reported.insert(key) {
                self.diags.push(DiagnosticKind::BadCMap, at);
            }
            // §9.3.3: word spacing applies to the single-byte code 32 only.
            let tw = if code.len == 1 && code.value == 32 {
                ts.word_spacing
            } else {
                0.
            };
            // §9.4.4, §9.7.4.3. In vertical writing the origin pdfium reports (and the
            // glyph matrix's origin) is the horizontal origin, displaced by -v from the
            // current point.
            let (matrix, shift, bbox, advance) = match font.vertical(code) {
                None => {
                    let tx = (w0 * ts.size + ts.char_spacing + tw) * ts.hscale;
                    let bbox = Rect {
                        x0: 0.,
                        y0: font.descent,
                        x1: w0,
                        y1: font.ascent,
                    };
                    (
                        trm,
                        translate(tx, 0.),
                        bbox,
                        tx.abs() * user.a.hypot(user.b),
                    )
                }
                Some(v) => {
                    let ty = v.w1 * ts.size + ts.char_spacing + tw;
                    let bbox = Rect {
                        x0: 0.,
                        y0: v.vy + v.w1,
                        x1: w0,
                        y1: v.vy,
                    };
                    let matrix = translate(-v.vx, -v.vy).concat(&trm);
                    (
                        matrix,
                        translate(0., ty),
                        bbox,
                        ty.abs() * user.c.hypot(user.d),
                    )
                }
            };
            let text = match font.unicode(code) {
                Some(t) => t,
                None => {
                    if self.unmapped_reported.insert(key) {
                        self.diags.push(DiagnosticKind::UnmappedGlyph, at);
                    }
                    char::REPLACEMENT_CHARACTER.to_string()
                }
            };
            self.out.glyphs.push(Glyph {
                text,
                code: code.value,
                font: index,
                size: ts.size,
                matrix,
                origin: matrix.apply(Point { x: 0., y: 0. }),
                advance,
                bbox: matrix.transform_rect(bbox),
                wmode: font.info.wmode,
                render_mode: ts.render_mode,
                fill: self.gs.fill.clone(),
                stroke: self.gs.stroke.clone(),
                fill_alpha: self.gs.fill_alpha,
                stroke_alpha: self.gs.stroke_alpha,
            });
            self.tm = shift.concat(&self.tm);
        }
    }
}

fn render_mode(n: i64) -> Option<RenderMode> {
    Some(match n {
        0 => RenderMode::Fill,
        1 => RenderMode::Stroke,
        2 => RenderMode::FillStroke,
        3 => RenderMode::Invisible,
        4 => RenderMode::FillClip,
        5 => RenderMode::StrokeClip,
        6 => RenderMode::FillStrokeClip,
        7 => RenderMode::Clip,
        _ => return None,
    })
}
