mod common;

use common::one_page;
use pawdf_core::DiagnosticKind;
use pawdf_reader::{Color, Document, FontKind, Glyph, Matrix, PageContent, Rect, RenderMode};

const HELV: &[u8] = b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>";

fn run(ops: &[u8], resources: &str, extra: &[(u32, &[u8])]) -> PageContent {
    Document::from_bytes(one_page(ops, resources, extra))
        .unwrap()
        .page(0)
        .unwrap()
        .content()
}

fn text(ops: &[u8]) -> PageContent {
    run(ops, "<< /Font << /F1 5 0 R >> >>", &[(5, HELV)])
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

#[test]
fn glyph_positions() {
    let c = text(b"BT /F1 10 Tf 100 200 Td (ab) Tj ET");
    assert!(c.diagnostics.is_empty(), "{:?}", c.diagnostics);
    let [a, b] = &c.glyphs[..] else {
        panic!("{:?}", c.glyphs)
    };
    assert_eq!((a.text.as_str(), b.text.as_str()), ("a", "b"));
    assert_eq!((a.code, a.size, a.font), (97, 10., 0));
    at(a, 100., 200.);
    at(b, 105.56, 200.);
    assert!(close(a.advance, 5.56));
    assert_eq!(
        a.matrix,
        Matrix {
            a: 10.,
            b: 0.,
            c: 0.,
            d: 10.,
            e: 100.,
            f: 200.
        }
    );
    assert_eq!(
        (a.render_mode, a.fill.clone()),
        (RenderMode::Fill, Color::Gray(0.))
    );
    assert_eq!(c.fonts.len(), 1);
    assert_eq!(c.fonts[0].base_font, "Helvetica");
    assert_eq!(c.fonts[0].kind, FontKind::Type1);
}

#[test]
fn char_and_word_spacing() {
    let c = text(b"BT /F1 10 Tf 1 Tc 2 Tw (a a) Tj ET");
    let adv: Vec<f64> = c.glyphs.iter().map(|g| g.advance).collect();
    assert!(
        close(adv[0], 6.56) && close(adv[1], 5.78) && close(adv[2], 6.56),
        "{adv:?}"
    );
    at(&c.glyphs[2], 12.34, 0.);
}

#[test]
fn horizontal_scaling_and_rise() {
    let c = text(b"BT /F1 10 Tf 50 Tz 3 Ts (a) Tj ET");
    let g = &c.glyphs[0];
    assert_eq!((g.matrix.a, g.matrix.d, g.matrix.f), (5., 10., 3.));
    assert!(close(g.advance, 2.78));
}

#[test]
fn tj_adjustment_moves_without_glyph() {
    let c = text(b"BT /F1 10 Tf [(a) -1000 (b)] TJ ET");
    assert_eq!(c.glyphs.len(), 2);
    at(&c.glyphs[1], 15.56, 0.);
}

#[test]
fn line_operators() {
    let c = text(b"BT /F1 10 Tf 12 TL 0 100 Td (a) Tj T* (b) Tj (c) ' 1 2 (d) \" ET");
    at(&c.glyphs[1], 0., 88.);
    at(&c.glyphs[2], 0., 76.);
    at(&c.glyphs[3], 0., 64.);
    assert!(close(c.glyphs[3].advance, 7.56));
    let c = text(b"BT /F1 10 Tf 5 5 TD (a) Tj T* (b) Tj 1 0 0 1 50 60 Tm (c) Tj ET");
    at(&c.glyphs[0], 5., 5.);
    at(&c.glyphs[1], 5., 10.);
    at(&c.glyphs[2], 50., 60.);
}

#[test]
fn tm_and_ctm_compose() {
    let c = text(b"2 0 0 2 0 0 cm BT /F1 10 Tf 1 0 0 1 5 5 Tm (a) Tj ET");
    at(&c.glyphs[0], 10., 10.);
    assert!(close(c.glyphs[0].advance, 11.12));
}

#[test]
fn bbox_uses_ascent_descent() {
    let c = text(b"BT /F1 10 Tf (a) Tj ET");
    let b = c.glyphs[0].bbox;
    let want = Rect {
        x0: 0.,
        y0: -2.,
        x1: 5.56,
        y1: 8.,
    };
    assert!(
        close(b.x0, want.x0)
            && close(b.y0, want.y0)
            && close(b.x1, want.x1)
            && close(b.y1, want.y1),
        "{b:?}"
    );
}

#[test]
fn render_mode_and_alpha_are_recorded() {
    let c = run(
        b"/G1 gs 1 0 0 RG BT /F1 10 Tf 3 Tr (a) Tj ET",
        "<< /Font << /F1 5 0 R >> /ExtGState << /G1 << /ca 0.5 >> >> >>",
        &[(5, HELV)],
    );
    let g = &c.glyphs[0];
    assert_eq!(g.render_mode, RenderMode::Invisible);
    assert_eq!(g.fill_alpha, 0.5);
    assert_eq!(g.stroke, Color::Rgb([1., 0., 0.]));
}

#[test]
fn text_state_is_saved_by_q() {
    let c = text(b"BT /F1 10 Tf q 20 Tf Q (a) Tj ET");
    assert_eq!(c.glyphs[0].size, 10.);
}

#[test]
fn text_without_font_uses_fallback() {
    let c = text(b"BT (a) Tj (b) Tj ET");
    assert_eq!(c.glyphs.len(), 2);
    assert_eq!(c.glyphs[0].text, "a");
    assert_eq!(kinds(&c), [DiagnosticKind::MissingResource]);
    let c = text(b"BT /Nope 10 Tf (a) Tj ET");
    assert_eq!(c.glyphs[0].text, "a");
    assert!(close(c.glyphs[0].advance, 5.56));
    assert_eq!(c.fonts[0].base_font, "Helvetica");
    assert_eq!(kinds(&c), [DiagnosticKind::MissingResource]);
}

#[test]
fn unmapped_reported_once_per_font() {
    let font: &[u8] = b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding << /Differences [1 /g1] >> >>";
    let c = run(
        b"BT /F1 10 Tf (\x01\x01\x01) Tj ET",
        "<< /Font << /F1 5 0 R >> >>",
        &[(5, font)],
    );
    assert_eq!(c.glyphs.len(), 3);
    assert!(c.glyphs.iter().all(|g| g.text == "\u{FFFD}"));
    assert_eq!(kinds(&c), [DiagnosticKind::UnmappedGlyph]);
}

#[test]
fn text_outside_bt_is_tolerated() {
    let c = text(b"/F1 10 Tf (a) Tj");
    assert_eq!(c.glyphs.len(), 1);
    assert_eq!(kinds(&c), [DiagnosticKind::MisplacedOperator]);
    let c = text(b"BT BT ET ET");
    assert_eq!(
        kinds(&c),
        [
            DiagnosticKind::MisplacedOperator,
            DiagnosticKind::UnbalancedRestore
        ]
    );
}

#[test]
fn gs_font_entry() {
    let c = run(
        b"/G1 gs BT (a) Tj ET",
        "<< /ExtGState << /G1 << /Font [5 0 R 12] >> >> >>",
        &[(5, HELV)],
    );
    assert_eq!(c.glyphs[0].size, 12.);
    assert!(c.diagnostics.is_empty(), "{:?}", c.diagnostics);
}

#[test]
fn same_font_gets_one_index() {
    let c = run(
        b"BT /F1 10 Tf (a) Tj /F2 10 Tf (b) Tj /F1 12 Tf (c) Tj ET",
        "<< /Font << /F1 5 0 R /F2 6 0 R >> >>",
        &[
            (5, HELV),
            (6, b"<< /Type /Font /Subtype /Type1 /BaseFont /Courier >>"),
        ],
    );
    let idx: Vec<usize> = c.glyphs.iter().map(|g| g.font).collect();
    assert_eq!(idx, [0, 1, 0]);
    assert_eq!(c.fonts.len(), 2);
}

#[test]
fn bad_text_operands_are_diagnosed() {
    let c = text(b"BT 9 Tr 10 Tf /F1 (x) Tf 1 (a) \" (a) TJ 5 Tj ET");
    assert_eq!(kinds(&c), [DiagnosticKind::BadOperands; 6]);
}

#[test]
fn font_diagnostics_do_not_depend_on_call_order() {
    let font: &[u8] = b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /Bogus >>";
    let doc = Document::from_bytes(one_page(
        b"BT /F1 10 Tf (a) Tj ET",
        "<< /Font << /F1 5 0 R >> >>",
        &[(5, font)],
    ))
    .unwrap();
    let page = doc.page(0).unwrap();
    let first = kinds(&page.content());
    assert_eq!(first, [DiagnosticKind::BadFont]);
    assert_eq!(kinds(&page.content()), first);
}

#[test]
fn direct_font_loads_once_per_pass() {
    let c = run(
        b"BT /F1 10 Tf (a) Tj /F1 12 Tf (b) Tj /F1 14 Tf (c) Tj ET",
        "<< /Font << /F1 << /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /Bogus >> >> >>",
        &[],
    );
    assert_eq!(kinds(&c), [DiagnosticKind::BadFont]);
    assert_eq!(c.fonts.len(), 1);
}
