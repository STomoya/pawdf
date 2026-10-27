mod common;

use common::{build_pdf, stream};
use pawdf_core::DiagnosticKind;
use pawdf_reader::{Document, PageContent};

const HELV: &[u8] = b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>";
const OCG: &[u8] = b"<< /Type /OCG /Name (L) >>";
/// Groups 10 and 12 are on, 11 is off.
const PROPS: &str = "<< /OCGs [10 0 R 11 0 R 12 0 R] /D << /OFF [11 0 R] >> >>";
const RES: &str = "/Font << /F1 5 0 R >> /Properties << /on 10 0 R /off 11 0 R /on2 12 0 R /m 20 0 R >> /XObject << /Fm 30 0 R /Im 31 0 R >>";

/// A one-page document with catalog `/OCProperties props` (none when empty), groups 10–12,
/// and `extra` objects.
fn doc(props: &str, content: &str, page_extra: &str, extra: &[(u32, Vec<u8>)]) -> Document {
    let catalog = if props.is_empty() {
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned()
    } else {
        format!("<< /Type /Catalog /Pages 2 0 R /OCProperties {props} >>")
    };
    let page = format!(
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Contents 4 0 R /Resources << {RES} >> {page_extra} >>"
    );
    let contents = stream("", content.as_bytes());
    let mut objects: Vec<(u32, &[u8])> = vec![
        (1, catalog.as_bytes()),
        (2, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>"),
        (3, page.as_bytes()),
        (4, &contents),
        (5, HELV),
        (10, OCG),
        (11, OCG),
        (12, OCG),
    ];
    objects.extend(extra.iter().map(|(n, b)| (*n, b.as_slice())));
    Document::from_bytes(build_pdf(&objects, "/Root 1 0 R").0).unwrap()
}

fn content(props: &str, ops: &str, extra: &[(u32, Vec<u8>)]) -> PageContent {
    doc(props, ops, "", extra).page(0).unwrap().content()
}

/// `BT /F1 10 Tf (s) Tj ET`.
fn t(s: &str) -> String {
    format!("BT /F1 10 Tf ({s}) Tj ET")
}

fn glyphs_hidden(c: &PageContent) -> Vec<bool> {
    c.glyphs.iter().map(|g| g.hidden).collect()
}

fn kinds(c: &PageContent) -> Vec<DiagnosticKind> {
    c.diagnostics.iter().map(|d| d.kind).collect()
}

fn obj(num: u32, body: &str) -> (u32, Vec<u8>) {
    (num, body.as_bytes().to_vec())
}

/// Whether a glyph shown under `/OC /m`, with object 20 as `ocmd`, is hidden, and the diagnostics.
fn under_ocmd(ocmd: &str, extra: &[(u32, Vec<u8>)]) -> (bool, Vec<DiagnosticKind>) {
    let mut objects = vec![obj(20, ocmd)];
    objects.extend_from_slice(extra);
    let c = content(PROPS, &format!("/OC /m BDC {} EMC", t("a")), &objects);
    (c.glyphs[0].hidden, kinds(&c))
}

#[test]
fn groups_off_in_default_config_hide_glyphs() {
    let c = content(
        PROPS,
        &format!(
            "/OC /on BDC {} EMC /OC /off BDC {} EMC {}",
            t("a"),
            t("b"),
            t("c")
        ),
        &[],
    );
    assert_eq!(glyphs_hidden(&c), [false, true, false]);
    assert_eq!(kinds(&c), []);
}

#[test]
fn nested_sequences_hide_when_any_is_off() {
    let c = content(
        PROPS,
        &format!(
            "/OC /off BDC /OC /on BDC /Span << >> BDC {} EMC EMC {} EMC {}",
            t("a"),
            t("b"),
            t("c")
        ),
        &[],
    );
    assert_eq!(glyphs_hidden(&c), [true, true, false]);
}

#[test]
fn base_state_off_with_on_list() {
    let props = "<< /OCGs [10 0 R 11 0 R 12 0 R] /D << /BaseState /OFF /ON [10 0 R] >> >>";
    let c = content(
        props,
        &format!(
            "/OC /on BDC {} EMC /OC /off BDC {} EMC /OC /on2 BDC {} EMC",
            t("a"),
            t("b"),
            t("c")
        ),
        &[],
    );
    assert_eq!(glyphs_hidden(&c), [false, true, true]);
    assert_eq!(kinds(&c), []);
}

#[test]
fn off_list_wins_over_on_list() {
    let props = "<< /OCGs [10 0 R 11 0 R 12 0 R] /D << /ON [11 0 R] /OFF [11 0 R] >> >>";
    let c = content(props, &format!("/OC /off BDC {} EMC", t("a")), &[]);
    assert_eq!(glyphs_hidden(&c), [true]);
}

#[test]
fn membership_policies() {
    // 10 on, 11 off.
    for (p, hidden) in [
        ("", false),
        ("/P /AnyOn", false),
        ("/P /AllOn", true),
        ("/P /AnyOff", false),
        ("/P /AllOff", true),
    ] {
        let ocmd = format!("<< /Type /OCMD /OCGs [10 0 R 11 0 R] {p} >>");
        assert_eq!(under_ocmd(&ocmd, &[]), (hidden, vec![]), "{p}");
    }
    for (p, hidden) in [
        ("/P /AnyOff", false),
        ("/P /AllOff", false),
        ("/P /AnyOn", true),
    ] {
        let ocmd = format!("<< /Type /OCMD /OCGs 11 0 R {p} >>");
        assert_eq!(under_ocmd(&ocmd, &[]).0, hidden, "single {p}");
    }
}

#[test]
fn untyped_membership_dictionary() {
    let got = under_ocmd("<< /OCGs [11 0 R] >>", &[]);
    assert_eq!(got, (true, vec![]));
}

#[test]
fn untyped_group_with_null_membership_keys() {
    let props = "<< /OCGs [20 0 R] /D << /OFF [20 0 R] >> >>";
    let c = content(
        props,
        &format!("/OC /m BDC {} EMC", t("a")),
        &[obj(20, "<< /Name (G) /OCGs null /VE null >>")],
    );
    assert_eq!(glyphs_hidden(&c), [true]);
    assert_eq!(kinds(&c), []);
}

#[test]
fn malformed_memberships_are_visible() {
    let got = under_ocmd("<< /Type /OCMD /OCGs 5 >>", &[]);
    assert_eq!(got, (false, vec![DiagnosticKind::BadOptionalContent]));
    // Not a dictionary: reported as a bad `/Properties` entry only.
    let got = under_ocmd("(x)", &[]);
    assert_eq!(got, (false, vec![DiagnosticKind::MissingResource]));
    // A group not in `/OCGs` has no effect, so inverting policies do not hide either.
    let extra = [obj(13, "<< /Type /OCG /Name (U) >>")];
    for ocmd in [
        "<< /Type /OCMD /OCGs [13 0 R] /P /AllOff >>",
        "<< /Type /OCMD /OCGs 13 0 R /P /AnyOff >>",
        "<< /Type /OCMD /VE [/Not 13 0 R] >>",
    ] {
        let want = (false, vec![DiagnosticKind::BadOptionalContent]);
        assert_eq!(under_ocmd(ocmd, &extra), want, "{ocmd}");
    }
    // An unknown policy falls back to the default `/AnyOn`.
    let got = under_ocmd("<< /Type /OCMD /OCGs [11 0 R] /P /Most >>", &[]);
    assert_eq!(got, (true, vec![DiagnosticKind::BadOptionalContent]));
}

#[test]
fn membership_without_groups_has_no_effect() {
    for ocmd in [
        "<< /Type /OCMD >>",
        "<< /Type /OCMD /OCGs [] /P /AllOff >>",
        "<< /Type /OCMD /OCGs [null 99 0 R] /P /AllOff >>",
    ] {
        assert_eq!(under_ocmd(ocmd, &[]), (false, vec![]), "{ocmd}");
    }
}

#[test]
fn visibility_expressions() {
    for (ve, hidden) in [
        ("11 0 R", true),
        ("[/Not 11 0 R]", false),
        ("[/And 10 0 R 12 0 R]", false),
        ("[/And 10 0 R 11 0 R]", true),
        ("[/Or 11 0 R 10 0 R]", false),
        ("[/Or 11 0 R [/Not 10 0 R]]", true),
        ("40 0 R", true),
        ("[/Or [/Not 13 0 R] 11 0 R]", true),
        ("[/And 10 0 R [/Not 13 0 R]]", false),
    ] {
        // `/VE` overrides `/OCGs` and `/P`.
        let ocmd = format!("<< /Type /OCMD /OCGs [11 0 R] /P /AllOff /VE {ve} >>");
        let extra = [
            obj(40, "[/And 10 0 R [/Not 12 0 R]]"),
            obj(13, "<< /Type /OCG /Name (U) >>"),
        ];
        let (got, kinds) = under_ocmd(&ocmd, &extra);
        assert_eq!(got, hidden, "{ve}");
        // 13 is not in `/OCGs`, so it is reported and left out.
        assert!(
            kinds
                .iter()
                .all(|&k| k == DiagnosticKind::BadOptionalContent),
            "{ve}"
        );
        assert_eq!(kinds.is_empty(), !ve.contains("13"), "{ve}");
    }
}

#[test]
fn malformed_expressions_are_visible() {
    for ve in [
        "[]",
        "[/Not 10 0 R 11 0 R]",
        "[/And]",
        "[/Xor 11 0 R]",
        "[/Not 5]",
        "(x)",
    ] {
        let ocmd = format!("<< /Type /OCMD /VE {ve} >>");
        let want = (false, vec![DiagnosticKind::BadOptionalContent]);
        assert_eq!(under_ocmd(&ocmd, &[]), want, "{ve}");
    }
}

#[test]
fn self_referencing_expression_hits_depth_limit() {
    let ocmd = "<< /Type /OCMD /VE 40 0 R >>";
    let extra = [obj(40, "[/Not 40 0 R]")];
    assert_eq!(
        under_ocmd(ocmd, &extra),
        (false, vec![DiagnosticKind::RecursionLimit])
    );
}

#[test]
fn depth_limit_is_retried_at_a_shallower_use() {
    // 80 is first reached at depth 31 under /m, where its operand exceeds the limit.
    let mut extra: Vec<(u32, Vec<u8>)> = (40..70)
        .map(|n| obj(n, &format!("[/And {} 0 R]", n + 1)))
        .collect();
    extra.push(obj(70, "[/And 80 0 R]"));
    extra.push(obj(80, "[/Not [/Not 11 0 R]]"));
    extra.push(obj(20, "<< /Type /OCMD /VE 40 0 R >>"));
    let c = content(
        PROPS,
        &format!(
            "/OC /m BDC {} EMC /OC << /Type /OCMD /VE 80 0 R >> BDC {} EMC",
            t("a"),
            t("b")
        ),
        &extra,
    );
    assert_eq!(glyphs_hidden(&c), [false, true]);
    assert_eq!(kinds(&c), [DiagnosticKind::RecursionLimit]);
}

#[test]
fn shared_subexpressions_are_evaluated_once() {
    // Each level references the next twice: 2^31 nodes when walked as a tree.
    let mut extra: Vec<(u32, Vec<u8>)> = (40..70)
        .map(|n| obj(n, &format!("[/And {0} 0 R {0} 0 R]", n + 1)))
        .collect();
    extra.push(obj(70, "[/Not 10 0 R]"));
    let shown = format!("/OC /m BDC {} EMC ", t("a")).repeat(1000);
    let c = content(
        PROPS,
        &format!("{shown}/OC /off BDC {} EMC", t("b")),
        &[[obj(20, "<< /Type /OCMD /VE 40 0 R >>")].as_slice(), &extra].concat(),
    );
    assert_eq!(glyphs_hidden(&c), [true; 1001]);
    assert_eq!(kinds(&c), []);
}

#[test]
fn intents_outside_the_configuration_are_ignored() {
    let groups = [
        obj(21, "<< /Type /OCG /Name (D) /Intent /Design >>"),
        obj(22, "<< /Type /OCG /Name (V) /Intent [/View /Design] >>"),
    ];
    let ops = format!(
        "/OC << /OCGs 21 0 R >> BDC {} EMC /OC << /OCGs 22 0 R >> BDC {} EMC",
        t("a"),
        t("b")
    );
    for (d, want) in [
        ("/OFF [21 0 R 22 0 R]", [false, true]),
        ("/OFF [21 0 R 22 0 R] /Intent /Design", [true, true]),
        ("/OFF [21 0 R 22 0 R] /Intent /All", [true, true]),
        ("/OFF [21 0 R 22 0 R] /Intent [/Other]", [false, false]),
    ] {
        let props = format!("<< /OCGs [21 0 R 22 0 R] /D << {d} >> >>");
        let c = content(&props, &ops, &groups);
        assert_eq!(glyphs_hidden(&c), want, "{d}");
        assert_eq!(kinds(&c), [], "{d}");
    }
    // An ignored group has no effect, so inverting policies do not hide either.
    let props = "<< /OCGs [21 0 R 22 0 R] /D << /OFF [21 0 R] >> >>";
    let ops = format!(
        "/OC << /OCGs [21 0 R] /P /AllOff >> BDC {} EMC /OC << /VE [/Not 21 0 R] >> BDC {} EMC",
        t("a"),
        t("b")
    );
    let c = content(props, &ops, &groups);
    assert_eq!(glyphs_hidden(&c), [false, false]);
    assert_eq!(kinds(&c), []);
}

#[test]
fn null_entries_in_the_configuration_are_deleted_groups() {
    let props = "<< /OCGs [10 0 R 11 0 R 12 0 R null] /D << /ON [null] /OFF [11 0 R null] >> >>";
    let c = content(props, &format!("/OC /off BDC {} EMC", t("a")), &[]);
    assert_eq!(glyphs_hidden(&c), [true]);
    assert_eq!(kinds(&c), []);
}

#[test]
fn null_oc_on_an_xobject_is_absent() {
    let image = stream(
        "/Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8 /OC null",
        b"\x00",
    );
    let c = content(PROPS, "/Im Do", &[(31, image)]);
    assert_eq!(
        c.images.iter().map(|i| i.hidden).collect::<Vec<_>>(),
        [false]
    );
    assert_eq!(kinds(&c), []);
}

#[test]
fn unknown_groups_are_visible_and_reported() {
    let c = content(
        PROPS,
        &format!("/OC /m BDC {} EMC", t("a")),
        &[obj(20, "<< /Type /OCG >>")],
    );
    assert_eq!(glyphs_hidden(&c), [false]);
    assert_eq!(kinds(&c), [DiagnosticKind::BadOptionalContent]);

    // Without `/OCProperties` no group is known.
    let c = content("", &format!("/OC /off BDC {} EMC", t("a")), &[]);
    assert_eq!(glyphs_hidden(&c), [false]);
    assert_eq!(kinds(&c), [DiagnosticKind::BadOptionalContent]);
}

#[test]
fn malformed_properties_are_reported_once_per_pass() {
    let bad = DiagnosticKind::BadOptionalContent;
    // Not a dictionary: no group is known either, so each lookup is reported too.
    let d = doc("5", &format!("/OC /on BDC {} EMC", t("a")), "", &[]);
    assert_eq!(kinds(&d.page(0).unwrap().content()), [bad, bad]);
    for props in [
        "<< /OCGs [10 0 R 11 0 R] >>",
        "<< /OCGs [10 0 R 11 0 R 12 0 R] /D << /OFF (11) >> >>",
        "<< /OCGs [10 0 R 11 0 R 12 0 R] /D << /BaseState /Maybe /OFF [11 0 R] >> >>",
        "<< /OCGs [10 0 R 11 0 R 12 0 R 7] /D << /OFF [11 0 R] >> >>",
        "<< /OCGs [10 0 R 11 0 R 12 0 R] /D << /OFF [11 0 R 13 0 R] >> >>",
        "<< /OCGs [10 0 R 11 0 R 12 0 R] /D << /OFF [11 0 R] /Intent 5 >> >>",
        "<< /OCGs [10 0 R 11 0 R 12 0 R] /D << /OFF [11 0 R] /Intent [/View 5] >> >>",
    ] {
        let d = doc(
            props,
            &format!("/OC /on BDC {} EMC /OC /on BDC {} EMC", t("a"), t("b")),
            "",
            &[],
        );
        for _ in 0..2 {
            let c = d.page(0).unwrap().content();
            assert_eq!(kinds(&c), [DiagnosticKind::BadOptionalContent], "{props}");
        }
    }
}

#[test]
fn documents_without_optional_content_are_silent() {
    let c = content("", &t("a"), &[]);
    assert_eq!(glyphs_hidden(&c), [false]);
    assert_eq!(kinds(&c), []);
}

#[test]
fn xobjects_with_oc() {
    let form = stream(
        "/Type /XObject /Subtype /Form /BBox [0 0 1000 1000] /OC 11 0 R /Resources << /Font << /F1 5 0 R >> >>",
        b"BT /F1 10 Tf (f) Tj ET 0 0 1 1 re f",
    );
    let image = stream(
        "/Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8 /OC 20 0 R",
        b"\x00",
    );
    let extra = [
        (30, form),
        (31, image),
        obj(20, "<< /Type /OCMD /OCGs [11 0 R] /P /AllOff >>"),
    ];
    let c = content(
        PROPS,
        &format!("/Fm Do /Im Do {} 0 0 1 1 re f", t("a")),
        &extra,
    );
    assert_eq!(glyphs_hidden(&c), [true, false]);
    assert_eq!(
        c.paths.iter().map(|p| p.hidden).collect::<Vec<_>>(),
        [true, false]
    );
    assert_eq!(
        c.images.iter().map(|i| i.hidden).collect::<Vec<_>>(),
        [false]
    );
    assert_eq!(kinds(&c), []);
}

#[test]
fn hidden_sequence_around_a_form_hides_its_content() {
    let form = stream(
        "/Type /XObject /Subtype /Form /BBox [0 0 1000 1000] /Resources << /Font << /F1 5 0 R >> >>",
        b"BT /F1 10 Tf (f) Tj ET",
    );
    let image = stream(
        "/Type /XObject /Subtype /Image /Width 1 /Height 1 /ColorSpace /DeviceGray /BitsPerComponent 8",
        b"\x00",
    );
    let c = content(
        PROPS,
        &format!("/OC /off BDC /Fm Do /Im Do EMC /Fm Do {}", t("a")),
        &[(30, form), (31, image)],
    );
    assert_eq!(glyphs_hidden(&c), [true, false, false]);
    assert_eq!(
        c.images.iter().map(|i| i.hidden).collect::<Vec<_>>(),
        [true]
    );
}

#[test]
fn unclosed_hidden_sequence_in_a_form_ends_with_it() {
    let form = stream(
        "/Type /XObject /Subtype /Form /BBox [0 0 1000 1000] /Resources << /Font << /F1 5 0 R >> /Properties << /off 11 0 R >> >>",
        b"/OC /off BDC BT /F1 10 Tf (f) Tj ET",
    );
    let c = content(PROPS, &format!("/Fm Do {}", t("a")), &[(30, form)]);
    assert_eq!(glyphs_hidden(&c), [true, false]);
    assert_eq!(kinds(&c), [DiagnosticKind::UnbalancedMarkedContent]);
}

#[test]
fn sequences_past_the_depth_limit_still_hide() {
    let open = "/Span BMC ".repeat(256);
    let close = "EMC ".repeat(256);
    let c = content(
        PROPS,
        &format!(
            "{open}/OC /off BDC {} EMC {} {close}{}",
            t("a"),
            t("b"),
            t("c")
        ),
        &[],
    );
    assert_eq!(glyphs_hidden(&c), [true, false, false]);
    assert_eq!(kinds(&c), [DiagnosticKind::RecursionLimit]);
}

#[test]
fn bad_oc_properties_past_the_depth_limit_are_reported() {
    let open = "/Span BMC ".repeat(256);
    let close = "EMC ".repeat(256);
    let c = content(
        PROPS,
        &format!("{open}/OC /missing BDC EMC /OC 5 BDC EMC {close}"),
        &[],
    );
    assert_eq!(
        kinds(&c),
        [
            DiagnosticKind::MissingResource,
            DiagnosticKind::RecursionLimit,
            DiagnosticKind::BadOperands
        ]
    );
}

#[test]
fn nested_hiding_sequences_past_the_depth_limit() {
    let open = "/Span BMC ".repeat(256);
    let close = "EMC ".repeat(256);
    let c = content(
        PROPS,
        &format!(
            "{open}/OC /on BDC /OC /off BDC /OC /off BDC {} EMC {} EMC {} EMC {} {close}",
            t("a"),
            t("b"),
            t("c"),
            t("d")
        ),
        &[],
    );
    assert_eq!(glyphs_hidden(&c), [true, true, false, false]);
}

#[test]
fn inline_membership_dictionary() {
    let c = content(
        PROPS,
        &format!("/OC << /Type /OCMD /OCGs [11 0 R] >> BDC {} EMC", t("a")),
        &[],
    );
    assert_eq!(glyphs_hidden(&c), [true]);
    assert_eq!(kinds(&c), []);
}

#[test]
fn annotations_with_oc() {
    let d = doc(
        PROPS,
        "",
        "/Annots [50 0 R 51 0 R 52 0 R 53 0 R]",
        &[
            obj(50, "<< /Subtype /Text /Rect [0 0 1 1] /OC 11 0 R >>"),
            obj(51, "<< /Subtype /Text /Rect [0 0 1 1] /OC 10 0 R >>"),
            obj(52, "<< /Subtype /Text /Rect [0 0 1 1] >>"),
            obj(53, "<< /Subtype /Text /Rect [0 0 1 1] /OC (x) >>"),
        ],
    );
    let a = d.page(0).unwrap().annotations();
    assert_eq!(
        a.items.iter().map(|a| a.hidden).collect::<Vec<_>>(),
        [true, false, false, false]
    );
    let kinds: Vec<_> = a.diagnostics.iter().map(|d| d.kind).collect();
    assert_eq!(kinds, [DiagnosticKind::BadOptionalContent]);
}
