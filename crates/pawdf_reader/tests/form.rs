mod common;

use common::build_pdf;
use pawdf_reader::{
    Diagnostic, DiagnosticKind, Document, FieldKind, FormField, FormFields, Location, Ref,
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
