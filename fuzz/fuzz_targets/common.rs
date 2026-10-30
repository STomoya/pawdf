//! Builds a small PDF around fuzz input so each parser is reachable through the public API.
#![allow(dead_code)]

fn stream(dict: &str, data: &[u8]) -> Vec<u8> {
    let mut out = format!("<< /Length {} {dict} >>\nstream\n", data.len()).into_bytes();
    out.extend_from_slice(data);
    out.extend_from_slice(b"\nendstream");
    out
}

/// One page whose content is `content`, with font `/F1`, image `/Im1` and form `/Fm1` in
/// its resources. `to_unicode`, if given, becomes `/F1`'s ToUnicode stream.
pub fn one_page(content: &[u8], to_unicode: Option<&[u8]>) -> Vec<u8> {
    let res = "<< /Font << /F1 5 0 R >> /XObject << /Im1 6 0 R /Fm1 7 0 R >> >>";
    let font = match to_unicode {
        Some(_) => "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /ToUnicode 8 0 R >>",
        None => "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
    };
    let objects: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources {res} >>").into_bytes(),
        stream("", content),
        font.as_bytes().to_vec(),
        stream("/Type /XObject /Subtype /Image /Width 2 /Height 2 /ColorSpace /DeviceGray /BitsPerComponent 8", b"\x00\x01\x02\x03"),
        stream(
            &format!("/Type /XObject /Subtype /Form /BBox [0 0 1 1] /Resources {res}"),
            b"q /Im1 Do Q 0 0 m 1 1 l S BT /F1 1 Tf (a) Tj ET",
        ),
        stream("", to_unicode.unwrap_or_default()),
    ];
    assemble(&objects)
}

/// One page showing `shown` (as a hex string, with `Tj` and `TJ`) in a Type0 font whose
/// `/Encoding` and `/ToUnicode` are both `cmap`, or whose encoding is Identity-V when
/// `vertical`.
pub fn type0_page(cmap: &[u8], shown: &[u8], vertical: bool) -> Vec<u8> {
    let hex: String = shown.iter().map(|b| format!("{b:02X}")).collect();
    let content = format!("BT /F1 12 Tf <{hex}> Tj [<{hex}> -250 <{hex}>] TJ ET");
    let encoding = if vertical { "/Identity-V" } else { "6 0 R" };
    let objects: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >>".to_vec(),
        stream("", content.as_bytes()),
        format!("<< /Type /Font /Subtype /Type0 /BaseFont /Foo /Encoding {encoding} /ToUnicode 6 0 R /DescendantFonts [7 0 R] >>").into_bytes(),
        stream("/Type /CMap /UseCMap /UniJIS-UTF16-H", cmap),
        b"<< /Type /Font /Subtype /CIDFontType0 /BaseFont /Foo /CIDSystemInfo << /Registry (Adobe) /Ordering (Japan1) /Supplement 6 >> /W [1 [500 600] 100 4294967295 700] /W2 [1 [-1000 250 880]] >>".to_vec(),
    ];
    assemble(&objects)
}

/// A file whose `/Encrypt` dictionary is `dict` (object 4), with an encrypted-looking string
/// (object 2) and a 32-byte stream behind a `/StdCF` crypt filter (object 3).
pub fn encrypted(dict: &[u8]) -> Vec<u8> {
    let objects: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog >>".to_vec(),
        b"(0123456789abcdef)".to_vec(),
        stream(
            "/Filter [/Crypt] /DecodeParms [<< /Name /StdCF >>]",
            b"0123456789abcdef0123456789abcdef",
        ),
        dict.to_vec(),
    ];
    assemble_with(
        &objects,
        "/Encrypt 4 0 R /ID [<00112233445566778899aabbccddeeff> <00>]",
    )
}

/// A one-page file with content stream `content` (object 4) whose objects from 5 on are the
/// fuzz input split at 0xFF, so the input builds its own object graph. `catalog` and `page`
/// are extra entries for the catalog (1) and the page (3), typically pointing at `5 0 R`.
pub fn objects(catalog: &str, page: &str, content: &[u8], data: &[u8]) -> Vec<u8> {
    let mut objects: Vec<Vec<u8>> = vec![
        format!("<< /Type /Catalog /Pages 2 0 R {catalog} >>").into_bytes(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R {page} >>")
            .into_bytes(),
        stream("", content),
    ];
    objects.extend(data.split(|&b| b == 0xFF).map(<[u8]>::to_vec));
    assemble(&objects)
}

fn assemble(objects: &[Vec<u8>]) -> Vec<u8> {
    assemble_with(objects, "")
}

fn assemble_with(objects: &[Vec<u8>], trailer_extra: &str) -> Vec<u8> {
    let mut out = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (i, body) in objects.iter().enumerate() {
        offsets.push(out.len());
        out.extend_from_slice(format!("{} 0 obj\n", i + 1).as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    }
    let xref = out.len();
    out.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for off in offsets {
        out.extend_from_slice(format!("{off:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R {trailer_extra} >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    out
}
