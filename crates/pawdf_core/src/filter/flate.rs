use miniz_oxide::inflate::TINFLStatus;
use miniz_oxide::inflate::core::{DecompressorOxide, decompress, inflate_flags};

use super::{Failure, FilterResult, MAX_DECODED_LEN, predictor};
use crate::object::Dict;

/// §7.4.4. Accepts zlib-wrapped and raw deflate data, and ignores Adler-32 mismatches,
/// which are common in real files.
pub(crate) fn decode(input: &[u8], params: &Dict) -> FilterResult {
    let flags =
        inflate_flags::TINFL_FLAG_PARSE_ZLIB_HEADER | inflate_flags::TINFL_FLAG_IGNORE_ADLER32;
    let inflated = match inflate(input, flags) {
        Err(Failure::Corrupt(out)) if out.is_empty() => inflate(input, 0),
        r => r,
    };
    match inflated {
        Ok(out) => predictor::apply(out, params),
        Err(Failure::Corrupt(out)) => match predictor::apply(out, params) {
            Ok(out) | Err(Failure::Corrupt(out)) => Err(Failure::Corrupt(out)),
            Err(Failure::TooLarge(out)) => Err(Failure::TooLarge(out)),
        },
        Err(Failure::TooLarge(out)) => Err(Failure::TooLarge(out)),
    }
}

// miniz_oxide's `decompress_to_vec*` return the zero-padded buffer on error, so the
// valid prefix length is lost. This loop tracks it.
fn inflate(input: &[u8], flags: u32) -> FilterResult {
    let flags = flags | inflate_flags::TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF;
    let mut state = Box::<DecompressorOxide>::default();
    let mut out = vec![0u8; input.len().saturating_mul(4).clamp(1024, MAX_DECODED_LEN)];
    let mut in_pos = 0;
    let mut out_pos = 0;
    loop {
        let (status, consumed, written) =
            decompress(&mut state, &input[in_pos..], &mut out, out_pos, flags);
        in_pos = (in_pos + consumed).min(input.len());
        out_pos += written;
        match status {
            TINFLStatus::Done => {
                out.truncate(out_pos);
                return Ok(out);
            }
            TINFLStatus::HasMoreOutput => {
                if out.len() >= MAX_DECODED_LEN {
                    out.truncate(out_pos);
                    return Err(Failure::TooLarge(out));
                }
                let new_len = out.len().saturating_mul(2).min(MAX_DECODED_LEN);
                out.resize(new_len, 0);
            }
            _ => {
                out.truncate(out_pos);
                return Err(Failure::Corrupt(out));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use miniz_oxide::deflate::{compress_to_vec, compress_to_vec_zlib};

    fn ok(r: FilterResult) -> Vec<u8> {
        match r {
            Ok(v) => v,
            Err(_) => panic!("expected success"),
        }
    }

    #[test]
    fn zlib_and_raw() {
        let text = b"hello hello hello hello".repeat(100);
        assert_eq!(
            ok(decode(&compress_to_vec_zlib(&text, 6), &Dict::new())),
            text
        );
        assert_eq!(ok(decode(&compress_to_vec(&text, 6), &Dict::new())), text);
    }

    #[test]
    fn bad_checksum_is_ignored() {
        let mut z = compress_to_vec_zlib(b"abc", 6);
        let n = z.len();
        z[n - 1] ^= 0xff;
        assert_eq!(ok(decode(&z, &Dict::new())), b"abc");
    }

    #[test]
    fn truncated_keeps_prefix() {
        let text: Vec<u8> = (0..20_000u32).map(|i| (i * 7 % 251) as u8).collect();
        let z = compress_to_vec_zlib(&text, 6);
        match decode(&z[..z.len() / 2], &Dict::new()) {
            Err(Failure::Corrupt(out)) => {
                assert!(!out.is_empty());
                assert_eq!(&text[..out.len()], &out[..]);
            }
            _ => panic!("expected Corrupt"),
        }
    }

    #[test]
    fn garbage_is_corrupt() {
        assert!(matches!(
            decode(b"\xff\xff\xff\xff", &Dict::new()),
            Err(Failure::Corrupt(_))
        ));
    }
}
