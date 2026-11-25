use pawdf_layout::Error;
use pawdf_layout::pdfminer::Params;
use pawdf_layout::table::{RuledParams, Table, Tables, ruled};
use pawdf_reader::{
    Color, DiagnosticKind, FillRule, Glyph, Matrix, PageContent, Path, Point, Rect, RenderMode,
    Rotation, Segment, WritingMode,
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

/// A point given in display space, in user space.
fn pt(x: f64, y: f64) -> Point {
    Point { x, y: 300. - y }
}

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

fn path(segments: Vec<Segment>) -> Path {
    Path {
        segments,
        clip: None,
        stroke: true,
        fill: None,
        line_width: 1.,
        ctm: Matrix::IDENTITY,
        fill_color: Color::Gray(0.),
        stroke_color: Color::Gray(0.),
        fill_alpha: 1.,
        stroke_alpha: 1.,
        hidden: false,
    }
}

/// A stroked line between two display-space points.
fn line(x0: f64, y0: f64, x1: f64, y1: f64) -> Path {
    path(vec![
        Segment::MoveTo(pt(x0, y0)),
        Segment::LineTo(pt(x1, y1)),
    ])
}

/// A closed rectangle, as `re` builds it, from display-space corners.
fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Path {
    path(vec![
        Segment::MoveTo(pt(x0, y0)),
        Segment::LineTo(pt(x1, y0)),
        Segment::LineTo(pt(x1, y1)),
        Segment::LineTo(pt(x0, y1)),
        Segment::Close,
    ])
}

fn page(paths: Vec<Path>, glyphs: Vec<Glyph>) -> PageContent {
    PageContent {
        glyphs,
        paths,
        images: vec![],
        fonts: vec![],
        marked_contents: vec![],
        diagnostics: vec![],
    }
}

fn find(c: &PageContent, region: Option<(f64, f64, f64, f64)>) -> Tables {
    let region = region.map(|(x0, y0, x1, y1)| Rect { x0, y0, x1, y1 });
    ruled(
        c,
        MEDIA,
        Rotation::R0,
        &DISPLAY,
        region,
        &RuledParams::default(),
        &Params::default(),
    )
    .unwrap()
}

fn tables(c: &PageContent, region: Option<(f64, f64, f64, f64)>) -> Vec<Table> {
    let t = find(c, region);
    assert_eq!(t.diagnostics, []);
    t.tables
}

/// `(row, col, row_span, col_span, text)` per cell.
fn grid(t: &Table) -> Vec<(usize, usize, usize, usize, String)> {
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
            (c.row, c.col, c.row_span, c.col_span, text)
        })
        .collect()
}

fn s(t: &str) -> String {
    t.into()
}

fn r(x0: f64, y0: f64, x1: f64, y1: f64) -> Rect {
    Rect { x0, y0, x1, y1 }
}

/// Rules at y = 10, 30, 50 and x = 10, 60, 110: two rows and two columns.
fn two_by_two() -> Vec<Path> {
    vec![
        line(10., 10., 110., 10.),
        line(10., 30., 110., 30.),
        line(10., 50., 110., 50.),
        line(10., 10., 10., 50.),
        line(60., 10., 60., 50.),
        line(110., 10., 110., 50.),
    ]
}

fn abcd() -> Vec<Glyph> {
    vec![
        glyph("a", 20., 12.),
        glyph("b", 70., 12.),
        glyph("c", 20., 32.),
        glyph("d", 70., 32.),
    ]
}

#[test]
fn ruled_grid() {
    let got = tables(&page(two_by_two(), abcd()), None);
    assert_eq!(got.len(), 1);
    let t = &got[0];
    assert_eq!((t.rows, t.cols), (2, 2));
    assert_eq!(t.bbox, r(10., 10., 110., 50.));
    assert_eq!(
        grid(t),
        [
            (0, 0, 1, 1, s("a")),
            (0, 1, 1, 1, s("b")),
            (1, 0, 1, 1, s("c")),
            (1, 1, 1, 1, s("d")),
        ]
    );
    assert_eq!(t.cells[3].bbox, Some(r(60., 30., 110., 50.)));
    assert!(t.cells.iter().all(|c| !c.header));
}

#[test]
fn rectangles_make_cells() {
    let mut cells = vec![
        rect(10., 10., 60., 30.),
        rect(60., 10., 110., 30.),
        rect(10., 30., 60., 50.),
        rect(60., 30., 110., 50.),
    ];
    // A filled-only path counts too: shaded cells have borders.
    cells[3].stroke = false;
    cells[3].fill = Some(FillRule::NonZero);
    let got = tables(&page(cells, abcd()), None);
    assert_eq!(grid(&got[0]).len(), 4);
    assert_eq!(grid(&got[0])[3].4, "d");
}

#[test]
fn merged_cell_spans_columns() {
    let mut paths = two_by_two();
    paths[4] = line(60., 30., 60., 50.);
    let got = tables(&page(paths, abcd()), None);
    assert_eq!(
        grid(&got[0]),
        [
            (0, 0, 1, 2, s("a|b")),
            (1, 0, 1, 1, s("c")),
            (1, 1, 1, 1, s("d")),
        ]
    );
}

#[test]
fn near_lines_snap_and_broken_lines_join() {
    let paths = vec![
        line(10., 10., 50., 10.),
        line(52., 11.5, 110., 11.5),
        line(10., 30., 110., 30.),
        line(10., 50., 110., 50.),
        line(10., 10., 10., 50.),
        line(61., 10., 61., 50.),
        line(110., 10., 110., 50.),
    ];
    let got = tables(&page(paths, abcd()), None);
    let t = &got[0];
    assert_eq!((t.rows, t.cols), (2, 2));
    assert_eq!(t.bbox.y0, 10.75);
}

#[test]
fn thin_filled_bar_is_a_rule() {
    let mut paths = two_by_two();
    let mut bar = rect(10., 29.75, 110., 30.25);
    bar.stroke = false;
    bar.fill = Some(FillRule::NonZero);
    paths[1] = bar;
    let got = tables(&page(paths, abcd()), None);
    assert_eq!((got[0].rows, got[0].cols), (2, 2));
}

#[test]
fn single_box_is_not_a_table() {
    assert!(tables(&page(vec![rect(10., 10., 60., 30.)], abcd()), None).is_empty());
}

#[test]
fn separate_tables_come_top_to_bottom() {
    let mut paths: Vec<Path> = two_by_two()
        .into_iter()
        .map(|mut p| {
            for s in &mut p.segments {
                if let Segment::MoveTo(q) | Segment::LineTo(q) = s {
                    q.y -= 100.;
                }
            }
            p
        })
        .collect();
    paths.extend(two_by_two());
    let got = tables(&page(paths, abcd()), None);
    assert_eq!(got.len(), 2);
    assert_eq!(got[0].bbox.y0, 10.);
    assert_eq!(got[1].bbox.y0, 110.);
    assert_eq!(grid(&got[1])[0].4, "");
}

#[test]
fn hidden_clipped_and_curved_paths_are_skipped() {
    let mut paths = two_by_two();
    paths[4].hidden = true;
    let mut clipped = line(60., 10., 60., 50.);
    clipped.clip = Some(r(0., 0., 0., 0.));
    paths.push(clipped);
    paths.push(path(vec![
        Segment::MoveTo(pt(60., 10.)),
        Segment::CurveTo(pt(60., 20.), pt(60., 40.), pt(60., 50.)),
    ]));
    paths.push(line(60., 10., 70., 50.));
    paths.push(line(60., f64::NAN, 60., 50.));
    let got = tables(&page(paths, abcd()), None);
    assert_eq!(grid(&got[0]).len(), 2);
}

#[test]
fn edges_are_clipped_to_the_clip_bound() {
    let mut paths = two_by_two();
    // In user space the clip keeps display y 30..50 only: the vertical at x = 60 loses its top.
    paths[4].clip = Some(r(0., 250., 300., 270.));
    // Entirely outside its clip bound: no edge.
    let mut outside = line(35., 10., 35., 50.);
    outside.clip = Some(r(200., 0., 300., 300.));
    paths.push(outside);
    let got = tables(&page(paths, abcd()), None);
    assert_eq!(grid(&got[0])[0].3, 2);
}

#[test]
fn glyph_on_a_shared_edge_goes_right_and_down() {
    let got = tables(&page(two_by_two(), vec![glyph("e", 55., 25.)]), None);
    assert_eq!(grid(&got[0])[3].4, "e");
}

#[test]
fn region_adds_a_frame_and_clips_lines() {
    let paths = vec![
        line(0., 30., 300., 30.),
        line(80., 0., 80., 300.),
        line(200., 0., 200., 300.),
    ];
    let glyphs = vec![
        glyph("a", 20., 12.),
        glyph("c", 20., 32.),
        // A word crossing the region's right edge: kept whole, in the last column.
        glyph("x", 102., 32.),
        glyph("y", 112., 32.),
        glyph("z", 250., 250.),
    ];
    let got = tables(&page(paths, glyphs), Some((110., 50., 10., 10.)));
    assert_eq!(got.len(), 1);
    let t = &got[0];
    assert_eq!(t.bbox, r(10., 10., 110., 50.));
    assert_eq!(
        grid(t),
        [
            (0, 0, 1, 1, s("a")),
            (0, 1, 1, 1, s("")),
            (1, 0, 1, 1, s("c")),
            (1, 1, 1, 1, s("xy")),
        ]
    );
}

#[test]
fn region_without_lines_is_one_cell() {
    let got = tables(&page(vec![], abcd()), Some((10., 10., 110., 50.)));
    assert_eq!(grid(&got[0]), [(0, 0, 1, 1, s("a|c|b|d"))]);
}

#[test]
fn too_many_lines_hit_the_work_limit() {
    let mut segs = Vec::new();
    for i in 0..1100 {
        let p = i as f64 * 5.;
        segs.extend([
            Segment::MoveTo(Point { x: 0., y: p }),
            Segment::LineTo(Point { x: 5500., y: p }),
            Segment::MoveTo(Point { x: p, y: 0. }),
            Segment::LineTo(Point { x: p, y: 5500. }),
        ]);
    }
    let got = find(&page(vec![path(segs)], vec![]), None);
    assert!(got.tables.is_empty());
    assert_eq!(got.diagnostics.len(), 1);
    assert_eq!(got.diagnostics[0].kind, DiagnosticKind::LayoutLimit);
}

#[test]
fn bad_settings_are_rejected() {
    let c = page(vec![], vec![]);
    let run = |ruling: RuledParams, region: Option<Rect>| {
        ruled(
            &c,
            MEDIA,
            Rotation::R0,
            &DISPLAY,
            region,
            &ruling,
            &Params::default(),
        )
    };
    let neg = RuledParams {
        join_tolerance: -1.,
        ..RuledParams::default()
    };
    assert_eq!(
        run(neg, None),
        Err(Error::InvalidParam {
            name: "join_tolerance"
        })
    );
    let nan = RuledParams {
        edge_min_length: f64::NAN,
        ..RuledParams::default()
    };
    assert!(run(nan, None).is_err());
    assert_eq!(
        run(RuledParams::default(), Some(r(f64::NAN, 0., 1., 1.))),
        Err(Error::InvalidParam { name: "region" })
    );
}

#[test]
fn side_by_side_tables_come_left_to_right() {
    let mut paths: Vec<Path> = two_by_two()
        .into_iter()
        .map(|mut p| {
            for s in &mut p.segments {
                if let Segment::MoveTo(q) | Segment::LineTo(q) = s {
                    q.x += 150.;
                }
            }
            p
        })
        .collect();
    paths.extend(two_by_two());
    let got = tables(&page(paths, abcd()), None);
    assert_eq!(got.len(), 2);
    assert_eq!((got[0].bbox.x0, got[1].bbox.x0), (10., 160.));
}

#[test]
fn missing_corner_leaves_no_cell() {
    let mut paths = two_by_two();
    paths[2] = line(10., 50., 60., 50.);
    paths[5] = line(110., 10., 110., 30.);
    let got = tables(&page(paths, abcd()), None);
    assert_eq!(
        grid(&got[0]),
        [
            (0, 0, 1, 1, s("a")),
            (0, 1, 1, 1, s("b")),
            (1, 0, 1, 1, s("c")),
        ]
    );
}

#[test]
fn column_of_crossings_hits_the_work_limit() {
    // One vertical rule crossed by short horizontal ones: no cells, but the search below each
    // crossing scans the rest of the column.
    let mut segs = vec![
        Segment::MoveTo(Point { x: 10., y: 0. }),
        Segment::LineTo(Point { x: 10., y: 20000. }),
    ];
    for i in 0..4200 {
        let y = i as f64 * 4.;
        segs.extend([
            Segment::MoveTo(Point { x: 0., y }),
            Segment::LineTo(Point { x: 20., y }),
        ]);
    }
    let got = find(&page(vec![path(segs)], vec![]), None);
    assert!(got.tables.is_empty());
    assert_eq!(got.diagnostics.len(), 1);
    assert_eq!(got.diagnostics[0].kind, DiagnosticKind::LayoutLimit);
}
