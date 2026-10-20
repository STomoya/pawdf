mod common;

use common::build_pdf;
use pawdf_reader::{Diagnostic, DiagnosticKind, Document, Location, Outline, OutlineItem, Ref};

fn r(num: u32) -> Ref {
    Ref { num, generation: 0 }
}

fn bad(num: u32) -> Diagnostic {
    Diagnostic {
        kind: DiagnosticKind::BadOutline,
        location: Location::Object(r(num)),
    }
}

/// Catalog 1 (plus `catalog_extra`), pages 2, pages 3 and 4, then `objects`.
fn outline(catalog_extra: &str, objects: &[(u32, String)]) -> Outline {
    let catalog = format!("<< /Type /Catalog /Pages 2 0 R {catalog_extra} >>");
    let mut all: Vec<(u32, &[u8])> = vec![
        (1, catalog.as_bytes()),
        (2, b"<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>"),
        (3, b"<< /Type /Page /Parent 2 0 R >>"),
        (4, b"<< /Type /Page /Parent 2 0 R >>"),
    ];
    all.extend(objects.iter().map(|(n, b)| (*n, b.as_bytes())));
    let (pdf, _) = build_pdf(&all, "/Root 1 0 R");
    Document::from_bytes(pdf).unwrap().outline()
}

fn obj(num: u32, body: &str) -> (u32, String) {
    (num, body.to_string())
}

fn titles(items: &[OutlineItem]) -> Vec<&str> {
    items.iter().map(|i| i.title.as_str()).collect()
}

#[test]
fn no_outline_is_empty() {
    assert_eq!(outline("", &[]), Outline::default());
}

#[test]
fn nested_items_in_order() {
    let o = outline(
        "/Outlines 10 0 R",
        &[
            obj(10, "<< /First 11 0 R >>"),
            obj(11, "<< /Title (A) /Next 12 0 R >>"),
            obj(12, "<< /Title (B) /First 13 0 R >>"),
            obj(13, "<< /Title (B1) /Next 14 0 R >>"),
            obj(14, "<< /Title (B2) >>"),
        ],
    );
    assert_eq!(o.diagnostics, []);
    assert_eq!(titles(&o.items), ["A", "B"]);
    assert_eq!(titles(&o.items[1].children), ["B1", "B2"]);
    assert!(o.items[0].children.is_empty());
    assert!(o.items.iter().all(|i| i.dest.is_none()));
}

#[test]
fn last_prev_count_are_ignored() {
    let o = outline(
        "/Outlines 10 0 R",
        &[
            obj(10, "<< /First 11 0 R /Last 11 0 R /Count 99 >>"),
            obj(11, "<< /Title (A) /Next 12 0 R /Prev 12 0 R >>"),
            obj(12, "<< /Title (B) /Prev 99 0 R >>"),
        ],
    );
    assert_eq!(o.diagnostics, []);
    assert_eq!(titles(&o.items), ["A", "B"]);
}

#[test]
fn utf16_title() {
    let o = outline(
        "/Outlines 10 0 R",
        &[
            obj(10, "<< /First 11 0 R >>"),
            obj(11, "<< /Title <FEFF30C630B930C8> >>"),
        ],
    );
    assert_eq!(titles(&o.items), ["テスト"]);
}

#[test]
fn missing_title_is_empty_with_diagnostic() {
    let o = outline(
        "/Outlines 10 0 R",
        &[
            obj(10, "<< /First 11 0 R >>"),
            obj(11, "<< /Next 12 0 R >>"),
            obj(12, "<< /Title 5 >>"),
        ],
    );
    assert_eq!(titles(&o.items), ["", ""]);
    assert_eq!(o.diagnostics, [bad(11), bad(12)]);
}

#[test]
fn next_cycle_is_cut() {
    let o = outline(
        "/Outlines 10 0 R",
        &[
            obj(10, "<< /First 11 0 R >>"),
            obj(11, "<< /Title (A) /Next 12 0 R >>"),
            obj(12, "<< /Title (B) /Next 11 0 R >>"),
        ],
    );
    assert_eq!(titles(&o.items), ["A", "B"]);
    assert_eq!(o.diagnostics, [bad(12)]);
}

#[test]
fn shared_child_is_cut() {
    let o = outline(
        "/Outlines 10 0 R",
        &[
            obj(10, "<< /First 11 0 R >>"),
            obj(11, "<< /Title (A) /First 13 0 R /Next 12 0 R >>"),
            obj(12, "<< /Title (B) /First 13 0 R >>"),
            obj(13, "<< /Title (C) >>"),
        ],
    );
    assert_eq!(titles(&o.items[0].children), ["C"]);
    assert!(o.items[1].children.is_empty());
    assert_eq!(o.diagnostics, [bad(12)]);
}

#[test]
fn depth_is_capped() {
    let mut objects = vec![obj(10, "<< /First 11 0 R >>")];
    for n in 11..81u32 {
        objects.push((n, format!("<< /Title (x) /First {} 0 R >>", n + 1)));
    }
    objects.push(obj(81, "<< /Title (x) >>"));
    let o = outline("/Outlines 10 0 R", &objects);
    let mut count = 0;
    let mut items = &o.items;
    while let Some(first) = items.first() {
        count += 1;
        items = &first.children;
    }
    assert_eq!(count, 65);
    assert_eq!(o.diagnostics.len(), 1);
    assert_eq!(o.diagnostics[0].kind, DiagnosticKind::BadOutline);
}

#[test]
fn long_flat_chain_is_iterative() {
    const N: u32 = 100_000;
    let mut objects = vec![obj(10, "<< /First 11 0 R >>")];
    for n in 11..11 + N {
        let next = if n + 1 < 11 + N {
            format!("/Next {} 0 R", n + 1)
        } else {
            String::new()
        };
        objects.push((n, format!("<< /Title (x) {next} >>")));
    }
    let o = outline("/Outlines 10 0 R", &objects);
    assert_eq!(o.items.len(), N as usize);
    assert_eq!(o.diagnostics, []);
}

#[test]
fn non_dict_root_and_links() {
    let o = outline("/Outlines 5", &[]);
    assert!(o.items.is_empty());
    assert_eq!(o.diagnostics, [bad(1)]);

    let o = outline(
        "/Outlines 10 0 R",
        &[
            obj(10, "<< /First 11 0 R >>"),
            obj(11, "<< /Title (A) /Next 7 >>"),
        ],
    );
    assert_eq!(titles(&o.items), ["A"]);
    assert_eq!(o.diagnostics, [bad(11)]);
}
