//! Cross-reference loading (§7.5.4, §7.5.8) and repair.

use std::collections::{BTreeMap, HashSet};

use crate::diag::{DiagnosticKind, Diagnostics, Location, Origin};
use crate::lexer::{Lexer, Token, is_regular, is_whitespace};
use crate::object::{Dict, Object, Ref};
use crate::parser::{find, parse_header, parse_indirect, parse_object};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Entry {
    Free,
    InFile { offset: usize },
    InStream { stream: u32, index: u32 },
}

pub(crate) type Entries = BTreeMap<u32, Entry>;

/// Bounds the number of `/Prev` sections followed.
const MAX_SECTIONS: usize = 4096;

/// Byte offset of the last `startxref` value.
fn find_startxref(data: &[u8]) -> Option<usize> {
    let pos = data.windows(9).rposition(|w| w == b"startxref")?;
    match Lexer::at(data, pos + 9).next_token()? {
        (_, Token::Int(off)) => usize::try_from(off).ok(),
        _ => None,
    }
}

/// Newer sections win. A free entry never hides an in-use entry from an older section,
/// because hybrid files mark objects free in the table that their `/XRefStm` defines.
fn merge(into: &mut Entries, from: Entries) {
    for (num, e) in from {
        match into.get(&num) {
            None | Some(Entry::Free) => {
                into.insert(num, e);
            }
            Some(_) => {}
        }
    }
}

fn merge_trailer(into: &mut Dict, from: &Dict) {
    for (k, v) in from.iter() {
        if !into.contains_key(k.as_bytes()) {
            into.insert(k.clone(), v.clone());
        }
    }
}

/// Loads the xref chain starting at `startxref`. Returns `None` if the first section is
/// unreadable, so the caller can repair.
pub(crate) fn load(data: &[u8], diags: &mut Diagnostics) -> Option<(Entries, Dict, usize)> {
    let start = find_startxref(data)?;
    let mut entries = Entries::new();
    let mut trailer = Dict::new();
    let mut seen = HashSet::new();
    let mut next = Some(start);
    while let Some(offset) = next.take() {
        if seen.len() >= MAX_SECTIONS || !seen.insert(offset) {
            diags.push(DiagnosticKind::BadXrefEntry, Location::File(offset as u64));
            break;
        }
        let Some((section, sec_trailer)) = read_section(data, offset, diags) else {
            if offset == start {
                return None;
            }
            diags.push(DiagnosticKind::BadXrefEntry, Location::File(offset as u64));
            break;
        };
        merge(&mut entries, section);
        if let Some(stm) = sec_trailer.get(b"XRefStm").and_then(int_offset) {
            match read_section(data, stm, diags) {
                Some((hidden, _)) => merge(&mut entries, hidden),
                None => diags.push(DiagnosticKind::BadXrefEntry, Location::File(stm as u64)),
            }
        }
        merge_trailer(&mut trailer, &sec_trailer);
        next = sec_trailer.get(b"Prev").and_then(int_offset);
    }
    Some((entries, trailer, start))
}

fn int_offset(o: &Object) -> Option<usize> {
    o.as_int().and_then(|i| usize::try_from(i).ok())
}

fn read_section(data: &[u8], offset: usize, diags: &mut Diagnostics) -> Option<(Entries, Dict)> {
    let mut lex = Lexer::at(data, offset);
    match lex.peek_token()? {
        (_, Token::Keyword(k)) if k == b"xref" => {
            lex.next_token();
            Some(read_table(&mut lex, diags))
        }
        _ => read_stream(data, offset, diags),
    }
}

/// Classic table (§7.5.4). Malformed entries end the table early; the trailer is then
/// searched for.
fn read_table(lex: &mut Lexer, diags: &mut Diagnostics) -> (Entries, Dict) {
    let mut entries = Entries::new();
    let mut first_subsection = true;
    'sections: while let Some((pos, tok)) = lex.next_token() {
        let (mut first, count) = match (tok, lex.next_token()) {
            (Token::Keyword(k), _) if k == b"trailer" => {
                lex.set_pos(pos);
                break;
            }
            (Token::Int(first), Some((_, Token::Int(count)))) if first >= 0 && count >= 0 => {
                (first, count)
            }
            _ => {
                diags.push(DiagnosticKind::BadXrefEntry, Location::File(pos as u64));
                break;
            }
        };
        for i in 0..count {
            let entry_pos = lex.pos();
            let (
                Some((_, Token::Int(off))),
                Some((_, Token::Int(_gen))),
                Some((_, Token::Keyword(kind))),
            ) = (lex.next_token(), lex.next_token(), lex.next_token())
            else {
                diags.push(
                    DiagnosticKind::BadXrefEntry,
                    Location::File(entry_pos as u64),
                );
                break 'sections;
            };
            // Some writers start the first subsection at 1 while listing object 0's entry.
            if first_subsection && i == 0 && first == 1 && kind == b"f" && off == 0 {
                diags.push(
                    DiagnosticKind::BadXrefEntry,
                    Location::File(entry_pos as u64),
                );
                first = 0;
            }
            let Ok(num) = u32::try_from(first + i) else {
                break 'sections;
            };
            let entry = match (kind.as_slice(), usize::try_from(off)) {
                (b"n", Ok(offset)) if offset > 0 => Entry::InFile { offset },
                (b"f", _) => Entry::Free,
                _ => {
                    diags.push(
                        DiagnosticKind::BadXrefEntry,
                        Location::File(entry_pos as u64),
                    );
                    Entry::Free
                }
            };
            entries.entry(num).or_insert(entry);
        }
        first_subsection = false;
    }
    let trailer = read_trailer(lex, diags);
    (entries, trailer)
}

fn read_trailer(lex: &mut Lexer, diags: &mut Diagnostics) -> Dict {
    let data = lex.data();
    let Some(rel) = find(&data[lex.pos()..], b"trailer") else {
        diags.push(
            DiagnosticKind::BadXrefEntry,
            Location::File(lex.pos() as u64),
        );
        return Dict::new();
    };
    lex.set_pos(lex.pos() + rel + 7);
    match parse_object(lex, Origin::File, diags) {
        Some(Object::Dict(d)) => d,
        _ => {
            diags.push(
                DiagnosticKind::BadXrefEntry,
                Location::File(lex.pos() as u64),
            );
            Dict::new()
        }
    }
}

/// Cross-reference stream (§7.5.8). Its dictionary doubles as the trailer.
fn read_stream(data: &[u8], offset: usize, diags: &mut Diagnostics) -> Option<(Entries, Dict)> {
    let (r, obj) = parse_indirect(data, offset, &|_| None, diags)?;
    let Object::Stream(stream) = obj else {
        return None;
    };
    let dict = &stream.dict;
    if dict.get(b"Type").and_then(|t| t.as_name()) != Some(b"XRef") {
        return None;
    }
    let loc = Location::Stream { obj: r, offset: 0 };
    let decoded = stream.decode(loc, diags).data;
    let w: Vec<usize> = dict
        .get(b"W")
        .and_then(|w| w.as_array())
        .unwrap_or_default()
        .iter()
        .filter_map(int_offset)
        .collect();
    let [w0, w1, w2] = w[..] else {
        diags.push(DiagnosticKind::BadXrefEntry, loc);
        return Some((Entries::new(), stream.dict));
    };
    if w0 > 8 || w1 > 8 || w2 > 8 {
        diags.push(DiagnosticKind::BadXrefEntry, loc);
        return Some((Entries::new(), stream.dict));
    }
    let size = dict.get(b"Size").and_then(|s| s.as_int()).unwrap_or(0);
    let index: Vec<i64> = match dict.get(b"Index").and_then(|i| i.as_array()) {
        Some(a) => a.iter().filter_map(|o| o.as_int()).collect(),
        None => vec![0, size],
    };
    let row_len = w0 + w1 + w2;
    let mut rows = decoded.chunks_exact(row_len.max(1));
    let mut entries = Entries::new();
    'outer: for &[first, count] in index.as_chunks::<2>().0 {
        let (Ok(first), Ok(count)) = (u32::try_from(first), u64::try_from(count)) else {
            diags.push(DiagnosticKind::BadXrefEntry, loc);
            continue;
        };
        for i in 0..count {
            let Some(row) = rows.next() else {
                diags.push(DiagnosticKind::BadXrefEntry, loc);
                break 'outer;
            };
            let Some(num) = u32::try_from(i).ok().and_then(|i| first.checked_add(i)) else {
                break 'outer;
            };
            let (f0, rest) = row.split_at(w0);
            let (f1, f2) = rest.split_at(w1);
            let kind = if w0 == 0 { 1 } else { be(f0) };
            let entry = match kind {
                0 => Entry::Free,
                1 => match usize::try_from(be(f1)) {
                    Ok(offset) if offset > 0 => Entry::InFile { offset },
                    _ => Entry::Free,
                },
                2 => match (u32::try_from(be(f1)), u32::try_from(be(f2))) {
                    (Ok(stream), Ok(index)) => Entry::InStream { stream, index },
                    _ => Entry::Free,
                },
                // §7.5.8.3: unknown types are treated as null references.
                _ => Entry::Free,
            };
            entries.entry(num).or_insert(entry);
        }
    }
    Some((entries, stream.dict))
}

fn be(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0, |acc, &b| acc << 8 | u64::from(b))
}

/// True if `offset` holds `num G obj`.
pub(crate) fn header_matches(data: &[u8], offset: usize, num: u32) -> bool {
    parse_header(&mut Lexer::at(data, offset)).is_some_and(|r| r.num == num)
}

/// Rebuilds the cross-reference by scanning for `N G obj` and `trailer` (what mature readers
/// do with broken files). Later definitions win. Objects inside object streams are added
/// when no direct definition exists. If no trailer names a `/Root`, the first
/// `/Type /Catalog` object is used.
pub(crate) fn repair(data: &[u8]) -> (Entries, Dict) {
    let mut entries = Entries::new();
    let mut quiet = Diagnostics::new();
    for pos in memfind_all(data, b"obj") {
        if let Some((num, start)) = object_start(data, pos) {
            entries.insert(num, Entry::InFile { offset: start });
        }
    }
    let mut trailer = Dict::new();
    for pos in memfind_all(data, b"trailer") {
        let mut lex = Lexer::at(data, pos + 7);
        if let Some(Object::Dict(d)) = parse_object(&mut lex, Origin::File, &mut quiet) {
            for (k, v) in d.iter() {
                trailer.insert(k.clone(), v.clone());
            }
        }
    }
    let direct: Vec<(u32, usize)> = entries
        .iter()
        .filter_map(|(&n, e)| match *e {
            Entry::InFile { offset } => Some((n, offset)),
            _ => None,
        })
        .collect();
    let mut catalog = None;
    for (num, offset) in direct {
        let Some((r, obj)) = parse_indirect(data, offset, &|_| None, &mut quiet) else {
            continue;
        };
        let Object::Stream(s) = &obj else {
            if catalog.is_none() && is_catalog(&obj) {
                catalog = Some(r);
            }
            continue;
        };
        match s.dict.get(b"Type").and_then(|t| t.as_name()) {
            Some(b"XRef") => merge_trailer(&mut trailer, &s.dict),
            Some(b"ObjStm") => {
                let origin = Origin::Stream(r);
                let decoded = s.decode(origin.at(0), &mut quiet).data;
                let first = s.dict.get(b"First").and_then(|o| o.as_int()).unwrap_or(0);
                let first = usize::try_from(first).unwrap_or(0);
                for (index, (member, off)) in
                    objstm_header(&s.dict, &decoded).into_iter().enumerate()
                {
                    let index = u32::try_from(index).unwrap_or(u32::MAX);
                    entries
                        .entry(member)
                        .or_insert(Entry::InStream { stream: num, index });
                    if catalog.is_none() {
                        let mut lex = Lexer::at(&decoded, first.saturating_add(off));
                        if parse_object(&mut lex, origin, &mut quiet)
                            .is_some_and(|o| is_catalog(&o))
                        {
                            catalog = Some(Ref {
                                num: member,
                                generation: 0,
                            });
                        }
                    }
                }
            }
            _ => {}
        }
    }
    if !trailer.contains_key(b"Root")
        && let Some(r) = catalog
    {
        trailer.insert("Root".into(), Object::Ref(r));
    }
    (entries, trailer)
}

fn is_catalog(obj: &Object) -> bool {
    obj.as_dict()
        .and_then(|d| d.get(b"Type"))
        .and_then(|t| t.as_name())
        == Some(b"Catalog")
}

/// `/N` pairs of `(object number, offset)` from an object stream header (§7.5.7).
/// Offsets are relative to `/First`.
pub(crate) fn objstm_header(dict: &Dict, decoded: &[u8]) -> Vec<(u32, usize)> {
    let n = dict.get(b"N").and_then(|o| o.as_int()).unwrap_or(0).max(0);
    let mut lex = Lexer::new(decoded);
    let mut out = Vec::new();
    for _ in 0..n {
        let (Some((_, Token::Int(num))), Some((_, Token::Int(off)))) =
            (lex.next_token(), lex.next_token())
        else {
            break;
        };
        let (Ok(num), Ok(off)) = (u32::try_from(num), usize::try_from(off)) else {
            break;
        };
        out.push((num, off));
    }
    out
}

fn memfind_all<'a>(data: &'a [u8], needle: &'a [u8]) -> impl Iterator<Item = usize> + 'a {
    data.windows(needle.len())
        .enumerate()
        .filter(move |(_, w)| *w == needle)
        .map(|(i, _)| i)
}

/// For an `obj` keyword at `pos`, walks back over `num WS gen WS` and returns the object
/// number and the offset of its first digit.
fn object_start(data: &[u8], pos: usize) -> Option<(u32, usize)> {
    if data.get(pos + 3).is_some_and(|&b| is_regular(b)) {
        return None;
    }
    let mut i = pos;
    let skip_ws = |i: &mut usize| {
        let s = *i;
        while *i > 0 && is_whitespace(data[*i - 1]) {
            *i -= 1;
        }
        *i < s
    };
    let skip_digits = |i: &mut usize| {
        let s = *i;
        while *i > 0 && data[*i - 1].is_ascii_digit() {
            *i -= 1;
        }
        *i < s
    };
    if !(skip_ws(&mut i) && skip_digits(&mut i) && skip_ws(&mut i)) {
        return None;
    }
    let end = i;
    if !skip_digits(&mut i) || (i > 0 && is_regular(data[i - 1])) {
        return None;
    }
    let num = std::str::from_utf8(&data[i..end]).ok()?.parse().ok()?;
    Some((num, i))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_start_finds_headers() {
        let data = b"x\n12 0 obj\n<<>>\nendobj 3 0 obj";
        let found: Vec<_> = memfind_all(data, b"obj")
            .filter_map(|p| object_start(data, p))
            .collect();
        assert_eq!(found, vec![(12, 2), (3, 23)]);
    }

    #[test]
    fn object_start_rejects_non_headers() {
        let data = b"a12 0 obj 1 objx";
        assert!(memfind_all(data, b"obj").all(|p| object_start(data, p).is_none()));
    }

    #[test]
    fn startxref_uses_last_occurrence() {
        assert_eq!(
            find_startxref(b"startxref 5 %%EOF startxref\n17\n%%EOF"),
            Some(17)
        );
        assert_eq!(find_startxref(b"no marker"), None);
    }

    #[test]
    fn merge_lets_in_use_override_free() {
        let mut a = Entries::from([(1, Entry::Free), (2, Entry::InFile { offset: 9 })]);
        let b = Entries::from([
            (1, Entry::InFile { offset: 5 }),
            (2, Entry::InFile { offset: 7 }),
        ]);
        merge(&mut a, b);
        assert_eq!(a[&1], Entry::InFile { offset: 5 });
        assert_eq!(a[&2], Entry::InFile { offset: 9 });
    }
}
