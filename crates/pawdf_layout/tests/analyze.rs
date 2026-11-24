use pawdf_layout::Error;
use pawdf_layout::pdfminer::{Params, analyze};
use pawdf_reader::{
    Color, DiagnosticKind, Glyph, Location, Matrix, PageContent, Point, Rect, RenderMode, Rotation,
    WritingMode,
};

const MEDIA: Rect = Rect {
    x0: 0.,
    y0: 0.,
    x1: 300.,
    y1: 300.,
};

fn glyph(text: &str, (x0, y0, x1, y1): (f64, f64, f64, f64)) -> Glyph {
    Glyph {
        text: text.into(),
        code: 0,
        font: 0,
        size: 10.,
        matrix: Matrix::IDENTITY,
        origin: Point { x: x0, y: y0 },
        advance: x1 - x0,
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

fn content(glyphs: Vec<Glyph>) -> PageContent {
    PageContent {
        glyphs,
        paths: vec![],
        images: vec![],
        fonts: vec![],
        marked_contents: vec![],
        diagnostics: vec![],
    }
}

fn run(glyphs: Vec<Glyph>) -> pawdf_layout::Layout {
    analyze(&content(glyphs), MEDIA, Rotation::R0, &Params::default()).unwrap()
}

#[test]
fn empty_page_gives_empty_layout() {
    let l = run(vec![]);
    assert!(l.blocks.is_empty());
    assert!(l.diagnostics.is_empty());
}

#[test]
fn all_hidden_glyphs_give_empty_layout() {
    let mut g = glyph("a", (0., 0., 10., 10.));
    g.hidden = true;
    let l = run(vec![g]);
    assert!(l.blocks.is_empty());
    assert!(l.diagnostics.is_empty());
}

#[test]
fn invisible_text_is_kept() {
    let mut g = glyph("a", (0., 0., 10., 10.));
    g.render_mode = RenderMode::Invisible;
    let l = run(vec![g]);
    assert_eq!(l.blocks[0].lines[0].words[0].glyphs, vec![0]);
}

#[test]
fn non_finite_box_is_skipped_and_reported() {
    let l = run(vec![
        glyph("a", (f64::NAN, 0., 10., 10.)),
        glyph("b", (0., 0., f64::INFINITY, 10.)),
    ]);
    assert!(l.blocks.is_empty());
    assert_eq!(l.diagnostics.len(), 1);
    assert_eq!(l.diagnostics[0].kind, DiagnosticKind::BadGlyphBox);
    assert_eq!(l.diagnostics[0].location, Location::File(0));
}

#[test]
fn box_overflowing_in_page_space_is_skipped() {
    let media = Rect {
        x0: -1e308,
        y0: 0.,
        x1: 300.,
        y1: 300.,
    };
    let c = content(vec![glyph("a", (1e308, 0., 1.5e308, 10.))]);
    let l = analyze(&c, media, Rotation::R0, &Params::default()).unwrap();
    assert!(l.blocks.is_empty());
    assert_eq!(l.diagnostics[0].kind, DiagnosticKind::BadGlyphBox);
}

#[test]
fn glyph_indices_survive_skipping() {
    let mut hidden = glyph("a", (0., 0., 10., 10.));
    hidden.hidden = true;
    let l = run(vec![hidden, glyph("b", (0., 0., 10., 10.))]);
    assert_eq!(l.blocks[0].lines[0].words[0].glyphs, vec![1]);
}

#[test]
fn invalid_params_are_rejected() {
    let c = content(vec![]);
    let check = |p: Params, name: &'static str| {
        assert_eq!(
            analyze(&c, MEDIA, Rotation::R0, &p),
            Err(Error::InvalidParam { name })
        );
    };
    let d = Params::default;
    check(
        Params {
            boxes_flow: Some(1.5),
            ..d()
        },
        "boxes_flow",
    );
    check(
        Params {
            boxes_flow: Some(f64::NAN),
            ..d()
        },
        "boxes_flow",
    );
    check(
        Params {
            line_overlap: f64::NAN,
            ..d()
        },
        "line_overlap",
    );
    check(
        Params {
            char_margin: f64::INFINITY,
            ..d()
        },
        "char_margin",
    );
    check(
        Params {
            line_margin: f64::NAN,
            ..d()
        },
        "line_margin",
    );
    check(
        Params {
            word_margin: f64::NAN,
            ..d()
        },
        "word_margin",
    );
}

#[test]
fn boxes_flow_bounds_are_allowed() {
    let c = content(vec![]);
    for flow in [Some(-1.0), Some(1.0), None] {
        let p = Params {
            boxes_flow: flow,
            ..Params::default()
        };
        assert!(analyze(&c, MEDIA, Rotation::R0, &p).is_ok());
    }
}

#[test]
fn defaults_match_pdfminer() {
    let p = Params::default();
    assert_eq!(
        (
            p.line_overlap,
            p.char_margin,
            p.line_margin,
            p.word_margin,
            p.boxes_flow,
            p.detect_vertical
        ),
        (0.5, 2.0, 0.5, 0.1, Some(0.5), false)
    );
}

fn g(text: &str, x: f64, y: f64) -> Glyph {
    glyph(text, (x, y, x + 10., y + 10.))
}

fn order(glyphs: Vec<Glyph>, boxes_flow: Option<f64>) -> Vec<String> {
    let p = Params {
        boxes_flow,
        ..Params::default()
    };
    let l = analyze(&content(glyphs), MEDIA, Rotation::R0, &p).unwrap();
    assert!(l.diagnostics.is_empty());
    l.blocks.into_iter().map(|b| b.text).collect()
}

// Expected orders below are pdfminer.six 20260107's, with ties broken by creation order.

#[test]
fn corners_read_top_row_first() {
    let glyphs = vec![
        g("a", 0., 0.),
        g("b", 100., 0.),
        g("c", 0., 100.),
        g("d", 100., 100.),
    ];
    assert_eq!(order(glyphs, Some(0.5)), ["c", "d", "a", "b"]);
}

#[test]
fn corner_input_order_is_irrelevant() {
    let glyphs = vec![
        g("d", 100., 100.),
        g("c", 0., 100.),
        g("b", 100., 0.),
        g("a", 0., 0.),
    ];
    assert_eq!(order(glyphs, Some(0.5)), ["c", "d", "a", "b"]);
}

#[test]
fn row_reads_left_to_right() {
    let glyphs = vec![
        g("a", 0., 0.),
        g("b", 100., 0.),
        g("c", 200., 0.),
        g("d", 290., 0.),
    ];
    assert_eq!(order(glyphs, Some(0.5)), ["a", "b", "c", "d"]);
}

#[test]
fn columns_by_flow() {
    let cols = || {
        vec![
            g("a", 0., 200.),
            g("b", 0., 100.),
            g("c", 150., 200.),
            g("d", 150., 100.),
        ]
    };
    for flow in [Some(0.5), Some(-1.0), Some(0.0)] {
        assert_eq!(order(cols(), flow), ["a", "b", "c", "d"], "{flow:?}");
    }
    assert_eq!(order(cols(), Some(1.0)), ["c", "d", "a", "b"]);
    assert_eq!(order(cols(), None), ["a", "c", "b", "d"]);
}

#[test]
fn single_block_page_is_returned() {
    assert_eq!(
        order(vec![g("a", 0., 0.), g("b", 11., 0.)], Some(0.5)),
        ["ab"]
    );
}

fn limits(l: &pawdf_layout::Layout) -> usize {
    l.diagnostics
        .iter()
        .filter(|d| d.kind == DiagnosticKind::LayoutLimit)
        .count()
}

#[test]
fn block_limit_falls_back_to_getkey() {
    let media = Rect {
        x0: 0.,
        y0: 0.,
        x1: 10000.,
        y1: 10000.,
    };
    let glyphs: Vec<_> = (0..1025)
        .map(|i| g("x", (i % 46) as f64 * 100., (i / 46) as f64 * 100.))
        .collect();
    let l = analyze(&content(glyphs), media, Rotation::R0, &Params::default()).unwrap();
    assert_eq!(l.blocks.len(), 1025);
    assert_eq!(limits(&l), 1);
    let keys: Vec<_> = l.blocks.iter().map(|b| (-b.bbox.y0, b.bbox.x0)).collect();
    assert!(keys.windows(2).all(|w| w[0] <= w[1]));
}

#[test]
fn budget_exhausted_while_grouping_lines() {
    let media = Rect {
        x0: 0.,
        y0: 0.,
        x1: 1e9,
        y1: 1e9,
    };
    let glyphs = vec![glyph("a", (0., 0., 1e9, 1e9)), g("b", 0., 2e9)];
    let l = analyze(&content(glyphs), media, Rotation::R0, &Params::default()).unwrap();
    assert_eq!(limits(&l), 1);
    assert_eq!(l.diagnostics[0].location, Location::File(0));
    let texts: Vec<_> = l.blocks.iter().map(|b| b.text.as_str()).collect();
    assert_eq!(texts, ["b", "a"]);
}

#[test]
fn alternating_glyphs_hit_the_budget() {
    let glyphs: Vec<_> = (0..1 << 13)
        .map(|i| g("x", if i % 2 == 0 { 0. } else { 100. }, 0.))
        .collect();
    let l = analyze(&content(glyphs), MEDIA, Rotation::R0, &Params::default()).unwrap();
    assert_eq!(limits(&l), 1);
    assert_eq!(l.blocks.len(), 1 << 13);
}
