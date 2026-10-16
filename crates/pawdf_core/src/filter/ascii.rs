use super::{Failure, FilterResult, MAX_DECODED_LEN, push_capped_to};
use crate::lexer::{hex_value, is_whitespace};
use crate::object::Dict;

/// §7.4.2. Output is at most half the input, so no size cap is needed.
pub(crate) fn decode_hex(input: &[u8], _: &Dict) -> FilterResult {
    let mut out = Vec::with_capacity(input.len() / 2);
    let mut hi: Option<u8> = None;
    let mut corrupt = false;
    for &b in input {
        if b == b'>' {
            break;
        }
        if is_whitespace(b) {
            continue;
        }
        let Some(v) = hex_value(b) else {
            corrupt = true;
            break;
        };
        match hi.take() {
            Some(h) => out.push(h << 4 | v),
            None => hi = Some(v),
        }
    }
    if let Some(h) = hi {
        out.push(h << 4);
    }
    if corrupt {
        Err(Failure::Corrupt(out))
    } else {
        Ok(out)
    }
}

/// §7.4.3.
pub(crate) fn decode_85(input: &[u8], _: &Dict) -> FilterResult {
    decode_85_capped(input, MAX_DECODED_LEN)
}

fn decode_85_capped(input: &[u8], cap: usize) -> FilterResult {
    let input = input.strip_prefix(b"<~").unwrap_or(input);
    let mut out = Vec::with_capacity(input.len());
    let mut group = [0u8; 5];
    let mut n = 0;
    for &b in input {
        match b {
            b'~' => break,
            b'z' if n == 0 => {
                if push_capped_to(&mut out, &[0; 4], cap).is_err() {
                    return Err(Failure::TooLarge(out));
                }
            }
            b'!'..=b'u' => {
                group[n] = b - b'!';
                n += 1;
                if n == 5 {
                    let Some(word) = group_value(&group) else {
                        return Err(Failure::Corrupt(out));
                    };
                    if push_capped_to(&mut out, &word.to_be_bytes(), cap).is_err() {
                        return Err(Failure::TooLarge(out));
                    }
                    n = 0;
                }
            }
            b if is_whitespace(b) => {}
            _ => return Err(Failure::Corrupt(out)),
        }
    }
    match n {
        0 => Ok(out),
        1 => Err(Failure::Corrupt(out)),
        _ => {
            // A final partial group of n characters is padded with `u` and yields n - 1 bytes.
            for slot in &mut group[n..] {
                *slot = b'u' - b'!';
            }
            let Some(word) = group_value(&group) else {
                return Err(Failure::Corrupt(out));
            };
            if push_capped_to(&mut out, &word.to_be_bytes()[..n - 1], cap).is_err() {
                return Err(Failure::TooLarge(out));
            }
            Ok(out)
        }
    }
}

fn group_value(group: &[u8; 5]) -> Option<u32> {
    let v = group.iter().fold(0u64, |acc, &d| acc * 85 + u64::from(d));
    u32::try_from(v).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(r: FilterResult) -> Vec<u8> {
        match r {
            Ok(v) => v,
            Err(_) => panic!("expected success"),
        }
    }

    fn partial(r: FilterResult) -> Vec<u8> {
        match r {
            Err(Failure::Corrupt(v)) => v,
            _ => panic!("expected Corrupt"),
        }
    }

    #[test]
    fn hex() {
        assert_eq!(
            ok(decode_hex(b"48 65\n6C6c6F>ignored", &Dict::new())),
            b"Hello"
        );
        assert_eq!(ok(decode_hex(b"414", &Dict::new())), b"A@");
        assert_eq!(partial(decode_hex(b"41g42", &Dict::new())), b"A");
    }

    #[test]
    fn ascii85() {
        assert_eq!(
            ok(decode_85(b"87cURD]i,\"Ebo80~>", &Dict::new())),
            b"Hello World!"
        );
        assert_eq!(ok(decode_85(b"<~z!!~>", &Dict::new())), vec![0, 0, 0, 0, 0]);
        assert_eq!(ok(decode_85(b"87cURDZ~>", &Dict::new())), b"Hello");
    }

    /// Review I3: `z` expands 1 byte to 4, so it must respect the output cap.
    #[test]
    fn ascii85_z_respects_cap() {
        match decode_85_capped(b"zzz~>", 6) {
            Err(Failure::TooLarge(v)) => assert_eq!(v, vec![0; 6]),
            _ => panic!("expected TooLarge"),
        }
    }

    #[test]
    fn ascii85_errors() {
        assert_eq!(
            partial(decode_85(b"87cURD]i,\"Ebo80!~>", &Dict::new())),
            b"Hello World!"
        );
        assert_eq!(partial(decode_85(b"s8W-\"", &Dict::new())), b"");
        assert_eq!(partial(decode_85(b"87cUR{", &Dict::new())), b"Hell");
    }
}
