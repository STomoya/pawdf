//! Stream filters (ISO 32000-2 §7.4).

mod ascii;
mod flate;
mod lzw;
mod predictor;
mod runlength;

use crate::diag::{DiagnosticKind, Diagnostics, Location};
use crate::object::{Dict, Object, Stream};

/// Decoded output is truncated beyond this many bytes, guarding against decompression bombs.
pub const MAX_DECODED_LEN: usize = 256 << 20;

/// Image codecs that are passed through undecoded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Codec {
    /// `DCTDecode` (JPEG).
    Dct,
    /// `JPXDecode` (JPEG 2000).
    Jpx,
    /// `JBIG2Decode`.
    Jbig2,
    /// `CCITTFaxDecode`.
    Ccitt,
}

/// The result of running a stream's filter chain.
#[derive(Debug, Clone, PartialEq)]
pub struct Decoded {
    /// Fully decoded bytes, or, when `codec` is set, the bytes still encoded with that codec.
    pub data: Vec<u8>,
    /// The codec filter decoding stopped at, with its `/DecodeParms` (empty if none).
    pub codec: Option<(Codec, Dict)>,
}

/// Why a filter could not produce complete output. Both variants carry the partial output.
pub(crate) enum Failure {
    Corrupt(Vec<u8>),
    TooLarge(Vec<u8>),
}

pub(crate) type FilterResult = Result<Vec<u8>, Failure>;

enum Filter {
    Data(fn(&[u8], &Dict) -> FilterResult),
    Codec(Codec),
}

fn lookup(name: &[u8]) -> Option<Filter> {
    // Abbreviations are from inline images (§8.9.7 Table 92).
    Some(match name {
        b"FlateDecode" | b"Fl" => Filter::Data(flate::decode),
        b"LZWDecode" | b"LZW" => Filter::Data(lzw::decode),
        b"ASCIIHexDecode" | b"AHx" => Filter::Data(ascii::decode_hex),
        b"ASCII85Decode" | b"A85" => Filter::Data(ascii::decode_85),
        b"RunLengthDecode" | b"RL" => Filter::Data(runlength::decode),
        b"DCTDecode" | b"DCT" => Filter::Codec(Codec::Dct),
        b"JPXDecode" => Filter::Codec(Codec::Jpx),
        b"JBIG2Decode" => Filter::Codec(Codec::Jbig2),
        b"CCITTFaxDecode" | b"CCF" => Filter::Codec(Codec::Ccitt),
        _ => return None,
    })
}

/// The `/Filter` names paired with their `/DecodeParms`, accepting both the single-value
/// and array forms. Inline-image abbreviations (`F`, `DP`) must already be expanded: in a
/// regular stream `/F` is a file specification.
fn chain(dict: &Dict) -> Vec<(Vec<u8>, Dict)> {
    let names: Vec<&Object> = match dict.get(b"Filter") {
        Some(Object::Array(a)) => a.iter().collect(),
        Some(Object::Null) | None => vec![],
        Some(o) => vec![o],
    };
    let params: Vec<&Object> = match dict.get(b"DecodeParms") {
        Some(Object::Array(a)) => a.iter().collect(),
        Some(o) => vec![o],
        None => vec![],
    };
    names
        .iter()
        .enumerate()
        .map(|(i, n)| {
            let name = n.as_name().unwrap_or_default().to_vec();
            let p = params
                .get(i)
                .and_then(|p| p.as_dict())
                .cloned()
                .unwrap_or_default();
            (name, p)
        })
        .collect()
}

impl Stream {
    /// Runs the filter chain. Problems are reported at `location` and the data decoded
    /// so far is returned. Indirect `/Filter` and `/DecodeParms` values must already be
    /// resolved (`File::resolve` does this).
    pub fn decode(&self, location: Location, diags: &mut Diagnostics) -> Decoded {
        let mut data = self.raw.clone();
        for (name, params) in chain(&self.dict) {
            // §7.4.10: `File` strips a stream's crypt filter after decrypting with it, so one
            // left here is only usable when it changes nothing.
            if name == b"Crypt" {
                match params.get(b"Name").and_then(Object::as_name) {
                    None | Some(b"Identity") => continue,
                    Some(_) => {
                        diags.push(DiagnosticKind::UnknownFilter, location);
                        break;
                    }
                }
            }
            match lookup(&name) {
                Some(Filter::Data(f)) => match f(&data, &params) {
                    Ok(out) => data = out,
                    Err(Failure::Corrupt(out)) => {
                        diags.push(DiagnosticKind::FilterError, location);
                        data = out;
                    }
                    Err(Failure::TooLarge(out)) => {
                        diags.push(DiagnosticKind::StreamTooLarge, location);
                        data = out;
                    }
                },
                Some(Filter::Codec(c)) => {
                    return Decoded {
                        data,
                        codec: Some((c, params)),
                    };
                }
                None => {
                    diags.push(DiagnosticKind::UnknownFilter, location);
                    break;
                }
            }
        }
        Decoded { data, codec: None }
    }
}

/// Appends to `out`, failing with `TooLarge` once the cap is hit.
pub(crate) fn push_capped(out: &mut Vec<u8>, bytes: &[u8]) -> Result<(), ()> {
    push_capped_to(out, bytes, MAX_DECODED_LEN)
}

pub(crate) fn push_capped_to(out: &mut Vec<u8>, bytes: &[u8], cap: usize) -> Result<(), ()> {
    let room = cap.saturating_sub(out.len());
    if bytes.len() > room {
        out.extend_from_slice(&bytes[..room]);
        return Err(());
    }
    out.extend_from_slice(bytes);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::Name;

    fn stream(dict: &[(&str, Object)], raw: &[u8]) -> Stream {
        let mut d = Dict::new();
        for (k, v) in dict {
            d.insert(Name::from(*k), v.clone());
        }
        Stream::new(d, raw.to_vec())
    }

    fn name(s: &str) -> Object {
        Object::Name(Name::from(s))
    }

    #[test]
    fn no_filter_returns_raw() {
        let mut d = Diagnostics::new();
        let out = stream(&[], b"abc").decode(Location::File(0), &mut d);
        assert_eq!(out.data, b"abc");
        assert_eq!(out.codec, None);
    }

    #[test]
    fn chain_runs_in_order_and_stops_at_codec() {
        let mut d = Diagnostics::new();
        let s = stream(
            &[(
                "Filter",
                Object::Array(vec![name("AHx"), name("DCTDecode")]),
            )],
            b"FFD8>",
        );
        let out = s.decode(Location::File(0), &mut d);
        assert_eq!(out.data, vec![0xff, 0xd8]);
        assert_eq!(out.codec, Some((Codec::Dct, Dict::new())));
        assert!(d.is_empty());
    }

    #[test]
    fn unknown_filter_reports_and_stops() {
        let mut d = Diagnostics::new();
        let s = stream(&[("Filter", name("Bogus"))], b"xyz");
        let out = s.decode(Location::File(9), &mut d);
        assert_eq!(out.data, b"xyz");
        assert_eq!(d.as_slice()[0].kind, DiagnosticKind::UnknownFilter);
        assert_eq!(d.as_slice()[0].location, Location::File(9));
    }

    #[test]
    fn crypt_identity_is_skipped() {
        let mut d = Diagnostics::new();
        let s = stream(
            &[
                ("Filter", Object::Array(vec![name("Crypt"), name("AHx")])),
                (
                    "DecodeParms",
                    Object::Array(vec![Object::Null, Object::Null]),
                ),
            ],
            b"4142>",
        );
        assert_eq!(s.decode(Location::File(0), &mut d).data, b"AB");
        assert!(d.is_empty());
    }

    #[test]
    fn crypt_named_filter_outside_file_is_unknown() {
        let mut d = Diagnostics::new();
        let mut params = Dict::new();
        params.insert("Name".into(), name("StdCF"));
        let s = stream(
            &[
                ("Filter", name("Crypt")),
                ("DecodeParms", Object::Dict(params)),
            ],
            b"xyz",
        );
        assert_eq!(s.decode(Location::File(0), &mut d).data, b"xyz");
        assert_eq!(d.as_slice()[0].kind, DiagnosticKind::UnknownFilter);
    }

    #[test]
    fn corrupt_data_keeps_partial_output() {
        let mut d = Diagnostics::new();
        let s = stream(&[("Filter", name("ASCIIHexDecode"))], b"41 42 x");
        let out = s.decode(Location::File(0), &mut d);
        assert_eq!(out.data, b"AB");
        assert_eq!(d.as_slice()[0].kind, DiagnosticKind::FilterError);
    }

    #[test]
    fn null_filter_is_no_filter() {
        let mut d = Diagnostics::new();
        let s = stream(&[("Filter", Object::Null)], b"xyz");
        assert_eq!(s.decode(Location::File(0), &mut d).data, b"xyz");
        assert!(d.is_empty());
    }

    #[test]
    fn file_specification_is_not_a_filter() {
        let mut d = Diagnostics::new();
        let s = stream(&[("F", Object::String(b"ext.dat".to_vec()))], b"xyz");
        assert_eq!(s.decode(Location::File(0), &mut d).data, b"xyz");
        assert!(d.is_empty());
    }
}
