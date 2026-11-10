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
#[ignore = "needs lines"]
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
#[ignore = "needs lines"]
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
