//! Bundled data: glyph names, base encodings, and standard-14 font widths. See `data/`
//! for sources and licenses, and `scripts/gen_reader_data.py` to regenerate.

use std::collections::HashMap;
use std::sync::OnceLock;

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

#[cfg(test)]
mod tests {
    use super::*;

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
