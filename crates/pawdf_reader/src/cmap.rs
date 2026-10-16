//! ToUnicode CMaps (§9.10.3): code space ranges, `bfchar`, and `bfrange`.

use std::collections::HashMap;

use pawdf_core::{DiagnosticKind, Diagnostics, Location, Object, Origin, parse_content};

/// Larger `bfrange`s are rejected; real ToUnicode ranges stay within one byte (§9.10.3).
const MAX_RANGE: u64 = 1 << 16;
/// Total mappings kept, so crafted input cannot exhaust memory.
const MAX_ENTRIES: usize = 1 << 20;

/// A parsed ToUnicode CMap: character code bytes to UTF-16 units.
#[derive(Debug, Clone, Default)]
pub(crate) struct ToUnicode {
    map: HashMap<Vec<u8>, Vec<u16>>,
}

impl ToUnicode {
    /// Parses CMap source. Malformed entries are skipped with `BadCMap`.
    pub fn parse(data: &[u8], origin: Origin, diags: &mut Diagnostics) -> ToUnicode {
        let mut t = ToUnicode::default();
        for op in parse_content(data, origin, diags) {
            let at = op.location;
            match op.operator.as_slice() {
                b"endbfchar" => {
                    for pair in op.operands.chunks(2) {
                        match pair {
                            [Object::String(code), Object::String(dst)] => {
                                t.insert(code.clone(), utf16_units(dst), diags, at);
                            }
                            _ => diags.push(DiagnosticKind::BadCMap, at),
                        }
                    }
                }
                b"endbfrange" => {
                    for triple in op.operands.chunks(3) {
                        t.bfrange(triple, diags, at);
                    }
                }
                _ => {}
            }
        }
        t
    }

    fn insert(&mut self, code: Vec<u8>, units: Vec<u16>, diags: &mut Diagnostics, at: Location) {
        if self.map.len() >= MAX_ENTRIES && !self.map.contains_key(&code) {
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
        let code_at = |i: u64| -> Vec<u8> {
            let v = (start + i).to_be_bytes();
            v[v.len() - lo.len()..].to_vec()
        };
        match dst {
            Object::String(d) => {
                let base = utf16_units(d);
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
                        Object::String(d) => self.insert(code_at(i), utf16_units(d), diags, at),
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
        let units = self.map.get(code)?;
        let s: String = char::decode_utf16(units.iter().copied())
            .map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER))
            .collect();
        (!s.chars().all(char::is_control)).then_some(s)
    }
}

/// A big-endian code of at most 4 bytes as an integer.
fn be(bytes: &[u8]) -> Option<u64> {
    if bytes.is_empty() || bytes.len() > 4 {
        return None;
    }
    Some(bytes.iter().fold(0, |acc, &b| acc << 8 | u64::from(b)))
}

/// UTF-16BE units. A single byte is taken as one unit, as some writers emit `<41>`.
fn utf16_units(bytes: &[u8]) -> Vec<u16> {
    match bytes {
        [b] => vec![u16::from(*b)],
        _ => bytes
            .chunks_exact(2)
            .map(|c| u16::from_be_bytes([c[0], c[1]]))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pawdf_core::{DiagnosticKind, Diagnostics, Origin};

    const HDR: &str = "/CIDInit /ProcSet findresource begin 12 dict begin begincmap \
                       1 begincodespacerange <00> <FF> endcodespacerange ";

    fn parse(body: &str) -> (ToUnicode, Vec<DiagnosticKind>) {
        let mut d = Diagnostics::new();
        let src =
            format!("{HDR}{body} endcmap CMapName currentdict /CMap defineresource pop end end");
        let t = ToUnicode::parse(src.as_bytes(), Origin::File, &mut d);
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
        let t = ToUnicode::parse(src, Origin::File, &mut d);
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
    fn huge_range_is_skipped() {
        let mut d = Diagnostics::new();
        let src = b"1 beginbfrange <000000> <FFFFFF> <0041> endbfrange";
        let t = ToUnicode::parse(src, Origin::File, &mut d);
        assert_eq!(t.lookup(&[0, 0, 0]), None);
        assert_eq!(d.as_slice()[0].kind, DiagnosticKind::BadCMap);
    }
}
