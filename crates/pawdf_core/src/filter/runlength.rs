use super::{Failure, FilterResult, push_capped};
use crate::object::Dict;

/// §7.4.5.
pub(crate) fn decode(input: &[u8], _: &Dict) -> FilterResult {
    let mut out = Vec::new();
    let mut i = 0;
    while let Some(&len) = input.get(i) {
        i += 1;
        let pushed = match len {
            128 => return Ok(out),
            0..=127 => {
                let n = usize::from(len) + 1;
                let Some(run) = input.get(i..i + n) else {
                    let rest = input.get(i..).unwrap_or_default();
                    let _ = push_capped(&mut out, rest);
                    return Err(Failure::Corrupt(out));
                };
                i += n;
                push_capped(&mut out, run)
            }
            _ => {
                let Some(&b) = input.get(i) else {
                    return Err(Failure::Corrupt(out));
                };
                i += 1;
                push_capped(&mut out, &[b; 128][..257 - usize::from(len)])
            }
        };
        if pushed.is_err() {
            return Err(Failure::TooLarge(out));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_and_repeat_runs() {
        match decode(&[2, b'a', b'b', b'c', 254, b'x', 128, 9], &Dict::new()) {
            Ok(v) => assert_eq!(v, b"abcxxx"),
            Err(_) => panic!("expected success"),
        }
    }

    #[test]
    fn truncated_literal_keeps_prefix() {
        match decode(&[4, b'a', b'b'], &Dict::new()) {
            Err(Failure::Corrupt(v)) => assert_eq!(v, b"ab"),
            _ => panic!("expected Corrupt"),
        }
    }
}
