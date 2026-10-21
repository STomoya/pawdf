mod common;

use common::build_pdf;
use pawdf_reader::{
    Destination, Diagnostic, DiagnosticKind, Document, Location, Outline, OutlineItem, Ref,
};

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

fn dest(page: usize, left: Option<f64>, top: Option<f64>) -> Option<Destination> {
    Some(Destination { page, left, top })
}

/// One top-level item per entry, objects 11.., each carrying `entries[i]` verbatim.
fn items(catalog_extra: &str, entries: &[&str], extra: &[(u32, String)]) -> Outline {
    let mut objects = vec![obj(10, "<< /First 11 0 R >>")];
    for (i, e) in entries.iter().enumerate() {
        let n = 11 + i as u32;
        let next = if i + 1 < entries.len() {
            format!("/Next {} 0 R", n + 1)
        } else {
            String::new()
        };
        objects.push((n, format!("<< /Title (t) {e} {next} >>")));
    }
    objects.extend_from_slice(extra);
    outline(&format!("/Outlines 10 0 R {catalog_extra}"), &objects)
}

fn dests(o: &Outline) -> Vec<Option<Destination>> {
    o.items.iter().map(|i| i.dest.clone()).collect()
}

#[test]
fn explicit_forms() {
    let o = items(
        "",
        &[
            "/Dest [4 0 R /XYZ 10 20 0]",
            "/Dest [4 0 R /XYZ null null null]",
            "/Dest [4 0 R /Fit]",
            "/Dest [4 0 R /FitH 30]",
            "/Dest [4 0 R /FitBH 31]",
            "/Dest [4 0 R /FitV 40]",
            "/Dest [4 0 R /FitBV 41]",
            "/Dest [4 0 R /FitR 1 2 3 4]",
            "/Dest [4 0 R /FitB]",
        ],
        &[],
    );
    assert_eq!(o.diagnostics, []);
    assert_eq!(
        dests(&o),
        [
            dest(1, Some(10.), Some(20.)),
            dest(1, None, None),
            dest(1, None, None),
            dest(1, None, Some(30.)),
            dest(1, None, Some(31.)),
            dest(1, Some(40.), None),
            dest(1, Some(41.), None),
            dest(1, Some(1.), Some(4.)),
            dest(1, None, None),
        ]
    );
}

#[test]
fn goto_action() {
    let o = items("", &["/A << /S /GoTo /D [3 0 R /Fit] >>"], &[]);
    assert_eq!(o.diagnostics, []);
    assert_eq!(dests(&o), [dest(0, None, None)]);
}

#[test]
fn indirect_action_and_dest() {
    let o = items(
        "",
        &["/A 20 0 R"],
        &[
            obj(20, "<< /S /GoTo /D 21 0 R >>"),
            obj(21, "[4 0 R /FitH 5]"),
        ],
    );
    assert_eq!(o.diagnostics, []);
    assert_eq!(dests(&o), [dest(1, None, Some(5.))]);
}

#[test]
fn non_goto_action_has_no_dest_and_no_diagnostic() {
    let o = items("", &["/A << /S /URI /URI (https://x) >>"], &[]);
    assert_eq!(o.diagnostics, []);
    assert_eq!(dests(&o), [None]);
}

#[test]
fn dest_wins_over_action() {
    let o = items(
        "",
        &["/Dest [4 0 R /Fit] /A << /S /GoTo /D [3 0 R /Fit] >>"],
        &[],
    );
    assert_eq!(dests(&o), [dest(1, None, None)]);
    assert_eq!(o.diagnostics, [bad(11)]);
}

#[test]
fn integer_page_is_accepted() {
    let o = items("", &["/Dest [1 /Fit]", "/Dest [9 /Fit]"], &[]);
    assert_eq!(dests(&o), [dest(1, None, None), None]);
    assert_eq!(o.diagnostics, [bad(11), bad(12)]);
}

#[test]
fn bad_destinations() {
    let o = items(
        "",
        &[
            "/Dest [5 0 R /Fit]",
            "/Dest []",
            "/Dest [3 0 R]",
            "/Dest [3 0 R /Bogus]",
            "/Dest [3 0 R /XYZ (a) 5]",
            "/Dest 7",
        ],
        &[obj(5, "<< /Type /Font >>")],
    );
    assert_eq!(
        dests(&o),
        [
            None,
            None,
            dest(0, None, None),
            dest(0, None, None),
            dest(0, None, Some(5.)),
            None,
        ]
    );
    assert_eq!(
        o.diagnostics,
        [bad(11), bad(12), bad(13), bad(14), bad(15), bad(16)]
    );
}

const TREE: &str = "/Names << /Dests 30 0 R >>";

fn tree(leaf_limits: &str) -> Vec<(u32, String)> {
    vec![
        obj(30, "<< /Kids [31 0 R] >>"),
        (
            31,
            format!("<< /Limits {leaf_limits} /Names [(sec.1) [4 0 R /Fit] (sec.2) 32 0 R] >>"),
        ),
        obj(32, "<< /D [3 0 R /XYZ 0 700 0] >>"),
    ]
}

#[test]
fn string_name_via_name_tree() {
    let o = items(
        TREE,
        &["/A << /S /GoTo /D (sec.1) >>", "/Dest (sec.2)"],
        &tree("[(a) (z)]"),
    );
    assert_eq!(o.diagnostics, []);
    assert_eq!(
        dests(&o),
        [dest(1, None, None), dest(0, Some(0.), Some(700.))]
    );
}

#[test]
fn wrong_limits_are_ignored() {
    let o = items(TREE, &["/Dest (sec.1)"], &tree("[(x) (y)]"));
    assert_eq!(o.diagnostics, []);
    assert_eq!(dests(&o), [dest(1, None, None)]);
}

#[test]
fn name_via_legacy_dests() {
    let o = items("/Dests << /chap1 [4 0 R /Fit] >>", &["/Dest /chap1"], &[]);
    assert_eq!(o.diagnostics, []);
    assert_eq!(dests(&o), [dest(1, None, None)]);
}

#[test]
fn string_and_name_share_keys() {
    let o = items(
        &format!("{TREE} /Dests << /chap1 [4 0 R /Fit] >>"),
        &["/Dest /sec.1", "/Dest (chap1)"],
        &tree("[(a) (z)]"),
    );
    assert_eq!(o.diagnostics, []);
    assert_eq!(dests(&o), [dest(1, None, None), dest(1, None, None)]);
}

#[test]
fn name_tree_wins_over_legacy() {
    let o = items(
        &format!("{TREE} /Dests << /sec.1 [3 0 R /Fit] >>"),
        &["/Dest (sec.1)"],
        &tree("[(a) (z)]"),
    );
    assert_eq!(dests(&o), [dest(1, None, None)]);
}

#[test]
fn unknown_name() {
    let o = items(TREE, &["/Dest (nope)"], &tree("[(a) (z)]"));
    assert_eq!(dests(&o), [None]);
    assert_eq!(o.diagnostics, [bad(11)]);
}

#[test]
fn name_tree_cycle_is_cut() {
    let o = items(
        TREE,
        &["/Dest (a)"],
        &[
            obj(30, "<< /Kids [31 0 R 30 0 R] >>"),
            obj(31, "<< /Names [(a) [3 0 R /Fit]] >>"),
        ],
    );
    assert_eq!(dests(&o), [dest(0, None, None)]);
    assert_eq!(o.diagnostics, [bad(30)]);
}

#[test]
fn odd_names_array() {
    let o = items(
        TREE,
        &["/Dest (a)"],
        &[obj(30, "<< /Names [(a) [3 0 R /Fit] (b)] >>")],
    );
    assert_eq!(dests(&o), [dest(0, None, None)]);
    assert_eq!(o.diagnostics, [bad(30)]);
}

#[test]
fn aliased_name_is_not_followed() {
    let o = items(
        TREE,
        &["/Dest (x)"],
        &[obj(30, "<< /Names [(x) (y) (y) [3 0 R /Fit]] >>")],
    );
    assert_eq!(dests(&o), [None]);
    assert_eq!(o.diagnostics, [bad(11)]);
}

#[test]
fn indirect_action_type_is_resolved() {
    let o = items(
        "",
        &["/A << /S 20 0 R /D [4 0 R /Fit] >>"],
        &[obj(20, "/GoTo")],
    );
    assert_eq!(o.diagnostics, []);
    assert_eq!(dests(&o), [dest(1, None, None)]);
}

#[test]
fn page_ref_generation_is_ignored() {
    let o = items("", &["/Dest [4 1 R /Fit]"], &[]);
    assert_eq!(o.diagnostics, []);
    assert_eq!(dests(&o), [dest(1, None, None)]);
}

#[test]
fn action_without_name_type_is_diagnosed() {
    let o = items(
        "",
        &[
            "/A << /D [4 0 R /Fit] >>",
            "/A << /S (GoTo) /D [4 0 R /Fit] >>",
        ],
        &[],
    );
    assert_eq!(dests(&o), [None, None]);
    assert_eq!(o.diagnostics, [bad(11), bad(12)]);
}

#[test]
fn empty_name_tree_node_is_diagnosed() {
    let o = items(TREE, &["/Dest (a)"], &[obj(30, "<< >>")]);
    assert_eq!(dests(&o), [None]);
    assert_eq!(o.diagnostics, [bad(30), bad(11)]);
}
