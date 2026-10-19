mod common;

use common::{one_page, stream};
use pawdf_reader::{Document, PageContent, Rect};

const HELV: &[u8] = b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>";
const RES: &str = "<< /Font << /F1 5 0 R >> /XObject << /Fm0 6 0 R >> >>";

fn run(ops: &str, extra: &[(u32, &[u8])]) -> PageContent {
    let mut objects = vec![(5, HELV)];
    objects.extend_from_slice(extra);
    let doc = Document::from_bytes(one_page(ops.as_bytes(), RES, &objects)).unwrap();
    doc.page(0).unwrap().content()
}

fn content(ops: &str) -> PageContent {
    run(ops, &[])
}

fn r(x0: f64, y0: f64, x1: f64, y1: f64) -> Option<Rect> {
    Some(Rect { x0, y0, x1, y1 })
}

fn form(dict: &str, body: &str) -> Vec<u8> {
    stream(
        &format!("/Type /XObject /Subtype /Form /Resources << /Font << /F1 5 0 R >> >> {dict}"),
        body.as_bytes(),
    )
}

#[test]
fn clip_applies_after_paint() {
    let c = content("10 10 100 100 re W f 0 0 5 5 re f");
    assert_eq!(c.paths[0].clip, None);
    assert_eq!(c.paths[1].clip, r(10., 10., 110., 110.));
}

#[test]
fn w_n_clips() {
    let c = content("10 10 100 100 re W n 0 0 5 5 re f");
    assert_eq!(c.paths.len(), 1);
    assert_eq!(c.paths[0].clip, r(10., 10., 110., 110.));
}

#[test]
fn q_restores() {
    let c = content("q 0 0 10 10 re W n Q 0 0 1 1 re f");
    assert_eq!(c.paths[0].clip, None);
}

#[test]
fn nested_to_empty() {
    let c = content("0 0 10 10 re W n 20 20 10 10 re W n 0 0 1 1 re f");
    assert_eq!(c.paths[0].clip, r(20., 20., 20., 20.));
}

#[test]
fn even_odd() {
    let c = content("10 10 100 100 re W* n 0 0 5 5 re f");
    assert_eq!(c.paths[0].clip, r(10., 10., 110., 110.));
}

#[test]
fn curve_control_points() {
    let c = content("0 0 m 0 100 100 100 100 0 c W n 0 0 1 1 re f");
    assert_eq!(c.paths[0].clip, r(0., 0., 100., 100.));
}

#[test]
fn ctm_transformed() {
    let c = content("q 2 0 0 2 0 0 cm 0 0 10 10 re W n 0.5 0 0 0.5 0 0 cm 0 0 1 1 re f Q");
    assert_eq!(c.paths[0].clip, r(0., 0., 20., 20.));
}

#[test]
fn pending_until_paint() {
    let c = content("0 0 10 10 re W 20 20 1 1 re f 0 0 1 1 re f");
    assert_eq!(c.paths[0].clip, None);
    assert_eq!(c.paths[1].clip, r(0., 0., 21., 21.));
}

#[test]
fn empty_path_clip_ignored() {
    // pdfium's CPDF_StreamContentParser::AddPathObject drops a pending clip with no points.
    let c = content("W n 0 0 1 1 re f");
    assert_eq!(c.paths[0].clip, None);
}

#[test]
fn degenerate_ctm_clip() {
    let c =
        content("q 1e300 0 0 1e300 0 0 cm 1e300 0 0 1e300 0 0 cm 0 0 1 1 re W n Q 0 0 1 1 re f");
    let nan =
        |r: Option<Rect>| r.is_some_and(|r| [r.x0, r.y0, r.x1, r.y1].iter().any(|v| v.is_nan()));
    assert!(c.paths.iter().all(|p| !nan(p.clip)));
    let c =
        content("q 1e300 0 0 1e300 0 0 cm 1e300 0 0 1e300 0 0 cm 0 0 1 1 re W n 0 0 1 1 re f Q");
    assert!(c.paths.iter().all(|p| !nan(p.clip)));
    let c = content("q 0 0 0 0 0 0 cm 0 0 1 1 re W n 1 0 0 1 0 0 cm 0 0 1 1 re f Q");
    assert_eq!(c.paths[0].clip, r(0., 0., 0., 0.));
}

#[test]
fn text_clip() {
    let c = content("BT 7 Tr /F1 10 Tf 100 100 Td (AB) Tj ET 0 0 1 1 re f");
    let (a, b) = (c.glyphs[0].bbox, c.glyphs[1].bbox);
    let union = Rect {
        x0: a.x0.min(b.x0),
        y0: a.y0.min(b.y0),
        x1: a.x1.max(b.x1),
        y1: a.y1.max(b.y1),
    };
    assert_eq!(c.paths[0].clip, Some(union));
    assert!(c.glyphs.iter().all(|g| g.clip.is_none()));
}

#[test]
fn text_clip_spaces_only() {
    // §9.3.6: no clipping occurs when the only glyphs shown have no outlines.
    let c = content("BT 7 Tr /F1 10 Tf 100 100 Td ( ) Tj ET 0 0 1 1 re f");
    assert_eq!(c.paths[0].clip, None);
}

#[test]
fn unterminated_text_clip_does_not_leak() {
    let f = form("/BBox [0 0 1000 1000]", "BT 7 Tr /F1 10 Tf (A) Tj");
    let c = run("/Fm0 Do 0 0 1 1 re f", &[(6, &f)]);
    assert_eq!(c.paths[0].clip, None);
}

#[test]
fn form_bbox() {
    let f = form("/BBox [0 0 50 50] /Matrix [2 0 0 2 10 10]", "0 0 1 1 re f");
    let c = run("/Fm0 Do 0 0 1 1 re f", &[(6, &f)]);
    assert_eq!(c.paths[0].clip, r(10., 10., 110., 110.));
    assert_eq!(c.paths[1].clip, None);
}

#[test]
fn images_and_glyphs_record_clip() {
    let img = stream(
        "/Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8",
        b"\x00",
    );
    let c = run(
        "0 0 10 10 re W n BI /W 1 /H 1 /CS /G /BPC 8 ID \x00 EI /Fm0 Do BT /F1 10 Tf (a) Tj ET",
        &[(6, &img)],
    );
    let want = r(0., 0., 10., 10.);
    assert_eq!(c.images.len(), 2);
    assert!(c.images.iter().all(|i| i.clip == want));
    assert_eq!(c.glyphs[0].clip, want);
}
