//! Unit-test helpers. Integration tests have their own copy in `tests/common`.

use pawdf_core::File;

/// A PDF with the given objects (generation 0), a classic xref table, and `/Root 1 0 R`.
pub fn build_pdf(objects: &[(u32, &[u8])]) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (num, body) in objects {
        offsets.push((*num, out.len()));
        out.extend_from_slice(format!("{num} 0 obj\n").as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let size = objects.iter().map(|(n, _)| n + 1).max().unwrap_or(1);
    let xref_at = out.len();
    out.extend_from_slice(format!("xref\n0 {size}\n0000000000 65535 f\n").as_bytes());
    for num in 1..size {
        match offsets.iter().find(|(n, _)| *n == num) {
            Some((_, off)) => out.extend_from_slice(format!("{off:010} 00000 n\n").as_bytes()),
            None => out.extend_from_slice(b"0000000000 65535 f\n"),
        }
    }
    out.extend_from_slice(
        format!("trailer\n<< /Size {size} /Root 1 0 R >>\nstartxref\n{xref_at}\n%%EOF\n")
            .as_bytes(),
    );
    out
}

/// A stream object body with a correct `/Length`.
pub fn stream(dict_extra: &str, data: &[u8]) -> Vec<u8> {
    let mut out = format!("<< /Length {} {dict_extra} >>\nstream\n", data.len()).into_bytes();
    out.extend_from_slice(data);
    out.extend_from_slice(b"\nendstream");
    out
}

/// A `File` holding a catalog as object 1 plus `objects`.
pub fn file(objects: &[(u32, &[u8])]) -> File {
    let mut all: Vec<(u32, &[u8])> = vec![(1, b"<< /Type /Catalog /Pages 2 0 R >>")];
    all.extend_from_slice(objects);
    File::from_bytes(build_pdf(&all)).expect("test file opens")
}

/// One `cmap` subtable: platform, encoding, and `(codepoint, gid)` pairs.
pub type Subtable<'a> = (u16, u16, &'a [(u16, u16)]);

/// An sfnt holding only a `cmap` table with one format 4 subtable per
/// `(platform, encoding, [(codepoint, gid)])`, one segment per pair.
pub fn sfnt(subtables: &[Subtable]) -> Vec<u8> {
    let be16 = |v: &mut Vec<u8>, x: u16| v.extend_from_slice(&x.to_be_bytes());
    let mut bodies = Vec::new();
    for (_, _, pairs) in subtables {
        let mut segs: Vec<(u16, u16)> = pairs.to_vec();
        segs.sort();
        segs.push((0xFFFF, 0));
        let n = segs.len() as u16;
        let mut b = Vec::new();
        be16(&mut b, 4);
        be16(&mut b, 16 + 8 * n);
        be16(&mut b, 0);
        be16(&mut b, 2 * n);
        for _ in 0..3 {
            be16(&mut b, 0);
        }
        for (cp, _) in &segs {
            be16(&mut b, *cp);
        }
        be16(&mut b, 0);
        for (cp, _) in &segs {
            be16(&mut b, *cp);
        }
        for (cp, gid) in &segs {
            let delta = if *cp == 0xFFFF {
                1
            } else {
                gid.wrapping_sub(*cp)
            };
            be16(&mut b, delta);
        }
        for _ in &segs {
            be16(&mut b, 0);
        }
        bodies.push(b);
    }
    let mut cmap = Vec::new();
    be16(&mut cmap, 0);
    be16(&mut cmap, subtables.len() as u16);
    let mut offset = 4 + 8 * subtables.len();
    for ((platform, encoding, _), body) in subtables.iter().zip(&bodies) {
        be16(&mut cmap, *platform);
        be16(&mut cmap, *encoding);
        cmap.extend_from_slice(&(offset as u32).to_be_bytes());
        offset += body.len();
    }
    for body in &bodies {
        cmap.extend_from_slice(body);
    }
    let mut out = Vec::new();
    out.extend_from_slice(&0x0001_0000u32.to_be_bytes());
    be16(&mut out, 1);
    for _ in 0..3 {
        be16(&mut out, 0);
    }
    out.extend_from_slice(b"cmap");
    out.extend_from_slice(&0u32.to_be_bytes());
    out.extend_from_slice(&28u32.to_be_bytes());
    out.extend_from_slice(&(cmap.len() as u32).to_be_bytes());
    out.extend_from_slice(&cmap);
    out
}

/// A CFF INDEX with 2-byte offsets.
fn cff_index(items: &[&[u8]]) -> Vec<u8> {
    if items.is_empty() {
        return vec![0, 0];
    }
    let mut out = (items.len() as u16).to_be_bytes().to_vec();
    out.push(2);
    let mut off = 1u16;
    out.extend_from_slice(&off.to_be_bytes());
    for item in items {
        off += item.len() as u16;
        out.extend_from_slice(&off.to_be_bytes());
    }
    for item in items {
        out.extend_from_slice(item);
    }
    out
}

/// A bare CFF font: `.notdef` plus one empty glyph per `(code, name)`, with custom glyph
/// names and a format 0 encoding mapping each code to its glyph.
pub fn cff(glyphs: &[(u8, &str)]) -> Vec<u8> {
    let names: Vec<&[u8]> = glyphs.iter().map(|(_, n)| n.as_bytes()).collect();
    let mut head = vec![1, 0, 4, 1];
    head.extend(cff_index(&[b"Test"]));
    let top_len = cff_index(&[&[0; 18]]).len();
    let strings = cff_index(&names);
    let charset_at = head.len() + top_len + strings.len() + 2;
    let mut charset = vec![0];
    for i in 0..glyphs.len() {
        charset.extend_from_slice(&(391 + i as u16).to_be_bytes());
    }
    let encoding_at = charset_at + charset.len();
    let mut encoding = vec![0, glyphs.len() as u8];
    encoding.extend(glyphs.iter().map(|(c, _)| *c));
    let char_strings_at = encoding_at + encoding.len();
    let mut top = Vec::new();
    for (value, op) in [(charset_at, 15), (encoding_at, 16), (char_strings_at, 17)] {
        top.push(0x1d);
        top.extend_from_slice(&(value as i32).to_be_bytes());
        top.push(op);
    }
    let endchar: &[u8] = &[0x0e];
    let mut out = head;
    out.extend(cff_index(&[&top]));
    out.extend(strings);
    out.extend([0, 0]);
    out.extend(charset);
    out.extend(encoding);
    out.extend(cff_index(&vec![endchar; glyphs.len() + 1]));
    out
}

/// An sfnt holding the given tables.
pub fn sfnt_tables(tables: &[([u8; 4], &[u8])]) -> Vec<u8> {
    let mut out = b"OTTO".to_vec();
    out.extend_from_slice(&(tables.len() as u16).to_be_bytes());
    out.extend_from_slice(&[0; 6]);
    let mut offset = 12 + 16 * tables.len();
    for (tag, data) in tables {
        out.extend_from_slice(tag);
        out.extend_from_slice(&0u32.to_be_bytes());
        out.extend_from_slice(&(offset as u32).to_be_bytes());
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        offset += data.len().next_multiple_of(4);
    }
    for (_, data) in tables {
        out.extend_from_slice(data);
        out.resize(out.len().next_multiple_of(4), 0);
    }
    out
}
