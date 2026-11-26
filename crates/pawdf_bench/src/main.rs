//! Timing harness for the pawdf pipeline. See `README.md` in this crate.

#![forbid(unsafe_code)]

use std::error::Error;
use std::hint::black_box;
use std::time::{Duration, Instant};

use pawdf_core::{Diagnostics, File, Location, Object, Origin, Ref, parse_content};
use pawdf_layout::{pdfminer, table};
use pawdf_reader::{Document, Page, PageContent};

type Res<T> = Result<T, Box<dyn Error>>;

const USAGE: &str = "usage: pawdf_bench <open|resolve|lex|stages|firstcontent|flow|cells|tagged|annots|docapi> <file.pdf>";

fn reps() -> usize {
    std::env::var("REPS")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|&n| n > 0)
        .unwrap_or(5)
}

fn median(mut v: Vec<Duration>) -> Duration {
    v.sort();
    v[v.len() / 2]
}

/// Median wall time of `REPS` runs of `f`, its result kept from being optimized away.
fn time<T>(mut f: impl FnMut() -> Res<T>) -> Res<Duration> {
    let mut v = Vec::new();
    for _ in 0..reps() {
        let t = Instant::now();
        black_box(f()?);
        v.push(t.elapsed());
    }
    Ok(median(v))
}

/// Every page with its content, interpreted once up front so later stages time only themselves.
fn pages(doc: &Document) -> Res<Vec<(Page, PageContent)>> {
    (0..doc.page_count())
        .map(|i| {
            let p = doc.page(i)?;
            let c = p.content();
            Ok((p, c))
        })
        .collect()
}

fn analyze(pages: &[(Page, PageContent)], params: &pdfminer::Params) -> Res<Duration> {
    time(|| {
        for (p, c) in pages {
            black_box(pdfminer::analyze(c, p.media_box(), p.rotate(), params)?);
        }
        Ok(())
    })
}

fn borderless(pages: &[(Page, PageContent)]) -> Res<Duration> {
    let params = pdfminer::Params::default();
    time(|| {
        for (p, c) in pages {
            black_box(table::borderless(
                c,
                p.media_box(),
                p.rotate(),
                &p.display_matrix(),
                None,
                &Default::default(),
                &params,
            )?);
        }
        Ok(())
    })
}

fn content_refs(f: &File, page: &Object) -> Vec<Ref> {
    match page.as_dict().and_then(|d| d.get(b"Contents")) {
        Some(Object::Ref(r)) => match f.resolve(*r) {
            Object::Array(a) => a.iter().filter_map(Object::as_ref).collect(),
            _ => vec![*r],
        },
        Some(Object::Array(a)) => a.iter().filter_map(Object::as_ref).collect(),
        _ => vec![],
    }
}

fn run(mode: &str, data: &[u8]) -> Res<String> {
    let doc = || Document::from_bytes(data.to_vec());
    Ok(match mode {
        "open" => format!("open {:?}", time(|| Ok(doc()?))?),
        "resolve" => {
            // The first pass parses every object; the second is all cache hits.
            let (mut cold, mut warm, mut n) = (vec![], vec![], 0);
            for _ in 0..reps() {
                let f = File::from_bytes(data.to_vec())?;
                let objs = f.objects();
                n = objs.len();
                let t = Instant::now();
                for r in &objs {
                    black_box(f.resolve(*r));
                }
                cold.push(t.elapsed());
                let t = Instant::now();
                for _ in 0..10 {
                    for r in &objs {
                        black_box(f.resolve(*r));
                    }
                }
                warm.push(t.elapsed() / 10);
            }
            format!(
                "objects={n} cold {:?} warm {:?}",
                median(cold),
                median(warm)
            )
        }
        "lex" => {
            let f = File::from_bytes(data.to_vec())?;
            let mut streams = vec![];
            for r in f.objects() {
                let o = f.resolve(r);
                if o.as_dict()
                    .and_then(|d| d.get(b"Type"))
                    .and_then(Object::as_name)
                    == Some(b"Page")
                {
                    for c in content_refs(&f, o) {
                        if let Object::Stream(s) = f.resolve(c) {
                            streams.push(s.decode(Location::File(0), &mut Diagnostics::new()).data);
                        }
                    }
                }
            }
            let bytes: usize = streams.iter().map(Vec::len).sum();
            let d = time(|| {
                for s in &streams {
                    black_box(parse_content(s, Origin::File, &mut Diagnostics::new()));
                }
                Ok(())
            })?;
            format!(
                "streams={} MB={:.1} parse_content {d:?}",
                streams.len(),
                bytes as f64 / 1e6
            )
        }
        "stages" => {
            let d = doc()?;
            let pages = pages(&d)?;
            let params = pdfminer::Params::default();
            let c = time(|| {
                for (p, _) in &pages {
                    black_box(p.content());
                }
                Ok(())
            })?;
            let l = analyze(&pages, &params)?;
            let r = time(|| {
                for (p, c) in &pages {
                    black_box(table::ruled(
                        c,
                        p.media_box(),
                        p.rotate(),
                        &p.display_matrix(),
                        None,
                        &Default::default(),
                        &params,
                    )?);
                }
                Ok(())
            })?;
            let b = borderless(&pages)?;
            format!(
                "pages={} content {c:?} layout {l:?} ruled {r:?} borderless {b:?}",
                pages.len()
            )
        }
        "firstcontent" => {
            // A fresh document each run, so font loading is included.
            let d = time(|| {
                let d = doc()?;
                for i in 0..d.page_count() {
                    black_box(d.page(i)?.content());
                }
                Ok(())
            })?;
            format!("content {d:?}")
        }
        "flow" => {
            let d = doc()?;
            let pages = pages(&d)?;
            let full = analyze(&pages, &pdfminer::Params::default())?;
            let flat = analyze(
                &pages,
                &pdfminer::Params {
                    boxes_flow: None,
                    ..Default::default()
                },
            )?;
            format!("analyze default {full:?} boxes_flow=None {flat:?}")
        }
        "cells" => {
            let d = doc()?;
            let pages = pages(&d)?;
            let params = pdfminer::Params::default();
            let (mut tables, mut cells, mut empty, mut one) = (0, 0, 0, 0);
            for (p, c) in &pages {
                let t = table::borderless(
                    c,
                    p.media_box(),
                    p.rotate(),
                    &p.display_matrix(),
                    None,
                    &Default::default(),
                    &params,
                )?;
                for tb in &t.tables {
                    tables += 1;
                    for c in &tb.cells {
                        cells += 1;
                        match c.layout.blocks.iter().map(|b| b.lines.len()).sum::<usize>() {
                            0 => empty += 1,
                            1 => one += 1,
                            _ => {}
                        }
                    }
                }
            }
            let a = analyze(&pages, &params)?;
            let b = borderless(&pages)?;
            format!(
                "tables={tables} cells={cells} empty={empty} one_line={one} analyze {a:?} borderless {b:?}"
            )
        }
        "tagged" => {
            let d = doc()?;
            let pages = pages(&d)?;
            let params = pdfminer::Params::default();
            let tree = d.structure_tree();
            let empty = PageContent::default();
            let full = time(|| {
                for (i, (p, c)) in pages.iter().enumerate() {
                    black_box(table::tagged(
                        c,
                        &tree,
                        i,
                        p.media_box(),
                        p.rotate(),
                        &p.display_matrix(),
                        None,
                        &params,
                    )?);
                }
                Ok(())
            })?;
            // No glyphs: what is left is the structure-tree walk.
            let walk = time(|| {
                for (i, (p, _)) in pages.iter().enumerate() {
                    black_box(table::tagged(
                        &empty,
                        &tree,
                        i,
                        p.media_box(),
                        p.rotate(),
                        &p.display_matrix(),
                        None,
                        &params,
                    )?);
                }
                Ok(())
            })?;
            format!("all pages {full:?} empty content {walk:?}")
        }
        "annots" => {
            let d = doc()?;
            let mut n = 0;
            for i in 0..d.page_count() {
                n += d.page(i)?.annotations().items.len();
            }
            let t = time(|| {
                for i in 0..d.page_count() {
                    black_box(d.page(i)?.annotations());
                }
                Ok(())
            })?;
            format!("annotations={n} all pages {t:?}")
        }
        "docapi" => {
            let d = doc()?;
            let o = time(|| Ok(d.outline()))?;
            let f = time(|| Ok(d.form_fields()))?;
            let s = time(|| Ok(d.structure_tree()))?;
            format!("outline {o:?} forms {f:?} struct {s:?}")
        }
        _ => return Err(USAGE.into()),
    })
}

fn main() -> Res<()> {
    let args: Vec<String> = std::env::args().collect();
    let [_, mode, path] = args.as_slice() else {
        return Err(USAGE.into());
    };
    let data = std::fs::read(path)?;
    println!("{mode} {path}: {}", run(mode, &data)?);
    Ok(())
}
