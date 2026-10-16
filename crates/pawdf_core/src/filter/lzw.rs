use super::{Failure, FilterResult, predictor, push_capped};
use crate::object::Dict;

const CLEAR: usize = 256;
const EOD: usize = 257;

/// §7.4.4. Variable-width codes of 9 to 12 bits, MSB first.
pub(crate) fn decode(input: &[u8], params: &Dict) -> FilterResult {
    let early = params
        .get(b"EarlyChange")
        .and_then(|o| o.as_int())
        .unwrap_or(1)
        != 0;
    let (out, corrupt) = match lzw(input, early) {
        Ok(out) => (out, false),
        Err(Failure::Corrupt(out)) => (out, true),
        Err(Failure::TooLarge(out)) => return Err(Failure::TooLarge(out)),
    };
    match predictor::apply(out, params) {
        Ok(out) if corrupt => Err(Failure::Corrupt(out)),
        r => r,
    }
}

fn lzw(input: &[u8], early: bool) -> FilterResult {
    let mut table: Vec<Vec<u8>> = (0..=255u8).map(|b| vec![b]).collect();
    table.push(vec![]);
    table.push(vec![]);
    let mut out = Vec::new();
    let mut width = 9;
    let mut prev: Option<usize> = None;
    let mut bits: u32 = 0;
    let mut nbits = 0;
    let mut bytes = input.iter();
    loop {
        while nbits < width {
            let Some(&b) = bytes.next() else {
                // Missing EOD is tolerated.
                return Ok(out);
            };
            bits = bits << 8 | u32::from(b);
            nbits += 8;
        }
        let code = ((bits >> (nbits - width)) & ((1 << width) - 1)) as usize;
        nbits -= width;
        bits &= (1 << nbits) - 1;
        match code {
            CLEAR => {
                table.truncate(258);
                width = 9;
                prev = None;
                continue;
            }
            EOD => return Ok(out),
            _ => {}
        }
        let entry = match (table.get(code), prev) {
            (Some(e), _) if !(CLEAR..=EOD).contains(&code) => e.clone(),
            (None, Some(p)) if code == table.len() => {
                let mut e = table[p].clone();
                e.push(table[p][0]);
                e
            }
            _ => return Err(Failure::Corrupt(out)),
        };
        if push_capped(&mut out, &entry).is_err() {
            return Err(Failure::TooLarge(out));
        }
        if let Some(p) = prev
            && table.len() < 4096
        {
            let mut e = table[p].clone();
            e.push(entry[0]);
            table.push(e);
        }
        prev = Some(code);
        let next = table.len() + usize::from(early);
        width = match next {
            ..512 => 9,
            512..1024 => 10,
            1024..2048 => 11,
            _ => 12,
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Packs codes MSB-first at the widths an encoder with EarlyChange=1 would use.
    fn pack(codes: &[(u32, u32)]) -> Vec<u8> {
        let mut out = Vec::new();
        let (mut acc, mut n) = (0u64, 0);
        for &(code, width) in codes {
            acc = acc << width | u64::from(code);
            n += width;
            while n >= 8 {
                out.push((acc >> (n - 8)) as u8);
                n -= 8;
            }
        }
        if n > 0 {
            out.push((acc << (8 - n)) as u8);
        }
        out
    }

    #[test]
    fn spec_example() {
        // §7.4.4.2 Example: "-----A---B" encodes to 256 45 258 258 65 259 66 257.
        let codes: Vec<(u32, u32)> = [256, 45, 258, 258, 65, 259, 66, 257]
            .iter()
            .map(|&c| (c, 9))
            .collect();
        match decode(&pack(&codes), &Dict::new()) {
            Ok(v) => assert_eq!(v, b"-----A---B"),
            Err(_) => panic!("expected success"),
        }
    }

    #[test]
    fn invalid_code_is_corrupt_with_prefix() {
        let codes = [(65, 9), (300, 9)];
        match decode(&pack(&codes), &Dict::new()) {
            Err(Failure::Corrupt(v)) => assert_eq!(v, b"A"),
            _ => panic!("expected Corrupt"),
        }
    }
}
