//! Simple-font encodings (§9.6.5) and glyph names to Unicode.

use crate::data::agl_lookup;

/// Only the cleartext start of a Type1 program is scanned for its encoding.
const MAX_CLEARTEXT: usize = 64 << 10;

/// Unicode for a glyph name, per the Adobe Glyph List specification: drop any suffix
/// after `.`, split ligatures on `_`, then map each component through the AGL or the
/// `uniXXXX` / `uXXXX[XX]` forms.
pub(crate) fn glyph_unicode(name: &str) -> Option<String> {
    let base = name.split('.').next().unwrap_or_default();
    let out: String = base.split('_').filter_map(component).collect();
    (!out.is_empty()).then_some(out)
}

fn component(c: &str) -> Option<String> {
    if let Some(s) = agl_lookup(c) {
        return Some(s.to_owned());
    }
    if let Some(hex) = c.strip_prefix("uni")
        && !hex.is_empty()
        && hex.len() % 4 == 0
        && hex.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return hex
            .as_bytes()
            .chunks(4)
            .map(|g| {
                let v = u32::from_str_radix(std::str::from_utf8(g).ok()?, 16).ok()?;
                char::from_u32(v)
            })
            .collect();
    }
    if let Some(hex) = c.strip_prefix('u')
        && (4..=6).contains(&hex.len())
        && hex.bytes().all(|b| b.is_ascii_hexdigit())
    {
        let v = u32::from_str_radix(hex, 16).ok()?;
        return char::from_u32(v).map(String::from);
    }
    None
}

/// Glyph names from the `dup <code> /<name> put` entries of a Type1 font's cleartext
/// `/Encoding` array. `None` when the font uses StandardEncoding or has no entries.
pub(crate) fn type1_builtin(program: &[u8]) -> Option<Vec<Option<String>>> {
    let clear = &program[..program.len().min(MAX_CLEARTEXT)];
    let clear = match find(clear, b"eexec") {
        Some(i) => &clear[..i],
        None => clear,
    };
    let start = find(clear, b"/Encoding")?;
    // `/` is a PostScript delimiter, so compact programs write `dup 1/A put`.
    let mut spaced = Vec::with_capacity(clear.len() - start);
    for &b in &clear[start..] {
        if b == b'/' {
            spaced.push(b' ');
        }
        spaced.push(b);
    }
    let words: Vec<&[u8]> = spaced
        .split(|b| b.is_ascii_whitespace())
        .filter(|w| !w.is_empty())
        .collect();
    if words.get(1) == Some(&b"StandardEncoding".as_slice()) {
        return None;
    }
    let mut table = vec![None; 256];
    let mut found = false;
    for w in words.windows(4) {
        if let [b"dup", code, name, b"put"] = w
            && let Some(name) = name.strip_prefix(b"/")
            && let Some(slot) = std::str::from_utf8(code)
                .ok()
                .and_then(|c| c.parse::<usize>().ok())
                .and_then(|c| table.get_mut(c))
        {
            *slot = Some(String::from_utf8_lossy(name).into_owned());
            found = true;
        }
    }
    found.then_some(table)
}

/// Glyph names of a bare CFF program's built-in encoding, or of the `CFF ` table of an
/// OpenType program. `None` when the program does not parse or is CID-keyed.
pub(crate) fn cff_builtin(program: &[u8], opentype: bool) -> Option<Vec<Option<String>>> {
    let data = if opentype {
        ttf_parser::RawFace::parse(program, 0)
            .ok()?
            .table(ttf_parser::Tag::from_bytes(b"CFF "))?
    } else {
        program
    };
    let table = ttf_parser::cff::Table::parse(data)?;
    // ponytail: ttf-parser's glyph_index falls back to StandardEncoding for codes a custom
    // encoding omits. Those codes draw .notdef, so only their text is affected; read the
    // encoding ourselves if that matters.
    let names: Vec<Option<String>> = (0..=255u8)
        .map(|code| {
            let gid = table.glyph_index(code).filter(|g| g.0 != 0)?;
            table.glyph_name(gid).map(str::to_owned)
        })
        .collect();
    names.iter().any(Option::is_some).then_some(names)
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glyph_names() {
        assert_eq!(glyph_unicode("A").as_deref(), Some("A"));
        assert_eq!(glyph_unicode("uni00410042").as_deref(), Some("AB"));
        assert_eq!(glyph_unicode("u1F600").as_deref(), Some("\u{1F600}"));
        assert_eq!(glyph_unicode("f_i").as_deref(), Some("fi"));
        assert_eq!(glyph_unicode("a.sc").as_deref(), Some("a"));
        assert_eq!(glyph_unicode("uniD800"), None);
        assert_eq!(glyph_unicode("u110000"), None);
        assert_eq!(glyph_unicode("uni004"), None);
        assert_eq!(glyph_unicode("g123"), None);
        assert_eq!(glyph_unicode(".notdef"), None);
    }

    #[test]
    fn type1_builtin_scan() {
        let src = b"%!PS-AdobeFont-1.0\n/Encoding 256 array\n0 1 255 {1 index exch /.notdef put} for\n\
                    dup 65 /Alpha put\ndup 300 /bad put\ndup 66 /Beta put\nreadonly def\ncurrentfile eexec\x80\x81dup 67 /C put";
        let t = type1_builtin(src).unwrap();
        assert_eq!(t[65].as_deref(), Some("Alpha"));
        assert_eq!(t[66].as_deref(), Some("Beta"));
        assert_eq!(t[67], None);
        assert!(type1_builtin(b"/Encoding StandardEncoding def currentfile eexec").is_none());
    }

    #[test]
    fn type1_builtin_compact_syntax() {
        let src = b"/Encoding 256 array\n0 1 255{1 index exch/.notdef put}for\ndup 1/Euro.symbol put\nreadonly def";
        let t = type1_builtin(src).unwrap();
        assert_eq!(t[1].as_deref(), Some("Euro.symbol"));
    }
}
