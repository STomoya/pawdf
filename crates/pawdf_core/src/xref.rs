//! Cross-reference loading (§7.5.4, §7.5.8) and repair.

use std::collections::{BTreeMap, HashSet};

use crate::diag::{DiagnosticKind, Diagnostics, Location, Origin};
use crate::lexer::{Lexer, Token, is_regular, is_whitespace};
use crate::object::{Dict, Object, Ref};
use crate::parser::{find, parse_header, parse_indirect, parse_indirect_end, parse_object};

/// An in-use object's location. Free entries are not stored: a free entry and a missing
/// one both resolve to null, and storing them would let a tiny compressed xref stream
/// allocate millions of entries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Entry {
    InFile { offset: usize },
    InStream { stream: u32, index: u32 },
}

pub(crate) type Entries = BTreeMap<u32, Entry>;

/// Bounds the number of `/Prev` sections followed.
const MAX_SECTIONS: usize = 4096;

/// Largest object number kept: the 8,388,607 indirect-object limit of ISO 32000-1 Annex C.
/// It bounds the entries a crafted xref stream can create.
const MAX_OBJECT_NUM: u32 = 8_388_607;

/// Byte offset of the last `startxref` value.
fn find_startxref(data: &[u8]) -> Option<usize> {
    let pos = data.windows(9).rposition(|w| w == b"startxref")?;
    match Lexer::at(data, pos + 9).next_token()? {
        (_, Token::Int(off)) => usize::try_from(off).ok(),
        _ => None,
    }
}

/// Newer sections win. Free entries are never stored, so a table that marks an object
/// free does not hide the in-use entry a hybrid file's `/XRefStm` gives it.
fn merge(into: &mut Entries, from: Entries) {
    for (num, e) in from {
        into.entry(num).or_insert(e);
    }
}

/// Loads the xref chain starting at `startxref`. Returns `None` if the first section is
/// unreadable, so the caller can repair.
pub(crate) fn load(data: &[u8], diags: &mut Diagnostics) -> Option<(Entries, Dict, usize)> {
    let start = find_startxref(data)?;
    let mut entries = Entries::new();
    let mut trailers = Vec::new();
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
            if !seen.insert(stm) {
                diags.push(DiagnosticKind::BadXrefEntry, Location::File(stm as u64));
            } else {
                match read_section(data, stm, diags) {
                    Some((hidden, _)) => merge(&mut entries, hidden),
                    None => diags.push(DiagnosticKind::BadXrefEntry, Location::File(stm as u64)),
                }
            }
        }
        next = sec_trailer.get(b"Prev").and_then(int_offset);
        trailers.push(sec_trailer);
    }
    // Merged once, newest section last so it wins: merging per section is quadratic in
    // the number of trailer keys.
    let trailer = Dict::from_entries(
        trailers
            .into_iter()
            .rev()
            .flat_map(Dict::into_entries)
            .collect(),
    );
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
    let data_len = lex.data().len();
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
            match (kind.as_slice(), usize::try_from(off)) {
                (b"n", Ok(offset)) if offset > 0 && offset < data_len && num <= MAX_OBJECT_NUM => {
                    entries.entry(num).or_insert(Entry::InFile { offset });
                }
                (b"f", _) => {}
                _ => diags.push(
                    DiagnosticKind::BadXrefEntry,
                    Location::File(entry_pos as u64),
                ),
            }
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
            // Type 0 is free; §7.5.8.3 treats unknown types as null references.
            let entry = match kind {
                1 => usize::try_from(be(f1))
                    .ok()
                    .filter(|&offset| offset > 0 && offset < data.len())
                    .map(|offset| Entry::InFile { offset }),
                2 => match (u32::try_from(be(f1)), u32::try_from(be(f2))) {
                    (Ok(stream), Ok(index)) => Some(Entry::InStream { stream, index }),
                    _ => None,
                },
                _ => None,
            };
            if let Some(entry) = entry
                && num <= MAX_OBJECT_NUM
            {
                entries.entry(num).or_insert(entry);
            }
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
///
/// The scan is a single forward pass: candidates inside an object or trailer already
/// parsed are skipped, so crafted files cannot make it quadratic.
pub(crate) fn repair(data: &[u8]) -> (Entries, Dict) {
    let mut hits: Vec<(usize, Option<u32>)> = memfind_all(data, b"obj")
        .filter_map(|pos| object_start(data, pos))
        .map(|(num, start)| (start, Some(num)))
        .chain(memfind_all(data, b"trailer").map(|pos| (pos, None)))
        .collect();
    hits.sort_unstable_by_key(|&(pos, _)| pos);
    let mut entries = Entries::new();
    // `trailer` dictionaries beat xref-stream dictionaries; later trailers win, and the
    // first xref stream wins. Collected and merged once to stay linear in the key count.
    let mut trailer_parts = Vec::new();
    let mut xref_dicts = Vec::new();
    let mut catalog = None;
    let mut quiet = Diagnostics::new();
    let mut consumed = 0;
    for (pos, num) in hits {
        if pos < consumed {
            continue;
        }
        let Some(num) = num else {
            let mut lex = Lexer::at(data, pos + 7);
            if let Some(Object::Dict(d)) = parse_object(&mut lex, Origin::File, &mut quiet) {
                trailer_parts.extend(d.into_entries());
            }
            consumed = lex.pos();
            continue;
        };
        let Some((r, obj, end)) = parse_indirect_end(data, pos, &|_| None, &mut quiet) else {
            continue;
        };
        consumed = end;
        if num > MAX_OBJECT_NUM {
            continue;
        }
        entries.insert(num, Entry::InFile { offset: pos });
        let Object::Stream(s) = &obj else {
            if catalog.is_none() && is_catalog(&obj) {
                catalog = Some(r);
            }
            continue;
        };
        match s.dict.get(b"Type").and_then(|t| t.as_name()) {
            Some(b"XRef") => xref_dicts.push(s.dict.clone()),
            Some(b"ObjStm") => {
                let origin = Origin::Stream(r);
                let decoded = s.decode(origin.at(0), &mut quiet).data;
                let first = s.dict.get(b"First").and_then(|o| o.as_int()).unwrap_or(0);
                let first = usize::try_from(first).unwrap_or(0);
                for (index, (member, off)) in
                    objstm_header(&s.dict, &decoded).into_iter().enumerate()
                {
                    if member > MAX_OBJECT_NUM {
                        continue;
                    }
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
    let mut parts: Vec<_> = xref_dicts
        .into_iter()
        .rev()
        .flat_map(Dict::into_entries)
        .collect();
    parts.extend(trailer_parts);
    let mut trailer = Dict::from_entries(parts);
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

    fn xref_stream(extra: &str, rows: &[u8]) -> Vec<u8> {
        let mut out = format!(
            "1 0 obj\n<< /Type /XRef {extra} /Length {} >>\nstream\n",
            rows.len()
        )
        .into_bytes();
        out.extend_from_slice(rows);
        out.extend_from_slice(b"\nendstream\nendobj\n");
        out
    }

    /// Review I2: free rows cost nothing, however many a stream declares.
    #[test]
    fn free_rows_are_not_stored() {
        let data = xref_stream("/Size 1000 /W [1 0 0]", &[0; 1000]);
        let (entries, _) = read_stream(&data, 0, &mut Diagnostics::new()).unwrap();
        assert!(entries.is_empty());
    }

    /// Review I2: object numbers beyond the 8,388,607 implementation limit are dropped.
    #[test]
    fn object_numbers_beyond_limit_are_dropped() {
        let data = xref_stream(
            "/Size 9000000 /Index [8388606 3] /W [1 1 0]",
            &[1, 5, 1, 5, 1, 5],
        );
        let (entries, _) = read_stream(&data, 0, &mut Diagnostics::new()).unwrap();
        assert_eq!(
            entries.keys().copied().collect::<Vec<_>>(),
            vec![8388606, 8388607]
        );
    }

    #[test]
    fn offsets_past_eof_are_dropped() {
        let data = xref_stream("/Size 3 /W [1 2 0]", &[1, 0, 5, 1, 0xff, 0xff, 1, 0, 6]);
        let (entries, _) = read_stream(&data, 0, &mut Diagnostics::new()).unwrap();
        assert_eq!(entries.keys().copied().collect::<Vec<_>>(), vec![0, 2]);
    }

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
    fn repair_trailer_precedence() {
        let mut data = b"trailer << /Root 1 0 R /A 1 >>\n".to_vec();
        data.extend(xref_stream("/Root 2 0 R /A 2 /B 1 /Size 0 /W [1 0 0]", b""));
        data.extend(xref_stream("/B 2 /Size 0 /W [1 0 0]", b""));
        data.extend(b"trailer << /A 3 >>\n");
        let (_, t) = repair(&data);
        assert_eq!(
            t.get(b"Root"),
            Some(&Object::Ref(Ref {
                num: 1,
                generation: 0
            }))
        );
        assert_eq!(t.get(b"A"), Some(&Object::Int(3)));
        assert_eq!(t.get(b"B"), Some(&Object::Int(1)));
    }

    #[test]
    fn merge_keeps_newer_entries() {
        let mut a = Entries::from([(2, Entry::InFile { offset: 9 })]);
        let b = Entries::from([
            (1, Entry::InFile { offset: 5 }),
            (2, Entry::InFile { offset: 7 }),
        ]);
        merge(&mut a, b);
        assert_eq!(a[&1], Entry::InFile { offset: 5 });
        assert_eq!(a[&2], Entry::InFile { offset: 9 });
    }
}
