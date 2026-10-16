use super::{Failure, FilterResult};
use crate::object::Dict;

/// §7.4.4.4. Applies the `/Predictor` from Flate or LZW `/DecodeParms`.
pub(crate) fn apply(data: Vec<u8>, params: &Dict) -> FilterResult {
    let get =
        |key: &[u8], default: i64| params.get(key).and_then(|o| o.as_int()).unwrap_or(default);
    let predictor = get(b"Predictor", 1);
    if predictor < 2 {
        return Ok(data);
    }
    let colors = get(b"Colors", 1);
    let bpc = get(b"BitsPerComponent", 8);
    let columns = get(b"Columns", 1);
    if !(1..=32).contains(&colors)
        || ![1, 2, 4, 8, 16].contains(&bpc)
        || !(1..=1 << 20).contains(&columns)
    {
        return Err(Failure::Corrupt(data));
    }
    // Bounded by the checks above, so these fit in usize.
    let bits_per_pixel = (colors * bpc) as usize;
    let bpp = bits_per_pixel.div_ceil(8);
    let row_len = (bits_per_pixel * columns as usize).div_ceil(8);
    if predictor == 2 {
        tiff(data, row_len, colors as usize, bpc)
    } else {
        Ok(png(&data, row_len, bpp))
    }
}

/// PNG predictors: each row starts with its own filter-type byte, whatever `/Predictor`
/// says. A trailing partial row is decoded as far as it goes.
fn png(data: &[u8], row_len: usize, bpp: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut prev = vec![0u8; row_len];
    for chunk in data.chunks(row_len + 1) {
        let (kind, row) = (chunk[0], &chunk[1..]);
        let mut cur = row.to_vec();
        for i in 0..cur.len() {
            let left = if i >= bpp { cur[i - bpp] } else { 0 };
            let up = prev[i];
            let up_left = if i >= bpp { prev[i - bpp] } else { 0 };
            let pred = match kind {
                1 => left,
                2 => up,
                3 => ((u16::from(left) + u16::from(up)) / 2) as u8,
                4 => paeth(left, up, up_left),
                _ => 0,
            };
            cur[i] = cur[i].wrapping_add(pred);
        }
        out.extend_from_slice(&cur);
        prev[..cur.len()].copy_from_slice(&cur);
    }
    out
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = i16::from(a) + i16::from(b) - i16::from(c);
    let (pa, pb, pc) = (
        (p - i16::from(a)).abs(),
        (p - i16::from(b)).abs(),
        (p - i16::from(c)).abs(),
    );
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

/// TIFF predictor 2, horizontal differencing. Only 8- and 16-bit components are supported;
/// other depths are returned undecoded as corrupt.
fn tiff(mut data: Vec<u8>, row_len: usize, colors: usize, bpc: i64) -> FilterResult {
    match bpc {
        8 => {
            for row in data.chunks_mut(row_len) {
                for i in colors..row.len() {
                    row[i] = row[i].wrapping_add(row[i - colors]);
                }
            }
            Ok(data)
        }
        16 => {
            let step = colors * 2;
            for row in data.chunks_mut(row_len) {
                let mut i = step;
                while i + 1 < row.len() {
                    let prev = u16::from_be_bytes([row[i - step], row[i - step + 1]]);
                    let cur = u16::from_be_bytes([row[i], row[i + 1]]);
                    row[i..i + 2].copy_from_slice(&cur.wrapping_add(prev).to_be_bytes());
                    i += 2;
                }
            }
            Ok(data)
        }
        _ => Err(Failure::Corrupt(data)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::object::{Name, Object};

    fn params(pairs: &[(&str, i64)]) -> Dict {
        let mut d = Dict::new();
        for (k, v) in pairs {
            d.insert(Name::from(*k), Object::Int(*v));
        }
        d
    }

    fn ok(r: FilterResult) -> Vec<u8> {
        match r {
            Ok(v) => v,
            Err(_) => panic!("expected success"),
        }
    }

    #[test]
    fn no_predictor_is_identity() {
        assert_eq!(ok(apply(vec![1, 2, 3], &Dict::new())), vec![1, 2, 3]);
    }

    #[test]
    fn png_up_and_sub() {
        // 2 columns, 1 byte per pixel. Row 1: Sub. Row 2: Up.
        let data = vec![1, 10, 5, 2, 1, 1];
        let p = params(&[("Predictor", 12), ("Columns", 2)]);
        assert_eq!(ok(apply(data, &p)), vec![10, 15, 11, 16]);
    }

    #[test]
    fn png_average_and_paeth() {
        let data = vec![0, 10, 20, 3, 1, 1, 4, 1, 1];
        let p = params(&[("Predictor", 15), ("Columns", 2)]);
        // Row 2 Average: [10/2+1, (6+20)/2+1] = [6, 14]. Row 3 Paeth: [6+1, 14+1] = [7, 15].
        assert_eq!(ok(apply(data, &p)), vec![10, 20, 6, 14, 7, 15]);
    }

    #[test]
    fn png_partial_last_row() {
        let p = params(&[("Predictor", 12), ("Columns", 3)]);
        assert_eq!(ok(apply(vec![0, 1, 2, 3, 2, 1], &p)), vec![1, 2, 3, 2]);
    }

    #[test]
    fn tiff_8_and_16_bit() {
        let p = params(&[("Predictor", 2), ("Columns", 3)]);
        assert_eq!(ok(apply(vec![1, 1, 1], &p)), vec![1, 2, 3]);
        let p = params(&[("Predictor", 2), ("Columns", 2), ("BitsPerComponent", 16)]);
        assert_eq!(ok(apply(vec![0, 1, 0, 2], &p)), vec![0, 1, 0, 3]);
    }

    #[test]
    fn bad_params_are_corrupt() {
        let p = params(&[("Predictor", 12), ("Colors", 0)]);
        assert!(matches!(apply(vec![1], &p), Err(Failure::Corrupt(_))));
    }
}
