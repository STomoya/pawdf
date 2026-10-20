//! Test-only helper that assembles small PDFs with correct xref offsets.

#[allow(dead_code)]
pub mod crypt;

/// Builds a PDF with the given objects (generation 0), a classic xref table, and a trailer
/// containing `/Size` plus `trailer_extra`. Returns the bytes and each object's offset.
pub fn build_pdf(objects: &[(u32, &[u8])], trailer_extra: &str) -> (Vec<u8>, Vec<(u32, usize)>) {
    build_pdf_eol(objects, trailer_extra, "\n")
}

/// Like [`build_pdf`], with `eol` as the line ending everywhere outside object bodies.
pub fn build_pdf_eol(
    objects: &[(u32, &[u8])],
    trailer_extra: &str,
    eol: &str,
) -> (Vec<u8>, Vec<(u32, usize)>) {
    let mut out = format!("%PDF-1.7{eol}").into_bytes();
    let mut offsets = Vec::new();
    for (num, body) in objects {
        offsets.push((*num, out.len()));
        out.extend_from_slice(format!("{num} 0 obj{eol}").as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(format!("{eol}endobj{eol}").as_bytes());
    }
    let size = objects.iter().map(|(n, _)| n + 1).max().unwrap_or(1);
    let xref_at = out.len();
    out.extend_from_slice(format!("xref{eol}0 {size}{eol}0000000000 65535 f{eol}").as_bytes());
    for num in 1..size {
        match offsets.iter().find(|(n, _)| *n == num) {
            Some((_, off)) => out.extend_from_slice(format!("{off:010} 00000 n{eol}").as_bytes()),
            None => out.extend_from_slice(format!("0000000000 65535 f{eol}").as_bytes()),
        }
    }
    out.extend_from_slice(
        format!("trailer{eol}<< /Size {size} {trailer_extra} >>{eol}startxref{eol}{xref_at}{eol}%%EOF{eol}")
            .as_bytes(),
    );
    (out, offsets)
}

/// A minimal catalog + pages pair as objects 1 and 2.
pub const CATALOG: &[u8] = b"<< /Type /Catalog /Pages 2 0 R >>";
pub const PAGES: &[u8] = b"<< /Type /Pages /Kids [] /Count 0 >>";
