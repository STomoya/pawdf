mod common;

use common::{CATALOG, build_pdf, one_page, stream};
use pawdf_core::DiagnosticKind;
use pawdf_reader::{Document, MarkedContent, PageContent};

const HELV: &[u8] = b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>";
const RES: &str = "<< /Font << /F1 5 0 R >> >>";

fn run(ops: &[u8], resources: &str, extra: &[(u32, &[u8])]) -> PageContent {
    let mut objects = vec![(5, HELV)];
    objects.extend_from_slice(extra);
    let doc = Document::from_bytes(one_page(ops, resources, &objects)).unwrap();
    doc.page(0).unwrap().content()
}

fn content(ops: &str) -> PageContent {
    run(ops.as_bytes(), RES, &[])
}

/// `BT /F1 10 Tf (s) Tj ET`.
fn t(s: &str) -> String {
    format!("BT /F1 10 Tf ({s}) Tj ET")
}

fn kinds(c: &PageContent) -> Vec<DiagnosticKind> {
    c.diagnostics.iter().map(|d| d.kind).collect()
}

fn span(tag: &str, glyphs: std::ops::Range<usize>) -> MarkedContent {
    MarkedContent {
        tag: tag.into(),
        mcid: None,
        actual_text: None,
        artifact_type: None,
        artifact_subtype: None,
        glyphs,
        paths: 0..0,
        images: 0..0,
    }
}

fn form(body: &str) -> Vec<u8> {
    stream(
        "/Type /XObject /Subtype /Form /BBox [0 0 1000 1000] /Resources << /Font << /F1 5 0 R >> >>",
        body.as_bytes(),
    )
}

const FORM_RES: &str = "<< /Font << /F1 5 0 R >> /XObject << /Fm0 6 0 R >> >>";

#[test]
fn inline_actual_text() {
    let c = content(&format!("/Span << /ActualText (fi) >> BDC {} EMC", t("ab")));
    let want = MarkedContent {
        actual_text: Some("fi".into()),
        ..span("Span", 0..2)
    };
    assert_eq!(c.marked_contents, [want]);
    assert!(c.diagnostics.is_empty());
}

#[test]
fn properties_name() {
    let ops = format!("/P /P0 BDC {} EMC", t("a"));
    let c = run(
        ops.as_bytes(),
        "<< /Font << /F1 5 0 R >> /Properties << /P0 6 0 R >> >>",
        &[(6, b"<< /MCID 3 >>")],
    );
    let want = MarkedContent {
        mcid: Some(3),
        ..span("P", 0..1)
    };
    assert_eq!(c.marked_contents, [want]);
    assert!(c.diagnostics.is_empty());
}

#[test]
fn indirect_values() {
    let c = run(
        b"/Span << /ActualText 7 0 R >> BDC EMC",
        RES,
        &[(7, b"(x)")],
    );
    assert_eq!(c.marked_contents[0].actual_text.as_deref(), Some("x"));
}

#[test]
fn text_string_encodings() {
    for (value, want) in [("<FEFF0041>", "A"), ("<EFBBBFC3A9>", "é"), ("(\\223)", "ﬁ")] {
        let c = content(&format!("/Span << /ActualText {value} >> BDC EMC"));
        assert_eq!(c.marked_contents[0].actual_text.as_deref(), Some(want));
    }
}

#[test]
fn artifact_type_and_subtype() {
    let c = content("/Artifact << /Type /Pagination /Subtype /Footer >> BDC 0 0 1 1 re f EMC");
    let want = MarkedContent {
        artifact_type: Some("Pagination".into()),
        artifact_subtype: Some("Footer".into()),
        paths: 0..1,
        ..span("Artifact", 0..0)
    };
    assert_eq!(c.marked_contents, [want]);
    let c = content("/Span << /Type /Pagination /Subtype /Footer >> BDC EMC");
    assert_eq!(c.marked_contents, [span("Span", 0..0)]);
}

#[test]
fn ranges_cover_images() {
    let c = content("/Figure BMC BI /W 1 /H 1 /CS /G /BPC 8 ID \x00 EI EMC");
    let want = MarkedContent {
        images: 0..1,
        ..span("Figure", 0..0)
    };
    assert_eq!(c.marked_contents, [want]);
}

#[test]
fn nesting_order_and_inner_actual_text_dropped() {
    let c = content(&format!(
        "/A << /ActualText (x) >> BDC /B << /ActualText (y) >> BDC {} EMC {} EMC",
        t("a"),
        t("b")
    ));
    let a = MarkedContent {
        actual_text: Some("x".into()),
        ..span("A", 0..2)
    };
    assert_eq!(c.marked_contents, [a, span("B", 0..1)]);
}

#[test]
fn plain_bmc() {
    let c = content(&format!("/Foo BMC {} EMC", t("a")));
    assert_eq!(c.marked_contents, [span("Foo", 0..1)]);
}

#[test]
fn empty_span() {
    let c = content(&format!("{} /Span << /ActualText (x) >> BDC EMC", t("a")));
    let want = MarkedContent {
        actual_text: Some("x".into()),
        ..span("Span", 1..1)
    };
    assert_eq!(c.marked_contents, [want]);
}

#[test]
fn stray_emc() {
    let c = content("EMC");
    assert_eq!(kinds(&c), [DiagnosticKind::UnbalancedMarkedContent]);
    assert!(c.marked_contents.is_empty());
}

#[test]
fn unclosed_at_end() {
    let c = content(&format!("/Span << /ActualText (x) >> BDC {}", t("a")));
    let want = MarkedContent {
        actual_text: Some("x".into()),
        ..span("Span", 0..1)
    };
    assert_eq!(c.marked_contents, [want]);
    assert_eq!(kinds(&c), [DiagnosticKind::UnbalancedMarkedContent]);
}

#[test]
fn sequence_crosses_contents_array() {
    let page = format!(
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents [4 0 R 6 0 R] /Resources {RES} >>"
    );
    let first = stream("", format!("/S BMC {}", t("a")).as_bytes());
    let second = stream("", format!("{} EMC", t("b")).as_bytes());
    let (pdf, _) = build_pdf(
        &[
            (1, CATALOG),
            (2, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>"),
            (3, page.as_bytes()),
            (4, &first),
            (5, HELV),
            (6, &second),
        ],
        "/Root 1 0 R",
    );
    let c = Document::from_bytes(pdf)
        .unwrap()
        .page(0)
        .unwrap()
        .content();
    assert_eq!(c.marked_contents, [span("S", 0..2)]);
    assert!(c.diagnostics.is_empty());
}

#[test]
fn form_has_own_stack() {
    let f = form(&format!("EMC /F BMC {}", t("a")));
    let ops = format!("/S BMC /Fm0 Do {} EMC", t("c"));
    let c = run(ops.as_bytes(), FORM_RES, &[(6, &f)]);
    assert_eq!(c.marked_contents, [span("S", 0..2), span("F", 0..1)]);
    assert_eq!(
        kinds(&c),
        [
            DiagnosticKind::UnbalancedMarkedContent,
            DiagnosticKind::UnbalancedMarkedContent
        ]
    );
}

#[test]
fn actual_text_in_form_dropped_under_outer() {
    let f = form(&format!("/I << /ActualText (y) >> BDC {} EMC", t("a")));
    let c = run(
        b"/S << /ActualText (x) >> BDC /Fm0 Do EMC",
        FORM_RES,
        &[(6, &f)],
    );
    assert_eq!(c.marked_contents[1].tag, "I");
    assert_eq!(c.marked_contents[1].actual_text, None);
    assert_eq!(c.marked_contents[0].actual_text.as_deref(), Some("x"));
}

#[test]
fn bad_values() {
    let c = content("/Span << /ActualText 5 /MCID -1 >> BDC EMC");
    assert_eq!(
        kinds(&c),
        [DiagnosticKind::BadOperands, DiagnosticKind::BadOperands]
    );
    assert_eq!(c.marked_contents, [span("Span", 0..0)]);
}

#[test]
fn missing_properties() {
    let c = content("/P /Nope BDC EMC");
    assert_eq!(kinds(&c), [DiagnosticKind::MissingResource]);
    assert_eq!(c.marked_contents, [span("P", 0..0)]);
}

#[test]
fn bad_tag_still_balances() {
    let c = content("5 BMC EMC");
    assert_eq!(kinds(&c), [DiagnosticKind::BadOperands]);
    assert_eq!(c.marked_contents, [span("", 0..0)]);
}

#[test]
fn depth_cap() {
    let c = content(&format!("{}{}", "/A BMC ".repeat(300), "EMC ".repeat(300)));
    assert_eq!(c.marked_contents.len(), 256);
    assert_eq!(kinds(&c), [DiagnosticKind::RecursionLimit]);
}

#[test]
fn marked_content_budget() {
    let f = form(&"/A BMC EMC ".repeat(1024));
    let c = run("/Fm0 Do ".repeat(1025).as_bytes(), FORM_RES, &[(6, &f)]);
    assert_eq!(c.marked_contents.len(), 1 << 20);
    assert_eq!(kinds(&c), [DiagnosticKind::RecursionLimit]);
}
