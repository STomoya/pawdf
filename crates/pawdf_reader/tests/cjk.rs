mod common;

use common::{one_page, stream};
use pawdf_core::DiagnosticKind;
use pawdf_reader::{Document, FontKind, Glyph, PageContent, WritingMode};

const FONT_RES: &str = "<< /Font << /F1 5 0 R >> >>";

fn run(ops: &[u8], extra: &[(u32, &[u8])]) -> PageContent {
    Document::from_bytes(one_page(ops, FONT_RES, extra))
        .unwrap()
        .page(0)
        .unwrap()
        .content()
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

fn at(g: &Glyph, x: f64, y: f64) {
    assert!(
        close(g.origin.x, x) && close(g.origin.y, y),
        "{:?} at {:?}, expected ({x}, {y})",
        g.text,
        g.origin
    );
}

fn kinds(c: &PageContent) -> Vec<DiagnosticKind> {
    c.diagnostics.iter().map(|d| d.kind).collect()
}

fn texts(c: &PageContent) -> Vec<&str> {
    c.glyphs.iter().map(|g| g.text.as_str()).collect()
}

/// A Type0 font (object 5) over a CIDFontType2 descendant (object 6) with `encoding` and
/// extra descendant entries; object 7 is free for a ToUnicode or CMap stream.
fn type0(encoding: &str, descendant_extra: &str) -> (String, String) {
    (
        format!(
            "<< /Type /Font /Subtype /Type0 /BaseFont /Foo /Encoding {encoding} /DescendantFonts [6 0 R] >>"
        ),
        format!(
            "<< /Type /Font /Subtype /CIDFontType2 /BaseFont /Foo /CIDSystemInfo << /Registry (Adobe) /Ordering (Identity) /Supplement 0 >> {descendant_extra} >>"
        ),
    )
}

// Spec §11: the bundled CMap path, with no embedded font and no ToUnicode.
#[test]
fn predefined_encoding_without_tounicode() {
    let c = run(
        b"BT /F1 10 Tf 0 0 Td <30423044> Tj ET",
        &[
            (5, b"<< /Type /Font /Subtype /Type0 /BaseFont /KozMinPr6N-Regular /Encoding /UniJIS-UTF16-H /DescendantFonts [6 0 R] >>"),
            (6, b"<< /Type /Font /Subtype /CIDFontType0 /BaseFont /KozMinPr6N-Regular /CIDSystemInfo << /Registry (Adobe) /Ordering (Japan1) /Supplement 6 >> /DW 1000 /W [843 [500]] /FontDescriptor 7 0 R >>"),
            (7, b"<< /Type /FontDescriptor /FontName /KozMinPr6N-Regular /Flags 4 /Ascent 880 /Descent -120 /FontBBox [0 -120 1000 880] >>"),
        ],
    );
    assert!(c.diagnostics.is_empty(), "{:?}", c.diagnostics);
    assert_eq!(texts(&c), ["\u{3042}", "\u{3044}"]);
    assert_eq!((c.glyphs[0].code, c.glyphs[1].code), (0x3042, 0x3044));
    at(&c.glyphs[1], 5., 0.);
    assert!(close(c.glyphs[0].bbox.y0, -1.2) && close(c.glyphs[0].bbox.y1, 8.8));
    let f = &c.fonts[0];
    assert_eq!(
        (f.kind, f.wmode, f.embedded),
        (FontKind::Type0, WritingMode::Horizontal, false)
    );
    assert_eq!(f.base_font, "KozMinPr6N-Regular");
}

#[test]
fn identity_h_with_tounicode_and_dw() {
    let (f, d) = type0("/Identity-H", "");
    let (f, d) = (
        f.replace("/DescendantFonts", "/ToUnicode 7 0 R /DescendantFonts"),
        d,
    );
    let tu = stream(
        "",
        b"1 begincodespacerange <0000> <FFFF> endcodespacerange 1 beginbfchar <0001> <65E5> endbfchar",
    );
    let c = run(
        b"BT /F1 10 Tf <00010001> Tj ET",
        &[(5, f.as_bytes()), (6, d.as_bytes()), (7, &tu)],
    );
    assert!(c.diagnostics.is_empty(), "{:?}", c.diagnostics);
    assert_eq!(texts(&c), ["\u{65E5}", "\u{65E5}"]);
    assert!(close(c.glyphs[0].advance, 10.));
    at(&c.glyphs[1], 10., 0.);
}

#[test]
fn sjis_mixed_lengths() {
    let f = b"<< /Type /Font /Subtype /Type0 /BaseFont /Foo /Encoding /90ms-RKSJ-H /DescendantFonts [6 0 R] >>";
    let d = b"<< /Type /Font /Subtype /CIDFontType0 /BaseFont /Foo /CIDSystemInfo << /Registry (Adobe) /Ordering (Japan1) /Supplement 2 >> /W [231 325 500] >>";
    let c = run(b"BT /F1 10 Tf (A\x82\xa0) Tj ET", &[(5, f), (6, d)]);
    assert!(c.diagnostics.is_empty(), "{:?}", c.diagnostics);
    assert_eq!(texts(&c), ["A", "\u{3042}"]);
    assert_eq!((c.glyphs[0].code, c.glyphs[1].code), (0x41, 0x82a0));
    at(&c.glyphs[1], 5., 0.);
}

#[test]
fn w_array_forms() {
    let (f, d) = type0("/Identity-H", "/W [1 [100 200] 10 20 300] /DW 900");
    let c = run(
        b"BT /F1 1 Tf <0001000200140015> Tj ET",
        &[(5, f.as_bytes()), (6, d.as_bytes())],
    );
    let xs: Vec<f64> = c.glyphs.iter().map(|g| g.origin.x).collect();
    assert!(
        close(xs[1], 0.1) && close(xs[2], 0.3) && close(xs[3], 0.6),
        "{xs:?}"
    );
    assert!(close(c.glyphs[3].advance, 0.9));
}

#[test]
fn bad_w_entries_are_skipped() {
    let (f, d) = type0("/Identity-H", "/W [-5 [500] 0 4294967295 700 (x) 3]");
    let c = run(
        b"BT /F1 1 Tf <0007> Tj ET",
        &[(5, f.as_bytes()), (6, d.as_bytes())],
    );
    assert!(close(c.glyphs[0].advance, 0.7));
    assert!(
        kinds(&c).contains(&DiagnosticKind::BadFont),
        "{:?}",
        kinds(&c)
    );
}

#[test]
fn truncated_code_is_one_glyph() {
    let (f, d) = type0("/Identity-H", "");
    let c = run(
        b"BT /F1 1 Tf <4E> Tj <4E> Tj ET",
        &[(5, f.as_bytes()), (6, d.as_bytes())],
    );
    assert_eq!(c.glyphs.len(), 2);
    assert_eq!(c.glyphs[0].code, 0x4E);
    let bad = kinds(&c)
        .iter()
        .filter(|k| **k == DiagnosticKind::BadCMap)
        .count();
    assert_eq!(bad, 1, "{:?}", kinds(&c));
}

#[test]
fn tw_ignores_multibyte_32() {
    let (f, d) = type0("/Identity-H", "");
    let c = run(
        b"BT /F1 1 Tf 5 Tw <00200020> Tj ET",
        &[(5, f.as_bytes()), (6, d.as_bytes())],
    );
    at(&c.glyphs[1], 1., 0.);
}

#[test]
fn embedded_encoding_cmap_with_usecmap() {
    let (f, d) = type0("7 0 R", "/W [843 [500]]");
    let d = d.replace("(Identity)", "(Japan1)");
    let cmap = stream(
        "/Type /CMap /CMapName /Custom /UseCMap /UniJIS-UTF16-H",
        b"1 begincidchar <0041> 843 endcidchar",
    );
    let c = run(
        b"BT /F1 10 Tf <00413044> Tj ET",
        &[(5, f.as_bytes()), (6, d.as_bytes()), (7, &cmap)],
    );
    assert!(c.diagnostics.is_empty(), "{:?}", c.diagnostics);
    assert_eq!(texts(&c), ["\u{3042}", "\u{3044}"]);
}

#[test]
fn broken_type0_still_emits() {
    let f = b"<< /Type /Font /Subtype /Type0 /BaseFont /Foo /Encoding /Nope-H >>";
    let c = run(b"BT /F1 1 Tf <0001> Tj ET", &[(5, f)]);
    assert_eq!(texts(&c), ["\u{FFFD}"]);
    assert!(close(c.glyphs[0].advance, 1.));
    let k = kinds(&c);
    for want in [
        DiagnosticKind::BadCMap,
        DiagnosticKind::BadFont,
        DiagnosticKind::UnmappedGlyph,
    ] {
        assert!(k.contains(&want), "{k:?}");
    }
}

#[test]
fn vertical_identity_v_geometry() {
    let (f, d) = type0("/Identity-V", "");
    let c = run(
        b"BT /F1 10 Tf 100 700 Td <00010002> Tj ET",
        &[(5, f.as_bytes()), (6, d.as_bytes())],
    );
    let [a, b] = &c.glyphs[..] else {
        panic!("{:?}", c.glyphs)
    };
    assert_eq!(a.wmode, WritingMode::Vertical);
    assert_eq!(c.fonts[0].wmode, WritingMode::Vertical);
    at(a, 95., 691.2);
    at(b, 95., 681.2);
    assert!(close(a.advance, 10.));
    let r = a.bbox;
    assert!(
        close(r.x0, 95.) && close(r.y0, 690.) && close(r.x1, 105.) && close(r.y1, 700.),
        "{r:?}"
    );
}

#[test]
fn vertical_w2_tc_and_tj() {
    let (f, d) = type0("/Identity-V", "/W2 [1 [-500 250 400]]");
    let c = run(
        b"BT /F1 10 Tf 1 Tc 100 700 Td [<0001> 500 <0001>] TJ ET",
        &[(5, f.as_bytes()), (6, d.as_bytes())],
    );
    // ty = w1·Tfs + Tc = -4; the TJ number moves down by 500/1000·Tfs = 5.
    at(&c.glyphs[0], 97.5, 696.);
    at(&c.glyphs[1], 97.5, 687.);
    assert!(close(c.glyphs[0].advance, 4.));
}
