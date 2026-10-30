//! Glyph-to-Unicode data from an embedded TrueType or OpenType program's `cmap`.

use std::collections::HashMap;
use std::sync::Arc;

use pawdf_core::{DiagnosticKind, Diagnostics, Dict, File, Location, Object, Ref};
use ttf_parser::{PlatformId, RawFace, Tag, cmap};

use super::{Shared, replay};

/// Codepoints inverted per Unicode subtable: the BMP and planes 1–2 (CJK extensions).
/// Asking each codepoint keeps the work bounded however the subtable's ranges are drawn.
// ponytail: planes 3+ are skipped; extend the range if real fonts need them.
const MAX_CODEPOINT: u32 = 0x2FFFF;

/// The parts of a font's `cmap` the reader uses.
#[derive(Debug, Clone)]
pub(crate) struct TrueType {
    by_gid: HashMap<u16, String>,
    simple: [Option<u16>; 256],
}

impl TrueType {
    /// Reads only the `cmap` table, since subset programs in PDFs often lack others. Each
    /// unreadable subtable is skipped with a `BadFont` at its offset in `obj`'s decoded data.
    pub fn parse(program: &[u8], obj: Ref, diags: &mut Diagnostics) -> Option<TrueType> {
        let raw = RawFace::parse(program, 0).ok()?;
        let tag = Tag::from_bytes(b"cmap");
        let base = raw
            .table_records
            .into_iter()
            .find(|r| r.tag == tag)
            .map_or(0, |r| r.offset);
        let data = without_oversized(raw.table(tag)?);
        let table = cmap::Table::parse(&data)?;
        // ttf-parser's subtable iterator ends at the first subtable it cannot parse, hiding
        // every later one, so subtables are fetched by index instead.
        let subtables: Vec<_> = (0..table.subtables.len())
            .filter_map(|i| {
                let s = table.subtables.get(i);
                if s.is_none() {
                    let rec = 8 + 8 * usize::from(i);
                    let at = data.get(rec..rec + 4).and_then(|b| b.try_into().ok());
                    let offset = u64::from(base) + u64::from(at.map_or(0, u32::from_be_bytes));
                    diags.push(DiagnosticKind::BadFont, Location::Stream { obj, offset });
                }
                s
            })
            .collect();
        // A font may declare up to 65535 records, all pointing at one subtable, so only
        // the first full-repertoire and the first BMP Unicode subtable are inverted.
        let full = |s: &cmap::Subtable| {
            matches!(
                (s.platform_id, s.encoding_id),
                (PlatformId::Windows, 10) | (PlatformId::Unicode, 4 | 6)
            )
        };
        let unicode = || subtables.iter().copied().filter(|s| s.is_unicode());
        let picked = [unicode().find(full), unicode().find(|s| !full(s))];
        let mut by_gid = HashMap::new();
        // GID 0 is `.notdef`: a code reaching it has no glyph, let alone a character.
        let gid = |s: &cmap::Subtable, c: u32| s.glyph_index(c).filter(|g| g.0 != 0);
        for sub in picked.into_iter().flatten() {
            for cp in 0..=MAX_CODEPOINT {
                if let (Some(g), Some(c)) = (gid(&sub, cp), char::from_u32(cp)) {
                    by_gid.entry(g.0).or_insert_with(|| c.to_string());
                }
            }
        }
        let find = |platform: PlatformId, encoding: u16| {
            subtables
                .iter()
                .copied()
                .find(|s| s.platform_id == platform && s.encoding_id == encoding)
        };
        let symbol = find(PlatformId::Windows, 0);
        let mac = find(PlatformId::Macintosh, 0);
        let mut simple = [None; 256];
        for (code, slot) in (0u32..).zip(simple.iter_mut()) {
            // §9.6.5.4: a (3,0) subtable maps codes at 0xF000 + code, or at the code itself.
            *slot = symbol
                .and_then(|s| gid(&s, 0xF000 + code).or_else(|| gid(&s, code)))
                .or_else(|| mac.and_then(|s| gid(&s, code)))
                .map(|g| g.0);
        }
        Some(TrueType { by_gid, simple })
    }

    /// The Unicode text the font's Unicode `cmap` gives `gid`. The full-repertoire subtable
    /// wins over the other Unicode one; within a subtable the lowest codepoint wins.
    pub fn unicode(&self, gid: u16) -> Option<&str> {
        self.by_gid.get(&gid).map(String::as_str)
    }

    /// The glyph a simple font's code selects through the built-in `cmap` (§9.6.5.4).
    pub fn simple_gid(&self, code: u8) -> Option<u16> {
        self.simple[usize::from(code)]
    }
}

/// A copy of `cmap` where each subtable whose 32-bit count runs past the table gets an unknown
/// format. ttf-parser 0.25.1 would reject them anyway, but only after a `debug_assert!` that
/// panics (#21); an unknown format is rejected before any parsing.
// ponytail: drop once a ttf-parser release includes upstream dcb583b9.
fn without_oversized(cmap: &[u8]) -> Vec<u8> {
    let be = |at: usize, n: usize| {
        let b = cmap.get(at..at.checked_add(n)?)?;
        Some(b.iter().fold(0u64, |a, &x| a << 8 | u64::from(x)))
    };
    let fits = |at: usize| {
        // (count offset, header length, record size) per format.
        let (count_at, head, size) = match be(at, 2) {
            Some(10) => (16, 20, 2),
            Some(12 | 13) => (12, 16, 12),
            Some(14) => (6, 10, 11),
            _ => return true,
        };
        be(at + count_at, 4).is_none_or(|c| head + c * size <= (cmap.len() - at) as u64)
    };
    let mut out = cmap.to_vec();
    for i in 0..be(2, 2).unwrap_or(0) as usize {
        let Some(offset) = be(8 + 8 * i, 4) else {
            break;
        };
        let at = offset as usize;
        if !fits(at)
            && let Some(format) = out.get_mut(at..at + 2)
        {
            format.copy_from_slice(&[0xFF, 0xFF]);
        }
    }
    out
}

/// The TrueType (or OpenType) program in `desc`, parsed once per document. `BadFont`
/// when one is present but unreadable, or for each `cmap` subtable it cannot read.
pub(crate) fn load(
    file: &File,
    shared: &Shared,
    desc: Option<&Dict>,
    at: Location,
    diags: &mut Diagnostics,
) -> Option<Arc<TrueType>> {
    let desc = desc?;
    let r = match (desc.get(b"FontFile2"), desc.get(b"FontFile3")) {
        (Some(Object::Ref(r)), _) => *r,
        (Some(o), _) if *o != Object::Null => {
            diags.push(DiagnosticKind::BadFont, at);
            return None;
        }
        (_, Some(Object::Ref(r)))
            if matches!(file.resolve(*r), Object::Stream(s)
                if s.dict.get(b"Subtype").and_then(Object::as_name) == Some(b"OpenType")) =>
        {
            *r
        }
        _ => return None,
    };
    let lock = || shared.programs.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((tt, d)) = lock().get(&r) {
        replay(diags, d);
        return tt.clone();
    }
    let s = match file.resolve(r) {
        Object::Stream(s) => s,
        Object::Null => return None,
        _ => {
            diags.push(DiagnosticKind::BadFont, at);
            return None;
        }
    };
    let at = Location::Stream { obj: r, offset: 0 };
    let mut own = Diagnostics::new();
    let program = s.decode(at, &mut own);
    let tt = TrueType::parse(&program.data, r, &mut own).map(Arc::new);
    if tt.is_none() {
        own.push(DiagnosticKind::BadFont, at);
    }
    replay(diags, own.as_slice());
    lock().insert(r, (tt.clone(), own.into_vec()));
    tt
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::sfnt;

    const OBJ: Ref = Ref {
        num: 6,
        generation: 0,
    };

    fn parse(program: &[u8]) -> Option<TrueType> {
        TrueType::parse(program, OBJ, &mut Diagnostics::new())
    }

    #[test]
    fn inverts_unicode_subtables() {
        let t = parse(&sfnt(&[(3, 1, &[(0x41, 5), (0xC0, 5), (0x65E5, 9)])])).unwrap();
        assert_eq!(t.unicode(5), Some("A"));
        assert_eq!(t.unicode(9), Some("\u{65E5}"));
        assert_eq!(t.unicode(1), None);
        assert_eq!(t.simple_gid(0x41), None);
    }

    #[test]
    fn simple_codes_through_symbol_or_mac() {
        let t = parse(&sfnt(&[(3, 0, &[(0xF041, 7), (0x42, 8)])])).unwrap();
        assert_eq!((t.simple_gid(0x41), t.simple_gid(0x42)), (Some(7), Some(8)));
        let t = parse(&sfnt(&[(1, 0, &[(0x43, 4)])])).unwrap();
        assert_eq!(t.simple_gid(0x43), Some(4));
    }

    #[test]
    fn notdef_is_never_a_mapping() {
        let t = parse(&sfnt(&[(3, 1, &[(0x41, 0)])])).unwrap();
        assert_eq!(t.unicode(0), None);
        let t = parse(&sfnt(&[(3, 0, &[(0xF041, 0), (0x41, 7)])])).unwrap();
        assert_eq!(t.simple_gid(0x41), Some(7));
    }

    #[test]
    fn many_subtable_records_cost_two_inversions() {
        let one: &[(u16, u16)] = &[(0x41, 5)];
        let records: Vec<_> = (0..3000).map(|_| (3, 1, one)).collect();
        let start = std::time::Instant::now();
        let t = parse(&sfnt(&records)).unwrap();
        assert_eq!(t.unicode(5), Some("A"));
        assert!(start.elapsed().as_secs() < 5, "{:?}", start.elapsed());
    }

    #[test]
    fn programs_are_cached_per_stream() {
        let program = sfnt(&[(3, 1, &[(0x41, 5)])]);
        let ff = crate::testutil::stream("", &program);
        let f = crate::testutil::file(&[
            (5, b"<< /Type /FontDescriptor /FontFile2 6 0 R >>"),
            (6, &ff),
        ]);
        let r = pawdf_core::Ref {
            num: 5,
            generation: 0,
        };
        let desc = f.resolve(r).as_dict().unwrap().clone();
        let shared = crate::font::Shared::default();
        let mut d = Diagnostics::new();
        let a = load(&f, &shared, Some(&desc), Location::File(0), &mut d).unwrap();
        let b = load(&f, &shared, Some(&desc), Location::File(0), &mut d).unwrap();
        assert!(std::sync::Arc::ptr_eq(&a, &b));
    }

    #[test]
    fn cached_program_replays_its_diagnostics() {
        let ff = crate::testutil::stream("", b"garbage");
        let f = crate::testutil::file(&[
            (5, b"<< /Type /FontDescriptor /FontFile2 6 0 R >>"),
            (6, &ff),
        ]);
        let r = pawdf_core::Ref {
            num: 5,
            generation: 0,
        };
        let desc = f.resolve(r).as_dict().unwrap().clone();
        let shared = crate::font::Shared::default();
        let at = Location::Stream {
            obj: OBJ,
            offset: 0,
        };
        for _ in 0..2 {
            let mut d = Diagnostics::new();
            assert!(load(&f, &shared, Some(&desc), Location::File(0), &mut d).is_none());
            let got: Vec<_> = d.as_slice().iter().map(|d| (d.kind, d.location)).collect();
            assert_eq!(got, [(DiagnosticKind::BadFont, at)]);
        }
    }

    #[test]
    fn oversized_subtable_count_does_not_panic() {
        // A format 14 subtable whose record count overflows a u32 byte length (#21), then a
        // format 12 one mapping U+0041 to glyph 5. The bad one coming first also checks that
        // it does not hide the later one.
        let be32 = |x: u32| x.to_be_bytes();
        let mut cmap = vec![
            0, 0, 0, 2, 0, 0, 0, 5, 0, 0, 0, 20, 0, 3, 0, 10, 0, 0, 0, 30,
        ];
        cmap.extend([0, 14, 0, 0, 0, 10]);
        cmap.extend(be32(0x1800_0000));
        cmap.extend([0, 12, 0, 0]);
        for x in [28, 0, 1, 0x41, 0x41, 5] {
            cmap.extend(be32(x));
        }
        let program = crate::testutil::sfnt_tables(&[(*b"cmap", &cmap)]);
        let mut d = Diagnostics::new();
        let t = TrueType::parse(&program, OBJ, &mut d).unwrap();
        assert_eq!(t.unicode(5), Some("A"));
        let base = program.windows(cmap.len()).position(|w| w == cmap).unwrap() as u64;
        let got: Vec<_> = d.as_slice().iter().map(|d| (d.kind, d.location)).collect();
        let at = Location::Stream {
            obj: OBJ,
            offset: base + 20,
        };
        assert_eq!(got, [(DiagnosticKind::BadFont, at)]);
    }

    #[test]
    fn garbage_is_none() {
        assert!(parse(b"garbage").is_none());
        assert!(parse(&[]).is_none());
    }
}
