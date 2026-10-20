//! Composite fonts (§9.7): a Type0 font over one CIDFont descendant.

use std::sync::Arc;

use pawdf_core::{DiagnosticKind, Diagnostics, Dict, File, Location, Object};

use super::truetype::{self, TrueType};
use super::{Font, GLYPH_SCALE, Glyphs, Shared, embedded_cmap, metrics};
use crate::cmap::{CMap, Code};
use crate::data::{predefined_cmap, ucs2};
use crate::model::{FontInfo, FontKind, WritingMode};
use crate::ranges::last_wins;

/// Vertical metrics at size 1, in text space (§9.7.4.3).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Vertical {
    /// Vertical displacement, usually negative.
    pub w1: f64,
    /// Position vector from the horizontal to the vertical origin.
    pub vx: f64,
    pub vy: f64,
}

/// Per-code data of a composite font.
#[derive(Debug, Clone)]
pub(crate) struct Cid {
    pub cmap: Arc<CMap>,
    to_unicode: Option<Arc<CMap>>,
    ucs2: Option<Arc<CMap>>,
    /// `(first, last, w0)` sorted by `first`.
    w: Vec<(u32, u32, f64)>,
    dw: f64,
    /// `(first, last, [w1, vx, vy])` sorted by `first`.
    w2: Vec<(u32, u32, [f64; 3])>,
    /// `/DW2`: `[vy, w1]`.
    dw2: [f64; 2],
    /// For Identity-encoded CIDFontType2 fonts: the embedded program's cmap, and the
    /// `/CIDToGIDMap` stream (`None` for `/Identity`).
    truetype: Option<(Arc<TrueType>, Option<Vec<u8>>)>,
}

/// The entry covering `cid` in disjoint ranges sorted by start.
fn find<T: Copy>(ranges: &[(u32, u32, T)], cid: u32) -> Option<T> {
    let i = ranges.partition_point(|r| r.0 <= cid);
    let r = ranges.get(i.checked_sub(1)?)?;
    (cid <= r.1).then_some(r.2)
}

impl Cid {
    pub fn width(&self, cid: u32) -> f64 {
        find(&self.w, cid).unwrap_or(self.dw)
    }

    pub fn vertical(&self, cid: u32) -> Vertical {
        match find(&self.w2, cid) {
            Some([w1, vx, vy]) => Vertical { w1, vx, vy },
            None => Vertical {
                w1: self.dw2[1],
                vx: self.width(cid) / 2.,
                vy: self.dw2[0],
            },
        }
    }

    /// Unicode for `code` (spec §6.2): ToUnicode, then the character collection's UCS2
    /// table, then the embedded TrueType cmap through `/CIDToGIDMap`.
    pub fn unicode(&self, code: Code, cid: u32) -> Option<String> {
        if let Some(s) = self
            .to_unicode
            .as_ref()
            .and_then(|t| t.lookup(&code.bytes()))
        {
            return Some(s);
        }
        let cid = u16::try_from(cid).ok()?;
        if let Some(s) = self
            .ucs2
            .as_ref()
            .and_then(|u| u.lookup(&cid.to_be_bytes()))
        {
            return Some(s);
        }
        let (tt, map) = self.truetype.as_ref()?;
        let gid = match map {
            None => cid,
            Some(m) => {
                let i = usize::from(cid) * 2;
                u16::from_be_bytes([*m.get(i)?, *m.get(i + 1)?])
            }
        };
        tt.unicode(gid).map(str::to_owned)
    }
}

/// Loads a Type0 font dictionary (§9.7.6). Problems become `BadFont` or `BadCMap` at `at`.
pub(crate) fn load_type0(
    file: &File,
    shared: &Shared,
    dict: &Dict,
    at: Location,
    diags: &mut Diagnostics,
) -> Font {
    let get = |k: &[u8]| dict.get(k).map(|o| file.deref(o));
    let identity = || Arc::new(CMap::identity(WritingMode::Horizontal));
    let cmap = match dict.get(b"Encoding") {
        Some(Object::Ref(r)) if matches!(file.resolve(*r), Object::Stream(_)) => {
            // Without codespace ranges every byte would become a code mapped to CID 0.
            match embedded_cmap(file, shared, *r, diags) {
                Some(m) if m.has_codespace() => m,
                m => {
                    diags.push(DiagnosticKind::BadCMap, at);
                    let wmode = m.map_or(WritingMode::Horizontal, |m| m.wmode);
                    Arc::new(CMap::identity(wmode))
                }
            }
        }
        _ => match get(b"Encoding").and_then(Object::as_name) {
            Some(n) => predefined_cmap(n).unwrap_or_else(|| {
                diags.push(DiagnosticKind::BadCMap, at);
                identity()
            }),
            None => {
                diags.push(DiagnosticKind::BadFont, at);
                identity()
            }
        },
    };
    let descendant = get(b"DescendantFonts")
        .and_then(Object::as_array)
        .and_then(|a| a.first())
        .and_then(|o| file.deref(o).as_dict());
    let subtype = descendant
        .and_then(|d| d.get(b"Subtype"))
        .and_then(|o| file.deref(o).as_name());
    if !matches!(subtype, Some(b"CIDFontType0" | b"CIDFontType2")) {
        diags.push(DiagnosticKind::BadFont, at);
    }
    let dget = |k: &[u8]| descendant.and_then(|d| d.get(k)).map(|o| file.deref(o));
    let desc = dget(b"FontDescriptor").and_then(Object::as_dict);
    let embedded = [b"FontFile".as_slice(), b"FontFile2", b"FontFile3"]
        .iter()
        .any(|k| desc.is_some_and(|d| d.contains_key(k)));

    let num = |o: &Object| file.deref(o).as_number().filter(|n| n.is_finite());
    let dw = dget(b"DW").and_then(num).unwrap_or(1000.) / 1000.;
    let w = dget(b"W")
        .and_then(Object::as_array)
        .map(|a| cid_ranges::<1>(file, a, at, diags))
        .unwrap_or_default()
        .into_iter()
        .map(|(lo, hi, [w])| (lo, hi, w))
        .collect();
    let w2 = dget(b"W2")
        .and_then(Object::as_array)
        .map(|a| cid_ranges::<3>(file, a, at, diags))
        .unwrap_or_default();
    let dw2 = match dget(b"DW2").and_then(Object::as_array) {
        Some([vy, w1]) => match (num(vy), num(w1)) {
            (Some(vy), Some(w1)) => [vy / 1000., w1 / 1000.],
            _ => {
                diags.push(DiagnosticKind::BadFont, at);
                [0.88, -1.]
            }
        },
        _ => [0.88, -1.],
    };

    let to_unicode = match dict.get(b"ToUnicode") {
        Some(Object::Ref(r)) => embedded_cmap(file, shared, *r, diags),
        _ => None,
    };
    // §9.10.2 (b): the ordering of the font's CMap, then of the CIDFont.
    let info = dget(b"CIDSystemInfo").and_then(Object::as_dict);
    let info_str = |k: &[u8]| match info.and_then(|d| d.get(k)).map(|o| file.deref(o)) {
        Some(Object::String(s)) => Some(String::from_utf8_lossy(s).into_owned()),
        _ => None,
    };
    let ucs2 = match (&cmap.registry, &cmap.ordering) {
        (Some(r), Some(o)) => ucs2(r, o),
        _ => None,
    }
    .or_else(|| ucs2(&info_str(b"Registry")?, &info_str(b"Ordering")?));

    let identity =
        cmap.registry.as_deref() == Some("Adobe") && cmap.ordering.as_deref() == Some("Identity");
    let truetype = match subtype {
        Some(b"CIDFontType2") if identity => {
            truetype::load(file, shared, desc, at, diags).map(|tt| {
                let map = match descendant.and_then(|d| d.get(b"CIDToGIDMap")) {
                    Some(Object::Ref(r)) => match file.resolve(*r) {
                        Object::Stream(s) => Some(
                            s.decode(Location::Stream { obj: *r, offset: 0 }, diags)
                                .data,
                        ),
                        _ => None,
                    },
                    _ => None,
                };
                (tt, map)
            })
        }
        _ => None,
    };
    let (ascent, descent) = metrics(file, FontKind::Type0, &GLYPH_SCALE, dict, desc);
    Font {
        info: FontInfo {
            base_font: get(b"BaseFont")
                .and_then(Object::as_name)
                .map(|n| String::from_utf8_lossy(n).into_owned())
                .unwrap_or_default(),
            kind: FontKind::Type0,
            embedded,
            wmode: cmap.wmode,
        },
        ascent,
        descent,
        glyphs: Glyphs::Composite(Box::new(Cid {
            cmap,
            to_unicode,
            ucs2,
            w,
            dw,
            w2,
            dw2,
            truetype,
        })),
    }
}

/// `/W` (N = 1) or `/W2` (N = 3) entries (§9.7.4.3): `c [v…]` lists N values per CID,
/// `cfirst clast v…` gives N values to a range. Values are divided by 1000. A bad entry
/// is skipped up to the next element, with one `BadFont` per array.
fn cid_ranges<const N: usize>(
    file: &File,
    arr: &[Object],
    at: Location,
    diags: &mut Diagnostics,
) -> Vec<(u32, u32, [f64; N])> {
    let cid = |o: &Object| file.deref(o).as_int().and_then(|c| u32::try_from(c).ok());
    let num = |o: &Object| file.deref(o).as_number().filter(|n| n.is_finite());
    let values = |items: &[Object]| -> Option<[f64; N]> {
        let mut out = [0.; N];
        for (slot, o) in out.iter_mut().zip(items) {
            *slot = num(o)? / 1000.;
        }
        (items.len() >= N).then_some(out)
    };
    let mut out = Vec::new();
    let mut bad = false;
    let mut i = 0;
    while let Some(first) = arr.get(i) {
        let next = arr.get(i + 1).map(|o| file.deref(o));
        match (cid(first), next) {
            (Some(c), Some(Object::Array(list))) => {
                for (k, chunk) in list.chunks(N).enumerate() {
                    let id = u32::try_from(k).ok().and_then(|k| c.checked_add(k));
                    match (id, values(chunk)) {
                        (Some(id), Some(v)) => out.push((id, id, v)),
                        _ => bad = true,
                    }
                }
                i += 2;
            }
            (Some(lo), Some(last)) => {
                match (cid(last), arr.get(i + 2..i + 2 + N).and_then(values)) {
                    (Some(hi), Some(v)) if lo <= hi => {
                        out.push((lo, hi, v));
                        i += 2 + N;
                    }
                    _ => {
                        bad = true;
                        i += 1;
                    }
                }
            }
            _ => {
                bad = true;
                i += 1;
            }
        }
    }
    if bad {
        diags.push(DiagnosticKind::BadFont, at);
    }
    // pdfium's CPDF_CIDFont::GetCharWidthF scans the entries in order, so the first listed
    // wins.
    last_wins(out.into_iter().rev(), |v, _| v)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ranges<const N: usize>(src: &[u8]) -> (Vec<(u32, u32, [f64; N])>, usize) {
        let f = crate::testutil::file(&[(5, src)]);
        let r = pawdf_core::Ref {
            num: 5,
            generation: 0,
        };
        let arr = f.resolve(r).as_array().unwrap().to_vec();
        let mut d = Diagnostics::new();
        let out = cid_ranges::<N>(&f, &arr, Location::File(0), &mut d);
        (out, d.as_slice().len())
    }

    #[test]
    fn w2_forms() {
        let (r, bad) = ranges::<3>(b"[1 [-500 250 400 -600 300 500] 10 12 -1000 500 880]");
        assert_eq!(bad, 0);
        assert_eq!(
            r,
            [
                (1, 1, [-0.5, 0.25, 0.4]),
                (2, 2, [-0.6, 0.3, 0.5]),
                (10, 12, [-1., 0.5, 0.88])
            ]
        );
    }

    fn cid_width(src: &[u8], cid: u32) -> Option<[f64; 1]> {
        find(&ranges::<1>(src).0, cid)
    }

    #[test]
    fn overlapping_w_first_listed_wins() {
        assert_eq!(cid_width(b"[1 3 500 2 [700]]", 2), Some([0.5]));
        let src = b"[2 [700] 1 3 500]";
        assert_eq!(
            [1, 2, 3].map(|c| cid_width(src, c)),
            [Some([0.5]), Some([0.7]), Some([0.5])]
        );
    }

    #[test]
    fn overlapping_w2_first_listed_wins() {
        let (r, _) = ranges::<3>(b"[1 3 -1000 500 880 2 [-500 250 400]]");
        assert_eq!(find(&r, 2), Some([-1., 0.5, 0.88]));
    }

    #[test]
    fn short_list_is_bad() {
        let (r, bad) = ranges::<3>(b"[1 [-500 250]]");
        assert!(r.is_empty());
        assert_eq!(bad, 1);
    }
}
