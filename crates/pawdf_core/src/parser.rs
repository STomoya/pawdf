use std::collections::HashSet;

use crate::diag::{DiagnosticKind, Diagnostics, Origin};
use crate::lexer::{Lexer, Token, is_whitespace};
use crate::object::{Dict, Name, Object, Ref, Stream};

/// Maximum array/dictionary nesting. Deeper input is cut off to keep recursion bounded.
pub const MAX_DEPTH: usize = 256;

/// Parses one object. Returns `None` only at end of input.
pub(crate) fn parse_object(
    lex: &mut Lexer,
    origin: Origin,
    diags: &mut Diagnostics,
) -> Option<Object> {
    let (start, tok) = lex.next_token()?;
    Some(parse_object_from(lex, start, tok, origin, diags))
}

/// Parses an object whose first token has already been read.
pub(crate) fn parse_object_from(
    lex: &mut Lexer,
    start: usize,
    tok: Token,
    origin: Origin,
    diags: &mut Diagnostics,
) -> Object {
    parse_with_depth(lex, start, tok, origin, diags, 0)
}

fn parse_with_depth(
    lex: &mut Lexer,
    start: usize,
    tok: Token,
    origin: Origin,
    diags: &mut Diagnostics,
    depth: usize,
) -> Object {
    match tok {
        Token::Int(n) => match try_ref(lex, n) {
            Some(r) => Object::Ref(r),
            None => Object::Int(n),
        },
        Token::Real(r) => Object::Real(r),
        Token::String(s) => Object::String(s),
        Token::Name(n) => Object::Name(Name(n)),
        Token::ArrayOpen | Token::DictOpen if depth >= MAX_DEPTH => {
            diags.push(DiagnosticKind::RecursionLimit, origin.at(start));
            Object::Null
        }
        Token::ArrayOpen => parse_array(lex, origin, diags, depth + 1),
        Token::DictOpen => Object::Dict(parse_dict(lex, origin, diags, depth + 1)),
        Token::Keyword(k) => match k.as_slice() {
            b"true" => Object::Bool(true),
            b"false" => Object::Bool(false),
            b"null" => Object::Null,
            _ => {
                diags.push(DiagnosticKind::BadObject, origin.at(start));
                Object::Null
            }
        },
        Token::ArrayClose | Token::DictClose => {
            diags.push(DiagnosticKind::BadObject, origin.at(start));
            Object::Null
        }
    }
}

/// After an integer `num`, checks for `gen R` and consumes it if present.
fn try_ref(lex: &mut Lexer, num: i64) -> Option<Ref> {
    let save = lex.pos();
    let r = (|| {
        let num = u32::try_from(num).ok()?;
        let (_, Token::Int(generation)) = lex.next_token()? else {
            return None;
        };
        let generation = u16::try_from(generation).ok()?;
        let (_, Token::Keyword(k)) = lex.next_token()? else {
            return None;
        };
        (k == b"R").then_some(Ref { num, generation })
    })();
    if r.is_none() {
        lex.set_pos(save);
    }
    r
}

fn parse_array(lex: &mut Lexer, origin: Origin, diags: &mut Diagnostics, depth: usize) -> Object {
    let mut items = Vec::new();
    loop {
        let Some((start, tok)) = lex.next_token() else {
            diags.push(DiagnosticKind::BadObject, origin.at(lex.pos()));
            break;
        };
        match tok {
            Token::ArrayClose => break,
            Token::DictClose => {
                diags.push(DiagnosticKind::BadObject, origin.at(start));
                break;
            }
            // A stray keyword such as `endobj` means the closing `]` is missing.
            Token::Keyword(k) if k == b"endobj" || k == b"stream" => {
                diags.push(DiagnosticKind::BadObject, origin.at(start));
                lex.set_pos(start);
                break;
            }
            tok => items.push(parse_with_depth(lex, start, tok, origin, diags, depth)),
        }
    }
    Object::Array(items)
}

fn parse_dict(lex: &mut Lexer, origin: Origin, diags: &mut Diagnostics, depth: usize) -> Dict {
    let mut entries = Vec::new();
    let mut seen = HashSet::new();
    loop {
        let Some((start, tok)) = lex.next_token() else {
            diags.push(DiagnosticKind::BadObject, origin.at(lex.pos()));
            break;
        };
        match tok {
            Token::DictClose => break,
            Token::Name(key) => {
                // `/Key >>` has no value; treat it as null and end the dictionary.
                if let Some((_, Token::DictClose)) = lex.peek_token() {
                    diags.push(DiagnosticKind::BadObject, origin.at(start));
                    entries.push((Name(key), Object::Null));
                    continue;
                }
                // `/Key endobj`: the value and the closing `>>` are both missing.
                if let Some((_, Token::Keyword(k))) = lex.peek_token()
                    && (k == b"endobj" || k == b"stream")
                {
                    diags.push(DiagnosticKind::BadObject, origin.at(start));
                    entries.push((Name(key), Object::Null));
                    break;
                }
                let Some((vstart, vtok)) = lex.next_token() else {
                    diags.push(DiagnosticKind::BadObject, origin.at(lex.pos()));
                    break;
                };
                let value = parse_with_depth(lex, vstart, vtok, origin, diags, depth);
                if !seen.insert(key.clone()) {
                    diags.push(DiagnosticKind::BadObject, origin.at(start));
                }
                entries.push((Name(key), value));
            }
            // A stray keyword such as `endobj` means the closing `>>` is missing.
            Token::Keyword(k) if k == b"endobj" || k == b"stream" => {
                diags.push(DiagnosticKind::BadObject, origin.at(start));
                lex.set_pos(start);
                break;
            }
            tok => {
                diags.push(DiagnosticKind::BadObject, origin.at(start));
                parse_with_depth(lex, start, tok, origin, diags, depth);
            }
        }
    }
    Dict::from_entries(entries)
}

/// Parses an indirect object `num gen obj ... endobj` starting at `offset` in the file.
/// Returns `None` if there is no `num gen obj` header there.
///
/// `length_of` resolves an indirect stream `/Length`.
pub fn parse_indirect(
    data: &[u8],
    offset: usize,
    length_of: &dyn Fn(Ref) -> Option<i64>,
    diags: &mut Diagnostics,
) -> Option<(Ref, Object)> {
    parse_indirect_end(data, offset, length_of, diags).map(|(r, obj, _)| (r, obj))
}

/// [`parse_indirect`], plus the offset just past what was consumed.
pub(crate) fn parse_indirect_end(
    data: &[u8],
    offset: usize,
    length_of: &dyn Fn(Ref) -> Option<i64>,
    diags: &mut Diagnostics,
) -> Option<(Ref, Object, usize)> {
    let mut lex = Lexer::at(data, offset);
    let r = parse_header(&mut lex)?;
    let Some(obj) = parse_object(&mut lex, Origin::File, diags) else {
        diags.push(DiagnosticKind::BadObject, Origin::File.at(lex.pos()));
        return Some((r, Object::Null, lex.pos()));
    };
    match (obj, lex.peek_token()) {
        (Object::Dict(dict), Some((_, Token::Keyword(k)))) if k == b"stream" => {
            lex.next_token();
            let (raw, end) = stream_data(data, lex.pos(), &dict, length_of, diags);
            Some((r, Object::Stream(Stream::new(dict, raw)), end))
        }
        (obj, _) => Some((r, obj, lex.pos())),
    }
}

/// Reads `num gen obj`.
pub(crate) fn parse_header(lex: &mut Lexer) -> Option<Ref> {
    let (_, Token::Int(num)) = lex.next_token()? else {
        return None;
    };
    let (_, Token::Int(generation)) = lex.next_token()? else {
        return None;
    };
    let (_, Token::Keyword(k)) = lex.next_token()? else {
        return None;
    };
    if k != b"obj" {
        return None;
    }
    Some(Ref {
        num: u32::try_from(num).ok()?,
        generation: u16::try_from(generation).ok()?,
    })
}

/// §7.3.8.1. `pos` is just after the `stream` keyword. Returns the data and the offset
/// just past it (past `endstream` when that was found).
fn stream_data(
    data: &[u8],
    pos: usize,
    dict: &Dict,
    length_of: &dyn Fn(Ref) -> Option<i64>,
    diags: &mut Diagnostics,
) -> (Vec<u8>, usize) {
    let mut start = pos;
    while data.get(start) == Some(&b' ') {
        start += 1;
    }
    if data.get(start) == Some(&b'\r') {
        start += 1;
    }
    if data.get(start) == Some(&b'\n') {
        start += 1;
    }
    let length = match dict.get(b"Length") {
        Some(Object::Ref(r)) => length_of(*r),
        Some(o) => o.as_int(),
        None => None,
    };
    if let Some(end) = length
        .and_then(|l| usize::try_from(l).ok())
        .and_then(|l| start.checked_add(l))
        .filter(|&end| end <= data.len())
    {
        let mut after = end;
        while data.get(after).is_some_and(|&b| is_whitespace(b)) {
            after += 1;
        }
        if data[after..].starts_with(b"endstream") {
            return (data[start..end].to_vec(), after + 9);
        }
    }
    diags.push(DiagnosticKind::BadStreamLength, Origin::File.at(start));
    let rest = &data[start..];
    // Prefer `endstream`, since the data itself may contain `endobj`. A `stream` keyword
    // after an `endobj` belongs to a later object, so the search ends there: scanning on
    // to EOF would make repeated streams without `endstream` quadratic.
    let mut endobj = None;
    let mut found = None;
    for i in 0..rest.len() {
        let r = &rest[i..];
        if r.starts_with(b"endstream") {
            found = Some((i, start + i + 9));
            break;
        }
        if endobj.is_none() {
            if r.starts_with(b"endobj") {
                endobj = Some(i);
            }
        } else if r.starts_with(b"stream") {
            break;
        }
    }
    let (mut end, consumed) = found
        .or(endobj.map(|i| (i, start + i)))
        .unwrap_or((rest.len(), data.len()));
    if rest[..end].ends_with(b"\r\n") {
        end -= 2;
    } else if rest[..end].ends_with(b"\n") || rest[..end].ends_with(b"\r") {
        end -= 1;
    }
    (rest[..end].to_vec(), consumed)
}

pub(crate) fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(input: &[u8]) -> (Object, Diagnostics) {
        let mut d = Diagnostics::new();
        let o = parse_object(&mut Lexer::new(input), Origin::File, &mut d).unwrap_or(Object::Null);
        (o, d)
    }

    fn no_len(_: Ref) -> Option<i64> {
        None
    }

    #[test]
    fn scalars_and_refs() {
        assert_eq!(parse(b"true").0, Object::Bool(true));
        assert_eq!(parse(b"null").0, Object::Null);
        assert_eq!(
            parse(b"12 0 R").0,
            Object::Ref(Ref {
                num: 12,
                generation: 0
            })
        );
        assert_eq!(parse(b"12 0 obj").0, Object::Int(12));
        assert_eq!(
            parse(b"[1 2 R 3]").0,
            Object::Array(vec![
                Object::Ref(Ref {
                    num: 1,
                    generation: 2
                }),
                Object::Int(3)
            ])
        );
    }

    #[test]
    fn nested_dict() {
        let (o, d) = parse(b"<< /A [1 (x)] /B << /C /D >> >>");
        let dict = o.as_dict().unwrap();
        assert_eq!(dict.get(b"A").unwrap().as_array().unwrap().len(), 2);
        assert_eq!(
            dict.get(b"B")
                .unwrap()
                .as_dict()
                .unwrap()
                .get(b"C")
                .unwrap()
                .as_name(),
            Some(&b"D"[..])
        );
        assert!(d.is_empty());
    }

    #[test]
    fn missing_value_and_bad_key() {
        let (o, d) = parse(b"<< 5 /A >>");
        assert_eq!(o.as_dict().unwrap().get(b"A"), Some(&Object::Null));
        assert_eq!(d.as_slice().len(), 2);
    }

    #[test]
    fn duplicate_key_is_reported_and_last_wins() {
        let (o, d) = parse(b"<< /A 1 /B 2 /A 3 >>");
        let dict = o.as_dict().unwrap();
        assert_eq!(dict.get(b"A"), Some(&Object::Int(3)));
        assert_eq!(dict.len(), 2);
        assert_eq!(d.as_slice().len(), 1);
        assert_eq!(d.as_slice()[0].kind, DiagnosticKind::BadObject);
        assert_eq!(d.as_slice()[0].location, crate::diag::Location::File(13));
    }

    #[test]
    fn unterminated_containers_close_at_eof() {
        let (o, d) = parse(b"[1 [2");
        assert_eq!(
            o,
            Object::Array(vec![Object::Int(1), Object::Array(vec![Object::Int(2)])])
        );
        assert!(!d.is_empty());
    }

    #[test]
    fn duplicate_keys_keep_last_value() {
        let (o, _) = parse(b"<< /A 1 /B 2 /A 3 >>");
        let d = o.as_dict().unwrap();
        assert_eq!(d.len(), 2);
        assert_eq!(d.get(b"A"), Some(&Object::Int(3)));
    }

    /// Review I5: dictionary parsing must not be quadratic in the number of keys.
    #[test]
    fn large_dict_parses_in_linear_time() {
        let mut input = b"<<".to_vec();
        for i in 0..50_000 {
            input.extend_from_slice(format!(" /k{i} 1").as_bytes());
        }
        input.extend_from_slice(b" >>");
        let start = std::time::Instant::now();
        let (o, _) = parse(&input);
        let took = start.elapsed();
        assert_eq!(o.as_dict().unwrap().len(), 50_000);
        assert!(took < std::time::Duration::from_secs(1), "took {took:?}");
    }

    #[test]
    fn deep_nesting_is_cut_off() {
        let input = vec![b'['; 100_000];
        let (_, d) = parse(&input);
        assert!(
            d.as_slice()
                .iter()
                .any(|x| x.kind == DiagnosticKind::RecursionLimit)
        );
    }

    #[test]
    fn indirect_stream_with_correct_length() {
        let data = b"7 0 obj << /Length 5 >> stream\r\nhello\nendstream endobj";
        let mut d = Diagnostics::new();
        let (r, o) = parse_indirect(data, 0, &no_len, &mut d).unwrap();
        assert_eq!(
            r,
            Ref {
                num: 7,
                generation: 0
            }
        );
        assert_eq!(o.as_stream().unwrap().raw(), b"hello");
        assert!(d.is_empty());
    }

    #[test]
    fn indirect_stream_with_wrong_length_uses_endstream() {
        let data = b"7 0 obj << /Length 99 >> stream\nhello\nendstream endobj";
        let mut d = Diagnostics::new();
        let (_, o) = parse_indirect(data, 0, &no_len, &mut d).unwrap();
        assert_eq!(o.as_stream().unwrap().raw(), b"hello");
        assert_eq!(d.as_slice()[0].kind, DiagnosticKind::BadStreamLength);
    }

    #[test]
    fn wrong_length_prefers_endstream_over_endobj_in_data() {
        let data = b"7 0 obj << /Length 99 >> stream\nhello endobj world\nendstream endobj";
        let mut d = Diagnostics::new();
        let (_, o) = parse_indirect(data, 0, &no_len, &mut d).unwrap();
        assert_eq!(o.as_stream().unwrap().raw(), b"hello endobj world");
    }

    #[test]
    fn wrong_length_without_endstream_stops_at_endobj() {
        let data = b"7 0 obj << /Length 99 >> stream\nhello\nendobj 8 0 obj << /Length 1 >> stream\nx\nendstream endobj";
        let mut d = Diagnostics::new();
        let (_, o) = parse_indirect(data, 0, &no_len, &mut d).unwrap();
        assert_eq!(o.as_stream().unwrap().raw(), b"hello");
    }

    #[test]
    fn indirect_length_is_resolved() {
        let data = b"7 0 obj << /Length 8 0 R >> stream\nhello\nendstream endobj";
        let mut d = Diagnostics::new();
        let len = |r: Ref| (r.num == 8).then_some(5);
        let (_, o) = parse_indirect(data, 0, &len, &mut d).unwrap();
        assert_eq!(o.as_stream().unwrap().raw(), b"hello");
        assert!(d.is_empty());
    }

    #[test]
    fn zero_length_stream() {
        let data = b"7 0 obj << /Length 0 >> stream\n\nendstream endobj";
        let mut d = Diagnostics::new();
        let (_, o) = parse_indirect(data, 0, &no_len, &mut d).unwrap();
        assert_eq!(o.as_stream().unwrap().raw(), b"");
        assert!(d.is_empty());
    }

    #[test]
    fn missing_header_is_none() {
        let mut d = Diagnostics::new();
        assert!(parse_indirect(b"xref 0 1", 0, &no_len, &mut d).is_none());
    }
}
