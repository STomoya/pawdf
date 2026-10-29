//! Content-stream operations (ISO 32000-2 §7.8.2).

use crate::diag::{DiagnosticKind, Diagnostics, Location, Origin};
use crate::lexer::{Lexer, Token, is_whitespace};
use crate::object::{Dict, Name, Object, Stream};
use crate::parser::parse_object_from;

/// Operands beyond this many are dropped, so a run of bare numbers cannot grow one
/// operation without bound.
pub const MAX_OPERANDS: usize = 4096;

/// One content-stream operation.
///
/// An inline image (§8.9.7) is operator `BI` with a single [`Object::Stream`] operand
/// holding the image dictionary (abbreviated keys kept as-is) and the bytes between
/// `ID` and `EI`.
#[derive(Debug, Clone, PartialEq)]
pub struct Operation {
    /// The operator, such as `Tj`. Empty for operands left at the end of the data.
    pub operator: Vec<u8>,
    /// The operands, in order.
    pub operands: Vec<Object>,
    /// Where the operation starts.
    pub location: Location,
}

/// Parses content-stream data into operations.
///
/// Operands left at the end of `data` come back as a final operation with an empty
/// `operator`; a caller joining several streams prepends them to the next stream's
/// first operation (§7.8.2 allows a split at any token boundary).
pub fn parse_content(data: &[u8], origin: Origin, diags: &mut Diagnostics) -> Vec<Operation> {
    let mut lex = Lexer::new(data);
    let mut out = Vec::new();
    let mut operands = Vec::new();
    let mut first: Option<usize> = None;
    let mut capped = false;
    while let Some((start, tok)) = lex.next_token() {
        let begin = *first.get_or_insert(start);
        match tok {
            Token::Keyword(k) if matches!(k.as_slice(), b")" | b">" | b"{" | b"}") => {
                diags.push(DiagnosticKind::BadObject, origin.at(start));
            }
            Token::ArrayClose | Token::DictClose => {
                diags.push(DiagnosticKind::BadObject, origin.at(start));
            }
            Token::Keyword(k) if !matches!(k.as_slice(), b"true" | b"false" | b"null") => {
                if k == b"BI" {
                    operands.push(inline_image(&mut lex, start, origin, diags));
                }
                out.push(Operation {
                    operator: k,
                    operands: std::mem::take(&mut operands),
                    location: origin.at(begin),
                });
                first = None;
                capped = false;
            }
            tok => {
                let obj = parse_object_from(&mut lex, start, tok, origin, diags);
                if operands.len() < MAX_OPERANDS {
                    operands.push(obj);
                } else if !capped {
                    capped = true;
                    diags.push(DiagnosticKind::BadObject, origin.at(start));
                }
            }
        }
    }
    if !operands.is_empty() {
        out.push(Operation {
            operator: Vec::new(),
            operands,
            location: origin.at(first.unwrap_or(0)),
        });
    }
    out
}

/// Reads an inline image after its `BI` keyword, leaving the lexer after `EI`.
fn inline_image(lex: &mut Lexer, bi: usize, origin: Origin, diags: &mut Diagnostics) -> Object {
    let mut dict = Dict::new();
    loop {
        let Some((start, tok)) = lex.next_token() else {
            diags.push(DiagnosticKind::BadImage, origin.at(bi));
            return Object::Stream(Stream::new(dict, Vec::new()));
        };
        match tok {
            Token::Keyword(k) if k == b"ID" => break,
            Token::Name(key) => {
                // `/BPC ID`: the value is missing, so `ID` still starts the data.
                if let Some((_, Token::Keyword(k))) = lex.peek_token()
                    && k == b"ID"
                {
                    diags.push(DiagnosticKind::BadObject, origin.at(start));
                    continue;
                }
                let Some((vstart, vtok)) = lex.next_token() else {
                    continue;
                };
                let value = parse_object_from(lex, vstart, vtok, origin, diags);
                if dict.contains_key(&key) {
                    diags.push(DiagnosticKind::BadObject, origin.at(start));
                }
                dict.insert(Name(key), value);
            }
            tok => {
                diags.push(DiagnosticKind::BadObject, origin.at(start));
                parse_object_from(lex, start, tok, origin, diags);
            }
        }
    }
    let data = lex.data();
    let mut begin = lex.pos();
    if data.get(begin).is_some_and(|&b| is_whitespace(b)) {
        begin += 1;
    }
    match find_ei(data, begin) {
        Some((end, after)) => {
            lex.set_pos(after);
            Object::Stream(Stream::new(dict, data[begin..end].to_vec()))
        }
        None => {
            diags.push(DiagnosticKind::BadImage, origin.at(bi));
            lex.set_pos(data.len());
            Object::Stream(Stream::new(dict, data[begin..].to_vec()))
        }
    }
}

/// Finds the `EI` that ends inline image data starting at `begin`, as pdf.js does: `EI`
/// between whitespace (or EOF), followed by text rather than binary bytes, and then by
/// a plausible token. Returns the end of the data and the offset after `EI`.
fn find_ei(data: &[u8], begin: usize) -> Option<(usize, usize)> {
    let mut i = begin;
    while i + 2 <= data.len() {
        let at_ei = data[i..].starts_with(b"EI")
            && i > 0
            && is_whitespace(data[i - 1])
            && data.get(i + 2).is_none_or(|&b| is_whitespace(b));
        if at_ei && plausible_after(data, i + 2) {
            return Some(((i - 1).max(begin), i + 2));
        }
        i += 1;
    }
    None
}

fn plausible_after(data: &[u8], pos: usize) -> bool {
    let window = &data[pos..data.len().min(pos + 10)];
    if window
        .iter()
        .any(|&b| !(is_whitespace(b) || (0x20..0x7f).contains(&b)))
    {
        return false;
    }
    // Bounded so a crafted run of `EI (` candidates stays linear.
    match Lexer::at(&data[..data.len().min(pos + 64)], pos).next_token() {
        Some((_, Token::Keyword(k))) => {
            k.len() <= 3
                && k.iter()
                    .all(|&b| b.is_ascii_alphabetic() || matches!(b, b'\'' | b'"' | b'*'))
        }
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diag::{DiagnosticKind, Diagnostics, Location, Origin};
    use crate::object::Object;

    fn ops(input: &[u8]) -> (Vec<Operation>, Diagnostics) {
        let mut d = Diagnostics::new();
        let v = parse_content(input, Origin::File, &mut d);
        (v, d)
    }

    fn names(v: &[Operation]) -> Vec<&[u8]> {
        v.iter().map(|o| o.operator.as_slice()).collect()
    }

    fn kinds(d: &Diagnostics) -> Vec<DiagnosticKind> {
        d.as_slice().iter().map(|x| x.kind).collect()
    }

    #[test]
    fn operators_collect_operands() {
        let (v, d) = ops(b"q 1 0 0 1 10 20 cm /F1 12 Tf [(a) -250 (b)] TJ Q");
        assert_eq!(names(&v), [b"q".as_slice(), b"cm", b"Tf", b"TJ", b"Q"]);
        assert_eq!(v[1].operands.len(), 6);
        assert_eq!(v[2].operands, [Object::Name("F1".into()), Object::Int(12)]);
        assert!(matches!(&v[3].operands[0], Object::Array(a) if a.len() == 3));
        assert_eq!(v[1].location, Location::File(2));
        assert!(d.is_empty());
    }

    #[test]
    fn true_false_null_are_operands() {
        let (v, _) = ops(b"true false null d0");
        assert_eq!(names(&v), [b"d0".as_slice()]);
        assert_eq!(
            v[0].operands,
            [Object::Bool(true), Object::Bool(false), Object::Null]
        );
    }

    #[test]
    fn trailing_operands_become_empty_operator() {
        let (v, _) = ops(b"BT 10 20");
        assert_eq!(names(&v), [b"BT".as_slice(), b""]);
        assert_eq!(v[1].operands, [Object::Int(10), Object::Int(20)]);
    }

    #[test]
    fn inline_image() {
        let (v, d) = ops(b"BI /W 2 /H 1 /CS /G /BPC 8 ID \x00\xff EI Q");
        assert_eq!(names(&v), [b"BI".as_slice(), b"Q"]);
        let Object::Stream(s) = &v[0].operands[0] else {
            panic!("not a stream")
        };
        assert_eq!(s.dict.get(b"W"), Some(&Object::Int(2)));
        assert_eq!(s.raw(), b"\x00\xff");
        assert!(d.is_empty());
    }

    #[test]
    fn inline_image_data_containing_ei() {
        let (v, _) = ops(b"BI /W 4 /H 1 /BPC 8 /CS /G ID \x01 EI \x02 EI Q");
        let Object::Stream(s) = &v[0].operands[0] else {
            panic!("not a stream")
        };
        assert_eq!(s.raw(), b"\x01 EI \x02");
        assert_eq!(names(&v)[1], b"Q");
    }

    #[test]
    fn inline_image_without_ei_runs_to_end() {
        let (v, d) = ops(b"q BI /W 1 /H 1 ID \x01\x02");
        assert_eq!(names(&v), [b"q".as_slice(), b"BI"]);
        let Object::Stream(s) = &v[1].operands[0] else {
            panic!("not a stream")
        };
        assert_eq!(s.raw(), b"\x01\x02");
        assert_eq!(kinds(&d), [DiagnosticKind::BadImage]);
        assert_eq!(d.as_slice()[0].location, Location::File(2));
    }

    #[test]
    fn inline_image_key_before_id_has_no_value() {
        let (v, d) = ops(b"BI /W 1 /H 1 /BPC ID \x00 EI 0 0 m 1 1 l S");
        assert_eq!(names(&v), [b"BI".as_slice(), b"m", b"l", b"S"]);
        let Object::Stream(s) = &v[0].operands[0] else {
            panic!("not a stream")
        };
        assert_eq!(s.dict.get(b"BPC"), None);
        assert_eq!(s.raw(), b"\x00");
        assert_eq!(kinds(&d), [DiagnosticKind::BadObject]);
        assert_eq!(d.as_slice()[0].location, Location::File(13));
    }

    #[test]
    fn inline_image_duplicate_key_is_reported() {
        let (v, d) = ops(b"BI /W 1 /W 2 /H 1 ID \x00 EI");
        let Object::Stream(s) = &v[0].operands[0] else {
            panic!("not a stream")
        };
        assert_eq!(s.dict.get(b"W"), Some(&Object::Int(2)));
        assert_eq!(kinds(&d), [DiagnosticKind::BadObject]);
        assert_eq!(d.as_slice()[0].location, Location::File(8));
    }

    #[test]
    fn stray_delimiters_do_not_stall() {
        let (v, d) = ops(b") } ] >> q");
        assert_eq!(names(&v), [b"q".as_slice()]);
        assert_eq!(kinds(&d), [DiagnosticKind::BadObject; 4]);
    }

    #[test]
    fn operand_count_is_capped() {
        let input = "1 ".repeat(MAX_OPERANDS + 10) + "n";
        let (v, d) = ops(input.as_bytes());
        assert_eq!(v[0].operands.len(), MAX_OPERANDS);
        assert_eq!(kinds(&d), [DiagnosticKind::BadObject]);
    }

    #[test]
    fn diagnostic_kind_names() {
        assert_eq!(DiagnosticKind::XrefRepaired.name(), "xref_repaired");
        assert_eq!(DiagnosticKind::UnmappedGlyph.name(), "unmapped_glyph");
        assert_eq!(DiagnosticKind::BadCMap.name(), "bad_cmap");
    }
}
