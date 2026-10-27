mod common;

use common::build_pdf;
use pawdf_reader::{
    Diagnostic, DiagnosticKind, Document, FieldKind, FormField, FormFields, Location, Rect, Ref,
    Widget,
};

fn r(num: u32) -> Ref {
    Ref { num, generation: 0 }
}

fn bad(num: u32) -> Diagnostic {
    Diagnostic {
        kind: DiagnosticKind::BadFormField,
        location: Location::Object(r(num)),
    }
}

fn obj(num: u32, body: &str) -> (u32, String) {
    (num, body.to_string())
}

/// Catalog 1 (plus `catalog_extra`), pages 2, pages 3 and 4 (plus `page3` and `page4`), then
/// `objects`.
fn form(catalog_extra: &str, page3: &str, page4: &str, objects: &[(u32, String)]) -> FormFields {
    let catalog = format!("<< /Type /Catalog /Pages 2 0 R {catalog_extra} >>");
    let p3 = format!("<< /Type /Page /Parent 2 0 R {page3} >>");
    let p4 = format!("<< /Type /Page /Parent 2 0 R {page4} >>");
    let mut all: Vec<(u32, &[u8])> = vec![
        (1, catalog.as_bytes()),
        (2, b"<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>"),
        (3, p3.as_bytes()),
        (4, p4.as_bytes()),
    ];
    all.extend(objects.iter().map(|(n, b)| (*n, b.as_bytes())));
    let (pdf, _) = build_pdf(&all, "/Root 1 0 R");
    Document::from_bytes(pdf).unwrap().form_fields()
}

/// `/AcroForm << /Fields [roots] >>` with `objects`.
fn fields(roots: &str, objects: &[(u32, String)]) -> FormFields {
    form(
        &format!("/AcroForm << /Fields [{roots}] >>"),
        "",
        "",
        objects,
    )
}

/// One root field, object 10, with `body` inside its dictionary.
fn one(body: &str) -> FormFields {
    fields("10 0 R", &[obj(10, &format!("<< {body} >>"))])
}

fn names(f: &FormFields) -> Vec<&str> {
    f.items.iter().map(|i| i.name.as_str()).collect()
}

fn field(f: &FormFields) -> &FormField {
    assert_eq!(f.items.len(), 1, "{f:?}");
    &f.items[0]
}

#[test]
fn no_acroform_is_empty() {
    assert_eq!(form("", "", "", &[]), FormFields::default());
}

#[test]
fn acroform_not_dict() {
    let f = form("/AcroForm 5", "", "", &[]);
    assert!(f.items.is_empty());
    assert_eq!(f.diagnostics, [bad(1)]);
}

#[test]
fn fields_not_array() {
    let f = form("/AcroForm << /Fields 5 >>", "", "", &[]);
    assert!(f.items.is_empty());
    assert_eq!(f.diagnostics, [bad(1)]);
}

#[test]
fn qualified_names() {
    let f = fields(
        "10 0 R",
        &[
            obj(10, "<< /T (a) /Kids [11 0 R] >>"),
            obj(11, "<< /T (b) /Kids [12 0 R 13 0 R] >>"),
            obj(12, "<< /T (c) /FT /Tx >>"),
            obj(13, "<< /T (d) /FT /Tx >>"),
        ],
    );
    assert_eq!(names(&f), ["a.b.c", "a.b.d"]);
    assert_eq!(f.diagnostics, []);
}

#[test]
fn unnamed_level_adds_no_segment() {
    let f = fields(
        "10 0 R",
        &[
            obj(10, "<< /T (a) /Kids [11 0 R] >>"),
            obj(11, "<< /Kids [12 0 R] >>"),
            obj(12, "<< /T (c) /FT /Tx >>"),
        ],
    );
    assert_eq!(names(&f), ["a.c"]);
    assert_eq!(f.diagnostics, []);
}

#[test]
fn unnamed_root_is_empty_name() {
    let f = one("/FT /Tx");
    assert_eq!(names(&f), [""]);
    assert_eq!(f.diagnostics, []);
}

#[test]
fn kinds() {
    let bodies = [
        "/FT /Tx",
        "/FT /Sig",
        "/FT /Ch",
        "/FT /Ch /Ff 131072",
        "/FT /Btn",
        "/FT /Btn /Ff 32768",
        "/FT /Btn /Ff 65536",
        "",
    ];
    let objects: Vec<_> = (10..)
        .zip(bodies)
        .map(|(n, b)| obj(n, &format!("<< /T (k{n}) {b} >>")))
        .collect();
    let f = fields(
        "10 0 R 11 0 R 12 0 R 13 0 R 14 0 R 15 0 R 16 0 R 17 0 R",
        &objects,
    );
    let kinds: Vec<_> = f.items.iter().map(|i| i.kind).collect();
    assert_eq!(
        kinds,
        [
            FieldKind::Text,
            FieldKind::Signature,
            FieldKind::ListBox,
            FieldKind::ComboBox,
            FieldKind::CheckBox,
            FieldKind::RadioButton,
            FieldKind::PushButton,
            FieldKind::Unknown,
        ]
    );
    assert_eq!(f.diagnostics, [bad(17)]);
}

#[test]
fn unknown_ft_name() {
    let f = one("/FT /Xx");
    assert_eq!(field(&f).kind, FieldKind::Unknown);
    assert_eq!(f.diagnostics, [bad(10)]);
}

#[test]
fn ft_and_ff_inherited() {
    let f = fields(
        "10 0 R",
        &[
            obj(10, "<< /T (p) /FT /Btn /Ff 32768 /Kids [11 0 R] >>"),
            obj(11, "<< /T (x) >>"),
        ],
    );
    let x = field(&f);
    assert_eq!(
        (x.name.as_str(), x.kind, x.flags),
        ("p.x", FieldKind::RadioButton, 32768)
    );
    assert_eq!(f.diagnostics, []);
}

#[test]
fn label_from_tu() {
    let f = one("/T (x) /TU (Your name) /FT /Tx");
    assert_eq!(field(&f).label.as_deref(), Some("Your name"));
}

#[test]
fn utf16_names_decode() {
    let f = one("/T <FEFF540D> /TU <FEFF6C0F540D> /FT /Tx");
    let x = field(&f);
    assert_eq!(x.name, "名");
    assert_eq!(x.label.as_deref(), Some("氏名"));
}

#[test]
fn bad_ff_defaults_to_zero() {
    let f = one("/T (x) /FT /Tx /Ff (x)");
    assert_eq!(field(&f).flags, 0);
    assert_eq!(f.diagnostics, [bad(10)]);
}

#[test]
fn kids_cycle_is_cut() {
    let f = fields(
        "10 0 R",
        &[
            obj(10, "<< /T (a) /FT /Tx /Kids [11 0 R] >>"),
            obj(11, "<< /T (b) /Kids [10 0 R] >>"),
        ],
    );
    assert_eq!(names(&f), ["a.b"]);
    assert_eq!(f.diagnostics, [bad(11)]);
}

#[test]
fn kid_not_dict_skipped() {
    let f = one("/T (a) /FT /Tx /Kids [5]");
    assert_eq!(names(&f), ["a"]);
    assert_eq!(f.diagnostics, [bad(10)]);
}

#[test]
fn mixed_kids() {
    let f = fields(
        "10 0 R",
        &[
            obj(10, "<< /T (a) /FT /Tx /Kids [11 0 R 12 0 R] >>"),
            obj(11, "<< /T (b) >>"),
            obj(12, "<< /Subtype /Widget /Rect [0 0 1 1] >>"),
        ],
    );
    assert_eq!(names(&f), ["a", "a.b"]);
}

#[test]
fn duplicate_names_kept() {
    let f = fields(
        "10 0 R 11 0 R",
        &[
            obj(10, "<< /T (a) /FT /Tx >>"),
            obj(11, "<< /T (a) /FT /Tx >>"),
        ],
    );
    assert_eq!(names(&f), ["a", "a"]);
    assert_eq!(f.diagnostics, []);
}

#[test]
fn depth_is_capped() {
    let mut objects = Vec::new();
    for n in 10..100 {
        objects.push(obj(
            n,
            &format!("<< /T (x) /FT /Tx /Kids [{} 0 R] >>", n + 1),
        ));
    }
    objects.push(obj(100, "<< /T (x) /FT /Tx >>"));
    let f = fields("10 0 R", &objects);
    assert!(!f.diagnostics.is_empty());
    assert!(f.items.is_empty());
}

fn values(f: &FormFields) -> Vec<&str> {
    field(f).values.iter().map(String::as_str).collect()
}

#[test]
fn text_value() {
    let f = one("/T (x) /FT /Tx /V (hello)");
    assert_eq!(values(&f), ["hello"]);
    assert_eq!(f.diagnostics, []);
}

#[test]
fn unfilled_text() {
    let f = one("/T (x) /FT /Tx");
    assert!(values(&f).is_empty());
    assert_eq!(f.diagnostics, []);
}

#[test]
fn text_value_stream() {
    let f = fields(
        "10 0 R",
        &[
            obj(10, "<< /T (x) /FT /Tx /V 11 0 R >>"),
            obj(11, "<< /Length 5 >>\nstream\nhello\nendstream"),
        ],
    );
    assert_eq!(values(&f), ["hello"]);
    assert_eq!(f.diagnostics, []);
}

#[test]
fn text_value_wrong_type() {
    let f = one("/T (x) /FT /Tx /V 5");
    assert!(values(&f).is_empty());
    assert_eq!(f.diagnostics, [bad(10)]);
}

#[test]
fn password_returned() {
    let f = one("/T (x) /FT /Tx /Ff 8192 /V (secret)");
    assert_eq!(values(&f), ["secret"]);
}

#[test]
fn checkbox_on() {
    assert_eq!(values(&one("/T (x) /FT /Btn /V /Yes")), ["Yes"]);
}

#[test]
fn checkbox_off() {
    let f = one("/T (x) /FT /Btn /V /Off");
    assert!(values(&f).is_empty());
    assert_eq!(f.diagnostics, []);
}

#[test]
fn checkbox_opt_index() {
    assert_eq!(values(&one("/T (x) /FT /Btn /V /1 /Opt [(a) (b)]")), ["b"]);
}

#[test]
fn checkbox_opt_out_of_range() {
    assert_eq!(values(&one("/T (x) /FT /Btn /V /5 /Opt [(a)]")), ["5"]);
}

#[test]
fn radio_value_inherited_from_parent() {
    let f = fields(
        "10 0 R",
        &[
            obj(
                10,
                "<< /T (r) /FT /Btn /Ff 32768 /V /B /Kids [11 0 R 12 0 R] >>",
            ),
            obj(11, "<< /Subtype /Widget /Rect [0 0 1 1] /AS /B >>"),
            obj(12, "<< /Subtype /Widget /Rect [2 2 3 3] /AS /Off >>"),
        ],
    );
    assert_eq!(names(&f), ["r"]);
    assert_eq!(values(&f), ["B"]);
}

#[test]
fn choice_single() {
    assert_eq!(values(&one("/T (x) /FT /Ch /V (x)")), ["x"]);
}

#[test]
fn choice_multi() {
    let f = one("/T (x) /FT /Ch /Ff 2097152 /V [(x) 5 (y)]");
    assert_eq!(values(&f), ["x", "y"]);
    assert_eq!(f.diagnostics, [bad(10)]);
}

#[test]
fn choice_keeps_export_value() {
    assert_eq!(
        values(&one("/T (x) /FT /Ch /V (e1) /Opt [[(e1) (Shown)]]")),
        ["e1"]
    );
}

#[test]
fn push_and_sig_ignore_v() {
    let f = fields(
        "10 0 R 11 0 R",
        &[
            obj(10, "<< /T (p) /FT /Btn /Ff 65536 /V /X >>"),
            obj(11, "<< /T (s) /FT /Sig /V << >> >>"),
        ],
    );
    assert!(f.items.iter().all(|i| i.values.is_empty()));
    assert_eq!(f.diagnostics, []);
}

#[test]
fn dv_not_used() {
    assert!(values(&one("/T (x) /FT /Tx /DV (d)")).is_empty());
}

fn widget(page: Option<usize>, x0: f64, y0: f64, x1: f64, y1: f64) -> Widget {
    Widget {
        page,
        rect: Rect::from_corners(x0, y0, x1, y1),
    }
}

/// One root field, object 10, with `body`, and `/Annots` arrays for pages 3 and 4.
fn on_pages(page3: &str, page4: &str, body: &str) -> FormFields {
    form(
        "/AcroForm << /Fields [10 0 R] >>",
        page3,
        page4,
        &[obj(10, &format!("<< /T (a) /FT /Tx {body} >>"))],
    )
}

#[test]
fn merged_widget_page_from_annots() {
    let f = on_pages("", "/Annots [10 0 R]", "/Subtype /Widget /Rect [1 2 3 4]");
    assert_eq!(field(&f).widgets, [widget(Some(1), 1., 2., 3., 4.)]);
    assert_eq!(f.diagnostics, []);
}

#[test]
fn kid_widgets() {
    let f = form(
        "/AcroForm << /Fields [10 0 R] >>",
        "/Annots [11 0 R]",
        "/Annots [12 0 R]",
        &[
            obj(10, "<< /T (a) /FT /Btn /Ff 32768 /Kids [11 0 R 12 0 R] >>"),
            obj(11, "<< /Subtype /Widget /Rect [0 0 1 1] >>"),
            obj(12, "<< /Subtype /Widget /Rect [2 2 3 3] >>"),
        ],
    );
    assert_eq!(
        field(&f).widgets,
        [
            widget(Some(0), 0., 0., 1., 1.),
            widget(Some(1), 2., 2., 3., 3.)
        ]
    );
    assert_eq!(f.diagnostics, []);
}

#[test]
fn widget_on_two_pages_takes_first() {
    let f = on_pages(
        "/Annots [10 0 R]",
        "/Annots [10 0 R]",
        "/Subtype /Widget /Rect [0 0 1 1]",
    );
    assert_eq!(field(&f).widgets, [widget(Some(0), 0., 0., 1., 1.)]);
}

#[test]
fn p_fallback() {
    let f = on_pages("", "", "/Subtype /Widget /Rect [0 0 1 1] /P 4 0 R");
    assert_eq!(field(&f).widgets[0].page, Some(1));
    assert_eq!(f.diagnostics, [bad(10)]);
}

#[test]
fn no_page() {
    let f = on_pages("", "", "/Subtype /Widget /Rect [0 0 1 1]");
    assert_eq!(field(&f).widgets[0].page, None);
    assert_eq!(f.diagnostics, [bad(10)]);
}

#[test]
fn direct_widget_uses_p() {
    let f = on_pages(
        "",
        "",
        "/Kids [<< /Subtype /Widget /Rect [0 0 1 1] /P 3 0 R >>]",
    );
    assert_eq!(field(&f).widgets, [widget(Some(0), 0., 0., 1., 1.)]);
    assert_eq!(f.diagnostics, [bad(10)]);
}

#[test]
fn bad_rect() {
    let f = on_pages("/Annots [10 0 R]", "", "/Subtype /Widget");
    assert_eq!(field(&f).widgets, [widget(Some(0), 0., 0., 0., 0.)]);
    assert_eq!(f.diagnostics, [bad(10)]);
}

#[test]
fn no_widget_kids() {
    let f = fields(
        "10 0 R",
        &[
            obj(10, "<< /T (a) /Kids [11 0 R] >>"),
            obj(11, "<< /T (b) /FT /Tx /Kids [] >>"),
        ],
    );
    assert_eq!(names(&f), ["a.b"]);
    assert_eq!(field(&f).widgets, []);
    assert_eq!(f.diagnostics, []);
}

#[test]
fn field_without_widget_entries_has_no_widgets() {
    let f = one("/T (a) /FT /Tx");
    assert_eq!(field(&f).widgets, []);
    assert_eq!(f.diagnostics, []);
}
