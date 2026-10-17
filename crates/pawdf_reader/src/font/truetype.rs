//! Glyph-to-Unicode data from an embedded TrueType or OpenType program's `cmap`.

use std::collections::HashMap;
use std::sync::Arc;

use pawdf_core::{DiagnosticKind, Diagnostics, Dict, File, Location, Object};
use ttf_parser::{PlatformId, RawFace, Tag, cmap};

use super::Shared;

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
    /// Reads only the `cmap` table, since subset programs in PDFs often lack others.
    pub fn parse(program: &[u8]) -> Option<TrueType> {
        let raw = RawFace::parse(program, 0).ok()?;
        let table = cmap::Table::parse(raw.table(Tag::from_bytes(b"cmap"))?)?;
        // A font may declare up to 65535 records, all pointing at one subtable, so only
        // the first full-repertoire and the first BMP Unicode subtable are inverted.
        let full = |s: &cmap::Subtable| {
            matches!(
                (s.platform_id, s.encoding_id),
                (PlatformId::Windows, 10) | (PlatformId::Unicode, 4 | 6)
            )
        };
        let unicode = || table.subtables.into_iter().filter(|s| s.is_unicode());
        let picked = [unicode().find(full), unicode().find(|s| !full(s))];
        let mut by_gid = HashMap::new();
        for sub in picked.into_iter().flatten() {
            for cp in 0..=MAX_CODEPOINT {
                if let (Some(g), Some(c)) = (sub.glyph_index(cp), char::from_u32(cp)) {
                    by_gid.entry(g.0).or_insert_with(|| c.to_string());
                }
            }
        }
        let find = |platform: PlatformId, encoding: u16| {
            table
                .subtables
                .into_iter()
                .find(|s| s.platform_id == platform && s.encoding_id == encoding)
        };
        let symbol = find(PlatformId::Windows, 0);
        let mac = find(PlatformId::Macintosh, 0);
        let mut simple = [None; 256];
        for (code, slot) in (0u32..).zip(simple.iter_mut()) {
            // §9.6.5.4: a (3,0) subtable maps codes at 0xF000 + code, or at the code itself.
            *slot = symbol
                .and_then(|s| s.glyph_index(0xF000 + code).or_else(|| s.glyph_index(code)))
                .or_else(|| mac.and_then(|s| s.glyph_index(code)))
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

/// The TrueType (or OpenType) program in `desc`, parsed once per document. `BadFont`
/// when one is present but unreadable.
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
        (_, Some(Object::Ref(r)))
            if matches!(file.resolve(*r), Object::Stream(s)
                if s.dict.get(b"Subtype").and_then(Object::as_name) == Some(b"OpenType")) =>
        {
            *r
        }
        _ => return None,
    };
    let lock = || shared.programs.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(tt) = lock().get(&r) {
        return tt.clone();
    }
    let Object::Stream(s) = file.resolve(r) else {
        return None;
    };
    let program = s.decode(Location::Stream { obj: r, offset: 0 }, diags);
    let tt = TrueType::parse(&program.data).map(Arc::new);
    if tt.is_none() {
        diags.push(DiagnosticKind::BadFont, at);
    }
    lock().insert(r, tt.clone());
    tt
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::sfnt;

    #[test]
    fn inverts_unicode_subtables() {
        let t = TrueType::parse(&sfnt(&[(3, 1, &[(0x41, 5), (0xC0, 5), (0x65E5, 9)])])).unwrap();
        assert_eq!(t.unicode(5), Some("A"));
        assert_eq!(t.unicode(9), Some("\u{65E5}"));
        assert_eq!(t.unicode(1), None);
        assert_eq!(t.simple_gid(0x41), None);
    }

    #[test]
    fn simple_codes_through_symbol_or_mac() {
        let t = TrueType::parse(&sfnt(&[(3, 0, &[(0xF041, 7), (0x42, 8)])])).unwrap();
        assert_eq!((t.simple_gid(0x41), t.simple_gid(0x42)), (Some(7), Some(8)));
        let t = TrueType::parse(&sfnt(&[(1, 0, &[(0x43, 4)])])).unwrap();
        assert_eq!(t.simple_gid(0x43), Some(4));
    }

    #[test]
    fn many_subtable_records_cost_two_inversions() {
        let one: &[(u16, u16)] = &[(0x41, 5)];
        let records: Vec<_> = (0..3000).map(|_| (3, 1, one)).collect();
        let start = std::time::Instant::now();
        let t = TrueType::parse(&sfnt(&records)).unwrap();
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
    fn garbage_is_none() {
        assert!(TrueType::parse(b"garbage").is_none());
        assert!(TrueType::parse(&[]).is_none());
    }
}
