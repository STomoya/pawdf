//! Cross-reference loading (§7.5.4, §7.5.8) and repair.

use std::collections::{BTreeMap, HashSet};
use std::ops::Range;

use crate::crypt::Crypt;
use crate::diag::{DiagnosticKind, Diagnostics, Location, Origin};
use crate::lexer::{Lexer, Token, is_regular, is_whitespace};
use crate::object::{Dict, Object, Ref};
use crate::parser::{find, parse_header, parse_indirect, parse_indirect_end, parse_object};

/// An in-use object's location. Free entries are not stored: a free entry and a missing
/// one both resolve to null.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Entry {
    InFile { offset: usize, generation: u16 },
    InStream { stream: u32, index: u32 },
}

impl Entry {
    /// §7.5.8.3: objects in object streams have generation 0.
    pub(crate) fn generation(self) -> u16 {
        match self {
            Entry::InFile { generation, .. } => generation,
            Entry::InStream { .. } => 0,
        }
    }
}

pub(crate) type Entries = BTreeMap<u32, Entry>;

/// One cross-reference section. Free rows only delete older revisions, so they are kept
/// as runs: a tiny compressed xref stream cannot allocate millions of them.
#[derive(Default)]
struct Section {
    entries: Entries,
    free: Vec<Range<u32>>,
}

impl Section {
    fn free(&mut self, num: u32) {
        if num > MAX_OBJECT_NUM {
            return;
        }
        match self.free.last_mut() {
            Some(run) if run.end == num => run.end += 1,
            _ => self.free.push(num..num + 1),
        }
    }

    /// Folds in a hybrid file's `/XRefStm` section, which is searched after the table and
    /// before `/Prev` (§7.5.8.4). Its in-use entries beat the table's free rows because
    /// [`apply`] deletes before it inserts.
    fn hide(&mut self, hidden: Section) {
        for (num, e) in hidden.entries {
            self.entries.entry(num).or_insert(e);
        }
        self.free.extend(hidden.free);
    }
}

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

/// Applies `section` over the older ones in `into`: its free rows delete their objects
/// (§7.5.4), then its in-use entries replace older ones. Each removal undoes an earlier
/// insertion, so applying every section stays linear.
fn apply(into: &mut Entries, section: Section) {
    for run in section.free {
        let freed: Vec<u32> = into.range(run).map(|(&num, _)| num).collect();
        for num in freed {
            into.remove(&num);
        }
    }
    into.extend(section.entries);
}

/// Loads the xref chain starting at `startxref`. Returns `None` if the first section is
/// unreadable, so the caller can repair.
pub(crate) fn load(data: &[u8], diags: &mut Diagnostics) -> Option<(Entries, Dict, usize)> {
    let start = find_startxref(data)?;
    let mut sections = Vec::new();
    let mut trailers = Vec::new();
    let mut seen = HashSet::new();
    let mut next = Some(start);
    while let Some(offset) = next.take() {
        if seen.len() >= MAX_SECTIONS || !seen.insert(offset) {
            diags.push(DiagnosticKind::BadXrefEntry, Location::File(offset as u64));
            break;
        }
        let Some((mut section, sec_trailer)) = read_section(data, offset, diags) else {
            if offset == start {
                return None;
            }
            diags.push(DiagnosticKind::BadXrefEntry, Location::File(offset as u64));
            break;
        };
        if let Some(stm) = sec_trailer.get(b"XRefStm").and_then(int_offset) {
            if !seen.insert(stm) {
                diags.push(DiagnosticKind::BadXrefEntry, Location::File(stm as u64));
            } else {
                match read_section(data, stm, diags) {
                    Some((hidden, _)) => section.hide(hidden),
                    None => diags.push(DiagnosticKind::BadXrefEntry, Location::File(stm as u64)),
                }
            }
        }
        next = sec_trailer.get(b"Prev").and_then(int_offset);
        sections.push(section);
        trailers.push(sec_trailer);
    }
    let mut entries = Entries::new();
    for section in sections.into_iter().rev() {
        apply(&mut entries, section);
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

fn read_section(data: &[u8], offset: usize, diags: &mut Diagnostics) -> Option<(Section, Dict)> {
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
fn read_table(lex: &mut Lexer, diags: &mut Diagnostics) -> (Section, Dict) {
    let data_len = lex.data().len();
    let mut section = Section::default();
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
                Some((_, Token::Int(generation))),
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
                    let generation =
                        generation_or_zero(generation, Location::File(entry_pos as u64), diags);
                    section
                        .entries
                        .entry(num)
                        .or_insert(Entry::InFile { offset, generation });
                }
                (b"f", _) => section.free(num),
                _ => diags.push(
                    DiagnosticKind::BadXrefEntry,
                    Location::File(entry_pos as u64),
                ),
            }
        }
        first_subsection = false;
    }
    let trailer = read_trailer(lex, diags);
    (section, trailer)
}

/// Generations above 65,535 are invalid (§7.5.4); the entry is kept with generation 0,
/// since generations are not checked on resolve.
fn generation_or_zero(generation: impl TryInto<u16>, at: Location, diags: &mut Diagnostics) -> u16 {
    generation.try_into().unwrap_or_else(|_| {
        diags.push(DiagnosticKind::BadXrefEntry, at);
        0
    })
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
fn read_stream(data: &[u8], offset: usize, diags: &mut Diagnostics) -> Option<(Section, Dict)> {
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
        return Some((Section::default(), stream.dict));
    };
    if w0 > 8 || w1 > 8 || w2 > 8 {
        diags.push(DiagnosticKind::BadXrefEntry, loc);
        return Some((Section::default(), stream.dict));
    }
    let size = dict.get(b"Size").and_then(|s| s.as_int()).unwrap_or(0);
    let index: Vec<i64> = match dict.get(b"Index").and_then(|i| i.as_array()) {
        Some(a) => a.iter().filter_map(|o| o.as_int()).collect(),
        None => vec![0, size],
    };
    let row_len = w0 + w1 + w2;
    let mut rows = decoded.chunks_exact(row_len.max(1));
    let mut section = Section::default();
    let mut row_at = 0;
    let (pairs, unpaired) = index.as_chunks::<2>();
    if !unpaired.is_empty() {
        diags.push(DiagnosticKind::BadXrefEntry, loc);
    }
    'outer: for &[first, count] in pairs {
        let (Ok(first), Ok(count)) = (u32::try_from(first), u64::try_from(count)) else {
            diags.push(DiagnosticKind::BadXrefEntry, loc);
            continue;
        };
        for i in 0..count {
            let Some(row) = rows.next() else {
                diags.push(DiagnosticKind::BadXrefEntry, loc);
                break 'outer;
            };
            let row_loc = Location::Stream {
                obj: r,
                offset: row_at as u64,
            };
            row_at += row.len();
            let Some(num) = u32::try_from(i).ok().and_then(|i| first.checked_add(i)) else {
                diags.push(DiagnosticKind::BadXrefEntry, row_loc);
                break 'outer;
            };
            let (f0, rest) = row.split_at(w0);
            let (f1, f2) = rest.split_at(w1);
            let kind = if w0 == 0 { 1 } else { be(f0) };
            // Type 0 is free; §7.5.8.3 treats unknown types as null references.
            let entry = match kind {
                1 => match usize::try_from(be(f1)) {
                    Ok(offset) if offset > 0 && offset < data.len() => Ok(Entry::InFile {
                        offset,
                        generation: generation_or_zero(be(f2), row_loc, diags),
                    }),
                    _ => Err(()),
                },
                2 => match (u32::try_from(be(f1)), u32::try_from(be(f2))) {
                    (Ok(stream), Ok(index)) => Ok(Entry::InStream { stream, index }),
                    _ => Err(()),
                },
                _ => {
                    section.free(num);
                    continue;
                }
            };
            match entry {
                Ok(entry) if num <= MAX_OBJECT_NUM => {
                    section.entries.entry(num).or_insert(entry);
                }
                _ => diags.push(DiagnosticKind::BadXrefEntry, row_loc),
            }
        }
    }
    Some((section, stream.dict))
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
/// when no direct definition exists. If no trailer names a `/Root` the scan found, the
/// last `/Type /Catalog` object is used, since an incremental update may add a new one.
///
/// The scan is a single forward pass: candidates inside an object or trailer already
/// parsed are skipped, so crafted files cannot make it quadratic.
///
/// In an encrypted file, object streams are decrypted with `crypt` before being decoded;
/// without it their members are not found. Indirect `/Filter` and `/DecodeParms` values,
/// which name the crypt filter, are read through `known`, a table from an earlier scan or
/// the xref.
pub(crate) fn repair(data: &[u8], crypt: Option<(&Crypt, &Entries)>) -> (Entries, Dict) {
    let mut hits: Vec<(usize, Option<(u32, u16)>)> = memfind_all(data, b"obj")
        .filter_map(|pos| object_start(data, pos))
        .map(|(num, generation, start)| (start, Some((num, generation))))
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
        let Some((num, generation)) = num else {
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
        entries.insert(
            num,
            Entry::InFile {
                offset: pos,
                generation,
            },
        );
        let Object::Stream(s) = &obj else {
            if is_catalog(&obj) {
                catalog = Some(r);
            }
            continue;
        };
        match s.dict.get(b"Type").and_then(|t| t.as_name()) {
            Some(b"XRef") => xref_dicts.push(s.dict.clone()),
            Some(b"ObjStm") => {
                let origin = Origin::Stream(r);
                let decrypted;
                let s = match crypt {
                    Some((c, known)) => {
                        let mut copy = s.clone();
                        for key in [&b"Filter"[..], b"DecodeParms"] {
                            if let Some(v) = copy.dict.get_mut(key) {
                                in_file_values(data, known, v);
                            }
                        }
                        c.decrypt_stream(r, &mut copy, Location::File(pos as u64), &mut quiet);
                        decrypted = copy;
                        &decrypted
                    }
                    None => s,
                };
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
                    let entry = Entry::InStream { stream: num, index };
                    if *entries.entry(member).or_insert(entry) == entry {
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
    let root_found =
        matches!(trailer.get(b"Root"), Some(Object::Ref(r)) if entries.contains_key(&r.num));
    if !root_found && let Some(r) = catalog {
        trailer.insert("Root".into(), Object::Ref(r));
    }
    (entries, trailer)
}

/// Replaces a reference in `value`, or in its top-level array items, with the in-file
/// object `known` places there. Objects in object streams are left as references.
fn in_file_values(data: &[u8], known: &Entries, value: &mut Object) {
    let load = |o: &mut Object| {
        if let Object::Ref(r) = o
            && let Some(Entry::InFile { offset, .. }) = known.get(&r.num)
            && header_matches(data, *offset, r.num)
            && let Some((_, obj, _)) =
                parse_indirect_end(data, *offset, &|_| None, &mut Diagnostics::new())
        {
            *o = obj;
        }
    };
    load(value);
    if let Object::Array(items) = value {
        items.iter_mut().for_each(load);
    }
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
/// number, the generation (0 if out of range), and the offset of the number's first digit.
fn object_start(data: &[u8], pos: usize) -> Option<(u32, u16, usize)> {
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
    if !skip_ws(&mut i) {
        return None;
    }
    let gen_end = i;
    if !skip_digits(&mut i) {
        return None;
    }
    let gen_start = i;
    if !skip_ws(&mut i) {
        return None;
    }
    let end = i;
    if !skip_digits(&mut i) || (i > 0 && is_regular(data[i - 1])) {
        return None;
    }
    let num = std::str::from_utf8(&data[i..end]).ok()?.parse().ok()?;
    let generation = std::str::from_utf8(&data[gen_start..gen_end])
        .ok()
        .and_then(|g| g.parse().ok())
        .unwrap_or(0);
    Some((num, generation, i))
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

    /// Review I2: free rows cost one run, however many a stream declares.
    #[test]
    fn free_rows_are_stored_as_runs() {
        let data = xref_stream("/Size 1000 /W [1 0 0]", &[0; 1000]);
        let (section, _) = read_stream(&data, 0, &mut Diagnostics::new()).unwrap();
        assert!(section.entries.is_empty());
        assert_eq!(section.free, vec![0..1000]);
    }

    /// #17: unknown row types are null references (§7.5.8.3), so they free the object too.
    #[test]
    fn stream_rows_give_generations_and_free_runs() {
        let data = xref_stream("/Size 4 /W [1 1 1]", &[0, 0, 0, 1, 5, 3, 7, 0, 0, 0, 0, 0]);
        let (section, _) = read_stream(&data, 0, &mut Diagnostics::new()).unwrap();
        assert_eq!(section.entries[&1].generation(), 3);
        assert_eq!(section.free, vec![0..1, 2..4]);
    }

    /// Review I2: object numbers beyond the 8,388,607 implementation limit are dropped.
    #[test]
    fn object_numbers_beyond_limit_are_dropped() {
        let data = xref_stream(
            "/Size 9000000 /Index [8388606 3] /W [1 1 0]",
            &[1, 5, 1, 5, 1, 5],
        );
        let (section, _) = read_stream(&data, 0, &mut Diagnostics::new()).unwrap();
        assert_eq!(
            section.entries.keys().copied().collect::<Vec<_>>(),
            vec![8388606, 8388607]
        );
    }

    /// An oversized generation keeps the entry with generation 0, as in a classic table.
    #[test]
    fn oversized_stream_generation_keeps_entry() {
        let data = xref_stream("/Size 1 /W [1 1 4]", &[1, 5, 0, 1, 0, 0]);
        let mut d = Diagnostics::new();
        let (section, _) = read_stream(&data, 0, &mut d).unwrap();
        assert_eq!(section.entries[&0], at(5));
        assert_eq!(
            d.as_slice()
                .iter()
                .filter(|x| x.kind == DiagnosticKind::BadXrefEntry)
                .count(),
            1
        );
    }

    #[test]
    fn offsets_past_eof_are_dropped() {
        let data = xref_stream("/Size 3 /W [1 2 0]", &[1, 0, 5, 1, 0xff, 0xff, 1, 0, 6]);
        let mut d = Diagnostics::new();
        let (section, _) = read_stream(&data, 0, &mut d).unwrap();
        assert_eq!(
            section.entries.keys().copied().collect::<Vec<_>>(),
            vec![0, 2]
        );
        assert_eq!(
            d.as_slice()
                .iter()
                .filter(|x| x.kind == DiagnosticKind::BadXrefEntry)
                .count(),
            1
        );
    }

    /// Review (PR #3): §7.5.8.2 `/Index` holds pairs; a trailing unpaired number is diagnosed.
    #[test]
    fn odd_index_length_is_diagnosed() {
        let data = xref_stream("/Size 3 /Index [0 1 2] /W [1 2 0]", &[1, 0, 5]);
        let mut d = Diagnostics::new();
        let (section, _) = read_stream(&data, 0, &mut d).unwrap();
        assert_eq!(section.entries.keys().copied().collect::<Vec<_>>(), vec![0]);
        assert_eq!(
            d.as_slice()
                .iter()
                .filter(|x| x.kind == DiagnosticKind::BadXrefEntry)
                .count(),
            1
        );
    }

    #[test]
    fn object_start_finds_headers() {
        let data = b"x\n12 0 obj\n<<>>\nendobj 3 0 obj";
        let found: Vec<_> = memfind_all(data, b"obj")
            .filter_map(|p| object_start(data, p))
            .collect();
        assert_eq!(found, vec![(12, 0, 2), (3, 0, 23)]);
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
        let (_, t) = repair(&data, None);
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

    fn at(offset: usize) -> Entry {
        Entry::InFile {
            offset,
            generation: 0,
        }
    }

    fn section(entries: &[(u32, Entry)], free: &[u32]) -> Section {
        let mut s = Section {
            entries: entries.iter().copied().collect(),
            free: Vec::new(),
        };
        free.iter().for_each(|&num| s.free(num));
        s
    }

    #[test]
    fn apply_frees_then_inserts() {
        let mut entries = Entries::from([(1, at(5)), (2, at(7)), (3, at(9))]);
        apply(&mut entries, section(&[(2, at(11))], &[1, 2]));
        assert_eq!(entries, Entries::from([(2, at(11)), (3, at(9))]));
    }

    /// §7.5.8.4: a hybrid table's free row does not hide the `/XRefStm` entry, and the
    /// table's own in-use entries win.
    #[test]
    fn hidden_section_beats_free_rows_only() {
        let mut table = section(&[(2, at(7))], &[1]);
        table.hide(section(&[(1, at(5)), (2, at(9))], &[3]));
        let mut entries = Entries::from([(3, at(1))]);
        apply(&mut entries, table);
        assert_eq!(entries, Entries::from([(1, at(5)), (2, at(7))]));
    }
}
