//! Fonts (§9.5–§9.7): simple fonts here, composite fonts in [`cid`].

pub(crate) mod cid;
pub(crate) mod encoding;
pub(crate) mod truetype;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use pawdf_core::{DiagnosticKind, Diagnostics, Dict, File, Location, Object, Origin, Ref};

use crate::cmap::{CMap, Code, MAX_USECMAP_DEPTH, UseCMap};
use crate::data::{BaseEncoding, encoding_names, predefined_cmap, std14_width};
use crate::document::numbers;
use crate::geom::Matrix;
use crate::model::{FontInfo, FontKind, WritingMode};
use cid::{Cid, Vertical, load_type0};
use encoding::{cff_builtin, glyph_unicode, type1_builtin};

pub(crate) const GLYPH_SCALE: Matrix = Matrix {
    a: 0.001,
    b: 0.,
    c: 0.,
    d: 0.001,
    e: 0.,
    f: 0.,
};

/// Parsed streams shared by the fonts of one document, keyed by stream object, so fonts
/// that share a CMap or a font program parse it once.
#[derive(Debug, Default)]
pub(crate) struct Shared {
    pub(crate) cmaps: Mutex<HashMap<Ref, Arc<CMap>>>,
    pub(crate) programs: Mutex<HashMap<Ref, Option<Arc<truetype::TrueType>>>>,
}

/// A loaded font: per-code widths and Unicode, plus the metrics glyph boxes need.
#[derive(Debug, Clone)]
pub(crate) struct Font {
    pub info: FontInfo,
    /// Ascent and descent in text space at size 1.
    pub ascent: f64,
    pub descent: f64,
    glyphs: Glyphs,
}

#[derive(Debug, Clone)]
enum Glyphs {
    /// Text-space widths at size 1 and Unicode, per 1-byte code.
    Simple {
        widths: Vec<f64>,
        unicode: Vec<Option<String>>,
    },
    Composite(Box<Cid>),
}

impl Font {
    /// The first `limit` character codes in a string: one byte each for simple fonts,
    /// split by the CMap's codespace for composite fonts (§9.7.6.2).
    pub fn codes(&self, s: &[u8], limit: usize) -> Vec<Code> {
        match &self.glyphs {
            Glyphs::Simple { .. } => s
                .iter()
                .take(limit)
                .map(|&b| Code {
                    value: u32::from(b),
                    len: 1,
                    matched: true,
                })
                .collect(),
            Glyphs::Composite(c) => {
                let mut out = Vec::new();
                let mut rest = s;
                while !rest.is_empty() && out.len() < limit {
                    let code = c.cmap.next_code(rest);
                    out.push(code);
                    rest = rest.get(usize::from(code.len).max(1)..).unwrap_or_default();
                }
                out
            }
        }
    }

    /// Horizontal displacement at size 1, in text space.
    pub fn width(&self, code: Code) -> f64 {
        match &self.glyphs {
            Glyphs::Simple { widths, .. } => widths.get(code.value as usize).copied().unwrap_or(0.),
            Glyphs::Composite(c) => c.width(c.cmap.cid(code)),
        }
    }

    /// Vertical metrics for fonts in vertical writing mode, else `None`.
    pub fn vertical(&self, code: Code) -> Option<Vertical> {
        match &self.glyphs {
            Glyphs::Composite(c) if self.info.wmode == WritingMode::Vertical => {
                Some(c.vertical(c.cmap.cid(code)))
            }
            _ => None,
        }
    }

    /// Unicode text for a code, if known.
    pub fn unicode(&self, code: Code) -> Option<String> {
        match &self.glyphs {
            Glyphs::Simple { unicode, .. } => unicode.get(code.value as usize)?.clone(),
            Glyphs::Composite(c) => c.unicode(code, c.cmap.cid(code)),
        }
    }

    /// Helvetica with StandardEncoding, for text shown without a usable font.
    pub fn fallback() -> Font {
        let names = names_of(encoding_names(BaseEncoding::Standard));
        Font {
            info: FontInfo {
                base_font: "Helvetica".into(),
                kind: FontKind::Type1,
                embedded: false,
                wmode: WritingMode::Horizontal,
            },
            ascent: 0.8,
            descent: -0.2,
            glyphs: Glyphs::Simple {
                widths: names
                    .iter()
                    .map(|n| glyph_width("Helvetica", n.as_deref()) / 1000.)
                    .collect(),
                unicode: names
                    .iter()
                    .map(|n| n.as_deref().and_then(glyph_unicode))
                    .collect(),
            },
        }
    }
}

/// Glyph space to text space. Dividing by 1000 for the default scale keeps widths such
/// as 350 exactly 0.35.
fn to_text(v: f64, scale: f64) -> f64 {
    if scale == 0.001 { v / 1000. } else { v * scale }
}

fn names_of(table: &[Option<&'static str>; 256]) -> Vec<Option<String>> {
    table.iter().map(|n| n.map(str::to_owned)).collect()
}

fn glyph_width(std14: &str, name: Option<&str>) -> f64 {
    name.and_then(|n| std14_width(std14, n)).unwrap_or(0.)
}

/// Loads a simple font dictionary (§9.6). Problems become `BadFont` at `at`.
pub(crate) fn load_font(
    file: &File,
    shared: &Shared,
    dict: &Dict,
    at: Location,
    diags: &mut Diagnostics,
) -> Font {
    let get = |k: &[u8]| dict.get(k).map(|o| file.deref(o));
    let kind = match get(b"Subtype").and_then(Object::as_name) {
        Some(b"Type1") => FontKind::Type1,
        Some(b"MMType1") => FontKind::MmType1,
        Some(b"TrueType") => FontKind::TrueType,
        Some(b"Type3") => FontKind::Type3,
        Some(b"Type0") => return load_type0(file, shared, dict, at, diags),
        _ => {
            diags.push(DiagnosticKind::BadFont, at);
            FontKind::Type1
        }
    };
    let base_font = get(b"BaseFont")
        .and_then(Object::as_name)
        .map(|n| String::from_utf8_lossy(n).into_owned())
        .unwrap_or_default();
    let desc = get(b"FontDescriptor").and_then(Object::as_dict);
    let dget = |k: &[u8]| desc.and_then(|d| d.get(k)).map(|o| file.deref(o));
    let font_file = [b"FontFile".as_slice(), b"FontFile2", b"FontFile3"]
        .iter()
        .find_map(|k| desc.and_then(|d| d.get(k)));
    let embedded = kind == FontKind::Type3 || font_file.is_some();
    let font_matrix = match kind {
        FontKind::Type3 => get(b"FontMatrix")
            .and_then(|m| numbers::<6>(file, m))
            .map(|[a, b, c, d, e, f]| Matrix { a, b, c, d, e, f })
            .unwrap_or(GLYPH_SCALE),
        _ => GLYPH_SCALE,
    };
    let std14 = std14_name(&base_font);
    let symbolic = dget(b"Flags")
        .and_then(Object::as_int)
        .is_some_and(|f| f & 4 != 0);

    let builtin = |diags: &mut Diagnostics| -> Vec<Option<String>> {
        match std14 {
            Some("Symbol") => return names_of(encoding_names(BaseEncoding::Symbol)),
            Some("ZapfDingbats") => return names_of(encoding_names(BaseEncoding::ZapfDingbats)),
            _ => {}
        }
        if matches!(kind, FontKind::Type1 | FontKind::MmType1)
            && let Some(Object::Ref(r)) = desc.and_then(|d| d.get(b"FontFile"))
            && let Object::Stream(s) = file.resolve(*r)
        {
            let program = s.decode(Location::Stream { obj: *r, offset: 0 }, diags);
            if let Some(t) = type1_builtin(&program.data) {
                return t;
            }
        }
        if matches!(kind, FontKind::Type1 | FontKind::MmType1)
            && let Some(Object::Ref(r)) = desc.and_then(|d| d.get(b"FontFile3"))
            && let Object::Stream(s) = file.resolve(*r)
            && let Some(sub @ (b"Type1C" | b"OpenType")) = s
                .dict
                .get(b"Subtype")
                .map(|o| file.deref(o))
                .and_then(Object::as_name)
        {
            let program = s.decode(Location::Stream { obj: *r, offset: 0 }, diags);
            match cff_builtin(&program.data, sub == b"OpenType") {
                Some(t) => return t,
                None => diags.push(DiagnosticKind::BadFont, at),
            }
        }
        names_of(encoding_names(BaseEncoding::Standard))
    };
    let named = |n: &[u8]| match n {
        b"StandardEncoding" => Some(BaseEncoding::Standard),
        b"WinAnsiEncoding" => Some(BaseEncoding::WinAnsi),
        b"MacRomanEncoding" => Some(BaseEncoding::MacRoman),
        b"MacExpertEncoding" => Some(BaseEncoding::MacExpert),
        _ => None,
    };
    let mut names = match get(b"Encoding") {
        // §9.6.5: the built-in encoding is what draws the glyphs. pdfium uses StandardEncoding
        // for nonsymbolic fonts here; this deviation is deliberate.
        None => builtin(diags),
        Some(Object::Name(n)) => match named(n.as_bytes()) {
            Some(e) => names_of(encoding_names(e)),
            None => {
                diags.push(DiagnosticKind::BadFont, at);
                builtin(diags)
            }
        },
        Some(Object::Dict(e)) => {
            let base = e.get(b"BaseEncoding").map(|o| file.deref(o));
            let mut names = match base.and_then(Object::as_name).and_then(named) {
                Some(b) => names_of(encoding_names(b)),
                // §9.6.5.1 says an embedded program's built-in encoding is the base, but
                // pdfium uses StandardEncoding for every nonsymbolic font; we follow pdfium.
                None if symbolic || matches!(std14, Some("Symbol" | "ZapfDingbats")) => {
                    builtin(diags)
                }
                None => names_of(encoding_names(BaseEncoding::Standard)),
            };
            if let Some(diffs) = e
                .get(b"Differences")
                .map(|o| file.deref(o))
                .and_then(Object::as_array)
            {
                apply_differences(&mut names, diffs, file);
            }
            names
        }
        Some(_) => {
            diags.push(DiagnosticKind::BadFont, at);
            builtin(diags)
        }
    };
    names.resize(256, None);

    let scale = font_matrix.a;
    let first = get(b"FirstChar").and_then(Object::as_int).unwrap_or(0);
    let widths_arr = get(b"Widths").and_then(Object::as_array);
    let missing = dget(b"MissingWidth").and_then(Object::as_number);
    let widths = (0..256i64)
        .map(|code| {
            let listed = widths_arr
                .and_then(|w| {
                    code.checked_sub(first)
                        .and_then(|i| usize::try_from(i).ok())
                        .and_then(|i| w.get(i))
                })
                .and_then(|o| file.deref(o).as_number());
            let w = listed.or(missing).or_else(|| {
                let name = names.get(code as usize).and_then(|n| n.as_deref());
                std14.and_then(|f| name.and_then(|n| std14_width(f, n)))
            });
            to_text(w.filter(|w| w.is_finite()).unwrap_or(0.), scale)
        })
        .collect();

    let to_unicode = match dict.get(b"ToUnicode") {
        Some(Object::Ref(r)) => embedded_cmap(file, shared, *r, diags),
        _ => None,
    };
    // §9.6.5.4: without /Encoding, a TrueType font's own cmap selects the glyphs.
    let builtin_cmap = match (kind, dict.get(b"Encoding")) {
        (FontKind::TrueType, None) => truetype::load(file, shared, desc, at, diags),
        _ => None,
    };
    let unicode = (0..=255u8)
        .map(|code| {
            to_unicode
                .as_ref()
                .and_then(|t| t.lookup(&[code]))
                .or_else(|| {
                    let tt = builtin_cmap.as_ref()?;
                    tt.unicode(tt.simple_gid(code)?).map(str::to_owned)
                })
                .or_else(|| {
                    names
                        .get(usize::from(code))?
                        .as_deref()
                        .and_then(glyph_unicode)
                })
        })
        .collect();

    let (ascent, descent) = metrics(file, kind, &font_matrix, dict, desc);
    Font {
        info: FontInfo {
            base_font,
            kind,
            embedded,
            wmode: WritingMode::Horizontal,
        },
        ascent,
        descent,
        glyphs: Glyphs::Simple { widths, unicode },
    }
}

/// An embedded CMap stream, following `/UseCMap` and `usecmap` to other streams or
/// predefined names. `None` if `r` is not a stream. A `usecmap` cycle is cut on its
/// first repeat, and chains deeper than `MAX_USECMAP_DEPTH` are cut.
pub(crate) fn embedded_cmap(
    file: &File,
    shared: &Shared,
    r: Ref,
    diags: &mut Diagnostics,
) -> Option<Arc<CMap>> {
    embedded_cmap_in(file, shared, r, &mut Vec::new(), &mut false, diags)
}

/// `visiting` holds the streams of the chain being parsed. `cut` is set when a cycle or
/// the depth limit truncated the chain below `r`.
fn embedded_cmap_in(
    file: &File,
    shared: &Shared,
    r: Ref,
    visiting: &mut Vec<Ref>,
    cut: &mut bool,
    diags: &mut Diagnostics,
) -> Option<Arc<CMap>> {
    let lock = || shared.cmaps.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(m) = lock().get(&r) {
        return Some(Arc::clone(m));
    }
    let at = Location::Stream { obj: r, offset: 0 };
    if visiting.contains(&r) || visiting.len() > MAX_USECMAP_DEPTH {
        diags.push(DiagnosticKind::RecursionLimit, at);
        *cut = true;
        return None;
    }
    let Object::Stream(s) = file.resolve(r) else {
        return None;
    };
    let decoded = s.decode(at, diags);
    let base = match s.dict.get(b"UseCMap") {
        Some(Object::Ref(b)) => Some(UseCMap::Stream(*b)),
        Some(Object::Name(n)) => Some(UseCMap::Name(n.as_bytes())),
        _ => None,
    };
    let depth = visiting.len();
    visiting.push(r);
    let mut resolve = |u: UseCMap, _: usize, diags: &mut Diagnostics| match u {
        UseCMap::Stream(b) => embedded_cmap_in(file, shared, b, visiting, cut, diags),
        UseCMap::Name(n) => predefined_cmap(n),
    };
    let m = Arc::new(CMap::parse_based(
        &decoded.data,
        Origin::Stream(r),
        depth,
        base,
        &mut resolve,
        diags,
    ));
    visiting.pop();
    // A map built with its parent cut depends on where the chain was entered, so caching
    // it would hand a truncated map to a font that points at it directly.
    if !*cut {
        lock().insert(r, Arc::clone(&m));
    }
    Some(m)
}

fn apply_differences(names: &mut [Option<String>], diffs: &[Object], file: &File) {
    let mut code: Option<usize> = None;
    for item in diffs {
        match file.deref(item) {
            Object::Int(n) => code = usize::try_from(*n).ok(),
            Object::Name(n) => {
                if let Some(c) = code {
                    if let Some(slot) = names.get_mut(c) {
                        *slot = Some(String::from_utf8_lossy(n.as_bytes()).into_owned());
                    }
                    code = Some(c + 1);
                }
            }
            _ => {}
        }
    }
}

/// Ascent and descent at size 1: descriptor values, then `/FontBBox`, then a default.
pub(crate) fn metrics(
    file: &File,
    kind: FontKind,
    fm: &Matrix,
    dict: &Dict,
    desc: Option<&Dict>,
) -> (f64, f64) {
    let num = |d: Option<&Dict>, k: &[u8]| {
        d.and_then(|d| d.get(k))
            .and_then(|o| file.deref(o).as_number())
            .filter(|n| n.is_finite())
    };
    let (a, d) = (num(desc, b"Ascent"), num(desc, b"Descent"));
    if kind != FontKind::Type3
        && let (Some(a), Some(d)) = (a, d)
        && (a != 0. || d != 0.)
    {
        return (a / 1000., d / 1000.);
    }
    let bbox = if kind == FontKind::Type3 {
        dict.get(b"FontBBox")
    } else {
        desc.and_then(|d| d.get(b"FontBBox"))
    };
    match bbox.and_then(|b| numbers::<4>(file, b)) {
        Some([_, lly, _, ury]) if ury != lly => (to_text(ury, fm.d), to_text(lly, fm.d)),
        _ => (0.8, -0.2),
    }
}

/// The standard-14 font a `/BaseFont` name stands for, if any (§9.6.2.2), after
/// stripping a subset tag and mapping common aliases such as Arial.
pub(crate) fn std14_name(base_font: &str) -> Option<&'static str> {
    let name = match base_font.split_once('+') {
        Some((tag, rest)) if tag.len() == 6 && tag.bytes().all(|b| b.is_ascii_uppercase()) => rest,
        _ => base_font,
    };
    let name: String = name.chars().filter(|c| *c != ' ').collect();
    match name.as_str() {
        "Symbol" => return Some("Symbol"),
        "ZapfDingbats" => return Some("ZapfDingbats"),
        "Times-Roman" => return Some("Times-Roman"),
        _ => {}
    }
    let (family, style) = match name.find([',', '-']) {
        Some(i) => (&name[..i], &name[i + 1..]),
        None => (name.as_str(), ""),
    };
    let family = match family {
        "Arial" | "ArialMT" | "Helvetica" => "Helvetica",
        "TimesNewRoman" | "TimesNewRomanPSMT" | "Times" => "Times",
        "CourierNew" | "CourierNewPSMT" | "Courier" => "Courier",
        _ => return None,
    };
    let bold = style.contains("Bold");
    let italic = style.contains("Italic") || style.contains("Oblique");
    Some(match (family, bold, italic) {
        ("Helvetica", false, false) => "Helvetica",
        ("Helvetica", true, false) => "Helvetica-Bold",
        ("Helvetica", false, true) => "Helvetica-Oblique",
        ("Helvetica", true, true) => "Helvetica-BoldOblique",
        ("Times", false, false) => "Times-Roman",
        ("Times", true, false) => "Times-Bold",
        ("Times", false, true) => "Times-Italic",
        ("Times", true, true) => "Times-BoldItalic",
        ("Courier", false, false) => "Courier",
        ("Courier", true, false) => "Courier-Bold",
        ("Courier", false, true) => "Courier-Oblique",
        _ => "Courier-BoldOblique",
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{file, stream};
    use pawdf_core::{Location, Ref};

    fn load(objects: &[(u32, &[u8])]) -> (Font, Vec<pawdf_core::DiagnosticKind>) {
        let f = file(objects);
        let shared = Shared::default();
        let (font, d) = load_with(&f, &shared);
        (font, d)
    }

    fn load_with(f: &File, shared: &Shared) -> (Font, Vec<pawdf_core::DiagnosticKind>) {
        load_num(f, shared, 5)
    }

    fn load_num(f: &File, shared: &Shared, num: u32) -> (Font, Vec<pawdf_core::DiagnosticKind>) {
        let r = Ref { num, generation: 0 };
        let dict = f.resolve(r).as_dict().unwrap().clone();
        let mut d = Diagnostics::new();
        let font = load_font(f, shared, &dict, Location::File(0), &mut d);
        (font, d.as_slice().iter().map(|x| x.kind).collect())
    }

    fn code(c: u8) -> Code {
        Code {
            value: u32::from(c),
            len: 1,
            matched: true,
        }
    }

    fn text(font: &Font, c: u8) -> Option<String> {
        font.unicode(code(c))
    }

    #[test]
    fn std14_names() {
        for (i, o) in [
            ("Helvetica", "Helvetica"),
            ("ABCDEF+Arial,Bold", "Helvetica-Bold"),
            ("Arial-BoldItalicMT", "Helvetica-BoldOblique"),
            ("Arial-ItalicMT", "Helvetica-Oblique"),
            ("TimesNewRomanPSMT", "Times-Roman"),
            ("TimesNewRoman,Italic", "Times-Italic"),
            ("Times-BoldItalic", "Times-BoldItalic"),
            ("Times New Roman,Bold", "Times-Bold"),
            ("CourierNew", "Courier"),
            ("Courier-Oblique", "Courier-Oblique"),
            ("Symbol", "Symbol"),
            ("ZapfDingbats", "ZapfDingbats"),
        ] {
            assert_eq!(std14_name(i), Some(o), "{i}");
        }
        assert_eq!(std14_name("CMR10"), None);
        assert_eq!(std14_name("ABCDEF+CMR10"), None);
    }

    #[test]
    fn widths_then_missing_width_then_std14() {
        let (f, d) = load(&[
            (5, b"<< /Type /Font /Subtype /Type1 /BaseFont /Foo /FirstChar 65 /Widths [500 600] /FontDescriptor 6 0 R >>"),
            (6, b"<< /Type /FontDescriptor /MissingWidth 250 /Flags 32 >>"),
        ]);
        assert_eq!(
            (f.width(code(65)), f.width(code(66)), f.width(code(67))),
            (0.5, 0.6, 0.25)
        );
        assert!(d.is_empty());
        let (f, _) = load(&[(5, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>")]);
        assert_eq!(f.width(code(b'a')), 0.556);
        assert_eq!(f.info.base_font, "Helvetica");
        assert_eq!(f.info.kind, FontKind::Type1);
        assert!(!f.info.embedded);
    }

    #[test]
    fn differences_over_winansi() {
        let (f, _) = load(&[(
            5,
            b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding << /BaseEncoding /WinAnsiEncoding /Differences [65 /Euro /bullet] >> >>",
        )]);
        assert_eq!(text(&f, 65).as_deref(), Some("\u{20AC}"));
        assert_eq!(text(&f, 66).as_deref(), Some("\u{2022}"));
        assert_eq!(text(&f, 0x80).as_deref(), Some("\u{20AC}"));
        assert_eq!(text(&f, b'a').as_deref(), Some("a"));
        // std-14 widths follow the glyph the code selects, not the code.
        assert_eq!(f.width(code(66)), 0.35);
    }

    #[test]
    fn tounicode_wins_then_falls_through() {
        let cmap = stream(
            "",
            b"1 begincodespacerange <00> <FF> endcodespacerange 2 beginbfchar <41> <005A> <42> <0000> endbfchar",
        );
        let (f, _) = load(&[
            (5, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding /ToUnicode 6 0 R >>"),
            (6, &cmap),
        ]);
        assert_eq!(text(&f, 65).as_deref(), Some("Z"));
        assert_eq!(text(&f, 66).as_deref(), Some("B"));
    }

    #[test]
    fn tounicode_usecmap_cycle_is_parsed_once() {
        let cmap = stream(
            "/UseCMap 6 0 R",
            b"1 begincodespacerange <00> <FF> endcodespacerange 2 beginbfchar <41> <005A> <42> endbfchar",
        );
        let (f, d) = load(&[
            (
                5,
                b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /ToUnicode 6 0 R >>",
            ),
            (6, &cmap),
        ]);
        assert_eq!(text(&f, 65).as_deref(), Some("Z"));
        use pawdf_core::DiagnosticKind::{BadCMap, RecursionLimit};
        assert_eq!(d, [RecursionLimit, BadCMap, BadCMap]);
    }

    #[test]
    fn usecmap_cycle_member_is_not_cached_truncated() {
        let a = stream(
            "/UseCMap 7 0 R",
            b"1 begincodespacerange <00> <FF> endcodespacerange 1 beginbfchar <41> <0058> endbfchar",
        );
        let b = stream(
            "/UseCMap 6 0 R",
            b"1 begincodespacerange <00> <FF> endcodespacerange 1 beginbfchar <42> <0059> endbfchar",
        );
        let f = file(&[
            (
                5,
                b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /ToUnicode 6 0 R >>",
            ),
            (6, &a),
            (7, &b),
            (
                8,
                b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /ToUnicode 7 0 R >>",
            ),
        ]);
        let (fresh, _) = load_num(&f, &Shared::default(), 8);
        let shared = Shared::default();
        load_num(&f, &shared, 5);
        let (after, _) = load_num(&f, &shared, 8);
        assert_eq!(text(&fresh, 0x41).as_deref(), Some("X"));
        assert_eq!(text(&after, 0x41), text(&fresh, 0x41));
    }

    #[test]
    fn shared_tounicode_is_parsed_once_per_document() {
        let cmap = stream(
            "",
            b"1 beginbfchar <41> endbfchar 1 beginbfchar <42> <005A> endbfchar",
        );
        let f = file(&[
            (
                5,
                b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /ToUnicode 6 0 R >>",
            ),
            (6, &cmap),
        ]);
        let shared = Shared::default();
        let (a, d) = load_with(&f, &shared);
        assert_eq!(d, [pawdf_core::DiagnosticKind::BadCMap]);
        let (b, d) = load_with(&f, &shared);
        assert!(d.is_empty(), "{d:?}");
        assert_eq!(text(&a, 66), text(&b, 66));
        assert_eq!(shared.cmaps.lock().unwrap().len(), 1);
    }

    #[test]
    fn symbolic_without_base_uses_builtin() {
        let (f, _) = load(&[(5, b"<< /Type /Font /Subtype /Type1 /BaseFont /Symbol >>")]);
        assert_eq!(text(&f, 0x61).as_deref(), Some("\u{3B1}"));
        let (f, _) = load(&[(
            5,
            b"<< /Type /Font /Subtype /Type1 /BaseFont /ZapfDingbats /Encoding << /Differences [34 /a1] >> >>",
        )]);
        assert_eq!(text(&f, 0x21).as_deref(), Some("\u{2701}"));
    }

    #[test]
    fn type1_builtin_encoding() {
        let program = b"/Encoding 256 array 0 1 255 {1 index exch /.notdef put} for dup 65 /Alpha put readonly def currentfile eexec";
        let ff = stream(&format!("/Length1 {}", program.len()), program);
        let (f, _) = load(&[
            (
                5,
                b"<< /Type /Font /Subtype /Type1 /BaseFont /ABCDEF+Foo /FontDescriptor 6 0 R >>",
            ),
            (6, b"<< /Type /FontDescriptor /Flags 4 /FontFile 7 0 R >>"),
            (7, &ff),
        ]);
        assert_eq!(text(&f, 65).as_deref(), Some("\u{391}"));
        assert!(f.info.embedded);
        assert_eq!(f.info.base_font, "ABCDEF+Foo");
    }

    fn cff_font(
        subtype: &str,
        flags: u32,
        encoding: &str,
        program: &[u8],
    ) -> (Font, Vec<pawdf_core::DiagnosticKind>) {
        let font = format!(
            "<< /Type /Font /Subtype /Type1 /BaseFont /ABCDEF+Foo {encoding} /FontDescriptor 6 0 R >>"
        );
        let desc = format!("<< /Type /FontDescriptor /Flags {flags} /FontFile3 7 0 R >>");
        let ff = stream(&format!("/Subtype /{subtype}"), program);
        load(&[(5, font.as_bytes()), (6, desc.as_bytes()), (7, &ff)])
    }

    fn alpha_snowman() -> Vec<u8> {
        crate::testutil::cff(&[(65, "Alpha"), (66, "uni2603")])
    }

    #[test]
    fn cff_builtin_encoding() {
        let (f, d) = cff_font("Type1C", 4, "", &alpha_snowman());
        assert_eq!(text(&f, 65).as_deref(), Some("\u{391}"));
        assert_eq!(text(&f, 66).as_deref(), Some("\u{2603}"));
        assert_eq!(text(&f, 67), None);
        assert!(d.is_empty());
    }

    #[test]
    fn cff_symbolic_differences_win() {
        let (f, _) = cff_font(
            "Type1C",
            4,
            "/Encoding << /Differences [66 /beta] >>",
            &alpha_snowman(),
        );
        assert_eq!(text(&f, 65).as_deref(), Some("\u{391}"));
        assert_eq!(text(&f, 66).as_deref(), Some("\u{3B2}"));
    }

    #[test]
    fn cff_nonsymbolic_dictionary_uses_standard() {
        let (f, _) = cff_font(
            "Type1C",
            32,
            "/Encoding << /Differences [1 /g1] >>",
            &alpha_snowman(),
        );
        assert_eq!(text(&f, 65).as_deref(), Some("A"));
    }

    #[test]
    fn corrupt_cff_is_bad_font() {
        let (f, d) = cff_font("Type1C", 4, "", b"garbage");
        assert!(d.contains(&pawdf_core::DiagnosticKind::BadFont));
        assert_eq!(text(&f, 65).as_deref(), Some("A"));
    }

    #[test]
    fn opentype_cff() {
        let program = crate::testutil::sfnt_tables(&[(*b"CFF ", &alpha_snowman())]);
        let (f, d) = cff_font("OpenType", 4, "", &program);
        assert_eq!(text(&f, 65).as_deref(), Some("\u{391}"));
        assert!(d.is_empty());
    }

    #[test]
    fn nonsymbolic_differences_use_standard() {
        let (f, _) = load(&[
            (5, b"<< /Type /Font /Subtype /TrueType /BaseFont /Foo /Encoding << /Differences [1 /g1] >> /FontDescriptor 6 0 R >>"),
            (6, b"<< /Type /FontDescriptor /Flags 32 >>"),
        ]);
        assert_eq!(text(&f, 0x27).as_deref(), Some("\u{2019}"));
        assert_eq!(text(&f, 1).as_deref(), None);
        assert_eq!(f.info.kind, FontKind::TrueType);
    }

    #[test]
    fn type3_uses_font_matrix() {
        let (f, _) = load(&[(
            5,
            b"<< /Type /Font /Subtype /Type3 /FontMatrix [0.002 0 0 0.002 0 0] /FontBBox [0 -100 500 400] /FirstChar 65 /Widths [500] /Encoding << /Differences [65 /A] >> /CharProcs << >> >>",
        )]);
        assert_eq!(f.width(code(65)), 1.0);
        assert_eq!((f.ascent, f.descent), (0.8, -0.2));
        assert_eq!(text(&f, 65).as_deref(), Some("A"));
        assert_eq!(f.info.kind, FontKind::Type3);
    }

    #[test]
    fn ascent_descent_fallbacks() {
        let desc = |d: &'static [u8]| {
            load(&[
                (
                    5,
                    b"<< /Type /Font /Subtype /Type1 /BaseFont /Foo /FontDescriptor 6 0 R >>",
                ),
                (6, d),
            ])
            .0
        };
        let f = desc(b"<< /Ascent 900 /Descent -200 /FontBBox [0 -300 1000 700] >>");
        assert_eq!((f.ascent, f.descent), (0.9, -0.2));
        let f = desc(b"<< /Ascent 0 /Descent 0 /FontBBox [0 -300 1000 700] >>");
        assert_eq!((f.ascent, f.descent), (0.7, -0.3));
        let f = desc(b"<< >>");
        assert_eq!((f.ascent, f.descent), (0.8, -0.2));
    }

    #[test]
    fn unknown_subtype_is_bad_font() {
        let (f, d) = load(&[(5, b"<< /Type /Font /Subtype /Bogus /BaseFont /Helvetica >>")]);
        assert_eq!(d, [pawdf_core::DiagnosticKind::BadFont]);
        assert_eq!(f.width(code(b'a')), 0.556);
    }

    #[test]
    fn bad_encoding_entries_are_diagnosed() {
        for enc in ["/Bogus", "42"] {
            let body =
                format!("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding {enc} >>");
            let (f, d) = load(&[(5, body.as_bytes())]);
            assert_eq!(d, [pawdf_core::DiagnosticKind::BadFont], "{enc}");
            assert_eq!(text(&f, b'a').as_deref(), Some("a"));
        }
    }

    #[test]
    fn extreme_first_char_does_not_overflow() {
        let (f, _) = load(&[(
            5,
            b"<< /Type /Font /Subtype /Type1 /BaseFont /Foo /FirstChar -9223372036854775807 /Widths [500] >>",
        )]);
        assert_eq!(f.width(code(0)), 0.);
    }

    #[test]
    fn fallback_is_helvetica() {
        let f = Font::fallback();
        assert_eq!(f.info.base_font, "Helvetica");
        assert_eq!(f.width(code(b'a')), 0.556);
        assert_eq!(f.unicode(code(b'a')).as_deref(), Some("a"));
        assert_eq!(f.codes(b"abc", 2).len(), 2);
        assert_eq!(
            f.codes(b"ab", usize::MAX)
                .iter()
                .map(|c| c.value)
                .collect::<Vec<_>>(),
            [97, 98]
        );
    }
}
