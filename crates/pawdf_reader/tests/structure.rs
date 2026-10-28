mod common;

use common::build_pdf;
use pawdf_reader::{
    Diagnostic, DiagnosticKind, Document, Location, MarkedContentRef, ObjectRef, Ref,
    StructElement, StructKid, StructTree,
};

fn r(num: u32) -> Ref {
    Ref { num, generation: 0 }
}

fn bad(num: u32) -> Diagnostic {
    Diagnostic {
        kind: DiagnosticKind::BadStructTree,
        location: Location::Object(r(num)),
    }
}

fn obj(num: u32, body: &str) -> (u32, String) {
    (num, body.to_string())
}

/// Catalog 1 with `/StructTreeRoot 10 0 R` when `objects` is not empty, pages 2, page 3 (with
/// `/Annots [20 0 R 21 0 R 20 0 R 22 0 R]`), page 4, then `objects`.
fn tree(objects: &[(u32, String)]) -> StructTree {
    let root = if objects.is_empty() {
        ""
    } else {
        "/StructTreeRoot 10 0 R"
    };
    let catalog = format!("<< /Type /Catalog /Pages 2 0 R {root} >>");
    let mut all: Vec<(u32, &[u8])> = vec![
        (1, catalog.as_bytes()),
        (2, b"<< /Type /Pages /Kids [3 0 R 4 0 R] /Count 2 >>"),
        (
            3,
            b"<< /Type /Page /Parent 2 0 R /Annots [20 0 R 21 0 R 20 0 R 22 0 R] >>",
        ),
        (4, b"<< /Type /Page /Parent 2 0 R >>"),
        (20, b"<< /Type /Annot /Subtype /Text /Rect [0 0 1 1] >>"),
        (21, b"42"),
        (22, b"<< /Type /Annot /Subtype /Link /Rect [0 0 1 1] >>"),
    ];
    all.extend(objects.iter().map(|(n, b)| (*n, b.as_bytes())));
    let (pdf, _) = build_pdf(&all, "/Root 1 0 R");
    Document::from_bytes(pdf).unwrap().structure_tree()
}

fn mc(page: Option<usize>, mcid: u32) -> StructKid {
    StructKid::MarkedContent(MarkedContentRef {
        page,
        mcid,
        stream: None,
    })
}

fn roles(kids: &[StructKid]) -> Vec<&str> {
    kids.iter()
        .filter_map(|k| match k {
            StructKid::Element(e) => Some(e.role.as_str()),
            _ => None,
        })
        .collect()
}

fn child(e: &StructElement, i: usize) -> &StructElement {
    match &e.children[i] {
        StructKid::Element(e) => e,
        k => panic!("kid {i} is not an element: {k:?}"),
    }
}

#[test]
fn untagged_is_empty() {
    assert_eq!(tree(&[]), StructTree::default());
}

#[test]
fn nested_tree_with_content_items() {
    let t = tree(&[
        obj(
            10,
            "<< /Type /StructTreeRoot /K 11 0 R /RoleMap << /Para /P /Heading /H1 >> >>",
        ),
        obj(
            11,
            "<< /S /Document /P 10 0 R /Lang (en) /K [12 0 R 13 0 R 14 0 R 15 0 R] >>",
        ),
        obj(
            12,
            "<< /Type /StructElem /S /Heading /P 11 0 R /Pg 3 0 R /T (Intro) /K 0 >>",
        ),
        obj(
            13,
            "<< /S /Para /P 11 0 R /Pg 3 0 R /ActualText (x) /E (ex) /K [1 2 << /Type /MCR /Pg 4 0 R /MCID 0 >>] >>",
        ),
        obj(
            14,
            "<< /S /Figure /P 11 0 R /Pg 4 0 R /Alt (A cat) /K << /Type /MCR /MCID 1 /Stm 30 0 R >> >>",
        ),
        obj(
            15,
            "<< /S /Link /P 11 0 R /Pg 3 0 R /K [3 << /Type /OBJR /Obj 22 0 R >> << /Type /OBJR /Obj 4 0 R /Pg 4 0 R >>] >>",
        ),
    ]);
    assert_eq!(t.diagnostics, []);
    assert_eq!(t.roots.len(), 1);
    let doc = &t.roots[0];
    assert_eq!(
        (doc.role.as_str(), doc.lang.as_deref(), doc.page),
        ("Document", Some("en"), None)
    );
    assert_eq!(roles(&doc.children), ["Heading", "Para", "Figure", "Link"]);

    let h = child(doc, 0);
    assert_eq!(
        (h.standard.as_str(), h.title.as_deref(), h.page),
        ("H1", Some("Intro"), Some(0))
    );
    assert_eq!(h.children, [mc(Some(0), 0)]);

    let p = child(doc, 1);
    assert_eq!(p.standard, "P");
    assert_eq!(
        (p.actual_text.as_deref(), p.expansion.as_deref()),
        (Some("x"), Some("ex"))
    );
    assert_eq!(p.children, [mc(Some(0), 1), mc(Some(0), 2), mc(Some(1), 0)]);

    let fig = child(doc, 2);
    assert_eq!((fig.alt.as_deref(), fig.page), (Some("A cat"), Some(1)));
    assert_eq!(
        fig.children,
        [StructKid::MarkedContent(MarkedContentRef {
            page: Some(1),
            mcid: 1,
            stream: Some(r(30)),
        })]
    );

    // Object 22 is the second annotation: the repeated and non-dictionary entries are skipped.
    let link = child(doc, 3);
    assert_eq!(
        link.children,
        [
            mc(Some(0), 3),
            StructKid::Object(ObjectRef {
                page: Some(0),
                annotation: Some(1),
            }),
            StructKid::Object(ObjectRef {
                page: Some(1),
                annotation: None,
            }),
        ]
    );
}

#[test]
fn root_kids_array_and_direct_elements() {
    let t = tree(&[
        obj(10, "<< /K [11 0 R << /S /Sect /K [] >>] >>"),
        obj(11, "<< /S /Part /P 10 0 R >>"),
    ]);
    assert_eq!(
        t.roots.iter().map(|e| e.role.as_str()).collect::<Vec<_>>(),
        ["Part", "Sect"]
    );
    // The direct element has no `/P`.
    assert_eq!(t.diagnostics, [bad(10)]);
}

#[test]
fn mcid_without_own_page_inherits_with_diagnostic() {
    let t = tree(&[
        obj(10, "<< /K 11 0 R >>"),
        obj(11, "<< /S /Sect /P 10 0 R /Pg 4 0 R /K 12 0 R >>"),
        obj(12, "<< /S /P /P 11 0 R /K 5 >>"),
    ]);
    assert_eq!(child(&t.roots[0], 0).children, [mc(Some(1), 5)]);
    assert_eq!(t.diagnostics, [bad(12)]);
}

#[test]
fn mcid_with_no_page_at_all() {
    let t = tree(&[
        obj(10, "<< /K 11 0 R >>"),
        obj(11, "<< /S /P /P 10 0 R /K 5 >>"),
    ]);
    assert_eq!(t.roots[0].children, [mc(None, 5)]);
    assert_eq!(t.diagnostics, [bad(11)]);
}

#[test]
fn bad_page_is_reported() {
    let t = tree(&[
        obj(10, "<< /K 11 0 R >>"),
        obj(
            11,
            "<< /S /P /P 10 0 R /Pg 20 0 R /K << /Type /MCR /MCID 0 /Pg 7 >> >>",
        ),
    ]);
    assert_eq!(t.roots[0].page, None);
    assert_eq!(t.roots[0].children, [mc(None, 0)]);
    assert_eq!(t.diagnostics, [bad(11), bad(11)]);
}

#[test]
fn wrong_or_missing_parent_is_reported() {
    let t = tree(&[
        obj(10, "<< /K [11 0 R 12 0 R] >>"),
        obj(11, "<< /S /P /P 12 0 R >>"),
        obj(12, "<< /S /P >>"),
    ]);
    assert_eq!(t.roots.len(), 2);
    assert_eq!(t.diagnostics, [bad(11), bad(12)]);
}

#[test]
fn cycle_and_shared_element_are_cut() {
    let t = tree(&[
        obj(10, "<< /K [11 0 R 12 0 R] >>"),
        obj(11, "<< /S /Sect /P 10 0 R /K [11 0 R 10 0 R 12 0 R] >>"),
        obj(12, "<< /S /P /P 11 0 R >>"),
    ]);
    assert_eq!(t.roots.len(), 1);
    assert_eq!(roles(&t.roots[0].children), ["P"]);
    assert_eq!(t.diagnostics, [bad(11), bad(11), bad(10)]);
}

#[test]
fn deep_nesting_is_cut() {
    let mut objects = vec![obj(10, "<< /K 100 0 R >>")];
    for i in 0..100u32 {
        let parent = if i == 0 { 10 } else { 99 + i };
        objects.push(obj(
            100 + i,
            &format!("<< /S /Div /P {parent} 0 R /K {} 0 R >>", 101 + i),
        ));
    }
    let t = tree(&objects);
    let mut depth = 0;
    let mut e = &t.roots[0];
    while let Some(StructKid::Element(c)) = e.children.first() {
        e = c;
        depth += 1;
    }
    assert_eq!(depth + 1, 64);
    assert_eq!(t.diagnostics, [bad(164)]);
}

#[test]
fn role_map_cycle_and_bad_value() {
    let t = tree(&[
        obj(
            10,
            "<< /K [11 0 R 12 0 R 13 0 R] /RoleMap << /A /B /B /A /C 5 /P /Para >> >>",
        ),
        obj(11, "<< /S /A /P 10 0 R >>"),
        obj(12, "<< /S /C /P 10 0 R >>"),
        // §14.7.3: a standard name is mapped too.
        obj(13, "<< /S /P /P 10 0 R >>"),
    ]);
    let standard: Vec<_> = t.roots.iter().map(|e| e.standard.as_str()).collect();
    assert_eq!(standard, ["A", "C", "Para"]);
    assert_eq!(t.diagnostics, [bad(11), bad(12)]);
}

#[test]
fn malformed_kids_are_skipped() {
    let t = tree(&[
        obj(10, "<< /K [11 0 R 5] >>"),
        obj(
            11,
            "<< /P 10 0 R /Pg 3 0 R /Alt 5 /K [-1 (x) << /Type /MCR >> << /Type /OBJR >> << /Type /MCR /MCID 2 /Stm 3 >>] >>",
        ),
    ]);
    let e = &t.roots[0];
    assert_eq!((e.role.as_str(), e.alt.as_deref()), ("", None));
    assert_eq!(e.children, [mc(Some(0), 2)]);
    // /S, /Alt, -1, (x), the empty MCR and OBJR, /Stm, then the root's MCID: no page, not an
    // element.
    assert_eq!(
        t.diagnostics,
        [
            bad(11),
            bad(11),
            bad(11),
            bad(11),
            bad(11),
            bad(11),
            bad(11),
            bad(10),
            bad(10),
        ]
    );
}

#[test]
fn bad_root_is_reported() {
    let t = tree(&[obj(10, "[1 2]")]);
    assert_eq!(t.roots, []);
    assert_eq!(
        t.diagnostics,
        [Diagnostic {
            kind: DiagnosticKind::BadStructTree,
            location: Location::Object(r(1)),
        }]
    );
}

#[test]
fn bad_role_map_dangling_kid_and_direct_parent() {
    let t = tree(&[obj(
        10,
        "<< /RoleMap 5 /K [99 0 R << /S /Sect /P 10 0 R /K << /S /P /P 10 0 R >> >>] >>",
    )]);
    let sect = &t.roots[0];
    // A direct parent has no reference to check `/P` against.
    assert_eq!(roles(&sect.children), ["P"]);
    assert_eq!(t.diagnostics, [bad(10)]);
}
