//! CMaps (§9.7.5, §9.10.3): codespace ranges, CID mappings, `bf` mappings (ToUnicode),
//! `/WMode`, `CIDSystemInfo`, and `usecmap`.

use std::collections::HashMap;
use std::sync::Arc;

use pawdf_core::{DiagnosticKind, Diagnostics, Location, Object, Origin, Ref, parse_content};

use crate::model::WritingMode;

/// Larger `bfrange`s are rejected; real ToUnicode ranges stay within one byte (§9.10.3).
const MAX_RANGE: u64 = 1 << 16;
/// Total mappings kept (`bf` entries plus CID ranges), so crafted input cannot exhaust memory.
const MAX_ENTRIES: usize = 1 << 20;
/// Total `bfrange` codes expanded per CMap. Repeated ranges over the same codes do not
/// grow the map, so the entry cap alone does not bound the work.
const MAX_RANGE_WORK: u64 = 1 << 20;
/// Codespace ranges kept per CMap, inherited ones included. Real CMaps declare a handful,
/// and every code scans them all.
const MAX_CODESPACE: usize = 64;
/// Longest `usecmap` chain followed.
pub(crate) const MAX_USECMAP_DEPTH: usize = 8;

/// One character code taken from a string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Code {
    pub value: u32,
    /// Bytes consumed, 1..=4 (0 only for an empty string).
    pub len: u8,
    /// Whether the bytes matched a codespace range (§9.7.6.2).
    pub matched: bool,
}

impl Code {
    /// The code as big-endian bytes of its length.
    pub fn bytes(self) -> Vec<u8> {
        let n = usize::from(self.len).min(4);
        self.value.to_be_bytes()[4 - n..].to_vec()
    }
}

/// What a `usecmap` refers to.
pub(crate) enum UseCMap<'a> {
    /// A predefined CMap name.
    Name(&'a [u8]),
    /// An embedded CMap stream (`/UseCMap` in a stream dictionary).
    Stream(Ref),
}

/// Resolves a `usecmap` at the given depth.
pub(crate) type Resolve<'a> =
    &'a mut dyn FnMut(UseCMap<'_>, usize, &mut Diagnostics) -> Option<Arc<CMap>>;

#[derive(Debug, Clone, Copy)]
struct CidRange {
    len: u8,
    lo: u32,
    hi: u32,
    cid: u32,
}

/// A parsed CMap.
#[derive(Debug, Clone)]
pub(crate) struct CMap {
    codespace: Vec<(Vec<u8>, Vec<u8>)>,
    cids: Vec<CidRange>,
    notdef: Vec<CidRange>,
    map: HashMap<Vec<u8>, Vec<u16>>,
    parent: Option<Arc<CMap>>,
    pub wmode: WritingMode,
    pub registry: Option<String>,
    pub ordering: Option<String>,
    range_work: u64,
    range_work_exhausted: bool,
}

impl CMap {
    fn empty() -> CMap {
        CMap {
            codespace: Vec::new(),
            cids: Vec::new(),
            notdef: Vec::new(),
            map: HashMap::new(),
            parent: None,
            wmode: WritingMode::Horizontal,
            registry: None,
            ordering: None,
            range_work: MAX_RANGE_WORK,
            range_work_exhausted: false,
        }
    }

    /// `Identity-H` or `Identity-V`: 2-byte codes, CID = code.
    pub fn identity(wmode: WritingMode) -> CMap {
        let mut m = CMap::empty();
        m.codespace.push((vec![0, 0], vec![0xFF, 0xFF]));
        m.cids.push(CidRange {
            len: 2,
            lo: 0,
            hi: 0xFFFF,
            cid: 0,
        });
        m.wmode = wmode;
        m.registry = Some("Adobe".into());
        m.ordering = Some("Identity".into());
        m
    }

    /// Parses CMap source. Malformed entries are skipped with `BadCMap`.
    pub fn parse(
        data: &[u8],
        origin: Origin,
        depth: usize,
        resolve: Resolve,
        diags: &mut Diagnostics,
    ) -> CMap {
        CMap::parse_based(data, origin, depth, None, resolve, diags)
    }

    /// [`CMap::parse`] with a base CMap from the stream dictionary's `/UseCMap`.
    pub fn parse_based(
        data: &[u8],
        origin: Origin,
        depth: usize,
        base: Option<UseCMap>,
        resolve: Resolve,
        diags: &mut Diagnostics,
    ) -> CMap {
        let mut t = CMap::empty();
        if depth > MAX_USECMAP_DEPTH {
            diags.push(DiagnosticKind::RecursionLimit, origin.at(0));
            return t;
        }
        if let Some(base) = base {
            t.parent = resolve(base, depth + 1, diags);
            if t.parent.is_none() {
                diags.push(DiagnosticKind::BadCMap, origin.at(0));
            }
        }
        let mut wmode = None;
        let mut codespace_full = false;
        for op in parse_content(data, origin, diags) {
            let at = op.location;
            let args = op.operands.as_slice();
            match op.operator.as_slice() {
                b"endcodespacerange" => {
                    for pair in args.chunks(2) {
                        match pair {
                            [Object::String(lo), Object::String(hi)]
                                if lo.len() == hi.len() && (1..=4).contains(&lo.len()) =>
                            {
                                if t.codespace.len() < MAX_CODESPACE {
                                    t.codespace.push((lo.clone(), hi.clone()));
                                } else if !codespace_full {
                                    codespace_full = true;
                                    diags.push(DiagnosticKind::BadCMap, at);
                                }
                            }
                            _ => diags.push(DiagnosticKind::BadCMap, at),
                        }
                    }
                }
                b"endcidrange" | b"endnotdefrange" => {
                    let notdef = op.operator.as_slice() == b"endnotdefrange";
                    for triple in args.chunks(3) {
                        match triple {
                            [Object::String(lo), Object::String(hi), cid] => {
                                t.cid_range(lo, hi, cid, notdef, diags, at);
                            }
                            _ => diags.push(DiagnosticKind::BadCMap, at),
                        }
                    }
                }
                b"endcidchar" => {
                    for pair in args.chunks(2) {
                        match pair {
                            [Object::String(c), cid] => t.cid_range(c, c, cid, false, diags, at),
                            _ => diags.push(DiagnosticKind::BadCMap, at),
                        }
                    }
                }
                b"endbfchar" => {
                    for pair in args.chunks(2) {
                        match pair {
                            [Object::String(code), Object::String(dst)] => {
                                t.insert(code.clone(), utf16_units(dst, diags, at), diags, at);
                            }
                            _ => diags.push(DiagnosticKind::BadCMap, at),
                        }
                    }
                }
                b"endbfrange" => {
                    for triple in args.chunks(3) {
                        t.bfrange(triple, diags, at);
                    }
                }
                b"def" => {
                    if let [Object::Name(k), v] = args {
                        match (k.as_bytes(), v) {
                            (b"WMode", Object::Int(n)) => {
                                wmode = Some(if *n == 1 {
                                    WritingMode::Vertical
                                } else {
                                    WritingMode::Horizontal
                                });
                            }
                            (b"Registry", Object::String(s)) => t.registry = Some(text(s)),
                            (b"Ordering", Object::String(s)) => t.ordering = Some(text(s)),
                            (b"CIDSystemInfo", Object::Dict(d)) => {
                                if let Some(Object::String(s)) = d.get(b"Registry") {
                                    t.registry = Some(text(s));
                                }
                                if let Some(Object::String(s)) = d.get(b"Ordering") {
                                    t.ordering = Some(text(s));
                                }
                            }
                            _ => {}
                        }
                    }
                }
                b"usecmap" => match args {
                    [Object::Name(n)] => {
                        t.parent = resolve(UseCMap::Name(n.as_bytes()), depth + 1, diags);
                        if t.parent.is_none() {
                            diags.push(DiagnosticKind::BadCMap, at);
                        }
                    }
                    _ => diags.push(DiagnosticKind::BadCMap, at),
                },
                _ => {}
            }
        }
        t.cids.sort_by_key(|r| (r.len, r.lo));
        t.notdef.sort_by_key(|r| (r.len, r.lo));
        if let Some(p) = t.parent.clone() {
            let room = MAX_CODESPACE.saturating_sub(t.codespace.len());
            t.codespace.extend(p.codespace.iter().take(room).cloned());
            t.wmode = p.wmode;
            t.registry = t.registry.or_else(|| p.registry.clone());
            t.ordering = t.ordering.or_else(|| p.ordering.clone());
        }
        if let Some(w) = wmode {
            t.wmode = w;
        }
        t
    }

    fn full(&self) -> bool {
        self.map.len() + self.cids.len() + self.notdef.len() >= MAX_ENTRIES
    }

    fn cid_range(
        &mut self,
        lo: &[u8],
        hi: &[u8],
        cid: &Object,
        notdef: bool,
        diags: &mut Diagnostics,
        at: Location,
    ) {
        let cid = cid.as_int().and_then(|c| u32::try_from(c).ok());
        let (Some(start), Some(end), Some(cid)) = (be(lo), be(hi), cid) else {
            diags.push(DiagnosticKind::BadCMap, at);
            return;
        };
        if lo.len() != hi.len() || end < start || self.full() {
            diags.push(DiagnosticKind::BadCMap, at);
            return;
        }
        let r = CidRange {
            len: lo.len() as u8,
            lo: start as u32,
            hi: end as u32,
            cid,
        };
        if notdef {
            self.notdef.push(r);
        } else {
            self.cids.push(r);
        }
    }

    fn insert(&mut self, code: Vec<u8>, units: Vec<u16>, diags: &mut Diagnostics, at: Location) {
        if self.full() && !self.map.contains_key(&code) {
            diags.push(DiagnosticKind::BadCMap, at);
            return;
        }
        self.map.insert(code, units);
    }

    fn bfrange(&mut self, triple: &[Object], diags: &mut Diagnostics, at: Location) {
        let [Object::String(lo), Object::String(hi), dst] = triple else {
            diags.push(DiagnosticKind::BadCMap, at);
            return;
        };
        let (Some(start), Some(end)) = (be(lo), be(hi)) else {
            diags.push(DiagnosticKind::BadCMap, at);
            return;
        };
        if lo.len() != hi.len() || end < start || end - start >= MAX_RANGE {
            diags.push(DiagnosticKind::BadCMap, at);
            return;
        }
        let count = end - start + 1;
        if self.range_work < count {
            if !self.range_work_exhausted {
                self.range_work_exhausted = true;
                diags.push(DiagnosticKind::BadCMap, at);
            }
            return;
        }
        self.range_work -= count;
        let code_at = |i: u64| -> Vec<u8> {
            let v = (start + i).to_be_bytes();
            v[v.len() - lo.len()..].to_vec()
        };
        match dst {
            Object::String(d) => {
                let base = utf16_units(d, diags, at);
                for i in 0..=end - start {
                    let mut units = base.clone();
                    // §9.10.3: the last unit of the destination is incremented.
                    let next = units.last().map(|&u| u64::from(u) + i);
                    match (units.last_mut(), next.and_then(|n| u16::try_from(n).ok())) {
                        (Some(last), Some(n)) => *last = n,
                        _ => {
                            diags.push(DiagnosticKind::BadCMap, at);
                            return;
                        }
                    }
                    self.insert(code_at(i), units, diags, at);
                }
            }
            Object::Array(items) => {
                for (i, item) in (0..=end - start).zip(items) {
                    match item {
                        Object::String(d) => {
                            let units = utf16_units(d, diags, at);
                            self.insert(code_at(i), units, diags, at);
                        }
                        _ => diags.push(DiagnosticKind::BadCMap, at),
                    }
                }
            }
            _ => diags.push(DiagnosticKind::BadCMap, at),
        }
    }

    /// The text for `code`; `None` when unmapped, empty, or only control characters,
    /// so lookup can fall through to the next source (spec §6).
    pub fn lookup(&self, code: &[u8]) -> Option<String> {
        let Some(units) = self.map.get(code) else {
            return self.parent.as_ref()?.lookup(code);
        };
        let s: String = char::decode_utf16(units.iter().copied())
            .map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
            .collect();
        (!s.chars().all(char::is_control)).then_some(s)
    }

    /// The next code in `s` (§9.7.6.2). A prefix of length n matches when a codespace
    /// range of length n contains it byte by byte. Without a match, the length is the
    /// shortest range whose first byte matches, else the shortest range.
    pub fn next_code(&self, s: &[u8]) -> Code {
        let Some(&first) = s.first() else {
            return Code {
                value: 0,
                len: 0,
                matched: false,
            };
        };
        if self.codespace.is_empty() {
            return Code {
                value: u32::from(first),
                len: 1,
                matched: true,
            };
        }
        let within = |b: &[u8], lo: &[u8], hi: &[u8]| {
            b.iter().zip(lo).zip(hi).all(|((b, l), h)| l <= b && b <= h)
        };
        for n in 1..=s.len().min(4) {
            let prefix = &s[..n];
            if self
                .codespace
                .iter()
                .any(|(lo, hi)| lo.len() == n && within(prefix, lo, hi))
            {
                return Code {
                    value: value(prefix),
                    len: n as u8,
                    matched: true,
                };
            }
        }
        let shortest = |ranges: &mut dyn Iterator<Item = usize>| ranges.min();
        let n = shortest(
            &mut self
                .codespace
                .iter()
                .filter(|(lo, hi)| within(&s[..1], lo, hi))
                .map(|(lo, _)| lo.len()),
        )
        .or_else(|| shortest(&mut self.codespace.iter().map(|(lo, _)| lo.len())))
        .unwrap_or(1)
        .clamp(1, s.len().min(4));
        Code {
            value: value(&s[..n]),
            len: n as u8,
            matched: false,
        }
    }

    /// The CID for `code` (§9.7.6.3): CID mappings, then notdef mappings, then 0.
    pub fn cid(&self, code: Code) -> u32 {
        self.mapped(code, false)
            .or_else(|| self.mapped(code, true))
            .unwrap_or(0)
    }

    fn mapped(&self, code: Code, notdef: bool) -> Option<u32> {
        let ranges = if notdef { &self.notdef } else { &self.cids };
        // ponytail: overlapping ranges resolve to the nearest start; pdfium takes the first
        // defined. Add an interval index if a real file needs it.
        let i = ranges.partition_point(|r| (r.len, r.lo) <= (code.len, code.value));
        let own = i
            .checked_sub(1)
            .and_then(|i| ranges.get(i))
            .filter(|r| r.len == code.len && code.value <= r.hi)
            .and_then(|r| {
                if notdef {
                    Some(r.cid)
                } else {
                    r.cid.checked_add(code.value - r.lo)
                }
            });
        own.or_else(|| self.parent.as_ref()?.mapped(code, notdef))
    }
}

/// A big-endian code of at most 4 bytes as an integer.
fn be(bytes: &[u8]) -> Option<u64> {
    if bytes.is_empty() || bytes.len() > 4 {
        return None;
    }
    Some(bytes.iter().fold(0, |acc, &b| acc << 8 | u64::from(b)))
}

fn value(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0, |acc, &b| acc << 8 | u32::from(b))
}

fn text(s: &[u8]) -> String {
    String::from_utf8_lossy(s).into_owned()
}

/// UTF-16BE units. A single byte is taken as one unit, as some writers emit `<41>`;
/// a trailing odd byte of a longer string is dropped.
fn utf16_units(bytes: &[u8], diags: &mut Diagnostics, at: Location) -> Vec<u16> {
    if let [b] = bytes {
        return vec![u16::from(*b)];
    }
    let (units, rest) = bytes.as_chunks::<2>();
    if !rest.is_empty() {
        diags.push(DiagnosticKind::BadCMap, at);
    }
    units.iter().map(|&c| u16::from_be_bytes(c)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pawdf_core::{DiagnosticKind, Diagnostics, Origin};

    fn parse_src(src: &[u8], d: &mut Diagnostics) -> CMap {
        CMap::parse(src, Origin::File, 0, &mut |_, _, _| None, d)
    }

    fn cmap(src: &str) -> CMap {
        parse_src(src.as_bytes(), &mut Diagnostics::new())
    }

    fn code(value: u32, len: u8) -> Code {
        Code {
            value,
            len,
            matched: true,
        }
    }

    #[test]
    fn codespace_splits_mixed_lengths() {
        let m = cmap(
            "4 begincodespacerange <00> <80> <8140> <9FFC> <A0> <DF> <E040> <FCFC> endcodespacerange",
        );
        assert_eq!(m.next_code(b"A\x82\xa0"), code(0x41, 1));
        assert_eq!(m.next_code(b"\x82\xa0"), code(0x82a0, 2));
        // §9.7.6.2 matches byte by byte: 0x30 is outside <40>..<FC> in the second byte.
        let bad = m.next_code(b"\x81\x30");
        assert_eq!((bad.len, bad.matched), (2, false));
        assert_eq!(
            m.next_code(b"\x82"),
            Code {
                value: 0x82,
                len: 1,
                matched: false
            }
        );
    }

    #[test]
    fn no_codespace_splits_single_bytes() {
        let m = cmap("");
        assert_eq!(m.next_code(b"\x01\x02"), code(1, 1));
    }

    #[test]
    fn cid_ranges_chars_and_notdef() {
        let m = cmap(
            "1 begincodespacerange <0000> <FFFF> endcodespacerange \
             1 begincidrange <3041> <3093> 842 endcidrange 1 begincidchar <3000> 633 endcidchar \
             1 beginnotdefrange <0000> <001f> 1 endnotdefrange",
        );
        let cids = [0x3042, 0x3000, 0x0005, 0x4e00].map(|v| m.cid(code(v, 2)));
        assert_eq!(cids, [843, 633, 1, 0]);
        // A code of another length never matches a 2-byte range.
        assert_eq!(m.cid(code(0x30, 1)), 0);
    }

    #[test]
    fn wmode_and_system_info() {
        let m = cmap(
            "/CIDSystemInfo 3 dict dup begin /Registry (Adobe) def /Ordering (Japan1) def \
             /Supplement 2 def end def /WMode 1 def",
        );
        assert_eq!(m.wmode, WritingMode::Vertical);
        assert_eq!(
            (m.registry.as_deref(), m.ordering.as_deref()),
            (Some("Adobe"), Some("Japan1"))
        );
        let m = cmap("/CIDSystemInfo << /Registry (Adobe) /Ordering (UCS) >> def");
        assert_eq!(m.ordering.as_deref(), Some("UCS"));
        assert_eq!(m.wmode, WritingMode::Horizontal);
    }

    #[test]
    fn usecmap_inherits_and_overrides() {
        let parent = Arc::new(cmap(
            "/WMode 1 def 1 begincodespacerange <0000> <FFFF> endcodespacerange \
             1 begincidrange <3041> <3093> 842 endcidrange \
             1 beginbfchar <3041> <0041> endbfchar",
        ));
        let mut d = Diagnostics::new();
        let mut seen = Vec::new();
        let m = CMap::parse(
            b"/P usecmap 1 begincidchar <3042> 9 endcidchar",
            Origin::File,
            0,
            &mut |u, depth, _| {
                seen.push(depth);
                matches!(u, UseCMap::Name(b"P")).then(|| Arc::clone(&parent))
            },
            &mut d,
        );
        assert_eq!(seen, [1]);
        assert!(d.as_slice().is_empty());
        assert_eq!(m.next_code(b"\x30\x41"), code(0x3041, 2));
        assert_eq!((m.cid(code(0x3041, 2)), m.cid(code(0x3042, 2))), (842, 9));
        assert_eq!(m.lookup(&[0x30, 0x41]).as_deref(), Some("A"));
        assert_eq!(m.wmode, WritingMode::Vertical);
    }

    #[test]
    fn usecmap_depth_is_capped() {
        fn chain(depth: usize, d: &mut Diagnostics) -> CMap {
            CMap::parse(
                b"/Self usecmap",
                Origin::File,
                depth,
                &mut |_, depth, d| Some(Arc::new(chain(depth, d))),
                d,
            )
        }
        let mut d = Diagnostics::new();
        chain(0, &mut d);
        let kinds: Vec<_> = d.as_slice().iter().map(|x| x.kind).collect();
        assert_eq!(kinds, [DiagnosticKind::RecursionLimit]);
    }

    #[test]
    fn unknown_usecmap_is_bad_cmap() {
        let (_, d) = parse("/Nope usecmap");
        assert_eq!(d, [DiagnosticKind::BadCMap]);
    }

    #[test]
    fn codespace_ranges_are_capped() {
        let src = format!(
            "100 begincodespacerange {} endcodespacerange 100 begincodespacerange {} endcodespacerange",
            "<FFFFFFFF> <FFFFFFFF> ".repeat(100),
            "<00> <FF> ".repeat(100)
        );
        let (m, d) = parse(&src);
        assert_eq!(m.codespace.len(), MAX_CODESPACE);
        assert_eq!(d, [DiagnosticKind::BadCMap]);
        let parent = Arc::new(m);
        let mut d = Diagnostics::new();
        let child = CMap::parse(
            b"1 begincodespacerange <00> <7F> endcodespacerange /P usecmap",
            Origin::File,
            0,
            &mut |_, _, _| Some(Arc::clone(&parent)),
            &mut d,
        );
        assert_eq!(child.codespace.len(), MAX_CODESPACE);
    }

    #[test]
    fn identity() {
        let m = CMap::identity(WritingMode::Vertical);
        let c = m.next_code(b"\x12\x34");
        assert_eq!(c, code(0x1234, 2));
        assert_eq!(m.cid(c), 0x1234);
        assert_eq!(m.wmode, WritingMode::Vertical);
    }

    const HDR: &str = "/CIDInit /ProcSet findresource begin 12 dict begin begincmap \
                       1 begincodespacerange <00> <FF> endcodespacerange ";

    fn parse(body: &str) -> (CMap, Vec<DiagnosticKind>) {
        let mut d = Diagnostics::new();
        let src =
            format!("{HDR}{body} endcmap CMapName currentdict /CMap defineresource pop end end");
        let t = parse_src(src.as_bytes(), &mut d);
        (t, d.as_slice().iter().map(|x| x.kind).collect())
    }

    #[test]
    fn bfchar() {
        let (t, d) = parse("2 beginbfchar <01> <0041> <02> <00660069> endbfchar");
        assert_eq!(t.lookup(&[1]).as_deref(), Some("A"));
        assert_eq!(t.lookup(&[2]).as_deref(), Some("fi"));
        assert_eq!(t.lookup(&[3]), None);
        assert!(d.is_empty());
    }

    #[test]
    fn bfrange_increments_last_unit() {
        let (t, d) = parse("2 beginbfrange <10> <12> <0061> <20> <21> <D83DDE00> endbfrange");
        assert_eq!(t.lookup(&[0x10]).as_deref(), Some("a"));
        assert_eq!(t.lookup(&[0x12]).as_deref(), Some("c"));
        assert_eq!(t.lookup(&[0x21]).as_deref(), Some("\u{1F601}"));
        assert_eq!(t.lookup(&[0x13]), None);
        assert!(d.is_empty());
    }

    #[test]
    fn bfrange_array_destination() {
        let (t, _) = parse("1 beginbfrange <30> <31> [<0058> <0059>] endbfrange");
        assert_eq!(t.lookup(&[0x30]).as_deref(), Some("X"));
        assert_eq!(t.lookup(&[0x31]).as_deref(), Some("Y"));
    }

    #[test]
    fn bfrange_overflow_is_diagnosed() {
        let (t, d) = parse("1 beginbfrange <01> <02> <FFFF> endbfrange");
        assert_eq!(t.lookup(&[1]).as_deref(), Some("\u{FFFF}"));
        assert_eq!(t.lookup(&[2]), None);
        assert_eq!(d, [DiagnosticKind::BadCMap]);
    }

    #[test]
    fn odd_trailing_byte_is_diagnosed() {
        let (t, d) = parse("1 beginbfchar <01> <004100> endbfchar");
        assert_eq!(t.lookup(&[1]).as_deref(), Some("A"));
        assert_eq!(d, [DiagnosticKind::BadCMap]);
    }

    #[test]
    fn control_only_counts_as_missing() {
        let (t, _) = parse("2 beginbfchar <05> <0000> <06> <0009> endbfchar");
        assert_eq!(t.lookup(&[5]), None);
        assert_eq!(t.lookup(&[6]), None);
    }

    #[test]
    fn unpaired_surrogate_becomes_replacement() {
        let (t, _) = parse("1 beginbfchar <07> <D800> endbfchar");
        assert_eq!(t.lookup(&[7]).as_deref(), Some("\u{FFFD}"));
    }

    #[test]
    fn two_byte_codes() {
        let mut d = Diagnostics::new();
        let src = b"1 begincodespacerange <0000> <FFFF> endcodespacerange \
                    1 beginbfchar <0102> <4E00> endbfchar";
        let t = parse_src(src, &mut d);
        assert_eq!(t.lookup(&[1, 2]).as_deref(), Some("\u{4E00}"));
        assert_eq!(t.lookup(&[2]), None);
    }

    #[test]
    fn garbage_is_tolerated() {
        let (t, d) =
            parse("1 beginbfchar <01> endbfchar 1 beginbfrange <05> <01> <0041> endbfrange");
        assert_eq!(t.lookup(&[1]), None);
        assert_eq!(d, [DiagnosticKind::BadCMap, DiagnosticKind::BadCMap]);
    }

    #[test]
    fn repeated_ranges_are_bounded() {
        let mut d = Diagnostics::new();
        let src = format!(
            "1 begincodespacerange <0000> <FFFF> endcodespacerange 100 beginbfrange {} endbfrange",
            "<0000> <7FFF> <0041> ".repeat(100)
        );
        let t = parse_src(src.as_bytes(), &mut d);
        let bad = d
            .as_slice()
            .iter()
            .filter(|x| x.kind == DiagnosticKind::BadCMap)
            .count();
        assert_eq!(bad, 1);
        assert_eq!(t.lookup(&[0, 0]).as_deref(), Some("A"));
    }

    #[test]
    fn huge_range_is_skipped() {
        let mut d = Diagnostics::new();
        let src = b"1 beginbfrange <000000> <FFFFFF> <0041> endbfrange";
        let t = parse_src(src, &mut d);
        assert_eq!(t.lookup(&[0, 0, 0]), None);
        assert_eq!(d.as_slice()[0].kind, DiagnosticKind::BadCMap);
    }
}
