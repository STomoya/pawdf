mod common;

use common::{one_page, stream};
use pawdf_core::DiagnosticKind;
use pawdf_reader::{
    CcittParams, ColorSpace, Document, ImageData, Matrix, PageContent, Point, Rect, Segment,
    SoftMask,
};

fn run(ops: &[u8], resources: &str, extra: &[(u32, &[u8])]) -> PageContent {
    Document::from_bytes(one_page(ops, resources, extra))
        .unwrap()
        .page(0)
        .unwrap()
        .content()
}

fn kinds(c: &PageContent) -> Vec<DiagnosticKind> {
    c.diagnostics.iter().map(|d| d.kind).collect()
}

fn with_image(dict: &str, data: &[u8]) -> PageContent {
    let im = stream(&format!("/Type /XObject /Subtype /Image {dict}"), data);
    run(b"/Im1 Do", "<< /XObject << /Im1 5 0 R >> >>", &[(5, &im)])
}

#[test]
fn image_xobject_samples() {
    let im = stream(
        "/Type /XObject /Subtype /Image /Width 2 /Height 1 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /ASCIIHexDecode",
        b"010203040506>",
    );
    let c = run(
        b"q 20 0 0 10 5 5 cm /Im1 Do Q",
        "<< /XObject << /Im1 5 0 R >> >>",
        &[(5, &im)],
    );
    assert!(c.diagnostics.is_empty(), "{:?}", c.diagnostics);
    let i = &c.images[0];
    assert_eq!(i.name.as_deref(), Some("Im1"));
    assert!(!i.inline && !i.image_mask);
    assert_eq!((i.width, i.height, i.bpc), (2, 1, 8));
    assert_eq!(i.color_space, Some(ColorSpace::Rgb));
    assert_eq!(i.data, ImageData::Samples(vec![1, 2, 3, 4, 5, 6]));
    assert_eq!(
        i.ctm,
        Matrix {
            a: 20.,
            b: 0.,
            c: 0.,
            d: 10.,
            e: 5.,
            f: 5.
        }
    );
    assert_eq!(
        i.bbox,
        Rect {
            x0: 5.,
            y0: 5.,
            x1: 25.,
            y1: 15.
        }
    );
    assert_eq!((i.decode.clone(), i.smask.clone()), (None, None));
}

#[test]
fn dct_passes_through() {
    let c = with_image(
        "/Width 1 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8 /Filter /DCTDecode",
        b"\xff\xd8junk",
    );
    assert_eq!(c.images[0].data, ImageData::Jpeg(b"\xff\xd8junk".to_vec()));
    assert!(c.diagnostics.is_empty());
}

#[test]
fn jbig2_globals_and_ccitt_params() {
    let im = stream(
        "/Type /XObject /Subtype /Image /Width 8 /Height 1 /ImageMask true /Filter /JBIG2Decode /DecodeParms << /JBIG2Globals 6 0 R >>",
        b"PAGE",
    );
    let globals = stream("", b"GLOB");
    let c = run(
        b"/Im1 Do",
        "<< /XObject << /Im1 5 0 R >> >>",
        &[(5, &im), (6, &globals)],
    );
    assert_eq!(
        c.images[0].data,
        ImageData::Jbig2 {
            data: b"PAGE".to_vec(),
            globals: Some(b"GLOB".to_vec())
        }
    );
    assert_eq!(
        (c.images[0].bpc, c.images[0].color_space.clone()),
        (1, None)
    );
    let c = with_image(
        "/Width 8 /Height 1 /ImageMask true /Filter /CCITTFaxDecode /DecodeParms << /K -1 /Columns 8 /BlackIs1 true >>",
        b"FAX",
    );
    assert_eq!(
        c.images[0].data,
        ImageData::Ccitt {
            data: b"FAX".to_vec(),
            params: CcittParams {
                k: -1,
                columns: 8,
                rows: 0,
                black_is_1: true,
                encoded_byte_align: false,
                end_of_line: false,
                end_of_block: true,
            }
        }
    );
}

#[test]
fn indexed_and_smask() {
    let im = stream(
        "/Type /XObject /Subtype /Image /Width 2 /Height 1 /BitsPerComponent 8 /ColorSpace [/Indexed /DeviceRGB 1 <ff000000ff00>] /SMask 6 0 R /Decode [0 1]",
        b"\x00\x01",
    );
    let mask = stream(
        "/Type /XObject /Subtype /Image /Width 2 /Height 1 /BitsPerComponent 8 /ColorSpace /DeviceGray",
        b"\x00\xff",
    );
    let c = run(
        b"/Im1 Do",
        "<< /XObject << /Im1 5 0 R >> >>",
        &[(5, &im), (6, &mask)],
    );
    let i = &c.images[0];
    assert_eq!(
        i.color_space,
        Some(ColorSpace::Indexed {
            base: Box::new(ColorSpace::Rgb),
            hival: 1,
            lookup: vec![0xff, 0, 0, 0, 0xff, 0]
        })
    );
    assert_eq!(i.decode, Some(vec![0., 1.]));
    assert_eq!(
        i.smask,
        Some(SoftMask {
            width: 2,
            height: 1,
            bpc: 8,
            decode: None,
            data: ImageData::Samples(vec![0, 255])
        })
    );
}

#[test]
fn image_color_space_from_resources() {
    let im = stream(
        "/Type /XObject /Subtype /Image /Width 1 /Height 1 /BitsPerComponent 8 /ColorSpace /CS0",
        b"\x00\x00\x00\x00",
    );
    let c = run(
        b"/Im1 Do",
        "<< /XObject << /Im1 5 0 R >> /ColorSpace << /CS0 /DeviceCMYK >> >>",
        &[(5, &im)],
    );
    assert_eq!(c.images[0].color_space, Some(ColorSpace::Cmyk));
}

#[test]
fn inline_image() {
    let c = run(
        b"q 10 0 0 10 0 0 cm BI /W 2 /H 2 /CS /G /BPC 8 ID \x00\x01\x02\x03 EI Q",
        "<< >>",
        &[],
    );
    assert!(c.diagnostics.is_empty(), "{:?}", c.diagnostics);
    let i = &c.images[0];
    assert!(i.inline);
    assert_eq!(i.name, None);
    assert_eq!(i.color_space, Some(ColorSpace::Gray));
    assert_eq!(i.data, ImageData::Samples(vec![0, 1, 2, 3]));
    assert_eq!(
        i.bbox,
        Rect {
            x0: 0.,
            y0: 0.,
            x1: 10.,
            y1: 10.
        }
    );
}

#[test]
fn inline_abbreviations() {
    let c = run(
        b"BI /W 1 /H 1 /CS /RGB /BPC 8 /F /AHx ID 0a0b0c> EI",
        "<< >>",
        &[],
    );
    assert_eq!(c.images[0].color_space, Some(ColorSpace::Rgb));
    assert_eq!(c.images[0].data, ImageData::Samples(vec![10, 11, 12]));
    let c = run(b"BI /W 8 /H 1 /IM true ID \x00 EI", "<< >>", &[]);
    assert!(c.images[0].image_mask);
    assert_eq!(
        (c.images[0].bpc, c.images[0].color_space.clone()),
        (1, None)
    );
}

#[test]
fn short_samples_are_diagnosed() {
    let c = with_image(
        "/Width 4 /Height 4 /ColorSpace /DeviceGray /BitsPerComponent 8",
        b"abc",
    );
    assert_eq!(c.images[0].data, ImageData::Samples(b"abc".to_vec()));
    assert_eq!(kinds(&c), [DiagnosticKind::BadImage]);
}

#[test]
fn huge_dimensions_do_not_allocate() {
    let c = with_image(
        "/Width 4294967295 /Height 4294967295 /ColorSpace /DeviceRGB /BitsPerComponent 16",
        b"x",
    );
    assert!(c.images.is_empty());
    assert_eq!(kinds(&c), [DiagnosticKind::BadImage]);
    let c = with_image(
        "/Width -1 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8",
        b"x",
    );
    assert!(c.images.is_empty());
    assert_eq!(kinds(&c), [DiagnosticKind::BadImage]);
}

#[test]
fn missing_xobject_is_diagnosed() {
    let c = run(b"/Nope Do", "<< >>", &[]);
    assert_eq!(kinds(&c), [DiagnosticKind::MissingResource]);
}

fn form(dict: &str, content: &[u8]) -> Vec<u8> {
    stream(
        &format!("/Type /XObject /Subtype /Form /BBox [0 0 100 100] {dict}"),
        content,
    )
}

#[test]
fn form_xobject() {
    let f = form("/Matrix [1 0 0 1 100 0]", b"0 0 m 10 0 l S");
    let c = run(
        b"/Fm1 Do 0 0 m 1 1 l S",
        "<< /XObject << /Fm1 5 0 R >> >>",
        &[(5, &f)],
    );
    let p = |x, y| Point { x, y };
    assert_eq!(
        c.paths[0].segments,
        [Segment::MoveTo(p(100., 0.)), Segment::LineTo(p(110., 0.))]
    );
    assert_eq!(c.paths[1].segments[1], Segment::LineTo(p(1., 1.)));
    assert!(c.diagnostics.is_empty(), "{:?}", c.diagnostics);
}

#[test]
fn form_q_cannot_pop_page_state() {
    let f = form("", b"Q Q");
    let c = run(
        b"q 0.5 g /Fm1 Do 0 0 m 1 1 l f Q",
        "<< /XObject << /Fm1 5 0 R >> >>",
        &[(5, &f)],
    );
    assert_eq!(c.paths[0].fill_color, pawdf_reader::Color::Gray(0.5));
    assert_eq!(kinds(&c), [DiagnosticKind::UnbalancedRestore; 2]);
}

#[test]
fn form_falls_back_to_parent_resources() {
    let f = form("", b"BT /F1 10 Tf (a) Tj ET");
    let helv: &[u8] = b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>";
    let c = run(
        b"/Fm1 Do",
        "<< /XObject << /Fm1 5 0 R >> /Font << /F1 6 0 R >> >>",
        &[(5, &f), (6, helv)],
    );
    assert_eq!(c.glyphs[0].text, "a");
    assert!(c.diagnostics.is_empty(), "{:?}", c.diagnostics);
}

#[test]
fn self_referencing_form_is_cut() {
    let f = form(
        "/Resources << /XObject << /Fm1 5 0 R >> >>",
        b"0 0 m 1 1 l S /Fm1 Do",
    );
    let c = run(b"/Fm1 Do", "<< /XObject << /Fm1 5 0 R >> >>", &[(5, &f)]);
    assert_eq!(c.paths.len(), 1);
    assert_eq!(kinds(&c), [DiagnosticKind::RecursionLimit]);
}

#[test]
fn image_drawn_twice_yields_two_images() {
    let f = form("/Resources << /XObject << /Im1 6 0 R >> >>", b"/Im1 Do");
    let im = stream(
        "/Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8",
        b"\x80",
    );
    let c = run(
        b"/Fm1 Do 1 0 0 1 50 0 cm /Fm1 Do",
        "<< /XObject << /Fm1 5 0 R >> >>",
        &[(5, &f), (6, &im)],
    );
    assert_eq!(c.images.len(), 2);
    assert_eq!((c.images[0].ctm.e, c.images[1].ctm.e), (0., 50.));
}

#[test]
fn bad_image_parameters_are_diagnosed() {
    let c = with_image(
        "/Width 1 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 3",
        b"\x00",
    );
    assert_eq!(c.images[0].bpc, 8);
    assert_eq!(kinds(&c), [DiagnosticKind::BadImage]);
    let c = with_image(
        "/Width 1 /Height 1 /ColorSpace /Nope /BitsPerComponent 8",
        b"\x00",
    );
    assert_eq!(c.images[0].color_space, None);
    assert_eq!(kinds(&c), [DiagnosticKind::MissingResource]);
    let c = with_image("/Width 1 /Height 1 /Filter /JPXDecode", b"jp2");
    assert_eq!(
        (c.images[0].bpc, c.images[0].data.clone()),
        (0, ImageData::Jpx(b"jp2".to_vec()))
    );
    assert!(c.diagnostics.is_empty());
}
