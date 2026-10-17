//! Bundled data: glyph names, base encodings, standard-14 font widths, predefined CMaps,
//! and CID-to-Unicode tables. See `data/` for sources and licenses, and
//! `scripts/gen_reader_data.py` to regenerate.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use pawdf_core::{Diagnostics, Origin};

use crate::cmap::{CMap, UseCMap};
use crate::model::WritingMode;

/// A predefined simple-font encoding (§9.6.5, Annex D).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BaseEncoding {
    Standard,
    WinAnsi,
    MacRoman,
    MacExpert,
    Symbol,
    ZapfDingbats,
}

type Table = [Option<&'static str>; 256];

/// Codes below this have no entry in any base encoding.
const FIRST_CODE: usize = 32;

fn inflate(data: &[u8]) -> String {
    miniz_oxide::inflate::decompress_to_vec_zlib(data)
        .ok()
        .and_then(|v| String::from_utf8(v).ok())
        .unwrap_or_default()
}

/// Glyph names for each code of `enc`.
pub(crate) fn encoding_names(enc: BaseEncoding) -> &'static Table {
    static TEXT: OnceLock<String> = OnceLock::new();
    static TABLES: OnceLock<HashMap<&'static str, Table>> = OnceLock::new();
    const EMPTY: Table = [None; 256];
    let tables = TABLES.get_or_init(|| {
        let text = TEXT.get_or_init(|| inflate(include_bytes!("../data/encodings.txt.zz")));
        text.lines()
            .filter_map(|line| {
                let mut words = line.split(' ');
                let name = words.next()?;
                let mut table = EMPTY;
                for (slot, glyph) in table.iter_mut().skip(FIRST_CODE).zip(words) {
                    *slot = (glyph != "-").then_some(glyph);
                }
                Some((name, table))
            })
            .collect()
    });
    let key = match enc {
        BaseEncoding::Standard => "StandardEncoding",
        BaseEncoding::WinAnsi => "WinAnsiEncoding",
        BaseEncoding::MacRoman => "MacRomanEncoding",
        BaseEncoding::MacExpert => "MacExpertEncoding",
        BaseEncoding::Symbol => "Symbol",
        BaseEncoding::ZapfDingbats => "ZapfDingbats",
    };
    tables.get(key).unwrap_or(&EMPTY)
}

/// The Unicode text of an Adobe Glyph List (or ITC Zapf Dingbats list) glyph name.
pub(crate) fn agl_lookup(name: &str) -> Option<&'static str> {
    static TEXT: OnceLock<String> = OnceLock::new();
    static MAP: OnceLock<HashMap<&'static str, String>> = OnceLock::new();
    let map = MAP.get_or_init(|| {
        let text = TEXT.get_or_init(|| inflate(include_bytes!("../data/glyphlist.txt.zz")));
        let mut map = HashMap::new();
        for line in text.lines() {
            let Some((name, codes)) = line.split_once(';') else {
                continue;
            };
            let s: Option<String> = codes
                .split(' ')
                .map(|h| u32::from_str_radix(h, 16).ok().and_then(char::from_u32))
                .collect();
            // The list repeats some names; the first entry is the preferred one.
            if let Some(s) = s {
                map.entry(name).or_insert(s);
            }
        }
        map
    });
    map.get(name).map(String::as_str)
}

/// Width of `glyph` in a standard-14 font, in glyph space units (1/1000 em).
pub(crate) fn std14_width(font: &str, glyph: &str) -> Option<f64> {
    static TEXT: OnceLock<String> = OnceLock::new();
    static FONTS: OnceLock<HashMap<&'static str, HashMap<&'static str, f64>>> = OnceLock::new();
    let fonts = FONTS.get_or_init(|| {
        let text = TEXT.get_or_init(|| inflate(include_bytes!("../data/std14.txt.zz")));
        let mut fonts: HashMap<&str, HashMap<&str, f64>> = HashMap::new();
        let mut current = None;
        for line in text.lines() {
            if let Some(font) = line.strip_prefix('#') {
                current = Some(font);
            } else if let (Some(font), Some((glyph, w))) = (current, line.split_once(' '))
                && let Ok(w) = w.parse()
            {
                fonts.entry(font).or_default().insert(glyph, w);
            }
        }
        fonts
    });
    fonts.get(font)?.get(glyph).copied()
}

/// Archive entries by name: an index of `name offset length` lines, a blank line, then
/// one zlib stream per entry.
fn cmap_index() -> &'static HashMap<&'static str, &'static [u8]> {
    static DATA: &[u8] = include_bytes!("../data/cmaps.bin");
    static INDEX: OnceLock<HashMap<&'static str, &'static [u8]>> = OnceLock::new();
    INDEX.get_or_init(|| {
        let Some(split) = DATA.windows(2).position(|w| w == b"\n\n") else {
            return HashMap::new();
        };
        let (Ok(index), Some(body)) = (std::str::from_utf8(&DATA[..split]), DATA.get(split + 2..))
        else {
            return HashMap::new();
        };
        index
            .lines()
            .filter_map(|line| {
                let mut f = line.split(' ');
                let name = f.next()?;
                let offset: usize = f.next()?.parse().ok()?;
                let len: usize = f.next()?.parse().ok()?;
                Some((name, body.get(offset..offset.checked_add(len)?)?))
            })
            .collect()
    })
}

fn parse_bundled(name: &[u8], depth: usize, diags: &mut Diagnostics) -> Option<CMap> {
    let blob = cmap_index().get(std::str::from_utf8(name).ok()?)?;
    let data = miniz_oxide::inflate::decompress_to_vec_zlib(blob).ok()?;
    let mut resolve = |u: UseCMap, depth: usize, diags: &mut Diagnostics| match u {
        UseCMap::Name(n) => bundled(n, depth, diags),
        UseCMap::Stream(_) => None,
    };
    Some(CMap::parse(&data, Origin::File, depth, &mut resolve, diags))
}

fn bundled(name: &[u8], depth: usize, diags: &mut Diagnostics) -> Option<Arc<CMap>> {
    static CACHE: OnceLock<Mutex<HashMap<Vec<u8>, Arc<CMap>>>> = OnceLock::new();
    let cache = CACHE.get_or_init(Mutex::default);
    if let Some(m) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(name) {
        return Some(Arc::clone(m));
    }
    // Not holding the lock while parsing: `usecmap` re-enters here.
    let m = Arc::new(match name {
        b"Identity-H" => CMap::identity(WritingMode::Horizontal),
        b"Identity-V" => CMap::identity(WritingMode::Vertical),
        _ => parse_bundled(name, depth, diags)?,
    });
    cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(name.to_vec(), Arc::clone(&m));
    Some(m)
}

/// A predefined CMap (§9.7.5.2, Table 116) by name.
pub(crate) fn predefined_cmap(name: &[u8]) -> Option<Arc<CMap>> {
    if name.starts_with(b"Adobe-") {
        return None;
    }
    bundled(name, 0, &mut Diagnostics::new())
}

/// The `registry-ordering-UCS2` CID-to-Unicode CMap (§9.10.2).
pub(crate) fn ucs2(registry: &str, ordering: &str) -> Option<Arc<CMap>> {
    if registry != "Adobe" || !matches!(ordering, "Japan1" | "GB1" | "CNS1" | "Korea1" | "KR") {
        return None;
    }
    bundled(
        format!("Adobe-{ordering}-UCS2").as_bytes(),
        0,
        &mut Diagnostics::new(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::WritingMode;

    fn cid(name: &str, bytes: &[u8]) -> Option<u32> {
        predefined_cmap(name.as_bytes()).map(|m| m.cid(m.next_code(bytes)))
    }

    #[test]
    fn predefined_unicode_cmap() {
        assert_eq!(cid("UniJIS-UTF16-H", &[0x30, 0x42]), Some(843));
        let m = predefined_cmap(b"UniJIS-UTF16-H").unwrap();
        assert_eq!(m.ordering.as_deref(), Some("Japan1"));
        assert_eq!(m.wmode, WritingMode::Horizontal);
    }

    #[test]
    fn predefined_vertical_uses_parent() {
        let m = predefined_cmap(b"90ms-RKSJ-V").unwrap();
        assert_eq!(m.wmode, WritingMode::Vertical);
        assert_eq!(m.cid(m.next_code(b"\x82\xa0")), 843);
        assert_eq!(m.next_code(b"A").len, 1);
        assert_eq!(cid("UniJIS-UTF16-V", &[0x30, 0x01]), Some(7887));
        assert_eq!(cid("V", &[0x24, 0x22]), cid("H", &[0x24, 0x22]));
    }

    #[test]
    fn identity_is_built_in() {
        let m = predefined_cmap(b"Identity-V").unwrap();
        assert_eq!(m.wmode, WritingMode::Vertical);
        assert_eq!(m.cid(m.next_code(&[0x12, 0x34])), 0x1234);
        assert_eq!(cid("Identity-H", &[0, 7]), Some(7));
    }

    #[test]
    fn ucs2_tables() {
        let j = ucs2("Adobe", "Japan1").unwrap();
        assert_eq!(j.lookup(&843u16.to_be_bytes()).as_deref(), Some("\u{3042}"));
        assert_eq!(
            j.lookup(&7887u16.to_be_bytes()).as_deref(),
            Some("\u{3001}")
        );
        for o in ["GB1", "CNS1", "Korea1", "KR"] {
            assert!(ucs2("Adobe", o).is_some(), "{o}");
        }
        assert!(ucs2("Adobe", "Identity").is_none());
        assert!(ucs2("Foo", "Japan1").is_none());
        assert!(predefined_cmap(b"Nope-H").is_none());
    }

    #[test]
    fn every_bundled_entry_parses_cleanly() {
        let names: Vec<&str> = cmap_index().keys().copied().collect();
        assert_eq!(names.len(), 64);
        for name in names {
            let mut d = Diagnostics::new();
            let m = parse_bundled(name.as_bytes(), 0, &mut d);
            assert!(m.is_some(), "{name}");
            assert!(d.as_slice().is_empty(), "{name}: {:?}", d.as_slice());
        }
    }

    #[test]
    fn agl() {
        assert_eq!(agl_lookup("Aacute"), Some("\u{c1}"));
        assert_eq!(agl_lookup("a1"), Some("\u{2701}"));
        assert_eq!(agl_lookup("nope"), None);
    }

    #[test]
    fn encodings() {
        assert_eq!(encoding_names(BaseEncoding::WinAnsi)[0x80], Some("Euro"));
        assert_eq!(
            encoding_names(BaseEncoding::Standard)[0x27],
            Some("quoteright")
        );
        assert_eq!(
            encoding_names(BaseEncoding::MacRoman)[0x80],
            Some("Adieresis")
        );
        assert_eq!(encoding_names(BaseEncoding::Symbol)[0x61], Some("alpha"));
        assert_eq!(encoding_names(BaseEncoding::ZapfDingbats)[0x21], Some("a1"));
        assert_eq!(
            encoding_names(BaseEncoding::MacExpert)[0x21],
            Some("exclamsmall")
        );
        assert_eq!(encoding_names(BaseEncoding::Standard)[0x10], None);
        assert_eq!(encoding_names(BaseEncoding::Standard)[0x80], None);
    }

    #[test]
    fn std14() {
        assert_eq!(std14_width("Helvetica", "a"), Some(556.));
        assert_eq!(std14_width("Courier-Bold", "W"), Some(600.));
        assert_eq!(std14_width("ZapfDingbats", "a1"), Some(974.));
        assert_eq!(std14_width("Helvetica", "nope"), None);
        // A glyph named like the old font-header marker must still parse as a glyph.
        assert_eq!(std14_width("Helvetica", "F"), Some(611.));
        assert_eq!(std14_width("Times-Roman", "F"), Some(556.));
        assert_eq!(std14_width("Nope", "a"), None);
    }
}
