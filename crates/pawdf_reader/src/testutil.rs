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
