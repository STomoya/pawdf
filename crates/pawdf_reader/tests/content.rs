mod common;

use common::{CATALOG, build_pdf, one_page, stream};
use pawdf_core::{DiagnosticKind, Location, Ref};
use pawdf_reader::Segment::{Close, CurveTo, LineTo, MoveTo};
use pawdf_reader::{Color, ColorSpaceFamily, Document, FillRule, Matrix, PageContent, Point};

fn content_with(ops: &[u8], resources: &str, extra: &[(u32, &[u8])]) -> PageContent {
    let doc = Document::from_bytes(one_page(ops, resources, extra)).unwrap();
    doc.page(0).unwrap().content()
}

fn content(ops: &[u8]) -> PageContent {
    content_with(ops, "<< >>", &[])
}

fn pt(x: f64, y: f64) -> Point {
    Point { x, y }
}

fn kinds(c: &PageContent) -> Vec<DiagnosticKind> {
    c.diagnostics.iter().map(|d| d.kind).collect()
}

#[test]
fn rectangle_fill() {
    let c = content(b"1 0 0 rg 10 20 30 40 re f");
    assert_eq!(c.paths.len(), 1);
    let p = &c.paths[0];
    assert_eq!(
        p.segments,
        [
            MoveTo(pt(10., 20.)),
            LineTo(pt(40., 20.)),
            LineTo(pt(40., 60.)),
            LineTo(pt(10., 60.)),
            Close
        ]
    );
    assert_eq!((p.fill, p.stroke), (Some(FillRule::NonZero), false));
    assert_eq!(p.fill_color, Color::Rgb([1., 0., 0.]));
    assert_eq!(p.stroke_color, Color::Gray(0.));
    assert_eq!((p.fill_alpha, p.stroke_alpha), (1., 1.));
    assert!(c.diagnostics.is_empty());
}

#[test]
fn segments_are_in_user_space() {
    let c = content(b"2 0 0 2 5 5 cm 0 0 m 1 1 l S");
    let p = &c.paths[0];
    assert_eq!(p.segments, [MoveTo(pt(5., 5.)), LineTo(pt(7., 7.))]);
    assert_eq!(
        p.ctm,
        Matrix {
            a: 2.,
            b: 0.,
            c: 0.,
            d: 2.,
            e: 5.,
            f: 5.
        }
    );
    assert_eq!(p.line_width, 1.);
}

#[test]
fn painting_operators() {
    let paint = |op: &str| {
        let c = content(format!("0 0 m 1 0 l 1 1 l {op}").as_bytes());
        let p = &c.paths[0];
        (p.fill, p.stroke, p.segments.last() == Some(&Close))
    };
    assert_eq!(paint("S"), (None, true, false));
    assert_eq!(paint("s"), (None, true, true));
    assert_eq!(paint("f"), (Some(FillRule::NonZero), false, false));
    assert_eq!(paint("F"), (Some(FillRule::NonZero), false, false));
    assert_eq!(paint("f*"), (Some(FillRule::EvenOdd), false, false));
    assert_eq!(paint("B"), (Some(FillRule::NonZero), true, false));
    assert_eq!(paint("B*"), (Some(FillRule::EvenOdd), true, false));
    assert_eq!(paint("b"), (Some(FillRule::NonZero), true, true));
    assert_eq!(paint("b*"), (Some(FillRule::EvenOdd), true, true));
    let c = content(b"0 0 m 1 1 l n 0 0 m 1 1 l W n");
    assert!(c.paths.is_empty());
    assert!(c.diagnostics.is_empty());
}

#[test]
fn curves_v_and_y() {
    let c = content(b"0 0 m 1 1 2 2 3 3 c 4 4 5 5 v 6 6 7 7 y S");
    assert_eq!(
        c.paths[0].segments[1..],
        [
            CurveTo(pt(1., 1.), pt(2., 2.), pt(3., 3.)),
            CurveTo(pt(3., 3.), pt(4., 4.), pt(5., 5.)),
            CurveTo(pt(6., 6.), pt(7., 7.), pt(7., 7.)),
        ]
    );
}

#[test]
fn path_without_current_point_is_dropped() {
    let c = content(b"1 1 l 0 0 m 1 0 l S");
    assert_eq!(c.paths[0].segments.len(), 2);
    assert_eq!(kinds(&c), [DiagnosticKind::BadOperands]);
}

#[test]
fn q_restores_state_and_unbalanced_q_is_diagnosed() {
    let c = content(b"q 0.5 g Q 0 0 m 1 1 l f Q");
    assert_eq!(c.paths[0].fill_color, Color::Gray(0.));
    assert_eq!(kinds(&c), [DiagnosticKind::UnbalancedRestore]);
}

#[test]
fn q_depth_is_capped() {
    let ops = "q ".repeat(300) + "0 0 m 1 1 l S";
    let c = content(ops.as_bytes());
    assert_eq!(c.paths.len(), 1);
    assert_eq!(kinds(&c), [DiagnosticKind::RecursionLimit]);
}

#[test]
fn gs_sets_alpha_and_line_width() {
    let c = content_with(
        b"/G1 gs 0 0 m 1 1 l B /Nope gs",
        "<< /ExtGState << /G1 << /ca 0.5 /CA 0.25 /LW 3 >> >> >>",
        &[],
    );
    let p = &c.paths[0];
    assert_eq!(
        (p.fill_alpha, p.stroke_alpha, p.line_width),
        (0.5, 0.25, 3.)
    );
    assert_eq!(kinds(&c), [DiagnosticKind::MissingResource]);
}

#[test]
fn color_spaces() {
    let fill = |ops: &[u8], res: &str, extra: &[(u32, &[u8])]| {
        let c = content_with(&[ops, b" 0 0 m 1 1 l f"].concat(), res, extra);
        assert!(c.diagnostics.is_empty(), "{:?}", c.diagnostics);
        c.paths[0].fill_color.clone()
    };
    assert_eq!(
        fill(b"0 0 0 1 k", "<< >>", &[]),
        Color::Cmyk([0., 0., 0., 1.])
    );
    let icc = stream("/N 1", b"");
    assert_eq!(
        fill(
            b"/CS0 cs 0.3 sc",
            "<< /ColorSpace << /CS0 [/ICCBased 5 0 R] >> >>",
            &[(5, &icc)]
        ),
        Color::Gray(0.3)
    );
    assert_eq!(
        fill(b"/Pattern cs /P1 scn", "<< >>", &[]),
        Color::Pattern("P1".into())
    );
    let tint = b"<< /FunctionType 2 /Domain [0 1] /C0 [0 0 0 0] /C1 [1 0 0 0] /N 1 >>";
    assert_eq!(
        fill(
            b"/CS1 cs 1 scn",
            "<< /ColorSpace << /CS1 [/Separation /Spot /DeviceCMYK 6 0 R] >> >>",
            &[(6, tint)]
        ),
        Color::Other {
            family: ColorSpaceFamily::Separation,
            components: vec![1.]
        }
    );
    assert_eq!(
        fill(b"/DeviceRGB cs", "<< >>", &[]),
        Color::Rgb([0., 0., 0.])
    );
    assert_eq!(
        fill(b"/DeviceCMYK cs", "<< >>", &[]),
        Color::Cmyk([0., 0., 0., 1.])
    );
}

#[test]
fn stroke_color_operators() {
    let c = content(b"0.2 G 0 0 m 1 1 l S 1 0 0 RG 0 0 m 1 1 l S 0 0 0 1 K 0 0 m 1 1 l S");
    let colors: Vec<_> = c.paths.iter().map(|p| p.stroke_color.clone()).collect();
    assert_eq!(
        colors,
        [
            Color::Gray(0.2),
            Color::Rgb([1., 0., 0.]),
            Color::Cmyk([0., 0., 0., 1.])
        ]
    );
}

#[test]
fn missing_color_space_is_diagnosed() {
    let c = content(b"/Nope cs 0 0 m 1 1 l f");
    assert_eq!(c.paths[0].fill_color, Color::Gray(0.));
    assert_eq!(kinds(&c), [DiagnosticKind::MissingResource]);
}

#[test]
fn bad_operands_skip_operator() {
    let c = content(b"1 0 0 rg 10 20 re f");
    assert!(c.paths.is_empty());
    assert_eq!(kinds(&c), [DiagnosticKind::BadOperands]);
}

#[test]
fn unknown_operator_outside_bx_only() {
    let c = content(b"foo BX bar EX");
    assert_eq!(kinds(&c), [DiagnosticKind::UnknownOperator]);
}

#[test]
fn operands_carry_across_content_streams() {
    let s4 = stream("", b"10 20 30 40");
    let s5 = stream("", b"re f");
    let (pdf, _) = build_pdf(
        &[
            (1, CATALOG),
            (2, b"<< /Type /Pages /Kids [3 0 R] >>"),
            (
                3,
                b"<< /Type /Page /MediaBox [0 0 1 1] /Contents [4 0 R 5 0 R] >>",
            ),
            (4, &s4),
            (5, &s5),
        ],
        "/Root 1 0 R",
    );
    let c = Document::from_bytes(pdf)
        .unwrap()
        .page(0)
        .unwrap()
        .content();
    assert_eq!(c.paths.len(), 1);
    assert!(c.diagnostics.is_empty(), "{:?}", c.diagnostics);
}

#[test]
fn bad_content_stream_is_skipped() {
    let s4 = stream("/Filter /Bogus", b"junk");
    let s5 = stream("", b"0 0 m 1 1 l S");
    let (pdf, _) = build_pdf(
        &[
            (1, CATALOG),
            (2, b"<< /Type /Pages /Kids [3 0 R] >>"),
            (
                3,
                b"<< /Type /Page /MediaBox [0 0 1 1] /Contents [4 0 R 5 0 R] >>",
            ),
            (4, &s4),
            (5, &s5),
        ],
        "/Root 1 0 R",
    );
    let c = Document::from_bytes(pdf)
        .unwrap()
        .page(0)
        .unwrap()
        .content();
    assert_eq!(c.paths.len(), 1);
    assert_eq!(kinds(&c), [DiagnosticKind::UnknownFilter]);
}

#[test]
fn diagnostics_point_into_the_stream() {
    let c = content(b"0 0 m Q");
    assert_eq!(
        c.diagnostics[0].location,
        Location::Stream {
            obj: Ref {
                num: 4,
                generation: 0
            },
            offset: 6
        }
    );
}

#[test]
fn page_without_contents_is_empty() {
    let (pdf, _) = build_pdf(
        &[
            (1, CATALOG),
            (2, b"<< /Type /Pages /Kids [3 0 R] >>"),
            (3, b"<< /Type /Page /MediaBox [0 0 1 1] >>"),
        ],
        "/Root 1 0 R",
    );
    let c = Document::from_bytes(pdf)
        .unwrap()
        .page(0)
        .unwrap()
        .content();
    assert_eq!(c, PageContent::default());
}

#[test]
fn unusable_content_streams_are_diagnosed() {
    let dct = stream("/Filter /DCTDecode", b"xx");
    let (pdf, _) = build_pdf(
        &[
            (1, CATALOG),
            (2, b"<< /Type /Pages /Kids [3 0 R] >>"),
            (
                3,
                b"<< /Type /Page /MediaBox [0 0 1 1] /Contents [4 0 R 5 0 R] >>",
            ),
            (4, b"<< /NotAStream true >>"),
            (5, &dct),
        ],
        "/Root 1 0 R",
    );
    let c = Document::from_bytes(pdf)
        .unwrap()
        .page(0)
        .unwrap()
        .content();
    assert_eq!(
        kinds(&c),
        [DiagnosticKind::BadObject, DiagnosticKind::UnknownFilter]
    );
}

#[test]
fn other_color_space_families() {
    let fill = |res: &str, ops: &[u8], extra: &[(u32, &[u8])]| {
        let c = content_with(&[ops, b" 0 0 m 1 1 l f"].concat(), res, extra);
        assert!(c.diagnostics.is_empty(), "{:?}", c.diagnostics);
        c.paths[0].fill_color.clone()
    };
    let other = |family, components: Vec<f32>| Color::Other { family, components };
    assert_eq!(
        fill(
            "<< /ColorSpace << /C [/CalRGB << >>] >> >>",
            b"/C cs 0.1 0.2 0.3 sc",
            &[]
        ),
        other(ColorSpaceFamily::CalRgb, vec![0.1, 0.2, 0.3])
    );
    assert_eq!(
        fill("<< /ColorSpace << /C [/CalGray << >>] >> >>", b"/C cs", &[]),
        other(ColorSpaceFamily::CalGray, vec![0.])
    );
    assert_eq!(
        fill(
            "<< /ColorSpace << /C [/Lab << >>] >> >>",
            b"/C cs 50 0 0 sc",
            &[]
        ),
        other(ColorSpaceFamily::Lab, vec![50., 0., 0.])
    );
    let tint = b"<< /FunctionType 2 /Domain [0 1] /C0 [0] /C1 [1] /N 1 >>";
    assert_eq!(
        fill(
            "<< /ColorSpace << /C [/DeviceN [/A /B] /DeviceGray 5 0 R] >> >>",
            b"/C cs 1 0.5 scn",
            &[(5, tint)]
        ),
        other(ColorSpaceFamily::DeviceN, vec![1., 0.5])
    );
    // §8.6.6.4, §8.6.6.5: selecting the space alone gives full tint.
    assert_eq!(
        fill(
            "<< /ColorSpace << /C [/DeviceN [/A /B] /DeviceGray 5 0 R] >> >>",
            b"/C cs",
            &[(5, tint)]
        ),
        other(ColorSpaceFamily::DeviceN, vec![1., 1.])
    );
    assert_eq!(
        fill(
            "<< /ColorSpace << /C [/Separation /Spot /DeviceGray 5 0 R] >> >>",
            b"/C cs",
            &[(5, tint)]
        ),
        other(ColorSpaceFamily::Separation, vec![1.])
    );
    let icc2 = stream("/N 2", b"");
    assert_eq!(
        fill(
            "<< /ColorSpace << /C [/ICCBased 5 0 R] >> >>",
            b"/C cs 0 1 sc",
            &[(5, &icc2)]
        ),
        other(ColorSpaceFamily::Unknown, vec![0., 1.])
    );
    let lookup = stream("", b"\x00\x00\x00\xff\xff\xff");
    assert_eq!(
        fill(
            "<< /ColorSpace << /C [/Indexed /DeviceRGB 1 5 0 R] >> >>",
            b"/C cs 1 sc",
            &[(5, &lookup)]
        ),
        other(ColorSpaceFamily::Indexed, vec![1.])
    );
}

#[test]
fn bad_color_space_is_missing_resource() {
    let c = content_with(b"/C cs", "<< /ColorSpace << /C [/Bogus] >> >>", &[]);
    assert_eq!(kinds(&c), [DiagnosticKind::MissingResource]);
}

#[test]
fn leftover_operands_are_diagnosed() {
    assert_eq!(kinds(&content(b"0 0 m 1 2")), [DiagnosticKind::BadOperands]);
    assert_eq!(kinds(&content(b"BX 1 2")), [DiagnosticKind::BadOperands]);
}

#[test]
fn non_array_contents_is_diagnosed() {
    let (pdf, _) = build_pdf(
        &[
            (1, CATALOG),
            (2, b"<< /Type /Pages /Kids [3 0 R] >>"),
            (3, b"<< /Type /Page /MediaBox [0 0 1 1] /Contents 5 >>"),
        ],
        "/Root 1 0 R",
    );
    let c = Document::from_bytes(pdf)
        .unwrap()
        .page(0)
        .unwrap()
        .content();
    assert_eq!(kinds(&c), [DiagnosticKind::BadPageTree]);
    assert_eq!(
        c.diagnostics[0].location,
        Location::Object(Ref {
            num: 3,
            generation: 0
        })
    );
}

#[test]
fn non_reference_contents_entry_is_diagnosed() {
    let s4 = stream("", b"0 0 m 1 1 l S");
    let (pdf, _) = build_pdf(
        &[
            (1, CATALOG),
            (2, b"<< /Type /Pages /Kids [3 0 R] >>"),
            (
                3,
                b"<< /Type /Page /MediaBox [0 0 1 1] /Contents [4 0 R 5] >>",
            ),
            (4, &s4),
        ],
        "/Root 1 0 R",
    );
    let c = Document::from_bytes(pdf)
        .unwrap()
        .page(0)
        .unwrap()
        .content();
    assert_eq!(c.paths.len(), 1);
    assert_eq!(kinds(&c), [DiagnosticKind::BadPageTree]);
    assert_eq!(
        c.diagnostics[0].location,
        Location::Object(Ref {
            num: 3,
            generation: 0
        })
    );
}

#[test]
fn bad_form_matrix_is_diagnosed() {
    let form = stream(
        "/Type /XObject /Subtype /Form /Matrix [1 0 0]",
        b"0 0 m 1 1 l S",
    );
    let c = content_with(b"/X Do", "<< /XObject << /X 5 0 R >> >>", &[(5, &form)]);
    assert_eq!(c.paths.len(), 1);
    assert_eq!(kinds(&c), [DiagnosticKind::BadObject]);
}

#[test]
fn unknown_xobject_subtype_is_diagnosed() {
    let ps = stream("/Type /XObject /Subtype /PS", b"");
    let c = content_with(b"/X Do", "<< /XObject << /X 5 0 R >> >>", &[(5, &ps)]);
    assert_eq!(kinds(&c), [DiagnosticKind::BadObject]);
}

#[test]
fn out_of_range_hival_is_diagnosed() {
    let c = content_with(
        b"/C cs 0 sc",
        "<< /ColorSpace << /C [/Indexed /DeviceGray 300 5 0 R] >> >>",
        &[(5, &stream("", &[0u8; 256]))],
    );
    assert_eq!(kinds(&c), [DiagnosticKind::BadObject]);
}

#[test]
fn pattern_tint_operands_are_checked() {
    let c = content_with(b"/Pattern cs /X /P1 scn", "<< >>", &[]);
    assert_eq!(kinds(&c), [DiagnosticKind::BadOperands]);
}

#[test]
fn indexed_over_pattern_or_indexed_is_rejected() {
    for base in ["/Pattern", "[/Indexed /DeviceGray 0 <00>]"] {
        let res = format!("<< /ColorSpace << /C [/Indexed {base} 0 <00>] >> >>");
        let c = content_with(b"/C cs", &res, &[]);
        assert_eq!(
            kinds(&c),
            [DiagnosticKind::BadObject, DiagnosticKind::MissingResource],
            "{base}"
        );
    }
}

#[test]
fn icc_without_n_uses_alternate() {
    let icc = stream("/Alternate /DeviceRGB", b"");
    let c = content_with(
        b"/C cs 0 1 0 sc 0 0 m 1 1 l f",
        "<< /ColorSpace << /C [/ICCBased 5 0 R] >> >>",
        &[(5, &icc)],
    );
    assert_eq!(kinds(&c), [DiagnosticKind::BadObject]);
    assert_eq!(c.paths[0].fill_color, Color::Rgb([0., 1., 0.]));
}

#[test]
fn q_past_depth_cap_does_not_unbalance_q() {
    let ops = "q ".repeat(300) + "0.5 g " + &"Q ".repeat(300) + "0 0 m 1 1 l f";
    let c = content(ops.as_bytes());
    assert_eq!(c.paths[0].fill_color, Color::Gray(0.));
    assert_eq!(kinds(&c), [DiagnosticKind::RecursionLimit]);
}

#[test]
fn path_segments_are_capped() {
    let ops = "0 0 1 1 re ".repeat(500_000) + "f";
    let c = content(ops.as_bytes());
    let segments: usize = c.paths.iter().map(|p| p.segments.len()).sum();
    assert!(
        (1 << 21) - 5 < segments && segments <= 1 << 21,
        "{segments}"
    );
    assert_eq!(kinds(&c), [DiagnosticKind::RecursionLimit]);
}

#[test]
fn carried_operands_are_capped() {
    let ints = "1 ".repeat(4096);
    let s4 = stream("", ints.as_bytes());
    let (pdf, _) = build_pdf(
        &[
            (1, CATALOG),
            (2, b"<< /Type /Pages /Kids [3 0 R] >>"),
            (
                3,
                b"<< /Type /Page /MediaBox [0 0 1 1] /Contents [4 0 R 4 0 R 4 0 R] >>",
            ),
            (4, &s4),
        ],
        "/Root 1 0 R",
    );
    let c = Document::from_bytes(pdf)
        .unwrap()
        .page(0)
        .unwrap()
        .content();
    assert_eq!(
        kinds(&c),
        [DiagnosticKind::BadObject, DiagnosticKind::BadOperands]
    );
}
