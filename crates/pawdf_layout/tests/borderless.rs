use pawdf_layout::Error;
use pawdf_layout::pdfminer::Params;
use pawdf_layout::table::{BorderlessParams, Table, Tables, borderless};
use pawdf_reader::{
    Color, DiagnosticKind, Glyph, Matrix, PageContent, Point, Rect, RenderMode, Rotation,
    WritingMode,
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

/// One glyph per character of `text`, side by side from `(x, y)`.
fn word(text: &str, x: f64, y: f64) -> Vec<Glyph> {
    text.chars()
        .enumerate()
        .map(|(i, c)| glyph(&c.to_string(), x + 10. * i as f64, y))
        .collect()
}

fn page(glyphs: Vec<Glyph>) -> PageContent {
    PageContent {
        glyphs,
        paths: vec![],
        images: vec![],
        fonts: vec![],
        marked_contents: vec![],
        diagnostics: vec![],
    }
}

fn find_with(
    c: &PageContent,
    region: Option<(f64, f64, f64, f64)>,
    align: &BorderlessParams,
) -> Tables {
    let region = region.map(|(x0, y0, x1, y1)| Rect { x0, y0, x1, y1 });
    borderless(
        c,
        MEDIA,
        Rotation::R0,
        &DISPLAY,
        region,
        align,
        &Params::default(),
    )
    .unwrap()
}

fn tables(c: &PageContent, region: Option<(f64, f64, f64, f64)>) -> Vec<Table> {
    let t = find_with(c, region, &BorderlessParams::default());
    assert_eq!(t.diagnostics, []);
    t.tables
}

/// Each cell's text, row by row.
fn texts(t: &Table) -> Vec<Vec<String>> {
    let mut out = vec![vec![String::new(); t.cols]; t.rows];
    for c in &t.cells {
        assert_eq!((c.row_span, c.col_span), (1, 1));
        out[c.row][c.col] = c
            .layout
            .blocks
            .iter()
            .map(|b| b.text.trim())
            .collect::<Vec<_>>()
            .join("|");
    }
    out
}

fn r(x0: f64, y0: f64, x1: f64, y1: f64) -> Rect {
    Rect { x0, y0, x1, y1 }
}

/// Three left-aligned columns at x = 20, 80 and 140, with rows at y = 20, 40 and 60.
fn three_by_three() -> Vec<Glyph> {
    let mut g = Vec::new();
    for (row, y) in [20., 40., 60.].into_iter().enumerate() {
        for (col, x) in [20., 80., 140.].into_iter().enumerate() {
            g.extend(word(&format!("{}{}", row, col), x, y));
        }
    }
    g
}

fn strings(rows: &[&[&str]]) -> Vec<Vec<String>> {
    rows.iter()
        .map(|r| r.iter().map(|s| s.to_string()).collect())
        .collect()
}

#[test]
fn aligned_words_make_a_grid() {
    let [t] = &tables(&page(three_by_three()), None)[..] else {
        panic!("one table expected");
    };
    assert_eq!(
        texts(t),
        strings(&[
            &["00", "01", "02"],
            &["10", "11", "12"],
            &["20", "21", "22"]
        ])
    );
    // Columns start at each group's left and end at the rightmost group's right; rows meet
    // halfway across the gap between them.
    assert_eq!(t.bbox, r(20., 20., 160., 70.));
    assert_eq!(t.cells[0].bbox, Some(r(20., 20., 80., 35.)));
    assert_eq!(t.cells[8].bbox, Some(r(140., 55., 160., 70.)));
}

#[test]
fn centred_words_make_a_column() {
    let mut g = Vec::new();
    for (y, w) in [(20., "b"), (40., "eee"), (60., "h")] {
        g.extend(word(&w[..1], 20., y));
        g.extend(word(w, 100. - 5. * w.len() as f64, y));
    }
    let [t] = &tables(&page(g), None)[..] else {
        panic!("one table expected");
    };
    assert_eq!(
        texts(t),
        strings(&[&["b", "b"], &["e", "eee"], &["h", "h"]])
    );
    assert_eq!(t.cells[1].bbox, Some(r(85., 20., 115., 35.)));
}

#[test]
fn two_aligned_words_are_not_a_column() {
    let g: Vec<_> = three_by_three()
        .into_iter()
        .filter(|g| g.bbox.y1 > 240.)
        .collect();
    assert_eq!(tables(&page(g), None), []);
}

#[test]
fn single_cell_without_region_is_left_out() {
    let align = BorderlessParams {
        min_words_vertical: 1,
        ..BorderlessParams::default()
    };
    let t = find_with(&page(word("ab", 20., 20.)), None, &align);
    assert_eq!(t.tables, []);
    assert_eq!(t.diagnostics, []);
}

#[test]
fn no_row_means_no_table() {
    let align = BorderlessParams {
        min_words_horizontal: 4,
        ..BorderlessParams::default()
    };
    assert_eq!(find_with(&page(three_by_three()), None, &align).tables, []);
}

#[test]
fn region_is_the_table_box() {
    let mut g = three_by_three();
    // Left-aligned at x = 50 below the region: a column boundary if they counted.
    for y in [100., 120., 140.] {
        g.extend(word("zz", 50., y));
    }
    let [t] = &tables(&page(g), Some((160., 80., 10., 10.)))[..] else {
        panic!("one table expected");
    };
    assert_eq!(t.bbox, r(10., 10., 160., 80.));
    assert_eq!((t.rows, t.cols), (3, 3));
    // The space between the text and the region's border joins the outer rows and columns.
    assert_eq!(t.cells[0].bbox, Some(r(10., 10., 80., 35.)));
    assert_eq!(t.cells[8].bbox, Some(r(140., 55., 160., 80.)));
    assert_eq!(texts(t)[2], ["20", "21", "22"]);
}

#[test]
fn region_without_words_is_one_cell() {
    let [t] = &tables(&page(vec![]), Some((10., 10., 60., 40.)))[..] else {
        panic!("one table expected");
    };
    assert_eq!((t.rows, t.cols, t.bbox), (1, 1, r(10., 10., 60., 40.)));
}

#[test]
fn word_crossing_the_region_is_kept_whole() {
    let g = word("ab", 50., 20.);
    let [t] = &tables(&page(g), Some((10., 10., 60., 40.)))[..] else {
        panic!("one table expected");
    };
    assert_eq!(texts(t), strings(&[&["ab"]]));
}

#[test]
fn too_many_cells_hit_the_limit() {
    // Every word on its own row and column, so the grid is 1,100 × 1,100.
    let g: Vec<_> = (0..1100)
        .map(|i| glyph("a", 11. * i as f64, 11. * i as f64))
        .collect();
    let align = BorderlessParams {
        min_words_vertical: 1,
        ..BorderlessParams::default()
    };
    let t = find_with(&page(g), None, &align);
    assert_eq!(t.tables, []);
    assert_eq!(t.diagnostics.len(), 1);
    assert_eq!(t.diagnostics[0].kind, DiagnosticKind::LayoutLimit);
}

#[test]
fn many_columns_hit_the_work_limit() {
    let g: Vec<_> = (0..2100)
        .map(|i| glyph("a", 11. * i as f64, 11. * i as f64))
        .collect();
    let align = BorderlessParams {
        min_words_vertical: 1,
        ..BorderlessParams::default()
    };
    let t = find_with(&page(g), None, &align);
    assert_eq!(t.tables, []);
    assert_eq!(t.diagnostics[0].kind, DiagnosticKind::LayoutLimit);
}

#[test]
fn bad_settings_are_rejected() {
    let c = page(three_by_three());
    let run = |region, params: &Params| {
        borderless(
            &c,
            MEDIA,
            Rotation::R0,
            &DISPLAY,
            region,
            &BorderlessParams::default(),
            params,
        )
    };
    let nan = Some(r(f64::NAN, 0., 10., 10.));
    assert_eq!(
        run(nan, &Params::default()),
        Err(Error::InvalidParam { name: "region" })
    );
    let bad = Params {
        char_margin: f64::NAN,
        ..Params::default()
    };
    assert_eq!(
        run(None, &bad),
        Err(Error::InvalidParam {
            name: "char_margin"
        })
    );
}

#[test]
fn hidden_glyph_in_a_gap_keeps_no_empty_row() {
    let mut g = three_by_three();
    let mut hidden = glyph("x", 20., 30.);
    hidden.hidden = true;
    g.push(hidden);
    let [t] = &tables(&page(g), None)[..] else {
        panic!("one table expected");
    };
    assert_eq!((t.rows, t.cols), (3, 3));
    assert_eq!(t.cells[0].bbox, Some(r(20., 20., 80., 35.)));
}

#[test]
fn degenerate_region_is_one_cell() {
    let [t] = &tables(&page(three_by_three()), Some((10., 10., 10., 50.)))[..] else {
        panic!("one table expected");
    };
    assert_eq!((t.rows, t.cols), (1, 1));
    assert_eq!((t.cells[0].row_span, t.cells[0].col_span), (1, 1));
}

#[test]
fn qualifying_row_outside_the_columns_makes_no_table() {
    let mut g = Vec::new();
    for y in [20., 40., 60.] {
        g.extend(word("a", 20., y));
        g.extend(word("b", 80., y + 10.));
    }
    g.extend(word("cc", 200., 200.));
    g.extend(word("dd", 260., 200.));
    let align = BorderlessParams {
        min_words_horizontal: 2,
        ..BorderlessParams::default()
    };
    assert_eq!(find_with(&page(g), None, &align).tables, []);
}
