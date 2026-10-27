mod common;

use common::build_pdf;
use pawdf_reader::{
    Annotation, Annotations, Destination, Diagnostic, DiagnosticKind, Document, Location, Point,
    Rect, Ref,
};

fn r(num: u32) -> Ref {
    Ref { num, generation: 0 }
}

fn bad(num: u32) -> Diagnostic {
    Diagnostic {
        kind: DiagnosticKind::BadAnnotation,
        location: Location::Object(r(num)),
    }
}

fn obj(num: u32, body: &str) -> (u32, String) {
    (num, body.to_string())
}

/// Catalog 1 (plus `catalog_extra`), pages 2, page 3 (plus `page_extra`), page 4, then
/// `objects`. Returns the annotations of page 3.
fn annots_with(catalog_extra: &str, page_extra: &str, objects: &[(u32, String)]) -> Annotations {
    let catalog = format!("<< /Type /Catalog /Pages 2 0 R {catalog_extra} >>");
    let page = format!("<< /Type /Page /Parent 2 0 R {page_extra} >>");
    let mut all: Vec<(u32, &[u8])> = vec![
        (1, catalog.as_bytes()),
        (2, b"<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>"),
        (3, page.as_bytes()),
        (4, b"<< /Type /Page /Parent 2 0 R >>"),
    ];
    all.extend(objects.iter().map(|(n, b)| (*n, b.as_bytes())));
    let (pdf, _) = build_pdf(&all, "/Root 1 0 R");
    Document::from_bytes(pdf)
        .unwrap()
        .page(0)
        .unwrap()
        .annotations()
}

fn annots(page_extra: &str, objects: &[(u32, String)]) -> Annotations {
    annots_with("", page_extra, objects)
}

/// One annotation, object 10, with `body` inside its dictionary.
fn one(body: &str) -> Annotations {
    annots(
        "/Annots [10 0 R]",
        &[obj(10, &format!("<< /Type /Annot {body} >>"))],
    )
}

#[test]
fn no_annots_is_empty() {
    assert_eq!(annots("", &[]), Annotations::default());
}

#[test]
fn fields_in_order() {
    let a = annots(
        "/Annots [10 0 R 11 0 R]",
        &[
            obj(
                10,
                "<< /Subtype /Text /Rect [100 200 50 150] /Contents (Note) /T (Ann) /NM (n1) /F 4 >>",
            ),
            obj(11, "<< /Subtype /Popup /Rect [0 0 1 1] >>"),
        ],
    );
    assert_eq!(a.diagnostics, []);
    assert_eq!(
        a.items,
        [
            Annotation {
                subtype: "Text".into(),
                rect: Rect {
                    x0: 50.,
                    y0: 150.,
                    x1: 100.,
                    y1: 200.
                },
                contents: Some("Note".into()),
                author: Some("Ann".into()),
                name: Some("n1".into()),
                flags: 4,
                quad_points: vec![],
                dest: None,
                uri: None,
                hidden: false,
            },
            Annotation {
                subtype: "Popup".into(),
                rect: Rect {
                    x0: 0.,
                    y0: 0.,
                    x1: 1.,
                    y1: 1.
                },
                contents: None,
                author: None,
                name: None,
                flags: 0,
                quad_points: vec![],
                dest: None,
                uri: None,
                hidden: false,
            },
        ]
    );
}

#[test]
fn direct_annotation_and_indirect_array() {
    let a = annots(
        "/Annots 10 0 R",
        &[obj(10, "[<< /Subtype /Square /Rect [0 0 1 1] >>]")],
    );
    assert_eq!(a.diagnostics, []);
    assert_eq!(a.items[0].subtype, "Square");
}

#[test]
fn utf16_contents() {
    let a = one("/Subtype /Text /Rect [0 0 1 1] /Contents <FEFF65E5672C>");
    assert_eq!(a.items[0].contents.as_deref(), Some("日本"));
}

#[test]
fn quad_points() {
    let a =
        one("/Subtype /Highlight /Rect [0 0 9 9] /QuadPoints [1 2 3 4 5 6 7 8 9 9 9 9 9 9 9 9]");
    assert_eq!(a.diagnostics, []);
    let p = |x, y| Point { x, y };
    assert_eq!(
        a.items[0].quad_points,
        [[p(1., 2.), p(3., 4.), p(5., 6.), p(7., 8.)], [p(9., 9.); 4]]
    );
}

#[test]
fn quad_points_partial_group_is_dropped() {
    let a = one("/Subtype /Highlight /Rect [0 0 1 1] /QuadPoints [1 2 3 4 5 6 7 8 9]");
    assert_eq!(a.diagnostics, [bad(10)]);
    assert_eq!(a.items[0].quad_points.len(), 1);
}

#[test]
fn quad_points_non_number_empties() {
    let a = one("/Subtype /Highlight /Rect [0 0 1 1] /QuadPoints [1 2 3 4 5 6 7 /x]");
    assert_eq!(a.diagnostics, [bad(10)]);
    assert!(a.items[0].quad_points.is_empty());
}

#[test]
fn link_explicit_dest() {
    let a = one("/Subtype /Link /Rect [0 0 1 1] /Dest [4 0 R /XYZ 10 20 0]");
    assert_eq!(a.diagnostics, []);
    assert_eq!(
        a.items[0].dest,
        Some(Destination {
            page: 1,
            left: Some(10.),
            top: Some(20.)
        })
    );
    assert_eq!(a.items[0].uri, None);
}

#[test]
fn link_goto_named_dest() {
    let a = annots_with(
        "/Names << /Dests << /Names [(sec) [3 0 R /Fit]] >> >>",
        "/Annots [10 0 R]",
        &[obj(
            10,
            "<< /Subtype /Link /Rect [0 0 1 1] /A << /S /GoTo /D (sec) >> >>",
        )],
    );
    assert_eq!(a.diagnostics, []);
    assert_eq!(a.items[0].dest.as_ref().map(|d| d.page), Some(0));
}

#[test]
fn link_uri() {
    let a = one("/Subtype /Link /Rect [0 0 1 1] /A << /S /URI /URI (https://example.com/a?b=1) >>");
    assert_eq!(a.diagnostics, []);
    assert_eq!(a.items[0].uri.as_deref(), Some("https://example.com/a?b=1"));
    assert_eq!(a.items[0].dest, None);
}

#[test]
fn uri_utf8_bytes() {
    let a = one("/Subtype /Link /Rect [0 0 1 1] /A << /S /URI /URI <68747470733A2F2FC3A9> >>");
    assert_eq!(a.diagnostics, []);
    assert_eq!(a.items[0].uri.as_deref(), Some("https://é"));
}

#[test]
fn uri_utf16_is_decoded_with_diagnostic() {
    let a = one("/Subtype /Link /Rect [0 0 1 1] /A << /S /URI /URI <FEFF0061> >>");
    assert_eq!(a.diagnostics, [bad(10)]);
    assert_eq!(a.items[0].uri.as_deref(), Some("a"));
}

#[test]
fn uri_invalid_utf8_is_lossy() {
    let a = one("/Subtype /Link /Rect [0 0 1 1] /A << /S /URI /URI <61FF> >>");
    assert_eq!(a.diagnostics, [bad(10)]);
    assert_eq!(a.items[0].uri.as_deref(), Some("a\u{FFFD}"));
}

#[test]
fn uri_missing() {
    let a = one("/Subtype /Link /Rect [0 0 1 1] /A << /S /URI >>");
    assert_eq!(a.diagnostics, [bad(10)]);
    assert_eq!(a.items[0].uri, None);
}

#[test]
fn other_action_is_silent() {
    let a = one("/Subtype /Link /Rect [0 0 1 1] /A << /S /Named /N /NextPage >>");
    assert_eq!(a.diagnostics, []);
    assert_eq!((&a.items[0].dest, &a.items[0].uri), (&None, &None));
}

#[test]
fn bad_dest_reports_bad_annotation() {
    let a = one("/Subtype /Link /Rect [0 0 1 1] /Dest (nowhere)");
    assert_eq!(a.diagnostics, [bad(10)]);
    assert_eq!(a.items[0].dest, None);
}

#[test]
fn dest_and_action_both_present() {
    let a = one("/Subtype /Link /Rect [0 0 1 1] /Dest [3 0 R /Fit] /A << /S /URI /URI (x) >>");
    assert_eq!(a.diagnostics, [bad(10)]);
    assert_eq!(a.items[0].dest.as_ref().map(|d| d.page), Some(0));
}

#[test]
fn missing_subtype_and_rect() {
    let a = one("");
    assert_eq!(a.diagnostics, [bad(10), bad(10)]);
    assert_eq!(a.items[0].subtype, "");
    assert_eq!(
        a.items[0].rect,
        Rect {
            x0: 0.,
            y0: 0.,
            x1: 0.,
            y1: 0.
        }
    );
}

#[test]
fn bad_strings_and_flags() {
    let a = one("/Subtype /Text /Rect [0 0 1 1] /Contents 5 /T /x /NM [] /F -1");
    assert_eq!(a.diagnostics, [bad(10); 4]);
    let i = &a.items[0];
    assert_eq!(
        (&i.contents, &i.author, &i.name, i.flags),
        (&None, &None, &None, 0)
    );
}

#[test]
fn annots_not_an_array() {
    let a = annots("/Annots 5", &[]);
    assert_eq!(a.diagnostics, [bad(3)]);
    assert!(a.items.is_empty());
}

#[test]
fn non_dict_entries_are_skipped() {
    let a = annots(
        "/Annots [7 (x) 10 0 R]",
        &[obj(10, "<< /Subtype /Text /Rect [0 0 1 1] >>")],
    );
    assert_eq!(a.diagnostics, [bad(3), bad(3)]);
    assert_eq!(a.items.len(), 1);
}

#[test]
fn null_and_missing_entries_are_silent() {
    let a = annots("/Annots [null 99 0 R]", &[]);
    assert_eq!(a, Annotations::default());
}

#[test]
fn malformed_actions_and_dests() {
    for body in [
        "/A 5",
        "/A << /D [3 0 R /Fit] >>",
        "/A << /S /GoTo >>",
        "/Dest << /D 5 >>",
    ] {
        let a = one(&format!("/Subtype /Link /Rect [0 0 1 1] {body}"));
        assert_eq!(a.diagnostics, [bad(10)], "{body}");
        assert_eq!(a.items[0].dest, None, "{body}");
    }
}

#[test]
fn malformed_name_trees() {
    for tree in [
        "/Kids [5]",
        "/Names 5",
        "/Names [/k [3 0 R /Fit]]",
        "/Kids 5",
    ] {
        let a = annots_with(
            &format!("/Names << /Dests << {tree} >> >>"),
            "/Annots [10 0 R]",
            &[obj(10, "<< /Subtype /Link /Rect [0 0 1 1] /Dest (k) >>")],
        );
        assert_eq!(a.diagnostics, [bad(1), bad(10)], "{tree}");
        assert_eq!(a.items[0].dest, None, "{tree}");
    }
}

#[test]
fn repeated_reference_is_skipped() {
    let a = annots(
        "/Annots [10 0 R 10 0 R 10 0 R]",
        &[obj(10, "<< /Subtype /Text /Rect [0 0 1 1] >>")],
    );
    assert_eq!(a.diagnostics, [bad(3), bad(3)]);
    assert_eq!(a.items.len(), 1);
}

#[test]
fn widget_has_no_author() {
    let a = one("/Subtype /Widget /Rect [0 0 1 1] /T (email)");
    assert_eq!(a.diagnostics, []);
    assert_eq!(a.items[0].author, None);
}
