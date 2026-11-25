use pawdf_layout::Error;
use pawdf_layout::pdfminer::Params;
use pawdf_layout::table::{Table, tagged};
use pawdf_reader::{
    Color, Glyph, MarkedContent, MarkedContentRef, Matrix, ObjectRef, PageContent, Point, Rect,
    Ref, RenderMode, Rotation, StructElement, StructKid, StructTree, WritingMode,
};

const MEDIA: Rect = Rect {
    x0: 0.,
    y0: 0.,
    x1: 300.,
    y1: 300.,
};

const DISPLAY: Matrix = Matrix {
    a: 1.,
    b: 0.,
    c: 0.,
    d: -1.,
    e: 0.,
    f: 300.,
};

/// A glyph with its top-left corner at `(x, y)` in display space, 10 points square.
fn glyph(text: &str, x: f64, y: f64) -> Glyph {
    let (x0, y0, x1, y1) = (x, 290. - y, x + 10., 300. - y);
    Glyph {
        text: text.into(),
        code: 0,
        font: 0,
        size: 10.,
        matrix: Matrix::IDENTITY,
        origin: Point { x: x0, y: y0 },
        advance: 10.,
        bbox: Rect { x0, y0, x1, y1 },
        wmode: WritingMode::Horizontal,
        render_mode: RenderMode::Fill,
        fill: Color::Gray(0.),
        stroke: Color::Gray(0.),
        fill_alpha: 1.,
        stroke_alpha: 1.,
        clip: None,
        hidden: false,
    }
}

fn marked(stream: Option<Ref>, mcid: u32, glyph: usize) -> MarkedContent {
    MarkedContent {
        tag: "Span".into(),
        mcid: Some(mcid),
        stream,
        actual_text: None,
        artifact_type: None,
        artifact_subtype: None,
        glyphs: glyph..glyph + 1,
        paths: 0..0,
        images: 0..0,
    }
}

/// Glyph `i` is marked with MCID `i`.
fn content(glyphs: Vec<Glyph>) -> PageContent {
    PageContent {
        marked_contents: (0..glyphs.len())
            .map(|i| marked(None, i as u32, i))
            .collect(),
        glyphs,
        paths: vec![],
        images: vec![],
        fonts: vec![],
        diagnostics: vec![],
    }
}

fn el(role: &str, children: Vec<StructKid>) -> StructKid {
    StructKid::Element(StructElement {
        role: role.into(),
        standard: role.into(),
        title: None,
        alt: None,
        actual_text: None,
        lang: None,
        expansion: None,
        page: None,
        row_span: 1,
        col_span: 1,
        children,
    })
}

fn span(kid: StructKid, rows: u32, cols: u32) -> StructKid {
    match kid {
        StructKid::Element(e) => StructKid::Element(StructElement {
            row_span: rows,
            col_span: cols,
            ..e
        }),
        k => k,
    }
}

fn mc_on(page: usize, mcid: u32) -> StructKid {
    StructKid::MarkedContent(MarkedContentRef {
        page: Some(page),
        mcid,
        stream: None,
    })
}

fn td(mcid: u32) -> StructKid {
    el("TD", vec![mc_on(0, mcid)])
}

fn th(mcid: u32) -> StructKid {
    el("TH", vec![mc_on(0, mcid)])
}

fn tree(roots: Vec<StructKid>) -> StructTree {
    StructTree {
        roots: roots
            .into_iter()
            .filter_map(|k| match k {
                StructKid::Element(e) => Some(e),
                _ => None,
            })
            .collect(),
        diagnostics: vec![],
    }
}

fn run(content: &PageContent, roots: Vec<StructKid>, region: Option<Rect>) -> Vec<Table> {
    tagged(
        content,
        &tree(roots),
        0,
        MEDIA,
        Rotation::R0,
        &DISPLAY,
        region,
        &Params::default(),
    )
    .unwrap()
    .tables
}

/// `(row, col, row_span, col_span, header, text)` per cell.
fn grid(t: &Table) -> Vec<(usize, usize, usize, usize, bool, String)> {
    t.cells
        .iter()
        .map(|c| {
            let text = c
                .layout
                .blocks
                .iter()
                .map(|b| b.text.trim())
                .collect::<Vec<_>>()
                .join("|");
            (c.row, c.col, c.row_span, c.col_span, c.header, text)
        })
        .collect()
}

fn s(t: &str) -> String {
    t.into()
}

fn abcd() -> PageContent {
    content(vec![
        glyph("a", 10., 10.),
        glyph("b", 50., 10.),
        glyph("c", 10., 30.),
        glyph("d", 50., 30.),
    ])
}

#[test]
fn simple_table() {
    let got = run(
        &abcd(),
        vec![el(
            "Table",
            vec![el("TR", vec![th(0), th(1)]), el("TR", vec![td(2), td(3)])],
        )],
        None,
    );
    assert_eq!(got.len(), 1);
    let t = &got[0];
    assert_eq!((t.rows, t.cols), (2, 2));
    assert_eq!(
        grid(t),
        [
            (0, 0, 1, 1, true, s("a")),
            (0, 1, 1, 1, true, s("b")),
            (1, 0, 1, 1, false, s("c")),
            (1, 1, 1, 1, false, s("d")),
        ]
    );
    assert_eq!(
        t.cells[0].bbox,
        Some(Rect {
            x0: 10.,
            y0: 10.,
            x1: 20.,
            y1: 20.
        })
    );
    assert_eq!(
        t.bbox,
        Rect {
            x0: 10.,
            y0: 10.,
            x1: 60.,
            y1: 40.
        }
    );
}

#[test]
fn sections_and_nested_content() {
    let got = run(
        &abcd(),
        vec![el(
            "Document",
            vec![el(
                "Table",
                vec![
                    el("THead", vec![el("TR", vec![th(0), th(1)])]),
                    el(
                        "TBody",
                        vec![el(
                            "TR",
                            vec![el("TD", vec![el("P", vec![mc_on(0, 2), mc_on(0, 3)])])],
                        )],
                    ),
                ],
            )],
        )],
        None,
    );
    assert_eq!(
        grid(&got[0]),
        [
            (0, 0, 1, 1, true, s("a")),
            (0, 1, 1, 1, true, s("b")),
            (1, 0, 1, 1, false, s("c|d")),
        ]
    );
}

#[test]
fn spans_push_later_cells_right() {
    // a spans two rows, b spans two columns: row 1 is d at column 1, then c at column 2.
    let got = run(
        &abcd(),
        vec![el(
            "Table",
            vec![
                el("TR", vec![span(td(0), 2, 1), span(td(1), 1, 2)]),
                el("TR", vec![td(3), td(2)]),
            ],
        )],
        None,
    );
    let t = &got[0];
    assert_eq!((t.rows, t.cols), (2, 3));
    assert_eq!(
        grid(t),
        [
            (0, 0, 2, 1, false, s("a")),
            (0, 1, 1, 2, false, s("b")),
            (1, 1, 1, 1, false, s("d")),
            (1, 2, 1, 1, false, s("c")),
        ]
    );
}

#[test]
fn huge_spans_are_clipped_to_the_rows() {
    let got = run(
        &abcd(),
        vec![el(
            "Table",
            vec![
                el("TR", vec![span(td(0), u32::MAX, u32::MAX)]),
                el("TR", vec![td(1)]),
            ],
        )],
        None,
    );
    let t = &got[0];
    assert_eq!(t.rows, 2);
    assert_eq!(t.cells[0].row_span, 2);
    assert_eq!(t.cells[1].col, u32::MAX as usize);
}

#[test]
fn nested_table_is_returned_too() {
    let got = run(
        &abcd(),
        vec![el(
            "Table",
            vec![el(
                "TR",
                vec![
                    td(0),
                    el("TD", vec![el("Table", vec![el("TR", vec![td(1)])])]),
                ],
            )],
        )],
        None,
    );
    assert_eq!(got.len(), 2);
    assert_eq!(
        grid(&got[0]),
        [(0, 0, 1, 1, false, s("a")), (0, 1, 1, 1, false, s("b"))]
    );
    assert_eq!(grid(&got[1]), [(0, 0, 1, 1, false, s("b"))]);
}

#[test]
fn rows_on_other_pages_are_dropped() {
    let other = el("TD", vec![mc_on(1, 2)]);
    let got = run(
        &abcd(),
        vec![el(
            "Table",
            vec![el("TR", vec![other.clone()]), el("TR", vec![td(0), other])],
        )],
        None,
    );
    let t = &got[0];
    assert_eq!(t.rows, 2);
    assert_eq!(
        grid(t),
        [(1, 0, 1, 1, false, s("a")), (1, 1, 1, 1, false, s(""))]
    );
    assert_eq!(t.cells[1].bbox, None);
}

#[test]
fn table_without_glyphs_here_is_skipped() {
    let got = run(
        &abcd(),
        vec![
            el("Table", vec![el("TR", vec![el("TD", vec![mc_on(1, 0)])])]),
            el("Table", vec![el("TR", vec![el("TD", vec![mc_on(0, 9)])])]),
            el("Table", vec![td(0), el("TR", vec![])]),
        ],
        None,
    );
    assert!(got.is_empty());
}

#[test]
fn hidden_glyphs_are_left_out() {
    let mut c = abcd();
    c.glyphs[1].hidden = true;
    let got = run(
        &c,
        vec![el(
            "Table",
            vec![el("TR", vec![el("TD", vec![mc_on(0, 0), mc_on(0, 1)])])],
        )],
        None,
    );
    let cell = &got[0].cells[0];
    assert_eq!(grid(&got[0])[0].5, "a");
    assert_eq!(cell.bbox.map(|b| b.x1), Some(20.));
}

#[test]
fn form_xobject_content_is_matched_by_stream() {
    let form = Ref {
        num: 9,
        generation: 0,
    };
    let mut c = abcd();
    c.marked_contents.push(marked(Some(form), 0, 3));
    let in_form = el(
        "TD",
        vec![
            StructKid::Object(ObjectRef {
                object: form,
                page: Some(0),
                annotation: None,
            }),
            StructKid::MarkedContent(MarkedContentRef {
                page: Some(0),
                mcid: 0,
                stream: Some(form),
            }),
        ],
    );
    let got = run(&c, vec![el("Table", vec![el("TR", vec![in_form])])], None);
    assert_eq!(grid(&got[0])[0].5, "d");
}

#[test]
fn region_keeps_overlapping_tables() {
    let roots = || {
        vec![
            el("Table", vec![el("TR", vec![td(0)])]),
            el("Table", vec![el("TR", vec![td(3)])]),
        ]
    };
    // Reversed corners, overlapping only the second table (display box 50..60 × 30..40).
    let r = Rect {
        x0: 70.,
        y0: 45.,
        x1: 55.,
        y1: 35.,
    };
    let got = run(&abcd(), roots(), Some(r));
    assert_eq!(got.len(), 1);
    assert_eq!(grid(&got[0])[0].5, "d");
    // Touching an edge is not overlapping.
    let r = Rect {
        x0: 60.,
        y0: 0.,
        x1: 70.,
        y1: 100.,
    };
    assert!(run(&abcd(), roots(), Some(r)).is_empty());
}

#[test]
fn non_finite_region_is_rejected() {
    let r = Rect {
        x0: f64::NAN,
        y0: 0.,
        x1: 1.,
        y1: 1.,
    };
    let got = tagged(
        &abcd(),
        &tree(vec![]),
        0,
        MEDIA,
        Rotation::R0,
        &DISPLAY,
        Some(r),
        &Params::default(),
    );
    assert_eq!(got, Err(Error::InvalidParam { name: "region" }));
}

#[test]
fn row_with_only_missing_sequences_is_dropped() {
    let got = run(
        &abcd(),
        vec![el(
            "Table",
            vec![
                el("TR", vec![td(0)]),
                el("TR", vec![el("TD", vec![mc_on(0, 9)])]),
            ],
        )],
        None,
    );
    assert_eq!(grid(&got[0]), [(0, 0, 1, 1, false, s("a"))]);
}

#[test]
fn row_span_ends_after_its_rows() {
    let got = run(
        &abcd(),
        vec![el(
            "Table",
            vec![
                el("TR", vec![span(td(0), 2, 1), td(0)]),
                el("TR", vec![td(1)]),
                el("TR", vec![td(2), el("TD", vec![mc_on(0, 0), mc_on(0, 0)])]),
            ],
        )],
        None,
    );
    let g: Vec<_> = grid(&got[0]).into_iter().map(|c| (c.0, c.1, c.5)).collect();
    assert_eq!(
        g,
        [
            (0, 0, s("a")),
            (0, 1, s("a")),
            (1, 1, s("b")),
            (2, 0, s("c")),
            (2, 1, s("a")),
        ]
    );
}

#[test]
fn stream_generation_is_ignored() {
    let mut c = abcd();
    c.marked_contents.push(marked(
        Some(Ref {
            num: 9,
            generation: 0,
        }),
        0,
        3,
    ));
    let cell = el(
        "TD",
        vec![StructKid::MarkedContent(MarkedContentRef {
            page: Some(0),
            mcid: 0,
            stream: Some(Ref {
                num: 9,
                generation: 1,
            }),
        })],
    );
    let got = run(&c, vec![el("Table", vec![el("TR", vec![cell])])], None);
    assert_eq!(grid(&got[0])[0].5, "d");
}

#[test]
fn table_with_only_hidden_glyphs_is_skipped() {
    let mut c = abcd();
    c.glyphs[0].hidden = true;
    c.marked_contents.push(MarkedContent {
        mcid: None,
        ..marked(None, 0, 1)
    });
    assert!(run(&c, vec![el("Table", vec![el("TR", vec![td(0)])])], None).is_empty());
}
